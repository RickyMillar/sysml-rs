package AdventOfSysMLv2_Day5_Abstract {

    part def SantaSleigh {
        part cargoBay : CargoBay;
        ref item santa;
    }

    abstract part def CargoBay;
}
