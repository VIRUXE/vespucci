//! D3D11 on Linux through DXVK-native.
//!
//! `ffi` is the raw bindgen output over DXVK's own `d3d11.h`/`dxgi.h`, with C
//! vtables. Everything else here is a thin safe layer: reference-counted COM
//! pointers, device creation on a headless (offscreen SDL3) WSI, resources,
//! shaders, drawing and CPU readback. Only what the renderer needs is wrapped;
//! anything else is reachable through `ffi` and [`com_call!`].

pub mod ffi {
    #![allow(
        non_camel_case_types,
        non_snake_case,
        non_upper_case_globals,
        dead_code,
        clippy::all
    )]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use anyhow::{anyhow, bail, Context, Result};
use core::ffi::c_void;
use core::ptr::{null, null_mut, NonNull};
use ffi::*;

/// Calls a COM method through the object's vtable: `com_call!(ptr, Method, arg, ...)`.
#[macro_export]
macro_rules! com_call {
    ($this:expr, $method:ident $(, $arg:expr)* $(,)?) => {{
        let this = $this;
        ((*(*this).lpVtbl).$method.expect(concat!("missing vtable entry ", stringify!($method))))(this $(, $arg)*)
    }};
}

/// Owning COM pointer; `Release` on drop. `T` must start with an `IUnknown` vtable.
pub struct ComPtr<T>(NonNull<T>);

impl<T> ComPtr<T> {
    /// Takes ownership of a pointer that already holds a reference.
    pub fn from_raw(p: *mut T) -> Option<Self> {
        NonNull::new(p).map(ComPtr)
    }
    pub fn as_ptr(&self) -> *mut T {
        self.0.as_ptr()
    }
    /// Another owning reference to the same object (`AddRef`).
    pub fn clone_ptr(&self) -> ComPtr<T> {
        unsafe {
            com_call!(self.as_ptr() as *mut IUnknown, AddRef);
        }
        ComPtr(self.0)
    }
    /// The same object as another interface, via `QueryInterface`.
    pub fn query<U>(&self, iid: &GUID) -> Result<ComPtr<U>> {
        let mut out: *mut c_void = null_mut();
        let hr = unsafe {
            com_call!(
                self.as_ptr() as *mut IUnknown,
                QueryInterface,
                iid,
                &mut out
            )
        };
        check(hr, "QueryInterface")?;
        ComPtr::from_raw(out as *mut U).ok_or_else(|| anyhow!("QueryInterface returned null"))
    }
}

impl<T> Drop for ComPtr<T> {
    fn drop(&mut self) {
        unsafe {
            com_call!(self.as_ptr() as *mut IUnknown, Release);
        }
    }
}

pub fn check(hr: HRESULT, what: &str) -> Result<()> {
    if hr < 0 {
        bail!("{what} failed: HRESULT {hr:#010x}");
    }
    Ok(())
}

pub const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> GUID {
    GUID {
        Data1: a,
        Data2: b,
        Data3: c,
        Data4: d,
    }
}
pub const IID_IDXGIDEVICE: GUID = guid(
    0x54ec77fa,
    0x1377,
    0x44e6,
    [0x8c, 0x32, 0x88, 0xfd, 0x5f, 0x44, 0xc8, 0x4c],
);
pub const IID_IDXGIADAPTER: GUID = guid(
    0x2411e7e1,
    0x12ac,
    0x4ccf,
    [0xbd, 0x14, 0x97, 0x98, 0xe8, 0x53, 0x4d, 0xc0],
);

/// Environment DXVK-native needs to run headless on lavapipe. Values already
/// set in the environment win, so a caller can point at another driver.
/// On Windows D3D11 is native and this does nothing.
#[cfg(windows)]
pub fn setup_env() {}

#[cfg(not(windows))]
pub fn setup_env() {
    let defaults = [
        ("DXVK_WSI_DRIVER", "SDL3"),
        ("SDL_VIDEO_DRIVER", "offscreen"),
        ("VK_DRIVER_FILES", "/usr/share/vulkan/icd.d/lvp_icd.json"),
        ("DXVK_FILTER_DEVICE_NAME", "llvmpipe"),
        ("DXVK_LOG_LEVEL", "error"),
        ("DXVK_LOG_PATH", "none"),
        ("LP_NUM_THREADS", "4"),
    ];
    let cache = std::env::var("HOME")
        .map(|h| format!("{h}/.cache/vespucci"))
        .unwrap_or_else(|_| "/tmp/vespucci".into());
    for (k, v) in defaults {
        if std::env::var_os(k).is_none() {
            std::env::set_var(k, v);
        }
    }
    for (k, sub) in [
        ("DXVK_STATE_CACHE_PATH", "dxvk"),
        ("MESA_SHADER_CACHE_DIR", "mesa"),
    ] {
        if std::env::var_os(k).is_none() {
            let dir = format!("{cache}/{sub}");
            let _ = std::fs::create_dir_all(&dir);
            std::env::set_var(k, dir);
        }
    }
}

pub struct Device {
    pub dev: ComPtr<ID3D11Device>,
    pub ctx: ComPtr<ID3D11DeviceContext>,
    pub feature_level: D3D_FEATURE_LEVEL,
}

pub struct Texture2D {
    pub tex: ComPtr<ID3D11Texture2D>,
    pub width: u32,
    pub height: u32,
    pub format: DXGI_FORMAT,
}

pub struct RenderTarget {
    pub texture: Texture2D,
    pub rtv: ComPtr<ID3D11RenderTargetView>,
}

pub struct InputElement {
    pub semantic: std::ffi::CString,
    pub index: u32,
    pub format: DXGI_FORMAT,
    pub slot: u32,
    pub offset: u32,
    /// Vertex-buffer stride 0 with this element reads the same value for every vertex.
    pub per_vertex: bool,
}

impl InputElement {
    pub fn new(semantic: &str, index: u32, format: DXGI_FORMAT, slot: u32, offset: u32) -> Self {
        InputElement {
            semantic: std::ffi::CString::new(semantic).unwrap(),
            index,
            format,
            slot,
            offset,
            per_vertex: true,
        }
    }
}

pub struct DepthTarget {
    pub texture: Texture2D,
    pub dsv: ComPtr<ID3D11DepthStencilView>,
}

impl Device {
    /// Creates the device on whatever adapter the Vulkan loader offers first
    /// (lavapipe, given [`setup_env`]).
    pub fn create() -> Result<Device> {
        setup_env();
        let levels = [D3D_FEATURE_LEVEL_11_0];
        let mut dev: *mut ID3D11Device = null_mut();
        let mut ctx: *mut ID3D11DeviceContext = null_mut();
        let mut got: D3D_FEATURE_LEVEL = 0;
        let hr = unsafe {
            D3D11CreateDevice(
                null_mut(),
                D3D_DRIVER_TYPE_HARDWARE,
                null_mut(),
                0,
                levels.as_ptr(),
                levels.len() as u32,
                D3D11_SDK_VERSION,
                &mut dev,
                &mut got,
                &mut ctx,
            )
        };
        check(hr, "D3D11CreateDevice")?;
        Ok(Device {
            dev: ComPtr::from_raw(dev).context("null device")?,
            ctx: ComPtr::from_raw(ctx).context("null context")?,
            feature_level: got,
        })
    }

    /// Adapter name as DXGI reports it, e.g. "llvmpipe (LLVM 21.1.8, 256 bits)".
    pub fn adapter_description(&self) -> Result<String> {
        let dxgi: ComPtr<IDXGIDevice> = self.dev.query(&IID_IDXGIDEVICE)?;
        let mut adapter: *mut IDXGIAdapter = null_mut();
        check(
            unsafe { com_call!(dxgi.as_ptr(), GetAdapter, &mut adapter) },
            "IDXGIDevice::GetAdapter",
        )?;
        let adapter = ComPtr::from_raw(adapter).context("null adapter")?;
        let mut desc: DXGI_ADAPTER_DESC = unsafe { core::mem::zeroed() };
        check(
            unsafe { com_call!(adapter.as_ptr(), GetDesc, &mut desc) },
            "IDXGIAdapter::GetDesc",
        )?;
        let name: Vec<u16> = desc
            .Description
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u16)
            .collect();
        Ok(String::from_utf16_lossy(&name))
    }

    pub fn create_texture2d(
        &self,
        desc: &D3D11_TEXTURE2D_DESC,
        init: Option<&D3D11_SUBRESOURCE_DATA>,
    ) -> Result<Texture2D> {
        let mut tex: *mut ID3D11Texture2D = null_mut();
        let init_ptr = init.map_or(null(), |d| d as *const _);
        check(
            unsafe { com_call!(self.dev.as_ptr(), CreateTexture2D, desc, init_ptr, &mut tex) },
            "CreateTexture2D",
        )?;
        Ok(Texture2D {
            tex: ComPtr::from_raw(tex).context("null texture")?,
            width: desc.Width,
            height: desc.Height,
            format: desc.Format,
        })
    }

    pub fn create_render_target(
        &self,
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
    ) -> Result<RenderTarget> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let texture = self.create_texture2d(&desc, None)?;
        let mut rtv: *mut ID3D11RenderTargetView = null_mut();
        let res = texture.tex.as_ptr() as *mut ID3D11Resource;
        check(
            unsafe {
                com_call!(
                    self.dev.as_ptr(),
                    CreateRenderTargetView,
                    res,
                    null(),
                    &mut rtv
                )
            },
            "CreateRenderTargetView",
        )?;
        Ok(RenderTarget {
            texture,
            rtv: ComPtr::from_raw(rtv).context("null rtv")?,
        })
    }

    pub fn create_buffer(
        &self,
        data: &[u8],
        bind: D3D11_BIND_FLAG,
        usage: D3D11_USAGE,
    ) -> Result<ComPtr<ID3D11Buffer>> {
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: data.len() as u32,
            Usage: usage,
            BindFlags: bind as u32,
            CPUAccessFlags: if usage == D3D11_USAGE_DYNAMIC {
                D3D11_CPU_ACCESS_WRITE as u32
            } else {
                0
            },
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let init = D3D11_SUBRESOURCE_DATA {
            pSysMem: data.as_ptr() as *const c_void,
            SysMemPitch: 0,
            SysMemSlicePitch: 0,
        };
        let mut buf: *mut ID3D11Buffer = null_mut();
        check(
            unsafe { com_call!(self.dev.as_ptr(), CreateBuffer, &desc, &init, &mut buf) },
            "CreateBuffer",
        )?;
        ComPtr::from_raw(buf).context("null buffer")
    }

    pub fn create_vertex_shader(&self, dxbc: &[u8]) -> Result<ComPtr<ID3D11VertexShader>> {
        let mut vs: *mut ID3D11VertexShader = null_mut();
        check(
            unsafe {
                com_call!(
                    self.dev.as_ptr(),
                    CreateVertexShader,
                    dxbc.as_ptr() as *const c_void,
                    dxbc.len() as _,
                    null_mut(),
                    &mut vs
                )
            },
            "CreateVertexShader",
        )?;
        ComPtr::from_raw(vs).context("null vertex shader")
    }

    pub fn create_pixel_shader(&self, dxbc: &[u8]) -> Result<ComPtr<ID3D11PixelShader>> {
        let mut ps: *mut ID3D11PixelShader = null_mut();
        check(
            unsafe {
                com_call!(
                    self.dev.as_ptr(),
                    CreatePixelShader,
                    dxbc.as_ptr() as *const c_void,
                    dxbc.len() as _,
                    null_mut(),
                    &mut ps
                )
            },
            "CreatePixelShader",
        )?;
        ComPtr::from_raw(ps).context("null pixel shader")
    }

    /// Input layout validated against the vertex shader's input signature (DXVK reads ISGN from `vs_dxbc`).
    pub fn create_input_layout(
        &self,
        elements: &[InputElement],
        vs_dxbc: &[u8],
    ) -> Result<ComPtr<ID3D11InputLayout>> {
        let descs: Vec<D3D11_INPUT_ELEMENT_DESC> = elements
            .iter()
            .map(|e| D3D11_INPUT_ELEMENT_DESC {
                SemanticName: e.semantic.as_ptr(),
                SemanticIndex: e.index,
                Format: e.format,
                InputSlot: e.slot,
                AlignedByteOffset: e.offset,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            })
            .collect();
        let mut il: *mut ID3D11InputLayout = null_mut();
        check(
            unsafe {
                com_call!(
                    self.dev.as_ptr(),
                    CreateInputLayout,
                    descs.as_ptr(),
                    descs.len() as u32,
                    vs_dxbc.as_ptr() as *const c_void,
                    vs_dxbc.len() as _,
                    &mut il
                )
            },
            "CreateInputLayout",
        )?;
        ComPtr::from_raw(il).context("null input layout")
    }

    pub fn clear(&self, rt: &RenderTarget, rgba: [f32; 4]) {
        unsafe {
            com_call!(
                self.ctx.as_ptr(),
                ClearRenderTargetView,
                rt.rtv.as_ptr(),
                rgba.as_ptr()
            )
        }
    }

    pub fn bind_render_target(&self, rt: &RenderTarget) {
        let rtvs = [rt.rtv.as_ptr()];
        let vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: rt.texture.width as f32,
            Height: rt.texture.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        unsafe {
            com_call!(
                self.ctx.as_ptr(),
                OMSetRenderTargets,
                1,
                rtvs.as_ptr(),
                null_mut()
            );
            com_call!(self.ctx.as_ptr(), RSSetViewports, 1, &vp);
        }
    }

    /// Reads a texture back as tightly packed rows of `bytes_per_pixel` bytes.
    pub fn read_back(&self, tex: &Texture2D, bytes_per_pixel: usize) -> Result<Vec<u8>> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: tex.width,
            Height: tex.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: tex.format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ as u32,
            MiscFlags: 0,
        };
        let staging = self.create_texture2d(&desc, None)?;
        let ctx = self.ctx.as_ptr();
        let dst = staging.tex.as_ptr() as *mut ID3D11Resource;
        let src = tex.tex.as_ptr() as *mut ID3D11Resource;
        let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { core::mem::zeroed() };
        unsafe {
            com_call!(ctx, CopyResource, dst, src);
            check(
                com_call!(ctx, Map, dst, 0, D3D11_MAP_READ, 0, &mut mapped),
                "Map staging",
            )?;
        }
        let row = tex.width as usize * bytes_per_pixel;
        let mut out = vec![0u8; row * tex.height as usize];
        for y in 0..tex.height as usize {
            let src_row = unsafe {
                core::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y * mapped.RowPitch as usize),
                    row,
                )
            };
            out[y * row..(y + 1) * row].copy_from_slice(src_row);
        }
        unsafe { com_call!(ctx, Unmap, dst, 0) };
        Ok(out)
    }

    /// Rewrites a DYNAMIC buffer's contents (`Map` with discard).
    pub fn update_buffer(&self, buf: &ComPtr<ID3D11Buffer>, data: &[u8]) -> Result<()> {
        let res = buf.as_ptr() as *mut ID3D11Resource;
        let mut mapped: D3D11_MAPPED_SUBRESOURCE = unsafe { core::mem::zeroed() };
        unsafe {
            check(
                com_call!(
                    self.ctx.as_ptr(),
                    Map,
                    res,
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    &mut mapped
                ),
                "Map dynamic buffer",
            )?;
            core::ptr::copy_nonoverlapping(data.as_ptr(), mapped.pData as *mut u8, data.len());
            com_call!(self.ctx.as_ptr(), Unmap, res, 0);
        }
        Ok(())
    }

    /// An immutable 2D texture with a full mip chain; `mips` are (data, row pitch) per level.
    pub fn create_texture2d_mips(
        &self,
        width: u32,
        height: u32,
        format: DXGI_FORMAT,
        mips: &[(&[u8], u32)],
    ) -> Result<Texture2D> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: mips.len() as u32,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let init: Vec<D3D11_SUBRESOURCE_DATA> = mips
            .iter()
            .map(|(d, pitch)| D3D11_SUBRESOURCE_DATA {
                pSysMem: d.as_ptr() as *const c_void,
                SysMemPitch: *pitch,
                SysMemSlicePitch: d.len() as u32,
            })
            .collect();
        let mut tex: *mut ID3D11Texture2D = null_mut();
        check(
            unsafe {
                com_call!(
                    self.dev.as_ptr(),
                    CreateTexture2D,
                    &desc,
                    init.as_ptr(),
                    &mut tex
                )
            },
            "CreateTexture2D (mips)",
        )?;
        Ok(Texture2D {
            tex: ComPtr::from_raw(tex).context("null texture")?,
            width,
            height,
            format,
        })
    }

    pub fn create_srv(&self, tex: &Texture2D) -> Result<ComPtr<ID3D11ShaderResourceView>> {
        let mut srv: *mut ID3D11ShaderResourceView = null_mut();
        let res = tex.tex.as_ptr() as *mut ID3D11Resource;
        check(
            unsafe {
                com_call!(
                    self.dev.as_ptr(),
                    CreateShaderResourceView,
                    res,
                    null(),
                    &mut srv
                )
            },
            "CreateShaderResourceView",
        )?;
        ComPtr::from_raw(srv).context("null srv")
    }

    pub fn create_sampler(
        &self,
        filter: D3D11_FILTER,
        address: D3D11_TEXTURE_ADDRESS_MODE,
        max_anisotropy: u32,
    ) -> Result<ComPtr<ID3D11SamplerState>> {
        let desc = D3D11_SAMPLER_DESC {
            Filter: filter,
            AddressU: address,
            AddressV: address,
            AddressW: address,
            MipLODBias: 0.0,
            MaxAnisotropy: max_anisotropy,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0; 4],
            // Debug: VESPUCCI_MINLOD / VESPUCCI_MAXLOD pin every sampler to a mip range.
            MinLOD: std::env::var("VESPUCCI_MINLOD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            MaxLOD: std::env::var("VESPUCCI_MAXLOD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3.402_823_5e38),
        };
        let mut s: *mut ID3D11SamplerState = null_mut();
        check(
            unsafe { com_call!(self.dev.as_ptr(), CreateSamplerState, &desc, &mut s) },
            "CreateSamplerState",
        )?;
        ComPtr::from_raw(s).context("null sampler")
    }

    /// Sampler for `sample_c` shadow lookups: linear PCF, LESS_EQUAL, white border (lit outside the map).
    pub fn create_comparison_sampler(&self) -> Result<ComPtr<ID3D11SamplerState>> {
        let desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_COMPARISON_MIN_MAG_LINEAR_MIP_POINT,
            AddressU: D3D11_TEXTURE_ADDRESS_BORDER,
            AddressV: D3D11_TEXTURE_ADDRESS_BORDER,
            AddressW: D3D11_TEXTURE_ADDRESS_BORDER,
            MipLODBias: 0.0,
            MaxAnisotropy: 1,
            ComparisonFunc: D3D11_COMPARISON_LESS_EQUAL,
            BorderColor: [1.0; 4],
            // Debug: VESPUCCI_MINLOD / VESPUCCI_MAXLOD pin every sampler to a mip range.
            MinLOD: std::env::var("VESPUCCI_MINLOD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            MaxLOD: std::env::var("VESPUCCI_MAXLOD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3.402_823_5e38),
        };
        let mut s: *mut ID3D11SamplerState = null_mut();
        check(
            unsafe { com_call!(self.dev.as_ptr(), CreateSamplerState, &desc, &mut s) },
            "CreateSamplerState (comparison)",
        )?;
        ComPtr::from_raw(s).context("null sampler")
    }

    pub fn create_depth_target(&self, width: u32, height: u32) -> Result<DepthTarget> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_D32_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_DEPTH_STENCIL as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let texture = self.create_texture2d(&desc, None)?;
        let mut dsv: *mut ID3D11DepthStencilView = null_mut();
        let res = texture.tex.as_ptr() as *mut ID3D11Resource;
        check(
            unsafe {
                com_call!(
                    self.dev.as_ptr(),
                    CreateDepthStencilView,
                    res,
                    null(),
                    &mut dsv
                )
            },
            "CreateDepthStencilView",
        )?;
        Ok(DepthTarget {
            texture,
            dsv: ComPtr::from_raw(dsv).context("null dsv")?,
        })
    }

    pub fn create_rasterizer(
        &self,
        cull: D3D11_CULL_MODE,
        front_ccw: bool,
        wireframe: bool,
    ) -> Result<ComPtr<ID3D11RasterizerState>> {
        let desc = D3D11_RASTERIZER_DESC {
            FillMode: if wireframe {
                D3D11_FILL_WIREFRAME
            } else {
                D3D11_FILL_SOLID
            },
            CullMode: cull,
            FrontCounterClockwise: front_ccw as i32,
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: 1,
            ScissorEnable: 0,
            MultisampleEnable: 0,
            AntialiasedLineEnable: 0,
        };
        let mut rs: *mut ID3D11RasterizerState = null_mut();
        check(
            unsafe { com_call!(self.dev.as_ptr(), CreateRasterizerState, &desc, &mut rs) },
            "CreateRasterizerState",
        )?;
        ComPtr::from_raw(rs).context("null rasterizer state")
    }

    pub fn bind_targets(&self, rt: &RenderTarget, depth: Option<&DepthTarget>) {
        let rtvs = [rt.rtv.as_ptr()];
        let dsv = depth.map_or(null_mut(), |d| d.dsv.as_ptr());
        let vp = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: rt.texture.width as f32,
            Height: rt.texture.height as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        unsafe {
            com_call!(self.ctx.as_ptr(), OMSetRenderTargets, 1, rtvs.as_ptr(), dsv);
            com_call!(self.ctx.as_ptr(), RSSetViewports, 1, &vp);
        }
    }

    pub fn clear_depth(&self, depth: &DepthTarget) {
        self.clear_depth_to(depth, 1.0);
    }

    pub fn clear_depth_to(&self, depth: &DepthTarget, value: f32) {
        unsafe {
            com_call!(
                self.ctx.as_ptr(),
                ClearDepthStencilView,
                depth.dsv.as_ptr(),
                D3D11_CLEAR_DEPTH as u32,
                value,
                0
            )
        }
    }

    /// Depth test with or without writes; `func` is the comparison that passes.
    pub fn create_depth_state(
        &self,
        write: bool,
        func: D3D11_COMPARISON_FUNC,
    ) -> Result<ComPtr<ID3D11DepthStencilState>> {
        let face = D3D11_DEPTH_STENCILOP_DESC {
            StencilFailOp: D3D11_STENCIL_OP_KEEP,
            StencilDepthFailOp: D3D11_STENCIL_OP_KEEP,
            StencilPassOp: D3D11_STENCIL_OP_KEEP,
            StencilFunc: D3D11_COMPARISON_ALWAYS,
        };
        let desc = D3D11_DEPTH_STENCIL_DESC {
            DepthEnable: 1,
            DepthWriteMask: if write {
                D3D11_DEPTH_WRITE_MASK_ALL
            } else {
                D3D11_DEPTH_WRITE_MASK_ZERO
            },
            DepthFunc: func,
            StencilEnable: 0,
            StencilReadMask: 0xff,
            StencilWriteMask: 0xff,
            FrontFace: face,
            BackFace: face,
        };
        let mut ds: *mut ID3D11DepthStencilState = null_mut();
        check(
            unsafe { com_call!(self.dev.as_ptr(), CreateDepthStencilState, &desc, &mut ds) },
            "CreateDepthStencilState",
        )?;
        ComPtr::from_raw(ds).context("null depth-stencil state")
    }

    pub fn set_depth_state(&self, ds: &ComPtr<ID3D11DepthStencilState>) {
        unsafe { com_call!(self.ctx.as_ptr(), OMSetDepthStencilState, ds.as_ptr(), 0) }
    }

    /// Opaque writes, or straight alpha blending (`src.a`, `1 - src.a`).
    pub fn create_blend_state(&self, alpha: bool) -> Result<ComPtr<ID3D11BlendState>> {
        let rt = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: alpha as i32,
            SrcBlend: D3D11_BLEND_SRC_ALPHA,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL as u8,
        };
        let desc = D3D11_BLEND_DESC {
            AlphaToCoverageEnable: 0,
            IndependentBlendEnable: 0,
            RenderTarget: [rt; 8],
        };
        let mut bs: *mut ID3D11BlendState = null_mut();
        check(
            unsafe { com_call!(self.dev.as_ptr(), CreateBlendState, &desc, &mut bs) },
            "CreateBlendState",
        )?;
        ComPtr::from_raw(bs).context("null blend state")
    }

    pub fn set_blend_state(&self, bs: &ComPtr<ID3D11BlendState>) {
        unsafe {
            com_call!(
                self.ctx.as_ptr(),
                OMSetBlendState,
                bs.as_ptr(),
                null(),
                0xffff_ffff
            )
        }
    }

    pub fn set_rasterizer(&self, rs: &ComPtr<ID3D11RasterizerState>) {
        unsafe { com_call!(self.ctx.as_ptr(), RSSetState, rs.as_ptr()) }
    }

    pub fn set_pipeline(
        &self,
        layout: &ComPtr<ID3D11InputLayout>,
        vs: &ComPtr<ID3D11VertexShader>,
        ps: &ComPtr<ID3D11PixelShader>,
    ) {
        unsafe {
            com_call!(self.ctx.as_ptr(), IASetInputLayout, layout.as_ptr());
            com_call!(
                self.ctx.as_ptr(),
                IASetPrimitiveTopology,
                D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST
            );
            com_call!(self.ctx.as_ptr(), VSSetShader, vs.as_ptr(), null(), 0);
            com_call!(self.ctx.as_ptr(), PSSetShader, ps.as_ptr(), null(), 0);
        }
    }

    pub fn set_vertex_buffer(&self, slot: u32, buf: &ComPtr<ID3D11Buffer>, stride: u32) {
        let bufs = [buf.as_ptr()];
        let strides = [stride];
        let offsets = [0u32];
        unsafe {
            com_call!(
                self.ctx.as_ptr(),
                IASetVertexBuffers,
                slot,
                1,
                bufs.as_ptr(),
                strides.as_ptr(),
                offsets.as_ptr()
            )
        }
    }

    pub fn set_index_buffer(&self, buf: &ComPtr<ID3D11Buffer>, format: DXGI_FORMAT) {
        unsafe { com_call!(self.ctx.as_ptr(), IASetIndexBuffer, buf.as_ptr(), format, 0) }
    }

    pub fn set_constant_buffer(&self, stage: Stage, slot: u32, buf: &ComPtr<ID3D11Buffer>) {
        let bufs = [buf.as_ptr()];
        unsafe {
            match stage {
                Stage::Vertex => com_call!(
                    self.ctx.as_ptr(),
                    VSSetConstantBuffers,
                    slot,
                    1,
                    bufs.as_ptr()
                ),
                Stage::Pixel => com_call!(
                    self.ctx.as_ptr(),
                    PSSetConstantBuffers,
                    slot,
                    1,
                    bufs.as_ptr()
                ),
            }
        }
    }

    pub fn set_shader_resource(
        &self,
        stage: Stage,
        slot: u32,
        srv: &ComPtr<ID3D11ShaderResourceView>,
    ) {
        let srvs = [srv.as_ptr()];
        unsafe {
            match stage {
                Stage::Vertex => com_call!(
                    self.ctx.as_ptr(),
                    VSSetShaderResources,
                    slot,
                    1,
                    srvs.as_ptr()
                ),
                Stage::Pixel => com_call!(
                    self.ctx.as_ptr(),
                    PSSetShaderResources,
                    slot,
                    1,
                    srvs.as_ptr()
                ),
            }
        }
    }

    pub fn set_sampler(&self, stage: Stage, slot: u32, sampler: &ComPtr<ID3D11SamplerState>) {
        let s = [sampler.as_ptr()];
        unsafe {
            match stage {
                Stage::Vertex => com_call!(self.ctx.as_ptr(), VSSetSamplers, slot, 1, s.as_ptr()),
                Stage::Pixel => com_call!(self.ctx.as_ptr(), PSSetSamplers, slot, 1, s.as_ptr()),
            }
        }
    }

    pub fn draw_indexed(&self, index_count: u32) {
        unsafe { com_call!(self.ctx.as_ptr(), DrawIndexed, index_count, 0, 0) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Vertex,
    Pixel,
}
