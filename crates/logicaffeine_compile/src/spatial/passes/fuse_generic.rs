//! `fuse_generic` — put back into one register window what legalization had to take apart.
//!
//! ★ THE POINT OF SPLITTING FIRST AND FUSING AFTER. A fused operator written by hand is a claim
//! nobody can check: it is one opaque kernel, and whether it computes the composition it is named
//! after is a matter of reading it carefully. Legalizing into stages and then fusing them makes the
//! same kernel a REWRITE — and a rewrite has a before and an after that `equivalent_under`
//! (`spatial/equiv.rs`) can evaluate against each other. The performance is identical; the
//! difference is that one of them is checkable.
//!
//! What this fuses: two adjacent `ComputeOp::Generic` stages where the second consumes the first's
//! only output, both walk the same fully-parallel iteration space, and the composition is one the
//! destination registers can carry — the outer operation's second operand must be a plain tile,
//! because the broadcast instruction family (`mul_tiles_bcast_cols` and friends) reads two circular
//! buffers and has no dest-reuse spelling. Everything else is left alone rather than guessed at.
//!
//! It runs at `Phase::Template`, before `tile` and `place`, so removing a stage costs only a dense
//! renumber of stages/buffers/edges — there is no placement, memory map or schedule to keep in step.

use super::SpatialReject;
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::target::SpatialTarget;

/// Substitute `inner` for `Operand(slot)` inside `body`, shifting every other operand index by the
/// map `shift` gives it.
fn splice(
    body: &TensorBody,
    slot: usize,
    inner: &TensorBody,
    shift: &dyn Fn(usize) -> usize,
) -> TensorBody {
    match body {
        TensorBody::Operand(i) if *i == slot => inner.clone(),
        TensorBody::Operand(i) => TensorBody::Operand(shift(*i)),
        TensorBody::Const(c) => TensorBody::Const(*c),
        TensorBody::Add(a, b) => TensorBody::Add(
            Box::new(splice(a, slot, inner, shift)),
            Box::new(splice(b, slot, inner, shift)),
        ),
        TensorBody::Sub(a, b) => TensorBody::Sub(
            Box::new(splice(a, slot, inner, shift)),
            Box::new(splice(b, slot, inner, shift)),
        ),
        TensorBody::Mul(a, b) => TensorBody::Mul(
            Box::new(splice(a, slot, inner, shift)),
            Box::new(splice(b, slot, inner, shift)),
        ),
        TensorBody::Min(a, b) => TensorBody::Min(
            Box::new(splice(a, slot, inner, shift)),
            Box::new(splice(b, slot, inner, shift)),
        ),
        TensorBody::Max(a, b) => TensorBody::Max(
            Box::new(splice(a, slot, inner, shift)),
            Box::new(splice(b, slot, inner, shift)),
        ),
        TensorBody::FloorDiv(a, b) => TensorBody::FloorDiv(
            Box::new(splice(a, slot, inner, shift)),
            Box::new(splice(b, slot, inner, shift)),
        ),
        TensorBody::Shr { arg, by } => TensorBody::Shr {
            arg: Box::new(splice(arg, slot, inner, shift)),
            by: *by,
        },
        TensorBody::Unary { kind, arg } => TensorBody::Unary {
            kind: *kind,
            arg: Box::new(splice(arg, slot, inner, shift)),
        },
        // A predicate's operand-bound is re-indexed like any operand; it is never the spliced slot
        // (`fuse_once` declines that case — a bound is an index into the operand list, not a body).
        TensorBody::Select {
            cond,
            then,
            otherwise,
        } => TensorBody::Select {
            cond: cond.map_operand(shift),
            then: Box::new(splice(then, slot, inner, shift)),
            otherwise: Box::new(splice(otherwise, slot, inner, shift)),
        },
    }
}

/// Re-index a body's operands through `shift` (used for the inner body, whose operands move to the
/// front of the fused operand list).
fn reindex(body: &TensorBody, shift: &dyn Fn(usize) -> usize) -> TensorBody {
    splice(body, usize::MAX, &TensorBody::Const(0), shift)
}

/// A map as its points see it on `iters`: an axis of extent 1 is always 0, so a term along it reads nothing, and two
/// maps that differ only there read the same element at every point.
fn on_space(map: &ElemAccess, iters: &[IterSpec]) -> ElemAccess {
    let mut terms: Vec<(LoopVar, i64)> = map
        .terms
        .iter()
        .copied()
        .filter(|(v, stride)| *stride != 0 && iters.iter().any(|ax| ax.var == *v && ax.extent > 1))
        .collect();
    terms.sort();
    ElemAccess { terms, ..map.clone() }
}

/// All-parallel and the same extents — the precondition for two stages to share a window at all.
fn same_parallel_space(a: &[IterSpec], b: &[IterSpec]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            x.var == y.var
                && x.extent == y.extent
                && matches!(x.kind, IterKind::Parallel)
                && matches!(y.kind, IterKind::Parallel)
        })
}

/// One fusion, or `None` when nothing in the program qualifies.
fn fuse_once(p: &SpatialProgram) -> Option<SpatialProgram> {
    for (bi, b) in p.stages.iter().enumerate() {
        let StageKind::Compute(ComputeOp::Generic {
            iters: bit,
            ins: bmaps,
            out: bout,
            body: bbody,
        }) = &b.kind
        else {
            continue;
        };
        for (slot, feed) in b.ins.iter().enumerate() {
            // The producer must be a Generic whose ONLY consumer is this stage.
            let Some(ai) = p.stages.iter().position(|s| {
                s.outs.first() == Some(feed)
                    && matches!(s.kind, StageKind::Compute(ComputeOp::Generic { .. }))
            }) else {
                continue;
            };
            if ai == bi {
                continue;
            }
            if p.stages
                .iter()
                .enumerate()
                .any(|(i, s)| i != bi && s.ins.contains(feed))
            {
                continue; // the intermediate is read elsewhere; fusing would delete a value someone needs
            }
            // ★ ONE READ OF THE INTERMEDIATE, OR NO FUSION. The splice replaces one slot's read; a consumer that reads the
            // intermediate in another slot too (a rotation reads each element and its partner across the half of its
            // head) would be left reading a buffer the fusion deletes.
            if b.ins.iter().enumerate().any(|(i, x)| i != slot && x == feed) {
                continue;
            }
            let a = &p.stages[ai];
            let StageKind::Compute(ComputeOp::Generic {
                iters: ait,
                ins: amaps,
                out: aout,
                body: abody,
            }) = &a.kind
            else {
                continue;
            };
            if !same_parallel_space(ait, bit) {
                continue;
            }
            // ★ A FOLD KEEPS WHAT THE PAGE BETWEEN KEPT. The producer packs the intermediate into its page at the page's
            // width and the consumer reads that back; the spliced body carries it in the destination registers instead.
            // A bf16 or fp16 page rounds what a register holds, so across one the two computes stay two, the page between
            // them rounding as the program apart rounds.
            if matches!(p.buffer(*feed).dtype, Precision::Bf16 | Precision::Fp16 | Precision::Fp8) {
                continue;
            }
            // ★ THE CONSUMER MUST READ THE INTERMEDIATE WHERE IT WAS WRITTEN. Splicing the producer's body
            // into the slot replaces the READ, so the consumer's map for that slot is discarded — which is
            // only the same computation when that map is the producer's own output map. A shifted read
            // (`at(-1)`, a band's base) would pick up the value at the WRONG point, and an indirect term
            // is not an address at all but a check: a stride-0 gather is how a row write proves its row
            // is in range, and splicing it away deleted the proof (an out-of-range `Set row` wrote nothing
            // and reported nothing).
            if on_space(&bmaps[slot], bit) != on_space(aout, ait) || bmaps[slot].indirect.is_some() {
                continue;
            }
            // A predicate whose BOUND is the intermediate cannot be spliced: a bound is an operand
            // index, and the producer's body is an expression, not an operand.
            if bbody.pred_names_operand(slot) {
                continue;
            }
            // The consumer's OTHER operands must be plain tiles: a broadcast has no dest-reuse form.
            if bmaps
                .iter()
                .enumerate()
                .any(|(i, m)| i != slot && bit.iter().any(|ax| ax.extent > 1 && !m.varies_along(ax.var)))
            {
                continue;
            }
            // Operand list: the producer's operands first, then the consumer's others in order.
            let mut ins: Vec<BufferId> = a.ins.clone();
            let mut maps: Vec<ElemAccess> = amaps.clone();
            let mut shift = vec![usize::MAX; b.ins.len()];
            for (i, (buf, m)) in b.ins.iter().zip(bmaps).enumerate() {
                if i == slot {
                    continue;
                }
                shift[i] = ins.len();
                ins.push(*buf);
                maps.push(m.clone());
            }
            // ★ THE COMPOSED SIDE MUST END UP ON THE LEFT. A fused body is emitted by leaving the
            // inner result in the destination registers and having the outer operation consume DST
            // via its dest-reuse form — and dest-reuse takes the OTHER operand from a circular
            // buffer. So `f(x) ⊕ b` is emittable and `b ⊕ f(x)` is not. Substituting blindly produced
            // the second shape (fusing `rsqrt` into `rescale` gave `x · rsqrt(...)`), which lowered to
            // a refusal the printer was right to make and this pass was wrong to cause. Addition and
            // multiplication commute, so the operands are swapped; subtraction does not, so it is
            // left un-fused rather than reassociated into something that computes a different thing.
            let bbody = match bbody {
                TensorBody::Add(x, y) if matches!(**y, TensorBody::Operand(i) if i == slot) => {
                    TensorBody::Add(y.clone(), x.clone())
                }
                TensorBody::Mul(x, y) if matches!(**y, TensorBody::Operand(i) if i == slot) => {
                    TensorBody::Mul(y.clone(), x.clone())
                }
                TensorBody::Sub(_, y) if matches!(**y, TensorBody::Operand(i) if i == slot) => {
                    continue
                }
                other => other.clone(),
            };
            let inner = reindex(abody, &|i| i);
            let body = splice(&bbody, slot, &inner, &|i| {
                shift.get(i).copied().unwrap_or(i)
            });

            // ── Rebuild without stage `ai` and without the intermediate buffer ──────────────────
            let dead_buf = *feed;
            let mut q = p.clone();
            q.stages[bi].kind = StageKind::Compute(ComputeOp::Generic {
                iters: bit.clone(),
                ins: maps,
                out: bout.clone(),
                body,
            });
            q.stages[bi].ins = ins;
            let keep_stage: Vec<bool> = (0..q.stages.len()).map(|i| i != ai).collect();
            let keep_buf: Vec<bool> = q.buffers.iter().map(|x| x.id != dead_buf).collect();
            let smap: Vec<usize> = keep_stage
                .iter()
                .scan(0, |n, k| {
                    let v = *n;
                    if *k {
                        *n += 1
                    }
                    Some(v)
                })
                .collect();
            let bmap: Vec<usize> = keep_buf
                .iter()
                .scan(0, |n, k| {
                    let v = *n;
                    if *k {
                        *n += 1
                    }
                    Some(v)
                })
                .collect();
            let rs = |s: StageId| StageId::new(smap[s.idx()]);
            let rb = |x: BufferId| BufferId::new(bmap[x.idx()]);

            let mut stages: Vec<Stage> = Vec::new();
            for (i, s) in q.stages.iter().enumerate() {
                if !keep_stage[i] {
                    continue;
                }
                let mut s = s.clone();
                s.id = StageId::new(stages.len());
                s.ins = s.ins.iter().map(|x| rb(*x)).collect();
                s.outs = s.outs.iter().map(|x| rb(*x)).collect();
                s.kind.remap_buffers(&rb);
                stages.push(s);
            }
            let mut buffers: Vec<Buffer> = Vec::new();
            for (i, x) in q.buffers.iter().enumerate() {
                if !keep_buf[i] {
                    continue;
                }
                let mut x = x.clone();
                x.id = BufferId::new(buffers.len());
                x.alias_of = x.alias_of.map(rb);
                buffers.push(x);
            }
            let mut edges: Vec<Edge> = Vec::new();
            for e in &q.edges {
                if e.buffer == dead_buf {
                    continue; // the hand-off that fusion removed
                }
                let mut e = e.clone();
                // The producer's operand edges now feed the fused stage.
                let to: Vec<StageId> =
                    e.to.iter()
                        .map(|t| {
                            if *t == StageId::new(ai) {
                                StageId::new(bi)
                            } else {
                                *t
                            }
                        })
                        .collect();
                e.id = EdgeId::new(edges.len());
                e.from = rs(e.from);
                e.to = to.iter().map(|t| rs(*t)).collect();
                e.buffer = rb(e.buffer);
                edges.push(e);
            }
            q.stages = stages;
            q.buffers = buffers;
            q.edges = edges;
            if q.validate().is_err() {
                continue; // never hand on a program this pass could not keep legal
            }
            return Some(q);
        }
    }
    None
}

/// Fuse until nothing more qualifies.
pub fn fuse_generic(
    p: &SpatialProgram,
    _target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let mut cur = p.clone();
    // Bounded by the stage count: each fusion removes one stage, so this cannot spin.
    for _ in 0..p.stages.len() {
        match fuse_once(&cur) {
            Some(next) => cur = next,
            None => break,
        }
    }
    Ok(cur)
}
