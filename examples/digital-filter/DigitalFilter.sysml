package DigitalFilter {
    doc /* Discrete-time exponential moving average (EMA) filter.
     *
     * Difference equation: x(k+1) = x(k) + alpha * (input(k) - x(k))
     * which is: getDifference = alpha * (u - x)
     *
     * Uses spec-standard StateSpaceRepresentation composite pattern:
     *   action def :> DiscreteStateSpaceDynamics with getDifference.
     *
     * Parameters:
     *   alpha = 0.1 (smoothing factor, 0 < alpha <= 1)
     *
     * Initial condition: filtered = 0.0 */

    private import ScalarValues::*;
    private import StateSpaceRepresentation::*;

    part def EMAFilter {
        doc /* First-order exponential moving average filter.
         * Smooths a noisy input signal with configurable alpha. */

        // Filter parameter
        attribute alpha : Real default 0.1;

        // Input signal (driven externally)
        in attribute u : Real default 0.0;

        // Filtered output (discrete state variable)
        out attribute filtered : Real default 0.0;

        // Formal composite SSR discrete dynamics
        action def EMADynamics :> DiscreteStateSpaceDynamics {
            doc /* Discrete EMA: x(k+1) = x(k) + alpha * (u - x(k)) */

            calc def EMADifference :> GetDifference {
                return dfiltered = alpha * (u - filtered);
            }

            // Output: just the filtered value itself
            calc def FilteredOutput :> GetOutput {
                return y = filtered;
            }
        }
    }

    // Dual-filter example: low-pass + high-pass from same input
    part def DualFilter {
        doc /* Dual EMA: slow filter (alpha=0.05) and fast filter (alpha=0.3).
         * The difference between them approximates a band-pass filter. */

        attribute alpha_slow : Real default 0.05;
        attribute alpha_fast : Real default 0.3;

        in attribute u : Real default 0.0;

        out attribute slow : Real default 0.0;
        out attribute fast : Real default 0.0;

        // Discrete dynamics with two state variables
        action def DualDynamics :> DiscreteStateSpaceDynamics {
            calc def SlowDifference :> GetDifference {
                return dslow = alpha_slow * (u - slow);
            }

            calc def FastDifference :> GetDifference {
                return dfast = alpha_fast * (u - fast);
            }
        }
    }
}
