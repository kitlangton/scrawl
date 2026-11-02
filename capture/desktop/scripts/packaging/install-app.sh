#!/usr/bin/env bash
set -euo pipefail
# Keep process inspection independent of the caller's locale.
export LC_ALL=C

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
# APPARENT_APP_DIR builds the signed bundle somewhere else (staging for remote installs, see
# install-app-remote.sh); APPARENT_NO_LAUNCH=1 skips the ~/bin link and launch,
# but never permits replacement of a running app.
APP_DIR="${APPARENT_APP_DIR:-${HOME}/Applications/Apparent.app}"
CONTENTS_DIR="${APP_DIR}/Contents"
MACOS_DIR="${CONTENTS_DIR}/MacOS"
RESOURCES_DIR="${CONTENTS_DIR}/Resources"
FRAMEWORKS_DIR="${CONTENTS_DIR}/Frameworks"
SIGN_ID="Developer ID Application: Christopher Langton (QC99C9JE59)"
SPARKLE_PUBLIC_KEY="mIek27lttJe8cIBqVZFhh6reRKjpTx1h9ZY9OKWPtuM="

cd "${REPO_DIR}"
chmod +x "${REPO_DIR}/capture/desktop/scripts/release/setup-sparkle.sh" "${REPO_DIR}/capture/desktop/scripts/release/release-app.sh" 2>/dev/null || true
SPARKLE_DIR="$("${REPO_DIR}/capture/desktop/scripts/release/setup-sparkle.sh" 2>/dev/null || true)"
taskpolicy -c utility cargo build -j 3

# Inspect executable paths, not argv: other installations and CLI commands that
# merely mention this app are not ours. Helpers inside this bundle count too.
if ! PROCESSES="$(ps -axww -o pid= -o comm=)"; then
  echo "can't inspect running processes; destination app is untouched; retry when process inspection works" >&2
  exit 1
fi
while read -r PID EXECUTABLE; do
  if [[ "${EXECUTABLE}" == "${APP_DIR}/Contents/"* ]]; then
    echo "${APP_DIR} is in use (pid ${PID}: ${EXECUTABLE}); finish any capture, quit the app and its helpers, then retry" >&2
    exit 1
  fi
done <<< "${PROCESSES}"

mkdir -p "${MACOS_DIR}" "${RESOURCES_DIR}" "${FRAMEWORKS_DIR}" "${HOME}/bin"

rm -f "${MACOS_DIR}/apparent" "${MACOS_DIR}/apparent-helper"
cp -f "${REPO_DIR}/target/debug/apparent" "${MACOS_DIR}/apparent"
cp -f "${REPO_DIR}/target/debug/apparent-helper" "${MACOS_DIR}/apparent-helper"
chmod +x "${MACOS_DIR}/apparent" "${MACOS_DIR}/apparent-helper"

cp -f "${REPO_DIR}/capture/desktop/assets/Apparent.icns" "${RESOURCES_DIR}/Apparent.icns"
cp -f "${REPO_DIR}/shared/assets/fonts/HostGrotesk-Variable.ttf" "${RESOURCES_DIR}/HostGrotesk-Variable.ttf"
cp -f "${REPO_DIR}/shared/assets/brand/apparent-wordmark.png" "${RESOURCES_DIR}/apparent-wordmark.png"
cp -f "${REPO_DIR}/shared/assets/brand/apparent-a.png" "${RESOURCES_DIR}/apparent-a.png"

if [[ -n "${SPARKLE_DIR}" && -d "${SPARKLE_DIR}/Sparkle.framework" ]]; then
  rm -rf "${FRAMEWORKS_DIR}/Sparkle.framework"
  /usr/bin/ditto "${SPARKLE_DIR}/Sparkle.framework" "${FRAMEWORKS_DIR}/Sparkle.framework"
fi

cat > "${CONTENTS_DIR}/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleURLTypes</key>
  <array><dict>
    <key>CFBundleURLName</key><string>Apparent launch</string>
    <key>CFBundleURLSchemes</key><array><string>apparent</string></array>
    <key>CFBundleTypeRole</key><string>Viewer</string>
  </dict></array>
  <key>CFBundleExecutable</key>
  <string>apparent</string>
  <key>CFBundleIconFile</key>
  <string>Apparent</string>
  <key>CFBundleIdentifier</key>
  <string>dev.kitlangton.apparent</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Apparent</string>
  <key>CFBundleDisplayName</key>
  <string>Apparent</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>15.0</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSScreenCaptureUsageDescription</key>
  <string>Apparent captures your screen to build annotated agent bundles.</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>Apparent records your voice to align transcripts with keyframes.</string>
  <key>NSCameraUsageDescription</key>
  <string>Apparent mirrors and records a cabled iPhone's screen.</string>
  <key>NSSpeechRecognitionUsageDescription</key>
  <string>Apparent transcribes your narration on this Mac.</string>
  <key>NSAppleEventsUsageDescription</key>
  <string>Apparent reads the address of the browser tab you captured to record its context.</string>
  <key>SUEnableAutomaticChecks</key>
  <false/>
  <key>SUAutomaticallyUpdate</key>
  <false/>
  <key>SUAllowsAutomaticUpdates</key>
  <false/>
  <key>SUFeedURL</key>
  <string>https://app.tryapparent.com/appcast.xml</string>
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
codesign --force --sign "${IDENTITY}" --identifier "dev.kitlangton.apparent.helper" "${MACOS_DIR}/apparent-helper"
codesign --force --sign "${IDENTITY}" --identifier "dev.kitlangton.apparent" "${APP_DIR}"

if [[ "${APPARENT_NO_LAUNCH:-0}" == "1" ]]; then
  exit 0
fi

if [[ -e "${HOME}/bin/apparent" || -L "${HOME}/bin/apparent" ]]; then
  if [[ ! -L "${HOME}/bin/apparent" ]] || [[ "$(readlink "${HOME}/bin/apparent")" != "${MACOS_DIR}/apparent" ]]; then
    echo "${HOME}/bin/apparent belongs to another program; leaving it alone" >&2
    exit 1
  fi
fi
ln -sfn "${MACOS_DIR}/apparent" "${HOME}/bin/apparent"

open -a "${APP_DIR}" || (sleep 0.3 && open -n -a "${APP_DIR}")
sleep 0.3
ps -axww -o pid= -o comm= | while read -r PID EXECUTABLE; do
  [[ "${EXECUTABLE}" != "${MACOS_DIR}/apparent" ]] || echo "${PID} ${EXECUTABLE}"
done
