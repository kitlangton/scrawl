#!/usr/bin/env bash
# Build an isolated Mac preview. Apple signing is explicit and fails closed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
VERSION="${APPARENT_PREVIEW_VERSION:?set APPARENT_PREVIEW_VERSION (e.g. 0.123.1)}"
BUILD_NUMBER="${APPARENT_PREVIEW_BUILD_NUMBER:?set APPARENT_PREVIEW_BUILD_NUMBER (e.g. 123.1)}"
PUBLIC_KEY="${APPARENT_PREVIEW_PUBLIC_KEY:?set the preview Sparkle EdDSA public key}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "Invalid preview version" >&2; exit 1; }
[[ "$BUILD_NUMBER" =~ ^[0-9]+\.[0-9]+$ ]] || { echo "Invalid preview build number" >&2; exit 1; }
python3 - "$PUBLIC_KEY" <<'PY'
import base64, sys
try:
    key = base64.b64decode(sys.argv[1], validate=True)
    if len(key) != 32:
        raise ValueError('wrong public key length')
except ValueError:
    sys.exit('Preview Sparkle public key must be 32 base64-encoded bytes')
PY

cd "$ROOT"
NOTARIZE="${APPARENT_PREVIEW_NOTARIZE:-false}"
SIGN=(codesign --force --sign -)
case "$NOTARIZE" in
  false) ;;
  true)
    : "${APPARENT_APPLE_SIGNING_IDENTITY:?set the Developer ID Application identity}"
    : "${APPARENT_APPLE_TEAM_ID:?set the Apple team ID}"
    : "${APPARENT_APPLE_SIGNING_DIR:?run capture/desktop/scripts/release/prepare-mac-signing.sh first}"
    : "${APPARENT_NOTARY_KEY_ID:?set the notarization API key ID}"
    : "${APPARENT_NOTARY_ISSUER:?set the notarization API issuer}"
    [[ "$APPARENT_APPLE_SIGNING_IDENTITY" == "Developer ID Application: "*" ($APPARENT_APPLE_TEAM_ID)" ]] || {
      echo 'Expected a Developer ID Application certificate for the configured team' >&2; exit 1;
    }
    test -f "$APPARENT_APPLE_SIGNING_DIR/signing.keychain-db"
    test -s "$APPARENT_APPLE_SIGNING_DIR/notary.p8"
    SIGN=(codesign --force --options runtime --timestamp --keychain "$APPARENT_APPLE_SIGNING_DIR/signing.keychain-db" --sign "$APPARENT_APPLE_SIGNING_IDENTITY")
    ;;
  *) echo 'APPARENT_PREVIEW_NOTARIZE must be true or false' >&2; exit 1 ;;
esac
SPARKLE_DIR="$(capture/desktop/scripts/release/setup-sparkle.sh)"
APPARENT_PREVIEW_BUILD=1 APPARENT_HELPER_BUNDLE_ID=dev.simonvanlaak.apparent.preview.helper \
  cargo build --release --locked -j 3

dist="$ROOT/dist"
app="$dist/Apparent Preview.app"
staging="$dist/preview-staging"
name="Apparent-Preview-${VERSION}-arm64.dmg"
zip="Apparent-Preview-${VERSION}-arm64.zip"
rm -rf "$app" "$staging"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks" "$staging"
cp target/release/apparent target/release/apparent-helper "$app/Contents/MacOS/"
chmod +x "$app/Contents/MacOS/apparent" "$app/Contents/MacOS/apparent-helper"
cp capture/desktop/assets/Apparent.icns "$app/Contents/Resources/Apparent.icns"
cp shared/assets/fonts/HostGrotesk-Variable.ttf "$app/Contents/Resources/HostGrotesk-Variable.ttf"
cp shared/assets/brand/apparent-wordmark.png "$app/Contents/Resources/apparent-wordmark.png"
cp shared/assets/brand/apparent-a.png "$app/Contents/Resources/apparent-a.png"
cp shared/assets/brand/connect-frogs.png "$app/Contents/Resources/connect-frogs.png"
node capture/desktop/ui/frontend/scripts/package-notes.mjs "$app/Contents/Resources/note-ui"
ditto "$SPARKLE_DIR/Sparkle.framework" "$app/Contents/Frameworks/Sparkle.framework"

cat > "$app/Contents/Info.plist" <<PLIST
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
  <key>CFBundleExecutable</key><string>apparent</string>
  <key>CFBundleIconFile</key><string>Apparent</string>
  <key>CFBundleIdentifier</key><string>dev.simonvanlaak.apparent.preview</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Apparent Preview</string>
  <key>CFBundleDisplayName</key><string>Apparent Preview</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${BUILD_NUMBER}</string>
  <key>LSMinimumSystemVersion</key><string>15.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSScreenCaptureUsageDescription</key><string>Apparent captures your screen to build annotated agent bundles.</string>
  <key>NSMicrophoneUsageDescription</key><string>Apparent records your voice to align transcripts with keyframes.</string>
  <key>NSCameraUsageDescription</key><string>Apparent mirrors and records a cabled iPhone's screen.</string>
  <key>NSSpeechRecognitionUsageDescription</key><string>Apparent transcribes your narration on this Mac.</string>
  <key>NSAppleEventsUsageDescription</key><string>Apparent reads the address of the browser tab you captured to record its context.</string>
  <key>SUEnableAutomaticChecks</key><true/>
  <key>SUAutomaticallyUpdate</key><false/>
  <key>SUAllowsAutomaticUpdates</key><false/>
  <key>SUFeedURL</key><string>https://raw.githubusercontent.com/tryapparent/apparent/preview-mac-feed/appcast.xml</string>
  <key>SUPublicEDKey</key><string>${PUBLIC_KEY}</string>
  <key>SUVerifyUpdateBeforeExtraction</key><true/>
</dict>
</plist>
PLIST

if [[ "$NOTARIZE" == true ]]; then
  python3 "$ROOT/capture/desktop/scripts/packaging/stage-settings.py" "$app/Contents/Resources/settings" \
    --sign "$APPARENT_APPLE_SIGNING_IDENTITY" --keychain "$APPARENT_APPLE_SIGNING_DIR/signing.keychain-db"
else
  python3 "$ROOT/capture/desktop/scripts/packaging/stage-settings.py" "$app/Contents/Resources/settings" --sign -
fi

plutil -lint "$app/Contents/Info.plist"
sparkle="$app/Contents/Frameworks/Sparkle.framework/Versions/B"
"${SIGN[@]}" "$sparkle/XPCServices/Installer.xpc"
"${SIGN[@]}" --preserve-metadata=entitlements "$sparkle/XPCServices/Downloader.xpc"
"${SIGN[@]}" "$sparkle/Autoupdate"
"${SIGN[@]}" "$sparkle/Updater.app"
"${SIGN[@]}" "$app/Contents/Frameworks/Sparkle.framework"
if [[ "$NOTARIZE" == true ]]; then
  "${SIGN[@]}" --entitlements "$ROOT/capture/desktop/scripts/packaging/Apparent.entitlements" --identifier dev.simonvanlaak.apparent.preview.helper "$app/Contents/MacOS/apparent-helper"
  "${SIGN[@]}" --entitlements "$ROOT/capture/desktop/scripts/packaging/Apparent.entitlements" --identifier dev.simonvanlaak.apparent.preview "$app"
else
  "${SIGN[@]}" --identifier dev.simonvanlaak.apparent.preview.helper "$app/Contents/MacOS/apparent-helper"
  "${SIGN[@]}" --identifier dev.simonvanlaak.apparent.preview "$app"
fi
codesign --verify --deep --strict --verbose=2 "$app"

if [[ "$NOTARIZE" == true ]]; then
  # Verify the actual signature, not just the requested identity string.
  signature="$(codesign -dv --verbose=4 "$app" 2>&1)"
  grep -Fxq "TeamIdentifier=$APPARENT_APPLE_TEAM_ID" <<< "$signature"
  grep -Fxq "Authority=$APPARENT_APPLE_SIGNING_IDENTITY" <<< "$signature"
  # Notarize/staple the app before copying it into either distribution archive.
  notary_zip="$dist/preview-notary.zip"
  trap 'rm -f "$notary_zip"' EXIT
  ditto -c -k --sequesterRsrc --keepParent "$app" "$notary_zip"
  xcrun notarytool submit "$notary_zip" --key "$APPARENT_APPLE_SIGNING_DIR/notary.p8" \
    --key-id "$APPARENT_NOTARY_KEY_ID" --issuer "$APPARENT_NOTARY_ISSUER" --wait --timeout 30m
  xcrun stapler staple "$app"
  xcrun stapler validate "$app"
  spctl --assess --type execute --verbose=2 "$app"
fi

# A separately signed launcher uses the shared React installer and the payload's
# already signed Electron runtime. Sparkle ZIP below still contains only the app.
installer="$dist/Install Apparent.app"
rm -rf "$installer"
mkdir -p "$installer/Contents/MacOS" "$installer/Contents/Resources" "$installer/Contents/SharedSupport"
cp capture/desktop/assets/Apparent.icns "$installer/Contents/Resources/Apparent.icns"
installer_arch="$(lipo -archs "$app/Contents/MacOS/apparent")"
case "$installer_arch" in arm64|x86_64) ;; *) echo 'Installer needs a single supported Mac architecture' >&2; exit 1 ;; esac
xcrun swiftc -O -parse-as-library -target "$installer_arch-apple-macos15.0" capture/desktop/native/installer/Installer.swift -o "$installer/Contents/MacOS/Installer"
ditto "$app" "$installer/Contents/SharedSupport/Apparent Preview.app"
ditto "$app/Contents/Resources/settings/app" "$installer/Contents/Resources/installer-ui"
python3 - "$installer" "$VERSION" "$BUILD_NUMBER" <<'PYINSTALLER'
import json, pathlib, plistlib, sys
app = pathlib.Path(sys.argv[1])
(app / 'Contents/Info.plist').write_bytes(plistlib.dumps({
    'CFBundleIdentifier': 'dev.simonvanlaak.apparent.preview.installer',
    'CFBundleName': 'Install Apparent', 'CFBundleDisplayName': 'Install Apparent',
    'CFBundleExecutable': 'Installer', 'CFBundlePackageType': 'APPL', 'CFBundleIconFile': 'Apparent',
    'CFBundleShortVersionString': sys.argv[2], 'CFBundleVersion': sys.argv[3],
    'LSMinimumSystemVersion': '15.0', 'NSHighResolutionCapable': True,
}))
(app / 'Contents/Resources/installer-ui/package.json').write_text(json.dumps({
    'name': 'apparent-installer', 'version': '0.0.0', 'main': 'electron/installer/main.cjs',
}) + '\n')
PYINSTALLER
"${SIGN[@]}" --identifier dev.simonvanlaak.apparent.preview.installer "$installer"
codesign --verify --deep --strict "$installer"
if [[ "$NOTARIZE" == true ]]; then
  installer_notary="$dist/installer-notary.zip"
  ditto -c -k --sequesterRsrc --keepParent "$installer" "$installer_notary"
  xcrun notarytool submit "$installer_notary" --key "$APPARENT_APPLE_SIGNING_DIR/notary.p8" \
    --key-id "$APPARENT_NOTARY_KEY_ID" --issuer "$APPARENT_NOTARY_ISSUER" --wait --timeout 30m
  rm -f "$installer_notary"
  xcrun stapler staple "$installer"
  xcrun stapler validate "$installer"
  spctl --assess --type execute --verbose=2 "$installer"
fi
ditto "$installer" "$staging/Install Apparent.app"
printf '%s\n' 'Double-click Install Apparent to install with the frog, or drag Apparent Preview to Applications in Finder.' \
  'Opening the downloaded disk image does not automatically run the installer.' > "$staging/Start here.txt"
cp -R "$app" "$staging/Apparent Preview.app"
ln -s /Applications "$staging/Applications"
rm -f "$dist/$name" "$dist/$name.sha256"
hdiutil create -quiet -fs HFS+ -format UDZO -volname "Apparent Preview" -srcfolder "$staging" "$dist/$name"
hdiutil verify "$dist/$name"
if [[ "$NOTARIZE" == true ]]; then
  "${SIGN[@]}" "$dist/$name"
  xcrun notarytool submit "$dist/$name" --key "$APPARENT_APPLE_SIGNING_DIR/notary.p8" \
    --key-id "$APPARENT_NOTARY_KEY_ID" --issuer "$APPARENT_NOTARY_ISSUER" --wait --timeout 30m
  xcrun stapler staple "$dist/$name"
  xcrun stapler validate "$dist/$name"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$dist/$name"
fi
(cd "$dist" && shasum -a 256 "$name" > "$name.sha256")
ditto -c -k --sequesterRsrc --keepParent "$app" "$dist/$zip"
printf 'Built %s and %s\n' "$dist/$name" "$dist/$zip"
