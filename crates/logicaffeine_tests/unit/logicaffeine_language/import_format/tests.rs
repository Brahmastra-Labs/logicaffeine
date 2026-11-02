use super::*;

#[test]
fn sorts_and_dedupes_imports() {
    let src = "# App\n\nImport Zebra.\nImport Alpha.\nImport Zebra.\n\n## Main\nShow 1.\n";
    let out = normalize_imports(src);
    assert_eq!(
        out, "# App\n\nImport Alpha.\nImport Zebra.\n\n## Main\nShow 1.\n",
        "imports must be sorted + de-duplicated, prose/code untouched"
    );
}

#[test]
fn normalization_is_idempotent() {
    let src = "# App\n\nImport Zebra.\nImport Alpha.\nImport Alpha.\n\n## Main\nShow 1.\n";
    let once = normalize_imports(src);
    assert_eq!(
        normalize_imports(&once),
        once,
        "normalization must be a fixed point"
    );
}

#[test]
fn a_single_import_is_untouched() {
    let src = "# App\n\nImport Util.\n\n## Main\nShow double(1).\n";
    assert_eq!(normalize_imports(src), src);
}

#[test]
fn prose_starting_with_import_is_not_reordered() {
    // "Import your data first." is prose, not an import statement.
    let src = "# App\n\nImport your data first.\nImport Beta.\nImport Alpha.\n\n## Main\nShow 1.\n";
    let out = normalize_imports(src);
    assert!(
        out.contains("Import your data first."),
        "prose must survive: {out}"
    );
    // The two real imports are sorted.
    let alpha = out.find("Import Alpha.").unwrap();
    let beta = out.find("Import Beta.").unwrap();
    assert!(alpha < beta, "real imports sorted: {out}");
}
