//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! A SECTION RENDEZVOUS IS A GATHER AND A RELEASE — LINEAR IN THE PROGRAMS IT JOINS, AND PLACED.
//!
//! `compose` runs a program's sections back to back, and a rendezvous at every boundary is what makes
//! "section k happens before section k+1" a fact about the schedule. The rendezvous is the centralized
//! barrier: every one of the `P` engine programs posts ONE arrival at a root site; the root's program
//! waits for all `P`, then posts ONE release at each of the `S` participating sites; every program waits
//! for its own site's release. That is `2P + S + 1` ops per boundary and `P + S` arrival places in the
//! marked graph.
//!
//! The all-to-all form (every program posts at every site, every program waits for `P`) is `P·S` ops and
//! `P·S` places per boundary. At Qwen3.8's real widths one layer spans hundreds of sites, and that product
//! is what the composed certificate spends its time on — the section graphs of a layer sum to a few
//! million places, and the rendezvous would add tens of millions. The count assertions below pin the
//! linear form exactly, so a quadratic encoding cannot come back.
//!
//! ★ WHAT MAKES IT A BARRIER IS WHERE ITS OPS SIT, NOT HOW MANY THERE ARE. An arrival posted before the
//! section's work, or a release posted before the gather completes, balances every count and deadlocks
//! nothing — and orders nothing either. So the IR validates placement as well as arithmetic: in every
//! program the arrival follows all of its section-k work, the release wait precedes all of its section-k+1
//! work, and the root waits on the gather before it posts a single release. Each mutation below breaks
//! exactly one of those and names the check that refuses it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_rendezvous_lock`
//!
//! ★ MUTATION PROBES (run when this landed): delete the `validate_rendezvous` call from `validate` — the
//! three placement mutations red, and each one's message shows the certificate returning `Ok(())` for a
//! barrier that orders nothing, so placement is enforced by the IR or not at all. Drop the per-site post
//! check from the `Barrier` count rule — the retargeted release reds (the marked graph still reports the
//! unreleased site `Starved`, but the IR must name it first).
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_compile::spatial::certificate::{
    build_marked_graph, certify_choreography, PlaceKind,
};
use logicaffeine_compile::spatial::ir::{
    BarrierPhase, ChoreoOp, Engine, IrViolation, Rendezvous, SemCond, SemId, SemRole, Site,
    SpatialProgram,
};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

/// A matmul feeding two row-wide elementwise ops: three sections, two boundaries. A row op places one
/// work item per tile, so a 4096-wide row spreads each section across many sites — enough that `P·S` and
/// `2P + S + 1` are far apart — and the matmul puts a differently-shaped section on the same sites.
fn chained() -> SpatialProgram {
    let src = "## Main\n\
               Let X be a 1 by 256 tensor of Real.\n\
               Let W be a 256 by 4096 tensor of Real.\n\
               Let Y be X matmul W.\n\
               Let Z be sigmoid of Y.\n\
               Let O be sigmoid of Z.\n";
    let target = RduChip::Sn50.facts().spatial_target();
    let l = compile_program_to_spatial(src, &target)
        .expect("a matmul and two sigmoids are a tensor program");
    assert_eq!(
        l.lowered, l.statements,
        "every statement lowers: {:?}",
        l.missing
    );
    l.program.expect("the three sections compose and certify")
}

fn sites_of(p: &SpatialProgram) -> BTreeSet<Site> {
    p.schedule
        .as_ref()
        .expect("scheduled")
        .programs
        .iter()
        .map(|q| q.site)
        .collect()
}

fn boundaries(p: &SpatialProgram) -> Vec<(usize, Rendezvous)> {
    p.sections
        .iter()
        .enumerate()
        .filter_map(|(k, s)| s.barrier_after.map(|r| (k, r)))
        .collect()
}

/// Where each rendezvous op of one boundary sits: `(program index, top-level op index, op)`.
fn rendezvous_ops(p: &SpatialProgram, r: Rendezvous) -> Vec<(usize, usize, ChoreoOp)> {
    let mut out = Vec::new();
    for (pi, prog) in p
        .schedule
        .as_ref()
        .expect("scheduled")
        .programs
        .iter()
        .enumerate()
    {
        for (oi, t) in prog.ops.iter().enumerate() {
            let sem = match &t.op {
                ChoreoOp::SemInc { sem, .. } | ChoreoOp::SemWait { sem, .. } => *sem,
                _ => continue,
            };
            if sem == r.gather || sem == r.release {
                out.push((pi, oi, t.op.clone()));
            }
        }
    }
    out
}

fn fresh_certificate(p: &SpatialProgram) -> Result<(), String> {
    let mut q = p.clone();
    q.certificate = None;
    certify_choreography(&q)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

/// Apply `f` to the ops of program `pi` and demand the IR refuses the result by the named violation.
fn refused_by(
    p: &SpatialProgram,
    what: &str,
    pi: usize,
    f: impl FnOnce(&mut Vec<logicaffeine_compile::spatial::ir::TaggedOp>),
    expect: fn(&IrViolation) -> bool,
) {
    let mut q = p.clone();
    q.certificate = None;
    f(&mut q.schedule.as_mut().expect("scheduled").programs[pi].ops);
    match q.validate() {
        Err(v) if expect(&v) => {}
        Err(v) => panic!("{what}: refused, but by the wrong check: {v:?}"),
        Ok(()) => panic!("{what}: the IR accepted it (certificate: {:?}) — a rendezvous that does not order must never validate", fresh_certificate(&q)),
    }
    assert!(
        fresh_certificate(&q).is_err(),
        "{what}: the certificate must refuse it too"
    );
}

fn unbalanced(v: &IrViolation) -> bool {
    matches!(v, IrViolation::SectionBarrierUnbalanced { .. })
}

fn misplaced(v: &IrViolation) -> bool {
    matches!(v, IrViolation::SectionBarrierMisplaced { .. })
}

/// The program index whose ops hold the gather wait of `r` — the root.
fn root_of(p: &SpatialProgram, r: Rendezvous) -> usize {
    rendezvous_ops(p, r)
        .into_iter()
        .find(|(_, _, op)| matches!(op, ChoreoOp::SemWait { sem, .. } if *sem == r.gather))
        .map(|(pi, _, _)| pi)
        .expect("a gather wait")
}

fn index_of(p: &SpatialProgram, pi: usize, pred: impl Fn(&ChoreoOp) -> bool) -> usize {
    p.schedule.as_ref().expect("scheduled").programs[pi]
        .ops
        .iter()
        .position(|t| pred(&t.op))
        .expect("the op is in the program")
}

/// ── THE SHAPE, EXACTLY. One gather and one release per boundary, and nothing quadratic anywhere.
#[test]
fn every_boundary_is_one_gather_and_one_release() {
    let p = chained();
    let sched = p.schedule.as_ref().expect("scheduled");
    let programs = sched.programs.len();
    let sites = sites_of(&p);
    assert!(
        sites.len() >= 16,
        "the fixture must spread across many sites to tell P·S from 2P+S, got {}",
        sites.len()
    );
    let bs = boundaries(&p);
    assert_eq!(
        bs.len(),
        p.sections.len() - 1,
        "every section but the last closes with a rendezvous"
    );
    for (k, r) in bs {
        let (g, rl) = (
            &p.semaphores[r.gather.idx()],
            &p.semaphores[r.release.idx()],
        );
        assert_eq!(
            g.role,
            SemRole::Barrier {
                n: programs,
                phase: BarrierPhase::Gather
            },
            "boundary {k}: the gather joins every program"
        );
        assert_eq!(
            rl.role,
            SemRole::Barrier {
                n: programs,
                phase: BarrierPhase::Release
            },
            "boundary {k}: the release frees every program"
        );
        assert_eq!(g.sites.len(), 1, "boundary {k}: a gather has one root");
        assert_eq!(
            rl.sites.iter().copied().collect::<BTreeSet<_>>(),
            sites,
            "boundary {k}: a release reaches every participating site"
        );
        let span = p.sections[k].semaphores;
        assert!(
            (span.0..span.1).contains(&r.gather.idx())
                && (span.0..span.1).contains(&r.release.idx()),
            "boundary {k}: the rendezvous belongs to the section it closes"
        );

        let root = g.sites[0];
        let ops = rendezvous_ops(&p, r);
        let mut arrivals: BTreeMap<usize, usize> = BTreeMap::new();
        let mut release_waits: BTreeMap<usize, usize> = BTreeMap::new();
        let mut posts: BTreeMap<Site, usize> = BTreeMap::new();
        let mut gather_waits = 0usize;
        for (pi, _, op) in &ops {
            match op {
                ChoreoOp::SemInc { sem, at, by } if *sem == r.gather => {
                    assert_eq!(
                        (*at, *by),
                        (root, 1),
                        "boundary {k}: an arrival is one post at the root"
                    );
                    *arrivals.entry(*pi).or_default() += 1;
                }
                ChoreoOp::SemWait { sem, until } if *sem == r.gather => {
                    assert_eq!(
                        *until,
                        SemCond::AtLeast(programs),
                        "boundary {k}: the root waits for every arrival"
                    );
                    assert_eq!(
                        sched.programs[*pi].site, root,
                        "boundary {k}: the gather is waited on at its root"
                    );
                    gather_waits += 1;
                }
                ChoreoOp::SemInc { sem, at, by } if *sem == r.release => {
                    assert_eq!(*by, 1);
                    *posts.entry(*at).or_default() += 1;
                }
                ChoreoOp::SemWait { sem, until } if *sem == r.release => {
                    assert_eq!(
                        *until,
                        SemCond::AtLeast(1),
                        "boundary {k}: a release wait needs its site's one post"
                    );
                    *release_waits.entry(*pi).or_default() += 1;
                }
                other => panic!("boundary {k}: unexpected rendezvous op {other:?}"),
            }
        }
        assert_eq!(
            arrivals,
            (0..programs).map(|pi| (pi, 1)).collect(),
            "boundary {k}: every program arrives exactly once"
        );
        assert_eq!(
            release_waits,
            (0..programs).map(|pi| (pi, 1)).collect(),
            "boundary {k}: every program waits for its release exactly once"
        );
        assert_eq!(
            gather_waits, 1,
            "boundary {k}: exactly one program waits on the gather"
        );
        assert_eq!(
            posts,
            sites.iter().map(|s| (*s, 1)).collect(),
            "boundary {k}: the release is posted once at each site"
        );
        assert_eq!(
            ops.len(),
            2 * programs + sites.len() + 1,
            "boundary {k}: a rendezvous costs 2P + S + 1 ops"
        );
        println!(
            "boundary {k}: P = {programs}, S = {}, {} rendezvous ops (all-to-all would be {})",
            sites.len(),
            ops.len(),
            programs * sites.len() + programs
        );
    }
}

/// ── THE CERTIFICATE PAYS P + S PER BOUNDARY. Arrival places are the rendezvous' whole footprint in the
/// marked graph: P arrivals at the root and one release token per site.
#[test]
fn the_rendezvous_costs_the_certificate_p_plus_s_places() {
    let p = chained();
    let programs = p.schedule.as_ref().expect("scheduled").programs.len();
    let sites = sites_of(&p).len();
    let g = build_marked_graph(&p).expect("the whole graph builds");
    let mut arrivals: BTreeMap<SemId, usize> = BTreeMap::new();
    for pl in &g.places {
        if let PlaceKind::Arrival { sem, .. } = pl.kind {
            *arrivals.entry(sem).or_default() += 1;
        }
    }
    for (k, r) in boundaries(&p) {
        assert_eq!(
            arrivals.get(&r.gather).copied().unwrap_or(0),
            programs,
            "boundary {k}: one arrival place per program"
        );
        assert_eq!(
            arrivals.get(&r.release).copied().unwrap_or(0),
            sites,
            "boundary {k}: one release place per site"
        );
    }
    assert_eq!(
        arrivals.values().sum::<usize>(),
        (p.sections.len() - 1) * (programs + sites),
        "no other arrival places exist"
    );
}

/// ── AND IT CLOSES ITS SECTION IN EVERY PROGRAM. The arrival follows the program's last op of section k;
/// the release wait precedes its first op of section k+1; the root waits on the gather before posting.
#[test]
fn the_rendezvous_closes_its_section_in_every_program() {
    let p = chained();
    let sched = p.schedule.as_ref().expect("scheduled");
    for (k, r) in boundaries(&p) {
        let root = root_of(&p, r);
        for (pi, prog) in sched.programs.iter().enumerate() {
            let arrive = index_of(
                &p,
                pi,
                |op| matches!(op, ChoreoOp::SemInc { sem, .. } if *sem == r.gather),
            );
            let wait = index_of(
                &p,
                pi,
                |op| matches!(op, ChoreoOp::SemWait { sem, .. } if *sem == r.release),
            );
            assert!(
                arrive < wait,
                "program {pi}, boundary {k}: arrive before waiting for the release"
            );
            for (oi, t) in prog.ops.iter().enumerate() {
                let rendezvous = matches!(&t.op, ChoreoOp::SemInc { sem, .. } | ChoreoOp::SemWait { sem, .. } if p.sections.iter().any(|s| s.barrier_after.is_some_and(|b| b.gather == *sem || b.release == *sem)));
                if rendezvous {
                    continue;
                }
                let sec = p
                    .section_of_stage(t.stage)
                    .expect("every op is in a section");
                if sec <= k {
                    assert!(oi < arrive, "program {pi}: op {oi} of section {sec} runs after the arrival that closes section {k}");
                } else {
                    assert!(oi > wait, "program {pi}: op {oi} of section {sec} runs before the release of section {k}");
                }
            }
            if pi == root {
                let gw = index_of(
                    &p,
                    pi,
                    |op| matches!(op, ChoreoOp::SemWait { sem, .. } if *sem == r.gather),
                );
                let first_post = index_of(
                    &p,
                    pi,
                    |op| matches!(op, ChoreoOp::SemInc { sem, .. } if *sem == r.release),
                );
                assert!(
                    arrive < gw && gw < first_post && first_post < wait,
                    "the root arrives, gathers, releases, then waits"
                );
            }
        }
    }
    fresh_certificate(&p).expect("the composed program certifies");
}

/// ── BREAKING 1: a program that never arrives starves the root.
#[test]
fn a_missing_arrival_is_refused() {
    let p = chained();
    let (_, r) = boundaries(&p)[0];
    let victim = (root_of(&p, r) + 1) % p.schedule.as_ref().unwrap().programs.len();
    let at = index_of(
        &p,
        victim,
        |op| matches!(op, ChoreoOp::SemInc { sem, .. } if *sem == r.gather),
    );
    refused_by(
        &p,
        "a dropped arrival",
        victim,
        |ops| drop(ops.remove(at)),
        unbalanced,
    );
}

/// ── BREAKING 2: a root that never gathers releases everyone before anyone has finished.
#[test]
fn a_missing_gather_wait_is_refused() {
    let p = chained();
    let (_, r) = boundaries(&p)[0];
    let root = root_of(&p, r);
    let at = index_of(
        &p,
        root,
        |op| matches!(op, ChoreoOp::SemWait { sem, .. } if *sem == r.gather),
    );
    refused_by(
        &p,
        "a dropped gather wait",
        root,
        |ops| drop(ops.remove(at)),
        unbalanced,
    );
}

/// ── BREAKING 3: a site whose release is never posted never proceeds.
#[test]
fn a_missing_release_post_is_refused() {
    let p = chained();
    let (_, r) = boundaries(&p)[0];
    let root = root_of(&p, r);
    let at = index_of(
        &p,
        root,
        |op| matches!(op, ChoreoOp::SemInc { sem, .. } if *sem == r.release),
    );
    refused_by(
        &p,
        "a dropped release post",
        root,
        |ops| drop(ops.remove(at)),
        unbalanced,
    );
}

/// ── BREAKING 4: the right number of posts at the wrong sites — one site released twice, one never.
#[test]
fn a_release_posted_twice_to_one_site_and_never_to_another_is_refused() {
    let p = chained();
    let (_, r) = boundaries(&p)[0];
    let root = root_of(&p, r);
    let root_site = p.schedule.as_ref().unwrap().programs[root].site;
    let at = index_of(
        &p,
        root,
        |op| matches!(op, ChoreoOp::SemInc { sem, at, .. } if *sem == r.release && *at != root_site),
    );
    refused_by(
        &p,
        "a release retargeted onto the root's site",
        root,
        |ops| {
            if let ChoreoOp::SemInc { at: to, .. } = &mut ops[at].op {
                *to = root_site;
            }
        },
        unbalanced,
    );
}

/// ── BREAKING 5: a program that does not wait for its release runs into the next section early.
#[test]
fn a_missing_release_wait_is_refused() {
    let p = chained();
    let (_, r) = boundaries(&p)[0];
    let victim = (root_of(&p, r) + 1) % p.schedule.as_ref().unwrap().programs.len();
    let at = index_of(
        &p,
        victim,
        |op| matches!(op, ChoreoOp::SemWait { sem, .. } if *sem == r.release),
    );
    refused_by(
        &p,
        "a dropped release wait",
        victim,
        |ops| drop(ops.remove(at)),
        unbalanced,
    );
}

/// The first top-level op of program `pi` that belongs to section `k` and is not a rendezvous op.
fn first_work_of(p: &SpatialProgram, pi: usize, k: usize) -> Option<usize> {
    let rv: BTreeSet<SemId> = p
        .sections
        .iter()
        .filter_map(|s| s.barrier_after)
        .flat_map(|b| [b.gather, b.release])
        .collect();
    p.schedule.as_ref().unwrap().programs[pi].ops.iter().position(|t| {
        let is_rv = matches!(&t.op, ChoreoOp::SemInc { sem, .. } | ChoreoOp::SemWait { sem, .. } if rv.contains(sem));
        !is_rv && p.section_of_stage(t.stage) == Some(k)
    })
}

/// A program with work in both section `k` and section `k + 1`, other than `avoid`.
fn busy_program(p: &SpatialProgram, k: usize, avoid: usize) -> usize {
    (0..p.schedule.as_ref().unwrap().programs.len())
        .find(|pi| {
            *pi != avoid
                && first_work_of(p, *pi, k).is_some()
                && first_work_of(p, *pi, k + 1).is_some()
        })
        .expect("some non-root program works on both sides of the boundary")
}

/// ── BREAKING 6: every count balances, nothing deadlocks, and the arrival announces work not yet done.
#[test]
fn an_arrival_hoisted_above_the_section_work_is_refused() {
    let p = chained();
    let (k, r) = boundaries(&p)[0];
    let victim = busy_program(&p, k, root_of(&p, r));
    let at = index_of(
        &p,
        victim,
        |op| matches!(op, ChoreoOp::SemInc { sem, .. } if *sem == r.gather),
    );
    let to = first_work_of(&p, victim, k).expect("section work");
    refused_by(
        &p,
        "an arrival posted before the section's work",
        victim,
        |ops| {
            let op = ops.remove(at);
            ops.insert(to, op);
        },
        misplaced,
    );
}

/// ── BREAKING 7: the root releases the sites before it has gathered — counts intact, order gone.
#[test]
fn a_release_posted_before_the_gather_completes_is_refused() {
    let p = chained();
    let (_, r) = boundaries(&p)[0];
    let root = root_of(&p, r);
    let gw = index_of(
        &p,
        root,
        |op| matches!(op, ChoreoOp::SemWait { sem, .. } if *sem == r.gather),
    );
    let last_post = p.schedule.as_ref().unwrap().programs[root]
        .ops
        .iter()
        .rposition(|t| matches!(&t.op, ChoreoOp::SemInc { sem, .. } if *sem == r.release))
        .expect("release posts");
    refused_by(
        &p,
        "a gather wait sunk below the release posts",
        root,
        |ops| {
            let op = ops.remove(gw);
            ops.insert(last_post, op);
        },
        misplaced,
    );
}

/// ── BREAKING 8: a program starts the next section, then waits — the wait orders nothing it guards.
#[test]
fn a_release_wait_sunk_below_the_next_sections_work_is_refused() {
    let p = chained();
    let (k, r) = boundaries(&p)[0];
    let victim = busy_program(&p, k, root_of(&p, r));
    let at = index_of(
        &p,
        victim,
        |op| matches!(op, ChoreoOp::SemWait { sem, .. } if *sem == r.release),
    );
    let next = first_work_of(&p, victim, k + 1).expect("next-section work");
    refused_by(
        &p,
        "a release wait after the next section's first op",
        victim,
        |ops| {
            let op = ops.remove(at);
            ops.insert(next, op);
        },
        misplaced,
    );
}

/// ── ONE ROOT PER BOUNDARY, AND IT IS A PARTICIPANT. The root is the first program in site order, so a
/// rendezvous is a function of the schedule and never of iteration order.
#[test]
fn the_root_is_the_first_participant_in_site_order() {
    let p = chained();
    let sched = p.schedule.as_ref().expect("scheduled");
    let first: (Site, Engine) = sched
        .programs
        .iter()
        .map(|q| (q.site, q.engine))
        .min_by_key(|(s, e)| (s.y, s.x, *e))
        .expect("programs");
    for (k, r) in boundaries(&p) {
        let root = root_of(&p, r);
        assert_eq!(
            (sched.programs[root].site, sched.programs[root].engine),
            first,
            "boundary {k}: the root is the first program in (y, x, engine) order"
        );
        assert_eq!(
            p.semaphores[r.gather.idx()].sites,
            vec![first.0],
            "boundary {k}: the gather lives at the root's site"
        );
    }
}
