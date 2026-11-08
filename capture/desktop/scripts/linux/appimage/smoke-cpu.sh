#!/usr/bin/env bash
# Run a relocated test executable against ONLY the libraries extracted from the
# AppImage. Remove Cargo's runtime copies so a missing packaged tier can't hide.
set -euo pipefail
root="$(cd "$(dirname "$0")/../../../../.." && pwd)"
image="$(realpath "$1")"
test_binary="$(realpath "$2")"
shift 2
smoke="$root/target/cpu-smoke"
mkdir -p "$smoke"
cd "$smoke"
"$image" --appimage-extract >/dev/null
cp "$test_binary" ./speech-test
lib="$smoke/squashfs-root/usr/lib"
# Set this before even asking the copied executable to list tests: its $ORIGIN
# now points into cpu-smoke, so only the extracted AppImage libraries can load.
export LD_LIBRARY_PATH="$lib"
# Keep the exact filter honest: Rust's test harness succeeds when it selects 0 tests.
test_name=capture::engine::runtime::audio::transcription::speech::tests::downloads_and_runs_parakeet
listed="$(./speech-test "$test_name" --exact --ignored --list)"
if [[ "$(printf '%s\n' "$listed" | grep -Fxc "$test_name: test")" != 1 ]]; then
  printf 'Expected exactly one ignored test matching %s; got:\n%s\n' "$test_name" "$listed" >&2
  exit 1
fi
export APPARENT_TEST_MODEL_DIR="${APPARENT_TEST_MODEL_DIR:-$smoke/model}"
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
  if ! LD_DEBUG=libs ./speech-test "$test_name" --exact --ignored --nocapture 2>loader.log; then
    tail -100 loader.log
    exit 1
  fi
  grep -E 'calling init: .*lib(ggml|transcribe)|device:|test result:' loader.log
  grep -q 'calling init: .*cpu-smoke/squashfs-root/usr/lib/libggml-cpu' loader.log
else
  "$@" -- ./speech-test "$test_name" --exact --ignored --nocapture
  # Also exercise the shipped Rust binary's keyframe/render/bundle path, without
  # a display, microphone, clipboard or an installed speech model.
  "$@" -- ./squashfs-root/usr/bin/apparent demo --out "$smoke/demo.apparent"
  test -s "$smoke/demo.apparent/apparent.json"
fi
