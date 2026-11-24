//! The real checkpoints under the gitignored `models/`, which a clone does not carry.

use std::path::PathBuf;

/// The checkpoint `name` names under `models/`: the GGUF `<dir>/<file>.gguf`, or the one GGUF in `<dir>/`; `None` where
/// it is absent. A directory of several GGUFs names none of them, and a test reading it by the directory alone would
/// be silently skipped: it panics, naming the files, so the test names its file or the extra moves to a directory of its
/// own (`models/<dir>-<variant>/`).
pub fn checkpoint(name: &str) -> Option<PathBuf> {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models").join(name);
    if name.ends_with(".gguf") {
        return d.is_file().then_some(d);
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(&d)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "gguf"))
        .collect();
    found.sort();
    match found.len() {
        0 => None,
        1 => Some(found.remove(0)),
        _ => panic!(
            "models/{name}/ holds {} GGUFs ({}): name one as `{name}/<file>.gguf`, or move the others to directories of their own",
            found.len(),
            found.iter().filter_map(|p| p.file_name()).map(|f| f.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
        ),
    }
}
