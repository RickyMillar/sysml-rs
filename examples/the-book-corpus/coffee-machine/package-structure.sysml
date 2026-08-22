// Smart Coffee Machine: Package Structure and Imports
// Demonstrates: packages, library packages, visibility, imports

package CoffeeMachineDomain {
    doc /* Top-level domain package for the Smart Coffee Machine.
         * Contains structural definitions, behaviors, and requirements.
         */

    // Import standard library types
    import ScalarValues::*;
    import SI::*;
    import ISQ::*;

    // Sub-packages organize the model by concern
    package Structure {
        doc /* Structural definitions: parts, items, attributes, ports. */

        // Re-export key definitions for convenience
        public import Definitions::*;
        public import Ports::*;
    }

    package Behavior {
        doc /* Behavioral definitions: actions, states, flows. */

        public import Actions::*;
        public import States::*;
    }

    package QualityModel {
        doc /* Requirements, constraints, and verification. */

        public import Requirements::*;
        public import Verification::*;
    }

    // Private helper package not visible outside the domain
    private package Internal {
        // Implementation details hidden from consumers
    }
}
