/* Fixture: duplicate_name_different_scope.sysml
Intent: valid
Expected: 0 diagnostics — duplicate short names in different packages are legal. */

package A {
part def Sensor;
}
package B {
part def Sensor;
}
