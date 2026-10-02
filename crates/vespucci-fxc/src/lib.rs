//! The game's compiled shaders: the `.fxc` container and DXBC reflection.

pub mod dxbc;
pub mod fxc;

pub use dxbc::{BindKind, Dxbc, ProgramType, Rdef, Signature};
pub use fxc::{FxcFile, Pass, Shader, Technique, VarType, Variable, STAGE_NAMES};

/// Where the game's current shaders live, most authoritative first. The
/// `update2.rpf` copies supersede the originals in `common.rpf`.
pub const SHADER_DIRS: [&str; 2] = [
    "update/update2.rpf/common/shaders/win32_40_final",
    "common.rpf/shaders/win32_40_final",
];
