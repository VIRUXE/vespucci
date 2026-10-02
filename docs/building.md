# Building and running

Vespucci is a Rust workspace. It renders through Direct3D 11: natively on Windows, and on Linux through [DXVK-native](https://github.com/doitsujin/dxvk) (D3D11 implemented on Vulkan, no Wine) with the lavapipe CPU driver or any Vulkan 1.3 GPU. Nothing from the game ships with the program; you point it at your own install.

## Requirements

| | Linux (headless) | Windows |
|---|---|---|
| Rust | stable (1.75 or newer; developed on 1.95) | same, GNU host toolchain (`stable-x86_64-pc-windows-gnu`), or cross-compiled from Linux with mingw-w64 |
| Graphics | Vulkan 1.3 driver. lavapipe (`mesa-vulkan-drivers`) works on any CPU; this is what the project is developed on | Direct3D 11 (any GPU) |
| DXVK-native | v3.1.1 built with the SDL3 backend (script below) | not needed |
| bindgen | `clang` + `libclang-dev` (generates the D3D11 FFI at build time) | MSYS2's `mingw-w64-ucrt-x86_64-clang` (libclang) and the UCRT64 toolchain headers; bindgen 0.73 or newer (0.71 emits every COM interface as an opaque struct against mingw-w64 14 / libclang 22) |
| Helper shaders | `vkd3d-compiler` (only to rebuild `shaders/*.hlsl`; the compiled DXBC is committed) | — |
| Game | GTA V **Legacy** PC build (Steam/Rockstar/Epic), any version with NG-encrypted archives. Developed against 1.0.3889.0 | same |
| Memory | 4 GB RAM is enough for 1280x720 renders (peak RSS ~0.8 GB) | — |

The game directory is the one holding `GTA5.exe`, `common.rpf`, `x64a.rpf`… and `update/`. It is passed with `--game DIR` or `GTAV_PATH`.

## Linux

### One-time setup

```sh
scripts/setup-linux.sh
```

The script installs the apt packages, Rust if missing, builds DXVK-native v3.1.1 into `/opt/dxvk-native` (D3D11 + DXGI only, SDL3 window backend) and registers its library directory with the dynamic loader. It is idempotent. Override `DXVK_PREFIX`, `DXVK_VERSION`, `DXVK_SRC`, `JOBS`, or set `SKIP_WINDOWS=1` to leave out the mingw cross toolchain.

What it does by hand, if you prefer:

```sh
sudo apt-get install build-essential clang libclang-dev pkg-config git meson ninja-build \
    glslang-tools libsdl3-dev libvulkan1 vulkan-tools mesa-vulkan-drivers vkd3d-compiler
git clone --recursive --branch v3.1.1 --depth 1 https://github.com/doitsujin/dxvk.git ~/src/dxvk-native
cd ~/src/dxvk-native
meson setup build.native --buildtype release --prefix /opt/dxvk-native --libdir lib/x86_64-linux-gnu \
    -Denable_d3d8=false -Denable_d3d9=false -Denable_d3d10=false \
    -Dnative_sdl3=enabled -Dnative_sdl2=disabled -Dnative_glfw=disabled
ninja -C build.native -j3 && sudo ninja -C build.native install
echo /opt/dxvk-native/lib/x86_64-linux-gnu | sudo tee /etc/ld.so.conf.d/dxvk-native.conf && sudo ldconfig
```

Why SDL3: DXVK needs a window-system backend even when nothing is shown, and SDL3 has an `offscreen` video driver. Why the `ld.so.conf` entry: `libdxvk_dxgi.so` is loaded by `libdxvk_d3d11.so`, and `RUNPATH` is not transitive, so the loader needs the directory (the binary itself carries a `DT_RPATH` from `.cargo/config.toml`).

### Build

```sh
cargo build --release
```

`crates/vespucci-d3d11/build.rs` runs bindgen over DXVK's `d3d11.h`/`dxgi.h` from `/opt/dxvk-native/include` (or `$DXVK_NATIVE_PREFIX/include`) and links `dxvk_d3d11` and `dxvk_dxgi`. A full release build takes a few minutes; incremental builds are quick.

### Run

```sh
export GTAV_PATH=/path/to/gtav
./target/release/vespucci doctor --gpu --out /tmp/test.png
./target/release/vespucci render --pos=-1280,-1450,4 --look=-1200,-1500,4 --out beach.png
```

The binary sets its own environment for headless use (`DXVK_WSI_DRIVER=SDL3`, `SDL_VIDEO_DRIVER=offscreen`, `VK_DRIVER_FILES` pointing at lavapipe, `DXVK_FILTER_DEVICE_NAME=llvmpipe`, quiet DXVK logging, shader caches under `~/.cache/vespucci/`) without overriding variables you already exported. To run other tools under the same conditions, `. scripts/vespucci-env.sh`.

To use a real GPU instead of lavapipe, export `VK_DRIVER_FILES` for its ICD (or unset it to let the loader pick) and `DXVK_FILTER_DEVICE_NAME` to a substring of the device name. DXVK requires Vulkan 1.3; older Intel iGPUs (Haswell exposes 1.2) are rejected, which is why lavapipe is the default.

### First run and caches

- **Keys**: the first `vespucci` command derives the archive keys from your `GTA5.exe` (SHA-1 search for the AES key; NG tables from the `rpf-archive` crate's data) and caches them in `~/.cache/vespucci/keys-<sha256 of the exe>.bin`.
- **Shader JIT**: lavapipe compiles every D3D11 pipeline on first use. A cold world render costs ~10 s more than a warm one; DXVK's state cache and Mesa's shader cache (both under `~/.cache/vespucci/`) make later runs fast.
- **World index**: archetypes and the ymap tree are built from the game files on every run (about 1 s); there is no on-disk index yet.

### Tests and goldens

```sh
cargo test --release -p vespucci-game -p vespucci-fxc -p vespucci-world   # no game needed (game-file tests skip)
VESPUCCI_GAME=$GTAV_PATH cargo test --release                            # includes game-file tests
scripts/golden.sh            # renders tests/golden/*.png and compares (PSNR >= 40 models, >= 35 world)
scripts/golden.sh --update   # re-freeze after an intentional rendering change
```

lavapipe is deterministic, so model renders match their goldens bit for bit. World renders group draws by model address and can differ by a few decal pixels between runs (~60 dB), which the threshold allows.

## Windows

The supported path is cross-compiling from Linux with mingw-w64 (the setup script installs it):

```sh
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
# -> target/x86_64-pc-windows-gnu/release/vespucci.exe
```

On Windows the same binary uses the system `d3d11.dll`/`dxgi.dll`, so DXVK is not involved and any GPU works. `vespucci.exe doctor --gpu` draws a test frame and reports the adapter.

### Developing natively on Windows

The build needs the GNU toolchain (the D3D11 bindings are generated from mingw-w64's headers; MSVC is not supported) and a libclang for bindgen. This is the setup the desktop work (M6) is done on; it was verified on 2026-10-02 (Windows 11, MSYS2 2026-06-11, Rust 1.99 GNU, RTX 3050 + AMD iGPU).

1. Install [MSYS2](https://www.msys2.org/) (`winget install MSYS2.MSYS2`) and, in its UCRT64 shell: `pacman -S mingw-w64-ucrt-x86_64-toolchain mingw-w64-ucrt-x86_64-clang`. The toolchain provides `gcc`, `x86_64-w64-mingw32-gcc` and the headers in `C:\msys64\ucrt64\include`, which `build.rs` finds by itself; the clang package provides `libclang.dll` for bindgen (the official LLVM installer works too; its `libclang` is the same major version).
2. Install Rust with [rustup](https://rustup.rs/) and add the GNU host toolchain: `rustup toolchain install stable-x86_64-pc-windows-gnu` and either `rustup default stable-x86_64-pc-windows-gnu` or, to leave other projects on MSVC, `rustup override set stable-x86_64-pc-windows-gnu` inside the checkout.
3. Put `C:\msys64\ucrt64\bin` on `PATH` (for the linker named in `.cargo/config.toml`) and point bindgen at libclang, then build:

```powershell
$env:PATH = "C:\msys64\ucrt64\bin;$env:PATH"
$env:LIBCLANG_PATH = "C:\msys64\ucrt64\bin"
$env:GTAV_PATH = "C:\Program Files (x86)\Steam\steamapps\common\Grand Theft Auto V"
cargo build --release
.\target\release\vespucci.exe doctor --gpu --out test.png
.\target\release\vespucci.exe render --pos=-1280,-1450,4 --look=-1200,-1500,4 --out beach.png
```

Notes from that first run:

- **bindgen must be 0.73 or newer.** With mingw-w64 14 headers and libclang 22, bindgen 0.71 generates every COM interface (`ID3D11Device`, `IUnknown`, …) as an opaque `{ _address: u8 }` struct, so `com_call!` fails with "no field `lpVtbl`". `Cargo.lock` pins 0.73; do not downgrade.
- **GPU choice.** D3D11's default adapter on a laptop is often the iGPU. The device is created on the hardware adapter with the most dedicated video memory; `VESPUCCI_ADAPTER=<index or name substring>` overrides, and `doctor --gpu --log debug` lists what DXGI enumerates.
- **Goldens.** `tests/golden/*.png` are lavapipe renders. On hardware the bag goldens match (59-60 dB) but the barrier (thin cutout geometry) lands at 35 dB and the world scenes at 31-34 dB, because anisotropic filtering, mip selection and alpha-tested edges differ between a software and a hardware rasteriser. `WORLD_PSNR=30 MODEL_PSNR=30 scripts/golden.sh` from Git Bash passes; a separate hardware golden set is not kept.
- `cargo test --release` passes (with `VESPUCCI_GAME` set, the game-file tests too). `rustfmt` is a separate component on the GNU toolchain: `rustup component add rustfmt --toolchain stable-x86_64-pc-windows-gnu`.
- Timing on the RTX 3050: device 0.05-0.8 s, beach render 1.5 s, whole command 3.3 s; peak working set 526 MB (reported as `peak_rss_mb`, measured with `GetProcessMemoryInfo`).

If bindgen cannot find `d3d11.h`, set `VESPUCCI_D3D_INCLUDE` to the directory that holds it (`;`-separated if several). The helper shaders under `shaders/` are committed as DXBC, so `vkd3d-compiler` is not needed on Windows. `scripts/setup-linux.sh` is Linux-only.

## Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `error while loading shared libraries: libdxvk_d3d11.so.0` | DXVK-native not installed where `.cargo/config.toml` expects (`/opt/dxvk-native/lib/x86_64-linux-gnu`), or `ldconfig` not run. |
| `doctor` says no suitable Vulkan device | DXVK needs Vulkan 1.3. Install `mesa-vulkan-drivers` for lavapipe; check `VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json vulkaninfo --summary`. |
| `game directory needed: --game DIR or $GTAV_PATH` | Point it at the folder containing `GTA5.exe`. |
| Keys fail to derive | The exe must be the PC Legacy build (unpacked, not the Enhanced edition). `doctor` prints the build it found. |
| bindgen cannot find `d3d11.h` | Linux: DXVK headers missing from the prefix. Windows target: `gcc-mingw-w64-x86-64` not installed. |
| Everything renders black or magenta | Read [rendering.md](rendering.md) and [binding.md](binding.md); the debug switches in [debugging.md](debugging.md) bisect it quickly. |
| Out of memory on large radii | Lower `--radius`, `--budget-mb`, `--max-draws`, or raise `--mip-skip`. lavapipe keeps all GPU resources in RAM. |

## Repository bootstrap

`scripts/github-bootstrap.sh` creates the labels, the milestones (M5, M6, After MVP) and the initial development issues on the GitHub repository with the `gh` CLI. It is idempotent: labels are updated in place, milestones and issues that already exist (by title) are skipped, so it is safe to re-run after adding new entries.
