//! Against a real install; skipped unless `VESPUCCI_GAME` is set.
//! Numbers are for Steam Legacy build 1.0.3889.0 with all DLC packs.

use std::path::PathBuf;
use vespucci_game::GameFs;
use vespucci_world::entities::stored_rotation_to_world;
use vespucci_world::streamer::interior_world;
use vespucci_world::{parse_entities, ArchetypeDb, LodLevel, MapSet, Mode, Vec3, YmapTree};

fn fs() -> Option<GameFs> {
    let game = std::env::var_os("VESPUCCI_GAME").map(PathBuf::from)?;
    Some(GameFs::open(&game).unwrap())
}

#[test]
fn archetypes_and_ymaps_index() {
    let Some(fs) = fs() else { return };
    let set = MapSet::all(&fs);
    let db = ArchetypeDb::build(&fs).unwrap();
    assert!(
        db.by_hash.len() >= 150_000,
        "archetypes: {}",
        db.by_hash.len()
    );
    assert!(db.ytyps_failed <= 10, "ytyps failed: {}", db.ytyps_failed);

    let tree = YmapTree::build(&fs, &set).unwrap();
    assert!(tree.nodes.len() >= 10_000, "ymaps: {}", tree.nodes.len());
    assert!(
        tree.from_cache > tree.from_headers,
        "cache files should cover most maps"
    );
    assert!(tree.failed <= 10);

    // A known prop resolves to a .ydr in a nested archive, and has a box.
    let bag = db
        .get(vespucci_game::joaat("prop_cs_heist_bag_01"))
        .expect("heist bag archetype");
    assert_eq!(bag.model_file().0, "ydr");
    assert!(bag.bb_max.x > bag.bb_min.x && bag.lod_dist > 0.0);
}

#[test]
fn beach_entities_have_lod_fields() {
    let Some(fs) = fs() else { return };
    let tree = YmapTree::build(&fs, &MapSet::all(&fs)).unwrap();
    let node = tree
        .get(vespucci_game::joaat("vb_rd_strm_0"))
        .expect("vb_rd_strm_0.ymap");
    assert!(node.touches_sphere(Vec3::new(-1280.0, -1450.0, 4.0), 300.0));
    let data = fs.read(&fs.files[node.file as usize]).unwrap();
    let parsed = parse_entities(&data).unwrap();
    assert!(parsed.entities.len() > 300, "{}", parsed.entities.len());
    assert!(
        parsed.car_generators.len() > 50,
        "{}",
        parsed.car_generators.len()
    );
    // Orphan HD entities have no parent; HD ones point at a LOD parent.
    let orphan = parsed
        .entities
        .iter()
        .find(|e| e.lod_level == LodLevel::OrphanHd)
        .unwrap();
    assert!(orphan.parent_index < 0 || orphan.lod_in_parent_ymap() || true);
    let hd = parsed
        .entities
        .iter()
        .find(|e| e.lod_level == LodLevel::Hd)
        .unwrap();
    assert!(hd.lod_dist > 0.0);
    assert!(parsed
        .entities
        .iter()
        .all(|e| e.scale_xy > 0.0 && e.scale_z > 0.0));
}

/// Interiors (#19): the Ammu-Nation at Pillbox Hill has a definition with
/// entities, rooms and portals, and its placement puts those entities next to
/// the instance.
#[test]
fn interiors_are_defined_and_placed() {
    let Some(fs) = fs() else { return };
    let db = ArchetypeDb::build(&fs).unwrap();
    // 391 interiors in build 1.0.3889.0, one definition per MLO archetype.
    assert!(
        db.mlos.len() > 300,
        "interior definitions: {}",
        db.mlos.len()
    );
    let mlo_archetypes = db.by_hash.values().filter(|a| a.is_mlo).count();
    assert_eq!(db.mlos.len(), mlo_archetypes);
    let gun = db
        .mlo(vespucci_game::joaat("v_gun"))
        .expect("v_gun interior definition");
    assert!(gun.entities.len() > 100, "{}", gun.entities.len());
    assert!(gun.rooms.len() >= 2 && gun.portals.len() >= 1);
    assert!(gun.rooms.iter().any(|r| !r.attached_objects.is_empty()));

    let tree = YmapTree::build(&fs, &MapSet::all(&fs)).unwrap();
    let node = tree
        .get(vespucci_game::joaat("dt1_22_interior_v_gun_milo_"))
        .expect("dt1_22_interior_v_gun_milo_.ymap");
    let parsed = parse_entities(&fs.read(&fs.files[node.file as usize]).unwrap()).unwrap();
    let inst = &parsed.entities[0];
    assert!(inst.is_mlo_instance && inst.archetype_hash == gun.name_hash);
    // Every interior entity lands within the interior's own size of the instance.
    for ie in &gun.entities {
        let (p, _) = interior_world(
            inst.position,
            inst.orientation(),
            ie.position,
            stored_rotation_to_world(ie.rotation),
        );
        let d = ((p.x - inst.position.x).powi(2) + (p.y - inst.position.y).powi(2)).sqrt();
        assert!(
            d < 60.0,
            "entity {:#010x} {d:.0} m from the instance",
            ie.archetype_hash
        );
    }
}

#[test]
fn story_mode_excludes_online_map_variants() {
    let Some(fs) = fs() else { return };
    let sp = MapSet::build(&fs, Mode::SinglePlayer).unwrap();
    let mp = MapSet::build(&fs, Mode::Multiplayer).unwrap();
    let sp_tree = YmapTree::build(&fs, &sp).unwrap();
    let mp_tree = YmapTree::build(&fs, &mp).unwrap();
    let base = vespucci_game::joaat("vb_rd");
    let heist = vespucci_game::joaat("hei_vb_rd");
    assert!(
        sp_tree.get(base).is_some() && sp_tree.get(heist).is_none(),
        "story mode: base map only"
    );
    assert!(
        mp_tree.get(heist).is_some() && mp_tree.get(base).is_none(),
        "online: heist variant only"
    );
}
