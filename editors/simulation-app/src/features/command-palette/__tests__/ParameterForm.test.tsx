/**
 * ParameterForm pure-logic tests.
 *
 * Exercises the auto-generated form's core behaviour without mounting
 * React: type classification, optional-param detection, and payload
 * building for every supported parameter kind.
 *
 * This is the contract that lets a generic form work for all 48+
 * backend commands — if it holds, the form renders the right input for
 * every command the catalogue can throw at it.
 */

import { describe, expect, it } from 'vitest';
import { buildPayload } from '../ParameterForm';
import {
  classifyParamType,
  isOptionalType,
  type CommandMeta,
  type ParamMeta,
} from '../commandCatalog';

// ── Fixture: one command covering every input kind ───────────────────────

const ALL_KINDS_COMMAND: CommandMeta = {
  name: 'demo.all_kinds',
  category: 'Query',
  description: 'Fixture that exercises every auto-generated input kind',
  params: [
    { name: 'text_in', ty: 'String', required: true, description: 'A string' },
    { name: 'count', ty: 'usize', required: true, description: 'An integer' },
    { name: 'ratio', ty: 'f64', required: true, description: 'A float' },
    { name: 'flag', ty: 'bool', required: true, description: 'A boolean' },
    { name: 'payload', ty: 'Vec<String>', required: true, description: 'A JSON value' },
    { name: 'maybe_name', ty: 'String?', required: false, description: 'Optional string' },
  ],
  returns: 'Value',
  stateful: false,
};

describe('ParameterForm — type classification', () => {
  it('classifies every Rust param type string into a UI kind', () => {
    expect(classifyParamType('String')).toBe('string');
    expect(classifyParamType('&str')).toBe('string');
    expect(classifyParamType('PathBuf')).toBe('string');

    expect(classifyParamType('usize')).toBe('number');
    expect(classifyParamType('i32')).toBe('number');
    expect(classifyParamType('f64')).toBe('number');
    expect(classifyParamType('integer')).toBe('number');

    expect(classifyParamType('bool')).toBe('boolean');
    expect(classifyParamType('boolean')).toBe('boolean');

    expect(classifyParamType('Vec<String>')).toBe('json');
    expect(classifyParamType('ElementKind')).toBe('json');
    expect(classifyParamType('HashMap<String, Value>')).toBe('json');
  });

  it('respects trailing ? as optional regardless of base kind', () => {
    expect(classifyParamType('String?')).toBe('string');
    expect(classifyParamType('bool?')).toBe('boolean');
    expect(classifyParamType('usize?')).toBe('number');
    expect(classifyParamType('Vec<u8>?')).toBe('json');

    expect(isOptionalType('String?')).toBe(true);
    expect(isOptionalType('String')).toBe(false);
    expect(isOptionalType('')).toBe(false);
  });

  it('generates an input kind for every ParamMeta on a realistic command', () => {
    const kinds = ALL_KINDS_COMMAND.params.map((p) => classifyParamType(p.ty));
    expect(kinds).toEqual(['string', 'number', 'number', 'boolean', 'json', 'string']);
  });
});

// ── Payload building ─────────────────────────────────────────────────────

function fields(entries: Array<[string, string] | [string, string, 'checked' | 'unchecked']>): Record<string, { raw: string; checked: boolean; error: null }> {
  const out: Record<string, { raw: string; checked: boolean; error: null }> = {};
  for (const entry of entries) {
    const [name, raw, flag] = entry;
    out[name] = { raw, checked: flag === 'checked', error: null };
  }
  return out;
}

describe('ParameterForm — buildPayload', () => {
  const params: readonly ParamMeta[] = ALL_KINDS_COMMAND.params;

  it('builds a valid payload when every field is populated', () => {
    const state = fields([
      ['text_in', 'hello'],
      ['count', '42'],
      ['ratio', '3.14'],
      ['flag', '', 'checked'],
      ['payload', '["a","b"]'],
      ['maybe_name', 'bob'],
    ]);
    const result = buildPayload(params, state);
    expect(result.ok).toBe(true);
    if (!result.ok) throw new Error('expected ok');
    expect(result.values).toEqual({
      text_in: 'hello',
      count: 42,
      ratio: 3.14,
      flag: true,
      payload: ['a', 'b'],
      maybe_name: 'bob',
    });
  });

  it('omits optional params when left blank', () => {
    const state = fields([
      ['text_in', 'x'],
      ['count', '1'],
      ['ratio', '1'],
      ['flag', '', 'unchecked'],
      ['payload', '[]'],
      ['maybe_name', ''],
    ]);
    const result = buildPayload(params, state);
    expect(result.ok).toBe(true);
    if (!result.ok) throw new Error('expected ok');
    expect(result.values).not.toHaveProperty('maybe_name');
    expect(result.values.flag).toBe(false);
  });

  it('reports required-field errors', () => {
    const state = fields([
      ['text_in', ''],
      ['count', ''],
      ['ratio', ''],
      ['flag', '', 'unchecked'],
      ['payload', ''],
      ['maybe_name', ''],
    ]);
    const result = buildPayload(params, state);
    expect(result.ok).toBe(false);
    if (result.ok) throw new Error('expected errors');
    expect(result.errors).toMatchObject({
      text_in: 'required',
      count: 'required',
      ratio: 'required',
      payload: 'required',
    });
    expect(result.errors.flag).toBeUndefined();
    expect(result.errors.maybe_name).toBeUndefined();
  });

  it('rejects non-numeric values for number inputs', () => {
    const state = fields([
      ['text_in', 'x'],
      ['count', 'not-a-number'],
      ['ratio', '1'],
      ['flag', '', 'unchecked'],
      ['payload', '[]'],
      ['maybe_name', ''],
    ]);
    const result = buildPayload(params, state);
    expect(result.ok).toBe(false);
    if (result.ok) throw new Error('expected errors');
    expect(result.errors.count).toMatch(/number/);
  });

  it('rejects malformed JSON for json inputs', () => {
    const state = fields([
      ['text_in', 'x'],
      ['count', '1'],
      ['ratio', '1'],
      ['flag', '', 'unchecked'],
      ['payload', '{ not json'],
      ['maybe_name', ''],
    ]);
    const result = buildPayload(params, state);
    expect(result.ok).toBe(false);
    if (result.ok) throw new Error('expected errors');
    expect(result.errors.payload).toMatch(/JSON/i);
  });

  it('accepts a command with no params at all', () => {
    const emptyParams: readonly ParamMeta[] = [];
    const result = buildPayload(emptyParams, {});
    expect(result.ok).toBe(true);
    if (!result.ok) throw new Error('expected ok');
    expect(result.values).toEqual({});
  });
});
