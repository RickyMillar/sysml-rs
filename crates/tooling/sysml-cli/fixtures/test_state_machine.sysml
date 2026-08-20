// ===========================================================================
// TEST: State machine simulation
// ===========================================================================
//
// WHAT TO LOOK FOR:
//
//   1. CODE LENS on state definition:
//      - "DoorController" (line 34) → should show "[Simulate]" button
//
//   2. COMMANDS TO TEST:
//      - Place cursor on "DoorController", run: sysml.simulate.start
//        Should create a simulation session and return session ID
//
//      - Run: sysml.simulate.step
//        Should advance to next state (e.g., Closed → Opening)
//        Output shows current state and transition taken
//
//      - Run: sysml.simulate.step (again)
//        Should advance again (Opening → Open)
//
//      - Run: sysml.simulate.reset
//        Should reset back to the initial state (Closed)
//
//      - Run: sysml.simulate.stop
//        Should end the session
//
//   3. OUTLINE:
//      - The outline panel should show DoorController as a state def
//      - Each state (Closed, Opening, Open, Closing) should appear
//      - Transitions should appear in the outline tree
//
// ===========================================================================

package StateMachineTest {

    state def DoorController {
        entry; then Closed;

        state Closed;
        state Opening;
        state Open;
        state Closing;

        transition closed_to_opening
            first Closed then Opening;

        transition opening_to_open
            first Opening then Open;

        transition open_to_closing
            first Open then Closing;

        transition closing_to_closed
            first Closing then Closed;
    }

    // Simple two-state toggle for minimal testing
    state def Toggle {
        entry; then Off;

        state Off;
        state On;

        transition turn_on
            first Off then On;

        transition turn_off
            first On then Off;
    }
}
