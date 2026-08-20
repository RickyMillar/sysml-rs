// SYNTAX TEST "source.sysml"

// --- Relationship keywords in context ---

part def B specializes A { }
//         ^^^^^^^^^^^ keyword.other.relationship.sysml

part def D subsets C { }
//         ^^^^^^^ keyword.other.relationship.sysml

part def F redefines E { }
//         ^^^^^^^^^ keyword.other.relationship.sysml

part def H references G { }
//         ^^^^^^^^^^ keyword.other.relationship.sysml

part def J chains I { }
//         ^^^^^^ keyword.other.relationship.sysml

part def L conjugates K { }
//         ^^^^^^^^^^ keyword.other.relationship.sysml

part def N inverses M { }
//         ^^^^^^^^ keyword.other.relationship.sysml

part def P disjoint O { }
//         ^^^^^^^^ keyword.other.relationship.sysml

part def R intersects Q { }
//         ^^^^^^^^^^ keyword.other.relationship.sysml

part def T unions S { }
//         ^^^^^^ keyword.other.relationship.sysml

part def V differences U { }
//         ^^^^^^^^^^^ keyword.other.relationship.sysml

part def X featuring W { }
//         ^^^^^^^^^ keyword.other.relationship.sysml
