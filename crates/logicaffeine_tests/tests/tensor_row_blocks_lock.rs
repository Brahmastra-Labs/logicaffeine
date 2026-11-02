//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ROW BLOCKS — the tensor sugars over a block of rows, as a prefill chunk writes them.
//!
//! A prefill chunk is `T` rows of one sequence at consecutive positions. Its attention layer rotates every row's
//! queries and keys by that row's own angles and appends every row's key and value to the cache — the same work a
//! decode step does for one row, over the block at once.
//!
//! Pinned, on the VM and the tree-walker (which must agree):
//!   1. `rope of Q with cos RC and sin RS [across H heads]` over an `R`-row `Q` ROTATES EVERY ROW. With `R`-row tables
//!      row `r` turns by table row `r` — exactly `rope` of row `r` alone with row `r` of the tables; with one-row tables
//!      every row turns by the same angles. A table of any other row count, and cos/sin tables of different shapes,
//!      are refused by name.
//!   2. `Set rows from P of T to X` WRITES `X`'s `R` rows at rows `P … P + R − 1` of `T` — exactly `R` single-row
//!      writes. A block leaving the table (`P < 1` or `P + R − 1` past its last row) fails at run time with one
//!      message on every tier; the position expression runs once; a width or element-kind mismatch is a parse error.
//!   3. THE SN50 COMPUTES THE ROPE OVER ROWS. Per-row tables and one-row tables, heads narrower than the site's row
//!      budget (the flat form) and heads streamed one per item (the blocked form), lower whole and evaluate to the
//!      VM's values — the fp32 twin EXACTLY (every input is a multiple of 1/64, so every product and sum is exact in
//!      f32), the bf16 chip within two bf16 ulps. A per-row table is never read as one table for every row.
//!   4. THE SN50 APPENDS THE BLOCK. `Set rows` lowers whole into a table of flat rows, of whole-tile rows, and into a
//!      DENSE cache (the layout a flash attention reads) — from any row, a block straddling two tile rows, ending at the
//!      table's last row, taller than a tile row, from a declared block or a matmul's — and every row of the table is
//!      the VM's, exactly. A computed position outside `1 ..= rows − R + 1` faults at its guard. A block assembled row by
//!      row (the prefill chunk's keys) is laid out as the table it is appended to — from its first row write, with no
//!      relayout between.
//!
//!   5. WHAT A STEP CARRIES. A table the step assembles whole — every row written at a literal row before anything reads
//!      it — is the step's own; a table written at a row the data gives, read before its rows are all written, or left
//!      partly unwritten is carried to the next step (`carried_state`, what a bundle's host allocates for a session).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tensor_row_blocks_lock`
//!
//! ★ MUTATION PROBES (each run 2026-09-25 in a private copy — break, observe red):
//!   • the blocked rope's tables do not move with the row (`load_rc` without its row step) → the SN50 rope test reds on
//!     the fp32 twin's exact rotation (`rows 2, heads 3, hd 64`, a table row per row);
//!   • the flat rope reads table row 0 for every head (`row_shift` = 0) → the same test reds (`rows 3, heads 2, hd 4`);
//!   • the recognizer reads a per-row table as one (`rope_table_index` → one row) → the same test reds: a per-row table
//!     is never lowered as a broadcast;
//!   • the dense append drops the item row from membership (`lo = c`) → the dense and assembled appends red on the fp32
//!     twin; drops the in-page shift (`o` stride 0) → the unaligned starts (17, 31) red; does not clamp its start
//!     (`K0' = K0`) → the block ending at the last row faults (`TileGatherOutOfBound`);
//!   • the flat form writes one row of the block → the flat appends and both prefill-chunk locks red; the whole-tile form
//!     writes one row → the whole-tile appends red;
//!   • the block is not tied to its dense table's layout → the assembled block gains a relayout (the structure assertion
//!     reds) and a declared block's bridge exposes its fill (red until the rows→matrix relayout carried fills — fixed);
//!   • the recognizer reads the block's check bound as the table's rows → the guard's extent, `Kc`'s element count and
//!     both prefill-chunk locks red;
//!   • `carried_state` keeps the tables a step assembles whole → the carried-state test and the prefill bundle's
//!     state set red (`RC_chunk`, `RS_chunk`, `bod*` listed as carried).
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

fn val(seed: usize) -> f64 {
    (((seed.wrapping_mul(7919).wrapping_add(13)) % 129) as f64 - 64.0) / 64.0
}

fn declare(name: &str, rows: usize, cols: usize, salt: usize) -> String {
    let mut s = format!("Let {name} be a {rows} by {cols} tensor of Real.\n");
    for i in 0..rows * cols {
        s += &format!(
            "Set item {} of {name} to {:?}.\n",
            i + 1,
            val(i * 31 + salt)
        );
    }
    s
}

fn lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Tree-walker and VM agree; returns (output, error).
fn tiers(src: &str) -> (Vec<String>, Option<String>) {
    let (tw, vm) = (tw_outcome(src), vm_outcome(src));
    assert_eq!(
        (lines(&tw.output), &tw.error),
        (lines(&vm.output), &vm.error),
        "the VM and the tree-walker disagree on:\n{src}"
    );
    (lines(&tw.output), tw.error)
}

fn ran(src: &str, what: &str) -> Vec<String> {
    let (out, err) = tiers(src);
    assert_eq!(err, None, "{what}: {err:?}\n{src}");
    out
}

fn show_all(name: &str, n: usize) -> String {
    (1..=n)
        .map(|i| format!("Show item {i} of {name}.\n"))
        .collect()
}

// ───────────────────────────── 1. rope over rows ─────────────────────────────

/// A rope shape: `rows` query rows of `heads` heads of `hd`, rotating the leading `2·half` of each head.
#[derive(Clone, Copy, Debug)]
struct Rope {
    rows: usize,
    heads: usize,
    hd: usize,
    half: usize,
}

const FULL: Rope = Rope {
    rows: 3,
    heads: 2,
    hd: 4,
    half: 2,
};
const PARTIAL: Rope = Rope {
    rows: 4,
    heads: 3,
    hd: 6,
    half: 1,
};

fn across(r: &Rope) -> String {
    if 2 * r.half == r.hd {
        String::new()
    } else {
        format!(" across {} heads", r.heads)
    }
}

/// The block: `Y` = rope of the whole `Q`, tables of `table_rows` rows.
fn rope_block(r: &Rope, table_rows: usize) -> String {
    let w = r.heads * r.hd;
    let mut s = String::from("## Main\n");
    s += &declare("Q", r.rows, w, 1);
    s += &declare("RC", table_rows, r.half, 2);
    s += &declare("RS", table_rows, r.half, 3);
    s += &format!("Let Y be rope of Q with cos RC and sin RS{}.\n", across(r));
    s += &show_all("Y", r.rows * w);
    s
}

/// The block as its rows: row `k` of `Q` rotated alone, by row `k` of the tables (or by the one-row tables).
fn rope_rows(r: &Rope, table_rows: usize) -> String {
    let w = r.heads * r.hd;
    let mut s = String::from("## Main\n");
    s += &declare("Q", r.rows, w, 1);
    s += &declare("RC", table_rows, r.half, 2);
    s += &declare("RS", table_rows, r.half, 3);
    for k in 1..=r.rows {
        let t = if table_rows == 1 { 1 } else { k };
        s += &format!(
            "Let q{k} be row {k} of Q.\nLet c{k} be row {t} of RC.\nLet s{k} be row {t} of RS.\n"
        );
        s += &format!(
            "Let y{k} be rope of q{k} with cos c{k} and sin s{k}{}.\n",
            across(r)
        );
    }
    for k in 1..=r.rows {
        s += &show_all(&format!("y{k}"), w);
    }
    s
}

#[test]
fn rope_over_rows_turns_each_row_by_its_own_table_row() {
    for r in [FULL, PARTIAL] {
        let block = ran(&rope_block(&r, r.rows), &format!("{r:?} block"));
        let rows = ran(&rope_rows(&r, r.rows), &format!("{r:?} rows"));
        assert_eq!(
            block.len(),
            r.rows * r.heads * r.hd,
            "{r:?}: every element of every row is shown"
        );
        assert_eq!(
            block, rows,
            "{r:?}: row k of the block is row k rotated alone by table row k"
        );
        let w = r.heads * r.hd;
        assert_ne!(
            block[..w],
            block[w..2 * w],
            "{r:?}: the rows are distinct rotations"
        );
    }
}

#[test]
fn rope_over_rows_with_one_row_tables_turns_every_row_by_the_same_angles() {
    for r in [FULL, PARTIAL] {
        let block = ran(&rope_block(&r, 1), &format!("{r:?} block, one-row tables"));
        let rows = ran(&rope_rows(&r, 1), &format!("{r:?} rows, one-row tables"));
        assert_eq!(block, rows, "{r:?}: every row turns by the one table row");
    }
}

#[test]
fn a_rope_table_of_another_row_count_or_shape_is_refused_by_name() {
    let r = FULL;
    let err = tiers(&rope_block(&r, 2))
        .1
        .unwrap_or_else(|| panic!("a 2-row table for a 3-row Q must be refused"));
    assert!(
        err.contains("rope") && err.contains("row"),
        "the refusal names the rope and its rows: {err}"
    );
    let w = r.heads * r.hd;
    let mut s = String::from("## Main\n");
    s += &declare("Q", r.rows, w, 1);
    s += &declare("RC", r.rows, r.half, 2);
    s += &declare("RS", 1, r.half, 3);
    s += "Let Y be rope of Q with cos RC and sin RS.\nShow item 1 of Y.\n";
    let err = tiers(&s)
        .1
        .unwrap_or_else(|| panic!("cos and sin tables of different shapes must be refused"));
    assert!(
        err.contains("rope") && err.contains("RS"),
        "the refusal names the mismatched table: {err}"
    );
}

#[test]
fn a_one_row_rope_is_unchanged() {
    let r = Rope { rows: 1, ..FULL };
    let out = ran(&rope_block(&r, 1), "one row");
    assert_eq!(
        out,
        ran(&rope_rows(&r, 1), "one row as a row"),
        "one row with one-row tables is the decode rope"
    );
}

// ───────────────────────────── 2. Set rows ─────────────────────────────

const ROWS: usize = 6;
const COLS: usize = 4;
const BLOCK: usize = 3;

fn set_rows_block(from: &str, bind: Option<usize>) -> String {
    let mut s = String::from("## Main\n");
    s += &declare("Kc", ROWS, COLS, 5);
    s += &declare("X", BLOCK, COLS, 6);
    if let Some(p) = bind {
        s += &format!("Let P be {p}.\n");
    }
    s += &format!("Set rows from {from} of Kc to X.\n");
    s += &show_all("Kc", ROWS * COLS);
    s
}

fn set_rows_one_by_one(p: usize) -> String {
    let mut s = String::from("## Main\n");
    s += &declare("Kc", ROWS, COLS, 5);
    s += &declare("X", BLOCK, COLS, 6);
    for k in 1..=BLOCK {
        s += &format!(
            "Let x{k} be row {k} of X.\nSet row {} of Kc to x{k}.\n",
            p + k - 1
        );
    }
    s += &show_all("Kc", ROWS * COLS);
    s
}

#[test]
fn set_rows_writes_the_block_at_its_position() {
    for p in 1..=ROWS + 1 - BLOCK {
        let want = ran(&set_rows_one_by_one(p), &format!("one by one from {p}"));
        assert_eq!(
            ran(
                &set_rows_block(&p.to_string(), None),
                &format!("literal from {p}")
            ),
            want,
            "from {p}: the block is its rows written in turn"
        );
        assert_eq!(
            ran(&set_rows_block("P", Some(p)), &format!("named from {p}")),
            want,
            "from a computed {p}"
        );
    }
    let untouched = ran(&set_rows_one_by_one(1), "from 1");
    let at_end = ran(&set_rows_one_by_one(ROWS + 1 - BLOCK), "at the end");
    assert_ne!(untouched, at_end, "the position moves the block");
}

#[test]
fn a_block_leaving_the_table_fails_with_one_message_on_every_tier() {
    for p in [0usize, ROWS + 2 - BLOCK] {
        for (src, what) in [
            (set_rows_block(&p.to_string(), None), "literal"),
            (set_rows_block("P", Some(p)), "named"),
        ] {
            let err = tiers(&src)
                .1
                .unwrap_or_else(|| panic!("a block from {p} ({what}) must fail"));
            assert!(
                err.contains("out of range") && err.contains("Kc has 6 rows"),
                "from {p} ({what}): {err}"
            );
        }
    }
}

#[test]
fn the_position_expression_runs_once() {
    let mut s = String::from("## To count (c: Int) -> Int:\n    Show c.\n    Return c.\n\n");
    s += &set_rows_block("(count(2))", None);
    let out = ran(&s, "counted");
    assert_eq!(
        out.iter().filter(|l| l.as_str() == "2").count(),
        1,
        "count(2) runs exactly once: {out:?}"
    );
}

#[test]
fn a_block_of_another_width_or_kind_is_a_parse_error() {
    let mut wide = String::from("## Main\n");
    wide += &declare("Kc", ROWS, COLS, 5);
    wide += &declare("X", BLOCK, COLS + 1, 6);
    wide += "Set rows from 1 of Kc to X.\n";
    let err = tiers(&wide)
        .1
        .unwrap_or_else(|| panic!("a block wider than the table must be refused"));
    assert!(err.contains("Set rows") && err.contains("Kc"), "{err}");

    let mut kinds = String::from("## Main\n");
    kinds += &declare("Kc", ROWS, COLS, 5);
    kinds += &format!("Let X be a {BLOCK} by {COLS} tensor of Int.\n");
    kinds += "Set rows from 1 of Kc to X.\n";
    let err = tiers(&kinds)
        .1
        .unwrap_or_else(|| panic!("an Int block into a Real table must be refused"));
    assert!(
        err.contains("Set rows") && err.contains("Int") && err.contains("Real"),
        "{err}"
    );
}

// ───────────────────────────── 3. the SN50 computes the rope over rows ─────────────────────────────

fn sn50() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

fn fp32_twin() -> SpatialTarget {
    let mut t = sn50();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. }
        | ComputeCap::Elementwise { precisions }
        | ComputeCap::RowReduce { precisions } = c
        {
            if precisions.contains(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

/// Heads streamed one per item: the rows' heads are more tiles than a site's row budget, a head is not.
const BLOCKED: Rope = Rope {
    rows: 2,
    heads: 3,
    hd: 64,
    half: 32,
};
/// The blocked form with a partial rotary slice.
const BLOCKED_PARTIAL: Rope = Rope {
    rows: 3,
    heads: 2,
    hd: 128,
    half: 32,
};

fn composed(src: &str, target: &SpatialTarget, what: &str) -> SpatialProgram {
    let pl = compile_program_to_spatial(src, target)
        .unwrap_or_else(|e| panic!("{what}: walk refused: {e:?}"));
    let sp = pl
        .whole()
        .unwrap_or_else(|| panic!("{what}: not lowered whole: {}", pl.refusal()))
        .clone();
    assert_eq!(sp.phase(), Phase::Verified, "{what}: the program certifies");
    assert!(
        sp.certificate.as_ref().is_some_and(|c| c.recheck(&sp)),
        "{what}: its certificate rechecks"
    );
    sp
}

fn rounded(src: &str) -> String {
    src.lines()
        .map(|l| {
            match l
                .strip_prefix("Show item ")
                .and_then(|r| r.strip_suffix('.'))
            {
                Some(item) => format!("Show round(item {item} * 1000000).\n"),
                None => format!("{l}\n"),
            }
        })
        .collect()
}

fn chip_verdict(src: &str, what: &str) {
    let src = rounded(src);
    let (out, err) = tiers(&src);
    assert_eq!(err, None, "{what}: {err:?}");
    let want = oracle_ints(&out.join("\n")).expect("ints");
    let twin = composed(&src, &fp32_twin(), what);
    let got = shown_from_outputs(
        &src,
        &twin,
        &eval_choreography(&twin, &inputs_from_fills(&twin).expect("fills"))
            .unwrap_or_else(|e| panic!("{what}: fp32 eval: {e:?}")),
    )
    .expect("shown");
    assert_eq!(
        got, want,
        "{what}: the fp32 twin computes the VM's rotation exactly"
    );
    let chip = composed(&src, &sn50(), what);
    let got = shown_from_outputs(
        &src,
        &chip,
        &eval_choreography(&chip, &inputs_from_fills(&chip).expect("fills"))
            .unwrap_or_else(|e| panic!("{what}: bf16 eval: {e:?}")),
    )
    .expect("shown");
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        let x = (*v as f64 / 1e6).abs();
        let ulp = if x == 0.0 {
            0.0
        } else {
            2f64.powi(x.log2().floor() as i32 - 7)
        };
        assert!(
            ((g - v).abs() as f64) <= (2.0 * ulp * 1e6).max(1.0),
            "{what}: CHIP Show #{}: bf16 {g} vs VM {v}",
            i + 1
        );
    }
}

#[test]
fn the_rope_over_rows_lowers_on_sn50_and_computes_the_vms_values() {
    for r in [FULL, PARTIAL, BLOCKED, BLOCKED_PARTIAL] {
        chip_verdict(
            &rope_block(&r, r.rows),
            &format!("{r:?}, a table row per row"),
        );
        chip_verdict(
            &rope_block(&r, 1),
            &format!("{r:?}, one table row for every row"),
        );
    }
}

// ───────────────────────────── 4. the SN50 appends the block ─────────────────────────────

/// A block append: `Kc` of `rows` rows `cols` wide, `X` of `block` rows written from `p` — a literal or a computed `P`,
/// `X` declared or a matmul's (`A · I`, `I` the identity) — and, `attend`, a flash attention reading `Kc` after it, so
/// `Kc` is laid out as the dense cache. Every element of `Kc` is shown.
fn append(
    rows: usize,
    cols: usize,
    block: usize,
    p: usize,
    named: bool,
    from_matmul: bool,
    attend: bool,
) -> String {
    let mut s = String::from("## Main\n");
    s += &declare("Kc", rows, cols, 5);
    if from_matmul {
        s += &declare("A", block, cols, 6);
        let identity: Vec<String> = (0..cols * cols)
            .filter(|i| i / cols == i % cols)
            .map(|i| format!("Set item {} of I to 1.0.\n", i + 1))
            .collect();
        s += &format!(
            "Let I be a {cols} by {cols} tensor of Real.\n{}",
            identity.concat()
        );
        s += "Let X be A matmul I.\n";
    } else {
        s += &declare("X", block, cols, 6);
    }
    let from = if named {
        s += &format!("Let P be {p}.\n");
        "P".to_string()
    } else {
        p.to_string()
    };
    s += &format!("Set rows from {from} of Kc to X.\n");
    if attend {
        s += &declare("q", 1, cols, 7);
        s += &declare("Vx", rows, cols, 8);
        s += &format!(
            "Let L be {}.\nLet C be attention of q over Kc and Vx with 1 heads 1 groups up to L.\n",
            rows
        );
    }
    s += &show_all("Kc", rows * cols);
    s
}

#[test]
fn the_block_append_lowers_on_sn50_into_every_table_layout() {
    for p in [1usize, 2, 4] {
        for named in [false, true] {
            chip_verdict(
                &append(6, 4, 3, p, named, false, false),
                &format!("flat rows, from {p} (named {named})"),
            );
            chip_verdict(
                &append(6, 32, 3, p, named, false, false),
                &format!("whole-tile rows, from {p} (named {named})"),
            );
        }
    }
    // The dense cache: two tile rows (32 + 8 rows). From the first row, straddling the tile-row boundary, inside the
    // second tile row, and ending at the last row.
    for p in [1usize, 17, 31, 38] {
        for (named, from_matmul) in [(true, false), (false, false), (true, true)] {
            chip_verdict(
                &append(40, 32, 3, p, named, from_matmul, true),
                &format!("dense, 3 rows from {p} (named {named}, matmul {from_matmul})"),
            );
        }
    }
    // A block taller than a tile row, into a cache of three tile rows (80 rows).
    for p in [1usize, 20, 47] {
        chip_verdict(
            &append(80, 32, 34, p, true, false, true),
            &format!("dense, 34 rows from {p}"),
        );
    }
}

/// The block a prefill chunk appends: its rows assembled one `Set row` at a time — so the block's layout is decided by
/// the table it is appended to, and its row writes take it.
fn append_assembled(rows: usize, cols: usize, block: usize, p: usize) -> String {
    let mut s = String::from("## Main\n");
    s += &declare("Kc", rows, cols, 5);
    s += &format!("Let X be a {block} by {cols} tensor of Real.\n");
    for r in 1..=block {
        s += &declare(&format!("x{r}"), 1, cols, 10 + r);
        s += &format!("Set row {r} of X to x{r}.\n");
    }
    s += &format!("Let P be {p}.\nSet rows from P of Kc to X.\n");
    s += &declare("q", 1, cols, 7);
    s += &declare("Vx", rows, cols, 8);
    s += &format!(
        "Let L be {rows}.\nLet C be attention of q over Kc and Vx with 1 heads 1 groups up to L.\n"
    );
    s += &show_all("Kc", rows * cols);
    s
}

#[test]
fn a_block_assembled_row_by_row_takes_the_layout_of_the_table_it_is_appended_to() {
    for p in [1usize, 31] {
        chip_verdict(
            &append_assembled(40, 32, 3, p),
            &format!("dense cache, an assembled block of 3 from {p}"),
        );
    }
    // The block is laid out as the cache from its first row write: no relayout bridges it.
    let sp = composed(
        &rounded(&append_assembled(40, 32, 3, 5)),
        &sn50(),
        "assembled",
    );
    assert!(
        !sp.sections.iter().any(|x| x.name.starts_with("relayout")),
        "the assembled block needs no relayout: {:?}",
        sp.sections.iter().map(|x| &x.name).collect::<Vec<_>>()
    );
}

#[test]
fn a_computed_block_position_outside_the_table_faults_at_its_guard() {
    use logicaffeine_compile::spatial::eval::ChoreoFault;
    use logicaffeine_compile::spatial::ir::TensorInit;
    // The flat form guards the block's first ELEMENT offset `(P − 1)·cols`, the dense form its first ROW `P − 1`.
    for (rows, cols, attend, extent, what) in [
        (6, 4, false, (6 - 3 + 1) * 4, "flat rows"),
        (40, 32, true, 40 - 3 + 1, "dense"),
    ] {
        let sp = composed(
            &rounded(&append(rows, cols, 3, 1, true, false, attend)),
            &sn50(),
            what,
        );
        for p in [0usize, rows - 3 + 2] {
            let mut bad = sp.clone();
            for t in &mut bad.tensors {
                if t.name == "P" {
                    t.init = TensorInit::Values(vec![p as i64]);
                }
            }
            match eval_choreography(&bad, &inputs_from_fills(&bad).expect("fills")) {
                Err(ChoreoFault::GatherOutOfRange { extent: e, .. }) => assert_eq!(e, extent, "{what}: from {p}, checked against the positions the block admits"),
                other => panic!("{what}: a block from {p} leaves the table and must fault at its guard, not {other:?}"),
            }
        }
    }
}

// ───────────────────────────── 5. what a step carries ─────────────────────────────

fn carried(src: &str) -> Vec<String> {
    let rp = logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages(src)
        .expect("the walk lifts it");
    logicaffeine_compile::spatial::passes::recognize_stages::carried_state(&rp)
}

#[test]
fn a_step_carries_what_it_does_not_assemble_whole() {
    let rows = |x: &str| -> String {
        (1..=3)
            .map(|r| {
                format!(
                    "{}Set row {r} of {x} to r{r}.\n",
                    declare(&format!("r{r}"), 1, 4, r)
                )
            })
            .collect()
    };
    let base = format!(
        "## Main\n{}{}",
        declare("Kc", 6, 4, 1),
        declare("B", 3, 4, 2)
    );
    // Assembled: every row at a literal row, read after.
    let assembled = format!(
        "{base}{}Set rows from 2 of Kc to B.\nLet P be 2.\nSet row P of Kc to r1.\n",
        rows("B")
    );
    assert_eq!(
        carried(&assembled),
        vec!["Kc".to_string()],
        "B is assembled whole; the cache, written at a data row, carries"
    );
    // Read before its last row is written: carried.
    let early = format!("## Main\n{}{}Set row 1 of B to r1.\nSet row 2 of B to r2.\nSet rows from 1 of Kc to B.\nSet row 3 of B to r3.\n",
        declare("Kc", 6, 4, 1), (1..=3).map(|r| declare(&format!("r{r}"), 1, 4, r)).collect::<String>() + &declare("B", 3, 4, 2));
    assert!(
        carried(&early).contains(&"B".to_string()),
        "a table read before its rows are all written shows its old rows: carried"
    );
    // Partly written: carried.
    let partial = format!(
        "## Main\n{}{}Set row 1 of B to r1.\nSet row 2 of B to r2.\n",
        declare("B", 3, 4, 2),
        (1..=2)
            .map(|r| declare(&format!("r{r}"), 1, 4, r))
            .collect::<String>()
    );
    assert_eq!(
        carried(&partial),
        vec!["B".to_string()],
        "a row the step never writes carries"
    );
    // A block at a data position: carried.
    let named = format!("{base}Let P be 1.\nSet rows from P of Kc to B.\n");
    assert_eq!(carried(&named), vec!["Kc".to_string()]);
}
