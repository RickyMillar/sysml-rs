// RSC-5.2b proving fixture — the value-binding (`=` / `:=`) form of UQ001
// (D-5.0.8). An attribute's declared ISQ dimension is compared against the
// dimension of its default *value expression*.
//
// The static scan resolves the value expression's bare operand names
// owner-scoped (D-3.0.6-B) and infers each operand's ISQ dimension. It fires a
// hard error only when the declared type dimension and the value-expression
// dimension are both known, non-zero, and unequal.
//
// A literal default (`= 5 [kg]`) is NOT in scope — the parser folds its unit
// onto the attribute, which `infer_m_ref` (D-5.0.5) takes as authoritative.
// This fixture uses only expression defaults so the declared/value split is
// real.
package QuantityValueMismatch {
    part def Probe {
        attribute width : ISQ::LengthValue;
        attribute height : ISQ::LengthValue;
        attribute mass : ISQ::MassValue;

        // UQ001 (error): declared a length, default value resolves to a mass
        // (mass * dimensionless). The static twin of the RSC-5.1b eval-time
        // hard error, reported at the declaration.
        attribute badLen : ISQ::LengthValue = mass * 2;

        // Clean: declared an area, default value is length * length = area.
        attribute area : ISQ::AreaValue = width * height;

        // Clean: declared a length, default value resolves to a length
        // (length * dimensionless) — exercises the equal-dimension path.
        attribute scaledLen : ISQ::LengthValue = width * 3;
    }
}
