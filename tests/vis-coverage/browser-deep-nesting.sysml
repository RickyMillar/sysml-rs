// Visualization Coverage Test: Deep Ownership Tree for Browser View
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Switch to "Browser" view
//
// WHAT TO CHECK:
// - Tree shows Level1 > Level2 > System > subsystemA / subsystemB
// - subsystemA shows attributes: weight, cost
// - subsystemB shows attribute: power, port: dataPort
// - DataPort shows as leaf node with port icon
// - Expand/collapse arrows at each nesting level

package Level1 {
    package Level2 {
        port def DataPort;

        part def System {
            part subsystemA {
                attribute weight : Real;
                attribute cost : Real;
            }
            part subsystemB {
                attribute power : Real;
                port dataPort : DataPort;
            }
        }

        attribute def Real;
    }
}
