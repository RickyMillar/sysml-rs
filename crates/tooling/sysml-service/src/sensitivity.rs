//! # sensitivity — Morris / Sobol sample generation + index computation (R7.4)
//!
//! This module is the backend engine for the "Sensitivity" workflow, a
//! sibling to `/analyze/sweep` and `/analyze/monte_carlo`. Callers drive
//! it in two phases:
//!
//!   1. **Sample generation.** [`morris_trajectories`] or
//!      [`sobol_sample_matrix`] turn a slice of [`ParamRange`]s into the
//!      children_params payload consumed by `sysml.batch.create { kind:
//!      "sensitivity", ... }`. The shape is a flat `Vec<Vec<f64>>` of
//!      design points where column `i` maps to `params[i].name`; the
//!      [`to_children_params`] helper converts that to the
//!      `[{ name: value, ... }, ...]` JSON the batch service consumes.
//!
//!   2. **Post-processing.** Once the batch completes, the caller pulls
//!      the scalar output metric for each child (verdict-derived or
//!      variable-derived — the service command decides) into a
//!      `Vec<f64>` in the same row order. [`compute_morris_indices`] or
//!      [`compute_sobol_indices`] turn that into per-parameter
//!      [`SensitivityResult`]s.
//!
//! ## Sampling methods
//!
//! **Morris Elementary Effects.** Each trajectory has `d + 1` points;
//! consecutive points differ in exactly one parameter, perturbed by
//! `Δ = p / (2·(p-1))` in normalised \[0, 1\] coordinates (Morris 1991,
//! Campolongo 2007). `r` trajectories × (d+1) points = `r·(d+1)` total
//! runs. The elementary effect of parameter `i` on trajectory `t` is
//! `EE_i^t = (f(x_after) - f(x_before)) / Δ`, where `x_before` and
//! `x_after` are the two consecutive points that differ in `i`. We
//! report the mean of absolute EEs (μ*, Campolongo's variant) as `mu`
//! and the plain std-dev as `sigma`. Large μ* = important; large σ =
//! nonlinear / interacting.
//!
//! **Sobol indices (Saltelli 2002).** Two base matrices `A`, `B` of
//! size `N × d` plus `d` "mix" matrices `C_i` where column `i` of `C_i`
//! comes from `B` and the rest from `A`. Total run count `N·(d+2)`.
//! Indices are estimated from `y_A = f(A)`, `y_B = f(B)`, and
//! `y_C_i = f(C_i)`:
//!
//! ```text
//!   f_0^2 = (1/N) Σ y_A_j · y_B_j              (approximation of E[f]^2)
//!   Var   = (1/N) Σ y_A_j^2 - f_0^2
//!   S_i   = ( (1/N) Σ y_B_j · (y_C_i_j - y_A_j) ) / Var     # first-order
//!   S_Ti  = 1 - ( (1/N) Σ y_A_j · (y_C_i_j - y_A_j) ) / Var # total-order,
//!                                                              equiv. to
//!         = ( (1/N) Σ (y_A_j - y_C_i_j)^2 / 2 ) / Var
//! ```
//!
//! We use the Jansen 1999 total-order estimator (second form) because
//! it's unbiased and numerically well-behaved. First-order uses
//! Saltelli's B-vs-C estimator.
//!
//! ## RNG
//!
//! Deterministic, seeded [`LcgRng`] — a splitmix64-style generator
//! adequate for sample-set generation (uniform in \[0, 1\]). Not
//! suitable for cryptographic use. Seed-stable across calls so fixture
//! tests can pin expected rows exactly.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ParamRange
// ---------------------------------------------------------------------------

/// One swept parameter's min / max for the sensitivity sampler.
///
/// The sampler works in normalised \[0, 1\] coordinates then unmaps to
/// \[`min`, `max`\] for the actual override value the model sees. `name`
/// is the parameter name the batch service hands to
/// `sysml_runtime::compiler::apply_overrides` (same convention as
/// sweep / monte carlo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamRange {
    /// Parameter name (as it appears on an AttributeUsage).
    pub name: String,
    /// Lower bound of the range, inclusive.
    pub min: f64,
    /// Upper bound of the range, inclusive.
    pub max: f64,
}

impl ParamRange {
    /// Map a normalised \[0, 1\] value to \[min, max\].
    pub fn unmap(&self, u: f64) -> f64 {
        self.min + (self.max - self.min) * u
    }
}

// ---------------------------------------------------------------------------
// Sensitivity method + result shapes (wire types)
// ---------------------------------------------------------------------------

/// Which sensitivity method to run. `serde(rename_all = "snake_case")`
/// so wire labels are `"morris"` / `"sobol"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitivityMethod {
    /// Morris Elementary Effects screening.
    Morris,
    /// Sobol variance-based indices (Saltelli 2002).
    Sobol,
}

impl SensitivityMethod {
    /// Machine-readable label (same as the `serde(rename_all)` wire
    /// format).
    pub fn as_str(self) -> &'static str {
        match self {
            SensitivityMethod::Morris => "morris",
            SensitivityMethod::Sobol => "sobol",
        }
    }

    /// Parse a lowercase-snake wire label back into the enum.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "morris" => Some(SensitivityMethod::Morris),
            "sobol" => Some(SensitivityMethod::Sobol),
            _ => None,
        }
    }
}

/// Per-parameter sensitivity result returned by
/// `sysml.sensitivity.analyze`.
///
/// Shape is deliberately sparse — Morris reports `mu` + `sigma`, Sobol
/// reports `s1` + `st`. The frontend picks which fields to render
/// based on the top-level `method` field on the response envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensitivityResult {
    /// Parameter name (matches [`ParamRange::name`]).
    pub name: String,
    /// Mean of absolute elementary effects (μ*, Morris-Campolongo).
    /// Populated for [`SensitivityMethod::Morris`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mu: Option<f64>,
    /// Standard deviation of elementary effects (σ, Morris).
    /// Populated for [`SensitivityMethod::Morris`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sigma: Option<f64>,
    /// First-order Sobol index S_i.
    /// Populated for [`SensitivityMethod::Sobol`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s1: Option<f64>,
    /// Total-order Sobol index S_Ti.
    /// Populated for [`SensitivityMethod::Sobol`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub st: Option<f64>,
}

// ---------------------------------------------------------------------------
// RNG — deterministic, seeded
// ---------------------------------------------------------------------------

/// Simple seeded splitmix64-style RNG. Portable, deterministic, and
/// fixture-stable. Not cryptographically secure; fine for sampling
/// design matrices where reproducibility is the only requirement.
#[derive(Debug, Clone)]
struct LcgRng {
    state: u64,
}

impl LcgRng {
    fn new(seed: u64) -> Self {
        // Avoid state == 0 which would lock a purely-multiplicative
        // generator; splitmix64 is fine at zero but we bias towards the
        // "fresh" range either way.
        let seed = seed.wrapping_add(0x9e3779b97f4a7c15);
        Self { state: seed }
    }

    /// Next 64-bit random integer (splitmix64).
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }

    /// Uniform \[0, 1) double.
    fn next_unit(&mut self) -> f64 {
        // Top 53 bits → divide by 2^53. Matches numpy's default
        // Uniform(0,1) precision exactly.
        let v = self.next_u64() >> 11;
        (v as f64) / ((1u64 << 53) as f64)
    }

    /// Uniform integer in \[0, n).
    fn gen_range(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        (self.next_u64() as usize) % n
    }
}

// ---------------------------------------------------------------------------
// Morris trajectories
// ---------------------------------------------------------------------------

/// Generate `r` Morris trajectories through `d`-dimensional parameter
/// space. Returns `r * (d + 1)` rows; each row is a length-`d` vector
/// of parameter values in their original (unmapped) ranges.
///
/// Row order is trajectory-major: trajectory 0 points 0..=d, then
/// trajectory 1, etc. Within a trajectory, consecutive points differ in
/// exactly one parameter (this is the invariant Morris EE analysis
/// relies on).
///
/// `p` is the level count — the normalised coordinate axis is
/// discretised to `p` evenly spaced levels `{0, 1/(p-1), ..., 1}`. A
/// typical choice is `p = 4` with `Δ = p / (2(p-1)) = 2/3`.
pub fn morris_trajectories(
    params: &[ParamRange],
    r: usize,
    p: usize,
    rng_seed: u64,
) -> Vec<Vec<f64>> {
    let d = params.len();
    assert!(p >= 2, "morris requires p >= 2 levels");
    assert!(d > 0, "morris requires at least one parameter");

    // Grid step in normalised space: Δ = p / (2(p-1)).
    let delta_norm = (p as f64) / (2.0 * ((p - 1) as f64));
    // Levels the base point can start at: 0, 1/(p-1), ..., (p/2 - 1)/(p-1)
    // so adding Δ stays within [0, 1].
    let half = p / 2; // floor division; p=4 → 2 → base levels 0, 1/3
    let base_levels: Vec<f64> = (0..half)
        .map(|k| (k as f64) / ((p - 1) as f64))
        .collect();

    let mut rng = LcgRng::new(rng_seed);
    let mut out: Vec<Vec<f64>> = Vec::with_capacity(r * (d + 1));

    for _ in 0..r {
        // Base point: random level per parameter in the lower half.
        let mut x_norm: Vec<f64> = (0..d)
            .map(|_| {
                let idx = rng.gen_range(base_levels.len());
                base_levels[idx]
            })
            .collect();

        // Random permutation of the d parameters — Fisher-Yates.
        let mut perm: Vec<usize> = (0..d).collect();
        for i in (1..d).rev() {
            let j = rng.gen_range(i + 1);
            perm.swap(i, j);
        }

        // Emit the base point first.
        out.push(unmap_row(params, &x_norm));

        // Walk the permutation; at each step perturb one parameter by
        // Δ. Direction is always + (levels were chosen so this stays in
        // range); a ± variant would add a Bernoulli here.
        for &pi in perm.iter() {
            x_norm[pi] = (x_norm[pi] + delta_norm).min(1.0);
            out.push(unmap_row(params, &x_norm));
        }
    }

    out
}

fn unmap_row(params: &[ParamRange], norm: &[f64]) -> Vec<f64> {
    params
        .iter()
        .zip(norm.iter())
        .map(|(pr, &u)| pr.unmap(u))
        .collect()
}

// ---------------------------------------------------------------------------
// Sobol sample matrix (Saltelli 2002)
// ---------------------------------------------------------------------------

/// Generate the Sobol design: two independent base matrices `A`, `B`
/// (each `N × d`) plus `d` "mix" matrices `C_i` where column `i` of
/// `C_i` comes from `B` and every other column from `A`.
///
/// Total rows if concatenated as `[A; B; C_0; C_1; ...; C_{d-1}]`:
/// `N · (d + 2)`.
///
/// Returns `(A, B, C_by_param)` where `C_by_param.len() == d`. All
/// values are already unmapped to the original `params[i].{min,max}`
/// range.
pub fn sobol_sample_matrix(
    params: &[ParamRange],
    n: usize,
    rng_seed: u64,
) -> (Vec<Vec<f64>>, Vec<Vec<f64>>, Vec<Vec<Vec<f64>>>) {
    let d = params.len();
    assert!(d > 0, "sobol requires at least one parameter");
    assert!(n > 0, "sobol requires N > 0");

    // Independent streams for A and B so they're truly decorrelated —
    // Saltelli 2002 requires this for unbiased estimators.
    let mut rng_a = LcgRng::new(rng_seed);
    let mut rng_b = LcgRng::new(rng_seed ^ 0xdead_beef_cafef00d);

    // Draw A and B in normalised space then unmap.
    let mut a_norm: Vec<Vec<f64>> = Vec::with_capacity(n);
    let mut b_norm: Vec<Vec<f64>> = Vec::with_capacity(n);
    for _ in 0..n {
        let row_a: Vec<f64> = (0..d).map(|_| rng_a.next_unit()).collect();
        let row_b: Vec<f64> = (0..d).map(|_| rng_b.next_unit()).collect();
        a_norm.push(row_a);
        b_norm.push(row_b);
    }

    let a: Vec<Vec<f64>> = a_norm
        .iter()
        .map(|r| unmap_row(params, r))
        .collect();
    let b: Vec<Vec<f64>> = b_norm
        .iter()
        .map(|r| unmap_row(params, r))
        .collect();

    // C_i: copy of A with column i replaced by column i of B.
    let mut c_by_param: Vec<Vec<Vec<f64>>> = Vec::with_capacity(d);
    for i in 0..d {
        let mut ci: Vec<Vec<f64>> = Vec::with_capacity(n);
        for j in 0..n {
            let mut row = a_norm[j].clone();
            row[i] = b_norm[j][i];
            ci.push(unmap_row(params, &row));
        }
        c_by_param.push(ci);
    }

    (a, b, c_by_param)
}

/// Concatenate a Sobol `(A, B, C_i...)` triple into the single
/// children-params row order the batch service consumes:
/// `[A_0, A_1, ..., A_{N-1}, B_0, ..., B_{N-1}, C_0_0, ..., C_{d-1}_{N-1}]`.
pub fn sobol_concat(
    a: &[Vec<f64>],
    b: &[Vec<f64>],
    c_by_param: &[Vec<Vec<f64>>],
) -> Vec<Vec<f64>> {
    let mut out: Vec<Vec<f64>> = Vec::with_capacity(
        a.len() + b.len() + c_by_param.iter().map(|c| c.len()).sum::<usize>(),
    );
    out.extend(a.iter().cloned());
    out.extend(b.iter().cloned());
    for ci in c_by_param {
        out.extend(ci.iter().cloned());
    }
    out
}

/// Convert a row of parameter values into the `{name: value, ...}` JSON
/// map expected by `sysml.batch.create { children_params }`.
pub fn to_children_params(
    params: &[ParamRange],
    rows: &[Vec<f64>],
) -> Vec<std::collections::BTreeMap<String, serde_json::Value>> {
    rows.iter()
        .map(|row| {
            let mut map = std::collections::BTreeMap::new();
            for (pr, &v) in params.iter().zip(row.iter()) {
                map.insert(pr.name.clone(), serde_json::json!(v));
            }
            map
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Morris index computation
// ---------------------------------------------------------------------------

/// Compute per-parameter Morris indices from a trajectory row order
/// (same as [`morris_trajectories`]) and the scalar output metric.
///
/// - `y.len()` must equal `r * (d + 1)` where `r = y.len() / (d + 1)`.
///   The caller decides what "y" is — verdict margin, a variable final
///   value, a KPI, whatever maps the child to a real number.
/// - Elementary effect per trajectory `t` and parameter `i`:
///   `EE = (y[t, after] - y[t, before]) / (Δ · (max - min))`, where
///   `before` / `after` are the two adjacent rows in trajectory `t`
///   that differ in parameter `i`. We walk each trajectory and
///   diff-detect which column moved; this avoids having to re-derive
///   the permutation from the samples.
/// - Returns `μ*` = mean(|EE|) and `σ` = stddev(EE).
///
/// The Morris-Campolongo μ* (not the raw μ) is the standard modern
/// screening statistic — it handles negative / cancelling effects
/// correctly, where plain μ can misleadingly average to zero.
pub fn compute_morris_indices(
    params: &[ParamRange],
    trajectory_rows: &[Vec<f64>],
    y: &[f64],
    p: usize,
) -> Vec<SensitivityResult> {
    let d = params.len();
    let per_traj = d + 1;
    assert!(p >= 2, "morris requires p >= 2 levels");
    assert_eq!(
        trajectory_rows.len() % per_traj,
        0,
        "trajectory row count must be divisible by (d+1)"
    );
    assert_eq!(trajectory_rows.len(), y.len(), "y must cover every row");
    let r = trajectory_rows.len() / per_traj;

    let delta_norm = (p as f64) / (2.0 * ((p - 1) as f64));

    // Collect EEs per parameter.
    let mut ee_by_param: Vec<Vec<f64>> = vec![Vec::new(); d];

    for t in 0..r {
        let base = t * per_traj;
        for step in 0..d {
            let i0 = base + step;
            let i1 = base + step + 1;
            // Identify which parameter moved by max-abs difference.
            let mut changed_idx = 0usize;
            let mut max_diff = 0.0f64;
            for k in 0..d {
                let diff = (trajectory_rows[i1][k] - trajectory_rows[i0][k]).abs();
                if diff > max_diff {
                    max_diff = diff;
                    changed_idx = k;
                }
            }
            if max_diff == 0.0 {
                continue; // degenerate (saturated) step
            }
            // Elementary effect in normalised space.
            let range = params[changed_idx].max - params[changed_idx].min;
            if range == 0.0 {
                continue;
            }
            let ee = (y[i1] - y[i0]) / (delta_norm * range);
            ee_by_param[changed_idx].push(ee);
        }
    }

    params
        .iter()
        .enumerate()
        .map(|(i, pr)| {
            let ees = &ee_by_param[i];
            let mu = mean_abs(ees);
            let sigma = stddev(ees);
            SensitivityResult {
                name: pr.name.clone(),
                mu: Some(mu),
                sigma: Some(sigma),
                s1: None,
                st: None,
            }
        })
        .collect()
}

fn mean_abs(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    let sum: f64 = xs.iter().map(|v| v.abs()).sum();
    sum / (xs.len() as f64)
}

fn mean(xs: &[f64]) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.iter().sum::<f64>() / (xs.len() as f64)
}

fn stddev(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let m = mean(xs);
    let var =
        xs.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (xs.len() as f64);
    var.sqrt()
}

// ---------------------------------------------------------------------------
// Sobol index computation (Saltelli 2002 / Jansen 1999)
// ---------------------------------------------------------------------------

/// Compute first-order (`S_i`) and total-order (`S_Ti`) Sobol indices
/// from the three sample blocks:
///
/// - `y_a`: outputs of the `A` matrix (length `N`)
/// - `y_b`: outputs of the `B` matrix (length `N`)
/// - `y_c_by_param`: `d` output vectors, each length `N`, for the `C_i`
///   matrices in parameter order.
///
/// Estimators:
///
/// ```text
///   Var      = (1/N) Σ y_A^2 - f_0^2
///   f_0^2    = ( (1/N) Σ y_A )^2      (mean-square approximation)
///   S_i      = [ (1/N) Σ y_B · (y_C_i - y_A) ] / Var          (Saltelli 2002)
///   S_Ti     = [ (1/(2N)) Σ (y_A - y_C_i)^2 ] / Var            (Jansen 1999)
/// ```
///
/// We use `f_0^2 = (mean(y_A))^2` rather than the original
/// `(1/N) Σ y_A · y_B` form — it's equivalent in expectation but less
/// noisy at small N and matches modern references (Saltelli 2010).
pub fn compute_sobol_indices(
    params: &[ParamRange],
    y_a: &[f64],
    y_b: &[f64],
    y_c_by_param: &[Vec<f64>],
) -> Vec<SensitivityResult> {
    let d = params.len();
    let n = y_a.len();
    assert_eq!(y_b.len(), n, "y_b length mismatch");
    assert_eq!(y_c_by_param.len(), d, "y_c must have one block per parameter");
    for ci in y_c_by_param {
        assert_eq!(ci.len(), n, "y_c[i] length mismatch");
    }
    assert!(n > 0, "sobol requires N > 0");

    // Combined sample used for variance estimate (Saltelli 2010 recommends
    // using A ∪ B which is 2·N samples and cuts variance roughly in half).
    let mut combined: Vec<f64> = Vec::with_capacity(2 * n);
    combined.extend_from_slice(y_a);
    combined.extend_from_slice(y_b);
    let mean_y = mean(&combined);
    let var_y: f64 = combined
        .iter()
        .map(|v| (v - mean_y).powi(2))
        .sum::<f64>()
        / (combined.len() as f64);

    if var_y == 0.0 || !var_y.is_finite() {
        // Degenerate constant output — every index is zero by
        // convention (no variance to decompose).
        return params
            .iter()
            .map(|pr| SensitivityResult {
                name: pr.name.clone(),
                mu: None,
                sigma: None,
                s1: Some(0.0),
                st: Some(0.0),
            })
            .collect();
    }

    params
        .iter()
        .enumerate()
        .map(|(i, pr)| {
            let y_ci = &y_c_by_param[i];

            // First-order (Saltelli 2002 estimator):
            //   S_i = (1/N) Σ y_B · (y_C_i - y_A) / Var
            let s1_num: f64 = (0..n)
                .map(|j| y_b[j] * (y_ci[j] - y_a[j]))
                .sum::<f64>()
                / (n as f64);
            let s1 = s1_num / var_y;

            // Total-order (Jansen 1999 estimator):
            //   S_Ti = (1/(2N)) Σ (y_A - y_C_i)^2 / Var
            let st_num: f64 = (0..n)
                .map(|j| (y_a[j] - y_ci[j]).powi(2))
                .sum::<f64>()
                / (2.0 * (n as f64));
            let st = st_num / var_y;

            SensitivityResult {
                name: pr.name.clone(),
                mu: None,
                sigma: None,
                s1: Some(s1),
                st: Some(st),
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn ranges_3() -> Vec<ParamRange> {
        vec![
            ParamRange { name: "a".into(), min: 0.0, max: 1.0 },
            ParamRange { name: "b".into(), min: 0.0, max: 1.0 },
            ParamRange { name: "c".into(), min: 0.0, max: 1.0 },
        ]
    }

    // -- Method enum wire format ----------------------------------------

    #[test]
    fn method_wire_labels_are_snake_case() {
        assert_eq!(
            serde_json::to_value(SensitivityMethod::Morris).unwrap(),
            serde_json::json!("morris"),
        );
        assert_eq!(
            serde_json::to_value(SensitivityMethod::Sobol).unwrap(),
            serde_json::json!("sobol"),
        );
        assert_eq!(SensitivityMethod::from_str("morris"), Some(SensitivityMethod::Morris));
        assert_eq!(SensitivityMethod::from_str("sobol"), Some(SensitivityMethod::Sobol));
        assert_eq!(SensitivityMethod::from_str("Morris"), None);
        assert!(SensitivityMethod::from_str("gibberish").is_none());
    }

    // -- Morris sampling -------------------------------------------------

    #[test]
    fn morris_trajectories_has_r_times_d_plus_one_rows() {
        let params = ranges_3();
        let r = 5;
        let p = 4;
        let rows = morris_trajectories(&params, r, p, 42);
        assert_eq!(rows.len(), r * (params.len() + 1));
        for row in &rows {
            assert_eq!(row.len(), params.len());
        }
    }

    #[test]
    fn morris_consecutive_points_differ_in_exactly_one_parameter() {
        let params = ranges_3();
        let r = 4;
        let p = 4;
        let d = params.len();
        let rows = morris_trajectories(&params, r, p, 1234);

        for t in 0..r {
            let base = t * (d + 1);
            for step in 0..d {
                let a = &rows[base + step];
                let b = &rows[base + step + 1];
                let mut different = 0;
                for k in 0..d {
                    if (a[k] - b[k]).abs() > 1e-12 {
                        different += 1;
                    }
                }
                assert_eq!(
                    different, 1,
                    "trajectory {t} step {step}: expected exactly 1 changed parameter"
                );
            }
        }
    }

    #[test]
    fn morris_deterministic_under_seed() {
        let params = ranges_3();
        let a = morris_trajectories(&params, 4, 4, 7);
        let b = morris_trajectories(&params, 4, 4, 7);
        assert_eq!(a, b);
        let c = morris_trajectories(&params, 4, 4, 8);
        assert_ne!(a, c);
    }

    #[test]
    fn morris_respects_range_bounds() {
        let params = vec![
            ParamRange { name: "x".into(), min: -2.0, max: 10.0 },
            ParamRange { name: "y".into(), min: 1.0, max: 3.0 },
        ];
        let rows = morris_trajectories(&params, 3, 4, 99);
        for row in &rows {
            assert!(row[0] >= -2.0 - 1e-9 && row[0] <= 10.0 + 1e-9);
            assert!(row[1] >= 1.0 - 1e-9 && row[1] <= 3.0 + 1e-9);
        }
    }

    // -- Sobol sampling --------------------------------------------------

    #[test]
    fn sobol_matrices_have_correct_shapes() {
        let params = ranges_3();
        let n = 16;
        let d = params.len();
        let (a, b, c) = sobol_sample_matrix(&params, n, 42);
        assert_eq!(a.len(), n);
        assert_eq!(b.len(), n);
        assert_eq!(c.len(), d);
        for ci in &c {
            assert_eq!(ci.len(), n);
        }
        for row in &a {
            assert_eq!(row.len(), d);
        }
    }

    #[test]
    fn sobol_c_i_mixes_column_i_from_b() {
        // In each C_i, column i must equal B's column i; every other
        // column must equal A's. That's the Saltelli mixing invariant.
        let params = ranges_3();
        let n = 8;
        let (a, b, c) = sobol_sample_matrix(&params, n, 42);
        let d = params.len();
        for i in 0..d {
            for j in 0..n {
                for k in 0..d {
                    let got = c[i][j][k];
                    let expected = if k == i { b[j][k] } else { a[j][k] };
                    assert!(
                        (got - expected).abs() < 1e-12,
                        "c[{i}][{j}][{k}] = {got}, expected {expected}"
                    );
                }
            }
        }
    }

    #[test]
    fn sobol_deterministic_under_seed() {
        let params = ranges_3();
        let (a1, b1, c1) = sobol_sample_matrix(&params, 8, 13);
        let (a2, b2, c2) = sobol_sample_matrix(&params, 8, 13);
        assert_eq!(a1, a2);
        assert_eq!(b1, b2);
        assert_eq!(c1, c2);
    }

    #[test]
    fn sobol_concat_uses_a_b_c_order() {
        let params = ranges_3();
        let n = 4;
        let d = params.len();
        let (a, b, c) = sobol_sample_matrix(&params, n, 1);
        let cat = sobol_concat(&a, &b, &c);
        assert_eq!(cat.len(), n + n + d * n);
        assert_eq!(cat[..n], a[..]);
        assert_eq!(cat[n..2 * n], b[..]);
    }

    // -- to_children_params ----------------------------------------------

    #[test]
    fn to_children_params_maps_values_by_name() {
        let params = vec![
            ParamRange { name: "x".into(), min: 0.0, max: 1.0 },
            ParamRange { name: "y".into(), min: 0.0, max: 1.0 },
        ];
        let rows = vec![vec![0.25, 0.75], vec![0.5, 0.1]];
        let out = to_children_params(&params, &rows);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["x"], serde_json::json!(0.25));
        assert_eq!(out[0]["y"], serde_json::json!(0.75));
        assert_eq!(out[1]["x"], serde_json::json!(0.5));
    }

    // -- Morris index computation ---------------------------------------

    #[test]
    fn morris_linear_function_has_mu_proportional_to_slope() {
        // f(x1, x2, x3) = 3·x1 + 1·x2 + 0·x3 with x in [0,1].
        // Δ (normalised) = p/(2(p-1)); real range = 1. So EE_i = slope_i.
        // μ* should be |slope|, σ ≈ 0 (linear → EEs constant).
        let params = ranges_3();
        let r = 10;
        let p = 4;
        let rows = morris_trajectories(&params, r, p, 42);
        let y: Vec<f64> = rows
            .iter()
            .map(|row| 3.0 * row[0] + 1.0 * row[1] + 0.0 * row[2])
            .collect();
        let out = compute_morris_indices(&params, &rows, &y, p);
        assert_eq!(out.len(), 3);

        let mu_a = out[0].mu.unwrap();
        let mu_b = out[1].mu.unwrap();
        let mu_c = out[2].mu.unwrap();
        assert!((mu_a - 3.0).abs() < 1e-9, "μ*(a) should be ~3.0, got {mu_a}");
        assert!((mu_b - 1.0).abs() < 1e-9, "μ*(b) should be ~1.0, got {mu_b}");
        assert!((mu_c - 0.0).abs() < 1e-9, "μ*(c) should be ~0.0, got {mu_c}");

        for r in &out {
            let sigma = r.sigma.unwrap();
            assert!(sigma < 1e-9, "linear model should produce σ ≈ 0, got {sigma}");
        }
    }

    // -- Sobol index computation on Ishigami --------------------------------

    /// Ishigami function — canonical Sobol-indices test problem.
    ///
    /// f(x1, x2, x3) = sin(x1) + a · sin(x2)^2 + b · x3^4 · sin(x1)
    /// with x_i ~ Uniform(-π, π), a = 7, b = 0.1.
    ///
    /// Analytical first-order indices (Saltelli et al. 2008):
    ///   S_1 ≈ 0.3139    S_2 ≈ 0.4424    S_3 = 0.0
    /// Total-order indices:
    ///   S_T1 ≈ 0.5576   S_T2 ≈ 0.4424   S_T3 ≈ 0.2437
    ///
    /// Note S_3 = 0 (x3 has no main effect) but S_T3 > 0 (x3 interacts
    /// with x1). Convergence is O(1/√N); a small-N estimate lands
    /// within a few tenths of each analytical value — we bound the test
    /// at ±0.15 for first-order and ±0.2 for total-order with
    /// N = 2048, which is textbook-comfortable.
    fn ishigami(x: &[f64]) -> f64 {
        let a = 7.0;
        let b = 0.1;
        x[0].sin() + a * x[1].sin().powi(2) + b * x[2].powi(4) * x[0].sin()
    }

    #[test]
    fn sobol_indices_converge_to_ishigami_analytical() {
        let pi = std::f64::consts::PI;
        let params = vec![
            ParamRange { name: "x1".into(), min: -pi, max: pi },
            ParamRange { name: "x2".into(), min: -pi, max: pi },
            ParamRange { name: "x3".into(), min: -pi, max: pi },
        ];
        let n = 2048;
        let (a, b, c) = sobol_sample_matrix(&params, n, 42);
        let y_a: Vec<f64> = a.iter().map(|r| ishigami(r)).collect();
        let y_b: Vec<f64> = b.iter().map(|r| ishigami(r)).collect();
        let y_c: Vec<Vec<f64>> = c
            .iter()
            .map(|ci| ci.iter().map(|r| ishigami(r)).collect())
            .collect();

        let out = compute_sobol_indices(&params, &y_a, &y_b, &y_c);
        assert_eq!(out.len(), 3);

        let s1 = [out[0].s1.unwrap(), out[1].s1.unwrap(), out[2].s1.unwrap()];
        let st = [out[0].st.unwrap(), out[1].st.unwrap(), out[2].st.unwrap()];

        // Analytical targets (Saltelli et al. 2008).
        let s1_ref = [0.3139, 0.4424, 0.0];
        let st_ref = [0.5576, 0.4424, 0.2437];

        // ±0.15 for S_i, ±0.2 for S_Ti at N = 2048. These bounds are
        // chosen to pin the test under the fixed rng seed but still
        // catch a real regression (swapping first- and total-order
        // estimators, for example, collapses the gap between S_1 and
        // S_T1, which would immediately fail).
        for i in 0..3 {
            assert!(
                (s1[i] - s1_ref[i]).abs() < 0.15,
                "S_{} = {} (ref {}); error {}",
                i + 1,
                s1[i],
                s1_ref[i],
                (s1[i] - s1_ref[i]).abs()
            );
            assert!(
                (st[i] - st_ref[i]).abs() < 0.2,
                "S_T{} = {} (ref {}); error {}",
                i + 1,
                st[i],
                st_ref[i],
                (st[i] - st_ref[i]).abs()
            );
        }

        // Ordering sanity: S_T > S for every parameter (interactions
        // can only add, never subtract).
        for i in 0..3 {
            assert!(
                st[i] >= s1[i] - 0.05,
                "S_T{} should be >= S_{} (within noise): {} vs {}",
                i + 1,
                i + 1,
                st[i],
                s1[i]
            );
        }

        // x3 has zero first-order effect by construction.
        assert!(s1[2].abs() < 0.15, "S_3 should be near zero, got {}", s1[2]);
        // x3 has real total-order effect through interaction with x1.
        assert!(st[2] > 0.05, "S_T3 should be materially > 0, got {}", st[2]);
    }

    #[test]
    fn sobol_constant_output_has_zero_indices() {
        // If the model ignores every input (constant output) variance
        // is zero; the helper returns S = S_T = 0 rather than NaN.
        let params = ranges_3();
        let n = 16;
        let y_a = vec![1.0; n];
        let y_b = vec![1.0; n];
        let y_c = vec![vec![1.0; n]; params.len()];
        let out = compute_sobol_indices(&params, &y_a, &y_b, &y_c);
        for r in out {
            assert_eq!(r.s1, Some(0.0));
            assert_eq!(r.st, Some(0.0));
        }
    }

    // -- Serde shapes ----------------------------------------------------

    #[test]
    fn sensitivity_result_skips_empty_optionals() {
        let morris_like = SensitivityResult {
            name: "a".into(),
            mu: Some(1.0),
            sigma: Some(0.1),
            s1: None,
            st: None,
        };
        let v = serde_json::to_value(&morris_like).unwrap();
        assert!(v.get("mu").is_some());
        assert!(v.get("sigma").is_some());
        assert!(v.get("s1").is_none(), "s1 should be skipped when None");
        assert!(v.get("st").is_none(), "st should be skipped when None");

        let sobol_like = SensitivityResult {
            name: "a".into(),
            mu: None,
            sigma: None,
            s1: Some(0.3),
            st: Some(0.5),
        };
        let v = serde_json::to_value(&sobol_like).unwrap();
        assert!(v.get("mu").is_none());
        assert!(v.get("sigma").is_none());
        assert!(v.get("s1").is_some());
        assert!(v.get("st").is_some());
    }
}
