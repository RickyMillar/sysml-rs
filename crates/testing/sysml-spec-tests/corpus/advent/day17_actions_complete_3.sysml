package L17_Actions_Complete {

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
        then for child in children {
            ref item toGive;
            action : ApproachHouse {
                in attribute :>> address = child.address;
            }
            then if child.isNice {
                action selectGift : GetGift {
                    in attribute :>> child;
                }
                then assign toGive := selectGift.gift;
            } else {
                action selectCoal : GetCoal;
                then assign toGive := selectCoal.coal;
            }
            then fork;
                then deliverItem;
                then drinkMilk;
            action deliverItem {
                in item = toGive;
            }
                then j;
            action drinkMilk;
                then j;
            join j;
        }
        then action returnHome;
    }
}
