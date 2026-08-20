package ActionPortMessageDelivery {
    doc /* RSC port-flow Wave B-inc-2 behavioural gate — an ACTION receiver
     * advances past `accept … via <port>` on the PRODUCTION orchestrator.
     *
     * This is the action-graph dual of port-message-send (which proved the
     * SM receiver). A fully-compiled SENDER state machine (`RelayLogic`)
     * settles into `idle`, takes its guard-only transition into `sending`,
     * and that state's entry runs `send TripCommand via tripOut`. The
     * orchestrator resolves the sender owner key (`relay`) and enqueues an
     * addressed MessageTransfer at `relay.tripOut`; the wired router (step 10b)
     * routes it along the declared flow to `receiverWired.inPort`, and GAP 2+3
     * delivery surfaces it as `port_events["recvWired"]`. The receiver is an
     * ACTION graph whose `accept cmd via inPort` node consumes that payload
     * from the unified `TickContext::port_payloads` channel and advances the
     * token past the Accept node to completion. NO `deliver_message`, NO
     * `send_to_router` — the whole send→route→deliver→accept arc runs inside
     * `build_workspace_orchestrator` + `step()`.
     *
     * `receiverUnwired` is the negative twin: same `accept cmd via inPort`
     * action, but NO flow targets its port, so its token stays blocked at the
     * Accept node (the action never completes). This proves the action advances
     * only on a genuine delivery, not unconditionally.
     *
     * The receiver actions carry DISTINCT names (`recvWired` / `recvUnwired`)
     * via distinct part defs so each reconciles to its own part-usage owner key
     * (a single action def maps to a single instantiating usage). Gate scope:
     * the action ADVANCES PAST its Accept node; CONSUMING the payload value in a
     * guard/effect is the receiver-consumes-payload follow-up (Wave-A binding). */

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

    part def ReceiverWired {
        port inPort : CmdIn;

        // The receiving ACTION: `accept cmd via inPort` lowers to an Accept
        // token-flow node with port_source = "inPort". A token parks here until
        // a payload is delivered for `inPort`, then advances to completion.
        action recvWired {
            accept cmd via inPort;
        }
    }

    part def ReceiverUnwired {
        port inPort : CmdIn;

        // Negative twin: same accept, but no flow feeds inPort → stays blocked.
        action recvUnwired {
            accept cmd via inPort;
        }
    }

    part def System {
        part relay : RelayDef;
        part receiverWired : ReceiverWired;
        part receiverUnwired : ReceiverUnwired;

        // Only receiverWired is wired to the sender. receiverUnwired's port is
        // intentionally left dangling.
        flow from relay.tripOut to receiverWired.inPort;
        connect relay.tripOut to receiverWired.inPort;
    }
}
