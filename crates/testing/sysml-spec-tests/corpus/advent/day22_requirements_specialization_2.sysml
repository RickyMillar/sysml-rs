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
    part def SantaSleigh specializes SantaSleighSpecification::SantaSleigh;
}

package RequirementBreakdown {
    private import SantaSleighDesign::*;

    requirement def <TR1> DeliverGifts specializes UserRequirements::DeliverGifts {
        subject redefines sleigh : SantaSleigh;
    }
}
