// ===========================================================================
// FIXTURE: LSP state-machine diagnostics showcase
// ===========================================================================
//
// Open this file in the editor to exercise elaboration-driven diagnostics.
//
// Expected highlights:
//   - EmptyMachine      -> SM001 (no states)
//   - MissingInitial    -> SM002 (no explicit initial), SM005 (dead-end state)
//   - NoTransitions     -> SM003 (no compiled transitions / not runnable)
//   - Disconnected      -> SM004 (unreachable Jammed), SM005 (dead-end states)
//   - OutsideEndpoint   -> SM006 (transition endpoint outside this machine)
//
// Notes:
//   - SM007 is emitted by name-targeted health checks, not whole-file publish.
//   - This fixture is intentionally "bad" for diagnostics UX testing.
//
// ===========================================================================

package StateMachineDiagnosticShowcase {

// SM001
state def EmptyMachine {
}

// SM002 + SM005
state def MissingInitial {
    state A;
    state B;

    transition go
    first A then B;
}

// SM003
state def NoTransitions {
entry; then Idle;

    state Idle;
    state Running;
}

// SM004 + SM005
state def Disconnected {
entry; then Closed;

    state Closed;
    state Opening;
    state Jammed;

    transition closed_to_opening
    first Closed then Opening;
}

// Helper machine to provide a valid state that is out-of-scope for
// OutsideEndpoint's state set.
state def ExternalTargetCarrier {
    state Ghost;
}

// SM006 (+ may also surface SM003 because no in-machine transition compiles)
state def OutsideEndpoint {
entry; then Local;

    state Local;

    transition escape
    first Local then Ghost;
}
}
