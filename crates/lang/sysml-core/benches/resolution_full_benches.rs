//! Full resolution, scaling, and parallel benchmarks for sysml-core resolution.

mod common;

use common::*;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use sysml_core::resolution::resolve_references;

// =============================================================================
// Full Resolution Benchmarks (End-to-end)
// =============================================================================

fn bench_simple_model_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/full/simple_model");

    for elements in [100, 500, 1000] {
        let graph = create_realistic_model(elements / 5, 2, 0, 1);

        group.bench_with_input(
            BenchmarkId::new("elements", elements),
            &graph,
            |b, graph| {
                b.iter_batched(
                    || graph.clone(),
                    |mut g| {
                        let result = resolve_references(&mut g);
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_with_library_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/full/with_library");

    for (model_size, lib_size) in [(100, 1000), (500, 5000)] {
        let graph = create_realistic_model(model_size / 5, 2, lib_size, 3);

        group.bench_with_input(
            BenchmarkId::new("model_lib", format!("{}+{}", model_size, lib_size)),
            &graph,
            |b, graph| {
                b.iter_batched(
                    || graph.clone(),
                    |mut g| {
                        let result = resolve_references(&mut g);
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_complex_model_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/full/complex_model");

    for elements in [500, 1000, 2000] {
        let graph = create_realistic_model(
            elements / 10, // definitions
            3,             // usages per def
            elements / 5,  // library members
            5,             // inheritance depth
        );

        group.bench_with_input(
            BenchmarkId::new("elements", elements),
            &graph,
            |b, graph| {
                b.iter_batched(
                    || graph.clone(),
                    |mut g| {
                        let result = resolve_references(&mut g);
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

// =============================================================================
// Scaling Benchmarks (O(n) vs O(n^2))
// =============================================================================

fn bench_linear_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/scaling/linear");

    // Test multiple sizes to verify O(n) behavior
    for elements in [100, 500, 1000, 2000, 5000] {
        let graph = create_realistic_model(elements / 10, 2, elements / 5, 3);

        group.bench_with_input(
            BenchmarkId::new("elements", elements),
            &graph,
            |b, graph| {
                b.iter_batched(
                    || graph.clone(),
                    |mut g| {
                        let result = resolve_references(&mut g);
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_library_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/scaling/library");

    // Test library lookup scaling
    for lib_size in [1000, 5000, 10000, 20000] {
        let graph = create_realistic_model(50, 2, lib_size, 3);

        group.bench_with_input(
            BenchmarkId::new("library_size", lib_size),
            &graph,
            |b, graph| {
                b.iter_batched(
                    || graph.clone(),
                    |mut g| {
                        let result = resolve_references(&mut g);
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

// =============================================================================
// Parallel Resolution Benchmarks
// =============================================================================
// These benchmarks exercise models large enough to trigger the parallel
// resolution path (>100 unresolved elements). Sizes are chosen to generate
// a high density of unresolved references per element.

fn bench_parallel_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/parallel");
    group.sample_size(10); // Larger models take longer

    // Each model generates approximately:
    //   definitions * usages_per_def FeatureTypings + definitions Specializations
    // So (100, 4, 200, 3) -> ~400 FeatureTypings + 99 Specializations = ~499 unresolved
    for (label, defs, usages, lib, depth) in [
        ("500_unresolved", 100, 4, 200, 3),
        ("1000_unresolved", 200, 4, 400, 3),
        ("2000_unresolved", 400, 4, 800, 5),
    ] {
        let graph = create_realistic_model(defs, usages, lib, depth);

        group.bench_with_input(BenchmarkId::new("elements", label), &graph, |b, graph| {
            b.iter_batched(
                || graph.clone(),
                |mut g| {
                    let result = resolve_references(&mut g);
                    black_box(result)
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

/// Benchmark the sequential path by creating models below the parallel threshold.
fn bench_sequential_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/sequential");

    // Below-threshold: ~50 unresolved elements (under PARALLEL_THRESHOLD=100)
    for (label, defs, usages, lib, depth) in [
        ("20_unresolved", 5, 3, 10, 2),
        ("50_unresolved", 10, 4, 20, 2),
        ("90_unresolved", 18, 4, 40, 3),
    ] {
        let graph = create_realistic_model(defs, usages, lib, depth);

        group.bench_with_input(BenchmarkId::new("elements", label), &graph, |b, graph| {
            b.iter_batched(
                || graph.clone(),
                |mut g| {
                    let result = resolve_references(&mut g);
                    black_box(result)
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

// =============================================================================
// Benchmark Groups
// =============================================================================

criterion_group!(
    full_resolution_benches,
    bench_simple_model_resolution,
    bench_with_library_resolution,
    bench_complex_model_resolution,
);

criterion_group!(scaling_benches, bench_linear_scaling, bench_library_scaling,);

criterion_group!(
    parallel_benches,
    bench_parallel_resolution,
    bench_sequential_resolution,
);

criterion_main!(full_resolution_benches, scaling_benches, parallel_benches,);
