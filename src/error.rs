use redis::RedisError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PubSubError {
  #[error("Serialization error: {0}")]
  Serialization(#[from] serde_json::Error),

  #[error("Backend error: {0}")]
  Backend(#[from] RedisError),

  #[error("No active subscribers for topic '{0}'")]
  NoSubscribers(String),
}