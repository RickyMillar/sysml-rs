// Coffee Machine Orchestration: Multi-Subsystem Simulation
//
// Demonstrates ALL simulation features:
// - Multi-subsystem orchestration with shared context
// - Entry actions with variable assignments
// - Cross-subsystem guards (machineReady)
// - Composite (nested) states (BrewController.brewing has sub-states)
// - Auto-transition chains (run-to-completion)
// - Completion transitions (do-activity → auto-advance)
// - Constraint monitoring with real expressions
// - Verification case playback with assertion markers
// - Causation tracking (variable writes → guard satisfaction)

package Orchestration {

// ══════════════════════════════════════════════════════════════
// Subsystem 1: Boiler Controller
// Simple flat state machine. Sets boilerTemp and machineReady.
// ══════════════════════════════════════════════════════════════

state def BoilerController {
    doc /* Controls the boiler heating element.
    * Writes: boilerTemp, machineReady, heaterOn
    */

    entry; then cold;

    state cold {
        entry action { boilerTemp = 20; machineReady = 0; heaterOn = 0; }
    }

    state heating {
        entry action { heaterOn = 1; boilerTemp = 45; }
    }

    state ready {
        entry action { boilerTemp = 93; machineReady = 1; heaterOn = 0; }
    }

    state overheated {
        entry action { boilerTemp = 105; heaterOn = 0; machineReady = 0; }
    }

    transition cold_to_heating first cold accept powerOn then heating;
    transition heating_to_ready first heating accept tempReached then ready;
    transition ready_to_heating first ready accept tempDropped then heating;
    transition heating_to_overheated first heating accept tempOverLimit then overheated;
    transition overheated_to_cold first overheated accept cooledDown then cold;
    transition ready_to_cold first ready accept powerOff then cold;
}

// ══════════════════════════════════════════════════════════════
// Subsystem 2: Brew Controller
// Uses COMPOSITE STATE: "brewing" contains sub-states
// (preInfusion → extraction → draining) that auto-chain.
// Demonstrates: nested states, completion transitions, guards.
// ══════════════════════════════════════════════════════════════

state def BrewController {
    doc /* Controls the espresso extraction process.
    * Reads: machineReady (from BoilerController via shared context)
    * Writes: brewActive, shotWeight, waterPressure
    *
    * The "brewing" state is COMPOSITE — it contains sub-states:
    *   preInfusion → extraction → draining (auto-chain)
    * When draining completes, the brewing state completes,
    * triggering the completion transition to "complete".
    */

    entry; then standby;

    state standby {
        entry action { brewActive = 0; shotWeight = 0; waterPressure = 0; }
    }

    // ── Composite state: brewing ──
    // Contains 3 sub-states that chain via events.
    // When the sub-machine reaches "draining" (final),
    // the outer "brewing" state's completion transition fires.
    state brewing {
        entry action { brewActive = 1; }

        state preInfusion {
            entry action { waterPressure = 3; shotWeight = 2; }
        }

        state extraction {
            entry action { waterPressure = 9; shotWeight = 17; }
        }

        state draining {
            entry action { waterPressure = 1; shotWeight = 22; }
        }

        transition preInfusion_to_extraction
            first preInfusion
            accept preInfusionComplete
            then extraction;

        transition extraction_to_draining
            first extraction
            accept extractionDone
            then draining;
    }

    state complete {
        entry action { brewActive = 0; waterPressure = 0; }
    }

    state error {
        entry action { brewActive = 0; waterPressure = 0; shotWeight = 0; }
    }

    // Transitions on outer state machine
    // startBrew is guarded by machineReady — cross-subsystem dependency
    transition standby_to_brewing
        first standby
        accept startBrew
        if machineReady > 0
        then brewing;

    // Completion: when brewing's sub-machine finishes (reaches draining),
    // this fires automatically
    transition brewing_to_complete
        first brewing
        accept brewingDone
        then complete;

    transition complete_to_standby
        first complete
        accept resetBrew
        then standby;

    transition brewing_to_error
        first brewing
        accept pressureFault
        then error;

    transition error_to_standby
        first error
        accept clearError
        then standby;
}

// ══════════════════════════════════════════════════════════════
// Subsystem 3: Grinder Controller
// Simple flat state machine. Independent subsystem.
// ══════════════════════════════════════════════════════════════

state def GrinderController {
    doc /* Controls the burr grinder.
    * Writes: grindWeight, grinderRunning
    */

    entry; then idle;

    state idle {
        entry action { grinderRunning = 0; }
    }

    state grinding {
        entry action { grinderRunning = 1; grindWeight = 0; }
    }

    state dosed {
        entry action { grinderRunning = 0; grindWeight = 18.5; }
    }

    transition idle_to_grinding first idle accept startGrind then grinding;
    transition grinding_to_dosed first grinding accept doseReached then dosed;
    transition dosed_to_idle first dosed accept clearDose then idle;
}

// ══════════════════════════════════════════════════════════════
// Constraints — monitored with real expressions
// ══════════════════════════════════════════════════════════════

constraint def SafeTemperature {
    doc /* Boiler temperature must stay below 100C. */
    constraint { boilerTemp < 100 }
}

constraint def ExtractionPressure {
    doc /* Water pressure must be 8-10 bar during extraction. */
    constraint { waterPressure >= 8 and waterPressure <= 10 }
}

constraint def GrindDoseWeight {
    doc /* Grind dose must be 18-22 grams. */
    constraint { grindWeight >= 18 and grindWeight <= 22 }
}

constraint def ShotWeightTarget {
    doc /* Final shot weight should be 36-44 grams (1:2 ratio). */
    constraint { shotWeight >= 18 and shotWeight <= 44 }
}

// ══════════════════════════════════════════════════════════════
// Verification Case: Full Brew Cycle
// Tests: grind → heat → brew (composite) → complete
// Exercises: cross-subsystem guards, composite state entry,
//            sub-state transitions, constraint monitoring
// ══════════════════════════════════════════════════════════════

verification def FullBrewCycleTest {
    doc /* Verify a complete espresso extraction with all subsystems.
    * 1. Grinder doses beans (grindWeight = 18.5)
    * 2. Boiler heats up (machineReady = 1)
    * 3. Brew starts (guard passes), enters composite brewing state
    * 4. Sub-states: preInfusion → extraction → draining
    * 5. Brewing completes, machine resets
    */

    subject brewSystem : EspressoMachine;

    action testScript {
        // Phase 1: Grind
        action startGrind;
        action doseReached;

        // Phase 2: Heat boiler
        action powerOn;
        action tempReached;

        // Phase 3: Brew (enters composite state → preInfusion sub-state)
        action startBrew;
        action preInfusionComplete;
        action extractionDone;

        // Phase 4: Complete and reset
        action brewingDone;
        action resetBrew;
        action clearDose;

        succession startGrind then doseReached;
        succession doseReached then powerOn;
        succession powerOn then tempReached;
        succession tempReached then startBrew;
        succession startBrew then preInfusionComplete;
        succession preInfusionComplete then extractionDone;
        succession extractionDone then brewingDone;
        succession brewingDone then resetBrew;
        succession resetBrew then clearDose;
    }

    requirement safeOperation {
        doc /* Temperature stays safe throughout. */
        constraint { boilerTemp < 100 }
    }

    requirement goodDose {
        doc /* Grind dose is within range. */
        constraint { grindWeight >= 18 and grindWeight <= 22 }
    }

    requirement extractionPressureOk {
        doc /* Pressure is correct during extraction. */
        constraint { waterPressure >= 1 and waterPressure <= 10 }
    }
}

// ══════════════════════════════════════════════════════════════
// Verification Case: Guard Blocking
// Tests: brew attempt without boiler ready → stays in standby
// Exercises: cross-subsystem guard failure, causation diagnosis
// ══════════════════════════════════════════════════════════════

verification def BrewWithoutHeatTest {
    doc /* Verify brewing is blocked when boiler is not ready.
    * The guard "machineReady > 0" should prevent transition.
    * Causation graph should show: machineReady = 0 blocks startBrew.
    */

    subject brewSystem : EspressoMachine;

    action testScript {
        action startBrew;
        succession startBrew then startBrew;
    }

    requirement brewBlocked {
        doc /* Brew controller should remain in standby. */
        constraint { brewActive == 0 }
    }
}

// ══════════════════════════════════════════════════════════════
// Verification Case: Overheat Recovery
// Tests: boiler overheats → recovers → can brew again
// Exercises: error states, recovery paths, variable reset
// ══════════════════════════════════════════════════════════════

verification def OverheatRecoveryTest {
    doc /* Verify the machine recovers from an overheated boiler.
    * 1. Power on → heating
    * 2. Overheat → overheated (machineReady = 0, boilerTemp = 105)
    * 3. Cool down → cold
    * 4. Power on again → heating → ready (machineReady = 1)
    */

    subject brewSystem : EspressoMachine;

    action testScript {
        action powerOn;
        action tempOverLimit;
        action cooledDown;
        action powerOn;
        action tempReached;

        succession powerOn then tempOverLimit;
        succession tempOverLimit then cooledDown;
        succession cooledDown then powerOn;
        succession powerOn then tempReached;
    }

    requirement recoveredSafely {
        doc /* Machine returns to ready state after recovery. */
        constraint { machineReady > 0 }
    }

    requirement tempNormal {
        doc /* Temperature is normal after recovery. */
        constraint { boilerTemp < 100 }
    }
}

// ══════════════════════════════════════════════════════════════
// Assembly: the actual coffee machine
// ══════════════════════════════════════════════════════════════

part def EspressoMachine {
    doc /* Dual-boiler espresso machine with grinder.
    * Three subsystems coordinate via shared context variables.
    */

    exhibit state boilerController : BoilerController;
    exhibit state brewController : BrewController;
    exhibit state grinderController : GrinderController;

    // Shared context attributes
    attribute boilerTemp : ScalarValues::Real;
    attribute machineReady : ScalarValues::Real;
    attribute heaterOn : ScalarValues::Real;
    attribute brewActive : ScalarValues::Real;
    attribute shotWeight : ScalarValues::Real;
    attribute waterPressure : ScalarValues::Real;
    attribute grindWeight : ScalarValues::Real;
    attribute grinderRunning : ScalarValues::Real;
}

}
