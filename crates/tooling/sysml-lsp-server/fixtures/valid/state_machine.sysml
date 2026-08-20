/* Fixture: state_machine.sysml
Expected: valid SysML state machine constructs.
Tests: StateDefinition, StateUsage, TransitionUsage, entry/exit/do */

package StateMachines {
    state def TrafficLight {
        entry; then Red;

        state Red {
            entry action setRed;
            do action waitRed;
        }

        state Yellow {
            entry action setYellow;
        }

        state Green {
            entry action setGreen;
            exit action clearGreen;
        }

        transition redToGreen
            first Red then Green;

        transition greenToYellow
            first Green then Yellow;

        transition yellowToRed
            first Yellow then Red;
    }

    state def DoorController {
        state Closed;
        state Opening;
        state Open;
        state Closing;

        transition open
            first Closed then Opening;
        transition opened
            first Opening then Open;
        transition close
            first Open then Closing;
        transition closed
            first Closing then Closed;
    }

    part def Vehicle {
        state engineState : TrafficLight;
    }
}
