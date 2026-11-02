//! The canonical SHAPE of a section — its choreography with every name forgotten.
//!
//! ★ WHY A SHAPE IS NOT A SECTION. Sixty-four decoder layers are the same handful of dataflows run over
//! and over against different weights. The compiler proves that today by unrolling: four real-width
//! layers compose into 153 sections, and sixty-four would be hundreds of gigabytes of IR describing a
//! program no SN50 could hold anyway — 432 MB of SRAM against 27 GB of weights means the chip runs ONE
//! layer's dataflow and streams the layers through it.
//!
//! Turning that unrolled form into a loop needs the regularity to be a CHECKED FACT, not an assumption
//! about how the model was authored. A shape is that fact: a section reduced to its op sequence with
//! every buffer, semaphore, group, site and engine renumbered by first appearance, and every TENSOR
//! forgotten. Two sections share a shape exactly when their event graphs are isomorphic — which is the
//! precise sense in which layer 7 and layer 8 "are the same".
//!
//! What a shape deliberately keeps: loop counts, buffer depths and page sizes, semaphore roles, access
//! patterns, and the compute bodies. Two sections that differ in any of those run different dataflows,
//! however alike they look in the source.

use std::collections::BTreeMap;

use super::super::ir::*;

/// FNV-1a over a canonical byte stream. The digest is a function of the program, never of an address.
struct Shape(u64);

impl Shape {
    fn new() -> Shape {
        Shape(0xcbf29ce484222325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= u64::from(*x);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn tag(&mut self, s: &str) {
        self.bytes(s.as_bytes());
        self.bytes(&[0xff]);
    }
    fn num(&mut self, n: usize) {
        self.bytes(&(n as u64).to_le_bytes());
    }
    fn json<T: serde::Serialize>(&mut self, v: &T) {
        self.bytes(serde_json::to_string(v).unwrap_or_default().as_bytes());
        self.bytes(&[0xfe]);
    }
}

/// Ids renumbered by first appearance, so two isomorphic sections canonicalize identically.
#[derive(Default)]
struct Canon {
    site: BTreeMap<(u16, u16, u16), usize>,
    engine: BTreeMap<String, usize>,
    buf: BTreeMap<usize, usize>,
    sem: BTreeMap<usize, usize>,
    grp: BTreeMap<usize, usize>,
}

impl Canon {
    fn of<K: Ord>(m: &mut BTreeMap<K, usize>, k: K) -> usize {
        let n = m.len();
        *m.entry(k).or_insert(n)
    }
    fn site(&mut self, s: Site) -> usize {
        Canon::of(&mut self.site, (s.chip, s.y, s.x))
    }
    fn engine(&mut self, e: Engine) -> usize {
        Canon::of(&mut self.engine, format!("{e:?}"))
    }
    fn buf(&mut self, b: BufferId) -> usize {
        Canon::of(&mut self.buf, b.idx())
    }
    fn sem(&mut self, s: SemId) -> usize {
        Canon::of(&mut self.sem, s.idx())
    }
    fn grp(&mut self, g: GroupId) -> usize {
        Canon::of(&mut self.grp, g.idx())
    }
}

/// Fold one op into the shape, renumbering every id it names. A TENSOR is never folded in: that is the
/// one thing a shape forgets, and the reason two layers of the same kind share one.
fn fold_op(p: &SpatialProgram, c: &mut Canon, h: &mut Shape, op: &ChoreoOp) {
    // A buffer's own geometry is part of the dataflow, so it travels with the renumbered id.
    let buf = |c: &mut Canon, h: &mut Shape, b: BufferId| {
        h.num(c.buf(b));
        if let Some(bf) = p.buffers.get(b.idx()) {
            h.json(&bf.kind);
            h.json(&bf.dtype);
            h.json(&bf.space);
        }
    };
    let sem = |c: &mut Canon, h: &mut Shape, s: SemId| {
        h.num(c.sem(s));
        if let Some(sm) = p.semaphores.get(s.idx()) {
            h.json(&sm.role);
            h.num(sm.init as usize);
        }
    };
    match op {
        ChoreoOp::Reserve { buf: b, pages } => {
            h.tag("reserve");
            buf(c, h, *b);
            h.num(*pages);
        }
        ChoreoOp::Push { buf: b, pages } => {
            h.tag("push");
            buf(c, h, *b);
            h.num(*pages);
        }
        ChoreoOp::Wait { buf: b, pages } => {
            h.tag("wait");
            buf(c, h, *b);
            h.num(*pages);
        }
        ChoreoOp::Pop { buf: b, pages } => {
            h.tag("pop");
            buf(c, h, *b);
            h.num(*pages);
        }
        ChoreoOp::ReadTiles {
            at,
            block_h,
            block_w,
            stride_w,
            stride_h,
            into,
            ..
        } => {
            h.tag("read");
            h.json(at);
            h.num(*block_h);
            h.num(*block_w);
            h.num(*stride_w);
            h.num(*stride_h);
            buf(c, h, *into);
        }
        ChoreoOp::WriteTiles {
            from,
            at,
            block_h,
            block_w,
            stride_w,
            stride_h,
            ..
        } => {
            h.tag("write");
            buf(c, h, *from);
            h.json(at);
            h.num(*block_h);
            h.num(*block_w);
            h.num(*stride_w);
            h.num(*stride_h);
        }
        ChoreoOp::ReadBarrier => h.tag("rbar"),
        ChoreoOp::WriteBarrier => h.tag("wbar"),
        ChoreoOp::McastSend {
            buf: b,
            pages,
            group,
            hold_path,
        } => {
            h.tag("mcast");
            buf(c, h, *b);
            h.num(*pages);
            h.num(c.grp(*group));
            h.num(usize::from(*hold_path));
        }
        ChoreoOp::McastSetValid { sem: s, group } => {
            h.tag("mvalid");
            sem(c, h, *s);
            h.num(c.grp(*group));
        }
        ChoreoOp::Flush { cond } => {
            h.tag("flush");
            h.json(cond);
        }
        ChoreoOp::SemSet { sem: s, value } => {
            h.tag("semset");
            sem(c, h, *s);
            h.json(value);
        }
        ChoreoOp::SemInc { sem: s, at, by } => {
            h.tag("seminc");
            sem(c, h, *s);
            h.num(c.site(*at));
            h.num(*by);
        }
        ChoreoOp::SemWait { sem: s, until } => {
            h.tag("semwait");
            sem(c, h, *s);
            h.json(until);
        }
        ChoreoOp::AcquireAcc { tiles } => {
            h.tag("acqacc");
            h.num(*tiles);
        }
        ChoreoOp::ReleaseAcc => h.tag("relacc"),
        ChoreoOp::Compute {
            op,
            ins,
            acc_reload,
        } => {
            h.tag("compute");
            h.json(op);
            h.num(ins.len());
            for b in ins {
                buf(c, h, *b);
            }
            match acc_reload {
                Some(b) => buf(c, h, *b),
                None => h.tag("noreload"),
            }
        }
        ChoreoOp::PackAcc {
            to,
            tiles,
            convert,
            accumulate,
        } => {
            h.tag("packacc");
            buf(c, h, *to);
            h.num(*tiles);
            h.json(convert);
            h.num(usize::from(*accumulate));
        }
        ChoreoOp::Barrier { group } => {
            h.tag("barrier");
            h.num(c.grp(*group));
        }
        ChoreoOp::Loop { var, count, body } => {
            h.tag("loop");
            h.json(var);
            // A trip keeps its arithmetic and forgets the tensor it reads, like every other tensor here.
            match count {
                LoopCount::Const(n) => h.num(*n),
                LoopCount::Trip { trip, max } => {
                    let t = p.trip(*trip);
                    h.tag("trip");
                    h.num(*max);
                    h.num(t.elem);
                    h.bytes(&t.admit.0.to_le_bytes());
                    h.bytes(&t.admit.1.to_le_bytes());
                    h.bytes(&t.bias.to_le_bytes());
                    h.num(t.per);
                }
            }
            h.num(body.len());
            for t in body {
                fold_op(p, c, h, &t.op);
            }
            h.tag("endloop");
        }
    }
}

/// The canonical shape of every section, in section order.
///
/// A program with no sections has no shapes — the function is total and reports nothing rather than
/// treating a single-section program as a degenerate repetition of itself.
pub fn section_shapes(p: &SpatialProgram) -> Vec<u64> {
    let Some(sched) = p.schedule.as_ref() else {
        return Vec::new();
    };
    // ★ ONE WALK OVER THE SCHEDULE, NOT ONE PER SECTION.
    //
    // Filtering every engine program's whole op list once per section is quadratic in the pair, and the
    // pair is not small: a real-width eight-layer forward is 301 sections over 178,436,487 top-level ops
    // across 1,088 engine programs, so the naive form is ~5e10 comparisons and never returns. Stage ids
    // already say which section an op belongs to, so bucketing the ops ONCE costs one pass and makes the
    // whole analysis linear. (The same shape of defect as the `validate_schedule` walk that cost 16
    // minutes and 13 GB before it was folded into a single pass.)
    let mut sec_of_stage = vec![usize::MAX; p.stages.len()];
    for (si, sp) in p.sections.iter().enumerate() {
        for slot in sec_of_stage
            .iter_mut()
            .take(sp.stages.1.min(p.stages.len()))
            .skip(sp.stages.0)
        {
            *slot = si;
        }
    }
    // `buckets[section][program]` = that program's ops belonging to that section, in program order.
    let np = sched.programs.len();
    let mut buckets: Vec<Vec<Vec<&TaggedOp>>> = vec![Vec::new(); p.sections.len()];
    for b in buckets.iter_mut() {
        b.resize_with(np, Vec::new);
    }
    for (pi, prog) in sched.programs.iter().enumerate() {
        for t in &prog.ops {
            let si = sec_of_stage
                .get(t.stage.idx())
                .copied()
                .unwrap_or(usize::MAX);
            if let Some(rows) = buckets.get_mut(si) {
                rows[pi].push(t);
            }
        }
    }
    p.sections
        .iter()
        .enumerate()
        .map(|(si, sp)| {
            let mut c = Canon::default();
            let mut h = Shape::new();
            h.num(sp.k_tiles);
            h.json(&sp.plan);
            h.num(sp.items.1.saturating_sub(sp.items.0));
            for (pi, prog) in sched.programs.iter().enumerate() {
                let ops = &buckets[si][pi];
                if ops.is_empty() {
                    continue;
                }
                h.tag("prog");
                h.num(c.site(prog.site));
                h.num(c.engine(prog.engine));
                h.num(ops.len());
                for t in ops {
                    fold_op(p, &mut c, &mut h, &t.op);
                }
            }
            h.0
        })
        .collect()
}

/// Sections grouped by shape: `(shape, the sections that have it)`, ordered by each group's first
/// section so the answer is stable. The groups partition the sections.
pub fn section_repeat_groups(p: &SpatialProgram) -> Vec<(u64, Vec<usize>)> {
    let mut by: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    for (i, s) in section_shapes(p).into_iter().enumerate() {
        by.entry(s).or_default().push(i);
    }
    let mut v: Vec<(u64, Vec<usize>)> = by.into_iter().collect();
    v.sort_by_key(|(_, m)| m.first().copied().unwrap_or(usize::MAX));
    v
}
