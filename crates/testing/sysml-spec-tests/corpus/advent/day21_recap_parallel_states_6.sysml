package AdventDay21_Example6 {

    private import ScalarValues::*;

    item def Status {
        attribute message : String;
    }

    state mission parallel {
        state monitoringReindeer {
            entry; then nominal;
            state nominal;
            state lowEnergy;
        }
        state controllingReindeer {
            entry; then takingOff;
            state takingOff;
            state flying;
            state landing;
            state waiting;
        }
    }
}
