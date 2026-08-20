/* Fixture: cascading.sysml
   Expected: 16+ syntax Errors — triggers phase gating.
     - Every line has garbled keywords, producing many parse errors
     - Phase gating: >= 16 errors suppresses resolution (no E200) and semantics (no S-series)
     - Total diagnostic count must stay <= 50 (cap applied)
     - No duplicate (span, code) pairs after dedup
     - No internal grammar names in messages
   Tests: cascading_triggers_phase_gating, total_cap_applied,
          dedup_no_repeated_spans, no_internal_grammar_names_in_messages,
          syntax_errors_not_structural_codes, diagnostics_priority_sorted */

packag A {
    part de B
    par C
    attribu D
    connectio E
    stat F
    actio G
    requiremen H
    allocatio I
    flo J
    interfac K
    enu L
    us M
    vie N
    concer O
    stakehold P
    rendr Q
}
