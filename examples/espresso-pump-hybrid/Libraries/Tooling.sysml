package PumpTooling {
    doc /* Local tooling metadata for the fixture.
     *
     * `DataSource` marks a `SampledFunction`-typed attribute as backed by an
     * external CSV table. The simulation runtime reads the annotation's
     * `file`/`domainColumn`/`rangeColumn`/`hasHeader` fields to resolve the CSV
     * (relative to the project source directory) and load it as the sampled
     * function's (domain, range) pairs. A declared `@DataSource` that cannot be
     * resolved is a hard compile error — never a silent empty table. */

    private import ScalarValues::*;

    metadata def DataSource {
        doc /* CSV backing for a SampledFunction. `file` is resolved relative to
         * the project source directory. Columns are zero-based. */
        attribute file : String;
        attribute domainColumn : Integer;
        attribute rangeColumn : Integer;
        attribute hasHeader : Boolean;
    }
}
