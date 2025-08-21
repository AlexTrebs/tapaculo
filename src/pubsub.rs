//! Unified publish/subscribe abstraction with in-memory and Redis backends.
//!
//! This module provides a trait [`PubSubBackend`] and two implementations:
//! [`InMemoryPubSub`] for local, in-process message passing, and [`RedisPubSub`]
//! for distributed Pub/Sub across processes and machines.
//!
//! Both backends serialize messages to JSON bytes internally, and expose a
//! consistent async interface for publishing and subscribing. In-memory
//! uses Tokio’s [`broadcast`] channels with a configurable per-topic buffer,
//! while Redis uses its native `PUBSUB` feature with automatic reconnects.
//!
//! ## Example Usage
//! ```no_run
//! use tapaculo::pubsub::{InMemoryPubSub, PubSubBackend};
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Serialize, Deserialize)]
//! struct MyMsg { val: String }
//!
//! #[tokio::main]
//! async fn main() {
//!     let backend = InMemoryPubSub::new();
//!
//!     // Subscribe to messages
//!     backend.subscribe("chat", |bytes| async move {
//!         let msg: MyMsg = serde_json::from_slice(&bytes).unwrap();
//!         println!("Got message: {}", msg.val);
//!     }).await.unwrap();
//!
//!     // Publish a message
//!     backend.publish("chat", &MyMsg { val: "hello".into() }).await.unwrap();
//! }
//! ```
use async_trait::async_trait;
use crate::error::PubSubError;
use redis::Client;
use serde::Serialize;
use serde_json::to_vec;
use std::{
  collections::HashMap,
  future::Future,
  sync::{Arc, Mutex},
};
use tokio::{sync::{broadcast, oneshot}, task::JoinHandle};

/// A handle to a running subscription task.
///
/// You can `stop()` it gracefully (unsubscribe where supported) or `abort()` it immediately.
pub struct Subscription {
  handle: JoinHandle<()>,
  stop_tx: Option<oneshot::Sender<()>>, // signals graceful shutdown
}

impl Subscription {
  /// Request a graceful stop. Waits until the underlying task exits.
  pub async fn stop(mut self) {
    if let Some(tx) = self.stop_tx.take() { let _ = tx.send(()); }
    // Move the handle out explicitly before awaiting
    let handle = self.handle;
    let _ = handle.await;
  }

  /// Abort immediately without unsubscribing.
  pub fn abort(self) { self.handle.abort(); }

  pub fn is_active(&self) -> bool {
    !self.handle.is_finished()
  }
}

/// Trait implemented by all Pub/Sub backends.
///
/// Defines the minimal interface to publish messages to topics and
/// subscribe to incoming messages.
///
/// All messages are serialized to `Vec<u8>` before sending, and handlers
/// receive the raw bytes to allow custom deserialization.
///
/// ## Example
/// ```ignore
/// backend.publish("topic", &my_struct).await?;
/// backend.subscribe("topic", |bytes| async move {
///     println!("Got {} bytes", bytes.len());
/// }).await?;
/// ```
#[async_trait]
pub trait PubSubBackend: Send + Sync {
  /// Publish a serializable message to the given topic.
  ///
  /// ## Parameters
  /// - `topic`: The logical topic name.
  /// - `msg`: Any [`serde::Serialize`] type.
  ///
  /// ## Returns
  /// - `Ok(())` if the message was queued/sent.
  /// - `Err(PubSubError)` if serialization fails or the backend rejects it.
  async fn publish<T: Serialize + Send + Sync>(&self, topic: &str, msg: &T) -> Result<(), PubSubError>;

  /// Subscribe to a topic and invoke `handler` for each incoming message.
  ///
  /// The handler runs on a spawned task; it should be lightweight or spawn its own work.
  ///
  /// ## Parameters
  /// - `topic`: The logical topic name.
  /// - `handler`: Async function taking the raw message payload.
  ///
  /// ## Returns
  /// - `Ok(Subscription)` if the subscription was successfully established. Call `Subscription::stop()` 
  //      to stop gracefully or `Subscription::abort()` to cancel immediately.
  /// - `Err(PubSubError)` if the backend connection or subscription fails.
  async fn subscribe<F, Fut>(&self, topic: &str, handler: F) -> Result<Subscription, PubSubError>
  where
    F: Fn(Vec<u8>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static;
}

/// In-memory Pub/Sub using a Tokio `broadcast` channel per topic.
///
/// Best for local, single-process, real-time messaging without external dependencies.
/// Each topic has a fixed-size ring buffer; older messages are dropped when the buffer is full.
#[derive(Clone)]
pub struct InMemoryPubSub {
  topics: Arc<Mutex<HashMap<String, broadcast::Sender<Vec<u8>>>>>,
  buffer: usize,
}

impl InMemoryPubSub {
  /// Create a new in-memory backend with a default per-topic buffer of 64 messages.
  pub fn new() -> Self {
    Self::with_buffer(64)
  }

  /// Create with a custom buffer size.
  ///
  /// ## Parameters
  /// - `buffer`: Number of messages retained per topic before overwriting oldest.
  ///
  /// Minimum size is 1.
  pub fn with_buffer(buffer: usize) -> Self {
    Self {
      topics: Arc::new(Mutex::new(HashMap::new())),
      buffer: buffer.max(1),
    }
  }

  /// Get or create the broadcast channel for a topic.
  fn get_or_create_sender(&self, topic: &str) -> broadcast::Sender<Vec<u8>> {
    let mut map = self.topics.lock().expect("InMemoryPubSub lock poisoned");

    map.entry(topic.to_string())
      .or_insert_with(|| {
        let (tx, _rx) = broadcast::channel(self.buffer);
        tx
      })
      .clone()
  }
}

#[async_trait]
impl PubSubBackend for InMemoryPubSub {
  async fn publish<T: Serialize + Send + Sync>(&self, topic: &str, msg: &T) -> Result<(), PubSubError> {
    let payload = to_vec(msg)?;
    let sender = self.get_or_create_sender(topic);

    match sender.send(payload) {
      Ok(_) => Ok(()),
      Err(_) => Err(PubSubError::NoSubscribers(topic.to_string())),
    }
  }

  async fn subscribe<F, Fut>(&self, topic: &str, handler: F) -> Result<Subscription, PubSubError>
  where
    F: Fn(Vec<u8>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
  {
    let sender = self.get_or_create_sender(topic);
    let mut rx = sender.subscribe();

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();

    let handle = tokio::spawn(async move {
      loop {
        tokio::select! {
          res = rx.recv() => {
            match res {
              Ok(msg) => handler(msg).await,
              Err(broadcast::error::RecvError::Lagged(_)) => {
                // Drop lagged messages, keep the subscription alive
                continue;
              }
              Err(broadcast::error::RecvError::Closed) => break,
            }
          }
          _ = &mut stop_rx => {
            break;
          }
        }
      }
    });

    Ok(Subscription {
      handle,
      stop_tx: Some(stop_tx),
    })
  }
}

const BASE_DELAY_MS: u64 = 100;
const MAX_DELAY_MS: u64 = 5_000;
const MAX_EXPONENT: u32 = 6;
const JITTER_RANGE: u64 = 50;

#[derive(Clone, Debug)]
pub struct BackoffConfig {
  pub base_delay_ms: u64,
  pub max_delay_ms: u64,
  pub max_exponent: u32,
  pub jitter_range: u64,
  pub max_retries: u32,
}

impl Default for BackoffConfig {
  fn default() -> Self {
    Self {
      base_delay_ms: BASE_DELAY_MS,
      max_delay_ms: MAX_DELAY_MS,
      max_exponent: MAX_EXPONENT,
      jitter_range: JITTER_RANGE,
      max_retries: 10,
    }
  }
}

/// Redis-backed Pub/Sub for inter-process messaging.
///
/// Uses a dedicated connection for publishing and a `PUBSUB` subscription for receiving.
/// Automatically attempts to reconnect with exponential backoff on connection loss.
#[derive(Clone)]
pub struct RedisPubSub {
  client: Client,
  backoff_config: BackoffConfig,
}

impl RedisPubSub {
  /// Create a new Redis backend from a connection string.
  ///
  /// Example: `RedisPubSub::new("redis://127.0.0.1/")?`
  pub fn new(addr: &str) -> Result<Self, PubSubError> {
    Ok(Self { 
      client: Client::open(addr)?,
      backoff_config: BackoffConfig::default()
    })
  }

  /// Create a new Redis backend from a connection string and configuration.
  ///
  /// Example: `RedisPubSub::with_config("redis://127.0.0.1/", BackoffConfig::default())?`
  pub fn with_config(addr: &str, config: BackoffConfig) -> Result<Self, PubSubError> {
    Ok(Self { 
      client: Client::open(addr)?,
      backoff_config: config
    })
  }
}

#[async_trait]
impl PubSubBackend for RedisPubSub {
  async fn publish<T: Serialize + Send + Sync>(&self, topic: &str, msg: &T) -> Result<(), PubSubError> {
    let payload = to_vec(msg)?;

    let mut conn: redis::aio::MultiplexedConnection = self.client.get_multiplexed_tokio_connection().await?;

    redis::AsyncCommands::publish::<&str, Vec<u8>, ()>(&mut conn, topic, payload).await?;
    Ok(())
  }

  async fn subscribe<F, Fut>(&self, topic: &str, handler: F) -> Result<tokio::task::JoinHandle<()>, PubSubError>
  where
    F: Fn(Vec<u8>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
  {
    let client = self.client.clone();
    let topic = topic.to_string();
    let cfg = self.backoff_config.clone();

    let handle = tokio::spawn(async move {
      let mut attempt: u32 = 0;

      loop {
        // Health check: PING before subscribe
        match client.get_multiplexed_tokio_connection().await {
          Ok(mut conn) => {
            let ping_result: redis::RedisResult<String> = redis::AsyncCommands::ping(&mut conn).await;
            if ping_result.is_err() {
              eprintln!("Redis health check failed: {:?}", ping_result.err());
              attempt += 1;
            } else {
              // Try to subscribe
              match client.get_async_pubsub().await {
                Ok(mut pubsub) => {
                  if let Err(e) = pubsub.subscribe(&topic).await {
                    eprintln!("Redis subscribe failed for '{topic}': {e}");
                    attempt += 1;
                  } else {
                    attempt = 0; // Reset on success
                    let mut stream = pubsub.on_message();
                    while let Some(msg) = stream.next().await {
                      match msg.get_payload::<Vec<u8>>() {
                        Ok(payload) => handler(payload).await,
                        Err(e) => eprintln!("Redis payload decode error on '{topic}': {e}"),
                      }
                    }
                    eprintln!("Redis pubsub stream ended for '{topic}', will reconnect…");
                  }
                }
                Err(e) => {
                  eprintln!("Redis pubsub connection error: {e}. Retrying…");
                  attempt += 1;
                }
              }
            }
          }
          Err(e) => {
            eprintln!("Redis connection error: {e}. Retrying…");
            attempt += 1;
          }
        }

        if attempt >= cfg.max_retries {
          eprintln!("Redis pubsub: exceeded max retries ({}) for topic '{}', giving up.", cfg.max_retries, topic);
          break;
        }

        // exponential backoff with jitter
        let exp = attempt.min(cfg.max_exponent);
        let base_ms = cfg.base_delay_ms.saturating_mul(2u64.saturating_pow(exp));
        let jitter_ms = (attempt as u64 % cfg.jitter_range) + 1;
        let delay = std::time::Duration::from_millis((base_ms + jitter_ms).min(cfg.max_delay_ms));
        tokio::time::sleep(delay).await;
      }
    });

    Ok(handle)
  }
}

/// ######################################## TESTS ########################################

#[cfg(test)]
mod tests {
  use super::*;
  use serde::{Deserialize, Serialize};
  use std::time::Duration;

  #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
  struct TestMsg {
    val: String,
  }

  #[tokio::test]
  async fn in_memory_pubsub_works() {
    let backend = InMemoryPubSub::new();

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    backend.subscribe("topic", move |data| {
      let tx = tx.clone();
      async move {
        let msg: TestMsg = serde_json::from_slice(&data).unwrap();
        tx.send(msg).await.ok();
      }
    }).await.unwrap();

    backend.publish("topic", &TestMsg { val: "hello".into() }).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(1), rx.recv())
      .await
      .unwrap()
      .unwrap();
    assert_eq!(received, TestMsg { val: "hello".into() });
  }

  // This test requires a running Redis instance at localhost:6379
  // Run: `docker run -p 6379:6379 redis`
  #[tokio::test]
  #[ignore]
  async fn redis_pubsub_works() {
    let backend = RedisPubSub::new("redis://127.0.0.1/").unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    backend.subscribe("topic", move |data| {
      let tx = tx.clone();
      async move {
        let msg: TestMsg = serde_json::from_slice(&data).unwrap();
        tx.send(msg).await.ok();
      }
    }).await.unwrap();

    backend.publish("topic", &TestMsg { val: "hello".into() }).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(1), rx.recv())
      .await
      .unwrap()
      .unwrap();
    assert_eq!(received, TestMsg { val: "hello".into() });
  }
}
