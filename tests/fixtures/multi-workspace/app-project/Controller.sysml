package Controller {
    import Sensors::*;
    import Actuators::*;

    part def ControlSystem {
        part tempSensor : TemperatureSensor;
        part pressureSensor : PressureSensor;
        part mainMotor : Motor;
        part safetyValve : Valve;
    }
}
