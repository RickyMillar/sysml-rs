package PumpCycleBehaviour {
    doc /* Reciprocating-cycle state machine with a latched safety path.
     *
     *   Idle -> Intake -> Compress -> Discharge -> Recover -> Intake ...
     *                                                     \-> Relieved (latched)
     *
     * The four cycle transitions are located zero-crossings of the actuator
     * state (imported here as `in attribute`s that the workspace binds to the
     * PumpDynamics ODE states of the same name):
     *
     *   Intake    -> Compress    when the stroke rises through mid  (stroke > 0)
     *   Compress  -> Discharge   when the velocity turns negative   (velocity < 0)
     *   Discharge -> Recover     when the stroke falls through mid  (stroke < 0)
     *   Recover   -> Intake      when the velocity turns positive   (velocity > 0)
     *
     * `Idle -> Intake` is the explicit initial branch. Because the actuator
     * starts at stroke = -0.5 with velocity = 0, the first located crossing is
     * `stroke > 0` (a quarter cycle in), so the cycle always begins Intake.
     *
     * The latched safety path fires from any cycle state when the exposure
     * integral crosses its trip level. `Relieved` is terminal (no outgoing
     * transition), so once entered it is never left — the first-activation time
     * is recorded once and never overwritten (an anti-chatter/debounce property
     * that the exposure integral itself provides: a brief pressure transient
     * cannot accumulate enough exposure to trip). */

    private import ScalarValues::*;

    state def PumpCycle {
        // Bound by name to the PumpDynamics ODE states / parameters at workspace
        // assembly time.
        in attribute stroke : Real;
        in attribute velocity : Real;
        in attribute exposure : Real;
        in attribute exposureTrip : Real;

        state idle;
        state intake;
        state compress;
        state discharge;
        state recover;
        state relieved;

        entry; then idle;

        transition idle_to_intake
            first idle
            then intake;

        // ── Reciprocating cycle (located actuator crossings) ─────────
        transition intake_to_compress
            first intake
            accept when stroke > 0.0
            then compress;

        transition compress_to_discharge
            first compress
            accept when velocity < 0.0
            then discharge;

        transition discharge_to_recover
            first discharge
            accept when stroke < 0.0
            then recover;

        transition recover_to_intake
            first recover
            accept when velocity > 0.0
            then intake;

        // ── Latched safety relief (exposure crossing) ────────────────
        transition intake_to_relieved
            first intake
            accept when exposure > exposureTrip
            then relieved;

        transition compress_to_relieved
            first compress
            accept when exposure > exposureTrip
            then relieved;

        transition discharge_to_relieved
            first discharge
            accept when exposure > exposureTrip
            then relieved;

        transition recover_to_relieved
            first recover
            accept when exposure > exposureTrip
            then relieved;
    }
}
