//! Unified publish/subscribe abstraction with in-memory and Redis backends.
//!
//! This module provides a trait [`PubSubBackend`] and two implementations:
//! [`InMemoryPubSub`] for local, in-process message passing, and [`RedisPubSub`]
//! for distributed Pub/Sub across processes and machines.
//!
//! Both backends serialize messages to JSON bytes internally, and expose a
//! consistent async interface for publishing and subscribing. In-memory
//! uses Tokio's [`broadcast`] channels with a configurable per-topic buffer,
//! while Redis uses its native `PUBSUB` feature with automatic reconnects.
//!
//! ## Example Usage
//! ```no_run
//! use tapaculo::pubsub::{InMemoryPubSub, PubSubBackend, PubSubExt};
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
//!     let _sub = backend.subscribe("chat", |bytes| async move {
//!         let msg: MyMsg = serde_json::from_slice(&bytes).unwrap();
//!         println!("Got message: {}", msg.val);
//!     }).await.unwrap();
//!
//!     // Publish a message
//!     backend.publish("chat", &MyMsg { val: "hello".into() }).await.unwrap();
//! }
//! ```

use crate::error::PubSubError;
use async_trait::async_trait;
use serde::Serialize;
use std::future::Future;
use tokio::{sync::oneshot, task::JoinHandle};

mod memory;
#[cfg(feature = "redis-backend")]
mod redis;

pub use memory::InMemoryPubSub;
#[cfg(feature = "redis-backend")]
pub use redis::{BackoffConfig, RedisPubSub};

/// A handle to a running subscription task.
///
/// You can `stop()` it gracefully (unsubscribe where supported) or `abort()` it immediately.
pub struct Subscription {
  handle: JoinHandle<()>,
  stop_tx: Option<oneshot::Sender<()>>, // signals graceful shutdown
}

impl Subscription {
  /// Create a new subscription handle
  pub(crate) fn new(handle: JoinHandle<()>, stop_tx: oneshot::Sender<()>) -> Self {
    Self {
      handle,
      stop_tx: Some(stop_tx),
    }
  }

  /// Request a graceful stop. Waits until the underlying task exits.
  pub async fn stop(mut self) {
    if let Some(tx) = self.stop_tx.take() {
      let _ = tx.send(());
    }
    let handle = self.handle;
    let _ = handle.await;
  }

  /// Abort immediately without unsubscribing.
  pub fn abort(self) {
    self.handle.abort();
  }

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
  /// Publish raw bytes to the given topic.
  ///
  /// ## Parameters
  /// - `topic`: The logical topic name.
  /// - `payload`: The serialized message bytes.
  ///
  /// ## Returns
  /// - `Ok(())` if the message was queued/sent.
  /// - `Err(PubSubError)` if the backend rejects it.
  async fn publish_bytes(&self, topic: &str, payload: Vec<u8>) -> Result<(), PubSubError>;

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
  ///   to stop gracefully or `Subscription::abort()` to cancel immediately.
  /// - `Err(PubSubError)` if the backend connection or subscription fails.
  async fn subscribe_bytes(
    &self,
    topic: &str,
    handler: Box<dyn Fn(Vec<u8>) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send>,
  ) -> Result<Subscription, PubSubError>;
}

/// Extension trait providing convenient generic methods for PubSubBackend.
#[async_trait]
pub trait PubSubExt: PubSubBackend {
  /// Publish a serializable message to the given topic.
  ///
  /// ## Parameters
  /// - `topic`: The logical topic name.
  /// - `msg`: Any [`serde::Serialize`] type.
  ///
  /// ## Returns
  /// - `Ok(())` if the message was queued/sent.
  /// - `Err(PubSubError)` if serialization fails or the backend rejects it.
  async fn publish<T: Serialize + Send + Sync>(
    &self,
    topic: &str,
    msg: &T,
  ) -> Result<(), PubSubError> {
    let payload = serde_json::to_vec(msg)?;
    self.publish_bytes(topic, payload).await
  }

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
  ///   to stop gracefully or `Subscription::abort()` to cancel immediately.
  /// - `Err(PubSubError)` if the backend connection or subscription fails.
  async fn subscribe<F, Fut>(&self, topic: &str, handler: F) -> Result<Subscription, PubSubError>
  where
    F: Fn(Vec<u8>) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
  {
    self
      .subscribe_bytes(
        topic,
        Box::new(move |bytes| Box::pin(handler(bytes))),
      )
      .await
  }
}

// Blanket implementation
impl<T: PubSubBackend + ?Sized> PubSubExt for T {}
