# Physics Diagnostics Demo

Open `physics-demo.sysml` in VS Code after deploying the extension to test:

## What to test

1. **Reload VS Code**: `Ctrl+Shift+P` → "Developer: Reload Window"

2. **Open the demo file**: `physics-demo.sysml`

3. **Check diagnostics** (Problems panel, `Ctrl+Shift+M`):
   - `[PH003]` on `TempProbePort` — "thermal but only has effort feature"
   - `[PH003]` on `CurrentSensePort` — "electrical but only has flow feature"

4. **Hover over ports**:
   - Hover over `PhasePort` definition → should show "Physics: electrical domain"
   - Hover over `ThermalPort` definition → should show "Physics: thermal domain"
   - Hover over `port electrical : PhasePort` in HoverDemo → should show physics info

5. **Code lenses** (above port definitions):
   - `PhasePort` → "Physics: electrical (effort: voltage, flow: current)"
   - `ThermalPort` → "Physics: thermal (effort: temperature, flow: heatFlow)"

## Also test on a larger model

Open `examples/espresso-production-cell/` files to see diagnostics on a larger,
multi-subsystem model with ISQ-typed hydraulic and thermal ports.
