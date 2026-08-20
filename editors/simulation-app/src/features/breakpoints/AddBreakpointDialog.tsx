/**
 * AddBreakpointDialog — modal for authoring a new breakpoint (R2.3).
 *
 * UX contract:
 *   - Kind dropdown picks one of the five planned `BreakpointKind`s.
 *   - The target picker is kind-dependent:
 *       state / transition / action / constraint → element-id autocomplete
 *       threshold                                → variable name + operator + number
 *   - "Advanced" expander exposes Round-4 inputs (condition, hit count,
 *     log message) behind a greyed-out "Coming in Round 4" badge. The UI
 *     real estate is reserved so the later agent drops in wiring only.
 *   - Save → call `onSubmit(breakpoint, advanced)` and close.
 *   - Cancel / Esc / backdrop click → close without saving.
 *   - Enter in any field submits when the form is valid.
 *
 * The dialog is deliberately pure-presentational — the panel owns the
 * store dispatch and the backend round-trip, so this component is easy
 * to snapshot-test and to reuse from a future "edit breakpoint" dialog.
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import type { Breakpoint, BreakpointKind, CompareOp } from '@/engine/types';
import { FuzzyCombobox, type FuzzyCandidate } from '@/shared/pickers/FuzzyCombobox';

// ── Types ────────────────────────────────────────────────────────────

export type ThresholdOperator = 'rising' | 'falling' | 'either';

/** Dropdown options for the 6 compare operators (kebab-case mirrors
 *  Rust `CompareOp` and the JSON tag). Exported so tests can assert the
 *  dialog covers them all. */
export const COMPARE_OPS: Array<{ value: CompareOp; label: string }> = [
  { value: 'lt', label: '<   less than' },
  { value: 'le', label: '≤   less or equal' },
  { value: 'gt', label: '>   greater than' },
  { value: 'ge', label: '≥   greater or equal' },
  { value: 'eq', label: '=   equal' },
  { value: 'ne', label: '≠   not equal' },
];

/** Round-4 extensibility payload — accepted today, ignored by the
 *  backend. Kept separate from `Breakpoint` so the spec-aligned engine
 *  type stays clean. */
export interface AdvancedFields {
  condition?: string;
  hitCount?: number;
  logMessage?: string;
}

export interface AddBreakpointDialogProps {
  open: boolean;
  onClose: () => void;
  onSubmit: (breakpoint: Breakpoint, advanced: AdvancedFields) => void;

  /**
   * Autocomplete pool for element-targeted breakpoints. Panels typically
   * supply a flat list built from model capabilities + session topology.
   * Optional — when empty the user types freely (validation still rejects
   * blanks).
   */
  elementCandidates?: readonly FuzzyCandidate[];
  /** Autocomplete pool for threshold breakpoints (variable names). */
  variableCandidates?: readonly string[];

  /** Pre-fill the kind (used when launching from a context menu). */
  defaultKind?: BreakpointKind;

  /** Busy flag — parent sets while the backend call is in-flight. */
  submitting?: boolean;
}

// Map the user-facing operator to the backend `direction` enum and a
// printable glyph for the dropdown.
const OPERATORS: Array<{ value: ThresholdOperator; label: string }> = [
  { value: 'either', label: '⇅  crossing' },
  { value: 'rising', label: '↑  rising' },
  { value: 'falling', label: '↓  falling' },
];

const KIND_LABELS: Record<BreakpointKind, string> = {
  'state-entry': 'State entry',
  'transition-fire': 'Transition fire',
  'action-invoke': 'Action invoke',
  'constraint-violation': 'Constraint violation',
  'threshold-crossing': 'Threshold crossing',
  'conditional': 'Conditional',
};

const ELEMENT_KINDS: ReadonlySet<BreakpointKind> = new Set([
  'state-entry',
  'transition-fire',
  'action-invoke',
  'constraint-violation',
]);

// ── Validation (pure, exported for tests) ────────────────────────────

export interface FormValues {
  kind: BreakpointKind;
  target: string;
  variable: string;
  threshold: string; // raw input — validated then parsed
  direction: ThresholdOperator;
  /** Debounce window (in ticks) for threshold-crossing — raw input; parsed
   *  on submit. Blank ⇒ 0 (no debouncing). */
  debounceTicks: string;
  /** Conditional breakpoint operator. */
  compareOp: CompareOp;
  /** Conditional breakpoint comparison value — raw input; parsed on submit. */
  conditionalValue: string;
  // Advanced
  condition: string;
  hitCount: string;
  logMessage: string;
}

export function defaultFormValues(kind: BreakpointKind = 'state-entry'): FormValues {
  return {
    kind,
    target: '',
    variable: '',
    threshold: '',
    direction: 'either',
    debounceTicks: '',
    compareOp: 'gt',
    conditionalValue: '',
    condition: '',
    hitCount: '',
    logMessage: '',
  };
}

export interface ValidationResult {
  ok: boolean;
  error?: string;
  breakpoint?: Breakpoint;
  advanced?: AdvancedFields;
}

/**
 * Build a `Breakpoint` + `AdvancedFields` tuple from a form snapshot.
 * Returns `{ ok: false, error }` when validation fails. Pure — unit-
 * tested without mounting the component.
 */
export function validateForm(values: FormValues): ValidationResult {
  const advanced: AdvancedFields = {};
  if (values.condition.trim()) advanced.condition = values.condition.trim();
  if (values.logMessage.trim()) advanced.logMessage = values.logMessage.trim();
  if (values.hitCount.trim()) {
    const n = Number(values.hitCount);
    if (!Number.isFinite(n) || n <= 0 || !Number.isInteger(n)) {
      return { ok: false, error: 'Hit count must be a positive integer.' };
    }
    advanced.hitCount = n;
  }

  if (ELEMENT_KINDS.has(values.kind)) {
    const target = values.target.trim();
    if (!target) {
      return { ok: false, error: 'Pick an element target.' };
    }
    const bp: Breakpoint = { kind: values.kind, target } as Breakpoint;
    return { ok: true, breakpoint: bp, advanced };
  }

  if (values.kind === 'conditional') {
    const target = values.target.trim();
    if (!target) {
      return { ok: false, error: 'Pick an element target for the conditional.' };
    }
    const variable = values.variable.trim();
    if (!variable) return { ok: false, error: 'Pick a variable to watch.' };
    const value = Number(values.conditionalValue);
    if (!Number.isFinite(value)) {
      return { ok: false, error: 'Comparison value must be a finite number.' };
    }
    const bp: Breakpoint = {
      kind: 'conditional',
      target,
      variable,
      op: values.compareOp,
      value,
    };
    return { ok: true, breakpoint: bp, advanced };
  }

  // threshold-crossing
  const variable = values.variable.trim();
  if (!variable) return { ok: false, error: 'Pick a variable.' };
  const threshold = Number(values.threshold);
  if (!Number.isFinite(threshold)) {
    return { ok: false, error: 'Threshold must be a finite number.' };
  }
  let debounceTicks = 0;
  if (values.debounceTicks.trim()) {
    const n = Number(values.debounceTicks);
    if (!Number.isFinite(n) || n < 0 || !Number.isInteger(n)) {
      return { ok: false, error: 'Debounce ticks must be a non-negative integer.' };
    }
    debounceTicks = n;
  }
  const bp: Breakpoint = {
    kind: 'threshold-crossing',
    target: variable,
    variable,
    threshold,
    direction: values.direction,
    ...(debounceTicks > 0 ? { debounce_ticks: debounceTicks } : {}),
  };
  return { ok: true, breakpoint: bp, advanced };
}

// ── Component ────────────────────────────────────────────────────────

export function AddBreakpointDialog(props: AddBreakpointDialogProps) {
  const {
    open,
    onClose,
    onSubmit,
    elementCandidates = [],
    variableCandidates = [],
    defaultKind = 'state-entry',
    submitting = false,
  } = props;

  const [values, setValues] = useState<FormValues>(() => defaultFormValues(defaultKind));
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const firstInputRef = useRef<HTMLSelectElement | null>(null);

  // Reset form each time the dialog reopens so stale values don't leak.
  useEffect(() => {
    if (!open) return;
    setValues(defaultFormValues(defaultKind));
    setShowAdvanced(false);
    setError(null);
    // Focus the kind picker on next tick.
    const t = window.setTimeout(() => firstInputRef.current?.focus(), 0);
    return () => window.clearTimeout(t);
  }, [open, defaultKind]);

  // Esc closes (modal-wide). The global shortcut hook also handles Esc,
  // but a local listener ensures focus trapping works even when the
  // panel isn't mounted.
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [open, onClose]);

  const isThreshold = values.kind === 'threshold-crossing';
  const isConditional = values.kind === 'conditional';

  const handleSubmit = useCallback(
    (event?: React.FormEvent) => {
      event?.preventDefault();
      const result = validateForm(values);
      if (!result.ok) {
        setError(result.error ?? 'Invalid input.');
        return;
      }
      setError(null);
      onSubmit(result.breakpoint!, result.advanced ?? {});
    },
    [values, onSubmit],
  );

  const set = useCallback(<K extends keyof FormValues>(key: K, value: FormValues[K]) => {
    setValues((prev) => ({ ...prev, [key]: value }));
    setError(null);
  }, []);

  // ── Render ────────────────────────────────────────────────────────
  if (!open) return null;

  return (
    <div
      data-testid="bp-dialog-overlay"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'var(--surface-scrim)',
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: '12vh',
        zIndex: 10000,
      }}
    >
      <form
        data-testid="bp-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Add breakpoint"
        onSubmit={handleSubmit}
        style={{
          width: 'min(480px, 92vw)',
          background: 'var(--surface-panel)',
          border: '1px solid var(--border-default)',
          borderRadius: 8,
          // shadow = warm ink, never black (tokens.css elevation rule)
          boxShadow: 'var(--shadow-float)',
          padding: 16,
          display: 'flex',
          flexDirection: 'column',
          gap: 12,
        }}
      >
        <header style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
          <span
            className="material-symbols-outlined"
            aria-hidden="true"
            style={{ fontSize: 18, color: 'var(--sim-breakpoint-mark)' }}
          >
            radio_button_checked
          </span>
          <h2 style={{ margin: 0, fontSize: 'var(--text-sm, 13px)', color: 'var(--text-primary)' }}>
            Add breakpoint
          </h2>
          <div style={{ flex: 1 }} />
          <button
            type="button"
            data-testid="bp-dialog-close"
            aria-label="Close dialog"
            onClick={onClose}
            style={iconButtonStyle()}
          >
            <span className="material-symbols-outlined" style={{ fontSize: 16 }} aria-hidden="true">
              close
            </span>
          </button>
        </header>

        {/* Kind picker */}
        <label style={fieldLabelStyle()}>
          Kind
          <select
            ref={firstInputRef}
            data-testid="bp-dialog-kind"
            value={values.kind}
            onChange={(e) => set('kind', e.target.value as BreakpointKind)}
            style={fieldInputStyle()}
          >
            {(Object.keys(KIND_LABELS) as BreakpointKind[]).map((k) => (
              <option key={k} value={k}>
                {KIND_LABELS[k]}
              </option>
            ))}
          </select>
        </label>

        {/* Target picker — kind-dependent */}
        {!isThreshold && !isConditional && (
          <ElementTargetField
            value={values.target}
            onChange={(v) => set('target', v)}
            candidates={elementCandidates}
          />
        )}
        {isThreshold && (
          <ThresholdTargetFields
            variable={values.variable}
            onVariableChange={(v) => set('variable', v)}
            direction={values.direction}
            onDirectionChange={(d) => set('direction', d)}
            threshold={values.threshold}
            onThresholdChange={(t) => set('threshold', t)}
            debounceTicks={values.debounceTicks}
            onDebounceTicksChange={(v) => set('debounceTicks', v)}
            candidates={variableCandidates}
          />
        )}
        {isConditional && (
          <ConditionalTargetFields
            target={values.target}
            onTargetChange={(v) => set('target', v)}
            variable={values.variable}
            onVariableChange={(v) => set('variable', v)}
            compareOp={values.compareOp}
            onCompareOpChange={(op) => set('compareOp', op)}
            conditionalValue={values.conditionalValue}
            onConditionalValueChange={(v) => set('conditionalValue', v)}
            elementCandidates={elementCandidates}
            variableCandidates={variableCandidates}
          />
        )}

        {/* Advanced (Round-4 hooks) */}
        <details
          data-testid="bp-dialog-advanced"
          open={showAdvanced}
          onToggle={(e) => setShowAdvanced((e.target as HTMLDetailsElement).open)}
          style={{
            borderTop: '1px dashed var(--border-default)',
            paddingTop: 8,
          }}
        >
          <summary
            style={{
              cursor: 'pointer',
              color: 'var(--text-muted)',
              fontSize: 'var(--text-xs, 11px)',
              userSelect: 'none',
            }}
          >
            Advanced <span style={comingBadgeStyle()}>Not wired yet</span>
          </summary>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8, marginTop: 8 }}>
            <AdvancedField
              testid="bp-dialog-condition"
              label="Condition"
              placeholder="e.g. I_total > 32"
              value={values.condition}
              onChange={(v) => set('condition', v)}
            />
            <AdvancedField
              testid="bp-dialog-hitcount"
              label="Hit count"
              placeholder="e.g. 10"
              value={values.hitCount}
              onChange={(v) => set('hitCount', v)}
              inputType="number"
            />
            <AdvancedField
              testid="bp-dialog-logmessage"
              label="Log message"
              placeholder="Logpoint — log instead of pause"
              value={values.logMessage}
              onChange={(v) => set('logMessage', v)}
            />
          </div>
        </details>

        {/* Error + footer */}
        {error && (
          <div
            role="alert"
            data-testid="bp-dialog-error"
            style={{
              color: 'var(--severity-error)',
              fontSize: 'var(--text-xs, 11px)',
              padding: '4px 6px',
              background: 'color-mix(in srgb, var(--severity-error) 8%, transparent)',
              border: '1px solid color-mix(in srgb, var(--severity-error) 30%, transparent)',
              borderRadius: 4,
            }}
          >
            {error}
          </div>
        )}

        <footer style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button
            type="button"
            data-testid="bp-dialog-cancel"
            onClick={onClose}
            style={secondaryButtonStyle()}
          >
            Cancel
          </button>
          <button
            type="submit"
            data-testid="bp-dialog-save"
            disabled={submitting}
            style={primaryButtonStyle(submitting)}
          >
            {submitting ? 'Saving…' : 'Save'}
          </button>
        </footer>
      </form>
    </div>
  );
}

// ── Sub-fields ───────────────────────────────────────────────────────

interface ElementTargetFieldProps {
  value: string;
  onChange: (v: string) => void;
  candidates: readonly FuzzyCandidate[];
}

function ElementTargetField({ value, onChange, candidates }: ElementTargetFieldProps) {
  return (
    <label style={fieldLabelStyle()}>
      Target
      <FuzzyCombobox
        testId="bp-dialog-target"
        placeholder="Element qualified name"
        value={value}
        onChange={onChange}
        candidates={candidates}
        inputStyle={fieldInputStyle()}
      />
    </label>
  );
}

interface ThresholdTargetFieldsProps {
  variable: string;
  onVariableChange: (v: string) => void;
  direction: ThresholdOperator;
  onDirectionChange: (d: ThresholdOperator) => void;
  threshold: string;
  onThresholdChange: (t: string) => void;
  debounceTicks: string;
  onDebounceTicksChange: (v: string) => void;
  candidates: readonly string[];
}

function ThresholdTargetFields(p: ThresholdTargetFieldsProps) {
  return (
    <>
      <label style={fieldLabelStyle()}>
        Variable
        <FuzzyCombobox
          testId="bp-dialog-variable"
          placeholder="Variable name"
          value={p.variable}
          onChange={p.onVariableChange}
          candidates={p.candidates}
          inputStyle={fieldInputStyle()}
        />
      </label>
      <div style={{ display: 'flex', gap: 8 }}>
        <label style={{ ...fieldLabelStyle(), flex: 1 }}>
          Direction
          <select
            data-testid="bp-dialog-direction"
            value={p.direction}
            onChange={(e) => p.onDirectionChange(e.target.value as ThresholdOperator)}
            style={fieldInputStyle()}
          >
            {OPERATORS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label style={{ ...fieldLabelStyle(), flex: 1 }}>
          Threshold
          <input
            data-testid="bp-dialog-threshold"
            type="number"
            step="any"
            placeholder="0.0"
            value={p.threshold}
            onChange={(e) => p.onThresholdChange(e.target.value)}
            style={fieldInputStyle()}
          />
        </label>
      </div>
      <label style={fieldLabelStyle()}>
        Debounce (ticks)
        <input
          data-testid="bp-dialog-debounce-ticks"
          type="number"
          step="1"
          min="0"
          placeholder="0 (no debounce)"
          value={p.debounceTicks}
          onChange={(e) => p.onDebounceTicksChange(e.target.value)}
          style={fieldInputStyle()}
        />
      </label>
    </>
  );
}

interface ConditionalTargetFieldsProps {
  target: string;
  onTargetChange: (v: string) => void;
  variable: string;
  onVariableChange: (v: string) => void;
  compareOp: CompareOp;
  onCompareOpChange: (op: CompareOp) => void;
  conditionalValue: string;
  onConditionalValueChange: (v: string) => void;
  elementCandidates: readonly FuzzyCandidate[];
  variableCandidates: readonly string[];
}

function ConditionalTargetFields(p: ConditionalTargetFieldsProps) {
  return (
    <>
      <label style={fieldLabelStyle()}>
        Element
        <FuzzyCombobox
          testId="bp-dialog-cond-target"
          placeholder="Owning element (part, state, ...)"
          value={p.target}
          onChange={p.onTargetChange}
          candidates={p.elementCandidates}
          inputStyle={fieldInputStyle()}
        />
      </label>
      <label style={fieldLabelStyle()}>
        Variable
        <FuzzyCombobox
          testId="bp-dialog-cond-variable"
          placeholder="Variable name (e.g. voltage)"
          value={p.variable}
          onChange={p.onVariableChange}
          candidates={p.variableCandidates}
          inputStyle={fieldInputStyle()}
        />
      </label>
      <div style={{ display: 'flex', gap: 8 }}>
        <label style={{ ...fieldLabelStyle(), flex: 1 }}>
          Operator
          <select
            data-testid="bp-dialog-cond-op"
            value={p.compareOp}
            onChange={(e) => p.onCompareOpChange(e.target.value as CompareOp)}
            style={fieldInputStyle()}
          >
            {COMPARE_OPS.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </select>
        </label>
        <label style={{ ...fieldLabelStyle(), flex: 1 }}>
          Value
          <input
            data-testid="bp-dialog-cond-value"
            type="number"
            step="any"
            placeholder="0.0"
            value={p.conditionalValue}
            onChange={(e) => p.onConditionalValueChange(e.target.value)}
            style={fieldInputStyle()}
          />
        </label>
      </div>
    </>
  );
}

interface AdvancedFieldProps {
  label: string;
  placeholder: string;
  value: string;
  onChange: (v: string) => void;
  inputType?: 'text' | 'number';
  testid: string;
}

function AdvancedField({ label, placeholder, value, onChange, inputType = 'text', testid }: AdvancedFieldProps) {
  return (
    <label style={{ ...fieldLabelStyle(), opacity: 0.65 }}>
      <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
        {label}
        <span style={comingBadgeStyle()}>Round 4</span>
      </span>
      <input
        data-testid={testid}
        type={inputType}
        placeholder={placeholder}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        style={{ ...fieldInputStyle(), fontStyle: 'italic' }}
      />
    </label>
  );
}

// ── Styles (inline — match CommandPalette conventions) ──────────────

function fieldLabelStyle(): React.CSSProperties {
  return {
    display: 'flex',
    flexDirection: 'column',
    gap: 4,
    color: 'var(--text-muted)',
    fontSize: 'var(--text-xs, 11px)',
  };
}

function fieldInputStyle(): React.CSSProperties {
  return {
    background: 'var(--surface-raised)',
    border: '1px solid var(--border-default)',
    color: 'var(--text-primary)',
    borderRadius: 4,
    padding: '6px 8px',
    fontSize: 'var(--text-sm, 12px)',
    outline: 'none',
  };
}

function iconButtonStyle(): React.CSSProperties {
  return {
    background: 'transparent',
    border: 'none',
    color: 'var(--text-muted)',
    cursor: 'pointer',
    padding: 2,
  };
}

function primaryButtonStyle(disabled: boolean): React.CSSProperties {
  return {
    background: disabled ? 'var(--border-default)' : 'var(--accent)',
    color: disabled ? 'var(--text-muted)' : 'var(--on-accent)',
    border: 'none',
    padding: '6px 14px',
    borderRadius: 4,
    cursor: disabled ? 'not-allowed' : 'pointer',
    fontSize: 'var(--text-xs, 11px)',
    fontWeight: 500,
  };
}

function secondaryButtonStyle(): React.CSSProperties {
  return {
    background: 'transparent',
    color: 'var(--text-primary)',
    border: '1px solid var(--border-default)',
    padding: '6px 14px',
    borderRadius: 4,
    cursor: 'pointer',
    fontSize: 'var(--text-xs, 11px)',
  };
}

function comingBadgeStyle(): React.CSSProperties {
  return {
    fontSize: 9,
    padding: '1px 4px',
    borderRadius: 3,
    background: 'var(--surface-raised)',
    color: 'var(--text-muted)',
    marginLeft: 6,
    letterSpacing: 0.3,
  };
}
