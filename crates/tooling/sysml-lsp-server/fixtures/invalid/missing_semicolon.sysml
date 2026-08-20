/* Fixture: missing_semicolon.sysml
   Expected: At least 1 syntax Error — `part def Foo` without ';' or '{...}' is invalid.
     - Message should mention ';' or 'expected' so the user knows what to add
     - Tree-sitter alone accepts this (too permissive); pest catches it as a strictness check
   Tests: missing_semicolon_detected, missing_semicolon_message_actionable */

package MissingSemicolon {
    part def Foo
    part def Bar;
}
