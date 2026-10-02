#!/usr/bin/env bash
# Environment for running Vespucci headless on lavapipe through DXVK-native.
# The binary sets the same defaults itself; this is for running other tools
# (vulkaninfo, tests) under the same conditions. Source it: `. scripts/vespucci-env.sh`
export DXVK_WSI_DRIVER=SDL3
export SDL_VIDEO_DRIVER=offscreen
export VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json
export DXVK_FILTER_DEVICE_NAME=llvmpipe
export DXVK_LOG_LEVEL=${DXVK_LOG_LEVEL:-warn}
export DXVK_STATE_CACHE_PATH="$HOME/.cache/vespucci/dxvk"
export MESA_SHADER_CACHE_DIR="$HOME/.cache/vespucci/mesa"
export MESA_SHADER_CACHE_MAX_SIZE=2G
export LP_NUM_THREADS=4
export LD_LIBRARY_PATH="/opt/dxvk-native/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export GTAV_PATH="${GTAV_PATH:-/root/gtav-legacy}"
mkdir -p "$DXVK_STATE_CACHE_PATH" "$MESA_SHADER_CACHE_DIR"
