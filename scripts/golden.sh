#!/usr/bin/env bash
# Renders the golden models and compares them with the frozen images (PSNR >= 40 dB).
# Usage: scripts/golden.sh [--update]     (needs GTAV_PATH and a release build)
set -euo pipefail
cd "$(dirname "$0")/.."
V=./target/release/vespucci
OUT=tests/out/golden; mkdir -p "$OUT"
render() { $V --log warn render-model "$1" --technique "$2" --out "$3" >/dev/null; }
declare -A CASES=(
  [bag_unlit]="prop_cs_heist_bag_01 unlit_draw"
  [bag_lit]="prop_cs_heist_bag_01 lightweightHighQuality0_draw"
  [barrier_lit]="prop_barrier_work05 lightweightHighQuality0_draw"
)
fail=0
for name in "${!CASES[@]}"; do
  read -r model technique <<<"${CASES[$name]}"
  render "$model" "$technique" "$OUT/$name.png"
  if [ "${1:-}" = "--update" ]; then cp "$OUT/$name.png" "tests/golden/$name.png"; echo "updated $name"; continue; fi
  printf "%-14s " "$name"
  $V compare "$OUT/$name.png" "tests/golden/$name.png" --min-psnr 40 || fail=1
done
exit $fail
