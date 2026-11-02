//! ★ LOCK — a build runs the program it built, whatever else shares its cargo target directory. ★
//!
//! `largo build` generates a cargo project and hands it to cargo. With `CARGO_TARGET_DIR` set —
//! which is how this repository's own suite runs, and how most CI caches builds — every generated
//! project writes into ONE directory, and cargo names a binary after its package. Two projects
//! both called `app` (the name half the fixtures in this crate use) therefore produced ONE
//! `<target>/debug/app`, each build overwriting the other's, and `largo build` reported that shared
//! path as its artifact. `largo run` executed it. So a project could print ANOTHER project's
//! answer — the suite caught two unrelated tests both printing `49` — which is the worst thing a
//! build tool can do: a wrong answer, delivered with a success code.
//!
//! The contract locked here: a build's artifact is its own. Its cargo binary carries an identity
//! unique to the project, the artifact is published at the project's stable
//! `target/<mode>/<name>`, and that — the file this build wrote — is what is reported and run.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo, stderr, stdout};

use std::path::{Path, PathBuf};
use std::process::Output;

/// The cargo target directory every build in this file shares: the suite's own when it has one
/// (its dependency builds are already warm), else one kept beside this crate's test scratch.
fn shared_target() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_TARGET_TMPDIR")).join("largo-shared-target"))
}

/// A project named `app` whose program shows `answer`.
fn app(dir: &Path, answer: i64) -> PathBuf {
    let root = dir.join(format!("app_{answer}"));
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/main.lg"),
        format!("# App\n\n## Main\n\nShow {answer}.\n"),
    )
    .unwrap();
    root
}

fn largo_shared(root: &Path, args: &[&str]) -> Output {
    largo()
        .args(args)
        .current_dir(root)
        .env("CARGO_TARGET_DIR", shared_target())
        .output()
        .expect("largo runs")
}

/// The artifact path `largo build` reported: its `Built <path> [debug]` line.
fn reported_artifact(out: &Output) -> PathBuf {
    let text = stdout(out);
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix("Built "))
        .unwrap_or_else(|| {
            panic!(
                "`largo build` reports what it built:\n{text}\n{}",
                stderr(out)
            )
        });
    PathBuf::from(line.rsplit_once(" [").map(|(p, _)| p).unwrap_or(line))
}

fn run(binary: &Path) -> String {
    let out = std::process::Command::new(binary)
        .output()
        .expect("the artifact runs");
    assert!(
        out.status.success(),
        "{} failed: {}",
        binary.display(),
        stderr(&out)
    );
    stdout(&out).trim().to_string()
}

fn cargo_is_here() -> bool {
    common::require_tool("cargo") && common::require_tool("rustc")
}

/// ★ THE LOAD-BEARING TEST, and it is deterministic. Two projects with one name build, one after
/// the other, into one target directory. The first one's artifact must still be the first one's
/// program.
#[test]
fn a_projects_artifact_is_its_own_after_a_same_named_project_builds() {
    if !cargo_is_here() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let (first, second) = (app(dir.path(), 42), app(dir.path(), 49));

    let built_first = largo_shared(&first, &["build"]);
    assert!(built_first.status.success(), "{}", stderr(&built_first));
    let built_second = largo_shared(&second, &["build"]);
    assert!(built_second.status.success(), "{}", stderr(&built_second));

    let first_artifact = reported_artifact(&built_first);
    let second_artifact = reported_artifact(&built_second);
    assert_ne!(
        first_artifact, second_artifact,
        "two projects must never be told the same file is their artifact"
    );
    assert_eq!(
        run(&first_artifact),
        "42",
        "the first project's reported artifact ({}) runs a DIFFERENT program — the second build \
         overwrote it",
        first_artifact.display()
    );
    assert_eq!(run(&second_artifact), "49");
    // And the stable, deployable path inside each project holds that project's program.
    assert_eq!(run(&first.join("target/debug/app")), "42");
    assert_eq!(run(&second.join("target/debug/app")), "49");
    assert!(
        first_artifact.starts_with(&first),
        "the reported artifact lives in the project that built it, where nothing else writes: {}",
        first_artifact.display()
    );
}

/// The same property under real concurrency: `largo run` in two same-named projects at once,
/// round after round, each printing its own answer.
#[test]
fn concurrent_runs_of_same_named_projects_each_print_their_own_answer() {
    if !cargo_is_here() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let projects = [app(dir.path(), 42), app(dir.path(), 20)];
    for round in 0..4 {
        let handles: Vec<_> = projects
            .iter()
            .cloned()
            .map(|root| std::thread::spawn(move || (largo_shared(&root, &["run"]), root)))
            .collect();
        for handle in handles {
            let (out, root) = handle.join().unwrap();
            let want = if root.ends_with("app_42") { "42" } else { "20" };
            assert!(out.status.success(), "round {round}: {}", stderr(&out));
            assert_eq!(
                stdout(&out).trim(),
                want,
                "round {round}: {} printed another project's answer",
                root.display()
            );
        }
    }
}
