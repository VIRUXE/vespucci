//! The placed entities of a `.ymap`: `CEntityDef` with the LOD fields the
//! streaming rule needs, plus car generators.

use crate::meta::{decode_meta_pointer, meta_array_records, read_meta_blocks};
use anyhow::{Context, Result};
use rage_formats::resource::{f32_le, u32_le, u64_le, vec3_le, ResReader};
use rage_formats::{prepare_rsc7, Vec3};

const HASH_CMAPDATA: u32 = 3_545_841_574;
const HASH_CENTITYDEF: u32 = 3_461_354_627;
const HASH_CMLOINSTANCEDEF: u32 = 164_374_718;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LodLevel {
    Hd = 0,
    Lod = 1,
    Slod1 = 2,
    Slod2 = 3,
    Slod3 = 4,
    OrphanHd = 5,
    Slod4 = 6,
}

impl LodLevel {
    pub fn from_u32(v: u32) -> LodLevel {
        match v {
            1 => LodLevel::Lod,
            2 => LodLevel::Slod1,
            3 => LodLevel::Slod2,
            4 => LodLevel::Slod3,
            5 => LodLevel::OrphanHd,
            6 => LodLevel::Slod4,
            _ => LodLevel::Hd,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entity {
    pub archetype_hash: u32,
    pub flags: u32,
    pub guid: u32,
    pub position: Vec3,
    /// Quaternion as stored (x, y, z, w). Map entities store the inverse
    /// rotation; use [`Entity::orientation`] for the world rotation.
    pub rotation: [f32; 4],
    pub scale_xy: f32,
    pub scale_z: f32,
    /// Index into this map's entities, or into the parent map's when
    /// [`Entity::lod_in_parent_ymap`] is set; negative for none.
    pub parent_index: i32,
    pub lod_dist: f32,
    pub child_lod_dist: f32,
    pub lod_level: LodLevel,
    pub num_children: u32,
    pub priority_level: u32,
    pub tint: u32,
    pub is_mlo_instance: bool,
}

impl Entity {
    /// `flags` bit 3: the parent lives in the parent ymap's entity list.
    pub fn lod_in_parent_ymap(&self) -> bool {
        (self.flags >> 3) & 1 == 1
    }

    /// World rotation as a quaternion (x, y, z, w). Non-MLO entities store the
    /// inverse, so this is the conjugate; MLO instances store it directly.
    pub fn orientation(&self) -> [f32; 4] {
        let [x, y, z, w] = self.rotation;
        if self.is_mlo_instance || (x == 0.0 && y == 0.0 && z == 0.0) {
            self.rotation
        } else {
            [-x, -y, -z, w]
        }
    }

    pub fn to_world(&self, local: Vec3) -> Vec3 {
        let scaled = Vec3::new(local.x * self.scale_xy, local.y * self.scale_xy, local.z * self.scale_z);
        rage_formats::ymap::rotate(scaled, self.orientation()) + self.position
    }
}

#[derive(Debug, Clone)]
pub struct CarGenerator {
    pub position: Vec3,
    pub orient_x: f32,
    pub orient_y: f32,
    pub perpendicular_length: f32,
    pub car_model: u32,
    pub flags: u32,
    pub body_colours: [u32; 4],
    pub pop_group: u32,
    pub livery: i8,
}

pub struct YmapEntities {
    /// `CMapData::flags`: bit 0 script-requested (not streamed by position), bit 1 LOD.
    pub flags: u32,
    pub content_flags: u32,
    pub entities: Vec<Entity>,
    pub car_generators: Vec<CarGenerator>,
}

pub fn parse_entities(data: &[u8]) -> Result<YmapEntities> {
    let (system, graphics) = prepare_rsc7(data)?;
    let reader = ResReader { system: &system, graphics: &graphics };
    let blocks = read_meta_blocks(&reader)?;
    let map = blocks.iter().find(|b| b.name_hash == HASH_CMAPDATA).context("CMapData block not found")?;
    let d = &map.data;
    anyhow::ensure!(d.len() >= 512, "CMapData block too small");

    let mut entities = Vec::new();
    let count = rage_formats::resource::u16_le(d, 96 + 8) as usize;
    if let Some((bi, off)) = decode_meta_pointer(u64_le(d, 96)) {
        if let Some(arr) = blocks.get(bi).and_then(|b| b.data.get(off..off + count * 8)) {
            for i in 0..count {
                let Some((ebi, eoff)) = decode_meta_pointer(u64_le(arr, i * 8)) else { continue };
                let Some(block) = blocks.get(ebi) else { continue };
                let is_mlo = match block.name_hash {
                    HASH_CENTITYDEF => false,
                    HASH_CMLOINSTANCEDEF => true,
                    _ => continue,
                };
                let Some(e) = block.data.get(eoff..eoff + 128) else { continue };
                entities.push(Entity {
                    archetype_hash: u32_le(e, 8),
                    flags: u32_le(e, 12),
                    guid: u32_le(e, 16),
                    position: vec3_le(e, 32),
                    rotation: [f32_le(e, 48), f32_le(e, 52), f32_le(e, 56), f32_le(e, 60)],
                    scale_xy: f32_le(e, 64),
                    scale_z: f32_le(e, 68),
                    parent_index: u32_le(e, 72) as i32,
                    lod_dist: f32_le(e, 76),
                    child_lod_dist: f32_le(e, 80),
                    lod_level: LodLevel::from_u32(u32_le(e, 84)),
                    num_children: u32_le(e, 88),
                    priority_level: u32_le(e, 92),
                    tint: u32_le(e, 120),
                    is_mlo_instance: is_mlo,
                });
            }
        }
    }

    let car_generators = meta_array_records(&blocks, d, 240, 80)
        .into_iter()
        .map(|c| CarGenerator {
            position: vec3_le(c, 16),
            orient_x: f32_le(c, 32),
            orient_y: f32_le(c, 36),
            perpendicular_length: f32_le(c, 40),
            car_model: u32_le(c, 44),
            flags: u32_le(c, 48),
            body_colours: [u32_le(c, 52), u32_le(c, 56), u32_le(c, 60), u32_le(c, 64)],
            pop_group: u32_le(c, 68),
            livery: c[72] as i8,
        })
        .collect();

    Ok(YmapEntities { flags: u32_le(d, 16), content_flags: u32_le(d, 20), entities, car_generators })
}
