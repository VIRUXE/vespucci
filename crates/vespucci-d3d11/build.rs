//! Generates Rust bindings for D3D11/DXGI and links the implementation.
//!
//! Linux (headless): DXVK-native's headers and libraries.
//!   Header lookup: `VESPUCCI_DXVK_INCLUDE` (dir with d3d11.h, or `a:b` for the
//!   source tree's `windows` and `directx` dirs), else `/opt/dxvk-native/include/dxvk`,
//!   else `/root/src/dxvk-native/include/native/{windows,directx}`.
//!   Library lookup: `VESPUCCI_DXVK_LIB`, else `/opt/dxvk-native/lib/x86_64-linux-gnu`, else `/opt/dxvk-native/lib`.
//!
//! Windows (desktop): mingw-w64's headers (same mingw-derived C vtables) and the
//! system `d3d11`/`dxgi` import libraries. Cross-compiling from Linux uses
//! `/usr/x86_64-w64-mingw32/include`; a native Windows build uses the toolchain's
//! own include path (set `VESPUCCI_D3D_INCLUDE` if bindgen cannot find d3d11.h).

use std::env;
use std::path::{Path, PathBuf};

fn linux_include_dirs() -> Vec<PathBuf> {
    if let Ok(v) = env::var("VESPUCCI_DXVK_INCLUDE") {
        return v.split(':').map(PathBuf::from).collect();
    }
    let installed = PathBuf::from("/opt/dxvk-native/include/dxvk");
    if installed.join("d3d11.h").exists() {
        return vec![installed];
    }
    let src = Path::new("/root/src/dxvk-native/include/native");
    vec![src.join("windows"), src.join("directx")]
}

fn windows_include_dirs() -> Vec<PathBuf> {
    if let Ok(v) = env::var("VESPUCCI_D3D_INCLUDE") {
        return v.split(':').map(PathBuf::from).collect();
    }
    let mingw = PathBuf::from("/usr/x86_64-w64-mingw32/include");
    if mingw.join("d3d11.h").exists() {
        return vec![mingw];
    }
    vec![]
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wrapper.h");
    for v in ["VESPUCCI_DXVK_INCLUDE", "VESPUCCI_DXVK_LIB", "VESPUCCI_D3D_INCLUDE"] {
        println!("cargo:rerun-if-env-changed={v}");
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let windows = target_os == "windows";
    let includes = if windows { windows_include_dirs() } else { linux_include_dirs() };

    let mut builder = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_args(["-x", "c", "-std=c11"])
        .clang_args(includes.iter().map(|d| format!("-I{}", d.display())))
        .use_core()
        .layout_tests(false)
        .generate_comments(false)
        .prepend_enum_name(false)
        .default_enum_style(bindgen::EnumVariation::Consts)
        .derive_default(true)
        .derive_debug(false)
        .allowlist_type("(ID3D11|IDXGI|IUnknown|D3D11_|DXGI_|D3D_|GUID|HRESULT|IID|REFIID|LUID).*")
        .allowlist_function("D3D11CreateDevice|CreateDXGIFactory1?")
        .allowlist_var("(D3D11_|DXGI_|D3D_).*")
        .blocklist_type("max_align_t");
    if windows {
        // Cross-compiling: tell clang about the target and mingw's header quirks.
        builder = builder.clang_args(["--target=x86_64-w64-mingw32", "-D_WIN32_WINNT=0x0A00", "-DCINTERFACE", "-DCOBJMACROS"]);
    }
    let bindings = builder.generate().expect("bindgen failed on D3D11 headers");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    bindings.write_to_file(&out).expect("write bindings.rs");

    if windows {
        println!("cargo:rustc-link-lib=d3d11");
        println!("cargo:rustc-link-lib=dxgi");
    } else {
        let lib = env::var("VESPUCCI_DXVK_LIB").unwrap_or_else(|_| {
            // meson installs into the multiarch libdir on Debian.
            let multiarch = "/opt/dxvk-native/lib/x86_64-linux-gnu";
            if Path::new(multiarch).join("libdxvk_d3d11.so").exists() { multiarch.into() } else { "/opt/dxvk-native/lib".into() }
        });
        println!("cargo:rustc-link-search=native={lib}");
        println!("cargo:rustc-link-lib=dylib=dxvk_d3d11");
        println!("cargo:rustc-link-lib=dylib=dxvk_dxgi");
    }
}
