use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde::{Deserialize, Serialize};
use tapaculo::pubsub::{InMemoryPubSub, PubSubExt};
use tokio::runtime::Runtime;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct BenchMessage {
  id: u64,
  data: String,
}

impl BenchMessage {
  fn new(id: u64) -> Self {
    Self {
      id,
      data: "benchmark message payload".to_string(),
    }
  }

  fn large(id: u64) -> Self {
    Self {
      id,
      data: "x".repeat(1024), // 1KB payload
    }
  }
}

fn publish_throughput(c: &mut Criterion) {
  let mut group = c.benchmark_group("pubsub_publish");
  let rt = Runtime::new().unwrap();

  for size in [1, 10, 100, 1000] {
    group.throughput(Throughput::Elements(size as u64));
    group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
      b.to_async(&rt).iter(|| async {
        let backend = InMemoryPubSub::new();
        let topic = "bench";

        for i in 0..size {
          let msg = BenchMessage::new(i as u64);
          backend.publish(topic, &msg).await.expect("publish failed");
        }
      });
    });
  }
  group.finish();
}

fn subscribe_and_receive(c: &mut Criterion) {
  let mut group = c.benchmark_group("pubsub_subscribe_receive");
  let rt = Runtime::new().unwrap();

  for num_messages in [10, 100, 1000] {
    group.throughput(Throughput::Elements(num_messages as u64));
    group.bench_with_input(
      BenchmarkId::from_parameter(num_messages),
      &num_messages,
      |b, &num_messages| {
        b.to_async(&rt).iter(|| async {
          let backend = InMemoryPubSub::new();
          let topic = "bench";

          let (tx, mut rx) = tokio::sync::mpsc::channel(num_messages);

          let _sub = backend
            .subscribe(topic, move |data: Vec<u8>| {
              let tx = tx.clone();
              async move {
                let _msg: BenchMessage = serde_json::from_slice(&data).unwrap();
                tx.send(()).await.ok();
              }
            })
            .await
            .expect("subscribe failed");

          // Give subscription time to start
          tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;

          // Publish messages
          for i in 0..num_messages {
            backend
              .publish(topic, &BenchMessage::new(i as u64))
              .await
              .expect("publish failed");
          }

          // Wait for all messages to be received
          for _ in 0..num_messages {
            rx.recv().await.expect("receive failed");
          }
        });
      },
    );
  }
  group.finish();
}

fn multiple_subscribers(c: &mut Criterion) {
  let mut group = c.benchmark_group("pubsub_multiple_subscribers");
  let rt = Runtime::new().unwrap();

  for num_subs in [1, 5, 10, 20] {
    group.bench_with_input(
      BenchmarkId::from_parameter(num_subs),
      &num_subs,
      |b, &num_subs| {
        b.to_async(&rt).iter(|| async {
          let backend = InMemoryPubSub::new();
          let topic = "bench";
          let num_messages = 100;

          let mut channels = Vec::new();
          let mut subscriptions = Vec::new();

          // Create multiple subscribers
          for _ in 0..num_subs {
            let (tx, rx) = tokio::sync::mpsc::channel(num_messages);
            channels.push(rx);

            let sub = backend
              .subscribe(topic, move |data: Vec<u8>| {
                let tx = tx.clone();
                async move {
                  let _msg: BenchMessage = serde_json::from_slice(&data).unwrap();
                  tx.send(()).await.ok();
                }
              })
              .await
              .expect("subscribe failed");
            subscriptions.push(sub);
          }

          // Give subscriptions time to start
          tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

          // Publish messages
          for i in 0..num_messages {
            backend
              .publish(topic, &BenchMessage::new(i as u64))
              .await
              .expect("publish failed");
          }

          // Wait for all subscribers to receive all messages
          for mut rx in channels {
            for _ in 0..num_messages {
              rx.recv().await.expect("receive failed");
            }
          }

          black_box(subscriptions);
        });
      },
    );
  }
  group.finish();
}

fn concurrent_publishers(c: &mut Criterion) {
  let mut group = c.benchmark_group("pubsub_concurrent_publishers");
  let rt = Runtime::new().unwrap();

  for num_publishers in [1, 5, 10] {
    group.bench_with_input(
      BenchmarkId::from_parameter(num_publishers),
      &num_publishers,
      |b, &num_publishers| {
        b.to_async(&rt).iter(|| async {
          let backend = InMemoryPubSub::new();
          let messages_per_publisher = 100;

          let mut handles = Vec::new();

          for pub_id in 0..num_publishers {
            let backend = backend.clone();
            let handle = tokio::spawn(async move {
              for i in 0..messages_per_publisher {
                backend
                  .publish(&format!("topic_{}", pub_id), &BenchMessage::new(i as u64))
                  .await
                  .expect("publish failed");
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

fn large_messages(c: &mut Criterion) {
  let mut group = c.benchmark_group("pubsub_large_messages");
  let rt = Runtime::new().unwrap();

  group.bench_function("publish_1kb_messages", |b| {
    b.to_async(&rt).iter(|| async {
      let backend = InMemoryPubSub::new();
      let topic = "bench";

      for i in 0..100 {
        let msg = BenchMessage::large(i);
        backend.publish(topic, &msg).await.expect("publish failed");
      }
    });
  });

  group.finish();
}

criterion_group!(
  benches,
  publish_throughput,
  subscribe_and_receive,
  multiple_subscribers,
  concurrent_publishers,
  large_messages
);
criterion_main!(benches);
