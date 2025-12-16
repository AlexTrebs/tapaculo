//! In-memory Pub/Sub implementation using Tokio broadcast channels.

use super::{PubSubBackend, Subscription};
use crate::error::PubSubError;
use async_trait::async_trait;
use std::{
  collections::HashMap,
  future::Future,
  sync::{Arc, Mutex},
};
use tokio::sync::{broadcast, oneshot};

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

    map
      .entry(topic.to_string())
      .or_insert_with(|| {
        let (tx, _rx) = broadcast::channel(self.buffer);
        tx
      })
      .clone()
  }
}

impl Default for InMemoryPubSub {
  fn default() -> Self {
    Self::new()
  }
}

#[async_trait]
impl PubSubBackend for InMemoryPubSub {
  async fn publish_bytes(&self, topic: &str, payload: Vec<u8>) -> Result<(), PubSubError> {
    let sender = self.get_or_create_sender(topic);

    // For broadcast channels, send() can fail if there are no active receivers.
    // This is not an error in a pub/sub context - the message is simply dropped.
    // We still return Ok(()) since the publish operation itself succeeded.
    let _ = sender.send(payload);
    Ok(())
  }

  async fn subscribe_bytes(
    &self,
    topic: &str,
    handler: Box<dyn Fn(Vec<u8>) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>> + Send>,
  ) -> Result<Subscription, PubSubError> {
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

  #[tokio::test]
  async fn in_memory_pubsub_works() {
    let backend = InMemoryPubSub::new();

    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

    let _sub = backend
      .subscribe("topic", move |data| {
        let tx = tx.clone();
        async move {
          let msg: TestMsg = serde_json::from_slice(&data).unwrap();
          tx.send(msg).await.ok();
        }
      })
      .await
      .unwrap();

    // Signal that subscription is ready
    let _ = ready_tx.send(());

    // Wait for subscription to be ready
    ready_rx.await.ok();

    // Give subscription task a moment to start polling
    tokio::time::sleep(Duration::from_millis(50)).await;

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

  #[tokio::test]
  async fn default_impl_works() {
    let backend = InMemoryPubSub::default();
    assert!(backend.buffer > 0);
  }
}
