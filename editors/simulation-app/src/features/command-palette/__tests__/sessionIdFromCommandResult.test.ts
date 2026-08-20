/**
 * The palette dispatches any command generically, so nothing else knows when a
 * result identifies a session. These pin the wire shapes that exist today —
 * see the function's doc comment for why keying off catalog `returns`
 * metadata beats a hardcoded list of command names.
 */
import { describe, expect, it } from 'vitest';
import { sessionIdFromCommandResult } from '../commandCatalog';

const ID = 'de350677-cc0e-4275-adaf-5d7dede1a488';

describe('sessionIdFromCommandResult', () => {
  it('reads the tuple shape returned by orchestrate.workspace.start', () => {
    const meta = { returns: '(session_key: string, ExecutionSnapshot)' };
    expect(sessionIdFromCommandResult(meta, [ID, { tick: 0 }])).toBe(ID);
  });

  it('reads the bare-string shape returned by action.start', () => {
    expect(sessionIdFromCommandResult({ returns: 'string (session_key)' }, ID)).toBe(ID);
  });

  it('reads the SessionSummary shape returned by sessions.create', () => {
    const meta = { returns: 'SessionSummary' };
    expect(sessionIdFromCommandResult(meta, { id: ID, kind: 'orchestrator' })).toBe(ID);
  });

  it('reads an object carrying session_key directly', () => {
    const meta = { returns: '{ session_key: string }' };
    expect(sessionIdFromCommandResult(meta, { session_key: ID })).toBe(ID);
  });

  it('adopts the session a non-creating command operated on', () => {
    // sessions.step/reset/resume also return a summary; selecting the session
    // you just acted on is right, so the rule is deliberately "talked about".
    expect(sessionIdFromCommandResult({ returns: 'SessionSummary' }, { id: ID })).toBe(ID);
  });

  it('ignores commands whose results are not about a session', () => {
    expect(sessionIdFromCommandResult({ returns: 'Vec<Element>' }, [{ id: 'el-1' }])).toBeNull();
    expect(sessionIdFromCommandResult({ returns: 'SessionDivergence' }, { a: 1, b: 2 })).toBeNull();
    expect(sessionIdFromCommandResult({ returns: 'string' }, 'not-a-session')).toBeNull();
  });

  it('does not mistake an empty or missing id for a session', () => {
    expect(sessionIdFromCommandResult({ returns: 'SessionSummary' }, { id: '' })).toBeNull();
    expect(sessionIdFromCommandResult({ returns: 'SessionSummary' }, {})).toBeNull();
    expect(sessionIdFromCommandResult({ returns: '(session_key: string, X)' }, [])).toBeNull();
    expect(sessionIdFromCommandResult({ returns: '(session_key: string, X)' }, [null])).toBeNull();
  });

  it('tolerates missing metadata', () => {
    expect(sessionIdFromCommandResult(null, ID)).toBeNull();
    expect(sessionIdFromCommandResult(undefined, { id: ID })).toBeNull();
  });
});
