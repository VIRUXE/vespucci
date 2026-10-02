# Third-party code and data

| What | Origin | Licence | Where in Vespucci |
|---|---|---|---|
| `rpf-archive`, `rage-formats`, `rage-render` crates | VIRUXE (github.com/VIRUXE) | Unlicense (public domain) | Cargo dependencies |
| Archive wrapper (`rpf.rs`), archive ranking and DLC load order (`index.rs`), key caching and exe version (`keys.rs`) | rage-cli, VIRUXE | Unlicense (public domain) | `crates/vespucci-game/src/{archive,load_order,keys}.rs`, adapted |
| D3D11/DXGI headers used for bindings | DXVK-native (`include/native`, mingw-w64 derived) / mingw-w64 | zlib (DXVK) / mingw-w64 licences | consumed at build time by `crates/vespucci-d3d11/build.rs` |
| DXVK-native runtime | doitsujin/dxvk v3.1.1 | zlib | `/opt/dxvk-native`, Linux headless only |

CodeWalker (dexyfex) has no licence and is used only as reference material; no code from it is included.
