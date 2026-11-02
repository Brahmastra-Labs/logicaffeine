//! demo_bringup_run_lock — `largo bringup --run` runs a checkpoint's cut on every chip on the box and says how each
//! did.
//!
//! DEMO-BRINGUP-RUN's RED tests (benchmarks/FUSIONS_WE_NEED.md, section 12.20; ONE-PROGRAM-EVERY-BACKEND). Bring-up
//! knows every backend the book names (section 3.3). With `--run --tokens N`, each backend that runs on this box
//! prints a line `ran:` with the tokens it generated, its tokens a second, and how many of them agree with the
//! reference; a backend that cannot run here says so by name.

#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "support/under_test.rs"]
mod under_test;

use std::collections::BTreeSet;
use std::process::Command;

use logicaffeine_compile::bringup::backends;
use tiny_models::Tiny;

#[test]
fn every_backend_in_the_book_is_a_bringup_backend() {
    let names: BTreeSet<&str> = backends().iter().map(|b| b.name()).collect();
    for want in ["vm", "jit", "c", "rust", "riscv-qemu", "wasm", "cuda", "d5005", "sn50", "fabric"] {
        assert!(names.contains(want), "bring-up has no `{want}` backend: {names:?}");
    }
}

#[test]
fn a_run_reports_tokens_speed_and_agreement_per_chip() {
    let dir = scratch::dir("logos_bringup_run");
    let spec = Tiny::Llama.spec(24);
    let ckpt = dir.join("llama.gguf");
    tiny_models::write_checkpoint(&spec, &ckpt);
    let out = Command::new(under_test::largo())
        .args(["bringup"])
        .arg(&ckpt)
        .args(["--target", "vm", "--ctx", "24", "--run", "--tokens", "2", "-o"])
        .arg(dir.join("out"))
        .output()
        .expect("run largo");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "largo bringup --run failed:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let ran = stdout.lines().find(|l| l.trim_start().starts_with("ran:")).unwrap_or_else(|| panic!("no `ran:` line:\n{stdout}"));
    assert!(ran.contains("tok/s") && ran.contains("agree"), "a run line names its speed and agreement: {ran}");
}

/// With `--golden` and `--prompts all`, a run generates every prompt of the golden and is judged against each one's own
/// continuation: the chip's `ran:` line counts the sequences and the tokens that agree over all of them, and a
/// `sequence` line a prompt gives its tokens and its agreement.
#[test]
fn a_run_over_every_prompt_of_a_golden_reports_each_sequence() {
    use logicaffeine_compile::bringup::Reference;
    let dir = scratch::dir("logos_bringup_run_prompts");
    let spec = Tiny::Llama.spec(24);
    let ckpt = dir.join("llama.gguf");
    tiny_models::write_checkpoint(&spec, &ckpt);
    let prompts: [&[u32]; 2] = [&[5, 17, 2, 30], &[9, 4]];
    let entries: Vec<String> = prompts
        .iter()
        .map(|p| {
            let r = Reference::of(&spec, &ckpt, p, 3).unwrap();
            format!(r#"{{"tokens": {:?}, "continuation": {:?}}}"#, p, r.tokens)
        })
        .collect();
    let golden = dir.join("golden-f32.json");
    std::fs::write(&golden, format!(r#"{{"prompts": [{}]}}"#, entries.join(", "))).unwrap();
    let out = Command::new(under_test::largo())
        .args(["bringup"])
        .arg(&ckpt)
        .args(["--target", "vm", "--ctx", "24", "--run", "--prompts", "all", "--golden"])
        .arg(&golden)
        .arg("-o")
        .arg(dir.join("out"))
        .output()
        .expect("run largo");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "largo bringup --run --prompts all failed:\n{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
    let ran = stdout.lines().find(|l| l.trim_start().starts_with("ran:")).unwrap_or_else(|| panic!("no `ran:` line:\n{stdout}"));
    assert!(ran.contains("2 sequences") && ran.contains("6 of 6 agree"), "the run line counts every sequence: {ran}");
    let each: Vec<&str> = stdout.lines().filter(|l| l.trim_start().starts_with("sequence ")).collect();
    assert_eq!(each.len(), 2, "a line a sequence:\n{stdout}");
    assert!(each.iter().all(|l| l.contains("3 of 3 agree")), "{each:?}");
}
