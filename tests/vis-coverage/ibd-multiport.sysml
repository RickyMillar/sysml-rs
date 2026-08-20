// Visualization Coverage Test: IBD with Multiple Port Types
//
// HOW TO TEST:
// 1. Open this file in VS Code
// 2. Ctrl+Shift+P → "SysML: Open Diagram"
// 3. Switch to "Interconnection" view
//
// WHAT TO CHECK:
// - Sensor, Controller, Actuator as rectangles with ports on boundaries
// - DataPort, PowerPort, ControlPort show as distinct port types
// - Flow edges (filled arrow): sensor.dataOut → controller.dataIn
// - Flow edges (filled arrow): controller.commandOut → actuator.commandIn

package IBDMultiPort {
    port def DataPort;
    port def PowerPort;
    port def ControlPort;

    // 3-level nesting: VoltagePort → PowerPort → CompositePort
    port def VoltagePort {
        in port signal;
    }

    port def PowerPort2 {
        out port fiveVolt : VoltagePort;
        out port twelveVolt : VoltagePort;
    }

    // 4C: Composite port with nested sub-ports (each has independent direction)
    port def CompositePort {
        in port data : DataPort;
        out port power : PowerPort2;
    }

    part def Sensor {
        port dataOut : DataPort;
        port powerIn : PowerPort;
    }

    part def Controller {
        port dataIn : DataPort;
        port commandOut : ControlPort;
        port powerIn : PowerPort;
    }

    part def Actuator {
        port commandIn : ControlPort;
        port powerIn : PowerPort;
    }

    // 4D: Gateway with composite port usage
    part def Gateway {
        port composite : CompositePort;
    }

    part def System {
        part sensor : Sensor;
        part controller : Controller;
        part actuator : Actuator;
        part gateway : Gateway;

        // PARSER_GAP: shorthand `flow a.port to b.port` not supported; using `flow of ... from ... to ...`
        flow of DataPort from sensor.dataOut to controller.dataIn;
        flow of ControlPort from controller.commandOut to actuator.commandIn;

        // Flows targeting individual sub-ports of the composite port
        flow of DataPort from sensor.dataOut to gateway.composite.data;
        flow of PowerPort from gateway.composite.power to controller.powerIn;
    }
}
