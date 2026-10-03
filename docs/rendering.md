# Rendering

How `vespucci render` turns a camera position into pixels, and what is still missing before the output matches the game frame for frame. The binding-level facts (which cbuffer, which offset, which slot) are in [binding.md](binding.md).

## Pipeline

```
camera ──► streamer (maps in radius → entities → LOD rule → instances, sorted by distance)
       ──► per instance: frustum test ── archetype → model file → GpuModel (materials, LOD geometry)
       ──► draw list: (pass, key, instance, geometry)
       ──► pass 1 opaque   : buckets 0,3,… ; depth write, GREATER (reversed-Z); grouped by model
       ──► pass 2 decal    : bucket 2 ; alpha blend, depth test GREATER_EQUAL, no write
       ──► pass 3 alpha    : bucket 1 ; alpha blend, depth test, no write, far-to-near
       ──► RGBA16F frame ──► read back ──► exposure, ACES, gamma 2.2 ──► PNG
```

### Streaming and budgets

`collect()` reads every ymap whose entity extents touch the sphere `--radius` around the camera (default 300 m) and every ymap whose streaming extents contain the camera (what the game itself would have loaded), plus their ancestors. Maps are parsed in parallel and linked into the LOD tree; the rule in [formats.md](formats.md), walked from the roots, decides which entities are drawn; the result is sorted by distance so that, when a budget is hit, the nearest things are the ones drawn. Three limits apply while loading: `--max-draws` (instances), `--budget-mb` (geometry + texture bytes uploaded) and `--mip-skip` (top mip levels dropped at <100 m, <500 m and beyond). Models are cached per render by file, dictionary entry and texture dictionary (a prop used 200 times is loaded once; each entry of a `*_slod_children.ydd` is its own model, and two archetypes sharing a drawable but not a texture dictionary get separate materials), texture dictionaries by hash.

### Materials

For each `ShaderFX` in a drawable's shader group:

1. The shader is found by name hash in the game's shader directories (`update2.rpf` first).
2. A technique is chosen by render bucket and lighting mode. `--lighting basic` tries `lightweightHighQuality0_draw`, `lightweight0_draw`, `draw`, `unlit_draw` (with the `…CutOut_draw` variants first for bucket 3); `--lighting none` tries `unlit_draw`, `draw`. Shaders with none of these (a handful, e.g. `grass_fur_mask`) are skipped and logged at `debug`.
3. The pass's VS and PS blobs become D3D11 shaders (cached by blob); their RDEF gives the cbuffers and bindings.
4. Material cbuffers get the `.fxc` defaults, then the drawable's parameters by name hash. Engine cbuffers bind the shared `Globals`.
5. Each texture binding is resolved through the material parameter of the same name to a texture name hash, looked up in the dictionaries listed in [formats.md](formats.md). Bindings with no material parameter are engine-supplied (`ReflectionSampler`, `FogRaySampler`) and get a flat stand-in; `gCSMShadowTexture` gets a 1×1 depth of 1.0 with a comparison sampler so every shadow test passes.
6. Samplers: anisotropic 16, wrap, for everything except the shadow comparison sampler and the point + clamp sampler for tint palettes (`Samplers::for_binding`). The `.fxc` sampler-state annotations are not applied yet (#6). `_tnt` materials also get `tintPaletteSelector` set per draw from the entity's tint.

### Preview globals (`globals_preview.rs`)

The forward techniques read per-frame engine constants that the real game fills from its time cycle: sun direction and colour, six ambient colours, fog parameters, ambient-occlusion and wetness factors, cascade-shadow matrices, screen size, and a global output scale. Vespucci sets constants that produce a plausible daylight look and, more importantly, avoid the values that produce NaN or black (fog start at 1e8 and non-zero scatter exponents; `globalScalars3.z = 1`; `gAmbientOcclusionEffect = 1`; a rotated, tiny-scale cascade transform; the cutout alpha reference `gAlphaRefVec0 = 0.5`). Every value has a comment with why it exists.

### Depth, colour, tonemap

Reversed-Z (near = 1, far = 0) in a 32-bit float depth buffer keeps coplanar decals and far geometry stable from 0.1 m to 4 km. The colour target is `R16G16B16A16_FLOAT` because the game's shaders emit linear HDR; the CPU applies `--exposure`, an ACES fit and gamma 2.2. NaN/Inf pixels are painted magenta and counted (a warning is logged), which turns shader-input mistakes into something visible rather than black.

### Passes and ordering

Opaque draws are grouped by model so consecutive draws share shaders, layouts and textures (lavapipe compiles a pipeline per unique state, and state changes are the main cost). Decals follow with blending and depth-test-only; alpha surfaces last, far to near by instance distance.

## What the output is and is not

Done: geometry placement, the game's LOD tree walked the way CodeWalker does it (parents hand over to loaded children, children drawn when handed over, streaming-extent map selection), materials and textures as the game binds them, tint palettes with the entity's tint, cutout alpha test, alpha blend/decal ordering, detail maps, time-of-day object variants, script-map filtering, power-line cables, a draw-id picking pass (`--pick`, `--id-map`). Known gaps are tracked as issues: a building with an interior keeps its LOD shell until interiors render (#19), and the brightness of LOD façades, which is the preview ambient (#2).

Not done (these are the differences you will see against a screenshot):

| Missing | Effect today | Planned |
|---|---|---|
| Sky dome, clouds, sun disc | flat blue background colour | M5 |
| Time cycle, weather, real sun/ambient/fog values | constant daylight look, no fog, no shadows | M5 (matched against RenderDoc captures) |
| Deferred lighting path (`deferred_draw` techniques, G-buffer, lights) | forward "lightweight" look; some materials appear too dark or flat (notably downtown glass/reflective buildings) | M5 |
| Post-processing (exposure adaptation, bloom, colour grading, FXAA) | simple ACES + gamma | M5 |
| Water, interiors (MLO), vehicles with skeletons, car generators, LOD lights | not drawn | after M5 |
| Reflection and fog-ray textures | flat stand-in colour | M5 |
| `.fxc` sampler states (filters, address modes, mip bias) | one anisotropic wrap sampler | M5 |

## Performance notes (lavapipe, 2-core Haswell)

A 1280×720 beach frame: ~13 s cold (pipeline compilation), ~5 s warm, peak RSS 775 MB. The time splits roughly into opening the game (1 s), indexing (1.5 s), streaming (0.05 s), loading models and textures (1–2 s), and drawing (2 s). The DXVK state cache and the Mesa shader cache under `~/.cache/vespucci/` make the cold part a one-time cost per shader/layout combination. On a real GPU the draw and load phases shrink to a fraction of a second.
