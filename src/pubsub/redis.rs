//! Redis Pub/Sub implementation with automatic reconnection.

use super::{PubSubBackend, Subscription};
use crate::error::PubSubError;
use async_trait::async_trait;
use redis::Client;
use tokio::sync::oneshot;
use tokio_stream::StreamExt;

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
      backoff_config: BackoffConfig::default(),
    })
  }

  /// Create a new Redis backend from a connection string and configuration.
  ///
  /// Example: `RedisPubSub::with_config("redis://127.0.0.1/", BackoffConfig::default())?`
  pub fn with_config(addr: &str, config: BackoffConfig) -> Result<Self, PubSubError> {
    Ok(Self {
      client: Client::open(addr)?,
      backoff_config: config,
    })
  }
}

#[async_trait]
impl PubSubBackend for RedisPubSub {
  async fn publish_bytes(&self, topic: &str, payload: Vec<u8>) -> Result<(), PubSubError> {
    let mut conn: redis::aio::MultiplexedConnection =
      self.client.get_multiplexed_tokio_connection().await?;

    redis::AsyncCommands::publish::<&str, Vec<u8>, ()>(&mut conn, topic, payload).await?;
    Ok(())
  }

  async fn subscribe_bytes(
    &self,
    topic: &str,
    handler: Box<dyn Fn(Vec<u8>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send>,
  ) -> Result<Subscription, PubSubError> {
    let client = self.client.clone();
    let topic = topic.to_string();
    let cfg = self.backoff_config.clone();

    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();

    let handle = tokio::spawn(async move {
      let mut attempt: u32 = 0;

      loop {
        // Check if we should stop
        if stop_rx.try_recv().is_ok() {
          tracing::debug!("Redis subscription for '{topic}' stopped gracefully");
          break;
        }

        // Health check: PING before subscribe
        match client.get_multiplexed_tokio_connection().await {
          Ok(mut conn) => {
            let ping_result: redis::RedisResult<String> =
              redis::AsyncCommands::ping(&mut conn).await;
            if ping_result.is_err() {
              tracing::error!("Redis health check failed: {:?}", ping_result.err());
              attempt += 1;
            } else {
              // Try to subscribe
              match client.get_async_pubsub().await {
                Ok(mut pubsub) => {
                  if let Err(e) = pubsub.subscribe(&topic).await {
                    tracing::error!("Redis subscribe failed for '{topic}': {e}");
                    attempt += 1;
                  } else {
                    attempt = 0; // Reset on success
                    tracing::info!("Successfully subscribed to Redis topic '{topic}'");
                    let mut stream = pubsub.on_message();
                    let should_stop = loop {
                      tokio::select! {
                        msg_opt = stream.next() => {
                          match msg_opt {
                            Some(msg) => {
                              match msg.get_payload::<Vec<u8>>() {
                                Ok(payload) => handler(payload).await,
                                Err(e) => tracing::error!("Redis payload decode error on '{topic}': {e}"),
                              }
                            }
                            None => {
                              tracing::warn!("Redis pubsub stream ended for '{topic}', will reconnect…");
                              break false;
                            }
                          }
                        }
                        _ = &mut stop_rx => {
                          tracing::debug!("Gracefully stopping Redis subscription for '{topic}'");
                          break true;
                        }
                      }
                    };

                    // Drop stream to release borrow before calling unsubscribe
                    drop(stream);

                    if should_stop {
                      let _ = pubsub.unsubscribe(&topic).await;
                      return;
                    }
                  }
                }
                Err(e) => {
                  tracing::error!("Redis pubsub connection error: {e}. Retrying…");
                  attempt += 1;
                }
              }
            }
          }
          Err(e) => {
            tracing::error!("Redis connection error: {e}. Retrying…");
            attempt += 1;
          }
        }

        if attempt >= cfg.max_retries {
          tracing::error!(
            "Redis pubsub: exceeded max retries ({}) for topic '{}', giving up.",
            cfg.max_retries,
            topic
          );
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

    Ok(Subscription::new(handle, stop_tx))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::pubsub::PubSubExt;
  use serde::{Deserialize, Serialize};
  use std::time::Duration;

  #[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
  struct TestMsg {
    val: String,
  }

  // This test requires a running Redis instance at localhost:6379
  // Run: `docker run -p 6379:6379 redis`
  #[tokio::test]
  #[ignore]
  async fn redis_pubsub_works() {
    let backend = RedisPubSub::new("redis://127.0.0.1/").unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    backend
      .subscribe("topic", move |data| {
        let tx = tx.clone();
        async move {
          let msg: TestMsg = serde_json::from_slice(&data).unwrap();
          tx.send(msg).await.ok();
        }
      })
      .await
      .unwrap();

    backend
      .publish(
        "topic",
        &TestMsg {
          val: "hello".into(),
        },
      )
      .await
      .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(1), rx.recv())
      .await
      .unwrap()
      .unwrap();
    assert_eq!(
      received,
      TestMsg {
        val: "hello".into()
      }
    );
  }
}
