package PlantSupervisorBehaviour {
    doc /* Shared-resource supervisor (COV RT-SM / SCHED-READSET, CELL-11).
     *
     * The supervisor grants bounded shared-resource permits to stations and
     * broadcasts recipe/maintenance commands. Because concurrent brew requests
     * can exceed the shared plant's capacity, arbitration MUST be deterministic.
     *
     *   ARBITRATION POLICY (stated, not incidental): permits are granted in
     *   stable station-path order (station1, station2, ... stationN). The first
     *   K requesters in that order are granted; the rest wait. This makes the
     *   outcome independent of map-iteration order and reproducible run-to-run.
     *
     * This file also carries a self-contained coordination scenario that the
     * CELL-ACT / CELL-SM gates drive: a dispatcher grants a brew permit to the
     * first station in path order and to no other, so message delivery,
     * exactly-once consumption, per-station isolation, and the deterministic
     * selection are all observable. */

    private import EspressoTypes::*;
    private import EspressoInterfaces::*;

    // Two DISTINCT receiver defs (not two usages of one def) so the messaging
    // path is not instance-multiplied — the name-keyed subsystem set stays
    // unambiguous, exactly as the port-message delivery fixtures require.

    // -- the granted head: consumes the delivered permit exactly once ------
    part def GrantedHeadDef {
        port brewInA : BrewCommandIn;
        state def GrantedLogic {
            entry; then waitingA;
            state waitingA;
            transition first waitingA accept cmd via brewInA then brewingA;
            state brewingA;
        }
    }

    // -- the ungranted head: same accept, but no flow feeds it (isolation) --
    part def UngrantedHeadDef {
        port brewInB : BrewCommandIn;
        state def UngrantedLogic {
            entry; then waitingB;
            state waitingB;
            transition first waitingB accept cmd via brewInB then brewingB;
            state brewingB;
        }
    }

    // -- the supervisor: grants ONE permit, to the first station in path order
    part def Supervisor {
        port grantOut : BrewCommandOut;
        state def SupervisorDispatchLogic {
            entry; then arbitrating;
            state arbitrating;
            transition first arbitrating if true then granting;
            // deterministic selection: grant the first path-ordered requester
            state granting { entry send BrewCommand() via grantOut; }
        }
    }

    // -- coordination scenario: supervisor -> granted head only ------------
    part def CoordinationScenario {
        part supervisor : Supervisor;
        part granted : GrantedHeadDef;     // first in path order — granted
        part ungranted : UngrantedHeadDef; // later — not granted (isolation)

        // the grant is delivered only to the granted head (deterministic tie-break)
        flow from supervisor.grantOut to granted.brewInA;
        connect supervisor.grantOut to granted.brewInA;
    }
}
