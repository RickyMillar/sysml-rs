// boiler-orchestration.sysml -- entry-action assignment pipeline fixture
//
// Minimal state machine for the full-pipeline test: parse -> elaborate ->
// StateMachineCompiler::compile_named -> StateMachineRunner. Each state's
// entry action assigns context variables, so stepping the runner through
// cold -> heating -> ready must leave boilerTemp = 93 and machineReady = 1
// in the evaluation context.

package BoilerOrchestration {

    state def BoilerController {
        doc /* Boiler heating phases.
             * Writes: boilerTemp, machineReady, heaterOn */

        entry; then cold;

        state cold {
            entry action { boilerTemp = 20; machineReady = 0; heaterOn = 0; }
        }

        state heating {
            entry action { heaterOn = 1; boilerTemp = 45; }
        }

        state ready {
            entry action { boilerTemp = 93; machineReady = 1; heaterOn = 0; }
        }

        transition cold_to_heating first cold accept powerOn then heating;
        transition heating_to_ready first heating accept tempReached then ready;
        transition ready_to_cold first ready accept powerOff then cold;
    }
}
