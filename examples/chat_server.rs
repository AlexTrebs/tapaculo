//! Example: group chat with message history, typing indicators, and user metadata.

use serde::{Deserialize, Serialize};
use tapaculo::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ChatMessage {
  Text { content: String },
  Typing { is_typing: bool },
  Reaction { message_id: String, emoji: String },
  SetDisplayName { name: String },
  SetAvatar { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ChatEvent {
  Message {
    from: String,
    from_name: String,
    content: String,
    timestamp: u64,
  },
  UserJoined {
    user_id: String,
    display_name: String,
  },
  UserLeft {
    user_id: String,
  },
  UserTyping {
    user_id: String,
    is_typing: bool,
  },
  Reaction {
    message_id: String,
    user_id: String,
    emoji: String,
  },
  RoomInfo {
    members: Vec<String>,
    history: Vec<String>,
  },
}

/// Chat-specific event handler
struct ChatEventHandler;

#[async_trait::async_trait]
impl RoomEventHandler for ChatEventHandler {
  async fn on_player_joined(&self, ctx: &Context, user_id: &str) {
    let metadata = ctx.get_user_metadata(user_id).await
      .unwrap_or_else(|| PlayerMetadata::new(user_id.to_string(), format!("User {}", user_id)));

    let _ = ctx.broadcast_to_others(ChatEvent::UserJoined {
      user_id: user_id.to_string(),
      display_name: metadata.display_name.clone(),
    }).await;

    let members = ctx.get_room_members().await;
    let history = ctx.get_message_history(50).await
      .iter()
      .map(|msg| format!("{}: {:?}", msg.from, msg.data))
      .collect();
    let _ = ctx.send_to(user_id, ChatEvent::RoomInfo { members, history }).await;
  }

  async fn on_player_left(&self, ctx: &Context, user_id: &str) {
    let _ = ctx.broadcast_to_others(ChatEvent::UserLeft { user_id: user_id.to_string() }).await;
  }

  async fn on_room_empty(&self, room_id: &str) {
    tracing::debug!("chat room {} empty", room_id);
  }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  // Initialize tracing
  tracing_subscriber::fmt()
    .with_env_filter("chat_server=debug,tapaculo=info")
    .init();

  let auth = JwtAuth::new("chat-secret-key");
  let pubsub = InMemoryPubSub::new();

  // Configure for group chat
  let room_settings = RoomSettings {
    max_players: Some(50), // Max 50 people in a chat room
    allow_spectators: true,
    store_message_history: true,
    max_history_messages: 200, // Keep last 200 messages
    empty_room_timeout: Some(std::time::Duration::from_secs(600)), // 10 minutes
  };

  // Configure rate limiting
  let limits = MessageLimits {
    max_size_bytes: 10 * 1024, // 10KB max per message (for images/links)
    max_messages_per_window: 10,
    window_duration: std::time::Duration::from_secs(1),
    ban_duration: std::time::Duration::from_secs(300), // 5 minute ban
  };

  Server::new()
    .with_auth(auth)
    .with_pubsub(pubsub)
    .with_room_settings(room_settings)
    .with_limits(limits)
    .with_event_handler(ChatEventHandler)
    .on_message_typed::<ChatMessage, _, _>(|ctx, envelope| async move {
      match envelope.data {
        ChatMessage::Text { content } => {
          let from_name = ctx.get_user_metadata(&envelope.from).await
            .map(|m| m.display_name.clone())
            .unwrap_or_else(|| envelope.from.clone());
          let _ = ctx.broadcast(ChatEvent::Message {
            from: envelope.from.clone(),
            from_name,
            content,
            timestamp: std::time::SystemTime::now()
              .duration_since(std::time::UNIX_EPOCH)
              .unwrap()
              .as_secs(),
          }).await;
        }
        ChatMessage::Typing { is_typing } => {
          let _ = ctx.broadcast_to_others(ChatEvent::UserTyping {
            user_id: envelope.from.clone(),
            is_typing,
          }).await;
        }
        ChatMessage::Reaction { message_id, emoji } => {
          let _ = ctx.broadcast(ChatEvent::Reaction {
            message_id,
            user_id: envelope.from.clone(),
            emoji,
          }).await;
        }
        ChatMessage::SetDisplayName { name } => {
          let mut metadata = ctx.get_user_metadata(&envelope.from).await
            .unwrap_or_else(|| PlayerMetadata::new(envelope.from.clone(), name.clone()));
          metadata.display_name = name.clone();
          let _ = ctx.set_user_metadata(metadata).await;
          let _ = ctx.broadcast_to_others(ChatEvent::UserJoined {
            user_id: envelope.from.clone(),
            display_name: name,
          }).await;
        }
        ChatMessage::SetAvatar { url } => {
          let mut metadata = ctx.get_user_metadata(&envelope.from).await
            .unwrap_or_else(|| PlayerMetadata::new(envelope.from.clone(), envelope.from.clone()));
          metadata.avatar_url = Some(url);
          let _ = ctx.set_user_metadata(metadata).await;
        }
      }
    })
    .on_message_validate(|_ctx, envelope| {
      if envelope.from.is_empty() {
        return Err("Invalid sender".to_string());
      }
      Ok(())
    })
    .listen("0.0.0.0:8081")
    .await?;

  Ok(())
}

