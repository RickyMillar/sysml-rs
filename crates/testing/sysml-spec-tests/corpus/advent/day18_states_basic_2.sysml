package L18_States_Basic {

    private import ScalarValues::*;

    item def StartMission;
    item def EndMission;

    item def Command {
        attribute cmd : String;
    }

    state def ReindeerController {
        attribute reindeerCount : Integer;
        attribute totalPower subsets ISQ::power;
        attribute averageEnergy : Real;
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
