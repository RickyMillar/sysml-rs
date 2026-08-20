package L18_States_EntryExit {

    private import ScalarValues::*;

    item def Status {
        attribute status : String;
    }

    state def ReindeerController {
        ref port hmi;

        state mission parallel {
            state monitoringReindeer {
                entry;
                    then nominal;
                state nominal;
                state lowEnergy {
                    entry send new Status("low energy") via hmi;
                    do action blinkLed;
                    exit send new Status("recharged") via hmi;
                }
            }
        }
    }
}
