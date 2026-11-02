//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ★★★ THE QWEN3.8-27B GOLDEN LOCK ★★★ — the LOGOS-authored 27B hybrid reproduces llama.cpp's greedy
//! continuation TOKEN-FOR-TOKEN, on the CPU reference AND on the GPU.
//!
//!   prompt  "The capital of France is"                      → [760, 6511, 314, 9338, 369]
//!   golden  " Paris.\nThe capital of Germany is Berlin.\nThe capital"
//!
//! Six real spec bugs stood between "runs" and "golden", and every one of them produced *fluent* output — the
//! model happily wrote grammatical sentences about the wrong country. Fluency is not a correctness signal for a
//! 27 B model; only the token stream is. That is why this lock exists and why it pins ELEMENTS, not sums.
//!
//! The pod legs (`nvcc` + a live GPU + the extracted weights) SKIP cleanly everywhere else, so the evidence leg
//! is the one that always runs: it holds the recorded result file to the same golden constant the GPU asserts,
//! so the published benchmark JSON can never drift away from the tokens this test defines.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::codegen_cuda::qwen38::{emit_qwen38_generate, Qwen38Cfg};

/// "The capital of France is" under Qwen3.8's own 248320-entry vocabulary. NOT Qwen3-30B's ids — reusing those
/// was the first of the six bugs, and it is invisible: the wrong ids still decode to *some* fluent English.
const PROMPT: [usize; 5] = [760, 6511, 314, 9338, 369];

/// llama.cpp b10450+, `--temp 0`, greedy, `-ngl 99`, same Q8_0 checkpoint.
/// " Paris.\nThe capital of Germany is Berlin.\nThe capital"
const GOLDEN: [usize; 13] = [
    11751, 13, 198, 760, 6511, 314, 9564, 369, 19241, 13, 198, 760, 6511,
];

fn have_nvcc() -> bool {
    Command::new("nvcc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
fn have_gpu() -> bool {
    Path::new("/dev/nvidia0").exists()
}
/// The per-tensor weight directory `extract_qwen38` produces on the pod (`scripts/accel/pod/run-qwen38.sh` step 1).
fn weight_dir() -> Option<PathBuf> {
    let d = PathBuf::from(
        std::env::var("LOGOS_Q38_WDIR").unwrap_or_else(|_| "/workspace/w38".to_string()),
    );
    d.join("Wout.q").exists().then_some(d)
}
fn results_json() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/results/qwen38_27b.json")
}

/// ── THE EVIDENCE LEG (always runs) ──
/// The published result file and this test must name the SAME token stream. Without this, "golden" is a claim
/// living in a JSON file that nothing checks; with it, the JSON is downstream of a constant the GPU asserts.
#[test]
fn the_recorded_result_names_exactly_the_golden_stream_this_lock_defines() {
    let raw = std::fs::read_to_string(results_json())
        .expect("benchmarks/results/qwen38_27b.json must exist");
    let v: serde_json::Value = serde_json::from_str(&raw).expect("result file must be valid JSON");
    let logos = &v["logos"];
    assert!(
        !logos.is_null(),
        "the result file must carry a `logos` section — that is the whole point of it"
    );

    let nums = |k: &str| -> Vec<usize> {
        logos[k]
            .as_array()
            .unwrap_or_else(|| panic!("logos.{k} must be an array"))
            .iter()
            .map(|x| x.as_u64().expect("token id") as usize)
            .collect()
    };
    assert_eq!(
        nums("golden_prompt_tokens"),
        PROMPT.to_vec(),
        "the recorded prompt must be the tokenized prompt this lock drives"
    );
    assert_eq!(
        nums("golden_tokens"),
        GOLDEN.to_vec(),
        "the recorded golden stream must be the one this lock asserts on the GPU"
    );

    // The two independent implementations that reproduced it. If either is ever recorded as false, this is no
    // longer a golden result and the file must not keep saying it is.
    assert_eq!(logos["cpu_reference"]["tokens_match"], serde_json::Value::Bool(true),
        "the CPU reference forward is the oracle the GPU is checked against — it must be recorded as matching");
    assert_eq!(
        logos["a100_sxm4_80gb"]["precise_lane"]["tokens_match"],
        serde_json::Value::Bool(true),
        "the precise CUDA lane is the golden GPU lane"
    );

    // The fast lane is HONESTLY recorded as diverging (split-K reorders float adds), exactly as llama.cpp's own
    // `-fa 1` diverges from its `-fa 0`. Recording it as golden would be the tempting lie; this pins the truth.
    assert_eq!(logos["a100_sxm4_80gb"]["fast_lane"]["tokens_match"], serde_json::Value::Bool(false),
        "the fast lane diverges — if that ever changes, update the claim deliberately, do not let it drift");

    let ours = logos["a100_sxm4_80gb"]["fast_lane"]["tg128_tok_s"]
        .as_f64()
        .expect("fast lane tok/s");
    let bar = logos["a100_sxm4_80gb"]["llama_cpp_tg128_fa1"]
        .as_f64()
        .expect("llama.cpp bar");
    assert!(
        ours > 0.90 * bar,
        "a measured {ours:.2} tok/s against llama.cpp's {bar:.2} would not be parity; \
        the recorded pair must stay inside 10% or the claim in the file is wrong"
    );
    assert!(logos["bugs_fixed"].as_array().map(|a| a.len()) >= Some(6),
        "all six spec bugs stay written down — each one produced FLUENT wrong output, which is exactly the \
         failure mode a future reader will otherwise rediscover the hard way");
}

/// ── THE GPU LEG ── the emitted precise lane must generate the golden stream token-for-token.
/// Needs the extracted 27B weights + nvcc + a live GPU: SKIPS cleanly off the pod.
#[test]
fn the_logos_authored_27b_generates_llama_cpps_golden_stream_token_for_token() {
    let (Some(wdir), true, true) = (weight_dir(), have_nvcc(), have_gpu()) else {
        eprintln!("SKIP 27B golden lock: needs extracted weights (LOGOS_Q38_WDIR) + nvcc + a live GPU (the pod).");
        return;
    };
    let cfg = Qwen38Cfg::real();
    let cu = emit_qwen38_generate(&cfg, wdir.to_str().unwrap(), &PROMPT, GOLDEN.len(), true, 1);
    let (toks, tok_s) = compile_run_generate(&cu, &wdir, "gen38_golden_lock");
    assert_eq!(toks, GOLDEN.to_vec(),
        "the LOGOS→CUDA 27B precise lane must match llama.cpp greedy token-for-token. A fluent-but-different \
         stream means a spec bug, not a rounding difference — diff the per-layer tensors with llama-eval-callback.");
    let floor: f64 = std::env::var("LOGOS_Q38_TGFLOOR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(35.0);
    assert!(tok_s > floor,
        "the golden lane measured {tok_s:.2} tok/s against a {floor:.1} floor (A100 reference: 41.29 golden, \
         llama.cpp 42.17). Batch-1 decode is launch-latency bound, so a drop here means a stage stopped fusing.");
    println!(
        "27B GOLDEN LOCK ✓ {} tokens token-exact vs llama.cpp at {tok_s:.2} tok/s: {toks:?}",
        toks.len()
    );
}

/// Compile an emitted generation `.cu` and run it. Tokens land on stdout after `GENERATED_TOKENS:`, throughput
/// on stderr after `GEN_TOK_S_TG128`.
fn compile_run_generate(cu: &str, dir: &Path, tag: &str) -> (Vec<usize>, f64) {
    let cu_path = dir.join(format!("{tag}.cu"));
    let bin = dir.join(tag);
    std::fs::write(&cu_path, cu).expect("write .cu");
    let arch = std::env::var("LOGOS_SM").unwrap_or_else(|_| "sm_80".to_string());
    let comp = Command::new("nvcc")
        .args(["-O3", &format!("-arch={arch}")])
        .arg(&cu_path)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("nvcc");
    assert!(
        comp.status.success(),
        "{tag} must compile:\n{}",
        String::from_utf8_lossy(&comp.stderr)
    );
    let run = Command::new(&bin).output().expect("run generation");
    assert!(
        run.status.success(),
        "{tag} run failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let stdout = String::from_utf8_lossy(&run.stdout);
    let line = stdout
        .lines()
        .find(|l| l.contains("GENERATED_TOKENS:"))
        .unwrap_or_else(|| panic!("no GENERATED_TOKENS line in:\n{stdout}"));
    let toks: Vec<usize> = line
        .split("GENERATED_TOKENS:")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();

    let stderr = String::from_utf8_lossy(&run.stderr);
    let tok_s = stderr
        .lines()
        .find(|l| l.contains("GEN_TOK_S_TG128"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no GEN_TOK_S_TG128 in:\n{stderr}"));
    (toks, tok_s)
}

/// ── THE LAUNCH RATCHET (always runs) ──
/// Batch-1 decode of a 27 B model is bandwidth- and launch-latency-bound, not compute-bound: the arithmetic
/// per byte of weight is one multiply-add. So the two things that decide throughput are how many bytes each
/// weight costs and how many kernels a token takes — and both are properties of the EMITTED source, checkable
/// without a GPU. This test pins them, so a refactor that quietly un-fuses a stage fails here in seconds
/// instead of showing up as a mysterious 6% on a rented A100 three weeks later.
///
/// The ceiling is a RATCHET: lowering it as fusions land is the intended edit. Raising it is not.
#[test]
fn the_decode_step_stays_fused_and_the_weights_stay_narrow() {
    let cfg = Qwen38Cfg::real();
    for &(precise, ceiling, lane) in &[(true, 800usize, "precise"), (false, 816, "fast")] {
        let cu = emit_qwen38_generate(&cfg, "/w", &PROMPT, 4, precise, 0);
        let step = cu
            .split("decode_step(cudaStream_t")
            .nth(1)
            .expect("decode_step must exist");
        let step = &step[..step.find("\n}").expect("decode_step must close")];
        let launches = step.matches("<<<").count();
        assert!(launches <= ceiling,
            "the {lane} decode step takes {launches} kernel launches, over the {ceiling} ceiling. Every launch \
             is a few microseconds of dispatch that no amount of bandwidth wins back — fuse, or justify and \
             raise the ceiling deliberately.");

        // The residual add belongs to the NEXT layer's entry norm, not to its own launch. Exactly one
        // survives: the last layer's, which feeds the final norm and has no next layer to fold into.
        assert_eq!(
            step.matches("gk_add<<<").count(),
            1,
            "[{lane}] the per-layer residual add must be folded into the following entry norm"
        );
        // The l2 normalize rides in the conv, and the activation quantization rides in the gate norm.
        assert_eq!(
            step.matches("gk_l2norm_heads<<<").count(),
            0,
            "[{lane}] the per-head L2 normalize belongs inside gk_conv_silu_l2"
        );
        assert_eq!(
            step.matches("gk_conv_silu_l2<<<").count(),
            cfg.layers - cfg.attn_layers(),
            "[{lane}] one fused conv per GDN layer"
        );
        assert_eq!(
            step.matches("gdn_gate_norm_q<<<").count(),
            cfg.layers - cfg.attn_layers(),
            "[{lane}] one fused gate-norm-and-quantize per GDN layer"
        );

        // Q8_0's scale is an f16 in the checkpoint. Carrying it as f32 on the GPU costs 4 bytes per 32
        // weights instead of 2 — 1.125 vs 1.0625 bytes per weight, a 5.6% surcharge on a decode that is
        // bandwidth bound — and buys nothing, because the inner loop converts it back to f32 either way.
        assert!(
            cu.contains("static __half*"),
            "[{lane}] Q8_0 block scales must be DECLARED as __half"
        );
        // Declaring the pointer narrow is half the job. The loader has to actually narrow the VALUES: an
        // edit that changed the type but left the upload copying raw f32 bytes ran at full speed, produced a
        // logit vector of zeros, and looked exactly like a working program from the outside. Assert the
        // conversion and the 2-byte upload, not just the type.
        assert!(
            cu.contains("__float2half(hs[i])"),
            "[{lane}] the weight loader must convert the f32 scale file to __half on the host"
        );
        assert!(cu.contains("cudaMalloc(&v##_s,(size_t)(SN)*2)") && !cu.contains("cudaMemcpy(v##_s,hs,"),
            "[{lane}] the scale upload must be 2 bytes per scale, from the CONVERTED buffer -- uploading the \
             f32 array into a __half allocation is silent garbage, not a compile error");
        assert_eq!(
            cu.matches("gk_gemv<<<").count(),
            0,
            "[{lane}] every Q8_0 GEMV must be the half-scale form gk_gemv_h"
        );
        // The generated source is C, not Rust: a `\n\` line continuation or an `\x20` space escape is a
        // SOURCE-level construct of the emitter's own string literals. If one reaches the output it means a
        // fragment was built by a nested `format!`, where those stop being escapes and become literal text —
        // which nvcc reads as stray tokens, hundreds of miles and one pod rental away from here.
        for stray in ["\\x20", "\\n\\"] {
            assert!(!cu.contains(stray),
                "[{lane}] the emitted CUDA contains the literal {stray:?} — an emitter escape leaked into the \
                 generated source, which will not compile");
        }
        println!("[{lane}] {launches} launches/token (ceiling {ceiling}), half-scale weights, no leaked escapes ✓");
    }
}
