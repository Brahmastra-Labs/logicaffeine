//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! CERTIFIER STRENGTH LOCK — the two programs the certifier used to REFUSE now earn real proofs,
//! and the refusals that remain are the ones that are actually true.
//!
//! The scalar-stage certificate proves `routed grid ≡ the body's own datapath` for every 8-bit
//! input and ships the refutation. Two shapes escaped it, both for the same reason: **a DAG was
//! being copied as a tree.**
//!
//!  1. `examples/rdu_demo/fib.lg` — a 75-cell fast-doubling body whose cells read each other's
//!     results. Lowered through `BoundedExpr` (a `Box` tree), that body UNFOLDS: 1 657 763 nodes
//!     from a few hundred, and a propositional encoding of 10^14 nodes on top. The obligation was
//!     priced and refused. The datapath is now built straight into the SHARED gate DAG
//!     (`cells_to_gates` / `grid_output_to_gates`), and the DAG is clausified in place
//!     (`bitblast::assert_not`), so nothing exponential is ever materialised and `fib` certifies.
//!  2. `examples/rdu_demo/`'s clamp — `If r is greater than 1000` compares a SELECT'S OUTPUT. The
//!     placer asserted a mux's comparison operands were inputs or constants. A computed predicate
//!     is an ordinary dataflow edge; it now takes one of the tile's two routed channels like any
//!     other operand, and two ports reading the same producer share one — which is exactly why
//!     `clamp` routes (its comparison and its else arm are both `r`: ONE channel) and `pick` does
//!     not (a computed predicate over two more computed values: THREE).
//!
//! Cheaper is not the same as weaker, and this lock's job is to prove that. `Cnf::encode`
//! hash-conses on `(op, lit, lit)`, so the grid and the spec collapse onto the same literals —
//! they always did; collapsing one level earlier only removes work. What must still hold is that
//! the certificate is REFUTED, that the INDEPENDENT `check_pr_refutation` accepts it, and that a
//! WRONG datapath earns nothing.
//!
//!  A. `a_multiply_chain_earns_a_real_certificate` — `fib.lg`'s body certifies on both chips: a
//!     real SAT certificate whose `recheck()` (the independent PR/RUP checker) accepts, inside a
//!     stated wall-clock and RSS budget, with the before/after node counts in the message.
//!  B. `the_bounded_form_is_linear_in_cells` — over a 1..6-block chain the shared form grows
//!     LINEARLY in cells (each steady-state block adds exactly 1 743 gates; 75 cells → 6 988
//!     gates) while the tree form multiplies by ~8.8 per block (75 cells → 1 657 763 nodes). The
//!     measured table is in the assert message.
//!  C. `a_wrong_datapath_still_earns_no_certificate` — fail-closed on both paths: mutate the MAC
//!     PE's `psum + a·w` to `psum − a·w` and the tree path returns `None`; mutate a scalar body's
//!     cells and the gate path returns `None`. Sharing must not make the miter vacuous.
//!  D. `a_computed_predicate_routes_and_certifies` — `clamp`'s two-comparison body routes, earns a
//!     certificate that rechecks, and its 64 lanes equal the VM; `pick`'s three-computed-value mux
//!     is refused BY NAME, with the reason text asserted.
//!  E. `the_budget_still_catches_a_genuine_runaway` — the safety net is live code: the same body
//!     that certifies under the production ceiling comes back as the typed `Uncertified { reason }`
//!     under a tiny one, naming both numbers; a datapath that genuinely exceeds the PRODUCTION
//!     ceiling is refused by `GateBudget`, not by the allocator; and the ceiling clears the widest
//!     body the product lowers by a stated margin.
//!
//! ★ MUTATION PROBES (each run once when this lock landed; the lock went red, the impl was
//!   restored and `cmp`-verified byte-identical):
//!   - `cells_to_gates` clones its operand `Word` through `blast_word(&cells_to_bounded(..))`
//!     (i.e. back through the tree) → B reds: the shared count collapses onto the tree count at
//!     the three-block chain, and A times out;
//!   - `bitblast::assert_not` asserts the miter instead of its negation → A, C and D red: nothing
//!     is refutable, so no body earns a certificate;
//!   - `words_equal` returns a constant-true predicate (the vacuity a cheaper encoding could hide)
//!     → C reds: a wrong datapath certifies;
//!   - `unroutable_cell` always returns `None` → D reds: `pick` is no longer refused by name and
//!     the placer panics inside `route_loop_body` instead;
//!   - `certify_gates_within` ignores its `budget` argument → E reds: a tiny ceiling admits an
//!     obligation it must refuse.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test certifier_strength_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use logicaffeine_compile::codegen_sva::bitblast::{
    word_binary, word_size, word_var, words_equal, BlastRefusal, Word,
};
use logicaffeine_compile::codegen_sva::fabric_grid::{
    cells_to_gates, grid_output_to_gates, route_loop_body, tile_channel_demand, unroutable_cell,
    TILE_OPERAND_CHANNELS,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    fabric_to_bounded, AluOp, FabricOp, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::pnr_certificate::{
    certify_gates, certify_gates_within, certify_mapping, CertifyRefusal, GATE_BUDGET_NODES,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::BitVecBoundedOp;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::certificate::{
    bounded_datapath_nodes, scalar_certificate_within, ComputeStageCert,
};
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{Phase, ScalarBody, SpatialProgram, StageKind, TensorRole};
use logicaffeine_compile::spatial::lower::rdu_lanes::lower_whole_program_lane;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

/// The certified bound `spatial::certificate::scalar_certificate` blasts the body at.
const WIDTH: u32 = 8;

/// The demo corpus's ALU + decision bodies, verbatim from `rdu_whole_program_lane_lock` and
/// `bitblast_budget_lock` so all three locks speak about the same programs.
const IF_ELSE: &str = "## To clamp (x: Int) -> Int:\n    Let mutable r be x.\n    If x is less than 0:\n        Set r to 0 - x.\n    If r is greater than 1000:\n        Set r to 1000.\n    Return r.\n\n## Main\n\nShow clamp(7).\n";
const TWO_INPUT: &str = "## To mix (a: Int, b: Int) -> Int:\n    Let mutable r be a * 3 - b.\n    If a is less than b:\n        Set r to b - a.\n    Return r.\n\n## Main\n\nShow mix(2, 9).\n";
const COMPUTED_PREDICATE: &str = "## To pick (n: Int) -> Int:\n    Let bit be (n shifted right by 1) & 1.\n    Let mutable r be n * 3.\n    If bit is 1:\n        Set r to n * 5.\n    Return r.\n\n## Main\n\nShow pick(6).\n";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn demo(name: &str) -> String {
    let p = repo_root().join("examples/rdu_demo").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// `fib.lg`'s fast-doubling body with only the TOP `k` of its six doubling blocks — the knob that
/// walks the multiply chain. At `k == 6` the function is byte-for-byte the demo's (asserted in A).
fn fib_source(k: usize) -> String {
    let mut s = String::from(
        "## To fib (n: Int) -> Int:\n    Let mutable a be 0.\n    Let mutable b be 1.\n",
    );
    for i in (0..6).rev().take(k) {
        let bit = if i == 0 {
            "    Let bit0 be n & 1.\n".to_string()
        } else {
            format!("    Let bit{i} be (n shifted right by {i}) & 1.\n")
        };
        s.push_str(&format!(
            "{bit}    Let c{i} be a * (2 * b - a).\n    Let d{i} be a * a + b * b.\n    Set a to c{i} + bit{i} * (d{i} - c{i}).\n    Set b to d{i} + bit{i} * c{i}.\n"
        ));
    }
    s.push_str("    Return a.\n\n## Main\n\nShow fib(10).\n");
    s
}

fn lower(src: &str, label: &str) -> SpatialProgram {
    lower_whole_program_lane(src, RduChip::Sn50).unwrap_or_else(|e| panic!("{label}: {e:?}"))
}

/// The whole-program lane may place the SAME body on more than one site, so "the Scalar body" is
/// only a well-formed phrase if every Scalar stage carries the same one. Asserted, not assumed.
fn scalar_body(p: &SpatialProgram) -> ScalarBody {
    let mut bodies = p.stages.iter().filter_map(|s| match &s.kind {
        StageKind::Scalar(b) => Some(b),
        _ => None,
    });
    let first = bodies
        .next()
        .expect("the whole-program lane carries a Scalar stage")
        .clone();
    for b in bodies {
        assert_eq!(
            *b, first,
            "{}: every Scalar stage carries the same body",
            p.name
        );
    }
    first
}

/// Likewise for the verdict: every Scalar stage of one program must earn the SAME certificate
/// verdict, or a lock that says "the Scalar stage certified" is speaking about an arbitrary one.
fn scalar_cert(p: &SpatialProgram) -> &ComputeStageCert {
    let c = p
        .certificate
        .as_ref()
        .expect("a verified program carries its certificate");
    let certs: Vec<&ComputeStageCert> = c
        .compute
        .iter()
        .filter(|(s, _)| matches!(p.stages[s.idx()].kind, StageKind::Scalar(_)))
        .map(|(_, c)| c)
        .collect();
    assert!(
        !certs.is_empty(),
        "{}: the Scalar stage's certificate",
        p.name
    );
    let verdict = |c: &ComputeStageCert| match c {
        ComputeStageCert::Sat(_) => "SAT".to_string(),
        ComputeStageCert::Uncertified { reason } => reason.clone(),
    };
    for c in &certs[1..] {
        assert_eq!(
            verdict(c),
            verdict(certs[0]),
            "{}: one body, one verdict",
            p.name
        );
    }
    certs[0]
}

/// The exact obligation `scalar_certificate` refutes, in the SHARED form: the routed grid's output
/// and the body's own datapath, both as gate DAGs at the certified 8-bit bound.
fn gate_obligation(body: &ScalarBody) -> (Word, Word) {
    assert!(
        unroutable_cell(&body.cells).is_none(),
        "only a routable body has a placement obligation"
    );
    let g = route_loop_body(&body.cells, body.n_in, &[body.result]);
    let placed = grid_output_to_gates(&g, 0, WIDTH, &|k| format!("in{k}"))
        .expect("an ALU body's placed grid blasts");
    let spec = cells_to_gates(&body.cells, body.result, WIDTH).expect("an ALU body blasts");
    (placed, spec)
}

/// Resident set size of THIS process, in bytes (Linux `/proc/self/statm`). `None` elsewhere, so the
/// memory half of A is asserted where it can be measured and skipped — never faked — where it cannot.
fn rss_bytes() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
    Some(resident * 4096)
}

/// The VM's answer for `fn_name(args)` in `source` (its own `## Main` replaced by the call).
fn vm_f(source: &str, fn_name: &str, args: &[i64]) -> i64 {
    let call = args
        .iter()
        .map(|p| {
            if *p < 0 {
                format!("(0 - {})", -p)
            } else {
                p.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let funcs = match source.find("## Main") {
        Some(idx) => &source[..idx],
        None => source,
    };
    let prog = format!("{funcs}## Main\n    Show {fn_name}({call}).\n");
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error for {fn_name}({call}): {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("an integer")
        .parse()
        .expect("i64")
}

fn lanes(p: &SpatialProgram, xs: &[i64]) -> Vec<i64> {
    let mut ins = Inputs::default();
    let input = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Input)
        .expect("one input vector");
    ins.tensors.insert(input.id, Value::Int(xs.to_vec()));
    let out = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .expect("one output vector");
    match &eval_choreography(p, &ins)
        .unwrap_or_else(|f| panic!("{}: {f:?}", p.name))
        .tensors[&out.id]
    {
        Value::Int(v) => v.clone(),
        Value::Float(_) => panic!("an integer tensor"),
    }
}

// ───────────────────────────── A ─────────────────────────────

/// The wall-clock a scalar certificate for the widest body in the demo corpus may take. Measured
/// at well under a second; the ceiling is what turns "it is fast" into an assertion.
const FIB_CERT_SECS: u64 = 30;
/// And the RSS it may cost. The tree form of the same obligation was ten petabytes.
const FIB_CERT_RSS_MIB: u64 = 1024;

#[test]
fn a_multiply_chain_earns_a_real_certificate() {
    let src = demo("fib.lg");
    // The generated six-block chain IS the demo program's function, so what certifies here is what
    // `largo emit rdu … fib.lg` certifies — not a lookalike.
    let generated = fib_source(6);
    let demo_fn = &src
        [src.find("## To fib").expect("the fib function")..src.find("## Main").expect("a Main")];
    assert_eq!(
        generated,
        format!("{demo_fn}## Main\n\nShow fib(10).\n"),
        "the generated chain is the demo's fib body verbatim"
    );

    for chip in RduChip::ALL {
        let before = rss_bytes();
        let t0 = Instant::now();
        let p = lower_whole_program_lane(&src, chip)
            .unwrap_or_else(|e| panic!("fib.lg on {}: {e:?}", chip.as_str()));
        let elapsed = t0.elapsed();
        let after = rss_bytes();

        assert_eq!(
            p.phase(),
            Phase::Verified,
            "fib.lg lowers Verified on {}",
            chip.as_str()
        );
        let body = scalar_body(&p);
        let tree_nodes = bounded_datapath_nodes(&body);
        let (placed, spec) = gate_obligation(&body);
        let gates = word_size(&spec).dag_nodes;
        let placed_gates = word_size(&placed).dag_nodes;
        let miter = words_equal(&placed, &spec).expect("the miter is one width");
        let miter_size = logicaffeine_compile::codegen_sva::bitblast::pred_size(&miter);

        let note = format!(
            "\n  fib.lg on {}: {} cells\n    bounded TREE form:  {tree_nodes} BoundedExpr nodes (what it used to build)\n\
             \x20   shared GATE form:   {gates} gates (spec) + {placed_gates} gates (routed grid)\n\
             \x20   miter:              {} gates, a tree of {} nodes never built\n\
             \x20   lowered in {elapsed:?}",
            chip.as_str(),
            body.cells.len(),
            miter_size.dag_nodes,
            if miter_size.saturated() { "≥ u128::MAX".to_string() } else { miter_size.tree_nodes.to_string() },
        );

        let c = match scalar_cert(&p) {
            ComputeStageCert::Sat(c) => c,
            ComputeStageCert::Uncertified { reason } => {
                panic!("fib.lg's body is a routable ALU datapath and must certify — got `{reason}`{note}")
            }
        };
        assert!(
            c.recheck(),
            "the INDEPENDENT PR/RUP checker must accept the certificate{note}"
        );
        assert!(
            !c.is_empty(),
            "a certificate with no refutation steps proves nothing{note}"
        );
        // And the refutation is not decoration: drop its last step and the checker REFUSES it. A
        // certificate whose CNF already falls to unit propagation on its own would still
        // `recheck()` with the proof gutted, and that is the degenerate shape a cheaper encoding
        // could silently drift into.
        assert!(
            !c.with_last_step_removed().recheck(),
            "fib's final refutation step must be load-bearing{note}"
        );

        assert!(
            elapsed < Duration::from_secs(FIB_CERT_SECS),
            "certifying the chain took {elapsed:?}, over the {FIB_CERT_SECS}s bar{note}"
        );
        if let (Some(b), Some(a)) = (before, after) {
            let grew = a.saturating_sub(b) / (1024 * 1024);
            assert!(
                grew < FIB_CERT_RSS_MIB,
                "certifying the chain grew RSS by {grew} MiB, over the {FIB_CERT_RSS_MIB} MiB bar{note}"
            );
        }
        println!(
            "{note}\n    certificate: {} steps over {} vars, rechecked; last step load-bearing: {}",
            c.len(),
            c.num_vars(),
            !c.with_last_step_removed().recheck()
        );
    }
}

// ───────────────────────────── B ─────────────────────────────

/// The shared form may allocate this many gates per CELL of the body it is given. The measured
/// high-water across the whole 1..6-block chain is 93; the ceiling is what makes "linear in cells"
/// an assertion rather than a hope.
const GATES_PER_CELL: u128 = 250;
/// Each extra doubling block MULTIPLIES the tree form by at least this much (measured: ~8.8×), so
/// the two forms are not two spellings of one cost.
const TREE_GROWTH_PER_BLOCK: u128 = 5;
/// `fib`'s chain seeds `a = 0, b = 1`, so the first two doubling blocks partly constant-fold
/// against those literals and cost less than a block in flight. From the THIRD block on every
/// block is a full `a·(2b−a)` / `a²+b²` circuit, and that is where "one more block costs one more
/// block's worth of gates" is a claim worth pinning.
const STEADY_STATE_BLOCK: usize = 3;
/// How far the steady-state per-block increments may drift from each other, in percent. Measured:
/// 0 — blocks 4, 5 and 6 each add exactly the same number of gates.
const INCREMENT_DRIFT_PCT: u64 = 10;

#[test]
fn the_bounded_form_is_linear_in_cells() {
    let mut table = String::from(
        "\n  blocks  cells   tree nodes   tree/cell      gates   gates/cell   +gates   build s\n",
    );
    let mut rows: Vec<(usize, usize, u128, u64)> = Vec::new();
    for k in 1..=6 {
        let body = scalar_body(&lower(&fib_source(k), &format!("fib x{k}")));
        let cells = body.cells.len();
        let tree = bounded_datapath_nodes(&body);
        let t0 = Instant::now();
        let spec = cells_to_gates(&body.cells, body.result, WIDTH).expect("an ALU body blasts");
        let gates = word_size(&spec).dag_nodes;
        let secs = t0.elapsed().as_secs_f64();
        let step = rows.last().map(|(_, _, _, g)| gates - g).unwrap_or(gates);
        table.push_str(&format!(
            "  {k:>6}  {cells:>5}  {tree:>10}  {:>9}  {gates:>9}  {:>10}  {step:>6}  {secs:>7.3}\n",
            tree / cells as u128,
            u128::from(gates) / cells as u128,
        ));
        // Checked HERE, inside the loop: if the shared form ever stops sharing it grows like the
        // tree, and the NEXT row is the one that takes the machine down. The first bad row stops
        // the measurement — this lock may not reproduce the bug it locks.
        assert!(
            u128::from(gates) <= (cells as u128) * GATES_PER_CELL,
            "the {k}-block body built {gates} gates for {cells} cells — more than \
             {GATES_PER_CELL}× a cell is an unfolding, not a share{table}"
        );
        rows.push((k, cells, tree, gates));
    }

    // Two-sided, and this is the whole claim: in steady state one more block costs one more
    // block's worth of GATES (a constant), while it MULTIPLIES the tree. A single per-cell ceiling
    // could be met by both forms if the chain were short enough; the shape of the growth cannot.
    let gates_of = |k: usize| rows[k - 1].3;
    let baseline = gates_of(STEADY_STATE_BLOCK + 1) - gates_of(STEADY_STATE_BLOCK);
    assert!(
        baseline > 0,
        "a block must cost SOMETHING, or this measures nothing{table}"
    );
    for k in STEADY_STATE_BLOCK + 1..=6 {
        let step = gates_of(k) - gates_of(k - 1);
        let drift = step.abs_diff(baseline) * 100 / baseline;
        assert!(
            drift <= INCREMENT_DRIFT_PCT,
            "block {k} added {step} gates where block {} added {baseline} — a {drift}% drift is \
             not a constant cost per block{table}",
            STEADY_STATE_BLOCK + 1
        );
    }
    for w in rows.windows(2) {
        let (ka, _, ta, _) = w[0];
        let (kb, _, tb, _) = w[1];
        assert!(
            tb >= ta.saturating_mul(TREE_GROWTH_PER_BLOCK),
            "the tree form must be measured growing super-linearly from {ka} blocks to {kb} \
             ({ta} → {tb}) — otherwise this lock is comparing two linear things{table}"
        );
    }
    let (_, cells6, tree6, gates6) = *rows.last().expect("six rows");
    assert!(
        tree6 > u128::from(gates6) * 10,
        "the six-block body is the whole point: {tree6} tree nodes vs {gates6} gates{table}"
    );
    println!("  {cells6} cells: {tree6} tree nodes → {gates6} shared gates{table}");
}

// ───────────────────────────── C ─────────────────────────────

#[test]
fn a_wrong_datapath_still_earns_no_certificate() {
    // ── the tree path: the MAC PE the tensor stages certify ──
    let pe = |op: AluOp| {
        vec![
            FabricOp {
                op: AluOp::Mul,
                a: Src::In(0),
                b: Src::In(2),
            },
            FabricOp {
                op,
                a: Src::In(1),
                b: Src::Val(0),
            },
        ]
    };
    let cells: Vec<LoopCell> = pe(AluOp::Add)
        .iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect();
    let g = route_loop_body(&cells, 3, &[Src::Val(1)]);
    let placed =
        grid_output_to_gates(&g, 0, WIDTH, &|k| format!("in{k}")).expect("the PE grid blasts");

    let right = fabric_to_bounded(&pe(AluOp::Add), 1, WIDTH);
    let wrong = fabric_to_bounded(&pe(AluOp::Sub), 1, WIDTH);
    let placed_tree = logicaffeine_compile::codegen_sva::fabric_grid::grid_output_to_bounded(
        &g,
        0,
        WIDTH,
        &|k| format!("in{k}"),
    );
    assert!(
        certify_mapping(placed_tree.clone(), right).is_some(),
        "the PE's own datapath must certify, or the negative below proves nothing"
    );
    assert!(
        certify_mapping(placed_tree, wrong).is_none(),
        "`psum − a·w` is not what the grid computes — a wrong datapath earns NO certificate"
    );

    // ── the gate path: the same fail-closed property, one level lower ──
    let right_gates = cells_to_gates(&cells, Src::Val(1), WIDTH).expect("the PE body blasts");
    assert!(
        certify_gates(&placed, &right_gates).is_some(),
        "the PE's own datapath must certify on the gate path too"
    );
    let wrong_cells: Vec<LoopCell> = pe(AluOp::Sub)
        .iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect();
    let wrong_gates =
        cells_to_gates(&wrong_cells, Src::Val(1), WIDTH).expect("the mutated body blasts");
    assert!(
        certify_gates(&placed, &wrong_gates).is_none(),
        "sharing must not make the miter vacuous: `psum − a·w` must still be refused"
    );

    // ── and on a REAL body, one operand at a time ──
    let body = scalar_body(&lower(TWO_INPUT, "mix"));
    let (placed, spec) = gate_obligation(&body);
    assert!(
        certify_gates(&placed, &spec).is_some(),
        "`mix` certifies against its own datapath"
    );
    let mut mutated = 0usize;
    for i in 0..body.cells.len() {
        let Some(cells) = flip_cell(&body.cells, i) else {
            continue;
        };
        let Some(wrong) = cells_to_gates(&cells, body.result, WIDTH) else {
            continue;
        };
        mutated += 1;
        assert!(
            certify_gates(&placed, &wrong).is_none(),
            "`mix` with cell {i}'s operation flipped is a DIFFERENT function — it must earn no \
             certificate against the grid that places the original"
        );
    }
    assert!(
        mutated >= 2,
        "the mutation sweep must actually mutate something (got {mutated})"
    );
    println!("fail-closed on the tree path, the gate path, and {mutated} single-cell mutations");
}

/// `cells` with cell `i`'s arithmetic flipped to a different operation (and the comparison of a
/// select negated) — a program that computes something else, however slightly.
fn flip_cell(cells: &[LoopCell], i: usize) -> Option<Vec<LoopCell>> {
    use logicaffeine_compile::codegen_sva::fabric_rtl::CmpKind;
    let mut out = cells.to_vec();
    out[i] = match cells[i] {
        LoopCell::Arith { op, a, b } => {
            let op = match op {
                AluOp::Add => AluOp::Sub,
                AluOp::Sub => AluOp::Add,
                AluOp::Mul => AluOp::Add,
                AluOp::And => AluOp::Or,
                AluOp::Or => AluOp::And,
                AluOp::Xor => AluOp::And,
                AluOp::Shl => AluOp::Shr,
                AluOp::Shr => AluOp::Shl,
                AluOp::UShr => AluOp::Shl,
            };
            LoopCell::Arith { op, a, b }
        }
        LoopCell::Select { cmp, ca, cb, t, e } => {
            let cmp = match cmp {
                CmpKind::Lt => CmpKind::Ge,
                CmpKind::Ge => CmpKind::Lt,
                CmpKind::Gt => CmpKind::Le,
                CmpKind::Le => CmpKind::Gt,
                CmpKind::Eq => CmpKind::Ne,
                CmpKind::Ne => CmpKind::Eq,
            };
            LoopCell::Select { cmp, ca, cb, t, e }
        }
        _ => return None,
    };
    Some(out)
}

// ───────────────────────────── D ─────────────────────────────

#[test]
fn a_computed_predicate_routes_and_certifies() {
    // `clamp`'s second comparison reads a SELECT's output. Its comparison operand and its else arm
    // are the same producer, so the mux needs ONE routed channel and the tile has two.
    let body = scalar_body(&lower(IF_ELSE, "clamp"));
    let computed_predicate_cells: Vec<usize> = body
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            matches!(c, LoopCell::Select { ca, cb, .. }
                if matches!(ca, Src::Val(_)) || matches!(cb, Src::Val(_)))
        })
        .map(|(i, _)| i)
        .collect();
    assert!(
        !computed_predicate_cells.is_empty(),
        "clamp's body must actually contain a mux that compares a computed value, or this test \
         is about nothing: {:?}",
        body.cells
    );
    for i in &computed_predicate_cells {
        assert!(
            tile_channel_demand(&body.cells[*i]) <= TILE_OPERAND_CHANNELS,
            "clamp's cell {i} must fit the interconnect"
        );
    }
    assert!(
        unroutable_cell(&body.cells).is_none(),
        "clamp's body routes"
    );

    for chip in RduChip::ALL {
        let p = lower_whole_program_lane(IF_ELSE, chip)
            .unwrap_or_else(|e| panic!("clamp on {}: {e:?}", chip.as_str()));
        assert_eq!(p.phase(), Phase::Verified);
        let c = match scalar_cert(&p) {
            ComputeStageCert::Sat(c) => c,
            ComputeStageCert::Uncertified { reason } => {
                panic!(
                    "clamp's routed body must certify on {} — got `{reason}`",
                    chip.as_str()
                )
            }
        };
        assert!(
            c.recheck(),
            "clamp's certificate must be accepted by the independent checker"
        );
        assert!(
            !c.with_last_step_removed().recheck(),
            "clamp's final refutation step must be load-bearing, not decoration"
        );

        // A certificate about the wrong program is worth nothing, so the lowering is anchored to
        // the VM at every lane as well.
        let xs: Vec<i64> = (0..64).map(|i| (i * 131 - 2000) % 4001).collect();
        let got = lanes(&p, &xs);
        for (i, x) in xs.iter().enumerate() {
            assert_eq!(
                got[i],
                vm_f(IF_ELSE, "clamp", &[*x]),
                "clamp lane {i} (x = {x}) != the VM"
            );
        }
        println!(
            "clamp on {}: routed computed predicate, {} steps over {} vars, rechecked, 64 lanes ≡ \
             VM; last step load-bearing: {}",
            chip.as_str(),
            c.len(),
            c.num_vars(),
            !c.with_last_step_removed().recheck()
        );
    }

    // And the shape that is still genuinely unroutable is refused BY NAME — a computed predicate
    // over two MORE computed values wants a third channel the tile does not have.
    let pick = scalar_body(&lower(COMPUTED_PREDICATE, "pick"));
    let (cell, demand) = unroutable_cell(&pick.cells)
        .expect("pick's mux compares a computed value AND selects between two more");
    assert!(
        demand > TILE_OPERAND_CHANNELS,
        "cell {cell} demands {demand} channels"
    );
    let reason = match scalar_cert(&lower(COMPUTED_PREDICATE, "pick")) {
        ComputeStageCert::Uncertified { reason } => reason.clone(),
        ComputeStageCert::Sat(_) => panic!("pick's mux has no placed form and must not certify"),
    };
    assert!(
        reason.contains("computed value"),
        "the refusal must name WHAT cannot be routed: {reason}"
    );
    assert!(
        reason.contains(&TILE_OPERAND_CHANNELS.to_string()) && reason.contains("channel"),
        "the refusal must name the LIMIT it hit, so a reader can tell a wall from a bug: {reason}"
    );
    assert!(
        reason.contains(&demand.to_string()),
        "the refusal must name what the cell demanded ({demand}): {reason}"
    );
    println!("pick: refused by name — {reason}");
}

// ───────────────────────────── E ─────────────────────────────

/// The production ceiling must clear the widest body the product lowers by at least this much, or
/// it is a tripwire across the path rather than a safety net under it.
const REQUIRED_HEADROOM: u64 = 8;

#[test]
fn the_budget_still_catches_a_genuine_runaway() {
    // ── the ceiling is live code: the SAME body that certifies under the production ceiling comes
    //    back as the typed `Uncertified { reason }` under a tiny one, naming both numbers ──
    let body = scalar_body(&lower(&fib_source(6), "fib x6"));
    let (placed, spec) = gate_obligation(&body);
    let miter = words_equal(&placed, &spec).expect("one width");
    let real = logicaffeine_compile::codegen_sva::bitblast::pred_size(&miter).dag_nodes;

    assert!(
        matches!(
            scalar_certificate_within(&body, GATE_BUDGET_NODES),
            ComputeStageCert::Sat(_)
        ),
        "fib's body certifies under the production ceiling, or the negative below proves nothing"
    );
    let tiny = real / 2;
    let reason = match scalar_certificate_within(&body, tiny) {
        ComputeStageCert::Uncertified { reason } => reason,
        ComputeStageCert::Sat(c) => panic!(
            "a ceiling of {tiny} under a {real}-gate obligation certified anyway ({} vars) — the \
             guard is dead code",
            c.num_vars()
        ),
    };
    assert!(
        reason.contains("budget"),
        "the refusal must NAME the budget: {reason}"
    );
    assert!(
        reason.contains(&tiny.to_string()),
        "the reason prints the ceiling: {reason}"
    );
    assert!(
        reason.contains("gates"),
        "the reason prints the unit it counted: {reason}"
    );

    // ── and the guard is STAGED: the cheap side (the body's own datapath) is priced first, so a
    //    body past the ceiling is refused WITHOUT ever allocating the (2m+2)×m tile array that
    //    would place it. Measured by the clock: this refusal costs one pass over the cells ──
    let spec_gates = word_size(&spec).dag_nodes;
    let t_early = Instant::now();
    let early = scalar_certificate_within(&body, spec_gates / 2);
    let early_secs = t_early.elapsed();
    match early {
        ComputeStageCert::Uncertified { reason } => {
            assert!(reason.contains("budget") && reason.contains(&(spec_gates / 2).to_string()));
            assert!(
                early_secs < Duration::from_secs(2),
                "the early gate is priced from the cells alone and must not build the grid — took \
                 {early_secs:?}"
            );
        }
        ComputeStageCert::Sat(_) => {
            panic!(
                "a ceiling of {} under a {spec_gates}-gate body certified anyway",
                spec_gates / 2
            )
        }
    }

    // ── and the same guard, one layer down, on the obligation itself ──
    match certify_gates_within(&placed, &spec, tiny) {
        Err(CertifyRefusal::Blast(BlastRefusal::GateBudget { dag_nodes, budget })) => {
            assert_eq!(
                budget, tiny,
                "the refusal reports the ceiling it was judged against"
            );
            assert_eq!(
                dag_nodes, real,
                "the price is the same whichever ceiling judges it"
            );
        }
        Err(other) => panic!(
            "a ceiling of {tiny} must refuse for SIZE, got `{}`",
            other.reason()
        ),
        Ok(c) => panic!("the gate ceiling is dead code ({} vars)", c.num_vars()),
    }

    // ── a datapath that genuinely exceeds the PRODUCTION ceiling is refused by name, not by the
    //    allocator: a chain of 64-bit multiplies, which is what a runaway actually looks like once
    //    the encoding is linear in the circuit ──
    let t0 = Instant::now();
    let big = wide_multiply_chain(64, 96);
    let big_gates = word_size(&big).dag_nodes;
    assert!(
        big_gates > GATE_BUDGET_NODES,
        "the runaway must actually exceed the production ceiling ({big_gates} vs \
         {GATE_BUDGET_NODES}) — raise the chain length if the circuits got cheaper"
    );
    // Mitered against ITSELF: the point is that a datapath this large is refused for its SIZE,
    // whatever it is compared with, and building a second one only pays to prove the same thing.
    match certify_gates_within(&big, &big, GATE_BUDGET_NODES) {
        Err(CertifyRefusal::Blast(BlastRefusal::GateBudget { dag_nodes, budget })) => {
            assert_eq!(budget, GATE_BUDGET_NODES);
            assert!(
                dag_nodes > GATE_BUDGET_NODES,
                "{dag_nodes} must be over {budget}"
            );
            let r = CertifyRefusal::Blast(BlastRefusal::GateBudget { dag_nodes, budget }).reason();
            assert!(
                r.contains("budget") && r.contains(&budget.to_string()),
                "{r}"
            );
            println!(
                "the production ceiling refused a {dag_nodes}-gate datapath in {:?}: {r}",
                t0.elapsed()
            );
        }
        Err(other) => panic!(
            "the production ceiling must refuse for SIZE, got `{}`",
            other.reason()
        ),
        Ok(_) => {
            panic!("a {big_gates}-gate datapath certified under a {GATE_BUDGET_NODES} ceiling")
        }
    }

    // ── and the ceiling is a net, not a tripwire: it clears the widest body the product lowers ──
    assert!(
        real.saturating_mul(REQUIRED_HEADROOM) <= GATE_BUDGET_NODES,
        "the production ceiling ({GATE_BUDGET_NODES}) must clear the widest body the demo corpus \
         lowers ({real} gates) by {REQUIRED_HEADROOM}×"
    );
}

/// A chain of `n` multiplies at `width` bits — a datapath whose circuit is genuinely enormous even
/// when nothing is unfolded, so the gate ceiling has something real to catch.
fn wide_multiply_chain(width: u32, n: usize) -> Word {
    let mut acc = word_var("x", width);
    let k = word_var("y", width);
    for _ in 0..n {
        acc = word_binary(BitVecBoundedOp::Mul, &acc, &k).expect("one width");
    }
    acc
}
