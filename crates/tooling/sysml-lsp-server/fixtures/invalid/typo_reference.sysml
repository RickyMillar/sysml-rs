/* Fixture: typo_reference.sysml
   Expected: E200 Error — "no definition 'Enginne' found in scope"
     - Message must name the unresolved symbol 'Enginne'
     - A note should mention "standard library" (resolution was without it)
     - No other error codes — only E200 (no spurious structural or semantic noise)
   Tests: typo_reference_e200_names_the_symbol, typo_reference_library_note,
          typo_reference_only_e200 */

package TypoRef {
    part def Engine;
    part myCar : Enginne;
}
