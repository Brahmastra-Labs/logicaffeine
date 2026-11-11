//! Maps the program writes as loops: a counted loop, or a nest of two, whose every turn computes one element of a tensor
//! ([`crate::tensor_recognize::map`]) — lowered as one section of `Generic` stages; a recurrence along the outer loop,
//! lowered turn by turn, or, linear in its carry, as a chunked scan.

use super::*;

/// A map the program wrote as a counted loop ([`MapSpec`](crate::tensor_recognize::map::MapSpec)) — `out[p] = body(p)` at every place `p` of the loop nest,
/// row-major — as ONE section whose stages are `Generic`, the last one's body the turn's arithmetic. An op nobody wrote
/// a sugar or a kernel for compiles as what it computes.
///   · An operand read at the turn's place is the flat row at `p`, a fixed entry a fixed element, and the turn's position
///     in a loop of the nest the entry `p` of a compile-time table of positions.
///   · A Real quotient is a product by the reciprocal; `e^x`, `ln x` and `√x` (the reciprocal of the reciprocal root) are
///     the datapath's transforms, and `tanh x` is `1 − 2/(e^{2x} + 1)`. An Int quotient truncates and an Int `//` floors,
///     as the VM's do, in integer arithmetic throughout.
///   · A branch is a `Select` decided by values: a stage of its own computes each comparison's difference, and the branch
///     tests it against the flat space's one row, row 0 — `0 < a − b` holds exactly where `a > b`, a NaN included, so the
///     arm taken is the VM's. Two infinities of one sign differ by a NaN, and so never compare equal here.
/// A power is lowered where its exponent is a whole literal (a product) or one half (a root), or its base the constant `e`
/// or another positive constant (`b^x = e^(x·ln b)`); any other power is refused by name.
pub fn from_map(
    spec: &crate::tensor_recognize::map::MapSpec,
    names: &StageNames,
    reals: &[bool],
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let n = spec.n();
    if n == 0 {
        return Err(SpatialReject::NotLowered("a map over no places"));
    }
    if spec.recurs() {
        let float = spec.real || reals.iter().any(|r| *r);
        return match linear_in_earlier(&spec.body) {
            Some((decay, input)) if float && spec.extents[0] > 2 => chunked_map(spec, (&decay, &input), names, reals, target),
            _ => stepped_map(spec, names, reals, target),
        };
    }
    let float = spec.real || reals.iter().any(|r| *r);
    if !float && row_needs_blocking(n, target) {
        return Err(SpatialReject::NotLowered(
            "an integer map wider than a site's page — its integers ride the integer datapath, a row a page holds",
        ));
    }
    let mut b = if float { FlatBuild::columns(n, target)? } else { FlatBuild::on(Datapath::Int, target)? };
    let out_dt = if spec.real { b.precision } else { Datapath::Int.precision(target)? };
    let mut section = MapSection { spec, names, reals, target, n, operands: vec![None; spec.ins.len()], positions: vec![None; spec.extents.len()], shared: None, held: Vec::new() };
    let differences = differences_of(&spec.body);
    let mut chain: Vec<StageId> = Vec::new();
    let mut pages: Vec<BufferId> = Vec::with_capacity(differences.len());
    for (k, (x, y)) in differences.iter().enumerate() {
        let mut stage = MapStage { tested: differences.clone(), ..MapStage::default() };
        let body = TensorBody::Sub(Box::new(section.term(&mut b, &mut stage, x, &pages)?), Box::new(section.term(&mut b, &mut stage, y, &pages)?));
        let name = format!("map_difference{k}");
        let page = if float {
            b.block_scratch(&name, n, target)
        } else {
            let p = b.precision;
            b.t.scratch(&name, tiles_of(n, target), p)
        };
        chain.extend(b.pointwise(&name, n, stage.slots, body, page, target)?);
        pages.push(page);
    }
    let mut stage = MapStage { tested: differences.clone(), ..MapStage::default() };
    let body = section.term(&mut b, &mut stage, &spec.body, &pages)?;
    let out_buf = b.result_page("out", n, out_dt, target);
    chain.extend(b.pointwise("map", n, stage.slots, body.clone(), out_buf, target)?);
    let claim = if float { approx("map", out_dt) } else { claim_of("map", &body, out_dt) };
    b.finish_as(&names.out, n, chain, out_buf, format!("map_{n}"), claim, out_dt, target)
}

/// A recurrence the program wrote ([`MapSpec::recurs`](crate::tensor_recognize::map::MapSpec::recurs)) as ONE section that takes its outer loop's turns in order: turn `t`
/// is a stage over the `stride` places of its row, reading every operand from the row's first place, its outer position
/// the literal `t`, and the row the stage before it wrote; a last stage lays the rows side by side. The turns of the
/// inner loop run at once, as the places of one row.
pub fn stepped_map(
    spec: &crate::tensor_recognize::map::MapSpec,
    names: &StageNames,
    reals: &[bool],
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (n, row) = (spec.n(), spec.stride());
    let float = spec.real || reals.iter().any(|r| *r);
    if row_needs_blocking(n, target) {
        return Err(SpatialReject::NotLowered(
            "a recurrence holds every turn's row in one site, and this one's rows are wider together than a site's page",
        ));
    }
    let mut b = if float { FlatBuild::new(target)? } else { FlatBuild::on(Datapath::Int, target)? };
    let out_dt = if spec.real { b.precision } else { Datapath::Int.precision(target)? };
    let mut section = MapSection {
        spec,
        names,
        reals,
        target,
        n,
        operands: vec![None; spec.ins.len()],
        positions: vec![None; spec.extents.len()],
        shared: Some(Vec::new()),
        held: Vec::new(),
    };
    let acc = AccPolicy::for_result(b.precision).interm_precision();
    let mut chain: Vec<StageId> = Vec::new();
    let mut rows: Vec<BufferId> = Vec::with_capacity(spec.extents[0]);
    for t in 0..spec.extents[0] {
        // The turn's own arithmetic: its outer position a literal, so every branch on it alone is decided here.
        let body = at_turn(&spec.body, t + 1);
        let differences = differences_of(&body);
        let place = |rows: &[BufferId]| MapStage {
            offset: (t * row) as i64,
            turn: Some(t),
            earlier: rows.last().copied(),
            tested: differences.clone(),
            ..MapStage::default()
        };
        let mut pages: Vec<BufferId> = Vec::with_capacity(differences.len());
        for (k, (x, y)) in differences.iter().enumerate() {
            let mut stage = place(&rows);
            let body = TensorBody::Sub(Box::new(section.term(&mut b, &mut stage, x, &pages)?), Box::new(section.term(&mut b, &mut stage, y, &pages)?));
            let name = format!("scan_turn{t}_difference{k}");
            let page = b.t.scratch_deep(&name, 1, tiles_of(row, target), acc);
            chain.append(&mut section.held);
            chain.extend(b.pointwise(&name, row, stage.slots, body, page, target)?);
            pages.push(page);
        }
        let mut stage = place(&rows);
        let body = section.term(&mut b, &mut stage, &body, &pages)?;
        let name = format!("scan_turn{t}");
        let page = b.t.scratch_deep(&name, 1, tiles_of(row, target), out_dt);
        chain.append(&mut section.held);
        chain.extend(b.pointwise(&name, row, stage.slots, body, page, target)?);
        rows.push(page);
    }
    // The rows side by side: band `t` holds row `t`, read from its first place.
    let slots: Vec<Slot> = rows.iter().enumerate().map(|(t, page)| Slot { buf: *page, map: at(-((t * row) as i64)) }).collect();
    let bands: Vec<(usize, TensorBody)> = (0..rows.len()).map(|t| ((t + 1) * row, TensorBody::Operand(t))).collect();
    let out_buf = b.result_page("out", n, out_dt, target);
    chain.extend(b.pointwise("scan_rows", n, slots, banded(bands), out_buf, target)?);
    let claim = if float { approx("recurrence", out_dt) } else { NumericsClaim::Exact };
    b.finish_fanned(&names.out, n, chain, out_buf, format!("scan_{}x{row}", spec.extents[0]), claim, out_dt, target)
}

/// `t` as `decay·carried + input`, both free of the carried value, when `t` is linear in it: the recurrence
/// `h_t = a_t·h_{t−1} + b_t` whose turns a chunked scan takes at once.
pub fn linear_in_earlier(
    t: &crate::tensor_recognize::map::Term,
) -> Option<(crate::tensor_recognize::map::Term, crate::tensor_recognize::map::Term)> {
    use crate::tensor_recognize::map::Term;
    let carries = |x: &Term| {
        let mut found = false;
        x.visit(&mut |y| found |= *y == Term::Earlier);
        found
    };
    let (zero, one) = (Term::Real(0.0), Term::Real(1.0));
    let bx = |x: &Term| Box::new(x.clone());
    if !carries(t) {
        return Some((zero, t.clone()));
    }
    Some(match t {
        Term::Earlier => (one, zero),
        Term::Add(x, y) | Term::Sub(x, y) => {
            let ((ax, bx_), (ay, by)) = (linear_in_earlier(x)?, linear_in_earlier(y)?);
            match t {
                Term::Add(..) => (Term::Add(Box::new(ax), Box::new(ay)), Term::Add(Box::new(bx_), Box::new(by))),
                _ => (Term::Sub(Box::new(ax), Box::new(ay)), Term::Sub(Box::new(bx_), Box::new(by))),
            }
        }
        Term::Mul(x, y) if !carries(x) => {
            let (ay, by) = linear_in_earlier(y)?;
            (Term::Mul(bx(x), Box::new(ay)), Term::Mul(bx(x), Box::new(by)))
        }
        Term::Mul(x, y) if !carries(y) => {
            let (ax, b_) = linear_in_earlier(x)?;
            (Term::Mul(Box::new(ax), bx(y)), Term::Mul(Box::new(b_), bx(y)))
        }
        Term::Div(x, y) if !carries(y) => {
            let (ax, b_) = linear_in_earlier(x)?;
            (Term::Div(Box::new(ax), bx(y)), Term::Div(Box::new(b_), bx(y)))
        }
        Term::Select { cond, then, otherwise } => {
            let mut in_cond = false;
            let probe = Term::Select { cond: cond.clone(), then: Box::new(Term::Int(0)), otherwise: Box::new(Term::Int(0)) };
            probe.visit(&mut |y| in_cond |= *y == Term::Earlier);
            if in_cond {
                return None;
            }
            let ((at, bt), (ao, bo)) = (linear_in_earlier(then)?, linear_in_earlier(otherwise)?);
            let select = |p: Term, q: Term| Term::Select { cond: cond.clone(), then: Box::new(p), otherwise: Box::new(q) };
            (select(at, ao), select(bt, bo))
        }
        _ => return None,
    })
}

/// A LINEAR recurrence the program wrote — `h_t = a_t·h_{t−1} + b_t` along its outer loop, every place of a row its
/// own channel — as ONE section that takes its turns a chunk at a time: the chunked scan.
///   · Every turn's `a_t` and `b_t` at once: the recurrence's arithmetic without its carry.
///   · Within each chunk of `L` turns, every chunk at once: `g`, the chunk's own recurrence from zero, and `p`, the running
///     product of its decays — `L − 1` stages, each advancing the turns `j` into every chunk.
///   · Across the chunks, in order: chunk `s`'s turns are `g + p·h`, `h` the last turn of the chunk before.
/// `L = ⌈√T⌉` turns a chunk: about `2√T` stages in order where the stepped form takes `T`.
pub fn chunked_map(
    spec: &crate::tensor_recognize::map::MapSpec,
    (decay, input): (&crate::tensor_recognize::map::Term, &crate::tensor_recognize::map::Term),
    names: &StageNames,
    reals: &[bool],
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (n, row, turns) = (spec.n(), spec.stride(), spec.extents[0]);
    if row_needs_blocking(n, target) {
        return Err(SpatialReject::NotLowered(
            "a recurrence holds every turn's row in one site, and this one's rows are wider together than a site's page",
        ));
    }
    let mut b = FlatBuild::new(target)?;
    let out_dt = b.precision;
    let acc = AccPolicy::for_result(b.precision).interm_precision();
    let mut section = MapSection {
        spec,
        names,
        reals,
        target,
        n,
        operands: vec![None; spec.ins.len()],
        positions: vec![None; spec.extents.len()],
        shared: Some(Vec::new()),
        held: Vec::new(),
    };
    let differences = differences_of(&spec.body);
    let mut chain: Vec<StageId> = Vec::new();
    let mut pages: Vec<BufferId> = Vec::with_capacity(differences.len());
    let page_of = |b: &mut FlatBuild, name: &str| b.t.scratch_deep(name, 1, tiles_of(n, target), acc);
    for (k, (x, y)) in differences.iter().enumerate() {
        let mut stage = MapStage { tested: differences.clone(), ..MapStage::default() };
        let body = TensorBody::Sub(Box::new(section.term(&mut b, &mut stage, x, &pages)?), Box::new(section.term(&mut b, &mut stage, y, &pages)?));
        let name = format!("scan_difference{k}");
        let page = page_of(&mut b, &name);
        chain.append(&mut section.held);
        chain.extend(b.pointwise(&name, n, stage.slots, body, page, target)?);
        pages.push(page);
    }
    let mut whole = |b: &mut FlatBuild, chain: &mut Vec<StageId>, name: &str, t: &crate::tensor_recognize::map::Term| -> Result<BufferId, SpatialReject> {
        let mut stage = MapStage { tested: differences.clone(), ..MapStage::default() };
        let body = section.term(b, &mut stage, t, &pages)?;
        let page = page_of(b, name);
        chain.append(&mut section.held);
        chain.extend(b.pointwise(name, n, stage.slots, body, page, target)?);
        Ok(page)
    };
    let a = whole(&mut b, &mut chain, "scan_decays", decay)?;
    let x = whole(&mut b, &mut chain, "scan_inputs", input)?;
    let chunk = ((turns as f64).sqrt().ceil() as usize).max(2);
    // Turn `r`'s row is band `r`: `then` on the turns `chosen` picks, the page as it stands elsewhere.
    let rows = |chosen: &dyn Fn(usize) -> Option<TensorBody>, keep: TensorBody| -> TensorBody {
        banded((0..turns).map(|r| ((r + 1) * row, chosen(r).unwrap_or_else(|| keep.clone()))).collect())
    };
    let (mut g, mut p) = (x, a);
    for j in 1..chunk.min(turns) {
        // Operand 0 the page as it stands, 1 the row before, 2 the decays.
        let advance = |page: BufferId| vec![Slot { buf: page, map: at(0) }, Slot { buf: page, map: at(-(row as i64)) }, Slot { buf: a, map: at(0) }];
        let sum = rows(&|r| (r % chunk == j).then(|| *add(mul(op(2), op(1)), op(0))), TensorBody::Operand(0));
        let next_g = page_of(&mut b, &format!("scan_chunk_sum{j}"));
        chain.extend(b.pointwise(&format!("scan_chunk_sum{j}"), n, advance(g), sum, next_g, target)?);
        let product = rows(&|r| (r % chunk == j).then(|| *mul(op(2), op(1))), TensorBody::Operand(0));
        let next_p = page_of(&mut b, &format!("scan_chunk_decay{j}"));
        chain.extend(b.pointwise(&format!("scan_chunk_decay{j}"), n, advance(p), product, next_p, target)?);
        (g, p) = (next_g, next_p);
    }
    let mut h = g;
    for s in 1..turns.div_ceil(chunk) {
        // Operand 0 the carried page as it stands, 1 the chunk's own sums, 2 its products, then the last turn of the chunk
        // before as each of the chunk's rows reads it.
        let mut slots = vec![Slot { buf: h, map: at(0) }, Slot { buf: g, map: at(0) }, Slot { buf: p, map: at(0) }];
        slots.extend((0..chunk).map(|i| Slot { buf: h, map: at(-(((i + 1) * row) as i64)) }));
        let carry = rows(&|r| (r / chunk == s).then(|| *add(op(1), mul(op(2), op(3 + r - s * chunk)))), TensorBody::Operand(0));
        let next = page_of(&mut b, &format!("scan_carry{s}"));
        chain.extend(b.pointwise(&format!("scan_carry{s}"), n, slots, carry, next, target)?);
        h = next;
    }
    let out_buf = b.result_page("out", n, out_dt, target);
    chain.extend(b.pointwise("scan_rows", n, vec![Slot { buf: h, map: at(0) }], TensorBody::Operand(0), out_buf, target)?);
    b.finish_fanned(&names.out, n, chain, out_buf, format!("scan_chunked_{turns}x{row}_by{chunk}"), approx("chunked recurrence", out_dt), out_dt, target)
}

/// `t` at the outer turn `turn` (from 1): the outer position a literal, and every branch decided by literals alone taken.
fn at_turn(t: &crate::tensor_recognize::map::Term, turn: usize) -> crate::tensor_recognize::map::Term {
    use crate::tensor_recognize::map::{Cmp, Cond, Term};
    fn int(t: &Term) -> Option<i64> {
        match t {
            Term::Int(v) => Some(*v),
            Term::Add(a, b) => int(a)?.checked_add(int(b)?),
            Term::Sub(a, b) => int(a)?.checked_sub(int(b)?),
            Term::Mul(a, b) => int(a)?.checked_mul(int(b)?),
            Term::Quot(a, b) => int(a)?.checked_div(int(b)?),
            Term::Rem(a, b) => int(a)?.checked_rem(int(b)?),
            _ => None,
        }
    }
    fn decided(c: &Cond) -> Option<bool> {
        match c {
            Cond::Cmp(op, a, b) => {
                let (a, b) = (int(a)?, int(b)?);
                Some(match op {
                    Cmp::Lt => a < b,
                    Cmp::LtEq => a <= b,
                    Cmp::Gt => a > b,
                    Cmp::GtEq => a >= b,
                    Cmp::Eq => a == b,
                    Cmp::NotEq => a != b,
                })
            }
            Cond::And(p, q) => match (decided(p), decided(q)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            Cond::Or(p, q) => match (decided(p), decided(q)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            Cond::Not(p) => decided(p).map(|v| !v),
        }
    }
    fn cond(c: &Cond, turn: usize) -> Cond {
        match c {
            Cond::Cmp(op, a, b) => Cond::Cmp(*op, Box::new(at_turn(a, turn)), Box::new(at_turn(b, turn))),
            Cond::And(p, q) => Cond::And(Box::new(cond(p, turn)), Box::new(cond(q, turn))),
            Cond::Or(p, q) => Cond::Or(Box::new(cond(p, turn)), Box::new(cond(q, turn))),
            Cond::Not(p) => Cond::Not(Box::new(cond(p, turn))),
        }
    }
    let r = |x: &Term| Box::new(at_turn(x, turn));
    match t {
        Term::Position(0) => Term::Int(turn as i64),
        Term::Select { cond: c, then, otherwise } => {
            let c = cond(c, turn);
            match decided(&c) {
                Some(true) => at_turn(then, turn),
                Some(false) => at_turn(otherwise, turn),
                None => Term::Select { cond: c, then: r(then), otherwise: r(otherwise) },
            }
        }
        Term::Add(a, b) => Term::Add(r(a), r(b)),
        Term::Sub(a, b) => Term::Sub(r(a), r(b)),
        Term::Mul(a, b) => Term::Mul(r(a), r(b)),
        Term::Div(a, b) => Term::Div(r(a), r(b)),
        Term::Quot(a, b) => Term::Quot(r(a), r(b)),
        Term::FloorDiv(a, b) => Term::FloorDiv(r(a), r(b)),
        Term::Rem(a, b) => Term::Rem(r(a), r(b)),
        Term::Pow(a, b) => Term::Pow(r(a), r(b)),
        Term::Call(f, args) => Term::Call(*f, args.iter().map(|x| at_turn(x, turn)).collect()),
        other => other.clone(),
    }
}

/// What a map's section declares once: each operand's tensor and element precision, and each loop's table of positions.
struct MapSection<'s> {
    spec: &'s crate::tensor_recognize::map::MapSpec,
    names: &'s StageNames,
    reals: &'s [bool],
    target: &'s SpatialTarget,
    n: usize,
    operands: Vec<Option<(TensorId, Precision, usize)>>,
    positions: Vec<Option<TensorId>>,
    /// The pages every stage of the section shares, an operand's each, where the section's pages feed every stage that
    /// reads them ([`FlatBuild::finish_fanned`]); `None` where each stage loads what it reads.
    shared: Option<Vec<(MapSource, BufferId)>>,
    /// The stages that copied a shared operand's load into its page, not yet in the chain: each goes before the first
    /// stage that reads it.
    held: Vec<StageId>,
}

/// What one stage of a map's section reads: a buffer each operand is loaded into for it, and a slot for each distinct read.
/// A stage of a recurrence computes one turn of the outer loop: its places start `offset` into every operand, its outer
/// position is the literal `turn + 1`, and the turn before it wrote `earlier`.
#[derive(Default)]
struct MapStage {
    slots: Vec<Slot>,
    buffers: Vec<(MapSource, BufferId)>,
    reads: Vec<(MapRead, usize)>,
    offset: i64,
    turn: Option<usize>,
    earlier: Option<BufferId>,
    /// The differences the section's pages hold, in page order ([`differences_of`]): what a branch tests is read off one.
    tested: Vec<(crate::tensor_recognize::map::Term, crate::tensor_recognize::map::Term)>,
}

/// A tensor a map's stage loads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MapSource {
    Operand(usize),
    Position(usize),
}

/// One read a map's stage makes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MapRead {
    Here(usize),
    Fixed(usize, usize),
    Position(usize),
    Difference(usize),
    Earlier,
}

/// Every difference a map's body tests — `a − b` for `a > b` and `a ≤ b`, `b − a` for `a < b` and `a ≥ b`, both for
/// `a = b` and `a ≠ b` — each after the differences its own terms test: each a stage of its own, which reads the pages
/// of those before it.
fn differences_of(body: &crate::tensor_recognize::map::Term) -> Vec<(crate::tensor_recognize::map::Term, crate::tensor_recognize::map::Term)> {
    use crate::tensor_recognize::map::{Cmp, Cond, Term};
    fn cond(c: &Cond, out: &mut Vec<(Term, Term)>) {
        match c {
            Cond::Cmp(op, a, b) => {
                term(a, out);
                term(b, out);
                let (ab, ba) = (((**a).clone(), (**b).clone()), ((**b).clone(), (**a).clone()));
                let used = match op {
                    Cmp::Gt | Cmp::LtEq => vec![ab],
                    Cmp::Lt | Cmp::GtEq => vec![ba],
                    Cmp::Eq | Cmp::NotEq => vec![ab, ba],
                };
                for d in used {
                    if !out.contains(&d) {
                        out.push(d);
                    }
                }
            }
            Cond::And(p, q) | Cond::Or(p, q) => {
                cond(p, out);
                cond(q, out);
            }
            Cond::Not(p) => cond(p, out),
        }
    }
    fn term(t: &Term, out: &mut Vec<(Term, Term)>) {
        if let Term::Select { cond: c, then, otherwise } = t {
            cond(c, out);
            term(then, out);
            term(otherwise, out);
            return;
        }
        match t {
            Term::Add(a, b)
            | Term::Sub(a, b)
            | Term::Mul(a, b)
            | Term::Div(a, b)
            | Term::Quot(a, b)
            | Term::FloorDiv(a, b)
            | Term::Rem(a, b)
            | Term::Pow(a, b) => {
                term(a, out);
                term(b, out);
            }
            Term::Call(_, args) => args.iter().for_each(|a| term(a, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    term(body, &mut out);
    out
}

impl MapSection<'_> {
    /// The slot through which `stage` reads `read`, loading what it reads from the first time it does.
    fn slot(&mut self, b: &mut FlatBuild, stage: &mut MapStage, read: MapRead, differences: &[BufferId]) -> Result<usize, SpatialReject> {
        if let Some((_, s)) = stage.reads.iter().find(|(r, _)| *r == read) {
            return Ok(*s);
        }
        let (source, map) = match read {
            MapRead::Here(k) => (Some(MapSource::Operand(k)), at(stage.offset)),
            MapRead::Fixed(k, i) => (Some(MapSource::Operand(k)), fixed(i as i64)),
            MapRead::Position(k) => (Some(MapSource::Position(k)), at(stage.offset)),
            MapRead::Difference(_) | MapRead::Earlier => (None, at(0)),
        };
        let buf = match (source, read) {
            (None, MapRead::Difference(d)) => differences[d],
            (None, MapRead::Earlier) => stage.earlier.ok_or(SpatialReject::NotLowered("a carried value read on the first turn"))?,
            (Some(src), _) => {
                let loaded = match &self.shared {
                    Some(shared) => shared.iter().find(|(s, _)| *s == src).map(|(_, buf)| *buf),
                    None => stage.buffers.iter().find(|(s, _)| *s == src).map(|(_, buf)| *buf),
                };
                match loaded {
                    Some(buf) => buf,
                    None => {
                        let buf = self.load(b, src)?;
                        match &mut self.shared {
                            Some(shared) => shared.push((src, buf)),
                            None => stage.buffers.push((src, buf)),
                        }
                        buf
                    }
                }
            }
            _ => unreachable!("a read with no source is a difference"),
        };
        stage.slots.push(Slot { buf, map });
        stage.reads.push((read, stage.slots.len() - 1));
        Ok(stage.slots.len() - 1)
    }

    /// A buffer `src` is loaded into for one stage: the operand's tensor or the loop's table of positions, declared the
    /// first time a stage loads it.
    fn load(&mut self, b: &mut FlatBuild, src: MapSource) -> Result<BufferId, SpatialReject> {
        let (id, precision, len, name) = match src {
            MapSource::Operand(k) => {
                let (name, fill) = self.names.operand(k);
                if self.operands[k].is_none() {
                    let len = self.spec.lens[k];
                    let precision = table_precision(b, self.reals[k], fill, self.target)?;
                    let id = b.t.tensor(name, TensorRole::Input, 1, len, precision, crate::spatial::passes::from_tensor_op::init_of(fill, len));
                    self.operands[k] = Some((id, precision, len));
                }
                let (id, precision, len) = self.operands[k].expect("declared above");
                (id, precision, len, name.to_string())
            }
            MapSource::Position(k) => {
                let ix = index_type(self.target, self.spec.extents[k] as u64 + 1)?;
                if self.positions[k].is_none() {
                    let inner: usize = self.spec.extents[k + 1..].iter().product();
                    let values: Vec<i64> = (0..self.n).map(|p| (p / inner % self.spec.extents[k]) as i64 + 1).collect();
                    let name = format!("{}__position{k}", self.names.out);
                    self.positions[k] = Some(b.t.tensor(&name, TensorRole::Param, 1, self.n, ix, TensorInit::Values(values)));
                }
                (self.positions[k].expect("declared above"), ix, self.n, format!("{}__position{k}", self.names.out))
            }
        };
        let tiles = tiles_of(len, self.target);
        let depth = if self.shared.is_some() { 1 } else { 2 };
        let buf = b.t.buffer_deep(&format!("in_{name}"), depth, tiles, precision);
        let l = b.t.load(&format!("load_{name}"), id, vec![], tiles, buf);
        b.loads.push((l, buf));
        if self.shared.is_none() {
            return Ok(buf);
        }
        // A load hands its page to one stage: a page every stage of the section reads is the load's, copied once.
        let page = b.t.scratch_deep(&format!("{name}_held"), 1, tiles, precision);
        let held = b.compute(&format!("hold_{name}"), flat_space(len), vec![Slot { buf, map: at(0) }], ident_map(wide(tiles, self.target)), TensorBody::Operand(0), page);
        self.held.push(held);
        Ok(page)
    }

    /// The body `t` is in `stage`, whose branches read the differences they test from `differences`, the pages of
    /// `stage.tested` in order.
    fn term(
        &mut self,
        b: &mut FlatBuild,
        stage: &mut MapStage,
        t: &crate::tensor_recognize::map::Term,
        differences: &[BufferId],
    ) -> Result<TensorBody, SpatialReject> {
        use crate::tensor_recognize::map::{Fun, Term};
        let sub_term = |this: &mut Self, b: &mut FlatBuild, stage: &mut MapStage, x: &Term| this.term(b, stage, x, differences);
        Ok(match t {
            Term::Here(k) => TensorBody::Operand(self.slot(b, stage, MapRead::Here(*k), differences)?),
            Term::Fixed(k, i) => TensorBody::Operand(self.slot(b, stage, MapRead::Fixed(*k, *i), differences)?),
            Term::Position(0) if stage.turn.is_some() => *konst(stage.turn.expect("a turn") as f64 + 1.0),
            Term::Position(k) => TensorBody::Operand(self.slot(b, stage, MapRead::Position(*k), differences)?),
            // The first turn takes no arm that carries ([`MapSpec::recurs`]'s guard): what it would read is never used.
            Term::Earlier if stage.earlier.is_none() => *konst(0.0),
            Term::Earlier => TensorBody::Operand(self.slot(b, stage, MapRead::Earlier, differences)?),
            Term::Int(v) => *konst(*v as f64),
            Term::Real(v) => *konst(*v),
            Term::Add(x, y) => TensorBody::Add(Box::new(sub_term(self, b, stage, x)?), Box::new(sub_term(self, b, stage, y)?)),
            Term::Sub(x, y) => TensorBody::Sub(Box::new(sub_term(self, b, stage, x)?), Box::new(sub_term(self, b, stage, y)?)),
            Term::Mul(x, y) => TensorBody::Mul(Box::new(sub_term(self, b, stage, x)?), Box::new(sub_term(self, b, stage, y)?)),
            Term::Div(x, y) => *mul(Box::new(sub_term(self, b, stage, x)?), unary(SfpuKind::Recip, Box::new(sub_term(self, b, stage, y)?))),
            Term::FloorDiv(x, y) => TensorBody::FloorDiv(Box::new(sub_term(self, b, stage, x)?), Box::new(sub_term(self, b, stage, y)?)),
            Term::Quot(x, y) => truncated(sub_term(self, b, stage, x)?, sub_term(self, b, stage, y)?),
            Term::Rem(x, y) => {
                let (x, y) = (sub_term(self, b, stage, x)?, sub_term(self, b, stage, y)?);
                let q = truncated(x.clone(), y.clone());
                TensorBody::Sub(Box::new(x), Box::new(TensorBody::Mul(Box::new(y), Box::new(q))))
            }
            Term::Pow(base, exponent) => self.power(b, stage, base, exponent, differences)?,
            Term::Call(f, args) => {
                let mut a: Vec<TensorBody> = Vec::with_capacity(args.len());
                for x in args {
                    a.push(sub_term(self, b, stage, x)?);
                }
                match (f, a.as_slice()) {
                    (Fun::Exp, [x]) => *unary(SfpuKind::Exp, Box::new(x.clone())),
                    (Fun::Ln, [x]) => *unary(SfpuKind::Ln, Box::new(x.clone())),
                    (Fun::Sqrt, [x]) => *unary(SfpuKind::Recip, unary(SfpuKind::Rsqrt { eps_nano: 0, over: 1 }, Box::new(x.clone()))),
                    (Fun::Tanh, [x]) => {
                        let e2x = unary(SfpuKind::Exp, mul(konst(2.0), Box::new(x.clone())));
                        *sub(konst(1.0), mul(konst(2.0), unary(SfpuKind::Recip, add(e2x, konst(1.0)))))
                    }
                    (Fun::Abs, [x]) => TensorBody::Max(Box::new(x.clone()), mul(konst(-1.0), Box::new(x.clone()))),
                    (Fun::Min, [x, y]) => TensorBody::Min(Box::new(x.clone()), Box::new(y.clone())),
                    (Fun::Max, [x, y]) => TensorBody::Max(Box::new(x.clone()), Box::new(y.clone())),
                    _ => return Err(SpatialReject::NotLowered("a library call with the wrong number of arguments")),
                }
            }
            Term::Select { cond, then, otherwise } => {
                let (then, otherwise) = (sub_term(self, b, stage, then)?, sub_term(self, b, stage, otherwise)?);
                self.select(b, stage, cond, then, otherwise, differences)?
            }
        })
    }

    /// `base ** exponent` as the datapath computes it: a product for a whole literal exponent, a root for one half, and
    /// `e^(x·ln b)` for a positive constant base.
    fn power(
        &mut self,
        b: &mut FlatBuild,
        stage: &mut MapStage,
        base: &crate::tensor_recognize::map::Term,
        exponent: &crate::tensor_recognize::map::Term,
        differences: &[BufferId],
    ) -> Result<TensorBody, SpatialReject> {
        use crate::tensor_recognize::map::Term;
        let constant = match base {
            Term::Real(v) => Some(*v),
            Term::Int(v) => Some(*v as f64),
            _ => None,
        };
        match (constant, exponent) {
            (_, Term::Int(k)) if *k >= 0 => {
                let x = self.term(b, stage, base, differences)?;
                Ok((0..*k).fold(*konst(1.0), |acc, _| TensorBody::Mul(Box::new(acc), Box::new(x.clone()))))
            }
            (_, Term::Int(k)) if self.real_term(base) => {
                let x = self.term(b, stage, base, differences)?;
                let up = (0..-*k).fold(*konst(1.0), |acc, _| TensorBody::Mul(Box::new(acc), Box::new(x.clone())));
                Ok(*unary(SfpuKind::Recip, Box::new(up)))
            }
            (_, Term::Real(h)) if *h == 0.5 => {
                let x = self.term(b, stage, base, differences)?;
                Ok(*unary(SfpuKind::Recip, unary(SfpuKind::Rsqrt { eps_nano: 0, over: 1 }, Box::new(x))))
            }
            (Some(c), _) if c > 0.0 => {
                let x = self.term(b, stage, exponent, differences)?;
                Ok(*unary(SfpuKind::Exp, mul(Box::new(x), konst(c.ln()))))
            }
            _ => Err(SpatialReject::NotLowered(
                "a power whose exponent is neither a whole literal nor one half and whose base is not a positive constant",
            )),
        }
    }

    /// Whether `t` computes a Real: it reads one, or divides, or takes a transform.
    fn real_term(&self, t: &crate::tensor_recognize::map::Term) -> bool {
        use crate::tensor_recognize::map::{Fun, Term};
        let mut real = false;
        t.visit(&mut |x| {
            real |= match x {
                Term::Here(k) | Term::Fixed(k, _) => self.reals[*k],
                Term::Real(_) | Term::Div(..) => true,
                Term::Call(f, _) => matches!(f, Fun::Exp | Fun::Ln | Fun::Tanh | Fun::Sqrt),
                _ => false,
            }
        });
        real
    }

    /// `cond ? then : otherwise`, each comparison read off its difference's page against the flat space's row 0.
    #[allow(clippy::too_many_arguments)]
    fn select(
        &mut self,
        b: &mut FlatBuild,
        stage: &mut MapStage,
        cond: &crate::tensor_recognize::map::Cond,
        then: TensorBody,
        otherwise: TensorBody,
        differences: &[BufferId],
    ) -> Result<TensorBody, SpatialReject> {
        use crate::tensor_recognize::map::{Cmp, Cond};
        let all = stage.tested.clone();
        let page = |x: &crate::tensor_recognize::map::Term, y: &crate::tensor_recognize::map::Term| {
            all.iter().position(|(a, c)| a == x && c == y).expect("every difference has its stage")
        };
        Ok(match cond {
            Cond::Cmp(op, x, y) => {
                // `0 < d` (the row is below `d`) and `0 ≥ d`, for `d` the page of `a − b` or of `b − a`.
                let below = |this: &mut Self, b: &mut FlatBuild, stage: &mut MapStage, d: usize| -> Result<Pred, SpatialReject> {
                    Ok(Pred::AxisLt(LV_GEN_ROW, PredRhs::Operand(this.slot(b, stage, MapRead::Difference(d), differences)?)))
                };
                let at_most = |this: &mut Self, b: &mut FlatBuild, stage: &mut MapStage, d: usize| -> Result<Pred, SpatialReject> {
                    Ok(Pred::AxisGe(LV_GEN_ROW, PredRhs::Operand(this.slot(b, stage, MapRead::Difference(d), differences)?)))
                };
                let one = |cond: Pred, then: TensorBody, otherwise: TensorBody| TensorBody::Select { cond, then: Box::new(then), otherwise: Box::new(otherwise) };
                match op {
                    Cmp::Gt => one(below(self, b, stage, page(x, y))?, then, otherwise),
                    Cmp::Lt => one(below(self, b, stage, page(y, x))?, then, otherwise),
                    Cmp::LtEq => one(at_most(self, b, stage, page(x, y))?, then, otherwise),
                    Cmp::GtEq => one(at_most(self, b, stage, page(y, x))?, then, otherwise),
                    Cmp::Eq | Cmp::NotEq => {
                        let (eq, ne) = if *op == Cmp::Eq { (then, otherwise) } else { (otherwise, then) };
                        let inner = one(at_most(self, b, stage, page(y, x))?, eq, ne.clone());
                        one(at_most(self, b, stage, page(x, y))?, inner, ne)
                    }
                }
            }
            Cond::And(p, q) => {
                let inner = self.select(b, stage, q, then, otherwise.clone(), differences)?;
                self.select(b, stage, p, inner, otherwise, differences)?
            }
            Cond::Or(p, q) => {
                let inner = self.select(b, stage, q, then.clone(), otherwise, differences)?;
                self.select(b, stage, p, then, inner, differences)?
            }
            Cond::Not(p) => self.select(b, stage, p, otherwise, then, differences)?,
        })
    }
}

/// `x / y` for Ints, truncated toward zero, in integer arithmetic: the floored quotients of the magnitudes, signed by the
/// divisor — `sign(y) · (⌊max(x, 0) / |y|⌋ − ⌊max(−x, 0) / |y|⌋)`. A zero divisor faults, as the VM's does.
fn truncated(x: TensorBody, y: TensorBody) -> TensorBody {
    let abs_y = TensorBody::Max(Box::new(y.clone()), mul(konst(-1.0), Box::new(y.clone())));
    let sign_y = TensorBody::Min(Box::new(TensorBody::Max(Box::new(y), konst(-1.0))), konst(1.0));
    let up = TensorBody::FloorDiv(Box::new(TensorBody::Max(Box::new(x.clone()), konst(0.0))), Box::new(abs_y.clone()));
    let down = TensorBody::FloorDiv(Box::new(TensorBody::Max(mul(konst(-1.0), Box::new(x)), konst(0.0))), Box::new(abs_y));
    TensorBody::Mul(Box::new(sign_y), Box::new(TensorBody::Sub(Box::new(up), Box::new(down))))
}
