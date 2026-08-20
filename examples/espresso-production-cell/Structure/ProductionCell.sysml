package ProductionCellStructure {
    doc /* The production cell at smoke scale (2 stations). This file is
     * the parameterized instantiation surface: `scripts/generate_cell.py
     * --stations N` reproduces it byte-for-byte at N=2 and emits the other
     * scales the same way (D9 — generated variants, never copy-pasted). Every
     * station is a true usage of the ONE BrewStation definition, so the runtime
     * multiplies it into independent per-instance subsystems with qualified
     * slot paths (COV RT-INSTANCE / RT-SLOT). Shared plants (hydraulic +
     * thermal) are single instances. Power planes are wired here as declared
     * connectors (PowerBond); measurement (SignalLink) and command
     * (MessageChannel) planes are wired in LinkCorpus / the supervisor.
     *
     * GENERATED — edit scripts/generate_cell.py, not this file. scale=smoke stations=2 */

    private import ScalarValues::*;
    private import BrewStationStructure::*;
    private import HydraulicPlantStructure::*;
    private import ThermalPlantStructure::*;

    part def ProductionCell {
        // -- shared plants --------------------------------------------------
        part hydraulicPlant : HydraulicPlant;
        part thermalPlant : ThermalPlant;

        // -- brew stations (repeated usages of the one BrewStation def) -----
        part station1 : BrewStation;
        part station2 : BrewStation;

        // -- power planes: shared plant <-> each station (PowerBond) ---------
        flow from hydraulicPlant.supplyOut to station1.supplyIn;
        connect hydraulicPlant.supplyOut to station1.supplyIn;
        flow from hydraulicPlant.supplyOut to station2.supplyIn;
        connect hydraulicPlant.supplyOut to station2.supplyIn;
        flow from station1.returnOut to hydraulicPlant.returnIn;
        connect station1.returnOut to hydraulicPlant.returnIn;
        flow from station2.returnOut to hydraulicPlant.returnIn;
        connect station2.returnOut to hydraulicPlant.returnIn;
        flow from thermalPlant.heatOut to station1.heatIn;
        connect thermalPlant.heatOut to station1.heatIn;
        flow from thermalPlant.heatOut to station2.heatIn;
        connect thermalPlant.heatOut to station2.heatIn;

        // -- aggregate accounting (COV VAL-CONSTR / CELL-09) ----------------
        // Explicit generated sums over the station instances; the CELL-PHYS
        // residual gate recomputes these independently from the exposed slots.
        attribute qTotal : Real = station1.qBranch + station2.qBranch;
        attribute pTotal : Real = thermalPlant.source.P_source + station1.thermal.P_heater + station2.thermal.P_heater;

        // -- conservation / envelope laws -----------------------------------
        // Manifold pressure stays inside its declared operating envelope.
        assert constraint pressureEnvelope {
            hydraulicPlant.manifold.p_m >= 1.0 and hydraulicPlant.manifold.p_m <= 20.0
        }
        // Boiler temperature stays inside a safe pressurised-boiler envelope.
        assert constraint sourceEnvelope {
            thermalPlant.source.tsource >= 20.0 and thermalPlant.source.tsource <= 130.0
        }
    }

    // -- the single top-level usage that drives orchestration ---------------
    part productionCell : ProductionCell;
}
