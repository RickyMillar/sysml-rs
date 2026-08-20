package CellLayout {
    doc /* Physical placement hints for the geometry view (COV UI-DIAGRAM /
     * CELL-16). A lightweight, declarative bench layout: the shared plants sit
     * on the service side, the stations along the bar. Coordinates are
     * normalized bench units, consumed by the geometry viewpoint in Views. */

    private import ScalarValues::*;

    attribute def Placement {
        attribute x : Real;
        attribute y : Real;
    }

    part def BenchLayout {
        attribute hydraulicPlantAt : Placement { attribute :>> x = 0.0; attribute :>> y = 0.0; }
        attribute thermalPlantAt : Placement { attribute :>> x = 0.0; attribute :>> y = 1.0; }
        attribute stationPitch : Real default 1.0;   // spacing between stations along the bar
        attribute barOriginX : Real default 2.0;
        attribute barOriginY : Real default 0.5;
    }
}
