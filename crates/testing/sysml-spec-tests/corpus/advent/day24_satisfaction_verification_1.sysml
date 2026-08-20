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

        satisfy requirement satisfiedReq1 : RequirementBreakdown::DeliverGifts;

    }

    abstract part def CargoBay {

        item payload [*] : Bag;

    }

    part def OrdinaryCaroBay specializes CargoBay {

        item redefines payload [0..1] : BottomlessGiftBag;

        satisfy requirement : RequirementBreakdown::CargoCapacity {

            doc /* The requirement is satisfied via the botomless gift bag
                    * that can hold an arbitrary number of gifts. */

        }

    }

    part def MagicalCargoBay specializes CargoBay {

        item redefines payload : OrdinaryBag;

        satisfy requirement : RequirementBreakdown::CargoCapacity {

            doc /* The requirement is satisfied directly by the magical cargo
                    * bay that can hold an arbitrary number of payloads. */

        }

    }

    abstract item def Bag {

        item gifts [*];

    }

    item def OrdinaryBag specializes Bag {

        item redefines gifts [100];

    }

    item def BottomlessGiftBag specializes Bag {

        item redefines gifts;

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

    }

    requirement def <'TR1.2'> CargoCapacity {

        subject cargoBay : CargoBay;

        doc /* The cargo bay must be able to accommodate any number of gifts. */

    }

}
