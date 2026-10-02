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
| `VESPUCCI_TRACE_ENTITY=substring` | `render` | Log the LOD decision (distance, lodDist with `(arch)` when taken from the archetype, childLodDist, children, level, parent, tint, map) of every entity whose model name matches; empty = every entity. |
| `VESPUCCI_ALL_MODELS=1` | `render-model` | Also draw models with render-mask bit 0 clear (shadow proxies). |
| `VESPUCCI_ONES=<cbuffer>\|material` | `render-model` | Fill a whole global cbuffer (or all material cbuffers) with 1.0. |
| `VESPUCCI_ONES_VAR=cb:var[:index]` | `render-model` | Set one global variable (or one element) to 1.0, the rest of it to 0. |

Log levels: `--log debug` explains every skipped map (script flags, time variants), model (missing file, no drawables), material (no technique) and texture (name, binding, shader, archetype). `--log trace` prints one line per drawn instance: model file, distance, LOD level and distance, bounding-box size, map, entity and archetype flags, time flags, and the applied material parameters with their values.

## Workflows

**What is this pixel?** `render … --pick X,Y` names the model, geometry, material, technique, bucket and every texture binding drawn at that pixel (exact: a second pass renders draw ids). `--id-map ids.png` colours the whole frame by draw, which makes stray geometry obvious. (This identified the pink "pole" as a tinted ladder, the magenta bar as a tree-LOD billboard and the cyan panel as a telegraph pole's cutout LOD.)

**Something draws where it should not.** `--log trace`, filter by distance and size to find candidates, confirm with `VESPUCCI_SKIP`. Then `VESPUCCI_TRACE_ENTITY=<name>` shows why the LOD rule kept it. (This found the 6.7 km reflection-proxy box drawn over Vespucci Beach: a script-only map.)

**A surface looks wrong.** Bisect the shader: `VESPUCCI_SKIP_SHADER` to find the material, then `VESPUCCI_SET_VAR` on its parameters one by one, then `VESPUCCI_SET_GLOBAL` for the lighting terms. If nothing changes the look, it is not that material: check for a second geometry drawing over it (`render-model --log debug` lists each model's render mask and vertex-colour statistics). (This found the shadow-proxy models drawn over palm trunks.)

**Black or magenta.** Magenta is a missing texture (the report counts them; `--log debug` names them) or a NaN/Inf pixel (a warning gives the count; `VESPUCCI_PROBE` reads the value). Black usually means a global the shader multiplies by is zero; `VESPUCCI_ONES_VAR` on the suspects, one at a time, is how `globalScalars3.z`, the fog parameters and the AO factor were found.

**Is the texture itself right?** `vespucci texture NAME --out dir` decodes it on the CPU; compare with what the GPU samples by pinning `VESPUCCI_MINLOD`/`MAXLOD`. (This ruled out mip-chain corruption on the palm bark.)

**What does the shader do with it?** `vespucci shader NAME --reflect` for bindings and layouts, `--blob ps:N --out x.dxbc` then `vkd3d-compiler -b d3d-asm x.dxbc -o x.asm` for the code. Look for the `sample` of the texture slot and follow the registers. Most facts in [binding.md](binding.md) came from this.

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
