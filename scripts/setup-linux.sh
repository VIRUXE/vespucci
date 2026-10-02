#!/usr/bin/env bash
# One-time setup of a Debian/Ubuntu machine for building and running Vespucci
# headless: build tools, DXVK-native (D3D11 on Vulkan, no Wine), the lavapipe
# CPU Vulkan driver, the HLSL compiler used for the helper shaders, and the
# mingw-w64 cross toolchain for the Windows build.
#
# Idempotent: re-running skips what is already there. Needs sudo for apt and
# for installing DXVK-native under /opt. Takes 15-30 minutes on a small box,
# almost all of it compiling DXVK.
#
#   scripts/setup-linux.sh                 # everything
#   DXVK_PREFIX=/usr/local scripts/setup-linux.sh
#   SKIP_WINDOWS=1 scripts/setup-linux.sh  # no mingw-w64
set -euo pipefail

DXVK_VERSION="${DXVK_VERSION:-v3.1.1}"
DXVK_PREFIX="${DXVK_PREFIX:-/opt/dxvk-native}"
DXVK_SRC="${DXVK_SRC:-$HOME/src/dxvk-native}"
JOBS="${JOBS:-$(( $(nproc) > 3 ? 3 : $(nproc) ))}"   # DXVK's C++ is memory-hungry; 3 jobs fit in ~4 GB

SUDO=""; [ "$(id -u)" -ne 0 ] && SUDO=sudo

echo "== apt packages"
pkgs=(build-essential clang libclang-dev pkg-config git curl ca-certificates
      meson ninja-build glslang-tools libsdl3-dev
      libvulkan1 vulkan-tools mesa-vulkan-drivers
      vkd3d-compiler)
[ -z "${SKIP_WINDOWS:-}" ] && pkgs+=(gcc-mingw-w64-x86-64)
$SUDO apt-get update
$SUDO apt-get install -y --no-install-recommends "${pkgs[@]}"

echo "== Rust"
if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
  # shellcheck disable=SC1091
  . "$HOME/.cargo/env"
fi
rustup component add rustfmt >/dev/null 2>&1 || true
[ -z "${SKIP_WINDOWS:-}" ] && rustup target add x86_64-pc-windows-gnu

echo "== DXVK-native $DXVK_VERSION -> $DXVK_PREFIX"
if [ ! -e "$DXVK_PREFIX/lib/x86_64-linux-gnu/libdxvk_d3d11.so" ]; then
  if [ ! -d "$DXVK_SRC" ]; then
    git clone --recursive --branch "$DXVK_VERSION" --depth 1 https://github.com/doitsujin/dxvk.git "$DXVK_SRC"
  fi
  cd "$DXVK_SRC"
  # Only D3D11 + DXGI, with SDL3 as the window-system backend: its "offscreen"
  # video driver lets DXVK run with no display at all.
  meson setup build.native --buildtype release --prefix "$DXVK_PREFIX" --libdir lib/x86_64-linux-gnu \
    -Denable_d3d8=false -Denable_d3d9=false -Denable_d3d10=false \
    -Dnative_sdl3=enabled -Dnative_sdl2=disabled -Dnative_glfw=disabled
  ninja -C build.native -j"$JOBS"
  $SUDO ninja -C build.native install
  rm -rf build.native   # the object tree is several GB
  cd - >/dev/null
fi
# The dxgi library is a transitive dependency without an rpath of its own, so
# the loader must know the directory.
echo "$DXVK_PREFIX/lib/x86_64-linux-gnu" | $SUDO tee /etc/ld.so.conf.d/dxvk-native.conf >/dev/null
$SUDO ldconfig

echo "== check"
VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json vulkaninfo --summary 2>/dev/null | grep -E "deviceName|apiVersion" | head -2 || echo "vulkaninfo: lavapipe not reported (is mesa-vulkan-drivers installed?)"
ls "$DXVK_PREFIX/lib/x86_64-linux-gnu/" | grep -E "libdxvk_(d3d11|dxgi)\.so$"
echo
echo "Done. Next:"
echo "  cargo build --release"
echo "  export GTAV_PATH=/path/to/your/GTA V"
echo "  ./target/release/vespucci doctor --gpu"
