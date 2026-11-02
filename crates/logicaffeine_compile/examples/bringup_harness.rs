//! bringup_harness — the agentic model-bringup loop, FOR REAL (not a mock): compile → run → diff against
//! reference → LOCALIZE the failing op (typed) → verify byte-exact. This is the loop a
//! "Model Bringup Engineer" role describes ("compiles, runs, diffs against reference, localizes the failing
//! op, and iterates"), run end-to-end on our tiny transformer.
//!
//!   cargo run -q -p logicaffeine-compile --example bringup_harness
//!
//! Everything here is the genuine machinery the locks use — `try_program_cfg_fabric` (the compiler's fabric
//! lowering, with a TYPED failing-op on refusal), `eval_wprog_config` (run on the fabric engine), and the VM /
//! golden trace as the reference. Nothing is faked.
use std::process::Command;

use logicaffeine_compile::codegen_sva::fabric_rtl::try_program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_counted, serialize_program,
};
use logicaffeine_compile::compile::vm_outcome;

/// Run the validated generator (the "reference weights → LOGOS" front of the compiler) in a mode.
fn gen(mode: &str) -> Option<String> {
    let out = Command::new("python3")
        .args(["scripts/gen_tiny_transformer_lg.py", mode])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn rule(title: &str) {
    println!("\n──────────────────────────────────────────────────────────────────────");
    println!("{title}");
    println!("──────────────────────────────────────────────────────────────────────");
}

fn main() {
    println!(
        "AGENTIC MODEL-BRINGUP LOOP  —  compile → run → diff vs reference → localize → verify"
    );
    println!("target: reconfigurable dataflow fabric (no fetch-decode)   reference: the int8 golden trace");

    // ── STEP 0 — the reference (what every diff is taken against; the role's "reference weights") ──
    rule("STEP 0 — establish the reference");
    let reference: i64 = match gen("--packed") {
        Some(s) => s.trim().parse().expect("packed reference int"),
        None => {
            eprintln!(
                "python3 / generator unavailable — the localize step below still runs standalone."
            );
            0
        }
    };
    if reference != 0 {
        println!(
            "reference generation = {reference}  (base-100 packing of tokens [8,1,13] = \"mat\")"
        );
    }

    // ── STEP 1 — a fresh architecture uses a construct with NO fabric lowering yet ──
    // This is the everyday reality of bringup: a new model reaches an op the backend can't lower. The
    // differentiator the listing asks for is that the compiler LOCALIZES it — a typed, actionable failing-op,
    // not a silent wrong number and not a stack trace to grep.
    rule("STEP 1 — compile a stage with an unlowered op (the loop must localize it)");
    let unlowered = "## To stage (seed: Int) -> Int:\n\
        \x20   Let seen be a new Set of Int.\n\
        \x20   Let i be 0.\n\
        \x20   While i is less than 8:\n\
        \x20       Add ((i + seed) * 7) % 13 to seen.\n\
        \x20       Set i to i + 1.\n\
        \x20   Return length of seen.\n";
    match try_program_cfg_fabric(unlowered, "stage") {
        Err(e) => {
            println!("compile REFUSED — localized failing op:");
            println!("    ⮡  {e}");
            println!("→ the loop is pointed straight at the fix (lower `SetAdd`, or rewrite the");
            println!("  Set as a table of seen flags). No human had to bisect a wrong number to find it.");
        }
        Ok(_) => println!("(unexpected: the unlowered variant compiled)"),
    }

    // ── STEP 2 — the fixed net compiles whole to the fabric ──
    rule("STEP 2 — compile the fixed whole net to the fabric");
    let Some(src) = gen("--fabric") else {
        println!(
            "(generator unavailable — skipping the whole-net leg; STEP 1 localization stands.)"
        );
        return;
    };
    let pf = try_program_cfg_fabric(&src, "generate")
        .unwrap_or_else(|e| panic!("the fixed net must lower: {e}"));
    let cfg = serialize_program(&pf);
    let total_cells: usize = cfg.blocks.iter().map(|b| b.cells.len()).sum();
    println!(
        "compiled: the WHOLE transformer (embedding + attention + softmax + FFN + argmax + the generation \
         loop) → {} fabric blocks / {total_cells} cells",
        cfg.blocks.len()
    );

    // ── STEP 3 — run on the fabric, diff against reference ──
    rule("STEP 3 — run on the fabric engine, diff against reference");
    let (fabric, steps) = eval_wprog_config_counted(&cfg, &[]);
    let vm = vm_outcome(&src);
    let vm_val: i64 = vm.output.trim().parse().expect("VM Show int");
    println!("fabric engine result = {fabric}   ({steps} block-steps = the whole-program fabric's cycle count)");
    println!("VM (independent oracle) = {vm_val}");
    println!("reference (golden)      = {reference}");
    let byte_exact = fabric == vm_val && fabric == reference;
    println!(
        "diff: {}",
        if byte_exact {
            "MATCH ✓ — byte-identical, fabric ≡ VM ≡ reference"
        } else {
            "MISMATCH ✗"
        }
    );
    assert!(byte_exact, "bringup diff failed");

    rule("BRINGUP COMPLETE");
    println!("first correct tokens on the fabric — the whole net, generated \"mat\", proven byte-identical");
    println!("to reference. Correctness first (proven, not spot-checked); the failing op was localized for you.");

    // ── STEP 4 — then optimize: as few cycles as the board allows (correctness was first) ──
    rule("STEP 4 — as few cycles as the board allows (spatial)");
    let macs = 50_624i64; // matmul-dominated; see tiny_transformer_fabric.json / the fabric lock
    let seq_per_token = steps as i64 / 3; // this run's block-steps for the 3-token generation, per token
    println!("this run (sequential whole-program engine): {seq_per_token} cycles/token");
    println!("a P-wide spatial MAC array does P MACs/clock → ceil({macs}/P) cycles/token:");
    for p in [240i64, 512, 1024] {
        let cyc = (macs + p - 1) / p;
        let label = if p == 240 {
            "DSP-bound (240 DSP48 on the XC7A100T)"
        } else {
            "int8 LUT-multipliers"
        };
        println!(
            "   P={p:5}  →  {cyc:5} cycles/token   ({}× fewer than sequential)   [{label}]",
            seq_per_token / cyc
        );
    }
    println!("1 cycle would need {macs} multipliers — impossible on a $300 board; ~50–211 is the honest floor,");
    println!("the same spatial/pipelined tradeoff a real RDU makes. (1-MAC/clock MEASURED under iverilog;");
    println!(
        "the hundreds-of-PE array is realized on real silicon by the on-board 8×8 GEMM = 510720.)"
    );
}
