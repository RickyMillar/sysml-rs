# Security Policy

## Supported versions

sysml-rs is in **preview**. There is no long-term support branch and no
backporting: only the most recent release receives security fixes, and fixes
land as a new release rather than as a patch to an older one.

| Version | Supported |
|---|---|
| Latest release | Yes |
| Any earlier release | No — upgrade to the latest release |
| `main` / unreleased commits | Best effort; report anyway |

If a fix requires a breaking change during preview, we will make the breaking
change and say so in the release notes.

## Reporting a vulnerability

**Please do not open a public issue for a security problem.**

Report privately through GitHub's private vulnerability reporting:

1. Go to the **Security** tab of this repository.
2. Choose **Report a vulnerability** to open a private Security Advisory.

That channel is private to you and the maintainers until an advisory is
published.

Useful things to include, as far as you have them: affected component (CLI,
LSP server, REST/WebSocket API, MCP server, VS Code extension, desktop app, or
a library crate), version or commit, platform, a minimal reproduction, and what
an attacker gains. A model file that triggers the problem is more useful than a
description of one — SysML sources are plain text, so please paste or attach it.

## What to expect

This is a small project, so these are honest expectations rather than an SLA:

- **Acknowledgement:** within about 7 days.
- **Initial assessment** (is it a vulnerability, and how severe): within about
  30 days of acknowledgement.
- **Fix or documented mitigation:** timing depends on severity and complexity;
  you will get a status update at least every 30 days while the report is open.
- **Disclosure:** coordinated. We will agree a disclosure date with you and
  credit you in the advisory unless you would rather stay anonymous.

There is **no bug bounty** and no monetary reward.

## Scope and threat model

The realistic threat model for this project is **processing untrusted input**:
a `.sysml` source file, a project manifest, a `.kpar` archive, or a request to
one of the servers. Reports we want to see include memory-unsafety or
unsoundness, crashes and hangs that a malicious input can trigger (the parser
is written to be error-tolerant, so a panic or an unbounded loop on malformed
input is a bug), path traversal or arbitrary file access during project and
library loading, command or code execution reachable from model content, and
any way to escape the intended working directory.

### Known, deliberate design limits — not vulnerabilities

The `sysml-api` server (REST + WebSocket, also the host for the MCP server) is
a **local development server**. It ships with:

- **no authentication unless you ask for it** — set `SYSML_API_TOKEN` and write
  and command routes require `Authorization: Bearer <token>`; leave it unset and
  they are open. Read routes are never authenticated.
- **no rate limiting**, and a 50 MB request body limit that is a resource
  guard rather than a security control.

Two defaults are deliberately narrow, and both widen only when you say so:

- **Bind address `127.0.0.1:8080`** — reachable from this machine only. Pass a
  different address (`sysml-api 0.0.0.0:8080`, or `sysml serve --host`) to go
  wider; the server warns on startup when it binds a non-loopback address, and
  warns again if it does so without a token set.
- **Browser origins restricted to loopback** — `localhost`, `127.0.0.1`, and
  `[::1]` on any port. `--permissive-cors` (or `SYSML_API_CORS=permissive`)
  restores allow-any, which is appropriate behind a trusted proxy and nowhere
  else.

Anyone who can reach the port can read and modify the loaded model and run
simulations on your machine, so keep it on loopback or put it behind your own
authenticating proxy. Reports that consist of "the API has no auth" will be
closed with a pointer to this section; reports that it fails to honour a bind
address you asked for, that a non-loopback origin gets past the default CORS
policy, or that it reaches outside the project directory, are in scope.

Similarly out of scope: vulnerabilities in third-party dependencies with no
demonstrated impact on sysml-rs (report those upstream, and tell us if we
should bump), findings that require an attacker who already has local code
execution as your user, and anything about the OMG specification itself.

## Handling of your data

sysml-rs has no telemetry, and makes no network requests during parsing,
analysis, or execution. The only HTTP client in the workspace lives in
`sysml-resolve`, which fetches the project dependencies you declare in
`sysml.toml` when you run `sysml fetch` or `sysml lock`. The other place the
network is used is the reference-material fetch script during setup, which you
run deliberately. If you find sysml-rs making a request you did not ask for,
that is a bug and we want the report.
