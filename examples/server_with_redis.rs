use serde::{Deserialize, Serialize};
use tapaculo::{Context, Envelope, RedisPubSub, RoomEventHandler, Server};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
enum GameMessage {
  Move { x: i32, y: i32 },
  Attack { target: String },
  Chat { message: String },
}

/// Game event handler for player join/leave notifications
struct GameEventHandler;

#[async_trait::async_trait]
impl RoomEventHandler for GameEventHandler {
  async fn on_player_joined(&self, ctx: &Context, user_id: &str) {
    let _ = ctx
      .broadcast(GameMessage::Chat {
        message: format!("{} joined the game!", user_id),
      })
      .await;
  }

  async fn on_player_left(&self, ctx: &Context, user_id: &str) {
    let _ = ctx
      .broadcast(GameMessage::Chat {
        message: format!("{} left the game", user_id),
      })
      .await;
  }

  async fn on_room_empty(&self, room_id: &str) {
    tracing::info!("room {} empty", room_id);
  }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  tracing_subscriber::fmt::init();

  let redis_pubsub = RedisPubSub::new("redis://127.0.0.1:6379")?;

  let server = Server::new()
    .with_pubsub(redis_pubsub)
    .with_event_handler(GameEventHandler)
    .on_message_typed::<GameMessage, _, _>(
      |ctx: Context, envelope: Envelope<GameMessage>| async move {
        let player_id = &envelope.from;

        match &envelope.data {
          GameMessage::Move { x, y } => {
            ctx.broadcast(GameMessage::Move { x: *x, y: *y }).await.ok();
          }
          GameMessage::Attack { target } => {
            ctx.broadcast_to_others(GameMessage::Attack { target: target.clone() }).await.ok();
          }
          GameMessage::Chat { message } => {
            ctx.broadcast(GameMessage::Chat {
              message: format!("{}: {}", player_id, message),
            }).await.ok();
          }
        }
      },
    );

  server.listen("127.0.0.1:3001").await?;

  Ok(())
}
