//! `largo emit rdu`, END TO END: the artifact on disk computes what the VM computes.
//!
//! Every other lock on this lane checks a piece — the certificate, the schema, the placement text, a
//! refusal. None of them run the file the command actually writes. So a bug that lives between the
//! compiler and the disk — serialization, CLI routing, the artifact drifting from the program it was
//! built from — would pass all of them. This file closes that gap. For each program it:
//!
//!   1. runs `largo emit rdu --chip sn50 -o DIR` as a subprocess, the way it is run on the day;
//!   2. reads `DIR/neutral.json` back OFF DISK and parses it into a program;
//!   3. re-certifies that parsed program independently, and requires the digest to equal the one
//!      `certificate.json` carries — the certificate describes THIS file, not some earlier program;
//!   4. executes the parsed program through the spatial evaluator on its recorded fills;
//!   5. runs the same `.lg` source on the VM, and compares every shown value.
//!
//! Integer programs must agree exactly. Float programs run the SN50's witnessed bf16 datapath against an
//! f64 oracle, so they are held to a stated, measured drift bound — and, for the model, the argmax must
//! agree exactly, because the token is what a model emits.
//!
//! Run: `cargo nextest run -p logicaffeine-cli -E 'binary(emit_rdu_e2e)'`
//!
//! ★ MUTATION PROBES — both RUN when this file landed, each against the product, never the test:
//!   - `certificate_summary` ships `schedule_digest ^ 1` → all three red at step 3: the certificate no
//!     longer describes the file beside it;
//!   - `blocked_rms_norm` computes `recip` where it should compute `rsqrt` — a well-formed, certifiable
//!     program with the wrong arithmetic, invisible to any test that only reads messages or schemas →
//!     the real-width RMSNorm reds at step 5 on the values, and the matmul and Qwen cases stay green,
//!     because neither lowers through the blocked form.

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use logicaffeine_compile::codegen_cuda::qwen38::{
    gen_qwen38_decode_batch_lg, gen_qwen38_decode_step_lg, gen_qwen38_lg, gen_qwen38_prefill_lg,
    Qwen38Cfg,
};
use logicaffeine_compile::compile::{tw_outcome, vm_outcome, RunOutcome};
use logicaffeine_compile::spatial::certificate::certify_choreography;
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{NumericsClaim, SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::lower::rdu::parse_neutral;
use tempfile::tempdir;

/// What one end-to-end run produced: the parsed artifact, the output tensor as the artifact computes it,
/// and the values the VM showed for the same source.
struct Run {
    program: SpatialProgram,
    artifact_out: Vec<f64>,
    vm_shown: Vec<i64>,
}

/// Steps 1–5 above, for one source whose output tensor is named `out_name`.
fn emit_and_execute(source: &str, out_name: &str) -> Run {
    emit_and_observe(source, source, out_name, vm_outcome)
}

/// Steps 1–5, with the COMPUTATION shared and only the OBSERVATION differing. `compiled` goes to `largo`;
/// `observed` goes to the VM. The caller builds both from one set of tensor statements and fills, so they
/// compute the same thing — `observed` only adds the lines that print every output element. That split
/// exists because the tensor lane (rightly) refuses a program containing a `While` loop rather than
/// lowering part of it, and the VM's register frame cannot hold thousands of straight-line `Show`s.
fn emit_and_observe(
    compiled: &str,
    observed: &str,
    out_name: &str,
    oracle: fn(&str) -> RunOutcome,
) -> Run {
    let source = compiled;
    let dir = tempdir().unwrap();
    let file = dir.path().join("prog.lg");
    std::fs::write(&file, source).unwrap();
    let bundle = dir.path().join("bundle");

    // 1. the command, as it is run on the day.
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--chip",
            "sn50",
            file.to_str().unwrap(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo emit rdu failed:\n{}",
        stderr(&out)
    );

    // 2. the artifact, read back off disk.
    let text =
        std::fs::read_to_string(bundle.join("neutral.json")).expect("neutral.json was written");
    let mut program =
        parse_neutral(&text).expect("the written neutral.json parses back into a program");

    // 3. an independent certification of THIS file, bound to the certificate that shipped beside it.
    let shipped: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(bundle.join("certificate.json"))
            .expect("certificate.json was written"),
    )
    .unwrap();
    program.certificate = None;
    let cert =
        certify_choreography(&program).expect("the program in neutral.json certifies on its own");
    assert_eq!(
        serde_json::json!(cert.schedule_digest),
        shipped["schedule_digest"],
        "certificate.json describes a different schedule than the one neutral.json contains"
    );

    // 4. the artifact, executed.
    let executed = eval_choreography(&program, &inputs_from_fills(&program))
        .expect("the artifact's choreography evaluates");
    let out_t = program
        .tensors
        .iter()
        .find(|t| t.name == out_name)
        .unwrap_or_else(|| panic!("the artifact has no tensor `{out_name}`"));
    let artifact_out = match executed
        .tensors
        .get(&out_t.id)
        .expect("the choreography writes the output")
    {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    };

    // 5. the oracle.
    let vm = oracle(observed);
    assert!(
        vm.error.is_none(),
        "the source runs on the VM: {:?}",
        vm.error
    );
    let vm_shown = vm
        .output
        .split_whitespace()
        .map(|t| t.parse::<i64>().expect("an integer line"))
        .collect();

    Run {
        program,
        artifact_out,
        vm_shown,
    }
}

/// Every Input/Param/State tensor materialized from the fill the artifact recorded for it.
fn inputs_from_fills(p: &SpatialProgram) -> Inputs {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let n = t.elems.rows * t.elems.cols;
        if t.init.is_float() {
            let v: Vec<f64> = (0..n)
                .map(|i| t.init.element_f64(i).expect("a deterministic float fill"))
                .collect();
            inputs.tensors.insert(t.id, Value::Float(v));
        } else {
            let v: Vec<i64> = (0..n)
                .map(|i| t.init.element(i).expect("a deterministic fill"))
                .collect();
            inputs.tensors.insert(t.id, Value::Int(v));
        }
    }
    inputs
}

/// Worst element-wise drift between two ×1e6-rounded vectors, relative to the largest magnitude.
fn worst_relative_drift(ours: &[i64], vm: &[i64]) -> f64 {
    let scale = vm.iter().map(|x| x.abs()).max().unwrap_or(1).max(1) as f64;
    ours.iter()
        .zip(vm)
        .map(|(a, b)| ((a - b) as f64).abs() / scale)
        .fold(0.0, f64::max)
}

/// ── INTEGERS AGREE EXACTLY. A matmul whose every output element is shown.
#[test]
fn an_integer_matmul_artifact_computes_exactly_what_the_vm_computes() {
    let (m, k, n) = (8usize, 12usize, 8usize);
    let mut s = String::from("## Main\n");
    s += &format!("Let A be a {m} by {k} tensor of Int.\n");
    for i in 1..=m * k {
        s += &format!("Set item {i} of A to {}.\n", (i as i64 * 7) % 11 - 5);
    }
    s += &format!("Let B be a {k} by {n} tensor of Int.\n");
    for i in 1..=k * n {
        s += &format!("Set item {i} of B to {}.\n", (i as i64 * 5) % 13 - 6);
    }
    s += "Let C be A matmul B.\n";
    for i in 1..=m * n {
        s += &format!("Show item {i} of C.\n");
    }
    let run = emit_and_execute(&s, "C");
    let ours: Vec<i64> = run.artifact_out.iter().map(|x| *x as i64).collect();
    assert_eq!(
        ours.len(),
        m * n,
        "the artifact produces the whole {m}x{n} result"
    );
    assert_eq!(
        ours, run.vm_shown,
        "the artifact on disk and the VM disagree on an integer matmul"
    );
    assert!(
        matches!(run.program.claim, NumericsClaim::Exact),
        "an integer matmul must certify as exact"
    );
}

/// ── A ROW WIDER THAN ONE ACCUMULATOR, THROUGH THE BLOCKED FORM. Qwen3.8's real `d = 5120`, EVERY element
/// of X and W filled, every element of the result compared.
///
/// ★ THE ORACLE HERE IS THE TREE-WALKER, AND THE REASON IS A MEASURED VM LIMIT. The register VM spends
/// exactly two registers per straight-line `Set` and never returns them (100 Sets → 207 registers, 4,000 →
/// 8,007), so a 10,240-element fill exhausts its 16,384-register frame before the norm is reached. The
/// tree-walker is the reference semantics the VM is itself locked against, and has no frame to exhaust.
#[test]
fn a_real_width_rmsnorm_artifact_computes_what_the_interpreter_computes() {
    let d = 5120usize;
    let mut s = String::from("## Main\n");
    s += &format!("Let X be a 1 by {d} tensor of Real.\n");
    for i in 1..=d {
        s += &format!(
            "Set item {i} of X to {:?}.\n",
            ((i * 37) % 101) as f64 / 50.0 - 1.0
        );
    }
    s += &format!("Let W be a 1 by {d} tensor of Real.\n");
    for i in 1..=d {
        s += &format!(
            "Set item {i} of W to {:?}.\n",
            0.5 + ((i * 13) % 17) as f64 / 34.0
        );
    }
    s += "Let Y be rmsnorm of X with weight W.\n";
    // Identical computation on both sides; only how the result is printed differs.
    let compiled = format!("{s}Show round(item 1 of Y * 1000000).\n");
    let observed = format!("{s}Let mutable i be 1.\nWhile i is at most {d}:\n    Show round(item i of Y * 1000000).\n    Set i to i + 1.\n");
    let run = emit_and_observe(&compiled, &observed, "Y", tw_outcome);
    assert!(
        run.program.sections.len() > 1,
        "a 5120-wide row lowers through the BLOCKED form, in several sections"
    );
    let ours: Vec<i64> = run
        .artifact_out
        .iter()
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();
    assert_eq!(ours.len(), d);
    assert_eq!(run.vm_shown.len(), d);
    assert!(
        run.vm_shown.iter().filter(|v| **v != 0).count() > d / 2,
        "the oracle's output is not mostly zeros"
    );

    // A single normalization at bf16 through four stages (square, sum-by-matmul, rsqrt, rescale).
    const BF16_RMSNORM_REL: f64 = 0.01;
    let worst = worst_relative_drift(&ours, &run.vm_shown);
    println!("rmsnorm 1x{d} from neutral.json: worst drift {worst:.5} vs the interpreter");
    assert!(
        worst <= BF16_RMSNORM_REL,
        "the artifact drifts {worst:.5} from the VM, past {BF16_RMSNORM_REL}"
    );
}

/// ── THE MODEL. The whole tiny Qwen3.8 forward — Gated DeltaNet and gated attention layers, carried
/// state, 100 tensor statements composed into one program — emitted, read back off disk, executed, and
/// compared logit for logit against the VM.
#[test]
fn the_qwen38_forward_artifact_picks_the_token_the_vm_picks() {
    let cfg = Qwen38Cfg::tiny();
    let (authored, _f, _q) = gen_qwen38_lg(&cfg, "/w");
    let mut s = String::new();
    for line in authored.lines() {
        s.push_str(line);
        s.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").expect("R by C");
        let n = r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap();
        let salt = var.bytes().map(u64::from).sum::<u64>();
        for i in 0..n {
            let h = (i as u64)
                .wrapping_mul(2_654_435_761)
                .wrapping_add(salt.wrapping_mul(40503));
            s.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                ((h % 97) as f64 / 97.0 - 0.5) * 0.4
            ));
        }
    }
    for i in 1..=cfg.vocab {
        s.push_str(&format!("Show round(item {i} of logits * 1000000).\n"));
    }

    let run = emit_and_execute(&s, "logits");
    assert!(
        run.program.sections.len() > 1,
        "the forward is one composed program of many sections"
    );
    let ours: Vec<i64> = run
        .artifact_out
        .iter()
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();
    assert_eq!(ours.len(), cfg.vocab);
    assert_eq!(run.vm_shown.len(), cfg.vocab);

    let argmax = |v: &[i64]| {
        v.iter()
            .enumerate()
            .max_by_key(|(_, x)| **x)
            .map(|(i, _)| i)
            .unwrap()
    };
    assert_eq!(
        argmax(&ours),
        argmax(&run.vm_shown),
        "the artifact picks a different token than the VM\n ours {ours:?}\n vm   {:?}",
        run.vm_shown
    );

    // Same bound, same reason, as the library-level test: a bf16 whole forward against an f64 oracle.
    const BF16_WHOLE_FORWARD_REL: f64 = 0.02;
    let worst = worst_relative_drift(&ours, &run.vm_shown);
    println!(
        "qwen38 tiny forward from neutral.json: {} sections, argmax agrees, worst drift {worst:.5}",
        run.program.sections.len()
    );
    assert!(
        worst <= BF16_WHOLE_FORWARD_REL,
        "the artifact drifts {worst:.5} from the VM, past {BF16_WHOLE_FORWARD_REL}"
    );
    if worst > 0.0 {
        assert!(
            matches!(run.program.claim, NumericsClaim::Approx { .. }),
            "the artifact drifts from the f64 oracle while claiming NumericsClaim::Exact"
        );
    }
}

/// The value the name-keyed fill gives element `i` of the tensor named `var` — the same formula the
/// whole-model source below is filled with, so any layer's weights can be produced from its NAME.
fn fill_value(var: &str, i: usize) -> f64 {
    let salt = var.bytes().map(u64::from).sum::<u64>();
    let h = (i as u64)
        .wrapping_mul(2_654_435_761)
        .wrapping_add(salt.wrapping_mul(40503));
    ((h % 97) as f64 / 97.0 - 0.5) * 0.4
}

/// ★ MUTATION PROBE (run when this landed): make the manifest bind every layer to the REPRESENTATIVE layer's
/// tensors instead of its own — the bundle still writes, parses and certifies, and this test reds.
///
/// ── THE WHOLE-MODEL BUNDLE, END TO END. `largo emit rdu --model qwen38-tiny -o DIR` writes one program per
/// layer kind, the head, and a manifest. Everything below is read back OFF DISK: each program is parsed,
/// re-certified, and required to carry the digest the manifest records; then the forward is run using ONLY
/// those programs and the manifest's per-layer tensor bindings, and the logits are compared with the VM
/// running the whole, unsplit model.
#[test]
fn the_model_bundle_on_disk_runs_the_forward_the_vm_runs() {
    let dir = tempdir().unwrap();
    let bundle = dir.path().join("qwen38");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--model",
            "qwen38-tiny",
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo emit rdu --model failed:\n{}",
        stderr(&out)
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json"),
    )
    .unwrap();
    let load = |sub: &str, digest: &serde_json::Value| -> SpatialProgram {
        let text = std::fs::read_to_string(bundle.join(sub).join("neutral.json"))
            .unwrap_or_else(|e| panic!("{sub}/neutral.json: {e}"));
        let mut p =
            parse_neutral(&text).unwrap_or_else(|e| panic!("{sub}/neutral.json parses: {e}"));
        p.certificate = None;
        let c = certify_choreography(&p)
            .unwrap_or_else(|e| panic!("{sub} certifies on its own: {e:?}"));
        assert_eq!(
            serde_json::json!(format!("{:016x}", c.schedule_digest)),
            *digest,
            "{sub}: the manifest records a different program than {sub}/neutral.json holds"
        );
        p
    };
    let kinds: Vec<SpatialProgram> = manifest["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| load(k["dir"].as_str().unwrap(), &k["schedule_digest"]))
        .collect();
    let head = load("head", &manifest["head"]["schedule_digest"]);
    let d = manifest["model"]["d"].as_u64().unwrap() as usize;

    let run = |p: &SpatialProgram,
               bind: &dyn Fn(&str) -> String,
               hidden: &(String, Vec<f64>),
               out: &str|
     -> Vec<f64> {
        let mut inputs = Inputs::default();
        for t in &p.tensors {
            if t.role == TensorRole::Output {
                continue;
            }
            let name = bind(&t.name);
            let v = if name == hidden.0 {
                hidden.1.clone()
            } else {
                (0..t.elems.rows * t.elems.cols)
                    .map(|i| fill_value(&name, i))
                    .collect()
            };
            inputs.tensors.insert(t.id, Value::Float(v));
        }
        let r = eval_choreography(p, &inputs).expect("evaluates");
        let o = p
            .tensors
            .iter()
            .find(|t| bind(&t.name) == out)
            .unwrap_or_else(|| panic!("writes `{out}`"));
        match r.tensors.get(&o.id).expect("written") {
            Value::Float(v) => v.clone(),
            Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
        }
    };

    let mut hidden = (
        "x".to_string(),
        (0..d).map(|i| fill_value("x", i)).collect::<Vec<f64>>(),
    );
    let layers = manifest["layers"].as_array().unwrap();
    for b in layers {
        let l = b["layer"].as_u64().unwrap();
        let kind = &kinds[b["kind"].as_u64().unwrap() as usize];
        let map: Vec<(String, String)> = b["tensors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t["program"].as_str().unwrap().to_string(),
                    t["layer"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        let bind = move |n: &str| {
            map.iter()
                .find(|(p, _)| p == n)
                .map(|(_, l)| l.clone())
                .unwrap_or_else(|| n.to_string())
        };
        let out = format!("xo{l}");
        hidden = (out.clone(), run(kind, &bind, &hidden, &out));
    }
    let logits = run(&head, &|n: &str| n.to_string(), &hidden, "logits");

    let cfg = Qwen38Cfg::tiny();
    let (authored, _, _) = gen_qwen38_lg(&cfg, "/w");
    let mut src = String::new();
    for line in authored.lines() {
        src.push_str(line);
        src.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").unwrap();
        for i in 0..r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap() {
            src.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                fill_value(var, i)
            ));
        }
    }
    for i in 1..=cfg.vocab {
        src.push_str(&format!("Show round(item {i} of logits * 1000000).\n"));
    }
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "{:?}", vm.error);
    let want: Vec<i64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().unwrap())
        .collect();
    let got: Vec<i64> = logits
        .iter()
        .map(|x| (x * 1_000_000.0).round() as i64)
        .collect();
    let argmax = |v: &[i64]| {
        v.iter()
            .enumerate()
            .max_by_key(|(_, x)| **x)
            .map(|(i, _)| i)
            .unwrap()
    };
    assert_eq!(
        argmax(&got),
        argmax(&want),
        "the bundle picks a different token\n ours {got:?}\n vm   {want:?}"
    );
    let worst = worst_relative_drift(&got, &want);
    println!("{} layers from the bundle on disk ({} kind programs + head): argmax agrees, worst drift {worst:.5}", layers.len(), kinds.len());
    assert!(worst <= 0.02, "the bundle drifts {worst:.5} from the VM");
}

/// ── THE DECODE BUNDLE, END TO END. `largo emit rdu --model qwen38-tiny --ctx 6 -o DIR` writes the deployment: an
/// embedding program, the layer kinds with their KV caches, a head that emits the token, and a manifest that says
/// which tensors come from weight files, which are carried state, and which two the host supplies each step.
/// Read back OFF DISK and run as that manifest says to run it — a file for every tensor it names one for, the
/// session's state for every `state` tensor, `tok` and `pos` from the host, and the program's own value for
/// everything else — one decode step must emit the token the VM's whole decode step emits.
#[test]
fn the_decode_bundle_on_disk_runs_the_step_the_vm_runs() {
    const CTX: usize = 6;
    const TOK: usize = 5;
    const POS: usize = 3;
    let dir = tempdir().unwrap();
    let bundle = dir.path().join("qwen38_decode");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--model",
            "qwen38-tiny",
            "--ctx",
            &CTX.to_string(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo emit rdu --model --ctx failed:\n{}",
        stderr(&out)
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json"),
    )
    .unwrap();
    assert_eq!(manifest["form"], "decode");
    assert_eq!(manifest["ctx"], CTX);
    assert_eq!(
        manifest["runtime_inputs"],
        serde_json::json!(["tok", "pos"])
    );
    let load = |sub: &str, digest: &serde_json::Value| -> SpatialProgram {
        let text = std::fs::read_to_string(bundle.join(sub).join("neutral.json"))
            .unwrap_or_else(|e| panic!("{sub}/neutral.json: {e}"));
        let mut p =
            parse_neutral(&text).unwrap_or_else(|e| panic!("{sub}/neutral.json parses: {e}"));
        p.certificate = None;
        let c = certify_choreography(&p)
            .unwrap_or_else(|e| panic!("{sub} certifies on its own: {e:?}"));
        assert_eq!(
            serde_json::json!(format!("{:016x}", c.schedule_digest)),
            *digest,
            "{sub}: the manifest records a different program than {sub}/neutral.json holds"
        );
        p
    };
    let embed = load("embed", &manifest["embed"]["schedule_digest"]);
    let kinds: Vec<SpatialProgram> = manifest["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| load(k["dir"].as_str().unwrap(), &k["schedule_digest"]))
        .collect();
    let head = load("head", &manifest["head"]["schedule_digest"]);

    // One program, run the way the manifest says: `entries` are its tensor bindings.
    let run = |p: &SpatialProgram,
               entries: &[serde_json::Value],
               hidden: Option<&(String, Vec<f64>)>,
               out: &str|
     -> Vec<f64> {
        let entry = |name: &str| {
            entries
                .iter()
                .find(|e| e["program"] == name)
                .unwrap_or_else(|| panic!("the manifest binds `{name}`"))
        };
        let mut inputs = Inputs::default();
        for t in &p.tensors {
            if t.role == TensorRole::Output {
                continue;
            }
            let e = entry(&t.name);
            let bound = e["layer"].as_str().unwrap();
            let n = t.elems.rows * t.elems.cols;
            let v = match bound {
                "tok" => vec![TOK as f64],
                "pos" => vec![POS as f64],
                _ if hidden.is_some_and(|h| h.0 == bound) => hidden.unwrap().1.clone(),
                _ if !e["file"].is_null() || e["role"] == "state" => {
                    (0..n).map(|i| fill_value(bound, i)).collect()
                }
                _ => continue,
            };
            inputs.tensors.insert(t.id, Value::Float(v));
        }
        let r = eval_choreography(p, &inputs).expect("evaluates");
        let o = p
            .tensors
            .iter()
            .find(|t| entry(&t.name)["layer"] == out)
            .unwrap_or_else(|| panic!("writes `{out}`"));
        match r.tensors.get(&o.id).expect("written") {
            Value::Float(v) => v.clone(),
            Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
        }
    };

    let mut hidden = (
        "x".to_string(),
        run(
            &embed,
            manifest["embed"]["tensors"].as_array().unwrap(),
            None,
            "x",
        ),
    );
    for b in manifest["layers"].as_array().unwrap() {
        let l = b["layer"].as_u64().unwrap();
        let kind = &kinds[b["kind"].as_u64().unwrap() as usize];
        let out = format!("xo{l}");
        hidden = (
            out.clone(),
            run(kind, b["tensors"].as_array().unwrap(), Some(&hidden), &out),
        );
    }
    let next = run(
        &head,
        manifest["head"]["tensors"].as_array().unwrap(),
        Some(&hidden),
        "next",
    );

    let cfg = Qwen38Cfg::tiny();
    let (step, _, _) = gen_qwen38_decode_step_lg(&cfg, "/w", CTX, TOK, POS);
    let mut src = String::new();
    for line in step.lines() {
        src.push_str(line);
        src.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").unwrap();
        for i in 0..r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap() {
            src.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                fill_value(var, i)
            ));
        }
    }
    src.push_str("Show next.\n");
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "{:?}", vm.error);
    let want: f64 = vm.output.trim().parse().expect("a token");
    assert_eq!(
        next,
        vec![want],
        "the decode bundle on disk emits a different token than the VM's decode step"
    );
    println!(
        "decode bundle on disk ({} kind programs + embed + head): token {want} agrees",
        kinds.len()
    );
}

/// ── THE BATCHED DECODE BUNDLE, END TO END. `largo emit rdu --model qwen38-tiny --ctx 6 --batch 3 -o DIR` writes the
/// batched deployment: every program over the stacked `3 × d` hidden states, each sequence with its own caches, and
/// a manifest whose runtime inputs are every sequence's token and position. Read back OFF DISK and run as that
/// manifest says, one step must emit, for every sequence, the token the VM's whole batched step emits.
#[test]
fn the_batched_decode_bundle_on_disk_runs_the_step_the_vm_runs() {
    const CTX: usize = 6;
    const SEQS: [(usize, usize); 3] = [(5, 3), (2, 1), (7, 6)];
    let dir = tempdir().unwrap();
    let bundle = dir.path().join("qwen38_batch");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--model",
            "qwen38-tiny",
            "--ctx",
            &CTX.to_string(),
            "--batch",
            &SEQS.len().to_string(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo emit rdu --model --ctx --batch failed:\n{}",
        stderr(&out)
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json"),
    )
    .unwrap();
    assert_eq!(manifest["form"], "decode");
    assert_eq!(manifest["ctx"], CTX);
    assert_eq!(manifest["batch"], SEQS.len());
    assert_eq!(
        manifest["runtime_inputs"],
        serde_json::json!(["toks", "pos_s0", "pos_s1", "pos_s2"])
    );
    let load = |sub: &str, digest: &serde_json::Value| -> SpatialProgram {
        let text = std::fs::read_to_string(bundle.join(sub).join("neutral.json"))
            .unwrap_or_else(|e| panic!("{sub}/neutral.json: {e}"));
        let mut p =
            parse_neutral(&text).unwrap_or_else(|e| panic!("{sub}/neutral.json parses: {e}"));
        p.certificate = None;
        let c = certify_choreography(&p)
            .unwrap_or_else(|e| panic!("{sub} certifies on its own: {e:?}"));
        assert_eq!(
            serde_json::json!(format!("{:016x}", c.schedule_digest)),
            *digest,
            "{sub}: the manifest records a different program than {sub}/neutral.json holds"
        );
        p
    };
    let embed = load("embed", &manifest["embed"]["schedule_digest"]);
    let kinds: Vec<SpatialProgram> = manifest["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| load(k["dir"].as_str().unwrap(), &k["schedule_digest"]))
        .collect();
    let head = load("head", &manifest["head"]["schedule_digest"]);

    let runtime = |name: &str| -> Option<Vec<f64>> {
        if name == "toks" {
            return Some(SEQS.iter().map(|(tok, _)| *tok as f64).collect());
        }
        SEQS.iter()
            .enumerate()
            .find_map(|(b, (_, pos))| (name == format!("pos_s{b}")).then(|| vec![*pos as f64]))
    };
    // One program, run the way the manifest says: `entries` are its tensor bindings.
    let run = |p: &SpatialProgram,
               entries: &[serde_json::Value],
               hidden: Option<&(String, Vec<f64>)>,
               outs: &[String]|
     -> Vec<Vec<f64>> {
        let entry = |name: &str| {
            entries
                .iter()
                .find(|e| e["program"] == name)
                .unwrap_or_else(|| panic!("the manifest binds `{name}`"))
        };
        let mut inputs = Inputs::default();
        for t in &p.tensors {
            if t.role == TensorRole::Output {
                continue;
            }
            let e = entry(&t.name);
            let bound = e["layer"].as_str().unwrap();
            let n = t.elems.rows * t.elems.cols;
            let v = match runtime(bound) {
                Some(v) => v,
                None if hidden.is_some_and(|h| h.0 == bound) => hidden.unwrap().1.clone(),
                None if !e["file"].is_null() || e["role"] == "state" => {
                    (0..n).map(|i| fill_value(bound, i)).collect()
                }
                None => continue,
            };
            inputs.tensors.insert(t.id, Value::Float(v));
        }
        let r = eval_choreography(p, &inputs).expect("evaluates");
        outs.iter()
            .map(|out| {
                let o = p
                    .tensors
                    .iter()
                    .find(|t| entry(&t.name)["layer"] == out.as_str())
                    .unwrap_or_else(|| panic!("writes `{out}`"));
                match r.tensors.get(&o.id).expect("written") {
                    Value::Float(v) => v.clone(),
                    Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
                }
            })
            .collect()
    };

    let mut hidden = (
        "X".to_string(),
        run(
            &embed,
            manifest["embed"]["tensors"].as_array().unwrap(),
            None,
            &["X".to_string()],
        )
        .remove(0),
    );
    for b in manifest["layers"].as_array().unwrap() {
        let l = b["layer"].as_u64().unwrap();
        let kind = &kinds[b["kind"].as_u64().unwrap() as usize];
        let out = format!("bxo{l}");
        hidden = (
            out.clone(),
            run(
                kind,
                b["tensors"].as_array().unwrap(),
                Some(&hidden),
                &[out],
            )
            .remove(0),
        );
    }
    let tokens: Vec<String> = (0..SEQS.len()).map(|b| format!("next_s{b}")).collect();
    let next: Vec<f64> = run(
        &head,
        manifest["head"]["tensors"].as_array().unwrap(),
        Some(&hidden),
        &tokens,
    )
    .into_iter()
    .map(|v| v[0])
    .collect();

    let cfg = Qwen38Cfg::tiny();
    let (step, _, _) = gen_qwen38_decode_batch_lg(&cfg, "/w", CTX, &SEQS);
    let mut src = String::new();
    for line in step.lines() {
        src.push_str(line);
        src.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").unwrap();
        for i in 0..r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap() {
            src.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                fill_value(var, i)
            ));
        }
    }
    for t in &tokens {
        src.push_str(&format!("Show {t}.\n"));
    }
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "{:?}", vm.error);
    let want: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("a token"))
        .collect();
    assert_eq!(
        next, want,
        "the batched decode bundle on disk emits different tokens than the VM's batched step"
    );
    println!("batched decode bundle on disk ({} sequences, {} kind programs + embed + head): tokens {want:?} agree", SEQS.len(), kinds.len());
}

#[test]
fn the_prefill_bundle_on_disk_runs_the_chunk_the_vm_runs() {
    const CTX: usize = 8;
    const TOKS: [usize; 3] = [5, 2, 7];
    const POS0: usize = 3;
    let dir = tempdir().unwrap();
    let bundle = dir.path().join("qwen38_prefill");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--model",
            "qwen38-tiny",
            "--ctx",
            &CTX.to_string(),
            "--prefill",
            &TOKS.len().to_string(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo emit rdu --model --ctx --prefill failed:\n{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("prefill, a chunk of 3 tokens of one sequence"),
        "the form is printed:\n{}",
        stderr(&out)
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json"),
    )
    .unwrap();
    assert_eq!(manifest["form"], "prefill");
    assert_eq!(manifest["chunk"], TOKS.len());
    assert_eq!(manifest["ctx"], CTX);
    assert_eq!(
        manifest["runtime_inputs"],
        serde_json::json!(["toks", "pos_s0"])
    );
    let load = |sub: &str, digest: &serde_json::Value| -> SpatialProgram {
        let text = std::fs::read_to_string(bundle.join(sub).join("neutral.json"))
            .unwrap_or_else(|e| panic!("{sub}/neutral.json: {e}"));
        let mut p =
            parse_neutral(&text).unwrap_or_else(|e| panic!("{sub}/neutral.json parses: {e}"));
        p.certificate = None;
        let c = certify_choreography(&p)
            .unwrap_or_else(|e| panic!("{sub} certifies on its own: {e:?}"));
        assert_eq!(
            serde_json::json!(format!("{:016x}", c.schedule_digest)),
            *digest,
            "{sub}: the manifest records a different program than {sub}/neutral.json holds"
        );
        p
    };
    let embed = load("embed", &manifest["embed"]["schedule_digest"]);
    let kinds: Vec<SpatialProgram> = manifest["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| load(k["dir"].as_str().unwrap(), &k["schedule_digest"]))
        .collect();
    let head = load("head", &manifest["head"]["schedule_digest"]);
    // The chunk's state is the decode step's: every state tensor a layer binds is named as the decode step names it.
    let decode_state: std::collections::BTreeSet<String> =
        gen_qwen38_decode_step_lg(&Qwen38Cfg::tiny(), "/w", CTX, 1, 1)
            .0
            .lines()
            .filter_map(|l| {
                l.strip_prefix("Let ")
                    .and_then(|r| r.split_once(" be a "))
                    .map(|(v, _)| v.to_string())
            })
            .filter(|v| {
                ["Kc", "Vc", "Csq", "Csk", "Csv", "St"].iter().any(|p| {
                    v.strip_prefix(p)
                        .is_some_and(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()))
                })
            })
            .collect();
    let bound_state: std::collections::BTreeSet<String> = manifest["layers"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|b| {
            b["tensors"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["role"] == "state")
                .map(|e| e["layer"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        bound_state.is_superset(&decode_state),
        "the prefill bundle carries the decode bundle's state, by name: missing {:?}",
        decode_state.difference(&bound_state).collect::<Vec<_>>()
    );
    assert!(
        bound_state
            .difference(&decode_state)
            .all(|v| v.starts_with('b')),
        "and beyond it only the chunk's stacks, assembled row by row: {:?}",
        bound_state.difference(&decode_state).collect::<Vec<_>>()
    );

    let runtime = |name: &str| -> Option<Vec<f64>> {
        match name {
            "toks" => Some(TOKS.iter().map(|tok| *tok as f64).collect()),
            "pos_s0" => Some(vec![POS0 as f64]),
            _ => None,
        }
    };
    let run = |p: &SpatialProgram,
               entries: &[serde_json::Value],
               hidden: Option<&(String, Vec<f64>)>,
               outs: &[String]|
     -> Vec<Vec<f64>> {
        let entry = |name: &str| {
            entries
                .iter()
                .find(|e| e["program"] == name)
                .unwrap_or_else(|| panic!("the manifest binds `{name}`"))
        };
        let mut inputs = Inputs::default();
        for t in &p.tensors {
            if t.role == TensorRole::Output {
                continue;
            }
            let e = entry(&t.name);
            let bound = e["layer"].as_str().unwrap();
            let n = t.elems.rows * t.elems.cols;
            let v = match runtime(bound) {
                Some(v) => v,
                None if hidden.is_some_and(|h| h.0 == bound) => hidden.unwrap().1.clone(),
                None if !e["file"].is_null() || e["role"] == "state" => {
                    (0..n).map(|i| fill_value(bound, i)).collect()
                }
                None => continue,
            };
            inputs.tensors.insert(t.id, Value::Float(v));
        }
        let r = eval_choreography(p, &inputs).expect("evaluates");
        outs.iter()
            .map(|out| {
                let o = p
                    .tensors
                    .iter()
                    .find(|t| entry(&t.name)["layer"] == out.as_str())
                    .unwrap_or_else(|| panic!("writes `{out}`"));
                match r.tensors.get(&o.id).expect("written") {
                    Value::Float(v) => v.clone(),
                    Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
                }
            })
            .collect()
    };
    let mut hidden = (
        "X".to_string(),
        run(
            &embed,
            manifest["embed"]["tensors"].as_array().unwrap(),
            None,
            &["X".to_string()],
        )
        .remove(0),
    );
    for b in manifest["layers"].as_array().unwrap() {
        let l = b["layer"].as_u64().unwrap();
        let kind = &kinds[b["kind"].as_u64().unwrap() as usize];
        let out = format!("bxo{l}");
        hidden = (
            out.clone(),
            run(
                kind,
                b["tensors"].as_array().unwrap(),
                Some(&hidden),
                &[out],
            )
            .remove(0),
        );
    }
    let next = run(
        &head,
        manifest["head"]["tensors"].as_array().unwrap(),
        Some(&hidden),
        &["next".to_string()],
    )
    .remove(0)[0];

    let (chunk, _, _) = gen_qwen38_prefill_lg(&Qwen38Cfg::tiny(), "/w", CTX, &TOKS, POS0);
    let mut src = String::new();
    for line in chunk.lines() {
        src.push_str(line);
        src.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").unwrap();
        for i in 0..r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap() {
            src.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                fill_value(var, i)
            ));
        }
    }
    src.push_str("Show next.\n");
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "{:?}", vm.error);
    let want: f64 = vm.output.trim().parse().expect("a token");
    assert_eq!(
        next, want,
        "the prefill bundle on disk emits a different token than the VM's chunk"
    );
    println!(
        "prefill bundle on disk ({} tokens, {} kind programs + embed + head): token {want} agrees",
        TOKS.len(),
        kinds.len()
    );
}

#[test]
fn a_prefill_that_is_not_one_sequences_prompt_chunk_is_refused() {
    let dir = tempdir().unwrap();
    let bundle = dir.path().join("refused");
    for (args, why) in [
        (vec!["--ctx", "8", "--prefill", "0"], "a chunk of no tokens"),
        (
            vec!["--ctx", "8", "--prefill", "9"],
            "longer than the 8-position KV cache",
        ),
        (
            vec!["--ctx", "8", "--prefill", "3", "--batch", "2"],
            "one sequence on one chip",
        ),
        (
            vec!["--ctx", "8", "--prefill", "3", "--tp", "2"],
            "one sequence on one chip",
        ),
        (
            vec!["--prefill", "3"],
            "fills the KV cache of a `--model --ctx` bundle",
        ),
    ] {
        let mut all = vec!["emit", "rdu", "--model", "qwen38-tiny"];
        all.extend(args.iter().copied());
        all.extend(["-o", bundle.to_str().unwrap()]);
        let out = largo_in(dir.path(), &all);
        assert_ne!(out.status.code(), Some(0), "{args:?} must be refused");
        assert!(
            stderr(&out).contains(why),
            "{args:?}: the refusal names why (`{why}`):\n{}",
            stderr(&out)
        );
        assert!(
            !bundle.join("manifest.json").exists(),
            "{args:?}: nothing is written"
        );
    }
}

/// A chip's slice, loaded the way a host loads it: flat element `i` of it (row-major in the slice's own shape) is the
/// full tensor's value at the manifest's ranges.
fn sliced(slice: &serde_json::Value, i: usize) -> f64 {
    let of = slice["of"].as_str().unwrap();
    let full_cols = slice["of_shape"][1].as_u64().unwrap() as usize;
    let ranges: Vec<(usize, usize)> = slice["ranges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r[0].as_u64().unwrap() as usize,
                r[1].as_u64().unwrap() as usize,
            )
        })
        .collect();
    let len: usize = ranges.iter().map(|(_, n)| n).sum();
    let rows_axis = slice["axis"] == "Rows";
    let cols = if rows_axis { full_cols } else { len };
    let (r, c) = (i / cols, i % cols);
    let along = |x: usize| {
        let mut at = x;
        for (start, n) in &ranges {
            if at < *n {
                return start + at;
            }
            at -= n;
        }
        panic!("past the slice")
    };
    let full = if rows_axis {
        along(r) * full_cols + c
    } else {
        r * full_cols + along(c)
    };
    fill_value(of, full)
}

/// Run a tensor-parallel decode bundle OFF DISK, the way its manifest says: every program parsed and re-certified
/// against the digest the manifest records; each chip tensor filled ONLY from its manifest slice (what a host loading
/// shards does); `runtime` the host's per-step inputs; the hidden state carried from `first` through every layer's
/// `layer_out(l)`. Returns each of `tokens` from the head.
fn run_tp_bundle_on_disk(
    bundle: &std::path::Path,
    tp: usize,
    runtime: &dyn Fn(&str) -> Option<Vec<f64>>,
    first: &str,
    layer_out: fn(u64) -> String,
    tokens: &[String],
) -> Vec<f64> {
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json"),
    )
    .unwrap();
    assert_eq!(manifest["form"], "decode");
    assert_eq!(manifest["tensor_parallel"], tp);
    let load = |sub: &str, digest: &serde_json::Value| -> SpatialProgram {
        let text = std::fs::read_to_string(bundle.join(sub).join("neutral.json"))
            .unwrap_or_else(|e| panic!("{sub}/neutral.json: {e}"));
        let mut p =
            parse_neutral(&text).unwrap_or_else(|e| panic!("{sub}/neutral.json parses: {e}"));
        p.certificate = None;
        let c = certify_choreography(&p)
            .unwrap_or_else(|e| panic!("{sub} certifies on its own: {e:?}"));
        assert_eq!(
            serde_json::json!(format!("{:016x}", c.schedule_digest)),
            *digest,
            "{sub}: the manifest records a different program than {sub}/neutral.json holds"
        );
        p
    };
    let embed = load("embed", &manifest["embed"]["schedule_digest"]);
    let kinds: Vec<SpatialProgram> = manifest["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| load(k["dir"].as_str().unwrap(), &k["schedule_digest"]))
        .collect();
    for k in &kinds {
        let chips: std::collections::BTreeSet<u16> = k
            .stages
            .iter()
            .filter_map(|s| s.site.map(|x| x.chip))
            .collect();
        assert_eq!(
            chips.len(),
            tp,
            "{}: the kind program runs over all {tp} chips",
            k.name
        );
    }
    let head = load("head", &manifest["head"]["schedule_digest"]);
    let run = |p: &SpatialProgram,
               entries: &[serde_json::Value],
               hidden: Option<&(String, Vec<f64>)>,
               outs: &[String]|
     -> Vec<Vec<f64>> {
        let entry = |name: &str| {
            entries
                .iter()
                .find(|e| e["program"] == name)
                .unwrap_or_else(|| panic!("the manifest binds `{name}`"))
        };
        let mut inputs = Inputs::default();
        for t in &p.tensors {
            if t.role == TensorRole::Output {
                continue;
            }
            let e = entry(&t.name);
            let bound = e["layer"].as_str().unwrap();
            let n = t.elems.rows * t.elems.cols;
            let v = match runtime(bound) {
                Some(v) => v,
                None if hidden.is_some_and(|h| h.0 == bound) => hidden.unwrap().1.clone(),
                None if e["slice"].is_object() => (0..n).map(|i| sliced(&e["slice"], i)).collect(),
                None if !e["file"].is_null() || e["role"] == "state" => {
                    (0..n).map(|i| fill_value(bound, i)).collect()
                }
                None => continue,
            };
            inputs.tensors.insert(t.id, Value::Float(v));
        }
        let r = eval_choreography(p, &inputs).expect("evaluates");
        outs.iter()
            .map(|out| {
                let o = p
                    .tensors
                    .iter()
                    .find(|t| entry(&t.name)["layer"] == out.as_str())
                    .unwrap_or_else(|| panic!("writes `{out}`"));
                match r.tensors.get(&o.id).expect("written") {
                    Value::Float(v) => v.clone(),
                    Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
                }
            })
            .collect()
    };
    let mut hidden = (
        first.to_string(),
        run(
            &embed,
            manifest["embed"]["tensors"].as_array().unwrap(),
            None,
            &[first.to_string()],
        )
        .remove(0),
    );
    for b in manifest["layers"].as_array().unwrap() {
        let l = b["layer"].as_u64().unwrap();
        let entries = b["tensors"].as_array().unwrap();
        for e in entries.iter().filter(|e| e["slice"].is_object()) {
            assert!(e["chip"].is_u64(), "layer {l}: a slice names its chip: {e}");
            assert!(
                e["slice"]["of"]
                    .as_str()
                    .is_some_and(|of| of.contains(&l.to_string())),
                "layer {l}: `{}` slices this layer's tensor: {e}",
                e["layer"]
            );
        }
        let out = layer_out(l);
        hidden = (
            out.clone(),
            run(
                &kinds[b["kind"].as_u64().unwrap() as usize],
                entries,
                Some(&hidden),
                &[out],
            )
            .remove(0),
        );
    }
    run(
        &head,
        manifest["head"]["tensors"].as_array().unwrap(),
        Some(&hidden),
        tokens,
    )
    .into_iter()
    .map(|v| v[0])
    .collect()
}

/// The VM's tokens for a generated step, every declared `Real` tensor filled from its name.
fn vm_tokens(step: &str, tokens: &[String]) -> Vec<f64> {
    let mut src = String::new();
    for line in step.lines() {
        src.push_str(line);
        src.push('\n');
        let Some(rest) = line.strip_prefix("Let ") else {
            continue;
        };
        let Some((var, tail)) = rest.split_once(" be a ") else {
            continue;
        };
        let Some(dims) = tail.strip_suffix(" tensor of Real.") else {
            continue;
        };
        let (r, c) = dims.split_once(" by ").unwrap();
        for i in 0..r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap() {
            src.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                fill_value(var, i)
            ));
        }
    }
    for t in tokens {
        src.push_str(&format!("Show {t}.\n"));
    }
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "{:?}", vm.error);
    vm.output
        .split_whitespace()
        .map(|t| t.parse().expect("a token"))
        .collect()
}

/// ── THE TENSOR-PARALLEL DECODE BUNDLE, END TO END. `largo emit rdu --model qwen38-mini --ctx 6 --tp 4 -o DIR` writes
/// the deployment over four chips: each layer kind one program over the chips, and a manifest that names, for every
/// chip tensor, its chip and — for a slice of a checkpoint or carried-state tensor — the full tensor, its file, the
/// axis and the ranges the chip loads. Read back OFF DISK, every chip tensor filled ONLY from what its manifest entry
/// says (the full tensor's values at the ranges — what a host loading shards does), one step emits the VM's token.
#[test]
fn the_tensor_parallel_bundle_on_disk_runs_the_step_the_vm_runs() {
    const CTX: usize = 6;
    const TOK: usize = 5;
    const POS: usize = 3;
    const TP: usize = 4;
    let dir = tempdir().unwrap();
    let bundle = dir.path().join("qwen38_tp");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--model",
            "qwen38-mini",
            "--ctx",
            &CTX.to_string(),
            "--tp",
            &TP.to_string(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo emit rdu --model --ctx --tp failed:\n{}",
        stderr(&out)
    );
    let runtime = |name: &str| match name {
        "tok" => Some(vec![TOK as f64]),
        "pos" => Some(vec![POS as f64]),
        _ => None,
    };
    let tokens = vec!["next".to_string()];
    let next = run_tp_bundle_on_disk(&bundle, TP, &runtime, "x", |l| format!("xo{l}"), &tokens);
    let want = vm_tokens(
        &gen_qwen38_decode_step_lg(&Qwen38Cfg::mini(), "/w", CTX, TOK, POS).0,
        &tokens,
    );
    assert_eq!(
        next, want,
        "the tensor-parallel bundle on disk emits a different token than the VM's unsharded step"
    );
}

/// ── THE BATCHED TENSOR-PARALLEL BUNDLE, END TO END — the serving shape: `--tp 4 --batch 3`, three sequences stacked
/// over four chips, read back off disk and loaded only through the manifest's slices, emits every sequence's token the
/// VM's unsharded batched step emits.
#[test]
fn the_batched_tensor_parallel_bundle_on_disk_runs_the_step_the_vm_runs() {
    const CTX: usize = 6;
    const TP: usize = 4;
    const SEQS: [(usize, usize); 3] = [(5, 3), (2, 1), (7, 6)];
    let dir = tempdir().unwrap();
    let bundle = dir.path().join("qwen38_tp_batch");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--model",
            "qwen38-mini",
            "--ctx",
            &CTX.to_string(),
            "--tp",
            &TP.to_string(),
            "--batch",
            &SEQS.len().to_string(),
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo emit rdu --model --ctx --tp --batch failed:\n{}",
        stderr(&out)
    );
    let runtime = |name: &str| -> Option<Vec<f64>> {
        if name == "toks" {
            return Some(SEQS.iter().map(|(tok, _)| *tok as f64).collect());
        }
        SEQS.iter()
            .enumerate()
            .find_map(|(b, (_, pos))| (name == format!("pos_s{b}")).then(|| vec![*pos as f64]))
    };
    let tokens: Vec<String> = (0..SEQS.len()).map(|b| format!("next_s{b}")).collect();
    let next = run_tp_bundle_on_disk(&bundle, TP, &runtime, "X", |l| format!("bxo{l}"), &tokens);
    let want = vm_tokens(
        &gen_qwen38_decode_batch_lg(&Qwen38Cfg::mini(), "/w", CTX, &SEQS).0,
        &tokens,
    );
    assert_eq!(next, want, "the batched tensor-parallel bundle on disk emits different tokens than the VM's batched step");
}

/// ── A WIDTH THE MODEL'S HEADS DO NOT SPLIT OVER is refused before anything lowers, naming the heads and the widths
/// that do split.
#[test]
fn a_tensor_parallel_width_the_model_does_not_split_over_is_refused() {
    let dir = tempdir().unwrap();
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--model",
            "qwen38-tiny",
            "--ctx",
            "6",
            "--tp",
            "2",
            "-o",
            dir.path().join("b").to_str().unwrap(),
        ],
    );
    assert_ne!(
        out.status.code(),
        Some(0),
        "the tiny model's single key head does not split over two chips"
    );
    let err = stderr(&out);
    assert!(
        err.contains("--tp 2") && err.contains("do not split over 2 chips"),
        "the refusal names the width and the heads: {err}"
    );
    assert!(
        err.contains("splits over 1 chip"),
        "and the widths that do split: {err}"
    );
}

/// ── THE AUTO DIALS READ THE SERVING PLAN. `--tp auto` compiles the widest width the model's heads admit in the
/// chip's scale-up domain (4 for the miniature's 8 query heads over 2 KV heads), and every decode manifest carries the
/// plan the chips and the model imply at its context — the library's own, field for field. A dial that is neither a
/// count nor `auto` is refused by name.
#[test]
fn the_auto_dials_read_the_serving_plan() {
    use logicaffeine_compile::spatial::fit::{qwen38_serving_plan, qwen38_tensor_parallel};
    use logicaffeine_compile::spatial::rdu_chip::RduChip;
    let dir = tempdir().unwrap();
    let bundle = dir.path().join("auto");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--model",
            "qwen38-mini",
            "--ctx",
            "6",
            "--tp",
            "auto",
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "largo emit rdu --tp auto failed:\n{}",
        stderr(&out)
    );
    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest.json"),
    )
    .unwrap();
    let facts = RduChip::Sn50.facts();
    let tp = qwen38_tensor_parallel(&Qwen38Cfg::mini(), &facts);
    assert_eq!(tp, 4, "the miniature's heads split over at most 4 chips");
    assert_eq!(
        manifest["tensor_parallel"], tp,
        "--tp auto compiled the plan's width"
    );
    let plan =
        serde_json::to_value(qwen38_serving_plan(&Qwen38Cfg::mini(), &facts, 6).expect("a plan"))
            .unwrap();
    assert_eq!(
        manifest["serving_plan"], plan,
        "the manifest carries the library's plan"
    );

    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            "--model",
            "qwen38-mini",
            "--ctx",
            "6",
            "--tp",
            "several",
            "-o",
            dir.path().join("x").to_str().unwrap(),
        ],
    );
    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr(&out).contains("`--tp several` is neither a count nor `auto`"),
        "{}",
        stderr(&out)
    );
}
