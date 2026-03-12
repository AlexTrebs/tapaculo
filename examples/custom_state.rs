//! Example: per-room custom state using a simple dice game.

use serde::{Deserialize, Serialize};
use tapaculo::{Context, Envelope, RoomEventHandler, Server};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
enum GameMessage {
  PlaceBet { amount: u32 },
  RollDice,
  Chat { message: String },
}

/// Custom state for a dice game room
#[derive(Debug, Clone)]
struct DiceGameState {
  pot: u32,
  current_turn: Option<String>,
  rolls: Vec<u32>,
}

impl DiceGameState {
  fn new() -> Self {
    Self {
      pot: 0,
      current_turn: None,
      rolls: Vec::new(),
    }
  }
}

struct DiceGameHandler;

#[async_trait::async_trait]
impl RoomEventHandler for DiceGameHandler {
  async fn on_player_joined(&self, ctx: &Context, user_id: &str) {
    if ctx.get_room_members().await.len() == 1 {
      ctx.set_custom_state(DiceGameState::new()).await.ok();
    }
    let _ = ctx.broadcast(GameMessage::Chat {
      message: format!("{} joined the game!", user_id),
    }).await;
  }

  async fn on_player_left(&self, ctx: &Context, _user_id: &str) {
    if ctx.get_room_members().await.is_empty() {
      let _ = ctx.clear_custom_state().await;
    }
  }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  tracing_subscriber::fmt::init();

  let server = Server::new()
    .with_event_handler(DiceGameHandler)
    .on_message_typed::<GameMessage, _, _>(|ctx: Context, envelope: Envelope<GameMessage>| async move {
      match &envelope.data {
        GameMessage::PlaceBet { amount } => {
          ctx.update_custom_state::<DiceGameState, _>(|state| {
            state.pot += amount;
            state.current_turn = Some(envelope.from.clone());
          }).await.ok();

          ctx.broadcast(GameMessage::Chat {
            message: format!("{} bet {}! Pot is now {}", envelope.from, amount,
              ctx.get_custom_state::<DiceGameState>().await.map(|s| s.pot).unwrap_or(0)),
          }).await.ok();
        }
        GameMessage::RollDice => {
          if let Some(state) = ctx.get_custom_state::<DiceGameState>().await {
            if state.current_turn.as_ref() == Some(&envelope.from) {
              use std::time::SystemTime;
              let roll = (SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos() % 6 + 1) as u32;

              ctx.update_custom_state::<DiceGameState, _>(|s| s.rolls.push(roll)).await.ok();
              ctx.broadcast(GameMessage::Chat {
                message: format!("{} rolled a {}!", envelope.from, roll),
              }).await.ok();
            }
          }
        }
        GameMessage::Chat { .. } => {
          ctx.broadcast_to_others(envelope.data.clone()).await.ok();
        }
      }
    });

  server.listen("127.0.0.1:3002").await?;

  Ok(())
}
