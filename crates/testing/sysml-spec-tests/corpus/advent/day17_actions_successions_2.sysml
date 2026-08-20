package L17_Actions_Successions {

    private import ScalarValues::*;

    attribute def ChildData {
        attribute name : String;
        attribute address : String;
        attribute isNice : Boolean;
    }

    action def ApproachHouse {
        in attribute address : String;
    }

    action def GetGift {
        in attribute child : ChildData;
        out item gift;
    }

    action def GetCoal {
        out item coal;
    }

    action def DeliverGifts {
        in attribute children : ChildData;
        first start;
        then action prepareForTakeOff;
        then action takeOff;
    }
}
