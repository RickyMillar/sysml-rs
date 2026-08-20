package AdventDay14_Example2 {
    private import ScalarValues::Real;


    part def Reindeer {
        constant attribute weight redefines ISQ::mass default 110 [kg];  // Default bound
        attribute energyLevel : Real default := 100;  // Default initial
    }

    part def Rudolph specializes Reindeer {
        constant attribute :>> weight = 100 [kg];  // Non-default bound (override)
        attribute :>> energyLevel := 200;  // Non-default initial (override)
    }
}
