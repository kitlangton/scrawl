#!/usr/bin/env bash
# Run only inside a disposable gui-vm.py guest. Never against a user's installation.
# Browser confirmation is deliberately interactive: inspect the job's private VNC view.
set -euo pipefail
: "${SCRAWL_GUI_BINARY:?disposable runner must stage the candidate}"
: "${SCRAWL_GUI_OUTPUT:?disposable runner must own the output directory}"
for tool in jq grim xdg-mime xdg-open update-desktop-database chromium hyprctl; do
  command -v "$tool" >/dev/null || { echo "Missing guest prerequisite: $tool" >&2; exit 1; }
done
while IFS='=' read -r key value; do
  case "$key" in DISPLAY|WAYLAND_DISPLAY|HYPRLAND_INSTANCE_SIGNATURE|DBUS_SESSION_BUS_ADDRESS|XDG_RUNTIME_DIR|XDG_CURRENT_DESKTOP) export "$key=$value";; esac
done < <(systemctl --user show-environment)
: "${HYPRLAND_INSTANCE_SIGNATURE:?missing test-user compositor}"
: "${DISPLAY:?missing XWayland display}"
if pgrep -u "$(id -u)" -f '[a]pparent (app|launch)' >/dev/null; then echo 'Refusing an occupied guest' >&2; exit 1; fi
bin=$SCRAWL_GUI_BINARY
out=$SCRAWL_GUI_OUTPUT
systemd-detect-virt --vm --quiet
if [[ -e "$HOME/.config/apparent/config.json" ]]; then
  echo 'Refusing to overwrite an existing Apparent config; use a fresh disposable guest' >&2
  exit 1
fi
mkdir -p "$HOME/.config/apparent"
# OS/portal launches need the real test-user config, not only our shell's override.
export APPARENT_CONFIG="$HOME/.config/apparent/config.json"
export LD_LIBRARY_PATH="$(dirname "$bin")${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
mkdir -p "$out/captures" "$out/page"
printf '#!/usr/bin/env sh\nprintf '\''{"version":1,"text":"Disposable native launch capture."}\\n'\''\n' > "$out/transcriber.sh"
chmod 700 "$out/transcriber.sh"
jq -n --arg captures "$out/captures" --arg transcriber "$out/transcriber.sh" '{onboarding:{seen:true},capturesDir:$captures,transcriber:"command",transcribeCommand:[$transcriber]}' > "$APPARENT_CONFIG"
cleanup() {
  cp "$APPARENT_CONFIG" "$out/config.json"
  for pid in ${video_pid:-} ${browser_pid:-} ${server_pid:-} ${app_pid:-}; do kill "$pid" 2>/dev/null || true; done
}
trap cleanup EXIT
find_gui() {
  for _ in $(seq 1 80); do
    for pid in $(hyprctl clients -j | jq -r '[.[] | .pid] | unique | .[]'); do
      if [[ "$(readlink -f "/proc/$pid/exe")" == "$bin" ]]; then
        app_pid=$pid
        return 0
      fi
    done
    sleep .25
  done
  echo 'Candidate has no visible Apparent window' >&2
  return 1
}
"$bin" launch-handler status > "$out/previous-handler.txt"
"$bin" launch-handler install > "$out/registration.txt"
"$bin" launch-handler status | tee "$out/selected-handler.txt" | grep -qx apparent-launch.desktop
gio mime x-scheme-handler/apparent > "$out/portal-handlers.txt"
grep -A2 'Registered applications:' "$out/portal-handlers.txt" | grep -q apparent-launch.desktop
cp "$HOME/.local/share/applications/apparent-launch.desktop" "$out/handler.desktop"
xdg-mime query default x-scheme-handler/apparent > "$out/shell-selected-handler.txt"
printf 'Session DISPLAY=%s WAYLAND_DISPLAY=%s desktop=%s\n' "$DISPLAY" "${WAYLAND_DISPLAY:-}" "${XDG_CURRENT_DESKTOP:-}"
echo 'NATIVE_XDG_COLD_START' >&2
# Both the browser and xdg-open inherit the isolated config and staged library directory.
xdg-open 'apparent://open' > "$out/cold-launch.log" 2>&1 &
for _ in $(seq 1 120); do
  if "$bin" capture state > "$out/state.json" 2>/dev/null; then break; fi
  sleep .25
done
jq -e '.state.phase=="idle"' "$out/state.json"
echo 'NATIVE_XDG_COLD_OPENED' >&2
find_gui
test "$(readlink "/proc/$app_pid/exe")" = "$bin"
hyprctl clients -j > "$out/cold-windows.json"
grim "$out/cold-settings.png"
for _ in 1 2 3; do timeout 8 xdg-open 'apparent://open'; done
sleep 1
kill -0 "$app_pid"
test "$(pgrep -u "$(id -u)" -x apparent | wc -l)" = 1
hyprctl clients -j > "$out/warm-windows.json"
for url in 'apparent://open/' 'apparent://open?record=true' 'apparent://record' 'APPARENT://open'; do
  if "$bin" launch "$url" >> "$out/rejected.log" 2>&1; then echo "Accepted malformed URL" >&2; exit 1; fi
done
jq '.experiments.nativeLaunch=false' "$APPARENT_CONFIG" > "$out/config-next.json"
mv "$out/config-next.json" "$APPARENT_CONFIG"
if "$bin" launch 'apparent://open' >> "$out/rejected.log" 2>&1; then echo 'Accepted disabled experiment' >&2; exit 1; fi
jq 'del(.experiments.nativeLaunch)' "$APPARENT_CONFIG" > "$out/config-next.json"
mv "$out/config-next.json" "$APPARENT_CONFIG"
kill "$app_pid"
for _ in $(seq 1 40); do kill -0 "$app_pid" 2>/dev/null || break; sleep .1; done
if kill -0 "$app_pid" 2>/dev/null; then echo 'Test app did not exit' >&2; exit 1; fi
unset app_pid
cat > "$out/page/index.html" <<'HTML'
<!doctype html><meta charset="utf-8"><title>Native Apparent launch proof</title>
<style>body{font:28px sans-serif;background:#f7f5ef;color:#252334;padding:60px}a{display:inline-block;background:#5940b3;color:white;padding:24px;border-radius:12px}</style>
<h1>Disposable native launch proof</h1><p>Real candidate, isolated desktop. No fallback or installation detection.</p>
<a href="apparent://open">Open Apparent</a><p>Accept the browser's external-application prompt to test the registered handler.</p>
HTML
python3 -m http.server 8765 --bind 127.0.0.1 --directory "$out/page" > "$out/http.log" 2>&1 & server_pid=$!
if command -v wf-recorder >/dev/null; then
  wf-recorder -f "$out/browser-native-launch.mp4" > "$out/video.log" 2>&1 & video_pid=$!
fi
chromium --no-first-run --password-store=basic --ozone-platform=x11 --user-data-dir="$HOME/native-launch-browser" http://127.0.0.1:8765 > "$out/browser.log" 2>&1 & browser_pid=$!
sleep 4
grim "$out/browser-ready.png"
echo 'NATIVE_BROWSER_READY: click Open Apparent and accept the native prompt' >&2
for _ in $(seq 1 320); do
  if "$bin" capture state > "$out/browser-state.json" 2>/dev/null; then break; fi
  sleep .25
done
jq -e '.state.phase=="idle"' "$out/browser-state.json"
find_gui
test "$(readlink "/proc/$app_pid/exe")" = "$bin"
hyprctl clients -j > "$out/browser-open-windows.json"
grim "$out/browser-open-settings.png"
echo 'NATIVE_BROWSER_OPENED' >&2
# Exercise the real shortcut/portal path; the URL must not interrupt it.
cat > "$out/picker.sh" <<'PICKER'
#!/usr/bin/env bash
printf '[SELECTION]/screen:%s\n' "$(hyprctl monitors -j | jq -er '.[0].name')"
PICKER
chmod 700 "$out/picker.sh"
printf 'screencopy {\n custom_picker_binary = %s\n}\n' "$out/picker.sh" > "$HOME/.config/hypr/xdph.conf"
systemctl --user restart xdg-desktop-portal-hyprland.service
"$bin" shortcut record
for _ in $(seq 1 120); do
  "$bin" capture state > "$out/recording-before.json"
  if jq -e '.state.phase=="recording"' "$out/recording-before.json" >/dev/null; then break; fi
  sleep .25
done
capture_id=$(jq -er '.state | select(.phase=="recording") | .capture_id' "$out/recording-before.json")
"$bin" launch 'apparent://open'
sleep 2
"$bin" capture state > "$out/recording-after.json"
jq -e --arg id "$capture_id" '.state.phase=="recording" and .state.capture_id==$id' "$out/recording-after.json"
kill -0 "$app_pid"
grim "$out/recording-uninterrupted.png"
"$bin" capture stop "$capture_id" > "$out/stop.json"
for _ in $(seq 1 240); do
  bundle=$(find "$out/captures" -name apparent.json -print -quit)
  test -n "$bundle" && break
  sleep .25
done
test -n "$bundle"
printf '%s\n' "$bundle" > "$out/bundle-path.txt"
hyprctl clients -j > "$out/final-windows.json"
grim "$out/completed.png"
if [[ -n "${video_pid:-}" ]]; then kill -INT "$video_pid"; wait "$video_pid"; unset video_pid; fi
printf 'Native xdg cold/warm, browser cold launch, rejects/flag-off, and active-capture preservation passed.\n'
