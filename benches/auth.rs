use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use tapaculo::auth::{JwtAuth, JwtAuthOptions};

fn sign_access_token(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth_sign_access");
    let auth = JwtAuth::new("benchmark-secret-key-12345");

    group.bench_function("sign_single", |b| {
        b.iter(|| {
            black_box(
                auth.sign_access(
                    "user123".to_string(),
                    "room456".to_string(),
                    "session789".to_string(),
                    3600,
                )
                .expect("sign failed"),
            );
        });
    });

    group.bench_function("sign_batch_100", |b| {
        b.iter(|| {
            for i in 0..100 {
                black_box(
                    auth.sign_access(
                        format!("user{}", i),
                        format!("room{}", i),
                        format!("session{}", i),
                        3600,
                    )
                    .expect("sign failed"),
                );
            }
        });
    });

    group.finish();
}

fn sign_refresh_token(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth_sign_refresh");
    let auth = JwtAuth::new("benchmark-secret-key-12345");

    group.bench_function("sign_single", |b| {
        b.iter(|| {
            black_box(
                auth.sign_refresh("user123".to_string(), 86400)
                    .expect("sign failed"),
            );
        });
    });

    group.bench_function("sign_batch_100", |b| {
        b.iter(|| {
            for i in 0..100 {
                black_box(
                    auth.sign_refresh(format!("user{}", i), 86400)
                        .expect("sign failed"),
                );
            }
        });
    });

    group.finish();
}

fn verify_access_token(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth_verify_access");
    let auth = JwtAuth::new("benchmark-secret-key-12345");

    // Pre-generate a token for verification
    let token = auth
        .sign_access(
            "user123".to_string(),
            "room456".to_string(),
            "session789".to_string(),
            3600,
        )
        .expect("sign failed");

    group.bench_function("verify_single", |b| {
        b.iter(|| {
            black_box(auth.verify_access(&token).expect("verify failed"));
        });
    });

    // Generate multiple tokens for batch verification
    let tokens: Vec<String> = (0..100)
        .map(|i| {
            auth.sign_access(
                format!("user{}", i),
                format!("room{}", i),
                format!("session{}", i),
                3600,
            )
            .expect("sign failed")
        })
        .collect();

    group.bench_function("verify_batch_100", |b| {
        b.iter(|| {
            for token in &tokens {
                black_box(auth.verify_access(token).expect("verify failed"));
            }
        });
    });

    group.finish();
}

fn verify_refresh_token(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth_verify_refresh");
    let auth = JwtAuth::new("benchmark-secret-key-12345");

    // Pre-generate a refresh token
    let token = auth
        .sign_refresh("user123".to_string(), 86400)
        .expect("sign failed");

    group.bench_function("verify_single", |b| {
        b.iter(|| {
            black_box(auth.verify_refresh(&token).expect("verify failed"));
        });
    });

    group.finish();
}

fn refresh_access_flow(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth_refresh_flow");
    let auth = JwtAuth::new("benchmark-secret-key-12345");

    // Pre-generate refresh token
    let refresh_token = auth
        .sign_refresh("user123".to_string(), 86400)
        .expect("sign failed");

    group.bench_function("refresh_access", |b| {
        b.iter(|| {
            black_box(
                auth.refresh_access(
                    &refresh_token,
                    "new_room".to_string(),
                    "new_session".to_string(),
                    3600,
                )
                .expect("refresh failed"),
            );
        });
    });

    group.finish();
}

fn sign_verify_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth_roundtrip");
    let auth = JwtAuth::new("benchmark-secret-key-12345");

    group.bench_function("access_token_roundtrip", |b| {
        b.iter(|| {
            let token = auth
                .sign_access(
                    "user123".to_string(),
                    "room456".to_string(),
                    "session789".to_string(),
                    3600,
                )
                .expect("sign failed");
            black_box(auth.verify_access(&token).expect("verify failed"));
        });
    });

    group.bench_function("refresh_token_roundtrip", |b| {
        b.iter(|| {
            let token = auth
                .sign_refresh("user123".to_string(), 86400)
                .expect("sign failed");
            black_box(auth.verify_refresh(&token).expect("verify failed"));
        });
    });

    group.finish();
}

fn auth_with_options(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth_with_validation_options");

    let options = JwtAuthOptions {
        leeway: 60,
        issuer: Some("tapaculo-server".to_string()),
        audience: Some("tapaculo-client".to_string()),
    };
    let auth = JwtAuth::with_options("benchmark-secret-key-12345", options);

    group.bench_function("sign_with_options", |b| {
        b.iter(|| {
            black_box(
                auth.sign_access(
                    "user123".to_string(),
                    "room456".to_string(),
                    "session789".to_string(),
                    3600,
                )
                .expect("sign failed"),
            );
        });
    });

    let token = auth
        .sign_access(
            "user123".to_string(),
            "room456".to_string(),
            "session789".to_string(),
            3600,
        )
        .expect("sign failed");

    group.bench_function("verify_with_options", |b| {
        b.iter(|| {
            black_box(auth.verify_access(&token).expect("verify failed"));
        });
    });

    group.finish();
}

fn concurrent_token_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("auth_concurrent");

    for num_threads in [1, 2, 4, 8] {
        group.bench_with_input(
            BenchmarkId::new("sign_concurrent", num_threads),
            &num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let auth = JwtAuth::new("benchmark-secret-key-12345");
                    let handles: Vec<_> = (0..num_threads)
                        .map(|i| {
                            let auth = auth.clone();
                            std::thread::spawn(move || {
                                for j in 0..25 {
                                    let _ = auth.sign_access(
                                        format!("user{}_{}", i, j),
                                        format!("room{}_{}", i, j),
                                        format!("session{}_{}", i, j),
                                        3600,
                                    );
                                }
                            })
                        })
                        .collect();

                    for handle in handles {
                        handle.join().unwrap();
                    }
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    sign_access_token,
    sign_refresh_token,
    verify_access_token,
    verify_refresh_token,
    refresh_access_flow,
    sign_verify_roundtrip,
    auth_with_options,
    concurrent_token_operations
);
criterion_main!(benches);
