#!/usr/bin/env bash
# Installs the current `main` as ~/Applications/Apparent.app on another Mac over SSH (default: pro).
#
# Two macOS rules shape this. Signing needs the login keychain, which only processes in the GUI
# session can use, so the build and signing run as a one-off launchd job there. App Management
# stops that job from writing inside an installed app, so it builds and signs a staging copy, and
# SSH (which may write there) copies the signed files into place and launches the app.
# Finish captures and quit the destination app before installing; it is never killed.
set -euo pipefail

HOST="${1:-pro}"
STAGE="/tmp/ApparentStage.app"
LABEL="dev.kitlangton.apparent-install-$(date +%s)"

ssh "${HOST}" bash -s -- "${STAGE}" "${LABEL}" <<'REMOTE'
set -euo pipefail
# Keep process inspection independent of the caller's locale.
export LC_ALL=C
STAGE="$1"; LABEL="$2"
JOB="/tmp/${LABEL}.sh"; PLIST="/tmp/${LABEL}.plist"; LOG="/tmp/${LABEL}.log"
assert_app_stopped() {
  local app="$1" processes pid executable
  # comm is executable identity, not arguments mentioning another installation.
  if ! processes="$(ps -axww -o pid= -o comm=)"; then
    echo "can't inspect running processes; destination app is untouched; retry when process inspection works" >&2
    exit 1
  fi
  while read -r pid executable; do
    if [[ "${executable}" == "${app}/Contents/"* ]]; then
      echo "${app} is in use (pid ${pid}: ${executable}); finish any capture, quit the app and its helpers, then retry" >&2
      exit 1
    fi
  done <<< "${processes}"
}
cd ~/code/open-source/apparent
git pull -q --ff-only
git log --oneline -1

cat > "${JOB}" <<EOF
#!/bin/zsh
export PATH="\$HOME/.cargo/bin:\$HOME/.bun/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
exec > "${LOG}" 2>&1
trap 'echo "STAGE-EXIT=\$?"' EXIT
cd ~/code/open-source/apparent
$(declare -f assert_app_stopped)
assert_app_stopped "${STAGE}"
rm -rf "${STAGE}"
APPARENT_APP_DIR="${STAGE}" APPARENT_NO_LAUNCH=1 ./capture/desktop/scripts/packaging/install-app.sh && codesign --verify --deep --strict "${STAGE}"
EOF
chmod +x "${JOB}"
cat > "${PLIST}" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>${LABEL}</string>
<key>ProgramArguments</key><array><string>${JOB}</string></array>
<key>RunAtLoad</key><true/>
</dict></plist>
EOF
launchctl bootstrap "gui/$(id -u)" "${PLIST}"
for _ in $(seq 1 180); do
  grep -q "STAGE-EXIT=" "${LOG}" 2>/dev/null && break
  sleep 5
done
launchctl bootout "gui/$(id -u)/${LABEL}" 2>/dev/null || true
if ! grep -q "STAGE-EXIT=0" "${LOG}" 2>/dev/null; then
  tail -n 30 "${LOG}" 2>/dev/null
  echo "staging build or signing failed; the installed app is untouched" >&2
  exit 1
fi

APP="${HOME}/Applications/Apparent.app"
assert_app_stopped "${APP}"
mkdir -p "${APP}"
/usr/bin/ditto "${STAGE}" "${APP}"
codesign --verify --deep --strict "${APP}"
codesign -dvv "${APP}" 2>&1 | grep -E "^Authority=Developer ID Application" || echo "warning: not Developer ID signed" >&2
mkdir -p "${HOME}/bin"
if [[ -e "${HOME}/bin/apparent" || -L "${HOME}/bin/apparent" ]]; then
  if [[ ! -L "${HOME}/bin/apparent" ]] || [[ "$(readlink "${HOME}/bin/apparent")" != "${APP}/Contents/MacOS/apparent" ]]; then
    echo "${HOME}/bin/apparent belongs to another program; leaving it alone" >&2
    exit 1
  fi
fi
ln -sfn "${APP}/Contents/MacOS/apparent" "${HOME}/bin/apparent"
open -a "${APP}"
sleep 2
ps -axww -o pid= -o comm= | while read -r PID EXECUTABLE; do
  [[ "${EXECUTABLE}" != "${APP}/Contents/MacOS/apparent" ]] || echo "${PID} ${EXECUTABLE}"
done
assert_app_stopped "${STAGE}"
rm -rf "${STAGE}" "${JOB}" "${PLIST}"
REMOTE
