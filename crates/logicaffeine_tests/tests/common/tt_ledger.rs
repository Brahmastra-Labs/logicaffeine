//! Shared helpers for every lock that reads the Tenstorrent GEMM ledger
//! (`benchmarks/silicon/tenstorrent_gemm.json`, written only by `scripts/tt-gemm-ledger.py`) or pins a
//! deck to it. Included per-lock via `#[path = "common/tt_ledger.rs"] mod tt_ledger;` so the thousand
//! other test binaries do not pay for it.
#![allow(dead_code)]

use std::path::PathBuf;

pub const LEDGER: &str = "benchmarks/silicon/tenstorrent_gemm.json";
pub const ORIGIN_TTNN_DEFAULT: &str = "ttnn.matmul default heuristic";
pub const ORIGIN_TTNN_OURS: &str = "ttnn.matmul with LOGOS program config";

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

pub fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

pub fn json(rel: &str) -> serde_json::Value {
    serde_json::from_str(&read(rel)).unwrap_or_else(|e| panic!("parse {rel}: {e}"))
}

pub fn ledger() -> serde_json::Value {
    json(LEDGER)
}

pub fn rows(d: &serde_json::Value) -> Vec<serde_json::Value> {
    d["rows"].as_array().cloned().unwrap_or_default()
}

pub fn current_rows(d: &serde_json::Value) -> Vec<serde_json::Value> {
    rows(d)
        .into_iter()
        .filter(|r| r["current"] == true)
        .collect()
}

pub fn row(d: &serde_json::Value, id: &str) -> serde_json::Value {
    rows(d)
        .into_iter()
        .find(|r| r["id"] == id)
        .unwrap_or_else(|| panic!("ledger row {id} missing"))
}

pub fn comparison(d: &serde_json::Value, id: &str) -> serde_json::Value {
    d["comparisons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("ledger comparison {id} missing"))
}

/// The headline comparison and its subject row (the deck's anchor numbers).
pub fn headline(d: &serde_json::Value) -> (serde_json::Value, serde_json::Value) {
    let h = &d["headline"];
    assert!(
        !h.is_null(),
        "the ledger has no headline — record a logos row with --headline plus a matched reference"
    );
    let c = comparison(d, h["comparison"].as_str().unwrap());
    let s = row(d, h["subject"].as_str().unwrap());
    (c, s)
}

/// The example a page compares against for PARITY: the multi-core tt-metal example whose ratio is closest
/// to 1.0 (the one-core example is the SPATIAL comparison, reported separately) — the same rule every
/// generator uses, so a lock and a generator can never disagree about which example row is "the" reference.
pub fn parity_example(comp: &serde_json::Value) -> Option<(String, serde_json::Value)> {
    let ex = comp["vs"]["tt_metal_example"].as_object()?;
    ex.iter()
        .filter(|(p, _)| !p.contains("single_core"))
        .min_by(|a, b| {
            let da = (a.1["ratio"].as_f64().unwrap() - 1.0).abs();
            let db = (b.1["ratio"].as_f64().unwrap() - 1.0).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(p, v)| (p.clone(), v.clone()))
}

pub fn shape_str(r: &serde_json::Value) -> String {
    format!(
        "{}x{}x{}",
        r["shape"]["m"], r["shape"]["n"], r["shape"]["k"]
    )
}

pub fn knobs_str(k: &serde_json::Value) -> String {
    format!(
        "{}-fp32_{}-l1acc_{}",
        k["fidelity"].as_str().unwrap(),
        k["fp32_dest_acc"].as_bool().unwrap() as u8,
        k["packer_l1_acc"].as_bool().unwrap() as u8
    )
}

/// The text with digit-group commas removed, so "9,735" in prose matches the ledger's 9735.
pub fn digits(s: &str) -> String {
    s.replace(",", "")
}

/// Does `text` quote the integer `n` as a whole number (not as a substring of a longer number)?
pub fn quotes_number(text: &str, n: u64) -> bool {
    let d = digits(text);
    let needle = n.to_string();
    let bytes = d.as_bytes();
    let mut from = 0;
    while let Some(pos) = d[from..].find(&needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_digit();
        let after_ok = end >= bytes.len() || !bytes[end].is_ascii_digit();
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// ★ A CYCLE COUNT IS NOT A ROW ID — resolve a document's table lines against ledger rows as a MATCHING.
///
/// `quotes_number` answers "does this text quote this number", which is the right question for a
/// sentence and the WRONG one for a table: several current rows now share a cycle count (the 640^3
/// multicast-split plan measures 8,912 at HiFi4 twice and at HiFi2 once), so "the line for this row"
/// does not exist as a lookup. A first-match lookup silently lets two rows claim one line while a third
/// row has none, and then fails whichever row it resolved second — an accurate table, failed for a
/// reason that is not about the table.
///
/// So each row claims a DISTINCT line that describes it completely. With a line count that already
/// matches the row count, that says every row appears exactly once. Returns the lines no row claimed.
pub fn claim_distinct_lines<'a>(
    rows: &[serde_json::Value],
    lines: &[&'a str],
    describes: impl Fn(&str, &serde_json::Value) -> bool,
    id: impl Fn(&serde_json::Value) -> String,
) -> Vec<&'a str> {
    let mut unclaimed: Vec<&str> = lines.to_vec();
    for r in rows {
        let i = unclaimed
            .iter()
            .position(|l| describes(l, r))
            .unwrap_or_else(|| {
                panic!(
                "no unclaimed table line describes {} — a line must carry every field the row is \
                 identified by, and no two rows may share one line. Unclaimed lines:\n{}",
                id(r),
                unclaimed.join("\n")
            )
            });
        unclaimed.remove(i);
    }
    unclaimed
}

pub fn lines_quoting(text: &str, n: u64) -> Vec<String> {
    text.lines()
        .filter(|l| quotes_number(l, n))
        .map(|l| l.to_string())
        .collect()
}

fn near(line: &str, word: &str, anchor: &str, window: usize) -> bool {
    let low = line.to_lowercase();
    let mut from = 0;
    while let Some(pos) = low[from..].find(word) {
        let at = from + pos;
        let lo = at.saturating_sub(window);
        let hi = (at + word.len() + window).min(low.len());
        if low[lo..hi].contains(anchor) {
            return true;
        }
        from = at + 1;
    }
    false
}

/// The vocabulary that keeps "tt-metal example" and "ttnn production op" from being conflated:
/// on every line that talks about Tenstorrent, "hand-written/hand-tuned/hand-optimized" must
/// sit within 60 chars of "example", "vendor/production" within 60 chars of "ttnn", and
/// "cycle-for-cycle" must share its line with the tt-metal example. The BEATS phrases are allowed only
/// while the ledger's headline matched-numerics ratio against ttnn's default is <= 1.0.
pub fn assert_gemm_vocabulary(md: &str, d: &serde_json::Value) {
    let (c, _) = headline(d);
    let beats_allowed = c["vs"]["ttnn_production_default"]["ratio"]
        .as_f64()
        .map(|r| r <= 1.0)
        .unwrap_or(false);
    // Only lines ABOUT Tenstorrent are policed for the example-vs-vendor vocabulary (a TileLoom or CUDA
    // sentence may say "vendor libs" or "hand-tuned CUDA" without conflating anything here).
    let cue = |l: &str| {
        let low = l.to_lowercase();
        ["tenstorrent", "ttnn", "tensix", "tt-metal"]
            .iter()
            .any(|c| low.contains(c))
    };
    for line in md.lines().filter(|l| cue(l)) {
        let low = line.to_lowercase();
        for word in [
            "hand-written",
            "handwritten",
            "hand-tuned",
            "hand-optimized",
            "hand written",
            "hand tuned",
        ] {
            if low.contains(word) {
                assert!(
                    near(line, word, "example", 60),
                    "`{word}` must name the tt-metal EXAMPLE within 60 chars (it is not the vendor's production op):\n{line}"
                );
            }
        }
        for word in ["vendor", "production"] {
            if low.contains(word) {
                assert!(
                    near(line, word, "ttnn", 60),
                    "`{word}` must be tied to `ttnn` within 60 chars:\n{line}"
                );
            }
        }
        if low.contains("cycle-for-cycle") {
            assert!(
                low.contains("tt-metal") && low.contains("example"),
                "cycle-for-cycle parity is against the tt-metal EXAMPLE and the line must say so:\n{line}"
            );
            let tie = parity_example(&c)
                .map(|(_, e)| e["ratio"].as_f64().unwrap() == 1.0)
                .unwrap_or(false);
            assert!(tie, "`cycle-for-cycle` may be said only at an EXACT tie with the parity example (ledger ratio != 1.0):\n{line}");
        }
        for phrase in [
            "beats the vendor",
            "beat the vendor",
            "beats ttnn",
            "faster than ttnn",
            "faster than the vendor",
            "beats the production",
            "ahead of ttnn",
        ] {
            if low.contains(phrase) {
                assert!(
                    beats_allowed,
                    "`{phrase}` is an overclaim: the ledger's matched-numerics ratio vs ttnn's default is > 1.0 (ttnn is ahead):\n{line}"
                );
            }
        }
    }
}

/// Every line that quotes a ledger row's cycle count must carry that row's math fidelity, so the
/// LoFi/HiFi2/HiFi4 rows can never be quoted as if they were comparable.
pub fn assert_every_quoted_cycle_names_its_fidelity(md: &str, d: &serde_json::Value) {
    // A cycle count is matched to a row BY VALUE, so two rows of different shapes can share one number (a
    // 640^3 HiFi2 row and a 1024^3 HiFi4 row both landed on 7,077). A line that carries SOME row's fidelity
    // for that number is labelled; demanding every colliding row's fidelity on one line is not satisfiable
    // and is not the property we want. The property is: no cycle count appears unlabelled.
    let mut by_cycles: std::collections::BTreeMap<u64, Vec<String>> =
        std::collections::BTreeMap::new();
    for r in current_rows(d) {
        by_cycles
            .entry(r["cycles"].as_u64().unwrap())
            .or_default()
            .push(r["knobs"]["fidelity"].as_str().unwrap().to_string());
    }
    for (cyc, fids) in by_cycles {
        for line in lines_quoting(md, cyc) {
            assert!(
                fids.iter().any(|f| line.contains(f.as_str())),
                "the line quotes {cyc} cycles without naming a fidelity (rows with that count are {fids:?}):\n{line}"
            );
        }
    }
}

/// No line may quote both sides of an unmatched-numerics pair unless it also labels both fidelities
/// and says the pair is unmatched.
pub fn assert_no_unmatched_pair_quoted_as_head_to_head(md: &str, d: &serde_json::Value) {
    // ★ A CYCLE COUNT DOES NOT IDENTIFY A ROW, so this check must be stated over every READING of a line,
    // not over one arbitrary resolution of it.
    //
    // The ledger now holds THREE current rows at 8,912 cycles — 640^3 multicast-split at HiFi4 twice and
    // at HiFi2 once. Resolving "8912" to whichever row happened to be found first made this check fail
    // three accurate sentences in a row: "8912 vs 9735, both HiFi4" (a matched pair), "8912 vs 7554, both
    // HiFi4" (a matched pair), and an unmatched line that labels itself correctly for the HiFi4 row while
    // the checker was reading the HiFi2 one. Each "fix" only moved the collision along.
    //
    // The property that is actually wanted: a line quoting two cycle counts must have SOME true reading —
    // some pair of current rows with those counts whose fidelities the line names, and which the line
    // describes correctly (same fidelity, or flagged unmatched). If no reading is true, the line is
    // wrong however it was meant; if one is, the line is defensible and the ambiguity is the ledger's,
    // not the page's. Pinning by row id in the page would remove the ambiguity at the source.
    let current: Vec<(u64, String)> = current_rows(d)
        .iter()
        .filter_map(|r| {
            Some((
                r["cycles"].as_u64()?,
                r["knobs"]["fidelity"].as_str()?.to_string(),
            ))
        })
        .collect();
    for u in d["unmatched_comparisons"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let s = row(d, u["subject"].as_str().unwrap());
        let o = row(d, u["other"].as_str().unwrap());
        let (sc, oc) = (s["cycles"].as_u64().unwrap(), o["cycles"].as_u64().unwrap());
        let (sf, of) = (
            s["knobs"]["fidelity"].as_str().unwrap(),
            o["knobs"]["fidelity"].as_str().unwrap(),
        );
        for line in md.lines() {
            if !(quotes_number(line, sc) && quotes_number(line, oc)) {
                continue;
            }
            let low = line.to_lowercase();
            let flagged = low.contains("unmatched")
                || low.contains("not matched")
                || low.contains("different numerics");
            // Every pair of current rows the line could be quoting, and whether that reading is true.
            let has_true_reading = current.iter().filter(|(c, _)| *c == sc).any(|(_, fa)| {
                current.iter().filter(|(c, _)| *c == oc).any(|(_, fb)| {
                    line.contains(fa.as_str())
                        && line.contains(fb.as_str())
                        && (fa == fb || flagged)
                })
            });
            assert!(
                has_true_reading,
                "{sc} ({sf}) and {oc} ({of}) differ in numerics — no reading of this line is true: it must name the fidelity of both rows it quotes, and say `unmatched` unless they share one:\n{line}"
            );
        }
    }
}

/// The frontier entry for a (chip, shape, knobs) id, if the ledger has one.
pub fn frontier(id: &str) -> Option<serde_json::Value> {
    ledger()["frontier"]
        .as_array()?
        .iter()
        .find(|f| f["id"] == id)
        .cloned()
}

/// **A claim about what the COMPILER does may only quote a compiler-chosen row.** A row measured with the
/// blocking, packing or strategy pinned on the command line shows what the MACHINE can do at a chosen plan;
/// only an unpinned row shows what the compiler picks unaided. So any sentence in `md` that says the compiler
/// chooses / auto-tunes / picks, and quotes a cycle count, must quote a number the compiler actually chose.
pub fn assert_compiler_choice_claims_quote_a_chosen_row(md: &str) {
    let chosen: Vec<u64> = ledger()["frontier"]
        .as_array()
        .map(|fs| {
            fs.iter()
                .filter_map(|f| f["compiler_chosen"]["cycles"].as_u64())
                .collect()
        })
        .unwrap_or_default();
    let d = digits(md);
    for line in d.lines() {
        let low = line.to_lowercase();
        let claims_choice = [
            "compiler chooses",
            "compiler picks",
            "auto-tunes",
            "autotunes",
            "chooses the plan",
            "unaided",
        ]
        .iter()
        .any(|p| low.contains(p));
        if !claims_choice {
            continue;
        }
        let quoted: Vec<u64> = line
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|t| t.parse::<u64>().ok())
            .filter(|n| *n > 1000)
            .collect();
        for n in quoted {
            assert!(
                chosen.contains(&n),
                "a sentence claiming the compiler chooses quotes {n}, which no compiler-chosen row achieved \
                 (chosen rows: {chosen:?}). A pinned row shows what the machine can do, not what the compiler does: {line}"
            );
        }
    }
}

/// ★ THE RETRACTION, made mechanical (2026-09-03, settled and independently reproduced).
///
/// The ttsim whole-program counter does NOT measure work. Holding the configuration fixed and
/// sweeping K produced bit-identical counts across a 4x range (`[6205]` at K=512/1024/2048), and the
/// fidelity ladder ran BACKWARDS. What it actually tracks is how many output tiles a core MOVES
/// across the mesh: monotone in output area over a 256x range, flat in K over a 6.4x range.
///
/// So a ttsim cycle row is a real measurement of movement choreography at matched numerics, and is
/// NOT a throughput or speed claim. This assertion is what stops any deck from drifting back: a
/// document quoting these rows must not pair them with speed language, and must say what the measure
/// is. Both `gc_supercompiler_pdf_lock` and `gc_supercompiler_v2_pdf_lock` call it, so the two decks
/// cannot end up on opposite sides of the retraction in front of the same reader.
pub fn assert_cycles_are_not_quoted_as_throughput(md: &str, d: &serde_json::Value) {
    let cycles: Vec<u64> = rows(d)
        .iter()
        .filter_map(|r| r["cycles"].as_u64())
        .collect();
    let speech = [
        "faster than",
        "beats",
        "beat ",
        "outruns",
        "speedup",
        "speed-up",
        "throughput win",
        "x faster",
        "% faster",
        "slower than",
    ];
    let flat = digits(md);
    for line in flat.lines() {
        let low = line.to_lowercase();
        // Only lines that actually quote a ledger cycle count are policed.
        let quotes_row = cycles.iter().any(|c| quotes_number(line, *c));
        if !quotes_row {
            continue;
        }
        for phrase in speech {
            assert!(
                !low.contains(phrase),
                "RETRACTED FRAMING: this line quotes a ttsim cycle count together with `{phrase}`. \
                 The counter tracks OUTPUT MOVEMENT across the mesh, not work — bit-identical across \
                 a 4x sweep of K at fixed config — so these rows are a movement-choreography result \
                 at matched numerics and can never be a speed claim. Say what the measure is \
                 instead:\n{line}"
            );
        }
    }
    // If the document quotes ttsim rows at all, it owes the reader the measurement basis.
    let any = cycles.iter().any(|c| quotes_number(&flat, *c));
    if any {
        let low = flat.to_lowercase();
        assert!(
            low.contains("output movement")
                || low.contains("output-tile movement")
                || low.contains("movement choreography"),
            "this document quotes ttsim cycle rows but never says what the counter measures. It \
             tracks output movement across the mesh, not work — state that, or drop the rows."
        );
    }
}
