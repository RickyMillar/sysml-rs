/* Fixture: actions_and_flows.sysml
Expected: valid SysML action and flow constructs.
Tests: ActionDef/Usage, Send/Accept/If/While/For actions, FlowConnectionUsage */

package ActionsAndFlows {
    part def Signal;
    part def Sensor {
        attribute reading : Integer;
    }

    action def ProcessData {
        in item inputData;
        out item outputData;

        action step1;
        action step2;

        flow from step1.outputData to step2.inputData;
    }

    action def ControlLoop {
        action monitor : MonitorAction;
        action respond : RespondAction;
    }

    action def MonitorAction {
        accept sensor : Signal;
        send result : Signal;
    }

    action def RespondAction {
        in item trigger;
    }

    action def ConditionalAction {
        if true {
            action doTrue;
        }
    }

    action def LoopAction {
        while true {
            action iterate;
        }

        for item in items {
            action processItem;
        }
    }

    action runProcess : ProcessData;

    part def Controller {
        perform action mainLoop : ControlLoop;
    }
}
