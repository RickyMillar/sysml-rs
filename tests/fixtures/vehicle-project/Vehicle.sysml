package Vehicle {
    import Engine::*;
    import Wheel::*;

    part def Vehicle {
        part engine : Engine;
        part wheels : Wheel[4];
    }

    part myVehicle : Vehicle {
        part redefines engine {
            attribute redefines horsepower = 250;
            attribute redefines cylinders = 4;
        }
    }
}
