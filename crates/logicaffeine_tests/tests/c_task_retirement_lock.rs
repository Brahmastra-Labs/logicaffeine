//! Completed task handles retain their identity, but not their execution storage.
//! A parked task must retain its frame and payload until it resumes.
#![cfg(all(target_os = "linux", not(target_arch = "wasm32")))]
mod common;
use std::process::Command;

fn check(tier: &str, prefix: &str, runtime: &str, fixture: &str, modes: &[Option<u32>]) {
    assert!(common::require_tool("gcc"), "task retirement requires GCC");
    assert!(
        common::require_tool("clang"),
        "task retirement requires AddressSanitizer"
    );
    let carrier = logicaffeine_compile::compile::compile_to_c_via_bytecode("## Main\nShow 1.\n")
        .expect("compile collector carrier");
    let marker = "/* LC_GC_COLLECTOR_END */";
    let end = carrier.find(marker).expect("collector emitted") + marker.len();
    let source = format!(
        "{}\n{prefix}\n{}\n{runtime}\n{}\n{fixture}",
        &carrier[..end],
        include_str!("../../logicaffeine_compile/src/codegen_c/sched_core.c"),
        include_str!("support/c_task_retirement_trace.c")
    );
    let dir = std::env::temp_dir().join(format!(
        "logos_task_retirement_{}_{tier}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create task retirement scratch");
    std::fs::write(dir.join("probe.c"), source).expect("write probe");
    for (compiler, flags) in [
        ("gcc", vec!["-O2", "-g", "-pthread"]),
        (
            "clang",
            vec![
                "-O1",
                "-g",
                "-pthread",
                "-fsanitize=address",
                "-fno-omit-frame-pointer",
            ],
        ),
    ] {
        let binary = dir.join(compiler);
        let built = Command::new(compiler)
            .args(flags)
            .arg(dir.join("probe.c"))
            .arg("-o")
            .arg(&binary)
            .output()
            .expect("compile task retirement probe");
        assert!(
            built.status.success(),
            "{tier}/{compiler}: {}",
            String::from_utf8_lossy(&built.stderr)
        );
        for mode in modes {
            let mut cmd = Command::new(&binary);
            if let Some(mode) = mode {
                cmd.arg(mode.to_string());
            }
            // The collector is explicitly swept from the held handle below.
            // AddressSanitizer checks accesses; the exact live census checks retention.
            let out = cmd
                .env("ASAN_OPTIONS", "detect_leaks=0:halt_on_error=1")
                .output()
                .expect("run task retirement probe");
            assert!(out.status.success(), "{tier}/{compiler}/{mode:?}: {out:?}");
        }
    }
    std::fs::remove_dir_all(dir).expect("remove task retirement scratch");
}

#[test]
fn finished_or_stopped_stackless_tasks_release_frames_but_parked_tasks_keep_them() {
    check(
        "stackless",
        include_str!("support/c_task_retirement_stackless_prefix.c"),
        include_str!("../../logicaffeine_compile/src/codegen_c/stackless_tier.c"),
        include_str!("support/c_task_retirement_stackless_fixture.c"),
        &[Some(0), Some(1), Some(2)],
    );
}

#[test]
fn a_finished_coroutine_handle_does_not_retain_its_stack_or_arguments() {
    check(
        "ucontext",
        include_str!("support/c_task_retirement_ucontext_prefix.c"),
        include_str!("../../logicaffeine_compile/src/codegen_c/ucontext_tier.c"),
        include_str!("support/c_task_retirement_ucontext_fixture.c"),
        &[None],
    );
}

#[test]
fn woken_and_finished_tasks_release_wait_only_references() {
    check(
        "wait_fields",
        include_str!("support/c_task_lifetime_core_prefix.c"),
        "",
        include_str!("support/c_task_lifetime_fields.c"),
        &[Some(0), Some(1), Some(2), Some(3), Some(4), Some(5)],
    );
}

#[test]
fn task_cleanup_waits_until_its_step_function_returns() {
    check(
        "retirement_timing",
        include_str!("support/c_task_retirement_stackless_prefix.c"),
        include_str!("../../logicaffeine_compile/src/codegen_c/stackless_tier.c"),
        include_str!("support/c_task_retirement_timing.c"),
        &[None],
    );
}

#[test]
fn completed_tasks_release_waiter_storage_but_live_tasks_keep_waiters() {
    check(
        "awaiters",
        include_str!("support/c_task_lifetime_core_prefix.c"),
        "",
        include_str!("support/c_task_lifetime_awaiters.c"),
        &[Some(0), Some(1)],
    );
}
