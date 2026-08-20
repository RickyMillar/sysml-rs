package CoffeeParts {
    part def Grinder {
        attribute grindSize : String;
    }

    part def Brewer {
        attribute waterTemp : Integer;
        attribute brewPressure : Integer;
    }

    part def WaterTank {
        attribute currentLevel : Integer;
        attribute maxCapacity : Integer;
    }

    part def CoffeeMachine {
        part grinder : Grinder;
        part brewer : Brewer;
        part waterTank : WaterTank;
        attribute serialNumber : String;
    }
}
