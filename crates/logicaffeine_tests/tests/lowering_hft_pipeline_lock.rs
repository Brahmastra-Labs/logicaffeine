//! lowering_hft_pipeline_lock — a REALISTIC whole program lowers on the fabric ≡ the VM.
//!
//! The corpus locks (`lowering_coverage_lock`) pin individual constructs. This pins a full, non-trivial
//! program: an HFT tick-to-trade pipeline written in English — ITCH-lite decode → a direct-mapped price-ladder
//! order book (dynamic-index `Set item tick of bid`) → an O(16) best-of-book scan (a nested loop) → a
//! division-free microprice comparison (cross-multiplied) → a market-making decision → pre-trade risk. It
//! exercises Seq build + dynamic indexed read/write + nested `While` loops + branchy arithmetic together.
//!
//! Three locks:
//!   1. the English program runs on the VM and equals an independent Python reference (`scripts/hft_model.py`
//!      on the same feed = 7401040), so the spec is pinned;
//!   2. the whole-program fabric LOWERS it (no `try_` refusal) and runs it ≡ the VM;
//!   3. a measured whole-program fabric CYCLE count (`eval_wprog_config_counted`, ==VM) — the honest
//!      full-pipeline number the one-pager quotes. A regression here (a construct stops lowering, or the
//!      cycle count drifts) is RED.
//!
//! Provenance note: an earlier draft of this pipeline referenced a `Let LEVELS be 16` binding inside the
//! nested scan loop, where it is out of scope — the VM raised `Undefined variable: LEVELS` and the fabric
//! correctly REFUSED it (a `FailWith`). That was a bug in the test, not a fabric gap: a program that reaches
//! a `FailWith` errors in the VM too, so there is no ≡VM value to produce. The fabric must refuse it, never
//! "trap" it away. With the bug fixed (literals), the pipeline lowers strictly.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(lowering_hft_pipeline_lock)'`

use logicaffeine_compile::codegen_sva::fabric_rtl::try_program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    eval_wprog_config_counted, serialize_program,
};
use logicaffeine_compile::compile::vm_outcome;

/// The full tick-to-trade pipeline over a fixed 6-message ITCH-lite feed, as an English `f() -> Int`.
fn hft_pipeline_src() -> String {
    let feed: [i64; 48] = [
        65, 1, 0, 99, 0, 30, 0, 1, 65, 2, 0, 101, 0, 20, 0, 2, 65, 1, 0, 100, 0, 50, 0, 3, 69, 2,
        0, 101, 0, 20, 0, 2, 65, 2, 0, 102, 0, 40, 0, 4, 65, 1, 0, 100, 0, 25, 0, 5,
    ];
    let mut s = String::from("## To f () -> Int:\n    Let mutable feed be a new Seq of Int.\n");
    for b in feed {
        s.push_str(&format!("    Push {b} to feed.\n"));
    }
    s.push_str(
        "    Let PMIN be 93.
    Let FAIR be 10050.
    Let EDGE be 5.
    Let BASE be 10.
    Let MAXO be 50.
    Let PCAP be 100.
    Let mutable bid be a new Seq of Int.
    Let mutable ask be a new Seq of Int.
    Let mutable z be 0.
    While z is at most 16:
        Push 0 to bid.
        Push 0 to ask.
        Set z to z + 1.
    Let mutable inv be 0.
    Let mutable chk be 0.
    Let mutable i be 1.
    While i is at most 43:
        Let mtype be item i of feed.
        Let side be item (i + 1) of feed.
        Let price be (item (i + 2) of feed) * 256 + (item (i + 3) of feed).
        Let qty be (item (i + 4) of feed) * 256 + (item (i + 5) of feed).
        Let mutable tick be price - PMIN + 1.
        If tick is less than 1:
            Set tick to 1.
        If tick is greater than 16:
            Set tick to 16.
        Let mutable signed be 0 - qty.
        If mtype is equal to 65:
            Set signed to qty.
        If side is equal to 1:
            Let mutable nv be (item tick of bid) + signed.
            If nv is less than 0:
                Set nv to 0.
            Set item tick of bid to nv.
        Otherwise:
            Let mutable nw be (item tick of ask) + signed.
            If nw is less than 0:
                Set nw to 0.
            Set item tick of ask to nw.
        Let mutable bbt be 0.
        Let mutable bat be 16 + 1.
        Let mutable bbq be 0.
        Let mutable baq be 0.
        Let mutable t be 1.
        While t is at most 16:
            If item t of bid is greater than 0:
                If t is greater than bbt:
                    Set bbt to t.
                    Set bbq to item t of bid.
            If item t of ask is greater than 0:
                If t is less than bat:
                    Set bat to t.
                    Set baq to item t of ask.
            Set t to t + 1.
        Let mutable hasbid be 0.
        If bbt is at least 1:
            Set hasbid to 1.
        Let mutable hasask be 0.
        If bat is at most 16:
            Set hasask to 1.
        Let bookok be hasbid * hasask.
        Let bbp be bbt + PMIN - 1.
        Let bap be bat + PMIN - 1.
        Let denom be bbq + baq.
        Let mnum be (bbp * baq + bap * bbq) * 100.
        Let mutable oside be 0.
        Let mutable oprice be 0.
        Let mutable osize be 0.
        If bookok is equal to 1:
            If mnum is less than (FAIR - EDGE) * denom:
                Set oside to 1.
                Set oprice to bap.
                Set osize to BASE.
            Otherwise:
                If mnum is greater than (FAIR + EDGE) * denom:
                    Set oside to 2.
                    Set oprice to bbp.
                    Set osize to BASE.
        If osize is greater than MAXO:
            Set osize to MAXO.
        Let mutable so be 0.
        If oside is equal to 1:
            Set so to osize.
        If oside is equal to 2:
            Set so to 0 - osize.
        Let proj be inv + so.
        If proj is greater than PCAP:
            Set oside to 0.
            Set oprice to 0.
            Set osize to 0.
            Set so to 0.
        If proj is less than 0 - PCAP:
            Set oside to 0.
            Set oprice to 0.
            Set osize to 0.
            Set so to 0.
        Set inv to inv + so.
        Set chk to chk + oside * 1000000 + oprice * 1000 + osize.
        Set i to i + 8.
    Return chk.
",
    );
    s
}

/// The Python reference (`hft_model.run_pipeline`) on the same 6-message feed, checksummed the same way.
const PYTHON_ORACLE: i64 = 7_401_040;
const N_MESSAGES: u64 = 6;

fn vm_result(src: &str) -> Option<i64> {
    let out = vm_outcome(&format!("{src}## Main\n    Show f().\n"));
    if out.error.is_some() {
        return None;
    }
    out.output.split_whitespace().next()?.parse::<i64>().ok()
}

#[test]
fn the_english_pipeline_matches_the_python_reference_on_the_vm() {
    // Lock 1: the spec. The English pipeline runs on the VM and equals the independent Python model.
    assert_eq!(
        vm_result(&hft_pipeline_src()),
        Some(PYTHON_ORACLE),
        "the English HFT pipeline must run on the VM and equal the Python reference ({PYTHON_ORACLE})"
    );
}

#[test]
fn the_realistic_pipeline_lowers_on_the_fabric_equal_to_the_vm() {
    let src = hft_pipeline_src();
    // Lock 2: the whole-program fabric LOWERS it (strict — no trap), and runs it ≡ the VM.
    let pf = try_program_cfg_fabric(&src, "f")
        .expect("the whole-program fabric must lower the realistic tick-to-trade pipeline");
    let (res, cycles) = eval_wprog_config_counted(&serialize_program(&pf), &[]);
    assert_eq!(
        res, PYTHON_ORACLE,
        "fabric result must equal the VM / Python reference"
    );

    // Lock 3: the measured whole-program fabric cycle count (the honest full-pipeline number). Pinned as a
    // band so a real lowering regression is RED but trivial codegen jitter is not.
    let per_msg = cycles / N_MESSAGES;
    assert!(
        (80..=130).contains(&per_msg),
        "full-pipeline fabric cycles/message drifted to {per_msg} (expected ~98; band 80..=130) — total {cycles}"
    );
}
