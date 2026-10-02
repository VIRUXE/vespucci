# Status and development log

Newest first. The sections are dated and kept as written, so this file doubles as the project's log of what was learned when. For what the program is and how it is built, start with the [README](../README.md) and [architecture.md](architecture.md).

## Current state (2026-10-02)

- **Milestones M0–M4 done** on Linux headless; see [PLAN.md](../PLAN.md). `vespucci render` draws the streamed world through the game's own shaders with a preview lighting setup.
- **Next: M5** — a game-lit frame matched against RenderDoc captures of the real game (sky, time cycle, deferred lighting, post-processing). Captures wanted: A Vespucci Beach 18:30 EXTRASUNNY, B Legion Square 12:00 CLEAR, C sky, D Del Perro pier 21:30 CLEAR, E overcast — 1280×720, MSAA off, player hidden, time frozen.
- Then **M6**: the editor itself (Dear ImGui shell on Windows, MCP server on both platforms).
- Known rendering issues: downtown glass/reflective buildings render dark (forward techniques with stand-in reflections); distant SLOD pieces can appear to float when the LOD under them is outside `--radius` or in a different distance band; no sky, water, interiors or vehicles yet. Details in [rendering.md](rendering.md).
- **Native Windows verified (2026-10-02):** `doctor --gpu`, `render-model` and `render` run on the PC (RTX 3050, feature level 11.0), built natively with the MSYS2 UCRT64 toolchain; see [building.md](building.md). Development now happens there, with the Steam install.

## Development environment (where the numbers below come from)

| Item | Value |
|---|---|
| Game | Steam GTA V Legacy 1.0.3889.0, 121 GB, all archives NG-encrypted, 92 DLC packs |
| Machine | Intel i3-4130T (2 cores / 4 threads, AVX2), 5 GB RAM, no display; the Haswell iGPU only exposes Vulkan 1.2, which DXVK rejects, so everything renders on **lavapipe** (CPU) |
| DXVK-native | v3.1.1, SDL3 `offscreen` backend, installed to `/opt/dxvk-native` (see [building.md](building.md)) |
| Toolchain | Rust 1.95, bindgen with clang 21, `vkd3d-compiler` 2.1 for the helper HLSL, mingw-w64 (gcc 15) for the Windows target |

## Verified numbers (build 1.0.3889.0)

- `doctor --gpu` on lavapipe: device 0.23 s, first draw 0.10 s (pipeline JIT), cached draw 0.002 s.
- `GameFs`: 179 archives, 389,309 files, scan 0.8 s, 229 MB RSS. Keys recovered from the exe and cached.
- Shaders: 696 `.fxc` files / 21,374 DXBC blobs parse and reflect without failures; the current ones live in `update/update2.rpf/common/shaders/win32_40_final/` (321 files), originals in `common.rpf`.
- Index: 159,367 archetypes from 2,759 ytyps in 0.9 s; 11,082 ymaps (10,821 from `*_cache_y.dat`) in 0.3 s.
- Map sets: story mode 4,602 ymaps, online 5,425. Beach probe (story mode, 300 m): 171 maps, 21,380 entities, 3,702 models, 499 MB of model data referenced.
- Tests: `VESPUCCI_GAME=… cargo test --release` — game crate 2, world crate 3, fxc 1 (+1 with `VESPUCCI_FXC_DIR`); goldens in `scripts/golden.sh`.

## Update 2026-10-02 (night) — native Windows build on the PC (issue #23)

Built and run on the Windows PC (Windows 11, Steam GTA V Legacy 1.0.3889.0 at `C:\Program Files (x86)\Steam\steamapps\common\Grand Theft Auto V`, AMD Radeon iGPU + NVIDIA RTX 3050 Laptop, Rust 1.99 GNU, MSYS2 mingw-w64 14 / gcc 15, libclang 22).

What broke and what changed:

- **bindgen 0.71 produced opaque COM interfaces** (`pub struct ID3D11Device { _address: u8 }`, 46 "no field `lpVtbl`" errors) against MSYS2's headers with libclang 22; every `Vtbl` struct was fine, only the interface structs themselves lost their definition. Reproduced with a three-line header; bindgen 0.73 is correct. `vespucci-d3d11` now requires 0.73 (`prettyplease` updated with it for a single `syn`).
- **Adapter choice.** `D3D11CreateDevice(null, HARDWARE)` took adapter 0, the AMD iGPU. `Device::create` now enumerates with `IDXGIFactory1::EnumAdapters1` and picks the non-software adapter with the most dedicated video memory (the RTX), or whatever `VESPUCCI_ADAPTER` names. Headless Linux is unchanged: DXVK flags lavapipe as software, so the list yields nothing and the old call is made.
- **`peak_rss_mb` was 0** on Windows (`/proc/self/status`); it now reads the peak working set through `K32GetProcessMemoryInfo`, no new crate.
- Numbers on the RTX 3050: `doctor --gpu` device 0.07 s, draw 0.003 s; beach 640x360: stream 0.02 s, render 1.5 s, 3.3 s total, 526 MB peak; Legion Square 3.1 s, 492 MB. Keys recovered from the Steam exe and cached under `%USERPROFILE%\.cache\vespucci\keys\`.
- Goldens against the lavapipe images: `bag_unlit` 59.0 dB, `bag_lit` 60.4 dB, `barrier_lit` 35.0 dB, `world_beach` 31.5 dB, `world_legion` 33.9 dB. The renders are visually identical; the gap is hardware texture filtering and alpha-tested edges. `scripts/golden.sh` takes `WORLD_PSNR`/`MODEL_PSNR` for this (30 on a GPU) and runs from Git Bash.
- `cargo test --release` with `VESPUCCI_GAME` set: all pass (game 2, world 3, fxc 2).

## Update 2026-10-02 — M4 done (headless): streamed world

`vespucci render --pos X,Y,Z --look X,Y,Z [--fov 50] [--size 1280x720] [--radius 300] [--lod-scale 1] [--lighting basic|none] [--time 12:00] [--script-maps] [--max-draws N] [--budget-mb N] [--mip-skip 0,1,2] [--exposure 1] [--flip-sun] [--json] --out frame.png`
draws the streamed world around a camera through the game's own shaders. Exit test (`tests/out/m4_*.png`, goldens `tests/golden/world_{beach,legion}.png` at 640x360, PSNR ≥ 35 in `scripts/golden.sh`):

| scene | instances (culled) | models | draws | geometry + textures | peak RSS | time (warm) |
|---|---|---|---|---|---|---|
| Vespucci Beach `-1280,-1450,4 → -1200,-1500,4`, radius 300, 1280x720 | 964 (615) | 120 | 984 | 13 + 50 MB | 775 MB | 4.6 s (≈13 s cold, shader JIT) |
| Legion Square `195,-934,30 → 230,-900,28` | 1076 (816) | 89 | 877 | 11 + 42 MB | 769 MB | 7.4 s |

Pipeline (`crates/vespucci-world/src/streamer.rs`, `crates/vespucci-render/src/world_view.rs`, `crates/vespucci/src/render.rs`):
1. `MapSet` → `YmapTree::touching(camera, radius)` → entities parsed in parallel → LOD rule (`d < lodDist` and not hidden by nearer children) → `Instance` list sorted by distance.
2. Frustum cull by archetype sphere; models loaded on demand (`ydr`/`ydd`/`yft` by hash), textures from the archetype txd + `gtxd` parents + a model-named ytd + the shared `mapdetail`/`vehshare` dictionaries; drawable LOD by `lod_distances`; mip levels dropped by distance; geometry/texture byte budget.
3. Three passes with D3D11 state: opaque (render buckets 0,3,… ; depth write), decals (bucket 2; alpha blend, depth test only), alpha (bucket 1; blended, far-to-near). Bucket 3 materials prefer the `…CutOut_draw` techniques.
4. Reversed-Z `D32_FLOAT` depth (`Camera::proj_reversed`, clear 0, GREATER) — kills decal z-fighting. HDR `R16G16B16A16_FLOAT` target, tonemapped on the CPU (`tonemap.rs`: exposure → ACES → gamma 2.2; NaN/Inf → magenta and a warning).

Things learned (all empirical, see also `docs/binding.md`):
- **Script-only maps**: `CMapData.flags` bit 0 marks ymaps the game loads only on a script's request (`vb_29_day/night`, `*_reflproxy`/`*_reflection_lod` boxes, crime-scene props). They are skipped unless `--script-maps`. The giant grey "wall" in the first renders was `dt1_21_reflproxy` (a 6.7 km reflection-proxy box). Exception: time-of-day variants (`*_morning/_day/_evening/_night[_lod]`) are picked by `--time` with guessed hour bands (morning 06–11, day 12–17, evening 18–20, else night; missing variants fall back to `day`).
- **Time archetypes**: `CTimeArchetypeDef.timeFlags` bit h = visible at hour h; honoured via `--time` (vendored `rage-formats` exposes `time_flags` and `flags`).
- **Shadow proxies**: a `DrawableModel` whose `render_mask_flags` bit 0 is clear (e.g. `0xe2`) is shadow-only; drawing it put white vertex-coloured `cpv_only` meshes over the palm trunks. Visible models have `0xff`/`0xfd`.
- **Engine textures** without a material parameter (`ReflectionSampler`, `FogRaySampler`) get a flat dim-sky stand-in (`TextureCache::engine`), counted as "engine" not "missing". `FogRaySampler` is only read when `misc_globals` reg 19.y > 0.
- The pink blotches on the Vespucci plaza are real: `rsn_os_paintedamage` paint-splat decals.
- Shaders without any forward technique (`grass_fur_mask`, `trees_shadow_proxy` in unlit) are skipped and logged at debug level.
- Draw order groups by model pointer, so renders differ run-to-run by a few decal pixels (PSNR ≈ 60 dB); goldens use ≥ 35.

Debug switches (environment): `VESPUCCI_SKIP=name,…` (models by file name), `VESPUCCI_SKIP_SHADER=decal,…`, `VESPUCCI_SET_VAR=cb:var=a,b,c,d;…` (material variables), `VESPUCCI_SET_GLOBAL=cb:var=…` (preview globals), `VESPUCCI_ENGINE_TEX=r,g,b`, `VESPUCCI_MINLOD`/`VESPUCCI_MAXLOD` (sampler mip range), `VESPUCCI_PROBE=x,y` (print the HDR value of a pixel), `VESPUCCI_ALL_MODELS` (render-model: include shadow-only models); `--log trace` prints one line per drawn instance (name, distance, LOD, size, ymap, entity/archetype flags, time flags).

Not done in M4 (deferred to M5/M6): sky dome, real lighting/time-of-day/weather (the preview sun + ambient are constants), water, MLO interiors (`include_mlo_instances` false), vehicles/car generators, the desktop shell "flies around the streamed world" half of the milestone (Windows exe untested on the PC; it builds).

**Next: M5** — game-lit frame matched to RenderDoc captures (needs the user's captures A–E; see PLAN.md), then M6 editor + MCP.

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
- (Historical) first commit `f7de87e` holds M0–M3; M4 committed after it.

## 2026-10-02 (evening): missing objects and pink materials in the README renders

Committed (builds on both targets, no-GPU tests pass, world goldens re-frozen; verified on the beach-town camera `--pos=-1120,-1620,22 --look=-1280,-1450,4`). Development continues on the Windows PC from here (see building.md, "Developing natively on Windows").

- **Fixed: map entities with `lodDist` -1 were never drawn.** Half of all entities (16,184 of 32,585 here) store -1, meaning "use the archetype's lodDist". `streamer.rs` now falls back to the archetype; `StreamStats.lod_dist_from_archetype` counts them. Instances 1,328 -> 2,136 (parking meters, bins, lamps, fences, beach props appear).
- **Fixed: power lines.** `cable.fxc` projects with its own `gViewProj` and sizes with `gCableParams` (x = pixels per metre at 1 m = 0.5*height/tan(fov/2)); both live in the material cbuffer, so `Material` now has `frame_vars` filled from `Globals.frame` at bind time (`globals_preview::set_frame`). Cables render.
- **New diagnostic: `render --pick X,Y` and `--id-map FILE`.** A second pass draws every item with `shaders/id/id.hlsl` (writes the draw id to an R32_UINT target); `--pick` prints the exact model, geometry, material and textures under a pixel, `--id-map` writes one colour per draw.

Diagnosed, not yet fixed (each filed as an issue):

1. **Pink ladder/railings (`vb_30_ladder_05`, tint shaders).** Palette textures are 256x4; the VS samples at (COLOR0.b, tintPaletteSelector.x). Column 0 is the real colour, the rest is pink filler, and our anisotropic WRAP sampler blends column 0 with column 255 -> pink. Fix: point + clamp sampler for `TintPaletteSampler`, and set `tintPaletteSelector.x = (entity tint + 0.5) / rows` per draw (needs palette height from the texture cache).
2. **Holes in the ground and vanished prop clusters (the "floating grey boxes").** A LOD entity is hidden whenever `d < childLodDist`, but its HD children are often already beyond their own lodDist (childLodDist 299 vs archetype lodDist 60-100), so nothing draws. Implement the recursive rule: an entity draws iff `d < lodDist` and not (`numChildren > 0 && d < childLodDist && any child draws`); children resolve through `parent_index` (+ `flags` bit 3 = parent ymap). Open question: whether a -1 lodDist on an HD child should instead inherit the parent's childLodDist (check a cluster's children against its childLodDist once children lists exist).
3. **Cyan billboard on `prop_telegraph_02b` (and other cutout LODs).** `PS_Textured_Zero_CutOut` discards when `gAlphaRefVec0.x >= alpha`; `more_stuff.gAlphaRefVec0/1` are still 0 so nothing is discarded. Set them (0.5 to start) in `globals_preview`.
4. **75 NaN pixels (magenta bar) from `trees_lod` SLOD billboards.** The foliage PS divides by the Jacobian of the shadow-map coordinates (`dsx/dsy`); our dummy CSM transform is axis-aligned, so a camera-facing quad gets a zero determinant. Give the dummy CSM mapping a rotation with no zero entries (keep the 1e-3 scale).
5. **White SLOD buildings (`vb_27_slod_children`).** The texture `vb1_27_build_lod` in `vb_27_lod.ytd` is a normal brown facade atlas, so the white comes from shading/sampling, not the texture. Next step: `--pick` the face, then bisect with `VESPUCCI_SET_VAR`/`VESPUCCI_ONES`.
6. Glued texture names such as `prop_telegraph_02_lodprop_traffic_01_lod_a` are the game's own names (present in the .ytd), not a parser bug.
7. The entity trace now prints the tint value; adding the entity index would make parent/child checks easier for item 2.
