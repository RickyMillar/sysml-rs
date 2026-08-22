// =============================================================================
// DEMO: Phase 14 — External Solver Plugin (Analysis Cases)
// =============================================================================
//
// This demonstrates the solver plugin architecture via SysML v2 analysis defs.
// Analysis cases define parametric problems that the solver registry dispatches
// to registered solver implementations.
//
// TRY:
//   sysml analysis thermalCheck the-book/examples/coffee-machine/demo-analysis.sysml
//
//   Name the *usage* (thermalCheck), not the definition (ThermalAnalysis). Both are
//   accepted, but the definition has no bound values, so it reports every parameter
//   as unknown with nothing assigned. The usage is where the numbers live.
//
// EXPECTED OUTPUT (as of the current tool -- re-check if it changes):
//   Analysis: thermalCheck
//     Parameters:
//       waterVolume : unknown (in) = Int(250)
//       chamberHeatLoss : unknown (in) = Int(50)
//       heaterPower : unknown (in) = Int(1400)
//       ambientTemp : unknown (in) = Int(20)
//     Outputs:
//       ambientTemp = Int(20)
//       chamberHeatLoss = Int(50)
//       heaterPower = Int(1400)
//       waterVolume = Int(250)
//     Status: CONVERGED
//     Iterations: 1
//
//   Two caveats. The parameter *type* prints as `unknown` even though the model
//   types these, and `--set <name>=<value>` is currently ignored without warning:
//   `--set ambientTemp=25` still reports Int(20). Do not use `--set` to explore a
//   parameter space until that is fixed.
//
// SOLVER NOTES:
//   bind X = Y creates an equality constraint that the propagation solver uses
//   to seed known values through the parameter network.  The solver reads all
//   in-direction attributes, propagates via bind connectors, then evaluates
//   assert constraints.  When the solver is not yet available (sysml check),
//   constraints on directly-assigned in attributes still evaluate fully.
//
// =============================================================================

package CoffeeMachineAnalysis {

    // --- Supporting part definitions with thermal properties ---

    part def Heater {
        doc /* Resistive heating element with rated power and efficiency. */
        attribute power : ScalarValues::Real;        // watts
        attribute efficiency : ScalarValues::Real;   // 0.0 to 1.0
    }

    part def BrewChamber {
        doc /* Pressurized brewing chamber with thermal mass. */
        attribute volume : ScalarValues::Real;       // millilitres
        attribute heatLoss : ScalarValues::Real;     // watts lost to environment
    }

    // --- Thermal steady-state analysis ---
    //
    // Models whether the supplied heater power and ambient conditions fall
    // within the operating envelope for a valid brew.
    //
    // Constraints on the in-direction attributes evaluate immediately with
    // sysml check.  The bind connectors seed derived attributes (steadyStateTemp,
    // brewTemp) for use when the builtin:propagation solver is running.

    // Solver plugin annotation — per SysML v2 spec AnalysisTooling.sysml.
    // The SolverRegistry matches toolName to a registered SolverPlugin.
    metadata thermalToolConfig : ToolExecution about ThermalAnalysis {
        attribute toolName = "builtin:propagation";
        attribute uri = "sysml-rs://solver/propagation";
    }

    analysis def ThermalAnalysis {
        doc /* Steady-state thermal analysis of the coffee machine boiler. */

        // Input parameters — caller supplies these
        in attribute heaterPower : ScalarValues::Real;       // watts
        in attribute ambientTemp : ScalarValues::Real;       // degrees Celsius
        in attribute chamberHeatLoss : ScalarValues::Real;   // watts
        in attribute waterVolume : ScalarValues::Real;       // millilitres

        // Derived values — solved via propagation bindings
        attribute steadyStateTemp : ScalarValues::Real;
        attribute brewTemp : ScalarValues::Real;

        // Binding connectors: equalities for the propagation solver.
        // Seed derived attributes from the supplied inputs.
        bind steadyStateTemp = ambientTemp;
        bind brewTemp = heaterPower;

        // Constraints on the input parameters (evaluate with sysml check)
        assert constraint heaterInRange {
            heaterPower >= 1000
        }
        assert constraint heaterMaxPower {
            heaterPower <= 2000
        }
        assert constraint ambientRealistic {
            ambientTemp >= 0
        }
        assert constraint volumePositive {
            waterVolume > 0
        }
    }

    // --- Concrete analysis instance with nominal values ---

    analysis thermalCheck : ThermalAnalysis {
        in attribute heaterPower = 1400;
        in attribute ambientTemp = 20;
        in attribute chamberHeatLoss = 50;
        in attribute waterVolume = 250;
    }

    // --- Trade study: compare two heater configurations ---

    analysis def HeaterTradeStudy {
        doc /* Compare two heater power configurations for thermal viability. */

        // Configuration inputs
        in attribute config1Power : ScalarValues::Real;   // watts
        in attribute config2Power : ScalarValues::Real;   // watts

        // Derived temperatures for each configuration
        attribute config1Temp : ScalarValues::Real;
        attribute config2Temp : ScalarValues::Real;

        // Propagation seeds
        bind config1Temp = config1Power;
        bind config2Temp = config2Power;

        // Both configurations must be within rated limits
        assert constraint config1InBudget {
            config1Power <= 2000
        }
        assert constraint config2InBudget {
            config2Power <= 2000
        }
        assert constraint config1Positive {
            config1Power > 0
        }
        assert constraint config2Positive {
            config2Power > 0
        }
    }

    // --- Concrete trade study instance ---

    analysis compareHeaters : HeaterTradeStudy {
        in attribute config1Power = 1200;
        in attribute config2Power = 1400;
    }

    // --- Power budget rollup analysis ---
    //
    // Verifies that each subsystem power draw is within expected bounds and
    // that the heater draw (the dominant consumer) does not exceed the rated
    // circuit breaker limit.

    analysis def PowerBudgetAnalysis {
        doc /* Verify power budget across heater, grinder, and pump subsystems. */

        in attribute heaterDraw : ScalarValues::Real;   // watts
        in attribute grinderDraw : ScalarValues::Real;  // watts
        in attribute pumpDraw : ScalarValues::Real;     // watts

        attribute totalDraw : ScalarValues::Real;

        // Seed totalDraw from the dominant consumer for propagation
        bind totalDraw = heaterDraw;

        // Heater draw must not exceed rated circuit breaker limit
        assert constraint heaterUnderRatedLimit {
            heaterDraw < 2000
        }
        // Individual draws must be non-negative
        assert constraint grinderNonNegative {
            grinderDraw >= 0
        }
        assert constraint pumpNonNegative {
            pumpDraw >= 0
        }
        // Grinder draw stays within rated spec
        assert constraint grinderInSpec {
            grinderDraw <= 500
        }
    }

    analysis powerCheck : PowerBudgetAnalysis {
        in attribute heaterDraw = 1400;
        in attribute grinderDraw = 200;
        in attribute pumpDraw = 65;
    }
}
