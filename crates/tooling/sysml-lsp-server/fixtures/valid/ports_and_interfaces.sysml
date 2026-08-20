/* Fixture: ports_and_interfaces.sysml
Expected: valid SysML port and interface constructs.
Tests: PortDef/Usage, InterfaceDef/Usage, ConnectionDef/Usage */

package PortsAndInterfaces {
    attribute def Voltage;
    attribute def Current;

    port def ElectricalPort {
        attribute voltage : Voltage;
        attribute current : Current;
    }

    port def DataPort {
        in item request;
        out item response;
    }

    interface def PowerInterface {
        end port source : ElectricalPort;
        end port sink : ElectricalPort;
    }

    interface def DataInterface {
        end port client : DataPort;
        end port server : DataPort;
    }

    connection def PowerConnection :> PowerInterface;

    part def PowerSupply {
        port output : ElectricalPort;
    }

    part def Motor {
        port input : ElectricalPort;
        port data : DataPort;
    }

    part def System {
        part supply : PowerSupply;
        part motor : Motor;

        connection powerLink : PowerConnection
            connect supply.output to motor.input;

        interface dataLink : DataInterface
            connect motor.data to motor.data;
    }
}
