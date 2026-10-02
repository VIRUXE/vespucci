# Command-line reference

```
vespucci [--game DIR] [--log LEVEL] [--mapset sp|mp|all] <COMMAND>
```

Global options, accepted before or after the subcommand:

| Option | Meaning |
|---|---|
| `--game DIR` | The GTA V install (folder with `GTA5.exe`). Default: `$GTAV_PATH`. |
| `--log error\|warn\|info\|debug\|trace` | Log level on stderr (default `info`; `$VESPUCCI_LOG` sets the default). `debug` explains every skipped map, model and material; `trace` prints one line per drawn instance. |
| `--mapset sp\|mp\|all` | Which maps count as loaded: story mode (default), GTA Online, or every map file regardless of DLC change sets. Affects `index`, `probe`, `render`. |

All positions are game world coordinates (metres, Z up). Triples are written `X,Y,Z`; negative values need `--pos=-1280,-1450,4` (with `=`) so the shell parser does not read them as flags.

---

## `doctor` — check the environment

```
vespucci doctor [--gpu] [--out FILE.png] [--size N]
```

Checks the game folder, the exe build, key derivation, that `common.rpf` opens and `dlclist.xml` parses; on Linux also the Vulkan ICD and the DXVK libraries. With `--gpu` it creates a D3D11 device, draws a triangle, reads the pixels back, verifies them and times the first (JIT) and second draw. Non-zero exit on any failure. Run this first on a new machine.

## `ls` — list game files

```
vespucci ls [PATH] [-R] [-l]
```

Lists the virtual file system: all `.rpf` archives in load order (base game, then DLC packs in `dlclist.xml` order, nested archives mounted in place). `PATH` is `archive/dir`, e.g. `x64c.rpf/levels/gta5/props` or `update/update.rpf/common/data`. `-R` recurses, `-l` shows disk size, memory size and kind (binary/resource).

## `cat` — extract one file

```
vespucci cat PATH [-o FILE]
```

Decrypts and decompresses a file to stdout or `--out`. Resources (`.ydr`, `.ytd`, …) are written without the RSC7 header. Example: `vespucci cat update/update.rpf/common/data/dlclist.xml`.

## `find` — locate files

```
vespucci find [GLOB] [--ext EXT] [--name NAME] [--hash HEX] [-l]
```

Three modes: a glob on the file name (`find 'prop_cs_heist_bag*'`); the exact name of a type (`find --ext ydr --name prop_cs_heist_bag_01`) returning **the file the game would load** (last archive in load order wins); or a JOAAT stem hash (`find --ext ytd --hash 0x1a2b3c4d`).

## `index` — build the world index

```
vespucci index
```

Builds the archetype database (every `.ytyp`) and the ymap tree (from the game's `*_cache_y.dat` files, falling back to parsing ymap headers) for the selected map set, and prints counts, timings and memory. Useful to check a game install and to compare `--mapset sp` against `mp`.

## `probe` — what a camera would stream

```
vespucci probe --pos X,Y,Z [--radius 300] [--json]
```

No GPU. Lists the ymaps whose entity extents touch the sphere, with entity counts per LOD level and content flags, plus the total model data they reference. Use it to understand what `render` will load at a spot, or to find map names for a region.

## `render` — draw the world

```
vespucci render --pos X,Y,Z --look X,Y,Z [options] --out frame.png
```

| Option | Default | Meaning |
|---|---|---|
| `--fov DEG` | 50 | Vertical field of view. |
| `--size WxH` | 1280x720 | Output size. |
| `--radius M` | 300 | Maps touching this sphere around the camera are streamed. Larger radii bring in more distant LODs. |
| `--lod-scale F` | 1 | Multiplies every entity's LOD distances (2 = things stay high-detail twice as far). |
| `--lighting basic\|none` | basic | `basic`: the game's forward-lit techniques with a preview sun and ambient. `none`: `unlit_draw` (texture × vertex colour, no lighting). |
| `--time HH:MM` | 12:00 | Hour for time-dependent objects (`CTimeArchetypeDef` hour bits) and for picking the day/night variants of script-loaded maps. |
| `--script-maps` | off | Also stream maps the game loads only on a script's request (mission props, reflection proxies). |
| `--max-draws N` | 20000 | Stop adding instances past this many. |
| `--budget-mb N` | 1500 | Stop loading new models past this much geometry + texture data on the device. |
| `--mip-skip a,b,c` | 0,1,2 | Mip levels dropped for models nearer than 100 m, nearer than 500 m, and beyond. |
| `--exposure F` | 1 | Scale applied to the linear HDR frame before the display curve. |
| `--flip-sun` | off | Flip the preview sun direction. |
| `--pick X,Y` | — | Print the model, geometry, material and textures drawn at that pixel (a second pass writes draw ids). |
| `--id-map FILE` | — | Also write a PNG with one colour per draw, black where nothing was drawn (same picking pass). |
| `--json` | off | Print the report as one JSON object instead of text. |

The report gives instances kept and culled, models loaded, draw calls, bytes of geometry and textures, textures bound / missing / engine-supplied, peak RSS and wall time. Exit tests and examples:

```sh
# Vespucci Beach boardwalk
vespucci render --pos=-1280,-1450,4 --look=-1200,-1500,4 --out beach.png
# Elevated view over the beach, sharper textures, wider streaming
vespucci render --pos=-1330,-1460,14 --look=-1200,-1500,6 --radius 600 --mip-skip 0,0,1 --out beach_high.png
# Vinewood sign, narrower lens
vespucci render --pos=600,1262,332 --look=711,1198,352 --fov 40 --radius 500 --out vinewood.png
# Downtown with far LODs streamed in
vespucci render --pos=-420,-640,50 --look=-75,-818,130 --radius 1500 --max-draws 12000 --budget-mb 3000 --out downtown.png
```

## `render-model` — draw one model

```
vespucci render-model MODEL [--entry NAME] [--ytd DICT ...] [--lod high|med|low|vlow]
                      [--technique T ...] [--size WxH] [--yaw DEG] [--pitch DEG]
                      [--cull] [--wireframe] [--mip-skip N] [--transpose] [--flip-sun]
                      [--dump-binding FILE.json] --out model.png
```

`MODEL` is a name (`prop_cs_heist_bag_01`) or a game path. For `.ydd`/`.yft`, `--entry` picks the drawable. Textures come from the model's embedded dictionary and the archetype's dictionary chain; `--ytd` adds more (earlier wins; `mapdetail` holds the shared detail maps). `--technique` lists candidates most-wanted first (repeat the flag per candidate), default `unlit_draw` then `draw`; use `lightweightHighQuality0_draw` or `lightweight0_draw` for the lit look. The camera orbits the bounding sphere. Output is linear (no tonemap), so lit renders look dark; this command exists to check binding, not beauty. `--dump-binding` writes every cbuffer, texture and sampler binding and which material parameters matched, the first thing to read when a model looks wrong.

## `texture` — dump textures as PNG

```
vespucci texture WHAT [--out DIR]
```

`WHAT` is a `.ytd` name or path, or a model whose embedded dictionary is dumped. Decodes BC1–BC7 and the uncompressed formats on the CPU (top mip only) and writes one PNG per texture.

## `compare` — compare two PNGs

```
vespucci compare A.png B.png [--min-psnr N]
```

Prints PSNR and the share of differing pixels; with `--min-psnr` exits 1 below the threshold. This is what the golden tests use.

## `shader` — inspect a game shader

```
vespucci shader NAME [--techniques] [--vars] [--reflect] [--blob STAGE:INDEX --out FILE]
vespucci shader --all
```

Reads one of the game's `.fxc` containers (by name, e.g. `normal_spec`; the current files live in `update/update2.rpf/common/shaders/win32_40_final/`). `--techniques` lists techniques and the VS/PS/GS/HS/DS each pass uses (default). `--vars` lists constant buffers, variables with defaults, textures and samplers as the `.fxc` declares them. `--reflect` parses every DXBC blob's RDEF/ISGN: bindings with slots, cbuffer layouts with byte offsets, vertex inputs. `--blob ps:4 --out x.dxbc` writes one blob, which `vkd3d-compiler -b d3d-asm x.dxbc` disassembles. `--all` parses and reflects every shader in the game and reports failures (currently 696 files / 21,374 blobs, none failing).

---

## Environment variables

| Variable | Meaning |
|---|---|
| `GTAV_PATH` | Default for `--game`. |
| `VESPUCCI_LOG` | Default for `--log`. |
| `VESPUCCI_GAME` | Enables the game-file tests in `cargo test`. |
| `DXVK_*`, `VK_DRIVER_FILES`, `SDL_VIDEO_DRIVER`, `MESA_SHADER_CACHE_DIR`, `LP_NUM_THREADS` | Set by the binary for headless use unless already exported; see [building.md](building.md). |
| `VESPUCCI_SKIP`, `VESPUCCI_SKIP_SHADER`, `VESPUCCI_SET_VAR`, `VESPUCCI_SET_GLOBAL`, `VESPUCCI_ENGINE_TEX`, `VESPUCCI_MINLOD`, `VESPUCCI_MAXLOD`, `VESPUCCI_PROBE`, `VESPUCCI_TRACE_ENTITY`, `VESPUCCI_ALL_MODELS`, `VESPUCCI_ONES`, `VESPUCCI_ONES_VAR` | Debug switches, see [debugging.md](debugging.md). |
