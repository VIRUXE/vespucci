//! Access to a GTA V (Legacy PC) install.
//!
//! * [`keys`] — RPF keys recovered from the user's own `GTA5.exe`, cached per build.
//! * [`archive`] — a memory-mapped archive with nested archives as windows onto it.
//! * [`load_order`] — every `.rpf` under the game folder in the game's own
//!   override order: base, then `update.rpf`, then DLC packs by `dlclist.xml`/`setup2.xml`.
//! * [`vfs`] — [`GameFs`]: one flat table of every file across every archive,
//!   nested ones included, with last-archive-wins lookups by path and by name hash.
//!
//! `archive` and `load_order` are carried over from rage-cli (VIRUXE, public
//! domain / Unlicense); see `docs/THIRD_PARTY.md`.

pub mod archive;
pub mod keys;
pub mod load_order;
pub mod vfs;

pub use archive::Archive;
pub use rpf_archive::{rage_joaat as joaat, GtaKeys};
pub use vfs::{FileLoc, GameFs};
