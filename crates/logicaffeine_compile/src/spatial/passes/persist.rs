//! `persist` — one resident worker per site loops over ITS work items (the "persistent kernel").
//!
//! Where a site owns more than one work item, every engine program of the site is wrapped in an outer
//! item loop; the tile addresses are already affine in the item coordinates (`LV_ITEM_Y` / `LV_ITEM_X`),
//! which the choreography derives from the item index. Multicast groups whose members own different item
//! counts would pair blocks with nothing on the far side — refused typed.

use super::{SpatialReject, LV_ITEM};
use crate::spatial::ir::*;
use crate::spatial::target::SpatialTarget;

pub fn persist(
    p: &SpatialProgram,
    _target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (Some(pl), Some(sched)) = (&p.placement, &p.schedule) else {
        return Err(SpatialReject::WrongPhase {
            pass: "persist",
            expected: "Scheduled",
            found: p.phase(),
        });
    };
    for g in &pl.groups {
        let counts: Vec<(Site, usize)> = g
            .rect
            .sites()
            .iter()
            .map(|s| (*s, pl.items_at(*s).len()))
            .collect();
        if counts
            .iter()
            .map(|(_, c)| *c)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1
        {
            return Err(SpatialReject::GroupCountMismatch {
                group: g.id,
                counts,
            });
        }
    }
    let mut q = p.clone();
    let programs = q.schedule.as_mut().unwrap();
    for prog in programs.programs.iter_mut() {
        let items = pl.items_at(prog.site).len();
        let already = matches!(prog.ops.first(), Some(TaggedOp { op: ChoreoOp::Loop { var, .. }, .. }) if *var == LV_ITEM);
        if items > 1 && !already {
            let stage = prog.ops[0].stage;
            let body: Vec<TaggedOp> = prog.ops.drain(..).collect();
            prog.ops = vec![TaggedOp {
                stage,
                op: ChoreoOp::Loop {
                    var: LV_ITEM,
                    count: LoopCount::Const(items),
                    body,
                },
            }];
        }
    }
    let _ = sched;
    q.validate()?;
    Ok(q)
}
