// Comprehensive test fixture — exercises every activity type in one model.
//
// Element types present:
//   - StateDefinition with transitions  → Simulate, Sequence Trace
//   - ConstraintUsage (assert)          → Constraint Check, Monte Carlo
//   - RequirementDefinition             → Verification
//   - AnalysisCaseDefinition            → Trade Study
//   - PortUsage + FlowConnectionUsage   → Flow Inspect
//   - AttributeUsage (numeric)          → Sweep
//   - ActionDefinition                  → Action simulation
//
// Usage:
//   App:  http://localhost:3011/?workspace=.../tests/fixtures
//   CLI:  sysml check test-all-activities.sysml

package TestAllActivities {

    // ── Part with attributes (for constraint checking + sweeps) ────────

    part def Vehicle {
        attribute speed : Real = 60.0;
        attribute mass : Real = 1500.0;
        attribute fuelLevel : Real = 80.0;
        attribute maxSpeed : Real = 200.0;
        attribute brakingDistance : Real = 45.0;

        // Constraints for checking + Monte Carlo
        assert constraint speedLimit { speed <= maxSpeed }
        assert constraint positiveMass { mass > 0 }
        assert constraint fuelNotEmpty { fuelLevel >= 0 }
        assert constraint safeBraking { brakingDistance < 100 }
    }

    part testVehicle : Vehicle;

    // ── State Machine (for simulation) ─────────────────────────────────

    state def VehicleDrivingMode {
        entry state parked;
        state idle;
        state driving;
        state braking;

        transition parked_to_idle
            first parked then idle
            do action { /* ignition on */ }

        transition idle_to_driving
            first idle then driving
            do action { /* accelerate */ }

        transition driving_to_braking
            first driving then braking
            do action { /* brake pressed */ }

        transition braking_to_idle
            first braking then idle
            do action { /* stopped */ }

        transition idle_to_parked
            first idle then parked
            do action { /* ignition off */ }
    }

    // ── Requirements (for verification) ────────────────────────────────

    requirement def SafetyReq {
        doc /* The vehicle must operate safely under all conditions. */
    }

    requirement speedReq : SafetyReq {
        doc /* Speed must not exceed maximum rated speed. */
        assume constraint { testVehicle.speed >= 0 }
        require constraint { testVehicle.speed <= testVehicle.maxSpeed }
    }

    requirement fuelReq : SafetyReq {
        doc /* Fuel level must be non-negative during operation. */
        require constraint { testVehicle.fuelLevel >= 0 }
    }

    // ── Analysis Case / Trade Study ────────────────────────────────────

    analysis materialStudy {
        doc /* Select the best material for the chassis. */

        attribute studyObjective = "maximize";
        attribute expression = "0.5 * (strength / 1000) + 0.3 * (10 / density) - 0.2 * (cost / 50)";

        part aluminum {
            attribute strength : Real = 310.0;
            attribute density : Real = 2.7;
            attribute cost : Real = 3.50;
        }

        part steel {
            attribute strength : Real = 580.0;
            attribute density : Real = 7.8;
            attribute cost : Real = 2.00;
        }

        part carbon_fiber {
            attribute strength : Real = 600.0;
            attribute density : Real = 1.6;
            attribute cost : Real = 30.00;
        }
    }

    // ── Ports and Flows (for flow inspect) ─────────────────────────────

    part def Sensor {
        port dataOut : ~Real;
    }

    part def Controller {
        port sensorIn : Real;
        port commandOut : ~Real;
    }

    part def Actuator {
        port commandIn : Real;
    }

    part sensorUnit : Sensor;
    part controller : Controller;
    part actuator : Actuator;

    flow sensorToController
        from sensorUnit.dataOut to controller.sensorIn;

    flow controllerToActuator
        from controller.commandOut to actuator.commandIn;

    // ── Action Definition (for action simulation) ──────────────────────

    action def StartupSequence {
        action checkFuel { /* verify fuel > 0 */ }
        action warmEngine { /* wait for engine temp */ }
        action engageGear { /* shift to drive */ }

        first checkFuel then warmEngine;
        then engageGear;
    }
}
