package DampedOscillator {
    doc
    /*
     * A damped harmonic oscillator using the base StateSpaceDynamics pattern.
     *
     * Instead of defining derivatives (ContinuousStateSpaceDynamics) or
     * differences (DiscreteStateSpaceDynamics), this model uses the simplest
     * SSR form: directly computing the next state via GetNextState.
     *
     * Physics: x'' + 2*zeta*omega*x' + omega^2*x = 0
     * Discretized via Euler forward:
     *   v_next = v + dt * (-2*zeta*omega*v - omega^2*x)
     *   x_next = x + dt * v_next
     *
     * Parameters:
     *   omega = 2*pi (natural frequency, 1 Hz)
     *   zeta  = 0.1  (damping ratio, underdamped)
     */

    private import StateSpaceRepresentation::*;

    part def DampedOscillatorModel {
        attribute omega : Real = 6.2832;
        attribute zeta : Real = 0.1;

        action dynamics :> StateSpaceDynamics {
            in attribute input : Input;

            attribute stateSpace : StateSpace;
            out attribute x : Real = 1.0;
            out attribute v : Real = 0.0;

            calc :>> getNextState : GetNextState {
                in input = dynamics::input;
                in stateSpace = dynamics::stateSpace;
                in timeStep = 0.001;

                attribute v_next = v + timeStep * (-2.0 * zeta * omega * v - omega * omega * x);
                return result = x + timeStep * v_next;
            }

            calc :>> getOutput : GetOutput {
                in input = dynamics::input;
                in stateSpace = dynamics::stateSpace;
                return energy = 0.5 * omega * omega * x * x + 0.5 * v * v;
            }

            out attribute output : Output = getOutput(input, stateSpace);
        }
    }
}
