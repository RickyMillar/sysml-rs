/* Fixture: missing_import.sysml
Intent: invalid
Expected: unresolved reference diagnostics for cross-package type without import. */

package Sensors {
part def Sensor;
}
package Vehicle {
part mainSensor : Sensor;
}
