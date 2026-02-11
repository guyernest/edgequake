//! Benchmarks for S3 vector search performance.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use edgequake_storage::VectorStorage;
use edgequake_storage_aws::{S3Config, S3VectorStorage};
use serde_json::json;

/// Generate random vector
fn random_vector(dimension: usize, seed: u64) -> Vec<f32> {
    (0..dimension)
        .map(|i| ((seed as f64 * 0.1 + i as f64) % 1.0) as f32)
        .collect()
}

/// Benchmark vector insertion
fn bench_upsert(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("upsert");

    for batch_size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*batch_size as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            batch_size,
            |b, &size| {
                b.to_async(&runtime).iter(|| async move {
                    // Note: This benchmark requires a real S3 bucket
                    // Set TEST_S3_BUCKET to run
                    if std::env::var("TEST_S3_BUCKET").is_err() {
                        return;
                    }

                    let config = S3Config::new(
                        std::env::var("TEST_S3_BUCKET").unwrap(),
                        "bench-workspace",
                        128,
                    );
                    let storage = S3VectorStorage::new(config).await.unwrap();
                    storage.initialize().await.unwrap();

                    let data: Vec<_> = (0..size)
                        .map(|i| {
                            (
                                format!("vec-{}", i),
                                random_vector(128, i as u64),
                                json!({"index": i}),
                            )
                        })
                        .collect();

                    black_box(storage.upsert(&data).await.unwrap());
                });
            },
        );
    }

    group.finish();
}

/// Benchmark vector query
fn bench_query(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("query");

    for top_k in [1, 10, 100].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(top_k), top_k, |b, &k| {
            b.to_async(&runtime).iter(|| async move {
                if std::env::var("TEST_S3_BUCKET").is_err() {
                    return;
                }

                let config =
                    S3Config::new(std::env::var("TEST_S3_BUCKET").unwrap(), "bench-workspace", 128);
                let storage = S3VectorStorage::new(config).await.unwrap();
                storage.initialize().await.unwrap();

                let query = random_vector(128, 42);
                black_box(storage.query(&query, k, None).await.unwrap());
            });
        });
    }

    group.finish();
}

/// Benchmark index loading
fn bench_index_load(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    c.bench_function("index_load", |b| {
        b.to_async(&runtime).iter(|| async {
            if std::env::var("TEST_S3_BUCKET").is_err() {
                return;
            }

            let config =
                S3Config::new(std::env::var("TEST_S3_BUCKET").unwrap(), "bench-workspace", 128);
            let storage = S3VectorStorage::new(config).await.unwrap();
            black_box(storage.initialize().await.unwrap());
        });
    });
}

criterion_group!(benches, bench_upsert, bench_query, bench_index_load);
criterion_main!(benches);
