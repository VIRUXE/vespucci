//! Tests against a real install. Skipped unless `VESPUCCI_GAME` points at one.
//! Numbers below are for the Steam Legacy build 1.0.3889.0 with all 92 DLC packs.

use std::path::PathBuf;
use vespucci_game::{keys, GameFs};

fn game() -> Option<PathBuf> {
    std::env::var_os("VESPUCCI_GAME").map(PathBuf::from)
}

#[test]
fn keys_and_version() {
    let Some(game) = game() else { return };
    let exe = keys::resolve_exe(&game).unwrap();
    let version = keys::exe_version(&exe).expect("version resource");
    assert!(version.starts_with("1.0."), "{version}");
    keys::load(&game).expect("keys recover from the exe");
}

#[test]
fn whole_install_scans() {
    let Some(game) = game() else { return };
    let fs = GameFs::open(&game).unwrap();
    assert!(fs.archives.len() >= 100, "archives: {}", fs.archives.len());
    assert!(fs.files.len() >= 380_000, "files: {}", fs.files.len());

    // The three tiers are in load order: base first, update.rpf, then DLC packs.
    let first = fs.archive_display(0);
    let last = fs.archive_display(fs.archives.len() as u32 - 1);
    assert!(!first.contains("update/"), "{first}");
    assert!(last.contains("dlcpacks/"), "{last}");

    let shaders: Vec<_> = fs
        .files
        .iter()
        .filter(|f| f.ext == "fxc" && f.archive == 0)
        .collect();
    assert!(
        shaders.len() >= 300,
        "fxc in first archive: {}",
        shaders.len()
    );

    let dlclist = fs
        .get("update/update.rpf/common/data/dlclist.xml")
        .expect("dlclist.xml");
    let xml = String::from_utf8(fs.read(dlclist).unwrap()).unwrap();
    assert!(xml.matches("<Item>").count() >= 90);

    // Name resolution goes through nested archives.
    let bag = fs
        .by_name("ydr", "prop_cs_heist_bag_01")
        .expect("heist bag");
    assert_eq!(bag.nested.len(), 1);
    let bytes = fs.read(bag).unwrap();
    assert_eq!(&bytes[..4], b"RSC7");
}
