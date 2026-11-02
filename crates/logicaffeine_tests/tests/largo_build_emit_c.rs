//! `largo build [--release] --emit c|c-bytecode` and `largo run [--release] --emit c|c-bytecode`.
//!
//! The C build is the Rust build with another backend: the same debug and release profiles, the artifact
//! published at `target/<mode>/<name>`, the generated source kept in the build directory beside it
//! (`build-c/main.c`, where the Rust build keeps `build/src/main.rs`), and a program that prints the same
//! bytes as every other tier. `--emit c` lowers the optimized AST; `--emit c-bytecode` lowers the VM's
//! bytecode. The oracle is the product itself: `largo run --interpret` (the VM) and the Rust release build,
//! compared on exact stdout bytes.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo, require_tool, scaffold, stderr, stdout};

use logicaffeine_cli::project::build_c::{C_DEBUG_FLAGS, C_RELEASE_FLAGS};
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const PROGRAM: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To square (n: Int) -> Int:
    Return n * n.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable total be 0.
Let mutable i be 1.
While i is at most n:
    Set total to total + square(i).
    Set i to i + 1.
Show total.
Let mutable words be a new Seq of Text.
Push \"alpha\" to words.
Push \"beta\" to words.
Show length of words.
Show \"sum of squares to \" + n + \" is \" + total.
";

fn project(name: &str) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    scaffold(dir.path(), name);
    std::fs::write(dir.path().join("src/main.lg"), PROGRAM).unwrap();
    dir
}

fn run_largo(dir: &Path, args: &[&str]) -> Output {
    largo()
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawn largo")
}

fn ok(out: &Output, what: &str) {
    assert!(
        out.status.success(),
        "`{what}` failed:\nstdout: {}\nstderr: {}",
        stdout(out),
        stderr(out)
    );
}

fn run_bin(bin: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new(bin)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", bin.display()));
    assert!(
        out.status.success(),
        "{} exited {:?}:\n{}",
        bin.display(),
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// The VM's exact stdout for the same program and arguments — the product oracle every tier matches.
fn vm_bytes(dir: &Path, args: &[&str]) -> Vec<u8> {
    let mut argv = vec!["-q", "run", "--interpret", "--"];
    argv.extend_from_slice(args);
    let out = run_largo(dir, &argv);
    ok(&out, "largo run --interpret");
    out.stdout
}

fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn debug_and_release_c_builds_publish_beside_each_other_like_the_rust_build() {
    if !require_tool("cc") {
        return;
    }
    let dir = project("squares");
    ok(
        &run_largo(dir.path(), &["build", "--emit", "c"]),
        "largo build --emit c",
    );
    ok(
        &run_largo(dir.path(), &["build", "--release", "--emit", "c"]),
        "largo build --release --emit c",
    );
    for mode in ["debug", "release"] {
        let bin = dir.path().join("target").join(mode).join("squares");
        assert!(
            bin.is_file(),
            "the {mode} C build must publish its artifact at {}",
            bin.display()
        );
        let source = dir
            .path()
            .join("target")
            .join(mode)
            .join("build-c")
            .join("main.c");
        assert!(
            source.is_file(),
            "the {mode} C build must keep its generated source at {}",
            source.display()
        );
        let text = std::fs::read_to_string(&source).unwrap();
        assert!(
            text.contains("int main("),
            "{} must be the generated C program:\n{}",
            source.display(),
            &text[..text.len().min(400)]
        );
    }
}

#[test]
fn the_c_artifact_prints_the_vm_bytes_and_the_rust_bytes() {
    if !require_tool("cc") {
        return;
    }
    let dir = project("squares");
    let expected = vm_bytes(dir.path(), &["12"]);
    assert!(!expected.is_empty(), "the oracle printed nothing");

    ok(
        &run_largo(dir.path(), &["build", "--release", "--emit", "c"]),
        "largo build --release --emit c",
    );
    let c = run_bin(&dir.path().join("target/release/squares"), &["12"]);
    assert_eq!(
        show(&c),
        show(&expected),
        "LOGOS (C) must print exactly what the VM prints"
    );

    ok(
        &run_largo(dir.path(), &["build", "--release"]),
        "largo build --release",
    );
    let rust = run_bin(&dir.path().join("target/release/squares"), &["12"]);
    assert_eq!(
        show(&c),
        show(&rust),
        "LOGOS (C) must print exactly what LOGOS (Rust) prints"
    );
}

#[test]
fn largo_run_emit_c_builds_then_runs_with_program_arguments() {
    if !require_tool("cc") {
        return;
    }
    let dir = project("squares");
    let expected = vm_bytes(dir.path(), &["30"]);
    for extra in [&[][..], &["--release"][..]] {
        let mut argv = vec!["-q", "run"];
        argv.extend_from_slice(extra);
        argv.extend_from_slice(&["--emit", "c", "--", "30"]);
        let out = run_largo(dir.path(), &argv);
        ok(&out, &argv.join(" "));
        assert_eq!(
            show(&out.stdout),
            show(&expected),
            "`largo {}` must print the VM's bytes",
            argv.join(" ")
        );
    }
}

#[test]
fn the_bytecode_lowering_builds_runs_and_matches_the_vm() {
    if !require_tool("cc") {
        return;
    }
    let dir = project("squares");
    let expected = vm_bytes(dir.path(), &["9"]);
    ok(
        &run_largo(dir.path(), &["build", "--release", "--emit", "c-bytecode"]),
        "largo build --release --emit c-bytecode",
    );
    let bin = dir.path().join("target/release/squares");
    assert_eq!(
        show(&run_bin(&bin, &["9"])),
        show(&expected),
        "LOGOS (C, bytecode lowering) must print the VM's bytes"
    );

    let out = run_largo(
        dir.path(),
        &["-q", "run", "--release", "--emit", "c-bytecode", "--", "9"],
    );
    ok(&out, "largo run --release --emit c-bytecode");
    assert_eq!(show(&out.stdout), show(&expected));
}

/// The compiler invocation is visible under `-v` (as `cargo --verbose` shows rustc's), and it carries the
/// shared flag sets: release is the C baseline's `-O3 -march=native -flto -ffp-contract=off` (strict floating
/// point, as every compiled row in the suite takes), debug is `-O0 -g`.
#[test]
fn verbose_build_shows_the_exact_c_compiler_command() {
    if !require_tool("cc") {
        return;
    }
    let dir = project("squares");
    for (extra, flags) in [
        (&[][..], C_DEBUG_FLAGS),
        (&["--release"][..], C_RELEASE_FLAGS),
    ] {
        let mut argv = vec!["-v", "build"];
        argv.extend_from_slice(extra);
        argv.extend_from_slice(&["--emit", "c"]);
        let out = run_largo(dir.path(), &argv);
        ok(&out, &argv.join(" "));
        let err = common::strip_ansi(&stderr(&out));
        let line = err
            .lines()
            .find(|l| l.contains("Running") && l.contains("main.c"))
            .unwrap_or_else(|| {
                panic!(
                    "`largo {}` must print the C compiler command:\n{err}",
                    argv.join(" ")
                )
            });
        let joined = flags.join(" ");
        assert!(
            line.contains(&joined),
            "the compiler command must carry `{joined}`:\n{line}"
        );
        assert!(
            line.contains("-lm"),
            "the compiler command must link libm:\n{line}"
        );
    }
    assert_eq!(
        C_RELEASE_FLAGS,
        ["-O3", "-march=native", "-flto", "-ffp-contract=off"],
        "release C matches the C baseline's flags"
    );
    assert_eq!(C_DEBUG_FLAGS, ["-O0", "-g"]);
}

#[test]
fn cc_names_the_compiler_and_a_missing_one_fails_loudly() {
    let dir = project("squares");
    let out = largo()
        .current_dir(dir.path())
        .env("CC", "/nonexistent/logos-cc")
        .args(["build", "--emit", "c"])
        .output()
        .expect("spawn largo");
    assert!(
        !out.status.success(),
        "a missing C compiler must fail the build"
    );
    let err = stderr(&out);
    assert!(
        err.contains("/nonexistent/logos-cc"),
        "the error must name the compiler it could not run:\n{err}"
    );
    assert!(
        err.contains("C compiler"),
        "the error must say a C compiler is required:\n{err}"
    );

    if require_tool("clang") {
        let out = largo()
            .current_dir(dir.path())
            .env("CC", "clang")
            .args(["-v", "build", "--emit", "c"])
            .output()
            .expect("spawn largo");
        ok(&out, "CC=clang largo build --emit c");
        let err = common::strip_ansi(&stderr(&out));
        assert!(
            err.lines()
                .any(|l| l.contains("Running") && l.contains("`clang ")),
            "CC=clang must be the compiler run:\n{err}"
        );
    }
}

#[test]
fn the_unknown_emit_error_lists_the_c_backends() {
    let dir = project("squares");
    for sub in ["build", "run"] {
        let out = run_largo(dir.path(), &[sub, "--emit", "elf"]);
        assert!(!out.status.success(), "`largo {sub} --emit elf` must fail");
        let err = stderr(&out);
        assert!(
            err.contains("'c'") && err.contains("'c-bytecode'"),
            "`largo {sub}` must list the C backends:\n{err}"
        );
    }
}

/// A program the exact extension reaches: an infinitesimal, an algebraic number, an exact parameter and a
/// residue-dependent answer.
const EXACT: &str = "\
## To half (x: ExactReal) -> ExactReal:
    Return x / 2.

## Main
Let dt be an infinitesimal.
Let x: Algebraic be sqrt(2).
Show x * x.
Show half(pi).
Show 1 / dt.
Let n be 1 / dt.
Show n % 2.
";

fn exact_project(name: &str) -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    scaffold(dir.path(), name);
    std::fs::write(dir.path().join("src/main.lg"), EXACT).unwrap();
    dir
}

/// Both C lowerings of a program the exact extension reaches link the value runtime it asks for, and the artifact
/// prints the VM's bytes.
#[test]
fn an_exact_program_builds_on_both_c_lowerings_and_prints_the_vm_bytes() {
    if !require_tool("cc") {
        return;
    }
    let dir = exact_project("exact");
    let expected = vm_bytes(dir.path(), &[]);
    assert!(show(&expected).contains("1/dt"), "the oracle must run the extension:\n{}", show(&expected));
    for emit in ["c", "c-bytecode"] {
        ok(
            &run_largo(dir.path(), &["build", "--emit", emit]),
            &format!("largo build --emit {emit}"),
        );
        let bin = dir.path().join("target/debug/exact");
        assert_eq!(show(&run_bin(&bin, &[])), show(&expected), "--emit {emit}");
    }
}

/// `largo emit c -o` of a program the exact extension reaches writes the runtime it links beside the C and prints
/// the command that links them; that command builds a program that prints the VM's bytes. A program the extension
/// never reaches writes nothing beside its C.
#[test]
fn emitting_an_exact_program_writes_the_runtime_it_links_beside_it() {
    if !require_tool("cc") {
        return;
    }
    let expected = vm_bytes(exact_project("exact").path(), &[]);
    let dir = TempDir::new().expect("temp dir");
    std::fs::write(dir.path().join("prog.lg"), EXACT).unwrap();
    let out = run_largo(dir.path(), &["emit", "c", "prog.lg", "-o", "prog.c"]);
    ok(&out, "largo emit c prog.lg -o prog.c");
    assert!(
        dir.path().join("liblogos_value_runtime.a").is_file(),
        "the value runtime was not written beside the C"
    );
    let said = common::strip_ansi(&stdout(&out));
    let link = said
        .lines()
        .find_map(|l| l.split_once("Link with: ").map(|(_, cmd)| cmd.trim().to_string()))
        .unwrap_or_else(|| panic!("`largo emit c -o` must print the link command:\n{said}"));
    let mut words = link.split_whitespace();
    let cc = words.next().expect("a compiler");
    let built = Command::new(cc)
        .args(words)
        .args(["-o", "prog"])
        .current_dir(dir.path())
        .output()
        .expect("spawn the C compiler");
    assert!(
        built.status.success(),
        "the printed link command fails: {link}\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    assert_eq!(show(&run_bin(&dir.path().join("prog"), &[])), show(&expected));

    // On stdout, which carries only the C, the note that `-o` writes the runtime goes to stderr.
    let bare = run_largo(dir.path(), &["emit", "c", "prog.lg"]);
    ok(&bare, "largo emit c prog.lg");
    assert!(!stdout(&bare).contains("emit with -o"), "the note landed in the emitted C");
    assert!(
        common::strip_ansi(&stderr(&bare)).contains("emit with -o to write it beside the C"),
        "`largo emit c` on stdout must say where the runtime comes from:\n{}",
        stderr(&bare)
    );

    let plain = TempDir::new().expect("temp dir");
    std::fs::write(plain.path().join("prog.lg"), "## Main\nShow 7 / 2.\n").unwrap();
    ok(
        &run_largo(plain.path(), &["emit", "c", "prog.lg", "-o", "prog.c"]),
        "largo emit c of a plain program",
    );
    assert!(
        !plain.path().join("liblogos_value_runtime.a").exists(),
        "a program the extension never reaches has no runtime to write"
    );
}
