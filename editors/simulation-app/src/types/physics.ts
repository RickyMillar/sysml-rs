/**
 * Physics domain types for multi-physics simulation UX.
 *
 * These types drive the System Browser, Domain Lanes, Diagram Overlay,
 * and scope-aware panels. The backend derives them from the model graph
 * using sysml-core's physics classification layer (ISQ types, domain
 * detection, bond graph roles).
 */

// ── Physics Domains ───────────────────────────────────────────────────

export interface PhysicsDomain {
  id: string;
  label: string;
  color: string;
  icon: string;           // Material Symbol name
  conservationLaw?: string;
}

/** Static domain definitions.
 *
 * Colors are the ninebar categorical ramp (`--nb-cat-*` in
 * `styles/tokens.css`, mirrored as `--domain-*`), rendered to literal hex
 * because these values feed canvas-based consumers (uPlot) where CSS
 * `var()` strings cannot resolve. Keep in sync with tokens.css; never add
 * a hue in the reserved accent wedge (OKLCH hue 40–95). Domain identity
 * is also carried by the icon — color is never the only channel. */
export const PHYSICS_DOMAINS: Record<string, PhysicsDomain> = {
  electrical: {
    id: 'electrical',
    label: 'Electrical',
    color: '#2A5C8F', // cat-2 blue
    icon: 'bolt',
    conservationLaw: 'KCL',
  },
  thermal: {
    id: 'thermal',
    label: 'Thermal',
    color: '#8E3A6B', // cat-4 magenta
    icon: 'thermostat',
    conservationLaw: 'Energy balance',
  },
  hydraulic: {
    id: 'hydraulic',
    label: 'Hydraulic',
    color: '#2C6480', // cat-8 cyan
    icon: 'water_drop',
    conservationLaw: 'Mass balance',
  },
  mechanical_translational: {
    id: 'mechanical_translational',
    label: 'Mechanical',
    color: '#4A5F72', // cat-6 slate
    icon: 'settings',
    conservationLaw: "Newton's 2nd law",
  },
  mechanical_rotational: {
    id: 'mechanical_rotational',
    label: 'Rotational',
    color: '#5B4A9E', // cat-3 violet
    icon: 'rotate_right',
    conservationLaw: "Euler's equation",
  },
  protection: {
    id: 'protection',
    label: 'Protection',
    color: '#74438A', // cat-7 plum
    icon: 'shield',
  },
  signal: {
    id: 'signal',
    label: 'Signal',
    color: '#1D6E62', // cat-1 teal
    icon: 'sensors',
  },
  uncategorized: {
    id: 'uncategorized',
    label: 'Other',
    color: '#7E6E58', // n-500 neutral
    icon: 'category',
  },
};

// ── Health Status ─────────────────────────────────────────────────────

export type HealthStatus = 'nominal' | 'warning' | 'critical';

export interface HealthInfo {
  status: HealthStatus;
  message?: string;
}

// ── System Topology (from backend) ────────────────────────────────────

/** Top-level topology of a running simulation. */
export interface SystemTopology {
  /** Root label (e.g., "ProductionCell"). */
  rootLabel: string;

  /** Structural module groups (circuits, thermal network, etc.). */
  modules: ModuleNode[];

  /** Per-domain health summaries. */
  domainSummaries: DomainSummary[];
}

/** A structural group of subsystems (e.g., one station in the production cell). */
export interface ModuleNode {
  /** Unique ID within the topology (e.g., "circuit_7"). */
  id: string;

  /**
   * Diagram element ID (UUID) for overlay mapping.
   * Added per ADR-006: topology carries ElementId so the frontend can
   * attach overlays to diagram nodes without a mapping table.
   */
  element_id?: string;

  /** Display label (e.g., "Station 7"). */
  label: string;

  /** Rating or classification tag (e.g., "C32"). */
  rating?: string;

  /** Primary physics domain of this module. */
  domain: string;

  /** Subsystems within this module. */
  subsystems: SubsystemNode[];

  /** Rolled-up health status (worst of children). */
  health: HealthInfo;
}

/** A single subsystem within a module. */
export interface SubsystemNode {
  /** Subsystem name (matches orchestrator subsystem name). */
  name: string;

  /**
   * Diagram element ID (UUID) for overlay mapping.
   * Added per ADR-006.
   */
  element_id?: string;

  /** Subsystem kind. */
  kind: 'sm' | 'ode' | 'action' | 'discrete';

  /** Physics domain this subsystem belongs to. */
  domain: string;

  /** Current state or primary value. */
  currentState: string;

  /** Last N values for sparkline rendering (most recent last). */
  sparkline: number[];

  /** Health status of this subsystem. */
  health: HealthInfo;

  /** Threshold data for client-side health computation. */
  thresholds?: {
    warnValue?: number;
    criticalValue?: number;
    ratedValue?: number;
    unit?: string;
  };
}

/** Per-domain health summary. */
export interface DomainSummary {
  domain: string;
  status: HealthStatus;
  message: string;
  keyMetrics: DomainMetric[];
}

export interface DomainMetric {
  label: string;
  value: string;
  unit?: string;
  status?: HealthStatus;
}

// ── Diagram Overlay Data ──────────────────────────────────────────────

/** Data needed to render a status overlay on a diagram node. */
export interface NodeOverlay {
  /** Element ID in the diagram (matches the ViewModel scene node id). */
  elementId: string;

  /** Health dot color. */
  health: HealthInfo;

  /** Primary display value (e.g., "32A", "87°C"). */
  primaryValue?: string;

  /** State badge text (e.g., "armed", "Heating"). */
  stateBadge?: string;

  /** Domain for color coding. */
  domain: string;
}

/** Data for animating a flow connection edge. */
export interface FlowOverlay {
  /** Connection element ID in the diagram. */
  connectionId: string;

  /** Flow magnitude (drives stroke width). */
  magnitude: number;

  /** Normalized magnitude (0-1) for animation speed. */
  normalizedMagnitude: number;

  /** Domain for color. */
  domain: string;

  /** Flow direction: true = source→target, false = reverse. */
  forward: boolean;
}
