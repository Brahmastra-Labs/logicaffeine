//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RDU SAMBAFLOW EMIT LOCK — `largo emit rdu --format sambaflow`: the authored tensor program as the
//! PyTorch module SambaFlow ingests, with its weights manifest and a compile scaffold.
//!
//! SambaFlow takes PyTorch. So the vendor-format artifact for the SambaFlow front door is a `torch.nn.Module`
//! whose `forward` mirrors the program's tensor statements ONE OP PER STATEMENT, in dataflow order — an
//! emitted vendor-ingestion artifact exactly like an emitted `.cu`, never our compute path (the header says
//! so, and `the_emitted_python_touches_no_compute_path_of_ours` greps that nothing of ours is executed from
//! Python). Every op of the authored Qwen3.8 program has a torch spelling, and every spelling mirrors the
//! parser's desugar (the rmsnorm eps, the NEOX rope pairs with the partial-rotary pass-through, attention's
//! `1/√hd` and its `kvh = (head-1)/(H/KV)`, decay's `(1+e^u)^{alog}`, the conv ring shift, the Gated DeltaNet
//! recurrence with `kb = ((h-1) mod hk)·d`) — which is what `the_torch_module_matches_the_vm_when_torch_is_
//! present` proves numerically against the VM when torch is installed.
//!
//! ★ MUTATION PROBE: in `sambaflow.rs`, flip the sign inside `rope`'s first half (`a * c + b * s`) — the
//! rotated q·k scores change and the VM parity test goes RED (when torch is present). (Swapping the two
//! halves does NOT fire, and that is worth knowing: it is one orthogonal transform applied to both q and k,
//! so every score is invariant — a probe must change the ANSWER, not just the bytes.) Separately, make
//! `torch_spelling("decay")` an `Err` — `every_qwen38_op_has_a_torch_spelling_or_a_named_refusal` goes RED and
//! `emit_torch_module` refuses the Qwen program by name. Both run once, observed red, restored.
//!
//! The parity test is honestly tool-gated on `import torch` in the interpreter `LOGOS_PYTHON` names
//! (default `python3`): point it at an interpreter that has torch to run the numeric half, and set
//! `LOGOS_REQUIRE_TOOLS=torch` to turn an absent torch into a loud failure instead of a counted skip. The
//! shape half (one torch op per stage, the spelling table, the vendor header) runs everywhere.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_sambaflow_emit_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::collections::BTreeSet;
use std::process::Command;

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::lower::sambaflow::{
    emit_torch_module, read_tensor_sentences, torch_spelling, OpKind, TensorSentence, QWEN38_OPS,
    TEMPLATE_MARK, VENDOR_HEADER,
};

fn tiny_source() -> String {
    gen_qwen38_lg(&Qwen38Cfg::tiny(), "/w").0
}

/// The deterministic fill `qwen38_authored_lock` runs the tiny forward with: a hash of the element index
/// salted by the variable name, scaled into `[-0.2, 0.2)`.
fn fill(var: &str, i: usize) -> f64 {
    let salt = var.bytes().map(|b| b as u64).sum::<u64>();
    let h = (i as u64)
        .wrapping_mul(2654435761)
        .wrapping_add(salt.wrapping_mul(40503));
    ((h % 97) as f64 / 97.0 - 0.5) * 0.4
}

/// Every declared tensor of the authored program: `(var, rows, cols)`.
fn decls(src: &str) -> Vec<(String, usize, usize)> {
    read_tensor_sentences(src)
        .expect("the authored program reads as tensor sentences")
        .into_iter()
        .filter_map(|s| match s {
            TensorSentence::Decl {
                var, rows, cols, ..
            } => Some((var, rows, cols)),
            _ => None,
        })
        .collect()
}

/// Where `forward` binds `out`: a plain `out = …`, or the tuple `out, state = …` of a state-writing op.
fn assigned_at(model_py: &str, out: &str) -> Option<usize> {
    model_py
        .find(&format!("        {out} = "))
        .or_else(|| model_py.find(&format!("        {out}, ")))
}

/// The token in `model.py` that proves an op is spelled through its table entry: the helper or torch
/// function before `(`, or the operator of a binop spelling.
fn idiom_of(op: &str, spelling: &str) -> String {
    match op {
        "matmul" => " @ ".into(),
        "plus" => " + ".into(),
        "hadamard" => " * ".into(),
        "embedding" | "set row" => "[int(".into(),
        _ => spelling.split('(').next().unwrap().trim().to_string(),
    }
}

fn ops(src: &str) -> Vec<(String, OpKind)> {
    read_tensor_sentences(src)
        .expect("the authored program reads as tensor sentences")
        .into_iter()
        .filter_map(|s| match s {
            TensorSentence::Op { out, kind } => Some((out, kind)),
            _ => None,
        })
        .collect()
}

#[test]
fn the_stage_list_emits_one_torch_op_per_stage_in_dataflow_order() {
    let src = tiny_source();
    let stages = ops(&src);
    // The tiny hybrid: three GDN layers of 26 statements, one attention layer of 20, the final norm + head.
    assert_eq!(
        stages.len(),
        3 * 26 + 20 + 2,
        "the authored tiny forward's tensor statements"
    );
    let art = emit_torch_module(&src)
        .unwrap_or_else(|e| panic!("the authored program emits a torch module: {e}"));

    // One `# stage k: out = op` marker per statement, in program order, each followed by the assignment
    // that binds `out`.
    let markers: Vec<(usize, String, String)> = art
        .model_py
        .lines()
        .filter_map(|l| l.trim_start().strip_prefix("# stage "))
        .map(|rest| {
            let (k, tail) = rest.split_once(": ").expect("`# stage k: out = op`");
            let (out, op) = tail.split_once(" = ").expect("`out = op`");
            (k.parse::<usize>().unwrap(), out.to_string(), op.to_string())
        })
        .collect();
    assert_eq!(
        markers.len(),
        stages.len(),
        "one torch op per stage:\n{}",
        art.model_py
    );
    for (i, ((out, kind), (k, m_out, m_op))) in stages.iter().zip(&markers).enumerate() {
        assert_eq!(*k, i, "stages are numbered in dataflow order");
        assert_eq!(m_out, out, "stage {i} binds the statement's output");
        assert_eq!(m_op, kind.name(), "stage {i} names the statement's op");
        assert!(
            assigned_at(&art.model_py, out).is_some(),
            "stage {i}: `{out}` is assigned in forward:\n{}",
            art.model_py
        );
    }
    // The bindings appear in the same order as the markers (dataflow order is the emission order).
    let positions: Vec<usize> = stages
        .iter()
        .map(|(out, _)| assigned_at(&art.model_py, out).unwrap())
        .collect();
    assert!(
        positions.windows(2).all(|w| w[0] < w[1]),
        "assignments follow dataflow order"
    );
    assert!(
        art.model_py.contains("return logits"),
        "the module returns the program's last output"
    );

    // The manifest: every declared tensor with its shape and role; the GGUF name where the convention knows
    // it; the op list mirrors the stages.
    let m: serde_json::Value =
        serde_json::from_str(&art.manifest_json).expect("the manifest is JSON");
    assert_eq!(m["schema"], "logos.sambaflow.manifest");
    assert_eq!(m["output"], "logits");
    let tensors = m["tensors"].as_array().unwrap();
    let declared = decls(&src);
    assert_eq!(
        tensors.len(),
        declared.len(),
        "one manifest entry per declared tensor"
    );
    let entry = |var: &str| {
        tensors
            .iter()
            .find(|t| t["var"] == var)
            .unwrap_or_else(|| panic!("{var} in the manifest"))
    };
    for (var, r, c) in &declared {
        assert_eq!(
            entry(var)["shape"],
            serde_json::json!([r, c]),
            "{var}'s shape"
        );
    }
    assert_eq!(entry("x")["role"], "input");
    assert_eq!(entry("St0")["role"], "state");
    assert!(
        entry("St0")["gguf"].is_null(),
        "a carried state has no checkpoint tensor"
    );
    assert_eq!(entry("Csq0")["role"], "state");
    for (var, gguf) in [
        ("an0", "blk.0.attn_norm.weight"),
        ("fn2", "blk.2.post_attention_norm.weight"),
        ("Wq0", "blk.0.attn_qkv.weight"),
        ("Wz0", "blk.0.attn_gate.weight"),
        ("Wo0", "blk.0.ssm_out.weight"),
        ("Gn0", "blk.0.ssm_norm.weight"),
        ("Dt0", "blk.0.ssm_dt.bias"),
        ("Al0", "blk.0.ssm_a"),
        ("Wb0", "blk.0.ssm_beta.weight"),
        ("Wa0", "blk.0.ssm_alpha.weight"),
        ("Wcq0", "blk.0.ssm_conv1d.weight"),
        ("Wq3", "blk.3.attn_q.weight"),
        ("Wqg3", "blk.3.attn_q.weight"),
        ("Wk3", "blk.3.attn_k.weight"),
        ("Wv3", "blk.3.attn_v.weight"),
        ("Wo3", "blk.3.attn_output.weight"),
        ("Qn3", "blk.3.attn_q_norm.weight"),
        ("Kn3", "blk.3.attn_k_norm.weight"),
        ("Wg1", "blk.1.ffn_gate.weight"),
        ("Wu1", "blk.1.ffn_up.weight"),
        ("Wd1", "blk.1.ffn_down.weight"),
        ("on", "output_norm.weight"),
        ("Wout", "output.weight"),
    ] {
        assert_eq!(entry(var)["gguf"], gguf, "{var}'s checkpoint tensor");
        assert_eq!(entry(var)["role"], "weight");
    }
    let mops = m["ops"].as_array().unwrap();
    assert_eq!(mops.len(), stages.len());
    for (i, (out, kind)) in stages.iter().enumerate() {
        assert_eq!(mops[i]["out"], out.as_str());
        assert_eq!(mops[i]["op"], kind.name());
    }
    assert!(
        art.compile_py.contains(TEMPLATE_MARK),
        "the compile scaffold is marked TEMPLATE where the vendor API is unverified:\n{}",
        art.compile_py
    );
    assert!(
        art.compile_py.contains("sambaflow"),
        "the scaffold calls into the vendor SDK by name"
    );
    for (name, text) in [("model.py", &art.model_py), ("compile.py", &art.compile_py)] {
        assert!(
            text.starts_with(VENDOR_HEADER),
            "{name} opens with the vendor-ingestion header"
        );
    }
    assert!(
        art.manifest_json
            .contains("vendor-ingestion artifact emitted by LOGOS"),
        "the manifest carries the provenance too"
    );
    assert_eq!(
        emit_torch_module(&src).unwrap().model_py,
        art.model_py,
        "the emission is a function of the source"
    );
}

#[test]
fn every_qwen38_op_has_a_torch_spelling_or_a_named_refusal() {
    assert_eq!(
        QWEN38_OPS,
        &[
            "rmsnorm",
            "rope",
            "attention",
            "sigmoid",
            "hadamard",
            "causal conv",
            "silu",
            "l2norm",
            "decay",
            "gated delta",
            "swiglu",
            "matmul",
            "argmax",
            "embedding"
        ],
        "the op list the day must cover"
    );
    for op in QWEN38_OPS {
        let spelling =
            torch_spelling(op).unwrap_or_else(|e| panic!("`{op}` must have a torch spelling: {e}"));
        assert!(
            !spelling.trim().is_empty() && spelling.chars().any(|c| "(@[+*".contains(c)),
            "`{op}`'s spelling is a torch expression or helper call: {spelling}"
        );
    }
    for op in ["moe", "bogus"] {
        let refusal = torch_spelling(op).expect_err("an unspelled op is refused");
        assert!(refusal.contains(op), "the refusal names the op: {refusal}");
    }
    // The emitted module actually uses each spelling the program needs.
    let art = emit_torch_module(&tiny_source()).unwrap();
    let used: BTreeSet<&str> = ops(&tiny_source())
        .iter()
        .map(|(_, k)| k.name())
        .collect::<Vec<_>>()
        .into_iter()
        .collect();
    for op in &used {
        let spelling = torch_spelling(op).unwrap();
        let idiom = idiom_of(op, spelling);
        assert!(
            art.model_py.contains(&idiom),
            "the module spells `{op}` through `{idiom}`:\n{}",
            art.model_py
        );
    }
    for helper in [
        "def rmsnorm(",
        "def rope(",
        "def attention(",
        "def causal_conv(",
        "def l2norm(",
        "def decay(",
        "def gated_delta(",
        "def swiglu(",
    ] {
        assert!(art.model_py.contains(helper), "the module defines {helper}");
    }
    assert!(
        art.model_py.contains("torch.rsqrt("),
        "rmsnorm through rsqrt"
    );
    assert!(
        art.model_py.contains("torch.pow(1.0 + torch.exp("),
        "decay is (1+e^u)^alog, as the desugar says"
    );
    assert!(
        art.model_py.contains("torch.einsum("),
        "the GDN recurrence contracts the memory against the key"
    );
    assert!(art.model_py.contains("torch.sigmoid("), "sigmoid");
    assert!(art.model_py.contains("F.silu("), "silu");
    // A program with an op the table refuses is refused by name, not silently dropped.
    let with_moe = format!(
        "{}Let H be moe of xf with router Wout gate Wout up Wout down Wout choosing 1 of 2.\n",
        tiny_source()
    );
    match emit_torch_module(&with_moe) {
        Err(e) => assert!(e.contains("moe"), "the refusal names the op: {e}"),
        Ok(_) => panic!("an unspelled op must be refused by name"),
    }
}

/// The tiny Qwen3.8 forward in torch, from the same deterministic fills, against the VM's logits.
#[test]
fn the_torch_module_matches_the_vm_when_torch_is_present() {
    if !common::require_tool("torch") {
        return;
    }
    let cfg = Qwen38Cfg::tiny();
    let src = tiny_source();
    // The VM: fills after each declaration (as the authored lock does), then every logit.
    let mut filled = String::new();
    for line in src.lines() {
        filled.push_str(line);
        filled.push('\n');
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
        let n = r.parse::<usize>().unwrap() * c.parse::<usize>().unwrap();
        for i in 0..n {
            filled.push_str(&format!(
                "Set item {} of {var} to {:?}.\n",
                i + 1,
                fill(var, i)
            ));
        }
    }
    for i in 1..=cfg.vocab {
        filled.push_str(&format!("Show item {i} of logits.\n"));
    }
    let vm = vm_outcome(&filled);
    assert!(
        vm.error.is_none(),
        "the VM runs the filled forward: {:?}",
        vm.error
    );
    let expected: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|s| s.parse::<f64>().expect("a logit"))
        .collect();
    assert_eq!(expected.len(), cfg.vocab);

    // Torch: the same fills as raw float64 files, the module run at float64.
    let art = emit_torch_module(&src).unwrap();
    let dir = tempfile::tempdir().unwrap();
    for (var, r, c) in decls(&src) {
        let bytes: Vec<u8> = (0..r * c)
            .flat_map(|i| fill(&var, i).to_le_bytes())
            .collect();
        std::fs::write(dir.path().join(format!("{var}.f64")), bytes).unwrap();
    }
    std::fs::write(dir.path().join("model.py"), &art.model_py).unwrap();
    let out_path = dir.path().join("logits.f64");
    let run = Command::new(common::python_for_tools())
        .args([
            "model.py",
            "--weights",
            ".",
            "--dtype",
            "float64",
            "--out",
            out_path.to_str().unwrap(),
        ])
        .current_dir(dir.path())
        .output()
        .expect("python runs");
    assert!(
        run.status.success(),
        "model.py runs:\n{}\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let bytes = std::fs::read(&out_path).expect("the logits file");
    let got: Vec<f64> = bytes
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(got.len(), cfg.vocab, "one logit per vocabulary entry");
    let mut worst = 0f64;
    for (i, (g, e)) in got.iter().zip(&expected).enumerate() {
        let tol = 1e-3 * e.abs() + 1e-6;
        assert!(
            (g - e).abs() <= tol,
            "logit {i}: torch {g} vs VM {e} (tolerance {tol})"
        );
        worst = worst.max((g - e).abs() / (e.abs() + 1e-12));
    }
    println!(
        "torch ≡ VM on the tiny Qwen3.8 forward: {} logits, worst relative gap {worst:.3e}",
        got.len()
    );
}

#[test]
fn the_emitted_python_touches_no_compute_path_of_ours() {
    let art = emit_torch_module(&tiny_source()).unwrap();
    for (name, text) in [("model.py", &art.model_py), ("compile.py", &art.compile_py)] {
        assert!(
            text.starts_with(VENDOR_HEADER),
            "{name} states it is a vendor-ingestion artifact"
        );
        assert!(
            text.lines()
                .next()
                .unwrap()
                .contains("not our compute path"),
            "{name}'s first line says so"
        );
        for forbidden in [
            "subprocess",
            "largo",
            ".lg",
            "os.system",
            "logicaffeine_cli",
            "cargo ",
        ] {
            assert!(
                !text.contains(forbidden),
                "{name} must not touch `{forbidden}` — nothing of ours runs from Python:\n{text}"
            );
        }
    }
    let allowed: BTreeSet<&str> = [
        "torch",
        "torch.nn",
        "torch.nn.functional",
        "math",
        "json",
        "sys",
        "argparse",
        "os",
        "numpy",
    ]
    .into_iter()
    .collect();
    for line in art.model_py.lines() {
        let t = line.trim();
        let module = if let Some(rest) = t.strip_prefix("import ") {
            Some(rest.split_whitespace().next().unwrap())
        } else if let Some(rest) = t.strip_prefix("from ") {
            Some(rest.split_whitespace().next().unwrap())
        } else {
            None
        };
        if let Some(m) = module {
            assert!(
                allowed.contains(m),
                "model.py imports only torch and the standard library, not `{m}`"
            );
        }
    }
    assert!(
        art.model_py.contains("class LogosModule(nn.Module)"),
        "a torch module"
    );
    assert!(art.model_py.contains("def forward(self"), "with a forward");
}
