#!/bin/zsh
# Where `apparent-helper permission-panel` docks its card, for fixed window and screen rects
# (AppKit coordinates: origin bottom-left). Run after `cargo build`.
set -euo pipefail
H=${HELPER:-${0:A:h}/../../../../target/debug/apparent-helper}
failures=0
check() {
  local name=$1 settings=$2 visible=$3 want=$4
  local extra=${5:-}
  local out dock
  out=$($H permission-panel --plan --settings "$settings" --visible "$visible" ${=extra})
  dock=$(print -r -- "$out" | sed -E 's/.*"dock":"([a-z]+)".*/\1/')
  if [[ $dock == "$want" ]]; then
    print "ok    $name: $out"
  else
    print "FAIL  $name: want $want, got $out"
    failures=$((failures + 1))
  fi
}
# 2560×1440 screen, menu bar 30 pt, no Dock.
V=0,0,2560,1410
check "room below"                1640,320,740,820  $V below
check "no room below"             1640,100,740,820  $V inside
check "short window, no room"     1000,60,740,300   $V right
check "short, at the right edge"  1820,60,740,300   $V left
check "lower 30% off the screen"  1640,-200,740,820 $V left
check "small screen"              0,0,740,600       0,0,740,600 inside
check "window fills a tiny screen" 0,0,740,300      0,0,740,300 clamped
check "screen recording covers lower content with room below" 1640,320,740,820 $V inside --screen-recording
check "screen recording stays inside on small screen" 0,0,740,600 0,0,740,600 inside --screen-recording
# Exercise the actual helper geometry: the opaque card must span the lower
# content region while leaving the upper list exposed at standard/small sizes.
for settings in 1640,320,740,820 0,0,740,600; do
  out=$($H permission-panel --plan --settings "$settings" --visible "$V" --screen-recording)
  if ! python3 - "$out" "$settings" <<'PYTEST'
import json,sys
x,y,w,h=map(float,sys.argv[2].split(','))
cx,cy,cw,ch=json.loads(sys.argv[1])['frame']
assert cy <= y+12, (cy,y)
assert ch >= h*.5-20, (ch,h)
assert cy+ch <= y+h*.5, (cy,ch,y,h)
assert cx <= x+252 and cx+cw >= x+w-20, (cx,cw,x,w)
PYTEST
  then
    failures=$((failures + 1))
  fi
done
exit $failures
