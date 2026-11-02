//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ★ QWEN3.8-27B AUTHORED IN LOGOS ★ — the whole hybrid forward as a `.lg` program.
//!
//! 48 Gated-DeltaNet layers + 16 gated-attention layers, `full_attention_interval = 4`. This lock holds the
//! authored program itself: that it PARSES, that it RUNS end to end on the execution tiers, that its shape
//! matches the checkpoint's real architecture, and that the hybrid schedule is actually hybrid.
//!
//! The tiny config is structurally faithful — same op sequence, same 3:1 schedule — so the entire authored
//! model is exercised on every tier without 29 GB of weights. The real config is checked for structure only
//! (a 64-layer forward at d=5120 is not something to run on a tree-walker).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// The real checkpoint's numbers, from `benchmarks/results/qwen38_arch.json` (read out of the GGUF header).
/// If Qwen ever reshapes the model, this is the test that notices.
#[test]
fn the_real_config_matches_the_checkpoints_architecture() {
    let c = Qwen38Cfg::real();
    assert_eq!((c.d, c.layers, c.dff, c.vocab), (5120, 64, 17408, 248320));
    assert_eq!(
        (c.h, c.kv, c.hd),
        (24, 4, 256),
        "gated attention is GQA 24/4 at head_dim 256"
    );
    assert_eq!(
        c.rope_dim, 64,
        "PARTIAL rotary — 64 of each head's 256 dims rotate"
    );
    assert_eq!(
        c.rope_theta, 1.0e7,
        "rope.freq_base is 1e7, not the 1e6 Qwen3 used"
    );
    assert_eq!(
        (c.gdn_k_heads, c.gdn_v_heads, c.gdn_head_dim),
        (16, 48, 128)
    );
    assert_eq!(c.conv_taps, 4);
    assert_eq!(c.gdn_qk(), 2048);
    assert_eq!(c.gdn_v(), 6144, "ssm.inner_size = 48 value heads x 128");
    assert_eq!(c.aq(), 6144);
    assert_eq!(c.akv(), 1024);
    // The 3:1 schedule: 16 attention layers at indices 3, 7, 11 … 63, 48 GDN layers everywhere else.
    assert_eq!(c.attn_layers(), 16, "16 of 64 layers are full attention");
    assert!(
        c.is_attn(3) && c.is_attn(63),
        "attention lands on 3, 7, … 63"
    );
    assert!(
        !c.is_attn(0) && !c.is_attn(1) && !c.is_attn(2),
        "0,1,2 are Gated DeltaNet"
    );
}

/// The authored program must contain the ops the architecture calls for, in the right multiplicities — the
/// structural proof that this really is a hybrid and not 64 copies of one block.
#[test]
fn the_authored_forward_has_the_hybrid_structure() {
    let c = Qwen38Cfg::real();
    let (src, f32w, q8w) = gen_qwen38_lg(&c, "/w");
    let count = |needle: &str| src.matches(needle).count();

    assert_eq!(count("gated delta of"), 48, "48 Gated-DeltaNet layers");
    assert_eq!(count("attention of"), 16, "16 gated-attention layers");
    assert_eq!(
        count("causal conv of"),
        48 * 3,
        "each GDN layer convolves q, k and v"
    );
    assert_eq!(count("decay of"), 48, "one forget gate per GDN layer");
    assert_eq!(count("l2norm of"), 48 * 2, "GDN L2-normalizes q and k");
    assert_eq!(count("swiglu of"), 64, "every layer has a dense SwiGLU FFN");
    assert_eq!(count("rope of"), 16 * 2, "only the attention layers rotate");
    assert_eq!(
        count("across 24 heads"),
        16 * 2,
        "per-head QK-norm and partial rope over 24 query heads"
    );
    assert_eq!(
        count("sigmoid of"),
        48 + 16,
        "GDN beta, and the attention output gate"
    );
    assert!(
        src.contains("with 48 heads 16 groups"),
        "48 value heads sharing 16 key heads"
    );
    assert!(
        src.ends_with("Let logits be xf matmul Wout.\n"),
        "ends at the LM head"
    );

    // Untied embeddings: the LM head is its own tensor, and it is the vocab-sized one.
    assert!(
        q8w.contains_key("Wout"),
        "the LM head is a quantized weight"
    );
    // Every declared weight is in exactly one manifest.
    for (name, _) in f32w.iter() {
        assert!(!q8w.contains_key(name), "{name} is in both manifests");
    }
    println!(
        "authored Qwen3.8-27B: {} lines, {} f32 tensors, {} Q8_0 tensors, 48 GDN + 16 attention layers",
        src.lines().count(), f32w.len(), q8w.len()
    );
}

/// The whole authored model, at a faithful miniature, RUNS — and the tree-walker and the VM agree on every
/// logit. This is what "Qwen3.8-27B is authored in LOGOS and runs on the backends" means concretely: not a
/// recognizer matching a pattern, but the program actually executing.
#[test]
fn the_authored_forward_runs_end_to_end_and_the_tiers_agree() {
    let c = Qwen38Cfg::tiny();
    let (authored, _f32w, _q8w) = gen_qwen38_lg(&c, "/w");

    // Fill each declared tensor with deterministic values IMMEDIATELY after its declaration — a forward over
    // zeros would exercise very little, and hoisting all the fills to the top would place a layer's FFN fills
    // above the declarations they refer to (the weights are declared per-block, interleaved with compute).
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
        let (r, cc) = dims.split_once(" by ").expect("R by C");
        let n = r.parse::<usize>().unwrap() * cc.parse::<usize>().unwrap();
        let salt = var.bytes().map(|b| b as u64).sum::<u64>();
        for i in 0..n {
            let h = (i as u64)
                .wrapping_mul(2654435761)
                .wrapping_add(salt.wrapping_mul(40503));
            let v = ((h % 97) as f64 / 97.0 - 0.5) * 0.4;
            src.push_str(&format!("Set item {} of {var} to {v:?}.\n", i + 1));
        }
    }
    src.push_str("Show round(item 1 of logits * 1000000).\n");
    src.push_str(&format!(
        "Show round(item {} of logits * 1000000).\n",
        c.vocab
    ));

    let vm = vm_outcome(&src);
    assert!(
        vm.error.is_none(),
        "the authored forward failed on the VM: {:?}",
        vm.error
    );
    let tw = tw_outcome(&src);
    assert!(
        tw.error.is_none(),
        "the authored forward failed on the tree-walker: {:?}",
        tw.error
    );
    let (a, b): (Vec<_>, Vec<_>) = (
        vm.output.split_whitespace().collect(),
        tw.output.split_whitespace().collect(),
    );
    assert_eq!(a.len(), 2, "expected two logits, got {:?}", a);
    assert_eq!(a, b, "tree-walker and VM disagree on the authored forward");
    assert!(
        a.iter().all(|s| s.parse::<f64>().is_ok()),
        "logits are not numbers: {a:?}"
    );
    println!("the authored Qwen3.8 forward ({} layers) runs on the tree-walker AND the VM, agreeing: {a:?}", c.layers);
}

/// Emit the real-shape generation program to a file. Env-gated so it never runs in a normal suite — it is the
/// hand-off to a GPU pod, where the `.cu` is compiled against the extracted weights.
///
///   LOGOS_Q38_EMIT=/tmp/gen38.cu LOGOS_Q38_WDIR=/workspace/w38 \
///   LOGOS_Q38_PROMPT=785,6722,315,9625,374 LOGOS_Q38_N=24 [LOGOS_Q38_PRECISE=1] \
///   cargo test -p logicaffeine-tests --test qwen38_authored_lock emit_qwen38_cu -- --exact --nocapture
#[test]
fn emit_qwen38_cu() {
    let Ok(out) = std::env::var("LOGOS_Q38_EMIT") else {
        eprintln!("SKIP emit_qwen38_cu — set LOGOS_Q38_EMIT to write the .cu");
        return;
    };
    let wdir = std::env::var("LOGOS_Q38_WDIR").unwrap_or_else(|_| "/workspace/w38".to_string());
    let prompt: Vec<usize> = std::env::var("LOGOS_Q38_PROMPT")
        .unwrap_or_else(|_| "785,6722,315,9625,374".to_string())
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().parse().expect("token id"))
        .collect();
    let n_gen: usize = std::env::var("LOGOS_Q38_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24);
    let precise = std::env::var("LOGOS_Q38_PRECISE").is_ok();
    let deq: u8 = std::env::var("LOGOS_Q38_DEQ")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let cfg = Qwen38Cfg::real();
    let cu = logicaffeine_compile::codegen_cuda::qwen38::emit_qwen38_generate(
        &cfg, &wdir, &prompt, n_gen, precise, deq,
    );
    std::fs::write(&out, &cu).expect("write .cu");
    println!(
        "wrote {out} — {} bytes, {} layers ({} GDN + {} attention), {} lane, prompt {:?}, {n_gen} tokens",
        cu.len(), cfg.layers, cfg.layers - cfg.attn_layers(), cfg.attn_layers(),
        if precise { "precise" } else { "fast" }, prompt
    );
    // Structural sanity before it ever reaches nvcc.
    assert!(
        cu.contains("gdn_step<<<"),
        "the GDN recurrence must be launched"
    );
    assert!(
        cu.contains("cudaGraphInstantiate"),
        "the decode step must be captured as a graph"
    );
    assert!(
        cu.contains("GEN_TOK_S_TG128"),
        "must report the tg128 metric llama-bench reports"
    );
    assert_eq!(
        cu.matches("gdn_step<<<").count(),
        cfg.layers - cfg.attn_layers(),
        "one recurrence per GDN layer"
    );
}

/// Emit the BATCHED decode program. Env-gated like its single-sequence sibling.
///
///   LOGOS_Q38B_EMIT=/tmp/b8.cu LOGOS_Q38B_BATCH=8 [LOGOS_Q38B_HL=1] \
///   cargo test -p logicaffeine-tests --test qwen38_authored_lock emit_qwen38_batched_cu -- --exact --nocapture
#[test]
fn emit_qwen38_batched_cu() {
    let Ok(out) = std::env::var("LOGOS_Q38B_EMIT") else {
        eprintln!("SKIP emit_qwen38_batched_cu — set LOGOS_Q38B_EMIT to write the .cu");
        return;
    };
    let wdir = std::env::var("LOGOS_Q38B_WDIR").unwrap_or_else(|_| "/workspace/w38".to_string());
    let batch: usize = std::env::var("LOGOS_Q38B_BATCH")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let n_gen: usize = std::env::var("LOGOS_Q38B_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(13);
    let hl = std::env::var("LOGOS_Q38B_HL").is_ok();
    let prompt: Vec<usize> = std::env::var("LOGOS_Q38B_PROMPT")
        .unwrap_or_else(|_| "760,6511,314,9338,369".to_string())
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().parse().expect("token id"))
        .collect();

    let prefill_t: usize = std::env::var("LOGOS_Q38B_PREFILL_T")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
        .max(1);

    let cfg = Qwen38Cfg::real();
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
        &cfg, &wdir, &prompt, n_gen, batch, hl, prefill_t,
    );
    std::fs::write(&out, &cu).expect("write .cu");
    println!("wrote {out} — {} bytes, batch {batch}, {} lane, {:.2} GB of per-sequence state at batch {batch}",
        cu.len(), if hl { "two-pass" } else { "int8" },
        (cfg.state_bytes_per_seq(prompt.len() + n_gen + 8) * batch) as f64 / 1e9);

    // The weight matrices are read ONCE for the whole batch — that is the entire point, so assert it rather
    // than trust it: a batched program that launched one GEMV per sequence would still be correct and would
    // have thrown away the only reason to batch.
    // Count the matmul LOWERINGS, not one particular kernel. `matmul` in the `.lg` lowers to a
    // register-tiled GEMV at small batch and to a tensor-core GEMM once the batch fills the M tile; both are
    // valid, and the invariant is that every projection gets exactly ONE of them. Asserting on `bq_gemv_h`
    // specifically made this test fail the moment the shape rule chose the GEMM — which is the test noticing
    // the right thing for the wrong reason.
    // Every lowering the shape rule can choose must be counted, INCLUDING the shared-memory staged
    // forms. Counting only the register-tiled and tensor-core names made this assertion fail the moment
    // staging became the default at batch >= 2 — the test noticing the right thing for the wrong reason,
    // for the second time.
    // The census must list EVERY lowering the shape rule can choose. It has now gone stale twice for
    // the same reason and both times the test reported 0 of 257 — `bq_gemv_wide` (the row-parallel
    // GEMV the small-batch campaign made the batch-1 form) and `bq_gemm_tc1` (the single-pass tile)
    // shipped after the list was written, so a program in which every site was correctly lowered
    // counted as a program with no matmuls at all. The invariant is unchanged: exactly one lowering
    // per site, all-or-nothing.
    const LOWERINGS: [&str; 10] = [
        "bq_gemv_hl<<<",
        "bq_gemv_h<<<",
        "bq_gemv_hl_smem<<<",
        "bq_gemv_h_smem<<<",
        "bq_gemm_mma<<<",
        "bq_gemm_mmq<<<",
        "bq_gemm_tc<<<",
        "bq_gemv_wide<<<",
        "bq_gemm_tc1<<<",
        "bq_gemm_tcm<<<",
    ];
    // With a chunk width the program carries a SECOND body, and the two lanes choose INDEPENDENTLY:
    // decode lowers 32 rows, prefill lowers B*T. At batch 32 / T 32 decode takes `bq_gemm_tc1` and
    // prefill takes the rows-in-M tile `bq_gemm_tcm`, which is correct and which a whole-file
    // "one lowering only" check would call a mixture. So the invariant is asserted PER LANE, where
    // it actually holds. Prefill has no output head — the last prompt token goes through decode_step
    // so the logits for the first generated token exist.
    // The program carries up to THREE bodies and each chooses its lowering independently: decode
    // lowers B rows, prefill lowers B*T, and the MTP draft head lowers B rows through a different
    // (single-layer) stack. Slice on the function boundaries and check the invariant per lane.
    let mut bounds: Vec<(&str, usize)> = [
        "mtp_draft",
        "mtp_verify",
        "mtp_step",
        "prefill_chunk",
        "decode_step",
    ]
    .iter()
    .filter_map(|n| cu.find(&format!("static void {n}(")).map(|i| (*n, i)))
    .collect();
    bounds.sort_by_key(|&(_, i)| i);
    let slice = |name: &str| -> &str {
        match bounds.iter().position(|&(n, _)| n == name) {
            None => "",
            Some(k) => {
                let start = bounds[k].1;
                let end = bounds.get(k + 1).map(|&(_, i)| i).unwrap_or(cu.len());
                &cu[start..end]
            }
        }
    };
    let (draft, vfy, pre, dec) = (
        slice("mtp_draft"),
        slice("mtp_verify"),
        slice("prefill_chunk"),
        slice("decode_step"),
    );
    assert_eq!(
        prefill_t > 1,
        !pre.is_empty(),
        "a chunk width of {prefill_t} must emit prefill_chunk exactly when it is greater than 1"
    );
    // The draft head is one layer's four projections plus the 2E->E adapter plus the output head.
    for (lane, body, want) in [
        ("decode", dec, 4 * cfg.layers + 1),
        (
            "prefill",
            pre,
            if pre.is_empty() { 0 } else { 4 * cfg.layers },
        ),
        ("mtp draft", draft, if draft.is_empty() { 0 } else { 6 }),
        // verify is the chunked forward plus the LM head at EVERY row: prefill
        // omits the head because it wants hidden states, speculation needs the
        // logits because that is what checks the draft.
        ("mtp verify", vfy, if vfy.is_empty() { 0 } else { 1 }),
    ] {
        let n: usize = LOWERINGS.iter().map(|k| body.matches(k).count()).sum();
        assert_eq!(n, want,
            "the {lane} lane must give every projection exactly one matmul lowering: expected {want} \
             sites, counted {n}");
        let kinds = LOWERINGS.iter().filter(|k| body.contains(**k)).count();
        assert!(kinds <= 1,
            "the {lane} lane mixes {kinds} different matmul lowerings; the choice must cover every \
             site or none of them");
    }
    let projections: usize = LOWERINGS.iter().map(|k| cu.matches(k).count()).sum();
    let sites = 4 * cfg.layers
        + 1
        + if prefill_t > 1 { 4 * cfg.layers } else { 0 }
        + if draft.is_empty() { 0 } else { 7 };
    assert_eq!(
        projections, sites,
        "got {projections} matmul lowerings of {sites} projection sites"
    );
    // Whichever lowering is chosen, it must be chosen for EVERY site — including the output head, which is
    // emitted through its own path and once stayed on the GEMV form while the other 256 moved to the GEMM.
    // Staging is likewise all-or-nothing: a program that staged the projections but left the output head
    // on the L2-re-reading form would carry the cost this fix exists to remove, at the widest matmul in
    // the model.
    let staged = cu.contains("bq_gemv_h_smem<<<") || cu.contains("bq_gemv_hl_smem<<<");
    let unstaged = cu.contains("bq_gemv_h<<<") || cu.contains("bq_gemv_hl<<<");
    assert!(
        !(staged && unstaged),
        "staged and register-tiled GEMVs both appear; the staging decision must cover every site"
    );
    assert!(
        !cu.contains("for(int b=0;b<B;++b) bq_gemv"),
        "the batch must be inside the GEMV, not a loop around it"
    );
    assert!(
        cu.contains("BATCH_AGREEMENT"),
        "the program must self-check that every sequence agrees"
    );
    assert!(
        cu.contains("AGGREGATE_TOK_S"),
        "must report aggregate as well as per-sequence throughput"
    );
    assert!(
        cu.contains("cudaGraphInstantiate"),
        "the batched step must be captured as a graph too"
    );
    for stray in ["\\x20", "\\n\\"] {
        assert!(
            !cu.contains(stray),
            "emitter escape {stray:?} leaked into the generated CUDA"
        );
    }
}

/// Emit the MTP accept-rate probe. Env-gated; the numbers come from a pod.
///
///   LOGOS_Q38M_EMIT=/tmp/mtp.cu cargo test -p logicaffeine-tests --test qwen38_authored_lock \
///     emit_qwen38_mtp_probe_cu -- --exact --nocapture
#[test]
fn emit_qwen38_mtp_probe_cu() {
    let Ok(out) = std::env::var("LOGOS_Q38M_EMIT") else {
        eprintln!("SKIP emit_qwen38_mtp_probe_cu — set LOGOS_Q38M_EMIT to write the .cu");
        return;
    };
    let wdir = std::env::var("LOGOS_Q38M_WDIR").unwrap_or_else(|_| "/workspace/w38".to_string());
    let cfg = Qwen38Cfg::real();
    const PROMPT: [usize; 5] = [760, 6511, 314, 9338, 369];
    const GOLDEN: [usize; 13] = [
        11751, 13, 198, 760, 6511, 314, 9564, 369, 19241, 13, 198, 760, 6511,
    ];
    let cu = logicaffeine_compile::codegen_cuda::qwen38::emit_qwen38_mtp_probe(
        &cfg, &wdir, &PROMPT, &GOLDEN,
    );
    std::fs::write(&out, &cu).expect("write .cu");
    println!(
        "wrote {out} — {} bytes, MTP probe over {} golden tokens",
        cu.len(),
        GOLDEN.len()
    );

    // The draft must not disturb the decode it is speculating about: its own KV cache, and an argmax that
    // does not advance the position. A draft that stepped `d_pos` would corrupt the very stream it is
    // trying to predict, and the corruption would look like a low accept rate rather than like a bug.
    assert!(cu.contains("mtp_draft"), "the draft step must be emitted");
    assert!(
        cu.contains("gk_argmax_only<<<"),
        "the draft must use the non-advancing argmax"
    );
    assert!(
        !cu.contains("gk_argmax_adv<<<1,256,0,s>>>(dlogits"),
        "the draft must not advance the position"
    );
    assert!(
        cu.contains("mkc") && cu.contains("mvc"),
        "the draft needs its OWN KV cache"
    );
    assert!(
        cu.contains("MTP_ACCEPT_RATE"),
        "must report the accept rate"
    );
    assert!(cu.contains("ehproj_q"), "must load the nextn eh_proj");
    for stray in ["\\x20", "\\n\\"] {
        assert!(
            !cu.contains(stray),
            "emitter escape {stray:?} leaked into the generated CUDA"
        );
    }
}

/// The tensor-core tile must be chosen for EVERY matmul site, and its split-K must be well formed.
///
/// `emit_qwen38_batched_cu` above carries the structural assertions but is env-gated on
/// `LOGOS_Q38B_EMIT`, so by default it SKIPS and protects nothing. That gap is not hypothetical: the
/// projection counter it owns was silently blind to `bq_gemm_tc` for as long as it took to notice,
/// and the identical class of bug — the output head left behind on the previous lowering while the
/// other 256 sites moved — has already happened once in this file's history. This test runs
/// unconditionally and needs no GPU, no weights and no nvcc: it reads the emitted text.
#[test]
fn the_tensor_core_tile_covers_every_matmul_site_at_batch_thirty_two() {
    let cfg = Qwen38Cfg::real();
    let prompt = vec![760usize, 6511, 314, 9338, 369];
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
        &cfg, "/w", &prompt, 4, 32, true,
    );

    let sites = 4 * cfg.layers + 1;
    assert_eq!(
        cu.matches("bq_gemm_tc<<<").count(),
        sites,
        "at batch 32 on the two-pass lane every projection and the output head must lower to the \
         tensor-core tile; got {} of {sites}",
        cu.matches("bq_gemm_tc<<<").count()
    );
    for stale in [
        "bq_gemv_h<<<",
        "bq_gemv_hl<<<",
        "bq_gemv_h_smem<<<",
        "bq_gemv_hl_smem<<<",
        "bq_gemm_mmq<<<",
        "bq_gemm_mma<<<",
    ] {
        assert!(!cu.contains(stale),
            "{stale} survives alongside the tensor-core tile; the lowering choice must cover every \
             matmul site, not most of them");
    }

    // Every split launch must be paired with its reduction, and the partial buffer must exist.
    let splits: Vec<usize> = cu
        .match_indices("bq_gemm_tc<<<")
        .map(|(i, _)| {
            let tail = &cu[i..];
            let args = &tail[..tail.find(");").expect("launch ends")];
            args.rsplit(',')
                .next()
                .expect("split argument")
                .trim()
                .parse()
                .expect("split is a literal")
        })
        .collect();
    assert_eq!(splits.len(), sites);
    let split_sites = splits.iter().filter(|s| **s > 1).count();
    // Summed exactly ONCE -- by a `bq_sk_reduce` launch, or by a consumer that folded the sum into
    // itself. Never zero: a split projection whose partials nobody sums reads as a plausible tensor of
    // the right shape and is entirely wrong. Never twice: that doubles the layer just as quietly.
    let folded = cu.matches("_hl_sk<<<").count();
    assert_eq!(cu.matches("bq_sk_reduce<<<").count() + folded, split_sites,
        "split-K partials must be summed exactly once: {} reduce launches + {folded} folded consumers \
         against {split_sites} split launches",
        cu.matches("bq_sk_reduce<<<").count());
    assert!(
        folded > 0,
        "the consumer-side split-K fusion emitted nothing -- the lever is silently gone"
    );
    assert_eq!(
        cu.contains("cudaMalloc(&skp"),
        split_sites > 0,
        "the split-K partial buffer must be allocated exactly when something splits"
    );
    assert!(
        splits.iter().all(|s| (1..=16).contains(s)),
        "split factors must stay in the measured band 1..=16, got {splits:?}"
    );
    assert!(split_sites > 0,
        "at the real 27B shapes the N=5120 projections are 128 of the launches and yield 40 blocks \
         of the 128-row tile; if nothing splits, the rule has stopped filling the machine");

    assert_split_buffer_covers_every_launch(&cu, &cfg);

    // The shape constants must reach the program, or the kernel silently compiles at its defaults.
    for d in ["#define TC_TJ 32", "#define TC_WM 4", "#define TC_KC 128"] {
        assert!(cu.contains(d), "the emitted program must pin {d}");
    }
}

/// The lowering chosen at each batch must be the one that MEASURED fastest.
///
/// Three lowerings of one `matmul`, and the shape picks between them. Per-step GEMM over the five
/// real projection shapes, H100 NVL, 2026-09-09, ms:
///
///     sequences   register-tiled   row-parallel   tensor-core tile
///         1           15.6             9.6             15.9
///         2           18.7            11.9             16.0
///         4           28.3            17.0             16.4
///         8           54.1            34.1             17.0
///        16          102.6            76.7             22.4
///
/// and confirmed end-to-end on the whole decode step, which is what actually decides it: at two
/// sequences the row-parallel lowering gives 93.04 tok/s against the tile's 82.51, and at four the
/// tile gives 161.75 against 143.79. So the crossover sits at four, and this test carries the table
/// rather than a threshold — if the choice function drifts away from what was measured, it fails
/// here and names the batch where it drifted.
#[test]
fn the_lowering_chosen_at_each_batch_is_the_one_that_measured_fastest() {
    let cfg = Qwen38Cfg::real();
    let prompt = vec![760usize, 6511];
    let sites = 4 * cfg.layers + 1;
    // (batch, the kernel measurement says should win, the ones that must not appear)
    const TABLE: &[(usize, &str)] = &[
        (1, "bq_gemv_wide<<<"),
        (2, "bq_gemv_wide<<<"),
        (4, "bq_gemm_tc<<<"),
        (8, "bq_gemm_tc<<<"),
        (16, "bq_gemm_tc<<<"),
        (32, "bq_gemm_tc<<<"),
        (64, "bq_gemm_tc<<<"),
    ];
    for &(batch, want) in TABLE {
        let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
            &cfg, "/w", &prompt, 2, batch, true,
        );
        assert_eq!(cu.matches(want).count(), sites,
            "at {batch} sequences the measured winner is {want}, and it must cover every one of the \
             {sites} matmul sites; got {}", cu.matches(want).count());
        for other in [
            "bq_gemv_hl<<<",
            "bq_gemv_h<<<",
            "bq_gemv_hl_smem<<<",
            "bq_gemv_h_smem<<<",
            "bq_gemm_mmq<<<",
            "bq_gemm_mma<<<",
            "bq_gemv_wide<<<",
            "bq_gemm_tc<<<",
        ] {
            if other == want {
                continue;
            }
            assert!(
                !cu.contains(other),
                "at {batch} sequences {other} appears alongside the measured winner {want}; the \
                 lowering choice must cover every matmul site, not most of them"
            );
        }
        assert_split_buffer_covers_every_launch(&cu, &cfg);
    }
}

/// The split-K partial buffer must be big enough for the WIDEST launch that writes into it.
///
/// This is the invariant behind a real corruption. `skp` is sized from a high-water mark recorded as
/// the matmul sites are emitted, and `format!` evaluates its arguments in source order — so while the
/// output head was emitted *after* the allocation argument, the head's partials were not counted. At
/// batch 32 that was invisible: the head is 1940 blocks wide and the rule leaves it unsplit. At batch
/// 16 the tile is 256 rows, the head drops to 594 blocks, the rule splits it 2 ways, and 2 x 151936
/// overtakes every projection — a 100 KB overrun that corrupted every sequence but the zeroth.
/// `BATCH_AGREEMENT` reported 0/16 with 96 token mismatches, which is exactly the job that check has.
///
/// Asserting the ordering would be asserting the fix. This asserts the PROPERTY, so any future
/// reordering, a new matmul site, or a change to the split rule is caught by arithmetic instead of by
/// a rented GPU.
fn assert_split_buffer_covers_every_launch(cu: &str, cfg: &Qwen38Cfg) {
    let resolve = |t: &str| -> usize {
        match t.trim() {
            "VOCAB" => cfg.vocab,
            "E" => cfg.d,
            o => o.parse().expect("literal N"),
        }
    };
    let needed = cu
        .match_indices("bq_gemm_tc<<<")
        .map(|(i, _)| {
            let tail = &cu[i..];
            let args: Vec<&str> = tail
                [tail.find(">>>(").expect("launch") + 4..tail.find(");").expect("launch ends")]
                .split(',')
                .collect();
            let split: usize = args
                .last()
                .expect("split")
                .trim()
                .parse()
                .expect("split literal");
            if split > 1 {
                split * resolve(args[7])
            } else {
                0
            }
        })
        .max()
        .unwrap_or(0);
    let Some(i) = cu.find("cudaMalloc(&skp,(size_t)") else {
        assert_eq!(
            needed, 0,
            "something splits but no partial buffer is allocated"
        );
        return;
    };
    let tail = &cu[i + "cudaMalloc(&skp,(size_t)".len()..];
    let got: usize = tail[..tail.find('*').expect("size")]
        .parse()
        .expect("buffer literal");
    assert!(
        got >= needed,
        "split-K partial buffer holds {got} x B floats but the widest launch writes {needed} x B. \
         Every split projection past the allocation corrupts memory the next kernel reads, which \
         shows up as a wrong token stream on some sequences and not others."
    );
}

/// The buffer invariant must hold at every batch the tile is chosen for, not just the one measured.
#[test]
fn the_split_buffer_covers_every_launch_at_every_tile_batch() {
    let cfg = Qwen38Cfg::real();
    let prompt = vec![760usize, 6511, 314, 9338, 369];
    for batch in [16usize, 32, 64] {
        let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
            &cfg, "/w", &prompt, 4, batch, true,
        );
        assert_eq!(
            cu.matches("bq_gemm_tc<<<").count(),
            4 * cfg.layers + 1,
            "batch {batch} must lower every matmul site to the tile"
        );
        assert_split_buffer_covers_every_launch(&cu, &cfg);
    }
}

/// TTFT is half of every serving benchmark, and ours is the half we have never measured: the batched
/// program prefills by replaying the T=1 decode graph once per prompt token (`qwen38.rs:513`), so a
/// 512-token prompt costs 512 full forwards before a single token is emitted. vLLM reports
/// `median_ttft_ms` by construction (`benchmarks/serve.py`); if our program does not report the same
/// quantity we cannot be compared on it at all, and the deficit stays an inference instead of a number.
///
/// This pins the REPORT, not a value — the number itself is measured on the chip, and is expected to be
/// bad until the chunked prefill lands.
#[test]
fn the_batched_program_reports_its_prefill_time_so_ttft_is_measured() {
    let cfg = Qwen38Cfg::real();
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
        &cfg,
        "/w",
        &[760, 6511, 314, 9338, 369],
        13,
        32,
        true,
    );
    assert!(cu.contains("PREFILL_MS"),
        "the batched program must report the wall time of its prefill — without it there is no TTFT to \
         compare against vLLM's median_ttft_ms");
    assert!(cu.contains("TTFT_MS"),
        "the batched program must report time-to-first-token (prefill + the first decode step), the \
         quantity vLLM's benchmark reports per request");
}

/// ★ THE EMITTED PROGRAM MUST ACTUALLY COMPILE.
///
/// Emitting is not compiling and compiling is not correctness — but emitting something that does not
/// compile is the cheapest failure of the three to catch, and until now nothing caught it. The defect
/// this pins: `mmq_tj` fell back to **2** whenever the mmq path was not the chosen lowering
/// (`qwen38_batched.rs`, `if use_mmq { .. } else { 2 }`). `bq_gemm_mmq` derives its row tile as
/// `TI = (256/(MMQ_TJ/MMQ_RJ))*MMQ_RI`, so MMQ_TJ=2 makes TI 512 and `sW[512][MMQ_KC/4+1]` needs
/// 133 KB against the 48 KB static shared budget:
///
///   ptxas error : Entry function '..bq_gemm_mmq..' uses too much shared data (0x22c50 bytes, 0xc000 max)
///
/// The kernel is never LAUNCHED by those lanes — ptxas compiles every `__global__` in the translation
/// unit regardless, so an unused kernel's defines could break the whole build. It survived only
/// because no one had ever run nvcc over an emitted lane locally.
#[test]
fn every_emitted_batched_lane_compiles_for_hopper() {
    let nvcc = std::process::Command::new("nvcc").arg("--version").output();
    if !nvcc.map(|o| o.status.success()).unwrap_or(false) {
        require_tool("nvcc", false);
        return;
    }
    let cfg = Qwen38Cfg::real();
    let prompt: Vec<usize> = (0..8).map(|i| 1000 + i * 37).collect();
    let dir = std::env::temp_dir().join(format!("logos_q38_compile_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    // One lane per LOWERING the shape rule can choose, so every branch's defines are compiled:
    // batch 1 -> GEMV, 32 -> the tensor-core tile, on both the two-pass and single-pass lanes.
    // (1, false) is here because leaving it out hid a real defect: the stage-count budget was priced
    // against the lane's own TWOPASS, but BOTH instantiations are emitted and compiled, so the batch-1
    // fast lane asked for two stages on a budget only the single-pass body fits and nvcc rejected the
    // whole program. Every (batch, lane) corner that selects a different lowering belongs here.
    // PT > 1 corners are here because the chunked-prefill lane is emitted ONLY when a chunk width is
    // asked for. It is a whole second body — widened launches, T-major buffer offsets, the chunked
    // recurrence — and with the default PT=1 none of it ever reached nvcc, so a typo in it would have
    // surfaced on rented H100 time instead of here.
    // Speculation and Q4 are corners too, and neither reached nvcc before. The Q4 tile lives behind
    // `#ifdef BQ_ENABLE_Q4`, so a typo in it compiled to nothing and no gate could see it — the same
    // dead-code shape that hid a broken `emit_decode_gemm_bandwidth_bench` for weeks. The MTP lane
    // carries `mtp_draft`/`mtp_verify`/`mtp_step` plus the runtime-T `prefill_chunk`, and it crashed
    // on hardware the first time it was ever run.
    for &(batch, hl, pt, mtp, q4) in &[
        (32usize, false, 32usize, true, false),
        (8, false, 16, true, false),
        (32, false, 1, false, true),
        (8, false, 1, false, true),
    ] {
        let cu =
            logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_q4(
                &cfg, "/w", &prompt, 4, batch, hl, pt, mtp, q4,
            );
        let tag = format!(
            "{batch}_{}_{pt}{}{}",
            if hl { "hl" } else { "fast" },
            if mtp { "_mtp" } else { "" },
            if q4 { "_q4" } else { "" }
        );
        let src = dir.join(format!("lane_{tag}.cu"));
        std::fs::write(&src, &cu).expect("write .cu");
        let out = std::process::Command::new("nvcc")
            .args(["-arch=sm_90a", "-c", "-o", "/dev/null"])
            .arg(&src)
            .output()
            .expect("run nvcc");
        assert!(
            out.status.success(),
            "the {tag} lane does not compile for sm_90a:\n{}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .filter(|l| l.contains("error"))
                .take(6)
                .collect::<Vec<_>>()
                .join("\n")
        );
        println!("[compiles for sm_90a] {tag}");
    }

    for &(batch, hl, pt) in &[
        (1usize, true, 1usize),
        (1, false, 1),
        (8, false, 1),
        (32, true, 1),
        (32, false, 1),
        (64, false, 1),
        (32, false, 32),
        (32, true, 32),
        (8, false, 16),
        (64, false, 64),
    ] {
        let cu =
            logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
                &cfg, "/w", &prompt, 4, batch, hl, pt,
            );
        let src = dir.join(format!(
            "lane_{batch}_{}_{pt}.cu",
            if hl { "hl" } else { "fast" }
        ));
        std::fs::write(&src, &cu).expect("write .cu");
        let out = std::process::Command::new("nvcc")
            .args(["-O3", "-arch=sm_90", "-c"])
            .arg(&src)
            .args(["-o", "/dev/null"])
            .output()
            .expect("run nvcc");
        assert!(out.status.success(),
            "the emitted batch={batch} {} lane at prefill width {pt} does not compile for sm_90:\n{}",
            if hl { "two-pass" } else { "single-pass" },
            String::from_utf8_lossy(&out.stderr).lines()
                .filter(|l| l.contains("error")).take(6).collect::<Vec<_>>().join("\n"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The SINGLE-PASS lane's lowering, which had no lock at all — and that is precisely how it went
/// unnoticed that it never reached the tile.
///
/// Its sibling `the_lowering_chosen_at_each_batch_is_the_one_that_measured_fastest` passes `hl: true`
/// at every batch, so the fast lane's kernel choice was never asserted anywhere. Measured on an H100
/// NVL (2026-09-19) it was taking `bq_gemv_h` at batch 8, `bq_gemm_mmq` at 32 and `bq_gemm_mma` at 64
/// — three fallbacks, never the tensor cores — and losing to the two-pass lane while doing half the
/// arithmetic. A lane with no lock on its lowering is a lane that will silently regress.
#[test]
fn the_single_pass_lane_takes_the_tensor_core_tile_at_every_batch_it_reaches() {
    let cfg = Qwen38Cfg::real();
    let prompt = vec![760usize, 6511];
    let sites = 4 * cfg.layers + 1;
    for batch in [2usize, 4, 8, 16, 32, 64] {
        let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
            &cfg, "/w", &prompt, 2, batch, false,
        );
        assert_eq!(cu.matches("bq_gemm_tc1<<<").count(), sites,
            "at {batch} sequences the single-pass lane must take the tensor-core tile at every one of \
             the {sites} matmul sites; got {}", cu.matches("bq_gemm_tc1<<<").count());
        for other in [
            "bq_gemv_hl<<<",
            "bq_gemv_h<<<",
            "bq_gemv_hl_smem<<<",
            "bq_gemv_h_smem<<<",
            "bq_gemm_mmq<<<",
            "bq_gemm_mma<<<",
            "bq_gemv_wide<<<",
            "bq_gemm_tc<<<",
        ] {
            assert!(!cu.contains(other),
                "at {batch} sequences the single-pass lane emits {other} alongside the tile — this is \
                 the exact shape of the defect the tile was unreachable behind");
        }
        assert_split_buffer_covers_every_launch(&cu, &cfg);
    }
    // Batch 1 is the one place the GEMV is right: the tile's sequence dimension is 8 wide, so at one
    // sequence it would be 7/8 masked. That is the measured crossover, not an exception to the rule.
    let cu1 = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
        &cfg, "/w", &prompt, 2, 1, false,
    );
    assert!(
        !cu1.contains("bq_gemm_tc1<<<"),
        "at one sequence the tile is 7/8 masked and the GEMV is the measured winner"
    );
}

/// ★ A STAGING DEPTH THAT SILENTLY DROPS ARITHMETIC MUST NOT COMPILE.
///
/// `NB = TC_KC/32` is integer division and the tile's mma loop runs `blk < NB`, so any TC_KC that is
/// not a whole number of Q8_0 blocks stages K-elements it never multiplies. At TC_KC=48, NB=1: 16 of
/// every 48 staged elements are dropped. It does not crash, it does not spill, and it does not
/// diverge across sequences — `BATCH_AGREEMENT` reported a perfect 32/32 while every sequence was
/// equally wrong — it simply runs a third faster and computes a different model.
///
/// Measured on an H100 NVL (2026-09-20), same lane, same weights, greedy:
///   TC_KC 32/64/128 -> tokens `8294 37138 56117 51147 32029 ...`, 583.78 / 693.97 / 677.66 tok/s
///   TC_KC 48        -> tokens `102768 107634 66394 82919 ...`,    760.29 tok/s
/// The fastest configuration in the sweep was the wrong one, and the only thing standing between that
/// number and a published benchmark was noticing that 48 is not a multiple of 32.
#[test]
fn a_staging_depth_that_drops_k_elements_is_rejected_at_compile_time() {
    let nvcc = std::process::Command::new("nvcc").arg("--version").output();
    if !nvcc.map(|o| o.status.success()).unwrap_or(false) {
        require_tool("nvcc", false);
        return;
    }
    let cfg = Qwen38Cfg::real();
    let prompt = vec![760usize, 6511];
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
        &cfg, "/w", &prompt, 2, 32, false,
    );
    assert!(
        cu.contains("#define TC_KC "),
        "the emitted lane must pin its staging depth"
    );
    // Patch ONLY the staging depth to a value that truncates, exactly as the pod sweep did.
    let bad = cu
        .lines()
        .map(|l| {
            if l.starts_with("#define TC_KC ") {
                "#define TC_KC 48".to_string()
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let dir = std::env::temp_dir().join(format!("logos_q38_kcguard_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let src = dir.join("kc48.cu");
    std::fs::write(&src, &bad).expect("write .cu");
    let out = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_90", "-c"])
        .arg(&src)
        .args(["-o", "/dev/null"])
        .output()
        .expect("run nvcc");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "TC_KC=48 COMPILED. It stages 48 K-elements and multiplies 32 of them, which is a faster \
         kernel for a different model — the tile must refuse it rather than run it"
    );
    assert!(
        err.contains("TC_KC must be a multiple of 32"),
        "TC_KC=48 was rejected, but not by the guard that explains why:\n{}",
        err.lines()
            .filter(|l| l.contains("error"))
            .take(4)
            .collect::<Vec<_>>()
            .join("\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// ★ THE ASYNC PIPELINE MUST DRAIN ON ITS LAST CHUNK.
///
/// `cp.async` groups retire in order, so while a next chunk is in flight `wait_group(TC_STAGES-1)`
/// means "the chunk I am about to read has landed". On the LAST chunk there is no next issue, and the
/// single pending group is the one filling the buffer that chunk reads — `wait_group(TC_STAGES-1)`
/// permits exactly that one to still be in flight. The read races the copy.
///
/// It is asserted on the SOURCE rather than by comparing numbers, because a race is not reliably
/// observable: `batched_equivalence_lock` ran the racy kernel and PASSED it, 19/19, twice. The race
/// only surfaced on the real 27B, where it produced first token 29858 instead of 8294 and
/// `BATCH_AGREEMENT 0/32` — at **565.87 tok/s, the fastest number in that A/B**. A wrong kernel that
/// is faster than the right one and passes the equivalence lock is the worst failure mode this
/// campaign has, and it has now happened three times (TC_KC=48, the row-overrunning swizzle, this).
#[test]
fn the_async_staged_tile_drains_its_pipeline_on_the_last_chunk() {
    let cfg = Qwen38Cfg::real();
    let prompt = vec![760usize, 6511];
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
        &cfg, "/w", &prompt, 2, 32, false,
    );
    assert!(cu.contains("cp.async.wait_group 0;"),
        "the tile has no unconditional drain: on the chunk with no successor to issue, the pending \
         group IS the one being read, so the wait depth must be 0 there and not TC_STAGES-1");
    // And the in-flight depth must still be used while a successor exists, or the pipeline is pointless.
    assert!(cu.contains("cp.async.wait_group %0;"),
        "the tile never waits at depth TC_STAGES-1, so nothing is ever actually in flight across the mma");
    let drain = cu.find("cp.async.wait_group 0;").expect("drain");
    let issue = cu
        .find("BQTC_ISSUE(k0+(TC_STAGES-1)*TC_KC")
        .expect("issue site");
    assert!(issue < drain,
        "the drain must be the ELSE of the issue, not an unconditional wait that serialises every chunk");
    // ★ An S-stage pipeline must be PRIMED with S-1 chunks, and must fetch the chunk S-1 AHEAD.
    // Priming one chunk and fetching `k0+TC_KC` is correct at S=2 and silently racy above it, because
    // `wait_group(S-1)` only guarantees the oldest of S-1 in-flight groups has landed. Measured at
    // S=3, B=32: 565.73 tok/s with BATCH_AGREEMENT 0/32 — a plausible number from buffers that had
    // not arrived. After the fix, 558.27 and 32/32, i.e. correct AND slower than S=2's 563.45.
    assert!(cu.contains("st<TC_STAGES-1 && kp<kend"),
        "the pipeline is primed with a single chunk; at TC_STAGES>2 the first iterations read buffers \
         whose copies are still in flight");
    assert!(!cu.contains("BQTC_ISSUE(k0+TC_KC,"),
        "the in-loop fetch targets the NEXT chunk rather than the one TC_STAGES-1 ahead — the same \
         expression only by coincidence at TC_STAGES=2");
}

/// Write a standalone timing harness for the rows-in-M tile, so its configuration space can be swept
/// LOCALLY in seconds instead of on a rented GPU in minutes.
///
/// This exists because the roofline model and the measurement disagree: the winning tile computes as
/// MMA-bound by 1.6x — i.e. near peak — and measures 7.3% of int8 peak. `ncu` is permission-blocked on
/// RunPod, and `nsys` reports per-kernel totals only, so the stall is invisible. An isolated bench
/// restores per-variable visibility without a profiler: one shape, one kernel, many configs.
///
///   LOGOS_TCM_BENCH=/tmp/tcm.cu cargo test -p logicaffeine-compile \
///     --test qwen38_authored_lock emit_tcm_microbench -- --exact --nocapture
#[test]
fn emit_tcm_microbench() {
    let Ok(out) = std::env::var("LOGOS_TCM_BENCH") else {
        eprintln!("SKIP emit_tcm_microbench — set LOGOS_TCM_BENCH to write the .cu");
        return;
    };
    use logicaffeine_compile::codegen_cuda::gdn::{GDN_KERNELS, QWEN38_KERNELS};
    use logicaffeine_compile::codegen_cuda::qwen38::QWEN38_EXTRA_KERNELS;
    use logicaffeine_compile::codegen_cuda::qwen38_batched::QWEN38_BATCHED_KERNELS;
    use logicaffeine_compile::codegen_cuda::qwen_emit::GEN_DEVICE_KERNELS;
    // The FFN down-projection shape: the widest K in the model, so the most K-chunks per tile.
    let (k, n, m) = (17408usize, 5120usize, 1024usize);
    let src = format!(
"#include <cstdio>\n#include <cstdlib>\n#include <cmath>\n#include <cuda_runtime.h>\n\
#include <cuda_fp16.h>\n#include <chrono>\n\
#ifndef BTG\n#define BTG 8\n#endif\n#ifndef MAXSEQ\n#define MAXSEQ 64\n#endif\n\
#ifndef BMAX\n#define BMAX 64\n#endif\n\
{gdn}\n{q38}\n{gen}\n{extra}\n{bat}\n\
int main(){{\n\
\x20 const int K={k}, N={n}, M={m};\n\
\x20 signed char *wq,*xh; __half* ws; float *xs,*y;\n\
\x20 cudaMalloc(&wq,(size_t)N*K); cudaMalloc(&xh,(size_t)M*K);\n\
\x20 cudaMalloc(&ws,(size_t)N*(K/32)*2); cudaMalloc(&xs,(size_t)M*(K/32)*4);\n\
\x20 cudaMalloc(&y,(size_t)M*N*4);\n\
\x20 cudaMemset(wq,1,(size_t)N*K); cudaMemset(xh,1,(size_t)M*K);\n\
\x20 cudaMemset(ws,0,(size_t)N*(K/32)*2); cudaMemset(xs,0,(size_t)M*(K/32)*4);\n\
\x20 const int MI_B=16*(8/(MT_NT/(8*MT_WN)))*MT_WM;   /* exactly the kernel's own MI */\n\
\x20 dim3 g((N+MT_NT-1)/MT_NT,(M+MI_B-1)/MI_B);\n\
\x20 for(int i=0;i<3;i++) bq_gemm_tcm<<<g,256>>>(y,wq,ws,xh,xs,K,N,M,N);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 if(cudaGetLastError()!=cudaSuccess){{ printf(\"CUDA_ERR\\n\"); return 1; }}\n\
\x20 auto t0=std::chrono::steady_clock::now();\n\
\x20 const int IT=20; for(int i=0;i<IT;i++) bq_gemm_tcm<<<g,256>>>(y,wq,ws,xh,xs,K,N,M,N);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 double ms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-t0).count()/IT;\n\
\x20 double tflops=2.0*M*N*K/(ms*1e-3)/1e12;\n\
\x20 printf(\"MT_NT=%d MT_WN=%d MT_WM=%d MT_KC=%d MT_STAGES=%d  %.3f ms  %.1f TFLOPS\\n\",\n\
\x20        MT_NT,MT_WN,MT_WM,MT_KC,MT_STAGES,ms,tflops);\n\
\x20 return 0;\n}}\n",
        gdn = GDN_KERNELS, q38 = QWEN38_KERNELS, gen = GEN_DEVICE_KERNELS,
        extra = QWEN38_EXTRA_KERNELS, bat = QWEN38_BATCHED_KERNELS, k = k, n = n, m = m);
    std::fs::write(&out, &src).expect("write bench .cu");
    println!("wrote {out} — {} bytes, shape K={k} N={n} M={m}", src.len());
}

/// The DECODE-shape GEMM, measured as BANDWIDTH rather than FLOPS.
///
/// At 32 sequences the step is weight-streaming, not compute: `bq_gemm_tc1` reads the whole 24.33 GB
/// weight set once per token and the arithmetic is 32 rows deep. The H100 baseline put it at 728 GB/s
/// of ~3350 available — 22% of bandwidth, which IS the 2.13x decode gap against vLLM. TFLOPS is the
/// wrong unit for this shape and reporting it hid the problem; GB/s of weights is the number that
/// moves when the tile gets better.
#[test]
fn emit_decode_gemm_bandwidth_bench() {
    let Ok(out) = std::env::var("LOGOS_DEC_BENCH") else {
        eprintln!("SKIP emit_decode_gemm_bandwidth_bench — set LOGOS_DEC_BENCH to write the .cu");
        return;
    };
    use logicaffeine_compile::codegen_cuda::gdn::{GDN_KERNELS, QWEN38_KERNELS};
    use logicaffeine_compile::codegen_cuda::qwen38::QWEN38_EXTRA_KERNELS;
    use logicaffeine_compile::codegen_cuda::qwen38_batched::QWEN38_BATCHED_KERNELS;
    use logicaffeine_compile::codegen_cuda::qwen_emit::GEN_DEVICE_KERNELS;
    let (k, n) = (17408usize, 5120usize); // FFN down: the widest K, 89 MB of int8 weights
    let src = format!(
"#include <cstdio>\n#include <cstdlib>\n#include <cmath>\n#include <cuda_runtime.h>\n\
#include <cuda_fp16.h>\n#include <chrono>\n\
#ifndef BTG\n#define BTG 8\n#endif\n#ifndef MAXSEQ\n#define MAXSEQ 64\n#endif\n\
#ifndef BMAX\n#define BMAX 64\n#endif\n\
{gdn}\n{q38}\n{gen}\n{extra}\n{bat}\n\
__global__ void bq_bw_ceiling(const int4* __restrict__ p, long n, int4* sink){{\n\
\x20 int4 a=make_int4(0,0,0,0);\n\
\x20 for(long i=(long)blockIdx.x*blockDim.x+threadIdx.x;i<n;i+=(long)gridDim.x*blockDim.x){{\n\
\x20   int4 v=p[i]; a.x^=v.x; a.y^=v.y; a.z^=v.z; a.w^=v.w; }}\n\
\x20 if(a.x==0x7fffffff) *sink=a;\n\
}}\n\
int main(){{\n\
\x20 const int K={k}, N={n};\n\
\x20 signed char *wq,*xh; __half* ws; float *xs,*y,*pbuf;\n\
\x20 cudaMalloc(&wq,(size_t)N*K); cudaMalloc(&xh,(size_t)64*K);\n\
\x20 cudaMalloc(&ws,(size_t)N*(K/32)*2); cudaMalloc(&xs,(size_t)64*(K/32)*4);\n\
\x20 cudaMalloc(&y,(size_t)64*N*4); cudaMalloc(&pbuf,(size_t)16*N*64*4);\n\
\x20 cudaMemset(wq,1,(size_t)N*K); cudaMemset(xh,1,(size_t)64*K);\n\
\x20 cudaMemset(ws,0,(size_t)N*(K/32)*2); cudaMemset(xs,0,(size_t)64*(K/32)*4);\n\
\x20 double wgb=(double)N*K/1e9;\n\
#ifdef BQ_ENABLE_Q4\n\
\x20 signed char* wq4; cudaMalloc(&wq4,(size_t)N*K/2); cudaMemset(wq4,0x11,(size_t)N*K/2);\n\
#endif\n\
\x20 /* SELF-CALIBRATE. A bare GB/s says nothing without this chip's ACHIEVABLE ceiling, and that\n\
\x20    ceiling is not the spec sheet -- it is what a pure wide stream actually reaches on this part.\n\
\x20    The ratio is the difference between a number and a diagnosis: 155 GB/s tells you nothing,\n\
\x20    45%% of what the chip can do tells you whether to fix the tile or accept the chip. */\n\
\x20 double ceil_gbs;\n\
\x20 {{ long cb=1L<<28; int4 *cp,*cs; cudaMalloc(&cp,cb); cudaMalloc(&cs,16); cudaMemset(cp,1,cb);\n\
\x20    long cn=cb/16;\n\
\x20    for(int i=0;i<3;i++) bq_bw_ceiling<<<1024,256>>>(cp,cn,cs);\n\
\x20    cudaDeviceSynchronize();\n\
\x20    auto c0=std::chrono::steady_clock::now();\n\
\x20    for(int i=0;i<20;i++) bq_bw_ceiling<<<1024,256>>>(cp,cn,cs);\n\
\x20    cudaDeviceSynchronize();\n\
\x20    double cms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-c0).count();\n\
\x20    ceil_gbs=20.0*cb/(cms*1e-3)/1e9; cudaFree(cp); cudaFree(cs); }}\n\
\x20 printf(\"  pure-stream ceiling on this chip: %.0f GB/s\\n\", ceil_gbs);\n\
\x20 printf(\"TC_TJ=%d TC_WM=%d TC_KC=%d TC_STAGES=%d, weights %.3f GB int8 (split-K as shipped)\\n\",TC_TJ,TC_WM,TC_KC,TC_STAGES,wgb);\n\
\x20 for(int B=1;B<=64;B*=2){{\n\
\x20   if(B>TC_TJ) continue;\n\
\x20   const int TI=16*(8/(TC_TJ/8))*TC_WM;\n\
\x20   /* The shipped split rule, not SPLIT=1. Without it this bench measures a configuration the \n\
\x20      program never launches: at N=5120,TI=128 an unsplit grid is 40 blocks, which is starved on \n\
\x20      any real chip, and the bench then reports a tile-height \"win\" that is only TI compensating \n\
\x20      for the missing split. Measured that way it claimed +23.8% for TC_WM=1; with split-K the \n\
\x20      shipped height is fine. An instrument must run the shipped shape. */\n\
\x20   int base=(N+TI-1)/TI, cap=(K/TC_KC)/2; if(cap<1) cap=1;\n\
\x20   int SPLIT=(1024+base-1)/base; if(SPLIT<1) SPLIT=1; if(SPLIT>16) SPLIT=16; if(SPLIT>cap) SPLIT=cap;\n\
\x20   dim3 g(base,SPLIT,(B+TC_TJ-1)/TC_TJ);\n\
\x20   for(int i=0;i<3;i++) bq_gemm_tc1<<<g,256>>>(y,wq,ws,xh,xs,K,N,B,N,pbuf,SPLIT);\n\
\x20     if(SPLIT>1) bq_sk_reduce<<<((long)N*B+255)/256,256>>>(y,pbuf,N,B,N,SPLIT);\n\
\x20   cudaDeviceSynchronize();\n\
\x20   if(cudaGetLastError()!=cudaSuccess){{ printf(\"  B=%d CUDA_ERR\\n\",B); continue; }}\n\
\x20   auto t0=std::chrono::steady_clock::now();\n\
\x20   const int IT=20; for(int i=0;i<IT;i++) bq_gemm_tc1<<<g,256>>>(y,wq,ws,xh,xs,K,N,B,N,pbuf,SPLIT);\n\
\x20     if(SPLIT>1) bq_sk_reduce<<<((long)N*B+255)/256,256>>>(y,pbuf,N,B,N,SPLIT);\n\
\x20   cudaDeviceSynchronize();\n\
\x20   double ms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-t0).count()/IT;\n\
\x20   printf(\"  B=%2d int8: %.3f ms  %.0f GB/s  %.0f%% of ceiling  %.1f TFLOPS\\n\",\n\
\x20          B,ms,wgb/(ms*1e-3),100.0*(wgb/(ms*1e-3))/ceil_gbs,2.0*B*N*K/(ms*1e-3)/1e12);\n\
#ifdef BQ_ENABLE_Q4\n\
\x20   /* Decode is bound by the WEIGHT STREAM, so the one lever with chip-independent arithmetic\n\
\x20      behind it is moving fewer bytes, and Q4 halves them. The standing 0.84x verdict was taken\n\
\x20      before `fits` priced Q4's packed staging buffer and before TC_KC moved 128 -> 64 -- i.e.\n\
\x20      on a tile that cannot have been the shipped one. Measure it beside int8, same split. */\n\
\x20   for(int i=0;i<3;i++) bq_gemm_tc1_q4<<<g,256>>>(y,wq4,ws,xh,xs,K,N,B,N,pbuf,SPLIT);\n\
\x20     if(SPLIT>1) bq_sk_reduce<<<((long)N*B+255)/256,256>>>(y,pbuf,N,B,N,SPLIT);\n\
\x20   cudaDeviceSynchronize();\n\
\x20   if(cudaGetLastError()==cudaSuccess){{\n\
\x20     auto q0=std::chrono::steady_clock::now();\n\
\x20     for(int i=0;i<IT;i++) bq_gemm_tc1_q4<<<g,256>>>(y,wq4,ws,xh,xs,K,N,B,N,pbuf,SPLIT);\n\
\x20       if(SPLIT>1) bq_sk_reduce<<<((long)N*B+255)/256,256>>>(y,pbuf,N,B,N,SPLIT);\n\
\x20     cudaDeviceSynchronize();\n\
\x20     double qms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-q0).count()/IT;\n\
\x20     printf(\"  B=%2d   q4: %.3f ms  %.0f GB/s  %.0f%% of ceiling  -> %.2fx int8\\n\",\n\
\x20            B,qms,(wgb/2)/(qms*1e-3),100.0*((wgb/2)/(qms*1e-3))/ceil_gbs,ms/qms);\n\
\x20   }} else printf(\"  B=%2d   q4: CUDA_ERR\\n\",B);\n\
#endif\n\
\x20 }}\n\
\x20 return 0;\n}}\n",
        gdn = GDN_KERNELS, q38 = QWEN38_KERNELS, gen = GEN_DEVICE_KERNELS,
        extra = QWEN38_EXTRA_KERNELS, bat = QWEN38_BATCHED_KERNELS, k = k, n = n);
    std::fs::write(&out, &src).expect("write bench .cu");
    println!(
        "wrote {out} — decode GEMM K={k} N={n}, weights {:.2} GB",
        (n * k) as f64 / 1e9
    );
}

/// Time the GDN recurrence kernel in isolation, at the real Qwen3.8-27B dimensions.
///
/// Two models of where prefill time goes disagreed — one put the sequential recurrence at 46% of it,
/// the other at 71-85% — and the difference decides whether the next build is the chunked delta rule
/// or something else. `bq_gdn_step` carries only recurrent STATE (no weights), so unlike the full
/// model it fits a 12 GB card and can be measured for free.
///
///   LOGOS_GDN_BENCH=/tmp/gdn.cu cargo test -p logicaffeine-compile \
///     --test qwen38_authored_lock emit_gdn_microbench -- --exact --nocapture
#[test]
fn emit_gdn_microbench() {
    let Ok(out) = std::env::var("LOGOS_GDN_BENCH") else {
        eprintln!("SKIP emit_gdn_microbench — set LOGOS_GDN_BENCH to write the .cu");
        return;
    };
    use logicaffeine_compile::codegen_cuda::gdn::{GDN_KERNELS, QWEN38_KERNELS};
    use logicaffeine_compile::codegen_cuda::qwen38::QWEN38_EXTRA_KERNELS;
    use logicaffeine_compile::codegen_cuda::qwen38_batched::QWEN38_BATCHED_KERNELS;
    use logicaffeine_compile::codegen_cuda::qwen_emit::GEN_DEVICE_KERNELS;
    let c = Qwen38Cfg::real();
    let (nk, nv, ghd) = (c.gdn_k_heads, c.gdn_v_heads, c.gdn_head_dim);
    let (qk, vw) = (c.gdn_qk(), c.gdn_v());
    let convc = 2 * qk + vw;
    let src = format!(
"#include <cstdio>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n#include <chrono>\n\
#ifndef BTG\n#define BTG 8\n#endif\n#ifndef MAXSEQ\n#define MAXSEQ 64\n#endif\n\
#ifndef BMAX\n#define BMAX 64\n#endif\n\
{gdn}\n{q38}\n{gen}\n{extra}\n{bat}\n\
int main(){{\n\
\x20 const int BB=32, NK={nk}, NV={nv}, D={ghd}, CONVC={convc};\n\
\x20 float *cbuf,*bet,*alp,*st,*obuf;\n\
\x20 cudaMalloc(&cbuf,(size_t)BB*CONVC*4); cudaMalloc(&bet,(size_t)BB*NV*4);\n\
\x20 cudaMalloc(&alp,(size_t)BB*NV*4); cudaMalloc(&obuf,(size_t)BB*NV*D*4);\n\
\x20 cudaMalloc(&st,(size_t)BB*NV*D*D*4);\n\
\x20 cudaMemset(cbuf,0,(size_t)BB*CONVC*4); cudaMemset(st,0,(size_t)BB*NV*D*D*4);\n\
\x20 cudaMemset(bet,0,(size_t)BB*NV*4); cudaMemset(alp,0,(size_t)BB*NV*4);\n\
\x20 dim3 g(NV,(D+3)/4,BB);\n\
\x20 for(int i=0;i<3;i++) bq_gdn_step<<<g,128>>>(cbuf,cbuf+{qk},cbuf+{two_qk},bet,alp,st,obuf,D,NK,NV,CONVC);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 if(cudaGetLastError()!=cudaSuccess){{ printf(\"CUDA_ERR\\n\"); return 1; }}\n\
\x20 auto t0=std::chrono::steady_clock::now();\n\
\x20 const int IT=50; for(int i=0;i<IT;i++) bq_gdn_step<<<g,128>>>(cbuf,cbuf+{qk},cbuf+{two_qk},bet,alp,st,obuf,D,NK,NV,CONVC);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 double ms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-t0).count()/IT;\n\
\x20 double gb=(double)BB*NV*D*D*4*2/1e9;\n\
\x20 printf(\"bq_gdn_step B=%d: %.4f ms/layer/token, state %.2f GB r+w, %.0f GB/s\\n\",BB,ms,gb,gb/(ms*1e-3));\n\
\x20 printf(\"  48 GDN layers x 512 tokens => %.1f s of prefill\\n\", ms*48*512/1000.0);\n\
\x20 for(int i=0;i<3;i++) bq_gdn_chunk<<<dim3(NV,BQ_GDN_CHUNK_GY(D),BB),128>>>(cbuf,cbuf+{qk},cbuf+{two_qk},bet,alp,st,obuf,D,NK,NV,CONVC,1,BB);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 auto e0=std::chrono::steady_clock::now();\n\
\x20 for(int i=0;i<IT;i++) bq_gdn_chunk<<<dim3(NV,BQ_GDN_CHUNK_GY(D),BB),128>>>(cbuf,cbuf+{qk},cbuf+{two_qk},bet,alp,st,obuf,D,NK,NV,CONVC,1,BB);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 double ems=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-e0).count()/IT;\n\
\x20 printf(\"bq_gdn_chunk T=1 (DECODE):  %.4f ms/layer/token  => %.2fx vs bq_gdn_step\\n\",ems,ms/ems);\n\
\x20 const int T=32;\n\
\x20 float *cbT,*beT,*alT,*obT;\n\
\x20 cudaMalloc(&cbT,(size_t)T*BB*CONVC*4); cudaMalloc(&beT,(size_t)T*BB*NV*4);\n\
\x20 cudaMalloc(&alT,(size_t)T*BB*NV*4); cudaMalloc(&obT,(size_t)T*BB*NV*D*4);\n\
\x20 cudaMemset(cbT,0,(size_t)T*BB*CONVC*4); cudaMemset(beT,0,(size_t)T*BB*NV*4); cudaMemset(alT,0,(size_t)T*BB*NV*4);\n\
\x20 for(int i=0;i<2;i++) bq_gdn_chunk<<<dim3(NV,BQ_GDN_CHUNK_GY(D),BB),128>>>(cbT,cbT+{qk},cbT+{two_qk},beT,alT,st,obT,D,NK,NV,CONVC,T,BB);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 if(cudaGetLastError()!=cudaSuccess){{ printf(\"CUDA_ERR_CHUNK\\n\"); return 1; }}\n\
\x20 auto c0=std::chrono::steady_clock::now();\n\
\x20 const int CI=5; for(int i=0;i<CI;i++) bq_gdn_chunk<<<dim3(NV,BQ_GDN_CHUNK_GY(D),BB),128>>>(cbT,cbT+{qk},cbT+{two_qk},beT,alT,st,obT,D,NK,NV,CONVC,T,BB);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 double cms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-c0).count()/CI;\n\
\x20 printf(\"bq_gdn_chunk T=%d:  %.4f ms/layer/token  => %.2fx faster\\n\",T,cms/T,ms/(cms/T));\n\
\x20 printf(\"  48 GDN layers x 512 tokens => %.1f s of prefill\\n\", (cms/T)*48*512/1000.0);\n\
\x20 for(int i=0;i<2;i++) bq_gdn_chunkS<<<dim3(NV,BQ_GDN_CHUNK_GY(D),BB),128,(size_t)2*D*4>>>(cbT,cbT+{qk},cbT+{two_qk},beT,alT,st,obT,D,NK,NV,CONVC,T,BB);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 auto d0=std::chrono::steady_clock::now();\n\
\x20 for(int i=0;i<CI;i++) bq_gdn_chunkS<<<dim3(NV,BQ_GDN_CHUNK_GY(D),BB),128,(size_t)2*D*4>>>(cbT,cbT+{qk},cbT+{two_qk},beT,alT,st,obT,D,NK,NV,CONVC,T,BB);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 double dms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-d0).count()/CI;\n\
\x20 printf(\"bq_gdn_chunkS T=%d: %.4f ms/layer/token  => %.2fx faster (shared k/q, 1024 thr)\\n\",T,dms/T,ms/(dms/T));\n\
\x20 /* the chunked DELTA RULE: parallel across the chunk instead of scanning it */\n\
\x20 size_t dshm=(size_t)(2*T*T+T+2*T*D)*4;\n\
\x20 for(int i=0;i<2;i++) bq_gdn_delta<<<dim3(NV,BB,1),128,dshm>>>(cbT,cbT+{qk},cbT+{two_qk},beT,alT,st,obT,D,NK,NV,CONVC,T,BB);\n\
\x20 cudaDeviceSynchronize();\n\
\x20 if(cudaGetLastError()!=cudaSuccess){{ printf(\"CUDA_ERR_DELTA %s\\n\",cudaGetErrorString(cudaGetLastError())); }}\n\
\x20 else {{\n\
\x20   auto x0=std::chrono::steady_clock::now();\n\
\x20   for(int i=0;i<CI;i++) bq_gdn_delta<<<dim3(NV,BB,1),128,dshm>>>(cbT,cbT+{qk},cbT+{two_qk},beT,alT,st,obT,D,NK,NV,CONVC,T,BB);\n\
\x20   cudaDeviceSynchronize();\n\
\x20   double xms=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-x0).count()/CI;\n\
\x20   printf(\"bq_gdn_delta T=%d:  %.4f ms/layer/token  => %.2fx vs step, %.2fx vs chunk\\n\",T,xms/T,ms/(xms/T),(cms/T)/(xms/T));\n\
\x20   printf(\"  48 GDN layers x 512 tokens => %.1f s of prefill\\n\", (xms/T)*48*512/1000.0);\n\
\x20 }}\n\
\x20 return 0;\n}}\n",
        gdn = GDN_KERNELS, q38 = QWEN38_KERNELS, gen = GEN_DEVICE_KERNELS,
        extra = QWEN38_EXTRA_KERNELS, bat = QWEN38_BATCHED_KERNELS,
        nk = nk, nv = nv, ghd = ghd, convc = convc, qk = qk, two_qk = 2 * qk);
    std::fs::write(&out, &src).expect("write bench .cu");
    println!(
        "wrote {out} — NV={nv} D={ghd}, state {:.2} GB at B=32",
        (32.0 * nv as f64 * ghd as f64 * ghd as f64 * 4.0) / 1e9
    );
}

/// Every GDN layer must advance its recurrence with `bq_gdn_chunk`, never the per-token `bq_gdn_step`.
///
/// `bq_gdn_step` re-streams the whole 100 MB (at 32 sequences) recurrent state through HBM once per
/// token per layer: 4.95 TB of traffic for a 512-token prefill, at an arithmetic intensity of
/// 1 FLOP/byte. Measured on a 3060 it runs at 254 GB/s — 71% of bandwidth, so the kernel is fine and
/// the ROUND TRIP is the defect. `bq_gdn_chunk` keeps the state in registers across the chunk:
/// 0.7942 -> 0.1811 ms/layer/token (4.38x) at T=32, and 0.6117 (1.30x) even at T=1, where the win is
/// purely that templating on PER=D/32 stops `sreg[]` spilling to local memory.
///
/// The decode lane calls it with T=1, which the equivalence lock proves is bit-identical to the step
/// it replaces. Both lanes are asserted here because the prefill rewrite is the one that matters and
/// the decode one is the one that would be easy to leave behind.
#[test]
fn every_gdn_layer_advances_its_state_with_the_chunked_recurrence() {
    let cfg = Qwen38Cfg::real();
    let prompt = vec![760usize, 6511];
    let gdn_layers = cfg.layers - cfg.layers / 4;
    for batch in [1usize, 8, 32, 64] {
        let cu =
            logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
                &cfg, "/w", &prompt, 2, batch, false, 32,
            );
        assert!(!cu.contains("bq_gdn_step<<<"),
            "at {batch} sequences the program still launches the per-token bq_gdn_step — that kernel \
             re-reads and rewrites the entire recurrent state once per token, which is 4.95 TB of HBM \
             traffic across a 512-token prefill and the single largest item in it");
        let n = cu.matches("bq_gdn_chunk<<<").count();
        assert_eq!(
            n,
            2 * gdn_layers,
            "expected bq_gdn_chunk at all {gdn_layers} GDN layers in BOTH the decode and prefill \
             lanes ({} launches); got {n}",
            2 * gdn_layers
        );
        // `prefill_chunk` takes the chunk length at RUNTIME (`PTv`) so prefill can stay wide while a
        // speculative verify passes 2. With it baked, MTP could only run at PT=2 and paid 4.4 s of extra
        // prefill (16.04 vs 11.66 s) for a 6.9% decode gain — a net loss. The property is unchanged:
        // the recurrence advances the WHOLE chunk in one launch per layer, never one token at a time.
        assert_eq!(
            cu.matches(",PTv,B);").count(),
            gdn_layers,
            "the prefill lane must advance the whole chunk in one launch, not one token at a time"
        );
        assert_eq!(cu.matches(",1,B);").count(), gdn_layers,
            "the decode lane must call the same kernel with T=1 — the form the equivalence lock proves \
             bit-identical to bq_gdn_step");
    }
}

/// The chunked recurrence must keep its state in REGISTERS, which is only true because `PER = D/32`
/// reaches the body as a template argument.
///
/// This is the trap the whole rewrite exists to avoid, and it is invisible at the call site: with a
/// runtime `b < D` bound ptxas cannot prove the trip count, `sreg[]` is allocated in local memory —
/// which is HBM — and the kernel that was supposed to make the state resident quietly does exactly
/// what it replaced, only with more bookkeeping. `bq_gdn_step` still carries a 32-byte stack frame
/// for precisely this reason. The guard is structural: the body must be a template, the loops over
/// the state must be `#pragma unroll`, and the dispatcher must cover the head dims the model uses.
#[test]
fn the_chunked_recurrence_templates_its_state_loops_so_they_cannot_spill() {
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
        &Qwen38Cfg::real(),
        "/w",
        &[760usize, 6511],
        2,
        32,
        false,
    );
    assert!(
        cu.contains(
            "template<int PER, bool SHK>\n__device__ __forceinline__ void bq_gdn_chunk_body("
        ),
        "bq_gdn_chunk_body must be templated on PER — a runtime loop bound spills sreg[] to local \
         memory and silently undoes the residency the kernel exists for"
    );
    assert!(
        cu.contains("float sreg[SS*R];"),
        "the state array must be sized from the template arguments, not a runtime head dim"
    );
    let body = &cu[cu.find("void bq_gdn_chunk_body(").expect("body")
        ..cu.find("#define BQ_GDN_CHUNK_DISPATCH")
            .expect("dispatcher")];
    assert!(
        body.matches("#pragma unroll").count() >= 6,
        "every loop over the resident state must be unrolled; found only {} unroll pragmas",
        body.matches("#pragma unroll").count()
    );
    for per in [1usize, 2, 4, 8] {
        assert!(cu.contains(&format!("(D>>5)=={per}) bq_gdn_chunk_body<{per},SHK>")),
            "the dispatcher must instantiate PER={per}; a head dim that falls through to the generic \
             PER=0 body gets the spilling version back");
    }
    assert!(
        cu.contains("bq_gdn_chunk_body<0,SHK>"),
        "a head dim that is not a multiple of 32 still needs a correct (if slower) generic path"
    );
}

/// The Hopper warpgroup tile must keep compiling for `sm_90a`, and must stay OFF by default.
///
/// It is verified correct against an f32 oracle on real H100 silicon and measured SLOWER than the
/// Ampere tile it was built to replace (82.1 TFLOPS vs 119.1 on the prefill shape), so the shape rule
/// does not pick it — the house bar is golden AND faster on that chip, and it only clears the first.
/// The code is kept because the three things that are hard here — the D-fragment map, the descriptor's
/// leading-vs-stride convention, and the K order inside the core matrices — were settled against
/// hardware, and losing them costs days. This lock is what stops it rotting into something that no
/// longer builds, and what stops it being switched on by accident on the strength of "Hopper has wgmma".
#[test]
fn the_warpgroup_tile_still_builds_for_hopper_and_stays_unselected() {
    let cfg = Qwen38Cfg::real();
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
        &cfg,
        "/w",
        &[760usize, 6511],
        2,
        32,
        false,
        32,
    );
    assert!(
        cu.contains("#ifdef BQ_ENABLE_WGMMA") && cu.contains("wgmma.mma_async.sync.aligned"),
        "the warpgroup tile and its guard must both survive in the emitted kernel set"
    );
    assert!(!cu.contains("bq_gemm_wg<<<"),
        "bq_gemm_wg must NOT be launched: it is correct but measured slower than the mma.sync tile, \
         and the bar for adopting a lowering is golden AND faster on that chip");
    // The measured fragment map is load-bearing — every one of its 4096 register slots was checked
    // against the hardware, so a "tidy-up" of these two expressions is a silent correctness change.
    assert!(
        cu.contains("#define BQ_WG_ROW(w,l,i) ((w)*16 + ((l)>>2) + ((((i)&3)>>1)<<3))")
            && cu.contains("#define BQ_WG_COL(l,i)   ((((i)>>2)<<3) + (((l)&3)<<1) + ((i)&1))"),
        "the D-fragment map must stay exactly as the hardware probe measured it"
    );

    let have_nvcc = std::process::Command::new("nvcc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !require_tool("nvcc", have_nvcc) {
        return;
    }
    let dir = std::env::temp_dir().join(format!("logos_wg_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let src = dir.join("wg.cu");
    use logicaffeine_compile::codegen_cuda::gdn::{GDN_KERNELS, QWEN38_KERNELS};
    use logicaffeine_compile::codegen_cuda::qwen38::QWEN38_EXTRA_KERNELS;
    use logicaffeine_compile::codegen_cuda::qwen38_batched::QWEN38_BATCHED_KERNELS;
    use logicaffeine_compile::codegen_cuda::qwen_emit::GEN_DEVICE_KERNELS;
    std::fs::write(
        &src,
        format!(
            "#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
        #define BQ_ENABLE_WGMMA 1\n#define BTG 8\n#define MAXSEQ 64\n#define BMAX 64\n\
        {gdn}\n{q38}\n{gen}\n{extra}\n{bat}\n",
            gdn = GDN_KERNELS,
            q38 = QWEN38_KERNELS,
            gen = GEN_DEVICE_KERNELS,
            extra = QWEN38_EXTRA_KERNELS,
            bat = QWEN38_BATCHED_KERNELS
        ),
    )
    .expect("write");
    let out = std::process::Command::new("nvcc")
        .args(["-O3", "-arch=sm_90a", "-c"])
        .arg(&src)
        .args(["-o", "/dev/null"])
        .output()
        .expect("run nvcc");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "the warpgroup tile no longer compiles for sm_90a:\n{}",
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .take(12)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// The fp8 lane must keep building for Hopper, and must vanish cleanly on a toolkit without fp8.
///
/// Q8_0 carries a scale every 32 elements, so the accumulator is read out, converted and rescaled
/// FOUR times per mma issue — measured in-kernel on an H100, where that phase is over half the time.
/// fp8 e4m3 accumulates in fp32 natively and takes one scale per channel at the very end, which is
/// what vLLM's fp8 path does; this is therefore the MATCHED-PRECISION comparison, not a cheaper one.
///
/// Two guards, because the lane spans a toolkit boundary: fp8 mma needs PTX ISA 8.1 (CUDA 12.1+) and
/// `<cuda_fp8.h>`, while the equivalence locks compile with whatever `nvcc` is on PATH — 12.0 on this
/// box. An older toolkit must still build the whole kernel set with the lane simply absent, never
/// half-present and silently wrong.
#[test]
fn the_fp8_lane_builds_for_hopper_and_degrades_cleanly_without_it() {
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::QWEN38_BATCHED_KERNELS;
    assert!(
        cu.contains("mma.sync.aligned.m16n8k32.row.col.f32.e4m3.e4m3.f32"),
        "the fp8 lane must emit the e4m3 mma"
    );
    assert!(
        cu.contains("#define BQ_FP8_TOOLKIT 1") && cu.contains("#define BQ_FP8_TOOLKIT 0"),
        "the fp8 header must be guarded by toolkit version, both branches present"
    );
    assert!(
        cu.contains("__CUDA_ARCH__ >= 890"),
        "fp8 tensor cores are Ada and Hopper; the arch gate must say so"
    );
    assert!(
        cu.contains("out[i] = 0;"),
        "without an fp8 toolkit the requantiser must produce nothing, not a silently wrong value"
    );
    // It is UNVERIFIED: measured rel_L2 1.001 against an f64 oracle where the int8 lane reads 5.3e-3.
    // Nothing may launch it until that is resolved, and the marker must stay so it cannot be adopted
    // by someone reading only that it compiles.
    assert!(
        cu.contains("\u{26a0} UNVERIFIED AND NOT SELECTED"),
        "the fp8 tile's unverified marker must survive -- it compiles but computes the wrong thing"
    );
    let prog = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
        &Qwen38Cfg::real(),
        "/w",
        &[760usize, 6511],
        2,
        32,
        false,
        1,
    );
    assert!(
        !prog.contains("bq_gemm_fp8<<<"),
        "bq_gemm_fp8 must not be launched: it is measured WRONG (rel_L2 1.001 vs the f64 oracle)"
    );

    // find a toolkit that can actually assemble fp8; `nvcc` on PATH may predate it
    let fp8_nvcc = [
        "/usr/local/cuda-12.8/bin/nvcc",
        "/usr/local/cuda/bin/nvcc",
        "nvcc",
    ]
    .iter()
    .find(|c| {
        std::process::Command::new(c)
            .arg("--version")
            .output()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|l| l.contains("release") && !l.contains("release 12.0"))
            })
            .unwrap_or(false)
    })
    .copied();
    if !require_tool("nvcc-fp8", fp8_nvcc.is_some()) {
        return;
    }
    let nv = fp8_nvcc.expect("a CUDA toolkit newer than 12.0 answered");
    let dir = std::env::temp_dir().join(format!("logos_fp8_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("tmpdir");
    let src = dir.join("fp8.cu");
    use logicaffeine_compile::codegen_cuda::gdn::{GDN_KERNELS, QWEN38_KERNELS};
    use logicaffeine_compile::codegen_cuda::qwen38::QWEN38_EXTRA_KERNELS;
    use logicaffeine_compile::codegen_cuda::qwen_emit::GEN_DEVICE_KERNELS;
    std::fs::write(
        &src,
        format!(
            "#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n\
        #define BQ_TC_LANES 1\n#define BTG 8\n#define MAXSEQ 64\n#define BMAX 64\n\
        #define TC_KC 128\n#define TC_STAGES 2\n{gdn}\n{q38}\n{gen}\n{extra}\n{cu}\n",
            gdn = GDN_KERNELS,
            q38 = QWEN38_KERNELS,
            gen = GEN_DEVICE_KERNELS,
            extra = QWEN38_EXTRA_KERNELS
        ),
    )
    .expect("write");
    let out = std::process::Command::new(nv)
        .args(["-O3", "-arch=sm_90a", "-c"])
        .arg(&src)
        .args(["-o", "/dev/null"])
        .output()
        .expect("run nvcc");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "the fp8 lane no longer compiles for sm_90a with {nv}:\n{}",
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .take(10)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// Every split-K GEMM's partial planes are summed **exactly once** — by a `bq_sk_reduce` launch, or by
/// a consumer that folded the sum into itself.
///
/// This is the property that makes consumer-side fusion safe to keep extending. Dropping a reduce
/// without a consumer to replace it leaves the output buffer holding whatever the last step left there
/// — arithmetically plausible garbage that no equivalence oracle sees, because the oracles check the
/// KERNELS and this is a wiring error between them. Summing twice is just as invisible. The census is
/// per-launch, so either mistake is a count mismatch here.
#[test]
fn every_split_k_gemm_is_summed_exactly_once() {
    let cfg = Qwen38Cfg::real();
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
        &cfg,
        "/w",
        &[7, 11, 13],
        4,
        32,
        false,
    );

    let (mut split_gemms, mut reduces, mut folded) = (0usize, 0usize, 0usize);
    for line in cu.lines() {
        let l = line.trim();
        if l.starts_with("bq_sk_reduce<<<") {
            reduces += 1;
        }
        // A split tile launch takes the partial buffer; an unsplit one passes `(float*)0,1`.
        else if (l.starts_with("bq_gemm_tc<<<") || l.starts_with("bq_gemm_tc1<<<"))
            && l.contains(",skp,")
        {
            split_gemms += 1;
        } else if l.contains("_hl_sk<<<") {
            folded += 1;
        }
    }

    assert!(
        split_gemms > 0,
        "no split-K GEMM was emitted at all — the census proves nothing"
    );
    assert!(
        folded > 0,
        "the consumer-side split-K fusion emitted NOTHING: the lever is silently gone"
    );
    assert_eq!(
        split_gemms, reduces + folded,
        "split-K partials are not summed exactly once: {split_gemms} split GEMMs, but {reduces} reduce \
         launches + {folded} folded consumers. A GEMM whose partials nobody sums leaves its consumer \
         reading a stale buffer; one summed twice doubles the layer."
    );
}

/// The split-K partial buffer is 21.9% of the GEMM's memory traffic. That is a RATCHET, not a fact.
///
/// `split = clamp(ceil(TARGET/(N/TI)), 1, 16)` picks the split to maximise BLOCK COUNT and charges
/// nothing for the `SPLIT x N x B` floats each split writes and `bq_sk_reduce` reads back. Raising
/// TARGET therefore looks free at the call site and silently adds gigabytes to a step that is already
/// bandwidth-bound — 7.16 GB against 25.60 GB of weights at TARGET=1024, and 11.54 GB at 2048.
///
/// The number below is the measured cost of the shipped setting. It may go DOWN freely; going up means
/// someone traded HBM bandwidth for occupancy without measuring, which is the trade this campaign has
/// already lost twice (the split-K fold, the RMSNorm fusion).
#[test]
fn the_split_k_partials_stay_a_bounded_share_of_the_weight_stream() {
    let cfg = Qwen38Cfg::real();
    let batch = 32usize;
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched(
        &cfg,
        "/w",
        &[7, 11, 13],
        4,
        batch,
        false,
    );

    // The launches carry K and N as the program's own #defines, so resolve them from the same file.
    let mut defs: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for line in cu.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("#define ") {
            let mut it = rest.split_whitespace();
            if let (Some(k), Some(v)) = (it.next(), it.next()) {
                if it.next().is_none() {
                    if let Ok(n) = v.parse::<usize>() {
                        defs.insert(k, n);
                    }
                }
            }
        }
    }
    let val = |t: &str| -> Option<usize> {
        let t = t.trim();
        t.parse::<usize>().ok().or_else(|| defs.get(t).copied())
    };

    let (mut wbytes, mut pbytes, mut split_sites) = (0usize, 0usize, 0usize);
    for line in cu.lines() {
        let t = line.trim();
        if !(t.starts_with("bq_gemm_tc<<<") || t.starts_with("bq_gemm_tc1<<<")) {
            continue;
        }
        let args = &t[t.find(">>>(").expect("launch args") + 4..t.rfind(");").expect("launch end")];
        let a: Vec<&str> = args.split(',').collect();
        let Ok(sp) = a[a.len() - 1].trim().parse::<usize>() else {
            continue;
        };
        let (Some(k), Some(n)) = (val(a[a.len() - 6]), val(a[a.len() - 5])) else {
            panic!("could not resolve K/N for a tile launch: {t}");
        };
        wbytes += k * n;
        if sp > 1 {
            split_sites += 1;
            pbytes += sp * n * batch * 4 * 2;
        }
    }

    assert!(
        split_sites > 0 && wbytes > 0,
        "no split tile launches found — the census proves nothing"
    );
    let share = 100.0 * pbytes as f64 / (wbytes + pbytes) as f64;
    assert!(
        share <= 22.5,
        "split-K partials are {share:.1}% of the GEMM's memory traffic ({:.2} GB of partials against \
         {:.2} GB of weights across {split_sites} split launches). The shipped setting measures 21.9%. \
         Going UP means occupancy was bought with HBM bandwidth in a kernel that is bandwidth-bound — \
         measure it before raising LOGOS_Q38B_TC_TARGET.",
        pbytes as f64 / 1e9, wbytes as f64 / 1e9
    );
}

/// Every `#[test]` in this file must sit at brace depth ZERO, or the harness never collects it.
///
/// `emit_decode_gemm_bandwidth_bench` lived here for weeks with a `#[test]` attribute, a full body and
/// a latent compile error, and had **never run once**: its enclosing function was missing a closing
/// brace, which made it a NESTED fn, and `#[test]` on a nested fn compiles cleanly and is silently
/// ignored. Nothing was red. The file compiled. The only witness was the test COUNT.
///
/// That is the worst failure mode a suite can have — a gate that reports nothing because it does not
/// exist — and it is invisible to every other check here, so it gets its own.
#[test]
fn every_test_in_this_file_is_actually_collected() {
    let src = include_str!("qwen38_authored_lock.rs");
    let mut depth = 0i32;
    let mut in_line_comment;
    let mut offenders: Vec<&str> = Vec::new();
    let mut pending_test = false;

    for line in src.lines() {
        let t = line.trim_start();
        if t.starts_with("#[test]") {
            if depth != 0 {
                offenders.push("(a #[test] at nonzero brace depth)");
            }
            pending_test = true;
        } else if pending_test && t.starts_with("fn ") {
            if depth != 0 {
                let name = t.trim_start_matches("fn ").split('(').next().unwrap_or("?");
                offenders.push(name);
            }
            pending_test = false;
        }
        // Brace depth, ignoring line comments. NOTE: braces inside string literals ARE counted, so a
        // string containing an unbalanced `{` or `}` will unbalance the walk and make every later test
        // look nested — a false alarm that mimics the real defect. Keep test string literals balanced.
        // (A literal-aware walker was tried and reverted: Rust lifetimes `&'a str` read as char-literal
        // openers, which broke it worse.)
        in_line_comment = t.starts_with("//");
        if !in_line_comment {
            for c in line.chars() {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these #[test] functions are NESTED inside another function and are therefore NEVER COLLECTED \
         by the harness — they compile, nothing goes red, and they silently never run: {offenders:?}. \
         Check with `cargo test --test qwen38_authored_lock -- --list`."
    );
}

/// The sequence tile must cover the WIDEST pass the program launches, not the batch.
///
/// A speculative step verifies the draft and the current token together, so its pass carries 2x batch
/// rows. `tc_tj` was derived from `batch` alone, and the consequence was not a wrong answer — it was a
/// tile that covered half the verify, so the verify paid TWO full weight passes for two tokens and MTP
/// measured 933.51 tok/s against the non-speculative baseline's 936.89: a 90.6%-accept draft buying
/// EXACTLY NOTHING. Sizing the tile to the verify made the same accept rate worth +6.9% (1001.35).
///
/// The failure mode is invisible to every correctness gate — the tokens were identical throughout — and
/// invisible to the eye, because a tile sized to the batch looks obviously right.
#[test]
fn the_sequence_tile_covers_the_widest_pass_not_just_the_batch() {
    let cfg = Qwen38Cfg::real();
    let prompt = vec![7usize, 11, 13];
    for &batch in &[8usize, 32] {
        let plain =
            logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_mtp(
                &cfg, "/w", &prompt, 4, batch, false, 2, false,
            );
        let spec =
            logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_mtp(
                &cfg, "/w", &prompt, 4, batch, false, 2, true,
            );
        let tj = |cu: &str| -> usize {
            cu.lines()
                .find_map(|l| l.trim().strip_prefix("#define TC_TJ "))
                .and_then(|v| v.trim().parse().ok())
                .expect("the emitted program must define TC_TJ")
        };
        let (tj_plain, tj_spec) = (tj(&plain), tj(&spec));
        assert!(tj_plain >= batch,
            "at batch {batch} the non-speculative tile is {tj_plain}, narrower than the batch it serves");
        assert!(tj_spec >= 2 * batch || tj_spec == 64,
            "at batch {batch} the SPECULATIVE tile is {tj_spec}, but a verify pass carries {} rows. A \
             tile narrower than the widest pass costs an extra weight pass per step and silently erases \
             the entire speculative gain — the tokens stay correct, so nothing else here can catch it.",
            2 * batch);
        println!("[batch {batch}] TC_TJ plain={tj_plain} speculative={tj_spec}");
    }
}

/// Prefill and a speculative verify must be able to use DIFFERENT chunk lengths.
///
/// `prefill_chunk` once baked the compile-time `PT`, and `mtp_verify` calls it. A verify carries two
/// tokens, so the whole program had to be emitted at PT=2 for MTP to run at all — which made PREFILL
/// take 16.04 s instead of 11.66 s. A 4.4 s regression against a +6.9% decode gain is a net LOSS, so
/// speculation was unshippable for a reason that had nothing to do with speculation.
///
/// Prefill wants a wide chunk; a verify wants two rows. They are not the same number, and the only
/// thing that ever tied them together was a macro.
#[test]
fn prefill_and_verify_may_use_different_chunk_lengths() {
    let cfg = Qwen38Cfg::real();
    let prompt = vec![7usize, 11, 13];
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_mtp(
        &cfg, "/w", &prompt, 4, 32, false, 32, true,
    );

    assert!(cu.contains("static void prefill_chunk(cudaStream_t s, int MID, int PTv)"),
        "prefill_chunk must take the chunk length at RUNTIME; with it baked, prefill and verify are \
         forced to share one number and speculation costs 4.4 s of prefill to buy 6.9% of decode");
    assert!(
        cu.contains("prefill_chunk(s,1,2)"),
        "a speculative verify must pass 2 — it has the draft and the current token, never PT"
    );
    assert!(
        cu.contains("prefill_chunk(s,0,PT)"),
        "prefill must pass the full chunk length, not the verify's"
    );
    // and enabling speculation must NOT narrow the prefill chunk
    let pt = cu
        .lines()
        .find_map(|l| l.trim().strip_prefix("#define PT "))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .expect("PT must be defined");
    assert_eq!(pt, 32,
        "enabling MTP narrowed the prefill chunk to {pt}. That is the regression this separation \
         exists to prevent: prefill pays 16.04 s instead of 11.66 s and the speculative gain is lost.");
}

/// With speculation OFF, none of the above may perturb the shipped program.
#[test]
fn speculation_support_does_not_change_the_non_speculative_program() {
    let cfg = Qwen38Cfg::real();
    let prompt = vec![7usize, 11, 13];
    let plain =
        logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_mtp(
            &cfg, "/w", &prompt, 4, 32, false, 32, false,
        );
    let launches = |cu: &str| -> Vec<String> {
        cu.lines()
            .filter_map(|l| {
                let t = l.trim();
                t.find("<<<").map(|i| {
                    t[..i]
                        .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
            })
            .filter(|s| s.starts_with("bq_"))
            .collect()
    };
    let seq = launches(&plain);
    assert!(
        !seq.is_empty(),
        "the non-speculative program must launch kernels"
    );
    assert!(
        !plain.contains("mtp_step"),
        "speculation must not appear in the non-speculative program"
    );
    // the verify's two-row call may only exist where speculation does
    assert!(
        !plain.contains("prefill_chunk(s,1,2)"),
        "a verify-shaped call leaked into the non-speculative program"
    );
    println!(
        "non-speculative program: {} kernel launches, no speculative machinery",
        seq.len()
    );
}

/// DeepGEMM's Hopper fp8 GEMM must remain instantiable from a STANDALONE `.cu` — no torch, no Python,
/// no JIT. That is the precondition for this compiler emitting a program that uses it.
///
/// Four things had to be discovered to get there, and each is a silent failure if lost:
///   1. instantiate by taking the kernel's ADDRESS (`&sm90_fp8_gemm_1d1d_impl<...>`). An explicit
///      `template __global__ void ...` declaration does not name it and the symbol never appears.
///   2. `--expt-relaxed-constexpr` — without it `get_inner_block_atom_size` is a host constexpr called
///      from device code and nvcc rejects the whole translation unit.
///   3. the swizzle arguments are a size in BYTES from `get_swizzle_mode(block, elem_size)` — the first
///      of {128,64,32,16} dividing `block*elem_size` — NOT an enum index. Passing 3 compiles the
///      instantiation and then fails a static_assert deep inside TMA ("inner block must contain whole
///      atoms"), which reads like a shape bug rather than a units bug.
///   4. `BLOCK_K` must be 128: "Only support per-128-channel FP8 scaling".
///
/// Set `LOGOS_DEEPGEMM_DIR` and `LOGOS_CUTLASS_DIR` to run it; it skips otherwise so CI stays green
/// without vendoring ~30 MB of headers.
fn glob_cuda_bins() -> Vec<String> {
    std::fs::read_dir("/usr/local")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path().join("bin/nvcc");
            let n = e.file_name().to_string_lossy().to_string();
            (n.starts_with("cuda-") && p.exists()).then(|| p.to_string_lossy().to_string())
        })
        .collect()
}

#[test]
fn deepgemm_hopper_fp8_gemm_compiles_standalone() {
    let (Ok(dg), Ok(cutlass)) = (
        std::env::var("LOGOS_DEEPGEMM_DIR"),
        std::env::var("LOGOS_CUTLASS_DIR"),
    ) else {
        eprintln!("SKIP deepgemm compile probe — set LOGOS_DEEPGEMM_DIR and LOGOS_CUTLASS_DIR");
        return;
    };
    // CUTLASS 3.8 + DeepGEMM need a newer toolkit than the `nvcc` on PATH here (12.0), and the
    // failure is silent: nvcc exits non-zero with no line containing "error", so the assertion prints
    // an empty reason. Prefer an explicit toolkit, then the newest /usr/local/cuda-*, then PATH.
    let nvcc = std::env::var("LOGOS_NVCC")
        .ok()
        .or_else(|| {
            let mut found: Vec<String> = glob_cuda_bins();
            found.sort();
            found.pop()
        })
        .unwrap_or_else(|| "nvcc".to_string());
    let ver = std::process::Command::new(&nvcc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success());
    if !require_tool("nvcc", ver.is_some()) {
        return;
    }
    let ver = ver.expect("nvcc answered --version");
    let vtext = String::from_utf8_lossy(&ver.stdout).to_string();
    let dir = std::env::temp_dir().join(format!("logos_dg_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let src = dir.join("probe.cu");
    std::fs::write(
        &src,
        r#"
#include <deep_gemm/impls/sm90_fp8_gemm_1d1d.cuh>
using namespace deep_gemm;
static void __instantiate_kernel() {
    auto ptr = reinterpret_cast<void*>(&sm90_fp8_gemm_1d1d_impl<
        0, 5120, 17408,   1,   64, 128, 128,   128, 128,   4,   128, 128,   1, true,   132,
        GemmType::Normal, float>);
    (void)ptr;
}
"#,
    )
    .expect("write probe");
    let obj = dir.join("probe.o");
    let out = std::process::Command::new(&nvcc)
        .args([
            "-std=c++17",
            "-arch=sm_90a",
            "-c",
            "--expt-relaxed-constexpr",
            "--diag-suppress",
            "177",
        ])
        .arg("-I")
        .arg(format!("{dg}/deep_gemm/include"))
        .arg("-I")
        .arg(format!("{cutlass}/include"))
        .arg("-o")
        .arg(&obj)
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        out.status.success(),
        "DeepGEMM's sm90 fp8 GEMM no longer compiles standalone with {nvcc}\n({}):\n{}",
        vtext.lines().last().unwrap_or("?"),
        // print the tail unfiltered: the toolkit-too-old failure has NO line containing "error",
        // so filtering for it prints an empty reason and hides the cause.
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
    let size = std::fs::metadata(&obj).map(|m| m.len()).unwrap_or(0);
    assert!(size > 10_000,
        "the object is {size} bytes — the kernel was not actually instantiated, only the headers parsed");
    println!("DeepGEMM sm90 fp8 GEMM: standalone object {size} bytes");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A REAL emitted program must compile with the DeepGEMM lane enabled — not just the library probe.
///
/// The lane brings `<deep_gemm/...>` + CUTLASS, `cuTensorMapEncodeTiled`, the five-descriptor helpers
/// and the fp8-per-128 converters into the same translation unit as 5000 lines of our own kernels.
/// Each of those has its own arch and toolkit guards, and the first attempt failed on ordering alone:
/// the host helpers size shared memory with `BQ_DG_BLOCK_K`, which the converter block defined later.
/// That is invisible until the two are compiled together.
#[test]
fn the_emitted_program_compiles_with_the_deepgemm_lane() {
    let (Ok(dg), Ok(cutlass)) = (
        std::env::var("LOGOS_DEEPGEMM_DIR"),
        std::env::var("LOGOS_CUTLASS_DIR"),
    ) else {
        require_tool("deepgemm", false);
        return;
    };
    let mut cands = glob_cuda_bins();
    cands.sort();
    let Some(nvcc) = std::env::var("LOGOS_NVCC").ok().or_else(|| cands.pop()) else {
        require_tool("nvcc", false);
        return;
    };
    let cfg = Qwen38Cfg::real();
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
        &cfg,
        "/w",
        &[7usize, 11, 13],
        4,
        32,
        false,
        32,
    );
    let dir = std::env::temp_dir().join(format!("logos_dglane_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let src = dir.join("lane.cu");
    std::fs::write(&src, &cu).expect("write lane");
    let out = std::process::Command::new(&nvcc)
        .args([
            "-std=c++17",
            "-arch=sm_90a",
            "-c",
            "-o",
            "/dev/null",
            "-DBQ_ENABLE_DG",
            "--expt-relaxed-constexpr",
            "--diag-suppress",
            "177",
        ])
        .arg("-I")
        .arg(format!("{dg}/deep_gemm/include"))
        .arg("-I")
        .arg(format!("{cutlass}/include"))
        .arg(&src)
        .output()
        .expect("run nvcc");
    assert!(
        out.status.success(),
        "the emitted program does not compile with BQ_ENABLE_DG:\n{}",
        String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    );
    println!("emitted program compiles for sm_90a with the DeepGEMM lane enabled");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A routed DeepGEMM site is a SUBSTITUTION, not a new computation: it must consume the same K, produce
/// the same N, and write at the same row stride as the int8 launch it stands in for. Nothing in the
/// emitter enforces that today -- `dg_site` takes k, n and ys as three independent integers, and three
/// integers passed in the wrong order still compile, still run, and still produce a plausible tensor of
/// the right shape. This campaign has already shipped five fast-and-wrong results; a shape mismatch here
/// is exactly that failure with a 1.4x speedup attached to hide it.
///
/// Both halves of every site are emitted into the same source, so the check needs no GPU and no nvcc:
/// read the DeepGEMM instantiation's compile-time <N, K>, read the fallback launch's runtime (K, N, ys),
/// and require that they agree site by site.
#[test]
fn every_deepgemm_site_matches_the_shape_of_the_lane_it_replaces() {
    let cfg = Qwen38Cfg::real();
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
        &cfg,
        "/w",
        &[7usize, 11, 13],
        4,
        32,
        false,
        32,
    );

    // Each routed site is one `if(!bq_dg_on[i]) { <fallback> } else { <deepgemm> }`, and the whole block
    // sits inside `#ifdef BQ_ENABLE_DG`. Walk the sites in emission order.
    let mut sites = 0usize;
    let mut per_index = [0usize; 3];
    for (n, chunk) in cu
        .match_indices("if(!bq_dg_on[")
        .map(|(i, _)| i)
        .enumerate()
    {
        let tail = &cu[chunk..];
        let idx: usize = tail[13..14].parse().expect("site index");
        let end = tail
            .find("#endif")
            .map(|e| e + 6)
            .unwrap_or(tail.len().min(6000));
        let body = &tail[..end];

        // the fallback launch: bq_gemm_tcm<<<...>>>(Y, W_q, W_s, Xh, Xs, K, N, BT, ys);
        let fb = body.find("bq_gemm_tcm<<<").map(|p| {
            let args = &body[p..][body[p..].find(">>>(").unwrap() + 4..];
            let args = &args[..args.find(");").unwrap()];
            let f: Vec<&str> = args.split(',').collect();
            (
                f[5].trim().parse::<usize>().unwrap(), // K
                f[6].trim().parse::<usize>().unwrap(), // N
                f[8].trim().parse::<usize>().unwrap(),
            ) // ys
        });
        let Some((k_fb, n_fb, ys_fb)) = fb else {
            continue;
        };

        // the DeepGEMM half: sm90_fp8_gemm_1d1d_impl<0,N,K,...> and bq_dg_cd(out,_m,N,ys)
        let ip = body
            .find("sm90_fp8_gemm_1d1d_impl<0,")
            .expect("site has no deepgemm half");
        let targs = &body[ip + 26..];
        let targs = &targs[..targs.find(',').unwrap()
            + 1
            + targs[targs.find(',').unwrap() + 1..].find(',').unwrap()
            + 1];
        let mut it = targs.split(',');
        let n_dg: usize = it.next().unwrap().trim().parse().expect("N");
        let k_dg: usize = it.next().unwrap().trim().parse().expect("K");

        let cp = body
            .find("bq_dg_cd(")
            .expect("site has no output descriptor");
        let cargs = &body[cp + 9..][..body[cp + 9..].find(')').unwrap()];
        let cf: Vec<&str> = cargs.split(',').collect();
        let n_cd: usize = cf[2].trim().parse().expect("cd N");
        let ys_cd: usize = cf[3].trim().parse().expect("cd ys");

        assert_eq!((k_dg, n_dg), (k_fb, n_fb),
            "site {n} (index {idx}): the DeepGEMM instantiation computes {n_dg}x{k_dg} but the lane it \
             replaces computes {n_fb}x{k_fb} — a substitution that changes the shape is a different model");
        assert_eq!((n_cd, ys_cd), (n_fb, ys_fb),
            "site {n} (index {idx}): the output descriptor writes {n_cd} columns at stride {ys_cd}, the \
             lane it replaces writes {n_fb} at stride {ys_fb} — the rows would shear");
        sites += 1;
        per_index[idx] += 1;
    }

    assert!(
        sites > 0,
        "no routed DeepGEMM sites found — the lane vanished"
    );
    for (i, c) in per_index.iter().enumerate() {
        if *c > 0 {
            assert_eq!(*c, cfg.layers,
                "site index {i} is routed on {c} layers but the model has {} — a routed site must be \
                 routed on every layer or the lane is inconsistent", cfg.layers);
        }
    }
}

/// The second fp8 pass is what buys the accuracy back, and it is exactly the kind of thing that can go
/// half-wired without anyone noticing: a site that emits the hi GEMM and skips the lo one still runs,
/// still produces a tensor of the right shape, and is simply 2.3x lossier than the lane it replaced.
/// The speedup would even look BETTER. So every routed site must carry all three pieces -- the hi GEMM,
/// the lo GEMM behind the `bq_dg_hl` switch, and the fold that sums them -- and the fold must be told
/// the same (n, ys) the site writes at, or it would add the second pass onto the wrong columns.
#[test]
fn every_routed_site_carries_a_complete_second_fp8_pass() {
    let cfg = Qwen38Cfg::real();
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_pt(
        &cfg,
        "/w",
        &[7usize, 11, 13],
        4,
        32,
        false,
        32,
    );

    let mut sites = 0usize;
    for start in cu.match_indices("if(!bq_dg_on[").map(|(i, _)| i) {
        let tail = &cu[start..];
        let end = tail.find("#endif").unwrap_or(tail.len().min(6000));
        let body = &tail[..end];
        // count LAUNCHES, not template mentions: the name also appears in the cudaFuncSetAttribute
        // that raises the dynamic shared-memory cap, which both passes share.
        let launches = body.matches("<<<132,256,BQ_DG_SMEM,s>>>").count();
        assert_eq!(launches, 2, "site at byte {start}: expected a hi and a lo DeepGEMM launch, found \
                    {launches} — a site with one pass is silently 2.3x lossier than the int8 it replaced");
        assert_eq!(body.matches("if(bq_dg_hl)").count(), 1,
            "site at byte {start}: the second pass is not behind the bq_dg_hl switch, so it cannot be \
             turned off for an A/B and cannot be measured against single-pass");

        // the fold must use the site's own (n, ys), not the packed scratch's
        let (cp, ap) = (
            body.find("bq_dg_cd(").unwrap(),
            body.find("bq_dg_add<<<").unwrap(),
        );
        let cargs: Vec<&str> = body[cp + 9..][..body[cp + 9..].find(')').unwrap()]
            .split(',')
            .collect();
        let aargs: Vec<&str> = body[ap..][body[ap..].find(">>>(").unwrap() + 4..][..body[ap..]
            [body[ap..].find(">>>(").unwrap() + 4..]
            .find(')')
            .unwrap()]
            .split(',')
            .collect();
        let (n_cd, ys_cd) = (cargs[2].trim(), cargs[3].trim());
        let (out_cd, out_add, ys_add, n_add) = (
            cargs[0].trim(),
            aargs[0].trim(),
            aargs[1].trim(),
            aargs[3].trim(),
        );
        assert_eq!((out_add, ys_add, n_add), (out_cd, ys_cd, n_cd),
            "site at byte {start}: the hi pass writes {n_cd} columns of {out_cd} at stride {ys_cd} but \
             the fold adds {n_add} columns into {out_add} at stride {ys_add} — the second pass would \
             land on the wrong columns");
        sites += 1;
    }
    assert_eq!(
        sites,
        3 * cfg.layers,
        "expected a routed site per class per layer ({} total), found {sites}",
        3 * cfg.layers
    );
}

/// Q4 halves the decode weight stream, and decode is bound by exactly that, so it is the one lever on
/// the metric we lose with chip-independent arithmetic behind it. It is also the lever that spent this
/// campaign quietly invalid.
///
/// At TC_STAGES==1 the tile stages weights through registers via `BQTC_GLOAD`, which reads at the INT8
/// stride and deposits into `sW` -- while the Q4 fragment reader takes its nibbles from `sP`, which
/// nothing on that path writes. Against a packed buffer of N*K/2 that runs about 44 MB past the end,
/// and where it survives the fault it multiplies an uninitialised tile. The kernel now refuses the
/// combination outright with a static_assert; this lock is the other half, so the emitter can never ask
/// for what the kernel will refuse, and a Q4 build never silently drops to a depth Q4 cannot use.
///
/// The standing 0.84x Q4 verdict was taken before this was known. It is void: it measured a shape that
/// either faulted or read uninitialised shared memory, not Q4.
#[test]
fn the_q4_lane_never_selects_the_register_prefetch_path() {
    let cfg = Qwen38Cfg::real();
    let cu = logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_q4(
        &cfg,
        "/w",
        &[7usize, 11, 13],
        4,
        32,
        false,
        1,
        false,
        true,
    );

    assert!(
        cu.contains("#define BQ_ENABLE_Q4"),
        "the Q4 lane was requested but the program does not enable it"
    );

    let stages: usize = cu
        .split("#define TC_STAGES ")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .expect("emitted program declares a staging depth");
    assert!(stages > 1,
        "the Q4 lane was emitted at TC_STAGES={stages}. One stage is the register-prefetch path, which \
         reads packed weights at the int8 stride and fills the wrong shared tile — the kernel's \
         static_assert will reject this, and before that assert existed it read ~44 MB out of bounds");

    // Q4 pays for BOTH shared tiles -- `sW` is declared unconditionally and the packed `sP` sits beside
    // it -- so the chunk must be small enough that two stages of both still fit the 48 KB static budget.
    let kc: usize = cu
        .split("#define TC_KC ")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .expect("emitted program declares a K chunk");
    let tj: usize = cu
        .split("#define TC_TJ ")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .expect("tile width");
    let wm: usize = cu
        .split("#define TC_WM ")
        .nth(1)
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .expect("tile height");
    let ti = 16 * (8 / (tj / 8)) * wm;
    let per_stage = ti * (kc / 4) * 4
        + tj * (kc / 4) * 4
        + tj * (kc / 32) * 4
        + ti * (kc / 32) * 2
        + ti * (kc / 8) * 4;
    assert!(stages * per_stage <= 48 * 1024,
        "the Q4 lane asks for {} bytes of static shared at TC_KC={kc}, TC_STAGES={stages}, TI={ti} — \
         over the 48 KB budget. This is the failure the Hopper gate once caught as 0xec00 vs 0xc000, \
         and it happens because Q4's packed tile is charged ON TOP of the int8 one, not instead of it",
        stages * per_stage);
}

/// A DEFAULT THAT NO SHIPPED CONFIGURATION USES IS A TRAP, NOT A DEFAULT.
///
/// `TC_MINBLOCKS` feeds `__launch_bounds__(256, TC_MINBLOCKS)` and decides how many blocks an SM holds.
/// The emitter always writes the measured value ahead of the kernels (3 on Hopper, 2 below), so the
/// `#ifndef` fallback inside the kernel source is reached only by something compiling these kernels
/// WITHOUT the emitter — which is exactly what the decode bandwidth bench does. That fallback was 1.
///
/// The cost was not theoretical. The bench reported 155 GB/s at the decode shape and I read it as the
/// tile getting 47% of the chip; with the shipped residency the same tile reads 225 GB/s, 66% of
/// ceiling. The instrument was understating it by 45% and I had already written a conclusion on top of
/// that number. It is the same defect as the bench that once ran SPLIT=1 while the program split 16
/// ways, and the house rule it breaks is the one that matters most here: an instrument must run the
/// shipped configuration or it is measuring a program nobody has.
#[test]
fn the_kernel_defaults_are_the_shipped_defaults_not_placeholders() {
    use logicaffeine_compile::codegen_cuda::qwen38_batched::QWEN38_BATCHED_KERNELS;
    let src = QWEN38_BATCHED_KERNELS;

    let at = src
        .find("#ifndef TC_MINBLOCKS")
        .expect("the kernels declare an occupancy fallback");
    let block = &src[at..][..src[at..]
        .find("#endif\n#endif")
        .map(|e| e + 13)
        .unwrap_or(200)];

    assert!(!block.contains("#define TC_MINBLOCKS 1\n"),
        "the occupancy fallback is 1 block per SM, which no shipped configuration asks for. Anything \
         compiling these kernels without the emitter — the decode bandwidth bench above, for one — \
         then measures a residency the program never runs at, and at the decode shape that reads 45% \
         low. Make the fallback equal the shipped value");
    assert!(block.contains("__CUDA_ARCH__ >= 900"),
        "the occupancy fallback does not branch on the architecture, but the measured optimum differs \
         between them by 50%: Hopper wants 3 blocks (1148 GB/s vs 1045 at 2), Ampere wants 2 (199 vs \
         156 at 3, where ptxas spills). One constant gives up 10-15% on whichever chip it is not");

    // and the depth default must likewise be a depth the program actually emits
    let sat = src
        .find("#ifndef TC_STAGES")
        .expect("the kernels declare a staging fallback");
    let sblock = &src[sat..][..200.min(src.len() - sat)];
    assert!(
        sblock.contains("#define TC_STAGES"),
        "the staging fallback vanished; a bench compiling these kernels would not build"
    );
}

/// EVERY LAUNCH STARTS ITS OWN LINE, because half the census in this file counts line prefixes.
///
/// `every_split_k_gemm_is_summed_exactly_once` matches `l.starts_with("bq_gemm_tc1<<<")`, and several
/// other checks here walk lines the same way. Nothing asserted the invariant they all depend on, and
/// the day it broke was the day a runtime ablation guard was emitted as `if(!bq_da[4]){ bq_gemm_tc1<<<`
/// — 64 launches on shared lines, invisible to a line-prefix count, reported as 64 GEMMs whose
/// partials nobody summed. The emitted program was byte-identical; only its FORMATTING had moved.
///
/// That failure mode is worth guarding directly: a launch hidden from the census is a launch no lock
/// in this file can see, and the next one might be hidden by something that is not a whitespace change.
#[test]
fn every_emitted_launch_begins_its_own_line() {
    let cfg = Qwen38Cfg::real();
    for (batch, prefill_t, mtp) in [(32usize, 32usize, false), (32, 1, true), (8, 16, false)] {
        let cu =
            logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_mtp(
                &cfg,
                "/w",
                &[7usize, 11, 13],
                4,
                batch,
                false,
                prefill_t,
                mtp,
            );
        let mut shared = Vec::new();
        for (n, line) in cu.lines().enumerate() {
            let Some(at) = line.find("<<<") else { continue };
            // the launch's identifier is whatever runs back from `<<<`
            let head = &line[..at];
            let start = head
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map_or(0, |i| i + 1);
            if head[..start].trim().is_empty() {
                continue;
            } // only leading whitespace: fine
            if head[..start].trim_start().starts_with("//") {
                continue;
            }
            shared.push(format!(
                "line {}: {}",
                n + 1,
                line.trim().chars().take(90).collect::<String>()
            ));
        }
        assert!(shared.is_empty(),
            "at batch={batch} prefill_t={prefill_t} mtp={mtp}, {} kernel launch(es) do not begin their \
             line, so every line-prefix census in this file is blind to them:\n  {}",
            shared.len(), shared.join("\n  "));
    }
}

/// The honest tool-skip seam, mirroring `logicaffeine_tests::common::require_tool`. A test that prints
/// `SKIP` and returns reports PASS to cargo, so an absent tool becomes a green result nobody reads —
/// which is why `skip_budget_lock` ratchets those sites down and tells you not to raise its ceiling.
/// This emits the machine-readable marker a CI budget can grep, and turns an absent tool into a LOUD
/// failure when `LOGOS_REQUIRE_TOOLS` demands it.
fn require_tool(tool: &str, available: bool) -> bool {
    if available {
        return true;
    }
    let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
    let required = spec == "1"
        || spec.eq_ignore_ascii_case("all")
        || spec
            .split([',', ' ', '\t'])
            .map(str::trim)
            .any(|t| t == tool);
    assert!(
        !required,
        "required external tool '{tool}' is not available, but LOGOS_REQUIRE_TOOLS demands it — \
         the selected gate fails loud (never a silent SKIP-as-PASS)"
    );
    eprintln!("LOGOS_TOOL_SKIP: {tool}");
    false
}

/// ONE MATMUL LOWERING PER PROJECTION, CHECKED IN THE NORMAL SUITE — not only when someone emits a file.
///
/// `emit_qwen38_batched_cu` asserts this, but it runs only under `LOGOS_Q38B_EMIT`, and it writes the
/// file BEFORE it asserts. So when the DeepGEMM runtime mask started emitting the int8 fallback twice per
/// routed site (once in the mask branch, again under `#else`) — 448 lowerings for 256 prefill
/// projections — nothing in the suite noticed, and every pod driver kept treating "wrote" in the output
/// as success for a whole session. An invariant that only runs on request is a comment.
///
/// Checked here at the shipped decode shape and the prefill/verify shapes, per lane, in memory.
#[test]
fn every_projection_gets_exactly_one_matmul_lowering_in_every_lane() {
    const LOWERINGS: [&str; 10] = [
        "bq_gemv_hl<<<",
        "bq_gemv_h<<<",
        "bq_gemv_hl_smem<<<",
        "bq_gemv_h_smem<<<",
        "bq_gemm_mma<<<",
        "bq_gemm_mmq<<<",
        "bq_gemm_tc<<<",
        "bq_gemv_wide<<<",
        "bq_gemm_tc1<<<",
        "bq_gemm_tcm<<<",
    ];
    let cfg = Qwen38Cfg::real();
    for (batch, prefill_t, mtp) in [
        (32usize, 32usize, false),
        (32, 32, true),
        (8, 16, false),
        (32, 1, false),
    ] {
        let cu =
            logicaffeine_compile::codegen_cuda::qwen38_batched::emit_qwen38_generate_batched_mtp(
                &cfg,
                "/w",
                &[7usize, 11, 13],
                4,
                batch,
                false,
                prefill_t,
                mtp,
            );
        let mut bounds: Vec<(&str, usize)> = [
            "mtp_draft",
            "mtp_verify",
            "mtp_step",
            "prefill_chunk",
            "decode_step",
        ]
        .iter()
        .filter_map(|n| cu.find(&format!("static void {n}(")).map(|i| (*n, i)))
        .collect();
        bounds.sort_by_key(|&(_, i)| i);
        let slice = |name: &str| -> &str {
            match bounds.iter().position(|&(n, _)| n == name) {
                None => "",
                Some(k) => &cu[bounds[k].1..bounds.get(k + 1).map(|&(_, i)| i).unwrap_or(cu.len())],
            }
        };
        let (pre, dec) = (slice("prefill_chunk"), slice("decode_step"));
        for (lane, body, want) in [
            ("decode", dec, 4 * cfg.layers + 1),
            (
                "prefill",
                pre,
                if pre.is_empty() { 0 } else { 4 * cfg.layers },
            ),
        ] {
            let n: usize = LOWERINGS.iter().map(|k| body.matches(k).count()).sum();
            assert_eq!(n, want,
                "batch={batch} prefill_t={prefill_t} mtp={mtp}: the {lane} lane has {n} matmul lowerings for \
                 {want} projections. More than one per site is the DeepGEMM-mask defect (a fallback emitted \
                 in two branches); fewer means a projection was dropped");
        }
    }
}
