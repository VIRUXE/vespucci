//! The game world as data.
//!
//! * [`ArchetypeDb`] — every archetype from every `.ytyp`, last archive wins.
//! * [`YmapTree`] — every `.ymap` with extents and parent links.
//! * [`entities`] — a map's placed entities with their LOD fields, and car generators.

pub mod archetypes;
pub mod entities;
pub mod mapset;
pub mod meta;
pub mod streamer;
pub mod txd;
pub mod ymaps;

pub use archetypes::{ArchetypeDb, ArchetypeRec};
pub use entities::{parse_entities, CarGenerator, Entity, LodLevel, YmapEntities};
pub use mapset::{MapSet, Mode};
pub use rage_formats::Vec3;
pub use streamer::{child_lod_dist, collect, lod_dist, Instance, StreamOptions, StreamStats};
pub use txd::TxdParents;
pub use ymaps::{YmapNode, YmapTree};
