package ConfiguredSleigh {
    part def SantaSleigh specializes SleighDesign::SantaSleigh {
        private import MagicalBagVariant::*;
        //private import MagicalCargoBayVariant::*;
        part redefines cargoBay : CargoBay;
    }
}
