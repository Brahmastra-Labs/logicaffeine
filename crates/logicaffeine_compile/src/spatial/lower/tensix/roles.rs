//! Engine programs → kernel classes. Two programs whose site-independent signatures agree share ONE kernel
//! source; a launch (`CreateKernel`) is a distinct (source, engine, NOC) over the maximal rectangles of the
//! sites it covers. The 2-D GEMM's four reader variants fall out of the signature, unnamed.

use std::collections::{BTreeMap, BTreeSet};

use super::SpatialLowerError;
use crate::spatial::ir::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub noc: Option<NocId>,
    pub rects: Vec<Rect>,
    pub sites: Vec<Site>,
}

/// One kernel source: a signature class of engine programs, launched once per NOC it runs on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelClass {
    pub file_name: String,
    pub engine: Engine,
    pub signature: String,
    /// The representative program (the first site in order) the kernel text is printed from.
    pub representative: usize,
    /// Every site of the class, in site order.
    pub sites: Vec<Site>,
    /// One `CreateKernel` per NOC.
    pub launches: Vec<Launch>,
}

/// Greedy maximal-rectangle decomposition of a site set, deterministic (row-major seeds).
pub fn rect_decompose(sites: &BTreeSet<Site>) -> Vec<Rect> {
    let mut left: BTreeSet<(u16, u16)> = sites.iter().map(|s| (s.y, s.x)).collect();
    let mut out = Vec::new();
    while let Some(&(y0, x0)) = left.iter().next() {
        let mut x1 = x0;
        while left.contains(&(y0, x1 + 1)) {
            x1 += 1;
        }
        let mut y1 = y0;
        while (x0..=x1).all(|x| left.contains(&(y1 + 1, x))) {
            y1 += 1;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                left.remove(&(y, x));
            }
        }
        out.push(Rect { x0, y0, x1, y1 });
    }
    out
}

fn describe(p: &SpatialProgram, prog: &EngineProgram) -> String {
    // A readable role name from the ops: per operand `send` / `recv` / `read`, plus `writer`.
    let mut a = None;
    let mut b = None;
    let mut writer = false;
    let mut compute = false;
    let sched_ops: Vec<ChoreoOp> = prog.ops.iter().map(|t| t.op.clone()).collect();
    walk_ops(&sched_ops, 1, &mut |op, _| match op {
        ChoreoOp::ReadTiles { tensor, .. } => {
            let slot = if tensor.idx() == 0 { &mut a } else { &mut b };
            if slot.is_none() {
                *slot = Some("read");
            }
        }
        ChoreoOp::McastSend { group, .. } => {
            let op = p.placement.as_ref().unwrap().groups[group.idx()].operand;
            let slot = if op == Operand::A { &mut a } else { &mut b };
            *slot = Some("send");
        }
        ChoreoOp::SemInc { sem, .. } => {
            if let Some(op) = p.semaphores[sem.idx()].operand {
                let slot = if op == Operand::A { &mut a } else { &mut b };
                *slot = Some("recv");
            }
        }
        ChoreoOp::WriteTiles { .. } => writer = true,
        ChoreoOp::Compute { .. } => compute = true,
        _ => {}
    });
    let mut parts = Vec::new();
    if let Some(r) = a {
        parts.push(format!("a_{r}"));
    }
    if let Some(r) = b {
        parts.push(format!("b_{r}"));
    }
    if writer {
        parts.push("writer".into());
    }
    if compute {
        parts.push("compute".into());
    }
    if parts.is_empty() {
        parts.push("idle".into());
    }
    parts.join("_")
}

pub fn classes(p: &SpatialProgram) -> Result<Vec<KernelClass>, SpatialLowerError> {
    let pl = p
        .placement
        .as_ref()
        .ok_or(SpatialLowerError::Unsupported("a placed program"))?;
    let sched = p
        .schedule
        .as_ref()
        .ok_or(SpatialLowerError::Unsupported("a scheduled program"))?;
    // (engine, signature) → members with their NOC.
    let mut groups: BTreeMap<(u8, String), Vec<(usize, Option<u8>)>> = Default::default();
    for (i, prog) in sched.programs.iter().enumerate() {
        let noc = pl
            .engine_of
            .iter()
            .find(|e| e.site == prog.site && e.engine == prog.engine)
            .and_then(|e| e.noc);
        let ekey = match prog.engine {
            Engine::Movement(e) => e,
            Engine::Compute => 255,
        };
        groups
            .entry((ekey, p.engine_signature(prog)))
            .or_default()
            .push((i, noc.map(|n| n.0)));
    }
    let mut classes = Vec::new();
    let mut names: BTreeMap<String, usize> = Default::default();
    for ((_, sig), members) in groups {
        let rep = &sched.programs[members[0].0];
        let prefix = match rep.engine {
            Engine::Movement(e) => format!("dm{e}"),
            Engine::Compute => "compute".into(),
        };
        let base = format!("{prefix}_{}", describe(p, rep));
        let n = names.entry(base.clone()).or_insert(0);
        *n += 1;
        let file_name = if *n == 1 {
            format!("{base}.cpp")
        } else {
            format!("{base}_{}.cpp", *n)
        };
        let sites: Vec<Site> = members
            .iter()
            .map(|(i, _)| sched.programs[*i].site)
            .collect();
        let mut by_noc: BTreeMap<Option<u8>, BTreeSet<Site>> = Default::default();
        for (i, noc) in &members {
            by_noc
                .entry(*noc)
                .or_default()
                .insert(sched.programs[*i].site);
        }
        let launches = by_noc
            .into_iter()
            .map(|(noc, set)| Launch {
                noc: noc.map(NocId),
                sites: set.iter().copied().collect(),
                rects: rect_decompose(&set),
            })
            .collect();
        classes.push(KernelClass {
            file_name,
            engine: rep.engine,
            signature: sig,
            representative: members[0].0,
            sites,
            launches,
        });
    }
    Ok(classes)
}
