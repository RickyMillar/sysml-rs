package AdventDay21_Example7 {

    private import ScalarValues::*;

    item def Status {
        attribute message : String;
    }

    port def HmiPort;

    action def blinkLed;

    state lowEnergy {
        ref port hmi : HmiPort;
        entry send new Status("low energy") via hmi;
        do action blinkLed;
        exit send new Status("recharged") via hmi;
    }
}
