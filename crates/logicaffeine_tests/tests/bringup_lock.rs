//! Model bring-up (`logicaffeine_compile::bringup`): a checkpoint's one program, lowered by every chip that runs it.
//!
//! These locks hold the front door to the pitch it serves:
//!   * one checkpoint brought up on the CUDA lane, the D5005 engine and the SN50 names ONE program — the decode step
//!     each chip's artifact is a cut of carries the same identity on all three, and the program written beside them is
//!     that text;
//!   * each backend's artifacts are on disk and each names its cut;
//!   * a backend that does not run a model refuses it by name, each gap once with the layers it stands in, and the
//!     others still bring it up;
//!   * a checkpoint whose architecture is not described is refused in the spec reader's own words;
//!   * every backend the command line can name is in the registry, and an unknown name is refused before anything runs;
//!   * a run generates through the decode step's own statements — one generation program a chip, built once and run
//!     once: the weights read once, every sequence's state declared afresh, the step's statements in a loop over its
//!     positions, the prompt fed a token a step and every token after it the step's own — and the VM's tokens are the
//!     f64 reference's on every architecture the language writes; every CPU tier (the JIT, C, Rust, RISC-V,
//!     WebAssembly) lowers the decode step and generates the reference's tokens for every sequence it is given;
//!   * a chip that cannot run here says why by name, and a run is judged against the reference or a golden it names.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::Path;

use logicaffeine_compile::bringup::{
    backends, bring_up, bring_up_spec, run, run_sequences, Generated, Ran, Reference, Refused, Runs, Sequence, SequenceRuns,
};
use logicaffeine_compile::model::author;
use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::program::{written, Cut, Identity};
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use tiny_models::Tiny;

/// A checkpoint of `spec` written into `dir`.
fn checkpoint(spec: &ModelSpec, dir: &Path, name: &str) -> std::path::PathBuf {
    let path = dir.join(format!("{name}.gguf"));
    tiny_models::write_checkpoint(spec, &path);
    path
}

/// The Qwen3.8 hybrid at the tiny configuration's shape, widened so the D5005 engine's units take it.
fn hybrid() -> ModelSpec {
    let mut cfg = Qwen38Cfg::tiny();
    cfg.d = 64;
    cfg.dff = 128;
    cfg.vocab = 96;
    cfg.h = 4;
    cfg.kv = 2;
    cfg.hd = 16;
    cfg.rope_dim = 16;
    cfg.gdn_k_heads = 2;
    cfg.gdn_v_heads = 4;
    cfg.gdn_head_dim = 32;
    cfg.ctx_len = 64;
    cfg.spec()
}

#[test]
fn one_checkpoint_on_three_chips_is_one_program() {
    let dir = tempfile::tempdir().unwrap();
    let spec = hybrid();
    let ckpt = checkpoint(&spec, dir.path(), "hybrid");
    let out = dir.path().join("out");
    let ctx = 32;
    let chips_asked: Vec<String> = ["cuda", "d5005", "sn50"].map(String::from).to_vec();
    let b = bring_up_spec(spec.clone(), &ckpt, ctx, &out, &chips_asked).unwrap_or_else(|e| panic!("{e}"));
    let decode = written(&spec, Cut::Decode { ctx }).1;
    assert_eq!(b.decode, decode);
    assert_eq!(Identity::of(Cut::Decode { ctx }, &std::fs::read_to_string(out.join("program.lg")).unwrap()), decode);
    let chips: Vec<&str> = b.results.iter().map(|(c, _)| *c).collect();
    assert_eq!(chips, ["cuda", "d5005", "sn50"], "every chip asked for, in order");
    for (chip, result) in &b.results {
        let r = result.as_ref().unwrap_or_else(|e| panic!("{chip}: {e}"));
        assert_eq!(r.programs[0], decode, "{chip}: its decode step is the one program's");
        assert!(!r.files.is_empty() && r.files.iter().all(|f| f.is_file()), "{chip}: its artifacts are on disk");
    }
    let cu = std::fs::read_to_string(out.join("cuda/lane.cu")).unwrap();
    assert!(cu.contains(&format!("// {}\n", decode.line())), "the CUDA program names the decode step");
    assert_eq!(std::fs::read_to_string(out.join("d5005/program.txt")).unwrap(), format!("{}\n", decode.line()));
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out.join("sn50/manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["program"], decode.line(), "the SN50 bundle names the decode step");
    for chip in ["d5005", "sn50"] {
        assert_eq!(
            std::fs::read_to_string(out.join(chip).join("program.lg")).unwrap(),
            std::fs::read_to_string(out.join("program.lg")).unwrap(),
            "{chip}: the program it carries is the model's"
        );
    }
}

#[test]
fn a_chip_that_does_not_run_a_model_refuses_it_by_name_and_the_others_bring_it_up() {
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Qwen3Moe.spec(32);
    let ckpt = checkpoint(&spec, dir.path(), "moe");
    let b = bring_up_spec(spec.clone(), &ckpt, 32, &dir.path().join("out"), &[]).unwrap_or_else(|e| panic!("{e}"));
    let verdict = |chip: &str| b.results.iter().find(|(c, _)| *c == chip).map(|(_, r)| r.clone()).unwrap();
    assert!(verdict("cuda").is_ok(), "the lane compiles an expert mixture: {:?}", verdict("cuda").err());
    let d5005 = verdict("d5005").unwrap_err();
    assert!(d5005.0.contains("experts"), "the engine names what it has no unit for: {d5005}");
    let sn50 = verdict("sn50").unwrap_err();
    assert!(sn50.0.contains("Qwen3.5/3.8"), "the spatial lowering names what it is written for: {sn50}");
}

#[test]
fn the_targets_named_are_the_only_ones_run_and_an_unknown_one_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let spec = hybrid();
    let ckpt = checkpoint(&spec, dir.path(), "hybrid");
    let b = bring_up_spec(spec.clone(), &ckpt, 32, &dir.path().join("one"), &["d5005".to_string()]).unwrap();
    assert_eq!(b.results.iter().map(|(c, _)| *c).collect::<Vec<_>>(), ["d5005"]);
    assert!(!dir.path().join("one/cuda").exists(), "nothing written for a chip not asked for");
    let e = bring_up_spec(spec.clone(), &ckpt, 32, &dir.path().join("two"), &["tpu".to_string()]).err().expect("an unknown chip is refused");
    assert!(e.0.contains("tpu") && e.0.contains("cuda") && e.0.contains("sn50"), "{e}");
}

#[test]
fn every_chip_the_command_line_names_is_a_backend() {
    let names: Vec<&str> = backends().iter().map(|b| b.name()).collect();
    assert_eq!(names, ["vm", "jit", "c", "rust", "riscv-qemu", "wasm", "cuda", "d5005", "sn50", "fabric"]);
    let cli = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_cli/src/cli.rs")).unwrap();
    let doc = cli.lines().find(|l| l.contains("The chips to bring the model up on")).expect("the --target help");
    for n in &names {
        assert!(doc.contains(&format!("`{n}`")), "--target's help names {n}: {doc}");
    }
    let guide = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/cli.md")).unwrap();
    let section = guide.split("### `bringup`").nth(1).expect("the guide's bringup section").split("\n### ").next().unwrap();
    for n in &names {
        assert!(section.contains(&format!("| `{n}` |")), "the guide's table has a row for {n}");
    }
}

/// A refusal is the chip's gaps, each named once with the layers it stands in — never one sentence a layer, which for a
/// real checkpoint runs to hundreds.
#[test]
fn a_refusal_names_each_gap_once_with_the_layers_it_stands_in() {
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Qwen3Moe.spec(32);
    assert!(spec.layers.len() > 1, "the mixture has layers enough to repeat a gap");
    let ckpt = checkpoint(&spec, dir.path(), "moe");
    let b = bring_up_spec(spec.clone(), &ckpt, 32, &dir.path().join("out"), &["d5005".to_string()]).unwrap();
    let refusal = b.results[0].1.clone().unwrap_err().0;
    let gaps: Vec<&str> = refusal.split("; ").collect();
    let named: Vec<&str> = gaps.iter().map(|g| g.split(" (").next().unwrap()).collect();
    let mut distinct = named.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(distinct.len(), named.len(), "each gap is named once: {refusal}");
    let experts = gaps.iter().find(|g| g.contains("experts")).unwrap_or_else(|| panic!("the expert gap is named: {refusal}"));
    assert!(
        experts.ends_with("(every layer)") || experts.contains("(layer"),
        "the gap says which layers it stands in: {experts}"
    );
}

/// A checkpoint the spec reader cannot describe is refused in the reader's sentence, naming the checkpoint.
#[test]
fn an_undescribed_checkpoint_is_refused_in_the_spec_readers_words() {
    let dir = tempfile::tempdir().unwrap();
    let ckpt = checkpoint(&hybrid(), dir.path(), "bare");
    let why = ModelSpec::from_gguf(&Gguf::open(&ckpt).unwrap()).expect_err("a checkpoint without metadata describes no model");
    let e = bring_up(&ckpt, 32, &dir.path().join("out"), &[]).err().expect("the bring-up stops at the spec");
    assert_eq!(e.0, format!("{}: {why}", ckpt.display()));
}

/// The prompt every run here starts from: token ids from 0, each below the tiny models' vocabulary.
const PROMPT: &[u32] = &[5, 17, 2, 30];

/// What `chip` did in `runs`.
fn ran<'a>(runs: &'a Runs, chip: &str) -> &'a Result<Ran, Refused> {
    &runs.results.iter().find(|(c, _)| *c == chip).unwrap_or_else(|| panic!("{chip} was not run")).1
}

fn chips(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

/// The VM runs the one program's own cuts — the prompt as one prefill chunk, then a decode step a token, the caches,
/// memories and rings each leaves filled into the next — and generates the f64 reference's tokens.
fn the_vm_generates_the_references_tokens(arch: Tiny) {
    let dir = tempfile::tempdir().unwrap();
    let spec = arch.spec(24);
    author::check(&spec).unwrap_or_else(|e| panic!("{arch:?}: the language writes every tiny architecture: {e}"));
    let ckpt = checkpoint(&spec, dir.path(), arch.name());
    let out = dir.path().join("out");
    let b = bring_up_spec(spec.clone(), &ckpt, 24, &out, &chips(&["vm"])).unwrap_or_else(|e| panic!("{arch:?}: {e}"));
    let reference = Reference::of(&spec, &ckpt, PROMPT, 3).unwrap_or_else(|e| panic!("{arch:?}: {e}"));
    let runs = run(&b, &ckpt, 24, &out, PROMPT, reference.clone());
    let vm = ran(&runs, "vm").as_ref().unwrap_or_else(|e| panic!("{arch:?}: the VM's run: {e}"));
    assert_eq!(vm.tokens, reference.tokens, "{arch:?}: the VM generates the reference's tokens");
    assert!(vm.seconds > 0.0 && vm.tok_s() > 0.0, "{arch:?}: the run is timed");
}

#[test]
fn the_vm_runs_qwen3_to_the_references_tokens() {
    the_vm_generates_the_references_tokens(Tiny::Qwen3);
}

#[test]
fn the_vm_runs_llama_to_the_references_tokens() {
    the_vm_generates_the_references_tokens(Tiny::Llama);
}

#[test]
fn the_vm_runs_tied_llama_to_the_references_tokens() {
    the_vm_generates_the_references_tokens(Tiny::LlamaTied);
}

#[test]
fn the_vm_runs_qwen3_moe_to_the_references_tokens() {
    the_vm_generates_the_references_tokens(Tiny::Qwen3Moe);
}

#[test]
fn the_vm_runs_the_qwen35_hybrid_to_the_references_tokens() {
    the_vm_generates_the_references_tokens(Tiny::Qwen35);
}

#[test]
fn the_vm_runs_gemma3_to_the_references_tokens() {
    the_vm_generates_the_references_tokens(Tiny::Gemma3);
}

#[test]
fn the_vm_runs_gpt_oss_to_the_references_tokens() {
    the_vm_generates_the_references_tokens(Tiny::GptOss);
}

#[test]
fn the_vm_runs_deepseek2_to_the_references_tokens() {
    the_vm_generates_the_references_tokens(Tiny::DeepSeek2);
}

#[test]
fn the_vm_runs_split_deepseek2_to_the_references_tokens() {
    the_vm_generates_the_references_tokens(Tiny::DeepSeek2Split);
}

/// Every tiny architecture has its VM run above: a new one is added to both.
#[test]
fn every_tiny_architecture_has_a_vm_run() {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/bringup_lock.rs")).unwrap();
    for arch in tiny_models::ALL {
        let call = format!("the_vm_generates_the_references_tokens(Tiny::{arch:?});");
        assert!(src.contains(&call), "{arch:?} has no VM run: add a test calling `{call}`");
    }
}

/// A CPU tier lowers the decode step and generates the f64 reference's tokens — the VM's, which equal them on every
/// architecture — on the hybrid, which carries every kind of state across its cuts (caches, the delta memory, the
/// convolution rings), each cut reading its weights and state at run time (`readReals`). Where `needs` names a tool,
/// a machine without it refuses by naming it; where it names none, the tier runs on every machine this lock runs on.
fn the_tier_generates_the_references_tokens(chip: &str, needs: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Qwen35.spec(24);
    let ckpt = checkpoint(&spec, dir.path(), "qwen35");
    let out = dir.path().join("out");
    let b = bring_up_spec(spec.clone(), &ckpt, 24, &out, &chips(&[chip])).unwrap();
    let r = b.results[0].1.as_ref().unwrap_or_else(|e| panic!("{chip}: it lowers the decode step: {e}"));
    assert_eq!(r.programs, [b.decode], "{chip}: its artifact is the decode step's");
    assert!(!r.files.is_empty() && r.files.iter().all(|f| f.is_file()), "{chip}: its artifacts are on disk");
    let reference = Reference::of(&spec, &ckpt, PROMPT, 2).unwrap();
    let runs = run(&b, &ckpt, 24, &out, PROMPT, reference.clone());
    match ran(&runs, chip) {
        Ok(r) => assert_eq!(r.tokens, reference.tokens, "{chip} generates the reference's tokens"),
        Err(why) => match needs {
            Some(tool) => assert!(why.0.contains(tool), "{chip} refuses only for want of {tool}: {why}"),
            None => panic!("{chip} runs on this machine: {why}"),
        },
    }
}

/// The JIT, its native tier installed as `largo` installs it, runs wherever that tier exists.
#[test]
fn the_jit_generates_the_references_tokens() {
    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    logicaffeine_jit::install();
    let needs = if cfg!(all(target_arch = "x86_64", not(target_os = "windows"))) { None } else { Some("JIT") };
    the_tier_generates_the_references_tokens("jit", needs);
}

#[test]
fn the_c_tier_generates_the_references_tokens() {
    the_tier_generates_the_references_tokens("c", Some("cc"));
}

#[test]
fn the_rust_tier_generates_the_references_tokens() {
    the_tier_generates_the_references_tokens("rust", Some("cargo"));
}

#[test]
fn the_riscv_tier_generates_the_references_tokens() {
    the_tier_generates_the_references_tokens("riscv-qemu", Some("qemu-riscv64"));
}

#[test]
fn the_wasm_tier_generates_the_references_tokens() {
    the_tier_generates_the_references_tokens("wasm", Some("node"));
}

/// The prompts the runs of several sequences start from: unequal lengths, so each sequence's positions start over and
/// its state starts empty, and a one-token prompt whose first step is already a generated token's.
const PROMPTS: &[&[u32]] = &[&[5, 17, 2, 30], &[9], &[40, 3, 12]];

/// Each of [`PROMPTS`] with the tokens the model's f64 reference generates after it.
fn sequences(spec: &ModelSpec, ckpt: &Path, tokens: usize) -> Vec<Sequence> {
    PROMPTS
        .iter()
        .map(|p| Sequence { prompt: p.to_vec(), tokens: Reference::of(spec, ckpt, p, tokens).unwrap().tokens })
        .collect()
}

/// What `chip` generated in `runs`.
fn generated<'a>(runs: &'a SequenceRuns, chip: &str) -> &'a Result<Generated, Refused> {
    &runs.results.iter().find(|(c, _)| *c == chip).unwrap_or_else(|| panic!("{chip} was not run")).1
}

/// The decode step's lines a generation program holds: every line but `## Main` and the two bindings the loop gives
/// (the token and the position), split into its declarations (`Let V be a … tensor of Real.`) and the statements
/// that compute.
fn statements(decode: &str) -> (Vec<&str>, Vec<&str>) {
    decode
        .lines()
        .filter(|l| *l != "## Main" && !l.starts_with("Let tok be ") && !l.starts_with("Let pos be "))
        .partition(|l| l.starts_with("Let ") && l.contains(" be a "))
}

/// Whether `want` appears in `lines` in order (each after the one before).
fn in_order(lines: &[&str], want: &[&str]) -> Result<(), String> {
    let mut at = 0;
    for w in want {
        match lines[at..].iter().position(|l| l.trim() == w.trim()) {
            Some(i) => at += i + 1,
            None => return Err(format!("`{w}` is not in the generation program after line {at}")),
        }
    }
    Ok(())
}

/// A run of several sequences builds one generation program a chip and runs it once, and every sequence generates
/// its reference's tokens: the hybrid carries every kind of state (caches, the delta memory, the convolution rings),
/// so a sequence that saw another's state would part from its reference. The program the run wrote holds all of the
/// one program but the token and position the loop gives: every statement that computes, in order, inside its loops,
/// and every declaration once — each weight read once, before its first loop. It is the only program the run wrote.
/// Where `needs` names a tool, a machine without it refuses by naming it.
fn the_tier_generates_every_sequence_from_one_program(chip: &str, needs: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Qwen35.spec(24);
    let ckpt = checkpoint(&spec, dir.path(), "qwen35");
    let out = dir.path().join("out");
    let b = bring_up_spec(spec.clone(), &ckpt, 24, &out, &chips(&[chip])).unwrap();
    let seqs = sequences(&spec, &ckpt, 3);
    let runs = run_sequences(&b, &ckpt, 24, &out, &seqs);
    let g = match generated(&runs, chip) {
        Ok(g) => g,
        Err(why) => match needs {
            Some(tool) if why.0.contains(tool) => return,
            _ => panic!("{chip} runs on this machine: {why}"),
        },
    };
    let want: Vec<Vec<u32>> = seqs.iter().map(|s| s.tokens.clone()).collect();
    assert_eq!(g.tokens, want, "{chip}: every sequence generates its reference's tokens");
    assert!(g.seconds > 0.0, "{chip}: the run is timed");
    let text = std::fs::read_to_string(&g.program).unwrap_or_else(|e| panic!("{chip}: {}: {e}", g.program.display()));
    let lines: Vec<&str> = text.lines().collect();
    let first_loop = lines.iter().position(|l| l.trim_start().starts_with("Repeat for")).expect("the generation loops");
    let (declarations, computing) = statements(&b.program);
    in_order(&lines[first_loop..], &computing).unwrap_or_else(|e| panic!("{chip}: {e}"));
    for d in &declarations {
        assert_eq!(lines.iter().filter(|l| l.trim() == *d).count(), 1, "{chip}: `{d}` is declared once");
    }
    let reads: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].contains("readReals(")).collect();
    assert!(!reads.is_empty() && reads.iter().all(|&i| i < first_loop), "{chip}: every weight is read once, before the loops");
    let programs: Vec<_> = std::fs::read_dir(g.program.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "lg"))
        .collect();
    assert_eq!(programs, [g.program.clone()], "{chip}: the run wrote one program");
}

#[test]
fn the_vm_generates_every_sequence_from_one_program() {
    the_tier_generates_every_sequence_from_one_program("vm", None);
}

#[test]
fn the_jit_generates_every_sequence_from_one_program() {
    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    logicaffeine_jit::install();
    let needs = if cfg!(all(target_arch = "x86_64", not(target_os = "windows"))) { None } else { Some("JIT") };
    the_tier_generates_every_sequence_from_one_program("jit", needs);
}

#[test]
fn the_c_tier_generates_every_sequence_from_one_program() {
    the_tier_generates_every_sequence_from_one_program("c", Some("cc"));
}

#[test]
fn the_rust_tier_generates_every_sequence_from_one_program() {
    the_tier_generates_every_sequence_from_one_program("rust", Some("cargo"));
}

#[test]
fn the_riscv_tier_generates_every_sequence_from_one_program() {
    the_tier_generates_every_sequence_from_one_program("riscv-qemu", Some("qemu-riscv64"));
}

#[test]
fn the_wasm_tier_generates_every_sequence_from_one_program() {
    the_tier_generates_every_sequence_from_one_program("wasm", Some("node"));
}

/// The D5005's machine model runs every sequence from an empty state, each its own sequence of the machine.
#[test]
fn the_d5005_machine_model_runs_every_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.heads, a.kv_heads) = (4, 2);
        }
    }
    let ckpt = checkpoint(&spec, dir.path(), "hybrid");
    let out = dir.path().join("out");
    let b = bring_up_spec(spec.clone(), &ckpt, 64, &out, &chips(&["d5005"])).unwrap();
    let seqs = sequences(&spec, &ckpt, 2);
    let runs = run_sequences(&b, &ckpt, 64, &out, &seqs);
    let g = generated(&runs, "d5005").as_ref().unwrap_or_else(|e| panic!("the D5005's machine model runs: {e}"));
    assert_eq!(g.tokens.len(), seqs.len(), "one generation a sequence");
    assert!(g.tokens.iter().all(|t| t.len() == 2), "{:?}", g.tokens);
    let alone = run_sequences(&b, &ckpt, 64, &out, &seqs[1..2]);
    let first = generated(&alone, "d5005").as_ref().unwrap();
    assert_eq!(first.tokens[0], g.tokens[1], "a sequence generates what it generates alone: no state carries over");
}

/// A sequence whose prompt and tokens do not fit the caches is refused, naming the context, before anything runs.
#[test]
fn a_sequence_past_its_context_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Llama.spec(8);
    let ckpt = checkpoint(&spec, dir.path(), "llama");
    let out = dir.path().join("out");
    let b = bring_up_spec(spec.clone(), &ckpt, 8, &out, &chips(&["vm"])).unwrap();
    let seqs = vec![
        Sequence { prompt: vec![5, 17], tokens: vec![1, 2] },
        Sequence { prompt: PROMPT.to_vec(), tokens: vec![1; 6] },
    ];
    let runs = run_sequences(&b, &ckpt, 8, &out, &seqs);
    let why = generated(&runs, "vm").clone().unwrap_err();
    assert!(why.0.contains("8 positions"), "the refusal names the context: {why}");
}

/// A golden gives a run every one of its prompts, each with its own continuation as the tokens it is judged against.
#[test]
fn a_golden_gives_every_prompt_with_its_continuation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("golden-f32.json");
    std::fs::write(&path, r#"{"prompts": [{"tokens": [5, 17, 2, 30], "continuation": [11, 3, 7]}, {"tokens": [1], "continuation": [2]}]}"#).unwrap();
    let (seqs, what) = Reference::golden_sequences(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(seqs, vec![
        Sequence { prompt: PROMPT.to_vec(), tokens: vec![11, 3, 7] },
        Sequence { prompt: vec![1], tokens: vec![2] },
    ]);
    assert!(what.contains(&path.display().to_string()), "{what}");
    std::fs::write(&path, r#"{"prompts": []}"#).unwrap();
    assert!(Reference::golden_sequences(&path).is_err(), "a golden with no prompt is refused");
}

/// A Hugging Face directory — `config.json` beside its safetensors, the repository as the Hub serves it — brings up
/// with no GGUF, and is the one program the GGUF llama.cpp's converter writes from it is: the same decode step, by its
/// identity and its text. Run from the directory's own weights, each CPU tier generates what it generates from the
/// GGUF.
#[test]
#[ignore = "heavy: the bytecode VM runs twins with their families' real vocabularies, about 20 minutes in a debug build; the full suite runs it"]
fn a_hugging_face_directory_brings_up_as_the_program_its_gguf_is() {
    if !common::require_tool("hf-twins") {
        return;
    }
    for family in ["llama", "qwen35"] {
        let (hf, gguf) = common::hf_twins::twin(family);
        let dir = tempfile::tempdir().unwrap();
        let on = chips(&["vm", "c"]);
        let from_hf = bring_up(&hf, 24, &dir.path().join("hf"), &on).unwrap_or_else(|e| panic!("{family}: the directory: {e}"));
        let from_gguf = bring_up(&gguf, 24, &dir.path().join("gguf"), &on).unwrap_or_else(|e| panic!("{family}: the GGUF: {e}"));
        assert_eq!(from_hf.decode, from_gguf.decode, "{family}: the directory and its GGUF are one program");
        assert_eq!(from_hf.program, from_gguf.program, "{family}: the same text");
        let seqs = [Sequence { prompt: PROMPT.to_vec(), tokens: vec![0; 3] }];
        let (a, b) = (
            run_sequences(&from_hf, &hf, 24, &dir.path().join("hf"), &seqs),
            run_sequences(&from_gguf, &gguf, 24, &dir.path().join("gguf"), &seqs),
        );
        for chip in ["vm", "c"] {
            let (x, y) = (generated(&a, chip), generated(&b, chip));
            let x = x.as_ref().unwrap_or_else(|e| panic!("{family} {chip}: from the directory: {e}"));
            let y = y.as_ref().unwrap_or_else(|e| panic!("{family} {chip}: from the GGUF: {e}"));
            assert_eq!(x.tokens, y.tokens, "{family} {chip}: the directory's weights generate the GGUF's tokens");
        }
    }
}

/// A chip that cannot run here says why by name: the SN50 runs under SambaNova's gated SDK; the fabric does not lower a
/// model the author writes yet, and a chip that did not bring the model up is not run.
#[test]
fn a_chip_that_cannot_run_here_says_why_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let spec = hybrid();
    let ckpt = checkpoint(&spec, dir.path(), "hybrid");
    let out = dir.path().join("out");
    let b = bring_up_spec(spec.clone(), &ckpt, 32, &out, &chips(&["sn50", "fabric"])).unwrap();
    let fabric = b.results.iter().find(|(c, _)| *c == "fabric").unwrap().1.clone().unwrap_err();
    assert!(fabric.0.contains("does not lower a model the author writes"), "the fabric names what it does not do yet: {fabric}");
    let reference = Reference::of(&spec, &ckpt, PROMPT, 2).unwrap();
    let runs = run(&b, &ckpt, 32, &out, PROMPT, reference);
    assert!(runs.results.iter().all(|(c, _)| *c != "fabric"), "a chip that did not bring the model up is not run");
    let sn50 = ran(&runs, "sn50").clone().unwrap_err();
    assert!(sn50.0.contains("SDK"), "the SN50 names what it runs under: {sn50}");
}

/// With no card on this machine, the D5005 runs its engine's machine model and says so — on the hybrid at the width
/// the card lock runs the machine at, whose cached head rows the card's lines hold whole.
#[test]
fn the_d5005_runs_its_engines_machine_model_when_no_card_is_here() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.heads, a.kv_heads) = (4, 2);
        }
    }
    let ckpt = checkpoint(&spec, dir.path(), "hybrid");
    let out = dir.path().join("out");
    let b = bring_up_spec(spec.clone(), &ckpt, 64, &out, &chips(&["d5005"])).unwrap();
    let reference = Reference::of(&spec, &ckpt, PROMPT, 2).unwrap();
    let runs = run(&b, &ckpt, 64, &out, PROMPT, reference);
    let d5005 = ran(&runs, "d5005").as_ref().unwrap_or_else(|e| panic!("the D5005's machine model runs: {e}"));
    assert!(d5005.on.contains("machine model"), "the D5005's run says what ran it: {}", d5005.on);
    assert_eq!(d5005.tokens.len(), 2);
}

/// A run whose prompt and tokens do not fit the cuts' caches is refused, naming the context.
#[test]
fn a_run_past_its_context_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let spec = Tiny::Llama.spec(8);
    let ckpt = checkpoint(&spec, dir.path(), "llama");
    let out = dir.path().join("out");
    let b = bring_up_spec(spec.clone(), &ckpt, 8, &out, &chips(&["vm"])).unwrap();
    let reference = Reference::of(&spec, &ckpt, PROMPT, 6).unwrap();
    let runs = run(&b, &ckpt, 8, &out, PROMPT, reference);
    let why = ran(&runs, "vm").clone().unwrap_err();
    assert!(why.0.contains("8 positions"), "the refusal names the context: {why}");
}

/// A golden gives a run its prompt and the tokens it is judged against: its first prompt and that prompt's
/// continuation, named by the golden's path.
#[test]
fn a_golden_gives_the_prompt_and_the_tokens_a_run_is_judged_against() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("golden-f32.json");
    std::fs::write(&path, r#"{"prompts": [{"tokens": [5, 17, 2, 30], "continuation": [11, 3, 7]}, {"tokens": [1], "continuation": [2]}]}"#).unwrap();
    let (prompt, reference) = Reference::golden(&path).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(prompt, PROMPT);
    assert_eq!(reference.tokens, [11, 3, 7]);
    assert!(reference.what.contains(&path.display().to_string()), "{}", reference.what);
    std::fs::write(&path, r#"{"prompts": []}"#).unwrap();
    assert!(Reference::golden(&path).is_err(), "a golden with no prompt is refused");
}
