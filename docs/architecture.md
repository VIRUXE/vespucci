# Architecture

Vespucci is a Rust workspace of six crates plus one vendored dependency. The dependency direction is strictly downward: the CLI depends on everything, the render crate on the world and game crates, and nothing depends on the CLI.

```
                       ┌──────────────────────┐
                       │  vespucci (CLI)      │  clap subcommands; later the MCP server and the desktop shell
                       └──────────┬───────────┘
            ┌─────────────────────┼─────────────────────┐
┌───────────▼───────────┐ ┌───────▼────────┐ ┌──────────▼──────────┐
│  vespucci-render      │ │ vespucci-world │ │ vespucci-fxc        │
│  shaders, materials,  │ │ archetypes,    │ │ .fxc containers,    │
│  textures, frames     │ │ ymaps, LOD,    │ │ DXBC reflection     │
└───┬───────────┬───────┘ │ streaming      │ └──────────┬──────────┘
    │           │         └───────┬────────┘            │
┌───▼────────┐  │                 │                     │
│ vespucci-  │  │         ┌───────▼─────────────────────▼───────┐
│ d3d11      │  └─────────►  vespucci-game                      │
│ (FFI)      │            │  keys, RPF archives, load order, VFS │
└────────────┘            └───────────────┬──────────────────────┘
                                          │
                     ┌────────────────────▼─────────────────────┐
                     │ rpf-archive (crates.io)  rage-formats     │
                     │ RPF7 + NG crypto         (vendored) RSC7, │
                     │                          drawables, ytd,  │
                     │                          ymap/ytyp Meta   │
                     └──────────────────────────────────────────┘
```

## Crates

### `vespucci-d3d11` — Direct3D 11 without Windows

The only crate with `unsafe` code. `build.rs` runs bindgen over `d3d11.h`/`dxgi.h`: on Linux the headers shipped with DXVK-native, on Windows the mingw-w64 ones. The generated FFI exposes COM interfaces as C structs with vtables; the `com_call!` macro calls a vtable slot, `ComPtr<T>` owns a reference (`Release` on drop, `AddRef` on `clone_ptr`).

`Device` wraps the small part of D3D11 the renderer uses: device/context creation (feature level 11.0, hardware adapter, `D3D11CreateDevice`), immutable and dynamic buffers, 2D textures with full mip chains, shader resource views, samplers (including the comparison sampler shadow lookups need), vertex/pixel shaders from DXBC bytes, input layouts, rasterizer/blend/depth-stencil states, render and depth targets, clears, state binding, `DrawIndexed`, and `read_back` (copy to a staging texture, map, honour the row pitch). `setup_env()` fills in the environment a headless run needs (SDL3 offscreen backend, lavapipe ICD, caches) without overriding what the user exported.

The Linux build links `libdxvk_d3d11`/`libdxvk_dxgi`; the Windows build links the system `d3d11`/`dxgi`. Everything above this crate is platform-independent.

### `vespucci-game` — a GTA V install as a file system

- `keys.rs`: derives the archive keys from the user's own `GTA5.exe` (SHA-1 window search for the AES key; the NG tables come with the `rpf-archive` crate) and caches them under `~/.cache/vespucci/` keyed by the exe's hash.
- `archive.rs`: opens `.rpf` archives through `rpf-archive` on memory-mapped files; nested archives are mounted in place.
- `load_order.rs`: the base archives, then DLC packs in `dlclist.xml` order, each with its `setup2.xml`.
- `vfs.rs`: `GameFs`, a flat table of every file across every archive (389,309 files in 179 archives for build 1.0.3889.0, built in under a second). Lookups by full path, by `(extension, name)` and by `(extension, JOAAT hash)` return the file the game would load: the last archive in load order wins. `read()` decrypts, inflates and strips the RSC7 header.

### `vespucci-fxc` — the game's compiled shaders

GTA V ships its shaders compiled: each `.fxc` ("rgxe") container holds, per shader stage, the DXBC blobs, plus constant-buffer definitions, variables with defaults and editor annotations, texture and sampler declarations, and techniques made of passes that name a VS/PS pair (and GS/HS/DS). `fxc.rs` parses the container; `dxbc.rs` parses the DXBC chunk list and reflects `RDEF` (cbuffers with exact byte offsets, resource bindings with slots) and `ISGN` (vertex inputs). `SHADER_DIRS` lists where the current shaders live (the `update2.rpf` copies override `common.rpf`).

### `vespucci-world` — the map as data

- `archetypes.rs`: `ArchetypeDb` from every `.ytyp` in the game (159k archetypes): bounding box, LOD distance, texture dictionary, drawable dictionary, asset type, flags, time flags.
- `ymaps.rs`: `YmapTree`, one node per `.ymap` with its streaming and entity extents, read from the game's `*_cache_y.dat` files (with a header-parse fallback), and `touching(centre, radius)`.
- `mapset.rs`: which `.ymap`/`.ytyp` files are part of story mode versus GTA Online, evaluated from every DLC pack's `content.xml` change sets in two phases (start-up groups for all packs, then the map group), mirroring how the game enables and disables archives.
- `entities.rs`: `CEntityDef` records (position, rotation — stored conjugated for non-interior entities — scale, LOD distance, child LOD distance, LOD level, number of children, parent index, tint, flags) and car generators, plus the `CMapData` flags.
- `txd.rs`: the `gtxd.ymt` texture-dictionary parent chain.
- `streamer.rs`: `collect()` turns a camera position into the list of `Instance`s to draw: maps touching the radius, parsed in parallel; script-only maps skipped except for time-of-day variants; the LOD rule; time-archetype hour bits; sorted by distance.

### `vespucci-render` — the game's models through the game's shaders

- `shader_cache.rs`: `.fxc` by name hash → `FxcProgram` with lazily created D3D11 shaders per technique and their reflection.
- `layout.rs`: a drawable's vertex declaration → D3D11 input elements, validated against the shader's `ISGN`; inputs the mesh lacks get a zero-stride dummy slot.
- `cbuffer.rs`: `CBufferBlock`, a CPU shadow of one constant buffer laid out by `RDEF`, written by variable name hash, uploaded when dirty.
- `material.rs`: `Material::build` resolves one `ShaderFX` (the material record inside a drawable) to a technique's VS/PS, fills the material-owned cbuffers with `.fxc` defaults then the drawable's parameters, binds engine-owned cbuffers from the shared `Globals`, and resolves every texture binding through the material's parameters to a texture by name hash. `bind()` uploads and sets state for a draw.
- `texture.rs`: `.ytd` textures to D3D11 (BC1–BC7 and the uncompressed formats, full mip chains, optional top-level skipping) and `TextureCache` with the stand-ins: magenta for missing, a dim sky colour for engine-supplied inputs, a 1×1 "depth 1.0" for the shadow map.
- `globals_preview.rs`: values for the engine-owned cbuffers (matrices, screen size, sun, ambients, fog off, AO, cascade shadow dummies) that make the forward techniques render sensibly without the real frame pipeline. [binding.md](binding.md) records why each value exists.
- `camera.rs`, `frustum.rs`: look-at/perspective with Z up, reversed-Z projection, bounding-sphere culling.
- `model_view.rs`: `render-model` (one drawable, orbit camera, binding report).
- `world_view.rs`: `render` (instances → models → passes → HDR frame), described in [rendering.md](rendering.md).
- `tonemap.rs`: RGBA16F → RGBA8 display transform.

### `vespucci` — the command line

One subcommand per file: `doctor`, `ls`/`cat`/`find` (VFS), `index`/`probe` (world), `shader`, `texture`, `compare`, `render-model`, `render`. See [cli.md](cli.md). `files.rs` opens the game and reports peak RSS. The MCP server and the desktop shell (milestone M6) will be additional front ends over the same crates.

### `third_party/rage-formats` — vendored parser crate

VIRUXE's public-domain `rage-formats` 0.4.0 (RSC7 resources: drawables `.ydr`/`.ydd`/`.yft`, textures `.ytd`, Meta/PSO `.ymap`/`.ytyp`, vertex declarations, and more) with two fields added to `Archetype` (`flags`, `time_flags`). It is wired in through `[patch.crates-io]`; the intention is to upstream the change and drop the copy.

## How a frame is made

1. **Open the game** (`GameFs`): keys, archives, file table. ~1 s, 230 MB.
2. **Index** (`MapSet`, `ArchetypeDb`, `YmapTree`, `TxdParents`): ~1.5 s. Rebuilt every run for now.
3. **Stream** (`collect`): maps within `--radius` → entities → LOD rule and time filters → `Instance` list.
4. **Load** (`world_view`): for each instance inside the frustum, in distance order and within the byte budget: the model file (`.ydr`/`.ydd`/`.yft` by hash), its materials (shader by name hash → technique by render bucket → cbuffers → textures from the archetype's dictionary chain, the model's own dictionary and the shared `mapdetail`/`vehshare`), the drawable LOD for the distance, geometry buffers.
5. **Draw**: three passes (opaque, decal, alpha) with the matching blend and depth states, grouped to minimise pipeline changes; reversed-Z `D32_FLOAT` depth; `R16G16B16A16_FLOAT` colour.
6. **Read back and tonemap** to PNG.

Numbers for the Vespucci Beach exit test (1280×720): 964 instances streamed, 615 culled, 120 models, 984 draws, 13 MB geometry + 50 MB textures on the device, 775 MB peak RSS, 4.6 s warm / ~13 s cold on a 2-core Haswell with lavapipe.

## Design decisions

- **The game's own shaders, not look-alikes.** Every pixel comes from the DXBC the game ships, bound the way the game binds it (names and offsets from the shaders' own reflection data). That is what makes "looks exactly like the game" reachable; the remaining gap (M5) is feeding the engine-owned constants the real per-frame values rather than preview constants.
- **One code path for desktop and headless.** The D3D11 wrapper is the only platform-specific layer. On Windows it is the system D3D11; on Linux it is DXVK-native on Vulkan. Lavapipe makes the Linux path work on any machine, including CI-class boxes with no GPU, at the cost of speed.
- **Retail files only.** Formats are implemented from the files themselves and from public research. The archive keys are derived from the user's executable at first run and never shipped.
- **Verified facts live in `docs/`.** Shader-binding and format knowledge was established empirically (disassembly with `vkd3d-compiler`, bisecting with the debug switches) and is written down with how it was verified, so the next contributor does not re-derive it.
- **Everything in RAM, budgeted.** Lavapipe keeps GPU resources in system memory, so the streamer has byte budgets, per-distance mip trimming and draw limits rather than relying on GPU memory.
- **Determinism for tests.** lavapipe is deterministic; renders are frozen as golden PNGs and compared by PSNR in `scripts/golden.sh`.

## Conventions

- Coordinates: game world units (metres), Z up, right-handed; `Camera::view()` is `look_at_rh` with up = +Z.
- Matrices are uploaded as `glam` column-major arrays **without transposing**; the game's vertex shaders consume them as stored.
- Hashes: JOAAT (Jenkins one-at-a-time) over the lower-cased name, as the game does; `vespucci_game::joaat`.
- Errors: `anyhow` with context strings; per-model and per-material failures are logged at `debug` and skipped, never fatal to a frame.
- Logging: `log` + `env_logger`-style levels via `--log`; `trace` prints one line per drawn instance.
