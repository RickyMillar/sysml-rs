/* Fixture: corrupted_line.sysml
   Expected: Syntax Error on the garbled line below.
     - Error should point to the bad line, not to unrelated locations
     - No internal grammar names in the message
     - Structural/semantic diagnostics overlapping the syntax error span should be suppressed
     - No duplicate (span, code) pairs after dedup
   Tests: corrupted_line_detected, corrupted_line_points_to_bad_line,
          no_internal_grammar_names_in_messages, syntax_errors_suppress_overlapping_validation,
          dedup_no_repeated_spans, diagnostics_priority_sorted */

package Corrupted {
    part def Goodpackage Bad {
    }
    part def AfterError;
}
