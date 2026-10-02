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
# World renders (M4): camera position, look-at. Compared at PSNR >= 35 dB.
declare -A WORLD=(
  [world_beach]="-1280,-1450,4 -1200,-1500,4"
  [world_legion]="195,-934,30 230,-900,28"
)
fail=0
check() { # name min_psnr
  if [ "${UPDATE:-}" = 1 ]; then cp "$OUT/$1.png" "tests/golden/$1.png"; echo "updated $1"; return; fi
  printf "%-14s " "$1"
  $V compare "$OUT/$1.png" "tests/golden/$1.png" --min-psnr "$2" || fail=1
}
[ "${1:-}" = "--update" ] && UPDATE=1
for name in "${!CASES[@]}"; do
  read -r model technique <<<"${CASES[$name]}"
  render "$model" "$technique" "$OUT/$name.png"
  check "$name" 40
done
for name in "${!WORLD[@]}"; do
  read -r pos look <<<"${WORLD[$name]}"
  $V --log warn render --pos="$pos" --look="$look" --radius 300 --size 640x360 --lighting basic --out "$OUT/$name.png" >/dev/null
  check "$name" 35
done
exit $fail
