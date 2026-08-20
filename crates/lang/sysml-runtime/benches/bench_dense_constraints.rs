//! Benchmark: dense constraint evaluation (~100 constraints per tick).
//!
//! Synthesizes a constraint set of 100 arithmetic/comparison constraints
//! and evaluates them all against a populated EvalContext each iteration.
//! Measures the per-tick overhead of large constraint workloads.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sysml_core::Value;
use sysml_runtime::constraints::{precompile_constraint_set, ConstraintSet, EvalContext};
use sysml_runtime::ConstraintIR;

/// Build a constraint set with `n` constraints of the form:
///   x_i < threshold_i   (comparison)
///   x_i + x_j > 0       (arithmetic + comparison)
///   x_i * 2.0 <= limit   (mixed)
fn build_dense_constraint_set(n: usize) -> (ConstraintSet, EvalContext) {
    let mut set = ConstraintSet::new();
    let mut ctx = EvalContext::new();

    for i in 0..n {
        let var_name = format!("x_{i}");
        let threshold_name = format!("threshold_{i}");

        // Seed context with values
        ctx.set(var_name.clone(), Value::Float(i as f64 * 0.1));
        ctx.set(threshold_name.clone(), Value::Float(100.0 + i as f64));

        // Vary constraint shapes to exercise different ExprIR paths
        let expr = match i % 4 {
            0 => format!("{var_name} < {threshold_name}"),
            1 => {
                let j = (i + 1) % n;
                format!("{var_name} + x_{j} > 0")
            }
            2 => format!("{var_name} * 2.0 <= {threshold_name}"),
            _ => {
                let j = (i + 1) % n;
                format!("{var_name} >= 0 and x_{j} < {threshold_name}")
            }
        };

        set.add(ConstraintIR::new(expr));
    }

    (set, ctx)
}

fn bench_dense_constraints_100(c: &mut Criterion) {
    let (set, ctx) = build_dense_constraint_set(100);
    let precompiled = precompile_constraint_set(&set);

    assert!(
        precompiled.compiled_count() >= 90,
        "expected >= 90 compiled constraints, got {} (failed: {})",
        precompiled.compiled_count(),
        precompiled.failed_count()
    );

    let mut group = c.benchmark_group("runtime/dense_constraints");

    group.bench_function("evaluate_100_constraints", |b| {
        b.iter(|| {
            black_box(precompiled.evaluate_all(&ctx));
        });
    });

    // Also benchmark compilation + evaluation together (cold path)
    group.bench_function("compile_and_evaluate_100", |b| {
        b.iter(|| {
            let compiled = precompile_constraint_set(&set);
            black_box(compiled.evaluate_all(&ctx));
        });
    });

    group.finish();
}

fn bench_dense_constraints_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("runtime/dense_constraints_scaling");

    for n in [10, 50, 100] {
        let (set, ctx) = build_dense_constraint_set(n);
        let precompiled = precompile_constraint_set(&set);

        group.bench_function(format!("evaluate_{n}"), |b| {
            b.iter(|| {
                black_box(precompiled.evaluate_all(&ctx));
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_dense_constraints_100,
    bench_dense_constraints_scaling
);
criterion_main!(benches);
