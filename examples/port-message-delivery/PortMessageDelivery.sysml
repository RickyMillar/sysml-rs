package PortMessageDelivery {
    doc /* RSC-3.5d behavioural baseline — the compiled-model proof that the
     * production ExchangePlane router is wired (ledger L26 / step 10b
     * `wire_message_router`).
     *
     * A sender part pushes a TripCommand out `relay.tripOut`; a receiver part
     * `breaker` runs a state machine whose transition fires on `accept cmd via
     * tripIn`. A declared MessageChannel flow connects the two. When driven
     * through `build_workspace_orchestrator` + `step()` (NOT a hand-wired
     * `set_exchange_plane`), the injected message must route through the
     * compiled router to `breaker.tripIn` and trip the SM.
     *
     * Before step 10b this was dormant: the production router was an empty
     * `ExchangePlane::new()`, so `route_pending()` had no routing table and the
     * delivery / `accept-via-port` trigger were only ever proven hand-built. */

    item def TripCommand {
        attribute tripValue : Boolean;
    }

    port def CmdOut {
        out item cmd : TripCommand;
    }
    port def CmdIn {
        in item cmd : TripCommand;
    }

    part def RelayDef {
        port tripOut : CmdOut;
    }

    part def BreakerDef {
        port tripIn : CmdIn;

        // The receiving state machine: `accept cmd via tripIn` lowers to a
        // PortMessage trigger (L38), surfacing `tripIn` as an accept port that
        // the orchestrator registers as the acceptor `{breaker}.tripIn` on the
        // ExchangePlane.
        state def Logic {
            entry; then closed;
            state closed;
            transition first closed
                accept cmd via tripIn
                then tripped;
            state tripped;
        }
    }

    part def System {
        part relay : RelayDef;
        part breaker : BreakerDef;

        // The MessageChannel rides on a declared connector (FL018 topology
        // contract), the same shape as the exchange-plane fixture.
        flow from relay.tripOut to breaker.tripIn;
        connect relay.tripOut to breaker.tripIn;
    }
}
