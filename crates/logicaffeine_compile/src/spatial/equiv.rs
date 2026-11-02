//! `equiv` — the differential that decides whether two spatial programs MEAN the same thing.
//!
//! A pass, a legalization, an imported placement, a hand-written lowering: each hands us a second
//! [`SpatialProgram`] together with a claim that it computes what the first one computes.
//! [`equivalent_under`] is how that claim is discharged — evaluate both under the executable token
//! semantics of [`super::eval`], on the SAME inputs, over a corpus of them, and name the first place the
//! two part company.
//!
//! ★ WHY THIS EXISTS WHEN `certify_external_spatial` ALREADY HAS THE SHAPE. That function
//! (`spatial/certificate.rs:759`) looks like the same idea — same tensors, evaluate both, compare — and it
//! cannot validate a rewrite, for two independent reasons, either of which alone is fatal:
//!
//! 1. **It is a ONE-POINT differential.** It builds `dry_inputs(source)`: the recorded fills, and *zeros*
//!    wherever the data is external. One evaluation, at one point, and that point is the origin. A rewrite
//!    that is wrong everywhere except there passes it, and zero is exactly the argument at which dropped
//!    terms, swapped operands and lost scale factors are all invisible.
//! 2. **It gates on STRUCTURAL stage-kind equality.** Before evaluating anything it compares the two
//!    programs' sorted, deduplicated multisets of `discriminant(StageKind)` — and, for compute stages,
//!    `discriminant(ComputeOp)` — returning `None` when they differ. Every legalization worth the name
//!    violates that BY CONSTRUCTION: expanding one fused `ComputeOp::RowReduce` into the
//!    `Elementwise → ReduceRows → RowXform → RowRescale` chain changes those discriminants, and changing
//!    them IS the rewrite. A validator that refuses precisely the programs it exists to validate can never
//!    be called on one. (It has, today, no callers at all.)
//!
//! This module is the generalization: a CORPUS rather than a point, and no structural precondition
//! whatsoever — only the tensors that come out the other end. The two programs need not share a stage, a
//! buffer, a placement or a schedule. They need only agree on what they compute.
//!
//! Equality here is [`Outputs`]'s own equality, applied element by element so the answer can say WHERE:
//! `Value::Int` compares exactly, `Value::Float` compares with `==` (so a rewrite that turns a number into
//! `NaN` on both sides is reported as a divergence rather than waved through — fail-closed). `steps` is
//! deliberately NOT compared: how many rounds a schedule takes is an artifact of the arrangement, not of
//! the meaning, and a rewrite that changes it has changed nothing this module is asked about.

use std::fmt;

use super::eval::{eval_choreography, ChoreoFault, Inputs, Outputs, Value};
use super::ir::{SpatialProgram, TensorDecl, TensorId, TensorRole};
use super::target::precision_is_int;

/// Which of the two programs a divergence is being reported against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::A => "a",
            Side::B => "b",
        }
    }
}

/// One element, in whichever numeric tower its tensor lives in — enough to read a divergence without
/// carrying two whole tensors through a panic message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Elem {
    Int(i64),
    Float(f64),
}

impl fmt::Display for Elem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Elem::Int(v) => write!(f, "{v}"),
            Elem::Float(v) => write!(f, "{v}"),
        }
    }
}

/// Why two programs are not the same program.
///
/// ★ NEVER A BARE `bool`. "Not equivalent" is unactionable; a failed differential has to say which input
/// of the corpus broke it, which tensor, which element, and what the two sides put there — otherwise the
/// next person re-derives all of that by hand before they can start.
#[derive(Clone, Debug, PartialEq)]
pub enum Divergence {
    /// The caller passed no inputs. ★ Fail-closed: a validator handed nothing to check must never answer
    /// "equivalent", because a vacuous pass reads exactly like a proof.
    EmptyCorpus,
    /// One side could not be evaluated at all — a choreography fault, not an arithmetic disagreement.
    Faulted {
        side: Side,
        input: usize,
        fault: ChoreoFault,
    },
    /// One side produced an output tensor the other never produced.
    MissingTensor {
        side: Side,
        input: usize,
        tensor: TensorId,
    },
    /// The same tensor came back in two different representations (int vs float, or two lengths).
    Representation {
        input: usize,
        tensor: TensorId,
        a: &'static str,
        b: &'static str,
        a_len: usize,
        b_len: usize,
    },
    /// The two sides computed different numbers. `elem` is the FIRST differing flat index (row-major over
    /// the tensor's element shape); `differing` of `total` says how much of the tensor moved, which is what
    /// separates "one rounding" from "the whole rewrite is wrong".
    Element {
        input: usize,
        tensor: TensorId,
        elem: usize,
        a: Elem,
        b: Elem,
        differing: usize,
        total: usize,
    },
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Divergence::EmptyCorpus => write!(
                f,
                "an empty corpus proves nothing: pass at least one Inputs (see seeded_corpus)"
            ),
            Divergence::Faulted { side, input, fault } => {
                write!(
                    f,
                    "program `{}` faulted on corpus input {input}: {fault:?}",
                    side.name()
                )
            }
            Divergence::MissingTensor {
                side,
                input,
                tensor,
            } => {
                write!(
                    f,
                    "on corpus input {input}, program `{}` never produced output tensor {}",
                    side.name(),
                    tensor.idx()
                )
            }
            Divergence::Representation {
                input,
                tensor,
                a,
                b,
                a_len,
                b_len,
            } => {
                write!(f, "on corpus input {input}, output tensor {} came back as {a}[{a_len}] from `a` but {b}[{b_len}] from `b`", tensor.idx())
            }
            Divergence::Element {
                input,
                tensor,
                elem,
                a,
                b,
                differing,
                total,
            } => {
                write!(
                    f,
                    "on corpus input {input}, output tensor {} differs at element {elem}: a = {a}, b = {b} ({differing} of {total} elements differ)",
                    tensor.idx()
                )
            }
        }
    }
}

/// Do `a` and `b` compute the same outputs on every input of the corpus?
///
/// `Ok(())` is the equivalence claim discharged over exactly those points — no more, and the corpus is the
/// caller's argument for "no less". `Err` names the first divergence found, scanning inputs in order and
/// tensors in id order, so the report is deterministic and diffable across runs.
///
/// Both programs are evaluated under [`super::eval::Scheduler::RoundRobin`]; scheduler independence is a
/// separate property with its own oracle (`eval_choreography_confluent`) and conflating the two would make
/// a nondeterministic program look like an inequivalent one.
///
/// ★ WHAT THIS IS NOT. Both sides run under the SAME evaluator, so this validates a REWRITE against the
/// IR's semantics — never the semantics against the mathematics. A pass that preserves an operator the
/// compiler has always computed wrongly passes here and should: that error belongs to
/// `spatial_algebra_semantics_lock`, which checks `eval_choreography` against references written out
/// longhand from each operator's definition. The two oracles are complementary and neither substitutes.
pub fn equivalent_under(
    a: &SpatialProgram,
    b: &SpatialProgram,
    inputs: &[Inputs],
) -> Result<(), Divergence> {
    if inputs.is_empty() {
        return Err(Divergence::EmptyCorpus);
    }
    for (i, input) in inputs.iter().enumerate() {
        let oa = eval_choreography(a, input).map_err(|fault| Divergence::Faulted {
            side: Side::A,
            input: i,
            fault,
        })?;
        let ob = eval_choreography(b, input).map_err(|fault| Divergence::Faulted {
            side: Side::B,
            input: i,
            fault,
        })?;
        compare_outputs(i, &oa, &ob)?;
    }
    Ok(())
}

/// ★ THE UNION OF THE KEYS, NOT ONE SIDE'S. A rewrite that quietly stops producing an output would pass a
/// comparison that only walks `a`'s tensors, which is the one failure mode a validator must never have.
///
/// The union is never EMPTY, and that is an invariant of the IR rather than of this function:
/// `SpatialProgram::validate` refuses a program without an output tensor, and `Eval::finish` emits one
/// entry per output-role tensor or faults with `OutputTileNeverStored`. So there is no vacuous
/// "both sides produced nothing, therefore they agree" path to guard here — if that ever becomes
/// reachable it is `validate` that regressed, and this comment is the place the assumption is recorded.
fn compare_outputs(input: usize, a: &Outputs, b: &Outputs) -> Result<(), Divergence> {
    let mut ids: Vec<TensorId> = a.tensors.keys().copied().collect();
    for id in b.tensors.keys() {
        if !a.tensors.contains_key(id) {
            ids.push(*id);
        }
    }
    ids.sort();
    for id in ids {
        match (a.tensors.get(&id), b.tensors.get(&id)) {
            (Some(va), Some(vb)) => compare_value(input, id, va, vb)?,
            (None, Some(_)) => {
                return Err(Divergence::MissingTensor {
                    side: Side::A,
                    input,
                    tensor: id,
                })
            }
            (Some(_), None) => {
                return Err(Divergence::MissingTensor {
                    side: Side::B,
                    input,
                    tensor: id,
                })
            }
            (None, None) => unreachable!("an id came from one of the two maps"),
        }
    }
    Ok(())
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
    }
}

fn len_of(v: &Value) -> usize {
    match v {
        Value::Int(x) => x.len(),
        Value::Float(x) => x.len(),
    }
}

/// The relative tolerance a `NumericsClaim::Approx` program's rewrite is discharged against.
///
/// ★ WHY EXACTNESS IS THE WRONG QUESTION FOR SOME REWRITES. Fusing two stages removes a round trip:
/// the inner result stays in the destination registers instead of being packed to a buffer and
/// unpacked again. That is a REAL change to the arithmetic — it rounds LESS — so a fused kernel and
/// its unfused source differ in the last bits BY CONSTRUCTION. Demanding `==` would forbid the
/// optimization; waving it through unchecked would let a genuinely wrong rewrite hide behind "floating
/// point". So the program's own declared claim decides which question is asked: `Exact` uses
/// [`equivalent_under`], `Approx` uses [`equivalent_within`].
///
/// This bound is ~1e-6 relative — about eight fp32 ulps, and four orders of magnitude tighter than
/// the bf16 ulp (~4e-3) a wrong answer would have to hide inside.
pub const APPROX_REL_TOL: f64 = 1e-6;

/// [`equivalent_under`], but comparing floats within `rel` RELATIVE tolerance (falling back to `rel`
/// as an absolute bound near zero, so a divergence cannot pass merely because both sides are small).
/// Integers are still compared exactly — an approximate claim is about a float datapath, never about
/// an integer one.
pub fn equivalent_within(
    a: &SpatialProgram,
    b: &SpatialProgram,
    inputs: &[Inputs],
    rel: f64,
) -> Result<(), Divergence> {
    APPROX.with(|t| t.set(Some(rel)));
    let r = equivalent_under(a, b, inputs);
    APPROX.with(|t| t.set(None));
    r
}

thread_local! {
    /// Set only for the duration of an [`equivalent_within`] call. A thread-local keeps the tolerance
    /// out of the exact path's signature entirely, so `equivalent_under` cannot accidentally acquire
    /// a tolerance it was never asked for.
    static APPROX: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
}

/// Whether two floats agree under the tolerance currently in force (exact when none is).
fn floats_agree(p: f64, q: f64) -> bool {
    match APPROX.with(|t| t.get()) {
        None => p == q,
        Some(rel) => (p - q).abs() <= rel * p.abs().max(q.abs()).max(1.0),
    }
}

fn compare_value(input: usize, tensor: TensorId, a: &Value, b: &Value) -> Result<(), Divergence> {
    let mismatch = || Divergence::Representation {
        input,
        tensor,
        a: kind_of(a),
        b: kind_of(b),
        a_len: len_of(a),
        b_len: len_of(b),
    };
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => {
            if x.len() != y.len() {
                return Err(mismatch());
            }
            match x.iter().zip(y.iter()).position(|(p, q)| p != q) {
                None => Ok(()),
                Some(elem) => Err(Divergence::Element {
                    input,
                    tensor,
                    elem,
                    a: Elem::Int(x[elem]),
                    b: Elem::Int(y[elem]),
                    differing: x.iter().zip(y.iter()).filter(|(p, q)| p != q).count(),
                    total: x.len(),
                }),
            }
        }
        (Value::Float(x), Value::Float(y)) => {
            if x.len() != y.len() {
                return Err(mismatch());
            }
            // Exact unless an `equivalent_within` call is in force — see `floats_agree`.
            match x
                .iter()
                .zip(y.iter())
                .position(|(p, q)| !floats_agree(*p, *q))
            {
                None => Ok(()),
                Some(elem) => Err(Divergence::Element {
                    input,
                    tensor,
                    elem,
                    a: Elem::Float(x[elem]),
                    b: Elem::Float(y[elem]),
                    differing: x
                        .iter()
                        .zip(y.iter())
                        .filter(|(p, q)| !floats_agree(**p, **q))
                        .count(),
                    total: x.len(),
                }),
            }
        }
        _ => Err(mismatch()),
    }
}

// ── A seeded corpus ────────────────────────────────────────────────────────────────────────────────

/// The magnitude a corpus element is capped at.
///
/// ★ 128 BECAUSE BF16 IS THE NARROWEST OPERAND PRECISION THE TARGETS USE, and bf16 holds every integer up
/// to 256 exactly. A corpus inside that window can never manufacture an operand rounding difference the
/// workload did not already have, so a divergence it finds is a divergence of the REWRITE rather than of
/// the datapath. The caller keeps the other half of that bargain by not asking for a reduction so long
/// that the fp32 accumulator itself starts rounding (`K · 128²  <  2²⁴`).
pub const CORPUS_MAGNITUDE_CAP: i64 = 128;

/// The window used for a tensor that declares neither a range nor a bounded fill.
pub const DEFAULT_ABS_BOUND: i64 = 8;

/// The repo's deterministic hash PRNG — the same mixing `crates/logicaffeine_tests/tests/common/spatial.rs`
/// uses, restated here because a corpus builder that lives in the compiler cannot reach into the test tree.
/// Same seed, same index, same window ⇒ same value, on every machine and every run.
///
/// ★ TOTAL, AND THE WINDOW IS ORDERED BEFORE IT IS USED. A caller who hands the ends crossed gets a value
/// from the window those ends span, never one outside it: the whole force of a corpus is the promise that
/// every element it produces is inside the range the program declared and inside
/// [`CORPUS_MAGNITUDE_CAP`], and a builder that can quietly break that promise turns "the rewrite
/// diverged" into "the datapath rounded" with nothing in the report to tell them apart.
pub fn corpus_prng(seed: u64, i: usize, lo: i64, hi: i64) -> i64 {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    if lo == hi {
        return lo;
    }
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(i as u64 + 1);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    let span = (hi as i128 - lo as i128 + 1) as u128;
    (lo as i128 + (x as u128 % span) as i128) as i64
}

/// The window a tensor's corpus data is drawn from: its DECLARED range where it has one (that declaration
/// is what the numerics claim was derived from, so leaving it is leaving the program's premises), else the
/// bound its recorded fill implies, else [`DEFAULT_ABS_BOUND`] — capped either way.
///
/// ★ THE CAP WINS, AND IT CLAMPS THE TWO ENDS TOGETHER. A declared range lying wholly outside
/// ±[`CORPUS_MAGNITUDE_CAP`] (weights in `[500, 1000]`, say) must still come back as an ORDERED window:
/// clamping each end on its own returns `lo > hi`, and a "window" whose ends have crossed is exactly how a
/// corpus builder emits the value it promised it never would. Clamping and then re-ordering keeps the
/// guarantee the cap exists for — every element exactly representable in the narrowest operand precision —
/// at the price of a window narrower than the declaration asked for. That is the trade
/// [`CORPUS_MAGNITUDE_CAP`] makes on purpose: a corpus outside the exactness window cannot tell a diverged
/// rewrite from a rounded datapath, so it is worth less than a narrow one.
fn corpus_range(t: &TensorDecl) -> (i64, i64) {
    let (lo, hi) = match t.range {
        Some((lo, hi)) if hi > lo => (lo, hi),
        _ => match t.init.abs_bound() {
            Some(b) if b > 0 => (-b, b),
            _ => (-DEFAULT_ABS_BOUND, DEFAULT_ABS_BOUND),
        },
    };
    let (lo, hi) = (
        lo.clamp(-CORPUS_MAGNITUDE_CAP, CORPUS_MAGNITUDE_CAP),
        hi.clamp(-CORPUS_MAGNITUDE_CAP, CORPUS_MAGNITUDE_CAP),
    );
    (lo.min(hi), lo.max(hi))
}

/// A `Param` the compiler PLANTED, materialized from its own recorded fill — `Some` exactly when every
/// element of that fill reproduces.
///
/// ★ A `Param` WHOSE FILL REPRODUCES IS PART OF THE PROGRAM, NOT ITS DATA. The rmsnorm chain's
/// `rms_scaler` is literally `1/cols`, a constant the compiler planted so the reduction yields a mean;
/// overwriting it with corpus noise would compare two programs on data neither of them was built for, and
/// would silently break every rewrite that reads it. A `Param` the compiler left `External` has no
/// recorded value, so it is data like any other and must be supplied.
///
/// ★ THE TEST AND THE VALUE ARE ONE FUNCTION ON PURPOSE. Asking "does this fill reproduce?" in one place
/// and then rebuilding it in another invites the two to drift, and the only way to paper over the drift is
/// an `unwrap_or` — a corpus element silently replaced by zero, which reads downstream as a rewrite that
/// dropped a term. Returning the value IS the predicate, so there is nothing to keep in step.
fn planted_constant(t: &TensorDecl) -> Option<Value> {
    if t.role != TensorRole::Param {
        return None;
    }
    let n = t.elems.rows * t.elems.cols;
    if t.init.is_float() {
        (0..n)
            .map(|i| t.init.element_f64(i))
            .collect::<Option<Vec<f64>>>()
            .map(Value::Float)
    } else {
        (0..n)
            .map(|i| t.init.element(i))
            .collect::<Option<Vec<i64>>>()
            .map(Value::Int)
    }
}

/// One corpus point for `p`: every tensor the evaluator needs, keyed by [`TensorId`].
///
/// ★ KEYED BY ID, WHICH IS WHAT MAKES IT A DIFFERENTIAL. Two programs over the same workload agree on
/// tensor ids, so one corpus feeds both and each side genuinely sees the same numbers. A tensor one side
/// does not have (the chain's scaler, say) is simply never looked up.
///
/// Values are always INTEGRAL — `Value::Int` on an integer datapath, whole numbers as `Value::Float` on a
/// float one. That is deliberate: see [`CORPUS_MAGNITUDE_CAP`].
///
/// ★ WHAT THIS CORPUS DELIBERATELY HOLDS STILL, stated so a caller is never surprised by it: a `Param`
/// whose recorded fill reproduces is reproduced EXACTLY, identically at every seed (see
/// `planted_constant`). For a compiler-planted constant that is the only correct thing to do; for a
/// weight tensor the recognizer happened to record a fill for, it means the corpus explores the activations
/// and not the weights. The differential stays SOUND either way — both sides still see the same numbers —
/// but it is narrower, and a caller who wants that tensor varied must build the `Inputs` itself and supply
/// it rather than expecting this function to guess which `Param`s are data.
pub fn seeded_inputs(p: &SpatialProgram, seed: u64) -> Inputs {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let n = t.elems.rows * t.elems.cols;
        if let Some(planted) = planted_constant(t) {
            inputs.tensors.insert(t.id, planted);
            continue;
        }
        let (lo, hi) = corpus_range(t);
        // Mixing the tensor id into the seed keeps two operands of one program from being handed the same
        // stream (a matmul against its own transpose is a poor test), while keeping the SAME tensor of two
        // programs on the same stream, which is the whole point.
        let s = seed ^ (t.id.0 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        if precision_is_int(t.dtype) {
            inputs.tensors.insert(
                t.id,
                Value::Int((0..n).map(|i| corpus_prng(s, i, lo, hi)).collect()),
            );
        } else {
            inputs.tensors.insert(
                t.id,
                Value::Float((0..n).map(|i| corpus_prng(s, i, lo, hi) as f64).collect()),
            );
        }
    }
    inputs
}

/// A corpus: one [`seeded_inputs`] per seed, in the order given.
pub fn seeded_corpus(p: &SpatialProgram, seeds: &[u64]) -> Vec<Inputs> {
    seeds.iter().map(|s| seeded_inputs(p, *s)).collect()
}
