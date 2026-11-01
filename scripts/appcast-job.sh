#!/bin/bash
# Installed as ~/.local/share/scrawl-release/appcast.sh on Pro. The existing GUI-session
# dev.kit.scrawl-appcast job owns keychain access. No private key leaves that session.
set -euo pipefail
R="$HOME/.local/share/scrawl-release"
exec > "$R/appcast.log" 2>&1
export PATH="$HOME/bin:$HOME/.bun/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"
exec python3 "$R/release-channel.py" appcast-job
