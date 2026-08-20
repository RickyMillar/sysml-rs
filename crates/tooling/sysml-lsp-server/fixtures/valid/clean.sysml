/* Fixture: clean.sysml
Expected: 0 diagnostics - valid SysML, no errors, no warnings.
Tests: clean_file_zero_diagnostics */

package CleanExample {
part def Engine {
attribute horsePower;
}
part def Car {
part engine : Engine;
}
}
