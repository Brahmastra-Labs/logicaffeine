//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! Every checkpoint we bring up, held to llama.cpp's own runs of it.
//!
//! `scripts/accel/model-golden.sh` runs the pinned llama.cpp on the CPU over the prompt corpus
//! `benchmarks/models/prompts.txt` and records, per prompt, its token ids, the top five logits after it and a greedy
//! continuation. Each model has two such goldens in `benchmarks/models/<model>/`:
//!
//!   • `golden-f32.json` — llama.cpp over the checkpoint's weights dequantized to `f32` (`examples/gguf_f32`) with an
//!     `f32` KV cache: the same weights the `f64` reference reads, computed without llama.cpp's 8-bit activations or
//!     `f16` cache. The reference (`model::reference`, the spec read from the checkpoint) holds every one of its top
//!     logits within `SEMANTIC_TOLERANCE`, and continues greedily as it does except at a tie that close.
//!   • `golden-cpu.json` — llama.cpp at the precision it ships (the checkpoint as quantized, an `f16` cache). The
//!     reference follows it at least as long as both follow llama.cpp's `f32` run: where the reference parts from
//!     the shipped run, llama.cpp's own `f32` run has parted from it as well, or the reference parted from the `f32`
//!     run at a tie that close. This is the divergence the accuracy ruling admits — the reference parts from
//!     llama.cpp only where llama.cpp's own precision cannot decide.
//!
//! The checkpoint's vocabulary (`model::tokenizer`) tokenizes every prompt to llama.cpp's ids and renders both
//! goldens' continuations as llama.cpp's text.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::reference::{argmax, Reference};
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::model::tokenizer::Vocab;

/// How far a logit of the `f64` reference may sit from llama.cpp's `f32` run of the same weights.
const SEMANTIC_TOLERANCE: f64 = 0.05;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden(model: &str, which: &str) -> serde_json::Value {
    let path = root().join("benchmarks/models").join(model).join(format!("golden-{which}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("the golden")).expect("json")
}

fn ids(v: &serde_json::Value) -> Vec<u32> {
    v.as_array().expect("ids").iter().map(|t| t.as_u64().expect("an id") as u32).collect()
}

fn top(v: &serde_json::Value) -> Vec<(u32, f64)> {
    v.as_array().unwrap().iter().map(|e| (e[0].as_u64().unwrap() as u32, e[1].as_f64().unwrap())).collect()
}

/// The reference's own greedy run from a prompt, computed as far as a walk asks: the logits after the prompt, then
/// after each token it chose. Every walk follows this one run — the reference steps on its own tokens — so the walks
/// against both goldens read it, and it is computed once.
struct Trajectory<'a> {
    reference: &'a mut Reference,
    logits: Vec<Vec<f64>>,
}

impl<'a> Trajectory<'a> {
    fn from_prompt(reference: &'a mut Reference, tokens: &[u32]) -> Self {
        reference.reset();
        let first = reference.step(tokens).expect("prefill");
        Trajectory { reference, logits: vec![first] }
    }

    /// The logits after the prompt and the run's first `i` tokens.
    fn at(&mut self, i: usize) -> &[f64] {
        while self.logits.len() <= i {
            let next = argmax(self.logits.last().expect("the prompt's logits"));
            let logits = self.reference.step(&[next]).expect("step");
            self.logits.push(logits);
        }
        &self.logits[i]
    }
}

/// Walk a golden continuation along the reference's greedy run: the steps agreed before the two part. A parting is
/// admitted only at a tie within `tie`.
fn walk(run: &mut Trajectory, continuation: &[u32], tie: f64, what: &str) -> usize {
    for (i, &want) in continuation.iter().enumerate() {
        let logits = run.at(i);
        let got = argmax(logits);
        if got != want {
            let gap = logits[got as usize] - logits[want as usize];
            assert!(gap <= tie, "{what} step {i}: the reference takes {got}, llama.cpp {want}, {gap} below — not a tie within {tie}");
            return i;
        }
    }
    continuation.len()
}

/// Hold the checkpoint under `models/<model>/` to its goldens.
fn check(model: &str) {
    if !common::require_tool(&format!("checkpoint:{model}")) {
        return;
    }
    let (cpu, f32g) = (golden(model, "cpu"), golden(model, "f32"));
    let path = common::checkpoint(model).unwrap();
    let g = Gguf::open(&path).expect("the checkpoint opens");
    let vocab = Vocab::from_gguf(&g).expect("the vocabulary reads");
    let spec = ModelSpec::from_gguf(&g).expect("the spec reads");
    let mut reference = Reference::new(spec, g).expect("the reference runs the model");
    let (mut worst, mut parted) = (0f64, 0usize);
    let mut report = Vec::new();
    let cases = cpu["prompts"].as_array().expect("prompts");
    assert_eq!(cases.len(), f32g["prompts"].as_array().unwrap().len(), "{model}: the goldens cover one corpus");
    for (p, (c, f)) in cases.iter().zip(f32g["prompts"].as_array().unwrap()).enumerate() {
        let what = format!("{model} prompt {p}");
        let tokens = ids(&c["tokens"]);
        assert_eq!(ids(&f["tokens"]), tokens, "{what}: the goldens tokenize alike");
        assert_eq!(vocab.tokenize(c["text"].as_str().unwrap().as_bytes(), true, false).unwrap(), tokens, "{what} tokenizes");
        for gold in [c, f] {
            assert_eq!(
                String::from_utf8_lossy(&vocab.detokenize(&ids(&gold["continuation"]), false, false)),
                gold["continuation_text"].as_str().unwrap(),
                "{what}: a continuation renders"
            );
        }
        let (ct, ft) = (top(&c["top"]), top(&f["top"]));
        let f_at: HashMap<u32, f64> = ft.iter().copied().collect();
        let noise = ct.iter().filter_map(|(id, v)| f_at.get(id).map(|w| (v - w).abs())).fold(0.0, f64::max);

        let mut run = Trajectory::from_prompt(&mut reference, &tokens);
        let logits = run.at(0);
        let dev = ft.iter().map(|&(id, v)| (logits[id as usize] - v).abs()).fold(0.0, f64::max);
        assert!(dev <= SEMANTIC_TOLERANCE, "{what}: a top logit is {dev} from llama.cpp's f32 run");
        worst = worst.max(dev);
        let semantic = walk(&mut run, &ids(&f["continuation"]), SEMANTIC_TOLERANCE, &format!("{what} (f32)"));
        let shipped = walk(&mut run, &ids(&c["continuation"]), f64::INFINITY, &format!("{what} (cpu)"));
        let (fc, cc) = (ids(&f["continuation"]), ids(&c["continuation"]));
        let precisions_agree = fc.iter().zip(&cc).take_while(|(a, b)| a == b).count();
        assert!(
            shipped >= semantic.min(precisions_agree),
            "{what}: the reference follows the shipped run {shipped} steps, though it follows the f32 run {semantic} and the f32 run the shipped one {precisions_agree}"
        );
        parted += (shipped < cc.len()) as usize;
        report.push(format!(
            "prompt {p}: f32 {semantic}/{} within {dev:.5}; shipped {shipped}/{} (llama.cpp's precisions agree {precisions_agree}, top logits {noise:.4} apart)",
            ids(&f["continuation"]).len(),
            ids(&c["continuation"]).len()
        ));
    }
    eprintln!(
        "{model}: top logits within {worst:.5} of llama.cpp's f32 run; the shipped run parted from in {parted} prompt(s)\n  {}",
        report.join("\n  ")
    );
}

#[test]
fn qwen3_5_0_8b_matches_its_llama_cpp_goldens() {
    check("qwen3.5-0.8b");
}

#[test]
fn llama3_2_1b_matches_its_llama_cpp_goldens() {
    check("llama3.2-1b");
}

/// Llama 3.2 1B as bartowski's IQ4_XS quantization: its projections IQ4_XS, its values Q5_K, its tied table Q6_K.
#[test]
fn llama3_2_1b_iq4xs_matches_its_llama_cpp_goldens() {
    check("llama3.2-1b-iq4xs");
}

#[test]
fn gemma3_1b_matches_its_llama_cpp_goldens() {
    check("gemma3-1b");
}

/// Gemma 2 2B: every attention score soft-capped at 50 and the logits at 30, every other layer windowed (PARITY-SOFTCAP).
#[test]
fn gemma2_2b_matches_its_llama_cpp_goldens() {
    check("gemma2-2b");
}

/// OLMo 2 1B (the 0425 instruct release as AllenAI quantizes it): no norm before either sublayer, a norm after each, its
/// queries and keys each normed across every head.
#[test]
fn olmo2_1b_matches_its_llama_cpp_goldens() {
    check("olmo2-1b");
}

#[test]
fn gpt_oss_20b_matches_its_llama_cpp_goldens() {
    check("gpt-oss-20b");
}

#[test]
fn deepseek_v2_lite_matches_its_llama_cpp_goldens() {
    check("deepseek-v2-lite");
}

/// Qwen3-8B, the scoreboard's widest dense model: its `f32` copy run batched (`model-golden.sh` batch 1), the copy's
/// 33 GB read once a step for the whole corpus.
#[test]
fn qwen3_8b_matches_its_llama_cpp_goldens() {
    check("qwen3-8b");
}

/// Every model with goldens under `benchmarks/models/` has its test here, and both goldens — each run by llama.cpp on
/// the CPU alone: a llama.cpp built with CUDA runs an operation on a batch of 32 or more tokens on the GPU unless its
/// offload is off, so a "CPU" golden of a long prompt would carry the GPU's numerics.
#[test]
fn every_golden_is_checked() {
    let src = include_str!("model_golden_lock.rs");
    for entry in std::fs::read_dir(root().join("benchmarks/models")).unwrap() {
        let dir = entry.unwrap().path();
        if dir.join("golden-cpu.json").is_file() || dir.join("golden-f32.json").is_file() {
            let name = dir.file_name().unwrap().to_string_lossy().into_owned();
            assert!(src.contains(&format!("check(\"{name}\")")), "{name} has a golden and no test");
            assert!(dir.join("golden-cpu.json").is_file() && dir.join("golden-f32.json").is_file(), "{name} needs both goldens");
            for which in ["cpu", "f32"] {
                let g = golden(&name, which);
                assert_eq!(g["device"], "cpu", "{name}: the {which} golden ran on the CPU");
                assert_eq!(g["op_offload"], false, "{name}: the {which} golden offloaded no operation to a GPU");
            }
        }
    }
}

/// llama.cpp's own GPU run (`model-golden.sh … device=cuda`, the scoreboard's quality side) runs a checkpoint as
/// llama-bench runs it: every layer on the GPU and its context's batches llama-bench's defaults, 2,048 tokens a batch in
/// micro-batches of 512. A 4,096-token micro-batch's compute buffer does not fit DeepSeek-V2-Lite beside its weights on
/// a 12 GB card, and the context would not initialize. The JSON records the batch sizes the run used.
#[test]
fn the_gpu_golden_runs_with_llama_benchs_batches() {
    if !common::require_tool("checkpoint:qwen3.5-0.8b") || !common::require_gpu() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("model_golden_gpu_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let prompts = dir.join("prompts.txt");
    std::fs::write(&prompts, "The capital of France is").unwrap();
    let out = dir.join("golden.json");
    let gguf = root().join("models/qwen3.5-0.8b/Qwen3.5-0.8B-Q8_0.gguf");
    let run = std::process::Command::new(root().join("scripts/accel/model-golden.sh"))
        .arg(&gguf)
        .arg(&out)
        .args(["2", "16", "auto", "f16"])
        .arg(&prompts)
        .args(["0", "cuda"])
        .output()
        .expect("model-golden.sh");
    assert!(run.status.success(), "the GPU golden runs:\n{}", String::from_utf8_lossy(&run.stderr));
    let g: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(g["device"], "cuda");
    assert_eq!(g["op_offload"], true, "every operation offloaded, as llama-bench runs it");
    assert_eq!(g["n_batch"], 2048, "llama-bench's batch");
    assert_eq!(g["n_ubatch"], 512, "llama-bench's micro-batch");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The golden's JSON is valid UTF-8 whatever llama.cpp generates. A continuation of `n_gen` tokens can stop inside a
/// character — DeepSeek-V2-Lite's GPU run ended one in `\xd8`, half an Arabic letter, and the JSON could not be read —
/// so `model_golden.cpp`'s `json_escape` writes every ill-formed or truncated sequence as U+FFFD, a maximal subpart at a
/// time (as `String::from_utf8_lossy` does), and every well-formed one as it is. Held on the tool's own function: the
/// source compiled with its `main` renamed, each input escaped and compared with the escape of its lossy decoding.
#[test]
fn a_golden_writes_valid_utf8_where_a_continuation_stops_inside_a_character() {
    if !common::require_tool("llama-cpp-build") || !common::require_tool("g++") {
        return;
    }
    let llama = common::llama_cpp_root();
    let dir = std::env::temp_dir().join(format!("model_golden_escape_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let harness = dir.join("escape.cpp");
    std::fs::write(
        &harness,
        format!(
            "#define main model_golden_main\n#include \"{}\"\n#undef main\n#include <iostream>\n#include <iterator>\n\
             int main() {{ std::string s((std::istreambuf_iterator<char>(std::cin)), std::istreambuf_iterator<char>()); \
             std::cout << json_escape(s); return 0; }}\n",
            root().join("scripts/accel/model_golden.cpp").display()
        ),
    )
    .unwrap();
    let bin = dir.join("escape");
    let built = std::process::Command::new("g++")
        .args(["-O2", "-std=c++17"])
        .arg(format!("-I{}", llama.join("include").display()))
        .arg(format!("-I{}", llama.join("ggml/include").display()))
        .arg(&harness)
        .arg(format!("-L{}", llama.join("build/bin").display()))
        .args(["-lllama", "-lggml", "-lggml-base"])
        .arg(format!("-Wl,-rpath,{}", llama.join("build/bin").display()))
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("g++");
    assert!(built.status.success(), "the harness builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let escape = |s: &str| -> String {
        s.chars()
            .map(|c| match c {
                '"' => "\\\"".to_string(),
                '\\' => "\\\\".to_string(),
                '\n' => "\\n".to_string(),
                '\r' => "\\r".to_string(),
                '\t' => "\\t".to_string(),
                c if (c as u32) < 0x20 => format!("\\u{:04x}", c as u32),
                c => c.to_string(),
            })
            .collect()
    };
    let inputs: [&[u8]; 12] = [
        b"The capital of France is Paris.\n",
        "caf\u{e9} \u{6771}\u{4eac} \u{1f600} \"quoted\" \\ \t\x01".as_bytes(),
        b" \xd8\xba\xd9\x8a\xd8\xb1 \xd9\x82\xd8",
        b"\x80 a lone continuation",
        b"\xc0\xaf an overlong slash",
        b"\xed\xa0\x80 a surrogate",
        b"\xf4\x90\x80\x80 past U+10FFFF",
        b"\xe2\x82x a truncated three-byte sequence",
        b"\xf0\x9f\x98 a truncated four-byte sequence at the end \xf0\x9f",
        b"\xff\xfe never a lead byte",
        b"\xe0\x9f\xbf an overlong three-byte sequence",
        b"\xf0\x8f\xbf\xbf an overlong four-byte sequence",
    ];
    for input in inputs {
        let mut child = std::process::Command::new(&bin)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("the harness");
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), input).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success());
        let written = String::from_utf8(out.stdout.clone())
            .unwrap_or_else(|_| panic!("{input:?} is escaped as valid UTF-8, not {:?}", out.stdout));
        let lossy = String::from_utf8_lossy(input);
        assert_eq!(written, escape(&lossy), "{input:?}: every ill-formed sequence one U+FFFD, the rest as it is");
        let parsed: String = serde_json::from_str(&format!("\"{written}\"")).expect("a JSON string");
        assert_eq!(parsed, lossy, "{input:?} reads back as its lossy decoding");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
