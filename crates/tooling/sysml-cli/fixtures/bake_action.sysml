package BakingProcess {
    private import ScalarValues::Real;

    part def Oven {
        attribute temperature : Real;
        attribute timer : Real;

        constraint safeTemp { temperature < 300 }
    }

    action def Bake {
        in item dough;
        out item bread;
    }

    part oven : Oven {
        attribute temperature = 180;
        attribute timer = 30;
    }
}
