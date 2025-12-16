//! Chat server example demonstrating message history, user metadata, and flexible room sizes.

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
    tracing::info!("User {} joined chat room {}", user_id, ctx.room_id());

    // Get or set user metadata
    let metadata = ctx.get_user_metadata(user_id).await.unwrap_or_else(|| {
      PlayerMetadata::new(user_id.to_string(), format!("User {}", user_id))
    });

    // Send join event to all other members
    let event = ChatEvent::UserJoined {
      user_id: user_id.to_string(),
      display_name: metadata.display_name.clone(),
    };
    let _ = ctx.broadcast_to_others(event).await;

    // Send room info to the new joiner (members + recent history)
    let members = ctx.get_room_members().await;
    let history = ctx
      .get_message_history(50)
      .await
      .iter()
      .map(|msg| format!("{}: {:?}", msg.from, msg.data))
      .collect();

    let room_info = ChatEvent::RoomInfo { members, history };
    let _ = ctx.send_to(user_id, room_info).await;
  }

  async fn on_player_left(&self, ctx: &Context, user_id: &str) {
    tracing::info!("User {} left chat room {}", user_id, ctx.room_id());

    // Notify others
    let event = ChatEvent::UserLeft {
      user_id: user_id.to_string(),
    };
    let _ = ctx.broadcast_to_others(event).await;
  }

  async fn on_room_empty(&self, room_id: &str) {
    tracing::info!("Chat room {} is now empty and will be cleaned up", room_id);
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
      tracing::debug!("Chat message from {}: {:?}", envelope.from, envelope.data);

      match envelope.data {
        ChatMessage::Text { content } => {
          // Get user's display name
          let metadata = ctx.get_user_metadata(&envelope.from).await;
          let from_name = metadata
            .as_ref()
            .map(|m| m.display_name.clone())
            .unwrap_or_else(|| envelope.from.clone());

          // Broadcast message to all users (including sender for echo)
          let event = ChatEvent::Message {
            from: envelope.from.clone(),
            from_name,
            content,
            timestamp: std::time::SystemTime::now()
              .duration_since(std::time::UNIX_EPOCH)
              .unwrap()
              .as_secs(),
          };
          let _ = ctx.broadcast(event).await;
        }
        ChatMessage::Typing { is_typing } => {
          // Only send typing indicators to others
          let event = ChatEvent::UserTyping {
            user_id: envelope.from.clone(),
            is_typing,
          };
          let _ = ctx.broadcast_to_others(event).await;
        }
        ChatMessage::Reaction { message_id, emoji } => {
          let event = ChatEvent::Reaction {
            message_id,
            user_id: envelope.from.clone(),
            emoji,
          };
          let _ = ctx.broadcast(event).await;
        }
        ChatMessage::SetDisplayName { name } => {
          // Update user metadata
          let mut metadata = ctx
            .get_user_metadata(&envelope.from)
            .await
            .unwrap_or_else(|| PlayerMetadata::new(envelope.from.clone(), name.clone()));
          metadata.display_name = name.clone();
          let _ = ctx.set_user_metadata(metadata).await;

          // Notify others
          let event = ChatEvent::UserJoined {
            user_id: envelope.from.clone(),
            display_name: name,
          };
          let _ = ctx.broadcast_to_others(event).await;
        }
        ChatMessage::SetAvatar { url } => {
          // Update user metadata
          let mut metadata = ctx
            .get_user_metadata(&envelope.from)
            .await
            .unwrap_or_else(|| PlayerMetadata::new(envelope.from.clone(), envelope.from.clone()));
          metadata.avatar_url = Some(url);
          let _ = ctx.set_user_metadata(metadata).await;
        }
      }
    })
    // Validate messages
    .on_message_validate(|_ctx, envelope| {
      // Prevent empty messages
      if envelope.from.is_empty() {
        return Err("Invalid sender".to_string());
      }

      // Could add profanity filter here, link validation, etc.
      Ok(())
    })
    .listen("0.0.0.0:8081")
    .await?;

  Ok(())
}

/*
 * Example client usage (pseudocode):
 *
 * // Create token (typically done by your auth server)
 * let auth = JwtAuth::new("chat-secret-key");
 * let token = auth.sign_access(
 *   "user123".to_string(),
 *   "general-chat".to_string(),
 *   "session-abc".to_string(),
 *   3600
 * )?;
 *
 * // Connect to WebSocket
 * let url = format!("ws://localhost:8081/ws?token={}", token);
 * let mut ws = connect(url).await?;
 *
 * // Set display name
 * let msg = ChatMessage::SetDisplayName {
 *   name: "Alice".to_string(),
 * };
 * ws.send(serde_json::to_string(&Envelope {
 *   from: "user123".to_string(),
 *   data: msg,
 * })?).await?;
 *
 * // Send a message
 * let msg = ChatMessage::Text {
 *   content: "Hello everyone!".to_string(),
 * };
 * ws.send(serde_json::to_string(&Envelope {
 *   from: "user123".to_string(),
 *   data: msg,
 * })?).await?;
 *
 * // Receive events
 * while let Some(msg) = ws.next().await {
 *   let event: ChatEvent = serde_json::from_str(&msg)?;
 *   match event {
 *     ChatEvent::Message { from_name, content, .. } => {
 *       println!("{}: {}", from_name, content);
 *     }
 *     ChatEvent::UserJoined { display_name, .. } => {
 *       println!("{} joined the room", display_name);
 *     }
 *     ChatEvent::RoomInfo { members, history } => {
 *       println!("Room has {} members", members.len());
 *       println!("Recent messages:");
 *       for msg in history {
 *         println!("  {}", msg);
 *       }
 *     }
 *     _ => {}
 *   }
 * }
 */
