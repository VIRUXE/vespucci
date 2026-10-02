//! Parses shader files extracted from the game. Skipped unless
//! `VESPUCCI_FXC_DIR` holds `.fxc` files (`vespucci cat .../normal_spec.fxc -o dir/normal_spec.fxc`).

use vespucci_fxc::{Dxbc, FxcFile, ProgramType};

#[test]
fn every_blob_parses_and_techniques_resolve() {
    let Some(dir) = std::env::var_os("VESPUCCI_FXC_DIR") else { return };
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "fxc") {
            continue;
        }
        let data = std::fs::read(&path).unwrap();
        let fxc = FxcFile::parse(&data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!fxc.techniques.is_empty(), "{}: no techniques", path.display());
        assert!(!fxc.groups[0].is_empty() && !fxc.groups[1].is_empty(), "{}: no vs/ps", path.display());

        for (gi, group) in fxc.groups.iter().enumerate() {
            for shader in group {
                let dxbc = Dxbc::parse(&shader.dxbc).unwrap_or_else(|e| panic!("{} {}: {e}", path.display(), shader.name));
                let (_, _, ty) = dxbc.version().expect("version token");
                let expected = [ProgramType::Vertex, ProgramType::Pixel, ProgramType::Compute, ProgramType::Domain, ProgramType::Geometry, ProgramType::Hull][gi];
                assert_eq!(ty, expected, "{} {}", path.display(), shader.name);
                dxbc.rdef().unwrap();
                if gi == 0 {
                    dxbc.input_signature().unwrap();
                }
            }
        }
        for t in &fxc.techniques {
            for p in &t.passes {
                for stage in 0..6 {
                    let idx = p.stage[stage] as usize;
                    assert!(idx <= fxc.groups[stage].len(), "{} technique {} stage {stage} index {idx} out of range", path.display(), t.name);
                }
            }
        }
        seen += 1;
    }
    assert!(seen > 0, "no .fxc files in VESPUCCI_FXC_DIR");
}
