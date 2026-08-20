package PortMessagePayloadStructured {
    doc /* A-structured payload-carrying-messages gate — the structured upgrade
     * of A-scalar (examples/port-message-payload). A compiled sender SM sends a
     * CONSTRUCTED payload `new TripCommand(tripValue = <bool>)` whose named
     * field the receiver reads in a guard `if cmd.tripValue`.
     *
     *   - senderTrue  `send new TripCommand(tripValue = true)  via out1`
     *       → receiverTrue  guards `if cmd.tripValue` → `tripped`
     *   - senderFalse `send new TripCommand(tripValue = false) via out2`
     *       → receiverFalse guards `if cmd.tripValue` → stays `closed`
     *
     * `new T(field = value)` is a ConstructorExpression (KerMLExpressions.xtext;
     * the `new` keyword distinguishes it from a function InvocationExpression).
     * The parser preserves the named-argument NAME (`argName`); the runtime
     * lowers it to `ExprIR::ConstructorCall { named_args }` and evaluates it to
     * `Value::Map { tripValue → Bool(..) }`. The receiver's `accept cmd via
     * <port>` binds that Map to `cmd`, and member access `cmd.tripValue` (the
     * FeatureChain path over Value::Map) reads the field.
     *
     * The negative twin proves the FIELD value gates the transition: both
     * messages are delivered and both carry a Map, but only the one whose
     * `tripValue` field is true opens the guard. Spec: the accepted transfer's
     * payload binds the accept parameter (Transfers.kerml:254-266); named
     * arguments bind by parameter name (KerMLExpressions.xtext:470-485).
     *
     * Distinct state-def names keep the name-keyed subsystem set unambiguous. */

    item def TripCommand {
        attribute tripValue : Boolean;
    }

    port def CmdOut { out item cmd : TripCommand; }
    port def CmdIn { in item cmd : TripCommand; }

    part def SenderTrueDef {
        port out1 : CmdOut;
        state def SenderTrue {
            entry; then idle;
            state idle;
            transition first idle
                if true
                then sending;
            state sending {
                entry send new TripCommand(tripValue = true) via out1;
            }
        }
    }

    part def SenderFalseDef {
        port out2 : CmdOut;
        state def SenderFalse {
            entry; then idle;
            state idle;
            transition first idle
                if true
                then sending;
            state sending {
                entry send new TripCommand(tripValue = false) via out2;
            }
        }
    }

    part def ReceiverTrueDef {
        port in1 : CmdIn;
        state def StructGuardedTrue {
            entry; then closed;
            state closed;
            transition first closed
                accept cmd via in1
                if cmd.tripValue
                then tripped;
            state tripped;
        }
    }

    part def ReceiverFalseDef {
        port in2 : CmdIn;
        state def StructGuardedFalse {
            entry; then closed;
            state closed;
            transition first closed
                accept cmd via in2
                if cmd.tripValue
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
