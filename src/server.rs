//! WebSocket server for real-time multiplayer communication with room management.

use crate::{
  rate_limit::{MessageLimits, RateLimiter},
  room::{PlayerMetadata, RoomInfo, RoomManager, RoomSettings, StoredMessage},
  JwtAuth, PubSubBackend, PubSubExt,
};
use axum::{
  extract::{
    ws::{Message, WebSocket},
    Query, WebSocketUpgrade,
  },
  response::IntoResponse,
  routing::get,
  Router,
};
use futures::{SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, RwLock};

/// Message envelope for WebSocket communication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
  pub from: String,
  pub data: T,
}

pub type ClientMap = Arc<RwLock<HashMap<String, mpsc::UnboundedSender<Message>>>>;

/// Context provided to message handlers, containing room state and communication channels.
#[derive(Clone)]
pub struct Context {
  room_id: String,
  user_id: String,
  session_id: String,
  pubsub: Arc<dyn PubSubBackend>,
  clients: ClientMap,
  room_manager: RoomManager,
}

impl Context {
  /// Broadcast a message to all clients in the room via PubSub.
  pub async fn broadcast<T: Serialize + Send + Sync>(&self, data: T) -> Result<(), String> {
    let envelope = Envelope {
      from: self.user_id.clone(),
      data,
    };
    self
      .pubsub
      .publish(&self.room_id, &envelope)
      .await
      .map_err(|e| format!("Failed to broadcast: {}", e))
  }

  /// Broadcast a message to all OTHER clients (excluding sender) in the room.
  pub async fn broadcast_to_others<T: Serialize + Send + Sync>(
    &self,
    data: T,
  ) -> Result<(), String> {
    self
      .broadcast_filtered(data, |user_id| user_id != self.user_id)
      .await
  }

  /// Broadcast a message with a custom filter function.
  ///
  /// The filter receives each user_id and should return true to send to that user.
  pub async fn broadcast_filtered<T, F>(
    &self,
    data: T,
    filter: F,
  ) -> Result<(), String>
  where
    T: Serialize + Send + Sync,
    F: Fn(&str) -> bool,
  {
    let envelope = Envelope {
      from: self.user_id.clone(),
      data,
    };
    let json =
      serde_json::to_string(&envelope).map_err(|e| format!("Serialization error: {}", e))?;

    let clients = self.clients.read().await;
    for (user_id, sender) in clients.iter() {
      if filter(user_id) {
        let _ = sender.send(Message::Text(json.clone().into()));
      }
    }
    Ok(())
  }

  /// Send a message directly to a specific player via WebSocket.
  pub async fn send_to<T: Serialize>(&self, player_id: &str, msg: T) -> Result<(), String> {
    let json = serde_json::to_string(&msg).map_err(|e| format!("Serialization error: {}", e))?;
    let clients = self.clients.read().await;
    if let Some(sender) = clients.get(player_id) {
      sender
        .send(Message::Text(json.into()))
        .map_err(|e| format!("Failed to send message: {}", e))?;
      Ok(())
    } else {
      Err(format!("Client {} not found", player_id))
    }
  }

  /// Get the current user's ID
  pub fn user_id(&self) -> &str {
    &self.user_id
  }

  /// Get the current room ID
  pub fn room_id(&self) -> &str {
    &self.room_id
  }

  /// Get the current session ID
  pub fn session_id(&self) -> &str {
    &self.session_id
  }

  /// Get all user IDs currently in this room
  pub async fn get_room_members(&self) -> Vec<String> {
    if let Some(room) = self.room_manager.get_room(&self.room_id).await {
      let room = room.read().await;
      room.players.keys().cloned().collect()
    } else {
      Vec::new()
    }
  }

  /// Check if a specific user is in the room
  pub async fn has_member(&self, user_id: &str) -> bool {
    if let Some(room) = self.room_manager.get_room(&self.room_id).await {
      let room = room.read().await;
      room.players.contains_key(user_id)
    } else {
      false
    }
  }

  /// Get room capacity info
  pub async fn get_room_info(&self) -> Option<RoomInfo> {
    if let Some(room) = self.room_manager.get_room(&self.room_id).await {
      let room = room.read().await;
      Some(room.get_info())
    } else {
      None
    }
  }

  /// Get message history for this room (if enabled)
  pub async fn get_message_history(&self, limit: usize) -> Vec<StoredMessage> {
    if let Some(room) = self.room_manager.get_room(&self.room_id).await {
      let room = room.read().await;
      room
        .message_history
        .iter()
        .rev()
        .take(limit)
        .cloned()
        .collect()
    } else {
      Vec::new()
    }
  }

  /// Set metadata for the current user
  pub async fn set_user_metadata(&self, metadata: PlayerMetadata) -> Result<(), String> {
    if let Some(room) = self.room_manager.get_room(&self.room_id).await {
      let mut room = room.write().await;
      room.players.insert(self.user_id.clone(), metadata);
      Ok(())
    } else {
      Err("Room not found".to_string())
    }
  }

  /// Get metadata for a specific user
  pub async fn get_user_metadata(&self, user_id: &str) -> Option<PlayerMetadata> {
    if let Some(room) = self.room_manager.get_room(&self.room_id).await {
      let room = room.read().await;
      room.players.get(user_id).cloned()
    } else {
      None
    }
  }
}

/// Event handler trait for room lifecycle events.
#[async_trait::async_trait]
pub trait RoomEventHandler: Send + Sync {
  /// Called when a player joins a room
  async fn on_player_joined(&self, _ctx: &Context, _user_id: &str) {}

  /// Called when a player leaves a room
  async fn on_player_left(&self, _ctx: &Context, _user_id: &str) {}

  /// Called when a room becomes empty
  async fn on_room_empty(&self, _room_id: &str) {}

  /// Called when a room becomes full
  async fn on_room_full(&self, _ctx: &Context) {}
}

/// Default no-op event handler
struct NoOpEventHandler;
#[async_trait::async_trait]
impl RoomEventHandler for NoOpEventHandler {}

type MessageHandler = Arc<
  dyn Fn(Context, Envelope<serde_json::Value>) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
    + Send
    + Sync,
>;

type MessageValidator = Arc<
  dyn Fn(&Context, &Envelope<serde_json::Value>) -> Result<(), String> + Send + Sync,
>;

/// WebSocket server for multiplayer games.
pub struct Server {
  auth: JwtAuth,
  pubsub: Arc<dyn PubSubBackend>,
  room_manager: RoomManager,
  rate_limiter: Option<RateLimiter>,
  on_message: MessageHandler,
  message_validator: Option<MessageValidator>,
  event_handler: Arc<dyn RoomEventHandler>,
}

impl Server {
  /// Create a new server with default configuration.
  pub fn new() -> Self {
    Self {
      auth: JwtAuth::new("secret"),
      pubsub: Arc::new(crate::InMemoryPubSub::new()),
      room_manager: RoomManager::new(RoomSettings::default()),
      rate_limiter: None,
      on_message: Arc::new(|_, _| Box::pin(async {})),
      message_validator: None,
      event_handler: Arc::new(NoOpEventHandler),
    }
  }

  /// Configure the JWT authentication handler.
  pub fn with_auth(mut self, auth: JwtAuth) -> Self {
    self.auth = auth;
    self
  }

  /// Configure the PubSub backend.
  pub fn with_pubsub(mut self, ps: impl PubSubBackend + 'static) -> Self {
    self.pubsub = Arc::new(ps);
    self
  }

  /// Configure room settings.
  pub fn with_room_settings(mut self, settings: RoomSettings) -> Self {
    self.room_manager = RoomManager::new(settings);
    self
  }

  /// Configure rate limiting.
  pub fn with_limits(mut self, limits: MessageLimits) -> Self {
    self.rate_limiter = Some(RateLimiter::new(limits));
    self
  }

  /// Set the message handler callback.
  pub fn on_message<F, Fut>(mut self, f: F) -> Self
  where
    F: Fn(Context, Envelope<serde_json::Value>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
  {
    self.on_message = Arc::new(move |ctx, msg| Box::pin(f(ctx, msg)));
    self
  }

  /// Set a typed message handler (deserializes to a specific type).
  pub fn on_message_typed<T, F, Fut>(mut self, f: F) -> Self
  where
    T: DeserializeOwned + Send + 'static,
    F: Fn(Context, Envelope<T>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
  {
    let f = Arc::new(f);
    self.on_message = Arc::new(move |ctx, msg| {
      let f = f.clone();
      Box::pin(async move {
        match serde_json::from_value::<T>(msg.data) {
          Ok(typed_data) => {
            let typed_envelope = Envelope {
              from: msg.from,
              data: typed_data,
            };
            f(ctx, typed_envelope).await;
          }
          Err(e) => {
            tracing::warn!("Failed to deserialize message: {}", e);
          }
        }
      })
    });
    self
  }

  /// Set a message validator (called before processing).
  pub fn on_message_validate<F>(mut self, validator: F) -> Self
  where
    F: Fn(&Context, &Envelope<serde_json::Value>) -> Result<(), String> + Send + Sync + 'static,
  {
    self.message_validator = Some(Arc::new(validator));
    self
  }

  /// Set event handler for room lifecycle events.
  pub fn with_event_handler<H: RoomEventHandler + 'static>(mut self, handler: H) -> Self {
    self.event_handler = Arc::new(handler);
    self
  }

  /// Start the WebSocket server.
  pub async fn listen(self, addr: &str) -> anyhow::Result<()> {
    let clients: ClientMap = Arc::new(RwLock::new(HashMap::new()));
    let pubsub = self.pubsub.clone();
    let auth = self.auth.clone();
    let on_message = self.on_message.clone();
    let room_manager = self.room_manager.clone();
    let rate_limiter = self.rate_limiter.clone();
    let message_validator = self.message_validator.clone();
    let event_handler = self.event_handler.clone();

    // Spawn background task to cleanup empty rooms
    let room_manager_cleanup = room_manager.clone();
    tokio::spawn(async move {
      let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
      loop {
        interval.tick().await;
        room_manager_cleanup.cleanup_empty_rooms().await;
      }
    });

    let app = Router::new().route(
      "/ws",
      get({
        let clients = clients.clone();
        move |ws: WebSocketUpgrade, Query(params): Query<HashMap<String, String>>| {
          let clients = clients.clone();
          let pubsub = pubsub.clone();
          let auth = auth.clone();
          let on_message = on_message.clone();
          let room_manager = room_manager.clone();
          let rate_limiter = rate_limiter.clone();
          let message_validator = message_validator.clone();
          let event_handler = event_handler.clone();
          async move {
            if let Some(token) = params.get("token") {
              if let Ok(claims) = auth.verify_access(token) {
                return ws.on_upgrade(move |socket| {
                  handle_ws(
                    socket,
                    claims.sub,
                    claims.room,
                    claims.session_id,
                    clients,
                    pubsub,
                    room_manager,
                    rate_limiter,
                    on_message,
                    message_validator,
                    event_handler,
                  )
                });
              }
            }
            "Unauthorized".into_response()
          }
        }
      }),
    );

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("WebSocket server listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
  }
}

impl Default for Server {
  fn default() -> Self {
    Self::new()
  }
}

/// Handle a WebSocket connection for a specific player and room.
#[allow(clippy::too_many_arguments)]
async fn handle_ws(
  ws: WebSocket,
  user_id: String,
  room_id: String,
  session_id: String,
  clients: ClientMap,
  pubsub: Arc<dyn PubSubBackend>,
  room_manager: RoomManager,
  rate_limiter: Option<RateLimiter>,
  on_message: MessageHandler,
  message_validator: Option<MessageValidator>,
  event_handler: Arc<dyn RoomEventHandler>,
) {
  let (mut sender_ws, mut receiver_ws) = ws.split();
  let (tx, mut rx) = mpsc::unbounded_channel();

  // Get or create room and add player
  let room = room_manager.get_or_create_room(&room_id).await;
  let player_metadata = PlayerMetadata::new(user_id.clone(), user_id.clone());

  let join_result = {
    let mut room_guard = room.write().await;
    room_guard.add_player(player_metadata)
  };

  if let Err(e) = join_result {
    tracing::warn!("Failed to add player {} to room {}: {}", user_id, room_id, e);
    let _ = sender_ws
      .send(Message::Text(
        format!(r#"{{"error":"{}"}}"#, e).into(),
      ))
      .await;
    let _ = sender_ws.close().await;
    return;
  }

  // Register client
  clients.write().await.insert(user_id.clone(), tx.clone());
  tracing::info!("User {} (session {}) connected to room {}", user_id, session_id, room_id);

  // Check if room is now full
  let is_full = {
    let room_guard = room.read().await;
    room_guard.is_full()
  };

  // Subscribe to pubsub topic for this room
  let subscription = match pubsub
    .subscribe(&room_id, {
      let tx = tx.clone();
      move |bytes| {
        let tx = tx.clone();
        async move {
          if let Ok(msg) = String::from_utf8(bytes) {
            let _ = tx.send(Message::Text(msg.into()));
          }
        }
      }
    })
    .await
  {
    Ok(sub) => sub,
    Err(e) => {
      tracing::error!("Failed to subscribe to room {}: {}", room_id, e);
      return;
    }
  };

  let ctx = Context {
    room_id: room_id.clone(),
    user_id: user_id.clone(),
    session_id: session_id.clone(),
    pubsub: pubsub.clone(),
    clients: clients.clone(),
    room_manager: room_manager.clone(),
  };

  // Notify that player joined
  event_handler.on_player_joined(&ctx, &user_id).await;

  // Notify if room is full
  if is_full {
    event_handler.on_room_full(&ctx).await;
  }

  // Spawn task to handle incoming WS messages from client
  let ctx_clone = ctx.clone();
  let user_id_clone = user_id.clone();
  let room_id_clone = room_id.clone();
  let receiver_task = tokio::spawn(async move {
    while let Some(Ok(msg)) = receiver_ws.next().await {
      match msg {
        Message::Text(text) => {
          // Check rate limit
          if let Some(ref limiter) = rate_limiter {
            if let Err(e) = limiter.check_allowed(&user_id_clone, text.len()).await {
              tracing::warn!("Rate limit exceeded for {}: {}", user_id_clone, e);
              continue;
            }
          }

          if let Ok(envelope) = serde_json::from_str::<Envelope<serde_json::Value>>(&text) {
            // Validate message if validator is set
            if let Some(ref validator) = message_validator {
              if let Err(e) = validator(&ctx_clone, &envelope) {
                tracing::warn!("Message validation failed for {}: {}", user_id_clone, e);
                continue;
              }
            }

            // Store in history if enabled
            if let Some(room) = room_manager.get_room(&room_id_clone).await {
              let mut room_guard = room.write().await;
              room_guard.add_message(StoredMessage {
                from: envelope.from.clone(),
                data: envelope.data.clone(),
                timestamp: std::time::Instant::now(),
              });
            }

            // Process message
            (on_message)(ctx_clone.clone(), envelope).await;
          } else {
            tracing::warn!("Failed to parse message from {}: {}", user_id_clone, text);
          }
        }
        Message::Close(_) => {
          tracing::info!("User {} closed connection", user_id_clone);
          break;
        }
        _ => {}
      }
    }
  });

  // Spawn task to pump messages from rx → ws
  let user_id_clone = user_id.clone();
  let sender_task = tokio::spawn(async move {
    while let Some(msg) = rx.recv().await {
      if sender_ws.send(msg).await.is_err() {
        tracing::warn!("Failed to send message to {}", user_id_clone);
        break;
      }
    }
  });

  // Wait for either task to complete (client disconnect or error)
  tokio::select! {
    _ = receiver_task => {},
    _ = sender_task => {},
  }

  // Cleanup: remove client, remove from room, stop subscription
  clients.write().await.remove(&user_id);

  let is_empty = {
    let mut room_guard = room.write().await;
    room_guard.remove_player(&user_id);
    room_guard.is_empty()
  };

  subscription.abort();

  // Notify that player left
  event_handler.on_player_left(&ctx, &user_id).await;

  // Notify if room is now empty
  if is_empty {
    event_handler.on_room_empty(&room_id).await;
  }

  tracing::info!("User {} disconnected from room {}", user_id, room_id);
}
