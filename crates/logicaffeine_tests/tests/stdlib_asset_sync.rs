//! The stdlib ships from TWO asset trees: `crates/logicaffeine_compile/assets/std/`
//! (the `include_str!` source of truth the prelude embeds) and the repository's
//! `assets/std/` (the browsable copy the formatter corpus reads). They drifted
//! silently once (`## To native args` vs `## To native args ()`); this lock makes
//! that class of drift impossible: every root file must exist in the compile
//! tree and match it byte for byte. The compile copy is canonical — fix drift
//! by copying compile → root, never the other way.

use std::fs;
use std::path::Path;

#[test]
fn every_root_stdlib_asset_matches_the_canonical_compile_copy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/std");
    let canonical =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/assets/std");

    let mut checked = 0;
    for entry in fs::read_dir(&root).expect("root assets/std exists") {
        let entry = entry.unwrap();
        let name = entry.file_name();
        let canonical_path = canonical.join(&name);
        assert!(
            canonical_path.exists(),
            "{}: exists in assets/std but not in the canonical compile tree — \
             add it there (the prelude embeds ONLY the compile tree)",
            name.to_string_lossy()
        );
        let root_bytes = fs::read(entry.path()).unwrap();
        let canonical_bytes = fs::read(&canonical_path).unwrap();
        assert_eq!(
            root_bytes,
            canonical_bytes,
            "{}: assets/std drifted from crates/logicaffeine_compile/assets/std — \
             copy the compile version over the root copy (compile is canonical)",
            name.to_string_lossy()
        );
        checked += 1;
    }
    assert!(checked >= 6, "the root asset tree lost files (found {checked})");
}
