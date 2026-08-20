// ============================================================================
// Physics Diagnostics Demo
//
// Open this file in VS Code after reloading to see the new physics diagnostics.
// Each section demonstrates a different diagnostic.
// ============================================================================

package PhysicsDiagnosticsDemo {

    // ── Port Definitions ─────────────────────────────────────────────
    // These demonstrate PH003 (missing effort/flow pair) and code lenses.

    // GOOD: Complete electrical port (effort + flow) → no diagnostic
    // Should show code lens: "Physics: electrical (effort: voltage, flow: current)"
    port def PhasePort {
        attribute voltage : Real;
        attribute current : Real;
    }

    // GOOD: Complete thermal port
    // Should show code lens: "Physics: thermal (effort: temperature, flow: heatFlow)"
    port def ThermalPort {
        attribute temperature : Real;
        attribute heatFlow : Real;
    }

    // PH003: Thermal port with only effort (temperature) — missing flow
    // Expected diagnostic: "port 'TempProbePort' is thermal but only has effort feature"
    port def TempProbePort {
        attribute temperature : Real;
    }

    // PH003: Electrical port with only flow (current) — missing effort
    // Expected diagnostic: "port 'CurrentSensePort' is electrical but only has flow feature"
    port def CurrentSensePort {
        attribute current : Real;
    }

    // Signal port — no physics diagnostic expected
    port def CommandPort {
        attribute command : String;
    }

    // ── Well-formed electrical circuit ────────────────────────────────
    // This part should get a code lens showing "Physics: electrical" on PhasePort

    part def PowerSupply {
        port supplyOut : PhasePort;
    }

    part def BusbarAssembly {
        // PH002: If all ports are 'out', conservation imbalance
        // With proper directions (1 in + 2 out), no diagnostic
        port phaseIn : PhasePort;
        port circuitOut1 : PhasePort;
        port circuitOut2 : PhasePort;
    }

    part def Load {
        port powerIn : PhasePort;
    }

    // ── Direction conflict demo ──────────────────────────────────────
    // PH004: Two 'out' ports connected — missing conjugation

    part def DirectionConflictDemo {
        part source1 : PowerSupply;
        part source2 : PowerSupply;

        // This flow connects two 'out' ports — PH004 expected
        // (Only fires if elaboration resolves directions)
        flow of Real from source1.supplyOut to source2.supplyOut;

        // RSC-3.3b (FL018): declared topology so the demo stays scoped to
        // PH004 (the direction conflict above is preserved by the connect).
        connect source1.supplyOut to source2.supplyOut;
    }

    // ── Domain mismatch demo ─────────────────────────────────────────
    // PH001: Connecting electrical to thermal

    part def DomainMismatchDemo {
        part supply : PowerSupply;
        part heatsink {
            port heatIn : ThermalPort;
        }

        // This flow crosses domains — PH001 expected
        flow of Real from supply.supplyOut to heatsink.heatIn;

        // RSC-3.3b (FL018): declared topology so the demo stays scoped to
        // PH001 (the domain mismatch above is preserved by the connect).
        connect supply.supplyOut to heatsink.heatIn;
    }

    // ── R/C/I element detection demo ─────────────────────────────────
    // PH005: Part with resistance attribute but no constraint

    part def Resistor {
        attribute contactResistance : Real;  // Would need ISQ type for PH005
        port phaseIn : PhasePort;
        port phaseOut : PhasePort;
        // Missing: constraint { contactResistance * current = voltage_in - voltage_out }
    }

    // GOOD: Resistor with constraint — no PH005
    part def WiredResistor {
        attribute resistance : Real;
        port phaseIn : PhasePort;
        port phaseOut : PhasePort;
        constraint ohmsLaw { resistance * 1.0 == 1.0 }
    }

    // ── Hover demo ───────────────────────────────────────────────────
    // Hover over any of these port usages to see physics domain info

    part def HoverDemo {
        port electrical : PhasePort;      // Hover → "Physics: electrical domain"
        port thermal : ThermalPort;       // Hover → "Physics: thermal domain"
        port probe : TempProbePort;       // Hover → "Physics: thermal domain" (effort only)
        port signal : CommandPort;        // Hover → no physics section (signal domain)
    }
}
