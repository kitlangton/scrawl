#!/usr/bin/env bash
set -euo pipefail
umask 077
test -n "${SCRAWL_GUI_BINARY:-}"
test -n "${SCRAWL_GUI_OUTPUT:-}"
fixture_dir=$(dirname "$SCRAWL_GUI_BINARY")
exec node --experimental-strip-types "$fixture_dir/media-guest.ts"
