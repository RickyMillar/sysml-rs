/* Fixture: specialization_boundary.sysml
   Expected: S030 or S031 Error — cross-boundary specialization.
     - Part definition cannot specialize an attribute definition
     - Message should mention "attribute" to explain the boundary violation
     - Only S-series errors; no syntax or structural noise
   Tests: specialization_boundary_explains_boundary, specialization_boundary_only_s_series */

package BadSpecialization {
    attribute def MyData;
    part def MyPart :> MyData;
}
