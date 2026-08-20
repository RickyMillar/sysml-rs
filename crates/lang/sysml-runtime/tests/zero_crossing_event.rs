//! Regression test for spec-typed ZeroCrossingEventDef detection.
//!
//! Exercises the end-to-end zero-crossing detection path:
//! 1. Register an event function (temperature - threshold)
//! 2. Simulate a linear temperature ramp via the ODE solver
//! 3. Assert the crossing event fires at the correct time
//!
//! Spec reference: StateSpaceRepresentation.sysml §ZeroCrossingEventDef
//! Example model: examples/zero-crossing-event/ZeroCrossingEvent.sysml

use std::sync::Arc;

use sysml_core::Value;
use sysml_runtime::expressions::EvalContext;
use sysml_runtime::ode_events::{CrossingDirection, ZeroCrossingDetector};

/// Simulates the ZeroCrossingEvent example model:
/// - Temperature starts at 20 degC, heater rate = 5 degC/s
/// - Event should fire when temperature crosses 100 degC
/// - Expected crossing time: (100 - 20) / 5 = 16.0 s
#[test]
fn zero_crossing_event_fires_at_threshold() {
    let mut detector = ZeroCrossingDetector::new().with_tolerance(1e-8);

    // g(t, y, ctx) = temperature - boilingPoint
    // Rising: fires when temperature crosses 100 from below
    let threshold = 100.0;
    detector.add_event(
        "boilingReached",
        CrossingDirection::Rising,
        Arc::new(move |_t, y, _ctx| {
            // y[0] is the temperature state variable
            y[0] - threshold
        }),
    );

    let ctx = EvalContext::new();
    let heater_rate = 5.0; // degC/s
    let dt = 0.5; // time step

    // Initial state: T = 20.0
    let mut t = 0.0;
    let mut temperature = 20.0;

    detector.initialize(t, &[temperature], &ctx);

    let mut found_crossing = false;
    let mut crossing_time = 0.0;
    let mut crossing_name = String::new();

    // Simulate until t = 25 s (should cross at t = 16 s)
    while t < 25.0 {
        let t_next = t + dt;
        let temp_next = temperature + heater_rate * dt;

        let crossings = detector.check(t, t_next, &[temperature], &[temp_next], &ctx);

        if !crossings.is_empty() {
            found_crossing = true;
            crossing_time = crossings[0].time;
            crossing_name = crossings[0].name.clone();
            break;
        }

        t = t_next;
        temperature = temp_next;
    }

    assert!(found_crossing, "zero-crossing event should fire");
    assert_eq!(crossing_name, "boilingReached");

    // Expected crossing time: (100 - 20) / 5 = 16.0 s
    let expected_time = 16.0;
    assert!(
        (crossing_time - expected_time).abs() < 0.1,
        "crossing should occur near t={expected_time}s, got t={crossing_time:.4}s"
    );
}

/// Verify that the crossing does NOT fire when temperature stays below threshold.
#[test]
fn zero_crossing_event_does_not_fire_below_threshold() {
    let mut detector = ZeroCrossingDetector::new();
    detector.add_event(
        "boilingReached",
        CrossingDirection::Rising,
        Arc::new(|_t, y, _ctx| y[0] - 100.0),
    );

    let ctx = EvalContext::new();
    let dt = 1.0;

    // Temperature goes from 20 to 80 (never reaches 100)
    detector.initialize(0.0, &[20.0], &ctx);
    let crossings = detector.check(0.0, dt, &[20.0], &[80.0], &ctx);
    assert!(
        crossings.is_empty(),
        "should not fire when temperature stays below threshold"
    );
}

/// Verify falling-edge detection (e.g., cooldown below freezing).
#[test]
fn zero_crossing_falling_edge() {
    let mut detector = ZeroCrossingDetector::new();
    detector.add_event(
        "freezing",
        CrossingDirection::Falling,
        Arc::new(|_t, y, _ctx| y[0] - 0.0), // crosses zero
    );

    let ctx = EvalContext::new();

    // Temperature drops from 10 to -5
    detector.initialize(0.0, &[10.0], &ctx);
    let crossings = detector.check(0.0, 1.0, &[10.0], &[-5.0], &ctx);

    assert_eq!(crossings.len(), 1, "should detect freezing event");
    assert_eq!(crossings[0].name, "freezing");
    assert_eq!(crossings[0].direction, CrossingDirection::Falling);
}

/// Verify multiple zero-crossing events can be registered and detected
/// in a single simulation step.
#[test]
fn zero_crossing_multiple_thresholds() {
    let mut detector = ZeroCrossingDetector::new();

    // Two thresholds: 50 degC and 100 degC
    detector.add_event(
        "warmThreshold",
        CrossingDirection::Rising,
        Arc::new(|_t, y, _ctx| y[0] - 50.0),
    );
    detector.add_event(
        "boilingReached",
        CrossingDirection::Rising,
        Arc::new(|_t, y, _ctx| y[0] - 100.0),
    );

    let ctx = EvalContext::new();

    // Temperature jumps from 20 to 120 in one step (crosses both)
    detector.initialize(0.0, &[20.0], &ctx);
    let crossings = detector.check(0.0, 1.0, &[20.0], &[120.0], &ctx);

    assert_eq!(crossings.len(), 2, "should detect both crossings");
    // First crossing (50) should be earlier than second (100)
    assert!(
        crossings[0].time < crossings[1].time,
        "warmThreshold should fire before boilingReached"
    );
}

/// Guard-to-event-fn helper: verify it produces correct sign for threshold detection.
#[test]
fn guard_to_event_fn_integration() {
    let event_fn = sysml_runtime::ode_events::guard_to_event_fn("temperature".to_string(), 100.0);

    let mut ctx = EvalContext::new();

    // Below threshold
    ctx.set("temperature".to_string(), Value::Float(80.0));
    let g = (event_fn)(0.0, &[], &ctx);
    assert!(g < 0.0, "should be negative below threshold");

    // Above threshold
    ctx.set("temperature".to_string(), Value::Float(120.0));
    let g = (event_fn)(0.0, &[], &ctx);
    assert!(g > 0.0, "should be positive above threshold");

    // At threshold
    ctx.set("temperature".to_string(), Value::Float(100.0));
    let g = (event_fn)(0.0, &[], &ctx);
    assert!(g.abs() < 1e-15, "should be zero at threshold");
}

/// Test that the example file parses correctly (basic parse check).
#[test]
fn zero_crossing_example_parses() {
    use sysml_parser_incremental::TreeSitterParser;
    use sysml_parser_trait::{Parser, SysmlFile};

    let example_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .join("zero-crossing-event")
        .join("ZeroCrossingEvent.sysml");

    let source = std::fs::read_to_string(&example_path).expect("read ZeroCrossingEvent.sysml");
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile::new("ZeroCrossingEvent.sysml", source)]);

    assert!(
        result.diagnostics.is_empty()
            || result
                .diagnostics
                .iter()
                .all(|d| d.severity != sysml_span::Severity::Error),
        "example should parse without errors: {:?}",
        result.diagnostics
    );

    // Verify key elements exist in the graph
    let has_heated_vessel = result
        .graph
        .elements
        .values()
        .any(|e| e.name.as_deref() == Some("HeatedVessel"));
    assert!(has_heated_vessel, "should contain HeatedVessel part def");

    let has_heating_dynamics = result
        .graph
        .elements
        .values()
        .any(|e| e.name.as_deref() == Some("HeatingDynamics"));
    assert!(
        has_heating_dynamics,
        "should contain HeatingDynamics action def"
    );
}
