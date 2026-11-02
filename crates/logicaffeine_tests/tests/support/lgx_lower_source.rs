//! The LGX lowering's source as the source-reading locks see it: every `.rs` file of the
//! `parser/lgx_lower/` module, in name order. One reader, so no lock goes on reading a path the
//! module no longer has.

use std::path::{Path, PathBuf};

/// The `parser/lgx_lower/` module directory.
pub fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_language/src/parser/lgx_lower")
}

/// Every `.rs` file of the module as `(file name, source)`, sorted by name. A missing or empty
/// directory panics: a lock that read nothing would pass for the wrong reason.
pub fn files() -> Vec<(String, String)> {
    let dir = dir();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rs"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "{} holds no .rs files", dir.display());
    paths
        .into_iter()
        .map(|p| {
            let src = std::fs::read_to_string(&p)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()));
            (p.file_name().unwrap().to_string_lossy().into_owned(), src)
        })
        .collect()
}

/// The whole module's source: the files joined in name order.
pub fn source() -> String {
    files()
        .into_iter()
        .map(|(_, src)| src)
        .collect::<Vec<_>>()
        .join("\n")
}
