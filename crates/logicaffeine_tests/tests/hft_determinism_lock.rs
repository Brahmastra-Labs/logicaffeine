//! hft_determinism_lock — the CI lock for the LOGOS tick-to-trade demo (benchmarks/silicon/HFT_DEMO_README.md).
//!
//! The demo's whole claim is that an English trading strategy compiles to deterministic silicon that
//! does EXACTLY what the English says. This test pins that on the reproducible (no-board) side:
//!
//!   1. the committed `hft_pipeline.lg` produces the exact golden order stream on the VM (the spec is
//!      locked byte-for-byte — a strategy-logic change can't silently pass);
//!   2. an INDEPENDENT Rust reference of the same pipeline agrees with the VM on the demo feed AND on a
//!      broad seeded fuzz of random feeds (correctness to absurdity — empty book, clamped levels,
//!      out-of-ladder prices, position-limit hits);
//!   3. the program compiles to full-core silicon words DETERMINISTICALLY (same words every compile) and
//!      stays within the loader/heap bounds the demo claims.
//!
//! Silicon ≡ VM (cycle-accurate) is proven separately by `benchmarks/silicon/tb_hft.v` under iverilog
//! (reproducible via `scripts/demo-hft.sh`) and by the full-core `run_soft_core` ≡VM differential in
//! `codegen_sva::rtl.rs`; this lock guards the language/spec/compile side that lives in CI without a
//! simulator.

use logicaffeine_compile::codegen_sva::full_asm::compile_to_full_words;
use logicaffeine_compile::compile::vm_outcome;

const PIPELINE_LG: &str = include_str!("../../../benchmarks/silicon/programs/hft_pipeline.lg");

// The golden order stream for the committed demo feed: (order_side, order_price, order_size) per message.
const GOLDEN: &[i64] = &[
    0, 0, 0, 0, 0, 0, 2, 100, 10, 1, 101, 10, 2, 100, 10, 2, 100, 10, 2, 100, 10, 1, 101, 10,
];

// --- pipeline parameters (shared with scripts/hft_model.py and scripts/hft_gen.py) ---
const PRICE_MIN: i64 = 93;
const LEVELS: i64 = 16;
const FAIR_100: i64 = 10050;
const EDGE_100: i64 = 5;
const BASE_SIZE: i64 = 10;
const MAX_ORDER: i64 = 50;
const POS_CAP: i64 = 100;
const TYPE_ADD: i64 = 65;

/// The independent Rust reference of the tick-to-trade pipeline (mirrors hft_model.run_pipeline exactly,
/// integer-for-integer). Returns the flat Show stream: 3 ints per message.
fn reference_pipeline(feed: &[i64]) -> Vec<i64> {
    let n = LEVELS as usize + 1;
    let mut bid = vec![0i64; n];
    let mut ask = vec![0i64; n];
    let mut inventory = 0i64;
    let mut out = Vec::new();

    let mut i = 0;
    while i < feed.len() {
        let mtype = feed[i];
        let side = feed[i + 1];
        let price = feed[i + 2] * 256 + feed[i + 3];
        let qty = feed[i + 4] * 256 + feed[i + 5];
        let mut tick = price - (PRICE_MIN - 1);
        if tick < 1 {
            tick = 1;
        }
        if tick > LEVELS {
            tick = LEVELS;
        }
        let t = tick as usize;
        let signed = if mtype == TYPE_ADD { qty } else { -qty };
        if side == 1 {
            bid[t] += signed;
            if bid[t] < 0 {
                bid[t] = 0;
            }
        } else {
            ask[t] += signed;
            if ask[t] < 0 {
                ask[t] = 0;
            }
        }

        let mut best_bid_tick = 0i64;
        let mut best_ask_tick = LEVELS + 1;
        for tk in 1..=LEVELS {
            if bid[tk as usize] > 0 {
                best_bid_tick = tk;
            }
            if ask[tk as usize] > 0 && tk < best_ask_tick {
                best_ask_tick = tk;
            }
        }
        let has_bid = (best_bid_tick >= 1) as i64;
        let has_ask = (best_ask_tick <= LEVELS) as i64;
        let book_ok = has_bid * has_ask;
        let (bbp, bbq) = if has_bid == 1 {
            (best_bid_tick + PRICE_MIN - 1, bid[best_bid_tick as usize])
        } else {
            (0, 0)
        };
        let (bap, baq) = if has_ask == 1 {
            (best_ask_tick + PRICE_MIN - 1, ask[best_ask_tick as usize])
        } else {
            (0, 0)
        };
        let mut denom = bbq + baq;
        if denom < 1 {
            denom = 1;
        }
        let micro100 = (bbp * baq + bap * bbq) * 100 / denom;

        let mut order_side = 0i64;
        let mut order_price = 0i64;
        let mut order_size = 0i64;
        if book_ok == 1 {
            if micro100 > FAIR_100 + EDGE_100 {
                order_side = 2;
                order_price = bbp;
                order_size = BASE_SIZE;
            } else if micro100 < FAIR_100 - EDGE_100 {
                order_side = 1;
                order_price = bap;
                order_size = BASE_SIZE;
            }
        }
        if order_size > MAX_ORDER {
            order_size = MAX_ORDER;
        }
        let mut signed_order = if order_side == 1 {
            order_size
        } else if order_side == 2 {
            -order_size
        } else {
            0
        };
        let projected = inventory + signed_order;
        if projected > POS_CAP || projected < -POS_CAP {
            order_side = 0;
            order_price = 0;
            order_size = 0;
            signed_order = 0;
        }
        inventory += signed_order;
        out.push(order_side);
        out.push(order_price);
        out.push(order_size);
        i += 8;
    }
    out
}

/// Rust port of hft_gen.gen: emit the `.lg` for a feed (feed literal last — the RTL heap workaround).
/// A raw string preserves LOGOS's significant indentation exactly (a `\`-continuation would strip it).
fn gen_lg(feed: &[i64]) -> String {
    let lit = feed
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let pm1 = PRICE_MIN - 1;
    format!(
        r#"## Main
Let levels be {LEVELS}.
Let fair100 be {FAIR_100}.
Let edge100 be {EDGE_100}.
Let base_size be {BASE_SIZE}.
Let max_order be {MAX_ORDER}.
Let pos_cap be {POS_CAP}.
Let mutable bid_book be a new Seq of Int.
Let mutable ask_book be a new Seq of Int.
Let mutable z be 0.
While z is less than levels:
    Push 0 to bid_book.
    Push 0 to ask_book.
    Set z to z + 1.
Let feed be [{lit}].
Let n be length of feed.
Let mutable inventory be 0.
Let mutable i be 1.
While i is at most n:
    Let mtype be item i of feed.
    Let side be item (i + 1) of feed.
    Let price be item (i + 2) of feed * 256 + item (i + 3) of feed.
    Let qty be item (i + 4) of feed * 256 + item (i + 5) of feed.
    Let mutable tick be price - {pm1}.
    If tick is less than 1:
        Set tick to 1.
    If tick is greater than levels:
        Set tick to levels.
    If mtype is equal to {TYPE_ADD}:
        If side is equal to 1:
            Set item tick of bid_book to (item tick of bid_book) + qty.
        Otherwise:
            Set item tick of ask_book to (item tick of ask_book) + qty.
    Otherwise:
        If side is equal to 1:
            Set item tick of bid_book to (item tick of bid_book) - qty.
            If (item tick of bid_book) is less than 0:
                Set item tick of bid_book to 0.
        Otherwise:
            Set item tick of ask_book to (item tick of ask_book) - qty.
            If (item tick of ask_book) is less than 0:
                Set item tick of ask_book to 0.
    Let mutable best_bid_tick be 0.
    Let mutable best_ask_tick be levels + 1.
    Let mutable t be 1.
    While t is at most levels:
        If (item t of bid_book) is greater than 0:
            Set best_bid_tick to t.
        If (item t of ask_book) is greater than 0:
            If t is less than best_ask_tick:
                Set best_ask_tick to t.
        Set t to t + 1.
    Let mutable has_bid be 0.
    If best_bid_tick is at least 1:
        Set has_bid to 1.
    Let mutable has_ask be 0.
    If best_ask_tick is at most levels:
        Set has_ask to 1.
    Let book_ok be has_bid * has_ask.
    Let mutable best_bid_price be 0.
    Let mutable best_bid_qty be 0.
    If has_bid is equal to 1:
        Set best_bid_price to best_bid_tick + {pm1}.
        Set best_bid_qty to item best_bid_tick of bid_book.
    Let mutable best_ask_price be 0.
    Let mutable best_ask_qty be 0.
    If has_ask is equal to 1:
        Set best_ask_price to best_ask_tick + {pm1}.
        Set best_ask_qty to item best_ask_tick of ask_book.
    Let mutable denom be best_bid_qty + best_ask_qty.
    If denom is less than 1:
        Set denom to 1.
    Let micro100 be (best_bid_price * best_ask_qty + best_ask_price * best_bid_qty) * 100 / denom.
    Let mutable order_side be 0.
    Let mutable order_price be 0.
    Let mutable order_size be 0.
    If book_ok is equal to 1:
        If micro100 is greater than fair100 + edge100:
            Set order_side to 2.
            Set order_price to best_bid_price.
            Set order_size to base_size.
        Otherwise:
            If micro100 is less than fair100 - edge100:
                Set order_side to 1.
                Set order_price to best_ask_price.
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
        Set order_side to 0.
        Set order_price to 0.
        Set order_size to 0.
        Set signed_order to 0.
    Set inventory to inventory + signed_order.
    Show order_side.
    Show order_price.
    Show order_size.
    Set i to i + 8.
"#
    )
}

fn vm_shows(src: &str) -> Vec<i64> {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output
        .split_whitespace()
        .map(|t| t.parse::<i64>().expect("Show value is an int"))
        .collect()
}

/// The demo feed, byte-identical to hft_model.DEMO_FEED.
fn demo_feed() -> Vec<i64> {
    vec![
        65, 1, 0, 100, 0, 10, 0, 1, 65, 2, 0, 101, 0, 10, 0, 2, 65, 1, 0, 100, 0, 30, 0, 3, 65, 2,
        0, 101, 0, 60, 0, 4, 65, 1, 0, 100, 0, 100, 0, 5, 69, 2, 0, 101, 0, 50, 0, 4, 68, 1, 0,
        100, 0, 100, 0, 5, 65, 2, 0, 101, 0, 200, 0, 7,
    ]
}

#[test]
fn committed_pipeline_matches_golden_on_vm() {
    // The strategy IS the spec: the committed English program must emit the exact golden order stream.
    assert_eq!(
        vm_shows(PIPELINE_LG),
        GOLDEN,
        "committed hft_pipeline.lg drifted from the golden orders"
    );
}

#[test]
fn rust_reference_matches_golden() {
    // The independent reference agrees with the golden — the two implementations pin each other.
    assert_eq!(reference_pipeline(&demo_feed()), GOLDEN);
}

#[test]
fn generated_pipeline_matches_reference_on_demo() {
    // The Rust generator reproduces the committed program's behavior (VM ≡ reference on the demo feed).
    assert_eq!(
        vm_shows(&gen_lg(&demo_feed())),
        reference_pipeline(&demo_feed())
    );
}

#[test]
fn vm_equals_reference_over_random_feeds() {
    // Correctness to absurdity: a broad seeded fuzz. A tiny LCG keeps it dependency-free and reproducible.
    let mut state: u64 = 0xC0FFEE_1234_5678;
    let mut next = |m: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % m
    };
    let types = [TYPE_ADD, TYPE_ADD, TYPE_ADD, 68, 69];
    for _ in 0..120 {
        let nmsgs = 1 + next(12);
        let mut feed = Vec::new();
        for _ in 0..nmsgs {
            let mtype = types[next(types.len() as u64) as usize];
            let side = 1 + next(2) as i64;
            let price = 90 + next(22) as i64; // spans and overflows the [93,108] ladder
            let qty = next(201) as i64;
            let oid = 1 + next(250) as i64;
            feed.extend_from_slice(&[
                mtype,
                side,
                (price >> 8) & 0xFF,
                price & 0xFF,
                (qty >> 8) & 0xFF,
                qty & 0xFF,
                (oid >> 8) & 0xFF,
                oid & 0xFF,
            ]);
        }
        let expect = reference_pipeline(&feed);
        let got = vm_shows(&gen_lg(&feed));
        assert_eq!(
            got, expect,
            "VM diverged from the reference on feed {feed:?}"
        );
    }
}

#[test]
fn compiles_to_full_core_deterministically_within_bounds() {
    // Chip-runnable and reproducible: the full-core encoder accepts it and emits identical words twice.
    let w1 = compile_to_full_words(PIPELINE_LG).expect("pipeline must compile to the full core");
    let w2 = compile_to_full_words(PIPELINE_LG).expect("pipeline must compile to the full core");
    assert_eq!(w1, w2, "full-core compilation must be deterministic");
    assert!(
        w1.len() <= 1024,
        "program exceeds the 1024-word full-core imem: {}",
        w1.len()
    );
    // 3 Show ints per message.
    assert_eq!(GOLDEN.len(), 3 * (demo_feed().len() / 8));
}
