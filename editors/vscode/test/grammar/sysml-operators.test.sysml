// SYNTAX TEST "source.sysml"

// --- Comparison operators ---

x == y;
//^^ keyword.operator.comparison.sysml

x != y;
//^^ keyword.operator.comparison.sysml

x <= y;
//^^ keyword.operator.comparison.sysml

x >= y;
//^^ keyword.operator.comparison.sysml

x < y;
//^ keyword.operator.comparison.sysml

x > y;
//^ keyword.operator.comparison.sysml

x === y;
//^^^ keyword.operator.comparison.sysml

x !== y;
//^^^ keyword.operator.comparison.sysml

// --- Arithmetic operators ---

x + y;
//^ keyword.operator.arithmetic.sysml

x - y;
//^ keyword.operator.arithmetic.sysml

x * y;
//^ keyword.operator.arithmetic.sysml

x / y;
//^ keyword.operator.arithmetic.sysml

x % y;
//^ keyword.operator.arithmetic.sysml

x ** y;
//^^ keyword.operator.arithmetic.sysml

// --- Logical operators (words) ---

x and y;
//^^^ keyword.operator.logical.sysml

x or y;
//^^ keyword.operator.logical.sysml

not x;
//^ keyword.operator.logical.sysml

x xor y;
//^^^ keyword.operator.logical.sysml

x implies y;
//^^^^^^^ keyword.operator.logical.sysml

// --- Assignment operator ---

x = 1;
//^ keyword.operator.assignment.sysml

// --- Type testing operators ---

x hastype T;
//^^^^^^^ keyword.operator.other.sysml

x istype T;
//^^^^^^ keyword.operator.other.sysml

x as T;
//^^ keyword.operator.other.sysml

x meta T;
//^^^^ keyword.operator.other.sysml
