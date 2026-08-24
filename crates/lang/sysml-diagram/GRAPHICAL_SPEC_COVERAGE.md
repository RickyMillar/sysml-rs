# Graphical specification coverage

This document tracks `sysml-diagram`'s renderer-neutral modelling coverage.
It is implementation documentation, not an OMG conformance claim.

## Current contract

- Rust generates `DiagramIR` and `ViewModel` for the standard view families:
  General, Interconnection, StateTransition, ActionFlow, Sequence, Browser,
  Grid, and Geometry.
- The React-SVG application renders graph scenes from `ViewModel.scene`.
- Grid, Browser, and Geometry views use the typed `ViewModel.non_graph` data.
- Simulation, verdict, and diagnostic data are separate element-id keyed
  overlays.

`pipeline_coverage.toml`, generator tests, and frontend renderer tests are the
executable evidence for the currently implemented surface. Do not infer full
OMG graphical conformance from this inventory.
