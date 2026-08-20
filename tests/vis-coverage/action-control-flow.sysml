// Visualization Coverage Test: Action Flow with Sequential Control
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Action flow view shows pipeline of steps
//
// WHAT TO CHECK:
// - Actions render as rounded rectangles: validate, checkInventory, etc.
// - Succession edges (solid line) connect actions in sequence
// - Initial node (filled circle) before validate
// - Final node (filled circle with ring) after notifyCustomer

package ActionControlFlow {
    action def ProcessOrder {
        action validate;
        action checkInventory;
        action processPayment;
        action shipOrder;
        action notifyCustomer;

        succession first validate then checkInventory;
        succession first checkInventory then processPayment;
        succession first processPayment then shipOrder;
        succession first shipOrder then notifyCustomer;
    }
}
