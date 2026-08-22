// Smart Coffee Machine: Ports and Interfaces
// Demonstrates: port def, port usage, interface def, conjugation, direction

package Ports {
    import Definitions::*;

    // Port definitions define typed connection points
    port def WaterPort {
        doc /* Port for water flow between tank and brewer. */
        attribute flowRate : Real;      // liters per minute
        attribute temperature : Temperature;
    }

    port def PowerPort {
        doc /* Electrical power connection. */
        attribute voltage : Real;       // volts
        attribute current : Real;       // amps
    }

    port def CoffeeOutPort {
        doc /* Output port for brewed coffee. */
        out item brewedCoffee : BrewedCoffee;
    }

    port def SteamPort {
        doc /* Port for steam output. */
        out item steam : Steam;
    }

    port def ControlPort {
        doc /* Bidirectional control and status signals. */
        in attribute command : String;
        out attribute status : String;
    }

    // Interface definition specifying a contract between two ports
    interface def WaterInterface {
        doc /* Contract for water transfer between tank and brewer. */
        end supplierPort : WaterPort;
        end consumerPort : ~WaterPort;  // conjugated port
    }

    interface def PowerInterface {
        end sourcePort : PowerPort;
        end loadPort : ~PowerPort;
    }

    // Parts with port usages
    part def WaterTankWithPorts :> WaterTank {
        port waterOut : WaterPort;
        port controlIn : ~ControlPort;
    }

    part def BrewerWithPorts :> Brewer {
        port waterInlet : ~WaterPort;
        port brewOutput : CoffeeOutPort;
        port controlIn : ~ControlPort;
        port powerIn : ~PowerPort;
    }
}
