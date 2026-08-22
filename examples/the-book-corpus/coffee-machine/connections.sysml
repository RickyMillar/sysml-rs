// Smart Coffee Machine: Connections and Connectors
// Demonstrates: connection def, connection usage, connector, binding

package Connections {
    import Definitions::*;
    import Ports::*;

    // Connection definition for water piping
    connection def WaterPipe {
        doc /* Physical water pipe connecting two water ports. */
        end source : WaterPort;
        end sink : ~WaterPort;

        attribute pipeLength : Real;    // meters
        attribute pipeDiameter : Real;  // millimeters
    }

    // A fully connected machine using connection usages
    part def ConnectedCoffeeMachine :> CoffeeMachine {
        // Redefine parts with port-equipped variants
        part waterTank : WaterTankWithPorts :>> CoffeeMachine::waterTank;
        part brewer : BrewerWithPorts :>> CoffeeMachine::brewer;

        // Connect water tank output to brewer input
        connection waterLine : WaterPipe
            connect waterTank.waterOut to brewer.waterInlet;

        // Binding connector: control panel command directly bound to brewer.
        // (The named-binding form `binding <name> bind a = b;` doesn't yet
        //  accept multi-segment dotted feature paths in the Pest parser —
        //  the bare `bind` form does, so use it here.)
        bind controlPanel.selectedDrink = brewer.controlIn.command;

        // Simple connector without a definition
        connect waterTank.controlIn to controlPanel;
    }
}
