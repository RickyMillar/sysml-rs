package AdventDay21_Example5 {

    private import ScalarValues::*;
    private import SI::*;

    item def StartMission;
    item def EndMission;
    item def Command {
        attribute message : String;
    }
    item def Status {
        attribute message : String;
    }

    state def ReindeerController {
        attribute reindeerCount : Integer;
        attribute totalPower subsets ISQ::power;
        ref port eyelet;
        ref port hmi;

        entry;
            then idle;
        state idle;
            accept StartMission via hmi
                if reindeerCount == 9 and totalPower >= 36 [SI::kW]
                do send new Command("Gee-up!") via eyelet
                then mission;
        state mission;
            accept EndMission via hmi
            then idle;
    }
}
