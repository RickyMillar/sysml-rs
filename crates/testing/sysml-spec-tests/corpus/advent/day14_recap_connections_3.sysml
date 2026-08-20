package AdventDay14_Example3 {

    item def Command;
    item def Feedback;

    part def Sleigh {
        abstract ref part reindeer : Reindeer [9];
        port eyelet : HarnessConnectionPoint;
    }

    part def Reindeer {
        ref part pulling : Sleigh;
        port harnessLoop : ~HarnessConnectionPoint;  // Conjugated
    }

    port def HarnessConnectionPoint {
        out item command : Command;
        in item feedback : Feedback;
    }

    connection def Harness {
        end part sleigh : Sleigh crosses reindeer.pulling;
        end part reindeer : Reindeer crosses sleigh.reindeer;

        interface : HarnessInterface connect sleigh.eyelet to reindeer.harnessLoop;
    }

    interface def HarnessInterface {
        end port eyelet : HarnessConnectionPoint;
        end port harnessLoop : ~HarnessConnectionPoint;

        flow of Command from eyelet.command to harnessLoop.command;
        flow of Feedback from harnessLoop.feedback to eyelet.feedback;
    }
}
