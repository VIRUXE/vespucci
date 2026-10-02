//! DXBC container reflection: the chunks a renderer needs to bind a shader
//! without `d3dcompiler`. Layouts follow the public DXBC format as used by
//! SM4/SM5 shaders (`RDEF` resource definitions, `ISGN`/`OSGN` signatures).

use anyhow::{bail, ensure, Context, Result};

pub struct Dxbc<'a> {
    pub data: &'a [u8],
    pub chunks: Vec<(&'a [u8; 4], &'a [u8])>,
}

impl<'a> Dxbc<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        ensure!(data.len() >= 32 && &data[..4] == b"DXBC", "not a DXBC blob");
        let total = u32_at(data, 24)? as usize;
        ensure!(
            total <= data.len(),
            "DXBC size {total} exceeds blob {}",
            data.len()
        );
        let count = u32_at(data, 28)? as usize;
        let mut chunks = Vec::with_capacity(count);
        for i in 0..count {
            let off = u32_at(data, 32 + i * 4)? as usize;
            let fourcc: &[u8; 4] = data.get(off..off + 4).context("chunk fourcc")?.try_into()?;
            let size = u32_at(data, off + 4)? as usize;
            let body = data.get(off + 8..off + 8 + size).context("chunk body")?;
            chunks.push((fourcc, body));
        }
        Ok(Dxbc { data, chunks })
    }

    pub fn chunk(&self, fourcc: &[u8; 4]) -> Option<&'a [u8]> {
        self.chunks
            .iter()
            .find(|(f, _)| *f == fourcc)
            .map(|(_, b)| *b)
    }

    pub fn rdef(&self) -> Result<Rdef> {
        Rdef::parse(self.chunk(b"RDEF").context("no RDEF chunk")?)
    }

    pub fn input_signature(&self) -> Result<Signature> {
        if let Some(c) = self.chunk(b"ISGN") {
            return Signature::parse(c, 24);
        }
        if let Some(c) = self.chunk(b"ISG1") {
            return Signature::parse(c, 32);
        }
        bail!("no input signature chunk")
    }

    pub fn output_signature(&self) -> Result<Signature> {
        for (fourcc, stride) in [(b"OSGN", 24usize), (b"OSG1", 32), (b"OSG5", 32)] {
            if let Some(c) = self.chunk(fourcc) {
                return Signature::parse(c, stride);
            }
        }
        bail!("no output signature chunk")
    }

    /// Shader model and stage from the bytecode chunk's version token.
    pub fn version(&self) -> Option<(u8, u8, ProgramType)> {
        let c = self.chunk(b"SHEX").or_else(|| self.chunk(b"SHDR"))?;
        let v = u32::from_le_bytes(c.get(..4)?.try_into().ok()?);
        let minor = (v & 0xF) as u8;
        let major = ((v >> 4) & 0xF) as u8;
        let ty = ProgramType::from_u16(((v >> 16) & 0xFFFF) as u16);
        Some((major, minor, ty))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramType {
    Pixel,
    Vertex,
    Geometry,
    Hull,
    Domain,
    Compute,
    Unknown(u16),
}

impl ProgramType {
    fn from_u16(v: u16) -> Self {
        match v {
            0 => ProgramType::Pixel,
            1 => ProgramType::Vertex,
            2 => ProgramType::Geometry,
            3 => ProgramType::Hull,
            4 => ProgramType::Domain,
            5 => ProgramType::Compute,
            other => ProgramType::Unknown(other),
        }
    }
}

/// `RDEF`: constant buffers with their variables, and bound resources.
#[derive(Debug, Default, Clone)]
pub struct Rdef {
    pub major: u8,
    pub minor: u8,
    pub program_type: u16,
    pub cbuffers: Vec<RdefCBuffer>,
    pub bindings: Vec<RdefBinding>,
}

#[derive(Debug, Clone)]
pub struct RdefCBuffer {
    pub name: String,
    pub size: u32,
    pub kind: u32, // D3D_CT_CBUFFER=0, TBUFFER=1, ...
    pub vars: Vec<RdefVar>,
}

#[derive(Debug, Clone)]
pub struct RdefVar {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub flags: u32,
    /// D3D_SHADER_VARIABLE_CLASS / TYPE, rows, columns, array elements.
    pub class: u16,
    pub ty: u16,
    pub rows: u16,
    pub cols: u16,
    pub elements: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindKind {
    CBuffer,
    TBuffer,
    Texture,
    Sampler,
    Uav,
    Structured,
    ByteAddress,
    Other(u32),
}

#[derive(Debug, Clone)]
pub struct RdefBinding {
    pub name: String,
    pub kind: BindKind,
    pub input_type: u32,
    pub return_type: u32,
    pub dimension: u32,
    pub bind_point: u32,
    pub bind_count: u32,
    pub flags: u32,
}

impl Rdef {
    pub fn parse(c: &[u8]) -> Result<Rdef> {
        let cb_count = u32_at(c, 0)? as usize;
        let cb_off = u32_at(c, 4)? as usize;
        let res_count = u32_at(c, 8)? as usize;
        let res_off = u32_at(c, 12)? as usize;
        let minor = *c.get(16).context("rdef")?;
        let major = *c.get(17).context("rdef")?;
        let program_type = u16::from_le_bytes([c[18], c[19]]);
        let sm5 = major >= 5;
        // SM 5.1 (major 5, minor 1) resource bindings carry space + id.
        let res_stride = if major > 5 || (major == 5 && minor >= 1) {
            40
        } else {
            32
        };
        let var_stride = if sm5 { 40 } else { 24 };

        let mut bindings = Vec::with_capacity(res_count);
        for i in 0..res_count {
            let b = res_off + i * res_stride;
            let input_type = u32_at(c, b + 4)?;
            bindings.push(RdefBinding {
                name: cstr_at(c, u32_at(c, b)? as usize)?,
                kind: match input_type {
                    0 => BindKind::CBuffer,
                    1 => BindKind::TBuffer,
                    2 => BindKind::Texture,
                    3 => BindKind::Sampler,
                    4 => BindKind::Uav,
                    5 | 6 => BindKind::Structured,
                    7 | 8 => BindKind::ByteAddress,
                    other => BindKind::Other(other),
                },
                input_type,
                return_type: u32_at(c, b + 8)?,
                dimension: u32_at(c, b + 12)?,
                bind_point: u32_at(c, b + 20)?,
                bind_count: u32_at(c, b + 24)?,
                flags: u32_at(c, b + 28)?,
            });
        }

        let mut cbuffers = Vec::with_capacity(cb_count);
        for i in 0..cb_count {
            let b = cb_off + i * 24;
            let var_count = u32_at(c, b + 4)? as usize;
            let var_off = u32_at(c, b + 8)? as usize;
            let mut vars = Vec::with_capacity(var_count);
            for j in 0..var_count {
                let v = var_off + j * var_stride;
                let type_off = u32_at(c, v + 16)? as usize;
                let t = |k: usize| -> Result<u16> {
                    Ok(u16::from_le_bytes(
                        c.get(type_off + k..type_off + k + 2)
                            .context("rdef type")?
                            .try_into()?,
                    ))
                };
                vars.push(RdefVar {
                    name: cstr_at(c, u32_at(c, v)? as usize)?,
                    offset: u32_at(c, v + 4)?,
                    size: u32_at(c, v + 8)?,
                    flags: u32_at(c, v + 12)?,
                    class: t(0)?,
                    ty: t(2)?,
                    rows: t(4)?,
                    cols: t(6)?,
                    elements: t(8)?,
                });
            }
            cbuffers.push(RdefCBuffer {
                name: cstr_at(c, u32_at(c, b)? as usize)?,
                size: u32_at(c, b + 12)?,
                kind: u32_at(c, b + 20)?,
                vars,
            });
        }
        Ok(Rdef {
            major,
            minor,
            program_type,
            cbuffers,
            bindings,
        })
    }
}

/// `ISGN`/`OSGN`: one entry per semantic in/out of the shader.
#[derive(Debug, Clone, Default)]
pub struct Signature {
    pub elements: Vec<SigElement>,
}

#[derive(Debug, Clone)]
pub struct SigElement {
    pub semantic: String,
    pub index: u32,
    pub system_value: u32,
    /// D3D_REGISTER_COMPONENT_TYPE: 1 uint, 2 int, 3 float.
    pub component_type: u32,
    pub register: u32,
    pub mask: u8,
    pub rw_mask: u8,
}

impl Signature {
    fn parse(c: &[u8], stride: usize) -> Result<Signature> {
        let count = u32_at(c, 0)? as usize;
        let base = u32_at(c, 4)? as usize;
        let mut elements = Vec::with_capacity(count);
        for i in 0..count {
            // ISG1 entries lead with a stream index; the classic fields follow.
            let e = base + i * stride + if stride == 32 { 4 } else { 0 };
            elements.push(SigElement {
                semantic: cstr_at(c, u32_at(c, e)? as usize)?,
                index: u32_at(c, e + 4)?,
                system_value: u32_at(c, e + 8)?,
                component_type: u32_at(c, e + 12)?,
                register: u32_at(c, e + 16)?,
                mask: *c.get(e + 20).context("sig mask")?,
                rw_mask: *c.get(e + 21).context("sig rwmask")?,
            });
        }
        Ok(Signature { elements })
    }
}

fn u32_at(b: &[u8], off: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(
        b.get(off..off + 4)
            .with_context(|| format!("read u32 at {off}"))?
            .try_into()?,
    ))
}

fn cstr_at(b: &[u8], off: usize) -> Result<String> {
    let s = b.get(off..).context("string offset")?;
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    Ok(String::from_utf8_lossy(&s[..end]).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VS: &[u8] = include_bytes!("../../../shaders/m0/tri.vs.dxbc");
    const PS: &[u8] = include_bytes!("../../../shaders/m0/tri.ps.dxbc");

    #[test]
    fn parses_vkd3d_output() {
        let vs = Dxbc::parse(VS).unwrap();
        let isgn = vs.input_signature().unwrap();
        let names: Vec<_> = isgn
            .elements
            .iter()
            .map(|e| (e.semantic.as_str(), e.index))
            .collect();
        assert_eq!(names, vec![("POSITION", 0), ("COLOR", 0)]);
        let (major, _, ty) = vs.version().unwrap();
        assert_eq!((major, ty), (5, ProgramType::Vertex));
        let rdef = vs.rdef().unwrap();
        assert!(rdef.cbuffers.is_empty());

        let ps = Dxbc::parse(PS).unwrap();
        let osgn = ps.output_signature().unwrap();
        assert_eq!(osgn.elements[0].semantic, "SV_Target");
        assert_eq!(ps.version().unwrap().2, ProgramType::Pixel);
    }
}
