/* Fixture: simple_vehicle.sysml
   Intent: valid
   Expected: constraint diagnostics only when values violate constraints (no import/no syntax noise). */

package SimpleVehicle {
    part def Vehicle {
        attribute mass;
        attribute speed;
    }
    part vehicle : Vehicle {
        attribute mass = 2500;
        attribute speed = 105;
        constraint speedLimit { speed < 100 }
    }
}
