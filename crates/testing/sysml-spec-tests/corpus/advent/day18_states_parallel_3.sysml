package L18_States_Parallel {

    private import ScalarValues::*;

    item def StartMission;
    item def EndMission;

    state def ReindeerController {
        entry;
            then idle;
        state idle;

        state mission parallel {
            state monitoringReindeer {
                entry;
                    then nominal;
                state nominal;
                state lowEnergy;
            }
            state controllingReindeer {
                entry;
                    then takingOff;
                state takingOff;
                state flying;
                state landing;
                state waiting;
            }
        }
    }
}
