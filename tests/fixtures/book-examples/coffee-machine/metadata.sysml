// Smart Coffee Machine: Metadata and Documentation
// Demonstrates: comments, doc, metadata def, metadata usage, annotations

package CoffeeMachineMetadata {
import Definitions::*;

// Metadata definition for tracking model maturity
metadata def ModelMaturity {
attribute level : String;
attribute reviewer : String;
attribute date : String;
}

// Metadata definition for safety classification
metadata def SafetyClassification {
attribute level : String;       // "critical", "standard", "informational"
attribute standard : String;    // e.g. "IEC 61508"
}

// Apply metadata to the CoffeeMachine definition
@ModelMaturity {
:>> level = "reviewed";
:>> reviewer = "J. Smith";
:>> date = "2026-02-01";
}
part def AnnotatedCoffeeMachine :> CoffeeMachine {
doc /* A coffee machine with full model maturity annotations.
* This definition has been reviewed and approved.
*/

comment /* Implementation note: sensor calibration
* values are placeholders pending hardware testing.
*/

@SafetyClassification {
:>> level = "critical";
:>> standard = "IEC 61508";
}
part cupSensor :>> CoffeeMachine::cupSensor;
}
}
