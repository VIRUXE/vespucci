//! Game textures on the device.

use anyhow::{bail, Result};
use rage_formats::{level_size, to_dds_layout, TextureFormat, YtdTexture};
use std::collections::HashMap;
use vespucci_d3d11::ffi::*;
use vespucci_d3d11::{ComPtr, Device};

pub fn dxgi_format(f: TextureFormat) -> Option<DXGI_FORMAT> {
    Some(match f {
        TextureFormat::DXT1 => DXGI_FORMAT_BC1_UNORM,
        TextureFormat::DXT3 => DXGI_FORMAT_BC2_UNORM,
        TextureFormat::DXT5 => DXGI_FORMAT_BC3_UNORM,
        TextureFormat::ATI1 => DXGI_FORMAT_BC4_UNORM,
        TextureFormat::ATI2 => DXGI_FORMAT_BC5_UNORM,
        TextureFormat::BC7 => DXGI_FORMAT_BC7_UNORM,
        TextureFormat::A8R8G8B8 => DXGI_FORMAT_B8G8R8A8_UNORM,
        TextureFormat::X8R8G8B8 => DXGI_FORMAT_B8G8R8X8_UNORM,
        TextureFormat::A8B8G8R8 => DXGI_FORMAT_R8G8B8A8_UNORM,
        TextureFormat::A1R5G5B5 => DXGI_FORMAT_B5G5R5A1_UNORM,
        TextureFormat::A8 => DXGI_FORMAT_A8_UNORM,
        TextureFormat::L8 => DXGI_FORMAT_R8_UNORM,
        TextureFormat::Unknown => return None,
    })
}

fn is_block(f: TextureFormat) -> bool {
    matches!(
        f,
        TextureFormat::DXT1
            | TextureFormat::DXT3
            | TextureFormat::DXT5
            | TextureFormat::ATI1
            | TextureFormat::ATI2
            | TextureFormat::BC7
    )
}

/// Uploads a texture with its whole mip chain, dropping the top `skip` levels.
pub fn upload(dev: &Device, t: &YtdTexture, skip: u8) -> Result<ComPtr<ID3D11ShaderResourceView>> {
    let Some(format) = dxgi_format(t.format) else {
        bail!("{}: unsupported format {:?}", t.name, t.format)
    };
    let levels = t.levels.max(1);
    let data = to_dds_layout(t.format, t.width, t.height, t.stride, levels, &t.pixel_data);
    let skip = skip.min(levels - 1);
    let mut mips: Vec<(&[u8], u32)> = Vec::new();
    let (mut w, mut h) = (t.width, t.height);
    let mut at = 0usize;
    for level in 0..levels {
        let size = level_size(t.format, w, h);
        let end = (at + size).min(data.len());
        if level >= skip {
            let pitch = if is_block(t.format) {
                (w as u32).div_ceil(4).max(1)
                    * (size as u32
                        / ((h as u32).div_ceil(4).max(1) * (w as u32).div_ceil(4).max(1)))
            } else {
                size as u32 / h.max(1) as u32
            };
            mips.push((&data[at..end], pitch));
        }
        at = end;
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    let (tw, th) = (
        (t.width >> skip).max(1) as u32,
        (t.height >> skip).max(1) as u32,
    );
    let tex = dev.create_texture2d_mips(tw, th, format, &mips)?;
    dev.create_srv(&tex)
}

/// A 1x1 texture of one colour, for missing inputs.
pub fn solid(dev: &Device, rgba: [u8; 4]) -> Result<ComPtr<ID3D11ShaderResourceView>> {
    let tex = dev.create_texture2d_mips(1, 1, DXGI_FORMAT_R8G8B8A8_UNORM, &[(&rgba, 4)])?;
    dev.create_srv(&tex)
}

/// Textures by name hash, uploaded on first use.
pub struct TextureCache {
    srvs: HashMap<u32, ComPtr<ID3D11ShaderResourceView>>,
    pub missing: ComPtr<ID3D11ShaderResourceView>,
    /// For engine-owned inputs without a material parameter (reflection map, fog
    /// rays, ...): a flat dim sky colour, so reflections read as "sky" not magenta.
    pub engine: ComPtr<ID3D11ShaderResourceView>,
    /// 1x1 depth value 1.0: every `sample_c` shadow test passes (fully lit).
    pub unshadowed: ComPtr<ID3D11ShaderResourceView>,
    pub uploaded_bytes: usize,
}

impl TextureCache {
    pub fn new(dev: &Device) -> Result<TextureCache> {
        let one = 1.0f32.to_le_bytes();
        let depth_one = dev.create_texture2d_mips(1, 1, DXGI_FORMAT_R32_FLOAT, &[(&one, 4)])?;
        // Debug: VESPUCCI_ENGINE_TEX=r,g,b (0-255) changes the stand-in colour.
        let engine_rgba: [u8; 4] = std::env::var("VESPUCCI_ENGINE_TEX")
            .ok()
            .and_then(|v| {
                let c: Vec<u8> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                (c.len() >= 3).then(|| [c[0], c[1], c[2], 255])
            })
            .unwrap_or([77, 102, 140, 255]);
        Ok(TextureCache {
            srvs: HashMap::new(),
            missing: solid(dev, [255, 0, 255, 255])?,
            engine: solid(dev, engine_rgba)?,
            unshadowed: dev.create_srv(&depth_one)?,
            uploaded_bytes: 0,
        })
    }

    pub fn get(&self, name_hash: u32) -> Option<&ComPtr<ID3D11ShaderResourceView>> {
        self.srvs.get(&name_hash)
    }

    pub fn insert(
        &mut self,
        dev: &Device,
        t: &YtdTexture,
        skip: u8,
    ) -> Result<&ComPtr<ID3D11ShaderResourceView>> {
        if !self.srvs.contains_key(&t.name_hash) {
            let srv = upload(dev, t, skip)?;
            self.uploaded_bytes += t.pixel_data.len();
            self.srvs.insert(t.name_hash, srv);
        }
        Ok(&self.srvs[&t.name_hash])
    }
}
