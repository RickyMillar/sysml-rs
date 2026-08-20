package PortMessageDeliveryMulti {
    doc /* GAP 2 multi-instance regression (ledger L26).
     *
     * Two `part breaker1 / breaker2 : BreakerDef` are TWO usages of one
     * `BreakerDef` in one container, so instance multiplication produces the
     * prefixed subsystems `breaker1.Logic` and `breaker2.Logic`. The compiler's
     * owner-instance map keys each on its prefix (`breaker1 → [breaker1.Logic]`,
     * `breaker2 → [breaker2.Logic]`), so a message routed to `breaker1.tripIn`
     * must trip ONLY `breaker1.Logic` — `breaker2.Logic` stays `closed`.
     *
     * This is the only fixture exercising owner-key fan-out under multiplication
     * together with the `port_events.clear()` lifecycle trap: a delivery to one
     * instance must not leak to its sibling. The two relays are two usages of
     * `RelayDef` (no SM/ODE → NOT multiplied) and exist only as routing sources. */

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
        part relay1 : RelayDef;
        part relay2 : RelayDef;
        part breaker1 : BreakerDef;
        part breaker2 : BreakerDef;

        flow from relay1.tripOut to breaker1.tripIn;
        connect relay1.tripOut to breaker1.tripIn;
        flow from relay2.tripOut to breaker2.tripIn;
        connect relay2.tripOut to breaker2.tripIn;
    }
}
