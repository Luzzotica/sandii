#!/usr/bin/env bash
# Run the Sandii web (Trunk) dev server from the repo root:
#   ./scripts/run-web.sh
# Extra args are passed to `trunk serve` (e.g. --port 8080).

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}/crates/sandii_web"

if ! command -v trunk >/dev/null 2>&1; then
  echo "error: trunk not found. Install with: cargo install trunk" >&2
  exit 1
fi

# Trunk fails when NO_COLOR is set to a non-boolean value in some environments.
exec env -u NO_COLOR trunk serve "$@"
