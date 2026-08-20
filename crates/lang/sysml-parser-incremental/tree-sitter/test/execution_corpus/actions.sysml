// Tier 4: Action execution patterns
// Tests: send/accept, fork/join/merge/decide, if/while/for, assign

package ActionTests {

    // Basic action with send/accept
    action def Communicate {
        accept msg : Message;
        send Response() to sender;
    }

    // Action with control flow nodes
    action def ProcessOrder {
        action validateOrder;

        decide checkValidity;

        action processPayment;
        action rejectOrder;

        fork startParallel;
        action shipItem;
        action sendConfirmation;
        join waitForAll;

        action completeOrder;

        merge afterDecision;

        succession validateOrder then checkValidity;
        succession checkValidity then processPayment;
        succession processPayment then startParallel;
        succession startParallel then shipItem;
        succession startParallel then sendConfirmation;
        succession shipItem then waitForAll;
        succession sendConfirmation then waitForAll;
        succession waitForAll then completeOrder;
    }

    // If/else action
    action def ConditionalAction {
        if temperature > 100 {
            send OverheatWarning() via alarmPort;
        } else {
            send NormalStatus() via statusPort;
        }
    }

    // While loop
    action def PollingAction {
        while isActive {
            accept sensorReading : SensorData;
            send ProcessedData() to controller;
        }
    }

    // For loop
    action def BatchProcess {
        for item in inventory {
            action processItem;
        }
    }

    // Assignment action
    action def UpdateState {
        assign counter = counter + 1;
        assign status := "active";
    }

    // Perform action
    action def Orchestrator {
        perform action initialize;
        perform action mainLoop;
        perform action cleanup;
    }

    // Nested action with accept via port
    action def MessageHandler {
        accept incomingMsg : Message via inputPort;
        action transform;
        send TransformedMessage() via outputPort;
    }
}
