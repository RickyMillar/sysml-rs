package MagicalBagVariant {
    private import BaseComponents::*;
    part def <CargoBay> 'Ordinary Cargo Bay' specializes BaseComponents::CargoBay {
        item redefines payload [0..1] : BottomlessGiftBag;
    }
    item def BottomlessGiftBag specializes Bag {
        item redefines gifts;
    }
}

package MagicalCargoBayVariant {
    private import BaseComponents::*;
    part def 'Magical Cargo Bay' specializes BaseComponents::CargoBay {
        item redefines payload : OrdinaryBag;
    }

    alias CargoBay for 'Magical Cargo Bay';

    item def OrdinaryBag specializes Bag {
        item redefines gifts [100];
    }
}
