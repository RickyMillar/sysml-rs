// Smart Coffee Machine: States and Transitions
// Demonstrates: state def, state usage, transitions, entry/do/exit, triggers, guards

package States {
import Definitions::*;
import Actions::*;

// High-level machine power states (outer hierarchy)
state def MachinePowerStates {
    doc /* Top-level power lifecycle of the coffee machine.
    * The 'operating' state contains all the detailed brew/steam/clean states.
    */

    entry; then off;

    state off {
        doc /* Machine is powered off. */
    }

    state warmingUp {
        doc /* Boiler heating to operating temperature. */
        entry action startHeater;
        do action monitorBoilerTemp;
    }

    state operating {
        doc /* Machine is at temperature and ready for use.
        * Contains the detailed MachineStates sub-machine.
        */
    }
    
    state standby {
        doc /* Energy-saving mode after inactivity timeout. */
        entry action dimDisplay;
        exit action wakeDisplay;
    }

    // Power transitions
    transition off_to_warmingUp
        first off
        accept powerOn
        then warmingUp;

    transition warmingUp_to_off
        first warmingUp
        accept powerOff
        then off;

    transition warmingUp_to_operating
        first warmingUp
        accept boilerReady
        then operating;

    transition operating_to_warmingUp
        first operating
        accept boilerCooled
        then warmingUp;

    transition operating_to_standby
        first operating
        accept inactivityTimeout
        then standby;

    transition standby_to_operating
        first standby
        accept userWake
        then operating;

    transition operating_to_off
        first operating
        accept powerOff
        then off;

    transition standby_to_off
        first standby
        accept powerOff
        then off;
}

// State definition for the machine operating lifecycle
state def MachineStates {
    doc /* Detailed operating state machine for the coffee machine.
    * Models the brew/steam/clean modes and transitions between them.
    * Nested inside MachinePowerStates.operating.
    */

    entry; then idle;

    state idle {
        doc /* Machine is on and waiting for user input. */
        entry action resetDisplay;
    }

    state brewing {
        doc /* Machine is executing a brew cycle. */
        entry action startBrew : BrewCycle;
        do action monitorTemp;
        exit action stopPump;
    }

    state steaming {
        doc /* Steam wand is active. */
        entry action openSteamValve;
        do action regulateSteam;
        exit action closeSteamValve;
    }



    state cleaning {
        doc /* Machine is running an automatic clean cycle. */
        entry action startClean : CleanCycle;
    }

    state error {
        doc /* Machine detected a fault condition. */
        entry action displayError;
    }

    // Transitions with triggers and guards
    transition idle_to_brewing
        first idle
        accept brewCommand
        if cupDetected
        then brewing;

    transition idle_to_steaming
        first idle
        accept steamCommand
        then steaming;

    transition brewing_to_idle
        first brewing
        accept brewComplete
        then idle;

    transition steaming_to_idle
        first steaming
        accept steamComplete
        then idle;

    transition idle_to_cleaning
        first idle
        accept cleanCommand
        then cleaning;

    transition cleaning_to_idle
        first cleaning
        accept cleanComplete
        then idle;

    // Any state can transition to error on fault
    transition any_to_error
        first idle
        accept faultDetected
        then error;

    transition error_to_idle
        first error
        accept errorCleared
        then idle;

}

// ── Composite state machine coordinating both sub-machines ──
// Per SysML v2 spec, transitions must be owned by a StateDefinition
// or StateUsage. A composite state owns both sub-machines and the
// cross-machine transitions between them.

state def CoffeeMachineLifecycle {
    doc /* Top-level lifecycle that coordinates power management and
    * operating modes. Owns both sub-machines as state usages,
    * with transitions between them at this level.
    */

    entry; then powerStates;

    state powerStates : MachinePowerStates {
        doc /* Power management sub-machine. */
    }

    state operatingStates : MachineStates {
        doc /* Operating mode sub-machine (brew/steam/clean). */
    }

    // Cross-machine transitions (valid: owned by this composite state)
    transition power_to_ops
        first powerStates
        accept enterOperating
        then operatingStates;

    transition ops_to_power
        first operatingStates
        accept escalateShutdown
        then powerStates;
    


   

}}
