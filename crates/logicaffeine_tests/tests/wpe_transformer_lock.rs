//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! WPE-TRANSFORMER LOCK — the transformer's core compute runs on the reconfigurable WHOLE-PROGRAM FABRIC
//! ENGINE ("the fabric IS the processor" — no fetch-decode CPU): a genuine int8 PROJECTION (matmul →
//! fixed-point requant → argmax, the RDU's canonical inner workload) lowers to the config-driven engine
//! and runs byte-identical to the VM — as the ENGINE MODEL, and on the emitted ENGINE RTL under iverilog.
//!
//! This is the whole-program tier of the dual RDU demo: the same reconfigurable engine that
//! `wprog_soc_lock` proves runs recursion / collections / division / float, here runs a transformer
//! projection. The engine is a block-walk sequencer (1 cell/clock, no bytecode fetch), so its win is
//! "no fetch-decode of an instruction stream," distinct from the spatial CGRA's 1-MAC/clock
//! (transformer_fabric_lock). Together they are the full "runs on the reconfigurable dataflow fabric" story.
//!
//! The whole int8 PROJECTION (a heap-built weight matrix, a nested-loop matmul, fixed-point requant, and
//! argmax) runs on the config-driven whole-program ENGINE ≡ VM — the transformer's core workload on the
//! reconfigurable fabric with NO CPU. The engine RTL additionally runs the transformer's MAC ≡ VM on the
//! actual gates under iverilog (and `wprog_soc_lock` runs collections/division/float/recursion on the same
//! engine RTL). NOTE the engine is a SEQUENTIAL block-walk (1 cell/clock) — the "fabric is the processor"
//! completeness proof; the SPEED path (cranking tokens) is the SPATIAL CGRA at u MACs/clock, in
//! `transformer_fabric_lock` / the throughput ledger.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::process::Command;

use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_program_cfg_fabric, program_cfg_fabric, try_program_cfg_fabric, with_seq_reserve,
};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_testbench, emit_wprog_engine_verilog, eval_wprog_config,
    eval_wprog_config_counted, eval_wprog_config_counted_heap_preloaded, serialize_program,
    wprog_config_fits_engine, wprog_config_image,
};
use logicaffeine_compile::compile::vm_outcome;

#[path = "common/mod.rs"]
mod common;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Run the validated generator in the requested mode, returning its stdout (or `None` if python3 is absent).
fn gen(mode: &str) -> Option<String> {
    gen_args(&[mode])
}

/// As [`gen`] but with multiple flags (e.g. `["--fabric", "--stream"]`).
fn gen_args(flags: &[&str]) -> Option<String> {
    let out = Command::new("python3")
        .current_dir(root())
        .arg("scripts/gen_tiny_transformer_lg.py")
        .args(flags)
        .output()
        .ok()?;
    if !out.status.success() {
        eprintln!(
            "gen_tiny_transformer_lg.py {flags:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse the generator's `--blob` output (a JSON int list `[len, w0, w1, …]`) into the heap pre-load image.
fn parse_blob(s: &str) -> Vec<i64> {
    s.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .filter(|t| !t.trim().is_empty())
        .map(|t| t.trim().parse::<i64>().expect("blob int"))
        .collect()
}

/// A genuine int8 PROJECTION as a whole-program function: build an 8×8 weight matrix + an 8-vector (both
/// seeded so the argmax varies), compute `y[o] = requant(Σ_i x[i]·w[o,i])`, return `argmax_o y[o]`. This is
/// the transformer's inner workload — int8 MAC + fixed-point requant (`(acc·M)>>S`) + argmax — in one
/// function the engine runs end to end.
const PROJ: &str = "## To proj (seed: Int) -> Int:\n    Let din be 8.\n    Let dout be 8.\n    Let w be a new Seq of Int.\n    Let i be 0.\n    While i is less than din * dout:\n        Push (((i + seed) * 37) % 256) - 128 to w.\n        Set i to i + 1.\n    Let x be a new Seq of Int.\n    Set i to 0.\n    While i is less than din:\n        Push (((i + seed) * 53) % 256) - 128 to x.\n        Set i to i + 1.\n    Let best be -1000000000.\n    Let bestIdx be 0.\n    Let o be 0.\n    While o is less than dout:\n        Let acc be 0.\n        Let k be 0.\n        While k is less than din:\n            Set acc to acc + (item (k + 1) of x) * (item (o * din + k + 1) of w).\n            Set k to k + 1.\n        Let y be (acc * 1717986918) shifted right by 34.\n        If y is greater than best:\n            Set best to y.\n            Set bestIdx to o.\n        Set o to o + 1.\n    Return bestIdx.\n";

const SEEDS: [i64; 6] = [0, 1, 2, 3, 5, 7];

/// The transformer's core multiply-accumulate as a heap-free whole-program function: `macdot(n) =
/// Σ_{k=1..n} k·(n−k)` — a genuine int8-range MAC (the same kernel the CGRA lock certifies), in the
/// arithmetic form the engine RTL executes on real gates (no heap state-plane needed).
const MACDOT: &str = "## To macdot (n: Int) -> Int:\n    Let s be 0.\n    Let k be 1.\n    While k is at most n:\n        Set s to s + k * (n - k).\n        Set k to k + 1.\n    Return s.\n";

fn vm_call(src: &str, fname: &str, arg: i64) -> i64 {
    let full = format!("{src}## Main\n    Show {fname}({arg}).\n");
    let out = vm_outcome(&full);
    assert!(
        out.error.is_none(),
        "{fname}({arg}): VM error {:?}",
        out.error
    );
    out.output.trim().parse::<i64>().expect("int Show")
}

/// The int8 projection lowers to the whole-program engine, fits the (widened) generic engine, and the
/// ENGINE MODEL (`eval_wprog_config`) runs it byte-identical to the VM across a seed battery.
#[test]
fn int8_projection_runs_on_the_fabric_engine_model_matching_the_vm() {
    let pf = program_cfg_fabric(PROJ, "proj");
    let cfg = serialize_program(&pf);
    let total_cells: usize = cfg.blocks.iter().map(|b| b.cells.len()).sum();
    wprog_config_fits_engine(&cfg)
        .unwrap_or_else(|e| panic!("the int8 projection must fit the widened engine: {e}"));
    for &seed in &SEEDS {
        let got = eval_wprog_config(&cfg, &[seed]);
        assert_eq!(
            got,
            vm_call(PROJ, "proj", seed),
            "engine model proj({seed}) != VM"
        );
    }
    println!(
        "the int8 projection (matmul+requant+argmax) runs on the fabric ENGINE model ≡ VM \
         ({} blocks, {total_cells} cells) — the fabric is the processor.",
        cfg.blocks.len()
    );
}

/// ★ THE MONDAY HEADLINE — the WHOLE baby LLM runs DIRECTLY ON THE FABRIC. The entire autoregressive
/// generation (embedding + positional add, multi-head causal attention, integer exp-LUT softmax, ReLU FFN,
/// tied-embedding output projection, argmax, and the 3-token generation loop) is emitted as ONE call-free
/// LOGOS function `generate()` (`gen_tiny_transformer_lg.py --fabric`) and LOWERS whole to the config-driven
/// whole-program FABRIC engine — no fetch-decode CPU, the fabric IS the processor. The engine model runs it
/// byte-identical to the VM, producing the base-100 packing of the generated tokens [8,1,13] = "mat" (80113).
///
/// This is the full "we can put an LLM directly onto the reconfigurable dataflow fabric" proof: not a kernel,
/// not a single projection — the whole net's forward pass AND the generation loop, on the fabric engine, ≡ VM.
/// The engine is the SAME config-driven netlist `wprog_soc_lock` runs under iverilog for every construct this
/// net uses (collections / division / floordiv / recursion-frame calls / control flow); here the model runs
/// the whole net end to end (158 blocks / ~19k cells), and its block-step count IS the whole-program fabric's
/// cycle count for generating "mat". python3-gated (the generator is the validated source of the program).
#[test]
fn whole_transformer_generates_mat_directly_on_the_fabric_engine() {
    if !common::require_tool("python3") {
        return;
    }
    let src = gen("--fabric").expect("generate the call-free fabric transformer program");
    let packed: i64 = gen("--packed")
        .expect("packed golden")
        .trim()
        .parse()
        .expect("packed int");
    assert_eq!(
        packed, 80113,
        "the golden generation must pack to 80113 = \"mat\" ([8,1,13] base-100)"
    );

    // The VM runs the whole program (the reference the fabric must match).
    let vm = vm_outcome(&src);
    assert!(
        vm.error.is_none(),
        "VM error running generate(): {:?}",
        vm.error
    );
    let vm_val: i64 = vm.output.trim().parse().expect("VM Show int");
    assert_eq!(
        vm_val, packed,
        "the VM must generate \"mat\" (80113) before the fabric can match it"
    );

    // The WHOLE net lowers to the config-driven fabric engine and runs byte-identical to the VM.
    let pf = program_cfg_fabric(&src, "generate");
    let cfg = serialize_program(&pf);
    let total_cells: usize = cfg.blocks.iter().map(|b| b.cells.len()).sum();
    let (got, steps) = eval_wprog_config_counted(&cfg, &[]);
    assert_eq!(
        got,
        vm_val,
        "the fabric engine diverged from the VM generating \"mat\" \
         ({} blocks, {total_cells} cells)",
        cfg.blocks.len()
    );
    assert_eq!(
        got, 80113,
        "the fabric engine must generate \"mat\" (80113)"
    );
    println!(
        "★ THE WHOLE BABY LLM RUNS DIRECTLY ON THE FABRIC: the entire autoregressive generation \
         (attention + softmax + FFN + argmax + the 3-token loop) lowers to the config-driven whole-program \
         fabric engine ({} blocks / {total_cells} cells) and generates \"mat\" (packed {got}) byte-identical \
         to the VM in {steps} block-steps = the whole-program fabric's cycle count. The fabric is the processor.",
        cfg.blocks.len()
    );
}

/// ★ STREAMED WEIGHTS — the whole LLM on the fabric with its weights PRE-LOADED into the heap (the
/// DATA-segment model of an accelerator: kernel and weights arrive over separate channels), not Push-built
/// into the config image. `gen_tiny_transformer_lg.py --fabric --stream` reads every weight from `loaded()`
/// (heap pointer 0) instead of building a `W` Seq, so the ~3,136 straight-line weight-Push cells vanish. The
/// effect is decisive for fitting ONE chip: the config collapses from 19,374 cells to ~556 — small enough to
/// live in flip-flops — while only the heap (the state plane) needs block RAM, which the `Bram` backend
/// already provides. The engine model runs the streamed net byte-identical to the VM with the blob
/// pre-loaded, still generating "mat" (80113). Right-sizing the per-collection reserve (`with_seq_reserve`)
/// keeps the runtime heap ~18k words (≈1.2 Mb), well inside the video board's block RAM. This test is the
/// PROOF that streaming + right-sizing is behaviour-preserving AND achieves the collapse that makes the whole
/// net synthesizable on one FPGA. python3-gated.
#[test]
fn whole_transformer_generates_mat_on_the_fabric_engine_with_streamed_weights() {
    if !common::require_tool("python3") {
        return;
    }
    let packed: i64 = gen("--packed")
        .expect("packed golden")
        .trim()
        .parse()
        .expect("packed int");
    assert_eq!(
        packed, 80113,
        "golden generation must pack to 80113 = \"mat\""
    );

    // Inline vs streamed: the SAME weights, read from the config vs from `loaded()`. We lower BOTH and compare
    // cell counts to prove the collapse, then run the streamed net with its weights pre-loaded into the heap.
    let src_inline = gen("--fabric").expect("inline fabric program");
    let src_stream = gen_args(&["--fabric", "--stream"]).expect("streamed fabric program");
    let blob = parse_blob(&gen("--blob").expect("weight blob"));

    let cells_inline: usize = serialize_program(&program_cfg_fabric(&src_inline, "generate"))
        .blocks
        .iter()
        .map(|b| b.cells.len())
        .sum();

    // Right-size the per-collection reserve (8, not the 2048 default) so the runtime heap stays ~18k words; the
    // config lowering AND the engine's deep-copy both honour it, so the ≡VM run is self-consistent.
    let (got, steps, heap, cells_stream) = with_seq_reserve(8, || {
        let cfg = serialize_program(&program_cfg_fabric(&src_stream, "generate"));
        let cells: usize = cfg.blocks.iter().map(|b| b.cells.len()).sum();
        let (r, s, h) = eval_wprog_config_counted_heap_preloaded(&cfg, &[], &blob);
        (r, s, h, cells)
    });

    // `loaded()[0]` is the blob length; the runtime allocated PAST the pre-loaded weights.
    assert_eq!(
        blob[0] as usize,
        blob.len() - 1,
        "blob header must be its element count"
    );
    assert!(
        heap.len() > blob.len(),
        "runtime heap must extend past the pre-loaded weight region"
    );

    assert_eq!(
        got, packed,
        "streamed engine diverged from the golden \"mat\" (80113)"
    );
    assert_eq!(got, 80113, "streamed engine must generate \"mat\"");

    // The collapse: streaming the weights removes the giant weight-load block. This is the whole point —
    // without it the config needs block RAM (the hard registered-config-read rework); with it, flip-flops.
    assert!(
        cells_stream < 2000,
        "streamed config must collapse to flip-flop scale, got {cells_stream} cells"
    );
    assert!(
        cells_stream * 10 < cells_inline,
        "streaming must cut cells by >10x ({cells_inline} inline -> {cells_stream} streamed)"
    );
    assert!(
        heap.len() < 25_000,
        "right-sized heap must fit the block-RAM budget, got {} words",
        heap.len()
    );
    println!(
        "★ WHOLE LLM ON THE FABRIC, WEIGHTS STREAMED: config collapses {cells_inline} -> {cells_stream} cells \
         (>{}x) by reading {} pre-loaded weights from loaded(); the streamed net generates \"mat\" ({got}) \
         ≡VM in {steps} block-steps with a right-sized {}-word heap. Config -> flip-flops, heap -> block RAM, \
         one chip.",
        cells_inline / cells_stream.max(1),
        blob.len() - 1,
        heap.len()
    );
}

/// ★ LOCALIZE THE FAILING OP — the bringup-loop differentiator. When a model stage uses a construct with no
/// fabric lowering yet, the compiler returns a **typed, named failing-op** (not a silent wrong number, not a
/// panic to grep) — and the obvious rewrite lowers cleanly. This is the "compiles, runs, diffs, **localizes
/// the failing op**, iterates" step of the agentic bringup loop, made into a guarantee. Demonstrated live by
/// `logicaffeine_compile/examples/bringup_harness.rs`; locked here so the guarantee can't silently rot.
#[test]
fn an_unlowered_op_is_localized_with_a_typed_error() {
    // A stage that counts distinct residues through a `Set` — a collection the fabric cannot lower yet.
    let unlowered = "## To stage (seed: Int) -> Int:\n    Let seen be a new Set of Int.\n    Let i be 0.\n    While i is less than 8:\n        Add ((i + seed) * 7) % 13 to seen.\n        Set i to i + 1.\n    Return length of seen.\n";
    let err = try_program_cfg_fabric(unlowered, "stage")
        .expect_err("a Set has no fabric lowering yet — it must be REFUSED, never mis-run");
    let msg = err.to_string();
    assert!(
        msg.contains("SetAdd") && msg.contains("unsupported"),
        "the failing op must be localized BY NAME so the loop knows exactly what to fix (got: {msg})"
    );
    // The fix the loop applies next — the Set as a table of seen flags over every residue `%` can give (-12..12)
    // — lowers cleanly, runs ≡ VM, and computes what the original computes.
    let fixed = "## To stage (seed: Int) -> Int:\n    Let mutable seen be a new Seq of Int.\n    Let k be 0.\n    While k is less than 25:\n        Push 0 to seen.\n        Set k to k + 1.\n    Let distinct be 0.\n    Let i be 0.\n    While i is less than 8:\n        Let r be ((i + seed) * 7) % 13.\n        If item (r + 13) of seen equals 0:\n            Set item (r + 13) of seen to 1.\n            Set distinct to distinct + 1.\n        Set i to i + 1.\n    Return distinct.\n";
    let pf = try_program_cfg_fabric(fixed, "stage")
        .expect("the fixed stage (Set → seen-flag table) must lower");
    for seed in [-40i64, -3, 0, 1, 5, 12, 1000] {
        let shown = |src: &str| {
            vm_outcome(&format!("{src}\n## Main\n    Show stage({seed}).\n"))
                .output
                .trim()
                .to_string()
        };
        assert_eq!(
            shown(fixed),
            shown(unlowered),
            "seed {seed}: the rewrite must compute what the original computes"
        );
        assert_eq!(
            eval_program_cfg_fabric(&pf, &[seed]).to_string(),
            shown(fixed),
            "seed {seed}: the fixed stage on the fabric ≢ VM"
        );
    }
    println!(
        "LOCALIZE THE FAILING OP: an unlowered construct returns a typed, named failing-op (`SetAdd`), \
         and the rewrite lowers ≡ VM — the agentic loop's compile→localize→fix step, locked."
    );
}

/// A `for-each` over a sequence nothing in the loop changes lowers to the fabric: the compiler gives it the
/// counted loop over the sequence's length that every backend runs, and the fabric runs it ≡ VM. It was the
/// example of an unlowered construct above until it lowered.
#[test]
fn a_for_each_over_a_sequence_lowers_matching_the_vm() {
    let src = "## To stage (seed: Int) -> Int:\n    Let xs be a new Seq of Int.\n    Let i be 0.\n    While i is less than 8:\n        Push ((i + seed) * 7) % 13 to xs.\n        Set i to i + 1.\n    Let s be 0.\n    Repeat for v in xs:\n        Set s to s + v.\n    Return s.\n";
    let pf = try_program_cfg_fabric(src, "stage").expect("a for-each over a Seq lowers");
    for seed in [-40i64, -3, 0, 1, 5, 12, 1000] {
        let vm = vm_outcome(&format!("{src}\n## Main\n    Show stage({seed}).\n")).output;
        assert_eq!(
            eval_program_cfg_fabric(&pf, &[seed]).to_string(),
            vm.trim(),
            "seed {seed}: fabric ≢ VM"
        );
    }
}

/// The transformer's core MAC on the emitted ENGINE RTL under iverilog: the multiply-accumulate lowers to
/// the config-driven engine, loads into the generic reconfigurable engine, and every input reads back ≡ the
/// model ≡ VM — the transformer's matmul kernel executing on the fabric engine's actual gates, no CPU.
/// iverilog-gated. (The full heap-built projection is proven on the engine MODEL above; the engine RTL runs
/// the MAC here + collections/division/float/recursion in `wprog_soc_lock`.)
#[test]
fn transformer_mac_runs_on_the_fabric_engine_rtl_under_iverilog() {
    let inputs: [i64; 6] = [1, 2, 8, 16, 24, 32];
    let pf = program_cfg_fabric(MACDOT, "macdot");
    let cfg = serialize_program(&pf);
    wprog_config_fits_engine(&cfg).unwrap_or_else(|e| panic!("macdot must fit the engine: {e}"));
    // model ≡ VM first (the reference the RTL is checked against)
    let oracle: Vec<i64> = inputs
        .iter()
        .map(|&n| {
            let m = eval_wprog_config(&cfg, &[n]);
            assert_eq!(
                m,
                vm_call(MACDOT, "macdot", n),
                "engine model macdot({n}) != VM"
            );
            m
        })
        .collect();
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_wpe_tformer");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let name = "macdot";
    let im = wprog_config_image(&cfg);
    for (mem, hex) in im.hex_files() {
        std::fs::write(dir.join(format!("wpe_{name}_{mem}.hex")), hex).expect("write config mem");
    }
    let pin: String = inputs
        .iter()
        .map(|&n| format!("{:016x}\n", n as u64))
        .collect();
    let oout: String = oracle
        .iter()
        .map(|&x| format!("{:016x}\n", x as u64))
        .collect();
    std::fs::write(dir.join(format!("wpe_{name}_in.hex")), pin).expect("write in");
    std::fs::write(dir.join(format!("wpe_{name}_out.hex")), oout).expect("write out");

    let eng = emit_wprog_engine_verilog(name, cfg.n_params);
    let tb = emit_wprog_engine_testbench(name, cfg.n_params, inputs.len());
    let srcv = dir.join(format!("wpe_{name}.v"));
    let tbv = dir.join(format!("tb_wpe_{name}.v"));
    let sim = dir.join(format!("sim_wpe_{name}.vvp"));
    std::fs::write(&srcv, &eng).expect("write engine");
    std::fs::write(&tbv, &tb).expect("write tb");
    let build = Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "the engine RTL diverged from the model on the int8 projection:\n{stdout}"
    );
    println!("the int8 projection runs on the fabric ENGINE RTL under iverilog ≡ the model ≡ VM (config-driven, no CPU).");
}
