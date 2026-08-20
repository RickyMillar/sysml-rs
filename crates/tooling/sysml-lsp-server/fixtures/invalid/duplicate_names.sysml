/* Fixture: duplicate_names.sysml
   Expected: S001 Warning — "duplicate definition 'Foo' is already defined in this scope"
     - Severity must be Warning (not Error)
     - related_information should point to the first 'Foo' with "first defined here"
     - No syntax errors (file is syntactically valid)
     - No structural errors (E0xx)
   Tests: duplicate_names_flagged_s001, duplicate_names_is_warning,
          duplicate_names_related_location, duplicate_names_no_spurious_errors */

package DupExample {
    part def Foo;
    part def Foo;
}
