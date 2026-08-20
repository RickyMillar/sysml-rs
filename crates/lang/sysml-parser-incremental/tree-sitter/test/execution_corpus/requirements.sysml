// Tier 4: Requirement patterns
// Tests: satisfy, verify, subject, objective, frame constraints

package RequirementTests {

    // Basic requirement with subject
    requirement def PerformanceReq {
        subject sys : System;
        require constraint {
            sys.responseTime <= 100
        }
    }

    // Requirement with short name
    requirement def <'REQ-001'> SafetyReq {
        subject vehicle : Vehicle;
        doc /* Vehicle must stop within safe distance */
        assume constraint {
            vehicle.speed >= 0
        }
        require constraint {
            vehicle.brakingDistance <= vehicle.speed * 2.0
        }
    }

    // Satisfy requirement
    part design : Vehicle {
        satisfy requirement safetyCheck : SafetyReq;
    }

    // Verify requirement
    part testSetup {
        verify requirement performanceTest : PerformanceReq;
    }

    // Nested requirements
    requirement def SystemReqs {
        subject sys : System;
        requirement functionalReq {
            require constraint {
                sys.isOperational == true
            }
        }
        requirement reliabilityReq {
            require constraint {
                sys.uptime >= 0.999
            }
        }
    }

    // Objective requirement
    requirement def MissionReq {
        subject mission : Mission;
        objective missionObjective {
            require constraint {
                mission.successRate >= 0.95
            }
        }
    }

    // Frame constraint
    requirement def BoundedReq {
        subject sys : System;
        frame constraint {
            sys.operatingTemp >= -40 and sys.operatingTemp <= 85
        }
    }
}
