// Smart Coffee Machine: Actions and Control Behavior
// Demonstrates: action def, action usage, perform, send, accept, control nodes


package Actions {
    import Definitions::*;

    // Simple action definitions for individual steps
    action def GrindBeans {
        doc /* Grind coffee beans to the target coarseness. */
        in item beans : CoffeeBeans;
        out item grounds : GroundCoffee;
    }

    action def HeatWater {
        doc /* Heat water to the target temperature. */
        in attribute targetTemp : Real;
        out item hotWater : Water;
    }

    action def ExtractCoffee {
        doc /* Force hot water through coffee grounds. */
        in item grounds : GroundCoffee;
        in item hotWater : Water;
        out item coffee : BrewedCoffee;
    }

    action def DispenseCoffee {
        doc /* Dispense brewed coffee into the cup. */
        in item coffee : BrewedCoffee;
    }

    // Composite action: the full brew cycle
    action def BrewCycle {
        doc /* Complete brew cycle: grind, heat, extract, dispense.
             * Steps execute in sequence via succession.
             */

        in item beans : CoffeeBeans;
        in attribute targetTemp : Real;

        // Sequential steps
        action grind : GrindBeans {
            in item beans :>> GrindBeans::beans = BrewCycle::beans;
        }

        action heat : HeatWater {
            in attribute targetTemp :>> HeatWater::targetTemp = BrewCycle::targetTemp;
        }

        // Fork: grind and heat happen in parallel
        fork;
            then grind;
            then heat;

        // Join: wait for both to complete
        join;

        action extract : ExtractCoffee {
            in item grounds :>> ExtractCoffee::grounds = grind.grounds;
            in item hotWater :>> ExtractCoffee::hotWater = heat.hotWater;
        }

        action dispense : DispenseCoffee {
            in item coffee :>> DispenseCoffee::coffee = extract.coffee;
        }

        // Succession: grind and heat converge at join, then extract, then dispense
        first grind then join;
        first heat then join;
        first join then extract;
        first extract then dispense;
    }

    // Clean cycle action
    action def CleanCycle {
        action flush : HeatWater;
        then action rinse : HeatWater;
        then action dry;

        first flush then rinse;
        first rinse then dry;
    }
}
