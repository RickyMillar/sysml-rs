/* Fixture: stdlib_dependent.sysml
Expected: depends on standard library types (Integer, Real, String, Boolean).
Tests: behavior difference when stdlib IS vs IS NOT loaded.
- Without stdlib: unresolved type refs for Integer/Real/String suppressed
- With stdlib: type refs resolve, hover/goto-def follow into library */

package StdlibDependent {
    part def Sensor {
        attribute reading : Real;
        attribute id : Integer;
        attribute label : String;
        attribute active : Boolean;
    }

    part def System {
        part sensor1 : Sensor;
        part sensor2 : Sensor;
        attribute name : String = "TestSystem";
        attribute count : Integer = 2;
    }
}
