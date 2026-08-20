// Tier 4: Flow connection patterns
// Tests: flow connections, message routing, ports, interfaces

package FlowTests {

    // Basic flow connection
    part def Sensor {
        out port sensorOut : DataPort;
    }

    part def Controller {
        in port controlIn : DataPort;
        out port controlOut : CommandPort;
    }

    part def Actuator {
        in port actuatorIn : CommandPort;
    }

    // Flow connections between parts
    part system {
        part sensor : Sensor;
        part controller : Controller;
        part actuator : Actuator;

        flow of SensorData
            from sensor.sensorOut
            to controller.controlIn;

        flow of Command
            from controller.controlOut
            to actuator.actuatorIn;
    }

    // Interface definition with flow ports
    interface def DataInterface {
        end source : DataPort;
        end sink : DataPort;
        flow of DataPacket from source to sink;
    }

    // Connection usage
    part network {
        part nodeA;
        part nodeB;
        part nodeC;

        connect nodeA to nodeB;
        connect nodeB to nodeC;
    }

    // Binding usage
    part def Composite {
        part inner {
            port p;
        }
        port external;
        binding of inner.p = external;
    }
}
