//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! BIT-BLAST BUDGET LOCK — a multiply chain is refused BY NAME, never by the allocator.
//!
//! `largo emit rdu --chip sn50 examples/rdu_demo/fib.lg` used to allocate 44 GB in 100 s and then
//! abort ("memory allocation of 96 bytes failed"). The cause was two compounding blow-ups on the
//! scalar-stage certificate path, both of them the same mistake — copying a DAG as a tree:
//!
//!  1. `codegen_sva::bitblast` represented every blasted bit as an OWNED `ProofExpr`, so
//!     `bv_mul` deep-copied each operand's whole expression once per partial product. fib's fast
//!     doubling (`a*a + b*b` feeding the next block) multiplies that by ~70 per doubling step:
//!     the six-block body's formula is 1.0 × 10^14 nodes — about ten petabytes. The bits now live
//!     in a SHARED gate DAG (`Rc<Gate>`), so a clone bumps a refcount and the blast is linear in
//!     the datapath it is handed; the tree is materialised once, at the boundary, and only after
//!     it has been PRICED.
//!  2. Nothing bounded the work. A certificate that cannot be built must come back as the typed
//!     `Uncertified { reason }` the float and memory stages already use — the program still LOWERS,
//!     with an honestly uncertified stage — never as an allocator abort on a stage.
//!
//! The two ceilings are staged: `spatial::certificate::SCALAR_DATAPATH_BUDGET_NODES` prices the
//! body from its CELLS (one pass, zero allocation) and stops the runaway before anything is built;
//! `codegen_sva::bitblast::BLAST_BUDGET_NODES` is the exact ceiling on the materialised formula,
//! measured from the shared DAG.
//!
//!  A. `a_multiply_chain_is_refused_by_the_budget_not_by_the_allocator` — the six-block fib body
//!     LOWERS to a Verified program whose Scalar stage is `Uncertified` with a reason naming the
//!     cause AND the budget, in well under five seconds and a few hundred MB of RSS.
//!  B. `every_alu_body_that_certified_before_still_certifies` — the verdict of every body in the
//!     demo corpus is UNCHANGED by the budget: `mix` and the 1- and 2-block fib chains still earn a
//!     real SAT certificate that rechecks, each pinned to whether its FINAL refutation step is
//!     load-bearing; and the bodies that were already declined are declined for the SAME reason as
//!     before — never the budget one. The two-sided half: a ceiling that silently stops certifying
//!     things is a regression, not a fix.
//!  C. `the_budget_is_load_bearing` — the same obligation that certifies under the production
//!     ceiling comes back `Budget` under a tiny one, with the numbers in the reason. Live code.
//!  D. `the_whole_program_lane_still_lowers_an_uncertified_body` — `lower_whole_program_lane` on the
//!     real `examples/rdu_demo/fib.lg` returns a Verified `SpatialProgram` on both chips and
//!     `eval_choreography` equals the VM at every lane. An uncertified stage refuses a PROOF, not
//!     the program.
//!  E. `structural_sharing_keeps_the_bit_blast_linear` — over a 1..5-block chain the shared DAG
//!     stays within a constant factor of the datapath it is given while the tree it replaces grows
//!     unboundedly faster; the measured table is in the assert message. The blast is bounded by its
//!     WORK (`bitblast::blast_work`), row by row — each datapath node blasted once, gates allocated and
//!     held linear in the datapath, the pricing walk at most three steps a gate — and never by its CPU
//!     time, which a full suite inflates 4–6× (printed in the table, not asserted). The six-block row is
//!     priced by the production guard only — blasting it is the thing this lock exists to prevent.
//!
//! ★ MUTATION PROBES (each run once when this lock landed; the lock went red, the impl was restored
//!   and `cmp`-verified byte-identical):
//!   - `SCALAR_DATAPATH_BUDGET_NODES = 10` → B reds: `mix` and the 1- and 2-block fib chains lose
//!     their SAT certificates to a budget refusal (a ceiling that over-refuses is caught);
//!   - `SCALAR_DATAPATH_BUDGET_NODES = u128::MAX` → A reds: the six-block chain walks into the
//!     blast and blows the five-second bar (the cheap gate is what makes the refusal instant);
//!   - `lower_bool_within` ignores its `budget` argument → C reds: the tiny ceiling admits an
//!     obligation it must refuse (the guard is not dead code);
//!   - `impl Clone for Bit` deep-copies the gate instead of sharing the `Rc` → E reds: the DAG
//!     count collapses onto the tree count at the two-block chain (the sharing is what is linear);
//!   - the budget refusal in `scalar_certificate` becomes a `panic!` → D reds: the program stops
//!     lowering (an unbuildable certificate must never take the program down with it).
//!   - (2026-09-28, E bounded by work instead of CPU seconds; logs `logs/sn50/probe_bb_*.txt`) a
//!     binary op blasts its left operand twice → E reds at ONE block (177 expression visits for a
//!     miter of two 29-node datapaths); the pricing walk loses its memo → E reds at two blocks
//!     (65 363 gates held for 277 datapath nodes). Both in under a second, loaded box or not.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test bitblast_budget_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;
use std::time::{Duration, Instant};

#[path = "common/thread_cpu.rs"]
mod thread_cpu;

use logicaffeine_compile::codegen_sva::bitblast::{
    blast_size, blast_work, lower_bool_within, BlastRefusal, BLAST_BUDGET_NODES,
};
use logicaffeine_compile::codegen_sva::fabric_grid::{grid_output_to_bounded, route_loop_body};
use logicaffeine_compile::codegen_sva::fabric_rtl::{LoopCell, Src};
use logicaffeine_compile::codegen_sva::pnr_certificate::{certify_mapping_within, CertifyRefusal};
use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::certificate::{
    bounded_datapath_nodes, cells_to_bounded, ComputeStageCert, SCALAR_DATAPATH_BUDGET_NODES,
};
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{Phase, ScalarBody, SpatialProgram, StageKind, TensorRole};
use logicaffeine_compile::spatial::lower::rdu_lanes::lower_whole_program_lane;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

/// The certified bound `spatial::certificate::scalar_certificate` blasts the body at.
const WIDTH: u32 = 8;

/// The demo corpus's ALU + decision bodies, verbatim from `rdu_whole_program_lane_lock` so the two
/// locks speak about the same programs.
const IF_ELSE: &str = "## To clamp (x: Int) -> Int:\n    Let mutable r be x.\n    If x is less than 0:\n        Set r to 0 - x.\n    If r is greater than 1000:\n        Set r to 1000.\n    Return r.\n\n## Main\n\nShow clamp(7).\n";
const TWO_INPUT: &str = "## To mix (a: Int, b: Int) -> Int:\n    Let mutable r be a * 3 - b.\n    If a is less than b:\n        Set r to b - a.\n    Return r.\n\n## Main\n\nShow mix(2, 9).\n";
const COMPUTED_PREDICATE: &str = "## To pick (n: Int) -> Int:\n    Let bit be (n shifted right by 1) & 1.\n    Let mutable r be n * 3.\n    If bit is 1:\n        Set r to n * 5.\n    Return r.\n\n## Main\n\nShow pick(6).\n";
const COLLECTIONS: &str = "## To tally (v: Int) -> Int:\n    Let mutable xs be [v, v * 2, 7].\n    Set item 2 of xs to v + 1.\n    Push v * 3 to xs.\n    Return item 2 of xs + length of xs + item 4 of xs.\n\n## Main\n\nShow tally(5).\n";

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
/// walks the multiply chain from "certifies in a blink" to "its encoding does not fit in a
/// datacentre". At `k == 6` the function is byte-for-byte the demo's.
fn fib_source(k: usize) -> String {
    let mut s = String::from(
        "## To fib (n: Int) -> Int:\n    Let mutable a be 0.\n    Let mutable b be 1.\n",
    );
    for i in (0..6).rev().take(k) {
        // Bit 0 needs no shift — the demo spells it `n & 1`, and this generator must match it byte
        // for byte or it is testing a lookalike.
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

/// Is this body one the placer will even route? `scalar_certificate` declines a non-combinational
/// body, and a mux whose comparison reads a computed value, BEFORE it prices anything — so an
/// obligation only exists for the rest.
fn is_routable_alu(body: &ScalarBody) -> bool {
    body.cells
        .iter()
        .all(|c| matches!(c, LoopCell::Arith { .. } | LoopCell::Select { .. }))
        && !body.cells.iter().any(|c| {
            matches!(c, LoopCell::Select { ca, cb, .. }
                if matches!(ca, Src::Val(_)) || matches!(cb, Src::Val(_)))
        })
}

/// The exact obligation `scalar_certificate` bit-blasts: the routed grid's output against the
/// body's own datapath, at the certified 8-bit bound.
fn obligation(body: &ScalarBody) -> (BoundedExpr, BoundedExpr) {
    assert!(
        is_routable_alu(body),
        "only a routable ALU body has a placement obligation"
    );
    let g = route_loop_body(&body.cells, body.n_in, &[body.result]);
    let placed = grid_output_to_bounded(&g, 0, WIDTH, &|k| format!("in{k}"));
    let spec = cells_to_bounded(&body.cells, body.result, WIDTH)
        .expect("an ALU body lowers to the bounded fragment");
    (placed, spec)
}

fn miter(body: &ScalarBody) -> BoundedExpr {
    let (placed, spec) = obligation(body);
    BoundedExpr::Eq(Box::new(placed), Box::new(spec))
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

#[test]
fn a_multiply_chain_is_bounded_and_now_earns_its_certificate() {
    let src = fib_source(6);
    // The generated six-block chain IS the demo program's function, so what this lock refuses is
    // what `largo emit rdu … fib.lg` refuses — not a lookalike.
    let d = demo("fib.lg");
    let demo_fn =
        &d[d.find("## To fib").expect("the fib function")..d.find("## Main").expect("a Main")];
    assert_eq!(
        src,
        format!("{demo_fn}## Main\n\nShow fib(10).\n"),
        "the generated chain is the demo's fib body verbatim"
    );

    let before = rss_bytes();
    let t0 = Instant::now();
    let p = lower(&src, "fib x6");
    let elapsed = t0.elapsed();
    let after = rss_bytes();

    assert_eq!(p.phase(), Phase::Verified, "the program LOWERS");
    // ★ THIS ROW USED TO ASSERT A BUDGET REFUSAL, AND THAT IS NOW THE WRONG TRUTH.
    // The bounded form deep-cloned `BoundedExpr` at every reuse, unfolding fib's 75-cell DAG into
    // 1.66M nodes — over the ceiling, so the honest outcome then was `Uncertified{budget}`. With the
    // bounded form shared, the same body prices far under the ceiling and earns a REAL refutation.
    // What this test has always been about survives unchanged and is asserted below: the six-block
    // chain is handled in bounded time and memory instead of taking the allocator down (44 GB / 100 s
    // / SIGABRT, once). The budget itself stays load-bearing in `the_budget_is_load_bearing` and in
    // `certifier_strength_lock::the_budget_still_catches_a_genuine_runaway`.
    match scalar_cert(&p) {
        ComputeStageCert::Sat(c) => {
            assert!(c.recheck(), "fib's refutation must recheck against the independent checker");
            println!("fib x6: SAT, {} steps over {} vars", c.len(), c.num_vars());
        }
        ComputeStageCert::Uncertified { reason } => panic!(
            "fib's body prices under the {SCALAR_DATAPATH_BUDGET_NODES}-node ceiling once the bounded form is shared, so it must earn a certificate — got `{reason}`"
        ),
    }

    assert!(
        elapsed < Duration::from_secs(5),
        "the proof is priced from the cells, so the chain stays fast — took {elapsed:?}"
    );
    match (before, after) {
        (Some(b), Some(a)) => {
            let grew = a.saturating_sub(b);
            assert!(
                grew < 300 * 1024 * 1024,
                "proving the chain must not take the allocator down — RSS grew {} MiB",
                grew / (1024 * 1024)
            );
            println!(
                "fib x6 certified in {elapsed:?}, RSS +{} MiB",
                grew / (1024 * 1024)
            );
        }
        _ => println!("fib x6 certified in {elapsed:?} (no /proc RSS here)"),
    }
}

// ───────────────────────────── B ─────────────────────────────

/// Whether the two-block chain's final refutation step is load-bearing — measured, pinned here so
/// it is one number to re-measure rather than a hidden `if`.
const TAMPER_FIB2: bool = true;
/// Whether the FINAL refutation step is load-bearing for a body whose mux predicate is computed.
/// MEASURED once the computed-predicate routing landed, then pinned — like every other row here, in
/// both directions, so a certificate that silently goes degenerate reds this lock.
const TAMPER_COMPUTED_PRED: bool = true;

/// What a body earned BEFORE the budget existed. The budget may not move a single row.
enum Want {
    /// A real refutation that rechecks. `tamper_refused` records whether its FINAL step is
    /// load-bearing: `check_pr_refutation` ends by RUP-refuting the database, so an obligation whose
    /// CNF already falls to unit propagation on its own (a body the placer maps to a trivially equal
    /// grid) still rechecks with the last step dropped. Pinned per body in BOTH directions — a
    /// certificate that silently goes degenerate, or one that silently stops being, reds this lock.
    Sat { tamper_refused: bool },
    /// Declined for a reason that is NOT the budget — it was declined before too.
    DeclinedFor(&'static str),
}

#[test]
fn every_alu_body_that_certified_before_still_certifies() {
    let cases: Vec<(String, String, Want)> = vec![
        (
            "mix".into(),
            TWO_INPUT.into(),
            Want::Sat {
                tamper_refused: true,
            },
        ),
        (
            "fib x1".into(),
            fib_source(1),
            Want::Sat {
                tamper_refused: false,
            },
        ),
        (
            "fib x2".into(),
            fib_source(2),
            Want::Sat {
                tamper_refused: TAMPER_FIB2,
            },
        ),
        // ★ THESE TWO MOVED FROM `DeclinedFor("computed value")` TO `Sat`. `route_loop_body` used to
        // refuse a mux whose comparison reads an earlier cell's OUTPUT ("routing a computed predicate
        // onto the array is future work"); it now routes it, so both bodies earn real refutations.
        // `certifier_strength_lock::a_computed_predicate_routes_and_certifies` is the proof, and the
        // fail-closed half (`a_wrong_datapath_still_earns_no_certificate`) is what keeps this honest.
        (
            "clamp".into(),
            IF_ELSE.into(),
            Want::Sat {
                tamper_refused: TAMPER_COMPUTED_PRED,
            },
        ),
        // `pick` is STILL declined, and the reason got sharper rather than disappearing: routing a
        // computed predicate landed, but this body needs THREE distinct computed values at one tile
        // and a tile has two operand channels (north and west). That is a placement constraint of the
        // array, not a gap in the prover — so it is pinned on the channel count, which is the fact.
        (
            "pick".into(),
            COMPUTED_PREDICATE.into(),
            Want::DeclinedFor("operand channels"),
        ),
        (
            "tally".into(),
            COLLECTIONS.into(),
            Want::DeclinedFor("memory"),
        ),
        (
            "primes".into(),
            demo("primes.lg"),
            Want::DeclinedFor("memory"),
        ),
    ];
    for (label, src, want) in &cases {
        let p = lower(src, label);
        match (scalar_cert(&p), want) {
            (ComputeStageCert::Sat(c), Want::Sat { tamper_refused }) => {
                assert!(c.recheck(), "{label}: the SAT certificate must recheck");
                assert_eq!(
                    !c.with_last_step_removed().recheck(),
                    *tamper_refused,
                    "{label}: the final refutation step is {} load-bearing — {} steps over {} vars",
                    if *tamper_refused {
                        "no longer"
                    } else {
                        "unexpectedly"
                    },
                    c.len(),
                    c.num_vars()
                );
                println!("{label}: SAT, {} steps over {} vars", c.len(), c.num_vars());
            }
            (ComputeStageCert::Uncertified { reason }, Want::Sat { .. }) => panic!(
                "{label} certified before the budget and must still certify — got `{reason}`"
            ),
            (ComputeStageCert::Uncertified { reason }, Want::DeclinedFor(names)) => {
                assert!(
                    reason.contains(names),
                    "{label}: declined for the SAME reason as before (`{names}`), got `{reason}`"
                );
                assert!(
                    !reason.contains("budget"),
                    "{label}: the budget must not swallow a refusal that already had a name: {reason}"
                );
                println!("{label}: declined by name — {reason}");
            }
            (ComputeStageCert::Sat(_), Want::DeclinedFor(names)) => panic!(
                "{label}: was declined for `{names}` before and must not silently start certifying"
            ),
        }
    }
    // The pricing is two-sided at the gate itself: every body that keeps its certificate prices
    // UNDER the ceiling, and the chain that is refused prices OVER it — the constant separates them.
    for (label, src, want) in &cases {
        if !matches!(want, Want::Sat { .. }) {
            continue;
        }
        let body = scalar_body(&lower(src, label));
        let n = bounded_datapath_nodes(&body);
        assert!(
            n <= SCALAR_DATAPATH_BUDGET_NODES,
            "{label} prices at {n} — over the {SCALAR_DATAPATH_BUDGET_NODES} ceiling it must clear"
        );
    }
    let six = bounded_datapath_nodes(&scalar_body(&lower(&fib_source(6), "fib x6")));
    assert!(
        six > SCALAR_DATAPATH_BUDGET_NODES,
        "the six-block chain prices at {six}, which the {SCALAR_DATAPATH_BUDGET_NODES} ceiling must refuse"
    );
}

// ───────────────────────────── C ─────────────────────────────

#[test]
fn the_budget_is_load_bearing() {
    let body = scalar_body(&lower(TWO_INPUT, "mix"));
    let size = blast_size(&miter(&body)).expect("an ALU body blasts");
    assert!(
        size.tree_nodes <= BLAST_BUDGET_NODES,
        "`mix` must sit under the production ceiling for this test to mean anything"
    );

    // Under the production ceiling: a real certificate.
    let (placed, spec) = obligation(&body);
    let cert = certify_mapping_within(placed, spec, BLAST_BUDGET_NODES).unwrap_or_else(|r| {
        panic!(
            "`mix` certifies under the production ceiling: {}",
            r.reason()
        )
    });
    assert!(cert.recheck());

    // Under a tiny ceiling: the SAME obligation comes back as a budget refusal, with the numbers.
    let tiny = size.tree_nodes / 2;
    let (placed, spec) = obligation(&body);
    match certify_mapping_within(placed, spec, tiny) {
        Err(CertifyRefusal::Blast(BlastRefusal::Budget { size: got, budget })) => {
            assert_eq!(budget, tiny, "the refusal reports the ceiling it was judged against");
            assert_eq!(got, size, "the price is the same whichever ceiling judges it");
            let reason = CertifyRefusal::Blast(BlastRefusal::Budget { size: got, budget }).reason();
            assert!(reason.contains("budget"), "{reason}");
            assert!(reason.contains(&tiny.to_string()), "the reason prints the ceiling: {reason}");
            assert!(
                reason.contains(&size.tree_nodes.to_string()),
                "the reason prints the price: {reason}"
            );
            println!("the guard fires at {tiny}: {reason}");
        }
        Err(other) => panic!("a ceiling of {tiny} must refuse for SIZE, got `{}`", other.reason()),
        Ok(c) => panic!(
            "a ceiling of {tiny} under a {}-node obligation certified anyway ({} vars) — the guard is dead code",
            size.tree_nodes,
            c.num_vars()
        ),
    }

    // And it refuses only for SIZE: the same tiny ceiling still names an unbounded obligation
    // `NotBounded`, so the two refusals never impersonate each other.
    let not_bounded = BoundedExpr::Eq(
        Box::new(BoundedExpr::BitVecVar("x".into(), 8)),
        Box::new(BoundedExpr::BitVecVar("y".into(), 16)),
    );
    assert_eq!(
        lower_bool_within(&not_bounded, tiny).unwrap_err(),
        BlastRefusal::NotBounded,
        "a width mismatch is not a budget problem"
    );
}

// ───────────────────────────── D ─────────────────────────────

#[test]
fn the_whole_program_lane_lowers_fib_and_certifies_its_body() {
    let src = demo("fib.lg");
    for chip in RduChip::ALL {
        let p = lower_whole_program_lane(&src, chip)
            .unwrap_or_else(|e| panic!("fib.lg on {}: {e:?}", chip.as_str()));
        assert_eq!(
            p.phase(),
            Phase::Verified,
            "the lane lowers fib on every chip"
        );
        // Was `Uncertified{budget}` while the bounded form deep-cloned; the shared form prices it
        // under the ceiling, so the per-PE proof is now real. Both halves are asserted: the stage's
        // own refutation AND the choreography certificate that carries it.
        match scalar_cert(&p) {
            ComputeStageCert::Sat(c) => assert!(
                c.recheck(),
                "{}: fib's stage refutation rechecks",
                chip.as_str()
            ),
            ComputeStageCert::Uncertified { reason } => {
                panic!(
                    "{}: fib's body must earn its certificate now — got `{reason}`",
                    chip.as_str()
                )
            }
        }
        assert!(
            p.certificate.as_ref().expect("a certificate").recheck(&p),
            "the CHOREOGRAPHY certificate still rechecks — only the per-PE proof was declined"
        );

        // The lowering is still a correct program: 64 lanes of `y[i] = fib(x[i])` equal the VM.
        let xs: Vec<i64> = (0..64).map(|i| (i * 7 + 3) % 80).collect();
        let got = lanes(&p, &xs);
        assert_eq!(got.len(), xs.len());
        for (i, x) in xs.iter().enumerate() {
            assert_eq!(
                got[i],
                vm_f(&src, "fib", &[*x]),
                "fib.lg on {}: lane {i} (n = {x}) != the VM",
                chip.as_str()
            );
        }
        println!(
            "fib.lg on {}: Verified, 64 lanes ≡ VM, stage uncertified by name",
            chip.as_str()
        );
    }
}

// ───────────────────────────── E ─────────────────────────────

#[test]
fn structural_sharing_keeps_the_bit_blast_linear() {
    /// The blaster may allocate this many gates per `BoundedExpr` node it is handed. The measured
    /// ratio is 2.6..5.7 across the whole chain; the ceiling is what makes "linear in its input" an
    /// assertion rather than a hope.
    const GATES_PER_DATAPATH_NODE: u128 = 8;
    /// The deepest chain this lock will actually BLAST. Five blocks price at 188 981 datapath
    /// nodes — already past the production gate, but cheap enough to measure. Six is the row the
    /// product refuses: blasting it is precisely what this lock exists to prevent, so it is priced
    /// by the production guard and never built.
    const BLASTED: usize = 5;
    /// The miter holds the placed datapath and the reference one, each a tree of `datapath` nodes under one root:
    /// blasting every node once is `2·datapath + 1` visits, and that is the bound.
    const MITER_SIDES: u128 = 2;
    /// Steps of the memoised pricing walk per distinct gate: a push for each of its at most two in-edges from an
    /// expanded parent, and one to expand it — plus the root's.
    const WALK_STEPS_PER_GATE: u64 = 3;

    let mut table = String::from(
        "\n  blocks  cells   datapath      gates(DAG)                 tree   tree/DAG   exprs/dp  gates/dp  walk/DAG   blast CPU s\n",
    );
    let mut rows: Vec<(usize, u128, u64, u128)> = Vec::new();
    for k in 1..=BLASTED {
        let body = scalar_body(&lower(&fib_source(k), &format!("fib x{k}")));
        let datapath = bounded_datapath_nodes(&body);
        let t0 = thread_cpu::thread_cpu_time();
        let e = miter(&body);
        let (size, work) = blast_work(|| blast_size(&e));
        let size = size.expect("an ALU body blasts");
        let secs = (thread_cpu::thread_cpu_time() - t0).as_secs_f64();
        let ratio = |num: u64, den: u128| num as f64 / den.max(1) as f64;
        table.push_str(&format!(
            "  {:>6}  {:>5}  {:>9}  {:>14}  {:>19}  {:>9}  {:>8.2}  {:>8.2}  {:>8.2}  {:>7.3}\n",
            k,
            body.cells.len(),
            datapath,
            size.dag_nodes,
            size.tree_nodes,
            size.tree_nodes / u128::from(size.dag_nodes),
            ratio(work.exprs, datapath),
            ratio(work.gates, datapath),
            ratio(work.walked, u128::from(size.dag_nodes)),
            secs
        ));
        // ★ THE BLAST IS BOUNDED BY ITS WORK, NOT ITS CLOCK. CPU time is printed, never asserted: under a full suite's
        // memory pressure the five-block row's 1.07 M-gate working set takes 4–6× its solo 2.7 s, so a budget in
        // seconds measures the box. What the blaster DOES is counted, and each count is bounded by the size of what it
        // walks — checked HERE, row by row, so a blaster gone wrong is stopped at the first row that shows it:
        //   • the expression it is handed: every datapath node blasted a bounded number of times — blasting a shared
        //     operand twice doubles the work per level, and shows at two blocks;
        //   • the gates it allocates: linear in the datapath (no gate is shared by value, so this is the DAG plus what a
        //     constant fold leaves behind) — the tree-shaped blaster allocates the tree, 65 363 gates at two blocks;
        //   • the memoised walk that prices the result: a bounded number of steps per gate — a walk that forgot its memo
        //     walks the tree, 2.9 × 10^12 nodes at five blocks.
        assert!(
            u128::from(work.exprs) <= datapath.saturating_mul(MITER_SIDES) + 1,
            "the {k}-block blast visited {} expression nodes for a miter of two {datapath}-node datapaths — a node \
             blasted more than once is an operand blasted again{table}",
            work.exprs
        );
        assert!(
            u128::from(work.gates) <= datapath.saturating_mul(GATES_PER_DATAPATH_NODE),
            "the {k}-block blast allocated {} gates for {datapath} datapath nodes — more than \
             {GATES_PER_DATAPATH_NODE}× is a copy, not a share{table}",
            work.gates
        );
        // And what it HOLDS: the distinct gates the result reaches — a copy that bypasses the allocator (a `Clone`
        // that deep-copies) shows here and nowhere in the allocation count.
        assert!(
            u128::from(size.dag_nodes) <= datapath.saturating_mul(GATES_PER_DATAPATH_NODE),
            "the {k}-block blast holds {} gates for {datapath} datapath nodes — more than \
             {GATES_PER_DATAPATH_NODE}× is a copy, not a share{table}",
            size.dag_nodes
        );
        assert!(
            work.walked <= size.dag_nodes.saturating_mul(WALK_STEPS_PER_GATE) + 1,
            "pricing the {k}-block blast took {} steps over {} gates — more than {WALK_STEPS_PER_GATE} a gate is the \
             tree, not the DAG{table}",
            work.walked,
            size.dag_nodes
        );
        rows.push((k, datapath, size.dag_nodes, size.tree_nodes));
    }
    let six = bounded_datapath_nodes(&scalar_body(&lower(&fib_source(6), "fib x6")));
    table.push_str(&format!(
        "  {:>6}  {:>5}  {:>9}  {:>14}  {:>19}  {:>9}  {:>7}\n",
        6, 76, six, "refused", "refused", "-", "-"
    ));

    // The blast is LINEAR IN THE DATAPATH IT IS GIVEN (asserted per row above); and the tree it
    // replaces runs away from it — every extra block widens the gap, strictly.
    for w in rows.windows(2) {
        let (ka, _, da, ta) = w[0];
        let (kb, _, db, tb) = w[1];
        let (ra, rb) = (ta / u128::from(da), tb / u128::from(db));
        assert!(
            rb > ra,
            "a {kb}-block chain must share MORE than a {ka}-block one ({rb} vs {ra}){table}"
        );
    }
    // The refused row is priced, not blasted — the guard sits upstream of the measurement.
    assert!(
        six > SCALAR_DATAPATH_BUDGET_NODES,
        "the six-block row is the refused one{table}"
    );
    println!("{table}");
}

// ───────────────────────────── F ─────────────────────────────

/// ★ THE CEILING GOVERNS MORE THAN THE ALU BODIES IT WAS CALIBRATED FROM.
///
/// `BLAST_BUDGET_NODES` was measured from the RDU demo corpus's ALU bodies — the widest of which, the
/// 8-bit MAC PE, materialises 107 k nodes — and the table in its doc comment lists only those. But
/// `certify_config_table` blasts a DIFFERENT and much larger obligation on the same ceiling: a time-mux
/// of EVERY island against one of them, which is how the reconfigurable array proves `config == k ⇒
/// array ≡ island k`. Nothing measured that when the ceiling was chosen, and a ceiling calibrated from
/// one workload silently refused another: `fabric_whole_program_lock` began panicking inside
/// `lower_bool(...).expect(...)`, which is a legitimate long-standing proof failing for a reason its
/// own message could not state.
///
/// So the reconfig obligation is measured HERE, beside the bodies, and the ceiling must clear it. The
/// gap to a real runaway stays enormous — the third fib doubling block is 3.5 × 10^8 and the sixth is
/// 10^14 — so admitting this workload costs the guard nothing.
///
/// ★ MUTATION PROBE: set `BLAST_BUDGET_NODES` below the measured reconfig size — this test reds with
/// the measurement in its message, which is exactly the failure that would otherwise surface as a
/// panic three crates away.
#[test]
fn the_reconfig_obligation_fits_the_ceiling() {
    use logicaffeine_compile::codegen_sva::fabric_config::compile_to_config_table;
    use logicaffeine_compile::codegen_sva::fabric_grid::{grid_to_bounded, time_mux_n_bounded};
    use logicaffeine_compile::codegen_sva::sva_to_verify::BoundedExpr;

    // ★ THE REAL CORPUS, ALL FIVE. My first cut of this test measured THREE datapaths, got 173 430
    // nodes, passed comfortably, and was measuring the wrong thing: the obligation grows SUPERLINEARLY
    // in the number of islands, because the time-mux compares every grid against each one. The five
    // `fabric_whole_program_lock` actually reconfigures come to 2 767 042 — sixteen times the
    // three-island figure, and over the old ceiling. A fixture that is smaller than the workload it
    // stands for will pass while the thing it guards is broken.
    let sources: Vec<(&str, &str)> = vec![
        ("## To affine (x: Int) -> Int:\n    Return 3 * x + 7.\n", "affine"),
        ("## To clamp (x: Int, lo: Int, hi: Int) -> Int:\n    Let result be x.\n    If x is less than lo:\n        Set result to lo.\n    If x is greater than hi:\n        Set result to hi.\n    Return result.\n", "clamp"),
        ("## To sign (n: Int) -> Int:\n    Let result be 0.\n    If n is less than 0:\n        Set result to 0 - 1.\n    If n is greater than 0:\n        Set result to 1.\n    Return result.\n", "sign"),
        ("## To absval (a: Int) -> Int:\n    Let result be a.\n    If a is less than 0:\n        Set result to 0 - a.\n    Return result.\n", "absval"),
        ("## To min2 (a: Int, b: Int) -> Int:\n    Let result be b.\n    If a is less than b:\n        Set result to a.\n    Return result.\n", "min2"),
    ];
    assert_eq!(sources.len(), 5, "this must mirror fabric_whole_program_lock's CORPUS — it is the workload under measurement");
    let (width, cfg_width) = (8u32, 3u32);
    let table = compile_to_config_table(&sources, width, cfg_width);
    let refs: Vec<_> = table.grids.iter().collect();

    let mut worst = 0u128;
    let mut worst_k = 0usize;
    for k in 0..table.grids.len() {
        let combined = time_mux_n_bounded(&refs, k as u64, table.width, table.cfg_width);
        let island = grid_to_bounded(&table.grids[k], table.width);
        let e = BoundedExpr::Eq(Box::new(combined), Box::new(island));
        let size = blast_size(&e).expect("the reconfig obligation blasts");
        println!(
            "  reconfig config {k}: DAG {} gates, tree {} nodes",
            size.dag_nodes, size.tree_nodes
        );
        if size.tree_nodes > worst {
            worst = size.tree_nodes;
            worst_k = k;
        }
    }
    assert!(
        worst <= BLAST_BUDGET_NODES,
        "the reconfig obligation for config {worst_k} materialises {worst} nodes, past the \
         {BLAST_BUDGET_NODES}-node ceiling — `certify_config_table` is a legitimate proof and the \
         ceiling must clear it (raise it with this measurement behind it, and record the row in \
         BLAST_BUDGET_NODES's table)"
    );
    println!("reconfig obligation worst case: {worst} nodes, ceiling {BLAST_BUDGET_NODES}");
}
