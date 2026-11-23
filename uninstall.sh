#!/usr/bin/env bash
# uninstall.sh — Cleanly remove omarchy-spatial-audio files and PipeWire drop-in.
set -euo pipefail

DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/omarchy-spatial-audio"
BIN_DIR="$HOME/.local/bin"
PW_CONF="${XDG_CONFIG_HOME:-$HOME/.config}/pipewire/omarchy-speaker-engine.conf.d/91-spatial-fdtr.conf"

NO_SWITCH=0
[[ "${1:-}" == "--no-switch" ]] && NO_SWITCH=1

if (( NO_SWITCH == 0 )) && [[ -n "${XDG_RUNTIME_DIR:-}" ]] && [[ -x "$BIN_DIR/omarchy-audio-spatial" ]]; then
  echo "==> Checking active audio profile before removal"
  if command -v pactl >/dev/null 2>&1; then
    cur="$(pactl get-default-sink 2>/dev/null || true)"
    if [[ "$cur" == omarchy_speaker_spatial* ]]; then
      echo "    Active sink is $cur; rolling back to default speaker profile first..."
      "$BIN_DIR/omarchy-audio-spatial" rollback || true
    fi
  fi
fi

rm -rf "$DATA_DIR"
rm -f "$PW_CONF" "$BIN_DIR/omarchy-audio-spatial"

echo "Removed:"
echo "  $DATA_DIR"
echo "  $PW_CONF"
echo "  $BIN_DIR/omarchy-audio-spatial"
