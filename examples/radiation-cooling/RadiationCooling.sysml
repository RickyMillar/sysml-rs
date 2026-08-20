package RadiationCooling {
    doc /* Stefan-Boltzmann radiation cooling of a hot body in vacuum.
     *
     * Physics: Q_rad = e*s*A*(T^4 - T_amb^4)
     * ODE: dT/dt = -(e*s*A*(T^4 - T_amb^4)) / (m*c_p)
     *
     * Where:
     *   e = emissivity (0.9 for oxidized steel)
     *   s = 5.67e-8 W/(m^2*K^4) (Stefan-Boltzmann constant)
     *   A = surface area (m^2)
     *   T = body temperature (K)
     *   T_amb = ambient temperature (K)
     *   m*c_p = thermal capacity (J/K)
     *
     * This demonstrates nonlinear T^4 radiation in the ODE engine.
     * The body cools from 1000K to near ambient 300K over ~2000s.
     *
     * Uses spec-standard StateSpaceRepresentation pattern:
     * calc def :> GetDerivative defines dx/dt = f(u, x). */

    private import ISQ::*;
    private import SI::*;
    private import StateSpaceRepresentation::*;

    part def RadiatingBody {
        doc /* Steel billet radiating in vacuum. */

        // Parameters
        attribute emissivity : Real default 0.9;
        attribute stefanBoltzmann : Real default 5.67e-8;
        attribute surfaceArea : AreaValue default 0.1;
        attribute thermalCapacity : HeatCapacityValue default 500.0;
        attribute ambientTemp : ThermodynamicTemperatureValue default 300.0;

        // ODE state variable (direction=out marks it as integrated)
        out attribute temperature : ThermodynamicTemperatureValue default 1000.0;

        // SSR derivative — spec-standard GetDerivative pattern
        calc def TemperatureDerivative :> GetDerivative {
            return dTdt = -(emissivity * stefanBoltzmann * surfaceArea * (temperature ^ 4 - ambientTemp ^ 4)) / thermalCapacity;
        }
    }
}
