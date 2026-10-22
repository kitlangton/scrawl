#!/usr/bin/env bash
# Packs a built `scrawl` into an AppImage with GStreamer and its plugins (screen capture,
# H.264 encoding, decoding) inside, so a movie records on any desktop:
#
#   ./scripts/linux/appimage.sh [path/to/scrawl]   # default target/release/scrawl
#
# Writes target/appimage/Scrawl-<arch>.AppImage. Needs the GStreamer plugin packages installed
# (gstreamer1.0-plugins-{base,good,bad,ugly}, gstreamer1.0-pipewire) and patchelf; linuxdeploy and its
# GStreamer plugin are downloaded into target/appimage/tools the first time.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
binary="$(realpath "${1:-$root/target/release/scrawl}")"
arch="$(uname -m)"
out="$root/target/appimage"
tools="$out/tools"
appdir="$out/AppDir"
mkdir -p "$tools"
rm -rf "$appdir"

fetch() {
  [ -s "$tools/$2" ] || curl -fsSL -o "$tools/$2" "$1"
  chmod +x "$tools/$2"
}
fetch "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$arch.AppImage" linuxdeploy
fetch "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gstreamer/master/linuxdeploy-plugin-gstreamer.sh" linuxdeploy-plugin-gstreamer.sh

multiarch="$(gcc -print-multiarch 2>/dev/null || echo "$arch-linux-gnu")"
export GSTREAMER_PLUGINS_DIR="/usr/lib/$multiarch/gstreamer-1.0"
export GSTREAMER_HELPERS_DIR="/usr/lib/$multiarch/gstreamer1.0/gstreamer-1.0"
export GSTREAMER_INCLUDE_BAD_PLUGINS=1
# CI runners have no FUSE.
export APPIMAGE_EXTRACT_AND_RUN=1
export PATH="$tools:$PATH"

# These modules are dlopen'd, not DT_NEEDED dependencies: linuxdeploy cannot
# discover them from scrawl. Stage every tier next to libtranscribe in usr/lib.
libraries=()
for library in "$(dirname "$binary")"/libggml-cpu*.so; do
  [[ -f "$library" ]] || { echo "Missing transcribe runtime: $library" >&2; exit 1; }
  libraries+=(--library "$library")
done

cd "$out"
LDAI_OUTPUT="Scrawl-$arch.AppImage" linuxdeploy \
  --appdir "$appdir" \
  --executable "$binary" \
  "${libraries[@]}" \
  --executable "$(command -v wl-copy)" \
  --desktop-file "$root/packaging/linux/dev.kitlangton.scrawl.desktop" \
  --icon-file "$root/packaging/linux/dev.kitlangton.scrawl.png" \
  --plugin gstreamer \
  --output appimage
echo "$out/Scrawl-$arch.AppImage"
