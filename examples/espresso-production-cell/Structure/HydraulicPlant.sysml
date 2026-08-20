package HydraulicPlantStructure {
    doc /* The shared hydraulic plant: reservoir → pump → accumulator →
     * compliant manifold, feeding every station branch and receiving their
     * return/drain. The manifold owns the shared pressure ODE
     * (ManifoldDynamics); reservoir/pump/accumulator are structural parts at
     * COV-minimum detail. One instance of the manifold ⇒ one shared-plant
     * subsystem (not multiplied). */

    private import ScalarValues::*;
    private import EspressoTypes::*;
    private import EspressoInterfaces::*;
    private import ManifoldPhysics::*;

    part def HydraulicPlant {
        part reservoir {
            doc /* Feed-water reservoir; capacity in litres (structural). */
            attribute capacity : Real default 5.0;
        }
        part pump {
            doc /* Positive-displacement supply pump. */
            attribute rating : Real default 6.0;
        }
        part accumulator {
            doc /* Compliance vessel damping pump pulsation. */
            attribute volume : Real default 0.25;
        }

        // the shared pressure state
        part manifold : ManifoldDynamics;

        // supply out to the station branches; return back from them
        port supplyOut : HydraulicSupplyPort;
        port returnIn : HydraulicReturnInPort;
    }
}
