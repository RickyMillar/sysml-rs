package PortMessageSend {
    doc /* L26 SEND-half end-to-end gate (the dual of port-message-delivery).
     *
     * port-message-delivery proved the RECEIVE half: an externally-injected
     * message (`send_to_router("relay.tripOut")`) routes to `breaker.tripIn`
     * and trips the receiver SM's `accept cmd via tripIn` trigger. This model
     * proves the SEND half: a fully-compiled SENDER SM emits the message
     * itself, with NO `send_to_router` injection.
     *
     * The relay SM (`RelayLogic`) settles into `idle`, takes the guard-only
     * transition into `sending`, and that state's entry action runs
     * `send TripCommand via tripOut`. The SM compiler lowers the `via tripOut`
     * to the canonical `send via tripOut` string; the orchestrator decodes it,
     * resolves the sender's owner-instance key (`relay`, the inverse of the
     * GAP 2 `owner_to_subsystems` map) and enqueues an addressed MessageTransfer
     * at `relay.tripOut`. The wired router (step 10b) routes it along the
     * declared flow to `breaker.tripIn`, and GAP 2+3 delivery trips the
     * breaker SM `closed -> tripped`.
     *
     * The two SMs carry DISTINCT state-def names (`RelayLogic` vs `Logic`) so
     * the subsystem set is unambiguous (subsystem_states is name-keyed, and the
     * owner-key inverse relies on that uniqueness). Payload is carried as
     * Value::Null — payload-type conformance is the receiver-consumes-payload
     * wave (Transfers.kerml MessageTransfer is disjoint from FlowTransfer;
     * isMove/payload-binding are not this pass). */

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

        state def RelayLogic {
            entry; then idle;
            state idle;
            transition first idle
                if true
                then sending;
            state sending {
                entry send TripCommand via tripOut;
            }
        }
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
        part relay : RelayDef;
        part breaker : BreakerDef;

        flow from relay.tripOut to breaker.tripIn;
        connect relay.tripOut to breaker.tripIn;
    }
}
