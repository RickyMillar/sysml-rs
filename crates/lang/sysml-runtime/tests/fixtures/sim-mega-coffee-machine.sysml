// sim-mega-coffee-machine.sysml — Comprehensive coffee machine combining
// ODE via SSR (GetDerivative), state machines with entry actions and guards,
// action flows with fork/join, constraints, flows, and verification.

package MegaCoffeeMachine {

    import StateSpaceRepresentation::*;
    import AnalysisTooling::*;

    // --- Thermal body with ODE via SSR ---

    part def Boiler {
        @ToolExecution { toolName = "builtin:ode-rk4"; }

        out attribute temperature : ScalarValues::Real;

        attribute heaterPower : ScalarValues::Real;
        attribute ambientTemp : ScalarValues::Real;
        attribute thermalMass : ScalarValues::Real;
        attribute lossCoefficient : ScalarValues::Real;

        // SSR derivative — spec-standard GetDerivative pattern
        calc def TemperatureDerivative :> GetDerivative {
            return dTdt = (heaterPower - lossCoefficient * (temperature - ambientTemp)) / thermalMass;
        }

        assert constraint safeTemp {
            temperature < 120
        }
    }

    // --- Boiler state machine with entry actions and guards ---

    state def BoilerController {
        entry; then off;

        state off {
            entry action { heaterPower = 0; }
        }

        state heating {
            entry action { heaterPower = 500; }
        }

        state ready {
            entry action { heaterPower = 50; }
        }

        state overtemp {
            entry action { heaterPower = 0; }
        }

        transition off_to_heating
            first off
            accept powerOn
            then heating;

        transition heating_to_ready
            first heating
            if temperature >= 90.0
            then ready;

        transition ready_to_overtemp
            first ready
            if temperature >= 100.0
            then overtemp;

        transition overtemp_to_heating
            first overtemp
            if temperature < 85.0
            then heating;

        transition heating_to_off
            first heating
            accept powerOff
            then off;
    }

    // --- Action flow for brew cycle with fork/join ---

    action def FillWaterAction {
        out item water;
    }

    action def GrindBeansAction {
        out item grounds;
    }

    action def BrewAction {
        in item water;
        in item grounds;
        out item coffee;
    }

    action def BrewCycle {
        action fillWater : FillWaterAction;
        action grindBeans : GrindBeansAction;
        action brew : BrewAction;

        fork prepFork;
        first start then prepFork;
        first prepFork then fillWater;
        first prepFork then grindBeans;

        join prepJoin;
        first fillWater then prepJoin;
        first grindBeans then prepJoin;

        first prepJoin then brew;
        first brew then done;
    }

    // --- Ports for flow routing ---

    port def WaterPort {
        out attribute temperature : ScalarValues::Real;
        out attribute flowRate : ScalarValues::Real;
    }

    part def WaterTank {
        out port waterOut : WaterPort;
        attribute capacity : ScalarValues::Real;
    }

    part def Brewer {
        in port waterIn : ~WaterPort;
        attribute brewTime : ScalarValues::Real;
    }

    // --- Quality constraints ---

    part def QualityModel {
        attribute temperature : ScalarValues::Real;
        attribute brewDuration : ScalarValues::Real;
        attribute pressure : ScalarValues::Real;

        assert constraint brewQuality {
            temperature > 85
        }

        assert constraint tempSafe {
            temperature < 100
        }

        assert constraint timeSufficient {
            brewDuration >= 25
        }

        assert constraint pressureOk {
            pressure >= 8
        }
    }

    // --- Top-level machine assembly ---

    part machine {
        part boiler : Boiler {
            attribute temperature = 20.0;
            attribute heaterPower = 0.0;
            attribute ambientTemp = 20.0;
            attribute thermalMass = 500.0;
            attribute lossCoefficient = 0.05;
        }

        part tank : WaterTank {
            attribute capacity = 2000.0;
        }

        part brewer : Brewer {
            attribute brewTime = 30.0;
        }

        part quality : QualityModel {
            attribute temperature = 93.0;
            attribute brewDuration = 30.0;
            attribute pressure = 9.0;
        }

        exhibit state controller : BoilerController;

        flow waterFlow from tank.waterOut to brewer.waterIn;

        // RSC-3.3b (FL018): declared interface topology for the flow above —
        // Ports.sysml requires transfer endpoints to be interfacing ports
        // (ports connected by a declared interface/connection).
        connect tank.waterOut to brewer.waterIn;
    }
}
