# Tapaculo

A lightweight Rust server that handles real-time and turn-based communication for any multiplayer client interactions.

<h1 align="center">
  <img width="300px" src="https://raw.githubusercontent.com/AlexTrebs/tapaculo/refs/heads/main/docs/images/icon.png" />
</h1>

## Features

- ** Pluggable PubSub**: Swap between in-memory and Redis backends without code changes
- ** JWT Authentication**: Secure token-based auth with session tracking and reconnection support
- ** Room Management**: Capacity limits, player tracking, and lifecycle events
- ** Real-time WebSocket**: Bidirectional communication with automatic cleanup
- ** Rate Limiting**: Built-in spam prevention and abuse protection
- ** Message History**: Optional replay for new joiners
- ** User Metadata**: Associate custom data with players
- ** Typed Messages**: Type-safe message handling with serde
- ** Production Ready**: Comprehensive error handling, logging, and reconnection logic

## Quick Start

```toml
[dependencies]
tapaculo = "0.2"
```

### Basic Server

```rust
use tapaculo::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let auth = JwtAuth::new("your-secret-key");
    let pubsub = InMemoryPubSub::new();

    Server::new()
        .with_auth(auth)
        .with_pubsub(pubsub)
        .on_message(|ctx, envelope| async move {
            // Echo messages to all other players
            ctx.broadcast_to_others(envelope.data).await.ok();
        })
        .listen("0.0.0.0:8080")
        .await
}
```

## Use Cases

### Chess Server (2 players, turn-based)

```rust
use tapaculo::*;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
enum ChessMove {
    Move { from: String, to: String },
    Resign,
    OfferDraw,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let room_settings = RoomSettings {
        max_players: Some(2),  // Exactly 2 for chess
        allow_spectators: false,
        store_message_history: true,
        ..Default::default()
    };

    Server::new()
        .with_auth(JwtAuth::new("secret"))
        .with_pubsub(InMemoryPubSub::new())
        .with_room_settings(room_settings)
        .on_message_typed::<ChessMove, _, _>(|ctx, envelope| async move {
            // Validate and broadcast move to opponent only
            ctx.broadcast_to_others(envelope.data).await.ok();
        })
        .with_event_handler(ChessEventHandler)
        .listen("0.0.0.0:8080")
        .await
}

struct ChessEventHandler;

#[async_trait::async_trait]
impl RoomEventHandler for ChessEventHandler {
    async fn on_room_full(&self, ctx: &Context) {
        // Both players connected - start game
        let members = ctx.get_room_members().await;
        ctx.broadcast(GameStart {
            white: members[0].clone(),
            black: members[1].clone(),
        }).await.ok();
    }
}
```

### Chat Server (group messaging)

```rust
Server::new()
    .with_auth(JwtAuth::new("secret"))
    .with_pubsub(InMemoryPubSub::new())
    .with_room_settings(RoomSettings {
        max_players: Some(50),
        store_message_history: true,
        max_history_messages: 200,
        ..Default::default()
    })
    .with_limits(MessageLimits {
        max_messages_per_window: 10,
        window_duration: Duration::from_secs(1),
        ..Default::default()
    })
    .on_message(|ctx, envelope| async move {
        // Broadcast to everyone (including sender)
        ctx.broadcast(envelope.data).await.ok();
    })
    .listen("0.0.0.0:8081")
    .await
```

## Core Concepts

### 1. Room Management

Rooms are isolated multiplayer sessions with configurable limits:

```rust
let room_settings = RoomSettings {
    max_players: Some(4),              // Limit to 4 players
    allow_spectators: true,            // Allow additional viewers
    store_message_history: true,       // Enable history replay
    max_history_messages: 100,         // Keep last 100 messages
    empty_room_timeout: Some(Duration::from_secs(300)), // Auto-cleanup
};

Server::new()
    .with_room_settings(room_settings)
    // ...
```

### 2. Broadcast Filtering

Send messages to specific players:

```rust
// To all players in room
ctx.broadcast(data).await?;

// To all OTHER players (exclude sender)
ctx.broadcast_to_others(data).await?;

// Custom filter
ctx.broadcast_filtered(data, |user_id| {
    user_id != "spectator123" // Exclude specific user
}).await?;

// Direct message to one player
ctx.send_to("player456", data).await?;
```

### 3. Room Lifecycle Events

React to room state changes:

```rust
struct MyEventHandler;

#[async_trait::async_trait]
impl RoomEventHandler for MyEventHandler {
    async fn on_player_joined(&self, ctx: &Context, user_id: &str) {
        // Send welcome message, current game state, etc.
    }

    async fn on_player_left(&self, ctx: &Context, user_id: &str) {
        // Handle disconnects, pause game, etc.
    }

    async fn on_room_full(&self, ctx: &Context) {
        // Start game when all players connected
    }

    async fn on_room_empty(&self, room_id: &str) {
        // Cleanup, save game state, etc.
    }
}

Server::new()
    .with_event_handler(MyEventHandler)
    // ...
```

### 4. Room State Queries

Access room information:

```rust
// Get all members in room
let members = ctx.get_room_members().await;

// Check if specific user is in room
if ctx.has_member("user123").await {
    // ...
}

// Get room info
if let Some(info) = ctx.get_room_info().await {
    println!("Room has {} / {:?} players",
        info.member_count, info.max_members);
    println!("Is full: {}", info.is_full);
}

// Get message history
let history = ctx.get_message_history(50).await;
```

### 5. Rate Limiting

Prevent spam and abuse:

```rust
let limits = MessageLimits {
    max_size_bytes: 10 * 1024,      // 10KB per message
    max_messages_per_window: 10,    // 10 messages max
    window_duration: Duration::from_secs(1), // per second
    ban_duration: Duration::from_secs(60),   // 1 minute ban
};

Server::new()
    .with_limits(limits)
    // ...
```

### 6. Message Validation

Validate messages before processing:

```rust
Server::new()
    .on_message_validate(|ctx, envelope| {
        // Example: validate chess moves
        if !is_valid_move(&envelope.data) {
            return Err("Invalid move".to_string());
        }
        Ok(())
    })
    // ...
```

### 7. Typed Message Handlers

Type-safe message processing:

```rust
#[derive(Serialize, Deserialize)]
enum GameAction {
    Move { x: i32, y: i32 },
    Attack { target: String },
    UseItem { item_id: String },
}

Server::new()
    .on_message_typed::<GameAction, _, _>(|ctx, envelope| async move {
        match envelope.data {
            GameAction::Move { x, y } => {
                // Handle move
            }
            GameAction::Attack { target } => {
                // Handle attack
            }
            _ => {}
        }
    })
    // ...
```

### 8. User Metadata

Store custom player data:

```rust
// Set metadata
let metadata = PlayerMetadata {
    user_id: "player1".to_string(),
    display_name: "Alice".to_string(),
    avatar_url: Some("https://...".to_string()),
    custom: json!({ "level": 10, "team": "red" }),
    ..Default::default()
};
ctx.set_user_metadata(metadata).await?;

// Get metadata
if let Some(metadata) = ctx.get_user_metadata("player1").await {
    println!("Display name: {}", metadata.display_name);
}
```

### 9. Reconnection Support

Sessions persist across connections:

```rust
// Generate token with session ID
let auth = JwtAuth::new("secret");
let token = auth.sign_access(
    "user123".to_string(),
    "room456".to_string(),
    "session-abc".to_string(),  // Session ID
    3600
)?;

// Later, reconnect with same session ID
let new_token = auth.sign_access(
    "user123".to_string(),
    "room456".to_string(),
    "session-abc".to_string(),  // Same session
    3600
)?;

// Access session ID in handler
ctx.session_id(); // "session-abc"
```

## PubSub Backends

### In-Memory (Development)

```rust
let pubsub = InMemoryPubSub::new();
// or with custom buffer size
let pubsub = InMemoryPubSub::with_buffer(128);
```

### Redis (Production)

```toml
[dependencies]
tapaculo = { version = "0.2", features = ["redis-backend"] }
```

```rust
let pubsub = RedisPubSub::new("redis://localhost:6379")?;

// With custom retry configuration
let config = BackoffConfig {
    max_retries: 10,
    base_delay_ms: 100,
    max_delay_ms: 5000,
    ..Default::default()
};
let pubsub = RedisPubSub::with_config("redis://localhost", config)?;
```

## Authentication

### Creating Tokens

```rust
let auth = JwtAuth::new("your-secret-key");

// Access token
let access_token = auth.sign_access(
    "user_id".to_string(),
    "room_id".to_string(),
    "session_id".to_string(),
    3600  // 1 hour
)?;

// Refresh token
let refresh_token = auth.sign_refresh(
    "user_id".to_string(),
    86400  // 24 hours
)?;

// Refresh access token
let new_access = auth.refresh_access(
    &refresh_token,
    "new_room".to_string(),
    "new_session".to_string(),
    3600
)?;
```

### Client Connection

```javascript
const token = "eyJ0eXAiOiJKV1QiLCJhbGc...";
const ws = new WebSocket(`ws://localhost:8080/ws?token=${token}`);

ws.onmessage = (event) => {
  const envelope = JSON.parse(event.data);
  console.log(`Message from ${envelope.from}:`, envelope.data);
};

// Send message
ws.send(JSON.stringify({
  from: "user123",
  data: { type: "Move", from: "e2", to: "e4" }
}));
```

## Examples

See the `examples/` directory for complete implementations:

- **`chess_server.rs`**: 2-player chess with move validation
- **`chat_server.rs`**: Group chat with history and typing indicators

Run examples:
```bash
cargo run --example chess_server
cargo run --example chat_server
```

## Architecture

```
┌──────────────────────────────────────────────┐
│           WebSocket Connections              │
│  (JWT Auth + Session Tracking)               │
└──────────┬───────────────────────────────────┘
           │
┌──────────▼──────────────────────────────────┐
│         Server (Room Management)            │
│  • Rate Limiting                            │
│  • Message Validation                       │
│  • Event Handlers                           │
│  • User Metadata                            │
└──────────┬──────────────────────────────────┘
           │
┌──────────▼──────────────────────────────────┐
│         PubSub Backend                      │
│  ┌────────────────┬──────────────────────┐  │
│  │  InMemoryPubSub│    RedisPubSub      │  │
│  │  (Single Node) │  (Distributed)      │  │
│  └────────────────┴──────────────────────┘  │
└─────────────────────────────────────────────┘
```

## Production Checklist

- [x] Use Redis backend for multi-server deployments
- [x] Enable rate limiting to prevent spam
- [x] Set appropriate room size limits
- [x] Configure empty room timeouts
- [x] Use strong JWT secrets (env variables)
- [x] Add message validation for your use case
- [x] Implement custom event handlers for game logic
- [x] Enable message history if needed
- [x] Set up structured logging (tracing)
- [x] Monitor rate limit bans
- [x] Handle reconnections gracefully

## License

MIT

## Contributing

Contributions welcome! Please open an issue or PR.
