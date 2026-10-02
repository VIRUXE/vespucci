//! Input layouts: the drawable's vertex declaration matched to the vertex
//! shader's input signature.

use anyhow::Result;
use rage_formats::{VertexComponentType, VertexDeclaration, VertexSemantic};
use std::collections::HashMap;
use vespucci_d3d11::ffi::*;
use vespucci_d3d11::{ComPtr, Device, InputElement};
use vespucci_fxc::Signature;

/// Bytes bound at slot 1 with stride 0 for inputs the mesh does not carry.
pub const DUMMY_SLOT: u32 = 1;
pub const DUMMY_BYTES: usize = 64;

pub fn semantic_name(s: VertexSemantic) -> Option<(&'static str, u32)> {
    Some(match s {
        VertexSemantic::Position => ("POSITION", 0),
        VertexSemantic::BlendWeights => ("BLENDWEIGHT", 0),
        VertexSemantic::BlendIndices => ("BLENDINDICES", 0),
        VertexSemantic::Normal => ("NORMAL", 0),
        VertexSemantic::Colour0 => ("COLOR", 0),
        VertexSemantic::Colour1 => ("COLOR", 1),
        VertexSemantic::TexCoord0 => ("TEXCOORD", 0),
        VertexSemantic::TexCoord1 => ("TEXCOORD", 1),
        VertexSemantic::TexCoord2 => ("TEXCOORD", 2),
        VertexSemantic::TexCoord3 => ("TEXCOORD", 3),
        VertexSemantic::TexCoord4 => ("TEXCOORD", 4),
        VertexSemantic::TexCoord5 => ("TEXCOORD", 5),
        VertexSemantic::TexCoord6 => ("TEXCOORD", 6),
        VertexSemantic::TexCoord7 => ("TEXCOORD", 7),
        VertexSemantic::Tangent => ("TANGENT", 0),
        VertexSemantic::Binormal => ("BINORMAL", 0),
        VertexSemantic::Unknown(_) => return None,
    })
}

pub fn component_format(t: VertexComponentType) -> Option<DXGI_FORMAT> {
    Some(match t {
        VertexComponentType::Half2 => DXGI_FORMAT_R16G16_FLOAT,
        VertexComponentType::Float => DXGI_FORMAT_R32_FLOAT,
        VertexComponentType::Half4 => DXGI_FORMAT_R16G16B16A16_FLOAT,
        VertexComponentType::Float2 => DXGI_FORMAT_R32G32_FLOAT,
        VertexComponentType::Float3 => DXGI_FORMAT_R32G32B32_FLOAT,
        VertexComponentType::Float4 => DXGI_FORMAT_R32G32B32A32_FLOAT,
        VertexComponentType::UByte4 => DXGI_FORMAT_R8G8B8A8_UINT,
        VertexComponentType::Colour => DXGI_FORMAT_R8G8B8A8_UNORM,
        VertexComponentType::Rgba8Snorm => DXGI_FORMAT_R8G8B8A8_SNORM,
        _ => return None,
    })
}

/// Elements for `decl`, plus a slot-1 dummy for every shader input the
/// declaration lacks. Returns the elements and the dummies' semantic names.
pub fn build_elements(
    decl: &VertexDeclaration,
    inputs: &Signature,
) -> (Vec<InputElement>, Vec<String>) {
    let mut elements = Vec::new();
    for c in &decl.components {
        let Some((name, index)) = semantic_name(c.semantic) else {
            continue;
        };
        let Some(format) = component_format(c.component_type) else {
            continue;
        };
        elements.push(InputElement::new(name, index, format, 0, c.offset as u32));
    }
    let mut dummies = Vec::new();
    for e in &inputs.elements {
        if e.system_value != 0 {
            continue; // SV_VertexID, SV_InstanceID: generated, never from a buffer
        }
        let present = elements.iter().any(|x| {
            x.semantic
                .to_str()
                .unwrap()
                .eq_ignore_ascii_case(&e.semantic)
                && x.index == e.index
        });
        if !present {
            let format = match e.component_type {
                1 => DXGI_FORMAT_R32G32B32A32_UINT,
                2 => DXGI_FORMAT_R32G32B32A32_SINT,
                _ => DXGI_FORMAT_R32G32B32A32_FLOAT,
            };
            elements.push(InputElement::new(
                &e.semantic,
                e.index,
                format,
                DUMMY_SLOT,
                0,
            ));
            dummies.push(format!("{}{}", e.semantic, e.index));
        }
    }
    (elements, dummies)
}

pub struct LayoutCache {
    layouts: HashMap<(u64, u64), ComPtr<ID3D11InputLayout>>,
}

impl LayoutCache {
    pub fn new() -> Self {
        LayoutCache {
            layouts: HashMap::new(),
        }
    }

    /// Layout for a declaration against a vertex shader, cached by
    /// (declaration id, blob identity).
    pub fn get(
        &mut self,
        dev: &Device,
        decl: &VertexDeclaration,
        inputs: &Signature,
        vs_dxbc: &[u8],
    ) -> Result<(ComPtr<ID3D11InputLayout>, Vec<String>)> {
        let key = (decl.declaration_id(), blob_key(vs_dxbc));
        let (elements, dummies) = build_elements(decl, inputs);
        if let Some(l) = self.layouts.get(&key) {
            return Ok((l.clone_ptr(), dummies));
        }
        let layout = dev.create_input_layout(&elements, vs_dxbc)?;
        self.layouts.insert(key, layout.clone_ptr());
        Ok((layout, dummies))
    }
}

impl Default for LayoutCache {
    fn default() -> Self {
        Self::new()
    }
}

/// The DXBC header carries a 16-byte digest of the blob; two words of it suffice as a key.
fn blob_key(dxbc: &[u8]) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&dxbc[4..12]);
    u64::from_le_bytes(b)
}
