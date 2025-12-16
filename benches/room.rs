use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use serde_json::json;
use std::time::Duration;
use tapaculo::room::{PlayerMetadata, RoomManager, RoomSettings, StoredMessage};
use tokio::runtime::Runtime;

fn create_player_metadata(user_id: &str) -> PlayerMetadata {
  PlayerMetadata {
    user_id: user_id.to_string(),
    display_name: format!("Player {}", user_id),
    avatar_url: Some("https://example.com/avatar.png".to_string()),
    joined_at: std::time::Instant::now(),
    custom: json!({
        "level": 10,
        "score": 1000
    }),
  }
}

fn room_creation(c: &mut Criterion) {
  let mut group = c.benchmark_group("room_creation");
  let rt = Runtime::new().unwrap();

  group.bench_function("create_single_room", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings::default();
      let manager = RoomManager::new(settings);

      black_box(manager.get_or_create_room("room1").await);
    });
  });

  group.bench_function("create_100_rooms", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings::default();
      let manager = RoomManager::new(settings);

      for i in 0..100 {
        black_box(manager.get_or_create_room(&format!("room{}", i)).await);
      }
    });
  });

  group.finish();
}

fn room_player_operations(c: &mut Criterion) {
  let mut group = c.benchmark_group("room_player_operations");
  let rt = Runtime::new().unwrap();

  group.bench_function("add_player", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings::default();
      let manager = RoomManager::new(settings);
      let room = manager.get_or_create_room("room1").await;

      let metadata = create_player_metadata("user1");
      let _ = black_box(room.write().await.add_player(metadata));
    });
  });

  group.bench_function("add_10_players", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings::default();
      let manager = RoomManager::new(settings);
      let room = manager.get_or_create_room("room1").await;

      for i in 0..10 {
        let metadata = create_player_metadata(&format!("user{}", i));
        room.write().await.add_player(metadata).ok();
      }
    });
  });

  group.bench_function("remove_player", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings::default();
      let manager = RoomManager::new(settings);
      let room = manager.get_or_create_room("room1").await;

      let metadata = create_player_metadata("user1");
      room.write().await.add_player(metadata).ok();

      black_box(room.write().await.remove_player("user1"));
    });
  });

  group.finish();
}

fn room_capacity_limits(c: &mut Criterion) {
  let mut group = c.benchmark_group("room_capacity");
  let rt = Runtime::new().unwrap();

  for max_players in [2, 10, 50] {
    group.bench_with_input(
      BenchmarkId::from_parameter(max_players),
      &max_players,
      |b, &max_players| {
        b.to_async(&rt).iter(|| async {
          let settings = RoomSettings {
            max_players: Some(max_players),
            ..Default::default()
          };
          let manager = RoomManager::new(settings);
          let room = manager.get_or_create_room("room1").await;

          // Fill room to capacity
          for i in 0..max_players {
            let metadata = create_player_metadata(&format!("user{}", i));
            room.write().await.add_player(metadata).ok();
          }

          // Check if full
          black_box(room.read().await.is_full());
        });
      },
    );
  }

  group.finish();
}

fn message_history(c: &mut Criterion) {
  let mut group = c.benchmark_group("room_message_history");
  let rt = Runtime::new().unwrap();

  group.bench_function("add_message_no_history", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings {
        store_message_history: false,
        ..Default::default()
      };
      let manager = RoomManager::new(settings);
      let room = manager.get_or_create_room("room1").await;

      let msg = StoredMessage {
        from: "user1".to_string(),
        data: json!({"type": "chat", "message": "hello"}),
        timestamp: std::time::Instant::now(),
      };

      black_box(room.write().await.add_message(msg));
    });
  });

  group.bench_function("add_message_with_history", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings {
        store_message_history: true,
        max_history_messages: 100,
        ..Default::default()
      };
      let manager = RoomManager::new(settings);
      let room = manager.get_or_create_room("room1").await;

      let msg = StoredMessage {
        from: "user1".to_string(),
        data: json!({"type": "chat", "message": "hello"}),
        timestamp: std::time::Instant::now(),
      };

      black_box(room.write().await.add_message(msg));
    });
  });

  group.bench_function("fill_message_history", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings {
        store_message_history: true,
        max_history_messages: 100,
        ..Default::default()
      };
      let manager = RoomManager::new(settings);
      let room = manager.get_or_create_room("room1").await;

      // Fill history to capacity and beyond
      for i in 0..150 {
        let msg = StoredMessage {
          from: "user1".to_string(),
          data: json!({"type": "chat", "message": format!("message {}", i)}),
          timestamp: std::time::Instant::now(),
        };
        room.write().await.add_message(msg);
      }
    });
  });

  group.finish();
}

fn room_lookup_performance(c: &mut Criterion) {
  let mut group = c.benchmark_group("room_lookup");
  let rt = Runtime::new().unwrap();

  for num_rooms in [10, 100, 1000] {
    group.bench_with_input(
      BenchmarkId::from_parameter(num_rooms),
      &num_rooms,
      |b, &num_rooms| {
        b.to_async(&rt).iter(|| async {
          let settings = RoomSettings::default();
          let manager = RoomManager::new(settings);

          // Create rooms
          for i in 0..num_rooms {
            manager.get_or_create_room(&format!("room{}", i)).await;
          }

          // Lookup existing room
          black_box(manager.get_room("room50").await);
        });
      },
    );
  }

  group.finish();
}

fn concurrent_room_access(c: &mut Criterion) {
  let mut group = c.benchmark_group("room_concurrent_access");
  let rt = Runtime::new().unwrap();

  for num_tasks in [2, 5, 10] {
    group.bench_with_input(
      BenchmarkId::from_parameter(num_tasks),
      &num_tasks,
      |b, &num_tasks| {
        b.to_async(&rt).iter(|| async {
          let settings = RoomSettings::default();
          let manager = RoomManager::new(settings);
          let room = manager.get_or_create_room("shared_room").await;

          let mut handles = Vec::new();

          for i in 0..num_tasks {
            let room = room.clone();
            let handle = tokio::spawn(async move {
              let metadata = create_player_metadata(&format!("user{}", i));
              room.write().await.add_player(metadata).ok();

              // Simulate some operations
              for j in 0..10 {
                let msg = StoredMessage {
                  from: format!("user{}", i),
                  data: json!({ "msg": j }),
                  timestamp: std::time::Instant::now(),
                };
                room.write().await.add_message(msg);
              }

              room.write().await.remove_player(&format!("user{}", i));
            });
            handles.push(handle);
          }

          for handle in handles {
            handle.await.expect("task failed");
          }
        });
      },
    );
  }

  group.finish();
}

fn room_cleanup(c: &mut Criterion) {
  let mut group = c.benchmark_group("room_cleanup");
  let rt = Runtime::new().unwrap();

  group.bench_function("cleanup_empty_rooms", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings {
        empty_room_timeout: Some(Duration::from_millis(100)),
        ..Default::default()
      };
      let manager = RoomManager::new(settings);

      // Create some rooms
      for i in 0..50 {
        manager.get_or_create_room(&format!("room{}", i)).await;
      }

      // Wait for timeout
      tokio::time::sleep(Duration::from_millis(150)).await;

      black_box(manager.cleanup_empty_rooms().await);
    });
  });

  group.bench_function("cleanup_with_active_rooms", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings {
        empty_room_timeout: Some(Duration::from_millis(100)),
        ..Default::default()
      };
      let manager = RoomManager::new(settings);

      // Create rooms and add players to some
      for i in 0..50 {
        let room = manager.get_or_create_room(&format!("room{}", i)).await;
        if i % 2 == 0 {
          let metadata = create_player_metadata(&format!("user{}", i));
          room.write().await.add_player(metadata).ok();
        }
      }

      tokio::time::sleep(Duration::from_millis(150)).await;

      black_box(manager.cleanup_empty_rooms().await);
    });
  });

  group.finish();
}

fn room_info_queries(c: &mut Criterion) {
  let mut group = c.benchmark_group("room_info");
  let rt = Runtime::new().unwrap();

  group.bench_function("get_room_info", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings::default();
      let manager = RoomManager::new(settings);
      let room = manager.get_or_create_room("room1").await;

      // Add some players
      for i in 0..5 {
        let metadata = create_player_metadata(&format!("user{}", i));
        room.write().await.add_player(metadata).ok();
      }

      black_box(room.read().await.get_info());
    });
  });

  group.bench_function("get_all_room_ids", |b| {
    b.to_async(&rt).iter(|| async {
      let settings = RoomSettings::default();
      let manager = RoomManager::new(settings);

      // Create many rooms
      for i in 0..100 {
        manager.get_or_create_room(&format!("room{}", i)).await;
      }

      black_box(manager.get_all_room_ids().await);
    });
  });

  group.finish();
}

fn room_with_custom_settings(c: &mut Criterion) {
  let mut group = c.benchmark_group("room_custom_settings");
  let rt = Runtime::new().unwrap();

  // Chess game settings (2 players, history enabled)
  let chess_settings = RoomSettings {
    max_players: Some(2),
    allow_spectators: false,
    store_message_history: true,
    max_history_messages: 200,
    empty_room_timeout: Some(Duration::from_secs(600)),
  };

  // Chat room settings (50 players, history enabled)
  let chat_settings = RoomSettings {
    max_players: Some(50),
    allow_spectators: true,
    store_message_history: true,
    max_history_messages: 500,
    empty_room_timeout: Some(Duration::from_secs(300)),
  };

  group.bench_function("chess_room_simulation", |b| {
    b.to_async(&rt).iter(|| async {
      let manager = RoomManager::new(chess_settings.clone());
      let room = manager.get_or_create_room("chess1").await;

      // Add 2 players
      for i in 0..2 {
        let metadata = create_player_metadata(&format!("player{}", i));
        room.write().await.add_player(metadata).ok();
      }

      // Simulate 50 moves
      for i in 0..50 {
        let msg = StoredMessage {
          from: format!("player{}", i % 2),
          data: json!({ "move": format!("e{} to e{}", i, i + 1) }),
          timestamp: std::time::Instant::now(),
        };
        room.write().await.add_message(msg);
      }
    });
  });

  group.bench_function("chat_room_simulation", |b| {
    b.to_async(&rt).iter(|| async {
      let manager = RoomManager::new(chat_settings.clone());
      let room = manager.get_or_create_room("chat1").await;

      // Add 20 players
      for i in 0..20 {
        let metadata = create_player_metadata(&format!("user{}", i));
        room.write().await.add_player(metadata).ok();
      }

      // Simulate 100 chat messages
      for i in 0..100 {
        let msg = StoredMessage {
          from: format!("user{}", i % 20),
          data: json!({ "message": format!("Chat message {}", i) }),
          timestamp: std::time::Instant::now(),
        };
        room.write().await.add_message(msg);
      }
    });
  });

  group.finish();
}

criterion_group!(
  benches,
  room_creation,
  room_player_operations,
  room_capacity_limits,
  message_history,
  room_lookup_performance,
  concurrent_room_access,
  room_cleanup,
  room_info_queries,
  room_with_custom_settings
);
criterion_main!(benches);
