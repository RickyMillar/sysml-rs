package SantaSleighSpecification {
    part def SantaSleigh;
}

package UserRequirements {
    requirement def <UR1> DeliverGifts {
        subject sleigh : SantaSleighSpecification::SantaSleigh;
        doc /* Santa's sleigh shall deliver gifts to all nice children. */
    }
}

package SantaSleighDesign {
    part def SantaSleigh specializes SantaSleighSpecification::SantaSleigh {
        part cockpit;
        part cargoBay : CargoBay;
        ref part santa;
    }
    abstract part def CargoBay {
        item payload [*] : Bag;
    }
    abstract item def Bag {
        item gifts [*];
    }
}

package RequirementBreakdown {
    private import SantaSleighDesign::*;

    requirement def <TR1> DeliverGifts specializes UserRequirements::DeliverGifts {
        subject redefines sleigh : SantaSleigh;

        requirement : CockpitForSanta;
        requirement : CargoCapacity {
            subject :>> cargoBay = sleigh.cargoBay;
        }
    }

    requirement def <'TR1.1'> CockpitForSanta {
        subject sleigh : SantaSleigh;
        doc /* The sleigh must have a cockpit for Santa. */
    }

    requirement def <'TR1.2'> CargoCapacity {
        subject cargoBay : CargoBay;
        doc /* The cargo bay must be able to accommodate any number of gifts. */
    }
}
