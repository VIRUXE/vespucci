# Third-party code and data

| What | Origin | Licence | Where in Vespucci |
|---|---|---|---|
| `rpf-archive`, `rage-formats`, `rage-render` crates | VIRUXE (github.com/VIRUXE) | Unlicense (public domain) | Cargo dependencies; `rage-formats` 0.4.0 is vendored in `third_party/rage-formats` (via `[patch.crates-io]`) with two added `Archetype` fields, `flags` and `time_flags` — worth offering upstream |
| Archive wrapper (`rpf.rs`), archive ranking and DLC load order (`index.rs`), key caching and exe version (`keys.rs`) | rage-cli, VIRUXE | Unlicense (public domain) | `crates/vespucci-game/src/{archive,load_order,keys}.rs`, adapted |
| D3D11/DXGI headers used for bindings | DXVK-native (`include/native`, mingw-w64 derived) / mingw-w64 | zlib (DXVK) / mingw-w64 licences | consumed at build time by `crates/vespucci-d3d11/build.rs` |
| DXVK-native runtime | doitsujin/dxvk v3.1.1 | zlib | `/opt/dxvk-native`, Linux headless only |

CodeWalker (dexyfex) has no licence and is used only as reference material; no code from it is included.

The `rpf-archive`, `rage-formats` and `rage-render` crates are by the same author as Vespucci (VIRUXE), which is why the vendored copy with the two extra fields is a temporary measure rather than a fork: the change belongs upstream.

`scripts/setup-linux.sh` builds [DXVK](https://github.com/doitsujin/dxvk) (zlib licence) from source on the user's machine; nothing of it is redistributed here.
