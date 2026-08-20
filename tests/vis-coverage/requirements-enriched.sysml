// Visualization Coverage Test: Requirements View Enrichment (Phase 4)
// Switch to "Requirements" view to test these features.
//
// WHAT TO CHECK:
// 1. reqId label shows "id = REQ-001" below the header on MaxSpeed
// 2. Subject compartment shows "subject vehicle" on MaxSpeed
// 3. Satisfy edge (dashed, open arrow) from Engine to MaxSpeed
// 4. Nested sub-requirement (BrakingDistance inside MaxSpeed)

package RequirementsEnriched {
    requirement def MaxSpeed {
        attribute reqId = "REQ-001";
        subject vehicle;
        doc /* The vehicle shall achieve 200 km/h top speed */

        requirement def BrakingDistance {
            attribute reqId = "REQ-001.1";
            doc /* Stopping distance under 50m at 100 km/h */
        }
    }

    requirement def DerivedAccel {
        attribute reqId = "REQ-002";
        doc /* Derived from MaxSpeed: 0-100 km/h in 8 seconds */
    }

    part def Engine {
        satisfy MaxSpeed;
    }

    concern def EmissionsConcern {
        doc /* Emissions must comply with Euro 7 */
    }
}
