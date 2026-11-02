# Developer ID setup for the fork's Mac releases

The `Mac preview release` workflow published
[0.11.1](https://github.com/tryapparent/apparent/releases/tag/preview-mac-11-1) signed with
Developer ID and notarized by Apple. Keep `MAC_APPLE_SIGNING_ENABLED=true` for later releases.
The first older ad-hoc preview → notarized Sparkle upgrade remains unverified; release
publication does not prove that transition. User update steps are in
[Releasing Apparent](release.md#updating-an-older-preview).

It keeps `Apparent Preview.app`, bundle ID `dev.simonvanlaak.apparent.preview`, the existing
preview Sparkle key/feed and version ordering. It does not change Kit's production app,
register at login, install the CLI, enable silent updates or move to a stable channel.

## Credential setup and maintenance

1. Enroll in the paid Apple Developer Program. Create a **Developer ID Application**
   certificate under your team, not Apple Development, Apple Distribution or Developer ID
   Installer. On a trusted Mac, create the private key/CSR, issue the certificate through
   Apple's developer portal, install it, then export the certificate **and private key**
   from Keychain Access as a password-protected `.p12`. Back up the key securely. A `.cer`
   download or Team ID alone is not enough. Do not use Kit's certificate or key.
2. Create a **team App Store Connect API key** with permission to notarize (Developer role
   or higher; creation may require Account Holder/Admin). Download its `.p8` once and retain
   its key ID and issuer ID. This workflow uses a team key, not an individual key, because
   it passes an issuer. No App Store listing or TestFlight setup is required.
3. In **tryapparent/apparent → Settings → Secrets and variables → Actions**, add:

   | Kind | Name | Value |
   | --- | --- | --- |
   | Secret | `APPLE_DEVELOPER_ID_P12_BASE64` | Base64 of the `.p12`, one line, no whitespace |
   | Secret | `APPLE_DEVELOPER_ID_P12_PASSWORD` | Its nonempty export password |
   | Secret | `APPLE_NOTARY_PRIVATE_KEY` | Full `.p8` PEM contents including header/footer |
   | Variable | `APPLE_SIGNING_IDENTITY` | Exact `Developer ID Application: Name (TEAMID)` identity |
   | Variable | `APPLE_TEAM_ID` | Your 10-character team ID |
   | Variable | `APPLE_NOTARY_KEY_ID` | API key's 10-character ID |
   | Variable | `APPLE_NOTARY_ISSUER` | API issuer UUID |
   | Variable | `MAC_APPLE_SIGNING_ENABLED` | Keep `true` for notarized releases |

   Use GitHub's private secret fields or secure local CLI/file input. Never paste credentials
   into chat, git, issue/PR text, logs, NoMac drives or shell command arguments. Do not create
   placeholder secrets. Existing `SPARKLE_PREVIEW_PUBLIC_KEY` and `SPARKLE_PREVIEW_PRIVATE_KEY`
   **stay unchanged**: they authenticate updates independently of Apple's certificate.

## Signed releases

With `MAC_APPLE_SIGNING_ENABLED=true`, dispatch **Mac preview release** from `main`.
This is a **public release and update-feed publication**, not a dry run. Obtain explicit
release authorization before dispatching; preparing credentials does not authorize it.

The runner validates inputs before building and imports the certificate into a private,
job-only keychain, without importing keys into the login keychain. It adds the temporary
keychain to the runner's user search list so `codesign` can resolve the identity name,
preserving existing entries. The always-run cleanup deletes that temporary keychain.
Nested code is
signed with hardened runtime and secure timestamps. The packager verifies the actual team
and authority, notarizes/staples the app before packaging, then signs/notarizes/staples the
DMG. Signature, stapling and Gatekeeper checks must pass before publication. Missing
credentials or rejection never fall back to ad-hoc signing when the switch is true.
Checksums and Sparkle signatures are computed after stapling. The temporary keychain and
`.p8` are deleted in an always-run cleanup step; neither is cached or uploaded as an artifact.

## Release verification

For each published candidate, retain the applicable evidence below. This checklist is not
a receipt of completed native testing.

- Verify the tag's source SHA, downloaded DMG checksum, release assets and signed appcast.
- On an owned disposable Mac **with Gatekeeper enabled**, download through Safari, confirm
  quarantine, install and launch without Open Anyway, check permissions and make a real capture.
- Separately update an installed older ad-hoc preview to the signed version through Sparkle.
  Verify its version, new running process and Developer ID signature. A signing change can
  require permission reapproval. Keeping the bundle ID/feed/EdDSA key avoids an intentional
  channel migration, but does **not prove** that the first transition works.
- Retain the old release and feed entry for recovery. Never overwrite their tags/assets.

After the first signed release, leave the switch `true`. Fix expired/missing credentials
rather than publishing a higher-version ad-hoc downgrade on the same channel. Rotating
teams/certificates or Sparkle keys, renaming the app, and moving to stable need their own
migration tests. The current preview checks automatically but asks the user to install and
relaunch; notarization does not turn on silent installation.

## NoMac is optional

NoMac can import a Developer ID `.p12` and submit notarization using a connected Apple API
key. That is a separate credential store, not a requirement here: GitHub builds/releases;
NoMac can provide disposable install/update tests. Do not copy signing keys to rented Macs
merely to test a downloaded signed app. For Gatekeeper proof, explicitly enable Gatekeeper
on the disposable session; NoMac can otherwise default to a relaxed environment.

## Local checks

Run `python3 -m unittest discover -s scripts/tests -p 'test_preview_*.py'` and
`bash -n capture/desktop/scripts/release/prepare-mac-signing.sh capture/desktop/scripts/packaging/build-mac-preview.sh`.
The tests use synthetic credentials and fake Mac tools to exercise failure/ordering/layout.
They do not establish certificate validity, native tool compatibility, Apple acceptance,
Gatekeeper behavior or a successful live update. Record native acceptance separately; these local checks cannot close those gates.

References: [Developer ID](https://developer.apple.com/developer-id/),
[Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution),
[notarytool credentials](https://developer.apple.com/documentation/technotes/tn3147-migrating-to-the-latest-notarization-tool),
[NoMac signing API](https://nomac.app/docs/api).
