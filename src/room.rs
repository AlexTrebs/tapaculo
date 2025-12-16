//! Room management for multiplayer sessions.

use serde::{Deserialize, Serialize};
use std::{
  collections::{HashMap, VecDeque},
  sync::Arc,
  time::{Duration, Instant},
};
use tokio::sync::RwLock;

/// Player metadata associated with a user in a room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerMetadata {
  pub user_id: String,
  pub display_name: String,
  pub avatar_url: Option<String>,
  #[serde(skip, default = "Instant::now")]
  pub joined_at: Instant,
  #[serde(flatten)]
  pub custom: serde_json::Value,
}

impl PlayerMetadata {
  pub fn new(user_id: String, display_name: String) -> Self {
    Self {
      user_id,
      display_name,
      avatar_url: None,
      joined_at: Instant::now(),
      custom: serde_json::Value::Object(serde_json::Map::new()),
    }
  }
}

/// Configuration for room behavior.
#[derive(Debug, Clone)]
pub struct RoomSettings {
  /// Maximum number of players allowed (None = unlimited)
  pub max_players: Option<usize>,
  /// Whether to allow spectators (read-only participants)
  pub allow_spectators: bool,
  /// Whether to store message history
  pub store_message_history: bool,
  /// Maximum number of messages to keep in history
  pub max_history_messages: usize,
  /// Auto-delete room after this duration of being empty
  pub empty_room_timeout: Option<Duration>,
}

impl Default for RoomSettings {
  fn default() -> Self {
    Self {
      max_players: None,
      allow_spectators: true,
      store_message_history: false,
      max_history_messages: 100,
      empty_room_timeout: Some(Duration::from_secs(300)), // 5 minutes
    }
  }
}

/// Information about a room's current state.
#[derive(Debug, Clone, Serialize)]
pub struct RoomInfo {
  pub room_id: String,
  pub member_count: usize,
  pub max_members: Option<usize>,
  pub is_full: bool,
  #[serde(skip)]
  pub created_at: Instant,
  pub members: Vec<String>,
}

/// Stored message in room history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
  pub from: String,
  pub data: serde_json::Value,
  #[serde(skip, default = "Instant::now")]
  pub timestamp: Instant,
}

/// Internal room state.
pub struct Room {
  pub id: String,
  pub settings: RoomSettings,
  pub players: HashMap<String, PlayerMetadata>,
  pub message_history: VecDeque<StoredMessage>,
  pub created_at: Instant,
  pub last_activity: Instant,
}

impl Room {
  pub fn new(id: String, settings: RoomSettings) -> Self {
    let now = Instant::now();
    Self {
      id,
      settings,
      players: HashMap::new(),
      message_history: VecDeque::new(),
      created_at: now,
      last_activity: now,
    }
  }

  pub fn add_player(&mut self, metadata: PlayerMetadata) -> Result<(), String> {
    if let Some(max) = self.settings.max_players {
      if self.players.len() >= max {
        return Err(format!("Room is full (max {} players)", max));
      }
    }

    self.players.insert(metadata.user_id.clone(), metadata);
    self.last_activity = Instant::now();
    Ok(())
  }

  pub fn remove_player(&mut self, user_id: &str) -> Option<PlayerMetadata> {
    self.last_activity = Instant::now();
    self.players.remove(user_id)
  }

  pub fn is_full(&self) -> bool {
    if let Some(max) = self.settings.max_players {
      self.players.len() >= max
    } else {
      false
    }
  }

  pub fn is_empty(&self) -> bool {
    self.players.is_empty()
  }

  pub fn add_message(&mut self, msg: StoredMessage) {
    if self.settings.store_message_history {
      self.message_history.push_back(msg);
      while self.message_history.len() > self.settings.max_history_messages {
        self.message_history.pop_front();
      }
    }
    self.last_activity = Instant::now();
  }

  pub fn get_info(&self) -> RoomInfo {
    RoomInfo {
      room_id: self.id.clone(),
      member_count: self.players.len(),
      max_members: self.settings.max_players,
      is_full: self.is_full(),
      created_at: self.created_at,
      members: self.players.keys().cloned().collect(),
    }
  }
}

/// Manager for all rooms in the server.
pub struct RoomManager {
  rooms: Arc<RwLock<HashMap<String, Arc<RwLock<Room>>>>>,
  default_settings: RoomSettings,
}

impl RoomManager {
  pub fn new(default_settings: RoomSettings) -> Self {
    Self {
      rooms: Arc::new(RwLock::new(HashMap::new())),
      default_settings,
    }
  }

  pub async fn get_or_create_room(&self, room_id: &str) -> Arc<RwLock<Room>> {
    let rooms = self.rooms.read().await;
    if let Some(room) = rooms.get(room_id) {
      return room.clone();
    }
    drop(rooms);

    // Create new room
    let mut rooms = self.rooms.write().await;
    // Double-check after acquiring write lock
    if let Some(room) = rooms.get(room_id) {
      return room.clone();
    }

    let room = Arc::new(RwLock::new(Room::new(
      room_id.to_string(),
      self.default_settings.clone(),
    )));
    rooms.insert(room_id.to_string(), room.clone());
    room
  }

  pub async fn get_room(&self, room_id: &str) -> Option<Arc<RwLock<Room>>> {
    self.rooms.read().await.get(room_id).cloned()
  }

  pub async fn remove_room(&self, room_id: &str) -> Option<Arc<RwLock<Room>>> {
    self.rooms.write().await.remove(room_id)
  }

  pub async fn cleanup_empty_rooms(&self) {
    let now = Instant::now();
    let mut rooms_to_remove = Vec::new();

    let rooms = self.rooms.read().await;
    for (room_id, room) in rooms.iter() {
      let room_guard = room.read().await;
      if room_guard.is_empty() {
        if let Some(timeout) = room_guard.settings.empty_room_timeout {
          if now.duration_since(room_guard.last_activity) > timeout {
            rooms_to_remove.push(room_id.clone());
          }
        }
      }
    }
    drop(rooms);

    if !rooms_to_remove.is_empty() {
      let mut rooms = self.rooms.write().await;
      for room_id in rooms_to_remove {
        rooms.remove(&room_id);
        tracing::info!("Cleaned up empty room: {}", room_id);
      }
    }
  }

  pub async fn get_all_room_ids(&self) -> Vec<String> {
    self.rooms.read().await.keys().cloned().collect()
  }
}

impl Clone for RoomManager {
  fn clone(&self) -> Self {
    Self {
      rooms: self.rooms.clone(),
      default_settings: self.default_settings.clone(),
    }
  }
}
