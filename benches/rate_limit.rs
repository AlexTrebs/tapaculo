use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::time::Duration;
use tapaculo::rate_limit::{MessageLimits, RateLimiter};
use tokio::runtime::Runtime;

fn check_allowed_single_user(c: &mut Criterion) {
  let mut group = c.benchmark_group("rate_limit_single_user");
  let rt = Runtime::new().unwrap();

  let limits = MessageLimits::default();
  let limiter = RateLimiter::new(limits);

  group.bench_function("check_allowed_within_limit", |b| {
    b.to_async(&rt).iter(|| async {
      let limiter = limiter.clone();
      // Reset state for each iteration
      limiter.reset_user("user1").await;

      // Send messages within limit (default is 10 per second)
      for _ in 0..5 {
        black_box(
          limiter
            .check_allowed("user1", 1024)
            .await
            .expect("should be allowed"),
        );
      }
    });
  });

  group.bench_function("check_allowed_until_limit", |b| {
    b.to_async(&rt).iter(|| async {
      let limiter = limiter.clone();
      limiter.reset_user("user2").await;

      // Send up to the limit
      for _ in 0..10 {
        black_box(limiter.check_allowed("user2", 1024).await.is_ok());
      }
    });
  });

  group.finish();
}

fn check_allowed_multiple_users(c: &mut Criterion) {
  let mut group = c.benchmark_group("rate_limit_multiple_users");
  let rt = Runtime::new().unwrap();

  for num_users in [10, 50, 100] {
    group.bench_with_input(
      BenchmarkId::from_parameter(num_users),
      &num_users,
      |b, &num_users| {
        b.to_async(&rt).iter(|| async {
          let limits = MessageLimits::default();
          let limiter = RateLimiter::new(limits);

          // Each user sends 5 messages
          for user_id in 0..num_users {
            for _ in 0..5 {
              let _ = black_box(
                limiter
                  .check_allowed(&format!("user{}", user_id), 1024)
                  .await,
              );
            }
          }
        });
      },
    );
  }

  group.finish();
}

fn concurrent_rate_limiting(c: &mut Criterion) {
  let mut group = c.benchmark_group("rate_limit_concurrent");
  let rt = Runtime::new().unwrap();

  for num_concurrent in [2, 5, 10] {
    group.bench_with_input(
      BenchmarkId::from_parameter(num_concurrent),
      &num_concurrent,
      |b, &num_concurrent| {
        b.to_async(&rt).iter(|| async {
          let limits = MessageLimits::default();
          let limiter = RateLimiter::new(limits);

          let mut handles = Vec::new();

          for user_id in 0..num_concurrent {
            let limiter = limiter.clone();
            let handle = tokio::spawn(async move {
              for _ in 0..20 {
                limiter
                  .check_allowed(&format!("user{}", user_id), 1024)
                  .await
                  .ok();
              }
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

fn ban_operations(c: &mut Criterion) {
  let mut group = c.benchmark_group("rate_limit_ban");
  let rt = Runtime::new().unwrap();

  let limits = MessageLimits::default();
  let limiter = RateLimiter::new(limits);

  group.bench_function("ban_user", |b| {
    b.to_async(&rt).iter(|| async {
      black_box(limiter.ban_user("user1", Duration::from_secs(60)).await);
    });
  });

  group.bench_function("is_banned", |b| {
    b.to_async(&rt).iter(|| async {
      black_box(limiter.is_banned("user1").await);
    });
  });

  group.bench_function("unban_user", |b| {
    b.to_async(&rt).iter(|| async {
      black_box(limiter.unban_user("user1").await);
    });
  });

  group.bench_function("reset_user", |b| {
    b.to_async(&rt).iter(|| async {
      black_box(limiter.reset_user("user1").await);
    });
  });

  group.finish();
}

fn message_size_checking(c: &mut Criterion) {
  let mut group = c.benchmark_group("rate_limit_message_size");
  let rt = Runtime::new().unwrap();

  let limits = MessageLimits {
    max_size_bytes: 10 * 1024, // 10KB
    ..Default::default()
  };
  let limiter = RateLimiter::new(limits);

  group.bench_function("check_small_message", |b| {
    b.to_async(&rt).iter(|| async {
      let limiter = limiter.clone();
      limiter.reset_user("user1").await;
      let _ = black_box(limiter.check_allowed("user1", 1024).await); // 1KB
    });
  });

  group.bench_function("check_large_message_allowed", |b| {
    b.to_async(&rt).iter(|| async {
      let limiter = limiter.clone();
      limiter.reset_user("user2").await;
      let _ = black_box(limiter.check_allowed("user2", 9 * 1024).await); // 9KB (within limit)
    });
  });

  group.bench_function("check_oversized_message", |b| {
    b.to_async(&rt).iter(|| async {
      let limiter = limiter.clone();
      limiter.reset_user("user3").await;
      let _ = black_box(limiter.check_allowed("user3", 20 * 1024).await); // 20KB (exceeds limit)
    });
  });

  group.finish();
}

fn rate_limit_window_cleanup(c: &mut Criterion) {
  let mut group = c.benchmark_group("rate_limit_cleanup");
  let rt = Runtime::new().unwrap();

  group.bench_function("cleanup_old_messages", |b| {
    b.to_async(&rt).iter(|| async {
      let limits = MessageLimits {
        window_duration: Duration::from_millis(100),
        ..Default::default()
      };
      let limiter = RateLimiter::new(limits);

      // Send some messages
      for _ in 0..5 {
        limiter.check_allowed("user1", 1024).await.ok();
      }

      // Wait for window to expire
      tokio::time::sleep(Duration::from_millis(150)).await;

      // Next check should clean old messages
      let _ = black_box(limiter.check_allowed("user1", 1024).await);
    });
  });

  group.finish();
}

fn rate_limit_stress_test(c: &mut Criterion) {
  let mut group = c.benchmark_group("rate_limit_stress");
  let rt = Runtime::new().unwrap();

  group.bench_function("many_users_many_messages", |b| {
    b.to_async(&rt).iter(|| async {
      let limits = MessageLimits::default();
      let limiter = RateLimiter::new(limits);

      // Simulate 50 users each sending 20 messages
      for user_id in 0..50 {
        for _ in 0..20 {
          limiter
            .check_allowed(&format!("user{}", user_id), 1024)
            .await
            .ok();
        }
      }
    });
  });

  group.finish();
}

fn custom_limits(c: &mut Criterion) {
  let mut group = c.benchmark_group("rate_limit_custom_limits");
  let rt = Runtime::new().unwrap();

  // Strict limits
  let strict_limits = MessageLimits {
    max_size_bytes: 1024,
    max_messages_per_window: 5,
    window_duration: Duration::from_secs(1),
    ban_duration: Duration::from_secs(30),
  };

  // Relaxed limits
  let relaxed_limits = MessageLimits {
    max_size_bytes: 100 * 1024,
    max_messages_per_window: 100,
    window_duration: Duration::from_secs(1),
    ban_duration: Duration::from_secs(10),
  };

  group.bench_function("strict_limits", |b| {
    b.to_async(&rt).iter(|| async {
      let limiter = RateLimiter::new(strict_limits.clone());
      for _ in 0..10 {
        limiter.check_allowed("user1", 512).await.ok();
      }
      limiter.reset_user("user1").await;
    });
  });

  group.bench_function("relaxed_limits", |b| {
    b.to_async(&rt).iter(|| async {
      let limiter = RateLimiter::new(relaxed_limits.clone());
      for _ in 0..50 {
        limiter.check_allowed("user1", 10240).await.ok();
      }
      limiter.reset_user("user1").await;
    });
  });

  group.finish();
}

criterion_group!(
  benches,
  check_allowed_single_user,
  check_allowed_multiple_users,
  concurrent_rate_limiting,
  ban_operations,
  message_size_checking,
  rate_limit_window_cleanup,
  rate_limit_stress_test,
  custom_limits
);
criterion_main!(benches);
