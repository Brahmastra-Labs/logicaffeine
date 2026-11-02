//! ★ LOCK — `largo verify` verifies the program. ★
//!
//! It did not. It read the entry file, threw the source away, asked Z3 whether `true` is valid,
//! and reported success — so a program with a provable violation passed, for a paying customer,
//! with a straight face. These are unit tests rather than a `tests/` file because the command
//! itself needs the full build with Z3; the command's plumbing is not the part
//! that was broken, so the verification work is tested with the gate lifted off it.
use std::fs;
use std::path::PathBuf;

/// A project on disk with one entry file, assembled the way a real one is.
fn project(dir: &tempfile::TempDir, entry_src: &str) -> PathBuf {
    let root = dir.path().to_path_buf();
    fs::create_dir_all(root.join("src")).expect("src/");
    fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"verify_fixture\"\nversion = \"0.1.0\"\nauthors = []\nentry = \"src/main.lg\"\n\n[dependencies]\n",
    )
    .expect("Largo.toml");
    fs::write(root.join("src/main.lg"), entry_src).expect("entry");
    root
}

/// A loop declared `decreasing x` whose body INCREASES `x` does not terminate, and the Z3 pass
/// proves it. This is the case the old command reported as passing.
const CANNOT_TERMINATE: &str =
    "## Main\nLet x be 10.\nWhile x > 0 (decreasing x):\n    Set x to x + 1.\n";

/// The same loop, decreasing as declared.
const TERMINATES: &str =
    "## Main\nLet x be 10.\nWhile x > 0 (decreasing x):\n    Set x to x - 1.\n";

#[test]
fn a_program_that_cannot_be_verified_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = project(&dir, CANNOT_TERMINATE);
    let result = super::verify_project_source(&root);
    assert!(
        result.is_err(),
        "a loop that provably does not terminate must not pass verification — this is the \
             exact program the command used to call verified"
    );
}

#[test]
fn a_program_that_verifies_is_accepted() {
    // The control, and it is load-bearing: a command that rejected EVERYTHING would pass the
    // test above while being just as useless.
    let dir = tempfile::tempdir().expect("tempdir");
    let root = project(&dir, TERMINATES);
    let result = super::verify_project_source(&root);
    assert!(
        result.is_ok(),
        "a terminating loop must verify: {:?}",
        result.err()
    );
}

#[test]
fn a_refusal_embedded_in_the_generated_rust_is_not_silence() {
    // ★ `Ok` PROVES NOTHING: a refused program returns `Ok` carrying a `compile_error!`. If
    // this scanner missed it, `verify` would go back to reporting success on a program that
    // cannot even be built.
    let refused = "fn main() {}\ncompile_error!(\"Mount App cannot run on this tier\");\n";
    assert_eq!(
        super::embedded_refusal(refused).as_deref(),
        Some("Mount App cannot run on this tier"),
        "the refusal's own sentence must be recovered, not just its presence"
    );
    assert_eq!(
        super::embedded_refusal("fn main() { println!(\"ok\"); }"),
        None,
        "ordinary generated code carries no refusal"
    );
}
