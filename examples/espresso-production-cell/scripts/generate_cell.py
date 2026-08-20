#!/usr/bin/env python3
"""Deterministic generator for the parameterized ProductionCell instantiation
(D9). Emits Structure/ProductionCell.sysml for an arbitrary station count so the
`smoke` (2), `ci` (8), and `stress` (24) scales are generated variants, never
copy-pasted files.

Run at the default (smoke) to reproduce the checked-in file byte-for-byte:

    python3 scripts/generate_cell.py                 # smoke, 2 stations -> Structure/ProductionCell.sysml
    python3 scripts/generate_cell.py --stations 8    # ci
    python3 scripts/generate_cell.py --stations 24 --out /tmp/ProductionCell.sysml
"""

import argparse
import os

HEADER = '''\
package ProductionCellStructure {{
    doc /* The production cell at {scale} scale ({n} stations). This file is
     * the parameterized instantiation surface: `scripts/generate_cell.py
     * --stations N` reproduces it byte-for-byte at N={n} and emits the other
     * scales the same way (D9 — generated variants, never copy-pasted). Every
     * station is a true usage of the ONE BrewStation definition, so the runtime
     * multiplies it into independent per-instance subsystems with qualified
     * slot paths (COV RT-INSTANCE / RT-SLOT). Shared plants (hydraulic +
     * thermal) are single instances. Power planes are wired here as declared
     * connectors (PowerBond); measurement (SignalLink) and command
     * (MessageChannel) planes are wired in LinkCorpus / the supervisor.
     *
     * GENERATED — edit scripts/generate_cell.py, not this file. scale={scale} stations={n} */

    private import ScalarValues::*;
    private import BrewStationStructure::*;
    private import HydraulicPlantStructure::*;
    private import ThermalPlantStructure::*;

    part def ProductionCell {{
        // -- shared plants --------------------------------------------------
        part hydraulicPlant : HydraulicPlant;
        part thermalPlant : ThermalPlant;

        // -- brew stations (repeated usages of the one BrewStation def) -----
'''

FOOTER = '''\
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
'''

SCALE_NAME = {2: "smoke", 8: "ci", 24: "stress"}


def render(n):
    stations = [f"station{i}" for i in range(1, n + 1)]
    out = [HEADER.format(n=n, scale=SCALE_NAME.get(n, "custom"))]

    for s in stations:
        out.append(f"        part {s} : BrewStation;\n")

    out.append("\n        // -- power planes: shared plant <-> each station (PowerBond) ---------\n")
    for s in stations:
        out.append(f"        flow from hydraulicPlant.supplyOut to {s}.supplyIn;\n")
        out.append(f"        connect hydraulicPlant.supplyOut to {s}.supplyIn;\n")
    for s in stations:
        out.append(f"        flow from {s}.returnOut to hydraulicPlant.returnIn;\n")
        out.append(f"        connect {s}.returnOut to hydraulicPlant.returnIn;\n")
    for s in stations:
        out.append(f"        flow from thermalPlant.heatOut to {s}.heatIn;\n")
        out.append(f"        connect thermalPlant.heatOut to {s}.heatIn;\n")

    out.append("\n        // -- aggregate accounting (COV VAL-CONSTR / CELL-09) ----------------\n")
    out.append("        // Explicit generated sums over the station instances; the CELL-PHYS\n")
    out.append("        // residual gate recomputes these independently from the exposed slots.\n")
    q_sum = " + ".join(f"{s}.qBranch" for s in stations)
    out.append(f"        attribute qTotal : Real = {q_sum};\n")
    p_sum = " + ".join(f"{s}.thermal.P_heater" for s in stations)
    out.append(f"        attribute pTotal : Real = thermalPlant.source.P_source + {p_sum};\n\n")

    out.append(FOOTER)
    return "".join(out)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--stations", type=int, default=2)
    ap.add_argument("--out", default=None)
    args = ap.parse_args()
    assert args.stations >= 1, "need at least one station"

    here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out_path = args.out or os.path.join(here, "Structure", "ProductionCell.sysml")
    with open(out_path, "w", newline="\n") as f:
        f.write(render(args.stations))
    print(f"wrote {args.stations}-station ProductionCell -> {out_path}")


if __name__ == "__main__":
    main()
