#!/usr/bin/env bash
# install.sh — Build and install omarchy-spatial-audio (Rust FDTR-XTC DSP + LADSPA + PipeWire config + CLI).
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/omarchy-spatial-audio"
BIN_DIR="$HOME/.local/bin"
PW_CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/pipewire/omarchy-speaker-engine.conf.d"

SPEAKER_SPAN_CM="34.0"
LISTENING_DIST_CM="55.0"
SPEAKER_IRS=""
SOFA_FILE="/usr/share/libmysofa/MIT_KEMAR_normal_pinna.sofa"
TARGET_SINK=""
NATIVE_CPU=0

usage() {
  cat <<'EOF'
Usage: ./install.sh [OPTIONS]

Build the pure-Rust FDTR-XTC spatial DSP engine + LADSPA plugin, synthesize
4x 512-tap @ 48 kHz float32 WAV crosstalk-cancellation kernels for your laptop's
speaker geometry, render the PipeWire filter-chain config, and install the CLI.

Options:
  --speaker-span-cm CM         Acoustic center spacing between L/R speakers (default: 34.0)
  --listening-distance-cm CM   Desk listening distance from screen/speakers (default: 55.0)
  --target-sink NAME           PipeWire physical speaker sink node.name (auto-detected if omitted)
  --irs PATH                   Optional speaker EQ FIR (.irs/.wav); defaults to flat unity impulse
  --sofa PATH                  MIT KEMAR SOFA HRTF file (default: /usr/share/libmysofa/MIT_KEMAR_normal_pinna.sofa)
  --data-dir DIR               Data install directory (default: ~/.local/share/omarchy-spatial-audio)
  --bin-dir DIR                CLI install directory (default: ~/.local/bin)
  --native                     Compile with RUSTFLAGS="-C target-cpu=native"
  -h, --help                   Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --speaker-span-cm)       SPEAKER_SPAN_CM="$2"; shift 2 ;;
    --listening-distance-cm) LISTENING_DIST_CM="$2"; shift 2 ;;
    --target-sink)           TARGET_SINK="$2"; shift 2 ;;
    --irs)                   SPEAKER_IRS="$2"; shift 2 ;;
    --sofa)                  SOFA_FILE="$2"; shift 2 ;;
    --data-dir)              DATA_DIR="$2"; shift 2 ;;
    --bin-dir)               BIN_DIR="$2"; shift 2 ;;
    --native)                NATIVE_CPU=1; shift ;;
    -h|--help)               usage; exit 0 ;;
    *)                       echo "error: unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

for cmd in cargo gcc python3; do
  command -v "$cmd" >/dev/null 2>&1 || { echo "error: missing required build tool: $cmd" >&2; exit 1; }
done

echo "==> Building pure-Rust FDTR-XTC engine & LADSPA plugin (release)"
(
  cd "$SCRIPT_DIR"
  if (( NATIVE_CPU == 1 )); then
    RUSTFLAGS="${RUSTFLAGS:-} -C target-cpu=native" nice -n 19 cargo build --release
  else
    nice -n 19 cargo build --release
  fi
)

mkdir -p "$DATA_DIR"/{bin,lib,kernels,scripts} "$BIN_DIR" "$PW_CONF_DIR"

TARGET_DIR="${CARGO_TARGET_DIR:-$SCRIPT_DIR/target}"

echo "==> Installing binaries, LADSPA plugin, and validator into $DATA_DIR"
install -m 0755 "$TARGET_DIR/release/omarchy-spatial-dsp" "$DATA_DIR/bin/omarchy-spatial-dsp"
install -m 0755 "$TARGET_DIR/release/libomarchy_spatial_fdtr.so" "$DATA_DIR/lib/libomarchy_spatial_fdtr.so"
install -m 0755 "$SCRIPT_DIR/scripts/verify_pipewire_config.py" "$DATA_DIR/scripts/verify_pipewire_config.py"

echo "==> Compiling C-ABI LADSPA real-time verifier"
gcc -O2 -Wall -Wextra "$SCRIPT_DIR/tests/verify_ladspa.c" -ldl -lm -o "$DATA_DIR/bin/verify_ladspa"

echo "==> Synthesizing 4x 512-tap @ 48 kHz FDTR-XTC WAV kernels (S=${SPEAKER_SPAN_CM} cm, D=${LISTENING_DIST_CM} cm)"
"$DATA_DIR/bin/omarchy-spatial-dsp" generate-kernels \
  --out-dir "$DATA_DIR/kernels" \
  --speaker-span-cm "$SPEAKER_SPAN_CM" \
  --listening-distance-cm "$LISTENING_DIST_CM"

# Resolve speaker EQ IRS (fallback: generate stereo 48 kHz flat unity impulse so convolvers always load cleanly)
SPEAKER_IRS_GAIN="1.0"
if [[ -n "$SPEAKER_IRS" ]]; then
  [[ -r "$SPEAKER_IRS" ]] || { echo "error: --irs file not readable: $SPEAKER_IRS" >&2; exit 1; }
  SPEAKER_IRS_GAIN="1.258925412"
elif [[ -r "${XDG_DATA_HOME:-$HOME/.local/share}/easyeffects/irs/xps17-warm-rich.irs" ]]; then
  SPEAKER_IRS="${XDG_DATA_HOME:-$HOME/.local/share}/easyeffects/irs/xps17-warm-rich.irs"
  SPEAKER_IRS_GAIN="1.258925412"
else
  SPEAKER_IRS="$DATA_DIR/kernels/flat_unity_stereo.wav"
  python3 - "$SPEAKER_IRS" <<'PY'
import struct, sys
out = sys.argv[1]
sr, ch, frames = 48000, 2, 64
samples = [1.0, 1.0] + [0.0] * ((frames - 1) * ch)
data = struct.pack(f"<{len(samples)}f", *samples)
fmt = struct.pack("<HHIIHH", 3, ch, sr, sr * ch * 4, ch * 4, 32)
riff_sz = 4 + (8 + len(fmt)) + (8 + len(data))
with open(out, "wb") as f:
    f.write(b"RIFF" + struct.pack("<I", riff_sz) + b"WAVE")
    f.write(b"fmt " + struct.pack("<I", len(fmt)) + fmt)
    f.write(b"data" + struct.pack("<I", len(data)) + data)
PY
fi

# Auto-detect physical speaker sink if not specified
if [[ -z "$TARGET_SINK" ]] && command -v pactl >/dev/null 2>&1; then
  TARGET_SINK="$(pactl list short sinks 2>/dev/null | awk '$2 ~ /^alsa_output\..*(Speaker|analog-stereo).*sink$/ {print $2; exit}' || true)"
fi
TARGET_SINK="${TARGET_SINK:-alsa_output.pci-0000_00_1f.3-platform-sof_sdw.HiFi__Speaker__sink}"

echo "==> Rendering PipeWire filter-chain config -> $PW_CONF_DIR/91-spatial-fdtr.conf"
python3 - "$SCRIPT_DIR/config/91-spatial-fdtr.conf.in" "$PW_CONF_DIR/91-spatial-fdtr.conf" \
  "$DATA_DIR" "$DATA_DIR/lib/libomarchy_spatial_fdtr.so" "$SPEAKER_IRS" "$SPEAKER_IRS_GAIN" "$SOFA_FILE" "$TARGET_SINK" <<'PY'
import sys
src, dst, data_dir, ladspa_so, irs, irs_gain, sofa, target = sys.argv[1:9]
text = open(src, encoding="utf-8").read()
for k, v in [
    ("@DATA_DIR@", data_dir),
    ("@LADSPA_SO@", ladspa_so),
    ("@SPEAKER_IRS@", irs),
    ("@SPEAKER_IRS_GAIN@", irs_gain),
    ("@SOFA_FILE@", sofa),
    ("@TARGET_SINK@", target),
]:
    text = text.replace(k, v)
with open(dst, "w", encoding="utf-8") as f:
    f.write(text)
PY

echo "==> Installing CLI switcher -> $BIN_DIR/omarchy-audio-spatial"
install -m 0755 "$SCRIPT_DIR/bin/omarchy-audio-spatial" "$BIN_DIR/omarchy-audio-spatial"

echo
echo "Install complete!"
echo "  Data dir  : $DATA_DIR"
echo "  PW config : $PW_CONF_DIR/91-spatial-fdtr.conf"
echo "  CLI       : $BIN_DIR/omarchy-audio-spatial"
echo
echo "Verify offline:  omarchy-audio-spatial status"
echo "                 $DATA_DIR/bin/omarchy-spatial-dsp verify --kernels-dir $DATA_DIR/kernels"
echo "                 $DATA_DIR/bin/verify_ladspa $DATA_DIR/lib/libomarchy_spatial_fdtr.so"
