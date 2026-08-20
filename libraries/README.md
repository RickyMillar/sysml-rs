# Libraries

This directory contains SysML v2 libraries used for name resolution and validation.

## Standard Library (default)

`libraries/standard/` mirrors the SysML v2 standard libraries from the Pilot
Implementation reference distribution:

- `library.kernel/` (KerML kernel libraries)
- `library.systems/` (SysML systems libraries)
- `library.domain/` (domain libraries like ISQ, Geometry, etc.)

The loader defaults to this path when `SYSML_LIBRARY_PATH` is not set, so
tests and examples can resolve standard types without extra configuration.

## Overriding

Set `SYSML_LIBRARY_PATH` to point at another library directory (same structure)
to use a different standard library version or custom library set.
