//! Inheritance benchmarks (Phase 2) for sysml-core resolution.

mod common;

use common::*;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use sysml_core::resolution::ResolutionContext;

fn bench_shallow_inheritance(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/inheritance/shallow");

    for inherited_members in [10, 50, 100] {
        let (graph, def_ids) = create_inheritance_graph(inherited_members, 1);

        // Look up an inherited feature
        let child_id = &def_ids[0];

        group.bench_with_input(
            BenchmarkId::new("members", inherited_members),
            &(graph, child_id.clone()),
            |b, (graph, child_id)| {
                b.iter_batched(
                    || ResolutionContext::new(graph),
                    |mut ctx| {
                        let result = ctx.resolve_name(child_id, "feature_0_0");
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_deep_inheritance(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/inheritance/deep");

    for depth in [3, 5, 10] {
        let (graph, def_ids) = create_inheritance_graph(depth * 2, depth);

        // Get the deepest type in the first chain
        let deepest_id = &def_ids[depth - 1];

        group.bench_with_input(
            BenchmarkId::new("depth", depth),
            &(graph, deepest_id.clone()),
            |b, (graph, deepest_id)| {
                b.iter_batched(
                    || ResolutionContext::new(graph),
                    |mut ctx| {
                        // Look for a feature from the base type
                        let result = ctx.resolve_name(deepest_id, "feature_0_0");
                        black_box(result)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_diamond_inheritance(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/inheritance/diamond");

    for supertypes in [4, 8] {
        let (graph, derived_id) = create_diamond_inheritance_graph(supertypes);

        group.bench_with_input(
            BenchmarkId::new("supertypes", supertypes),
            &(graph, derived_id),
            |b, (graph, derived_id)| {
                b.iter_batched(
                    || ResolutionContext::new(graph),
                    |mut ctx| {
                        // Look for a feature from the base type
                        let result = ctx.resolve_name(derived_id, "baseFeature0");
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
    inheritance_benches,
    bench_shallow_inheritance,
    bench_deep_inheritance,
    bench_diamond_inheritance,
);

criterion_main!(inheritance_benches);
