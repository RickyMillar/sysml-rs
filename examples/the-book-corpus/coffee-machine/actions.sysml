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

    action def ExtractEspresso {
        doc /* Espresso-style extraction under pressure. */
        in item grounds : GroundCoffee;
        in item hotWater : Water;
        out item coffee : BrewedCoffee;
    }

    action def ExtractDrip {
        doc /* Drip-style extraction by gravity. */
        in item grounds : GroundCoffee;
        in item hotWater : Water;
        out item coffee : BrewedCoffee;
    }

    action def DispenseCoffee {
        doc /* Dispense brewed coffee into the cup. */
        in item coffee : BrewedCoffee;
    }

    // Composite action: the full brew cycle with fork/join + decide/merge
    action def BrewCycle {
        doc /* Complete brew cycle with parallel prep and extraction choice. */

        in attribute espressoMode : Boolean;
        in item beans : CoffeeBeans;
        in attribute targetTemp : Real;

        action grind : GrindBeans {
            in item beans :>> GrindBeans::beans = BrewCycle::beans;
        }

        action heat : HeatWater {
            in attribute targetTemp :>> HeatWater::targetTemp = BrewCycle::targetTemp;
        }

        action espressoExtract : ExtractEspresso;
        action dripExtract : ExtractDrip;

        action dispense : DispenseCoffee;

        // Grind and heat in parallel
        fork forkPrep;
            then grind;
            then heat;
        join joinPrep;
        first grind then joinPrep;
        first heat then joinPrep;

        // Choose extraction method
        decide decideMethod;
        first joinPrep then decideMethod;
            first decideMethod then espressoExtract;
            first decideMethod then dripExtract;
        merge mergeMethod;
        first espressoExtract then mergeMethod;
        first dripExtract then mergeMethod;

        // Dispense and finish
        first start then forkPrep;
        first mergeMethod then dispense;
        first dispense then done;
    }

    // Clean cycle action
    action def CleanCycle {
        action flush : HeatWater;
        action rinse : HeatWater;
        action dry;

        first start then flush;
        first flush then rinse;
        first rinse then dry;
        first dry then done;
    }
}
