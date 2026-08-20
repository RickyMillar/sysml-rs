// SYNTAX TEST "source.sysml"

// --- Integer literals ---

42;
// <- constant.numeric.sysml

0;
// <- constant.numeric.sysml

999;
// <- constant.numeric.sysml

// --- Float literals ---

3.14;
// <- constant.numeric.sysml

0.5;
// <- constant.numeric.sysml

// --- Scientific notation ---

1e10;
// <- constant.numeric.sysml

// --- Hex literals ---

0xFF;
// <- constant.numeric.hex.sysml

0x1A2B;
// <- constant.numeric.hex.sysml

// --- String literals ---

"hello";
// <--- string.quoted.double.sysml

"world";
// <--- string.quoted.double.sysml

// --- Boolean constants ---

true;
// <- constant.language.sysml

false;
// <-- constant.language.sysml

// --- Null constant ---

null;
// <- constant.language.sysml
