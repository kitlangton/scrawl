#!/usr/bin/env bash
# Packs a built `apparent` into an AppImage with GStreamer and its plugins (screen capture,
# H.264 encoding, decoding) inside, so a movie records on any desktop:
#
#   ./capture/desktop/scripts/linux/appimage/appimage.sh [path/to/apparent]   # default target/release/apparent
#
# Writes target/appimage/Apparent-<arch>.AppImage. Needs the GStreamer plugin packages installed
# (gstreamer1.0-plugins-{base,good,bad,ugly}, gstreamer1.0-pipewire) and patchelf; linuxdeploy and its
# GStreamer plugin are downloaded into target/appimage/tools the first time.
set -euo pipefail

root="$(cd "$(dirname "$0")/../../../../.." && pwd)"
binary="$(realpath "${1:-$root/target/release/apparent}")"
arch="$(uname -m)"
out="$root/target/appimage"
tools="$out/tools"
appdir="$out/AppDir"
mkdir -p "$tools"
rm -rf "$appdir"
python3 "$root/capture/desktop/scripts/packaging/stage-settings.py" "$appdir/usr/share/apparent/settings"

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
# discover them from apparent. Stage every tier next to libtranscribe in usr/lib.
libraries=()
for library in "$(dirname "$binary")"/libggml-cpu*.so; do
  [[ -f "$library" ]] || { echo "Missing transcribe runtime: $library" >&2; exit 1; }
  libraries+=(--library "$library")
done

cd "$out"
install -Dm755 "$root/capture/desktop/scripts/support/apparent-restore-clipboard.py" "$appdir/usr/bin/apparent-restore-clipboard.py"
node "$root/capture/desktop/ui/frontend/scripts/package-notes.mjs" "$appdir/usr/lib/apparent/note-ui"
LDAI_OUTPUT="Apparent-$arch.AppImage" linuxdeploy \
  --appdir "$appdir" \
  --executable "$binary" \
  --deploy-deps-only "$appdir/usr/share/apparent/settings/runtime" \
  "${libraries[@]}" \
  --executable "$(command -v wl-copy)" \
  --executable "$(command -v wl-paste)" \
  --desktop-file "$root/capture/desktop/packaging/linux/dev.kitlangton.apparent.desktop" \
  --icon-file "$root/capture/desktop/packaging/linux/dev.kitlangton.apparent.png" \
  --plugin gstreamer \
  --output appimage
bash "$root/capture/desktop/scripts/linux/appimage/check-appimage-encoder.sh" "$out/Apparent-$arch.AppImage"
echo "$out/Apparent-$arch.AppImage"
