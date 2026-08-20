package SleighDesign {

    private import BaseComponents::*;
    part def SantaSleigh {
        part cargoBay : CargoBay;
        part dashboard : NaughtyNiceDashboard;

        ref item santa;
        abstract ref part reindeer : Entities::Reindeer[9];
    }

}

package Entities {
    public import ScalarValues::*;
    part def Reindeer {
        public import Datatypes::Color;
        attribute energyLevel : Real;
        constant attribute noseColor : Color;
    }
}

package Datatypes {
    enum def Color {
        //...
    }
}

package BaseComponents {
    part def NaughtyNiceDashboard;

    abstract part def CargoBay {
        item payload [*] : Bag;
    }

    abstract item def Bag {
        item gifts [*];
    }
}
