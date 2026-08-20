#!/bin/bash
# Diagram-review harness runner — api + vite + shoot.mjs in ONE shell
# invocation (sandbox network namespaces are per-command; see
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="$HERE/../.."
REPO="$APP/../.."
# Optional second step (layout-quality brief §6): `run.sh --assert` runs the
# geometry gates (assert-geometry.mjs) after the contact sheet, sharing this
# invocation's api+vite servers.
ASSERT=0
ARGS=()
for a in "$@"; do
  if [ "$a" = "--assert" ]; then ASSERT=1; else ARGS+=("$a"); fi
done
OUT="${ARGS[0]:-$APP/test-results/diagram-review}"
mkdir -p "$OUT"

"$REPO/target/release/sysml-api" 127.0.0.1:8080 > "$OUT/api.log" 2>&1 &
API_PID=$!
(cd "$APP" && ./node_modules/.bin/vite --port 3010 --strictPort > "$OUT/vite.log" 2>&1) &
VITE_PID=$!

for i in $(seq 1 30); do
  curl -s -m 1 -o /dev/null http://127.0.0.1:3010/ && break
  sleep 1
done

node "$HERE/shoot.mjs" "$OUT"
STATUS=$?
if [ "$ASSERT" = "1" ] && [ "$STATUS" = "0" ]; then
  node "$HERE/assert-geometry.mjs" "$OUT"
  STATUS=$?
fi
kill $API_PID $VITE_PID 2>/dev/null
exit $STATUS
