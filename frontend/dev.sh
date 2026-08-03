#!/bin/bash
#
# Start the app for development, without the startup race that silently kills the UI.
#
# `tauri dev` opens the window as soon as the dev-server *port* accepts connections. Next
# is listening by then, but it has not compiled the route yet — it does that on demand, on
# the first request. So the webview would ask for a ~7MB `layout.js` while Next was still
# writing it, read a truncated file, and die with `SyntaxError: Unexpected EOF`.
#
# Nothing surfaced that. The bundle simply never executed: React never hydrated, so the
# server-rendered icons were painted but had no handlers on them, and every client-rendered
# control (the record button) was missing outright. The Rust log stayed clean throughout —
# it never sees a webview error — which is what made this so hard to pin down.
#
# The fix is to compile everything up front and prove the chunks are complete and parseable
# *before* the window opens.
set -euo pipefail

cd "$(dirname "$0")"

PORT=3118
export PATH="$PWD/node_modules/.bin:$PATH"
export RUST_LOG="${RUST_LOG:-info}"

cleanup() {
  [[ -n "${NEXT_PID:-}" ]] && kill "$NEXT_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "==> Starting Next dev server on :$PORT"
next dev -p "$PORT" &
NEXT_PID=$!

echo "==> Waiting for the route to compile"
for _ in $(seq 1 120); do
  curl -sf -o /dev/null "http://localhost:$PORT/" && break
  sleep 1
done

# Compiling the route is not enough: the chunks it references must also be complete. A
# truncated chunk still returns HTTP 200, so the only trustworthy check is to parse it.
echo "==> Verifying chunks are complete and parseable"
for _ in $(seq 1 60); do
  ok=1
  chunks=$(curl -sf "http://localhost:$PORT/" | grep -oE '/_next/static/chunks/[^"]+\.js' | sort -u)
  [[ -z "$chunks" ]] && { sleep 1; continue; }

  for chunk in $chunks; do
    tmp=$(mktemp)
    if ! curl -sf "http://localhost:$PORT$chunk" -o "$tmp" || ! node --check "$tmp" 2>/dev/null; then
      echo "    $chunk not ready yet"
      ok=0
    fi
    rm -f "$tmp"
  done

  if [[ $ok -eq 1 ]]; then
    echo "==> All chunks parse cleanly"
    break
  fi
  sleep 1
done

echo "==> Opening the app window"
# beforeDevCommand is emptied because Next is already running; letting Tauri start a second
# one would collide on the port.
tauri dev --config '{"build":{"beforeDevCommand":""}}' "$@"
