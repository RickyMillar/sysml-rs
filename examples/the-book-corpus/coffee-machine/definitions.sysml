// Smart Coffee Machine: Core Structural Definitions
// Demonstrates: part def, item def, attribute def, usages, ownership

package Definitions {
    import ScalarValues::*;
    import SI::*;

    // Attribute definitions for measurable properties
    attribute def Temperature :> ISQ::TemperatureValue;
    attribute def Pressure :> ISQ::PressureValue;
    attribute def Mass :> ISQ::MassValue;
    attribute def Duration :> ISQ::DurationValue;

    // Item definitions for things that flow through the system
    item def Water;
    item def CoffeeBeans;
    item def GroundCoffee;
    item def BrewedCoffee;
    item def Steam;

    // Part definitions for physical components
    part def CoffeeMachine {
        doc /* A smart coffee machine with grinder, brewer, and steam wand. */

        part grinder : Grinder;
        part brewer : Brewer;
        part waterTank : WaterTank;
        part steamWand : SteamWand;
        part controlPanel : ControlPanel;
        part cupSensor : CupSensor;
        part dripTray : DripTray;

        attribute serialNumber : String;
    }

    part def Grinder {
        doc /* Grinds coffee beans to a target coarseness. */

        attribute grindSize : Real;     // millimeters
        attribute motorSpeed : Real;    // RPM
    }

    part def Brewer {
        doc /* Heats water and forces it through coffee grounds. */

        attribute waterTemp : Temperature;
        attribute brewPressure : Pressure;
        attribute capacity : Mass;
    }

    part def WaterTank {
        attribute currentLevel : Real;  // percentage full
        attribute maxCapacity : Mass;
    }

    part def SteamWand {
        attribute steamTemp : Temperature;
        attribute isActive : Boolean;
    }

    part def ControlPanel {
        attribute selectedDrink : DrinkType;
        attribute isReady : Boolean;
    }

    part def CupSensor {
        attribute cupDetected : Boolean;
    }

    part def DripTray {
        attribute isFull : Boolean;
    }

    // Enumeration for drink selection
    enum def DrinkType {
        espresso;
        americano;
        cappuccino;
        latte;
    }
}
