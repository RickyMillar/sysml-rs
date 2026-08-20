// LSP fixture: action control flow (Sprint 8)
// Covers fork/join/merge/decision, assignment, successions in actions.

package ActionControlFlowFixture {

    action def MonitorAndRespond {
        // Basic action steps
        action readSensor;
        action processData;
        action respond;

        // Succession (ordering)
        succession first readSensor then processData;
        succession first processData then respond;

        // Assignment action
        assign threshold = 100;

        // Decision node
        decide checkLevel;

        // Fork and join for parallel paths
        fork splitWork;
        action pathA;
        action pathB;
        join mergeWork;

        // Merge node
        merge convergence;
    }

    action def AlertSequence {
        // Accept action
        accept evt : Notification;

        // Send action
        send Alert() to monitor;

        // If action
        if isUrgent {
            action escalate;
        }

        // While loop
        while retryNeeded {
            action attemptRetry;
        }

        // For loop
        for item in pendingItems {
            action processItem;
        }

        // Perform action
        perform action calibrate;
    }
}
