package BrewStationStructure {
    doc /* The one reusable brew-station definition, instantiated N times by
     * ProductionCell (COV RT-INSTANCE). Each station:
     *
     *   - nests `thermal : StationThermal` (the per-instance ODE — this is what
     *     makes every station an independent runtime subsystem with its own
     *     integrated temperature and qualified slot path);
     *   - computes a hydraulic branch flow from its valve setting and the
     *     signed-root orifice law (exposed for the aggregate accounting);
     *   - exposes hydraulic supply/return + thermal power ports (PowerBond),
     *     pressure/temperature/flow/phase measurement ports (SignalLink), and
     *     recipe/stop/purge/clean/permit command ports (MessageChannel);
     *   - carries local heater / valve actuator parts at COV-minimum detail;
     *   - owns the lifecycle state machine directly (see below).
     *
     * Compiler-forced structural choice: the lifecycle SM is a DIRECT state def
     * of BrewStation (not a nested controller part) because per-instance SM
     * multiplication only fires for a state def reachable as a direct child /
     * by type name. This is the documented deviation from the plan's separate
     * Behaviour/StationController file. The other behaviour packages
     * (PlantSupervisor, RecipeExecution, CleaningCycle) live under Behaviour/. */

    private import ScalarValues::*;
    private import EspressoTypes::*;
    private import EspressoInterfaces::*;
    private import StationThermalPhysics::*;

    part def BrewStation {
        // -- nested continuous physics (per-instance ODE bearer) ------------
        part thermal : StationThermal;

        // -- hydraulic branch --------------------------------------------------
        attribute valve : Real default 1.0;        // 0 = shut, 1 = full open
        attribute G : Real default 0.4;            // branch conductance
        attribute pSupply : Real default 14.0;     // nominal manifold supply pressure
        attribute pReturn : Real default 1.0;      // return-line pressure

        // branch flow via the signed-root orifice law (inlined so it evaluates
        // per instance without a cross-part calc-def reference):
        //   qBranch = valve * G * signed_root(pSupply - pReturn)
        attribute qBranch : Real =
            valve * G
            * (if pSupply - pReturn >= 0.0 ? 1.0 else 0.0 - 1.0)
            * sqrt(abs(pSupply - pReturn));

        // -- power ports (effort/flow ⇒ PowerBond) --------------------------
        port supplyIn : HydraulicSupplyInPort;
        port returnOut : HydraulicReturnPort;
        port heatIn : ThermalSinkPort;

        // -- measurement ports (single directed reading ⇒ SignalLink) -------
        port pressureOut : PressureSignalOut;
        port tempOut : TemperatureSignalOut;
        port flowOut : FlowSignalOut;
        port phaseOut : PhaseSignalOut;

        // -- command ports (discrete items ⇒ MessageChannel) ----------------
        port brewIn : BrewCommandIn;
        port stopIn : StopCommandIn;
        port purgeIn : PurgeCommandIn;
        port cleanIn : CleanCommandIn;
        port permitIn : PermitIn;

        // -- lifecycle state machine (per-instance SM bearer) ---------------
        // The station lifecycle SM is a DIRECT state def of BrewStation so the
        // runtime multiplies it per station instance (find_reachable_sms only
        // collects a state def reachable as a direct child / by type name — a
        // deeper nested SM-part is not multiplied). The command ports it accepts
        // are therefore in the same scope. See StationControllerBehaviour for
        // the reusable controller definition used elsewhere.
        state def StationLifecycle {
            entry; then idle;

            state idle;
            transition first idle if true then preheat;   // auto warm-up on power

            state preheat;
            transition preheat accept after(3.0) then ready;

            state ready;
            transition ready accept cmd via brewIn then extract;
            transition ready accept cmd via purgeIn then purge;
            transition ready accept cmd via cleanIn then cleaning;

            state extract;
            transition extract accept cmd via stopIn then ready;
            transition extract accept after(2.0) then recover;

            state recover;
            transition recover accept after(1.0) then ready;

            state purge;
            transition purge accept after(0.5) then ready;

            state cleaning;
            transition cleaning accept after(5.0) then ready;
        }

        // -- local actuator parts (COV-minimum detail) ----------------------
        part heater;
        part shutoffValve;
    }
}
