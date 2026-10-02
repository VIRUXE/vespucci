# Vespucci — plan

A map editor for GTA V (Legacy PC) in the spirit of Ariane: a desktop application with a Dear ImGui interface that shows the world the way the game draws it, using the game's own compiled shaders. It also runs **headless** — no window — driven by a CLI and an MCP server, which is the mode used on Linux servers and by AI agents.

**Shape:** one Rust codebase, one render core, two shells.

| Shell | Platform | Graphics | Purpose |
|---|---|---|---|
| **Desktop** | Windows (the user's PC with the game and a GPU) | native D3D11, SDL3 window, imgui-rs with our own D3D11 backend | the editor people use: viewport, object browser, gizmos, prefabs, save |
| **Headless** | Linux (this server, and any Linux box) | D3D11 via DXVK-native on Vulkan (lavapipe here) | `vespucci render …` to PNG, tests/CI |

**MCP server on both.** `vespucci serve` runs on Windows and Linux. Headless it owns the scene itself; on Windows it can also attach to the running desktop editor so an agent inspects and edits the same open map alongside the user (Ariane's Agent Alpha model). The tool set is identical in both cases: find/place/move/rotate/delete, undo, camera, time/weather, capture, save.

The Windows build is cross-compiled on this server (mingw-w64 + `x86_64-pc-windows-gnu`) and copied to the PC; nothing needs installing there.

**MVP (render core):** `vespucci render --game DIR --pos X,Y,Z --look X,Y,Z --fov 50 --time 18:30 --weather EXTRASUNNY --size 1280x720 --out frame.png` matches how the game draws that spot. The desktop shell becomes usable as soon as there is something to look at (M3) and grows alongside.

## Ground rules

- Input is only the retail game install and its own data files. No leaked source, no decompiled game code. No FiveM or in-game hooks.
- Format handling comes from VIRUXE's public-domain (Unlicense) Rust crates `rpf-archive`, `rage-formats` and `rage-render` (the libraries behind `rage-cli`). CodeWalker is reference material only (no licence). RenderDoc captures of the retail game are the spec for the frame pipeline.
- RPF keys are derived from the user's own `GTA5.exe`.
- No Rockstar trademarks in the product name, binary name or tool names.

## Environment

| Item | Value |
|---|---|
| Game install (server) | `/root/gtav-legacy` (Steam Legacy edition, build 1.0.3889.0, 121 GB; all RPFs NG-encrypted; 92 DLC packs) |
| Server | Intel i3-4130T (2c/4t, AVX2), 5 GB RAM + 3 GB swap, 18 GB free disk, **no display**, SSH only. Haswell iGPU exposes Vulkan 1.2 only and is rejected by DXVK → headless rendering runs on **lavapipe** (CPU). Fine for captures and tests, not interactive. |
| DXVK-native | v3.1.1 at `/opt/dxvk-native/lib/x86_64-linux-gnu` (SDL3 `offscreen` WSI), registered in `ld.so.conf.d`. M0 verified: device 0.23 s, first draw 0.10 s, second draw 0.002 s. |
| Toolchain | Rust 1.95 (cargo workspace), bindgen over DXVK's/mingw's `d3d11.h`, `vkd3d-compiler` for our few helper HLSL shaders, mingw-w64 for the Windows build |
| Project dir | `/root/vespucci` |

## Workspace

```
crates/vespucci-d3d11    bindgen FFI (Linux: DXVK-native headers + libs; Windows: mingw d3d11/dxgi) + safe wrappers: device, swapchain, textures, buffers, shaders, readback
crates/vespucci-fxc      .fxc container parser + DXBC chunk parser (RDEF, ISGN/OSGN)
crates/vespucci-world    game index (from rage-cli's, public domain), archetype DB, ymap tree, streaming, LOD, scene, camera
crates/vespucci-render   shader cache, material binding, GPU resources, frame passes, image compare
crates/vespucci-edit     selection, transforms, undo/redo, prefabs, save to mod folder (via rage-formats writers)
crates/vespucci-ui       SDL3 window, imgui-rs + our D3D11 backend, ImGuizmo, editor panels          (Windows desktop shell)
crates/vespucci          CLI: render, doctor, ls/cat/find, dump, shader, probe; `serve` (MCP); `edit` launches the desktop shell on Windows
config/  shaders/  tests/  captures/  docs/
```

Dependencies: `rpf-archive`, `rage-formats`, `rage-render` (comparison/fallback rasteriser only), `clap`, `anyhow`, `memmap2`, `rayon`, `png`, `serde_json`, `imgui` (+ `imgui-sys`), `sdl3`.

## Milestones

| # | Milestone | Done when |
|---|---|---|
| M0 ✅ | DXVK-native headless spike from Rust | `vespucci doctor --gpu` on lavapipe, no display: triangle drawn, read back, PNG written, JIT timed. |
| M1 ✅ (Linux) | Game access through the crates; Windows cross-build | `ls`/`cat`/`find` over all RPFs (nested + DLC order); keys from the exe, cached. `cargo build --target x86_64-pc-windows-gnu` produces a `vespucci.exe` whose `doctor --gpu` passes on the PC on native D3D11. |
| M2 ✅ | Shaders + index | `.fxc` and DXBC parsers (696 files / 21,374 blobs parse); `shader NAME --reflect`; archetype DB (159k) + ymap tree (from cache files) + story/online map sets from DLC change sets; `index`, `probe`. |
| M3 ✅ (headless) | One model via a game `.fxc`; first window | `render-model` draws a textured prop with the game's own shader, zero unbound inputs. Desktop shell opens a window on the PC with that model in the viewport, orbit camera, ImGui panels. |
| M4 ✅ (headless) | Streamed world | `render --lighting basic` at Vespucci Beach, RSS <3.5 GB headless (775 MB, 5 s warm); desktop shell flies around the streamed world. |
| M5 | Game-lit frame vs RenderDoc capture | Per-pass thresholds pass against the user's capture A. **Render core MVP.** |
| M6 | Editor | Ariane's core loop: select, move/rotate/scale with gizmos, snapping, undo/redo, object browser with previews, save to a mod folder; same operations exposed over MCP in headless mode. |

After that: vehicles (`.yft` + carcols) and car generators, prefabs, water/path editing, exact-look refinements, web viewer for the headless server.

Detailed design notes live in `docs/`. **Current state and how to resume: `docs/STATUS.md`** (M4 done 2026-10-02).
