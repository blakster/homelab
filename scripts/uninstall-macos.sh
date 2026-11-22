#!/usr/bin/env bash
# Remove the homelab launchd agent and binary. Keeps ~/.config/homelab unless --purge.
set -euo pipefail
LABEL=dev.blakster.homelab
PORT=2222
TS=/Applications/Tailscale.app/Contents/MacOS/Tailscale
command -v tailscale >/dev/null 2>&1 && TS=$(command -v tailscale)

launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
rm -f "$HOME/Library/LaunchAgents/$LABEL.plist" "$HOME/.local/bin/homelab"
# Only removes the tcp:2222 handler; other serve routes (e.g. :443) are untouched.
"$TS" serve --tcp="$PORT" off 2>/dev/null || true
if [ "${1:-}" = "--purge" ]; then
  rm -rf "$HOME/.config/homelab"
fi
echo "homelab removed"
