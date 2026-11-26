#!/usr/bin/env bash
# Installs the current `main` as /Applications/Scrawl.app on another Mac over SSH (default: pro).
#
# Two macOS rules shape this. Signing needs the login keychain, which only processes in the GUI
# session can use, so the build and signing run as a one-off launchd job there. App Management
# stops that job from writing inside an installed app, so it builds and signs a staging copy, and
# SSH (which may write there) copies the signed files into place and restarts the app.
set -euo pipefail

HOST="${1:-pro}"
STAGE="/tmp/ScrawlStage.app"
LABEL="dev.kitlangton.scrawl-install-$(date +%s)"

ssh "${HOST}" bash -s -- "${STAGE}" "${LABEL}" <<'REMOTE'
set -euo pipefail
# pgrep/pkill fail under some locales (C.UTF-8), which left the old app running.
export LC_ALL=C
STAGE="$1"; LABEL="$2"
JOB="/tmp/${LABEL}.sh"; PLIST="/tmp/${LABEL}.plist"; LOG="/tmp/${LABEL}.log"
cd ~/code/open-source/scrawl
git pull -q --ff-only
git log --oneline -1

cat > "${JOB}" <<EOF
#!/bin/zsh
export PATH="\$HOME/.cargo/bin:\$HOME/.bun/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
exec > "${LOG}" 2>&1
cd ~/code/open-source/scrawl
rm -rf "${STAGE}"
SCRAWL_APP_DIR="${STAGE}" SCRAWL_NO_LAUNCH=1 ./scripts/install-app.sh && codesign --verify --deep --strict "${STAGE}"
echo "STAGE-EXIT=\$?"
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

APP="/Applications/Scrawl.app"
mkdir -p "${APP}"
/usr/bin/ditto "${STAGE}" "${APP}"
codesign --verify --deep --strict "${APP}"
codesign -dvv "${APP}" 2>&1 | grep -E "^Authority=Developer ID Application" || echo "warning: not Developer ID signed" >&2
ln -sf "${APP}/Contents/MacOS/scrawl" "${HOME}/bin/scrawl"
pkill -f "Scrawl.app/Contents/MacOS/scrawl" 2>/dev/null || true
for _ in $(seq 1 40); do
  status=0
  pgrep -f "Scrawl.app/Contents/MacOS/scrawl" >/dev/null || status=$?
  [[ "${status}" == 0 ]] || break
  sleep 0.05
done
if [[ "${status}" != 1 ]]; then
  pgrep -fl "Scrawl.app/Contents/MacOS/scrawl" >&2 || true
  echo "the old Scrawl is still running (pgrep exit ${status}); quit it and run this again" >&2
  exit 1
fi
rm -rf "${HOME}/Applications/Scrawl.app"
open -a "${APP}"
sleep 2
pgrep -fl "Scrawl.app/Contents/MacOS/scrawl"
rm -rf "${STAGE}" "${JOB}" "${PLIST}"
REMOTE
