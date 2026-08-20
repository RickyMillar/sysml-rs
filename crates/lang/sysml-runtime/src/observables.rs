//! Consumer-neutral simulation observables.
//!
//! Live simulation snapshots are telemetry: per-tick state for UI charts,
//! debugging, and streaming. This module turns that telemetry into named run
//! results (`OutputBundle`) that can be bound back into model features and fed
//! to any consumer: verification, trade studies, what-if, sensitivity,
//! optimization, monitoring, or reports. It deliberately contains no
//! verification-specific concepts.

use std::collections::HashMap;

use sysml_core::Value;

use crate::expressions::EvalContext;
use crate::occurrence::OccurrenceTracker;
use crate::orchestrator::ExecutionSnapshot;

/// A named set of values produced by one simulation run.
///
/// `OutputBundle` is dynamic run state, not a graph derivative. Salsa may cache
/// the model-declared specs that request these outputs, but the measured values
/// belong to a concrete run/session and should remain outside salsa caches.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct OutputBundle {
    /// Named observable values produced by the run.
    pub values: Vec<OutputValue>,
}

impl OutputBundle {
    /// Create an empty bundle.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a named output value.
    pub fn push(&mut self, value: OutputValue) {
        self.values.push(value);
    }

    /// Look up an output by name.
    pub fn get(&self, name: &str) -> Option<&OutputValue> {
        self.values.iter().find(|v| v.name == name)
    }

    /// Convert this bundle into a flat overlay map suitable for layering onto
    /// an `EvalContext`. Outputs with a [`OutputTarget::FeatureKey`] bind under
    /// that model-facing key; unbound outputs bind under their own output name.
    pub fn to_overlay(&self) -> HashMap<String, Value> {
        self.values
            .iter()
            .map(|out| {
                let key = match &out.target {
                    OutputTarget::FeatureKey(key) => key.clone(),
                    OutputTarget::Unbound => out.name.clone(),
                };
                (key, out.value.clone())
            })
            .collect()
    }

    /// Overlay this run's measured outputs onto an evaluation context.
    ///
    /// This is the one thin adapter between consumer-neutral observables and any
    /// value consumer that reads through an [`EvalContext`] — verification (the
    /// first consumer, via [`crate::cases::VerificationRunner`]), trade studies,
    /// what-if, sensitivity, monitoring, or reporting. The orchestration layer
    /// runs the simulation, measures the observables it requested, then calls
    /// this to make the results available; the consumer reads them through the
    /// model-declared binding (`=` / BindingConnector) keyed by feature.
    ///
    /// Each output binds under its model-facing [`OutputTarget::FeatureKey`] (or
    /// its own name when [`OutputTarget::Unbound`]). Measured values overwrite
    /// any existing binding for the same key: they ARE this run's fresh result,
    /// so they win over inherited defaults — the same last-writer-wins overlay
    /// `run_and_verify` uses for solver-executed results.
    pub fn apply_to_context(&self, ctx: &mut EvalContext) {
        for (key, value) in self.to_overlay() {
            ctx.set(key, value);
        }
    }
}

/// One named value produced by a simulation run.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct OutputValue {
    /// Consumer-facing output name, e.g. `peak_temperature` or `trip_time_ms`.
    pub name: String,
    /// Optional model-facing binding target for this output.
    pub target: OutputTarget,
    /// Measured value.
    pub value: Value,
    /// Optional evidence pointing at where/how the value was measured.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub evidence: Option<OutputEvidence>,
}

impl OutputValue {
    pub fn new(name: impl Into<String>, value: Value) -> Self {
        Self {
            name: name.into(),
            target: OutputTarget::Unbound,
            value,
            evidence: None,
        }
    }

    pub fn with_target(mut self, target: OutputTarget) -> Self {
        self.target = target;
        self
    }

    pub fn with_evidence(mut self, evidence: OutputEvidence) -> Self {
        self.evidence = Some(evidence);
        self
    }
}

/// Where a measured output should bind when converted to a model-value overlay.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum OutputTarget {
    /// Bind under this model/evaluator feature key. Today this is a string key
    /// because `EvalContext` is still string-keyed; the semantic-core migration
    /// can deepen this to RuntimeId/ElementId without changing the observable
    /// measurement layer.
    FeatureKey(String),
    /// Output is named but not yet bound to a model feature.
    Unbound,
}

/// Evidence for a measured output.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct OutputEvidence {
    /// First tick included in the measurement window, if known.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub start_tick: Option<u64>,
    /// Last tick included in the measurement window, if known.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub end_tick: Option<u64>,
    /// First time included in the measurement window, in milliseconds.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub start_time_ms: Option<f64>,
    /// Last time included in the measurement window, in milliseconds.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub end_time_ms: Option<f64>,
    /// Optional subsystem this output was derived from.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub subsystem: Option<String>,
    /// Optional source variable this output was derived from.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub variable: Option<String>,
}

/// A consumer-neutral request for a named observable value.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ObservableSpec {
    /// Name to give the output in the resulting bundle.
    pub name: String,
    /// Measurement to perform.
    pub kind: ObservableKind,
    /// Optional binding target for the measured value.
    pub target: OutputTarget,
}

impl ObservableSpec {
    pub fn new(name: impl Into<String>, kind: ObservableKind) -> Self {
        Self {
            name: name.into(),
            kind,
            target: OutputTarget::Unbound,
        }
    }

    pub fn with_target(mut self, target: OutputTarget) -> Self {
        self.target = target;
        self
    }
}

/// Generic observable kinds over a simulation trace.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum ObservableKind {
    /// Final numeric value of a variable.
    LastValue { variable: String, window: Window },
    /// Minimum numeric value of a variable.
    Min { variable: String, window: Window },
    /// Maximum numeric value of a variable.
    Max { variable: String, window: Window },
    /// Arithmetic mean of a variable.
    Mean { variable: String, window: Window },
    /// Root-mean-square of a variable.
    Rms { variable: String, window: Window },
    /// Maximum absolute value of a variable.
    PeakAbs { variable: String, window: Window },
    /// Time spent in a state, in milliseconds, computed from a subsystem's state lane.
    StateDwellTime {
        subsystem: String,
        state: String,
        window: Window,
    },
    /// Fraction of observed high/low dwell time spent in `high_state`.
    ///
    /// This is generic state-machine duty cycle: `high / (high + low)`. Any
    /// domain-specific remapping (for example an asymmetry metric) should be a
    /// separate model expression or downstream observable, not baked into this
    /// primitive.
    StateDutyCycle {
        subsystem: String,
        high_state: String,
        low_state: String,
        window: Window,
    },
    /// First time a subsystem reaches `state`, in milliseconds.
    TimeToState {
        subsystem: String,
        state: String,
        window: Window,
    },
}

/// Measurement window in simulation time.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Window {
    /// Full available trace.
    Full,
    /// Inclusive time interval in milliseconds.
    BetweenMs { start: f64, end: f64 },
    /// From the given time to the end of the trace.
    FromMs { start: f64 },
    /// Last `duration` milliseconds of the trace.
    LastMs { duration: f64 },
}

impl Window {
    fn bounds(self, trace: &[ExecutionSnapshot]) -> Option<(f64, f64)> {
        let first = trace.first()?.time_ms;
        let last = trace.last()?.time_ms;
        match self {
            Window::Full => Some((first, last)),
            Window::BetweenMs { start, end } => Some((start, end)),
            Window::FromMs { start } => Some((start, last)),
            Window::LastMs { duration } => Some(((last - duration).max(first), last)),
        }
    }

    fn contains(self, trace: &[ExecutionSnapshot], t: f64) -> bool {
        let Some((start, end)) = self.bounds(trace) else {
            return false;
        };
        t >= start && t <= end
    }
}

/// Errors that can occur while measuring an observable.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ObservableError {
    #[error("cannot measure observable `{name}` from an empty trace")]
    EmptyTrace { name: String },
    #[error(
        "observable `{name}` has no samples for variable `{variable}` in the requested window"
    )]
    NoSamples { name: String, variable: String },
    #[error("observable `{name}` has no state samples for subsystem `{subsystem}` in the requested window")]
    NoStateSamples { name: String, subsystem: String },
    #[error("observable `{name}` has zero dwell time for the requested states")]
    ZeroDwell { name: String },
}

/// Measure every spec against a completed or in-progress simulation trace.
pub fn measure_observables(
    trace: &[ExecutionSnapshot],
    occurrences: Option<&OccurrenceTracker>,
    specs: &[ObservableSpec],
) -> Result<OutputBundle, ObservableError> {
    let mut bundle = OutputBundle::new();
    for spec in specs {
        bundle.push(measure_observable(trace, occurrences, spec)?);
    }
    Ok(bundle)
}

/// Measure one observable against a simulation trace.
pub fn measure_observable(
    trace: &[ExecutionSnapshot],
    _occurrences: Option<&OccurrenceTracker>,
    spec: &ObservableSpec,
) -> Result<OutputValue, ObservableError> {
    if trace.is_empty() {
        return Err(ObservableError::EmptyTrace {
            name: spec.name.clone(),
        });
    }

    let (value, evidence) = match &spec.kind {
        ObservableKind::LastValue { variable, window } => {
            let samples = numeric_samples(trace, variable, *window);
            let Some((tick, time_ms, value)) = samples.last().copied() else {
                return Err(ObservableError::NoSamples {
                    name: spec.name.clone(),
                    variable: variable.clone(),
                });
            };
            (
                Value::Float(value),
                OutputEvidence {
                    start_tick: Some(tick),
                    end_tick: Some(tick),
                    start_time_ms: Some(time_ms),
                    end_time_ms: Some(time_ms),
                    subsystem: None,
                    variable: Some(variable.clone()),
                },
            )
        }
        ObservableKind::Min { variable, window } => {
            numeric_stat(&spec.name, variable, trace, *window, |vals| {
                vals.iter().copied().fold(f64::INFINITY, f64::min)
            })?
        }
        ObservableKind::Max { variable, window } => {
            numeric_stat(&spec.name, variable, trace, *window, |vals| {
                vals.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            })?
        }
        ObservableKind::Mean { variable, window } => {
            numeric_stat(&spec.name, variable, trace, *window, |vals| {
                vals.iter().sum::<f64>() / vals.len() as f64
            })?
        }
        ObservableKind::Rms { variable, window } => {
            numeric_stat(&spec.name, variable, trace, *window, |vals| {
                (vals.iter().map(|v| v * v).sum::<f64>() / vals.len() as f64).sqrt()
            })?
        }
        ObservableKind::PeakAbs { variable, window } => {
            numeric_stat(&spec.name, variable, trace, *window, |vals| {
                vals.iter().map(|v| v.abs()).fold(0.0, f64::max)
            })?
        }
        ObservableKind::StateDwellTime {
            subsystem,
            state,
            window,
        } => {
            let dwell = state_dwell_ms(&spec.name, trace, subsystem, *window)?;
            let ms = dwell.get(state).copied().unwrap_or(0.0);
            (Value::Float(ms), state_evidence(trace, subsystem, *window))
        }
        ObservableKind::StateDutyCycle {
            subsystem,
            high_state,
            low_state,
            window,
        } => {
            let dwell = state_dwell_ms(&spec.name, trace, subsystem, *window)?;
            let high = dwell.get(high_state).copied().unwrap_or(0.0);
            let low = dwell.get(low_state).copied().unwrap_or(0.0);
            let denom = high + low;
            if denom <= f64::EPSILON {
                return Err(ObservableError::ZeroDwell {
                    name: spec.name.clone(),
                });
            }
            (
                Value::Float(high / denom),
                state_evidence(trace, subsystem, *window),
            )
        }
        ObservableKind::TimeToState {
            subsystem,
            state,
            window,
        } => {
            let Some(snap) = trace.iter().find(|snap| {
                window.contains(trace, snap.time_ms)
                    && snap
                        .subsystem_states
                        .get(subsystem)
                        .is_some_and(|s| s.current_state == *state)
            }) else {
                return Err(ObservableError::NoStateSamples {
                    name: spec.name.clone(),
                    subsystem: subsystem.clone(),
                });
            };
            (
                Value::Float(snap.time_ms),
                OutputEvidence {
                    start_tick: Some(snap.tick),
                    end_tick: Some(snap.tick),
                    start_time_ms: Some(snap.time_ms),
                    end_time_ms: Some(snap.time_ms),
                    subsystem: Some(subsystem.clone()),
                    variable: None,
                },
            )
        }
    };

    Ok(OutputValue::new(spec.name.clone(), value)
        .with_target(spec.target.clone())
        .with_evidence(evidence))
}

fn numeric_stat(
    name: &str,
    variable: &str,
    trace: &[ExecutionSnapshot],
    window: Window,
    f: impl FnOnce(&[f64]) -> f64,
) -> Result<(Value, OutputEvidence), ObservableError> {
    let samples = numeric_samples(trace, variable, window);
    if samples.is_empty() {
        return Err(ObservableError::NoSamples {
            name: name.to_owned(),
            variable: variable.to_owned(),
        });
    }
    let values: Vec<f64> = samples.iter().map(|(_, _, v)| *v).collect();
    let first = samples.first().expect("non-empty checked");
    let last = samples.last().expect("non-empty checked");
    Ok((
        Value::Float(f(&values)),
        OutputEvidence {
            start_tick: Some(first.0),
            end_tick: Some(last.0),
            start_time_ms: Some(first.1),
            end_time_ms: Some(last.1),
            subsystem: None,
            variable: Some(variable.to_owned()),
        },
    ))
}

fn numeric_samples(
    trace: &[ExecutionSnapshot],
    variable: &str,
    window: Window,
) -> Vec<(u64, f64, f64)> {
    trace
        .iter()
        .filter(|snap| window.contains(trace, snap.time_ms))
        .filter_map(|snap| numeric_value(snap, variable).map(|v| (snap.tick, snap.time_ms, v)))
        .filter(|(_, _, v)| v.is_finite())
        .collect()
}

fn numeric_value(snapshot: &ExecutionSnapshot, variable: &str) -> Option<f64> {
    let raw = snapshot
        .resolved_refs
        .get(variable)
        .or_else(|| snapshot.variables.get(variable))?;
    match raw {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        Value::Quantity { value, .. } => Some(*value),
        _ => None,
    }
}

fn state_dwell_ms(
    observable_name: &str,
    trace: &[ExecutionSnapshot],
    subsystem: &str,
    window: Window,
) -> Result<HashMap<String, f64>, ObservableError> {
    if trace.len() < 2 {
        return Ok(HashMap::new());
    }
    let Some((window_start, window_end)) = window.bounds(trace) else {
        return Ok(HashMap::new());
    };

    let mut dwell = HashMap::new();
    let mut saw_subsystem = false;
    for pair in trace.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        let Some(state) = a.subsystem_states.get(subsystem) else {
            continue;
        };
        saw_subsystem = true;
        let start = a.time_ms.max(window_start);
        let end = b.time_ms.min(window_end);
        let dt = end - start;
        if dt > 0.0 {
            *dwell.entry(state.current_state.clone()).or_insert(0.0) += dt;
        }
    }

    if !saw_subsystem {
        return Err(ObservableError::NoStateSamples {
            name: observable_name.to_owned(),
            subsystem: subsystem.to_owned(),
        });
    }
    Ok(dwell)
}

fn state_evidence(trace: &[ExecutionSnapshot], subsystem: &str, window: Window) -> OutputEvidence {
    let (start_time, end_time) = window.bounds(trace).unwrap_or((0.0, 0.0));
    let start_tick = trace
        .iter()
        .find(|snap| snap.time_ms >= start_time)
        .map(|snap| snap.tick);
    let end_tick = trace
        .iter()
        .rev()
        .find(|snap| snap.time_ms <= end_time)
        .map(|snap| snap.tick);
    OutputEvidence {
        start_tick,
        end_tick,
        start_time_ms: Some(start_time),
        end_time_ms: Some(end_time),
        subsystem: Some(subsystem.to_owned()),
        variable: None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::orchestrator::{ExecutionSnapshot, ValueMeasurement};
    use crate::SubsystemState;

    fn subsystem_state(name: &str, current_state: &str) -> SubsystemState {
        SubsystemState {
            name: name.to_owned(),
            kind: "stateMachine",
            current_state: current_state.to_owned(),
            completed: false,
            available_transitions: Vec::new(),
            outputs: Vec::new(),
            sends: Vec::new(),
            incoming_transition_trigger: None,
            deferred_event_count: 0,
            source_element_id: None,
        }
    }

    fn snapshot(
        tick: u64,
        time_ms: f64,
        vars: &[(&str, Value)],
        sm_state: Option<&str>,
    ) -> ExecutionSnapshot {
        let mut variables = HashMap::new();
        for (k, v) in vars {
            variables.insert((*k).to_owned(), v.clone());
        }
        let mut subsystem_states = HashMap::new();
        if let Some(state) = sm_state {
            subsystem_states.insert("sm".to_owned(), subsystem_state("sm", state));
        }
        ExecutionSnapshot {
            tick,
            time_ms,
            subsystem_states,
            variables: Arc::new(variables),
            messages: Vec::new(),
            constraint_results: Vec::new(),
            assertion_checkpoints: Vec::new(),
            guard_diagnoses: Vec::new(),
            causation_links: Vec::new(),
            completed: false,
            port_values: HashMap::new(),
            derivatives: HashMap::new(),
            resolved_refs: HashMap::new(),
            flow_drop_warnings: Vec::new(),
            value_units: Arc::<HashMap<String, ValueMeasurement>>::default(),
            step_size_health: Vec::new(),
        }
    }

    #[test]
    fn measures_numeric_stats_generically() {
        let trace = vec![
            snapshot(1, 0.0, &[("x", Value::Float(1.0))], None),
            snapshot(2, 10.0, &[("x", Value::Float(-3.0))], None),
            snapshot(3, 20.0, &[("x", Value::Float(2.0))], None),
        ];
        let specs = vec![
            ObservableSpec::new(
                "x_peak",
                ObservableKind::PeakAbs {
                    variable: "x".to_owned(),
                    window: Window::Full,
                },
            ),
            ObservableSpec::new(
                "x_mean",
                ObservableKind::Mean {
                    variable: "x".to_owned(),
                    window: Window::Full,
                },
            ),
        ];
        let bundle = measure_observables(&trace, None, &specs).unwrap();
        assert_eq!(bundle.get("x_peak").unwrap().value, Value::Float(3.0));
        assert_eq!(bundle.get("x_mean").unwrap().value, Value::Float(0.0));
    }

    #[test]
    fn measures_state_duty_cycle_from_state_lane() {
        let trace = vec![
            snapshot(1, 0.0, &[], Some("on")),
            snapshot(2, 10.0, &[], Some("off")),
            snapshot(3, 30.0, &[], Some("on")),
            snapshot(4, 40.0, &[], Some("off")),
        ];
        let spec = ObservableSpec::new(
            "duty",
            ObservableKind::StateDutyCycle {
                subsystem: "sm".to_owned(),
                high_state: "on".to_owned(),
                low_state: "off".to_owned(),
                window: Window::Full,
            },
        );
        let out = measure_observable(&trace, None, &spec).unwrap();
        // Intervals use the previous snapshot's state:
        // on: 0-10 + 30-40 = 20 ms, off: 10-30 = 20 ms.
        assert_eq!(out.value, Value::Float(0.5));
    }

    #[test]
    fn output_bundle_converts_targets_to_overlay() {
        let mut bundle = OutputBundle::new();
        bundle.push(
            OutputValue::new("peak_temperature", Value::Float(95.0))
                .with_target(OutputTarget::FeatureKey("analysis.result".to_owned())),
        );
        let overlay = bundle.to_overlay();
        assert_eq!(overlay.get("analysis.result"), Some(&Value::Float(95.0)));
    }

    #[test]
    fn apply_to_context_overlays_measured_value_under_feature_key() {
        let mut bundle = OutputBundle::new();
        bundle.push(
            OutputValue::new("peak", Value::Float(95.0))
                .with_target(OutputTarget::FeatureKey("analysis.peak".to_owned())),
        );
        // Unbound outputs fall back to their own name.
        bundle.push(OutputValue::new("trip_time_ms", Value::Float(12.0)));

        let mut ctx = EvalContext::new();
        // A fresh run output wins over an inherited default for the same key.
        ctx.set("analysis.peak", Value::Float(0.0));
        bundle.apply_to_context(&mut ctx);

        assert_eq!(ctx.get("analysis.peak"), Some(&Value::Float(95.0)));
        assert_eq!(ctx.get("trip_time_ms"), Some(&Value::Float(12.0)));
    }

    /// End-to-end runtime path (service/Snapshot integration deferred): a
    /// simulation trace is measured into an observable bound to a model feature
    /// key, overlaid onto the check-time context, and consumed by the ONE
    /// verdict engine through a model-declared binding. The verdict tracks the
    /// measured value — proof that a simulation result flows into a consumer.
    #[test]
    fn measured_observable_drives_verification_verdict() {
        use crate::cases::{
            RequirementBinding, RequirementCheck, VerdictKind, VerificationCaseIR,
            VerificationRunner,
        };
        use crate::expressions::{BinOp, ExprIR};

        // A requirement: the subject's peak must stay under 100. The model-declared
        // binding wires the requirement's `subject.peak` to the run's `analysis.peak`
        // observable (the `=` / BindingConnector form, resolved at check time).
        let make_req = || RequirementCheck {
            id: "peak-under-limit".into(),
            source_element_id: None,
            text: Some("subject peak must stay under 100".into()),
            assumptions: vec![],
            constraints: vec![ExprIR::BinaryOp {
                op: BinOp::LessThan,
                left: Box::new(ExprIR::FeatureChain(vec!["subject".into(), "peak".into()])),
                right: Box::new(ExprIR::LiteralReal(100.0)),
            }],
            constraint_element_ids: vec![None],
            compile_errors: vec![],
            subrequirements: vec![],
            bindings: vec![],
            binding_specs: vec![RequirementBinding::FeaturePath {
                name: "subject.peak".into(),
                path: "analysis.peak".into(),
            }],
        };
        let make_case = || VerificationCaseIR {
            id: "vc-peak".into(),
            name: "Peak Check".into(),
            subject: Some("subject".into()),
            setup_actions: vec![],
            requirements: vec![make_req()],
            sub_cases: vec![],
            verdict_expression: None,
            bindings: Vec::new(),
        };

        let spec = ObservableSpec::new(
            "peak",
            ObservableKind::PeakAbs {
                variable: "x".to_owned(),
                window: Window::Full,
            },
        )
        .with_target(OutputTarget::FeatureKey("analysis.peak".to_owned()));

        // Run A: a trace whose peak (95) satisfies the requirement -> Pass.
        let trace_ok = vec![
            snapshot(1, 0.0, &[("x", Value::Float(40.0))], None),
            snapshot(2, 10.0, &[("x", Value::Float(95.0))], None),
            snapshot(3, 20.0, &[("x", Value::Float(60.0))], None),
        ];
        let bundle_ok = measure_observables(&trace_ok, None, &[spec.clone()]).unwrap();
        let mut ctx_ok = EvalContext::new();
        bundle_ok.apply_to_context(&mut ctx_ok);
        let result_ok = VerificationRunner::new().verify(&make_case(), &ctx_ok);
        assert_eq!(result_ok.verdict, VerdictKind::Pass);

        // Run B: a higher-peak trace (150) violates the same requirement -> Fail.
        // Same model, same binding — only the simulated value changed.
        let trace_hot = vec![
            snapshot(1, 0.0, &[("x", Value::Float(40.0))], None),
            snapshot(2, 10.0, &[("x", Value::Float(150.0))], None),
            snapshot(3, 20.0, &[("x", Value::Float(60.0))], None),
        ];
        let bundle_hot = measure_observables(&trace_hot, None, &[spec]).unwrap();
        let mut ctx_hot = EvalContext::new();
        bundle_hot.apply_to_context(&mut ctx_hot);
        let result_hot = VerificationRunner::new().verify(&make_case(), &ctx_hot);
        assert_eq!(result_hot.verdict, VerdictKind::Fail);
    }
}
