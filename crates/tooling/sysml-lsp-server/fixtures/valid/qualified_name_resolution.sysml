/* Fixture: qualified_name_resolution.sysml
Intent: valid
Expected: 0 diagnostics — qualified type reference resolves across packages. */

package A {
part def T;
}
package B {
part myPart : A::T;
}
