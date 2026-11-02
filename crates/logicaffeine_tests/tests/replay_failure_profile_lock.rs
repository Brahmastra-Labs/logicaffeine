//! A failed replay task reports failure after the other tasks finish, in a
//! generated release project as well as in the debug replay corpus.
#![cfg(not(target_arch = "wasm32"))]
use logicaffeine_compile::compile::{
    compile_program_full, compile_program_full_deterministic, write_cargo_project,
};
use std::process::Command;

const FAILING_TASK: &str = "## To bad (n: Int):\n    Show \"bad starts\".\n    Show 10 / n.\n\n## Main\nLaunch a task to bad with 0.\nShow \"main goes on\".\n";

#[test]
fn ordinary_generated_programs_keep_the_abort_profile() {
    let output = compile_program_full("## Main\nShow 1.\n").expect("ordinary program compiles");
    let project = tempfile::tempdir().unwrap();
    write_cargo_project(&output.rust_code, &output.dependencies, project.path())
        .expect("ordinary project");
    let manifest = std::fs::read_to_string(project.path().join("Cargo.toml")).unwrap();
    assert!(
        manifest.contains("panic = \"abort\""),
        "ordinary failure handling keeps its established release profile"
    );
}

#[test]
fn a_replay_task_failure_finishes_the_other_tasks_in_a_generated_release_project() {
    let output = compile_program_full_deterministic(FAILING_TASK).expect("replay program compiles");
    let project = tempfile::tempdir().unwrap();
    write_cargo_project(&output.rust_code, &output.dependencies, project.path())
        .expect("replay project");
    let manifest = std::fs::read_to_string(project.path().join("Cargo.toml")).unwrap();
    assert!(
        manifest.contains("panic = \"unwind\""),
        "the replay scheduler catches task failures, so the generated release profile must unwind"
    );
    let target = std::env::temp_dir().join("logos_replay_failure_release_target");
    let built = Command::new("cargo")
        .args(["build", "--release", "--offline", "--quiet", "-j", "6"])
        .current_dir(project.path())
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("build generated release program");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let binary = target.join("release/logos_output");
    for seed in [0, 1, 2, 3, 7, 42] {
        let ran = Command::new(&binary)
            .env("LOGOS_SEED", seed.to_string())
            .output()
            .expect("run replay release program");
        assert_eq!(
            ran.status.code(),
            Some(1),
            "task failure must fail the completed run at seed {seed}"
        );
        assert_eq!(
            String::from_utf8(ran.stdout).unwrap(),
            "bad starts\nmain goes on\n",
            "seed {seed}"
        );
        assert_eq!(
            String::from_utf8(ran.stderr).unwrap(),
            "Division by zero\n",
            "one exact failure report at seed {seed}"
        );
    }
}

#[test]
fn ordinary_program_text_cannot_select_the_replay_panic_profile() {
    let source = "## Main\nShow \"logicaffeine_system::replay::run(\".\n";
    let output = compile_program_full(source).expect("ordinary text program compiles");
    let project = tempfile::tempdir().unwrap();
    write_cargo_project(&output.rust_code, &output.dependencies, project.path())
        .expect("ordinary text project");
    let manifest = std::fs::read_to_string(project.path().join("Cargo.toml")).unwrap();
    assert!(
        manifest.contains("panic = \"abort\""),
        "a string value is not a replay runtime requirement:\n{manifest}\n{}",
        output.rust_code
    );
}

#[test]
fn a_deadlock_reports_its_error_after_buffered_output() {
    let source = "## Main\nLet ch be a Pipe of Int.\nShow \"before\".\nReceive x from ch.\nShow \"never\".\n";
    let output = compile_program_full_deterministic(source).expect("deadlock program compiles");
    let project = tempfile::tempdir().unwrap();
    write_cargo_project(&output.rust_code, &output.dependencies, project.path())
        .expect("deadlock project");
    // The other release test builds concurrently: each program owns its binary.
    let target = std::env::temp_dir().join("logos_replay_deadlock_release_target");
    let built = Command::new("cargo")
        .args(["build", "--release", "--offline", "--quiet", "-j", "6"])
        .current_dir(project.path())
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("build generated deadlock program");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    for seed in [0, 1, 2, 3, 7, 42] {
        let combined_path = project.path().join(format!("combined-{seed}.txt"));
        let combined = std::fs::File::create(&combined_path).unwrap();
        let ran = Command::new(target.join("release/logos_output"))
            .env("LOGOS_SEED", seed.to_string())
            .stdout(std::process::Stdio::from(combined.try_clone().unwrap()))
            .stderr(std::process::Stdio::from(combined))
            .status()
            .expect("run deadlocked replay program");
        assert_eq!(ran.code(), Some(1), "deadlock must fail at seed {seed}");
        assert_eq!(
            std::fs::read_to_string(combined_path).unwrap(),
            "before\ndeadlock: every task is blocked waiting\n",
            "stdout must precede the terminal deadlock report at seed {seed}"
        );
    }
}
