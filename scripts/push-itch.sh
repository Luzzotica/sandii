#!/usr/bin/env bash
# Build sandii_web (Trunk release) and upload dist/ to itch.io with butler.
#
# Prerequisites:
#   rustup target add wasm32-unknown-unknown
#   cargo install trunk
#   butler — https://itch.io/docs/butler/  (then: butler login)
#
# Usage:
#   ./scripts/push-itch.sh
#
# Optional (override defaults):
#   ITCH_USER=luzzotica       (default: luzzotica)
#   ITCH_GAME=sandii          (default: sandii)
#   ITCH_CHANNEL=html5        (butler channel name; tag as HTML5 in itch project settings once)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEB="${ROOT}/crates/sandii_web"
DIST="${WEB}/dist"

ITCH_USER="${ITCH_USER:-luzzotica}"
ITCH_GAME="${ITCH_GAME:-sandii}"
ITCH_CHANNEL="${ITCH_CHANNEL:-html5}"

if ! command -v trunk >/dev/null 2>&1; then
  echo "error: trunk not found. Install with: cargo install trunk" >&2
  exit 1
fi
if ! command -v butler >/dev/null 2>&1; then
  echo "error: butler not found. See https://itch.io/docs/butler/" >&2
  exit 1
fi

echo "==> Building wasm (trunk release) in ${WEB}"
# Trunk fails when NO_COLOR is set to a non-boolean value (e.g. 1 in some setups).
(
  cd "${WEB}"
  env -u NO_COLOR trunk build --release
)

if [[ ! -f "${DIST}/index.html" ]]; then
  echo "error: expected ${DIST}/index.html after build" >&2
  exit 1
fi

TARGET="${ITCH_USER}/${ITCH_GAME}:${ITCH_CHANNEL}"
echo "==> Pushing ${DIST} -> ${TARGET}"
butler push "${DIST}" "${TARGET}"

echo "==> Done. If this channel is new, set it to 'HTML / Playable in browser' on the game's Edit page."
