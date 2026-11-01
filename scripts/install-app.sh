#!/usr/bin/env bash
set -euo pipefail
# pgrep/pkill fail under some locales (C.UTF-8), which left the old app running.
export LC_ALL=C

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# SCRAWL_APP_DIR builds the signed bundle somewhere else (staging for remote installs, see
# install-app-remote.sh); SCRAWL_NO_LAUNCH=1 skips the ~/bin link and the restart.
APP_DIR="${SCRAWL_APP_DIR:-${HOME}/Applications/Scrawl.app}"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"
FRAMEWORKS_DIR="${CONTENTS_DIR}/Frameworks"
SIGN_ID="Developer ID Application: Christopher Langton (QC99C9JE59)"
SPARKLE_PUBLIC_KEY="mIek27lttJe8cIBqVZFhh6reRKjpTx1h9ZY9OKWPtuM="

cd "${REPO_DIR}"
chmod +x "${REPO_DIR}/scripts/setup-sparkle.sh" "${REPO_DIR}/scripts/release-app.sh" 2>/dev/null || true
SPARKLE_DIR="$("${REPO_DIR}/scripts/setup-sparkle.sh" 2>/dev/null || true)"
taskpolicy -c utility cargo build -j 3

mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}" "${FRAMEWORKS_DIR}" "${HOME}/bin"

rm -f "${MACOS_DIR}/scrawl" "${MACOS_DIR}/scrawl-helper"
cp -f "${REPO_DIR}/target/debug/scrawl" "${MACOS_DIR}/scrawl"
cp -f "${REPO_DIR}/target/debug/scrawl-helper" "${MACOS_DIR}/scrawl-helper"
chmod +x "${MACOS_DIR}/scrawl" "${MACOS_DIR}/scrawl-helper"

if [[ -f "${REPO_DIR}/assets/Scrawl.icns" ]]; then
  cp -f "${REPO_DIR}/assets/Scrawl.icns" "${RESOURCES_DIR}/Scrawl.icns"
fi

if [[ -n "${SPARKLE_DIR}" && -d "${SPARKLE_DIR}/Sparkle.framework" ]]; then
  rm -rf "${FRAMEWORKS_DIR}/Sparkle.framework"
  /usr/bin/ditto "${SPARKLE_DIR}/Sparkle.framework" "${FRAMEWORKS_DIR}/Sparkle.framework"
fi

cat > "${CONTENTS_DIR}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>scrawl</string>
  <key>CFBundleIconFile</key>
  <string>Scrawl</string>
  <key>CFBundleIdentifier</key>
  <string>dev.kitlangton.scrawl</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Scrawl</string>
  <key>CFBundleDisplayName</key>
  <string>Scrawl</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>ScrawlDevBuild</key>
  <true/>
  <key>LSMinimumSystemVersion</key>
  <string>15.0</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSScreenCaptureUsageDescription</key>
  <string>Scrawl captures your screen to build annotated agent bundles.</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>Scrawl records your voice to align transcripts with keyframes.</string>
  <key>NSCameraUsageDescription</key>
  <string>Scrawl mirrors and records a cabled iPhone's screen.</string>
  <key>NSSpeechRecognitionUsageDescription</key>
  <string>Scrawl transcribes your narration on this Mac.</string>
  <key>NSAppleEventsUsageDescription</key>
  <string>Scrawl reads the address of the browser tab you captured, to send a capture of a dev site to its project's session.</string>
  <key>SUEnableAutomaticChecks</key>
  <false/>
  <key>SUAutomaticallyUpdate</key>
  <false/>
  <key>SUAllowsAutomaticUpdates</key>
  <false/>
  <key>SUFeedURL</key>
  <string>https://scrawl.kitlangton.com/appcast.xml</string>
  <key>SUPublicEDKey</key>
  <string>${SPARKLE_PUBLIC_KEY}</string>
  <key>SUVerifyUpdateBeforeExtraction</key>
  <true/>
</dict>
</plist>
PLIST

if security find-identity -v -p codesigning 2>/dev/null | grep -q "${SIGN_ID}"; then
  IDENTITY="${SIGN_ID}"
else
  IDENTITY="-"
fi
if [[ -d "${FRAMEWORKS_DIR}/Sparkle.framework/Versions/B" ]]; then
  SPARKLE_VER="${FRAMEWORKS_DIR}/Sparkle.framework/Versions/B"
  codesign --force --sign "${IDENTITY}" "${SPARKLE_VER}/XPCServices/Installer.xpc"
  codesign --force --sign "${IDENTITY}" --preserve-metadata=entitlements "${SPARKLE_VER}/XPCServices/Downloader.xpc"
  codesign --force --sign "${IDENTITY}" "${SPARKLE_VER}/Autoupdate"
  codesign --force --sign "${IDENTITY}" "${SPARKLE_VER}/Updater.app"
  codesign --force --sign "${IDENTITY}" "${FRAMEWORKS_DIR}/Sparkle.framework"
fi
codesign --force --sign "${IDENTITY}" --identifier "dev.kitlangton.scrawl.helper" "${MACOS_DIR}/scrawl-helper"
codesign --force --sign "${IDENTITY}" --identifier "dev.kitlangton.scrawl" "${APP_DIR}"

if [[ "${SCRAWL_NO_LAUNCH:-0}" == "1" ]]; then
  exit 0
fi

ln -sf "${MACOS_DIR}/scrawl" "${HOME}/bin/scrawl"

scrawl_running() {
  local status=0
  pgrep -f "Scrawl.app/Contents/MacOS/scrawl" >/dev/null || status=$?
  case "${status}" in
    0) return 0 ;;
    1) return 1 ;;
    *) echo "pgrep failed (exit ${status}); can't tell whether the old Scrawl stopped" >&2; exit 1 ;;
  esac
}

pkill -f "Scrawl.app/Contents/MacOS/scrawl" 2>/dev/null || true
pkill -f "target/debug/scrawl" 2>/dev/null || true
for _ in {1..40}; do
  scrawl_running || break
  sleep 0.05
done
if scrawl_running; then
  pgrep -fl "Scrawl.app/Contents/MacOS/scrawl" >&2 || true
  echo "the old Scrawl is still running; quit it and run this again" >&2
  exit 1
fi
open -a "${APP_DIR}" || (sleep 0.3 && open -n -a "${APP_DIR}")
sleep 0.3
pgrep -fl "Scrawl.app/Contents/MacOS/scrawl"
