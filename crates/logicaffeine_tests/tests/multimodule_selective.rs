//! Selective per-name imports: `Import double from Util.` brings ONLY the named
//! definitions into scope (not the whole module), with an optional per-name alias
//! (`Import double as dbl from Util.`). `from "quoted"` stays a PATH; `from
//! Bareword` is a selective source module.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use logicaffeine_compile::analysis::assemble_project;
use logicaffeine_compile::compile::enforce_project_scope;

fn interpret_project(entry: &Path) -> String {
    enforce_project_scope(entry).expect("project scope check");
    let assembled = assemble_project(entry).expect("assemble project");
    let result = futures::executor::block_on(logicaffeine_compile::interpret_for_ui_with_args(
        &assembled.source,
        &["main".to_string()],
    ));
    assert!(
        result.error.is_none(),
        "interpreter error: {:?}",
        result.error
    );
    result.lines.join("\n")
}

/// A module exporting BOTH `double` and `triple`.
const UTIL: &str = r#"# Util

## To double (n: Int) -> Int:
    Return n times 2.

## To triple (n: Int) -> Int:
    Return n times 3.
"#;

fn write_util_project(dir: &Path, main: &str) -> std::path::PathBuf {
    fs::write(dir.join("util.lg"), UTIL).unwrap();
    let main_path = dir.join("main.lg");
    fs::write(&main_path, main).unwrap();
    main_path
}

/// `Import double from Util.` brings ONLY `double` — the named function runs, but
/// `triple` (not selected) is out of scope.
#[test]
fn selective_import_brings_only_named() {
    let dir = tempdir().unwrap();
    let main = write_util_project(
        dir.path(),
        "# App\n\nImport double from Util.\n\n## Main\nShow double(21).\n",
    );
    assert_eq!(interpret_project(&main), "42");

    // `triple` was NOT selected → referencing it is a scope error.
    fs::write(
        &main,
        "# App\n\nImport double from Util.\n\n## Main\nShow triple(21).\n",
    )
    .unwrap();
    let err = enforce_project_scope(&main).expect_err("an unselected name must be out of scope");
    assert!(
        err.contains("triple"),
        "error must name the unselected function, got: {err}"
    );
}

/// `Import double, triple from Util.` selects several names (comma list).
#[test]
fn selective_import_multiple_names() {
    let dir = tempdir().unwrap();
    let main = write_util_project(
        dir.path(),
        "# App\n\nImport double, triple from Util.\n\n## Main\nShow double(10) plus triple(10).\n",
    );
    assert_eq!(interpret_project(&main), "50");
}

/// The English `and` list works for selection too.
#[test]
fn selective_import_and_list() {
    let dir = tempdir().unwrap();
    let main = write_util_project(dir.path(), "# App\n\nImport double and triple from Util.\n\n## Main\nShow double(10) plus triple(10).\n");
    assert_eq!(interpret_project(&main), "50");
}

/// `Import Util from "path".` with a QUOTED source is still a PATH import (whole
/// module), not a selective import of a name called `Util`.
#[test]
fn selective_from_quoted_is_still_a_path() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("lib")).unwrap();
    fs::write(dir.path().join("lib/util.lg"), UTIL).unwrap();
    let main = dir.path().join("main.lg");
    fs::write(
        &main,
        "# App\n\nImport Util from \"lib/util.lg\".\n\n## Main\nShow double(21).\n",
    )
    .unwrap();
    assert_eq!(interpret_project(&main), "42");
}

/// Whole-module `Import Util.` is unaffected — it still brings everything.
#[test]
fn whole_module_import_still_brings_everything() {
    let dir = tempdir().unwrap();
    let main = write_util_project(
        dir.path(),
        "# App\n\nImport Util.\n\n## Main\nShow double(10) plus triple(10).\n",
    );
    assert_eq!(interpret_project(&main), "50");
}

/// A per-name alias is LOCAL to the importing file — it must not rewrite an
/// unrelated identifier of the same spelling in ANOTHER module.
#[test]
fn selective_alias_is_scoped_to_the_importing_file() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    // `Other` has its OWN unrelated function `dbl`.
    fs::write(
        dir.path().join("other.lg"),
        "# Other\n\n## To dbl (n: Int) -> Int:\n    Return n plus 1000.\n",
    )
    .unwrap();
    // `mid` uses Other's `dbl`.
    fs::write(
        dir.path().join("mid.lg"),
        "# Mid\n\nImport Other.\n\n## To midVal (n: Int) -> Int:\n    Return dbl(n).\n",
    )
    .unwrap();
    let main = dir.path().join("main.lg");
    // main aliases Util's `double` AS `dbl` — this must not disturb Other/mid.
    fs::write(&main, "# App\n\nImport Mid.\nImport double as dbl from Util.\n\n## Main\nShow dbl(21) plus midVal(0).\n").unwrap();

    // main's `dbl` = Util's double(21) = 42; midVal(0) = Other's dbl(0) = 1000 → 1042.
    assert_eq!(interpret_project(&main), "1042");
}

/// A per-name alias: `Import double as dbl from Util.` binds `dbl` to `double`.
#[test]
fn selective_import_per_name_alias() {
    let dir = tempdir().unwrap();
    let main = write_util_project(
        dir.path(),
        "# App\n\nImport double as dbl from Util.\n\n## Main\nShow dbl(21).\n",
    );
    assert_eq!(interpret_project(&main), "42");
}

/// The alias resolves the same INSIDE a string interpolation — `{dbl(21)}` is a
/// real call, so `bake_selective_aliases` rewrites the hole to `double` just like
/// a bare `dbl(21)`. (The third leg of hole-aware name rewriting, alongside
/// private-mangle and qualified-call resolution.)
#[test]
fn selective_alias_resolves_inside_a_string_interpolation() {
    let dir = tempdir().unwrap();
    let main = write_util_project(
        dir.path(),
        "# App\n\nImport double as dbl from Util.\n\n## Main\nShow \"doubled is {dbl(21)}\".\n",
    );
    assert_eq!(interpret_project(&main), "doubled is 42");
}
