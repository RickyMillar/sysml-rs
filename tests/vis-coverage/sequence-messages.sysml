// Visualization Coverage Test: Sequence Diagram Messages
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Switch to "Sequence" view
//
// WHAT TO CHECK:
// - Lifelines render vertically: client, server, database
// - Message arrows flow left-to-right between lifelines
// - Activation boxes appear on lifelines during active calls
// - Return messages shown as dashed arrows

package SequenceMessages {
    action def ClientAction {
        action step;
    }
    action def ServerAction {
        action step;
    }
    action def DatabaseAction {
        action step;
    }

    action client : ClientAction;
    action server : ServerAction;
    action database : DatabaseAction;

    action def RequestResponse {
        doc /* Client-server request-response sequence with self-call. */
        action sendRequest : ClientAction;
        action handleRequest : ServerAction;
        action processCache : ServerAction;
        action queryDB : DatabaseAction;
        action returnData : ServerAction;
        action receiveResponse : ClientAction;

        succession first sendRequest then handleRequest;
        succession first handleRequest then processCache;
        succession first processCache then processCache; // self-message
        succession first processCache then queryDB;
        succession first queryDB then returnData;
        succession first returnData then receiveResponse;
    }
}
