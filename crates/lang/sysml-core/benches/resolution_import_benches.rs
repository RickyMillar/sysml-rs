//! Import benchmarks (Phase 3) for sysml-core resolution.

mod common;

use common::*;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use sysml_core::resolution::ResolutionContext;

fn bench_membership_import(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/imports/membership");

    for imports in [1, 10, 50] {
        let graph = create_import_graph(imports + 1, imports, 10);

        // Get the first package (has imports)
        let pkg_id = graph.roots().next().unwrap().id.clone();

        group.bench_with_input(
            BenchmarkId::new("imports", imports),
            &(graph, pkg_id),
            |b, (graph, pkg_id)| {
                b.iter_batched(
                    || ResolutionContext::new(graph),
                    |mut ctx| {
                        // Look for an imported type
                        let result = ctx.resolve_name(pkg_id, "Type1_5");
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_namespace_import(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/imports/namespace");

    for members in [10, 50, 200] {
        let graph = create_import_graph(2, 1, members);

        // Get the first package
        let pkg_id = graph.roots().next().unwrap().id.clone();

        group.bench_with_input(
            BenchmarkId::new("members", members),
            &(graph, pkg_id),
            |b, (graph, pkg_id)| {
                b.iter_batched(
                    || ResolutionContext::new(graph),
                    |mut ctx| {
                        // Look for an imported type from the middle
                        let target = format!("Type1_{}", members / 2);
                        let result = ctx.resolve_name(pkg_id, &target);
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_recursive_import(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/imports/recursive");

    for depth in [2, 3, 5] {
        let (graph, top_pkg_id) = create_recursive_import_graph(depth, 10);

        group.bench_with_input(
            BenchmarkId::new("depth", depth),
            &(graph, top_pkg_id),
            |b, (graph, top_pkg_id)| {
                b.iter_batched(
                    || ResolutionContext::new(graph),
                    |mut ctx| {
                        // Look for a type from the deepest level
                        let target = format!("TypeL{}_5", depth - 1);
                        let result = ctx.resolve_name(top_pkg_id, &target);
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    import_benches,
    bench_membership_import,
    bench_namespace_import,
    bench_recursive_import,
);

criterion_main!(import_benches);
