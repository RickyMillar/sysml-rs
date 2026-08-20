package VehicleVerification {
    private import ScalarValues::Real;

    part def Vehicle {
        attribute speed : Real;
        attribute mass : Real;

        constraint speedLimit { speed < 200 }
        constraint positiveWeight { mass > 0 }
    }

    part testVehicle : Vehicle {
        attribute speed = 120;
        attribute mass = 1500;
    }

    verification def SpeedTest {
        subject testVehicle : Vehicle;
    }
}
