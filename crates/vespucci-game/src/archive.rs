//! A parsed RPF archive over memory-mapped bytes. Carried over from
//! rage-cli's `rpf.rs` (VIRUXE, public domain).
//!
//! A top-level archive is memory-mapped, so only the pages actually touched
//! (the table of contents, the entries extracted) are read from disk. A
//! nested archive stored uncompressed and unencrypted — which is how the game
//! stores them — is a narrower window onto its parent's buffer, not a copy.

use anyhow::Result;
use rpf_archive::{build_directory_tree, list_all_files, DirNode, FileRef, GtaKeys, RpfArchive, RpfEncryption, RpfEntryKind, RpfVersion};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone)]
struct Backing {
    buffer: Arc<Buffer>,
    range: std::ops::Range<usize>,
}

enum Buffer {
    Owned(Vec<u8>),
    Mapped(memmap2::Mmap),
}

impl Backing {
    fn owned(data: Vec<u8>) -> Self {
        let range = 0..data.len();
        Self { buffer: Arc::new(Buffer::Owned(data)), range }
    }

    fn bytes(&self) -> &[u8] {
        let all: &[u8] = match &*self.buffer {
            Buffer::Owned(v) => v,
            Buffer::Mapped(m) => m,
        };
        &all[self.range.clone()]
    }
}

pub struct Archive {
    pub path: PathBuf,
    pub encryption: RpfEncryption,
    pub root: DirNode,
    archive: RpfArchive,
    data: Backing,
}

impl Archive {
    pub fn open(path: &Path, keys: Option<&GtaKeys>) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        // SAFETY: read-only map; a game archive modified while mapped would
        // yield garbage entries, not UB in code that only reads bytes.
        let backing = match unsafe { memmap2::Mmap::map(&file) } {
            Ok(map) => {
                let range = 0..map.len();
                Backing { buffer: Arc::new(Buffer::Mapped(map)), range }
            }
            Err(_) => Backing::owned(std::fs::read(path)?),
        };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        Self::from_backing(backing, &name, keys)
    }

    pub fn from_bytes(data: Vec<u8>, name: &str, keys: Option<&GtaKeys>) -> Result<Self> {
        Self::from_backing(Backing::owned(data), name, keys)
    }

    fn from_backing(data: Backing, name: &str, keys: Option<&GtaKeys>) -> Result<Self> {
        let archive = RpfArchive::parse(data.bytes(), name, keys)?;
        let encryption = archive.encryption;
        let root = build_directory_tree(&archive.entries);
        Ok(Self { path: PathBuf::from(name), encryption, root, archive, data })
    }

    /// Opens a nested `.rpf` entry: a window onto this archive's bytes when
    /// it is stored plain, an extracted copy otherwise.
    pub fn open_nested(&self, file: &FileRef, keys: Option<&GtaKeys>) -> Result<Self> {
        if let Some(range) = self.stored_range(file) {
            let mut data = self.data.clone();
            data.range = data.range.start + range.start..data.range.start + range.end;
            return Self::from_backing(data, &file.name, keys);
        }
        Self::from_bytes(self.extract(file, keys)?, &file.name, keys)
    }

    fn stored_range(&self, file: &FileRef) -> Option<std::ops::Range<usize>> {
        let RpfEntryKind::BinaryFile { file_offset, file_size, uncompressed_size, is_encrypted } = self.archive.entries[file.entry_index].kind else {
            return None;
        };
        if is_encrypted || (file_size != 0 && file_size != uncompressed_size) || uncompressed_size == 0 {
            return None;
        }
        let offset = match self.archive.version {
            RpfVersion::V7 => file_offset as usize * 512,
            _ => file_offset as usize,
        };
        let start = self.archive.start_offset + offset;
        let end = start.checked_add(uncompressed_size as usize)?;
        (end <= self.data.range.len()).then_some(start..end)
    }

    pub fn require_keys(&self, keys: Option<&GtaKeys>) -> Result<()> {
        if keys.is_none() && matches!(self.encryption, RpfEncryption::Ng | RpfEncryption::Aes) {
            anyhow::bail!("archive is {:?}-encrypted and no keys are available", self.encryption);
        }
        Ok(())
    }

    pub fn list_files(&self) -> Vec<&FileRef> {
        list_all_files(&self.root)
    }

    /// A file by its path inside this archive, case-insensitive, either separator.
    pub fn find_file(&self, path: &str) -> Option<&FileRef> {
        let path = path.replace('\\', "/").to_lowercase();
        let mut parts = path.split('/').filter(|p| !p.is_empty()).peekable();
        let mut dir = &self.root;
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                return dir.files.iter().find(|f| f.name.eq_ignore_ascii_case(part));
            }
            dir = dir.subdirs.iter().find(|d| d.name.eq_ignore_ascii_case(part))?;
        }
        None
    }

    pub fn extract(&self, file: &FileRef, keys: Option<&GtaKeys>) -> Result<Vec<u8>> {
        let entry = &self.archive.entries[file.entry_index];
        self.archive.extract_entry(self.data.bytes(), entry, keys)
    }

    pub fn entry_kind(&self, file: &FileRef) -> &RpfEntryKind {
        &self.archive.entries[file.entry_index].kind
    }
}
