#!/usr/bin/env bash
# Prepare a job-private signing keychain; never use the login keychain.
set -euo pipefail
# Disable tracing before expanding any secret, even if the caller enabled it.
set +x
umask 077
: "${RUNNER_TEMP:?this script is for an ephemeral GitHub Mac runner}"
for name in APPLE_DEVELOPER_ID_P12_BASE64 APPLE_DEVELOPER_ID_P12_PASSWORD \
            APPLE_NOTARY_PRIVATE_KEY APPARENT_APPLE_SIGNING_IDENTITY \
            APPARENT_APPLE_TEAM_ID APPARENT_NOTARY_KEY_ID APPARENT_NOTARY_ISSUER; do
  if [[ -z "${!name:-}" ]]; then echo "Missing Apple signing input: $name" >&2; exit 1; fi
done
[[ "$APPARENT_APPLE_TEAM_ID" =~ ^[A-Z0-9]{10}$ ]]
[[ "$APPARENT_NOTARY_KEY_ID" =~ ^[A-Z0-9]{10}$ ]]
[[ "$APPARENT_NOTARY_ISSUER" =~ ^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[a-fA-F0-9]{4}-[a-fA-F0-9]{4}-[a-fA-F0-9]{12}$ ]]
[[ "$APPARENT_APPLE_SIGNING_IDENTITY" == "Developer ID Application: "*" ($APPARENT_APPLE_TEAM_ID)" ]]

directory="$RUNNER_TEMP/apparent-apple-signing"
# Refuse an existing directory rather than deleting a previous owner's files.
mkdir -m 700 "$directory"
keychain="$directory/signing.keychain-db"
cleanup_on_error() {
  status=$?
  if [[ "$status" != 0 ]]; then
    security delete-keychain "$keychain" >/dev/null 2>&1 || true
    rm -rf "$directory"
  fi
}
trap cleanup_on_error EXIT
python3 - "$directory" <<'PY'
import base64, os, pathlib, sys
directory = pathlib.Path(sys.argv[1])
try:
    certificate = base64.b64decode(os.environ['APPLE_DEVELOPER_ID_P12_BASE64'], validate=True)
    key = os.environ['APPLE_NOTARY_PRIVATE_KEY']
    if not certificate or not key.startswith('-----BEGIN PRIVATE KEY-----') or '-----END PRIVATE KEY-----' not in key:
        raise ValueError()
except ValueError:
    sys.exit('Invalid certificate encoding or notarization private-key envelope')
(directory / 'certificate.p12').write_bytes(certificate)
(directory / 'notary.p8').write_text(key)
for name in ('certificate.p12', 'notary.p8'):
    (directory / name).chmod(0o600)
PY
password="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
echo "::add-mask::$password"
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
# No -A: restrict imported private-key access to Apple's signing tools.
security import "$directory/certificate.p12" -k "$keychain" \
  -P "$APPLE_DEVELOPER_ID_P12_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$password" "$keychain" >/dev/null
security find-identity -v -p codesigning "$keychain" | grep -Fq -- "\"$APPARENT_APPLE_SIGNING_IDENTITY\""
# codesign resolves identity names through the user's search list even with
# --keychain. Preserve existing entries, including paths containing spaces.
python3 - "$keychain" <<'PY'
import shlex, subprocess, sys
keychain = sys.argv[1]
existing = shlex.split(subprocess.check_output(
    ['security', 'list-keychains', '-d', 'user'], text=True))
subprocess.run(['security', 'list-keychains', '-d', 'user', '-s', keychain,
                *(path for path in existing if path != keychain)], check=True)
PY
rm -f "$directory/certificate.p12"
printf 'Developer ID identity imported into the job-private keychain.\n'
