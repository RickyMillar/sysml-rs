// LSP fixture: state machine conformance (Sprint 9)
// Covers parallel states, exhibit states, guard/trigger/effect, entry/do/exit, inline transitions.

package StateMachineConformance {

    // Parallel state with explicit keyword
    state def ParallelController {
        state operationalRegion parallel {
            state monitoring;
            state logging;
        }
    }

    // Entry/do/exit subactions
    state def SensorState {
        state idle {
            entry action initialize;
            do action monitor;
            exit action cleanup;
        }
        state active {
            entry action activate;
            exit action deactivate;
        }
        transition first idle then active;
    }

    // Transition with guard expression
    state def GuardedMachine {
        state off;
        state on;
        transition powerOn first off if [isReady] then on;
        transition powerOff first on then off;
    }

    // Transition with trigger
    state def TriggeredMachine {
        state waiting;
        state processing;
        transition start first waiting accept startEvt then processing;
    }

    // Transition with effect
    state def EffectMachine {
        state idle;
        state running;
        transition begin first idle do setup; then running;
    }

    // Inline transition chain in state body
    state def InlineTransitions {
        state ready {
            // Inline: if guard then target
            if [armed] then active;
        }
        state active;
    }

    // Exhibit state usage
    exhibit exhibitedState : SensorState;

    // Bare entry/exit (no action name)
    state def MinimalState {
        state simple {
            entry;
            exit;
        }
    }
}
