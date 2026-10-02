//! The game's `.fxc` shader container ("rgxe"): presets, six shader stage
//! groups with DXBC blobs, constant-buffer and variable declarations, and
//! techniques made of passes that pick one shader per stage.
//!
//! Layout as documented by public research on the format (CodeWalker's
//! `FxcFile.cs` is the reference); strings are a length byte that counts the
//! terminating NUL, then the bytes.

use anyhow::{bail, ensure, Context, Result};

pub const STAGE_NAMES: [&str; 6] = ["vs", "ps", "cs", "ds", "gs", "hs"];

#[derive(Debug, Clone)]
pub struct Preset {
    pub name: String,
    pub value: u32,
}

#[derive(Debug, Clone)]
pub struct Shader {
    pub name: String,
    /// Names of the variables this shader uses.
    pub variables: Vec<String>,
    /// Constant buffers this shader binds, with the slot each one takes.
    pub buffers: Vec<(String, u16)>,
    pub dxbc: Vec<u8>,
    pub version: Option<(u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct CBuffer {
    pub name: String,
    pub name_hash: u32,
    pub size: u32,
    /// Register slot per stage, in [`STAGE_NAMES`] order.
    pub slots: [u16; 6],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarType {
    Float,
    Float2,
    Float3,
    Float4,
    Texture,
    Bool,
    Float3x4,
    Float4x4,
    Int,
    Int4,
    Buffer,
    UavBuffer,
    UavTexture,
    Other(u8),
}

impl VarType {
    fn from_u8(v: u8) -> Self {
        match v {
            2 => VarType::Float,
            3 => VarType::Float2,
            4 => VarType::Float3,
            5 => VarType::Float4,
            6 => VarType::Texture,
            7 => VarType::Bool,
            8 => VarType::Float3x4,
            9 => VarType::Float4x4,
            11 => VarType::Int,
            14 => VarType::Int4,
            15 => VarType::Buffer,
            21 => VarType::UavBuffer,
            22 => VarType::UavTexture,
            o => VarType::Other(o),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ParamValue {
    Int(i32),
    Float(f32),
    Str(String),
}

#[derive(Debug, Clone)]
pub struct Variable {
    pub ty: VarType,
    pub count: u8,
    pub slot: u8,
    pub group: u8,
    /// Shader-side name and the material-side name it is set through.
    pub name: String,
    pub param_name: String,
    pub offset: u8,
    pub variant: u8,
    /// `joaat(lowercase cbuffer name)` of the constant buffer holding it; 0 for resources.
    pub cbuffer_hash: u32,
    /// Annotations such as UI ranges and sampler states.
    pub params: Vec<(String, ParamValue)>,
    /// Default value words; floats for float types, raw integers otherwise.
    pub values: Vec<u32>,
}

impl Variable {
    pub fn default_floats(&self) -> Vec<f32> {
        self.values.iter().map(|&v| f32::from_bits(v)).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Pass {
    /// Shader index per stage, in [`STAGE_NAMES`] order; 0 means none (the
    /// group's leading "NULL" entry), `n` means `groups[stage][n-1]`.
    pub stage: [u8; 6],
    pub params: Vec<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct Technique {
    pub name: String,
    pub passes: Vec<Pass>,
}

#[derive(Debug, Clone)]
pub struct FxcFile {
    pub vertex_type: u32,
    pub presets: Vec<Preset>,
    /// Shaders per stage in [`STAGE_NAMES`] order.
    pub groups: [Vec<Shader>; 6],
    pub cbuffers: Vec<CBuffer>,
    /// Constant-buffer contents.
    pub variables: Vec<Variable>,
    /// Second cbuffer list (rare) and resource variables: textures, samplers, buffers.
    pub cbuffers2: Vec<CBuffer>,
    pub resources: Vec<Variable>,
    pub techniques: Vec<Technique>,
}

impl FxcFile {
    pub fn parse(data: &[u8]) -> Result<FxcFile> {
        let mut r = Reader { data, pos: 0 };
        ensure!(r.u32()? == 0x6578_6772, "not an rgxe shader container");
        let vertex_type = r.u32()?;
        let presets = (0..r.u8()?)
            .map(|_| {
                let name = r.string()?;
                let _zero = r.u8()?;
                Ok(Preset {
                    name,
                    value: r.u32()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let mut groups: [Vec<Shader>; 6] = Default::default();
        for (gi, group) in groups.iter_mut().enumerate() {
            let mut count = r.u8()?;
            if count == 0 {
                // A stray zero byte precedes the count in some files (seen for hull shaders).
                count = r.u8()?;
            }
            let _null = r.string()?;
            let _ = (r.u8()?, r.u8()?, r.u32()?);
            for _ in 1..count {
                group.push(read_shader(&mut r, gi)?);
            }
        }

        let cbuffers = (0..r.u8()?)
            .map(|_| read_cbuffer(&mut r))
            .collect::<Result<Vec<_>>>()?;
        let variables = (0..r.u8()?)
            .map(|_| read_variable(&mut r))
            .collect::<Result<Vec<_>>>()?;
        let cbuffers2 = (0..r.u8()?)
            .map(|_| read_cbuffer(&mut r))
            .collect::<Result<Vec<_>>>()?;
        let resources = (0..r.u8()?)
            .map(|_| read_variable(&mut r))
            .collect::<Result<Vec<_>>>()?;

        let techniques = (0..r.u8()?)
            .map(|_| {
                let name = r.string()?;
                let passes = (0..r.u8()?)
                    .map(|_| {
                        let mut stage = [0u8; 6];
                        for s in &mut stage {
                            *s = r.u8()?;
                        }
                        let params = (0..r.u8()?)
                            .map(|_| Ok((r.u32()?, r.u32()?)))
                            .collect::<Result<Vec<_>>>()?;
                        Ok(Pass { stage, params })
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Technique { name, passes })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(FxcFile {
            vertex_type,
            presets,
            groups,
            cbuffers,
            variables,
            cbuffers2,
            resources,
            techniques,
        })
    }

    pub fn technique(&self, name: &str) -> Option<&Technique> {
        self.techniques.iter().find(|t| t.name == name)
    }

    /// The shader a pass uses for a stage (0 vs, 1 ps, ...), if any.
    pub fn pass_shader(&self, pass: &Pass, stage: usize) -> Option<&Shader> {
        let idx = pass.stage[stage] as usize;
        if idx == 0 {
            return None;
        }
        self.groups[stage].get(idx - 1)
    }

    pub fn cbuffer_by_hash(&self, hash: u32) -> Option<&CBuffer> {
        self.cbuffers
            .iter()
            .chain(&self.cbuffers2)
            .find(|c| c.name_hash == hash)
    }
}

fn read_shader(r: &mut Reader, group: usize) -> Result<Shader> {
    let mut name = r.string()?;
    if name.is_empty() {
        // Geometry shaders carry an empty name before the real one.
        name = r.string()?;
    }
    let variables = (0..r.u8()?)
        .map(|_| r.string())
        .collect::<Result<Vec<_>>>()?;
    let buffers = (0..r.u8()?)
        .map(|_| Ok((r.string()?, r.u16()?)))
        .collect::<Result<Vec<_>>>()?;
    if group == 4 {
        let _gs_extra = r.u8()?;
    }
    let len = r.u32()? as usize;
    let mut dxbc = Vec::new();
    let mut version = None;
    if len > 0 {
        let blob = r.bytes(len)?;
        ensure!(
            blob.len() >= 4 && &blob[..4] == b"DXBC",
            "shader {name}: bytecode does not start with DXBC"
        );
        dxbc = blob.to_vec();
        if matches!(group, 0 | 1 | 4) {
            version = Some((r.u8()?, r.u8()?));
        }
    }
    Ok(Shader {
        name,
        variables,
        buffers,
        dxbc,
        version,
    })
}

fn read_cbuffer(r: &mut Reader) -> Result<CBuffer> {
    let size = r.u32()?;
    let mut slots = [0u16; 6];
    for s in &mut slots {
        *s = r.u16()?;
    }
    let name = r.string()?;
    let name_hash = rpf_archive::rage_joaat(&name.to_lowercase());
    Ok(CBuffer {
        name,
        name_hash,
        size,
        slots,
    })
}

fn read_variable(r: &mut Reader) -> Result<Variable> {
    let ty = VarType::from_u8(r.u8()?);
    let count = r.u8()?;
    let slot = r.u8()?;
    let group = r.u8()?;
    let name = r.string()?;
    let param_name = r.string()?;
    let offset = r.u8()?;
    let variant = r.u8()?;
    let _ = (r.u8()?, r.u8()?);
    let cbuffer_hash = r.u32()?;
    let params = (0..r.u8()?)
        .map(|_| {
            let pname = r.string()?;
            let value = match r.u8()? {
                0 => ParamValue::Int(r.u32()? as i32),
                1 => ParamValue::Float(f32::from_bits(r.u32()?)),
                2 => ParamValue::Str(r.string()?),
                t => bail!("variable {name}: unknown annotation type {t}"),
            };
            Ok((pname, value))
        })
        .collect::<Result<Vec<_>>>()?;
    let values = (0..r.u8()?).map(|_| r.u32()).collect::<Result<Vec<_>>>()?;
    Ok(Variable {
        ty,
        count,
        slot,
        group,
        name,
        param_name,
        offset,
        variant,
        cbuffer_hash,
        params,
        values,
    })
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self
            .data
            .get(self.pos..self.pos + n)
            .with_context(|| format!("fxc truncated at {} (+{n})", self.pos))?;
        self.pos += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into()?))
    }
    /// Length byte (counting the NUL), then the bytes.
    fn string(&mut self) -> Result<String> {
        let len = self.u8()? as usize;
        if len == 0 {
            return Ok(String::new());
        }
        let b = self.bytes(len)?;
        Ok(String::from_utf8_lossy(&b[..len - 1]).into_owned())
    }
}
