//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! AN ATTENTION'S MASKS: LINEAR BIASES, A BIDIRECTIONAL PREFIX, EACH ROW'S OWN START.
//!
//! After its sinks an attention takes, in this order:
//!
//!   • `with linear biases M` — ALiBi: `M` a slope a query head, each score of a key at position `j` for a query at `n`
//!     taking `−M[h]·|n − j|` after its scale and cap;
//!   • `with a bidirectional prefix of P` — a causal block's first `P` positions attend one another whole (prefix-LM):
//!     a row at `n` attends `max(n, P)` positions, the rows inside the prefix the keys this block writes;
//!   • `with rows starting at S` — each row attends from its own first position `S[r]` (a block of packed sequences, each
//!     causal within itself), the later of that and a window's.
//!
//! The promises:
//!
//!   1. MEANING. Every mask alone and beside a window, chunks, a scale, a cap and sinks, in the single query's form and a
//!      causal block's, is the `f64` formula on the VM and the tree-walker, and on the C backend and AOT-Rust.
//!   2. A MASK THAT DOES NOT FIT IS REFUSED BY NAME: slopes or starts of another shape or kind, a prefix on a single query,
//!      and at run time a prefix past the keys its block writes or a row start outside `1..=n` — never an empty or a NaN
//!      row.
//!   3. NOTHING LIFTS A MASKED ATTENTION AS AN UNMASKED ONE: an attention's spec holds no mask yet, so the tensor front end
//!      lifts no attention at all from a masked one — not even the bare attention a window or a row's start leaves behind
//!      when its lower bound is dropped — while the same attention unmasked lifts.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tensor_attention_masks_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};

const L: usize = 8;
const HD: usize = 2;
const HEADS: usize = 2;

fn key(j: usize, c: usize) -> f64 {
    [0.3 * j as f64 - 1.0, 0.2 * (j % 3) as f64 - 0.1][c]
}

fn value(j: usize, c: usize) -> f64 {
    [0.5 * j as f64 - 1.0, 0.25 * (9 - j) as f64][c]
}

fn query(r: usize, head: usize, c: usize) -> f64 {
    [1.6, -1.2][c] + 0.3 * r as f64 - 0.7 * head as f64
}

fn sink(h: usize) -> f64 {
    [0.9, -0.4][h]
}

fn slope(h: usize) -> f64 {
    [0.5, 0.125][h]
}

/// What an attention's clauses ask of each row: the clauses before the masks, then the masks.
#[derive(Clone, Default)]
struct Masks {
    window: Option<usize>,
    chunks: Option<usize>,
    scale: Option<f64>,
    cap: Option<f64>,
    sinks: bool,
    biases: bool,
    prefix: Option<usize>,
    starts: Option<Vec<usize>>,
}

impl Masks {
    /// The clauses as the sugar writes them.
    fn clauses(&self) -> String {
        let mut s = String::new();
        if let Some(w) = self.window {
            s += &format!(" within a window of {w}");
        }
        if let Some(c) = self.chunks {
            s += &format!(" within chunks of {c}");
        }
        if let Some(x) = self.scale {
            s += &format!(" scaled by {x:?}");
        }
        if let Some(c) = self.cap {
            s += &format!(" capped at {c:?}");
        }
        if self.sinks {
            s += " with sinks S";
        }
        if self.biases {
            s += " with linear biases M";
        }
        if let Some(p) = self.prefix {
            s += &format!(" with a bidirectional prefix of {p}");
        }
        if self.starts.is_some() {
            s += " with rows starting at R";
        }
        s
    }
}

/// `Q` (`rows` by `HEADS·2`), `K` and `V` (`L` by 2), the sinks `S`, the slopes `M` and the starts `R` (where given)
/// filled with literals, `C` the attention `form` (`… groups` and what follows it) of them, every entry of `C` shown as
/// `round(c·1e9)`.
fn program(rows: usize, form: &str, m: &Masks) -> String {
    let mut s = format!("## Main\nLet Q be a {rows} by {} tensor of Real.\n", HEADS * HD);
    for r in 1..=rows {
        for h in 0..HEADS {
            for c in 0..HD {
                s += &format!("Set item {} of Q to {:?}.\n", (r - 1) * HEADS * HD + h * HD + c + 1, query(r, h, c));
            }
        }
    }
    s += &format!("Let K be a {L} by {HD} tensor of Real.\nLet V be a {L} by {HD} tensor of Real.\n");
    for j in 1..=L {
        for c in 0..HD {
            s += &format!("Set item {} of K to {:?}.\n", (j - 1) * HD + c + 1, key(j, c));
            s += &format!("Set item {} of V to {:?}.\n", (j - 1) * HD + c + 1, value(j, c));
        }
    }
    s += &format!("Let S be a 1 by {HEADS} tensor of Real.\nLet M be a 1 by {HEADS} tensor of Real.\n");
    for h in 0..HEADS {
        s += &format!("Set item {} of S to {:?}.\nSet item {} of M to {:?}.\n", h + 1, sink(h), h + 1, slope(h));
    }
    if let Some(starts) = &m.starts {
        s += &format!("Let R be a 1 by {rows} tensor of Int.\n");
        for (r, st) in starts.iter().enumerate() {
            s += &format!("Set item {} of R to {st}.\n", r + 1);
        }
    }
    s += &format!("Let pos be 6.\nLet C be {form}{}.\n", m.clauses());
    for i in 1..=rows * HEADS * HD {
        s += &format!("Show round(item {i} of C * 1000000000).\n");
    }
    s
}

/// The single query's form at `pos` (6), and a causal block of `rows` rows from `first`.
fn decode(m: &Masks) -> String {
    program(1, &format!("attention of Q over K and V with {HEADS} heads 1 groups up to pos"), m)
}

fn block(rows: usize, first: usize, m: &Masks) -> String {
    program(rows, &format!("causal attention of Q over K and V with {HEADS} heads 1 groups from {first}"), m)
}

/// The `f64` attention of query row `r` (1-based) at position `n`: each score `q·k` times the scale, capped as
/// `C·tanh(s/C)`, less its head's slope times the key's distance from `n`, over the positions from the later of the
/// window's first and the row's start to `max(n, P)`, each head's sink in its softmax.
fn oracle(r: usize, n: usize, m: &Masks) -> Vec<i64> {
    let window = match (m.window, m.chunks) {
        (Some(w), _) => n.saturating_sub(w - 1).max(1),
        (_, Some(c)) => (n - 1) / c * c + 1,
        _ => 1,
    };
    let lo = window.max(m.starts.as_ref().map_or(1, |s| s[r - 1]));
    let reach = n.max(m.prefix.unwrap_or(0));
    let scale = m.scale.unwrap_or(1.0 / (HD as f64).sqrt());
    (0..HEADS)
        .flat_map(|h| {
            let scores: Vec<f64> = (lo..=reach)
                .map(|j| {
                    let s = (0..HD).map(|c| query(r, h, c) * key(j, c)).sum::<f64>() * scale;
                    let s = m.cap.map_or(s, |cap| cap * (s / cap).tanh());
                    if m.biases {
                        s - slope(h) * (n as f64 - j as f64).abs()
                    } else {
                        s
                    }
                })
                .collect();
            let mx = scores.iter().copied().fold(if m.sinks { sink(h) } else { f64::NEG_INFINITY }, f64::max);
            let den: f64 = scores.iter().map(|s| (s - mx).exp()).sum::<f64>() + if m.sinks { (sink(h) - mx).exp() } else { 0.0 };
            (0..HD)
                .map(|c| {
                    let a: f64 = (lo..=reach).zip(&scores).map(|(j, s)| (s - mx).exp() / den * value(j, c)).sum();
                    (a * 1e9).round() as i64
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn shown(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error on:\n{src}\n{:?}", vm.error);
    let tw = tw_outcome(src);
    assert!(tw.error.is_none(), "tree-walker error: {:?}", tw.error);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    vm.output.split_whitespace().map(|t| t.parse().expect("an integer")).collect()
}

fn close(got: &[i64], want: &[i64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() <= 2, "{what}: {got:?} vs the formula's {want:?}");
    }
}

/// Every mask alone and with the clauses before it, as each form takes it.
fn cases() -> Vec<(&'static str, Masks)> {
    let m = Masks::default;
    vec![
        ("linear biases", Masks { biases: true, ..m() }),
        ("linear biases in a window", Masks { biases: true, window: Some(3), ..m() }),
        ("linear biases with a scale, a cap and sinks", Masks { biases: true, scale: Some(0.6), cap: Some(1.5), sinks: true, ..m() }),
        ("linear biases in chunks", Masks { biases: true, chunks: Some(4), ..m() }),
        ("rows starting at their sequences' starts", Masks { starts: Some(vec![1, 5, 5]), ..m() }),
        ("rows starting inside a window", Masks { starts: Some(vec![2, 3, 6]), window: Some(3), ..m() }),
        ("rows starting under linear biases and sinks", Masks { starts: Some(vec![1, 5, 6]), biases: true, sinks: true, ..m() }),
        ("a bidirectional prefix", Masks { prefix: Some(5), ..m() }),
        ("a bidirectional prefix in a window", Masks { prefix: Some(5), window: Some(2), ..m() }),
        ("a bidirectional prefix under linear biases", Masks { prefix: Some(6), biases: true, ..m() }),
        ("every mask", Masks { prefix: Some(5), biases: true, starts: Some(vec![1, 2, 5]), cap: Some(2.0), sinks: true, ..m() }),
    ]
}

#[test]
fn every_mask_clause_desugars_and_the_vm_runs_it() {
    let (first, rows) = (4, 3);
    for (what, m) in cases() {
        let got = shown(&block(rows, first, &m));
        let want: Vec<i64> = (1..=rows).flat_map(|r| oracle(r, first + r - 1, &m)).collect();
        close(&got, &want, &format!("{what}, a block of {rows} rows from {first}"));
        // The single query's form takes every mask but the prefix, a block's: its one row at `pos`.
        if m.prefix.is_none() {
            let one = Masks { starts: m.starts.as_ref().map(|s| vec![s[1].min(6)]), ..m.clone() };
            close(&shown(&decode(&one)), &oracle(1, 6, &one), &format!("{what}, one query at 6"));
        }
    }
    // A row whose start is its own position attends itself alone: its value row, whatever its scores.
    let alone = Masks { starts: Some(vec![4]), biases: true, ..Masks::default() };
    let got = shown(&block(1, 4, &alone));
    let want: Vec<i64> = (0..HEADS).flat_map(|_| (0..HD).map(|c| (value(4, c) * 1e9).round() as i64)).collect();
    close(&got, &want, "a row starting at its own position");
    // A block of one row inside no prefix it reaches past attends its own positions.
    close(&shown(&block(1, 6, &Masks { prefix: Some(5), ..Masks::default() })), &oracle(1, 6, &Masks { prefix: Some(5), ..Masks::default() }), "a row past the prefix");
}

#[test]
fn the_compiled_tiers_run_every_mask() {
    let every = Masks { prefix: Some(5), biases: true, starts: Some(vec![1, 2, 5]), window: Some(4), ..Masks::default() };
    for src in [block(3, 4, &every), decode(&Masks { biases: true, starts: Some(vec![3]), chunks: Some(4), sinks: true, ..Masks::default() })] {
        let want = vm_outcome(&src).output;
        common::assert_c_output(&src, want.trim());
        common::assert_exact_output(&src, want.trim());
    }
}

#[test]
fn a_mask_that_does_not_fit_is_refused_by_name() {
    let refused = |src: &str, why: &str| {
        let err = vm_outcome(src).error.unwrap_or_else(|| panic!("refused naming {why}:\n{src}"));
        assert!(err.contains(why), "refused naming {why}: {err}");
    };
    let base = Masks::default();
    // Slopes or starts of another shape or kind, and a prefix on a single query.
    refused(&block(3, 4, &base).replace("Let C be causal attention of Q over K and V with 2 heads 1 groups from 4", "Let C be causal attention of Q over K and V with 2 heads 1 groups from 4 with linear biases S2").replace("Let pos be 6.", "Let S2 be a 1 by 3 tensor of Real.\nLet pos be 6."), "with linear biases M");
    refused(
        &block(3, 4, &Masks { starts: Some(vec![1, 1]), ..base.clone() }).replace("Let R be a 1 by 3 tensor of Int.", "Let R be a 1 by 2 tensor of Int."),
        "with rows starting at S",
    );
    refused(&block(3, 4, &base).replace("from 4", "from 4 with rows starting at M"), "with rows starting at S");
    refused(&decode(&base).replace("up to pos", "up to pos with a bidirectional prefix of 3"), "a single query attends its own positions");
    // At run time: a prefix past the keys its block writes, and a row start outside 1..=n.
    refused(&block(3, 2, &Masks { prefix: Some(6), ..base.clone() }), "bidirectional prefix out of range");
    refused(&block(3, 4, &Masks { prefix: Some(9), ..base.clone() }), "bidirectional prefix out of range");
    refused(&block(3, 4, &Masks { starts: Some(vec![1, 0, 1]), ..base.clone() }), "attention row start out of range");
    refused(&block(3, 4, &Masks { starts: Some(vec![1, 6, 1]), ..base.clone() }), "attention row start out of range");
}

#[test]
fn the_front_end_lifts_no_masked_attention_as_an_unmasked_one() {
    let attends = |src: &str| {
        let rp = recognize_stages(src).expect("parses");
        rp.stages.iter().any(|s| matches!(s.op, TensorOpSpec::Attention(_)))
    };
    for (what, m) in cases() {
        let bare = Masks { biases: false, prefix: None, starts: None, ..m.clone() };
        assert!(attends(&block(3, 4, &bare)), "{what}: the block unmasked lifts as an attention");
        assert!(!attends(&block(3, 4, &m)), "{what}: the masked block lifts as an attention its spec does not mask");
        if m.prefix.is_none() {
            let one = Masks { starts: m.starts.as_ref().map(|s| vec![s[1].min(6)]), ..m.clone() };
            assert!(attends(&decode(&bare)), "{what}: the query unmasked lifts as an attention");
            assert!(!attends(&decode(&one)), "{what}: the masked query lifts as an attention its spec does not mask");
        }
    }
}
