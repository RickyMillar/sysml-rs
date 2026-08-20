package L18_States_Items {

    private import ScalarValues::*;

    item def StartMission;
    item def EndMission;
    item def Approach;
    item def Next;

    item def Command {
        attribute cmd : String;
    }

    item def Status {
        attribute status : String;
    }
}
