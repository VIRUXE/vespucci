//! The game's shaders on the device: `.fxc` files by name hash, compiled
//! stage blobs and their reflection, created once and shared.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::rc::Rc;
use vespucci_d3d11::ffi::{ID3D11PixelShader, ID3D11VertexShader};
use vespucci_d3d11::{ComPtr, Device};
use vespucci_fxc::{Dxbc, FxcFile, Rdef, Signature, SHADER_DIRS};
use vespucci_game::{joaat, GameFs};

pub struct CompiledStage {
    pub name: String,
    pub dxbc: Vec<u8>,
    pub rdef: Rdef,
    pub inputs: Option<Signature>,
}

pub struct VertexStage {
    pub stage: CompiledStage,
    pub shader: ComPtr<ID3D11VertexShader>,
}

pub struct PixelStage {
    pub stage: CompiledStage,
    pub shader: ComPtr<ID3D11PixelShader>,
}

pub struct FxcProgram {
    pub name: String,
    pub path: String,
    pub fxc: FxcFile,
    vs: HashMap<usize, Rc<VertexStage>>,
    ps: HashMap<usize, Rc<PixelStage>>,
}

impl FxcProgram {
    /// Vertex and pixel stage of a technique's first pass, compiled on first use.
    pub fn technique_stages(
        &mut self,
        dev: &Device,
        technique: &str,
    ) -> Result<(Rc<VertexStage>, Rc<PixelStage>)> {
        let tech = self
            .fxc
            .technique(technique)
            .with_context(|| format!("{}: no technique {technique}", self.name))?;
        let pass = tech
            .passes
            .first()
            .with_context(|| format!("{}: technique {technique} has no passes", self.name))?;
        let (vi, pi) = (pass.stage[0] as usize, pass.stage[1] as usize);
        if vi == 0 || pi == 0 {
            bail!(
                "{}: technique {technique} lacks a vertex or pixel shader",
                self.name
            );
        }
        let vs = match self.vs.get(&vi) {
            Some(v) => v.clone(),
            None => {
                let sh = &self.fxc.groups[0][vi - 1];
                let stage = reflect(&sh.name, &sh.dxbc, true)?;
                let shader = dev
                    .create_vertex_shader(&sh.dxbc)
                    .with_context(|| format!("{}: {}", self.name, sh.name))?;
                let v = Rc::new(VertexStage { stage, shader });
                self.vs.insert(vi, v.clone());
                v
            }
        };
        let ps = match self.ps.get(&pi) {
            Some(p) => p.clone(),
            None => {
                let sh = &self.fxc.groups[1][pi - 1];
                let stage = reflect(&sh.name, &sh.dxbc, false)?;
                let shader = dev
                    .create_pixel_shader(&sh.dxbc)
                    .with_context(|| format!("{}: {}", self.name, sh.name))?;
                let p = Rc::new(PixelStage { stage, shader });
                self.ps.insert(pi, p.clone());
                p
            }
        };
        Ok((vs, ps))
    }

    /// First of `candidates` that this shader file defines.
    pub fn pick_technique<'a>(&self, candidates: &[&'a str]) -> Option<&'a str> {
        candidates
            .iter()
            .copied()
            .find(|t| self.fxc.technique(t).is_some())
    }
}

fn reflect(name: &str, dxbc: &[u8], with_inputs: bool) -> Result<CompiledStage> {
    let d = Dxbc::parse(dxbc)?;
    Ok(CompiledStage {
        name: name.to_string(),
        dxbc: dxbc.to_vec(),
        rdef: d.rdef()?,
        inputs: if with_inputs {
            Some(d.input_signature()?)
        } else {
            None
        },
    })
}

pub struct ShaderCache {
    /// `joaat(stem)` -> full path of the authoritative `.fxc`.
    by_hash: HashMap<u32, String>,
    programs: HashMap<u32, Rc<std::cell::RefCell<FxcProgram>>>,
}

impl ShaderCache {
    pub fn new(fs: &GameFs) -> ShaderCache {
        let mut by_hash = HashMap::new();
        // Least authoritative first, so the update copies overwrite.
        for dir in SHADER_DIRS.iter().rev() {
            for f in fs.files.iter().filter(|f| f.ext == "fxc") {
                let path = f.full_path(fs);
                if path.starts_with(dir) {
                    by_hash.insert(f.stem_hash, path);
                }
            }
        }
        ShaderCache {
            by_hash,
            programs: HashMap::new(),
        }
    }

    pub fn path_for(&self, name_hash: u32) -> Option<&str> {
        self.by_hash.get(&name_hash).map(|s| s.as_str())
    }

    pub fn get(
        &mut self,
        fs: &GameFs,
        name_hash: u32,
    ) -> Result<Rc<std::cell::RefCell<FxcProgram>>> {
        if let Some(p) = self.programs.get(&name_hash) {
            return Ok(p.clone());
        }
        let path = self
            .by_hash
            .get(&name_hash)
            .with_context(|| format!("no .fxc with name hash {name_hash:#010x}"))?
            .clone();
        let loc = fs.get(&path).context("shader path vanished")?;
        let fxc = FxcFile::parse(&fs.read(loc)?).with_context(|| format!("parsing {path}"))?;
        let name = path
            .rsplit('/')
            .next()
            .unwrap_or(&path)
            .trim_end_matches(".fxc")
            .to_string();
        let p = Rc::new(std::cell::RefCell::new(FxcProgram {
            name,
            path,
            fxc,
            vs: HashMap::new(),
            ps: HashMap::new(),
        }));
        self.programs.insert(name_hash, p.clone());
        Ok(p)
    }

    pub fn get_by_name(
        &mut self,
        fs: &GameFs,
        name: &str,
    ) -> Result<Rc<std::cell::RefCell<FxcProgram>>> {
        self.get(fs, joaat(&name.to_lowercase()))
    }
}
