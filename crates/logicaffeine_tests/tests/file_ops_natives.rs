//! ★ LOCK — file.lg's `appendFile`, `fileExists`, `listDir`, `makeDirs` and `renameFile` on every engine. ★
//!
//! `read` and `write` replace a whole file; a program that keeps a ledger, a cache or a work
//! directory also needs to add to a file, ask whether something is there, list a directory, make one,
//! and replace a file atomically. Each is a `## To native` declaration in file.lg routed through the
//! ONE system-native table (`semantics::builtins::system_native`) the tree-walker and the VM read and
//! linked by the Rust backend to `logicaffeine_system::file`, so every tier answers alike. Inside a
//! `## Test` each one works on the sealed in-memory filesystem, exactly as `read`/`write` do.
//!
//! YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS. A divergence is a defect in an engine.
#![allow(dead_code)]
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted_eq};
use logicaffeine_compile::compile::compile_to_rust;

/// A fresh directory under the system temp dir, unique per test, as a LOGOS text literal body.
fn scratch(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("logos-file-ops-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.to_string_lossy().into_owned()
}

#[test]
fn append_adds_to_the_end_and_creates_an_absent_file() {
    let d = scratch("append");
    std::fs::create_dir_all(&d).unwrap();
    let src = format!(
        "## Main\n\
         Let a be appendFile(\"{d}/ledger.txt\", \"one\\n\").\n\
         Let b be appendFile(\"{d}/ledger.txt\", \"two\\n\").\n\
         Inspect read(\"{d}/ledger.txt\"):\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let w be write(\"{d}/ledger.txt\", \"\").\n"
    );
    assert_all_tiers(&src, "one\ntwo");
}

#[test]
fn append_into_a_missing_directory_is_an_err() {
    let d = scratch("append-missing");
    let src = format!(
        "## Main\n\
         Inspect appendFile(\"{d}/no/such/dir/x.txt\", \"x\"):\n\
         \x20   When Ok (u):\n\
         \x20       Show \"appended\".\n\
         \x20   When Err (e):\n\
         \x20       Show \"err\".\n"
    );
    assert_all_tiers(&src, "err");
}

#[test]
fn exists_answers_for_files_directories_and_nothing() {
    let d = scratch("exists");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(format!("{d}/present.txt"), "x").unwrap();
    let src = format!(
        "## Main\n\
         Show fileExists(\"{d}/present.txt\").\n\
         Show fileExists(\"{d}\").\n\
         Show fileExists(\"{d}/absent.txt\").\n"
    );
    assert_all_tiers(&src, "true\ntrue\nfalse");
}

#[test]
fn make_dirs_makes_every_missing_parent_and_is_idempotent() {
    let d = scratch("mkdirs");
    let src = format!(
        "## Main\n\
         Inspect makeDirs(\"{d}/a/b/c\"):\n\
         \x20   When Ok (u):\n\
         \x20       Show \"made\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Inspect makeDirs(\"{d}/a/b/c\"):\n\
         \x20   When Ok (u):\n\
         \x20       Show \"again\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Show fileExists(\"{d}/a/b/c\").\n"
    );
    assert_all_tiers(&src, "made\nagain\ntrue");
}

#[test]
fn make_dirs_over_a_file_is_an_err() {
    let d = scratch("mkdirs-file");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(format!("{d}/blocker"), "x").unwrap();
    let src = format!(
        "## Main\n\
         Inspect makeDirs(\"{d}/blocker\"):\n\
         \x20   When Ok (u):\n\
         \x20       Show \"made\".\n\
         \x20   When Err (e):\n\
         \x20       Show \"err\".\n"
    );
    assert_all_tiers(&src, "err");
}

#[test]
fn list_dir_answers_sorted_names_without_dot_entries() {
    let d = scratch("list");
    std::fs::create_dir_all(format!("{d}/sub")).unwrap();
    for f in ["zeta.txt", "alpha.txt", "Mid.txt"] {
        std::fs::write(format!("{d}/{f}"), "x").unwrap();
    }
    let src = format!(
        "## Main\n\
         Inspect listDir(\"{d}\"):\n\
         \x20   When Ok (names):\n\
         \x20       Repeat for n in names:\n\
         \x20           Show n.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n"
    );
    // Sorted by bytes: upper case before lower case.
    assert_all_tiers(&src, "Mid.txt\nalpha.txt\nsub\nzeta.txt");
}

#[test]
fn list_dir_of_a_missing_directory_is_an_err() {
    let d = scratch("list-missing");
    let src = format!(
        "## Main\n\
         Inspect listDir(\"{d}/nowhere\"):\n\
         \x20   When Ok (names):\n\
         \x20       Show \"listed\".\n\
         \x20   When Err (e):\n\
         \x20       Show \"err\".\n"
    );
    assert_all_tiers(&src, "err");
}

#[test]
fn rename_replaces_the_target_and_removes_the_source() {
    let d = scratch("rename");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(format!("{d}/old.txt"), "stale").unwrap();
    let src = format!(
        "## Main\n\
         Let w be write(\"{d}/new.tmp\", \"fresh\").\n\
         Inspect renameFile(\"{d}/new.tmp\", \"{d}/old.txt\"):\n\
         \x20   When Ok (u):\n\
         \x20       Show \"renamed\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Show fileExists(\"{d}/new.tmp\").\n\
         Inspect read(\"{d}/old.txt\"):\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let r be write(\"{d}/old.txt\", \"stale\").\n"
    );
    assert_all_tiers(&src, "renamed\nfalse\nfresh");
}

#[test]
fn rename_of_a_missing_source_is_an_err() {
    let d = scratch("rename-missing");
    let src = format!(
        "## Main\n\
         Inspect renameFile(\"{d}/absent\", \"{d}/elsewhere\"):\n\
         \x20   When Ok (u):\n\
         \x20       Show \"renamed\".\n\
         \x20   When Err (e):\n\
         \x20       Show \"err\".\n"
    );
    assert_all_tiers(&src, "err");
}

#[test]
fn a_program_that_defines_list_dir_keeps_its_own() {
    assert_all_tiers(
        "## To listDir (path: Text) -> Text:\n    Return \"mine: \" + path.\n\n## Main\nShow listDir(\"/x\").\n",
        "mine: /x",
    );
}

#[test]
fn the_compiled_binary_agrees_with_the_interpreters() {
    let d = scratch("aot");
    let src = format!(
        "## Main\n\
         Inspect makeDirs(\"{d}/w\"):\n\
         \x20   When Ok (u):\n\
         \x20       Show \"made\".\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let a be appendFile(\"{d}/w/log\", \"1\").\n\
         Let b be appendFile(\"{d}/w/log\", \"2\").\n\
         Let c be write(\"{d}/w/tmp\", \"t\").\n\
         Let r be renameFile(\"{d}/w/tmp\", \"{d}/w/final\").\n\
         Show fileExists(\"{d}/w/tmp\").\n\
         Inspect listDir(\"{d}/w\"):\n\
         \x20   When Ok (names):\n\
         \x20       Repeat for n in names:\n\
         \x20           Show n.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Inspect read(\"{d}/w/log\"):\n\
         \x20   When Ok (v):\n\
         \x20       Show v.\n\
         \x20   When Err (e):\n\
         \x20       Show e.\n\
         Let z be write(\"{d}/w/log\", \"\").\n"
    );
    // The interpreter runs first and leaves `log` emptied and `final` in place, so the compiled run
    // starts from the same state and must print the same lines.
    let _ = std::fs::remove_dir_all(&d);
    assert_compiled_equals_interpreted_eq(&src, "made\nfalse\nfinal\nlog\n12");
}

#[test]
fn the_rust_backend_links_each_native_to_the_file_module() {
    let src = "## Main\nShow fileExists(\"/\").\nLet a be appendFile(\"/tmp/x\", \"y\").\nLet l be listDir(\"/\").\nLet m be makeDirs(\"/tmp\").\nLet r be renameFile(\"/tmp/a\", \"/tmp/b\").\n";
    let rust = compile_to_rust(src).expect("compiles to Rust");
    for f in ["file::exists(", "file::append(", "file::list_dir(", "file::make_dirs(", "file::rename("] {
        assert!(rust.contains(&format!("logicaffeine_system::{f}")), "{f} is linked:\n{rust}");
    }
}

#[test]
fn the_sealed_filesystem_supports_every_operation() {
    let src = "## Test: the sealed filesystem appends, lists, makes and renames\n\
        \x20   Let m be makeDirs(\"/work/cache\").\n\
        \x20   Expect that fileExists(\"/work/cache\") is equal to true.\n\
        \x20   Expect that fileExists(\"/work/nothing\") is equal to false.\n\
        \x20   Let a be appendFile(\"/work/log.txt\", \"a\").\n\
        \x20   Let b be appendFile(\"/work/log.txt\", \"b\").\n\
        \x20   Inspect read(\"/work/log.txt\"):\n\
        \x20       When Ok (v):\n\
        \x20           Expect that v is equal to \"ab\".\n\
        \x20       When Err (e):\n\
        \x20           Expect that e is equal to \"the appended file\".\n\
        \x20   Let w be write(\"/work/tmp\", \"t\").\n\
        \x20   Let r be renameFile(\"/work/tmp\", \"/work/final\").\n\
        \x20   Expect that fileExists(\"/work/tmp\") is equal to false.\n\
        \x20   Inspect listDir(\"/work\"):\n\
        \x20       When Ok (names):\n\
        \x20           Expect that length of names is equal to 3.\n\
        \x20           Expect that item 1 of names is equal to \"cache\".\n\
        \x20           Expect that item 2 of names is equal to \"final\".\n\
        \x20           Expect that item 3 of names is equal to \"log.txt\".\n\
        \x20       When Err (e):\n\
        \x20           Expect that e is equal to \"a listing\".\n";
    let report = logicaffeine_compile::testrun::run_tests(src, &Default::default()).expect("the tests run");
    assert!(report.all_passed(), "{}", logicaffeine_compile::testrun::render_report(&report));
}
