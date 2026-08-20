/**
 * CommandPalette — developer-mode Cmd-K escape hatch.
 *
 * Lists every backend service command from `GET /commands`, filters by
 * fuzzy name/description, and auto-generates a parameter form for the
 * selected command. Dispatches via `POST /api/command`.
 *
 * Gated behind `import.meta.env.VITE_DEV_CMDK === '1'` by the caller in
 * `App.tsx`. When the flag is off, this file is never rendered.
 *
 * Accessibility:
 *   - `role="dialog"` + `aria-modal="true"` + `aria-label`
 *   - Focus moves to the filter input on open
 *   - Arrow keys navigate, Enter selects, Escape closes
 *   - Focus is trapped inside the modal while open
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import type { CommandMeta } from './commandCatalog';
import {
  sessionIdFromCommandResult,
  fetchCommandCatalog,
  filterCommands,
  runCommand,
  type CommandResult,
} from './commandCatalog';
import { ParameterForm, type ParamValues } from './ParameterForm';
import { useQueryClient } from '@tanstack/react-query';
import { useSessionStore } from '@/features/sessions/store';
import { sessionKeys } from '@/features/sessions/queries';
import { useCmdKShortcut } from './useCmdKShortcut';
import { getRailCommands } from './railCommands';

type Phase = 'picker' | 'params' | 'result';

export interface CommandPaletteProps {
  /** Whether the Cmd-K shortcut is armed. Defaults to true. */
  enabled?: boolean;
}

export function CommandPalette({ enabled = true }: CommandPaletteProps) {
  const navigate = useNavigate();
  const [isOpen, setIsOpen] = useState(false);
  const [phase, setPhase] = useState<Phase>('picker');
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [catalog, setCatalog] = useState<CommandMeta[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [picked, setPicked] = useState<CommandMeta | null>(null);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<CommandResult | null>(null);
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'failed'>('idle');

  const rootRef = useRef<HTMLDivElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const lastActiveElement = useRef<HTMLElement | null>(null);

  // Reset + load catalog on open.
  useEffect(() => {
    if (!isOpen) return;
    lastActiveElement.current = (document.activeElement as HTMLElement) ?? null;
    setPhase('picker');
    setQuery('');
    setSelectedIndex(0);
    setPicked(null);
    setResult(null);
    setRunning(false);
    setCopyState('idle');
    setLoadError(null);

    let cancelled = false;
    fetchCommandCatalog()
      // Client-side actions (rail open/close/pin) have no backend
      // command behind them — prepend them to the fetched catalog so
      // they're searchable alongside every `sysml.*` command.
      .then((list) => { if (!cancelled) setCatalog([...getRailCommands(), ...list]); })
      .catch((err: unknown) => {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        setLoadError(msg);
      });

    // Focus input after render.
    const t = window.setTimeout(() => inputRef.current?.focus(), 0);

    return () => {
      cancelled = true;
      window.clearTimeout(t);
    };
  }, [isOpen]);

  // Restore focus on close.
  useEffect(() => {
    if (isOpen) return;
    lastActiveElement.current?.focus?.();
  }, [isOpen]);

  const open = useCallback(() => setIsOpen(true), []);
  const close = useCallback(() => setIsOpen(false), []);

  const qc = useQueryClient();
  const setActiveSession = useSessionStore((st) => st.setActiveSession);
  const { pathname } = useLocation();

  // The palette is rendered above the router, so without this it survives a
  // tool-tab change — a result modal from one surface was found still
  // overlaying the next one (finding 29). Leaving the route means you are done
  // with the palette.
  //
  // Guarded on a ref because the effect must fire on route CHANGE, not on
  // mount: an unguarded version also runs during the first commit, which under
  // StrictMode's double-invoke slams the palette shut the moment it opens.
  const lastPathname = useRef(pathname);
  useEffect(() => {
    if (lastPathname.current === pathname) return;
    lastPathname.current = pathname;
    setIsOpen(false);
  }, [pathname]);

  // A picked command either has a `clientAction` (rail open/close/pin —
  // see `railCommands.ts`) and runs immediately with no params form, or
  // is a normal backend command that hands off to the params phase.
  const selectCommand = useCallback((cmd: CommandMeta) => {
    if (cmd.clientAction || cmd.navigateTo) {
      cmd.clientAction?.();
      if (cmd.navigateTo) navigate(cmd.navigateTo);
      close();
      return;
    }
    setPicked(cmd);
    setPhase('params');
  }, [close, navigate]);

  useCmdKShortcut({
    enabled,
    isOpen,
    onOpen: open,
    onClose: close,
  });

  // ── Filtering ──────────────────────────────────────────────────────────

  const filtered = useMemo(() => {
    if (!catalog) return [];
    return filterCommands(catalog, query);
  }, [catalog, query]);

  useEffect(() => {
    // Keep selection in range when the list shrinks.
    if (selectedIndex >= filtered.length) {
      setSelectedIndex(filtered.length === 0 ? 0 : filtered.length - 1);
    }
  }, [filtered.length, selectedIndex]);

  // ── Keyboard nav on the picker list ────────────────────────────────────

  const onPickerKeyDown = useCallback((e: React.KeyboardEvent<HTMLInputElement>) => {
    if (phase !== 'picker') return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, Math.max(0, filtered.length - 1)));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(0, i - 1));
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const cmd = filtered[selectedIndex];
      if (cmd) selectCommand(cmd);
    }
  }, [filtered, phase, selectCommand, selectedIndex]);

  // ── Execute the picked command ─────────────────────────────────────────

  const handleRun = useCallback(async (values: ParamValues) => {
    if (!picked) return;
    setRunning(true);
    setResult(null);
    try {
      const r = await runCommand(picked.name, values);

      // The palette is a generic dispatcher, so a command that acts on a
      // session would otherwise never reach the app's session lifecycle —
      // finding 28, the header reading "no session" beside "1/80 sessions".
      // Adopt the session the command identified, refresh the list, and get
      // out of the way: once the app is showing the session, a wall of
      // ExecutionSnapshot JSON is noise, not confirmation (finding 29).
      const sessionId = r.ok ? sessionIdFromCommandResult(picked, r.value) : null;
      if (sessionId) {
        setActiveSession(sessionId);
        void qc.invalidateQueries({ queryKey: sessionKeys.lists() });
        close();
        return;
      }

      setResult(r);
      setPhase('result');
    } finally {
      setRunning(false);
    }
  }, [picked, setActiveSession, qc, close]);

  const handleCopyResult = useCallback(async () => {
    if (!result) return;
    const payload = result.ok ? JSON.stringify(result.value, null, 2) : (result.error ?? '');
    try {
      await navigator.clipboard.writeText(payload);
      setCopyState('copied');
      window.setTimeout(() => setCopyState('idle'), 1500);
    } catch {
      setCopyState('failed');
      window.setTimeout(() => setCopyState('idle'), 1500);
    }
  }, [result]);

  // ── Focus trap (simple Tab cycling within the modal) ───────────────────

  const onRootKeyDown = useCallback((e: React.KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== 'Tab') return;
    const root = rootRef.current;
    if (!root) return;
    const focusable = root.querySelectorAll<HTMLElement>(
      'input, textarea, button, select, [tabindex]:not([tabindex="-1"])',
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (!first || !last) return;
    const active = document.activeElement as HTMLElement | null;
    if (e.shiftKey && active === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  }, []);

  if (!isOpen) return null;

  return (
    <div
      data-testid="command-palette-overlay"
      onClick={(e) => {
        // Click outside the panel closes.
        if (e.target === e.currentTarget) close();
      }}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0, 0, 0, 0.55)',
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: '10vh',
        zIndex: 9999,
      }}
    >
      <div
        ref={rootRef}
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        onKeyDown={onRootKeyDown}
        className="flex flex-col"
        style={{
          width: 'min(640px, 92vw)',
          maxHeight: '70vh',
          background: 'var(--surface-panel)',
          border: '1px solid var(--border-default)',
          borderRadius: '8px',
          boxShadow: '0 10px 30px rgba(0,0,0,0.5), 0 2px 8px rgba(0,0,0,0.3)',
          overflow: 'hidden',
        }}
      >
        {/* Header */}
        <div
          className="flex items-center gap-2 px-3 py-2"
          style={{ borderBottom: '1px solid var(--border-default)' }}
        >
          <span
            className="material-symbols-outlined"
            style={{ color: 'var(--text-muted)', fontSize: 18 }}
            aria-hidden="true"
          >
            terminal
          </span>
          {phase === 'picker' && (
            <input
              ref={inputRef}
              data-testid="cmdk-filter-input"
              type="text"
              value={query}
              onChange={(e) => { setQuery(e.target.value); setSelectedIndex(0); }}
              onKeyDown={onPickerKeyDown}
              placeholder="Search commands…"
              aria-label="Filter commands"
              className="flex-1 mono-text"
              style={{
                background: 'transparent',
                border: 'none',
                color: 'var(--text-primary)',
                fontSize: 'var(--text-sm)',
                outline: 'none',
              }}
            />
          )}
          {phase !== 'picker' && picked && (
            <div className="flex items-center gap-2 flex-1">
              <button
                onClick={() => { setPhase('picker'); setResult(null); }}
                data-testid="cmdk-back"
                style={{
                  background: 'transparent',
                  border: 'none',
                  color: 'var(--text-muted)',
                  cursor: 'pointer',
                  fontSize: 'var(--text-xs)',
                }}
                aria-label="Back to command list"
              >
                ← back
              </button>
              <code
                style={{
                  color: 'var(--text-primary)',
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--text-sm)',
                }}
              >
                {picked.name}
              </code>
              <span style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>
                {picked.category}
              </span>
            </div>
          )}
          <button
            data-testid="cmdk-close"
            onClick={close}
            aria-label="Close command palette"
            style={{
              background: 'transparent',
              border: 'none',
              color: 'var(--text-muted)',
              cursor: 'pointer',
              fontSize: 'var(--text-xs)',
              padding: '2px 6px',
            }}
          >
            ESC
          </button>
        </div>

        {/* Body */}
        <div style={{ overflow: 'auto', flex: 1 }}>
          {phase === 'picker' && (
            <PickerList
              loading={!catalog && !loadError}
              error={loadError}
              items={filtered}
              selectedIndex={selectedIndex}
              onSelect={selectCommand}
              onHover={(idx) => setSelectedIndex(idx)}
            />
          )}

          {phase === 'params' && picked && (
            <div className="px-4 py-3 flex flex-col gap-3">
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>
                {picked.description}
              </div>
              <div style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>
                <span style={{ fontFamily: 'var(--font-mono)' }}>returns:</span>{' '}
                <span style={{ fontFamily: 'var(--font-mono)', color: 'var(--text-primary)' }}>
                  {picked.returns}
                </span>
              </div>
              <ParameterForm
                command={picked}
                onSubmit={handleRun}
                onCancel={() => setPhase('picker')}
                submitting={running}
              />
            </div>
          )}

          {phase === 'result' && result && (
            <ResultView
              result={result}
              copyState={copyState}
              onCopy={handleCopyResult}
              onRunAgain={() => { setPhase('params'); setResult(null); }}
              onBackToList={() => { setPhase('picker'); setPicked(null); setResult(null); }}
            />
          )}
        </div>

        {/* Footer hints */}
        <div
          className="flex items-center justify-between px-3 py-1.5"
          style={{
            borderTop: '1px solid var(--border-default)',
            color: 'var(--text-muted)',
            fontSize: 'var(--text-xs)',
          }}
        >
          <span>dev mode</span>
          <span>
            {phase === 'picker' && '↑↓ navigate · Enter select · Esc close'}
            {phase === 'params' && 'Enter run · Esc close'}
            {phase === 'result' && 'Esc close'}
          </span>
        </div>
      </div>
    </div>
  );
}

// ── Picker list ─────────────────────────────────────────────────────────

interface PickerListProps {
  loading: boolean;
  error: string | null;
  items: CommandMeta[];
  selectedIndex: number;
  onSelect: (cmd: CommandMeta) => void;
  onHover: (index: number) => void;
}

function PickerList({ loading, error, items, selectedIndex, onSelect, onHover }: PickerListProps) {
  if (loading) {
    return (
      <div data-testid="cmdk-loading" className="px-4 py-3" style={{ color: 'var(--text-muted)', fontSize: 'var(--text-sm)' }}>
        Loading command catalog…
      </div>
    );
  }
  if (error) {
    return (
      <div data-testid="cmdk-error" role="alert" className="px-4 py-3" style={{ color: 'var(--severity-error)', fontSize: 'var(--text-sm)' }}>
        Failed to load catalog: {error}
      </div>
    );
  }
  if (items.length === 0) {
    return (
      <div data-testid="cmdk-empty" className="px-4 py-3" style={{ color: 'var(--text-muted)', fontSize: 'var(--text-sm)' }}>
        No matching commands.
      </div>
    );
  }
  return (
    <ul
      data-testid="cmdk-list"
      role="listbox"
      aria-label="Available commands"
      style={{ listStyle: 'none', margin: 0, padding: 0 }}
    >
      {items.map((cmd, i) => {
        const active = i === selectedIndex;
        return (
          <li
            key={cmd.name}
            role="option"
            aria-selected={active}
            data-testid={`cmdk-item-${cmd.name}`}
            onClick={() => onSelect(cmd)}
            onMouseEnter={() => onHover(i)}
            style={{
              cursor: 'pointer',
              padding: '8px 12px',
              background: active ? 'var(--surface-raised)' : 'transparent',
              borderLeft: active ? '2px solid var(--accent-fg)' : '2px solid transparent',
              display: 'flex',
              flexDirection: 'column',
              gap: 2,
            }}
          >
            <div className="flex items-center gap-2">
              <code
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--text-sm)',
                  color: 'var(--text-primary)',
                }}
              >
                {cmd.name}
              </code>
              <span style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>
                {cmd.category}
              </span>
            </div>
            <span style={{ color: 'var(--text-muted)', fontSize: 'var(--text-xs)' }}>
              {cmd.description}
            </span>
          </li>
        );
      })}
    </ul>
  );
}

// ── Result view ─────────────────────────────────────────────────────────

interface ResultViewProps {
  result: CommandResult;
  copyState: 'idle' | 'copied' | 'failed';
  onCopy: () => void;
  onRunAgain: () => void;
  onBackToList: () => void;
}

function ResultView({ result, copyState, onCopy, onRunAgain, onBackToList }: ResultViewProps) {
  const body = result.ok
    ? JSON.stringify(result.value, null, 2)
    : (result.error ?? 'Unknown error');
  return (
    <div className="px-4 py-3 flex flex-col gap-2" data-testid="cmdk-result">
      <div className="flex items-center justify-between">
        <span
          style={{
            color: result.ok ? 'var(--verdict-pass)' : 'var(--verdict-fail)',
            fontSize: 'var(--text-xs)',
            fontWeight: 500,
          }}
        >
          {result.ok ? 'OK' : 'ERROR'} · {result.latencyMs.toFixed(0)} ms
        </span>
        <div className="flex items-center gap-1">
          <button
            onClick={onCopy}
            data-testid="cmdk-copy"
            style={{
              background: 'transparent',
              border: '1px solid var(--border-default)',
              color: 'var(--text-primary)',
              borderRadius: 4,
              cursor: 'pointer',
              fontSize: 'var(--text-xs)',
              padding: '2px 8px',
            }}
          >
            {copyState === 'copied' ? 'copied!' : copyState === 'failed' ? 'copy failed' : 'copy'}
          </button>
          <button
            onClick={onRunAgain}
            data-testid="cmdk-run-again"
            style={{
              background: 'transparent',
              border: '1px solid var(--border-default)',
              color: 'var(--text-primary)',
              borderRadius: 4,
              cursor: 'pointer',
              fontSize: 'var(--text-xs)',
              padding: '2px 8px',
            }}
          >
            run again
          </button>
          <button
            onClick={onBackToList}
            data-testid="cmdk-back-to-list"
            style={{
              background: 'transparent',
              border: '1px solid var(--border-default)',
              color: 'var(--text-primary)',
              borderRadius: 4,
              cursor: 'pointer',
              fontSize: 'var(--text-xs)',
              padding: '2px 8px',
            }}
          >
            back to list
          </button>
        </div>
      </div>
      <pre
        data-testid="cmdk-result-body"
        style={{
          background: 'var(--surface-raised)',
          color: 'var(--text-primary)',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--text-xs)',
          padding: 8,
          borderRadius: 4,
          border: '1px solid var(--border-default)',
          maxHeight: '40vh',
          overflow: 'auto',
          margin: 0,
          whiteSpace: 'pre-wrap',
          wordBreak: 'break-word',
        }}
      >
        {body}
      </pre>
    </div>
  );
}
