//! Runtime arguments: the site-dependent values of a kernel class, declared ONCE as a layout that both the
//! kernel text (`get_arg_val<uint32_t>(i)`) and the host's per-site `SetRuntimeArgs` are printed from.

use super::roles::KernelClass;
use super::SpatialLowerError;
use crate::spatial::ir::*;
use crate::spatial::passes::{LV_ITEM, LV_ITEM_X, LV_ITEM_Y};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgSource {
    /// DRAM base address of a tensor.
    TensorAddr(TensorId),
    /// First work item (linear id) of the site — items are consecutive in row-major block order.
    ItemStart,
    /// Number of work items of the site.
    ItemCount,
    /// Strided item enumeration (a wrapped blocked plan): first block row / column and the count per row.
    ItemY0,
    ItemX0,
    ItemCountX,
    /// Multicast rectangle of the RECEIVERS of the site's group for an operand (near corner / far corner).
    McastStart {
        operand: Operand,
        axis: Axis,
    },
    McastEnd {
        operand: Operand,
        axis: Axis,
    },
    McastNumDests {
        operand: Operand,
    },
    /// Physical coordinate of the sender of the site's group for an operand.
    McastSender {
        operand: Operand,
        axis: Axis,
    },
    /// The program-wide semaphore handle (`get_semaphore(id)` in the kernel).
    SemHandle(SemId),
    /// Base of the device-cycle scratch, and this site's word in it. Stamped by the kernel that writes the
    /// output, whose lifetime spans the whole program on its core.
    CyclesAddr,
    CyclesSlot(u8),
    /// A tensor's tile-grid extent (`rows_t` / `cols_t`), handed to a data-movement kernel at run time: the
    /// ONLY way shape enters a kernel. Every tile stride and loop count is printed as a block-extent literal
    /// times or over one of these, so the address arithmetic has exactly the vendor's form — constant-folding
    /// the shape restructures it and slows the NOC issue rate (+844 cycles at 640³ on ttsim), while a runtime
    /// value the compiler cannot see is 1 costs a multiply per tile (+300 cycles). A movement kernel is thereby
    /// shape-generic: the same binary serves every shape of the same blocking.
    TensorDim {
        tensor: TensorId,
        axis: Axis,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArgSlot {
    pub name: String,
    pub source: ArgSource,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct RuntimeArgLayout {
    pub per_kernel: Vec<(String, Vec<ArgSlot>)>,
}

/// How a class enumerates its items: consecutive linear ids, or a (y, x) stride lattice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemMode {
    Linear,
    Strided,
}

pub fn operand_tag(o: Operand) -> &'static str {
    match o {
        Operand::A => "a",
        Operand::B => "b",
    }
}

fn operand_of_sem(p: &SpatialProgram, sem: SemId) -> Option<Operand> {
    p.semaphores[sem.idx()].operand
}

/// Whether every member site's items are consecutive linear ids.
pub fn item_mode(p: &SpatialProgram, class: &KernelClass) -> Result<ItemMode, SpatialLowerError> {
    let pl = p.placement.as_ref().unwrap();
    let mut linear = true;
    for site in &class.sites {
        let items = pl.items_at(*site);
        if items.windows(2).any(|w| w[1].idx() != w[0].idx() + 1) {
            linear = false;
        }
    }
    if linear {
        return Ok(ItemMode::Linear);
    }
    // Strided: items = {(y0 + iy·sy, x0 + ix·sx)} in row-major (iy, ix) order.
    for site in &class.sites {
        let coords: Vec<(usize, usize)> = pl
            .items_at(*site)
            .iter()
            .map(|i| p.work.items[i.idx()].coord)
            .collect();
        let xs: std::collections::BTreeSet<usize> = coords.iter().map(|c| c.1).collect();
        let ys: std::collections::BTreeSet<usize> = coords.iter().map(|c| c.0).collect();
        let mut expect = Vec::new();
        for y in &ys {
            for x in &xs {
                expect.push((*y, *x));
            }
        }
        if expect != coords {
            return Err(SpatialLowerError::Unsupported(
                "a site's work items are neither a linear range nor a stride lattice",
            ));
        }
    }
    Ok(ItemMode::Strided)
}

/// The argument slots of a kernel class, in first-use order.
pub fn slots_for(
    p: &SpatialProgram,
    class: &KernelClass,
) -> Result<Vec<ArgSlot>, SpatialLowerError> {
    let sched = p.schedule.as_ref().unwrap();
    let rep = &sched.programs[class.representative];
    let ops: Vec<ChoreoOp> = rep.ops.iter().map(|t| t.op.clone()).collect();
    let mut slots: Vec<ArgSlot> = Vec::new();
    let mut push = |name: String, source: ArgSource| {
        if !slots.iter().any(|s| s.source == source) {
            slots.push(ArgSlot { name, source });
        }
    };
    let mut uses_coords = false;
    let mut has_item_loop = false;
    let mut err = None;
    walk_ops(&ops, 1, &mut |op, _| match op {
        ChoreoOp::ReadTiles { tensor, at, .. } | ChoreoOp::WriteTiles { tensor, at, .. } => {
            push(
                format!("t{}_addr", tensor.idx()),
                ArgSource::TensorAddr(*tensor),
            );
            if at
                .steps
                .iter()
                .any(|(v, _)| *v == LV_ITEM_Y || *v == LV_ITEM_X)
            {
                uses_coords = true;
            }
        }
        ChoreoOp::Loop { var, .. } if *var == LV_ITEM => has_item_loop = true,
        _ => {}
    });
    let mode = item_mode(p, class)?;
    if uses_coords {
        match mode {
            ItemMode::Linear => {
                push("item_start".into(), ArgSource::ItemStart);
            }
            ItemMode::Strided => {
                push("item_y0".into(), ArgSource::ItemY0);
                push("item_x0".into(), ArgSource::ItemX0);
                push("item_count_x".into(), ArgSource::ItemCountX);
            }
        }
    }
    if has_item_loop {
        push("item_count".into(), ArgSource::ItemCount);
    }
    walk_ops(&ops, 1, &mut |op, _| match op {
        ChoreoOp::McastSend { group, .. } | ChoreoOp::McastSetValid { group, .. } => {
            let operand = p.placement.as_ref().unwrap().groups[group.idx()].operand;
            let t = operand_tag(operand);
            push(
                format!("mc_{t}_start_x"),
                ArgSource::McastStart {
                    operand,
                    axis: Axis::X,
                },
            );
            push(
                format!("mc_{t}_start_y"),
                ArgSource::McastStart {
                    operand,
                    axis: Axis::Y,
                },
            );
            push(
                format!("mc_{t}_end_x"),
                ArgSource::McastEnd {
                    operand,
                    axis: Axis::X,
                },
            );
            push(
                format!("mc_{t}_end_y"),
                ArgSource::McastEnd {
                    operand,
                    axis: Axis::Y,
                },
            );
            push(
                format!("mc_{t}_num_dests"),
                ArgSource::McastNumDests { operand },
            );
            if let ChoreoOp::McastSetValid { sem, .. } = op {
                push(format!("sem{}_id", sem.idx()), ArgSource::SemHandle(*sem));
            }
        }
        ChoreoOp::SemInc { sem, .. } => match operand_of_sem(p, *sem) {
            Some(operand) => {
                let t = operand_tag(operand);
                push(
                    format!("mc_{t}_sender_x"),
                    ArgSource::McastSender {
                        operand,
                        axis: Axis::X,
                    },
                );
                push(
                    format!("mc_{t}_sender_y"),
                    ArgSource::McastSender {
                        operand,
                        axis: Axis::Y,
                    },
                );
                push(format!("sem{}_id", sem.idx()), ArgSource::SemHandle(*sem));
            }
            None => {
                err = Some(SpatialLowerError::Unsupported(
                    "a semaphore increment outside a multicast group",
                ))
            }
        },
        ChoreoOp::SemSet { sem, .. } | ChoreoOp::SemWait { sem, .. } => {
            push(format!("sem{}_id", sem.idx()), ArgSource::SemHandle(*sem));
        }
        ChoreoOp::Barrier { .. } => {
            err = Some(SpatialLowerError::Unsupported("a group barrier on Tensix"))
        }
        _ => {}
    });
    if let Some(e) = err {
        return Err(e);
    }
    // EVERY movement engine stamps its own span, and the host takes the max. Timing only the writer measures
    // the writer, whose work is the output tiles and therefore independent of the reduction depth — MEASURED,
    // 810 cycles at K=640 against 809 at K=1280. The operand reader is the engine whose work scales with K,
    // and which of the two is the slower one is exactly what the plan search is trying to change, so the
    // honest program length is the max over engines rather than any single one.
    if stamps_cycles(&ops) {
        let engine = match rep.engine {
            Engine::Movement(e) => e,
            Engine::Compute => 0,
        };
        push("cycles_addr".into(), ArgSource::CyclesAddr);
        push("cycles_slot".into(), ArgSource::CyclesSlot(engine));
    }
    Ok(slots)
}

/// Does this engine program touch DRAM? Every such engine is timed; the compute engine cannot be, since a
/// TRISC has no NOC of its own to write the result back with.
pub fn stamps_cycles(ops: &[ChoreoOp]) -> bool {
    let mut found = false;
    walk_ops(ops, 1, &mut |op, _| {
        if matches!(op, ChoreoOp::WriteTiles { .. } | ChoreoOp::ReadTiles { .. }) {
            found = true;
        }
    });
    found
}
