// ===========================================================================
// TEST: Action execution + visualization
// ===========================================================================
//
// WHAT TO LOOK FOR:
//
//   1. CODE LENS on action definition:
//      - "BrewCoffee" (line 35) → should show "[Run Action]" button
//
//   2. COMMANDS TO TEST:
//
//      a) Quick run (runs entire action, returns trace):
//         - Place cursor on "BrewCoffee", run: sysml.action.run
//         - Should show full execution trace in output:
//           Step 0: [Initial] → Step 1: [Perform] grindBeans → ...
//
//      b) Step-by-step execution:
//         - Run: sysml.action.start   → creates session
//         - Run: sysml.action.step    → executes one node
//         - Run: sysml.action.step    → next node
//         - Run: sysml.action.step    → continue...
//         - Run: sysml.action.reset   → restart from beginning
//         - Run: sysml.action.stop    → end session
//
//      c) Visualization:
//         - Run: sysml.action.visualize
//         - Should return DOT graph of the action flow
//         - Copy the DOT output to graphviz to see the diagram
//
//   3. OUTLINE:
//      - "BrewCoffee" should appear as action def in the outline
//      - Each perform action should appear nested under it
//
// ===========================================================================

package ActionTest {

    action def GrindBeans {
        in item beans;
        out item grounds;
    }

    action def HeatWater {
        in item water;
        out item hotWater;
    }

    action def PourOver {
        in item grounds;
        in item hotWater;
        out item coffee;
    }

    action def BrewCoffee {
        in item beans;
        in item water;
        out item coffee;

        // Sequential action steps
        perform grindBeans : GrindBeans;
        perform heatWater : HeatWater;
        perform pourOver : PourOver;
    }
}
