//! Constant buffers laid out by the shader's own reflection, written by
//! variable name and uploaded when changed.

use anyhow::Result;
use std::collections::HashMap;
use vespucci_d3d11::ffi::*;
use vespucci_d3d11::{ComPtr, Device};
use vespucci_fxc::dxbc::RdefCBuffer;
use vespucci_game::joaat;

pub struct CBufferBlock {
    pub name: String,
    pub bytes: Vec<u8>,
    /// Variable name hashes (of the exact name and of its lowercase form) -> (offset, size).
    vars: HashMap<u32, (usize, usize)>,
    pub names: Vec<String>,
    buffer: ComPtr<ID3D11Buffer>,
    dirty: bool,
}

impl CBufferBlock {
    pub fn new(dev: &Device, cb: &RdefCBuffer) -> Result<CBufferBlock> {
        let size = (cb.size as usize).div_ceil(16) * 16;
        let bytes = vec![0u8; size.max(16)];
        let mut vars = HashMap::new();
        let mut names = Vec::new();
        for v in &cb.vars {
            vars.insert(joaat(&v.name), (v.offset as usize, v.size as usize));
            vars.insert(joaat(&v.name.to_lowercase()), (v.offset as usize, v.size as usize));
            names.push(v.name.clone());
        }
        let buffer = dev.create_buffer(&bytes, D3D11_BIND_CONSTANT_BUFFER, D3D11_USAGE_DYNAMIC)?;
        Ok(CBufferBlock { name: cb.name.clone(), bytes, vars, names, buffer, dirty: true })
    }

    pub fn has(&self, name_hash: u32) -> bool {
        self.vars.contains_key(&name_hash)
    }

    /// The variable name behind a hash (exact or lowercase), for logs.
    pub fn name_of(&self, name_hash: u32) -> Option<&str> {
        self.names.iter().find(|n| joaat(n) == name_hash || joaat(&n.to_lowercase()) == name_hash).map(|s| s.as_str())
    }

    /// Writes floats at the variable; returns false when the buffer has no such variable.
    pub fn set_f32(&mut self, name_hash: u32, values: &[f32]) -> bool {
        let Some(&(off, size)) = self.vars.get(&name_hash) else { return false };
        let n = values.len().min(size / 4);
        for (i, v) in values.iter().take(n).enumerate() {
            self.bytes[off + i * 4..off + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        self.dirty = true;
        true
    }

    pub fn set_u32(&mut self, name_hash: u32, values: &[u32]) -> bool {
        let Some(&(off, size)) = self.vars.get(&name_hash) else { return false };
        let n = values.len().min(size / 4);
        for (i, v) in values.iter().take(n).enumerate() {
            self.bytes[off + i * 4..off + i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        self.dirty = true;
        true
    }

    pub fn set_by_name(&mut self, name: &str, values: &[f32]) -> bool {
        self.set_f32(joaat(name), values)
    }

    /// Debug aid: every float set to `v`.
    pub fn fill_f32(&mut self, v: f32) {
        for c in self.bytes.chunks_mut(4) {
            c.copy_from_slice(&v.to_le_bytes());
        }
        self.dirty = true;
    }

    pub fn upload(&mut self, dev: &Device) -> Result<()> {
        if self.dirty {
            dev.update_buffer(&self.buffer, &self.bytes)?;
            self.dirty = false;
        }
        Ok(())
    }

    pub fn buffer(&self) -> &ComPtr<ID3D11Buffer> {
        &self.buffer
    }
}
