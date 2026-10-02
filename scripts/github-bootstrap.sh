#!/usr/bin/env bash
# Labels, milestones and the initial development issues for VIRUXE/vespucci.
# Idempotent: existing labels/milestones/issues (by title) are left alone.
set -euo pipefail
R=VIRUXE/vespucci

echo "== labels"
label() { gh label create "$1" -R $R --color "$2" --description "$3" --force >/dev/null && echo "  $1"; }
label rendering   8250df "Shaders, materials, passes, lighting, what pixels look like"
label formats     0e8a16 "Game file formats and verified facts about them"
label streaming   1d76db "Maps, entities, LOD selection, budgets"
label editor      fbca04 "The desktop editor (M6)"
label mcp         f9d0c4 "The MCP server / agent interface"
label windows     5319e7 "Windows build and native D3D11"
label performance d93f0b "Speed and memory"
label ci          bfdadc "Builds, tests, goldens, GitHub Actions"
label upstream    c2e0c6 "Belongs in rage-formats / rpf-archive / DXVK"
label captures    e99695 "Needs or produces RenderDoc captures of the real game"

echo "== milestones"
ms() { # title description
  if ! gh api "repos/$R/milestones?state=all&per_page=100" --jq '.[].title' | grep -qxF "$1"; then
    gh api "repos/$R/milestones" -f title="$1" -f description="$2" >/dev/null; echo "  created $1"
  else echo "  exists $1"; fi
}
ms "M5 — game-lit frame" "render --lighting game matches RenderDoc captures of the real game pass by pass: sky, time cycle, deferred lighting, post-processing. Render core MVP."
ms "M6 — editor and MCP" "Ariane's core loop on the streamed world: select, move/rotate/scale, undo, object browser, save as a mod; Dear ImGui shell on Windows; the same operations over MCP headless."
ms "After MVP" "Vehicles, interiors, water, occluders, web viewer, exact-look refinements."

echo "== issues"
existing=$(gh issue list -R $R --state all --limit 500 --json title --jq '.[].title')
mk() { # title labels milestone body
  if grep -qxF "$1" <<<"$existing"; then echo "  exists: $1"; return; fi
  local args=(-R $R -t "$1" -b "$4")
  [ -n "$2" ] && args+=(-l "$2")
  [ -n "$3" ] && args+=(-m "$3")
  gh issue create "${args[@]}" >/dev/null && echo "  created: $1"
}

# ---------- M5: game-lit frame ----------
mk "Sky dome, sun disc and clouds" "rendering" "M5 — game-lit frame" "Today the background is a flat colour (\`WorldViewOptions::background\`). The game draws a sky dome with the \`sky_system\` shader family, a sun disc and cloud layers driven by the time cycle.

**Done when**: \`render\` at Vespucci Beach shows the game's sky for the given \`--time\`/weather, and capture C (camera straight up) matches within the M5 thresholds (PLAN.md).

See docs/rendering.md (what is missing) and docs/binding.md."

mk "Time cycle and weather: feed the engine cbuffers the game's values" "rendering,formats" "M5 — game-lit frame" "\`globals_preview.rs\` fills \`misc_globals\`, \`lighting_globals\`, \`more_stuff\` and \`csmshader\` with constants chosen to avoid NaN/black. The game computes them from \`timecycle_*.xml\`, \`visualsettings.dat\` and the weather tables for the current hour and weather.

Tasks:
- parse the time-cycle files (keyframes per weather, modifiers) and \`visualsettings.dat\`
- sun direction from the hour (and the game's sun path), sun/ambient/fog colours, exposure
- a \`--weather NAME\` flag next to \`--time\`
- compare the resulting cbuffer values with the captured ones (#RenderDoc issue)

**Done when**: the engine cbuffers for capture A (18:30 EXTRASUNNY) are reproduced from the data files, not typed in."

mk "Deferred lighting path (G-buffer + deferred_draw techniques)" "rendering" "M5 — game-lit frame" "The forward \`lightweight*\` techniques give a preview look. The game renders opaque geometry with the \`deferred_draw\` techniques into a G-buffer (albedo, normals, specular, bake/AO from vertex colours) and lights it in a fullscreen pass.

Tasks:
- G-buffer targets with the formats seen in the captures
- \`deferred_draw\` technique selection per bucket; cutout/decal variants
- the deferred lighting pass: sun + ambient + fog with the game's shaders where bindable, own HLSL (flagged approximate) otherwise
- forward pass afterwards for alpha/decal/water

**Done when**: G-buffer albedo PSNR >= 30 dB and normals mean angle <= 5 deg against capture A (PLAN.md thresholds)."

mk "Post-processing: exposure, bloom, tonemap, colour correction, FXAA" "rendering" "M5 — game-lit frame" "\`tonemap.rs\` applies a fixed exposure, an ACES fit and gamma 2.2 on the CPU. The game runs its own post stack (adaptive exposure, bloom, filmic tonemap with time-cycle parameters, colour correction, FXAA) on the GPU.

**Done when**: the final image of capture A reaches PSNR >= 25 dB / SSIM >= 0.85 with the post stack in place (PLAN.md)."

mk "RenderDoc capture ingestion and per-pass comparison" "rendering,captures,ci" "M5 — game-lit frame" "M5 is validated against captures of the real game. Needed:

- \`scripts/renderdoc_export.py\` (RenderDoc Python API, runs on the Windows PC): writes \`captures/<name>/{events.json, cbuffers.json, samplers.json, gbuffer*.png, depth, shadow_atlas.png, hdr.exr, final.png, camera.json}\` with blob SHA-1s so draws map to our shaders/techniques
- \`render --from-capture DIR\`: use the captured camera and cbuffer values verbatim to isolate geometry/materials from lighting maths
- per-pass thresholds in \`tests/golden/thresholds.json\` and hand-painted masks (vehicles, peds, grass)

Captures to make (1280x720, MSAA off, FXAA on, TXAA/DoF/motion blur off, shadows High, player hidden, time frozen):
A Vespucci Beach boardwalk facing the pier 18:30 EXTRASUNNY (MVP target) · B Legion Square 12:00 CLEAR · C = A pointing straight up · D Del Perro pier 21:30 CLEAR · E = A at 13:00 OVERCAST."

mk "Apply the .fxc sampler states instead of one anisotropic wrap sampler" "rendering" "M5 — game-lit frame" "Every texture is sampled with aniso-16 / wrap (plus the comparison sampler for shadow bindings). The \`.fxc\` sampler variables carry the intended state in their annotations (filter, address modes, mip bias, comparison). Parse them in \`vespucci-fxc\` and create one \`ID3D11SamplerState\` per distinct state in \`material.rs\`. Validate against \`samplers.json\` from the captures."

mk "Reflection map and fog-ray textures (currently a flat stand-in)" "rendering" "M5 — game-lit frame" "\`ReflectionSampler\` (a 2D paraboloid reflection map in the game) and \`FogRaySampler\` get a 1x1 dim-sky stand-in (\`TextureCache::engine\`). Render the reflection map (cube/paraboloid of the sky and distant LODs) and the fog-ray buffer, or at least a sky-only reflection. Reflective downtown buildings depend on this (see the Legion Square issue)."

mk "Cascaded shadow maps" "rendering" "M5 — game-lit frame" "\`gCSMShadowTexture\` is a 1x1 depth of 1.0 with identity-ish cascade matrices, so everything is lit. Render the four cascades from the sun with the game's shadow techniques (\`*_shadow*\` passes), fill \`csmshader\` (gCSMShaderVars_shared: rotation, cascade scales/offsets) the way the captures show, bind the atlas. Done when the shadow atlas IoU >= 0.95 against capture A."

mk "Water (water.xml quads, ocean, pools)" "rendering,formats" "M5 — game-lit frame" "No water is drawn: the beach renders end at the sand. The game's water is a set of quads from \`water.xml\` plus the ocean, drawn with the \`water\` shader family (reflection, refraction, foam). A flat first version with the game's shader and a stand-in reflection is enough for M5; captures A and D show the target."

# ---------- bugs / correctness ----------
mk "Downtown glass and reflective buildings render dark (Legion Square)" "bug,rendering" "M5 — game-lit frame" "Repro: \`vespucci render --pos=150,-970,45 --look=230,-900,25 --radius 350 --out legion.png\` — the large buildings around Legion Square come out nearly black, while the plaza, props and vegetation are fine.

Not caused by a missing technique (\`--log debug\` shows only \`grass_fur*\` skipped). Likely candidates: materials whose look depends on the reflection map / env cubemap (\`normal_spec_reflect*\`, \`glass_*\`, \`emissive*\`) getting the flat stand-in, or vertex-colour bake channels interpreted by the forward techniques. Bisect with \`VESPUCCI_SKIP_SHADER\`, \`VESPUCCI_SET_VAR\`, \`VESPUCCI_PROBE\` (docs/debugging.md)."

mk "Distant SLOD pieces appear to float (Vinewood tower base, downtown)" "bug,streaming" "" "Repro: \`vespucci render --pos=560,1120,330 --look=711,1198,350 --radius 400\` shows three flat slabs in the sky above the Vinewood sign. \`VESPUCCI_TRACE_ENTITY=towerbase\` shows they are the \`ch2_03_towerbase_slod_children.ydd\` entries (LOD level, lodDist 500, childLodDist 110/164) at d=236–261 while the HD \`ch2_03_towerbase.ydr\` (lodDist 164) is correctly hidden — so the LOD rule picks what the game would show, but the pieces have no visible sides and the terrain they sit on at that LOD is not there. Downtown (\`--pos=-420,-640,50 --look=-75,-818,130\`) shows similar floating boxes and rooftop palms with no building under them; a larger \`--radius\` does not change it.

To investigate: whether the SLOD meshes' side geometry is skipped (render mask bits other than bit 0? geometry with a shader lacking a forward technique?), and whether the HD→LOD switch for the surrounding terrain uses a different distance band than the props on it."

mk "Entity tint value is not applied (tintPaletteSelector stays at the material default)" "bug,rendering,formats" "" "\`_tnt\` shaders sample \`TintPaletteSampler\` at \`(COLOR0.b, tintPaletteSelector.x)\` in the vertex shader (docs/binding.md). \`CEntityDef.tint\` is parsed into \`Instance::tint\` but never written to \`common_locals:tintPaletteSelector\`, so every instance uses palette row 0. Find the row formula ((tint + 0.5) / palette height?) from a capture or by comparing with the game, and set it per draw in \`world_view.rs\`."

mk "Tint palettes sampled with the wrap sampler: tinted railings and ladders render pink" "bug,rendering" "" "Repro: \`vespucci render --pos=-1120,-1620,22 --look=-1280,-1450,4 --radius 500 --pick 1150,450\` → \`vb_30_ladder_05.ydr\`, \`default_tnt\`. Its palette \`vb_30_ladder05_lod_pal\` is 256x4: column 0 holds the real colour, the rest is pink (255,127,255) filler. The \`_tnt\` vertex shader samples at (COLOR0.b, tintPaletteSelector.x) and our single anisotropic WRAP sampler blends column 0 with column 255 → pink.

Fix: bind a point + clamp sampler for \`TintPaletteSampler\` (part of applying the .fxc sampler states, #6), and set \`tintPaletteSelector.x\` per draw from the entity's tint (#12): the selector is the V coordinate, so \`(tint + 0.5) / rows\` with rows = palette height. Done when the ladders and railings in the beach-town render are grey/white and the model golden for a \`_tnt\` prop is frozen."

mk "LOD parents hide while their children are out of range: holes in the ground and vanished prop clusters" "bug,streaming" "" "Repro: the beach-town render (\`--pos=-1120,-1620,22 --look=-1280,-1450,4 --radius 500\`): sky shows through the boardwalk around 150–250 m and prop clusters are missing; \`--id-map\` shows nothing drawn there. Trace: \`vb_props_combo1208_slod_children.ydd d=170 lodDist=400 childLodDist=234 children=31 → children\` while its HD children (archetype lodDist 60–100) are all \`beyond\`.

The current rule hides a parent whenever \`d < childLodDist\`, assuming the children draw. Implement the recursive rule: an entity draws iff \`d < lodDist\` and not (\`numChildren > 0 && d < childLodDist && any child draws\`), resolving children through \`parent_index\` (same map, or the parent map when \`CEntityDef.flags\` bit 3 is set; the parent map may need loading even if outside the radius). Open question to settle with the children lists: whether a child's lodDist of -1 should inherit the parent's childLodDist rather than the archetype's lodDist (the exporter wrote childLodDist 299 for clusters whose archetypes say 60–100). Done when the beach-town and Legion Square renders have no sky holes below the horizon and the world goldens are re-frozen; likely also resolves #11."

mk "Cutout LOD billboards draw as solid quads: alpha-test reference (gAlphaRefVec0) is never set" "bug,rendering" "" "Repro: \`--pick 1025,380\` on the beach-town render → \`prop_telegraph_02b.ydr\`, material 3, \`default / lightweightHighQuality0CutOut_draw\`: a cyan panel. \`PS_Textured_Zero_CutOut\` discards when \`more_stuff.gAlphaRefVec0.x >= alpha * globalScalars.x\`; the cbuffer default is 0 so nothing is discarded and the billboard's transparent texels show their RGB.

Fix: set \`gAlphaRefVec0\`/\`gAlphaRefVec1\` in \`globals_preview\` (0.5 to start; the real value comes with the time-cycle work, #2) and check the palm \`trees\` cutouts are unaffected (they use their own \`AlphaTest\` material parameter). Done when telegraph poles, street lamps and fences at LOD distance show their billboard silhouettes."

mk "NaN pixels on trees_lod billboards (dummy CSM transform gives a zero shadow-coordinate Jacobian)" "bug,rendering" "" "Repro: the beach-town render warns \`75 pixels were NaN or infinite\`; \`--pick 680,325\` → \`vb_props_combo0212_slod_children.ydd\`, \`trees_lod / lightweightHighQuality0_draw\` (the water-reflection variant: trees/trees_lod have no other forward technique). The pixel shader computes the shadow-map texel footprint from \`dsx\`/\`dsy\` of the CSM coordinates and divides by their determinant. Our dummy \`csmshader\` transform is axis-aligned with scale 1e-3, so a camera-facing quad gets a determinant of exactly 0 → 1/0 → NaN.

Fix: give the dummy CSM mapping a rotation with no zero entries (keep the tiny scale so every shadow test still passes); goes away for good with real cascaded shadow maps (#8). Done when the NaN warning is gone on the beach-town and Legion Square renders."

mk "SLOD buildings render near-white (vb_27_slod_children, VB1_27_build_LOD)" "bug,rendering" "" "Repro: \`--pick 1050,320\` on the beach-town render → \`vb_27_slod_children.ydd\`, \`default / lightweightHighQuality0_draw\`, \`DiffuseSampler=VB1_27_build_LOD\` (found, in \`vb_27_lod.ytd\`, 256x512 DXT1). \`vespucci texture vb_27_lod.ytd\` shows a normal brown facade atlas, yet the faces come out (223,225,229). The same shader renders HD props correctly, so suspect the SLOD vertex data (COLOR0 mean r155 g139 b233, no TEXCOORD1) or the LOD mip/sampling path. Bisect with \`VESPUCCI_SET_VAR\`/\`VESPUCCI_ONES\` on \`render-model vb_27_slod_children\` with \`--ytd vb_27_lod\`. Done when distant building blocks show their LOD textures."

mk "Shaders without a forward technique are skipped (grass_fur, grass_fur_mask, trees_shadow_proxy)" "rendering" "" "\`technique_candidates()\` tries the lightweight/draw/unlit techniques; shaders that have none are skipped with a debug log. \`grass_fur*\` (fur-shell grass on terrain) is the visible one. Either map them to their own techniques (list with \`vespucci shader grass_fur --techniques\`) or decide they belong to the deferred path only."

mk "World draw order depends on heap addresses (renders differ by a few decal pixels between runs)" "bug,performance" "" "\`world_view.rs\` groups opaque draws by \`Rc::as_ptr(model)\`, so the order changes between runs and overlapping decals/coplanar surfaces resolve differently (world goldens pass at ~60 dB instead of exactly). Sort by a stable key (model hash, then instance index) so renders are bit-exact and the golden threshold can go back up."

mk "Cull with drawable and geometry bounds, not only the archetype sphere" "streaming,performance" "" "Frustum culling uses the archetype bounding sphere per instance. Drawables carry per-LOD and per-geometry AABBs (rage-formats exposes them); using them (and the entity extents of each map) would cull more and allow per-geometry culling of large ground meshes."

mk "Script-loaded map variants: replace the day/night name heuristic with the real logic" "streaming,formats" "" "Maps with \`CMapData\` flag bit 0 are loaded by scripts. Vespucci skips them except \`*_morning/_day/_evening/_night[_lod]\`, chosen by \`--time\` with guessed hour bands (streamer.rs \`wanted_variant\`). Find the real rule (which script toggles which IPL at which hour; the market at Vespucci Beach, the beach crowds, etc.) and encode it, ideally as a data file."

# ---------- streaming / performance ----------
mk "On-disk world index cache (archetypes + ymap tree)" "performance" "" "Every run rebuilds the archetype DB (159k, ~0.9 s) and the ymap tree (~0.3 s) from the game files. Cache them under \`~/.cache/vespucci/\` keyed by the game build and map set, with a \`vespucci index --rebuild\`."

mk "Stream LOD maps by level rather than by one radius" "streaming" "" "\`--radius\` decides which maps are read by entity extents. Far LODs (SLOD1–4, lodDist up to 15 km) live in \`_lod\`/\`_slod\` maps whose extents may not touch a 300 m sphere, so distant skylines and terrain are missing or partial. Stream by LOD level: HD/LOD maps near the camera, SLOD maps over the whole world (they are small), as the game's streaming does."

mk "Interiors (MLO): rooms, portals, entity sets" "formats,streaming" "After MVP" "\`CMloArchetypeDef\` and MLO instances are parsed (\`rage-formats\`) but \`include_mlo_instances\` is false and nothing inside is drawn. Place interior entities with the instance transform, honour rooms/portals for culling, and entity sets. Needed for anything indoors in the editor."

mk "Vehicles: fragments with skeletons, carcols, car generators" "formats,rendering" "After MVP" "\`.yft\` fragments render their main drawable with an identity bone palette. Vehicles need the skeleton (bone matrices into \`rage_bonemtx\`), \`carcols\`/\`carvariations\` for paint and liveries, and the \`vehshare\` textures (already in the dictionary chain). Car generators are parsed (\`entities.rs\`) and could place vehicles like the game does."

mk "Occluders, LOD lights and distant lights" "rendering" "After MVP" "Box/model occluders (\`_occl\` maps) for culling; \`LODLightsSOA\`/\`DistantLODLightsSOA\` for the night look (capture D)."

mk "Web viewer for the headless server" "mcp" "After MVP" "A small HTTP front end over the headless renderer: pick a position, get a frame; later a live view of the streamed world for a browser. Pairs with the MCP server."

# ---------- platform / project ----------
mk "Run vespucci.exe on Windows: doctor --gpu, render-model, render" "windows" "" "The \`x86_64-pc-windows-gnu\` build compiles and is checked by CI, but it has never been executed on Windows. Run \`vespucci.exe doctor --gpu\` (native D3D11 device, key recovery, archive scan), then \`render-model\` and \`render\` at the beach, and compare with the Linux goldens (\`vespucci compare\`). Note any D3D11 behaviour differences (DXVK is lenient about some state)."

mk "CI: Linux build with DXVK-native headers" "ci" "" "The CI only builds the Windows target (mingw headers). A Linux job needs DXVK-native's headers and libraries: either a cached DXVK build step (scripts/setup-linux.sh, ~20 min, cacheable by DXVK version) or a container image with it preinstalled. Then \`cargo build --release\` and \`cargo test\` on Linux run in CI too."

mk "Upstream Archetype.flags / time_flags to rage-formats and drop the vendored copy" "upstream" "" "\`third_party/rage-formats\` is rage-formats 0.4.0 with two added fields (\`Archetype::flags\`, \`Archetype::time_flags\`, see the \`ytyp.rs\` diff). Publish them in rage-formats, bump the dependency, delete \`third_party/\` and the \`[patch.crates-io]\` entry. Same for anything else that turns out to be a parser gap (e.g. geometry bounds, sampler annotations in \`.fxc\` if that ends up there)."

# ---------- M6: editor ----------
mk "Desktop shell: Dear ImGui window with a fly camera over the streamed world (Windows)" "editor,windows" "M6 — editor and MCP" "The desktop half of the project: a window (winit or SDL) with a D3D11 swap chain, Dear ImGui (imgui-rs) panels, a fly/orbit camera, and the world streamed around it as the camera moves (incremental loading/unloading instead of one shot). Reuses \`world_view\` with a persistent cache. First target: fly around Vespucci Beach at interactive rates on a real GPU."

mk "MCP server: render, probe, find and editor operations for agents (Linux and Windows)" "mcp" "M6 — editor and MCP" "An MCP server exposing what the CLI can do (render a frame at a position, probe, find/cat, shader reflection) and, once the editor exists, its operations (select, move, place, save). Headless on Linux; on Windows it can attach to the running editor. Same crate boundary as the CLI (\`vespucci\` front end over the library crates)."

mk "Editor operations: select, move/rotate/scale with gizmos, snapping, undo/redo" "editor" "M6 — editor and MCP" "Ariane's core loop: pick an entity in the viewport (ID buffer pass), transform gizmos with snapping, multi-select, duplicate/delete, undo/redo as a command stack. Every operation is also an MCP tool."

mk "Object browser with previews, and saving edits as a mod folder" "editor,formats" "M6 — editor and MCP" "Browse archetypes with thumbnail previews (render-model on a thread), drag into the world; write modified \`.ymap\` files (Meta/PSO writing exists in rage-formats) into a mod folder / DLC pack layout that the game loads."

echo "done"
