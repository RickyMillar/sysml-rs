// RSC-5.2 proving fixture — quantity boundary diagnostics (D-5.0.8 / UQ001).
//
// There is no cross-dimension binding anywhere in the existing corpus (every
// ISQ comparison is same-dimension; see rsc5_quantity_baseline.rs B0 findings),
// so this fixture deliberately authors the three binding shapes the two-tier
// severity rubric distinguishes. ISQ types resolve by NAME against the runtime
// ISQ table (no stdlib load required — `LengthValue`/`MassValue` strip from any
// `ISQ::` prefix), matching how the other rsc5 fixtures are loaded standalone.
package QuantityMismatch {
    attribute lengthAttr : ISQ::LengthValue;
    attribute massAttr : ISQ::MassValue;
    attribute anotherLength : ISQ::LengthValue;
    attribute untyped;

    // UQ001 HARD ERROR — length bound to mass (resolved-incompatible dims).
    binding bind lengthAttr = massAttr;

    // SILENT — same dimension (length = length). A pure scale difference, if
    // any, is RSC-5.3 boundary auto-conversion, not a mismatch.
    binding bind lengthAttr = anotherLength;

    // UQ001 WARNING — a dimensioned attribute bound to an untyped one
    // (two-tier: we lack the facts to fail hard).
    binding bind massAttr = untyped;
}
