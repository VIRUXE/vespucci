//! The Meta block table inside `.ymap`/`.ytyp` resources, enough to reach
//! records by their packed pointers. Copied from rage-formats' crate-private
//! helpers (VIRUXE, public domain).

use anyhow::{bail, Context, Result};
use rage_formats::resource::{u16_le, u32_le, u64_le, ResReader, GRAPHICS_BASE, SYSTEM_BASE};

pub struct MetaBlock {
    pub name_hash: u32,
    pub data: Vec<u8>,
}

/// Packed Meta pointer: block id (1-based) in the low 12 bits, offset in the next 20.
pub fn decode_meta_pointer(raw: u64) -> Option<(usize, usize)> {
    let block_id = (raw & 0xFFF) as usize;
    if block_id == 0 {
        return None;
    }
    Some((block_id - 1, ((raw >> 12) & 0xFFFFF) as usize))
}

pub fn read_meta_blocks(reader: &ResReader<'_>) -> Result<Vec<MetaBlock>> {
    let header = reader.resolve(SYSTEM_BASE, 0x70).context("Meta header out of bounds")?;
    let data_blocks_pointer = u64_le(header, 0x30);
    let data_blocks_count = u16_le(header, 0x4C) as usize;
    if data_blocks_pointer == 0 || data_blocks_count == 0 {
        bail!("no Meta data blocks");
    }
    let headers = reader.resolve(data_blocks_pointer, data_blocks_count * 16).context("Meta block array out of bounds")?;
    let mut blocks = Vec::with_capacity(data_blocks_count);
    for i in 0..data_blocks_count {
        let off = i * 16;
        let name_hash = u32_le(headers, off);
        let length = u32_le(headers, off + 4) as usize;
        let data_ptr = u64_le(headers, off + 8);
        let data = reader.resolve(data_ptr, length).map(|b| b.to_vec()).unwrap_or_default();
        let _ = GRAPHICS_BASE;
        blocks.push(MetaBlock { name_hash, data });
    }
    Ok(blocks)
}

/// A Meta array field: packed pointer at `off`, count at `off + 8`, as `stride`-byte records.
pub fn meta_array_records<'a>(blocks: &'a [MetaBlock], rec: &[u8], off: usize, stride: usize) -> Vec<&'a [u8]> {
    let count = u16_le(rec, off + 8) as usize;
    let Some((bi, boff)) = decode_meta_pointer(u64_le(rec, off)) else { return Vec::new() };
    let Some(block) = blocks.get(bi) else { return Vec::new() };
    (0..count).map_while(|i| block.data.get(boff + i * stride..boff + (i + 1) * stride)).collect()
}
