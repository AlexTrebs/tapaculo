//! Example: two-player turn-based game with room capacity enforcement.

use serde::{Deserialize, Serialize};
use tapaculo::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ChessMessage {
  Move { from: String, to: String, piece: String },
  Resign,
  OfferDraw,
  AcceptDraw,
  RequestUndo,
  Chat { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum GameEvent {
  GameStart { white: String, black: String },
  MoveMade { from: String, to: String, piece: String, by: String },
  GameOver { winner: Option<String>, reason: String },
  DrawOffered { by: String },
  PlayerJoined { user_id: String },
  PlayerLeft { user_id: String },
  WaitingForOpponent,
}

/// Chess-specific event handler
struct ChessEventHandler;

#[async_trait::async_trait]
impl RoomEventHandler for ChessEventHandler {
  async fn on_player_joined(&self, ctx: &Context, user_id: &str) {
    let _ = ctx.broadcast_to_others(GameEvent::PlayerJoined { user_id: user_id.to_string() }).await;

    if let Some(info) = ctx.get_room_info().await {
      if info.member_count == 1 {
        let _ = ctx.send_to(user_id, GameEvent::WaitingForOpponent).await;
      }
    }
  }

  async fn on_room_full(&self, ctx: &Context) {
    let members = ctx.get_room_members().await;
    if members.len() == 2 {
      tracing::info!("game starting in room {}: {} vs {}", ctx.room_id(), members[0], members[1]);
      let _ = ctx.broadcast(GameEvent::GameStart {
        white: members[0].clone(),
        black: members[1].clone(),
      }).await;
    }
  }

  async fn on_player_left(&self, ctx: &Context, user_id: &str) {
    let _ = ctx.broadcast_to_others(GameEvent::PlayerLeft { user_id: user_id.to_string() }).await;

    let members = ctx.get_room_members().await;
    if !members.is_empty() {
      let _ = ctx.broadcast(GameEvent::GameOver {
        winner: members.first().cloned(),
        reason: "Opponent disconnected".to_string(),
      }).await;
    }
  }

  async fn on_room_empty(&self, room_id: &str) {
    tracing::debug!("chess room {} empty", room_id);
  }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  // Initialize tracing
  tracing_subscriber::fmt()
    .with_env_filter("chess_server=debug,tapaculo=info")
    .init();

  let auth = JwtAuth::new("chess-secret-key");
  let pubsub = InMemoryPubSub::new();

  // Configure for chess (exactly 2 players)
  let room_settings = RoomSettings {
    max_players: Some(2), // Chess needs exactly 2 players
    allow_spectators: false,
    store_message_history: true,
    max_history_messages: 100,
    empty_room_timeout: Some(std::time::Duration::from_secs(300)),
  };

  // Configure rate limiting
  let limits = MessageLimits {
    max_size_bytes: 1024, // 1KB max per message
    max_messages_per_window: 20,
    window_duration: std::time::Duration::from_secs(1),
    ban_duration: std::time::Duration::from_secs(60),
  };

  Server::new()
    .with_auth(auth)
    .with_pubsub(pubsub)
    .with_room_settings(room_settings)
    .with_limits(limits)
    .with_event_handler(ChessEventHandler)
    .on_message_typed::<ChessMessage, _, _>(|ctx, envelope| async move {
      match envelope.data {
        ChessMessage::Move { from, to, piece } => {
          let _ = ctx.broadcast_to_others(GameEvent::MoveMade {
            from,
            to,
            piece,
            by: envelope.from.clone(),
          }).await;
        }
        ChessMessage::Resign => {
          let members = ctx.get_room_members().await;
          let winner = members.iter().find(|id| *id != &envelope.from).cloned();
          let _ = ctx.broadcast(GameEvent::GameOver {
            winner,
            reason: format!("{} resigned", envelope.from),
          }).await;
        }
        ChessMessage::OfferDraw => {
          let _ = ctx.broadcast_to_others(GameEvent::DrawOffered { by: envelope.from.clone() }).await;
        }
        ChessMessage::AcceptDraw => {
          let _ = ctx.broadcast(GameEvent::GameOver {
            winner: None,
            reason: "Draw by agreement".to_string(),
          }).await;
        }
        ChessMessage::Chat { .. } | ChessMessage::RequestUndo => {
          let _ = ctx.broadcast_to_others(envelope.data).await;
        }
      }
    })
    .on_message_validate(|_ctx, envelope| {
      if envelope.from.is_empty() {
        return Err("Invalid sender".to_string());
      }
      Ok(())
    })
    .listen("0.0.0.0:8080")
    .await?;

  Ok(())
}

