# Contributing to Vespucci

Thanks for taking a look. Vespucci is early (the render core is at milestone M4 of 6, see [PLAN.md](PLAN.md)), so the most useful contributions are bug reports with a reproducible camera position, verified facts about the game's formats and shaders, and fixes with a golden test.

## Ground rules

- **Retail game files only.** Everything Vespucci knows about GTA V comes from reading a legally owned install and from publicly documented research. Do not contribute anything derived from leaked source code, and do not commit game assets (archives, models, textures, shaders, extracted XML). Rendered screenshots of your own game are fine.
- **No Rockstar / Take-Two trademarks** in names of crates, commands or files.
- **Facts go in `docs/`.** When you establish how a format or shader binding really behaves (by disassembly, by comparing with the game, by reading the files), write it down in [docs/binding.md](docs/binding.md) or [docs/formats.md](docs/formats.md) with how it was verified. Those files are the project's memory.
- **Third-party code** is listed in [docs/THIRD_PARTY.md](docs/THIRD_PARTY.md) with its licence. CodeWalker has no licence and is used only as reference material; no code from it is included.

## Building and testing

See [docs/building.md](docs/building.md). The short version on Linux:

```sh
scripts/setup-linux.sh            # apt packages, DXVK-native, lavapipe (once)
cargo build --release
export GTAV_PATH=/path/to/gtav
./target/release/vespucci doctor --gpu
```

Before opening a pull request:

```sh
cargo fmt --all
cargo build --release
cargo build --release --target x86_64-pc-windows-gnu     # the Windows build must keep compiling
VESPUCCI_GAME=$GTAV_PATH cargo test --release             # unit + game-file tests
scripts/golden.sh                                         # render goldens (PSNR thresholds)
```

If a change intentionally alters rendering, regenerate the goldens with `scripts/golden.sh --update` and say so in the pull request, with before/after images.

## Reporting a rendering problem

Include the command line (position, look-at, radius, time), the game build (`vespucci doctor` prints it), your platform (Linux/lavapipe or Windows/GPU), and the output of `--log debug` if it is short. For a single object, `vespucci render-model NAME --dump-binding b.json` and the JSON are usually enough to see what was bound. The debug switches in [docs/debugging.md](docs/debugging.md) help bisect which model, shader or variable is responsible.

## Style

Plain Rust 2021, `cargo fmt` defaults, no `unsafe` outside `vespucci-d3d11`. Comments explain *why* and what was verified; the code shows *what*. Keep a module's doc comment (`//!`) accurate.
