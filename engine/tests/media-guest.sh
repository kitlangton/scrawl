#!/usr/bin/env bash
set -euo pipefail
umask 077
test -n "${SCRAWL_GUI_BINARY:-}"
test -n "${SCRAWL_GUI_OUTPUT:-}"
fixture_dir=$(dirname "$SCRAWL_GUI_BINARY")
# Keep the repository's auditable static import layout in the isolated payload.
mkdir -p "$fixture_dir/engine/tests" "$fixture_dir/engine/sdk/src"
cp "$fixture_dir/media-guest.ts" "$fixture_dir/engine/tests/media-guest.ts"
cp "$fixture_dir/media.ts" "$fixture_dir/engine/sdk/src/media.ts"
exec node --experimental-strip-types "$fixture_dir/engine/tests/media-guest.ts"
