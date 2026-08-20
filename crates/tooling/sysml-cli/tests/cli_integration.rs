use std::process::Command;

fn sysml_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sysml"))
}

#[test]
fn eval_integer_addition() {
    let output = sysml_bin()
        .args(["eval", "2 + 3"])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success(), "exit code should be 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().contains('5'),
        "expected 5, got: {}",
        stdout.trim()
    );
}

#[test]
fn eval_boolean_expression() {
    let output = sysml_bin()
        .args(["eval", "true"])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().contains("true"),
        "expected true, got: {}",
        stdout.trim()
    );
}

#[test]
fn eval_string_literal() {
    let output = sysml_bin()
        .args(["eval", "\"hello\""])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().contains("hello"),
        "expected hello, got: {}",
        stdout.trim()
    );
}

#[test]
fn eval_invalid_expression_fails() {
    let output = sysml_bin()
        .args(["eval", "undefined_var + 1"])
        .output()
        .expect("failed to run sysml");

    // Should fail because undefined_var is not in context.
    assert!(
        !output.status.success(),
        "should fail for undefined variable"
    );
    // Exit code 1 = user error
    assert_eq!(
        output.status.code(),
        Some(1),
        "undefined variable should use exit code 1"
    );
}

#[test]
fn check_missing_file_fails() {
    let output = sysml_bin()
        .args(["check", "/nonexistent/file.sysml"])
        .output()
        .expect("failed to run sysml");

    assert!(!output.status.success());
    assert_eq!(
        output.status.code(),
        Some(2),
        "IO error should use exit code 2"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot read") || stderr.contains("io error on"),
        "expected read error, got: {}",
        stderr
    );
}

#[test]
fn help_flag_works() {
    let output = sysml_bin()
        .args(["--help"])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("SysML v2 execution tool"));
    assert!(stdout.contains("eval"));
    assert!(stdout.contains("check"));
    assert!(stdout.contains("verify"));
    assert!(stdout.contains("simulate"));
    assert!(stdout.contains("run"));
}

#[test]
fn version_flag_works() {
    let output = sysml_bin()
        .args(["--version"])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sysml"));
}

#[test]
fn check_vehicle_constraints_mixed() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/simple_vehicle.sysml");
    let output = sysml_bin()
        .args(["check", fixture])
        .output()
        .expect("failed to run sysml");

    // Fixture has speed=105, fuel=5, mass=2500
    // speedLimit (speed < 100) → FAIL, fuelCheck (fuel > 40) → FAIL, massCheck (mass > 0) → PASS
    assert!(
        !output.status.success(),
        "should exit non-zero when any constraint fails"
    );
    // Exit code 3 = verification failure
    assert_eq!(
        output.status.code(),
        Some(3),
        "constraint failure should use exit code 3"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[PASS]"),
        "should show [PASS] for massCheck"
    );
    assert!(
        stdout.contains("[FAIL]"),
        "should show [FAIL] for speedLimit"
    );
    assert!(
        stdout.contains("speed < 100"),
        "should show constraint expression"
    );
    assert!(
        stdout.contains("1/3 constraints passed"),
        "should show summary"
    );
}

#[test]
fn check_with_override_fails() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/simple_vehicle.sysml");
    let output = sysml_bin()
        .args(["check", fixture, "--set", "speed=120"])
        .output()
        .expect("failed to run sysml");

    assert!(
        !output.status.success(),
        "should exit non-zero when constraint fails"
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "constraint failure should use exit code 3"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[FAIL]"), "should show [FAIL]");
    // With speed=120, speedLimit still fails, fuelCheck still fails, massCheck passes
    assert!(
        stdout.contains("1/3 constraints passed"),
        "should show 1 passed"
    );
}

#[test]
fn check_json_output_structured() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/simple_vehicle.sysml");
    let output = sysml_bin()
        .args(["check", fixture, "--json"])
        .output()
        .expect("failed to run sysml");

    // JSON output should work even when constraints fail
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("should be valid JSON");
    assert_eq!(json["constraints"], 3);
    assert!(json["results"].is_array());
    // massCheck (mass > 0) should pass
    let results = json["results"].as_array().unwrap();
    let mass_check = results.iter().find(|r| r["description"] == "massCheck");
    assert!(mass_check.is_some(), "should have massCheck result");
    assert_eq!(mass_check.unwrap()["satisfied"], true);
}

#[test]
fn check_verification_multiple_constraints() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/verification.sysml");
    let output = sysml_bin()
        .args(["check", fixture])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[PASS]"), "should show [PASS]");
    assert!(
        stdout.contains("2/2 constraints passed"),
        "should show 2/2 passed"
    );
}

#[test]
fn check_bake_action_constraint() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/bake_action.sysml");
    let output = sysml_bin()
        .args(["check", fixture])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[PASS]"), "should show [PASS]");
    assert!(
        stdout.contains("temperature < 300"),
        "should show constraint expression"
    );
}

#[test]
fn check_traffic_light_no_constraints() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/traffic_light.sysml");
    let output = sysml_bin()
        .args(["check", fixture])
        .output()
        .expect("failed to run sysml");

    // No constraints → exit 0 with "no constraints found" message.
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("no constraints found"),
        "should report no constraints, got: {}",
        stdout
    );
}

#[test]
fn verify_missing_case_fails() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/simple_vehicle.sysml");
    let output = sysml_bin()
        .args(["verify", "NonExistentCase", fixture])
        .output()
        .expect("failed to run sysml");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("error"),
        "expected 'not found' error, got: {}",
        stderr
    );
}

#[test]
fn simulate_missing_file_fails() {
    let output = sysml_bin()
        .args(["simulate", "TrafficLight", "/nonexistent/file.sysml"])
        .output()
        .expect("failed to run sysml");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot read") || stderr.contains("io error on"),
        "expected read error, got: {}",
        stderr
    );
}

#[test]
fn simulate_fixture_no_crash() {
    // The vehicle fixture doesn't have a state machine, so this should fail
    // gracefully without panicking.
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/simple_vehicle.sysml");
    let output = sysml_bin()
        .args(["simulate", "Vehicle", fixture, "--events", "start"])
        .output()
        .expect("failed to run sysml");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "should not panic: {}", stderr);
}

#[test]
fn simulate_selects_requested_state_machine() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/test_state_machine.sysml"
    );
    let output = sysml_bin()
        .args(["simulate", "Toggle", fixture, "--events", "turn_on"])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success(), "simulate should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("State Machine: Toggle"),
        "expected Toggle header, got: {}",
        stdout
    );
}

#[test]
fn simulate_missing_state_machine_name_fails_cleanly() {
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/test_state_machine.sysml"
    );
    let output = sysml_bin()
        .args(["simulate", "DoesNotExist", fixture])
        .output()
        .expect("failed to run sysml");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("state machine 'DoesNotExist' not found") || stderr.contains("SM007"),
        "expected missing state-machine diagnostic, got: {}",
        stderr
    );
}

#[test]
fn run_missing_action_fails() {
    let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/simple_vehicle.sysml");
    let output = sysml_bin()
        .args(["run", "NonExistentAction", fixture])
        .output()
        .expect("failed to run sysml");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("error"),
        "expected error, got: {}",
        stderr
    );
}

#[test]
fn run_missing_file_fails() {
    let output = sysml_bin()
        .args(["run", "SomeAction", "/nonexistent/file.sysml"])
        .output()
        .expect("failed to run sysml");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot read") || stderr.contains("io error on"),
        "expected read error, got: {}",
        stderr
    );
}

#[test]
fn simulate_help_works() {
    let output = sysml_bin()
        .args(["simulate", "--help"])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("state machine"));
    assert!(stdout.contains("--events"));
    assert!(stdout.contains("--interactive"));
    assert!(stdout.contains("--trace"));
}

#[test]
fn run_help_works() {
    let output = sysml_bin()
        .args(["run", "--help"])
        .output()
        .expect("failed to run sysml");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("action"));
    assert!(stdout.contains("--trace"));
    assert!(stdout.contains("--json"));
}
