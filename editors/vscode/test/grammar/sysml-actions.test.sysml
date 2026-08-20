// SYNTAX TEST "source.sysml"

// --- Action and state keywords in context ---

action def MyAction {
//         ^^^^^^^^ entity.name.type.definition.sysml

    entry;
//  ^^^^^ keyword.other.action.sysml

    exit;
//  ^^^^ keyword.other.action.sysml

    first start;
//  ^^^^^ keyword.other.action.sysml

    send x to y;
//  ^^^^ keyword.other.action.sysml

    accept a;
//  ^^^^^^ keyword.other.action.sysml

    assign x;
//  ^^^^^^ keyword.other.action.sysml

    perform p;
//  ^^^^^^^ keyword.other.action.sysml

    include i;
//  ^^^^^^^ keyword.other.action.sysml

    assert c;
//  ^^^^^^ keyword.other.action.sysml

    bind x = y;
//  ^^^^ keyword.other.action.sysml

    connect a to b;
//  ^^^^^^^ keyword.other.action.sysml

    allocate a to b;
//  ^^^^^^^^ keyword.other.action.sysml

    succession s;
//  ^^^^^^^^^^ keyword.other.action.sysml

    transition t;
//  ^^^^^^^^^^ keyword.other.action.sysml

    verify v;
//  ^^^^^^ keyword.other.action.sysml

    exhibit e;
//  ^^^^^^^ keyword.other.action.sysml

    satisfy s;
//  ^^^^^^^ keyword.other.action.sysml

    require r;
//  ^^^^^^^ keyword.other.action.sysml

    terminate;
//  ^^^^^^^^^ keyword.other.action.sysml
}

// --- Control keywords that overlap with action context ---
// 'then' and 'first' are matched by control-keywords in the grammar,
// but 'first' is in action-keywords. Let's check actual scoping:

then next;
// <- keyword.control.sysml

fork f { }
// <- keyword.control.sysml

join j { }
// <- keyword.control.sysml

merge m { }
// <-- keyword.control.sysml

decide d { }
// <--- keyword.control.sysml

parallel p { }
// <---- keyword.control.sysml
