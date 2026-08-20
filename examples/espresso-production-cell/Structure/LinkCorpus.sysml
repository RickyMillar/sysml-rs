package LinkCorpus {
    doc /* Purpose-built classified link corpus (COV EX-POWER / EX-SIGNAL /
     * EX-MSG, CELL-14). A small self-contained system that exercises all three
     * exchange-plane classes with name-matched, reconcilable typed port
     * features, plus intentional negatives — the same idiom as
     * examples/exchange-plane-fixture, in espresso terms:
     *
     *   PowerBond      pump.hydOut (pressure+flow) -> head.hydIn
     *   SignalLink     gauge.pressureOut.reading   -> monitor.pressureIn.reading
     *   MessageChannel dispatcher.brewOut.cmd      -> group.brewIn.cmd  (SM send)
     *
     * Every payload flow rides on a DECLARED connector so the topology contract
     * is satisfied (flow + connect per pair, as the corpus classifier expects).
     *
     * INTENTIONAL NEGATIVE: `monitor.tempIn` is fed from `gauge.pressureOut`, a
     * dimension-mismatched signal (a pressure reading into a temperature port).
     * The classifier still forms the link but the mismatch is the cross-dimension
     * signal case the EX-SIGNAL oracle covers. */

    private import EspressoTypes::*;
    private import EspressoInterfaces::*;

    // -- PowerBond endpoints (effort/flow conjugate pair) ------------------
    part def PumpEnd { port hydOut : HydraulicSupplyPort; }
    part def HeadEnd { port hydIn : HydraulicSupplyInPort; }

    // -- SignalLink endpoints (single directed reading, @Signal) -----------
    part def Gauge { port pressureOut : PressureSignalOut; }
    part def Monitor {
        port pressureIn : PressureSignalIn;
        port tempIn : TemperatureSignalIn;   // fed by the mismatched negative
    }

    // -- MessageChannel endpoints (command item via SM send) ---------------
    part def Dispatcher {
        port brewOut : BrewCommandOut;
        // the send that drives the MessageChannel classification
        action emitBrew { send BrewCommand() via brewOut; }
        state def LinkDispatchLogic {
            entry; then idle;
            state idle;
            transition first idle if true then dispatching;
            state dispatching { entry send BrewCommand() via brewOut; }
        }
    }
    part def Group { port brewIn : BrewCommandIn; }

    // -- the wired corpus --------------------------------------------------
    part def LinkSystem {
        part pump : PumpEnd;
        part head : HeadEnd;
        part gauge : Gauge;
        part monitor : Monitor;
        part dispatcher : Dispatcher;
        part group : Group;

        // PowerBond: pressure+flow conjugate pair
        flow from pump.hydOut to head.hydIn;
        connect pump.hydOut to head.hydIn;

        // SignalLink: pressure reading, name-matched `reading` feature
        flow from gauge.pressureOut to monitor.pressureIn;
        connect gauge.pressureOut to monitor.pressureIn;

        // MessageChannel: brew command via SM send
        flow from dispatcher.brewOut to group.brewIn;
        connect dispatcher.brewOut to group.brewIn;

        // INTENTIONAL NEGATIVE (cross-dimension signal): a pressure reading
        // routed into a temperature measurement port. It still classifies as a
        // SignalLink (classification is structural), but the payload dimension
        // does not reconcile — the EX-SIGNAL cross-dimension case. The link gate
        // pins that this does not corrupt the distribution or raise an Unknown.
        flow from gauge.pressureOut to monitor.tempIn;
        connect gauge.pressureOut to monitor.tempIn;
    }

    part linkSystem : LinkSystem;
}
