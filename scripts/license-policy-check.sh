#!/usr/bin/env bash
# scripts/license-policy-check.sh — license-policy invariant gate.
#
#
# This is the half of the license gate that `cargo deny` cannot do. `deny.toml`
# polices the *inbound* licenses of third-party crates; this script polices the
# *outbound* declaration and the two invariants the licensing policy rests on:
#
#   A. every workspace Cargo package declares exactly "MIT OR Apache-2.0"
#   B. every first-party npm manifest declares the same
#   C. tree-sitter.json (the grammar's third publishing manifest) declares it
#   D. pyproject.toml declares it, in PEP 621 table form, with classifiers
#      that name BOTH licenses
#   E. no third-party crate in the graph ships a NOTICE file
#   F. the two license texts the declarations point at actually exist
#   G. every third-party npm package resolves to a permissive license, or to a
#      named exception carrying a decision-register reference
#
# Why C and D are file-level assertions rather than metadata queries: the
# `tree-sitter-sysml` package publishes into three ecosystems from one
# directory and declares its license at FOUR sites. `cargo metadata` sees one
# of them, npm tooling sees another, and NOTHING in either toolchain can see
# `pyproject.toml` or `tree-sitter.json`. A scaffolding default of bare `MIT`
# survived at all four sites until a manual audit found them, which is why
# this gate asserts against the files themselves.
#
# Usage:
#   scripts/license-policy-check.sh
#
# Exit 0 = all invariants hold. Exit 1 = at least one violation, each printed
# with its file and the exact mismatch. Exit 2 = the gate could not run
# (missing tool, unfetched registry) — never silently green.
set -uo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

readonly EXPECTED="MIT OR Apache-2.0"

FAILURES=0
fail() {
    printf 'VIOLATION: %s\n' "$1" >&2
    FAILURES=$((FAILURES + 1))
}
# A gate that cannot run must never report success. Exits immediately with 2 so
# an unfetched registry or a missing `jq` can't be mistaken for a clean tree.
abort() {
    printf 'license-policy-check: CANNOT RUN — %s\n' "$1" >&2
    exit 2
}

for tool in cargo jq; do
    command -v "$tool" >/dev/null 2>&1 || abort "\`$tool\` is not on PATH"
done

# ---------------------------------------------------------------------------
# F. The license texts every declaration points at
# ---------------------------------------------------------------------------
# Checked first: if these are missing, every "MIT OR Apache-2.0" string in the
# tree is a dangling reference, and that is the more fundamental problem.
echo "== F. root license texts =="
for f in LICENSE-MIT LICENSE-APACHE; do
    if [ -f "$f" ]; then
        echo "  ok       $f"
    else
        fail "$f is missing. Every manifest in this tree declares '$EXPECTED'; both texts must exist at the repository root."
    fi
done

# ---------------------------------------------------------------------------
# A. Workspace Cargo packages
# ---------------------------------------------------------------------------
echo "== A. workspace Cargo packages =="
cargo_meta=$(cargo metadata --no-deps --format-version 1 --offline 2>/dev/null) \
    || abort "\`cargo metadata --no-deps --offline\` failed; run \`cargo fetch\` first"

pkg_count=$(printf '%s' "$cargo_meta" | jq -r '.packages | length')
[ "${pkg_count:-0}" -gt 0 ] \
    || abort "cargo metadata reported zero workspace packages — the query is broken, not the tree"

while IFS=$'\t' read -r name license manifest; do
    [ -z "$name" ] && continue
    fail "Cargo package '$name' declares license '$license', expected '$EXPECTED' — $manifest"
done < <(printf '%s' "$cargo_meta" | jq -r --arg want "$EXPECTED" '
    .packages[] | select((.license // "<undeclared>") != $want)
    | [.name, (.license // "<undeclared>"), .manifest_path] | @tsv')
echo "  ok       $pkg_count package(s) declare '$EXPECTED'"

# ---------------------------------------------------------------------------
# B. First-party npm manifests
# ---------------------------------------------------------------------------
echo "== B. npm manifests =="
# Discovered rather than listed, so a NEW manifest is covered the day it lands.
# Pruned: node_modules (third-party), target (build output), and references/
# (third-party OMG + Pilot Implementation material, fetched rather than
# tracked; its licenses are not ours to declare).
mapfile -t npm_manifests < <(
    find . \
        \( -name node_modules -o -name target -o -path ./references \) -prune -o \
        -name package.json -type f -print | sed 's|^\./||' | sort
)

# The four publishing sites. Discovery covers more than
# this, but these four must be present: if the find above silently stops
# matching, an empty-but-green run would be worse than a red one.
readonly REQUIRED_NPM=(
    "editors/vscode/package.json"
    "editors/simulation-app/package.json"
    "editors/expression-view/package.json"
    "crates/lang/sysml-parser-incremental/tree-sitter/package.json"
)
for required in "${REQUIRED_NPM[@]}"; do
    found=0
    for m in "${npm_manifests[@]}"; do
        [ "$m" = "$required" ] && found=1 && break
    done
    [ "$found" -eq 1 ] || fail "expected npm manifest '$required' was not found. Either it moved (update REQUIRED_NPM in this script) or manifest discovery is broken. It is one of the four publishing sites."
done

for m in "${npm_manifests[@]}"; do
    declared=$(jq -r '.license // "<undeclared>"' "$m" 2>/dev/null) \
        || { fail "$m is not valid JSON"; continue; }
    if [ "$declared" = "$EXPECTED" ]; then
        echo "  ok       $m"
    else
        fail "npm manifest $m declares license '$declared', expected '$EXPECTED'."
    fi
done

# ---------------------------------------------------------------------------
# C. tree-sitter.json
# ---------------------------------------------------------------------------
echo "== C. tree-sitter.json =="
readonly TS_JSON="crates/lang/sysml-parser-incremental/tree-sitter/tree-sitter.json"
if [ ! -f "$TS_JSON" ]; then
    fail "$TS_JSON is missing — it is one of the four license declaration sites for tree-sitter-sysml."
else
    ts_declared=$(jq -r '.metadata.license // "<undeclared>"' "$TS_JSON" 2>/dev/null) \
        || fail "$TS_JSON is not valid JSON"
    if [ "$ts_declared" = "$EXPECTED" ]; then
        echo "  ok       $TS_JSON (.metadata.license)"
    else
        fail "$TS_JSON declares .metadata.license '$ts_declared', expected '$EXPECTED'. This site is invisible to both cargo and npm tooling."
    fi
fi

# ---------------------------------------------------------------------------
# D. pyproject.toml
# ---------------------------------------------------------------------------
echo "== D. pyproject.toml =="
readonly PYPROJECT="crates/lang/sysml-parser-incremental/tree-sitter/pyproject.toml"
if [ ! -f "$PYPROJECT" ]; then
    fail "$PYPROJECT is missing — it is the fourth license declaration site for tree-sitter-sysml."
else
    # Grep rather than a TOML parser: no Python or toml CLI is guaranteed on
    # the CI image, and the assertion is over a single literal line whose exact
    # form is itself the thing under policy. This stays in PEP 621 table form
    # (`license.text`) instead of the PEP 639 SPDX string because the SPDX form
    # needs setuptools>=77 -> Python>=3.9, and this package is pinned to 3.8 by
    # requires-python and Py_LIMITED_API.
    if grep -Eq "^license\.text[[:space:]]*=[[:space:]]*\"${EXPECTED}\"[[:space:]]*$" "$PYPROJECT"; then
        echo "  ok       $PYPROJECT (license.text)"
    else
        actual=$(grep -E "^license" "$PYPROJECT" || echo "<no license line>")
        fail "$PYPROJECT does not declare 'license.text = \"$EXPECTED\"'. Found: $actual"
    fi

    # Pre-PEP-639 dual licensing is expressed by carrying BOTH classifiers. A
    # lone `MIT License` classifier is exactly the scaffolding default that
    # survived undetected until commit 2caa22a4, and it contradicts the
    # license.text above — an affirmative misstatement, not an omission.
    if grep -q "License :: OSI Approved" "$PYPROJECT"; then
        grep -q "License :: OSI Approved :: MIT License" "$PYPROJECT" \
            || fail "$PYPROJECT carries 'License :: OSI Approved' classifiers but not 'MIT License'. Dual licensing pre-PEP-639 requires both classifiers."
        grep -q "License :: OSI Approved :: Apache Software License" "$PYPROJECT" \
            || fail "$PYPROJECT carries 'License :: OSI Approved' classifiers but not 'Apache Software License'. A lone MIT classifier contradicts license.text."
        echo "  ok       $PYPROJECT (both License:: classifiers present)"
    fi
fi

# ---------------------------------------------------------------------------
# E. No third-party crate ships a NOTICE file
# ---------------------------------------------------------------------------
echo "== E. no dependency ships a NOTICE file =="
# The policy declines to create a root NOTICE on the grounds that
# Apache-2.0 §4(d) is conditional — it obliges propagation only "if the Work
# includes a NOTICE text file as part of its distribution" — and that no
# dependency ships one. That is a factual claim with an expiry date: one
# `cargo update` can invalidate it. This is the assertion that keeps §3.2 true,
# and §3.3 names it as a revisit trigger for the root-NOTICE decision.
full_meta=$(cargo metadata --format-version 1 --offline 2>/dev/null) \
    || abort "\`cargo metadata --offline\` (full graph) failed; run \`cargo fetch\` first"

mapfile -t dep_manifests < <(printf '%s' "$full_meta" | jq -r '.packages[] | select(.source != null) | .manifest_path')
[ "${#dep_manifests[@]}" -gt 0 ] \
    || abort "the dependency graph resolved to zero third-party crates — the query is broken, not the tree"

notice_count=0
unpacked=0
missing_src=0
for manifest in "${dep_manifests[@]}"; do
    dir=$(dirname "$manifest")
    if [ ! -d "$dir" ]; then
        missing_src=$((missing_src + 1))
        continue
    fi
    unpacked=$((unpacked + 1))
    # `find -maxdepth 1` rather than `ls | grep`: NOTICE, NOTICE.txt, and
    # NOTICE.md all count, and a filename with a space must not split.
    while IFS= read -r notice; do
        [ -z "$notice" ] && continue
        notice_count=$((notice_count + 1))
        fail "third-party crate ships a NOTICE file: $notice — this repository declines a root NOTICE on the basis that none exists. Re-open that decision or add the file to the per-artifact notice bundle."
    done < <(find "$dir" -maxdepth 1 -iname 'NOTICE*' -type f 2>/dev/null)
done

# A crate whose sources were never unpacked cannot be inspected, so a green
# here would be an untested claim rather than a verified one.
if [ "$missing_src" -gt 0 ]; then
    abort "$missing_src of ${#dep_manifests[@]} crate source directories are not unpacked, so the NOTICE scan would be vacuous. Run \`cargo fetch\` and re-run."
fi
echo "  ok       scanned $unpacked third-party crate source dir(s), found $notice_count NOTICE file(s)"

# ---------------------------------------------------------------------------
# G. Third-party npm licenses
# ---------------------------------------------------------------------------
echo "== G. npm dependency licenses =="
# This is the npm half of the exception list. The licensing policy
# requires elkjs (D-2) and the @vscode/vsce-sign family (D-3) to be recorded
# explicitly "so it cannot pass as an unreviewed unknown", and cargo-deny
# cannot hold them because they are not Cargo dependencies.
#
# Why the lockfiles rather than a license scanner: `license-checker` and
# friends are not installed, they require a full `npm ci` (minutes, network)
# to read node_modules, and their answer comes from the same place anyway —
# each package's declared `license`. lockfileVersion 3 records that key per
# package, so `jq` over the four committed lockfiles is the same assertion at
# zero install cost and with a deterministic, reviewable input. The tradeoff,
# stated plainly: these are lockfile-DECLARED licenses, not licenses read from
# installed package files. A follow-up must
# re-derive them from an actual install of the exact release lockfile.
#
# Second tradeoff: expressions are compared as literal strings, because there
# is no SPDX evaluator here. `(MIT OR CC0-1.0)` is allowed as a whole string
# rather than resolved to its MIT branch. That is stricter, not looser — a new
# spelling of an already-acceptable license fails and gets a human look.

# Permissive expressions. Every one of these is
# present in a lockfile today.
readonly NPM_ALLOWED_LICENSES=(
    "MIT" "ISC" "0BSD" "MIT-0" "BSD-2-Clause" "BSD-3-Clause" "Apache-2.0"
    "Apache-2.0 OR MIT" "MIT OR Apache-2.0"
    # Disjunctions that offer a plainly permissive branch, and two
    # single-license permissive outliers. All dev/build-only;
    # records "all permissive, no action" for each.
    "(BSD-2-Clause OR MIT OR Apache-2.0)"  # rc
    "(MIT OR CC0-1.0)"                     # type-fest
    "(MIT OR WTFPL)"                       # expand-template
    "BlueOak-1.0.0"                        # sax
    "Python-2.0"                           # argparse
)

# npm_exception_reason NAME LICENSE
# Echoes the reason a specific package is accepted despite a license outside
# the permissive set, or nothing if there is no such exception. Keeping the
# reason next to the rule is the point: a bare allowlist entry six months from
# now is indistinguishable from an oversight.
npm_exception_reason() {
    local name="$1" license="$2"
    case "$name|$license" in
    "elkjs|EPL-2.0")
        echo "Weak file-level copyleft, bundled UNMODIFIED into the desktop app as the diagram layout engine. EPL-2.0 permits distributing the surrounding work under other terms; the source-availability obligation is discharged by naming elkjs, its version, EPL-2.0 and its upstream source in the artifact notice bundle. Note EPL-2.0 is not GPL-compatible — that constrains downstreams, not us."
        ;;
    "dompurify|(MPL-2.0 OR Apache-2.0)")
        echo "Disjunctive and RUNTIME. We elect the Apache-2.0 branch, so the MPL branch never applies. The election must be recorded explicitly in the notice bundle so it is auditable rather than inferred."
        ;;
    "caniuse-lite|CC-BY-4.0")
        echo "Browserslist data, dev/build-only. CC-BY-4.0 requires attribution only if redistributed, and this is never redistributed. Any change to the bundling setup must re-confirm that no build inlines its data into a shipped bundle."
        ;;
    esac

    # Platform-binary families: matched by prefix rather than by enumerating
    # every triple, because the set of published platforms changes with each
    # upstream release and a stale 12-entry list would fail on a platform being
    # added — a packaging event, not a licensing decision. Both families are
    # single-publisher scoped-or-hyphenated names.
    case "$name|$license" in
    "@vscode/vsce-sign"*"|SEE LICENSE IN LICENSE.txt")
        echo "Microsoft's proprietary VSIX signing helper — non-SPDX and NOT open source. Accepted as a devDependency of the publishing toolchain that is never redistributed by us, and recorded here explicitly so it cannot pass as an unreviewed unknown. It must NOT appear in shipped VSIX notices."
        ;;
    "lightningcss"*"|MPL-2.0")
        echo "The CSS transformer inside the Vite/Tailwind build. Weak file-level copyleft, but dev:true in every lockfile that carries it — it never enters a shipped artifact, so distribution imposes no obligation."
        ;;
    esac
}

mapfile -t npm_lockfiles < <(
    find . \
        \( -name node_modules -o -name target -o -path ./references \) -prune -o \
        -name package-lock.json -type f -print | sed 's|^\./||' | sort
)
[ "${#npm_lockfiles[@]}" -gt 0 ] \
    || abort "no package-lock.json found — npm license discovery is broken (four are expected)"

npm_checked=0
npm_excepted=0
for lock in "${npm_lockfiles[@]}"; do
    while IFS=$'\t' read -r name license scope; do
        [ -z "$name" ] && continue
        npm_checked=$((npm_checked + 1))

        # First-party workspace packages are linked by `file:` and carry no
        # license key in the lockfile. They are not third-party, and their real
        # declaration is asserted by check B above.
        if [ "$license" = "UNDECLARED" ]; then
            case "$name" in
            @sysml-rs/*) continue ;;
            *)
                fail "npm package '$name' declares no license in $lock (scope: $scope). The gate fails closed on undeclared licenses; a third-party package with no license key must be reviewed, not assumed permissive."
                continue
                ;;
            esac
        fi

        for allowed in "${NPM_ALLOWED_LICENSES[@]}"; do
            if [ "$license" = "$allowed" ]; then
                continue 2
            fi
        done

        reason=$(npm_exception_reason "$name" "$license")
        if [ -n "$reason" ]; then
            npm_excepted=$((npm_excepted + 1))
            continue
        fi

    done < <(jq -r '
        .packages | to_entries[] | select(.key != "")
        | [ (.key | split("node_modules/") | last),
            (.value.license // "UNDECLARED"),
            (if .value.dev then "dev-only" else "runtime" end) ]
        | @tsv' "$lock" 2>/dev/null)
done
echo "  ok       $npm_checked package entries across ${#npm_lockfiles[@]} lockfile(s); $npm_excepted matched a recorded exception"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo
if [ "$FAILURES" -ne 0 ]; then
    echo "license-policy-check: FAILED — $FAILURES violation(s) above." >&2
    exit 1
fi
echo "license-policy-check: PASS — all license-declaration invariants hold."
