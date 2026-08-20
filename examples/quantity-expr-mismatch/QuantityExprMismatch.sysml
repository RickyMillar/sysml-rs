// RSC-5.2b proving fixture — quantity diagnostics inside constraint
// expressions (D-5.0.8 UQ003 / UQ004).
//
// The static scan resolves bare operand names owner-scoped (D-3.0.6-B) and
// infers each attribute's ISQ dimension. It fires only when both operands of an
// ordering comparison carry known, non-zero, non-equal dimensions (UQ003), or a
// dimensionless-only function receives a dimensioned argument (UQ004).
//
// This is intended to be the ONLY cross-dimension comparison in the examples
// corpus, so UQ003/UQ004 cannot false-fire elsewhere.
package QuantityExprMismatch {
    part def Probe {
        attribute len : ISQ::LengthValue;
        attribute mass : ISQ::MassValue;
        attribute alsoLen : ISQ::LengthValue;

        // UQ003 (error): an ordering comparison between a length and a mass —
        // the static twin of the RSC-5.1b eval-time hard error.
        constraint crossDim { len < mass }

        // Clean: same dimension (length vs length) — never flagged.
        constraint sameDim { len <= alsoLen }

        // Clean: length compared to a bare number — one side is dimensionless,
        // so it is boundary/scale territory, not a dimension mismatch.
        constraint dimVsLiteral { len > 0 }

        // UQ004 (error): sin requires a dimensionless argument; len is a length.
        // (The outer `> 0` is dimensionless-vs-dimensionless, so no UQ003.)
        constraint badTrig { sin(len) > 0 }
    }
}
