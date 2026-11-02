//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TRANSFORMER-FABRIC LOCK — the baby transformer's hot compute runs on the certified reconfigurable
//! dataflow fabric, and the cycle numbers are the headline: a MAC-wave is ONE clock.
//!
//! The transformer's dominant work is int8 matmul (Q/K/V/out/FFN projections + attention dots), whose
//! kernel is the multiply-accumulate `acc += a·b`, followed by a fixed-point requant `(acc·M) >> S`.
//! This lock proves BOTH lower to the fabric and are byte-identical to the VM, and MEASURES the cycle
//! cost — the property a dataflow-chip buyer interrogates ("is it one cycle?"):
//!
//!   Tier 1 — MAC ≡ VM on the loop fabric. The int8 dot-product lowers to the reconfigurable loop fabric
//!            (`loop_fabric_from_source`), emits a real `Mul`+`Add` tile, ≡ VM across a battery.
//!   Tier 1 — REQUANT ≡ VM on the residual fabric. `(acc·M) >> S` lowers to a straight-line dataflow
//!            circuit (`residual_from_source`), emits a `Mul`+`Shr` tile, ≡ VM — a COMBINATIONAL wave.
//!   Tier 2 — PLACED ≡ DATAPATH by our own SAT. Both the MAC datapath and the requant datapath, placed
//!            onto the tile array (`route_loop_body`), are certified equal to the abstract datapath for
//!            ALL inputs (bit-blasted, `prove_unsat`).
//!   NUMBERS — MEASURED 1 MAC/CLOCK. The MAC dot emitted as a CGRA (`emit_loop_fabric_verilog`) runs under
//!            iverilog in EXACTLY `len` cycles for a length-`len` dot (body = circuit, loop = counter):
//!            one MAC per clock. The cycle ledger (per-MAC fabric vs soft-core, whole-token projection) this lock
//!            derives is the committed `benchmarks/silicon/tiny_transformer_fabric.json`: a run compares it, and
//!            `scripts/test/gen-silicon-goldens.sh ledger` writes it.
//!
//! Reuses the machinery certified by `gemm_fabric_lock` / `verified_placer_lock`. iverilog-gated: the
//! cycle-measurement leg skips cleanly (loud note) where the simulator is unavailable.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;
#[path = "common/golden.rs"]
mod golden;

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::codegen_sva::bitblast::lower_bool;
use logicaffeine_compile::codegen_sva::fabric_grid::{grid_output_to_bounded, route_loop_body};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_loop_fabric_verilog, emit_loop_fabric_verilog_int8, eval_fabric, eval_loop_fabric,
    eval_loop_fabric_seq, fabric_to_bounded, loop_fabric_from_source, residual_from_source, AluOp,
    FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

// ── the two hot kernels, in LOGOS surface form ──────────────────────────────────────────────────────

/// The int8 MAC dot-product (the projection inner kernel): `dot(n) = Σ_{k=1..n} k·(n-k)` — a genuine
/// multiply-accumulate over int8-range operands. Same kernel `gemm_fabric_lock` certifies; here we
/// MEASURE its cycles at the transformer's dot lengths (head_dim, d_model).
const DOT_FN: &str = "## To dot (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for k from 1 to n:\n        Set s to s + k * (n - k).\n    Return s.\n";

/// The fixed-point requant that follows every matmul: `requant(acc, m) = (acc·m) >> 24` — a Mul then an
/// arithmetic Shr, the exact integer op the int8 model uses (`scripts/train_tiny_transformer.py`).
const REQUANT_FN: &str =
    "## To requant (acc: Int, m: Int) -> Int:\n    Return (acc * m) shifted right by 24.\n";

fn dot_ref(n: i64) -> i64 {
    (1..=n).map(|k| k * (n - k)).sum()
}
fn vm_call(src: &str, fname: &str, args: &[i64]) -> String {
    let argstr = args
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let full = format!("{src}## Main\n    Show {fname}({argstr}).\n");
    let out = vm_outcome(&full);
    assert!(out.error.is_none(), "{fname}: VM error {:?}", out.error);
    out.output.trim().to_string()
}
fn body_has_mul(body: &[LoopCell]) -> bool {
    body.iter()
        .any(|c| matches!(c, LoopCell::Arith { op: AluOp::Mul, .. }))
}
fn body_has_add(body: &[LoopCell]) -> bool {
    body.iter()
        .any(|c| matches!(c, LoopCell::Arith { op: AluOp::Add, .. }))
}
fn to_cells(body: &[FabricOp]) -> Vec<LoopCell> {
    body.iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect()
}

/// Certify `eval_grid(route_loop_body(body)) == eval_fabric(body)` for ALL inputs at width `w` by SAT.
fn placement_is_certified(body: &[FabricOp], n_in: usize, result: usize, w: u32) -> bool {
    let cells = to_cells(body);
    let g = route_loop_body(&cells, n_in, &[Src::Val(result)]);
    let grid = grid_output_to_bounded(&g, 0, w, &|k| format!("in{k}"));
    let datapath = fabric_to_bounded(body, result, w);
    let claim = BoundedExpr::Eq(Box::new(grid), Box::new(datapath));
    let p = lower_bool(&claim).expect("claim lowers to a bounded formula");
    prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted
}

// ════════════════════════════════════════════════════════════════════════════════════════════════

/// Tier 1 — the int8 MAC dot lowers to the reconfigurable loop fabric, emits a real Mul + Add tile, and
/// evaluates byte-identically to the VM across a battery (the transformer's projection inner kernel).
#[test]
fn int8_mac_dot_lowers_to_the_fabric_matching_the_vm() {
    let lf = loop_fabric_from_source(DOT_FN, "dot");
    assert!(
        body_has_mul(&lf.body) && body_has_add(&lf.body),
        "the MAC body must emit a Mul tile and an Add tile"
    );
    for n in [1i64, 2, 8, 16, 24] {
        let got = eval_loop_fabric(&lf, &[n]);
        assert_eq!(got, dot_ref(n), "fabric dot({n}) != oracle");
        assert_eq!(
            got.to_string(),
            vm_call(DOT_FN, "dot", &[n]),
            "fabric dot({n}) != VM"
        );
    }
    println!("Tier 1: the int8 MAC dot lowers to the loop fabric (Mul+Add) ≡ VM ≡ oracle.");
}

/// Tier 1 — the fixed-point requant `(acc·M) >> S` lowers to a straight-line dataflow circuit, emits a
/// Mul + Shr tile, and evaluates byte-identically to the VM (a COMBINATIONAL wave — settles in one clock).
#[test]
fn requant_lowers_to_the_fabric_matching_the_vm() {
    let (ops, result, n_in) = residual_from_source(REQUANT_FN, "requant");
    assert_eq!(n_in, 2, "requant takes (acc, m)");
    assert!(
        ops.iter().any(|o| matches!(o.op, AluOp::Mul)),
        "requant must emit a Mul tile"
    );
    assert!(
        ops.iter().any(|o| matches!(o.op, AluOp::Shr)),
        "requant must emit an arithmetic Shr tile"
    );
    for &(acc, m) in &[
        (0i64, 1717986918),
        (12345, 1717986918),
        (-4096, 590000),
        (1_000_000, 33_000),
    ] {
        let got = eval_fabric(&ops, &[acc, m], result);
        assert_eq!(
            got,
            (acc * m) >> 24,
            "fabric requant != oracle at (acc {acc}, m {m})"
        );
        assert_eq!(
            got.to_string(),
            vm_call(REQUANT_FN, "requant", &[acc, m]),
            "fabric requant != VM"
        );
    }
    println!("Tier 1: the fixed-point requant (acc·M)>>S lowers to the residual fabric (Mul+Shr) ≡ VM (combinational).");
}

/// Tier 2 — both hot datapaths, placed-and-routed onto the tile array, are certified EQUAL to the
/// abstract datapath for ALL inputs by our own SAT: the MAC `acc + a·b`, and the requant `(acc·m) >> S`.
#[test]
fn hot_datapaths_placements_are_certified_semantics_preserving() {
    // MAC: In0=a, In1=b, In2=acc; cell0=a*b; cell1=acc+cell0; result=cell1.
    let mac = vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(1),
        },
        FabricOp {
            op: AluOp::Add,
            a: Src::In(2),
            b: Src::Val(0),
        },
    ];
    assert!(
        placement_is_certified(&mac, 3, 1, 8),
        "MAC placement must be certified over all int8 inputs"
    );

    // requant routing: Mul then arithmetic Shr. We certify the Mul→Shr PLACEMENT (the routing Tier 2 is
    // about) at width 8 with a small shift so real bits are exercised; the model's `>>24` shares this
    // exact routing (only the constant differs). In0=acc, In1=m; cell0=acc*m; cell1=cell0>>3; result=1.
    let requant = vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(1),
        },
        FabricOp {
            op: AluOp::Shr,
            a: Src::Val(0),
            b: Src::Const(3),
        },
    ];
    assert!(
        placement_is_certified(&requant, 2, 1, 8),
        "requant Mul→Shr placement must be certified semantics-preserving"
    );
    println!("Tier 2: the MAC and requant datapaths placed on the tile array are certified ≡ the datapath by SAT.");
}

// ── NUMBERS: measured 1 MAC/clock on the fabric + the whole-token cycle ledger ──────────────────────

/// Build a cgra testbench that drives `in0=n`, resets, counts core clocks until `done`, checks `result`.
fn cgra_tb(module: &str) -> String {
    format!(
        "`timescale 1ns/1ps\nmodule tb;\n  reg clk=0, rst; reg [63:0] in0; wire [63:0] result; wire done;\n  integer c;\n  {module} dut(.clk(clk), .rst(rst), .in0(in0), .result(result), .done(done));\n  always #5 clk=~clk;\n  initial begin\n    in0=64'd0;\n    if (!$value$plusargs(\"N=%d\", in0)) in0=64'd0;\n    @(negedge clk); rst=1; @(negedge clk); rst=0; c=0;\n    while (!done && c<100000) begin @(negedge clk); c=c+1; end\n    $display(\"RESULT: %0d CYCLES=%0d\", $signed(result), c);\n    $finish;\n  end\nendmodule\n"
    )
}

/// NUMBERS — the headline, MEASURED: the int8 MAC dot emitted as a CGRA runs in EXACTLY `len` cycles for
/// a length-`len` dot (one MAC per clock; body = circuit, loop = counter), for the transformer's dot
/// lengths (head_dim=8, d_model=16, and 24). Proven ≡ VM at each length. iverilog-gated.
#[test]
fn fabric_computes_one_mac_per_clock_measured() {
    if !common::require_tool("iverilog") {
        return;
    }
    let lf = loop_fabric_from_source(DOT_FN, "dot");
    let rtl = emit_loop_fabric_verilog("dotmac", &lf);
    let tmp = std::env::temp_dir().join("logos_tformer_fabric");
    std::fs::create_dir_all(&tmp).expect("tmp");
    let vsrc = tmp.join("dotmac.v");
    std::fs::write(&vsrc, format!("{rtl}\n{}", cgra_tb("cgra_dotmac"))).expect("write rtl+tb");
    let sim = tmp.join("dotmac.vvp");
    let build = Command::new("iverilog")
        .args(["-g2012", "-s", "tb", "-o"])
        .arg(&sim)
        .arg(&vsrc)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "iverilog build failed:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    for &len in &[8i64, 16, 24] {
        let run = Command::new("vvp")
            .arg(&sim)
            .arg(format!("+N={len}"))
            .output()
            .expect("vvp");
        let out = String::from_utf8_lossy(&run.stdout);
        let line = out
            .lines()
            .find_map(|l| l.trim().strip_prefix("RESULT:"))
            .unwrap_or_else(|| panic!("no RESULT:\n{out}"));
        let (res, cyc) = line.split_once("CYCLES").expect("CYCLES field");
        let result: i64 = res.trim().parse().expect("result int");
        let cycles: i64 = cyc
            .trim_start_matches('=')
            .trim()
            .parse()
            .expect("cycles int");
        assert_eq!(result, dot_ref(len), "cgra dot({len}) result != oracle");
        // "body = circuit, loop = counter": one loop-body wave per clock => EXACTLY `len` cycles for a
        // length-`len` dot. The per-MAC rate is exactly 1 clock.
        assert_eq!(
            cycles, len,
            "length-{len} dot must run in exactly {len} cycles (1 MAC/clock), got {cycles}"
        );
        println!("  measured: length-{len} int8 dot = {cycles} cycles on the fabric (exactly 1 cyc/MAC).");
    }
    println!("NUMBERS: the fabric computes ONE MAC per clock (measured under iverilog) — a MAC-wave is one cycle.");
}

const LEDGER: &str = "benchmarks/silicon/tiny_transformer_fabric.json";
const LEDGER_REGEN: &str = "scripts/test/gen-silicon-goldens.sh ledger";

/// The cycle ledger this lock derives, whole: the per-token cycle projection, the throughput by unroll, and the
/// whole-net spatial floor. One derivation, so the committed document cannot depend on which test ran last.
fn ledger() -> serde_json::Value {
    let mut ledger = token_cycle_ledger();
    ledger["throughput"] = throughput_section();
    ledger["whole_net_spatial"] = whole_net_section();
    ledger
}

/// NUMBERS — the whole-token cycle ledger, from the model dims (tiny_transformer.json) and the MEASURED per-MAC
/// rates (fabric = 1 clock/MAC here; soft-core = 10.6 clock/MAC from gemm_fabric.json), is the committed
/// `benchmarks/silicon/tiny_transformer_fabric.json`.
#[test]
fn the_cycle_ledger_is_the_one_this_lock_derives() {
    golden::committed_json(LEDGER, &ledger(), LEDGER_REGEN);
}

/// The per-token matmul cycle projection: MAC counts by stage at the deepest context length, the measured
/// cycles per MAC, and the projected cycles per token.
fn token_cycle_ledger() -> serde_json::Value {
    let jpath = root().join("benchmarks/silicon/tiny_transformer.json");
    let raw = std::fs::read_to_string(&jpath).expect("read tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&raw).expect("parse");
    let cfg = &j["config"];
    let d = cfg["d_model"].as_u64().unwrap() as i64;
    let h = cfg["n_heads"].as_u64().unwrap() as i64;
    let dff = cfg["d_ff"].as_u64().unwrap() as i64;
    let v = cfg["vocab"].as_u64().unwrap() as i64;
    let hd = d / h;
    let g = &j["golden_trace"];
    let ctx = g["steps"].as_array().unwrap().last().unwrap()["context_len"]
        .as_i64()
        .unwrap();
    let n = ctx; // the deepest generation step

    // MAC count for one forward pass at context length n (matmuls only — the fabric work):
    let qkv = 3 * n * d * d;
    let scores = h * (n * (n + 1) / 2) * hd; // causal Q·Kᵀ dots
    let ctxsum = h * (n * (n + 1) / 2) * hd; // attention·V
    let wo = n * d * d;
    let ffn = n * d * dff + n * dff * d;
    let logits = v * d; // last position only
    let macs = qkv + scores + ctxsum + wo + ffn + logits;

    let fabric_cyc_per_mac = 1.0; // MEASURED here
    let softcore_cyc_per_mac = 10.625; // MEASURED in gemm_fabric.json (scalar soft-core)
    let ledger = serde_json::json!({
        "meta": {
            "artifact": "baby-transformer fabric cycle ledger",
            "note": "per-MAC rates are MEASURED (fabric 1 clock/MAC via transformer_fabric_lock::fabric_computes_one_mac_per_clock_measured, run under iverilog; soft-core 10.625 clock/MAC from gemm_fabric.json). The whole-token cycle figures are PROJECTED = matmul_MACs * cyc_per_mac / unroll. No wall-clock claim vs any vendor part.",
        },
        "model": {"d_model": d, "n_heads": h, "d_ff": dff, "vocab": v, "context_len": n},
        "matmul_macs_per_token": {
            "qkv": qkv, "attention_scores": scores, "attention_weighted_sum": ctxsum,
            "out_proj": wo, "ffn": ffn, "logits": logits, "total": macs
        },
        "measured_cyc_per_mac": {"fabric_unroll1": fabric_cyc_per_mac, "soft_core_scalar": softcore_cyc_per_mac},
        "projected_token_matmul_cycles": {
            "fabric_unroll1": (macs as f64 * fabric_cyc_per_mac) as i64,
            "fabric_unroll8": (macs as f64 * fabric_cyc_per_mac / 8.0) as i64,
            "soft_core_scalar": (macs as f64 * softcore_cyc_per_mac) as i64,
        },
        "fabric_vs_softcore_cycle_reduction_x": softcore_cyc_per_mac / fabric_cyc_per_mac,
    });
    println!(
        "NUMBERS: token matmul = {macs} MACs → fabric {} cyc (1/clock), soft-core {} cyc (10.6/MAC) = {:.0}× fewer cycles.",
        macs, (macs as f64 * softcore_cyc_per_mac) as i64, softcore_cyc_per_mac
    );
    ledger
}

// ── CRANK TOKENS: spatial unroll → u MACs/clock → token throughput ─────────────────────────────────

fn count_muls(body: &[LoopCell]) -> usize {
    body.iter()
        .filter(|c| matches!(c, LoopCell::Arith { op: AluOp::Mul, .. }))
        .count()
}

/// A `u`-wide unrolled int8 dot: each iteration accumulates `u` INDEPENDENT products, so the spatial fabric
/// places `u` multiply tiles that all fire in one feed-forward wave — `u` MACs/clock (the throughput lever).
fn dot_unrolled_fn(u: usize) -> String {
    let terms: Vec<String> = (0..u)
        .map(|j| {
            if j == 0 {
                "k * (n - k)".into()
            } else {
                format!("(k + {j}) * (n - k - {j})")
            }
        })
        .collect();
    format!(
        "## To dotU (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for k from 1 to n:\n        Set s to s + {}.\n    Return s.\n",
        terms.join(" + ")
    )
}
fn dot_unrolled_ref(u: i64, n: i64) -> i64 {
    (1..=n)
        .map(|k| (0..u).map(|j| (k + j) * (n - k - j)).sum::<i64>())
        .sum()
}

/// CRANK TOKENS — the spatial fabric scales to `u` MACs/clock (proven ≡ VM as the unroll grows 1→16),
/// and the ledger records the resulting token throughput: at unroll `u` a token's 50,624 matmul MACs
/// take `50624/u` fabric cycles, so at a given clock the fabric CRANKS `clock / cycles` tokens/second — the
/// fastest path. Per-MAC = 1 clock is MEASURED (fabric_computes_one_mac_per_clock_measured); tokens/sec is
/// PROJECTED = clock / (MACs/unroll) and labeled as such. No wall-clock claim vs any vendor part.
#[test]
fn spatial_unroll_cranks_tokens_and_the_ledger_records_the_throughput() {
    // (a) spatial parallelism scales: unroll u places u multiply tiles (u MACs/clock), ≡ VM.
    for u in [1usize, 2, 4, 8, 16] {
        let lf = loop_fabric_from_source(&dot_unrolled_fn(u), "dotU");
        assert_eq!(
            count_muls(&lf.body),
            u,
            "unroll×{u} must place {u} multiply tiles (u MACs/clock)"
        );
        for n in [4i64, 8, 16] {
            assert_eq!(
                eval_loop_fabric(&lf, &[n]),
                dot_unrolled_ref(u as i64, n),
                "unroll×{u} dotU({n}) != oracle"
            );
        }
    }

    // (b) the committed ledger's throughput section is the one derived from the model and the measured rate.
    assert_eq!(
        golden::committed_json_value(LEDGER, LEDGER_REGEN)["throughput"],
        throughput_section(),
        "{LEDGER}: the throughput section is not what this lock derives — regenerate with `{LEDGER_REGEN}`"
    );
}

/// The throughput section: from the model's per-token matmul MAC count and the MEASURED 1-MAC/clock rate, the
/// token rate at each unroll and cited clock.
fn throughput_section() -> serde_json::Value {
    let jpath = root().join("benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&jpath).unwrap()).unwrap();
    let cfg = &j["config"];
    let (d, h, dff, v) = (
        cfg["d_model"].as_i64().unwrap(),
        cfg["n_heads"].as_i64().unwrap(),
        cfg["d_ff"].as_i64().unwrap(),
        cfg["vocab"].as_i64().unwrap(),
    );
    let hd = d / h;
    let n = j["golden_trace"]["steps"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["context_len"]
        .as_i64()
        .unwrap();
    let macs =
        3 * n * d * d + 2 * (h * (n * (n + 1) / 2) * hd) + n * d * d + 2 * n * d * dff + v * d;

    // clocks we can honestly cite: the board core clock (6.25 MHz, measured on-silicon for the full core),
    // the decision-fabric measured clock (66.67 MHz), and the 100 MHz board oscillator.
    let clocks: [(&str, f64); 3] = [
        ("core_6p25MHz", 6.25e6),
        ("fabric_66MHz", 66.67e6),
        ("board_100MHz", 100.0e6),
    ];
    // spatial unroll = MACs computed per clock. u MACs/clock is proven ≡VM up to 16 above; the ledger
    // projects the linear scaling to wider fabrics (a systolic array is hundreds of MACs). "As fast as
    // possible" = the highest unroll the area/Fmax budget allows; the fully-spatial limit is the whole
    // matmul in one wave (all 50,624 MACs, area-bound — the theoretical ceiling, noted not claimed).
    let unrolls = [1i64, 8, 16, 32, 64, 128];
    let mut tp = serde_json::Map::new();
    let mut best = 0.0f64;
    for &u in &unrolls {
        let cyc = (macs + u - 1) / u; // ceil(MACs/u) fabric cycles for the token's matmuls at u MACs/clock
        let mut per = serde_json::Map::new();
        per.insert("matmul_cycles_per_token".into(), serde_json::json!(cyc));
        for (nm, hz) in clocks {
            let tps = hz / cyc as f64;
            per.insert(
                format!("tokens_per_sec_{nm}"),
                serde_json::json!(tps.round() as i64),
            );
            best = best.max(tps);
        }
        tp.insert(format!("unroll_{u}"), serde_json::Value::Object(per));
    }
    println!(
        "CRANK TOKENS: spatial fabric scales to u MACs/clock; a token's {macs} matmul-MACs → up to \
         {} tokens/sec (unroll {} @ 100 MHz, projected from the measured 1-MAC/clock rate).",
        best.round() as i64,
        unrolls.last().unwrap()
    );
    serde_json::json!({
        "note": "per-MAC = 1 clock is MEASURED (fabric_computes_one_mac_per_clock_measured, iverilog). tokens/sec \
                 is PROJECTED = clock / (matmul_MACs / unroll) for the matmul work (softmax/argmax orchestration \
                 extra). Unroll places u independent multiply tiles = u MACs/clock (proven ≡VM above). No wall-clock \
                 claim vs any vendor part; achievable Fmax at high unroll needs a place-and-route measurement.",
        "matmul_macs_per_token": macs,
        "by_unroll": serde_json::Value::Object(tp),
        "headline_tokens_per_sec": best.round() as i64,
    })
}

/// ★ THE SPATIAL MAC ARRAY, AUTO-EMITTED FROM LOGOS (the generalization to any model). A P-wide int8
/// MAC array is NOT hand-written RTL — it is emitted by the compiler from a LOGOS dot
/// (`loop_fabric_from_source` → `emit_loop_fabric_verilog`). This is the substrate a bitstream is built from
/// (`scripts/synth-fabric-matmul.sh`): "write the matmul in LOGOS, the compiler emits the fabric, it
/// synthesizes." This proves the AUTO-EMITTED array runs byte-identical to the reference and does **P MACs per
/// clock** under iverilog, for P = 1,2,4,8 (the width is a knob, so the same path scales to a real model).
#[test]
fn spatial_mac_array_emitted_from_logos_does_p_macs_per_clock_under_iverilog() {
    if !common::require_tool("iverilog") {
        // The compiler still EMITS the array from LOGOS (no simulator needed) — assert that much.
        for p in [1usize, 2, 4, 8] {
            let lf = loop_fabric_from_source(&dot_unrolled_fn(p), "dotU");
            assert_eq!(
                count_muls(&lf.body),
                p,
                "the emitted array must place {p} multiply tiles"
            );
            assert_eq!(
                eval_loop_fabric(&lf, &[16]),
                dot_unrolled_ref(p as i64, 16),
                "P={p} array != oracle (model)"
            );
        }
        return;
    }
    let tmp = std::env::temp_dir().join("logos_matmul_array");
    std::fs::create_dir_all(&tmp).unwrap();
    for p in [1usize, 2, 4, 8] {
        // AUTO-EMIT the P-wide array from LOGOS — no hand-written RTL.
        let lf = loop_fabric_from_source(&dot_unrolled_fn(p), "dotU");
        assert_eq!(
            count_muls(&lf.body),
            p,
            "the emitted P={p} array must place {p} multiply tiles (P MACs/clock)"
        );
        let name = format!("arr{p}");
        let rtl = emit_loop_fabric_verilog(&name, &lf);
        let vsrc = tmp.join(format!("{name}.v"));
        std::fs::write(
            &vsrc,
            format!("{rtl}\n{}", cgra_tb(&format!("cgra_{name}"))),
        )
        .unwrap();
        let sim = tmp.join(format!("{name}.vvp"));
        let build = Command::new("iverilog")
            .args(["-g2012", "-s", "tb", "-o"])
            .arg(&sim)
            .arg(&vsrc)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "iverilog build failed for P={p}:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        for &n in &[8i64, 16] {
            let run = Command::new("vvp")
                .arg(&sim)
                .arg(format!("+N={n}"))
                .output()
                .unwrap();
            let out = String::from_utf8_lossy(&run.stdout);
            let line = out
                .lines()
                .find_map(|l| l.trim().strip_prefix("RESULT:"))
                .unwrap_or_else(|| panic!("no RESULT:\n{out}"));
            let (res, cyc) = line.split_once("CYCLES").unwrap();
            let result: i64 = res.trim().parse().unwrap();
            let cycles: i64 = cyc.trim_start_matches('=').trim().parse().unwrap();
            assert_eq!(
                result,
                dot_unrolled_ref(p as i64, n),
                "P={p} emitted array dot({n}) != oracle"
            );
            // n loop iterations, each ONE feed-forward wave doing P MACs → n cycles, P MACs/clock.
            assert_eq!(
                cycles, n,
                "P={p} length-{n} dot must run in {n} cycles (one wave/iter), got {cycles}"
            );
        }
        println!("  P={p}: MAC array AUTO-EMITTED from LOGOS runs ≡ oracle under iverilog — {p} MACs/clock.");
    }
    println!("★ SPATIAL MAC ARRAY AUTO-EMITTED FROM LOGOS: P MACs/clock, byte-identical — the bitstream substrate (no hand-written RTL).");
}

/// ★ OPERAND-FED int8 DOT PRODUCT — a REAL two-vector matmul kernel from LOGOS, run on gates. The two vectors
/// are streamed INTERLEAVED [a0,b0,a1,b1,…] into the array's on-chip buffer (MemRead), and a parity+temp carry
/// makes the single operand read a pairwise MAC — exactly ONE int8 multiply per pair (one DSP48E1), the parity
/// gate a Select. Emitted from LOGOS with the int8 datapath, the buffer loaded via the write port, it runs ≡ the
/// model ≡ the a·b oracle under iverilog. Proves `LoopCell::MemRead` RTL emission + that a genuine
/// vector·vector dot lowers whole to the fabric — the compute core of `nexys_matmul_soc`. iverilog-gated.
#[test]
fn operand_fed_int8_dot_product_runs_from_a_loaded_buffer_under_iverilog() {
    const DOT: &str = "## To dot (xs: Seq of Int) -> Int:\n    Let acc be 0.\n    Let tmp be 0.\n    Let odd be 0.\n    Repeat for x in xs:\n        Let prod be tmp * x.\n        If odd is 1:\n            Set acc to acc + prod.\n        Set tmp to x.\n        Set odd to 1 - odd.\n    Return acc.\n";
    let lf = loop_fabric_from_source(DOT, "dot");
    assert_eq!(
        lf.seq_input,
        Some(0),
        "dot iterates a streamed (interleaved) vector"
    );
    assert!(
        lf.body
            .iter()
            .any(|c| matches!(c, LoopCell::MemRead { .. })),
        "must read operands via MemRead"
    );
    assert_eq!(
        lf.body
            .iter()
            .filter(|c| matches!(c, LoopCell::Arith { op: AluOp::Mul, .. }))
            .count(),
        1,
        "a real dot must lower to exactly ONE int8 multiply (one DSP48E1 per MAC)"
    );
    let rtl = emit_loop_fabric_verilog_int8("dot", &lf, 8);
    assert!(
        rtl.contains("reg signed [63:0] mem"),
        "a seq loop must emit an operand buffer"
    );
    assert!(rtl.contains("mem[amr"), "MemRead must read mem[addr]");

    // interleave two int8 vectors → a·b oracle
    let dot = |a: &[i64], b: &[i64]| -> (Vec<i64>, i64) {
        let xs: Vec<i64> = a.iter().zip(b).flat_map(|(&x, &y)| [x, y]).collect();
        (xs, a.iter().zip(b).map(|(&x, &y)| x * y).sum())
    };
    let cases: [(&[i64], &[i64]); 3] = [
        (&[3, -4, 5], &[6, 7, -8]),
        (&[10, -7, 25, -3], &[100, -128, 127, -1]),
        (&[42], &[9]),
    ];
    // model leg (always): the fabric ≡ the a·b oracle.
    for (a, b) in cases {
        let (xs, oracle) = dot(a, b);
        assert_eq!(
            eval_loop_fabric_seq(&lf, &[], &xs),
            oracle,
            "fabric a·b != oracle for {a:?}·{b:?}"
        );
    }
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_operand_fed_dot");
    std::fs::create_dir_all(&dir).unwrap();
    let tb = "`timescale 1ns/1ps\nmodule tb;\n  reg clk=0, rst; reg [63:0] seq_len; reg mem_we; reg [8:0] mem_waddr; reg signed [63:0] mem_wdata;\n  wire [63:0] result; wire done; integer i, c; integer L; reg signed [63:0] vec [0:255];\n  cgra_dot dut(.clk(clk), .rst(rst), .seq_len(seq_len), .mem_we(mem_we), .mem_waddr(mem_waddr), .mem_wdata(mem_wdata), .result(result), .done(done));\n  always #5 clk=~clk;\n  initial begin\n    if (!$value$plusargs(\"L=%d\", L)) L=0;\n    $readmemh(\"vec.hex\", vec);\n    rst=1; mem_we=0; seq_len=0; @(negedge clk);\n    for (i=0;i<L;i=i+1) begin mem_we=1; mem_waddr=i[8:0]; mem_wdata=vec[i]; @(negedge clk); end\n    mem_we=0; seq_len=L; @(negedge clk);\n    rst=1; @(negedge clk); rst=0; c=0;\n    while (!done && c<100000) begin @(negedge clk); c=c+1; end\n    $display(\"RESULT: %0d CYCLES=%0d\", $signed(result), c);\n    $finish;\n  end\nendmodule\n";
    let vsrc = dir.join("dot.v");
    std::fs::write(&vsrc, format!("{rtl}\n{tb}")).unwrap();
    let sim = dir.join("dot.vvp");
    let build = Command::new("iverilog")
        .args(["-g2012", "-s", "tb", "-o"])
        .arg(&sim)
        .arg(&vsrc)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "iverilog:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );
    for (a, b) in cases {
        let (xs, oracle) = dot(a, b);
        let hex: String = xs.iter().map(|&x| format!("{:016x}\n", x as u64)).collect();
        std::fs::write(dir.join("vec.hex"), hex).unwrap();
        let run = Command::new("vvp")
            .arg(&sim)
            .arg(format!("+L={}", xs.len()))
            .current_dir(&dir)
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&run.stdout);
        let line = out
            .lines()
            .find_map(|l| l.trim().strip_prefix("RESULT:"))
            .unwrap_or_else(|| panic!("no RESULT:\n{out}"));
        let (res, cyc) = line.split_once("CYCLES").unwrap();
        let got: i64 = res.trim().parse().unwrap();
        let cycles: i64 = cyc.trim_start_matches('=').trim().parse().unwrap();
        assert_eq!(got, oracle, "RTL a·b != oracle on {a:?}·{b:?}");
        assert_eq!(
            cycles,
            xs.len() as i64,
            "dot must run in (2·pairs) cycles (1 MAC/clock)"
        );
    }
    println!("OPERAND-FED int8 DOT PRODUCT (two vectors interleaved, MemRead from a loaded buffer) runs ≡ model ≡ a·b oracle under iverilog — the matmul-SoC compute core, from LOGOS.");
}

/// ★ INT8-NATIVE DATAPATH — byte-identical, and one DSP48E1 per MAC. The 64-bit fabric multiply decomposes
/// into ~10 DSPs (synth-measured: the 8-wide array = 80 DSP48E1); an int8-native datapath emits each multiply
/// with 8-bit signed operands (`emit_loop_fabric_verilog_int8`), which maps to a SINGLE DSP48E1 and runs
/// faster — while the accumulator stays 64-bit so sums are exact. This proves the int8-datapath array is
/// BYTE-IDENTICAL to the 64-bit array (and the VM oracle) under iverilog for P = 1,2,4,8: the narrow multiply
/// is exact for the int8-range operands the model lives in. (The DSP-count win is checked by
/// `scripts/synth-fabric-matmul.sh <P> int8` at the synth stage.)
#[test]
fn int8_native_datapath_array_is_byte_identical_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let tmp = std::env::temp_dir().join("logos_int8_datapath");
    std::fs::create_dir_all(&tmp).unwrap();
    for p in [1usize, 2, 4, 8] {
        let lf = loop_fabric_from_source(&dot_unrolled_fn(p), "dotU");
        // AUTO-EMIT the same array with an INT8-NATIVE datapath (8-bit signed multiply operands → 1 DSP/MAC).
        let name = format!("i8arr{p}");
        let rtl = emit_loop_fabric_verilog_int8(&name, &lf, 8);
        // the emitted RTL must actually narrow the multiply operands (the DSP-packable form).
        assert!(
            rtl.contains("wire signed [7:0] ma"),
            "int8 datapath must emit 8-bit signed multiply operands"
        );
        let vsrc = tmp.join(format!("{name}.v"));
        std::fs::write(
            &vsrc,
            format!("{rtl}\n{}", cgra_tb(&format!("cgra_{name}"))),
        )
        .unwrap();
        let sim = tmp.join(format!("{name}.vvp"));
        let build = Command::new("iverilog")
            .args(["-g2012", "-s", "tb", "-o"])
            .arg(&sim)
            .arg(&vsrc)
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "iverilog build failed for int8 P={p}:\n{}",
            String::from_utf8_lossy(&build.stderr)
        );
        for &n in &[8i64, 16] {
            let run = Command::new("vvp")
                .arg(&sim)
                .arg(format!("+N={n}"))
                .output()
                .unwrap();
            let out = String::from_utf8_lossy(&run.stdout);
            let line = out
                .lines()
                .find_map(|l| l.trim().strip_prefix("RESULT:"))
                .unwrap_or_else(|| panic!("no RESULT:\n{out}"));
            let (res, _cyc) = line.split_once("CYCLES").unwrap();
            let result: i64 = res.trim().parse().unwrap();
            // BYTE-IDENTICAL to the 64-bit oracle — the narrow multiply is exact for int8-range operands.
            assert_eq!(
                result,
                dot_unrolled_ref(p as i64, n),
                "int8-datapath P={p} dot({n}) != 64-bit oracle"
            );
        }
        println!("  P={p}: int8-native datapath array runs BYTE-IDENTICAL to the 64-bit oracle (1 DSP/MAC form).");
    }
    println!("★ INT8-NATIVE DATAPATH: byte-identical to the 64-bit fabric, and each MAC is one DSP48E1 (DSP-packable, faster).");
}

/// ★ THE WHOLE NET, AS FEW CYCLES AS THE BOARD ALLOWS. "1 cycle" for a whole token is physically
/// impossible — a token is 50,624 multiply-accumulates, so one cycle would need 50,624 multipliers, and the
/// Artix-7 has 240 DSPs (~1,000 int8 LUT-multipliers if you spend the whole chip). A real RDU doesn't do it
/// either: it lays the graph out SPATIALLY and pipelines, ~1 cycle per stage in steady state. So "as few as
/// we can" = a P-wide MAC array doing P MACs/clock → ⌈50,624/P⌉ cycles/token. This test PROVES the spatial
/// scaling (u MACs/clock ≡ VM as the array widens) and locks the honest whole-net cycle floor at the board's
/// area budget, against the sequential whole-program engine (124,559 block-steps/token, measured in
/// `wpe_transformer_lock`) — the spatial win the RDU pitch rests on.
#[test]
fn the_whole_net_runs_spatially_in_the_fewest_cycles_the_board_allows() {
    // (a) the array is real: widening the unroll places that many independent multiply tiles = that many
    //     MACs/clock, byte-identical to the VM (proven here up to the tractable width; the on-silicon GEMM
    //     array in `gemm_fabric_lock` is the hundreds-of-PE realization).
    for u in [1usize, 2, 4, 8, 16] {
        let lf = loop_fabric_from_source(&dot_unrolled_fn(u), "dotU");
        assert_eq!(
            count_muls(&lf.body),
            u,
            "a {u}-wide array must place {u} multiply tiles = {u} MACs/clock"
        );
        assert_eq!(
            eval_loop_fabric(&lf, &[16]),
            dot_unrolled_ref(u as i64, 16),
            "{u}-wide array != VM"
        );
    }

    // (b)–(d) derive the board-bound cycle floors; the committed ledger's whole-net section is that derivation.
    assert_eq!(
        golden::committed_json_value(LEDGER, LEDGER_REGEN)["whole_net_spatial"],
        whole_net_section(),
        "{LEDGER}: the whole-net section is not what this lock derives — regenerate with `{LEDGER_REGEN}`"
    );
}

/// The whole-net spatial section: the per-token MAC count, the sequential engine's baseline and the cycle floor at
/// each board-bound datapath width, each floor asserted against the targets the RDU pitch rests on.
fn whole_net_section() -> serde_json::Value {
    // (b) the whole net's MAC count (matmul-dominated), at the deepest generation step.
    let jpath = root().join("benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&jpath).unwrap()).unwrap();
    let cfg = &j["config"];
    let (d, h, dff, v) = (
        cfg["d_model"].as_i64().unwrap(),
        cfg["n_heads"].as_i64().unwrap(),
        cfg["d_ff"].as_i64().unwrap(),
        cfg["vocab"].as_i64().unwrap(),
    );
    let hd = d / h;
    let n = j["golden_trace"]["steps"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["context_len"]
        .as_i64()
        .unwrap();
    let macs =
        3 * n * d * d + 2 * (h * (n * (n + 1) / 2) * hd) + n * d * d + 2 * n * d * dff + v * d;
    assert_eq!(
        macs, 50_624,
        "the whole-net MAC count changed — re-derive the cycle floors"
    );

    // (c) the sequential whole-program engine baseline (MEASURED ≡VM in wpe_transformer_lock: 373,677
    //     block-steps for the 3-token generation → per token).
    let seq_per_token = 373_677i64 / 3;

    // (d) the honest board-bound array width depends on the DATAPATH: how many DSP48E1s a multiply costs.
    //     SYNTH-MEASURED (scripts/synth-fabric-matmul.sh 8 → yosys synth_xilinx): the emitted 8-wide array =
    //     80 DSP48E1s → the current 64-bit fabric datapath costs ~10 DSPs per MAC. So on the XC7A100T's 240
    //     DSPs the current datapath fits P≈24; an int8-NATIVE datapath (1 DSP/MAC, the named optimization)
    //     fits P≈240. Both are ⌈MACs/P⌉ cycles/token, both are a huge cut vs the 124,559-cyc sequential engine.
    const BOARD_DSPS: i64 = 240;
    const DSPS_PER_MAC_64BIT: i64 = 10; // SYNTH-MEASURED (P=8 → 80 DSP48E1)
    let scenarios: [(&str, i64, &str); 3] = [
        ("datapath_64bit", BOARD_DSPS / DSPS_PER_MAC_64BIT, "64-bit datapath, ~10 DSP/MAC (synth-measured, old)"),
        ("datapath_int8_native", BOARD_DSPS, "int8-native datapath, 1 DSP/MAC — BUILT (emit_loop_fabric_verilog_int8; synth-measured 8 DSP for the 8-wide array; routed at 66-128 MHz)"),
        ("lut_multipliers_1024", 1024, "int8 LUT-multipliers, ~whole chip (no DSP)"),
    ];
    let mut spatial = serde_json::Map::new();
    for (name, p, why) in scenarios {
        let cyc = (macs + p - 1) / p;
        assert_eq!(cyc, (macs as f64 / p as f64).ceil() as i64);
        assert!(cyc >= 1);
        spatial.insert(
            name.into(),
            serde_json::json!({
                "macs_per_clock": p, "cycles_per_token": cyc,
                "reduction_vs_sequential_x": (seq_per_token as f64 / cyc as f64).round() as i64,
                "why": why,
            }),
        );
    }
    let cur_p = BOARD_DSPS / DSPS_PER_MAC_64BIT; // 24
    let cur_cyc = (macs + cur_p - 1) / cur_p;
    let int8_cyc = (macs + BOARD_DSPS - 1) / BOARD_DSPS;
    // even the CURRENT datapath is a big cut; the int8 datapath is the ~211-cyc/token target.
    assert!(
        seq_per_token / cur_cyc >= 50,
        "current-datapath spatial must be ≥50× fewer cycles (got {}×)",
        seq_per_token / cur_cyc
    );
    assert!(
        int8_cyc <= 220,
        "int8-native datapath must reach a few-hundred cycles/token (got {int8_cyc})"
    );
    assert!(
        seq_per_token / int8_cyc >= 500,
        "int8-native spatial must be ≥500× fewer cycles (got {}×)",
        seq_per_token / int8_cyc
    );

    println!(
        "★ WHOLE NET ON THE FABRIC, AS FEW CYCLES AS THE BOARD ALLOWS: {macs} MACs/token vs sequential engine \
         {seq_per_token} cyc/token. Board-bound spatial (240 DSPs): current 64-bit datapath (synth-measured \
         10 DSP/MAC) → P≈{cur_p} → {cur_cyc} cyc/token ({}× fewer); int8-native datapath (1 DSP/MAC) → P≈240 → \
         {int8_cyc} cyc/token ({}× fewer). 1 cycle = 50,624 multipliers — impossible on a $300 board.",
        seq_per_token / cur_cyc,
        seq_per_token / int8_cyc,
    );

    // (e) the honest whole-net spatial cycle section.
    serde_json::json!({
        "note": "The WHOLE net on the fabric, as few cycles as the board allows. 1 cycle is impossible \
                 (50,624 multipliers needed). Board-bound array width = 240 DSPs / (DSPs per MAC). The int8-native \
                 datapath is BUILT: emit_loop_fabric_verilog_int8 makes each MAC ONE DSP48E1 (synth-measured 8 DSP \
                 for the 8-wide array, vs 80 for the 64-bit datapath), so P≈240 fits the 240 DSPs → ~211 cyc/token; \
                 the routed int8-DSP bitstream (fabric_matmul_i8_r8.bit) is 8 DSP + 953 LUT at 66-128 MHz (the open \
                 nextpnr DSP-packer bug was patched). cycles/token = ceil(MACs/P). Per-MAC=1-clock is MEASURED \
                 (iverilog); the array is AUTO-EMITTED from LOGOS. No wall-clock claim vs any vendor part.",
        "matmul_macs_per_token": macs,
        "sequential_engine_cycles_per_token": seq_per_token,
        "synth_measured_dsps_per_mac_64bit": DSPS_PER_MAC_64BIT,
        "board_dsps": BOARD_DSPS,
        "by_datapath": serde_json::Value::Object(spatial),
    })
}
