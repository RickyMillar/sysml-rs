// Visualization Coverage Test: State Diagram with Control Nodes
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. State transition view should show circular traffic light cycle
//
// WHAT TO CHECK:
// - States render as rounded rectangles: red, yellow, green
// - Transitions show as directed edges (solid line, open arrow)
// - Initial pseudostate (filled circle) before "red" state
// - Cycle: red → green → yellow → red

package StateControlNodes {
    state def TrafficLight {
        entry; then red;

        state red;
        state yellow;
        state green;

        transition first red then green;
        transition first green then yellow;
        transition first yellow then red;
    }
}
