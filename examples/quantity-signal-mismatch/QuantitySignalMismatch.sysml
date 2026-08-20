// RSC-5.2 proving fixture — UQ002 (cross-dimension signal link).
//
// UQ002 is the signal-seam twin of UQ001 (binding connector) and UQ003
// (constraint comparison): a SignalLink whose source and target port-feature
// slots carry incompatible (non-zero, non-equal) ISQ dimensions is a hard
// error — the boundary has no meaningful conversion (same-dimension scale
// differences are RSC-5.3 auto-conversion, not a mismatch).
//
// No cross-dimension signal link exists anywhere else in the corpus (the
// exchange-plane fixture's `reading` is a plain `Reading` item, dimensionless),
// so this fixture deliberately authors one. The signal ports carry a single
// directed ISQ ATTRIBUTE feature `reading` (reinforced by @Signal) — an
// incomplete effort/flow pair classifies as a SignalLink, and the ISQ-typed
// AttributeUsage is what `infer_m_ref` stamps onto the minted slot so the
// cross-dim check has facts to act on. ISQ types resolve by name against the
// runtime ISQ table (no stdlib load required), like the other rsc5 fixtures.
package QuantitySignalMismatch {

    metadata def Signal;

    // Source signal port: emits a current-dimensioned `reading`.
    port def CurrentOut {
        @Signal;
        out attribute reading : ISQ::ElectricCurrentValue;
    }

    // Target signal port: expects a temperature-dimensioned `reading`. SAME
    // feature name, DIFFERENT dimension — the cross-dim mismatch UQ002 catches.
    port def TempIn {
        @Signal;
        in attribute reading : ISQ::ThermodynamicTemperatureValue;
    }

    part def SensorDef {
        port currentOut : CurrentOut;
    }

    part def MonitorDef {
        port tempIn : TempIn;

        // A trivial state machine so the workspace orchestrator has an
        // executable subsystem to build (the signal-propagation pass — where
        // UQ002 is emitted — runs during that build).
        state def MonitorLogic {
            entry; then watching;
            state watching;
        }
    }

    part def System {
        part sensor : SensorDef;
        part monitor : MonitorDef;

        // The cross-dimension signal link (flow + declared connector, the
        // corpus's flow+connect shape; the propagation plane dedups the two
        // distinct links to one logical path, L29/L30).
        flow from sensor.currentOut to monitor.tempIn;
        connect sensor.currentOut to monitor.tempIn;
    }
}
