# Vespucci — status and how to resume

Written 2026-09-30 when work was paused mid-M3. Read this first, then `PLAN.md`.

## What Vespucci is (decisions so far)

- An Ariane-style **desktop map editor for GTA V (Legacy PC)** with a Dear ImGui interface (imgui-rs), that **also runs headless** (CLI now, MCP server later). MCP server ships on **both** Windows and Linux; on Windows it can attach to the running editor.
- **Desktop shell = Windows** (user's PC with the game + GPU, native D3D11, no DXVK). **Linux = headless only** (this server: no display, Haswell iGPU rejected by DXVK → lavapipe CPU rendering).
- **Rust** workspace, building on VIRUXE's public-domain crates `rpf-archive`, `rage-formats`, `rage-render` (from rage-cli). CodeWalker is reference only (no licence). Inputs are retail game files only; the leaked source is out of scope and stays out.
- Name "Vespucci" (no Rockstar trademarks anywhere).

## Environment (this server)

| Item | Value |
|---|---|
| Project | `/root/vespucci` (git initialised, branch `main`, **nothing committed yet** — user never asked for commits) |
| Game | `/root/gtav-legacy` — Steam Legacy 1.0.3889.0, 121 GB, all RPFs NG-encrypted, 92 DLC packs. Set `GTAV_PATH=/root/gtav-legacy` |
| Machine | i3-4130T (2c/4t AVX2), 5 GB RAM, 18 GB free disk, no X/Wayland, SSH only |
| DXVK-native | v3.1.1 built with SDL3 (`offscreen` WSI) → `/opt/dxvk-native/lib/x86_64-linux-gnu`, registered via `/etc/ld.so.conf.d/dxvk-native.conf`; source tree `/root/src/dxvk-native` |
| Toolchain | Rust 1.95, bindgen (clang 21), `vkd3d-compiler` (HLSL→DXBC on Linux), mingw-w64 + target `x86_64-pc-windows-gnu` for the Windows build |
| Caches | `~/.cache/vespucci/{keys,dxvk,mesa}`, `~/.cache/dxvk` |
| Reference sources | `/root/src/{rage-cli,rage-formats,rpf-archive-rs,rage-render}` (public domain); CodeWalker files fetched to `/tmp/FxcFile.cs`, `/tmp/GameFileCache.cs` (temporary) |
| Windows exe | `/srv/samba/share/vespucci/vespucci.exe` (release build of M1 state). **User has not yet run `vespucci.exe doctor --gpu` on the PC** — that is the open half of M1. |

Env for headless runs is set by the binary itself (`vespucci_d3d11::setup_env`); `scripts/vespucci-env.sh` mirrors it for other tools.

## Workspace

```
crates/vespucci-d3d11   FFI over DXVK-native / mingw d3d11.h (bindgen, C vtables) + safe wrappers   ✅ M0 verified (Linux + Windows cross-build)
crates/vespucci-game    keys from GTA5.exe (cached), mmap archives, DLC load order, GameFs flat file table   ✅ M1 (Linux)
crates/vespucci-fxc     .fxc container + DXBC RDEF/ISGN reflection   ✅ M2 (696 files / 21,374 blobs, 0 failures)
crates/vespucci-world   ArchetypeDb, YmapTree (cache_y.dat), entity reader (LOD fields, car gens), MapSet (SP/MP change sets)   ✅ M2
crates/vespucci-render  ⚠️ INCOMPLETE — see "Where work stopped"
crates/vespucci         CLI: doctor, ls, cat, find, index, probe, shader
docs/binding.md         shader-binding facts (read before touching render)
docs/THIRD_PARTY.md     provenance of carried-over code
shaders/m0/tri.hlsl + .dxbc   M0 triangle (vkd3d-compiler output, committed as bytes)
tests/out/              scratch outputs (gitignored): extracted fxc files, setup2.xml of all packs, mpheist content.xml
```

`.cargo/config.toml`: Linux rpath flags (DT_RPATH via `--disable-new-dtags`) and the mingw linker for Windows.

## Verified numbers (build 1.0.3889.0)

- `doctor --gpu` on lavapipe: device 0.23 s, first draw 0.10 s (JIT), cached 0.002 s, pixels correct.
- GameFs: 179 archives, 389,309 files, scan 0.8 s, 229 MB RSS. Keys recovered from the exe (AES key by SHA-1 search; NG keys via rpf-archive's bundled magic data).
- Shaders: current ones live in `update/update2.rpf/common/shaders/win32_40_final/` (321), originals in `common.rpf/...`; `_lq_` and `_nvstereo_` variants exist → lookup is **path-based** (`vespucci_fxc::SHADER_DIRS`).
- Index: 159,367 archetypes / 2,759 ytyps in 0.9 s; 11,082 ymaps (10,821 from `*_cache_y.dat`) in 0.3 s.
- Map sets: story mode 4,602 ymaps, online 5,425; archetypes unaffected. Beach probe (SP, 300 m): 171 maps, 21,380 entities, 3,702 models, 499 MB model data.
- Tests: `VESPUCCI_GAME=/root/gtav-legacy cargo test --release` — game crate 2, world crate 3, fxc unit 1 (+1 with `VESPUCCI_FXC_DIR=tests/out/fxc`). All passing as of the last run before M3 edits.

## Commands that work

```
export GTAV_PATH=/root/gtav-legacy
V=./target/release/vespucci
$V doctor --gpu --out /tmp/m0.png
$V ls common.rpf/shaders/win32_40_final | wc -l
$V cat update/update.rpf/common/data/dlclist.xml | head
$V find 'prop_cs_heist_bag*' -l ;  $V find --ext ydr --name prop_cs_heist_bag_01
$V --mapset sp index ;  $V --mapset mp index ;  $V --mapset all index
$V probe --pos=-1280,-1450,4 --radius 300          # story mode by default
$V shader normal_spec [--vars] [--reflect] [--blob vs:6 --out x.dxbc] ;  $V shader --all
cargo build --release --target x86_64-pc-windows-gnu && cp target/x86_64-pc-windows-gnu/release/vespucci.exe /srv/samba/share/vespucci/
```

## Update 2026-10-02 — M3 done (headless)

`vespucci render-model NAME [--technique T] [--ytd …] [--lod …] [--size WxH] [--yaw/--pitch] [--out x.png] [--dump-binding b.json]` draws a drawable through the game's own shaders on lavapipe:
- `unlit_draw` (default): texture × vertex colour, exactly what the shader computes (vertex colours are baked masks, so props look tinted — by design).
- `lightweightHighQuality0_draw` / `draw`: forward-lit with preview sun/ambient; bump + spec maps bound; material cbuffer params all matched. Needs the preview globals listed in `docs/binding.md` (fog NaN trap, `globalScalars3.z`, AO effect, cascade vars) — those were the whole debugging story.
- Goldens: `tests/golden/{bag_unlit,bag_lit,barrier_lit}.png`, checked by `scripts/golden.sh` (PSNR ≥ 40). `vespucci compare a.png b.png [--min-psnr N]`, `vespucci texture NAME --out DIR` (dump textures as PNG) added.
- Verified conventions: matrices uploaded as glam `to_cols_array()` untransposed; shader-param hashes match RDEF names; `vkd3d-compiler -b d3d-asm` disassembles game blobs (used to find every black-output cause).
- The Windows exe on the share now includes `render-model` (still untested on the PC).

**Next: M4** — streamed world: `vespucci-world` streamer (ymaps touching the frustum → entities → LOD rule → archetype → model file), `render --pos --look --fov --lighting basic`, resource budgets (lavapipe keeps everything in RAM), draw list sorted by shader, Vespucci Beach exit test (RSS < 3.5 GB). The per-model path in `model_view.rs` becomes the per-instance path (world matrix from entity position/orientation/scale).

The section below describes the state at the earlier pause and is kept for history.

## Where work stopped on 2026-09-30 (M3: one model through the game's `unlit_draw` shader)

Done in this session, **not yet compiled together**:

1. `crates/vespucci-d3d11/src/lib.rs` was extended (via a script) with: `InputElement::new` (owned `CString` semantic), `DepthTarget`, `update_buffer` (Map/discard), `create_texture2d_mips`, `create_srv`, `create_sampler`, `create_depth_target`, `create_rasterizer`, `bind_targets`, `clear_depth`, `set_rasterizer`, `set_pipeline`, `set_vertex_buffer`, `set_index_buffer`, `set_constant_buffer`, `set_shader_resource`, `set_sampler`, `draw_indexed`, and `enum Stage {Vertex, Pixel}`. `doctor.rs` was updated to `InputElement::new(...)`. `cargo build -p vespucci-d3d11 -p vespucci` **succeeded** with these changes (exit 0, confirmed after the stop). Still needed by the render crate but not yet in the wrapper: `ComPtr::clone_ptr()` (AddRef + wrap).
2. `crates/vespucci-render/` has `Cargo.toml`, `shader_cache.rs`, `layout.rs`, `cbuffer.rs`, `texture.rs`, `material.rs`. **Missing: `src/lib.rs`, `src/camera.rs`, `src/model_view.rs`** (their writes were declined when work was stopped). ⚠️ Because `Cargo.toml` lists `members = ["crates/*"]`, the workspace **will not build** until either those files exist or the crate is excluded (`exclude = ["crates/vespucci-render"]`, or rename the dir) — do that first if you just want the working CLI back.
3. Not started: `render-model` CLI subcommand; wiring `vespucci-render` into `crates/vespucci/Cargo.toml`.

Design of the missing pieces (all settled, see `docs/binding.md`):
- `camera.rs`: `Camera { position, target, fov_deg, aspect, near, far }`, Z-up, `Mat4::look_at_rh` / `perspective_rh` (glam 0.30), `orbit(centre, radius, aspect, yaw, pitch)` auto-framing; `pack(m, transpose)` → `[f32;16]`. Rage shaders use `mul(pos, gWorldViewProj)` (row vectors) so the default is **transpose = true**; if the model renders skewed/garbage, flip it (`--no-transpose`).
- `model_view.rs`: `render_drawable(dev, fs, shaders, drawable, extra_textures, opts) -> (rgba8, report)`: RT + depth + rasterizer (cull none by default) + aniso-16 wrap sampler; textures = drawable's embedded ytd + extra ytds by name hash; one `Material` per `ShaderFx` (technique candidates `["unlit_draw", "draw"]`, fallback first existing); geometry of chosen LOD: vertex buffer uploaded raw (`VertexBuffer.data`, stride `vertex_stride`), indices repacked to u16 when < 65536; input layout via `LayoutCache` (dummy slot 1, stride 0, 64 zero bytes for ISGN inputs the mesh lacks); globals: `rage_matrices` gWorld/gWorldView/gWorldViewProj/gViewInverse, `misc_globals.globalScreenSize`; then clear, bind, draw all, read back. Report JSON: per material cbuffers/textures found, params applied/unmatched, dummies.
- Shader-param name hashes: `ShaderFx.parameters[].name_hash` — unknown whether it is `joaat(name)` or `joaat(lowercase)`; `CBufferBlock` indexes both, `material.rs` tries both. Check `params_unmatched` in the report on the first run.
- Textures: `texture.rs::upload` uses `rage_formats::to_dds_layout` then slices mips with `level_size`; formats mapped to DXGI (BC1/2/3/4/5/7, B8G8R8A8, R8G8B8A8, B5G5R5A1, A8, R8). sRGB left for M5.
- Exit test for M3: `render-model --model prop_cs_heist_bag_01 --out bag.png` shows the textured bag (no magenta), `--dump-binding` shows zero unbound cbuffers/textures; freeze a golden PNG (PSNR ≥ 40 on rerun; lavapipe is deterministic).

## After M3 (unchanged plan)

M4 streamed world (`render --lighting basic`, Vespucci Beach, RSS < 3.5 GB) → M5 game-lit frame vs RenderDoc captures (user has a Windows PC and agreed to capture; checklist in the approved plan: A beach 18:30 EXTRASUNNY, B Legion Square 12:00 CLEAR, C sky, D Del Perro 21:30, E overcast) → M6 editor (ImGui shell on Windows, MCP on both).

## Open items / risks

- Windows: `vespucci.exe doctor --gpu` untested on the PC (validates native D3D11 + key recovery + scan there).
- `vespucci-render` incomplete (above). Workspace build currently broken until resolved.
- Matrix packing convention and shader-param hash case: decided empirically on the first `render-model` run.
- Vertex declarations with `Unknown` semantics/types are skipped by `layout.rs`; watch the skipped-geometry count.
- Memory: lavapipe keeps GPU resources in RAM; M4 must budget (~0.5 GB geometry, 0.8 GB textures, mip trimming by distance).
- Nothing committed to git. Suggested first commit: everything except `tests/out/`, `target/` (already gitignored).
