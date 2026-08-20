/* Fixture: flows_and_messages.sysml
Expected: valid SysML flow and message constructs.
Tests: FlowUsage, SuccessionFlowUsage, message declarations */

package FlowsAndMessages {
    part def Signal;

    action def Pipeline {
        action step1;
        action step2;

        // Plain flow with endpoints
        flow dataFlow from step1.outputData to step2.inputData;

        // Succession flow — delivery blocked until source completes
        succession flow sf from step1 to step2;
    }

    // Message declaration (FlowUsage with isMessage)
    message alert;

    // Named flow with dot-chained endpoints
    part sensor { attribute out; }
    part controller { attribute input; }
    flow namedFlow from sensor.out to controller.input;
}
