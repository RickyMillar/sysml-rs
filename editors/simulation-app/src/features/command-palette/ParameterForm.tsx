/**
 * ParameterForm — auto-generated form for a backend command.
 *
 * Given a `CommandMeta`, renders one input per declared `ParamMeta`:
 *   - string  -> text input
 *   - number  -> number input
 *   - boolean -> checkbox
 *   - json    -> textarea with JSON.parse validation
 *
 * Optional params (type ending in '?') get a "leave blank to omit" hint
 * and are dropped from the submitted payload when empty.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { CommandMeta, ParamMeta } from './commandCatalog';
import { classifyParamType, isOptionalType } from './commandCatalog';

export interface ParamValues {
  [name: string]: unknown;
}

export interface ParameterFormProps {
  command: CommandMeta;
  onSubmit: (values: ParamValues) => void;
  onCancel: () => void;
  submitting?: boolean;
}

interface FieldState {
  raw: string;
  checked: boolean;
  error: string | null;
}

function initialFieldState(param: ParamMeta): FieldState {
  const kind = classifyParamType(param.ty);
  return {
    raw: '',
    checked: kind === 'boolean' ? false : false,
    error: null,
  };
}

/** Build the payload object. Returns null if a required field is missing
 *  or a JSON field can't be parsed (caller displays per-field errors). */
export function buildPayload(
  params: readonly ParamMeta[],
  fields: Record<string, FieldState>,
): { ok: true; values: ParamValues } | { ok: false; errors: Record<string, string> } {
  const values: ParamValues = {};
  const errors: Record<string, string> = {};

  for (const p of params) {
    const kind = classifyParamType(p.ty);
    const optional = isOptionalType(p.ty) || !p.required;
    const f = fields[p.name] ?? initialFieldState(p);

    if (kind === 'boolean') {
      values[p.name] = f.checked;
      continue;
    }

    const raw = f.raw.trim();
    if (!raw) {
      if (!optional) {
        errors[p.name] = 'required';
      }
      // Optional + blank: skip altogether (don't set to null, the backend's
      // Option<T> deserialiser will just default).
      continue;
    }

    switch (kind) {
      case 'string':
        values[p.name] = f.raw;
        break;
      case 'number': {
        const n = Number(raw);
        if (Number.isNaN(n)) {
          errors[p.name] = 'must be a number';
        } else {
          values[p.name] = n;
        }
        break;
      }
      case 'json': {
        try {
          values[p.name] = JSON.parse(raw);
        } catch (e) {
          errors[p.name] = e instanceof Error ? `invalid JSON: ${e.message}` : 'invalid JSON';
        }
        break;
      }
    }
  }

  if (Object.keys(errors).length > 0) return { ok: false, errors };
  return { ok: true, values };
}

export function ParameterForm({
  command,
  onSubmit,
  onCancel,
  submitting,
}: ParameterFormProps) {
  const [fields, setFields] = useState<Record<string, FieldState>>(() => {
    const s: Record<string, FieldState> = {};
    for (const p of command.params) s[p.name] = initialFieldState(p);
    return s;
  });
  const [formError, setFormError] = useState<string | null>(null);

  // Focus the first field when the form appears.
  //
  // Without this, picking a command left `document.activeElement` on BODY, so
  // the user typed into nothing and had to click the field the form had just
  // asked them to fill (punch-list finding 31). The palette's own search input
  // has already handed focus off by the time this mounts, so nothing is
  // fighting us for it.
  //
  // A ref + effect rather than `autoFocus`: the first field is whichever param
  // comes first, and its element type varies (text / number / textarea /
  // checkbox), so the attribute would have to be threaded through every
  // branch. Re-runs on `command` so switching commands re-focuses.
  const firstFieldRef = useRef<HTMLElement | null>(null);
  useEffect(() => {
    firstFieldRef.current?.focus();
  }, [command]);

  // Reset field state whenever the command changes.
  useEffect(() => {
    const s: Record<string, FieldState> = {};
    for (const p of command.params) s[p.name] = initialFieldState(p);
    setFields(s);
    setFormError(null);
  }, [command]);

  const updateField = useCallback((name: string, patch: Partial<FieldState>) => {
    setFields((prev) => ({
      ...prev,
      [name]: { ...(prev[name] ?? { raw: '', checked: false, error: null }), ...patch },
    }));
  }, []);

  const handleSubmit = useCallback(() => {
    const result = buildPayload(command.params, fields);
    if (!result.ok) {
      setFields((prev) => {
        const next = { ...prev };
        for (const [k, msg] of Object.entries(result.errors)) {
          next[k] = { ...(next[k] ?? { raw: '', checked: false, error: null }), error: msg };
        }
        return next;
      });
      setFormError('Fix the highlighted fields before running.');
      return;
    }
    setFormError(null);
    onSubmit(result.values);
  }, [command.params, fields, onSubmit]);

  const inputs = useMemo(() => command.params.map((p, index) => {
    const kind = classifyParamType(p.ty);
    // Only the first field claims the focus ref (see the effect above).
    const focusRef = index === 0 ? firstFieldRef : undefined;
    const optional = isOptionalType(p.ty) || !p.required;
    const field = fields[p.name] ?? initialFieldState(p);
    const inputId = `cmdk-param-${p.name}`;
    const describedBy = `${inputId}-desc`;

    return (
      <div key={p.name} className="flex flex-col gap-1">
        <label
          htmlFor={inputId}
          className="flex items-center gap-2"
          style={{
            fontSize: 'var(--text-xs)',
            color: 'var(--text-primary)',
            fontWeight: 500,
          }}
        >
          <span>{p.name}</span>
          <span style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)', fontFamily: 'var(--font-mono)' }}>
            {p.ty}
          </span>
          {!optional && (
            <span style={{ color: 'var(--severity-error)', fontSize: 'var(--text-xs)' }}>required</span>
          )}
        </label>

        {kind === 'boolean' ? (
          <label className="inline-flex items-center gap-2" style={{ color: 'var(--text-muted)', fontSize: 'var(--text-sm)' }}>
            <input
              id={inputId}
              ref={focusRef as React.Ref<HTMLInputElement> | undefined}
              type="checkbox"
              data-testid={`cmdk-param-${p.name}`}
              checked={field.checked}
              onChange={(e) => updateField(p.name, { checked: e.target.checked, error: null })}
              aria-describedby={describedBy}
              disabled={submitting}
            />
            <span>{field.checked ? 'true' : 'false'}</span>
          </label>
        ) : kind === 'json' ? (
          <textarea
            id={inputId}
            ref={focusRef as React.Ref<HTMLTextAreaElement> | undefined}
            data-testid={`cmdk-param-${p.name}`}
            value={field.raw}
            onChange={(e) => updateField(p.name, { raw: e.target.value, error: null })}
            placeholder={optional ? 'leave blank to omit' : '{}'}
            rows={4}
            aria-describedby={describedBy}
            aria-invalid={field.error ? 'true' : 'false'}
            disabled={submitting}
            className="mono-text px-2 py-1 rounded"
            style={{
              background: 'var(--surface-raised)',
              border: `1px solid ${field.error ? 'var(--severity-error)' : 'var(--border-default)'}`,
              color: 'var(--text-primary)',
              fontSize: 'var(--text-sm)',
              outline: 'none',
              fontFamily: 'var(--font-mono)',
              resize: 'vertical',
            }}
          />
        ) : (
          <input
            id={inputId}
            ref={focusRef as React.Ref<HTMLInputElement> | undefined}
            data-testid={`cmdk-param-${p.name}`}
            type={kind === 'number' ? 'number' : 'text'}
            value={field.raw}
            onChange={(e) => updateField(p.name, { raw: e.target.value, error: null })}
            placeholder={optional ? 'leave blank to omit' : ''}
            aria-describedby={describedBy}
            aria-invalid={field.error ? 'true' : 'false'}
            disabled={submitting}
            className="mono-text px-2 py-1 rounded"
            style={{
              background: 'var(--surface-raised)',
              border: `1px solid ${field.error ? 'var(--severity-error)' : 'var(--border-default)'}`,
              color: 'var(--text-primary)',
              fontSize: 'var(--text-sm)',
              outline: 'none',
            }}
          />
        )}

        <span
          id={describedBy}
          style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}
        >
          {p.description}
        </span>

        {field.error && (
          <span
            role="alert"
            style={{ color: 'var(--severity-error)', fontSize: 'var(--text-xs)' }}
          >
            {field.error}
          </span>
        )}
      </div>
    );
  }), [command.params, fields, submitting, updateField]);

  return (
    <form
      data-testid="cmdk-parameter-form"
      onSubmit={(e) => {
        e.preventDefault();
        handleSubmit();
      }}
      className="flex flex-col gap-3"
    >
      {command.params.length === 0 && (
        <p style={{ color: 'var(--text-muted)', fontSize: 'var(--text-sm)' }}>
          This command takes no parameters.
        </p>
      )}
      {inputs}
      {formError && (
        <p role="alert" style={{ color: 'var(--severity-error)', fontSize: 'var(--text-xs)' }}>
          {formError}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          data-testid="cmdk-cancel"
          className="px-3 py-1.5 rounded"
          disabled={submitting}
          style={{
            background: 'var(--surface-raised)',
            color: 'var(--text-primary)',
            border: '1px solid var(--border-default)',
            fontSize: 'var(--text-sm)',
            cursor: submitting ? 'default' : 'pointer',
          }}
        >
          Cancel
        </button>
        <button
          type="submit"
          data-testid="cmdk-run"
          className="px-3 py-1.5 rounded font-medium"
          disabled={submitting}
          style={{
            background: submitting ? 'var(--surface-raised)' : 'var(--accent)',
            color: submitting ? 'var(--text-muted)' : 'var(--on-accent)',
            border: 'none',
            fontSize: 'var(--text-sm)',
            cursor: submitting ? 'default' : 'pointer',
          }}
        >
          {submitting ? 'Running…' : 'Run'}
        </button>
      </div>
    </form>
  );
}
