use super::*;
use tempfile::tempdir;

fn url(path: &std::path::Path) -> Url {
    Url::from_file_path(path).unwrap()
}

#[test]
fn module_exporting_names_the_owning_file() {
    let dir = tempdir().unwrap();
    let util = dir.path().join("util.lg");
    std::fs::write(
        &util,
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    let idx = WorkspaceIndex::new();
    idx.index_file(&util);

    let exp = idx.module_exporting("double");
    assert_eq!(exp.len(), 1, "exactly one module exports `double`");
    assert_eq!(exp[0].module_name, "Util");
    assert_eq!(exp[0].module_url, url(&util));
}

#[test]
fn imports_of_reads_the_abstract() {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.lg");
    std::fs::write(&main, "# App\n\nImport Util.\n\n## Main\nShow double(1).\n").unwrap();
    let idx = WorkspaceIndex::new();
    idx.index_file(&main);

    assert_eq!(idx.imports_of(&url(&main)), vec!["Util".to_string()]);
}

#[test]
fn two_modules_exporting_same_name_return_both() {
    let dir = tempdir().unwrap();
    let a = dir.path().join("a.lg");
    let b = dir.path().join("b.lg");
    std::fs::write(&a, "# A\n\n## To helper (n: Int) -> Int:\n    Return n.\n").unwrap();
    std::fs::write(&b, "# B\n\n## To helper (n: Int) -> Int:\n    Return n.\n").unwrap();
    let idx = WorkspaceIndex::new();
    idx.index_file(&a);
    idx.index_file(&b);

    assert_eq!(
        idx.module_exporting("helper").len(),
        2,
        "both modules export `helper`"
    );
}

#[test]
fn private_names_are_not_workspace_exports() {
    // `## To private secret` is usable inside util.lg but MUST NOT be
    // importable — so it must never surface as a workspace export (which
    // powers cross-module completion and the auto-import quickfix).
    let dir = tempdir().unwrap();
    let util = dir.path().join("util.lg");
    std::fs::write(
        &util,
        "# Util\n\n## To double (n: Int) -> Int:\n    Return secret(n).\n\n\
             ## To private secret (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    let idx = WorkspaceIndex::new();
    idx.index_file(&util);

    let exports = idx.exports_of(&url(&util));
    assert!(
        exports.contains(&"double".to_string()),
        "public `double` is exported: {exports:?}"
    );
    assert!(
        !exports.contains(&"secret".to_string()),
        "private `secret` must NOT be a workspace export: {exports:?}"
    );
    assert!(
        !exports.iter().any(|e| e == "private"),
        "the `private` marker must never leak as a symbol name: {exports:?}"
    );
    assert!(
        idx.module_exporting("secret").is_empty(),
        "no module publicly exports the private `secret`"
    );
}

#[test]
fn is_imported_matches_exporter() {
    let dir = tempdir().unwrap();
    let util = dir.path().join("util.lg");
    let main = dir.path().join("main.lg");
    let other = dir.path().join("other.lg");
    std::fs::write(
        &util,
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    std::fs::write(&main, "# App\n\nImport Util.\n\n## Main\nShow double(1).\n").unwrap();
    std::fs::write(&other, "# Other\n\n## Main\nShow 1.\n").unwrap();
    let idx = WorkspaceIndex::new();
    idx.index_file(&util);
    idx.index_file(&main);
    idx.index_file(&other);

    assert!(
        WorkspaceView::is_imported(&idx, &url(&main), "double"),
        "main imports Util → double is importable"
    );
    assert!(
        !WorkspaceView::is_imported(&idx, &url(&other), "double"),
        "other does not import Util"
    );
}
