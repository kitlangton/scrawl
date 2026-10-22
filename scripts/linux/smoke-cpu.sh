#!/usr/bin/env bash
# Run a relocated test executable against ONLY the libraries extracted from the
# AppImage. Remove Cargo's runtime copies so a missing packaged tier can't hide.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
image="$(realpath "$1")"
test_binary="$(realpath "$2")"
shift 2
smoke="$root/target/cpu-smoke"
mkdir -p "$smoke"
cd "$smoke"
"$image" --appimage-extract >/dev/null
cp "$test_binary" ./speech-test
lib="$smoke/squashfs-root/usr/lib"
export LD_LIBRARY_PATH="$lib"
export SCRAWL_TEST_MODEL_DIR="${SCRAWL_TEST_MODEL_DIR:-$smoke/model}"
# There must be no build-tree RPATH left in packaged libraries.
if find "$lib" -name '*transcribe*.so*' -o -name '*ggml*.so*' | while read -r file; do
  patchelf --print-rpath "$file"
done | grep -E '/(home|target|runner)/'; then
  echo 'Build-tree RPATH leaked into AppImage' >&2
  exit 1
fi
find "$root/target/release" -maxdepth 2 \( -name 'libtranscribe.so*' -o -name 'libggml*.so*' \) -exec rm -f {} +
# libtranscribe resolves its own location via dladdr, then loads CPU modules in
# that directory. LD_DEBUG on the native run records the actual loaded paths.
if [[ $# == 0 ]]; then
  if ! LD_DEBUG=libs ./speech-test speech::tests::downloads_and_runs_parakeet --exact --ignored --nocapture 2>loader.log; then
    tail -100 loader.log
    exit 1
  fi
  grep -E 'calling init: .*lib(ggml|transcribe)|device:|test result:' loader.log
  grep -q 'calling init: .*cpu-smoke/squashfs-root/usr/lib/libggml-cpu' loader.log
else
  "$@" -- ./speech-test speech::tests::downloads_and_runs_parakeet --exact --ignored --nocapture
  # Also exercise the shipped Rust binary's keyframe/render/bundle path, without
  # a display, microphone, clipboard or an installed speech model.
  "$@" -- ./squashfs-root/usr/bin/scrawl demo --out "$smoke/demo.scrawl"
  test -s "$smoke/demo.scrawl/scrawl.json"
fi
