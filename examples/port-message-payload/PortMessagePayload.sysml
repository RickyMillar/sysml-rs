package PortMessagePayload {
    doc /* A-scalar payload-carrying-messages gate — the Value-carrying upgrade
     * of the SM-send wire (examples/port-message-send proved the wire routes a
     * message; this proves it carries a REAL payload Value that the receiver
     * CONSUMES in a guard).
     *
     * Two independent sender/receiver pairs run in one production orchestrator:
     *
     *   - senderTrue  `entry send true via out1`  → receiverTrue  guards `if cmd`
     *   - senderFalse `entry send false via out2` → receiverFalse guards `if cmd`
     *
     * The sender SM lowers `send <payload> via <port>` to a `port_send_ops`
     * entry on its structured action; the runtime evaluates the payload
     * expression at tick time into a `Value` and routes it as the addressed
     * MessageTransfer payload (the `send via` string is trace only — the Value
     * channel routes). The receiver's `accept cmd via <port>` trigger binds the
     * delivered payload Value to `cmd`, and the guard `if cmd` reads it.
     *
     * POSITIVE: receiverTrue gets `Bool(true)` → guard passes → `tripped`.
     * NEGATIVE: receiverFalse gets `Bool(false)` → guard fails → stays `closed`.
     *
     * The negative twin is load-bearing: it proves the payload is genuinely
     * CONSUMED in the guard, not merely delivered (both messages are delivered;
     * only the value gates the transition). Spec: the accepted transfer's
     * payload binds the accept parameter (Transfers.kerml:254-266). Distinct
     * `state def` names keep the name-keyed subsystem set unambiguous.
     *
     * Payload-TYPE conformance and structured `new T(field=value)` payloads are
     * the A-structured follow-up (steward ruling a00bc266); this scalar wire is
     * the additive base. */

    port def SigOut { out item sig : Boolean; }
    port def SigIn { in item sig : Boolean; }

    part def SenderTrueDef {
        port out1 : SigOut;
        state def SenderTrue {
            entry; then idle;
            state idle;
            transition first idle
                if true
                then sending;
            state sending {
                entry send true via out1;
            }
        }
    }

    part def SenderFalseDef {
        port out2 : SigOut;
        state def SenderFalse {
            entry; then idle;
            state idle;
            transition first idle
                if true
                then sending;
            state sending {
                entry send false via out2;
            }
        }
    }

    part def ReceiverTrueDef {
        port in1 : SigIn;
        state def GuardedTrue {
            entry; then closed;
            state closed;
            transition first closed
                accept cmd via in1
                if cmd
                then tripped;
            state tripped;
        }
    }

    part def ReceiverFalseDef {
        port in2 : SigIn;
        state def GuardedFalse {
            entry; then closed;
            state closed;
            transition first closed
                accept cmd via in2
                if cmd
                then tripped;
            state tripped;
        }
    }

    part def System {
        part senderTrue : SenderTrueDef;
        part senderFalse : SenderFalseDef;
        part receiverTrue : ReceiverTrueDef;
        part receiverFalse : ReceiverFalseDef;

        flow from senderTrue.out1 to receiverTrue.in1;
        connect senderTrue.out1 to receiverTrue.in1;

        flow from senderFalse.out2 to receiverFalse.in2;
        connect senderFalse.out2 to receiverFalse.in2;
    }
}
