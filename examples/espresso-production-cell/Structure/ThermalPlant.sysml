package ThermalPlantStructure {
    doc /* The shared thermal plant: a single boiler / thermal source whose
     * temperature state (ThermalSourceDynamics) feeds every station's supply
     * loop through thermal power ports. One instance ⇒ one shared subsystem. */

    private import ScalarValues::*;
    private import EspressoInterfaces::*;
    private import ThermalSourcePhysics::*;

    part def ThermalPlant {
        // the shared source-temperature state
        part source : ThermalSourceDynamics;

        // heat delivered out to each station's thermal sink port
        port heatOut : ThermalSourcePort;
    }
}
