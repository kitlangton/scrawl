#!/bin/sh
# Assemble synced before/after clips from two capture.ts frame folders taken over the same range.
#
#   scripts/motion/clips.sh <before-dir> <after-dir> <out-prefix> [scale-width]
#
# Writes <out>.mp4 (side by side, 60 fps, h264 High yuv420p +faststart), <out>-strip.png (12 frames
# of each side, before on top) and checks the strip for blank tiles.
set -eu
before=$1
after=$2
out=$3
width=${4:-960}
tmp=$(mktemp -d)
# Homebrew's ffmpeg has no drawtext, so the corner labels are drawn as PNGs and overlaid.
uv run --quiet --with pillow python3 - "$tmp" <<'PY'
import sys
from PIL import Image, ImageDraw, ImageFont
font = ImageFont.truetype("/System/Library/Fonts/Supplemental/Arial Bold.ttf", 22)
for name in ("Before", "After"):
    w = int(font.getlength(name)) + 20
    im = Image.new("RGBA", (w, 34), (0, 0, 0, 150))
    ImageDraw.Draw(im).text((10, 5), name, font=font, fill="white")
    im.save(f"{sys.argv[1]}/{name}.png")
PY

ffmpeg -v error -y -framerate 60 -i "$before/frames/f%05d.png" -framerate 60 -i "$after/frames/f%05d.png" -i "$tmp/Before.png" -i "$tmp/After.png" \
  -filter_complex "[0]scale=$width:-2[a0];[a0][2]overlay=12:12[a];[1]scale=$width:-2[b0];[b0][3]overlay=12:12[b];[a][b]hstack=2,pad=ceil(iw/2)*2:ceil(ih/2)*2" \
  -r 60 -c:v libx264 -profile:v high -pix_fmt yuv420p -crf 18 -movflags +faststart -an "$out.mp4"

n=$(ls "$after/frames" | wc -l | tr -d ' ')
step=$(( n / 12 ))
[ "$step" -lt 1 ] && step=1
tile() {
  ffmpeg -v error -y -framerate 60 -i "$1/frames/f%05d.png" -vf "select='not(mod(n\,$step))',scale=$(( width / 2 )):-2,tile=6x2" -frames:v 1 "$2"
}
tile "$before" "$out-strip-before.png"
tile "$after" "$out-strip-after.png"
ffmpeg -v error -y -i "$out-strip-before.png" -i "$out-strip-after.png" -filter_complex vstack=2 "$out-strip.png"
rm "$out-strip-before.png" "$out-strip-after.png"

uv run --quiet --with pillow python3 - "$out-strip.png" <<'EOF'
import sys
from PIL import Image, ImageStat
im = Image.open(sys.argv[1]).convert("L"); w, h = im.size
tiles = [im.crop((c*w//6, r*h//4, (c+1)*w//6, (r+1)*h//4)) for r in range(4) for c in range(6)]
blank = [i for i, t in enumerate(tiles) if ImageStat.Stat(t).stddev[0] < 3]
print(f"{sys.argv[1]}: blank tiles:", blank or "none")
EOF
echo "wrote $out.mp4"
