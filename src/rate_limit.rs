//! Rate limiting for message spam prevention.

use std::{
  collections::HashMap,
  sync::Arc,
  time::{Duration, Instant},
};
use tokio::sync::RwLock;

/// Configuration for rate limiting.
#[derive(Debug, Clone)]
pub struct MessageLimits {
  /// Maximum message size in bytes
  pub max_size_bytes: usize,
  /// Maximum messages per time window
  pub max_messages_per_window: u32,
  /// Time window for rate limiting
  pub window_duration: Duration,
  /// How long to ban users who exceed limits
  pub ban_duration: Duration,
}

impl Default for MessageLimits {
  fn default() -> Self {
    Self {
      max_size_bytes: 64 * 1024, // 64 KB
      max_messages_per_window: 10,
      window_duration: Duration::from_secs(1),
      ban_duration: Duration::from_secs(60),
    }
  }
}

#[derive(Debug)]
struct UserRateState {
  message_times: Vec<Instant>,
  banned_until: Option<Instant>,
}

impl UserRateState {
  fn new() -> Self {
    Self {
      message_times: Vec::new(),
      banned_until: None,
    }
  }

  fn is_banned(&self) -> bool {
    if let Some(until) = self.banned_until {
      Instant::now() < until
    } else {
      false
    }
  }

  fn ban(&mut self, duration: Duration) {
    self.banned_until = Some(Instant::now() + duration);
  }

  fn clean_old_messages(&mut self, window: Duration) {
    let cutoff = Instant::now() - window;
    self.message_times.retain(|&time| time > cutoff);
  }

  fn record_message(&mut self) {
    self.message_times.push(Instant::now());
  }
}

/// Rate limiter for users.
pub struct RateLimiter {
  limits: MessageLimits,
  states: Arc<RwLock<HashMap<String, UserRateState>>>,
}

impl RateLimiter {
  pub fn new(limits: MessageLimits) -> Self {
    Self {
      limits,
      states: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  /// Check if a user is allowed to send a message.
  /// Returns Ok(()) if allowed, Err(reason) if rejected.
  pub async fn check_allowed(&self, user_id: &str, message_size: usize) -> Result<(), String> {
    // Check message size
    if message_size > self.limits.max_size_bytes {
      return Err(format!(
        "Message too large: {} bytes (max {})",
        message_size, self.limits.max_size_bytes
      ));
    }

    let mut states = self.states.write().await;
    let state = states.entry(user_id.to_string()).or_insert_with(UserRateState::new);

    // Check if banned
    if state.is_banned() {
      return Err("You are temporarily banned for sending too many messages".to_string());
    }

    // Clean old messages
    state.clean_old_messages(self.limits.window_duration);

    // Check rate limit
    if state.message_times.len() >= self.limits.max_messages_per_window as usize {
      // User exceeded rate limit - ban them
      state.ban(self.limits.ban_duration);
      tracing::warn!("User {} exceeded rate limit and was banned", user_id);
      return Err(format!(
        "Rate limit exceeded: max {} messages per {} seconds",
        self.limits.max_messages_per_window,
        self.limits.window_duration.as_secs()
      ));
    }

    // Record this message
    state.record_message();

    Ok(())
  }

  /// Reset rate limit state for a user (e.g., for testing or admin actions).
  pub async fn reset_user(&self, user_id: &str) {
    self.states.write().await.remove(user_id);
  }

  /// Manually ban a user.
  pub async fn ban_user(&self, user_id: &str, duration: Duration) {
    let mut states = self.states.write().await;
    let state = states.entry(user_id.to_string()).or_insert_with(UserRateState::new);
    state.ban(duration);
  }

  /// Check if a user is currently banned.
  pub async fn is_banned(&self, user_id: &str) -> bool {
    let states = self.states.read().await;
    states.get(user_id).is_some_and(|s| s.is_banned())
  }

  /// Unban a user.
  pub async fn unban_user(&self, user_id: &str) {
    let mut states = self.states.write().await;
    if let Some(state) = states.get_mut(user_id) {
      state.banned_until = None;
    }
  }
}

impl Clone for RateLimiter {
  fn clone(&self) -> Self {
    Self {
      limits: self.limits.clone(),
      states: self.states.clone(),
    }
  }
}
