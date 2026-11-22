#!/usr/bin/env bash
# Install homelab as a per-user launchd agent on macOS. Safe to re-run.
#
#   scripts/install-macos.sh               build, install, (re)start the agent
#   scripts/install-macos.sh --tailscale   also expose it on the tailnet: tailscale serve --tcp 2222
#
# Never overwrites an existing config.toml or authorized_keys.
set -euo pipefail

LABEL=dev.blakster.homelab
BIN="$HOME/.local/bin/homelab"
CONF_DIR="$HOME/.config/homelab"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
LOG="$HOME/Library/Logs/homelab.log"
PORT=2222
TS=/Applications/Tailscale.app/Contents/MacOS/Tailscale
command -v tailscale >/dev/null 2>&1 && TS=$(command -v tailscale)

cd "$(dirname "$0")/.."
# Prefer rustup's toolchain over any other rustc on PATH (e.g. Homebrew's).
export PATH="$HOME/.cargo/bin:$PATH"
CARGO=cargo

echo "→ building release binary"
"$CARGO" build --release --locked --no-default-features
mkdir -p "$(dirname "$BIN")" "$CONF_DIR" "$(dirname "$PLIST")" "$(dirname "$LOG")"
install -m 0755 target/release/homelab "$BIN"
echo "→ installed $BIN"

if [ ! -f "$CONF_DIR/config.toml" ]; then
  cp config.example.toml "$CONF_DIR/config.toml"
  echo "→ wrote $CONF_DIR/config.toml from config.example.toml (edit it)"
fi

if [ ! -f "$CONF_DIR/authorized_keys" ]; then
  : > "$CONF_DIR/authorized_keys"
  for k in "$HOME"/.ssh/*.pub; do
    [ -f "$k" ] || continue
    cat "$k" >> "$CONF_DIR/authorized_keys"
    echo "→ authorized $(basename "$k")"
  done
  chmod 600 "$CONF_DIR/authorized_keys"
fi

cat > "$PLIST" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL</string>
  <key>ProgramArguments</key>
  <array><string>$BIN</string><string>serve</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>$LOG</string>
  <key>StandardErrorPath</key><string>$LOG</string>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin</string></dict>
</dict>
</plist>
PL
plutil -lint "$PLIST" >/dev/null

launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$PLIST"
echo "→ launchd agent $LABEL loaded"

for _ in $(seq 1 20); do
  nc -z 127.0.0.1 "$PORT" 2>/dev/null && break
  sleep 0.5
done
nc -z 127.0.0.1 "$PORT" && echo "→ listening on 127.0.0.1:$PORT" || { echo "✗ not listening; see $LOG"; exit 1; }

if [ "${1:-}" = "--tailscale" ]; then
  "$TS" serve --bg --tcp "$PORT" "tcp://127.0.0.1:$PORT"
  "$TS" serve status
  echo "→ connect from any tailnet device: ssh -p $PORT $(whoami)@$("$TS" status --json | python3 -c 'import json,sys;print(json.load(sys.stdin)["Self"]["DNSName"].rstrip("."))')"
fi
