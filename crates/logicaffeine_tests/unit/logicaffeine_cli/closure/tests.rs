use super::*;

fn write(dir: &Path, rel: &str, body: &str) -> PathBuf {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn closure_lists_entry_and_every_import_once() {
    let dir = tempfile::tempdir().unwrap();
    // A diamond: main → {a, b}, a → c, b → c. `c` must appear exactly once.
    let entry = write(
        dir.path(),
        "main.lg",
        "# M\n\nImport A from \"a.lg\".\nImport B from \"deep/b.lg\".\n\n## Main\nShow one().\n",
    );
    write(
        dir.path(),
        "a.lg",
        "# A\n\nImport C from \"deep/c.lg\".\n\n## To one () -> Int:\n    Return base().\n",
    );
    write(
        dir.path(),
        "deep/b.lg",
        "# B\n\nImport C from \"c.lg\".\n\n## To two () -> Int:\n    Return base().\n",
    );
    write(
        dir.path(),
        "deep/c.lg",
        "# C\n\n## To base () -> Int:\n    Return 1.\n",
    );

    let closure = assemble_closure(&entry).expect("assembles");
    assert_eq!(
        closure.files.len(),
        4,
        "entry + 3 modules, diamond deduped: {:?}",
        closure.files
    );
    assert_eq!(
        closure.files[0],
        entry.canonicalize().unwrap(),
        "the entry leads"
    );
    for rel in ["a.lg", "deep/b.lg", "deep/c.lg"] {
        let canon = dir.path().join(rel).canonicalize().unwrap();
        assert!(
            closure.files.contains(&canon),
            "{rel} is in the closure: {:?}",
            closure.files
        );
    }
    // The combined source carries the imported definitions.
    for def in ["## To one", "## To two", "## To base"] {
        assert!(
            closure.source.contains(def),
            "{def} spliced into the source"
        );
    }
}

#[test]
fn an_unresolvable_import_is_a_loud_error() {
    let dir = tempfile::tempdir().unwrap();
    let entry = write(
        dir.path(),
        "main.lg",
        "# M\n\nImport Ghost from \"ghost.lg\".\n\n## Main\nShow 1.\n",
    );
    let err = assemble_closure(&entry).expect_err("a missing import cannot assemble");
    assert!(
        err.contains("ghost"),
        "the error names the missing module: {err}"
    );
}

#[test]
fn a_single_file_project_closes_over_just_its_entry() {
    let dir = tempfile::tempdir().unwrap();
    let entry = write(dir.path(), "main.lg", "# M\n\n## Main\nShow 1.\n");
    let closure = assemble_closure(&entry).expect("assembles");
    assert_eq!(closure.files, vec![entry.canonicalize().unwrap()]);
}
