//! Tapaculo - Lightweight Rust server for real-time and turn-based multiplayer communication.
//!
//! # Overview
//!
//! Tapaculo is a batteries-included framework for building multiplayer game servers and
//! real-time applications. It handles WebSocket connections, authentication, room management,
//! pub/sub messaging, rate limiting, and more - letting you focus on your game logic.
//!
//! # Features
//!
//! - **Pluggable PubSub**: Swap between in-memory and Redis backends without code changes
//! - **JWT Authentication**: Secure token-based auth with session tracking and reconnection support
//! - **Room Management**: Capacity limits, player tracking, and lifecycle events
//! - **WebSocket Server**: Bidirectional real-time communication with automatic cleanup
//! - **Rate Limiting**: Built-in spam prevention and abuse protection
//! - **Message History**: Optional replay for new joiners
//! - **User Metadata**: Associate custom data with players
//! - **Typed Messages**: Type-safe message handling with serde
//! - **Production Ready**: Comprehensive error handling, logging, and reconnection logic
//!
//! # Quick Start
//!
//! ```rust,no_run
//! use tapaculo::*;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let auth = JwtAuth::new("your-secret-key");
//!     let pubsub = InMemoryPubSub::new();
//!
//!     Server::new()
//!         .with_auth(auth)
//!         .with_pubsub(pubsub)
//!         .on_message(|ctx, envelope| async move {
//!             // Echo messages to all other players
//!             ctx.broadcast_to_others(envelope.data).await.ok();
//!         })
//!         .listen("0.0.0.0:8080")
//!         .await
//! }
//! ```
//!
//! # Examples
//!
//! ## Chess Server (2 players, turn-based)
//!
//! ```rust,no_run
//! use tapaculo::*;
//! use serde::{Serialize, Deserialize};
//!
//! #[derive(Serialize, Deserialize)]
//! enum ChessMove {
//!     Move { from: String, to: String },
//!     Resign,
//! }
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let room_settings = RoomSettings {
//!         max_players: Some(2),
//!         allow_spectators: false,
//!         store_message_history: true,
//!         ..Default::default()
//!     };
//!
//!     Server::new()
//!         .with_auth(JwtAuth::new("secret"))
//!         .with_pubsub(InMemoryPubSub::new())
//!         .with_room_settings(room_settings)
//!         .on_message_typed::<ChessMove, _, _>(|ctx, envelope| async move {
//!             ctx.broadcast_to_others(envelope.data).await.ok();
//!         })
//!         .listen("0.0.0.0:8080")
//!         .await
//! }
//! ```
//!
//! ## Chat Server (group messaging)
//!
//! ```rust,no_run
//! use tapaculo::*;
//! use std::time::Duration;
//!
//! # #[tokio::main]
//! # async fn main() -> anyhow::Result<()> {
//! Server::new()
//!     .with_auth(JwtAuth::new("secret"))
//!     .with_pubsub(InMemoryPubSub::new())
//!     .with_room_settings(RoomSettings {
//!         max_players: Some(50),
//!         store_message_history: true,
//!         max_history_messages: 200,
//!         ..Default::default()
//!     })
//!     .with_limits(MessageLimits {
//!         max_messages_per_window: 10,
//!         window_duration: Duration::from_secs(1),
//!         ..Default::default()
//!     })
//!     .on_message(|ctx, envelope| async move {
//!         ctx.broadcast(envelope.data).await.ok();
//!     })
//!     .listen("0.0.0.0:8081")
//!     .await
//! # }
//! ```
//!
//! # Core Concepts
//!
//! ## Rooms
//!
//! Rooms are isolated multiplayer sessions. Each WebSocket connection joins a specific room
//! identified by the JWT token's `room_id` claim.
//!
//! ```rust
//! use tapaculo::*;
//! use std::time::Duration;
//!
//! let room_settings = RoomSettings {
//!     max_players: Some(4),
//!     allow_spectators: true,
//!     store_message_history: true,
//!     max_history_messages: 100,
//!     empty_room_timeout: Some(Duration::from_secs(300)),
//! };
//! ```
//!
//! ## Broadcasting
//!
//! Send messages to specific subsets of players:
//!
//! ```rust,no_run
//! # use tapaculo::*;
//! # async fn example(ctx: &Context, data: serde_json::Value) {
//! // To all players in room
//! ctx.broadcast(data.clone()).await.ok();
//!
//! // To all OTHER players (exclude sender)
//! ctx.broadcast_to_others(data.clone()).await.ok();
//!
//! // Direct message to one player
//! ctx.send_to("player456", data).await.ok();
//! # }
//! ```
//!
//! ## Authentication
//!
//! Generate JWT tokens for client connections:
//!
//! ```rust
//! use tapaculo::JwtAuth;
//!
//! let auth = JwtAuth::new("your-secret-key");
//!
//! let access_token = auth.sign_access(
//!     "user_id".to_string(),
//!     "room_id".to_string(),
//!     "session_id".to_string(),
//!     3600  // 1 hour
//! )?;
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! Clients connect via WebSocket with the token as a query parameter:
//! ```text
//! ws://localhost:8080/ws?token=eyJ0eXAiOiJKV1QiLCJhbGc...
//! ```
//!
//! ## PubSub Backends
//!
//! ### In-Memory (Development)
//!
//! ```rust
//! use tapaculo::InMemoryPubSub;
//!
//! let pubsub = InMemoryPubSub::new();
//! // or with custom buffer size
//! let pubsub = InMemoryPubSub::with_buffer(128);
//! ```
//!
//! ### Redis (Production)
//!
//! Requires the `redis-backend` feature:
//!
//! ```toml
//! [dependencies]
//! tapaculo = { version = "0.2", features = ["redis-backend"] }
//! ```
//!
//! ```rust,ignore
//! use tapaculo::RedisPubSub;
//!
//! let pubsub = RedisPubSub::new("redis://localhost:6379")?;
//! ```

pub mod auth;
pub mod error;
pub mod pubsub;
pub mod rate_limit;
pub mod room;
pub mod server;

pub use auth::{AccessClaims, JwtAuth, JwtAuthOptions, RefreshClaims};
pub use error::PubSubError;
pub use pubsub::{InMemoryPubSub, PubSubBackend, PubSubExt, Subscription};
pub use rate_limit::{MessageLimits, RateLimiter};
pub use room::{PlayerMetadata, RoomInfo, RoomManager, RoomSettings, StoredMessage};
pub use server::{Context, Envelope, RoomEventHandler, Server};

#[cfg(feature = "redis-backend")]
pub use pubsub::{BackoffConfig, RedisPubSub};
