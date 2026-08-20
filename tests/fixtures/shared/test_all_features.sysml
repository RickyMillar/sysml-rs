// ===========================================================================
// TEST: All LSP features in one file (quick smoke test)
// ===========================================================================
//
// This file exercises every major LSP feature. Open it in Zed and check:
//
// ┌─────────────────────────────────────────────────────────────────────┐
// │ FEATURE              │ WHERE TO LOOK              │ EXPECTED        │
// ├─────────────────────────────────────────────────────────────────────┤
// │ Syntax highlighting  │ Everywhere                 │ Colored keywords│
// │ Hover (definition)   │ Hover "Rocket"             │ "part def"      │
// │ Hover (usage)        │ Hover "thrust" (line 46)   │ "attribute"     │
// │ Hover (keyword)      │ Hover "state" keyword      │ Syntax doc      │
// │ Go to definition     │ Ctrl+click "Engine" usage  │ Jumps to def    │
// │ Find references      │ Right-click "Rocket" → refs│ All usages      │
// │ Outline              │ Open outline panel          │ Tree of defs    │
// │ Completions          │ Type "part " inside package │ Suggestions     │
// │ Folding              │ Click fold arrow on package │ Collapses block │
// │ Diagnostics (PASS)   │ "thrustCheck" (line 50)    │ No squiggly     │
// │ Diagnostics (FAIL)   │ "fuelLimit" (line 51)      │ WARNING squiggly│
// │ Code lens (PASS)     │ Above "thrustCheck"         │ "PASS" lens     │
// │ Code lens (FAIL)     │ Above "fuelLimit"           │ "FAIL" lens     │
// │ Code lens (Simulate) │ Above "FlightMode"          │ "[Simulate]"    │
// │ Code lens (Action)   │ Above "Launch"              │ "[Run Action]"  │
// │ Code lens (Verify)   │ Above "PreflightCheck"      │ "[Run Verify]"  │
// │ Evaluate command     │ Cursor on "thrust", evaluate│ Shows "9500"    │
// │ What-if command      │ Cursor on "fuel", whatif    │ Flip fuelLimit  │
// │ Rename               │ Rename "rocket" usage       │ Updates all refs│
// │ Semantic tokens      │ Check token coloring        │ Types vs usages │
// └─────────────────────────────────────────────────────────────────────┘
//
// COMMANDS REFERENCE (Zed: Cmd+Shift+P → "execute command"):
//
//   sysml.evaluate          - Evaluate element at cursor
//   sysml.verify            - Run verification case at cursor
//   sysml.workspace.verify  - Run all verification cases
//   sysml.simulate.start    - Start state machine simulation
//   sysml.simulate.step     - Advance simulation one step
//   sysml.simulate.stop     - End simulation session
//   sysml.simulate.reset    - Reset simulation to initial state
//   sysml.action.run        - Run action to completion
//   sysml.action.start      - Start step-by-step action execution
//   sysml.action.step       - Execute one action step
//   sysml.action.stop       - End action session
//   sysml.action.reset      - Reset action to beginning
//   sysml.action.visualize  - Export action as DOT graph
//   sysml.flow.visualize    - Export flows as DOT graph
//   sysml.whatif             - What-if analysis on variable at cursor
//   sysml.whatif.sweep       - Sweep parameter across range
//   sysml.cache.clear       - Clear library cache
//   sysml.cache.status       - Show library cache info
//   sysml.cache.rebuild      - Force rebuild library cache
//
// ===========================================================================

package AllFeaturesTest {
    private import ScalarValues::Real;


    // --- Definitions ---

    part def Engine {
        attribute thrust : Real;
        attribute fuelConsumption : Real;
    }

    part def Rocket {
        attribute fuel : Real;
        attribute altitude : Real;
    }

    // --- State machine (test: Simulate) ---

    state def FlightMode {
        entry; then Grounded;

        state Grounded;
        state Ascending;
        state Orbiting;
        state Descending;

        transition launch
            first Grounded then Ascending;
        transition reach_orbit
            first Ascending then Orbiting;
        transition deorbit
            first Orbiting then Descending;
        transition land
            first Descending then Grounded;
    }

    // --- Action (test: Run Action) ---

    action def Ignite;
    action def Throttle;
    action def Separate;

    action def Launch {
        perform step1 : Ignite;
        perform step2 : Throttle;
        perform step3 : Separate;
    }

    // --- Part with constraints (test: Diagnostics + What-If) ---

    part rocket : Rocket {
        attribute fuel = 30;
        attribute altitude = 0;

        part engine : Engine {
            attribute thrust = 9500;
            attribute fuelConsumption = 120;
        }

        // PASS: thrust(9500) > 5000
        assert constraint thrustCheck { thrust > 5000 }

        // FAIL: fuel(30) < 50 is FALSE → diagnostic here
        assert constraint fuelLimit { fuel > 50 }

        // PASS: altitude(0) >= 0
        constraint altitudeCheck { altitude >= 0 }
    }

    // --- Verification (test: Run Verification) ---

    verification def PreflightCheck {
        // Verifies the rocket is ready for launch
    }
}
