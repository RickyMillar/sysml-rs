//! Library index benchmarks (Phase 1) for sysml-core resolution.

mod common;

use common::*;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn bench_library_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/library_index/lookup");

    for members in [100, 1000, 10000] {
        let mut graph = create_library_graph(10, members / 10);
        graph.ensure_library_index();

        // Pick a name from the middle of the library
        let target_name = format!("LibType5_{}", members / 20);

        group.bench_with_input(
            BenchmarkId::new("members", members),
            &(graph, target_name),
            |b, (graph, name)| {
                b.iter(|| black_box(graph.resolve_in_library(name)));
            },
        );
    }

    group.finish();
}

fn bench_build_library_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/library_index/build");

    for members in [100, 1000, 10000] {
        let graph = create_library_graph(10, members / 10);

        group.bench_with_input(BenchmarkId::new("members", members), &graph, |b, graph| {
            b.iter_batched(
                || graph.clone(),
                |mut g| {
                    g.build_library_index();
                    black_box(g)
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_nested_library_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/library_index/nested");

    for depth in [3, 5, 10] {
        let mut graph = create_nested_library_graph(depth, 10);
        graph.ensure_library_index();

        // Look for a type at the deepest level
        let target_name = format!("Type_L{}_5", depth - 1);

        group.bench_with_input(
            BenchmarkId::new("depth", depth),
            &(graph, target_name),
            |b, (graph, name)| {
                b.iter(|| black_box(graph.resolve_in_library(name)));
            },
        );
    }

    group.finish();
}

criterion_group!(
    library_index_benches,
    bench_library_lookup,
    bench_build_library_index,
    bench_nested_library_lookup,
);

criterion_main!(library_index_benches);
