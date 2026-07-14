#!/usr/bin/env bash
#
# Dev convenience: register the "Open With -> Winnow" launcher using the venv
# build. (End users get this automatically via install.sh, or can run
# `winnow --install-desktop` directly.)
#
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="$HERE/.venv/bin/winnow"

if [[ ! -x "$BIN" ]]; then
    echo "error: $BIN not found. Run:  uv venv && uv pip install -e ." >&2
    exit 1
fi

exec "$BIN" --install-desktop
