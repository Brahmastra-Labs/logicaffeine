//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! NO FUSION RULE IS TRUSTED WITHOUT A PROOF (benchmarks/FUSIONS_WE_NEED.md, OPT-5).
//!
//! `tensor_graph::smt` states what a graph computes as terms over arrays: the ops that move values are the index maps
//! they are, every other op a function left open, an in-place write a store into a range of rows or units. A rewrite's
//! obligation says the rewritten graph computes every tensor both keep as the reference does, for every input and every
//! meaning of the open ops; Z3 discharges it (`logicaffeine_tv::tensor`).
//!
//! Every step of the shared table — each rule, and each chip's own rule beside it — is proven on every witness
//! `fusion_class_lock` judges on numbers (every class's exemplar, every tiny architecture's cuts, every rule's witness
//! that its dataflow forbids it beside the twin where it fires) and on every lane cut under the lane's shim: the graph
//! the steps before it make, against the graph it makes of that. Each obligation is pinned in
//! `fixtures/tensor_rule_proofs.tsv` by its text's digest, so the default suite re-checks without a solver that every
//! rule's proofs are still the ones Z3 discharged; under the `verification` feature Z3 discharges them all again, and
//! with `LOGOS_UPDATE_FIXTURES=1` records them. The algebraic identities the chips' links compute fused classes by are
//! proven at every shape up to a bound, with products, sums and scales computed exactly.
//!
//! ★ MUTATION PROBE: let `slice_read` move a slice past a write to its source → its witness's obligation is refuted.
//! ★ MUTATION PROBE: read a join's second half at the first's offset in `smt` → every join-in-place proof is refuted.
//!
//!   cargo nextest run -p logicaffeine-tests --test tensor_rule_proof_lock
//!   Z3_SYS_Z3_HEADER=/usr/include/z3.h cargo nextest run -p logicaffeine-tests --features verification --test tensor_rule_proof_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

#[path = "common/fusion_corpus.rs"]
mod fusion_corpus;
#[path = "common/lane_corpus.rs"]
mod lane_corpus;
#[path = "common/lane_cuts.rs"]
mod lane_cuts;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
use logicaffeine_compile::tensor_graph::fuse::{optimize, read_in_place, steps};
use logicaffeine_compile::tensor_graph::front::TensorOpSpec;
use logicaffeine_compile::tensor_graph::graph::{Access, Dst, Elem, Op, Role, Storage, Strided, Sym, Tensor, TensorGraph};
use logicaffeine_compile::tensor_graph::shim::{ReferenceShim, Shim};
use logicaffeine_compile::tensor_graph::smt::{identity, obligation, Obligation, SmtError};
use logicaffeine_compile::tensor_graph::symbols::ClassId;

use fusion_corpus::{lifted, witnesses, HOLD_BACKS};
use lane_corpus::every_lane_cut;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tensor_rule_proofs.tsv")
}

/// The steps no compile holds: the views of a computed table's rows, the arg-max that advances the step, and the
/// numerics fold.
const UNHELD: &str = "(unheld)";

/// One proof: the witness, the step it proves (or `(unheld)`), and its obligation.
struct Proof {
    witness: String,
    step: String,
    obligation: Obligation,
}

impl Proof {
    fn row(&self) -> String {
        format!("{}\t{}\t{}\t{}\t{}", self.witness, self.step, self.obligation.compared, self.obligation.identical, self.obligation.digest())
    }
}

/// Every graph the steps are proven on, with the shim it is compiled under.
fn corpus() -> Vec<(String, TensorGraph, Box<dyn Shim + Send + Sync>)> {
    let mut out: Vec<(String, TensorGraph, Box<dyn Shim + Send + Sync>)> = Vec::new();
    for (what, g) in witnesses() {
        out.push((what, g, Box::new(ReferenceShim)));
    }
    for (rule, why, forbidding, twin) in HOLD_BACKS {
        for rows in [true, false] {
            out.push((format!("{rule}: {why} (rows {rows})"), lifted(forbidding, rows, None), Box::new(ReferenceShim)));
            out.push((format!("{rule}: twin of {why} (rows {rows})"), lifted(twin, rows, None), Box::new(ReferenceShim)));
        }
    }
    for c in every_lane_cut() {
        out.push((format!("lane {}", c.what), c.g, Box::new(c.shim)));
    }
    out
}

/// The proofs of one graph under its shim: the steps no compile holds, then each step that fires, each against the
/// graph the steps before it make. A step decides from the steps before it alone, so one that does not fire in the whole
/// compile fires in no prefix of it either: only the graphs around the steps that fire are made.
fn proofs_of(what: &str, g: &TensorGraph, shim: &dyn Shim) -> Vec<Proof> {
    let order = steps(shim);
    let whole = optimize(g, shim, &[]);
    let prove = |step: &str, reference: &TensorGraph, rewritten: &TensorGraph| Proof {
        witness: what.to_string(),
        step: step.to_string(),
        obligation: obligation(reference, rewritten).unwrap_or_else(|e| panic!("{what}: `{step}` gives no obligation: {e}")),
    };
    let mut before = optimize(g, shim, &order).graph;
    let mut out = vec![prove(UNHELD, g, &before)];
    for (k, name) in order.iter().enumerate() {
        if whole.fired(name) == 0 {
            continue;
        }
        let after = optimize(g, shim, &order[k + 1..]);
        assert!(after.fired(name) > 0, "{what}: `{name}` fires in the whole compile and not after the steps before it");
        out.push(prove(name, &before, &after.graph));
        before = after.graph;
    }
    out
}

/// What `f` makes of each proof of the corpus, in corpus order; each obligation is dropped once `f` has read it. The
/// graphs go to the threads one at a time, so a heavy one never holds back a thread's whole share.
fn each_proof<R: Send>(f: impl Fn(&Proof) -> R + Sync) -> Vec<R> {
    let corpus = corpus();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let (f, corpus, next) = (&f, &corpus, &next);
    let mut done: Vec<(usize, Vec<R>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(move || {
                    let mut mine = Vec::new();
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((what, g, shim)) = corpus.get(k) else { break mine };
                        mine.push((k, proofs_of(what, g, &**shim).iter().map(f).collect()));
                    }
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("a proof thread")).collect()
    });
    done.sort_by_key(|(k, _)| *k);
    done.into_iter().flat_map(|(_, r)| r).collect()
}

/// The command that proves every obligation with Z3 and records the proofs.
const RECORD: &str = "LOGOS_UPDATE_FIXTURES=1 Z3_SYS_Z3_HEADER=/usr/include/z3.h cargo nextest run -p logicaffeine-tests --features \
                      verification --test tensor_rule_proof_lock -E 'test(=proven::every_grouping_rule_is_proven_over_arrays)'";

fn recorded() -> Vec<String> {
    std::fs::read_to_string(fixture())
        .unwrap_or_else(|e| panic!("{}: {e} — prove and record them: `{RECORD}`", fixture().display()))
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(String::from)
        .collect()
}

/// Every step a compile can hold under the corpus's shims: the shared table's rules and the lane's own.
fn every_step() -> BTreeSet<String> {
    let lane = LaneShim { knobs: CudaKnobs::default(), batch: false, held: Vec::new() };
    steps(&ReferenceShim).into_iter().chain(steps(&lane)).map(String::from).collect()
}

/// The rows each step is proven by, from the recorded proofs.
fn by_step(rows: &[String]) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in rows {
        let f: Vec<&str> = r.split('\t').collect();
        assert_eq!(f.len(), 5, "a proof row is witness, step, compared, identical, digest: {r:?}");
        out.entry(f[1].to_string()).or_default().push(r.clone());
    }
    out
}

/// The recorded proofs are the obligations the rules make of the corpus today, byte for byte — re-checked with no
/// solver — and every rule of the table and of the lane has at least one, as do the steps no compile holds (the views,
/// the arg-max that advances the step, the numerics fold).
#[test]
fn every_rule_has_a_checked_in_witness() {
    let rows = recorded();
    let now: Vec<String> = each_proof(Proof::row);
    let (was, is): (BTreeSet<&String>, BTreeSet<&String>) = (rows.iter().collect(), now.iter().collect());
    let gone: Vec<&&String> = was.difference(&is).take(8).collect();
    let new: Vec<&&String> = is.difference(&was).take(8).collect();
    assert!(
        gone.is_empty() && new.is_empty(),
        "the rules' obligations moved since Z3 proved them (a rule, the front end or a chip's lane changed the graphs): prove \
         and record them again, `{RECORD}`\nrecorded, not made today: {gone:#?}\nmade today, not recorded: {new:#?}"
    );
    assert_eq!(rows.len(), now.len(), "one row a proof");
    let proven = by_step(&rows);
    let missing: Vec<String> = every_step().into_iter().chain([UNHELD.to_string()]).filter(|s| !proven.contains_key(s)).collect();
    assert!(missing.is_empty(), "steps with no proof on the corpus: {missing:?}");
}

/// Every rule's witness that its dataflow forbids it is among the proven, its twin too.
#[test]
fn every_forbidding_witness_and_its_twin_is_proven() {
    let rows = recorded();
    for (rule, why, _, _) in HOLD_BACKS {
        assert!(rows.iter().any(|r| r.starts_with(&format!("{rule}: {why} (rows "))), "`{rule}`'s witness that its dataflow forbids it ({why}) is proven");
        assert!(rows.iter().any(|r| r.starts_with(&format!("{rule}: twin of {why} (rows ")) && r.contains(&format!("\t{rule}\t"))), "`{rule}` is proven where it fires, on the twin of {why}");
    }
}

/// A row the program takes by a literal, and the same row named by a parameter the program fixes to that literal and
/// reads no row by: equal exactly when the parameter holds the literal, which only the program's interval for it says.
fn a_literal_row_named_by_its_parameter(proven_range: bool) -> (TensorGraph, TensorGraph) {
    let decl = "## Main\nLet T be a 4 by 8 tensor of Real.\nLet pos be 2.\n";
    let mut g = lifted(&format!("{decl}Let R be row 2 of T.\n"), false, None);
    if !proven_range {
        g.tensors.get_mut("pos").expect("the parameter").range = None;
    }
    (g, lifted(&format!("{decl}Let R be row pos of T.\n"), false, None))
}

/// A row taken by a parameter, and the same row taken from a view of the table's first two rows: equal only where the
/// parameter names one of those two, which the reference's own read of four rows does not say.
#[cfg_attr(not(feature = "verification"), allow(dead_code))]
fn row_through_a_view() -> (TensorGraph, TensorGraph) {
    let g = lifted("## Main\nLet T be a 4 by 8 tensor of Real.\nLet pos be 2.\nLet R be row pos of T.\n", false, None);
    let mut rewritten = g.clone();
    let at = rewritten.ops.iter().position(|o| o.kind() == "RowGather").expect("the row taken");
    let mut gather = rewritten.ops[at].clone();
    let Sym::Prim(TensorOpSpec::RowGather(spec)) = &mut gather.sym else { unreachable!("a row gather") };
    spec.rows = 2;
    gather.ins[0] = "V".to_string();
    let view = Op { sym: Sym::View { offset: 0 }, ins: vec!["T".to_string()], access: vec![Access::Whole], out: "V".to_string(), dst: Dst::to("V"), keeps: Vec::new(), covers: Vec::new() };
    rewritten.tensors.insert(
        "V".to_string(),
        Tensor { name: "V".to_string(), rows: 2, cols: 8, role: Role::Activation, elem: Elem::Float, fill: None, storage: Storage::Computed, range: None, observed: false, carried: false },
    );
    rewritten.ops[at] = gather;
    rewritten.ops.insert(at, view);
    (g, rewritten)
}

/// A rewrite whose meaning holds only by a fact the program proves assumes that fact, and cites it by the tensor's name;
/// the rewritten graph's reads are owed.
#[test]
fn a_range_fact_is_assumed_and_cited() {
    let (g, rewritten) = a_literal_row_named_by_its_parameter(true);
    assert_eq!(g.tensors["pos"].range, Some((2, 2)), "the program fixes `pos` and reads no row by it");
    let ob = obligation(&g, &rewritten).expect("an obligation");
    assert!(ob.assumed.iter().any(|a| a == "`pos` holds [2, 2], the interval the program proves"), "the range of `pos` is assumed: {:?}", ob.assumed);
    let text = ob.text();
    assert!(text.contains("; assumed: `pos` holds [2, 2], the interval the program proves\n(assert "), "and cited in the text:\n{text}");
    assert!(text.contains("; owed: `pos` names a row of `T`'s 4 rows, as the rewritten graph reads it"), "the named read is owed:\n{text}");
    let (g, rewritten) = a_literal_row_named_by_its_parameter(false);
    let bare = obligation(&g, &rewritten).expect("an obligation");
    assert!(!bare.text().contains("`pos` holds"), "no interval is assumed where the program proves none:\n{}", bare.text());
}

/// The slice_read witness's graph with the slice moved past the write to its source anyway: what `slice_read` would make
/// were the write between not to stop it.
fn slice_moved_past_its_sources_write() -> (TensorGraph, TensorGraph) {
    let (_, _, forbidding, _) = HOLD_BACKS.iter().find(|c| c.0 == "slice_read").expect("slice_read's witness");
    let g = lifted(forbidding, false, None);
    let slice = g.ops.iter().position(|o| o.kind() == "HeadSlice").expect("the slice");
    let reader = g.ops.iter().position(|o| o.ins.contains(&g.ops[slice].out) && o.kind() != "HeadSlice").expect("its reader");
    let (from, to) = (g.ops[slice].out.clone(), g.ops[slice].ins[0].clone());
    let mut moved = g.clone();
    read_in_place(&mut moved.ops[reader], &from, &to, Access::Strided(Strided { pitch: 16, offset: 8, width: 8 }));
    moved.ops.remove(slice);
    (g, moved)
}

/// An in-place write between a read and its reader makes the move it forbids an obligation no proof meets: the
/// obligation of the slice moved past its source's write is not unsatisfiable.
#[test]
fn a_write_between_a_slice_and_its_reader_makes_the_move_unprovable() {
    let (g, moved) = slice_moved_past_its_sources_write();
    let ob = obligation(&g, &moved).expect("an obligation");
    assert!(ob.compared > ob.identical, "the moved read is a different term, for Z3 to judge: {ob:?}");
}

/// The algebraic identities a chip's link computes a fused class by: each a pair of programs at a shape.
struct Identity {
    name: &'static str,
    classes: &'static [ClassId],
    pair: fn(&[usize]) -> (String, String),
    dims: usize,
}

fn decl(name: &str, rows: usize, cols: usize) -> String {
    format!("Let {name} be a {rows} by {cols} tensor of Real.\n")
}

const IDENTITIES: &[Identity] = &[
    Identity {
        name: "one product by weights side by side is the products by each, side by side",
        classes: &[ClassId::MergedQkv, ClassId::MergedGateUp, ClassId::GemvGateUpGlu],
        dims: 4,
        pair: |d| {
            let (m, k, na, nb) = (d[0], d[1], d[2], d[3]);
            let ins = format!("## Main\n{}{}{}", decl("X", m, k), decl("A", k, na), decl("B", k, nb));
            (
                format!("{ins}Let P be X matmul A.\nLet Q be X matmul B.\n"),
                format!(
                    "{ins}Let W be A joined with B.\nLet R be X matmul W.\nLet P be the first {na} of every {} of R.\nLet Q be the last {nb} of every {} of R.\n",
                    na + nb,
                    na + nb
                ),
            )
        },
    },
    Identity {
        name: "a product of products reassociates",
        classes: &[ClassId::Mla, ClassId::MlaUnabsorbInFold],
        dims: 4,
        pair: |d| {
            let (m, k, l, n) = (d[0], d[1], d[2], d[3]);
            let ins = format!("## Main\n{}{}{}", decl("X", m, k), decl("A", k, l), decl("B", l, n));
            (format!("{ins}Let T be X matmul A.\nLet Y be T matmul B.\n"), format!("{ins}Let U be A matmul B.\nLet Y be X matmul U.\n"))
        },
    },
    Identity {
        name: "a scale moves through a product",
        classes: &[ClassId::AffineInputFold, ClassId::GemmScale],
        dims: 3,
        pair: |d| {
            let (m, k, n) = (d[0], d[1], d[2]);
            let ins = format!("## Main\n{}{}", decl("X", m, k), decl("M", k, n));
            (format!("{ins}Let S be X scaled by 0.375.\nLet Y be S matmul M.\n"), format!("{ins}Let P be X matmul M.\nLet Y be P scaled by 0.375.\n"))
        },
    },
    Identity {
        name: "an arg-max reads through a soft cap, which is strictly increasing",
        classes: &[ClassId::CapArgmax],
        dims: 1,
        pair: |d| {
            let ins = format!("## Main\n{}", decl("X", 1, d[0]));
            (format!("{ins}Let Y be X capped at 30.0.\nLet next be argmax of Y.\n"), format!("{ins}Let next be argmax of X.\n"))
        },
    },
];

/// Every shape with each of `dims` sizes from 1 to `bound`.
fn shapes(dims: usize, bound: usize) -> Vec<Vec<usize>> {
    (0..dims).fold(vec![Vec::new()], |acc, _| acc.into_iter().flat_map(|s| (1..=bound).map(move |v| [s.clone(), vec![v]].concat())).collect())
}

fn graph(source: &str) -> TensorGraph {
    let rp = recognize_stages(source).unwrap_or_else(|e| panic!("{e:?}:\n{source}"));
    TensorGraph::lift(&rp, None, None, &|_| Storage::Values).unwrap_or_else(|e| panic!("{e}:\n{source}"))
}

/// The bound every identity is proven up to, each of its sizes.
const BOUND: usize = 3;

fn identity_obligations() -> Vec<(String, Obligation)> {
    let mut out = Vec::new();
    for id in IDENTITIES {
        for shape in shapes(id.dims, BOUND) {
            let (lhs, rhs) = (id.pair)(&shape);
            let ob = identity(&graph(&lhs), &graph(&rhs)).unwrap_or_else(|e: SmtError| panic!("{} at {shape:?}: {e}", id.name));
            assert!(ob.compared > 0 && ob.identical < ob.compared, "{} at {shape:?}: the two sides compute different terms for Z3 to equate", id.name);
            out.push((format!("{} at {shape:?}", id.name), ob));
        }
    }
    out
}

/// Two sides where one term of the second is paired with two of the first: `P` and `Q` each a product of a scaled `X`
/// by `A` on the right, by `A` and by `B` on the left. Proving the first pair joins the shared term to `P`'s class, and
/// the second pair then joins `Q`'s to it too.
fn a_term_two_pairs_share() -> (TensorGraph, TensorGraph) {
    let ins = format!("## Main\n{}{}{}", decl("X", 2, 2), decl("A", 2, 2), decl("B", 2, 2));
    (
        graph(&format!("{ins}Let P be X matmul A.\nLet Q be X matmul B.\n")),
        graph(&format!("{ins}Let S be X scaled by 1.0.\nLet P be S matmul A.\nLet Q be S matmul A.\n")),
    )
}

/// An obligation is made in finite steps: a term two proven pairs share is one class with both, so no pair is chosen
/// twice. The obligation names a lemma a pair.
#[test]
fn a_term_two_proven_pairs_share_is_one_class_with_both() {
    let (lhs, rhs) = a_term_two_pairs_share();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || tx.send(identity(&lhs, &rhs).map(|o| o.lemmas.len())));
    let lemmas = rx.recv_timeout(std::time::Duration::from_secs(30)).expect("the obligation is made in finite steps").expect("an obligation");
    assert_eq!(lemmas, 2, "one lemma for each pair the shared term is in");
}

/// Every identity names the classes it justifies, and every shape up to the bound gives an obligation.
#[test]
fn every_identity_states_its_classes_and_its_shapes() {
    let mut classes = BTreeSet::new();
    for id in IDENTITIES {
        assert!(!id.classes.is_empty(), "{}: names the classes whose links compute by it", id.name);
        classes.extend(id.classes.iter().copied());
    }
    for c in [ClassId::MergedQkv, ClassId::Mla, ClassId::AffineInputFold] {
        assert!(classes.contains(&c), "`{c}` is computed by an identity its link relies on, and none is stated");
    }
    let n: usize = IDENTITIES.iter().map(|id| BOUND.pow(id.dims as u32)).sum();
    assert_eq!(identity_obligations().len(), n, "one obligation a shape");
}

#[cfg(feature = "verification")]
mod proven {
    use super::*;
    use logicaffeine_tv::tensor::{discharge, Verdict};

    const TIMEOUT_MS: u32 = 60_000;

    fn header() -> &'static str {
        "# witness\tstep\ttensors compared\tof them one term on both sides\tsha256 of the obligation Z3 proved\n"
    }

    /// Z3 discharges every obligation the rules make of the corpus; under `LOGOS_UPDATE_FIXTURES=1` the proven rows are
    /// recorded.
    #[test]
    fn every_grouping_rule_is_proven_over_arrays() {
        let judged = each_proof(|p| (p.row(), discharge(&p.obligation, TIMEOUT_MS), format!("{} / `{}`", p.witness, p.step)));
        let failed: Vec<String> = judged.iter().filter(|(_, v, _)| *v != Verdict::Proven).map(|(_, v, what)| format!("{what}: {v:?}")).collect();
        assert!(failed.is_empty(), "{} obligations not proven:\n{}", failed.len(), failed.join("\n"));
        let rows: Vec<String> = judged.into_iter().map(|(row, _, _)| row).collect();
        if std::env::var("LOGOS_UPDATE_FIXTURES").is_ok_and(|v| v == "1") {
            std::fs::write(fixture(), format!("{}{}\n", header(), rows.join("\n"))).expect("write the proofs");
            return;
        }
        assert_eq!(rows, recorded(), "the proven obligations are the recorded ones");
    }

    /// The row named by `pos` is the literal's row by the program's interval for `pos`, and refuted without it; a view of
    /// the table's first two rows is refuted with the interval the reference's read of four rows gives.
    #[test]
    fn a_rewrite_whose_meaning_needs_a_range_fact_is_proven_with_it_and_refuted_without() {
        let (g, rewritten) = a_literal_row_named_by_its_parameter(true);
        assert_eq!(discharge(&obligation(&g, &rewritten).expect("an obligation"), TIMEOUT_MS), Verdict::Proven);
        let (g, rewritten) = a_literal_row_named_by_its_parameter(false);
        assert!(matches!(discharge(&obligation(&g, &rewritten).expect("an obligation"), TIMEOUT_MS), Verdict::Refuted(_)));
        let (g, rewritten) = row_through_a_view();
        assert!(matches!(discharge(&obligation(&g, &rewritten).expect("an obligation"), TIMEOUT_MS), Verdict::Refuted(_)));
    }

    /// Facts no input satisfies prove nothing: an interval for `pos` past the four rows the reference reads it by makes
    /// the premises unsatisfiable, and the obligation vacuous rather than proven.
    #[test]
    fn premises_no_input_satisfies_are_vacuous_not_proven() {
        let (mut g, rewritten) = row_through_a_view();
        g.tensors.get_mut("pos").expect("the position").range = Some((5, 5));
        assert_eq!(discharge(&obligation(&g, &rewritten).expect("an obligation"), TIMEOUT_MS), Verdict::Vacuous);
    }

    /// An arg-max means the first index of its row's greatest value: through a scale by a positive factor it picks what
    /// the row's own arg-max picks, and through a negative one Z3 finds a row it moves.
    #[test]
    fn an_arg_max_reads_through_an_increasing_map_and_not_a_decreasing_one() {
        let ins = format!("## Main\n{}", decl("X", 1, 8));
        let raw = graph(&format!("{ins}Let next be argmax of X.\n"));
        for (factor, proven) in [("2.0", true), ("-2.0", false)] {
            let scaled = graph(&format!("{ins}Let Y be X scaled by {factor}.\nLet next be argmax of Y.\n"));
            let verdict = discharge(&identity(&scaled, &raw).expect("an obligation"), TIMEOUT_MS);
            if proven {
                assert_eq!(verdict, Verdict::Proven, "a scale by {factor}");
            } else {
                assert!(matches!(verdict, Verdict::Refuted(_)), "a scale by {factor}: {verdict:?}");
            }
        }
    }

    /// Z3 refutes the move a write between forbids: a meaning of the rope under which the moved read differs.
    #[test]
    fn a_move_past_a_write_to_its_source_is_refuted() {
        let (g, moved) = slice_moved_past_its_sources_write();
        let ob = obligation(&g, &moved).expect("an obligation");
        assert!(matches!(discharge(&ob, TIMEOUT_MS), Verdict::Refuted(_)), "the move is refuted");
    }

    /// Every algebraic identity is proven at every shape up to the bound, products, sums and scales exact.
    #[test]
    fn every_algebraic_rewrite_is_proven_at_bounded_shapes() {
        let failed: Vec<String> = identity_obligations()
            .into_iter()
            .filter_map(|(what, ob)| match discharge(&ob, TIMEOUT_MS) {
                Verdict::Proven => None,
                other => Some(format!("{what}: {other:?}")),
            })
            .collect();
        assert!(failed.is_empty(), "identities not proven:\n{}", failed.join("\n"));
    }

    /// Of the two pairs a term shares, the false one is refuted.
    #[test]
    fn a_false_pair_beside_a_true_one_is_refuted() {
        let (lhs, rhs) = a_term_two_pairs_share();
        assert!(matches!(discharge(&identity(&lhs, &rhs).expect("an obligation"), TIMEOUT_MS), Verdict::Refuted(_)));
    }

    /// An identity that does not hold is refuted: the scale moved through a product with a different factor.
    #[test]
    fn a_false_identity_is_refuted() {
        let ins = format!("## Main\n{}{}", decl("X", 2, 2), decl("M", 2, 2));
        let lhs = graph(&format!("{ins}Let S be X scaled by 0.375.\nLet Y be S matmul M.\n"));
        let rhs = graph(&format!("{ins}Let P be X matmul M.\nLet Y be P scaled by 0.5.\n"));
        assert!(matches!(discharge(&identity(&lhs, &rhs).expect("an obligation"), TIMEOUT_MS), Verdict::Refuted(_)));
    }
}
