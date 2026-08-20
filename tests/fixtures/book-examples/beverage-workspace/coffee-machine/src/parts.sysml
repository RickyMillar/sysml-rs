package CoffeeParts {
    import ScalarValues::*;
    import BeverageTypes::*;

    part def Grinder {
        attribute grindSize : GrindSetting;
    }

    part def Brewer {
        attribute waterTemp : Real;
        attribute brewPressure : Real;
    }

    part def WaterTank {
        attribute currentLevel : Real;
        attribute maxCapacity : Real;
    }

    part def CoffeeMachine {
        part grinder : Grinder;
        part brewer : Brewer;
        part waterTank : WaterTank;
        attribute serialNumber : String;
    }
}
