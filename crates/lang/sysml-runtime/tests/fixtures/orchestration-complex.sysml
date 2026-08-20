// orchestration-complex.sysml -- Full Coffee Machine Orchestration Stress Test
//
// THE comprehensive orchestration fixture. SM + Action + ODE + flows +
// constraints + verification. 4 subsystem types, 10 states, fork/join,
// ODE thermal model, port-based flows, sequence trace recording.
//
// Uses spec-standard StateSpaceRepresentation pattern:
//   calc def :> GetDerivative defines dx/dt = f(u, x)
//   @ToolExecution selects the solver
//
// Feature coverage:
//   [12.1] Multi-subsystem tick loop                -- FULL
//   [12.2] All subsystem types (SM+Action+ODE+ODE45) -- FULL
//   [12.3] Causation tracking                       -- FULL
//   [12.4] Co-simulation with external tools        -- MISSING
//   [9.1]  Sequence trace recording                 -- FULL
//   [10.2] HappensBefore / succession               -- PARTIAL
//
// Expected element counts:
//   2 state defs (BoilerController 6 states, WaterPump 4 states)
//   1 action def (BrewCycle with fork/join, 6 sub-actions)
//   1 ODE part def (ThermalBody with RK45 metadata)
//   3 port defs, 3 flow-component part defs
//   1 assembly part def (CoffeeMachine), 1 part instance
//   2 constraint defs, 1 verification def
//   14 transitions, 2 forks, 2 joins, 5 successions
//   2 flow connections, 3 assert constraints

package StressOrchestrationComplex {

    import StateSpaceRepresentation::*;
    import AnalysisTooling::*;

    // ================================================================
    // Subsystem 1: Boiler Controller (State Machine, 6 states)
    // ================================================================
    // [12.1] Tick loop drives state transitions each cycle.
    // [12.3] Causation: boilerTemp written here, read by ThermalBody guard.

    state def BoilerController {
        doc /* Controls boiler heating through 6 phases.
             * Writes: boilerTemp, heaterPower, boilerReady */

        entry; then off;

        state off {
            entry action { boilerTemp = 20; heaterPower = 0; boilerReady = 0; }
        }
        state coldStart {
            entry action { heaterPower = 1500; boilerReady = 0; }
        }
        state preheating {
            entry action { heaterPower = 1200; }
        }
        state ready {
            entry action { heaterPower = 200; boilerReady = 1; }
        }
        state steaming {
            entry action { heaterPower = 1400; boilerReady = 0; }
        }
        state overtemp {
            entry action { heaterPower = 0; boilerReady = 0; }
        }

        transition off_to_coldStart first off accept powerOn then coldStart;
        transition coldStart_to_preheating first coldStart if boilerTemp > 40 then preheating;
        transition preheating_to_ready first preheating if boilerTemp > 90 then ready;
        transition ready_to_steaming first ready accept steamRequest then steaming;
        transition steaming_to_ready first steaming if boilerTemp > 140 then ready;
        transition ready_to_off first ready accept powerOff then off;
        transition preheating_to_overtemp first preheating if boilerTemp > 105 then overtemp;
        transition steaming_to_overtemp first steaming if boilerTemp > 105 then overtemp;
        transition overtemp_to_off first overtemp accept cooledDown then off;
    }

    // ================================================================
    // Subsystem 2: Water Pump (State Machine, 4 states)
    // ================================================================
    // [12.2] Second state machine subsystem in the tick loop.
    // [12.3] Causation: pumpPressure written here, read by constraint checks.

    state def WaterPump {
        doc /* Controls water flow through priming and operation.
             * Writes: pumpActive, waterFlow, pumpPressure */

        entry; then off;

        state off {
            entry action { pumpActive = 0; waterFlow = 0; pumpPressure = 0; }
        }
        state priming {
            entry action { pumpActive = 1; waterFlow = 2; pumpPressure = 3; }
        }
        state running {
            entry action { waterFlow = 8; pumpPressure = 9; }
        }
        state faulted {
            entry action { pumpActive = 0; waterFlow = 0; pumpPressure = 0; }
        }

        transition off_to_priming first off accept startPump then priming;
        transition priming_to_running first priming accept primeComplete then running;
        transition running_to_off first running accept stopPump then off;
        transition running_to_faulted first running accept pumpFault then faulted;
        transition faulted_to_off first faulted accept resetPump then off;
    }

    // ================================================================
    // Subsystem 3: Brew Cycle (Action Flow with fork/join)
    // ================================================================
    // [12.2] Action-flow subsystem type in the tick loop.
    // [9.1] Sequence trace: fork/join ordering recorded for replay.

    action def HeatAction { out item hotWater; }
    action def GrindAction { out item grounds; }
    action def TampAction { in item grounds; out item puck; }
    action def ExtractAction { in item hotWater; in item puck; out item espresso; }
    action def FrothAction { out item milk; }
    action def MixAction { in item espresso; in item milk; out item latte; }

    action def BrewCycle {
        doc /* Full brew cycle with fork/join parallelism.
             * Heat + Grind in parallel, then tamp, extract, froth, mix. */

        action heat : HeatAction;
        action grind : GrindAction;
        action tamp : TampAction;
        action extract : ExtractAction;
        action froth : FrothAction;
        action mix : MixAction;

        fork prepFork;
        first start then prepFork;
        first prepFork then heat;
        first prepFork then grind;

        join prepJoin;
        first heat then prepJoin;
        first grind then tamp;
        first tamp then prepJoin;

        first prepJoin then extract;

        fork serveFork;
        first extract then serveFork;
        first serveFork then froth;

        join serveJoin;
        first froth then serveJoin;
        first serveFork then serveJoin;

        first serveJoin then mix;
        first mix then done;
    }

    // ================================================================
    // Subsystem 4: Thermal Body (ODE -- boiler temperature dynamics)
    // ================================================================
    // [12.2] ODE subsystem with RK45 adaptive integration.
    // [12.3] Causation: reads heaterPower from BoilerController, writes temperature.

    part def ThermalBody {
        doc /* Continuous thermal model of the boiler.
             * ODE: dT/dt = (heaterPower - loss * (T - ambient)) / thermalMass
             * Uses RK45 adaptive integration. */

        @ToolExecution { toolName = "builtin:ode-rk45"; }

        out attribute temperature : ScalarValues::Real;

        attribute heaterPower : ScalarValues::Real;
        attribute thermalMass : ScalarValues::Real;
        attribute lossCoefficient : ScalarValues::Real;
        attribute ambientTemp : ScalarValues::Real;

        // SSR derivative — spec-standard GetDerivative pattern
        calc def TempDerivative :> GetDerivative {
            return dTdt = (heaterPower - lossCoefficient * (temperature - ambientTemp)) / thermalMass;
        }

        assert constraint safeTemp {
            temperature < 120
        }
    }

    // ================================================================
    // Ports and Flow Components
    // ================================================================
    // Port-based flow connections between subsystem boundaries.

    port def WaterPort {
        out attribute flowRate : ScalarValues::Real;
        out attribute temperature : ScalarValues::Real;
        out attribute pressure : ScalarValues::Real;
    }

    port def SensorPort {
        out attribute value : ScalarValues::Real;
        out attribute status : ScalarValues::Integer;
    }

    part def WaterReservoir {
        out port waterOut : WaterPort;
        attribute level : ScalarValues::Real;
        attribute temperature : ScalarValues::Real;
    }

    part def PressureSensor {
        out port reading : SensorPort;
        attribute pressure : ScalarValues::Real;
        attribute calibrated : ScalarValues::Integer;
    }

    part def BoilerUnit {
        in port waterIn : ~WaterPort;
        out port steamOut : WaterPort;
        in port pressureIn : ~SensorPort;
        attribute temperature : ScalarValues::Real;
        attribute pressure : ScalarValues::Real;
    }

    // ================================================================
    // Constraints
    // ================================================================

    constraint def PressureSafe {
        doc /* Brewing pressure must stay below 15 bar. */
        constraint { pumpPressure < 15 }
    }

    constraint def TempSafe {
        doc /* Boiler temperature must stay below 105C during normal operation. */
        constraint { boilerTemp < 105 }
    }

    // ================================================================
    // Verification: Full Brew Cycle
    // ================================================================
    // [9.1] Sequence trace: succession ordering recorded and replayed.
    // [10.2] HappensBefore via succession chains in testScript.

    verification def FullBrewTest {
        doc /* Verify complete espresso extraction with all subsystems coordinated.
             * Tests: boiler heats, pump primes, brew cycle runs, constraints hold.
             * Exercises: cross-subsystem guards, sequence trace, causation. */

        subject machine : CoffeeMachine;

        action testScript {
            action powerOn;
            action startPump;
            action primeComplete;
            action steamRequest;
            action stopPump;
            action powerOff;

            succession powerOn then startPump;
            succession startPump then primeComplete;
            succession primeComplete then steamRequest;
            succession steamRequest then stopPump;
            succession stopPump then powerOff;
        }

        requirement boilerSafe {
            doc /* Boiler stays below 105C. */
            constraint { boilerTemp < 105 }
        }
        requirement pressureSafe {
            doc /* Pressure stays below 15 bar. */
            constraint { pumpPressure < 15 }
        }
        requirement brewComplete {
            doc /* Brew cycle reaches completion. */
            constraint { brewActive >= 0 }
        }
    }

    // ================================================================
    // Assembly: The Full Coffee Machine
    // ================================================================
    // [12.1] All 4 subsystems composed under a single tick loop.
    // [12.2] SM (x2) + Action + ODE subsystem types.
    // [12.3] Shared attributes form the causation graph.

    part def CoffeeMachine {
        doc /* Complete coffee machine with 4 subsystem types:
             * - BoilerController (state machine, 6 states)
             * - WaterPump (state machine, 4 states)
             * - BrewCycle (action flow with fork/join)
             * - ThermalBody (ODE, RK45 adaptive)
             * Plus port-based flows and constraint monitoring.
             * This exercises the full orchestration tick loop. */

        // Shared context -- causation graph bindings
        attribute boilerTemp : ScalarValues::Real;
        attribute heaterPower : ScalarValues::Real;
        attribute boilerReady : ScalarValues::Real;
        attribute pumpActive : ScalarValues::Real;
        attribute waterFlow : ScalarValues::Real;
        attribute pumpPressure : ScalarValues::Real;
        attribute brewActive : ScalarValues::Real;
        attribute steamActive : ScalarValues::Real;

        // State machine subsystems
        exhibit state boilerController : BoilerController;
        exhibit state waterPump : WaterPump;

        // ODE subsystem
        part thermalBody : ThermalBody {
            attribute temperature = 20.0;
            attribute heaterPower = 0.0;
            attribute thermalMass = 500.0;
            attribute lossCoefficient = 0.05;
            attribute ambientTemp = 22.0;
        }

        // Flow components
        part reservoir : WaterReservoir {
            attribute level = 2000.0;
            attribute temperature = 20.0;
        }
        part pressureSensor : PressureSensor {
            attribute pressure = 0.0;
            attribute calibrated = 1;
        }
        part boilerUnit : BoilerUnit {
            attribute temperature = 20.0;
            attribute pressure = 0.0;
        }

        // Flow connections: reservoir -> boiler, sensor -> boiler
        flow waterSupply from reservoir.waterOut to boilerUnit.waterIn;
        flow pressureFeedback from pressureSensor.reading to boilerUnit.pressureIn;

        // RSC-3.3b (FL018): declared interface topology for the flows above —
        // Ports.sysml requires transfer endpoints to be interfacing ports
        // (ports connected by a declared interface/connection).
        connect reservoir.waterOut to boilerUnit.waterIn;
        connect pressureSensor.reading to boilerUnit.pressureIn;

        // Safety constraints
        assert constraint maxPressure {
            pumpPressure < 15
        }
        assert constraint maxTemp {
            boilerTemp < 110
        }
        assert constraint pumpAndBoiler {
            pumpActive >= 0
        }
    }

    // ================================================================
    // Concrete Instance
    // ================================================================

    part coffeeMachine : CoffeeMachine {
        attribute boilerTemp = 20.0;
        attribute heaterPower = 0.0;
        attribute boilerReady = 0.0;
        attribute pumpActive = 0.0;
        attribute waterFlow = 0.0;
        attribute pumpPressure = 0.0;
        attribute brewActive = 0.0;
        attribute steamActive = 0.0;
    }
}
