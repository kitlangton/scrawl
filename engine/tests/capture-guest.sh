#!/usr/bin/env bash
set -euo pipefail

node_bin=$(command -v node)
test -n "$node_bin"
binary_dir=${SCRAWL_GUI_BINARY%/*}
mkdir -m 700 -p "$SCRAWL_GUI_OUTPUT/sdk"
tar -xf "$binary_dir/capture-sdk-test.tar" -C "$SCRAWL_GUI_OUTPUT/sdk"
tar -xf "$binary_dir/capture-runtime.tar" -C "$SCRAWL_GUI_OUTPUT"
tar -xf "$binary_dir/x11-test-tools.tar" -C "$SCRAWL_GUI_OUTPUT"
export APPARENT_ENGINE_CAPTURE_BIN="$SCRAWL_GUI_OUTPUT/dist/apparent-engine-capture-launch"
if [ -f "$binary_dir/media-runtime.tar" ]; then
  mkdir -m 700 "$SCRAWL_GUI_OUTPUT/media-runtime"
  tar -xf "$binary_dir/media-runtime.tar" -C "$SCRAWL_GUI_OUTPUT/media-runtime"
  export APPARENT_ENGINE_MEDIA2_BIN="$SCRAWL_GUI_OUTPUT/media-runtime/dist/apparent-engine-frames"
fi
export APPARENT_XVFB="$SCRAWL_GUI_OUTPUT/x11-test-tools/Xvfb"
export APPARENT_XSETROOT="$SCRAWL_GUI_OUTPUT/x11-test-tools/xsetroot"
export APPARENT_FFPROBE="$SCRAWL_GUI_OUTPUT/x11-test-tools/ffprobe"
export APPARENT_FFMPEG="$SCRAWL_GUI_OUTPUT/x11-test-tools/ffmpeg"
export APPARENT_CAPTURE_EVIDENCE_DIR="$SCRAWL_GUI_OUTPUT/native-proof"
export APPARENT_CAPTURE_DEBUG_STDERR=1
export LD_LIBRARY_PATH="$binary_dir:$SCRAWL_GUI_OUTPUT/x11-test-tools/lib"
cd "$SCRAWL_GUI_OUTPUT/sdk"
"$node_bin" --experimental-strip-types --test engine/tests/capture-peer.test.ts
