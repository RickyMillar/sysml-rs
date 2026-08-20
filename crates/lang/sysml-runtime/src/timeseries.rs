//! Columnar time-series storage with LTTB decimation.
//!
//! Mirrors the TS `TimeSeriesBuffer` + `useTimeSeriesStore` contract so
//! the Rust side can own ingest and decimation for every sim consumer
//! (WebSocket stream, CLI, MCP). One fixed-capacity ring buffer per
//! variable; missing values are filled with `f64::NAN` so all columns
//! align with the timestamp ring.
//!

use std::collections::HashMap;

use crate::snapshot_view::NormalizedSnapshot;

/// ~32 MB default memory budget (was 100 MB per ADR-008; lowered
/// 2026-07-13 for the memory-tight demo host). NOTE: the effective
/// footprint is this budget × (1 + actual_series)/(1 + [`DEFAULT_ESTIMATED_SERIES`]),
/// so a model with more series than the estimate over-allocates
/// proportionally — see `from_budget`. For a ~40-series model this
/// budget yields ~125 MB/session; revisit if the estimate is made
/// series-aware.
const DEFAULT_MEMORY_BUDGET_BYTES: usize = 32 * 1024 * 1024;
const BYTES_PER_F64: usize = 8;
/// Default guess for how many variables will be tracked when callers
/// don't specify `capacity` explicitly.
const DEFAULT_ESTIMATED_SERIES: usize = 10;

/// Fixed-capacity ring buffer. Overwrites oldest entries when full.
#[derive(Debug, Clone)]
pub struct Ring<T> {
    buf: Vec<T>,
    capacity: usize,
    head: usize,
    count: usize,
}

impl<T: Copy + Default> Ring<T> {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Ring {
            buf: vec![T::default(); capacity],
            capacity,
            head: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, value: T) {
        let slot = self.head % self.capacity;
        self.buf[slot] = value;
        self.head = self.head.wrapping_add(1);
        if self.count < self.capacity {
            self.count += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Logical element at position `i` (0 = oldest). Returns `None` when out of bounds.
    pub fn at(&self, i: usize) -> Option<T> {
        if i >= self.count {
            return None;
        }
        if self.count < self.capacity {
            Some(self.buf[i])
        } else {
            let start = self.head % self.capacity;
            Some(self.buf[(start + i) % self.capacity])
        }
    }

    /// Materialize the buffer as a `Vec` in logical (oldest → newest) order.
    pub fn to_vec(&self) -> Vec<T> {
        let mut out = Vec::with_capacity(self.count);
        if self.count < self.capacity {
            out.extend_from_slice(&self.buf[..self.count]);
        } else {
            let start = self.head % self.capacity;
            out.extend_from_slice(&self.buf[start..]);
            out.extend_from_slice(&self.buf[..start]);
        }
        out
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.count = 0;
    }
}

/// Columnar ring buffer: one timestamp ring + one ring per variable.
#[derive(Debug, Clone)]
pub struct TimeSeriesBuffer {
    capacity: usize,
    timestamps: Ring<f64>,
    series: HashMap<String, Ring<f64>>,
    last_time_ms: Option<f64>,
}

impl TimeSeriesBuffer {
    /// Create with an explicit point capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        TimeSeriesBuffer {
            capacity,
            timestamps: Ring::new(capacity),
            series: HashMap::new(),
            last_time_ms: None,
        }
    }

    /// Create with a capacity derived from the default 100 MB budget and
    /// an estimate of how many series will be tracked.
    pub fn new() -> Self {
        Self::from_budget(DEFAULT_MEMORY_BUDGET_BYTES, DEFAULT_ESTIMATED_SERIES)
    }

    /// Derive capacity from `memory_budget_bytes` and `estimated_series` count.
    pub fn from_budget(memory_budget_bytes: usize, estimated_series: usize) -> Self {
        let estimated_series = estimated_series.max(1);
        let bytes_per_point = BYTES_PER_F64 * (1 + estimated_series);
        let capacity = (memory_budget_bytes / bytes_per_point).max(1);
        Self::with_capacity(capacity)
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.timestamps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.timestamps.is_empty()
    }

    pub fn series_names(&self) -> Vec<&str> {
        self.series.keys().map(String::as_str).collect()
    }

    /// Estimated memory footprint in bytes — 1 timestamp ring + 1 ring per series.
    pub fn memory_bytes(&self) -> usize {
        (1 + self.series.len()) * self.capacity * BYTES_PER_F64
    }

    /// Append a single tick. Missing variables get `NaN`; brand-new variables
    /// are back-filled with `NaN` for all prior ticks so columns stay aligned.
    pub fn append(&mut self, time_ms: f64, values: &HashMap<String, f64>) {
        self.timestamps.push(time_ms);
        self.last_time_ms = Some(time_ms);

        // Allocate rings for any new variable, back-filling with NaN for
        // the ticks that preceded its first appearance. `len()` already
        // counts *this* tick since we pushed the timestamp above.
        let prior_count = self.timestamps.len().saturating_sub(1);
        for key in values.keys() {
            if !self.series.contains_key(key) {
                let mut ring = Ring::<f64>::new(self.capacity);
                for _ in 0..prior_count {
                    ring.push(f64::NAN);
                }
                self.series.insert(key.clone(), ring);
            }
        }

        for (name, ring) in self.series.iter_mut() {
            let v = values.get(name).copied().unwrap_or(f64::NAN);
            ring.push(v);
        }
    }

    /// Append the scalar vars from a normalized snapshot. Returns `true` if
    /// the point was accepted, `false` if it was deduplicated (same or
    /// older `time_ms` than the last push).
    pub fn append_snapshot(&mut self, snap: &NormalizedSnapshot) -> bool {
        if let Some(prev) = self.last_time_ms {
            if snap.time_ms <= prev {
                return false;
            }
        }
        if !snap.time_ms.is_finite() {
            return false;
        }
        self.append(snap.time_ms, &snap.scalar_vars);
        true
    }

    /// Return the full (time, value) series for a variable in logical order.
    /// `NaN` entries are dropped so chart libraries don't have to skip them.
    pub fn series(&self, name: &str) -> Vec<(f64, f64)> {
        let Some(ring) = self.series.get(name) else {
            return Vec::new();
        };
        let ts = self.timestamps.to_vec();
        let vs = ring.to_vec();
        ts.into_iter()
            .zip(vs)
            .filter(|(_, v)| !v.is_nan())
            .collect()
    }

    /// Return `(time, value)` pairs for a variable, bounded inclusively by
    /// the given range. Pass `None`/`None` for an unbounded read.
    pub fn series_windowed(
        &self,
        name: &str,
        start_ms: Option<f64>,
        end_ms: Option<f64>,
    ) -> Vec<(f64, f64)> {
        self.series(name)
            .into_iter()
            .filter(|(t, _)| start_ms.map_or(true, |s| *t >= s))
            .filter(|(t, _)| end_ms.map_or(true, |e| *t <= e))
            .collect()
    }

    /// Most recent `(time_ms, value)` for one variable, skipping trailing
    /// `NaN` gaps, or `None` when the variable was never recorded or holds
    /// no finite sample.
    ///
    /// This is the "where did this variable end up" reading — the same
    /// series `sysml.sessions.timeseries` serves, taken at its last finite
    /// point — without materialising the whole series to look at its tail.
    /// A variable that exists but never produced a finite value returns
    /// `None` rather than `0.0`: absent and zero are different answers.
    pub fn last(&self, name: &str) -> Option<(f64, f64)> {
        let ring = self.series.get(name)?;
        let n = ring.len().min(self.timestamps.len());
        (0..n).rev().find_map(|i| {
            let v = ring.at(i)?;
            if v.is_nan() {
                return None;
            }
            Some((self.timestamps.at(i)?, v))
        })
    }

    /// Extract every series at once.
    pub fn snapshot_all(&self) -> HashMap<String, Vec<(f64, f64)>> {
        self.series
            .keys()
            .map(|k| (k.clone(), self.series(k)))
            .collect()
    }

    /// Clear all rings but retain capacity and variable set.
    pub fn clear(&mut self) {
        self.timestamps.clear();
        for ring in self.series.values_mut() {
            ring.clear();
        }
        self.last_time_ms = None;
    }

    /// Timestamp of the most recently pushed point, if any.
    pub fn last_time_ms(&self) -> Option<f64> {
        self.last_time_ms
    }
}

impl Default for TimeSeriesBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Largest-Triangle-Three-Buckets decimation.
///
/// Preserves visual extrema better than naive Nth-point sampling. Returns
/// the input unchanged when it already fits in `threshold` or the
/// threshold is below the minimum LTTB size of 3.
pub fn lttb(points: &[(f64, f64)], threshold: usize) -> Vec<(f64, f64)> {
    let len = points.len();
    if len <= threshold || threshold < 3 {
        return points.to_vec();
    }

    let mut out = Vec::with_capacity(threshold);
    out.push(points[0]);

    let bucket_size = (len - 2) as f64 / (threshold - 2) as f64;
    let mut prev_idx = 0usize;

    for i in 0..(threshold - 2) {
        let bucket_start = ((i as f64 + 1.0) * bucket_size).floor() as usize + 1;
        let bucket_end = (((i as f64 + 2.0) * bucket_size).floor() as usize + 1).min(len - 1);

        // Average point in the *next* bucket (used for triangle area).
        let next_start = ((i as f64 + 2.0) * bucket_size).floor() as usize + 1;
        let next_end = (((i as f64 + 3.0) * bucket_size).floor() as usize + 1).min(len - 1);
        let (avg_t, avg_v) = if next_end > next_start {
            let slice = &points[next_start..next_end];
            let n = slice.len() as f64;
            let sum_t: f64 = slice.iter().map(|(t, _)| *t).sum();
            let sum_v: f64 = slice.iter().map(|(_, v)| *v).sum();
            (sum_t / n, sum_v / n)
        } else {
            points[len - 1]
        };

        let (prev_t, prev_v) = points[prev_idx];
        let mut max_area = -1.0f64;
        let mut best_idx = bucket_start;
        for j in bucket_start..bucket_end {
            let (t, v) = points[j];
            let area = ((prev_t - avg_t) * (v - prev_v) - (prev_t - t) * (avg_v - prev_v)).abs();
            if area > max_area {
                max_area = area;
                best_idx = j;
            }
        }
        out.push(points[best_idx]);
        prev_idx = best_idx;
    }

    out.push(points[len - 1]);
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn build_values(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect()
    }

    #[test]
    fn ring_wraps_in_logical_order() {
        let mut r = Ring::<f64>::new(3);
        r.push(1.0);
        r.push(2.0);
        r.push(3.0);
        assert_eq!(r.to_vec(), vec![1.0, 2.0, 3.0]);
        r.push(4.0);
        r.push(5.0);
        assert_eq!(r.to_vec(), vec![3.0, 4.0, 5.0]);
        assert_eq!(r.at(0), Some(3.0));
        assert_eq!(r.at(2), Some(5.0));
        assert_eq!(r.at(3), None);
    }

    #[test]
    fn buffer_aligns_columns_and_backfills_new_series() {
        let mut buf = TimeSeriesBuffer::with_capacity(100);
        buf.append(0.0, &build_values(&[("a", 1.0)]));
        buf.append(10.0, &build_values(&[("a", 2.0)]));
        // new variable `b` appears on the 3rd tick — should back-fill NaN
        buf.append(20.0, &build_values(&[("a", 3.0), ("b", 30.0)]));

        let a = buf.series("a");
        assert_eq!(a, vec![(0.0, 1.0), (10.0, 2.0), (20.0, 3.0)]);
        // `b` only has a real value for tick 3 (the two NaNs are filtered out)
        let b = buf.series("b");
        assert_eq!(b, vec![(20.0, 30.0)]);
        assert_eq!(buf.len(), 3);
    }

    #[test]
    fn buffer_windowed_filter_includes_bounds() {
        let mut buf = TimeSeriesBuffer::with_capacity(10);
        for t in [0.0, 10.0, 20.0, 30.0, 40.0] {
            buf.append(t, &build_values(&[("x", t)]));
        }
        let w = buf.series_windowed("x", Some(10.0), Some(30.0));
        assert_eq!(w, vec![(10.0, 10.0), (20.0, 20.0), (30.0, 30.0)]);
        let open = buf.series_windowed("x", None, Some(15.0));
        assert_eq!(open, vec![(0.0, 0.0), (10.0, 10.0)]);
    }

    #[test]
    fn buffer_wraps_when_capacity_exceeded() {
        let mut buf = TimeSeriesBuffer::with_capacity(3);
        for t in 0..5 {
            buf.append(t as f64, &build_values(&[("a", t as f64)]));
        }
        // Oldest two entries were overwritten → we keep ticks 2, 3, 4.
        assert_eq!(buf.len(), 3);
        let a = buf.series("a");
        assert_eq!(a, vec![(2.0, 2.0), (3.0, 3.0), (4.0, 4.0)]);
    }

    #[test]
    fn append_snapshot_dedups_stale_time() {
        let mut buf = TimeSeriesBuffer::with_capacity(10);
        let mut snap = NormalizedSnapshot::default();
        snap.time_ms = 100.0;
        snap.scalar_vars.insert("x".into(), 1.0);
        assert!(buf.append_snapshot(&snap));
        assert!(
            !buf.append_snapshot(&snap),
            "same time_ms should be skipped"
        );
        snap.time_ms = 90.0;
        assert!(
            !buf.append_snapshot(&snap),
            "older time_ms should be skipped"
        );
        snap.time_ms = 200.0;
        snap.scalar_vars.insert("x".into(), 2.0);
        assert!(buf.append_snapshot(&snap));
        assert_eq!(buf.series("x"), vec![(100.0, 1.0), (200.0, 2.0)]);
    }

    #[test]
    fn append_snapshot_rejects_non_finite_time() {
        let mut buf = TimeSeriesBuffer::with_capacity(4);
        let mut snap = NormalizedSnapshot::default();
        snap.time_ms = f64::NAN;
        snap.scalar_vars.insert("x".into(), 1.0);
        assert!(!buf.append_snapshot(&snap));
        assert!(buf.is_empty());
    }

    #[test]
    fn clear_resets_rings_but_keeps_variable_set() {
        let mut buf = TimeSeriesBuffer::with_capacity(5);
        buf.append(0.0, &build_values(&[("x", 1.0), ("y", 2.0)]));
        buf.append(10.0, &build_values(&[("x", 3.0), ("y", 4.0)]));
        assert_eq!(buf.len(), 2);
        buf.clear();
        assert_eq!(buf.len(), 0);
        assert!(buf.last_time_ms().is_none());
        // variable set retained; series now empty
        let names = buf.series_names();
        assert!(names.contains(&"x") && names.contains(&"y"));
        assert!(buf.series("x").is_empty());
    }

    #[test]
    fn lttb_under_threshold_returns_input() {
        let pts = vec![(0.0, 0.0), (1.0, 1.0), (2.0, 2.0)];
        assert_eq!(lttb(&pts, 10), pts);
    }

    #[test]
    fn lttb_preserves_endpoints_and_count() {
        // 100 points on a sine-ish curve; decimate to 20 and verify bounds.
        let pts: Vec<(f64, f64)> = (0..100)
            .map(|i| (i as f64, (i as f64 * 0.1).sin()))
            .collect();
        let out = lttb(&pts, 20);
        assert_eq!(out.len(), 20);
        assert_eq!(out[0], pts[0]);
        assert_eq!(out[out.len() - 1], pts[pts.len() - 1]);
        // Output timestamps are monotonically non-decreasing.
        for w in out.windows(2) {
            assert!(w[0].0 <= w[1].0);
        }
    }

    #[test]
    fn lttb_preserves_visual_peak() {
        // Flat signal with one spike — a sensible decimator keeps the spike.
        let mut pts: Vec<(f64, f64)> = (0..50).map(|i| (i as f64, 0.0)).collect();
        pts[25] = (25.0, 100.0);
        let out = lttb(&pts, 10);
        assert!(
            out.iter().any(|(_, v)| (*v - 100.0).abs() < 1e-9),
            "expected LTTB output to retain the extreme point",
        );
    }

    #[test]
    fn lttb_handles_small_threshold() {
        let pts = vec![(0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (3.0, 3.0)];
        // threshold < 3 → pass-through
        assert_eq!(lttb(&pts, 2), pts);
    }

    #[test]
    fn memory_bytes_scales_with_series_count() {
        let mut buf = TimeSeriesBuffer::with_capacity(100);
        let before = buf.memory_bytes(); // only timestamp ring
        buf.append(0.0, &build_values(&[("a", 1.0), ("b", 2.0), ("c", 3.0)]));
        let after = buf.memory_bytes();
        assert!(after > before);
        assert_eq!(after, (1 + 3) * 100 * BYTES_PER_F64);
    }

    // -- last() ----------------------------------------------------------

    /// Helper: append one tick of named values.
    fn tick(buf: &mut TimeSeriesBuffer, t: f64, vals: &[(&str, f64)]) {
        let map: HashMap<String, f64> =
            vals.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
        buf.append(t, &map);
    }

    #[test]
    fn last_returns_the_most_recent_point() {
        let mut buf = TimeSeriesBuffer::new();
        tick(&mut buf, 0.0, &[("temperature", 1000.0)]);
        tick(&mut buf, 1.0, &[("temperature", 995.0)]);
        tick(&mut buf, 2.0, &[("temperature", 990.0)]);
        assert_eq!(buf.last("temperature"), Some((2.0, 990.0)));
    }

    #[test]
    fn last_agrees_with_the_tail_of_the_full_series() {
        // `last` exists to avoid materialising the series; it must not
        // disagree with the series it is shortcutting.
        let mut buf = TimeSeriesBuffer::new();
        for i in 0..50 {
            tick(&mut buf, i as f64, &[("x", (i * i) as f64)]);
        }
        assert_eq!(buf.last("x"), buf.series("x").last().copied());
    }

    #[test]
    fn last_is_none_for_a_variable_that_was_never_recorded() {
        let mut buf = TimeSeriesBuffer::new();
        tick(&mut buf, 0.0, &[("x", 1.0)]);
        assert_eq!(buf.last("nope"), None);
    }

    #[test]
    fn last_is_none_for_an_empty_buffer() {
        assert_eq!(TimeSeriesBuffer::new().last("x"), None);
    }

    #[test]
    fn last_skips_trailing_nan_gaps_rather_than_reporting_them() {
        // A NaN is "no sample at this tick", not a value. Reporting it would
        // hand a caller something that fails every finiteness check anyway,
        // and hide a perfectly good earlier reading.
        let mut buf = TimeSeriesBuffer::new();
        tick(&mut buf, 0.0, &[("x", 5.0)]);
        tick(&mut buf, 1.0, &[("x", f64::NAN)]);
        tick(&mut buf, 2.0, &[("x", f64::NAN)]);
        assert_eq!(buf.last("x"), Some((0.0, 5.0)));
    }

    #[test]
    fn last_is_none_when_a_variable_only_ever_held_nan() {
        // Distinguishable from `Some(0.0)` on purpose: absent and zero are
        // different answers, and a sweep outcome must not conflate them.
        let mut buf = TimeSeriesBuffer::new();
        tick(&mut buf, 0.0, &[("x", f64::NAN)]);
        assert_eq!(buf.last("x"), None);
    }

    #[test]
    fn last_reads_correctly_after_the_ring_has_wrapped() {
        // The ring's logical index 0 stops being physical index 0 once the
        // buffer is full; `last` walks logical order, so it must still land
        // on the newest sample.
        let mut buf = TimeSeriesBuffer::with_capacity(8);
        for i in 0..20 {
            tick(&mut buf, i as f64, &[("x", i as f64 * 10.0)]);
        }
        assert_eq!(buf.last("x"), Some((19.0, 190.0)));
    }
}
