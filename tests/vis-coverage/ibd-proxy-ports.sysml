// Visualization Coverage Test: IBD Proxy Ports & Multi-Container
//
// HOW TO TEST:
// 1. Open in VS Code → Ctrl+Shift+P → "SysML: Open Diagram"
// 2. Switch to "Interconnection" view
//
// WHAT TO CHECK:
// - Context frame (VehicleSystem) has proxy ports on boundary (circles)
// - Binding edges (dashed teal) connect proxy ports to internal part ports
// - Two main subsystems inside: PowerTrain and Electronics
// - Flow edges between subsystems
// - Proxy port triangle indicators flip based on side (in/out)

package IBDProxyPorts {
    port def DataPort;
    port def PowerPort;
    port def DiagPort;

    part def Sensor {
        port dataOut : DataPort;
        port powerIn : PowerPort;
    }

    part def Processor {
        port dataIn : DataPort;
        port resultOut : DataPort;
        port diagOut : DiagPort;
        port powerIn : PowerPort;
    }

    part def Motor {
        port commandIn : DataPort;
        port statusOut : DiagPort;
        port powerIn : PowerPort;
    }

    part def PowerSupply {
        port mainIn : PowerPort;
        port busOut : PowerPort;
    }

    // Top-level system definition with proxy ports on the context boundary
    part def VehicleSystem {
        // These become proxy ports on the IBD context frame
        port externalData : DataPort;
        port diagnosticOut : DiagPort;
        port mainPower : PowerPort;

        // Internal parts
        part sensor : Sensor;
        part processor : Processor;
        part motor : Motor;
        part psu : PowerSupply;

        // Data flow: sensor → processor → motor
        flow of DataPort from sensor.dataOut to processor.dataIn;
        flow of DataPort from processor.resultOut to motor.commandIn;

        // Power distribution: psu feeds all parts
        flow of PowerPort from psu.busOut to sensor.powerIn;
        flow of PowerPort from psu.busOut to processor.powerIn;
        flow of PowerPort from psu.busOut to motor.powerIn;

        // Proxy port bindings: context boundary ↔ internal parts
        binding of externalData = sensor.dataOut;
        binding of diagnosticOut = processor.diagOut;
        binding of mainPower = psu.mainIn;
    }
}
