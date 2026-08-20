package ValveGating {
    doc /* Flow gating via state machine.
     *
     * A Valve has a two-state SM: open → closed (on close_valve event).
     * When the SM enters "closed", the downstream flow gates to zero.
     *
     * A Pipe connects source pressure through the Valve to a sink.
     * Downstream pressure = upstream_pressure when valve is open, 0 when closed.
     *
     * Spec reference: Ports.sysml, Flows.sysml, StatePerformances.kerml */

    private import ISQ::*;
    private import SI::*;
    private import ScalarValues::*;

    // ── Item types that flow through the pipe ─────────────────────────

    item def PressureFlow {
        doc /* Pressurised fluid flowing through a pipe segment. */
        attribute pressure : PressureValue;
        attribute flow_rate : VolumeFlowRateValue;
    }

    // ── Port definitions ──────────────────────────────────────────────

    port def FlowOutPort {
        doc /* Upstream / source end of a pipe segment. */
        // `flow` is a SysML keyword; escape it to use as a feature name.
        out item 'flow' : PressureFlow;
    }

    port def FlowInPort {
        doc /* Downstream / sink end of a pipe segment. */
        in item 'flow' : PressureFlow;
    }

    // ── Valve state machine ───────────────────────────────────────────

    state def ValveSM {
        doc /* Two-state valve controller.
         * Starts open; transitions to closed on "close_valve" event.
         * The runtime treats "closed" as a terminal gating state —
         * any flow expression registered against this SM gates to zero. */

        state open;
        state closed;

        entry; then open;

        transition open_to_closed
            first open
            accept when true
            then closed;
    }

    // ── Valve part ────────────────────────────────────────────────────

    part def Valve {
        doc /* Electrically-actuated ball valve.
         * State machine ValveSM drives open/closed position.
         * When SM enters "closed", the runtime flow gate zeroes downstream flow. */

        port inlet  : FlowInPort;
        port outlet : FlowOutPort;

        // Upstream supply pressure (Pa)
        attribute upstream_pressure : PressureValue default 100.0;

        // Downstream pressure — equals upstream when open, zero when closed
        attribute downstream_pressure : PressureValue default 100.0;
    }

    // ── Pipe part ─────────────────────────────────────────────────────

    part def Pipe {
        doc /* Simple pipe segment connecting source, valve, and sink.
         * Demonstrates that structural flow connections propagate through
         * the valve's gating state. */

        // Three nodes: source → valve → sink
        part source_node : Valve;
        part gate_valve  : Valve;

        port source_out : FlowOutPort;
        port sink_in    : FlowInPort;

        // Flow path: source → valve inlet → valve outlet → sink
        flow from source_out to gate_valve.inlet;
        flow from gate_valve.outlet to sink_in;

        // RSC-3.3b (FL018): declared interface topology for the flows above —
        // Ports.sysml requires transfer endpoints to be interfacing ports
        // (ports connected by a declared interface/connection).
        connect source_out to gate_valve.inlet;
        connect gate_valve.outlet to sink_in;

        // Upstream supply pressure (propagated from source)
        attribute upstream_pressure : PressureValue default 100.0;
    }
}
