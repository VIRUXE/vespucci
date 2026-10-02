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

`collect()` reads every ymap whose entity extents touch the sphere `--radius` around the camera (default 300 m). Maps are parsed in parallel; the LOD rule in [formats.md](formats.md) decides which entities are candidates; the result is sorted by distance so that, when a budget is hit, the nearest things are the ones drawn. Three limits apply while loading: `--max-draws` (instances), `--budget-mb` (geometry + texture bytes uploaded) and `--mip-skip` (top mip levels dropped at <100 m, <500 m and beyond). Models and texture dictionaries are cached per render by hash, so a prop used 200 times is loaded once.

### Materials

For each `ShaderFX` in a drawable's shader group:

1. The shader is found by name hash in the game's shader directories (`update2.rpf` first).
2. A technique is chosen by render bucket and lighting mode. `--lighting basic` tries `lightweightHighQuality0_draw`, `lightweight0_draw`, `draw`, `unlit_draw` (with the `…CutOut_draw` variants first for bucket 3); `--lighting none` tries `unlit_draw`, `draw`. Shaders with none of these (a handful, e.g. `grass_fur_mask`) are skipped and logged at `debug`.
3. The pass's VS and PS blobs become D3D11 shaders (cached by blob); their RDEF gives the cbuffers and bindings.
4. Material cbuffers get the `.fxc` defaults, then the drawable's parameters by name hash. Engine cbuffers bind the shared `Globals`.
5. Each texture binding is resolved through the material parameter of the same name to a texture name hash, looked up in the dictionaries listed in [formats.md](formats.md). Bindings with no material parameter are engine-supplied (`ReflectionSampler`, `FogRaySampler`) and get a flat stand-in; `gCSMShadowTexture` gets a 1×1 depth of 1.0 with a comparison sampler so every shadow test passes.
6. Samplers: anisotropic 16, wrap, for everything except the shadow comparison sampler. The `.fxc` sampler-state annotations are not applied yet.

### Preview globals (`globals_preview.rs`)

The forward techniques read per-frame engine constants that the real game fills from its time cycle: sun direction and colour, six ambient colours, fog parameters, ambient-occlusion and wetness factors, cascade-shadow matrices, screen size, and a global output scale. Vespucci sets constants that produce a plausible daylight look and, more importantly, avoid the values that produce NaN or black (fog start at 1e8 and non-zero scatter exponents; `globalScalars3.z = 1`; `gAmbientOcclusionEffect = 1`; identity-ish cascade matrices). Every value has a comment with why it exists.

### Depth, colour, tonemap

Reversed-Z (near = 1, far = 0) in a 32-bit float depth buffer keeps coplanar decals and far geometry stable from 0.1 m to 4 km. The colour target is `R16G16B16A16_FLOAT` because the game's shaders emit linear HDR; the CPU applies `--exposure`, an ACES fit and gamma 2.2. NaN/Inf pixels are painted magenta and counted (a warning is logged), which turns shader-input mistakes into something visible rather than black.

### Passes and ordering

Opaque draws are grouped by model so consecutive draws share shaders, layouts and textures (lavapipe compiles a pipeline per unique state, and state changes are the main cost). Decals follow with blending and depth-test-only; alpha surfaces last, far to near by instance distance.

## What the output is and is not

Done: geometry placement, LOD selection, materials and textures as the game binds them, alpha test/blend/decal ordering, tint palettes, detail maps, time-of-day object variants, script-map filtering.

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
| Far LOD coverage | distant SLOD pieces can appear to float when the LOD that should sit under them is in a map outside `--radius` or is hidden by a different distance band | streaming by LOD level |

## Performance notes (lavapipe, 2-core Haswell)

A 1280×720 beach frame: ~13 s cold (pipeline compilation), ~5 s warm, peak RSS 775 MB. The time splits roughly into opening the game (1 s), indexing (1.5 s), streaming (0.05 s), loading models and textures (1–2 s), and drawing (2 s). The DXVK state cache and the Mesa shader cache under `~/.cache/vespucci/` make the cold part a one-time cost per shader/layout combination. On a real GPU the draw and load phases shrink to a fraction of a second.
