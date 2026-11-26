#!/usr/bin/env bash
# Plays `scrawl lab pill` scenes with one binary and records each: a motion-lint trace of every
# capsule frame and a 60 fps screen recording of the capsule's region, then runs motion-lint.
#
#   perf/pill/record.sh <scrawl-binary> <out-dir> [--time-scale 0.25] [--rune] [scene ...]
#
# Writes <out-dir>/runs/<scene>/trace/<scene>.jsonl (one motion-lint test per scene),
# <out-dir>/<scene>.mov and motion-lint's candidates.json.
# Scenes default to all of them. Each takes a few seconds; the capsule shows under the menu bar
# on the main display over a plain backdrop. Nothing is captured, recorded or sent by Scrawl.
set -euo pipefail

bin=$1
out=$2
shift 2
scale_args=()
while [[ ${1:-} == --* ]]; do
  case $1 in
    --time-scale) scale_args+=(--time-scale "$2"); shift 2 ;;
    # The Rune capsule (`scrawl lab pill --rune`), left in captures so the recorder sees it.
    --rune) scale_args+=(--rune); export SCRAWL_LAB_EXCLUDE=none; shift ;;
    *) echo "unknown option $1" >&2; exit 2 ;;
  esac
done
scenes=("$@")
if [[ ${#scenes[@]} -eq 0 ]]; then
  scenes=(tap-then-area tap-then-window tap-then-screen picker finish cancel dismiss)
fi

here=$(cd "$(dirname "$0")" && pwd)
"$here/../screen-guard.sh"
motion_lint=${MOTION_LINT:-$HOME/code/open-source/motion-lint/cli/motion-lint.ts}

for scene in "${scenes[@]}"; do
  rm -f "$out/$scene.mov"
  mkdir -p "$out/runs/$scene/trace"
  log="$out/.$scene.log"
  "$bin" lab pill "$scene" --trace "$out/runs/$scene/trace/$scene.jsonl" ${scale_args[@]+"${scale_args[@]}"} >"$log" 2>&1 &
  lab=$!
  region=""
  for _ in {1..30}; do
    region=$(sed -n 's/^region //p' "$log" 2>/dev/null | head -1)
    [[ -n $region ]] && break
    sleep 0.05
  done
  sleep 0.25 # the backdrop is up; the first step comes at 0.85 s (scaled)
  region=${region:-${REGION:-}} # older builds don't print it
  [[ -n $region ]] || { echo "no region from the lab ($log)"; kill "$lab"; exit 1; }
  screencapture -v -x -R"$region" "$out/$scene.mov" &
  rec=$!
  wait "$lab"
  sleep 0.25
  kill -INT "$rec" 2>/dev/null || true
  wait "$rec" 2>/dev/null || true
  rm -f "$log"
  echo "recorded $scene"
done

bun "$motion_lint" analyze "$out" --no-pixels --config "$here/motion-lint.config.json"
