# Shader binding facts (from the game's own files)

Verified on build 1.0.3889.0 with `vespucci shader NAME --vars --reflect` and
`vespucci shader --all` (696 `.fxc` files, 21,374 DXBC blobs, all parse).

## Where shaders live
- Current: `update/update2.rpf/common/shaders/win32_40_final/*.fxc` (321 files).
- Originals, superseded: `common.rpf/shaders/win32_40_final/`.
- Variants not used by us: `win32_40_lq_final` (low quality), `win32_nvstereo_final`.
- Lookup is by path in that order, never by bare name hash (the variants share names).

## Container → D3D11
- A material's `ShaderFX.nameHash` is `joaat("<fxc stem>")`; the `.sps` hash names a preset of the same shader.
- Techniques pick one shader per stage. Names seen in `normal_spec`: `draw`, `drawskinned`, `unlit_draw`, `unlit_drawskinned`, `deferred_draw`, `deferred_drawskinned`, `deferredalphaclip_draw`, `lightweight{0,4,8}[CutOut]_draw[skinned|instanced]`, `lightweightHighQuality…`, `…_drawtessellated[_sm50]`.
  - M3 (unlit): `unlit_draw` = `VS_TransformUnlit` + `PS_TexturedUnlit`.
  - M5 (G-buffer): `deferred_draw` = `VS_TransformD` + `PS_DeferredTextured`; alpha-clip and skinned variants alongside.
- Blobs are plain SM4.0/SM5.0 DXBC; DXVK and Windows D3D11 take them unchanged.

## Constant buffers
- RDEF cbuffer names equal the `.fxc` cbuffer names exactly. RDEF lists *every* variable with the true byte offset; the `.fxc` variable list is a subset and its `offset` field is one byte (wraps at 256), so **RDEF is authoritative for layout**, the `.fxc` for defaults and material-parameter names.
- Register slots come from RDEF per shader (the `.fxc` cbuffer `slots` agree for the shaders checked: `rage_clipplanes` b0, `rage_matrices` b1, `misc_globals` b2, `lighting_globals` b3, `rage_bonemtx` b4, `more_stuff` b5, `rage_cbinst_*` b5/b6, `warpshadow`/`csmshader` b6, `megashader_locals` b12).
- Engine-owned (filled per frame/draw by the renderer): `rage_matrices` (gWorld, gWorldView, gWorldViewProj, gViewInverse — 4×64 B), `rage_bonemtx` (255 float3x4), `rage_clipplanes`, `misc_globals` (352 B: globalFade, globalShaderQuality, globalScalars*, globalScreenSize, gTargetAAParams, colorize, fog intensity, player feet…), `lighting_globals` (960 B: sun dir/colour, forward lights, natural/artificial ambients, fog params/colours, gReflectionTweaks), `more_stuff` (alpha ref, wetness, reflection mip count, gUseFogRay), `warpshadow`, `csmshader` (cascade vars, biases), `rage_cbinst_*` (instancing).
- Material-owned: `<shader>_locals` — for `normal_spec` it is `megashader_locals` (b12). Filled from `.fxc` defaults, then the drawable's `ShaderFX` params by name hash.

## Textures and samplers
- RDEF binds a texture *and* a sampler under the same name (`DiffuseSampler` t0/s0, `BumpSampler` t2/s2, `SpecSampler` t3/s3). The `.fxc` resource variable of the same name carries the material param name and sampler-state annotations. No suffix games needed.

## Vertex inputs (ISGN)
- `VS_TransformUnlit`: POSITION0 COLOR0 TEXCOORD0 TEXCOORD1 NORMAL0.
- `VS_TransformD`: POSITION0 COLOR0 TEXCOORD0 NORMAL0 TANGENT0 SV_InstanceID0.
- Input layouts are built from the drawable's vertex declaration and validated by D3D11 against the blob's ISGN; a semantic the shader wants but the mesh lacks gets a zero-stride dummy slot.

## Matrices (verified M3, 2026-10-02)
- `rage_matrices` takes glam `Mat4::to_cols_array()` **without transposing** (gWorld, gWorldView = V·W, gWorldViewProj = P·V·W, gViewInverse). Camera: `look_at_rh` with up = +Z, `perspective_rh` (0..1 depth).
- `unlit_draw` on `prop_cs_heist_bag_01`: 1 geometry, DiffuseSampler bound from the embedded ytd, dummy input TEXCOORD1, material cbuffer not used by this technique (its 8 vector params only bind in `deferred_draw`).
- Disassembly (`vkd3d-compiler -b d3d-asm blob.dxbc` works on the game's DXBC): `VS_TransformUnlit` = `pos·cb1[8..11]` (gWorldViewProj) + saturate(COLOR0) + TEXCOORD0; `PS_TexturedUnlit` = `tex(DiffuseSampler) * vertexColour`, alpha × `misc_globals` reg 12.x. **Vertex colours are not tints**: the bag's mean COLOR0 is (251,251,0) — baked masks the deferred shaders interpret — so unlit renders come out yellow by design. The game's look needs `deferred_draw` + lighting (M5).
- Vertex layout of the bag: `Position:Float3@0 Normal:Float3@12 Colour0:Colour@24 TexCoord0:Float2@28 Tangent:Float4@36` (stride 52).

## Forward-lit techniques (verified M3, 2026-10-02: `lightweightHighQuality0_draw` on `default` and `normal_spec`)
Globals the `.fxc` defaults leave at values that produce black or NaN; the renderer's preview values (`model_view.rs`) are:
- `misc_globals.globalScalars3.z` — the pixel shaders multiply the final colour by it (`mul o0.xyz, r1, cb2[14].z`); default 0. Preview 1.0.
- `lighting_globals.globalFogParams` (5 regs) — all zeros make `0/0` and `log(0)*0` NaNs that blacken every pixel. Reg 0 `.x` is the fog start distance (preview 1e8 = fog off); regs 3 and 4 are sun-scatter terms `(dir.xyz, exponent.w)` whose exponents must be non-zero.
- `more_stuff.gAmbientOcclusionEffect` — multiplies ambient; default 0. Preview (1,1,1,1). `gDynamicBakesAndWetness` preview (1,1,0,0).
- `csmshader.gCSMShaderVars_shared` — rows 0..2 world→shadow rotation, regs 4..7 cascade scales, 8..11 offsets; all zero gives zero derivatives → `1/0`. Preview identity rotation + 1e-3 scales. `gCSMShadowTexture` (t15, `sample_c` with `comparisonMode` sampler s15) is bound to a 1x1 R32_FLOAT = 1.0 with a LESS_EQUAL comparison sampler → fully lit. Samplers whose binding name contains "shadow" get the comparison sampler.
- Sun: `gDirectionalLight.xyz` (sign still unverified), `gDirectionalColour`, six ambient colours, `gDirectionalAmbientColour`.
- Material cbuffer (`megashader_locals` / `default_locals`) binds in the lit techniques; all 8 `ShaderFX` params of `normal_spec` matched by name hash (exact or lowercase joaat both tried).
- Engine textures still unbound in previews: `ReflectionSampler` (t1), `FogRaySampler` (t11) → magenta 1x1; harmless with fog off and reflection tweaks zero.
- Output is linear HDR (the game tonemaps afterwards); the unlit `tex * vertexColour` path and the lit path are both frozen as goldens (`scripts/golden.sh`).
- Debug switches: `VESPUCCI_ONES=<cbuffer>|material` fills a buffer with 1.0; `VESPUCCI_ONES_VAR=<cbuffer>:<var>[:index]` sets one variable/element. `vkd3d-compiler -b d3d-asm blob.dxbc` disassembles any blob from `vespucci shader NAME --blob ps:N --out blob.dxbc`.

## World rendering (verified M4, 2026-10-02)
- Render buckets (`ShaderFx.render_bucket`): 0 opaque, 1 alpha (blend, no depth write, back-to-front), 2 decal (blend, depth test GREATER_EQUAL under reversed-Z, no write), 3 cutout (opaque pass with the `…CutOut_draw` technique). Everything else is drawn opaque for now.
- `DrawableModel.render_mask_flags` bit 0 = drawn in the visible pass; clear on shadow-proxy models (`cpv_only`/`trees_shadow_proxy` geometry).
- `CMapData.flags` bit 0 = script-requested map (not streamed by position); `CTimeArchetypeDef.timeFlags` bit h = visible during hour h.
- `_tnt` shaders: the vertex shader samples `TintPaletteSampler` at `(COLOR0.b, tintPaletteSelector.x)` (`sample_l`, level 0, the selector is used directly as the V coordinate); palettes are 256×N (N = 4 on `vb_30_ladder_05`) with one real colour per column and pink `(255,127,255)` filler elsewhere. Sampling them with a linear/wrap sampler blends column 0 with column 255 and turns tinted parts pink: the palette needs a point + clamp sampler. The entity `tint` value (`CEntityDef` +120) is not applied yet (selector stays at the material default).
- `CEntityDef.lodDist` of -1 (half of all entities) means "use the archetype's `lodDist`" (verified: the streamer dropped every such entity until it did).
- `cable.fxc` (power lines) projects with `cable_locals.gViewProj` instead of `gWorldViewProj` and sizes the strands with `gCableParams`: x = pixels per metre at 1 m (`0.5 × height / tan(fov/2)`), y scales `shader_radiusScale`, z scales `shader_fadeExponent`, w scales the alpha; both are material-cbuffer variables the engine writes per frame (`Globals.frame`).
- `…CutOut_draw` pixel shaders (`PS_Textured_Zero_CutOut`) discard when `more_stuff.gAlphaRefVec0.x >= alpha × globalScalars.x`; with the cbuffer default of 0 nothing is discarded, so cutout LOD billboards draw as solid quads (not set yet).
- `trees.fxc`/`trees_lod.fxc` have no plain forward technique: `draw` and every `lightweight*_draw` are the water-reflection variants (`VS/PS_PropFoliageWaterReflection`), which is what the preview uses. Their pixel shader divides by the Jacobian (`dsx`/`dsy`) of the shadow-map coordinates; an axis-aligned dummy CSM transform gives a zero determinant on camera-facing LOD quads → NaN pixels.
- `normal_spec_detail*`: detail uv = uv × `detailSettings.zw`, two taps (second at ×3.17) averaged; `detailSettings.x` darkens diffuse by `d.x × spec.a`, `.y` scales the detail normal; the shared detail maps live in `x64a.rpf/textures/mapdetail.ytd` (BC5).
- Forward pixel shaders square the diffuse sample (`mul r0.xyz, r0, r0`) — textures are sampled as plain UNORM, not sRGB views; the output is linear HDR, so the preview tonemap is exposure → ACES → gamma 2.2.
- `FogRaySampler` (t11) is read only when `misc_globals` reg 19.y > 0; `ReflectionSampler` (t1) is a 2D paraboloid map in the game — a flat sky-coloured stand-in is bound in previews.
