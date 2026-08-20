package L16_Advanced_Evaluation {

    private import NumericalFunctions::*;
    private import ScalarValues::Natural;
    private import SequenceFunctions::*;
    private import SI::*;
    private import USCustomaryUnits::*;

    attribute maxSafeLoad subsets ISQ::mass = 375 [kg];

    part def SantaSleigh {
        constant attribute sleighWeight subsets ISQ::mass := 850 [kg];

        abstract ref part reindeer : Reindeer [9] = (
            dasher, dancer, prancer, vixen, comet,
            cupid, donner, blitzen, rudolph
        );

        ref part dasher : Reindeer subsets reindeer {
            constant attribute :>> weight := 180 [kg];
            constant attribute :>> power := 5940 [W];
        }
        ref part dancer : Reindeer subsets reindeer {
            constant attribute :>> weight := 175 [kg];
            constant attribute :>> power := 7 [hp];
        }
        ref part prancer : Reindeer subsets reindeer {
            constant attribute :>> weight := 185 [kg];
            constant attribute :>> power := 5550 [W];
        }
        ref part vixen : Reindeer subsets reindeer {
            constant attribute :>> weight := 160 [kg];
            constant attribute :>> power := 4.8 [kW];
        }
        ref part comet : Reindeer subsets reindeer {
            constant attribute :>> weight := 195 [kg];
            constant attribute :>> power := 6142 [W];
        }
        ref part cupid : Reindeer subsets reindeer {
            constant attribute :>> weight := 190 [kg];
            constant attribute :>> power := 5.7 [kW];
        }
        ref part donner : Reindeer subsets reindeer {
            constant attribute :>> weight := 230 [kg];
            constant attribute :>> power := 7935 [W];
        }
        ref part blitzen : Reindeer subsets reindeer {
            constant attribute :>> weight := 225 [kg];
            constant attribute :>> power := 7762 [W];
        }
        ref part rudolph : Reindeer subsets reindeer {
            attribute :>> energyLevel := 0.2 [kW];
            constant attribute :>> weight := 100 [kg];
            constant attribute :>> power := 4950 [W];
        }

        // Example 1: Collection size and basic aggregation
        attribute reindeerCount : Natural = size(reindeer);
        attribute totalPower subsets ISQ::power := sum(reindeer.power);
        attribute averagePower subsets ISQ::power := totalPower / reindeerCount;

        // Example 2: Chained calculations for load analysis
        attribute totalWeight subsets ISQ::mass := sleighWeight + sum(reindeer.weight);
        attribute loadPerReindeer subsets ISQ::mass := totalWeight / reindeerCount;
        attribute safetyMargin subsets ISQ::mass := maxSafeLoad - loadPerReindeer;

        // Example 3: Conditional calculations
        attribute flightMode : FlightMode := if totalWeight > 2500 [kg] ? FlightMode::HeavyHaul else FlightMode::AgileMode;
        attribute maxSpeed = if flightMode == FlightMode::AgileMode ? 140.4 ['km/h'] else 70.2 [km/h];
    }

    enum def FlightMode { AgileMode; HeavyHaul; }

    part def Reindeer {
        attribute energyLevel subsets ISQ::power default 100.0 [W];
        constant attribute weight subsets ISQ::mass default 110 [kg];
        constant attribute power subsets ISQ::power default 5 [kW];
    }
}
