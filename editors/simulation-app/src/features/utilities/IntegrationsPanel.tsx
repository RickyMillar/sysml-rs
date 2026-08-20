/**
 * IntegrationsPanel — Phase 7 (redefined).
 *
 * Surfaces the three ways an outside tool can talk to this simulation
 * app's backend: the MCP server (stdio subprocess used by Claude
 * Desktop / Claude Code), the REST API (`/api/command`, `/api/query`,
 * `/commands`, `/health`), and the LSP WebSocket (`/lsp`). For each
 * surface we render the connection details + one-click copy snippets
 * + a "Test connection" affordance for the live REST surface.
 *
 * The MCP transport is stdio-only — there is no HTTP MCP endpoint.
 * Users wire it up by pointing their agent harness at the
 * `sysml-api --mcp` binary (or `sysml-mcp`) as a subprocess. The
 * snippet here is paste-into-config, not a fetchable URL.
 */

import { useCallback, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { httpGet } from '@/shared/api/http';
import {
  fetchCommandCatalog,
  type CommandMeta,
} from '@/features/command-palette/commandCatalog';

// ── URL resolvers ────────────────────────────────────────────────────

/**
 * The REST/LSP base URL the FE is currently talking to. Vite proxies
 * `/api/*`, `/commands`, `/health`, `/lsp` to the actual `sysml-api`
 * host, so from the FE's perspective the origin is always
 * `window.location.origin`. The panel still shows the resolved
 * external URL ("http://localhost:8080" or similar) so users have
 * something to paste into curl / Postman / `claude mcp add`.
 *
 * For SSR / vitest, `window` is undefined — return a placeholder.
 */
function restBaseUrl(): string {
  if (typeof window === 'undefined') return 'http://localhost:8080';
  return window.location.origin;
}

function lspUrl(): string {
  if (typeof window === 'undefined') return 'ws://localhost:8080/lsp';
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${window.location.host}/lsp`;
}

// ── Snippet generators ───────────────────────────────────────────────

/**
 * `claude_desktop_config.json` fragment. The user pastes this under
 * `mcpServers`. Path defaults to a placeholder — users replace with
 * their local `sysml-api` binary location (e.g. the output of
 * `which sysml-api`).
 */
function claudeDesktopSnippet(binaryPath: string): string {
  const obj = {
    mcpServers: {
      sysml: {
        command: binaryPath,
        args: ['--mcp'],
      },
    },
  };
  return JSON.stringify(obj, null, 2);
}

/** `claude mcp add` CLI invocation for Claude Code. */
function claudeCodeSnippet(binaryPath: string): string {
  return `claude mcp add sysml ${binaryPath} --mcp`;
}

/** `curl` example calling `sysml.workspace.info` against the REST API. */
function curlSnippet(baseUrl: string): string {
  return [
    `curl -X POST ${baseUrl}/api/command \\`,
    `  -H 'Content-Type: application/json' \\`,
    `  -d '{"command":"sysml.workspace.info","params":{}}'`,
  ].join('\n');
}

// ── Health probe ─────────────────────────────────────────────────────

interface HealthResult {
  status: string;
  version?: string;
}

async function probeHealth(): Promise<HealthResult> {
  return httpGet<HealthResult>('/health');
}

// ── Panel ────────────────────────────────────────────────────────────

export function IntegrationsPanel() {
  const [binaryPath, setBinaryPath] = useState<string>('sysml-api');
  const [copied, setCopied] = useState<string | null>(null);
  const baseUrl = useMemo(restBaseUrl, []);
  const lsp = useMemo(lspUrl, []);

  const handleCopy = useCallback(async (key: string, text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(key);
      window.setTimeout(() => setCopied((c) => (c === key ? null : c)), 1200);
    } catch {
      setCopied(`${key}:failed`);
    }
  }, []);

  // Live tool count from `/commands` — same registry the MCP server
  // exposes (both share the `#[service_command]` inventory).
  const commands = useQuery<CommandMeta[]>({
    queryKey: ['integrations', 'commands'],
    queryFn: fetchCommandCatalog,
    staleTime: 60_000,
  });

  const health = useQuery<HealthResult>({
    queryKey: ['integrations', 'health'],
    queryFn: probeHealth,
    enabled: false, // user-triggered via Test connection
    retry: false,
  });

  return (
    <div
      data-testid="integrations-panel"
      className="flex flex-col h-full min-h-0 overflow-auto"
      style={{ padding: 12, gap: 12, color: 'var(--text-primary)' }}
    >
      {/* ── MCP ───────────────────────────────────────────────────── */}
      <Section
        testid="integrations-mcp"
        title="MCP — Model Context Protocol"
        eyebrow="For AI agents (Claude Desktop, Claude Code)"
      >
        <p style={paragraphStyle}>
          The MCP server runs as a stdio subprocess of your agent
          harness. There is no HTTP endpoint — your harness launches
          <code style={codeStyle}> sysml-api --mcp </code>
          (or
          <code style={codeStyle}> sysml-mcp </code>)
          and talks JSON-RPC over stdin/stdout.
        </p>
        <label className="flex flex-col gap-1" style={{ fontSize: 11 }}>
          <span style={{ color: 'var(--text-muted)', fontWeight: 700 }}>
            sysml-api binary path
          </span>
          <input
            data-testid="integrations-binary-path"
            value={binaryPath}
            onChange={(e) => setBinaryPath(e.target.value)}
            placeholder="/usr/local/bin/sysml-api"
            style={inputStyle}
            spellCheck={false}
          />
          <span style={{ fontSize: 10, color: 'var(--text-muted)' }}>
            Run <code style={codeStyle}>which sysml-api</code> in your
            shell to find this.
          </span>
        </label>

        <SnippetBox
          testid="integrations-mcp-desktop"
          label="Claude Desktop — paste into ~/.config/claude/claude_desktop_config.json"
          text={claudeDesktopSnippet(binaryPath)}
          copied={copied === 'mcp-desktop'}
          onCopy={() =>
            handleCopy('mcp-desktop', claudeDesktopSnippet(binaryPath))
          }
        />
        <SnippetBox
          testid="integrations-mcp-code"
          label="Claude Code — run this in your terminal"
          text={claudeCodeSnippet(binaryPath)}
          copied={copied === 'mcp-code'}
          onCopy={() => handleCopy('mcp-code', claudeCodeSnippet(binaryPath))}
        />

        <KeyValue
          label="Tools exposed"
          value={
            commands.isLoading
              ? '…'
              : commands.isError
                ? '—'
                : `${commands.data?.length ?? 0}`
          }
          testid="integrations-mcp-tool-count"
        />
      </Section>

      {/* ── REST ──────────────────────────────────────────────────── */}
      <Section
        testid="integrations-rest"
        title="REST API"
        eyebrow="For scripts, curl, Postman, the live editor"
      >
        <KeyValue
          label="Base URL"
          value={baseUrl}
          testid="integrations-rest-base"
          mono
        />
        <KeyValue
          label="Auth"
          value="Set SYSML_API_TOKEN env var on the server for bearer-token auth"
          testid="integrations-rest-auth"
        />
        <KeyValue
          label="Commands available"
          value={
            commands.isLoading
              ? '…'
              : commands.isError
                ? '—'
                : `${commands.data?.length ?? 0}`
          }
          testid="integrations-rest-command-count"
        />

        <div className="flex items-center gap-2" style={{ marginTop: 4 }}>
          <button
            type="button"
            data-testid="integrations-rest-test"
            onClick={() => health.refetch()}
            disabled={health.isFetching}
            style={buttonStyle}
          >
            <span
              className="material-symbols-outlined"
              style={{ fontSize: 13 }}
            >
              {health.isFetching ? 'progress_activity' : 'wifi_tethering'}
            </span>
            Test connection
          </button>
          {health.isSuccess && (
            <span
              data-testid="integrations-rest-health-ok"
              style={{ fontSize: 11, color: 'var(--verdict-pass)' }}
            >
              ✓ {health.data?.status ?? 'ok'}
              {health.data?.version ? ` · v${health.data.version}` : ''}
            </span>
          )}
          {health.isError && (
            <span
              data-testid="integrations-rest-health-err"
              style={{ fontSize: 11, color: 'var(--verdict-fail)' }}
            >
              ✕{' '}
              {health.error instanceof Error
                ? health.error.message
                : String(health.error)}
            </span>
          )}
        </div>

        <SnippetBox
          testid="integrations-rest-curl"
          label="curl — workspace.info round-trip"
          text={curlSnippet(baseUrl)}
          copied={copied === 'rest-curl'}
          onCopy={() => handleCopy('rest-curl', curlSnippet(baseUrl))}
        />
      </Section>

      {/* ── LSP ───────────────────────────────────────────────────── */}
      <Section
        testid="integrations-lsp"
        title="LSP WebSocket"
        eyebrow="For editor integrators"
      >
        <KeyValue
          label="WebSocket URL"
          value={lsp}
          testid="integrations-lsp-url"
          mono
        />
        <p style={paragraphStyle}>
          The simulation app's Monaco editor already speaks LSP over
          this socket. External editors (Zed, VS Code, etc.) can
          attach the same way — see the per-editor extensions under
          <code style={codeStyle}> editors/ </code> for reference
          clients.
        </p>
      </Section>
    </div>
  );
}

// ── Sub-components ───────────────────────────────────────────────────

function Section({
  testid,
  title,
  eyebrow,
  children,
}: {
  testid: string;
  title: string;
  eyebrow: string;
  children: React.ReactNode;
}) {
  return (
    <section
      data-testid={testid}
      className="rounded-lg"
      style={{
        border: '1px solid var(--border-default)',
        background: 'var(--surface-panel)',
        padding: 12,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
      }}
    >
      <header className="flex items-baseline justify-between gap-3">
        <div>
          <div
            style={{
              fontSize: 10,
              color: 'var(--text-muted)',
              fontWeight: 800,
              textTransform: 'uppercase',
              letterSpacing: '0.06em',
            }}
          >
            {eyebrow}
          </div>
          <div style={{ fontSize: 13, fontWeight: 800 }}>{title}</div>
        </div>
      </header>
      {children}
    </section>
  );
}

function KeyValue({
  label,
  value,
  testid,
  mono,
}: {
  label: string;
  value: string;
  testid?: string;
  mono?: boolean;
}) {
  return (
    <div
      data-testid={testid}
      className="grid"
      style={{
        gridTemplateColumns: 'minmax(120px, max-content) 1fr',
        gap: 8,
        fontSize: 11,
        alignItems: 'baseline',
      }}
    >
      <span style={{ color: 'var(--text-muted)' }}>{label}</span>
      <span
        className={mono ? 'mono-text' : undefined}
        style={{ overflowWrap: 'anywhere' }}
      >
        {value}
      </span>
    </div>
  );
}

function SnippetBox({
  testid,
  label,
  text,
  copied,
  onCopy,
}: {
  testid: string;
  label: string;
  text: string;
  copied: boolean;
  onCopy: () => void;
}) {
  return (
    <div className="flex flex-col gap-1" data-testid={testid}>
      <div className="flex items-center justify-between">
        <span style={{ fontSize: 10, color: 'var(--text-muted)', fontWeight: 700 }}>
          {label}
        </span>
        <button
          type="button"
          data-testid={`${testid}-copy`}
          onClick={onCopy}
          style={buttonStyleSmall}
        >
          <span
            className="material-symbols-outlined"
            style={{ fontSize: 11 }}
          >
            {copied ? 'check' : 'content_copy'}
          </span>
          {copied ? 'Copied' : 'Copy'}
        </button>
      </div>
      <pre
        className="mono-text"
        style={{
          margin: 0,
          padding: 8,
          background: 'var(--surface-sunken)',
          border: '1px solid var(--border-default)',
          borderRadius: 6,
          fontSize: 10,
          whiteSpace: 'pre-wrap',
          color: 'var(--text-secondary)',
          overflowWrap: 'anywhere',
        }}
      >
        {text}
      </pre>
    </div>
  );
}

// ── Styles ───────────────────────────────────────────────────────────

const paragraphStyle: React.CSSProperties = {
  fontSize: 11,
  lineHeight: 1.5,
  color: 'var(--text-secondary)',
  margin: 0,
};

const codeStyle: React.CSSProperties = {
  fontFamily: 'var(--mono-font, ui-monospace, monospace)',
  background: 'var(--surface-sunken)',
  padding: '0 4px',
  borderRadius: 3,
  fontSize: '0.95em',
};

const inputStyle: React.CSSProperties = {
  width: '100%',
  border: '1px solid var(--border-default)',
  borderRadius: 6,
  background: 'var(--surface-sunken)',
  color: 'var(--text-primary)',
  padding: '6px 8px',
  fontSize: 11,
  fontFamily: 'var(--mono-font, ui-monospace, monospace)',
};

// "Test connection" primary action — treated as the genuine accent/primacy
// case (a filled call-to-action button), so the legacy primary-container /
// on-primary-container pair maps onto --accent / --on-accent rather than a
// status token.
const buttonStyle: React.CSSProperties = {
  border: '1px solid var(--border-default)',
  background: 'var(--accent)',
  color: 'var(--on-accent)',
  borderRadius: 4,
  padding: '4px 8px',
  fontSize: 11,
  fontWeight: 700,
  cursor: 'pointer',
  display: 'inline-flex',
  alignItems: 'center',
  gap: 4,
};

const buttonStyleSmall: React.CSSProperties = {
  border: '1px solid var(--border-default)',
  background: 'var(--surface-panel)',
  color: 'var(--text-secondary)',
  borderRadius: 4,
  padding: '2px 6px',
  fontSize: 10,
  fontWeight: 700,
  cursor: 'pointer',
  display: 'inline-flex',
  alignItems: 'center',
  gap: 3,
};
