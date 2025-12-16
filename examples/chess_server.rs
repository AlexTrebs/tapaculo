//! Chess server example demonstrating room capacity limits, typed messages, and game state management.

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
    tracing::info!("Player {} joined chess room {}", user_id, ctx.room_id());

    // Send join event to all other players
    let event = GameEvent::PlayerJoined {
      user_id: user_id.to_string(),
    };
    let _ = ctx.broadcast_to_others(event).await;

    // Check room state
    if let Some(info) = ctx.get_room_info().await {
      if info.member_count == 1 {
        // First player - waiting for opponent
        let _ = ctx.send_to(user_id, GameEvent::WaitingForOpponent).await;
      }
    }
  }

  async fn on_room_full(&self, ctx: &Context) {
    // Room is full (2 players) - start the game
    let members = ctx.get_room_members().await;
    if members.len() == 2 {
      let white = members[0].clone();
      let black = members[1].clone();

      tracing::info!("Starting chess game in room {}: {} vs {}", ctx.room_id(), white, black);

      // Broadcast game start to both players
      let event = GameEvent::GameStart {
        white: white.clone(),
        black: black.clone(),
      };
      let _ = ctx.broadcast(event).await;
    }
  }

  async fn on_player_left(&self, ctx: &Context, user_id: &str) {
    tracing::info!("Player {} left chess room {}", user_id, ctx.room_id());

    // If a player leaves during a game, the other player wins
    let event = GameEvent::PlayerLeft {
      user_id: user_id.to_string(),
    };
    let _ = ctx.broadcast_to_others(event).await;

    // End the game
    let members = ctx.get_room_members().await;
    if !members.is_empty() {
      let winner = members.first().cloned();
      let game_over = GameEvent::GameOver {
        winner,
        reason: "Opponent disconnected".to_string(),
      };
      let _ = ctx.broadcast(game_over).await;
    }
  }

  async fn on_room_empty(&self, room_id: &str) {
    tracing::info!("Chess room {} is now empty and will be cleaned up", room_id);
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
      tracing::debug!("Chess message from {}: {:?}", envelope.from, envelope.data);

      match envelope.data {
        ChessMessage::Move { from, to, piece } => {
          // In a real implementation, you'd validate the move here
          // For now, just broadcast it to the opponent
          let event = GameEvent::MoveMade {
            from: from.clone(),
            to: to.clone(),
            piece: piece.clone(),
            by: envelope.from.clone(),
          };
          let _ = ctx.broadcast_to_others(event).await;
        }
        ChessMessage::Resign => {
          // Player resigned - opponent wins
          let members = ctx.get_room_members().await;
          let winner = members
            .iter()
            .find(|id| *id != &envelope.from)
            .cloned();
          let event = GameEvent::GameOver {
            winner,
            reason: format!("{} resigned", envelope.from),
          };
          let _ = ctx.broadcast(event).await;
        }
        ChessMessage::OfferDraw => {
          let event = GameEvent::DrawOffered {
            by: envelope.from.clone(),
          };
          let _ = ctx.broadcast_to_others(event).await;
        }
        ChessMessage::AcceptDraw => {
          let event = GameEvent::GameOver {
            winner: None,
            reason: "Draw by agreement".to_string(),
          };
          let _ = ctx.broadcast(event).await;
        }
        ChessMessage::Chat { .. } => {
          // Just relay chat messages to opponent
          let _ = ctx.broadcast_to_others(envelope.data).await;
        }
        ChessMessage::RequestUndo => {
          // Forward undo request to opponent
          let _ = ctx.broadcast_to_others(envelope.data).await;
        }
      }
    })
    // Validate messages
    .on_message_validate(|_ctx, envelope| {
      // Example: could validate that messages are from valid players
      if envelope.from.is_empty() {
        return Err("Invalid sender".to_string());
      }
      Ok(())
    })
    .listen("0.0.0.0:8080")
    .await?;

  Ok(())
}

/*
 * Example client usage (pseudocode):
 *
 * // Create token (typically done by your auth server)
 * let auth = JwtAuth::new("chess-secret-key");
 * let token = auth.sign_access(
 *   "player1".to_string(),
 *   "game-room-123".to_string(),
 *   "session-xyz".to_string(),
 *   3600
 * )?;
 *
 * // Connect to WebSocket
 * let url = format!("ws://localhost:8080/ws?token={}", token);
 * let mut ws = connect(url).await?;
 *
 * // Send a move
 * let msg = ChessMessage::Move {
 *   from: "e2".to_string(),
 *   to: "e4".to_string(),
 *   piece: "pawn".to_string(),
 * };
 * ws.send(serde_json::to_string(&Envelope {
 *   from: "player1".to_string(),
 *   data: msg,
 * })?).await?;
 *
 * // Receive events
 * while let Some(msg) = ws.next().await {
 *   let event: GameEvent = serde_json::from_str(&msg)?;
 *   match event {
 *     GameEvent::GameStart { white, black } => { /* ... */ }
 *     GameEvent::MoveMade { from, to, .. } => { /* ... */ }
 *     _ => {}
 *   }
 * }
 */
