/**
 * Runnable model target discovery — type definitions.
 *
 * A RunTargetSummary represents a single executable entry point
 * discovered from the loaded workspace model (e.g. a simulation part,
 * analysis case, or verification case).
 *
 * RunTargetGroup bundles targets of the same kind for grouped display.
 */

/** The broad categories of runnable model elements. */
export type RunTargetKind =
  | 'simulation'
  | 'analysisCases'
  | 'verificationSuites';

/** A single executable entry point discovered from the model. */
export interface RunTargetSummary {
  /** Stable element ID from the backend. */
  id: string;
  /** Human-readable element name (may be null for anonymous elements). */
  name: string | null;
  /** Broad target kind for grouping. */
  kind: RunTargetKind;
  /** URI of the model file containing this element. */
  uri: string;
  /** Fully qualified name off the ownership chain (`Pkg::Sub::Case`).
   *  Absent when the element or an ancestor is unnamed. From the
   *  backend `ElementSummary` projection — never derived FE-side. */
  qualifiedName?: string | null;
  /** The qualified path of the OWNING scope (`qualifiedName` minus the
   *  final segment — `Pkg::Sub` for `Pkg::Sub::Case`). The structural
   *  grouping key for compliance-suite grouping: cases declared in the
   *  same package/suite share it. `null` when the target sits at the
   *  root namespace or has no qualified name. */
  ownerPath?: string | null;
  /** Extra metadata from the backend. */
  metadata: {
    /** Backend element kind string (e.g. "PartUsage", "AnalysisCaseUsage"). */
    elementKind: string;
    /** Number of owned child parameters (attributes/features), if known. */
    parameterCount?: number;
  };
}

/** A group of run targets sharing the same kind label. */
export interface RunTargetGroup {
  /** Display label for this group (e.g. "Simulations"). */
  label: string;
  /** The kind all targets in this group share. */
  kind: RunTargetKind;
  /** Material Symbols icon name. */
  icon: string;
  /** Targets in this group, sorted by name. */
  targets: RunTargetSummary[];
}
