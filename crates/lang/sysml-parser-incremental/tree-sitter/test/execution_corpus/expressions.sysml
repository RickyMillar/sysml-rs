// Tier 4: Expression patterns used in execution contexts
// Tests: binary ops, arrows, collect/select, invocations, conditionals

package ExpressionTests {
    private import Base::Anything;
    private import ScalarValues::Boolean;
    private import ScalarValues::Real;


    // Arithmetic in constraints
    constraint def MathCheck {
        attribute a : Real;
        attribute b : Real;
        a + b * 2.0 - 1.0 / (a + 1.0) >= 0
    }

    // Conditional expression
    calc def MaxValue {
        in x : Real;
        in y : Real;
        return result : Real;
        if x > y ? x else y
    }

    // Arrow expressions with collect/select
    part dataProcessor {
        attribute items : Item[0..*];

        // collect pattern
        attribute names = items->collect {in item; item.name};

        // select pattern
        attribute active = items->select {in item; item.isActive};

        // Simple arrow
        attribute count = items->size();

        // Arrow with body
        attribute total = items->reduce '+' ?? 0;
    }

    // Invocation expressions
    calc def ComputeArea {
        in width : Real;
        in height : Real;
        return area : Real;
        width * height
    }

    // Bracket and index expressions
    part indexed {
        attribute matrix : Real[3][3];
        attribute element = matrix#(1)#(2);
        attribute row = matrix[1];
    }

    // Boolean logic
    constraint def LogicCheck {
        attribute a : Boolean;
        attribute b : Boolean;
        attribute c : Boolean;
        (a and b) or (not c) implies a xor b
    }

    // Null coalescing
    part withDefaults {
        attribute name = providedName ?? "unknown";
        attribute value = computedValue ?? defaultValue ?? 0;
    }

    // Classification expressions
    part typed {
        attribute item : Anything;
        assert constraint { item hastype SpecificType }
    }
}
