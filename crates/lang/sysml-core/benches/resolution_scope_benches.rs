//! Scope table benchmarks (Phases 2-3) for sysml-core resolution.

mod common;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use sysml_core::resolution::ResolutionContext;
use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_id::ElementId;

fn bench_build_scope_table(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/scope_table/build");

    for members in [10, 50, 200] {
        // Create a package with many members
        let mut graph = ModelGraph::new();
        let mut pkg = Element::new_with_kind(ElementKind::Package);
        pkg.name = Some("TestPackage".to_string());
        let pkg_id = graph.add_element(pkg);

        for m in 0..members {
            let mut member = Element::new_with_kind(ElementKind::PartDefinition);
            member.name = Some(format!("Member{}", m));
            member.owner = Some(pkg_id.clone());
            let member_id = graph.add_element(member);

            let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
            membership.owner = Some(pkg_id.clone());
            membership.set_prop("memberElement", Value::Ref(member_id.clone()));
            membership.set_prop("memberName", Value::String(format!("Member{}", m)));
            membership.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(membership);
        }

        group.bench_with_input(
            BenchmarkId::new("members", members),
            &(graph, pkg_id),
            |b, (graph, pkg_id)| {
                b.iter_batched(
                    || ResolutionContext::new(graph),
                    |mut ctx| {
                        let _ = ctx.get_scope_table(pkg_id);
                        black_box(ctx)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_cached_scope_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/scope_table/cached_lookup");

    for lookups in [10, 100, 1000] {
        // Create a simple graph
        let mut graph = ModelGraph::new();
        let mut pkg = Element::new_with_kind(ElementKind::Package);
        pkg.name = Some("TestPackage".to_string());
        let pkg_id = graph.add_element(pkg);

        for m in 0..50 {
            let mut member = Element::new_with_kind(ElementKind::PartDefinition);
            member.name = Some(format!("Member{}", m));
            member.owner = Some(pkg_id.clone());
            let member_id = graph.add_element(member);

            let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
            membership.owner = Some(pkg_id.clone());
            membership.set_prop("memberElement", Value::Ref(member_id.clone()));
            membership.set_prop("memberName", Value::String(format!("Member{}", m)));
            membership.set_prop("visibility", Value::String("public".to_string()));
            graph.add_element(membership);
        }

        // Pre-build scope table
        let mut ctx = ResolutionContext::new(&graph);
        let _ = ctx.get_scope_table(&pkg_id);

        group.bench_with_input(
            BenchmarkId::new("lookups", lookups),
            &(ctx, pkg_id),
            |b, (ctx, pkg_id)| {
                b.iter(|| {
                    for i in 0..lookups {
                        let name = format!("Member{}", i % 50);
                        black_box(ctx.graph().resolve_name_in(pkg_id, &name));
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_prebuild_scope_table(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolution/scope_table/prebuild_full");

    for depth in [5, 10, 20] {
        // Create a deeply nested graph
        let mut graph = ModelGraph::new();
        let mut parent_id: Option<ElementId> = None;
        let mut deepest_id = ElementId::new_v4();

        for level in 0..depth {
            let mut pkg = Element::new_with_kind(ElementKind::Package);
            pkg.name = Some(format!("Level{}", level));
            if let Some(pid) = parent_id.clone() {
                pkg.owner = Some(pid);
            }
            let pkg_id = graph.add_element(pkg);

            // Add some members at each level
            for m in 0..5 {
                let mut member = Element::new_with_kind(ElementKind::PartDefinition);
                member.name = Some(format!("Member{}_{}", level, m));
                member.owner = Some(pkg_id.clone());
                let member_id = graph.add_element(member);

                let mut membership = Element::new_with_kind(ElementKind::OwningMembership);
                membership.owner = Some(pkg_id.clone());
                membership.set_prop("memberElement", Value::Ref(member_id.clone()));
                membership.set_prop(
                    "memberName",
                    Value::String(format!("Member{}_{}", level, m)),
                );
                membership.set_prop("visibility", Value::String("public".to_string()));
                graph.add_element(membership);
            }

            parent_id = Some(pkg_id.clone());
            deepest_id = pkg_id;
        }

        group.bench_with_input(
            BenchmarkId::new("depth", depth),
            &(graph, deepest_id),
            |b, (graph, deepest_id)| {
                b.iter_batched(
                    || ResolutionContext::new(graph),
                    |mut ctx| {
                        ctx.get_full_scope_table(deepest_id);
                        black_box(ctx)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

criterion_group!(
    scope_table_benches,
    bench_build_scope_table,
    bench_cached_scope_lookup,
    bench_prebuild_scope_table,
);

criterion_main!(scope_table_benches);
