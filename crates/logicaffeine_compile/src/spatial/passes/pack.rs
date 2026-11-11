//! Phase packing — independent sections run at once, on disjoint sites.
//!
//! ★ A BARRIER ONLY WHERE DATA FLOWS. A composed program's sections run in PHASES: every section of a phase starts
//! together, and a barrier closes the phase. Two sections may share one when neither reads what the other writes nor
//! writes what the other touches (by tensor name, the carried state included) — and, on one chip, when they hold
//! DISJOINT SITES: a site's local memory is laid out for the one section it runs in a phase, so two sections never
//! share a buffer. On a chip whose sites are interchangeable — a crossbar, no multicast rectangle whose geometry a
//! section's plan depends on — where a section runs is a renaming ([`for_each_site_mut`]), so each section is packed onto the
//! next free sites of the earliest phase after everything it depends on that still has room for it.
//!
//! A decode layer's sections each hold a few dozen of the SN50's 1,040 sites: the projections of one normalized row,
//! the convolutions of its three streams, the per-head norms beside them run in one phase each instead of one after
//! another with the rest of the fabric idle.

use std::collections::BTreeSet;

use crate::spatial::ir::*;
use crate::spatial::target::{Interconnect, SpatialTarget};
use crate::spatial::tensor_parallel::SectionTouch;

/// Every site `p` names, rewritten by `f`: its stages', buffers', placement's, memory map's, semaphores' and engine
/// programs', and the semaphore posts inside those programs. A bijection of sites moves where the program runs and
/// nothing it computes.
pub fn for_each_site_mut(p: &mut SpatialProgram, f: &mut dyn FnMut(&mut Site)) {
    fn posts(ops: &mut [TaggedOp], f: &mut dyn FnMut(&mut Site)) {
        for t in ops {
            match &mut t.op {
                ChoreoOp::SemInc { at, .. } => f(at),
                ChoreoOp::Loop { body, .. } => posts(body, f),
                _ => {}
            }
        }
    }
    for s in &mut p.stages {
        if let Some(x) = &mut s.site {
            f(x);
        }
    }
    for b in &mut p.buffers {
        if let Some(x) = &mut b.site {
            f(x);
        }
    }
    if let Some(pl) = &mut p.placement {
        pl.site_of.iter_mut().for_each(|x| f(x));
        pl.engine_of.iter_mut().for_each(|e| f(&mut e.site));
        pl.items_of_site.iter_mut().for_each(|(x, _)| f(x));
    }
    if let Some(mm) = &mut p.memory {
        mm.buffer_at.iter_mut().for_each(|a| f(&mut a.site));
    }
    for sem in &mut p.semaphores {
        sem.sites.iter_mut().for_each(|x| f(x));
    }
    if let Some(sch) = &mut p.schedule {
        for prog in &mut sch.programs {
            f(&mut prog.site);
            posts(&mut prog.ops, f);
        }
    }
}

/// The sites `p` runs on.
pub fn sites_used(p: &mut SpatialProgram) -> BTreeSet<Site> {
    let mut seen = BTreeSet::new();
    for_each_site_mut(p, &mut |s| {
        seen.insert(*s);
    });
    seen
}

/// Whether where a section runs is a renaming on `target`: its sites are interchangeable (no multicast rectangle), and
/// no section of `sections` plans a multicast group or spans more than one chip.
fn packable(sections: &mut [SpatialProgram], target: &SpatialTarget) -> bool {
    !target.provides_interconnect(Interconnect::MulticastRect)
        && sections.iter_mut().all(|p| {
            p.placement.as_ref().is_some_and(|pl| pl.groups.is_empty())
                && sites_used(p).iter().all(|s| s.chip == 0)
        })
}

/// Sections packed into phases ([`pack_phases`]), in phase order: `barrier_after[k]` closes each phase's last section,
/// and `order[k]` is the place section `k` had among the sections handed in.
#[derive(Clone, Debug)]
pub struct Packed {
    pub sections: Vec<SpatialProgram>,
    pub barrier_after: Vec<bool>,
    pub order: Vec<usize>,
}

/// The sections packed into phases: each moved onto the next free sites of the earliest phase after every section it
/// depends on that still has room, returned in phase order with `barrier_after[k]` closing each phase's last section
/// — or the sections back, untouched, when they cannot be moved on `target` (see `packable`).
///
/// A section depends on an earlier one exactly when some name witnesses it: it reads what the earlier one writes, or
/// writes what the earlier one reads or writes.
pub fn pack_phases(mut sections: Vec<SpatialProgram>, target: &SpatialTarget) -> Result<Packed, Vec<SpatialProgram>> {
    if sections.len() < 2 || !packable(&mut sections, target) {
        return Err(sections);
    }
    let mesh: Vec<Site> = {
        let mut all: Vec<Site> = (0..target.sites.x)
            .flat_map(|x| (0..target.sites.y).map(move |y| Site::new(x, y)))
            .collect();
        all.sort();
        all
    };
    let footprints: Vec<Vec<Site>> = sections
        .iter_mut()
        .map(|p| sites_used(p).into_iter().collect())
        .collect();
    if footprints.iter().any(|used| used.len() > mesh.len()) {
        return Err(sections);
    }
    let (mut written_in, mut read_in) = (
        std::collections::BTreeMap::<String, usize>::new(),
        std::collections::BTreeMap::<String, usize>::new(),
    );
    let mut occupied: Vec<usize> = Vec::new();
    let mut phase_of = Vec::with_capacity(sections.len());
    for (p, used) in sections.iter_mut().zip(&footprints) {
        let touch = SectionTouch::of(p);
        let raw = touch.reads.iter().filter_map(|n| written_in.get(n));
        let war_waw = touch
            .writes
            .iter()
            .flat_map(|n| [written_in.get(n), read_in.get(n)])
            .flatten();
        let after = raw.chain(war_waw).map(|ph| ph + 1).max().unwrap_or(0);
        // The first phase from `after` with room; a phase past every occupied one is empty, and every section fits one.
        let ph = (after..)
            .find(|ph| occupied.get(*ph).copied().unwrap_or(0) + used.len() <= mesh.len())
            .expect("an empty phase holds any placed section");
        if occupied.len() <= ph {
            occupied.resize(ph + 1, 0);
        }
        // The section's `i`-th site becomes the phase's next free one: a monotone renaming, so every list the program
        // keeps in site order stays in it.
        let target_sites = &mesh[occupied[ph]..occupied[ph] + used.len()];
        let rename: std::collections::BTreeMap<Site, Site> = used
            .iter()
            .copied()
            .zip(target_sites.iter().copied())
            .collect();
        for_each_site_mut(p, &mut |s| *s = rename[s]);
        occupied[ph] += used.len();
        phase_of.push(ph);
        for n in touch.reads {
            let r = read_in.entry(n).or_insert(ph);
            *r = (*r).max(ph);
        }
        for n in touch.writes {
            let w = written_in.entry(n).or_insert(ph);
            *w = (*w).max(ph);
        }
    }
    let mut order: Vec<usize> = (0..sections.len()).collect();
    order.sort_by_key(|&k| (phase_of[k], k));
    let barrier_after = order
        .iter()
        .enumerate()
        .map(|(pos, &k)| {
            order
                .get(pos + 1)
                .is_some_and(|&next| phase_of[next] != phase_of[k])
        })
        .collect();
    let mut slots: Vec<Option<SpatialProgram>> = sections.into_iter().map(Some).collect();
    Ok(Packed {
        sections: order
            .iter()
            .map(|&k| slots[k].take().expect("each section once"))
            .collect(),
        barrier_after,
        order,
    })
}
