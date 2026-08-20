/* Fixture: ownership_violation.sysml
   Expected: Syntax/structural errors — `entry action` is invalid inside a part def.
     - `entry` is a state-specific keyword, not valid in a definition body
     - No internal grammar names in messages
   Tests: no_internal_grammar_names_in_messages, syntax_errors_not_structural_codes */

package BadOwnership {
    part def Vehicle {
        state def Running;
        entry action doNothing;
    }
}
