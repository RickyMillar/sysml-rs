package ExchangePlaneFixture {
    private import ISQ::*;
    private import SI::*;
    doc /* RSC-3.5 corpus fixture — the real-data target the classified
     * ExchangePlane (ADR-017 Phase 3.5) is built against.
     *
     * Purpose: exercise ALL THREE link classes with NAME-MATCHED, reconcilable
     * typed port features — the property the legacy multi-circuit fixture cannot
     * provide because its port instances carry zero runtime features (ledger L23/L25).
     *
     *   SignalLink     sensor.currentOut.reading -> controller.currentIn.reading
     *   MessageChannel controller.tripOut.cmd    -> breaker.tripIn.cmd  (SM send)
     *   PowerBond      supply.powerOut (v/i)     -> breaker.powerIn (v/i)
     *
     * Every flow routes through a declared `connect` so the FL018 interface-
     * topology contract is satisfied.
     *
     * MANDATORY 2-instance case (blueprint risk #1, L19 def/usage
     * reconciliation): TWO controllers `controllerA`/`controllerB` of one
     * `ControllerDef`, each wired to its own sensor + breaker. Endpoint
     * resolution must yield per-USAGE identity, not shared-DEF identity. */

    // ── Declared-role metadata (RSC-1.2) ──────────────────────────────────
    // @Signal forces signal classification via the declared > inferred
    // precedence (classify.rs (h)). No stdlib metadata def is required, but we
    // declare it locally so the `@Signal;` application resolves. (We do NOT use
    // @PowerPort — see the PowerOut/PowerIn note: it would zero the domain.)
    metadata def Signal;

    // ── Item defs (the reconcilable typed payloads) ───────────────────────
    item def Reading {
        attribute value : Real;
    }

    item def TripCommand {
        attribute tripValue : Boolean;
    }

    // The power payload: a complete electrical effort/flow conjugate pair
    // (voltage = effort, current = flow). A port carrying this item classifies
    // into the `electrical` physics domain (both effort AND flow present →
    // power, not signal) — the prerequisite for the PowerBond class. This
    // mirrors the legacy multi-circuit fixture's ACPower convention.
    item def PowerSample {
        attribute voltage : ElectricPotentialValue;
        attribute current : ElectricCurrentValue;
    }

    // ── Port defs ─────────────────────────────────────────────────────────
    // Signal ports: a single directed item feature `reading` (no conjugate
    // effort/flow pair) — classifies as SignalLink, reinforced by @Signal.
    port def SignalOut {
        @Signal;
        out item reading : Reading;
    }
    port def SignalIn {
        @Signal;
        in item reading : Reading;
    }

    // Command (message) ports: item-typed payload `cmd : TripCommand`
    // carried by SM send/accept — classifies as MessageChannel.
    port def CmdOut {
        out item cmd : TripCommand;
    }
    port def CmdIn {
        in item cmd : TripCommand;
    }

    // Power ports: carry a PowerSample (voltage effort + current flow), so the
    // ISQ-typed conjugate pair infers the `electrical` domain → power, the
    // prerequisite for PowerBond. NOTE: we deliberately do NOT add @PowerPort
    // here — the declared-power override forces domain=None (classify.rs (h)),
    // which would zero the inferred domain and defeat `is_power` (which requires
    // a resolved domain). The ISQ effort/flow inference is what yields the bond;
    // @PowerPort only suppresses signal classification, it does not synthesize a
    // domain. (PowerBond fed to the DAE machinery.)
    port def PowerOut {
        out item power : PowerSample;
    }
    port def PowerIn {
        in item power : PowerSample;
    }

    // ── Parts ───────────────────────────────────────────────────────────
    part def SensorDef {
        port currentOut : SignalOut;
    }

    part def ControllerDef {
        doc /* Two USAGES of this ONE def (controllerA/controllerB) force the
         * L19 reconciliation: per-usage endpoint identity, not shared-def. */
        port currentIn : SignalIn;
        port tripOut : CmdOut;

        attribute threshold : Real default 5.0;

        // The send that drives the MessageChannel. A `send X via port` action
        // node is what the L26 / RSC-3.5b SM-send payload extraction will read;
        // today the payload is bare (the SM-send string-projection boundary the
        // blueprint flags). Kept as an action node (the spec form an EffectBehavior
        // resolves to) rather than an inline transition `do send`, which the
        // tree-sitter grammar does not lower.
        action emitTrip { send TripCommand() via tripOut; }

        // On an over-threshold reading, transition to tripped (the guard reads
        // the typed signal-port feature reconciled across the SignalLink).
        state def TripLogic {
            entry; then monitoring;
            state monitoring;
            transition monitoring
                if currentIn.reading.value > threshold
                then tripped;
            state tripped;
        }
    }

    part def BreakerDef {
        port tripIn : CmdIn;
        port powerIn : PowerIn;
    }

    part def SupplyDef {
        port powerOut : PowerOut;
    }

    part def System {
        // Two sensors, two controllers (ONE def), two breakers + one supply.
        part sensorA : SensorDef;
        part sensorB : SensorDef;
        part controllerA : ControllerDef;
        part controllerB : ControllerDef;
        part breakerA : BreakerDef;
        part breakerB : BreakerDef;
        part supply : SupplyDef;

        // Each payload flow rides on a DECLARED connector — this is the corpus
        // invariant the RSC-3.B0 topology inventory pins at zero (every flow has
        // a declared connection; FL018 hard-error gate). The production FL018
        // diagnostic does not yet fire on an orphan flow, but rsc3_topology_inventory
        // (the authoritative gate) counts it — so flow + connect on each pair, the
        // same way the legacy multi-circuit fixture models its flows. The resulting flow-link +
        // connector-link doubling per pair is the normal corpus shape that L29 /
        // RSC-3.5e reconciles (dedup on element-id), not a fixture artifact.

        // ── SignalLink: sensor -> controller (item `reading : Reading`) ─────
        flow from sensorA.currentOut to controllerA.currentIn;
        flow from sensorB.currentOut to controllerB.currentIn;
        connect sensorA.currentOut to controllerA.currentIn;
        connect sensorB.currentOut to controllerB.currentIn;

        // ── MessageChannel: controller -> breaker (TripCommand via SM send) ─
        flow from controllerA.tripOut to breakerA.tripIn;
        flow from controllerB.tripOut to breakerB.tripIn;
        connect controllerA.tripOut to breakerA.tripIn;
        connect controllerB.tripOut to breakerB.tripIn;

        // ── PowerBond: supply -> breaker (v/i, shared supply) ───────────────
        // flow + connect (the flow carries the bond into ConnectionGraph, keeping
        // fixture_power_bond_feeds_connection_graph live; the connect is the
        // declared connector + the L18 connector-ingestion path).
        flow from supply.powerOut to breakerA.powerIn;
        connect supply.powerOut to breakerA.powerIn;
    }
}
