/* Fixture: stdlib_type_resolution.sysml
Intent: valid
Expected: stdlib type resolves via explicit import; unresolved local type emits E200 only. */

package StdlibTest {
private import ScalarValues::Integer;

part def Widget {
attribute count : Integer;
part custom : CustomThing;
}
}
