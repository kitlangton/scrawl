#!/usr/bin/env bash
# Builds, signs, notarizes and staples one candidate build: Scrawl-<version>-b<build>-arm64.dmg
# and the Sparkle .zip, in $SCRAWL_DIST_DIR. Publishing is scripts/release-beta.sh and promote.sh.
# The app carries a plain version and a build number, never a channel.
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${SCRAWL_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' "${REPO_DIR}/Cargo.toml" | head -1)}"
BUILD_NUMBER="${SCRAWL_BUILD_NUMBER:?Set a globally increasing SCRAWL_BUILD_NUMBER}"
BASE_URL="${SCRAWL_RELEASE_BASE_URL:-https://scrawl.kitlangton.com}"
SIGN_ID="Developer ID Application: Christopher Langton (QC99C9JE59)"
SPARKLE_PUBLIC_KEY="mIek27lttJe8cIBqVZFhh6reRKjpTx1h9ZY9OKWPtuM="
[[ "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Version must be X.Y.Z, got ${VERSION}" >&2; exit 1; }
[[ "${BUILD_NUMBER}" =~ ^[1-9][0-9]*$ ]] || { echo "Build number must be a positive integer" >&2; exit 1; }

DIST_DIR="${SCRAWL_DIST_DIR:-${REPO_DIR}/dist}"
APP_BUNDLE="${DIST_DIR}/Scrawl.app"
LABEL="${VERSION}-b${BUILD_NUMBER}"
DMG_NAME="Scrawl-${LABEL}-arm64.dmg"
ZIP_NAME="Scrawl-${LABEL}-arm64.zip"

cd "${REPO_DIR}"
SPARKLE_DIR="$("${REPO_DIR}/scripts/setup-sparkle.sh")"

# HiveMind may build on another Mac: fetch both binaries back, then prove they are this build.
STARTED="${DIST_DIR}/.build-started"
mkdir -p "${DIST_DIR}"
touch "${STARTED}"
hivemind run --kind cargo --fetch target/release/scrawl --fetch target/release/scrawl-helper -- \
  env SCRAWL_VERSION="${VERSION}" SCRAWL_BUILD_NUMBER="${BUILD_NUMBER}" cargo +stable build --release -j 2 --locked
for binary in scrawl scrawl-helper; do
  [[ "${REPO_DIR}/target/release/${binary}" -nt "${STARTED}" ]] || { echo "target/release/${binary} is stale" >&2; exit 1; }
done
ACTUAL="$("${REPO_DIR}/target/release/scrawl" --version)"
[[ "${ACTUAL}" == "scrawl ${VERSION} (${BUILD_NUMBER})" ]] || { echo "Built ${ACTUAL}, expected ${VERSION} (${BUILD_NUMBER})" >&2; exit 1; }

rm -rf "${APP_BUNDLE}" "${DIST_DIR}/staging"
mkdir -p "${APP_BUNDLE}/Contents/MacOS" "${APP_BUNDLE}/Contents/Resources" "${APP_BUNDLE}/Contents/Frameworks" "${DIST_DIR}/staging"

cp -f "${REPO_DIR}/target/release/scrawl" "${APP_BUNDLE}/Contents/MacOS/scrawl"
cp -f "${REPO_DIR}/target/release/scrawl-helper" "${APP_BUNDLE}/Contents/MacOS/scrawl-helper"
chmod +x "${APP_BUNDLE}/Contents/MacOS/scrawl" "${APP_BUNDLE}/Contents/MacOS/scrawl-helper"

if [[ -f "${REPO_DIR}/assets/Scrawl.icns" ]]; then
  cp -f "${REPO_DIR}/assets/Scrawl.icns" "${APP_BUNDLE}/Contents/Resources/Scrawl.icns"
fi

/usr/bin/ditto "${SPARKLE_DIR}/Sparkle.framework" "${APP_BUNDLE}/Contents/Frameworks/Sparkle.framework"

cat > "${APP_BUNDLE}/Contents/Info.plist" <<PLIST
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
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${BUILD_NUMBER}</string>
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
  <true/>
  <key>SUAutomaticallyUpdate</key>
  <false/>
  <key>SUAllowsAutomaticUpdates</key>
  <false/>
  <key>SUFeedURL</key>
  <string>${BASE_URL}/appcast.xml</string>
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
  echo "Developer ID signing identity is unavailable" >&2
  exit 1
fi

ENTITLEMENTS="${REPO_DIR}/scripts/Scrawl.entitlements"
# Notarization requires the hardened runtime and a secure timestamp on every signed binary.
SIGN=(codesign --force --options runtime --timestamp --sign "${IDENTITY}")

SPARKLE_VER="${APP_BUNDLE}/Contents/Frameworks/Sparkle.framework/Versions/B"
"${SIGN[@]}" "${SPARKLE_VER}/XPCServices/Installer.xpc"
"${SIGN[@]}" --preserve-metadata=entitlements "${SPARKLE_VER}/XPCServices/Downloader.xpc"
"${SIGN[@]}" "${SPARKLE_VER}/Autoupdate"
"${SIGN[@]}" "${SPARKLE_VER}/Updater.app"
"${SIGN[@]}" "${APP_BUNDLE}/Contents/Frameworks/Sparkle.framework"
"${SIGN[@]}" --entitlements "${ENTITLEMENTS}" --identifier "dev.kitlangton.scrawl.helper" "${APP_BUNDLE}/Contents/MacOS/scrawl-helper"
"${SIGN[@]}" --entitlements "${ENTITLEMENTS}" --identifier "dev.kitlangton.scrawl" "${APP_BUNDLE}"

/usr/bin/ditto "${APP_BUNDLE}" "${DIST_DIR}/staging/Scrawl.app"
ln -s /Applications "${DIST_DIR}/staging/Applications"
rm -f "${DIST_DIR}/${DMG_NAME}" "${DIST_DIR}/${ZIP_NAME}"
hdiutil create -quiet -fs APFS -format ULFO -volname Scrawl -srcfolder "${DIST_DIR}/staging" "${DIST_DIR}/${DMG_NAME}"
codesign --force --timestamp --sign "${IDENTITY}" "${DIST_DIR}/${DMG_NAME}"

# Notarize the DMG (it carries the app), then staple the tickets to both so Gatekeeper accepts
# them offline. Uses the App Store Connect API key for the same team.
NOTARY_KEY_ID="${SCRAWL_NOTARY_KEY_ID:-JC52MN949W}"
NOTARY_ISSUER="${SCRAWL_NOTARY_ISSUER:-69a6de84-f0e3-47e3-e053-5b8c7c11a4d1}"
NOTARY_KEY="${SCRAWL_NOTARY_KEY:-${HOME}/.appstoreconnect/private_keys/AuthKey_${NOTARY_KEY_ID}.p8}"
if [[ "${IDENTITY}" != "-" && -f "${NOTARY_KEY}" ]]; then
  xcrun notarytool submit "${DIST_DIR}/${DMG_NAME}" --key "${NOTARY_KEY}" --key-id "${NOTARY_KEY_ID}" \
    --issuer "${NOTARY_ISSUER}" --wait --timeout 30m
  xcrun stapler staple "${DIST_DIR}/${DMG_NAME}"
  xcrun stapler staple "${APP_BUNDLE}"
  spctl -a -vv -t install "${DIST_DIR}/${DMG_NAME}"
else
  echo "Notary key is unavailable; refusing to publish" >&2
  exit 1
fi
/usr/bin/ditto -c -k --sequesterRsrc --keepParent "${APP_BUNDLE}" "${DIST_DIR}/${ZIP_NAME}"
echo "Prepared ${DIST_DIR}/${DMG_NAME} and ${DIST_DIR}/${ZIP_NAME}"
