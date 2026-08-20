package EspressoInterfaces {
    doc /* Port definitions for the three exchange planes. Classification is
     * structural (independently derived, matches COV EX-POWER/EX-SIGNAL/EX-MSG):
     *
     *   - a port carrying a HydraulicSample / ThermalSample (effort + flow
     *     conjugate pair) infers a power domain ⇒ PowerBond;
     *   - a port carrying a single directed measurement item, tagged @Signal,
     *     ⇒ SignalLink;
     *   - a port carrying a command item delivered by an SM `send … via` ⇒
     *     MessageChannel.
     *
     * The @Signal metadata is declared locally so the tag resolves without a
     * stdlib dependency (same convention as examples/exchange-plane-fixture). */

    private import EspressoTypes::*;

    // Declared-role metadata: forces signal classification for measurement
    // ports via the declared > inferred precedence.
    metadata def Signal;

    // -- Hydraulic power ports (pressure effort + volume-flow) --------------
    port def HydraulicSupplyPort {
        doc /* Pressurised supply from the manifold into a station branch. */
        out item sample : HydraulicSample;
    }
    port def HydraulicSupplyInPort {
        in item sample : HydraulicSample;
    }
    port def HydraulicReturnPort {
        doc /* Spent flow / drain back toward the reservoir. */
        out item sample : HydraulicSample;
    }
    port def HydraulicReturnInPort {
        in item sample : HydraulicSample;
    }

    // -- Thermal power ports (temperature effort + heat-flow) --------------
    port def ThermalSourcePort {
        doc /* Heat delivered from the shared thermal source to a station. */
        out item sample : ThermalSample;
    }
    port def ThermalSinkPort {
        in item sample : ThermalSample;
    }

    // -- Measurement signal ports (single directed reading) ----------------
    port def PressureSignalOut { @Signal; out item reading : PressureReading; }
    port def PressureSignalIn { @Signal; in item reading : PressureReading; }
    port def TemperatureSignalOut { @Signal; out item reading : TemperatureReading; }
    port def TemperatureSignalIn { @Signal; in item reading : TemperatureReading; }
    port def FlowSignalOut { @Signal; out item reading : FlowReading; }
    port def FlowSignalIn { @Signal; in item reading : FlowReading; }
    port def PhaseSignalOut { @Signal; out item reading : PhaseReading; }
    port def PhaseSignalIn { @Signal; in item reading : PhaseReading; }

    // -- Command message ports (discrete item payloads) --------------------
    port def BrewCommandIn { in item cmd : BrewCommand; }
    port def BrewCommandOut { out item cmd : BrewCommand; }
    port def StopCommandIn { in item cmd : StopCommand; }
    port def PurgeCommandIn { in item cmd : PurgeCommand; }
    port def CleanCommandIn { in item cmd : CleanCommand; }
    port def CleanCommandOut { out item cmd : CleanCommand; }
    port def PermitIn { in item grant : PermitGrant; }
    port def PermitOut { out item grant : PermitGrant; }
}
