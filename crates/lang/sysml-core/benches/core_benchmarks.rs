//! Benchmarks for sysml-core crate
//!
//! Tests optimizations:
//! - #1: O(n) graph scans in resolution
//! - #4: ElementId cloning in ownership operations
//! - #8: Default HashMap hasher (FxHash opportunity)
//! - Validation performance

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use sysml_core::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};
use sysml_id::ElementId;

// =============================================================================
// Test Data Generators
// =============================================================================

/// Create a test graph with specified characteristics
fn create_test_graph(elements: usize, depth: usize) -> (ModelGraph, Vec<ElementId>) {
    let mut graph = ModelGraph::new();
    let mut ids = Vec::with_capacity(elements);
    let mut parent_stack: Vec<ElementId> = Vec::new();

    for i in 0..elements {
        let current_depth = i % depth.max(1);
        parent_stack.truncate(current_depth);

        let owner = parent_stack.last().cloned();

        let mut element = Element::new_with_kind(ElementKind::PartDefinition);
        element.name = Some(format!("Element{}", i));
        if let Some(o) = owner {
            element.owner = Some(o);
        }

        let id = graph.add_element(element);
        ids.push(id.clone());

        if current_depth < depth.saturating_sub(1) {
            parent_stack.push(id);
        }
    }

    (graph, ids)
}

/// Create a graph with many relationships
fn create_graph_with_relationships(elements: usize, rels_per_element: usize) -> ModelGraph {
    let mut graph = ModelGraph::new();
    let mut ids = Vec::with_capacity(elements);

    // Create elements
    for i in 0..elements {
        let mut element = Element::new_with_kind(ElementKind::PartDefinition);
        element.name = Some(format!("Part{}", i));
        let id = graph.add_element(element);
        ids.push(id);
    }

    // Create relationships
    for i in 0..elements {
        for j in 0..rels_per_element.min(elements - 1) {
            let target_idx = (i + j + 1) % elements;
            let rel = Relationship {
                id: ElementId::new_v4(),
                kind: RelationshipKind::Specialize,
                source: ids[i].clone(),
                target: ids[target_idx].clone(),
                props: Default::default(),
            };
            graph.add_relationship(rel);
        }
    }

    graph
}

/// Create a graph with wide namespaces (many children per parent)
fn create_wide_graph(parents: usize, children_per_parent: usize) -> (ModelGraph, Vec<ElementId>) {
    let mut graph = ModelGraph::new();
    let mut parent_ids = Vec::with_capacity(parents);

    for p in 0..parents {
        let mut parent = Element::new_with_kind(ElementKind::Package);
        parent.name = Some(format!("Package{}", p));
        let parent_id = graph.add_element(parent);
        parent_ids.push(parent_id.clone());

        for c in 0..children_per_parent {
            let mut child = Element::new_with_kind(ElementKind::PartDefinition);
            child.name = Some(format!("Part{}_{}", p, c));
            child.owner = Some(parent_id.clone());
            graph.add_element(child);
        }
    }

    (graph, parent_ids)
}

/// Create a deep chain for ancestry testing
fn create_deep_chain(depth: usize) -> (ModelGraph, ElementId) {
    let mut graph = ModelGraph::new();
    let mut parent: Option<ElementId> = None;
    let mut deepest_id = ElementId::new_v4();

    for i in 0..depth {
        let mut element = Element::new_with_kind(ElementKind::Package);
        element.name = Some(format!("Level{}", i));
        if let Some(p) = parent.clone() {
            element.owner = Some(p);
        }
        let id = graph.add_element(element);
        parent = Some(id.clone());
        deepest_id = id;
    }

    (graph, deepest_id)
}

// =============================================================================
// Element Operations
// =============================================================================

fn bench_add_element(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/add_element");

    for size in [100, 1000, 10000] {
        group.bench_with_input(
            BenchmarkId::new("to_graph_size", size),
            &size,
            |b, &size| {
                b.iter_batched(
                    || {
                        let (graph, _) = create_test_graph(size, 10);
                        let element =
                            Element::new_with_kind(ElementKind::PartUsage).with_name("NewElement");
                        (graph, element)
                    },
                    |(mut graph, element)| black_box(graph.add_element(element)),
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_element_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/element_lookup");

    for size in [100, 1000, 10000, 50000] {
        let (graph, ids) = create_test_graph(size, 10);

        group.bench_with_input(
            BenchmarkId::new("graph_size", size),
            &(graph, ids),
            |b, (graph, ids)| {
                let mut i = 0;
                b.iter(|| {
                    let id = &ids[i % ids.len()];
                    i += 1;
                    black_box(graph.get_element(id))
                });
            },
        );
    }

    group.finish();
}

fn bench_children_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/children_lookup");

    for children_per_parent in [5, 20, 100, 500] {
        let (graph, parent_ids) = create_wide_graph(10, children_per_parent);
        let parent = &parent_ids[0];

        group.bench_with_input(
            BenchmarkId::new("children_count", children_per_parent),
            &(graph, parent.clone()),
            |b, (graph, parent)| {
                b.iter(|| black_box(graph.children_of(parent).count()));
            },
        );
    }

    group.finish();
}

fn bench_elements_by_kind(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/elements_by_kind");

    for size in [1000, 10000, 50000] {
        let (graph, _) = create_test_graph(size, 10);

        group.bench_with_input(BenchmarkId::new("graph_size", size), &graph, |b, graph| {
            b.iter(|| {
                black_box(
                    graph
                        .elements
                        .values()
                        .filter(|e| e.kind == ElementKind::PartDefinition)
                        .count(),
                )
            });
        });
    }

    group.finish();
}

// =============================================================================
// Index Operations
// =============================================================================

fn bench_rebuild_indexes(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/rebuild_indexes");

    for size in [100, 1000, 10000] {
        let (graph, _) = create_test_graph(size, 10);

        group.bench_with_input(
            BenchmarkId::new("element_count", size),
            &graph,
            |b, graph| {
                b.iter_batched(
                    || graph.clone(),
                    |mut g| {
                        g.rebuild_indexes();
                        black_box(g)
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

// =============================================================================
// Ancestry Operations (Tests ElementId cloning)
// =============================================================================

fn bench_ancestry_chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/ancestry");

    for depth in [5, 10, 25, 50, 100] {
        let (graph, deepest_id) = create_deep_chain(depth);

        group.bench_with_input(
            BenchmarkId::new("depth", depth),
            &(graph, deepest_id),
            |b, (graph, id)| {
                b.iter(|| black_box(graph.ancestors(id).len()));
            },
        );
    }

    group.finish();
}

fn bench_qualified_name(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/qualified_name");

    for depth in [3, 5, 10, 20] {
        let (graph, deepest_id) = create_deep_chain(depth);

        group.bench_with_input(
            BenchmarkId::new("depth", depth),
            &(graph, deepest_id),
            |b, (graph, id)| {
                b.iter(|| black_box(graph.build_qualified_name(id)));
            },
        );
    }

    group.finish();
}

// =============================================================================
// Relationship Operations
// =============================================================================

fn bench_add_relationship(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/add_relationship");

    for size in [100, 1000, 5000] {
        let graph = create_graph_with_relationships(size, 0);
        let ids: Vec<_> = graph.elements.keys().cloned().collect();

        group.bench_with_input(
            BenchmarkId::new("graph_size", size),
            &(graph, ids),
            |b, (graph, ids)| {
                b.iter_batched(
                    || {
                        let g = graph.clone();
                        let rel = Relationship {
                            id: ElementId::new_v4(),
                            kind: RelationshipKind::Specialize,
                            source: ids[0].clone(),
                            target: ids[1].clone(),
                            props: Default::default(),
                        };
                        (g, rel)
                    },
                    |(mut g, rel)| black_box(g.add_relationship(rel)),
                    criterion::BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_outgoing_relationships(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/outgoing_rels");

    for rels_per_element in [1, 5, 10, 20] {
        let graph = create_graph_with_relationships(100, rels_per_element);
        let source_id = graph.elements.keys().next().unwrap().clone();

        group.bench_with_input(
            BenchmarkId::new("rels_per_element", rels_per_element),
            &(graph, source_id),
            |b, (graph, source)| {
                b.iter(|| black_box(graph.outgoing(source).count()));
            },
        );
    }

    group.finish();
}

// =============================================================================
// Validation Operations
// =============================================================================

fn bench_validate_structure(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/validate_structure");

    for size in [100, 1000, 5000, 10000] {
        let (graph, _) = create_test_graph(size, 10);

        group.bench_with_input(
            BenchmarkId::new("element_count", size),
            &graph,
            |b, graph| {
                b.iter(|| black_box(graph.validate_structure()));
            },
        );
    }

    group.finish();
}

// =============================================================================
// Graph Clone (Memory operations)
// =============================================================================

fn bench_graph_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/graph_clone");

    for size in [100, 1000, 5000] {
        let (graph, _) = create_test_graph(size, 10);

        group.bench_with_input(
            BenchmarkId::new("element_count", size),
            &graph,
            |b, graph| {
                b.iter(|| black_box(graph.clone()));
            },
        );
    }

    group.finish();
}

// =============================================================================
// Roots and Filtering
// =============================================================================

fn bench_roots(c: &mut Criterion) {
    let mut group = c.benchmark_group("core/roots");

    for (roots, children) in [(10, 100), (50, 50), (100, 10)] {
        let (graph, _) = create_wide_graph(roots, children);

        group.bench_with_input(
            BenchmarkId::new("roots_x_children", format!("{}x{}", roots, children)),
            &graph,
            |b, graph| {
                b.iter(|| black_box(graph.roots().count()));
            },
        );
    }

    group.finish();
}

// =============================================================================
// Benchmark Groups
// =============================================================================

criterion_group!(
    element_benches,
    bench_add_element,
    bench_element_lookup,
    bench_children_lookup,
    bench_elements_by_kind,
);

criterion_group!(index_benches, bench_rebuild_indexes,);

criterion_group!(ancestry_benches, bench_ancestry_chain, bench_qualified_name,);

criterion_group!(
    relationship_benches,
    bench_add_relationship,
    bench_outgoing_relationships,
);

criterion_group!(validation_benches, bench_validate_structure,);

criterion_group!(misc_benches, bench_graph_clone, bench_roots,);

criterion_main!(
    element_benches,
    index_benches,
    ancestry_benches,
    relationship_benches,
    validation_benches,
    misc_benches,
);
