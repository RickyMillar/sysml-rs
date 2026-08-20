// Visualization Coverage Test: Embedding Matrix
//
// PURPOSE: Systematic test of every embeddable view type, both standalone
// and nested inside a structural container. This isolates whether rendering
// issues come from the sub-diagram itself or from the embedding/nesting.
//
// STRUCTURE:
//   1. Standalone behavioral definitions (no nesting)
//   2. Same behaviors embedded inside a PartDefinition (one level)
//   3. Mega composite: everything in one container (stress test)
//
// PER SPEC (Clause 8.2.3, §1.2):
//   - StateDefinition/Usage → embeds state-transition-view
//   - ActionDefinition/Usage → embeds action-flow-view
//   - Part/Item/Connection/Occurrence → embeds sequence-view (via FlowUsage)
//   - Any → can embed general-view (interconnection)
//
// HOW TO TEST:
//   1. Generate fixtures: bash tools/scripts/generate-vis-fixtures.sh
//   2. Open http://localhost:3002
//   3. Compare "Embedding Matrix" (collapsed) vs "Embedding Matrix (expanded)"
//   4. Check browser console (F12) for zero ELK errors

package EmbeddingMatrix {

    // ═══════════════════════════════════════════════════
    // SECTION 1: Standalone behavioral definitions
    // These should render correctly on their own before
    // we try nesting them.
    // ═══════════════════════════════════════════════════

    // 1A: State machine (standalone)
    state def StandaloneStateMachine {
        state idle;
        state running;
        state error;

        transition first idle then running;
        transition first running then error;
        transition first error then idle;
    }

    // 1B: Action flow (standalone)
    action def StandaloneActionFlow {
        action validate;
        action process;
        action notify;
        action complete;

        succession first validate then process;
        succession first process then notify;
        succession first notify then complete;
    }

    // 1C: Interconnection parts (for IBD embedding)
    port def PowerPort;
    port def DataPort;
    port def ControlPort;

    part def Sensor {
        port dataOut : DataPort;
    }

    part def Controller {
        port dataIn : DataPort;
        port ctrlOut : ControlPort;
    }

    part def Actuator {
        port ctrlIn : ControlPort;
    }

    // ═══════════════════════════════════════════════════
    // SECTION 2: Single-level embedding
    // Each PartDefinition embeds exactly ONE sub-diagram type.
    // ═══════════════════════════════════════════════════

    // 2A: Part with embedded states only
    part def PartWithStates {
        state def InternalStateMachine {
            state off;
            state standby;
            state active;

            transition first off then standby;
            transition first standby then active;
            transition first active then off;
        }
    }

    // 2B: Part with embedded actions only
    part def PartWithActions {
        action def InternalWorkflow {
            action step1;
            action step2;
            action step3;

            succession first step1 then step2;
            succession first step2 then step3;
        }
    }

    // 2C: Part with embedded interconnection (ports + connections)
    part def PartWithInterconnection {
        part sensor : Sensor;
        part controller : Controller;
        part actuator : Actuator;

        connection sensorToCtrl
            connect sensor.dataOut to controller.dataIn;
        connection ctrlToActuator
            connect controller.ctrlOut to actuator.ctrlIn;
    }

    // 2D: Part with flows (triggers sequence embedding)
    // Sequence elements are wrapped in a comp:sequence compartment with
    // elk.algorithm=fixed to avoid ELK's edge containment validation
    // (message edges reference proxy nodes inside lifelines).
    part def PartWithFlows {
        port input : DataPort;
        port output : DataPort;

        part processor : Controller;
        part sink : Actuator;

        flow of DataPort
            from input to processor.dataIn;
    }

    // ═══════════════════════════════════════════════════
    // SECTION 3: Multi-level embedding (composite)
    // A single PartDefinition embeds MULTIPLE sub-diagram
    // types simultaneously. This is the stress test.
    // ═══════════════════════════════════════════════════

    // 3A: States + Actions (same as state-action-embedded but cleaner)
    part def CompositeStateAction {
        port power : PowerPort;

        state def OperatingMode {
            state startup;
            state nominal;
            state shutdown;

            transition first startup then nominal;
            transition first nominal then shutdown;
        }

        action def StartupSequence {
            action init;
            action calibrate;
            action ready;

            succession first init then calibrate;
            succession first calibrate then ready;
        }

        attribute maxLoad : Real;
    }

    // 3B: States + Actions + Interconnection
    part def CompositeAllViews {
        port powerIn : PowerPort;
        port dataOut : DataPort;

        // State machine
        state def SystemMode {
            state off;
            state booting;
            state online;

            transition first off then booting;
            transition first booting then online;
            transition first online then off;
        }

        // Action flow
        action def BootSequence {
            action loadConfig;
            action initHardware;
            action startServices;

            succession first loadConfig then initHardware;
            succession first initHardware then startServices;
        }

        // Internal structure (interconnection)
        part sensor : Sensor;
        part controller : Controller;
        part actuator : Actuator;

        connection sensorLink
            connect sensor.dataOut to controller.dataIn;
        connection controlLink
            connect controller.ctrlOut to actuator.ctrlIn;

        // Flow (triggers sequence embedding inside fixed-layout compartment)
        flow of DataPort
            from sensor.dataOut to controller.dataIn;
    }

    // ═══════════════════════════════════════════════════
    // SECTION 4: Top-level usages (instances of above)
    // Ensure the expand button works on usages too.
    // ═══════════════════════════════════════════════════

    attribute def Real;

    part system : CompositeAllViews;
    part machine : CompositeStateAction;
    part simple : PartWithStates;
}
