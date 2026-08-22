// Smart Coffee Machine: Flows and Messages
// Demonstrates: flow def, flow usage, flow end, succession flow, messages

package Flows {
    import Definitions::*;
    import Ports::*;

    // Flow definition for water movement
    flow def WaterFlow {
        doc /* Defines the flow of water through the machine. */
        end source : WaterPort;
        end sink : ~WaterPort;
        ref item payload : Water;
    }

    // Flow definition for coffee output
    flow def CoffeeFlow {
        end source : CoffeeOutPort;
        ref item payload : BrewedCoffee;
    }

    // Flow definition for steam
    flow def SteamFlow {
        end source : SteamPort;
        ref item payload : Steam;
    }

    // Usage within a machine context showing actual flows
    part def FlowingCoffeeMachine :> CoffeeMachine {
        part waterTank : WaterTankWithPorts :>> CoffeeMachine::waterTank;
        part brewer : BrewerWithPorts :>> CoffeeMachine::brewer;

        // Water flows from tank to brewer
        flow waterToBrewer : WaterFlow
            from waterTank.waterOut
            to brewer.waterInlet;

        // Coffee flows out of the brewer (endpoints omitted — no external target)
        flow coffeeOutput : CoffeeFlow;

        // Succession flow: water must arrive before coffee leaves
        succession flow waterThenCoffee
            from waterToBrewer
            to coffeeOutput;
    }
}
