// ===========================================================================
// DEMO: Phase 12 — Reactive Port Simulation
// ===========================================================================
//
// COMMANDS TO TEST:
//   sysml simulate BreakerController tests/fixtures/shared/test_reactive.sysml --events "overcurrent" --json
//   sysml flow tests/fixtures/shared/test_reactive.sysml
//
// SCENARIO:
//   A circuit breaker monitors a current sensor port. When the sensor
//   detects overcurrent (value > 100A), a flow delivers a message to
//   the breaker's "overcurrent" port, triggering a state transition
//   from "closed" to "tripped".
//
// EXPECTED:
//   - State machine starts in "closed"
//   - On "overcurrent" event → transitions to "tripped"
//   - Port payload (current value) captured in context
//
// ===========================================================================

package ReactiveDemo {

    port def CurrentPort {
        out attribute current : ScalarValues::Real;
    }

    port def SignalPort {
        in attribute level : ScalarValues::Real;
    }

    part def CurrentSensor {
        out port currentOut : CurrentPort;
    }

    part def CircuitBreaker {
        in port overcurrent : SignalPort;

        attribute tripCurrent = 100;

        state def BreakerController {
            entry state closed;
            state tripped; // final — breaker stays tripped

            // This transition fires when "overcurrent" event arrives
            // (port message trigger in orchestrator)
            transition first closed then tripped;
        }
    }

    // --- Instances ---

    part sensor : CurrentSensor;
    part breaker : CircuitBreaker;

    // --- Flow: sensor overcurrent → breaker trip ---

    flow overcurrentFlow from sensor.currentOut to breaker.overcurrent;
}
