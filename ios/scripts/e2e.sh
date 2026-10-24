#!/bin/bash
# The whole way from Scrawl for iPhone to an OpenCode session, in the simulator, without
# touching your own Scrawl or OpenCode: a `scrawl serve` with its own home folder, captures and
# port, whose OpenCode is a stand-in (scripts/fake-opencode.py lists sessions; a fake `opencode2`
# writes down the prompt it's handed instead of posting it). The UI tests pair with it, pick a
# session, send a test capture, and share a screenshot from Photos through the share extension,
# each waiting for "Sent to …"; then the bundles and prompts are checked.
#
#   ios/scripts/e2e.sh            # SIMULATOR="iPhone 17 Pro" to pick one
#
# Needs `cargo` (the Mac side is built first) and XcodeGen.
set -euo pipefail
IOS=$(cd "$(dirname "$0")/.." && pwd)
REPO=$(cd "$IOS/.." && pwd)
PORT=${SCRAWL_E2E_PORT:-47788}
OPENCODE_PORT=${SCRAWL_E2E_OPENCODE_PORT:-48760}
SESSION_ID=ses_e2e_scrawl
SESSION_TITLE="E2E SESSION"
SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/scrawl-e2e.XXXXXX")
echo "sandbox: $SANDBOX"

pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
}
trap cleanup EXIT

(cd "$REPO" && cargo build --quiet)

for port in "$PORT" "$OPENCODE_PORT"; do
  if nc -z 127.0.0.1 "$port" 2>/dev/null; then
    echo "Port $port is already taken (an earlier run still going?); set SCRAWL_E2E_PORT / SCRAWL_E2E_OPENCODE_PORT" >&2
    exit 1
  fi
done

python3 "$IOS/scripts/fake-opencode.py" "$OPENCODE_PORT" 2>"$SANDBOX/fake-opencode.log" &
pids+=($!)
mkdir -p "$SANDBOX/home" "$SANDBOX/state/opencode" "$SANDBOX/captures"
echo "{\"url\": \"http://127.0.0.1:$OPENCODE_PORT\"}" >"$SANDBOX/state/opencode/service.json"
cat >"$SANDBOX/opencode2" <<EOF
#!/bin/sh
# Writes down what Scrawl asks of OpenCode instead of asking it.
printf '%s\n' "\$*" >>"$SANDBOX/opencode2.log"
echo '{}'
EOF
chmod +x "$SANDBOX/opencode2"

SANDBOXED=(env HOME="$SANDBOX/home" XDG_STATE_HOME="$SANDBOX/state" OPENCODE_BIN="$SANDBOX/opencode2"
  SCRAWL_CAPTURES_DIR="$SANDBOX/captures" "$REPO/target/debug/scrawl")
# Started directly (env execs scrawl), so $! is the server itself and cleanup stops it.
"${SANDBOXED[@]}" serve --port "$PORT" >"$SANDBOX/serve.log" 2>&1 &
pids+=($!)
for _ in $(seq 50); do
  nc -z 127.0.0.1 "$PORT" 2>/dev/null && break
  sleep 0.2
done
LINK=$("${SANDBOXED[@]}" link --url "http://127.0.0.1:$PORT" 2>/dev/null)
case "$LINK" in scrawl://scrawl\?*) ;; *) echo "scrawl link printed: $LINK" >&2; exit 1 ;; esac

SIMULATOR=${SIMULATOR:-$(xcrun simctl list devices available -j | python3 -c '
import json, sys
names = [d["name"] for runtime, devices in json.load(sys.stdin)["devices"].items() if "iOS" in runtime for d in devices]
phones = [n for n in names if n.startswith("iPhone")]
print(next((n for n in phones if "Pro" in n), phones[0]))')}
echo "simulator: $SIMULATOR"
xcrun simctl boot "$SIMULATOR" 2>/dev/null || true
# A fresh install, so an earlier run's "Sent to" can't pass for this one's.
APP_ID=$(sed -n 's/^SCRAWL_APP_ID *= *//p' "$IOS/Config/Local.xcconfig" 2>/dev/null || true)
[ -n "$APP_ID" ] || APP_ID=$(sed -n 's/^SCRAWL_APP_ID *= *//p' "$IOS/Config/Scrawl.xcconfig")
xcrun simctl uninstall "$SIMULATOR" "$APP_ID" 2>/dev/null || true
# A real screenshot in Photos, for the share test: the Settings app.
xcrun simctl launch "$SIMULATOR" com.apple.Preferences >/dev/null
sleep 3
xcrun simctl io "$SIMULATOR" screenshot "$SANDBOX/screenshot.png" >/dev/null 2>&1
xcrun simctl terminate "$SIMULATOR" com.apple.Preferences
xcrun simctl addmedia "$SIMULATOR" "$SANDBOX/screenshot.png"

cd "$IOS"
xcodegen generate --quiet
TEST_RUNNER_SCRAWL_E2E_LINK="$LINK" TEST_RUNNER_SCRAWL_E2E_SESSION="$SESSION_TITLE" xcodebuild test \
  -project Scrawl.xcodeproj -scheme Scrawl \
  -destination "platform=iOS Simulator,name=$SIMULATOR" \
  -derivedDataPath .build -resultBundlePath "$SANDBOX/e2e.xcresult" \
  -only-testing:ScrawlUITests \
  | grep -E "error:|Test Case|TEST (SUCCEEDED|FAILED)|XCTSkip|skipped" || true
xcrun xcresulttool export attachments --path "$SANDBOX/e2e.xcresult" --output-path "$SANDBOX/screenshots" >/dev/null 2>&1 || true

fail() {
  echo "FAIL: $*" >&2
  echo "logs: $SANDBOX/serve.log, $SANDBOX/home/Library/Logs/Scrawl/scrawl.log" >&2
  exit 1
}
check() {
  local note=$1 bundle
  bundle=$(grep -l "^Note: $note" "$SANDBOX/captures"/*.scrawl/summary.md 2>/dev/null | head -1)
  [ -n "$bundle" ] || fail "no bundle has the note \"$note\""
  python3 - "$(dirname "$bundle")/scrawl.json" <<'EOF' || fail "$(dirname "$bundle") isn't a shared iPhone capture with a keyframe"
import json, sys
bundle = json.load(open(sys.argv[1]))
assert bundle["source"] == "iphone" and bundle["device"]["capture"] == "shared", bundle.get("device")
assert bundle["keyframes"], "no keyframes"
EOF
  echo "$(dirname "$bundle")"
}
test_capture=$(check "A test capture")
shared=$(check "The Action Button row has no icon")
prompts=$(grep -c "api post /api/session/$SESSION_ID/prompt" "$SANDBOX/opencode2.log" 2>/dev/null || true)
[ "$prompts" = 2 ] || fail "expected 2 captures queued into $SESSION_ID, got ${prompts:-0}"

echo
echo "PASS: the test capture and the shared screenshot reached $SESSION_ID"
echo "screenshots: $SANDBOX/screenshots"
for bundle in "$test_capture" "$shared"; do
  echo
  sed -n 1,6p "$bundle/summary.md"
done
