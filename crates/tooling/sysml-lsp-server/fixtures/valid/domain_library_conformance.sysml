/* Fixture: domain_library_conformance.sysml
Intent: valid
Expected: domain library names (ISQ, SI, USCustomary) resolve through the
salsa pipeline when the corresponding library graph is loaded.
Domain libraries are loaded recursively via namespace imports.

This tests Sprint 2 Item 8 — verifying end-to-end name resolution for
domain libraries including ISQ quantities and SI units. */

package DomainLibraryConformance {
    import ISQ::*;
    import SI::*;

    part def PhysicalSensor {
        // ISQ quantity types
        attribute mass : ISQ::MassValue;
        attribute length : ISQ::LengthValue;
        attribute temperature : ISQ::TemperatureValue;
    }

    part def MeasuredComponent {
        // SI unit types
        attribute weight : SI::kilogram;
        attribute size : SI::metre;
    }
}
