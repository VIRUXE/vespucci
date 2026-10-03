# Game data formats

What Vespucci reads from a GTA V (Legacy, PC) install, where it lives, and the facts the code relies on. Parsing of the resource formats is done by the vendored `rage-formats` crate and `rpf-archive`; this page is about how the pieces fit together and what was verified against build 1.0.3889.0. Verification methods are noted so they can be repeated.

## Archives (`.rpf`)

RPF version 7. A 16-byte header (`RPF7` magic, entry count, name-table length, encryption type), a table of contents and a name table, both encrypted. Entries are directories, binary files (optionally deflate-compressed) or resources (RSC7, compressed, with system/graphics segment sizes packed into flags). Nested archives are binary entries that are themselves RPF files; Vespucci mounts them in place, so a path like `x64c.rpf/levels/gta5/props/.../x.ydr` addresses a file inside a nested archive.

Encryption: the retail PC archives use the **NG** scheme (`0x0FEFFFFF`), a 17-round block cipher keyed per file name and size with tables derived from the executable. The AES key and the NG tables are recovered from `GTA5.exe` by `rpf-archive` (SHA-1 search over the binary); Vespucci caches the result per executable hash. Only `.ysc` resources are themselves encrypted inside archives.

Load order: `x64*.rpf` and `common.rpf`, then `update/update.rpf`, `update/update2.rpf`, then every DLC pack from `update/x64/dlcpacks/*/dlc.rpf` in the order of `update/update.rpf/common/data/dlclist.xml`. A file name that appears in several archives resolves to the last one. The current shaders, for example, are in `update/update2.rpf/common/shaders/win32_40_final/`, overriding `common.rpf`.

## Map sets (what is loaded in story mode)

Each DLC pack's `setup2.xml` names a `content.xml` with **change sets**: lists of archives to enable or disable and map files to add or remove, grouped by name. Which groups apply depends on the mode: story mode activates the start-up groups, GTA Online additionally activates `GROUP_MAP` sets that swap whole areas (and some DLC maps exist only online). Vespucci evaluates them in two phases over all packs (start-up first, then the map group), which matters because a later patch pack can re-enable archives an earlier pack disabled. Result for this build: 4,602 story-mode ymaps, 5,425 online. Archetype definitions are read from every archive regardless of map set.

## Resources (RSC7)

Resource files (`.ydr`, `.ydd`, `.yft`, `.ytd`, `.ymap`, `.ytyp`, …) start with an `RSC7` header giving a version and the sizes of a system (CPU) segment and a graphics (GPU) segment; pointers inside are virtual addresses into one of the two segments. `rage-formats` resolves them.

## Drawables (`.ydr`, `.ydd`, `.yft`)

A drawable is one model: a shader group (materials), up to four LOD levels (High/Medium/Low/VeryLow) with distance thresholds, bounding volumes, and per LOD a list of **models**, each a list of **geometries** (vertex buffer + index buffer + a shader index into the shader group). `.ydd` is a dictionary of drawables keyed by name hash; `.yft` is a fragment whose main drawable Vespucci draws (vehicles and breakable props are fragments; their skeletons are not used yet).

Facts used:
- `ShaderFX` (one per material): the shader name hash (= JOAAT of the `.fxc` file stem), the render bucket, and a list of parameters, each a texture reference or one or more float4 vectors, keyed by name hash. Parameter hashes equal `joaat(name)` of the variable name in the `.fxc` (the code tries exact and lower-case).
- `DrawableModel.render_mask_flags`: bit 0 set means the model is drawn in the visible pass; shadow-proxy models have it clear (`0xe2` seen on palms) and must be skipped or they draw over the real mesh.
- Vertex declarations: 16 semantic slots (position, blend weights/indices, normal, colour 0/1, texcoord 0–7, tangent, binormal) with a 4-bit type code each; offsets accumulate in slot order and must equal the stride. Colour is `R8G8B8A8_UNORM`; tangent is `Float4`.
- Vertex colours are **masks, not tints**: a prop's COLOR0 of (251,251,0) is read by the deferred shaders as bake/AO data. Tint shaders (`*_tnt`) sample a palette texture at `(COLOR0.b, tintPaletteSelector.x)` in the vertex shader.
- Textures a drawable needs are found in: its embedded texture dictionary, the archetype's `textureDictionary` and that dictionary's parents (`gtxd.ymt`), a dictionary named like the model, and finally the shared `mapdetail.ytd` (detail normal maps `env_*`) and `vehshare.ytd`.

## Textures (`.ytd`)

A dictionary of textures keyed by name hash. Each texture carries width, height, stride, format, mip count and the pixel data (in the graphics segment). Formats seen: DXT1/3/5 (BC1/2/3), ATI1/ATI2 (BC4/BC5), BC7, A8R8G8B8, A8B8G8R8, A1R5G5B5, A8, L8. The mip chain is stored contiguously; each level is a quarter of the previous (the game does not pad tiny block-compressed levels, so the last levels are re-laid out before upload). Forward pixel shaders square the diffuse sample as a cheap gamma decode, so textures are sampled as plain UNORM, not as sRGB views.

## Map data (`.ymap`) and archetypes (`.ytyp`)

Both are Meta-format (`PRD0`/`PSO`) resources holding `CMapData` / `CMapTypes` structures.

`CMapData`: name, parent, **flags** (bit 0: the game only loads the map on a script's request; bit 1: LOD map), content flags (HD, LOD, SLOD2, interiors, …), streaming extents, entity extents, entities, car generators, and more. The `*_cache_y.dat` files bundled with the game list every ymap with its extents, so the tree is built without opening 11k files.

`CEntityDef` (128 bytes): archetype name hash, flags, position, rotation quaternion (stored **conjugated** for everything except interior-instance entities), scale XY/Z, parent index, LOD distance, child LOD distance, LOD level (HD, LOD, SLOD1…SLOD4, ORPHANHD), number of children, tint, and more.

`CBaseArchetypeDef` (144 bytes): LOD distance, flags, bounding box and sphere, texture dictionary, drawable dictionary, asset type (drawable, fragment, drawable-dictionary entry, assetless) and asset name. `CTimeArchetypeDef` adds `timeFlags`, one bit per hour of the day. `CMloArchetypeDef` describes an interior: its own entities (`CEntityDef`s placed relative to the MLO origin), rooms (name, box, the indices of the entities each owns), portals (quads in MLO-local space joining two rooms; room 0 is the outside) and entity sets, named groups of extra entities a placement switches on. A `CMloInstanceDef` places one: the 128-byte `CEntityDef` plus `groupId`, `floorId`, `defaultEntitySets` (the set names enabled here), `numExitPortals` and flags. See "Interiors" below.

## The LOD system

Every entity has a `lodDist` (-1 means "the archetype's"). Entities form a tree across LOD levels (HD → LOD → SLOD1 → SLOD2 → SLOD3): `parentIndex` points at the parent, in the same map when that entry is a coarser level, otherwise in the map's parent map (`CMapData.parent`; `flags` bit 3 says so explicitly); a parent carries `numChildren` and `childLodDist` (-1 means half its own `lodDist`; 0 means it never steps aside for distance alone). The rule Vespucci applies, which is the one CodeWalker's renderer implements (`Renderer.RecurseAddVisibleLeaves`/`GetEntityChildren`), walked from the roots (entities without a parent):

```
root e:        drawn-or-descended only when d <= lodDist(e) * scale
descend(e):    all of e's children are loaded (and drawable)
               && (d <= childLodDist(e) * scale || some child c has d(c) <= lodDist(c) * scale)
otherwise:     e is a leaf and is drawn — even when it is a handed-over child beyond its own lodDist
```

So an HD building shows while its LOD parent is within `childLodDist` (or the building is within its own `lodDist`), and the parent takes over beyond that; a parent whose children are not all loaded keeps drawing itself, which is what the game does while a child map is not streamed. Children Vespucci cannot draw (entities without an archetype, interior placements under `--no-interiors`) count as not loaded, so such a parent keeps its LOD shell. Orphan HD entities simply obey their own distance. An interior placement is an ordinary child in this tree: its LOD parent typically has `childLodDist` 0 and hands over once the camera is within the placement's own `lodDist`.

## Interiors (MLO)

An interior placement (`CMloInstanceDef`) names a `CMloArchetypeDef` and is drawn as that definition's entities plus the entities of every entity set listed in the placement's `defaultEntitySets`. Each is moved into world space by the instance transform, the way CodeWalker's `MloInstanceData::UpdateEntity` does it:

```
world position = instance position + rotate(instance rotation, local position)
world rotation = instance rotation ⊗ local rotation        (local first, then the instance)
```

where the instance rotation is the placement's quaternion **as stored** (interior placements, unlike every other entity, do not store the inverse) and the local rotation is the conjugate of the definition entity's stored quaternion (those do store the inverse, like map entities). Verified by rendering from inside the Pillbox Hill Ammu-Nation (`v_gun`): shelves, counters and wall text come out where they belong and the right way round.

Every entity of a drawn interior is drawn, whatever room the camera is in and whatever the entity's own `lodDist` says (CodeWalker's rule), except proxies. The game additionally hides rooms behind closed portals and culls interior entities by their own distance; neither is done yet. Rooms and portals are parsed (`rage-formats` `MloDef`) and available for that work.

## Proxies

Some entities exist only to feed the shadow map or the reflection map and are never drawn into the frame: shadow proxies, marked by archetype `flags` bit 11 (2048), and reflection proxies, which CodeWalker recognises by exact `CEntityDef::flags` values (135790592, 135790593, 672661504, 536870912, 35127296, 39321602: golf-course and house water proxies, tree and tunnel reflection proxies, mirror-only emissives, interior reflection shells such as `v_7_gc_reflectproxy`). `streamer::is_proxy` leaves both out, for map and interior entities alike (`StreamStats::proxies_skipped`). The 6.7 km `*_reflproxy` boxes of the exterior are script-requested maps and are skipped as such.

Which maps are loaded: every ymap whose entity extents touch `--radius` around the camera, every ymap whose **streaming extents** contain the camera (the game's own rule — those extents are the entity extents grown by the entities' lodDists), all of their ancestor maps (the parents live there), and, for a loaded parent within its `childLodDist` that still misses children, the child maps of its map. `StreamStats` counts each group. The streamer then culls by the archetype's bounding sphere against the view frustum and picks the drawable's own LOD level from its `lod_distances`.

Script-requested maps (`CMapData` flag bit 0) are left out by default, with one heuristic exception: maps named `*_morning`, `*_day`, `*_evening`, `*_night` (and their `_lod` twins) are the variants the game's scripts swap by time of day, so the one matching `--time` is included. Time archetypes are shown when their hour bit is set.

## Shaders (`.fxc`) and DXBC

See [binding.md](binding.md) for the verified binding rules. In short: an `.fxc` holds, per stage, DXBC blobs and metadata (cbuffers, variables with defaults, textures/samplers, techniques → passes). RDEF inside each DXBC blob is authoritative for cbuffer layout and register slots; the `.fxc` supplies defaults and the material-parameter names. Engine-owned cbuffers (`rage_matrices`, `misc_globals`, `lighting_globals`, `more_stuff`, `csmshader`, …) are filled by the renderer; `<shader>_locals`/`megashader_locals` are filled from the material. `vkd3d-compiler -b d3d-asm` disassembles any blob, which is how most of the facts here were established.

## Render buckets

`ShaderFX.render_bucket`: 0 opaque, 1 alpha-blended, 2 decal, 3 alpha-tested (cutout); higher values (water, displacement) exist and are currently drawn as opaque. Decals and alpha surfaces are drawn after opaque geometry with blending and without depth writes.

## Not handled yet

Interior rooms and portals as culling (every entity of a drawn interior is drawn), vehicles beyond their main drawable (no skeleton/bone matrices, no `carcols`), car generators (parsed, not drawn), water (`water.xml`), time cycle / weather (`timecycle_*.xml`, `visualsettings.dat`), LOD lights, occluders, navmesh/paths, scripted IPL toggles other than the time-of-day heuristic.
