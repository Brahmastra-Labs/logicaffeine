//! hft_decision_fabric_lock — the CI lock for the HFT DECISION lowered to a combinational fabric.
//!
//! The soft-core runs the tick-to-trade pipeline in ~866 fetch-decode-execute cycles per trade. The
//! DECISION inside it — given the market-derived scalars (microprice, fair, edge, size, position,
//! risk caps) — is nested `If/Otherwise/Set` over mutable state. `decision_fabric_from_source`
//! if-CONVERTS that whole decision to a pure select-DAG: every branch becomes a predicated `Select`
//! (mux) tile, so the answer settles in ONE clock — no program counter, no fetch, no dispatch. This
//! is the fetch/decode-free "fast path".
//!
//! This lock pins the lowering's SOUNDNESS on the reproducible (no-simulator) side:
//!   1. the LOGOS decision function runs on the VM identically to an independent Rust reference;
//!   2. the if-converted fabric (`eval_decision_fabric`) equals the VM/reference EXACTLY over a broad
//!      seeded fuzz (both arms of every branch, the max-order clamp, both risk limits, empty book);
//!   3. the emitted combinational Verilog is deterministic and structurally a select-DAG (the tiles
//!      the RTL-to-GDSII flow tapes out).
//!
//! RTL ≡ VM (iverilog) and the Sky130 signoff are proven separately by the demo scripts; this is the
//! language/lowering guard that lives in CI without a simulator.

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, emit_decision_fabric_verilog, eval_decision_fabric, LoopCell,
};
use logicaffeine_compile::compile::vm_outcome;

/// The HFT decision as an English function: microprice market-making + fat-finger clamp + position
/// risk gate, returning the signed order (+buy / −sell / 0). No division (the microprice is an input
/// — a separate normalization stage), so the whole body is compare/arith/branch → a select-DAG.
const DECIDE_FN: &str = "\
## To decide (micro100: Int, fair100: Int, edge100: Int, base_size: Int, max_order: Int, inventory: Int, pos_cap: Int, book_ok: Int) -> Int:
    Let mutable order_side be 0.
    Let mutable order_size be 0.
    If book_ok is equal to 1:
        If micro100 is greater than fair100 + edge100:
            Set order_side to 2.
            Set order_size to base_size.
        Otherwise:
            If micro100 is less than fair100 - edge100:
                Set order_side to 1.
                Set order_size to base_size.
    If order_size is greater than max_order:
        Set order_size to max_order.
    Let mutable signed_order be 0.
    If order_side is equal to 1:
        Set signed_order to order_size.
    If order_side is equal to 2:
        Set signed_order to 0 - order_size.
    Let projected be inventory + signed_order.
    Let mutable risk_ok be 1.
    If projected is greater than pos_cap:
        Set risk_ok to 0.
    If projected is less than 0 - pos_cap:
        Set risk_ok to 0.
    If risk_ok is equal to 0:
        Set signed_order to 0.
    Return signed_order.
";

const FN_NAME: &str = "decide";

#[allow(clippy::too_many_arguments)]
fn decide_ref(
    micro100: i64,
    fair100: i64,
    edge100: i64,
    base_size: i64,
    max_order: i64,
    inventory: i64,
    pos_cap: i64,
    book_ok: i64,
) -> i64 {
    let mut order_side = 0i64;
    let mut order_size = 0i64;
    if book_ok == 1 {
        if micro100 > fair100 + edge100 {
            order_side = 2;
            order_size = base_size;
        } else if micro100 < fair100 - edge100 {
            order_side = 1;
            order_size = base_size;
        }
    }
    if order_size > max_order {
        order_size = max_order;
    }
    let mut signed_order = 0i64;
    if order_side == 1 {
        signed_order = order_size;
    }
    if order_side == 2 {
        signed_order = 0 - order_size;
    }
    let projected = inventory + signed_order;
    let mut risk_ok = 1i64;
    if projected > pos_cap {
        risk_ok = 0;
    }
    if projected < 0 - pos_cap {
        risk_ok = 0;
    }
    if risk_ok == 0 {
        signed_order = 0;
    }
    signed_order
}

/// A LOGOS integer literal argument that also expresses negatives (`0 - n`), which a bare `-n` call
/// argument would not parse.
fn arg(n: i64) -> String {
    if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    }
}

fn vm_decide(p: &[i64; 8]) -> i64 {
    let call = format!(
        "decide({}, {}, {}, {}, {}, {}, {}, {})",
        arg(p[0]),
        arg(p[1]),
        arg(p[2]),
        arg(p[3]),
        arg(p[4]),
        arg(p[5]),
        arg(p[6]),
        arg(p[7])
    );
    let src = format!("{DECIDE_FN}## Main\n    Show {call}.\n");
    let out = vm_outcome(&src);
    assert!(out.error.is_none(), "VM error on {p:?}: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

/// A dependency-free seeded fuzz over realistic + adversarial market states.
fn fuzz_params(n: usize) -> Vec<[i64; 8]> {
    let mut state: u64 = 0xDEC151_0F_ABC_u64;
    let mut next = |m: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % m
    };
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let fair100 = 10050;
        let edge100 = next(20) as i64; // 0..19
                                       // micro100 straddles the fair±edge band so both the buy and sell arms and the flat middle fire.
        let micro100 = 10000 + next(120) as i64; // 10000..10119
        let base_size = 1 + next(80) as i64; // spans past max_order to exercise the clamp
        let max_order = 10 + next(60) as i64;
        let inventory = next(260) as i64 - 130; // −130..129, straddles ±pos_cap
        let pos_cap = 100;
        let book_ok = next(2) as i64;
        out.push([
            micro100, fair100, edge100, base_size, max_order, inventory, pos_cap, book_ok,
        ]);
    }
    out
}

#[test]
fn vm_matches_reference() {
    // The English decision runs identically to the independent Rust reference — the spec is pinned.
    for p in fuzz_params(200) {
        assert_eq!(
            vm_decide(&p),
            decide_ref(p[0], p[1], p[2], p[3], p[4], p[5], p[6], p[7]),
            "VM decision diverged from reference on {p:?}"
        );
    }
}

#[test]
fn fabric_matches_vm_over_fuzz() {
    // The if-CONVERTED select-DAG equals the running VM exactly — control-as-data is sound.
    let df = decision_fabric_from_source(DECIDE_FN, FN_NAME);
    assert_eq!(df.n_in, 8, "decision has 8 market-scalar inputs");
    for p in fuzz_params(500) {
        let got = eval_decision_fabric(&df, &p);
        let want = vm_decide(&p);
        assert_eq!(got, want, "fabric ≠ VM on {p:?} (got {got}, want {want})");
    }
}

#[test]
fn fabric_is_a_select_dag_and_deterministic() {
    // The decision lowers to predicated Select tiles (the branches) over arith tiles, and the emitted
    // Verilog is byte-identical every time (a tapeable, reproducible artifact).
    let df = decision_fabric_from_source(DECIDE_FN, FN_NAME);
    let selects = df
        .cells
        .iter()
        .filter(|c| matches!(c, LoopCell::Select { .. }))
        .count();
    assert!(
        selects >= 5,
        "the nested decision must produce several Select tiles, got {selects}"
    );
    let v1 = emit_decision_fabric_verilog("hft", &df);
    let v2 = emit_decision_fabric_verilog("hft", &df);
    assert_eq!(v1, v2, "emitted decision Verilog must be deterministic");
    assert!(v1.contains("module decision_hft("), "emits a named module");
    assert!(v1.contains("?"), "contains ternary select tiles");
}
