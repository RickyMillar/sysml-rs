/* Fixture: comprehensive.sysml
Expected: exercises ALL major SysML constructs in one file.
Tests: All 15/15 semantic token types, all construct categories */

package ComprehensiveModel {

    // ── Enums (ENUM token type) ────────────────────────────
    enum def Status {
        enum Active;
        enum Inactive;
        enum Error;
    }

    // ── Attributes (PROPERTY token type) ───────────────────
    attribute def Mass;
    attribute def Speed;
    attribute def Temperature;

    // ── Items (CLASS token type) ───────────────────────────
    item def DataPacket {
        attribute payload : String = "empty";
        attribute size : Integer = 0;
    }

    // ── Parts (CLASS + VARIABLE token types) ───────────────
    part def Engine {
        attribute horsePower : Integer;
        attribute temp : Temperature;
    }

    part def Wheel {
        attribute radius : Real = 0.3;
    }

    // ── Ports and Interfaces (INTERFACE token type) ────────
    port def ControlPort {
        in item command;
        out item feedback;
    }

    interface def ControlInterface {
        end port controller : ControlPort;
        end port actuator : ControlPort;
    }

    // ── Constraints and Requirements (STRUCT token type) ───
    constraint def SpeedLimit {
        attribute maxSpeed : Speed;
    }

    requirement def SafetyReq {
        doc /* Vehicle must be safe. */
        subject part vehicle;
        require constraint limit {
            doc /* Speed must not exceed limit. */
        }
    }

    concern def EmissionsConcern {
        doc /* Emissions must be controlled. */
    }

    // ── Actions (FUNCTION token type) ──────────────────────
    action def StartEngine {
        in item ignitionSignal;
        out item engineStatus;
    }

    action def DriveAction {
        action accelerate;
        action brake;
        send signal : DataPacket;
        accept ack : DataPacket;
    }

    // ── State Machines (CLASS + FUNCTION token types) ──────
    state def EngineState {
        entry; then Off;

        state Off;
        state Starting;
        state Running {
            entry action warmUp;
            do action monitor;
            exit action coolDown;
        }
        state Stopping;

        transition start first Off then Starting;
        transition started first Starting then Running;
        transition stop first Running then Stopping;
        transition stopped first Stopping then Off;
    }

    // ── Calculations (FUNCTION via calc) ───────────────────
    calc def ComputePower {
        in attribute force : Real;
        in attribute velocity : Real;
        return result : Real;
    }

    // ── Connections ────────────────────────────────────────
    connection def DataLink;

    // ── Allocations ────────────────────────────────────────
    allocation def SoftwareAllocation;

    // ── Composite system (combines everything) ─────────────
    part def Vehicle {
        // Parts (VARIABLE)
        part engine : Engine;
        part frontLeft : Wheel;
        part frontRight : Wheel;
        part rearLeft : Wheel;
        part rearRight : Wheel;

        // Attributes (PROPERTY)
        attribute mass : Mass;
        attribute topSpeed : Speed;
        attribute status : Status;
        attribute name : String = "MyVehicle";
        attribute mileage : Real = 0.0;
        attribute vin : Integer = 12345;

        // Ports (INTERFACE)
        port control : ControlPort;

        // State (VARIABLE)
        state engineState : EngineState;

        // Actions (FUNCTION)
        action startSequence : StartEngine;
        action drive : DriveAction;

        // Constraints (STRUCT)
        constraint speedConstraint : SpeedLimit;

        // Requirements (STRUCT)
        requirement safety : SafetyReq;

        // Calculations (FUNCTION)
        calc power : ComputePower;

        // Connection
        connection link : DataLink;

        // Allocation
        allocation sw : SoftwareAllocation;
    }

    // ── Import (NAMESPACE token type) ──────────────────────
    part myCar : Vehicle;
}
