# Debugging renders

Most rendering problems so far were answered by one of three moves: skip things until the artefact disappears, override one variable at a time, or read what the shader actually does. The switches below make each of those a one-line experiment.

## Environment switches

All are read at run time by the release binary; none change the output unless set.

| Variable | Scope | Effect |
|---|---|---|
| `VESPUCCI_SKIP=a,b` | `render` | Leave out models whose file name contains any of the substrings (case-insensitive). |
| `VESPUCCI_SKIP_SHADER=decal,cpv_only` | `render` | Leave out materials whose shader has one of these names. |
| `VESPUCCI_SET_VAR=cb:var=a,b,c,d;cb2:var2=…` | `render`, `render-model` | Override a material cbuffer variable in every material, after the drawable's parameters were applied. Example: `megashader_locals:specularIntensityMult=0`. |
| `VESPUCCI_SET_GLOBAL=cb:var=a,b,c,d;…` | `render` | Override a preview global, e.g. `lighting_globals:gDirectionalColour=0,0,0,0`. |
| `VESPUCCI_ENGINE_TEX=r,g,b` | both | Colour (0–255) of the stand-in bound to engine-supplied textures. |
| `VESPUCCI_MINLOD=n`, `VESPUCCI_MAXLOD=n` | both | Clamp every sampler to a mip range (`0`/`0` = top level only). |
| `VESPUCCI_PROBE=x,y` | `render` | Print the linear HDR value of one pixel before tonemapping. |
| `VESPUCCI_TRACE_ENTITY=substring` | `render` | Log the LOD decision of every visited entity whose model name matches (`.` = every entity): `#index`, map, distance, lodDist with `(arch)` when taken from the archetype, childLodDist, linked/needed children, level, the resolved parent as `index@map` (or why it is unlinked), tint, position, the decision (`root beyond lodDist`, `children`, `leaf`, `leaf (forced beyond lodDist)`, `leaf (children not all loaded)`, `interior` for a drawn MLO placement, with `not drawn: …` when the emit stage dropped it: `unknown archetype`, `hidden at this hour`, `shadow/reflection proxy`, `interior instance` under `--no-interiors`) and `kids=[#i@map …]`. |
| `VESPUCCI_NO_STREAMING_EXTENTS=1` | `render` | Load only the maps in `--radius` (plus parents), not the maps whose streaming extents hold the camera; for A/B comparisons of the map selection. |
| `VESPUCCI_ALL_MODELS=1` | `render-model` | Also draw models with render-mask bit 0 clear (shadow proxies). |
| `VESPUCCI_ONES=<cbuffer>\|material` | `render-model` | Fill a whole global cbuffer (or all material cbuffers) with 1.0. |
| `VESPUCCI_ONES_VAR=cb:var[:index]` | `render-model` | Set one global variable (or one element) to 1.0, the rest of it to 0. |

Log levels: `--log debug` explains every skipped map (script flags, time variants), model (missing file, no drawables), material (no technique) and texture (name, binding, shader, archetype), says which dictionary each texture came from (`texture X from Y.ytd for model …`, the first time it is needed), which materials have a tint palette, and which parents within their childLodDist still miss children. `--log trace` prints one line per drawn instance: model file, distance, LOD level and distance, bounding-box size, map, entity and archetype flags, time flags, and the applied material parameters with their values.

## Workflows

**What is this pixel?** `render … --pick X,Y` names the model, geometry, material, technique, bucket and every texture binding drawn at that pixel (exact: a second pass renders draw ids). `--id-map ids.png` colours the whole frame by draw, which makes stray geometry obvious. (This identified the pink "pole" as a tinted ladder, the magenta bar as a tree-LOD billboard and the cyan panel as a telegraph pole's cutout LOD.)

**What should be drawn at this point?** `cover --at X,Y,Z [--margin M]` (no GPU) lists every entity whose world-space box contains the point, largest first, with its LOD fields and parent link; its `#index` and map match the entity trace, so a missing object can be followed from "it is in the data" to the walk's decision about it. (This mapped the Vinewood summit for #11: the three floating slabs were LOD entries of one `.ydd`, the ground under them was `ch2_03_land06` and it was drawn, so the slabs themselves were wrong.)

**The same mesh appears where different objects should be.** `--pick` two of them: when the model file and texture names are identical but `cover` says the entities are different archetypes, the model cache is handing out one entry for several. (#11: GPU models were cached by file only, so every archetype in a `*_slod_children.ydd` drew the first entry loaded; the summit's concrete pad appeared at the hut, dish and tower positions, and downtown drew palm clusters where building LODs belong.)

**Something draws where it should not.** `--log trace`, filter by distance and size to find candidates, confirm with `VESPUCCI_SKIP`. Then `VESPUCCI_TRACE_ENTITY=<name>` shows why the LOD rule kept it. (This found the 6.7 km reflection-proxy box drawn over Vespucci Beach: a script-only map.)

**A surface looks wrong.** Bisect the shader: `VESPUCCI_SKIP_SHADER` to find the material, then `VESPUCCI_SET_VAR` on its parameters one by one, then `VESPUCCI_SET_GLOBAL` for the lighting terms. If nothing changes the look, it is not that material: check for a second geometry drawing over it (`render-model --log debug` lists each model's render mask and vertex-colour statistics). (This found the shadow-proxy models drawn over palm trunks.)

**Black or magenta.** Magenta is a missing texture (the report counts them; `--log debug` names them) or a NaN/Inf pixel (a warning gives the count; `VESPUCCI_PROBE` reads the value). Black usually means a global the shader multiplies by is zero; `VESPUCCI_ONES_VAR` on the suspects, one at a time, is how `globalScalars3.z`, the fog parameters and the AO factor were found.

**Is the texture itself right?** `vespucci texture NAME --out dir` decodes it on the CPU; compare with what the GPU samples by pinning `VESPUCCI_MINLOD`/`MAXLOD`. (This ruled out mip-chain corruption on the palm bark.)

**What does the shader do with it?** `vespucci shader NAME --reflect` for bindings and layouts, `--blob ps:N --out x.dxbc` then `vkd3d-compiler -b d3d-asm x.dxbc -o x.asm` (Linux) or `python scripts/disasm.py x.dxbc > x.asm` (Windows, uses the system `d3dcompiler_47.dll`) for the code. Look for the `sample` of the texture slot and follow the registers. Most facts in [binding.md](binding.md) came from this. (This settled how `_tnt` palettes are addressed: `COLOR0.b` picks the column, `tintPaletteSelector.x` the row.)

**Which entities changed between two runs?** `VESPUCCI_TRACE_ENTITY=. … 2>&1 | rg "entity #" > a.log` twice (for example once with `VESPUCCI_NO_STREAMING_EXTENTS=1`) and diff the `leaf` lines by `#index`, map and name; the `kids=[…]` list on a parent shows which children were linked. (This showed that a skyline "lost" by the streaming-extent map selection was SLOD1 pieces 600 m away that the game would not stream either.)

**One model in isolation.** `vespucci render-model NAME --technique lightweight0_draw --dump-binding b.json` renders it with an orbit camera and writes every cbuffer, texture and sampler binding, which parameters matched, and which inputs got dummy slots.

## Regression tests

`scripts/golden.sh` renders three props and two world scenes and compares them with `tests/golden/*.png` by PSNR (`vespucci compare`). Run it after any change in `vespucci-render` or the vendored parser; use `--update` only for intentional changes, and look at the before/after images.

## Reading the report

```
964 instances (615 culled), 120 models loaded (0 failed), 984 draws, 13.3 MB geometry + 50.2 MB textures,
textures bound 1563 / missing 0 / engine 588, peak RSS 775 MB, 4.6 s total
```

- *instances*: candidates from the streamer; *culled*: outside the view frustum.
- *models loaded / failed*: distinct model files; failures are logged at `debug` with the reason.
- *draws*: geometry draw calls across all passes.
- *textures bound / missing / engine*: texture bindings resolved to a game texture / not found anywhere (magenta) / engine-supplied inputs with a stand-in.
- *budget hit*: loading stopped at `--budget-mb`; raise it or `--mip-skip`.
