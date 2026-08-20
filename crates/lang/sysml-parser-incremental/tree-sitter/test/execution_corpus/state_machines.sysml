// Tier 4: State machine execution patterns
// Tests: state defs, entry/do/exit, transitions, nested states, parallel states

package StateMachineTests {

    // Basic state definition with entry/do/exit
    state def TrafficLight {
        entry action turnOn;
        do action blink;
        exit action turnOff;
    }

    // State usage with inline body
    state trafficLight : TrafficLight {
        entry action illuminate;
        do action flash {
            send LightSignal() via signalPort;
        }
        exit action dim;
    }

    // Nested exclusive states with transitions
    state def VehicleState {
        state parked;
        state running {
            entry action startEngine;
            state idle;
            state moving;

            accept EngageGear
                then moving;

            accept Brake if [speed == 0]
                then idle;
        }

        accept TurnKey
            then running;

        accept ParkCommand
            then parked;
    }

    // Parallel states
    state def DualMode {
        state parallel operatingMode {
            state heatingSystem {
                state off;
                state heating;
                accept HeatRequest then heating;
                accept StopHeat then off;
            }
            state coolingSystem {
                state off;
                state cooling;
                accept CoolRequest then cooling;
                accept StopCool then off;
            }
        }
    }

    // Transition with guard and effect
    state def GatedState {
        state locked;
        state unlocked;

        transition unlock
            first locked
            accept KeyEvent
            if [hasValidKey]
            do action logAccess;
            then unlocked;

        transition lock
            first unlocked
            accept LockCommand
            then locked;
    }

    // State with send action in entry
    state def NotifyingState {
        state active {
            entry send StatusUpdate() via notificationPort;
        }
        state inactive;
    }
}
