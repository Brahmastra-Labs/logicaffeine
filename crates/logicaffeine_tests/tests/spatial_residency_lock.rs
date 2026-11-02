//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! WHAT LEAVES THE CHIP — the SRAM holds what fits, by what it is worth, and the work model counts the rest.
//!
//! The SN50's 432 MB of SRAM is the PMUs' memory, and any site reaches any other's across the crossbar: a tensor held
//! there between sections never goes to HBM. The composed programs called every intermediate and every carried state
//! SRAM-resident regardless of capacity, and the work model counted all of it as DRAM traffic anyway — at a batch of 8
//! a quarter of a decode layer's "DRAM bytes" were re-reads of on-chip activations, while 16 layers of KV cache that
//! cannot fit were labeled on chip. Now:
//!   1. `dram_bytes` is what leaves the chip — the tensors off the SRAM tier — and `onchip_bytes` the rest; the two
//!      partition every tile a program moves.
//!   2. An intermediate lives on chip only where the sites reach one another's memory (a crossbar); a target that
//!      shares data through global memory (Tensix, a GPU) keeps it there.
//!   3. On chip, intermediates are ALLOCATED: their live ranges over the program's phases share the SRAM its buffers
//!      leave, and a live set past it spills the one whose last use is furthest away first (linear scan).
//!   4. A program lowered whole is its own deployment: its carried state goes on chip where it fits beside the
//!      program's own footprint. A deployment of many programs plans again over all of them.
//!   5. The deployment's plan keeps the state that moves the most per byte on chip first, within each chip's SRAM
//!      less the largest program's own footprint.
//!   6. A real-width decode bundle plans every carried state of every layer kind, fits the SRAM, and its programs
//!      carry the plan's tier.
//!
//!   7. The bundle manifest tells the host where each carried state and every other tensor lives.
//!   8. Off the chip there are two tiers: what leaves the SRAM goes to the HBM while it holds it, densest first, and
//!      the DDR takes the rest; a chip with no HBM keeps it in its DRAM.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_residency_lock`
//!
//! ★ MUTATION PROBES (run 2026-09-27 in a private copy; logs `logs/sn50/probe_res_*.txt`) — every one reds:
//!   • the allocator never spills, or spills the intermediate that dies FIRST → the live-range test;
//!   • any target with an SRAM tier holds tensors, crossbar or not → Tensix and the GPU keep intermediates "on chip";
//!   • the plan ranks by size instead of bytes moved per byte → the 80 MB memory loses to two 30 MB caches;
//!   • the plan forgets the reserve → a whole program's state and the bundle's plan overfill the SRAM;
//!   • a class counted once instead of once per layer → the bundle test;
//!   • SRAM traffic counted as DRAM → the partition test;
//!   • compose leaves a whole program's state unplanned → the own-deployment test;
//!   • the manifest drops the memory's rows → the manifest test;
//!   • the HBM ranked by size, or unbounded → the tier test; never applied → the bundle and manifest tests; the
//!     manifest's off-chip list emptied → the manifest test.
//! ════════════════════════════════════════════════════════════════════════════════════════════════
#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};

use logicaffeine_compile::spatial::streamed::Storage;
use logicaffeine_compile::compile::RduModel;
use logicaffeine_compile::codegen_cuda::qwen38::Qwen38Cfg;
use logicaffeine_compile::spatial::ir::{
    walk_tagged, ChoreoOp, SpatialProgram, TensorRole, TensorSpace,
};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::streamed::lower_decode_streamed;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::residency::{
    buffer_footprint, plan_states, tensor_bytes, StateClass,
};
use logicaffeine_compile::spatial::target::{precision_bytes, SpatialTarget};
use logicaffeine_compile::spatial::work::work_of;

fn sn50() -> SpatialTarget {
    RduChip::Sn50.facts().spatial_target()
}

fn composed(src: &str, target: &SpatialTarget) -> SpatialProgram {
    let l = compile_program_to_spatial(src, target).expect("a tensor program");
    l.program
        .unwrap_or_else(|| panic!("the program composes on {}: {:?}", target.name, l.missing))
}

/// A decode step's attention after its cache append: activations (the regrouped queries and result) and carried state
/// (the caches), on the SN50.
fn attention_step() -> String {
    let (ctx, h, kv, hd) = (160, 6, 2, 32);
    format!(
        "## Main\nLet pos be 40.\nLet Kc be a {ctx} by {w} tensor of Real.\nLet Vc be a {ctx} by {w} tensor of Real.\nLet qp be a 1 by {q} tensor of Real.\n\
         Let kp be a 1 by {w} tensor of Real.\nLet vr be a 1 by {w} tensor of Real.\n\
         Set row pos of Kc to kp.\nSet row pos of Vc to vr.\nLet ct be attention of qp over Kc and Vc with {h} heads {kv} groups up to pos.\n",
        w = kv * hd,
        q = h * hd
    )
}

/// A chain of three products over a row of `n`: `h` then `y` intermediates, `z` the answer — `h` dies when `y` is
/// made, `y` lives to the end.
fn chain(n: usize) -> String {
    format!(
        "## Main\nLet x be a 1 by {n} tensor of Real.\nLet A be a {n} by {n} tensor of Real.\nLet B be a {n} by {n} tensor of Real.\n\
         Let C be a {n} by {n} tensor of Real.\nLet h be x matmul A.\nLet y be h matmul B.\nLet z be y matmul C.\n"
    )
}

/// Tile bytes each tensor moves, reads and writes, every loop at its bound.
fn moved(p: &SpatialProgram) -> BTreeMap<String, u64> {
    let tile = (p.tile.rows * p.tile.cols) as u64;
    let mut out = BTreeMap::new();
    for prog in &p.schedule.as_ref().expect("scheduled").programs {
        walk_tagged(&prog.ops, 1, &mut |t, m| {
            if let ChoreoOp::ReadTiles {
                tensor,
                block_h,
                block_w,
                ..
            }
            | ChoreoOp::WriteTiles {
                tensor,
                block_h,
                block_w,
                ..
            } = &t.op
            {
                *out.entry(p.tensor(*tensor).name.clone()).or_default() += m
                    * (*block_h * *block_w) as u64
                    * tile
                    * precision_bytes(p.tensor(*tensor).dtype) as u64;
            }
        });
    }
    out
}

fn space_of(p: &SpatialProgram, name: &str) -> TensorSpace {
    p.tensors
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("a tensor `{name}`"))
        .space
}

// ───────────────────────────── 1. what leaves the chip ─────────────────────────────

#[test]
fn dram_bytes_are_what_leaves_the_chip_and_the_rest_is_on_chip() {
    let target = sn50();
    for (what, src) in [
        ("the attention step", attention_step()),
        ("the product chain", chain(512)),
    ] {
        let p = composed(&src, &target);
        let by = moved(&p);
        let (mut off, mut on) = (0u64, 0u64);
        for (name, b) in &by {
            if space_of(&p, name) == TensorSpace::Sram {
                on += b;
            } else {
                off += b;
            }
        }
        assert!(
            on > 0 && off > 0,
            "{what}: some traffic stays on chip and some leaves it ({on} / {off})"
        );
        let w = work_of(&p, &target);
        assert_eq!(
            w.dram_bytes, off,
            "{what}: DRAM traffic is the traffic of the tensors off the SRAM tier"
        );
        assert_eq!(w.onchip_bytes, on, "{what}: and the SRAM tier's is on chip");
        println!("{what}: {off} bytes leave the chip, {on} stay");
    }
}

// ───────────────────────────── 2. only where the sites reach each other ─────────────────────────────

#[test]
fn an_intermediate_is_on_chip_only_where_the_sites_reach_one_anothers_memory() {
    for (target, crossbar) in [
        (sn50(), true),
        (SpatialTarget::wormhole(), false),
        (SpatialTarget::rtx3060(), false),
    ] {
        let p = composed(&chain(512), &target);
        let acts: Vec<_> = p
            .tensors
            .iter()
            .filter(|t| t.role == TensorRole::Activation)
            .collect();
        assert!(
            !acts.is_empty(),
            "{}: the chain hands intermediates between sections",
            target.name
        );
        for t in acts {
            let want = if crossbar {
                TensorSpace::Sram
            } else {
                TensorSpace::Dram
            };
            assert_eq!(
                t.space,
                want,
                "{}: `{}` on {}",
                target.name,
                t.name,
                if crossbar {
                    "the crossbar's SRAM"
                } else {
                    "the global memory the sites share"
                }
            );
        }
    }
}

// ───────────────────────────── 3. allocated over live ranges ─────────────────────────────

/// The SN50 with `bytes` of SRAM in all.
fn with_sram(bytes: u64) -> SpatialTarget {
    let mut t = sn50();
    t.dram.sram_kib = (bytes / 1024) as usize;
    t
}

#[test]
fn a_live_set_past_the_sram_spills_the_intermediate_used_last_first() {
    let big = composed(&chain(2048), &sn50());
    for t in big
        .tensors
        .iter()
        .filter(|t| t.role == TensorRole::Activation)
    {
        assert_eq!(
            t.space,
            TensorSpace::Sram,
            "`{}` fits the SN50's SRAM",
            t.name
        );
    }
    let (h, y) = (
        tensor_bytes(&big, big.tensors.iter().find(|t| t.name == "h").unwrap()),
        tensor_bytes(&big, big.tensors.iter().find(|t| t.name == "y").unwrap()),
    );
    let buffers: u64 = buffer_footprint(&big).values().copied().max().unwrap_or(0);
    // `h` and `y` are both live where `y` is made; room for both keeps both, room for one less keeps `h`.
    let roomy = composed(
        &chain(2048),
        &with_sram((buffers + h + y).div_ceil(1024) * 1024),
    );
    assert_eq!(
        (space_of(&roomy, "h"), space_of(&roomy, "y")),
        (TensorSpace::Sram, TensorSpace::Sram),
        "room for the peak keeps every intermediate on chip"
    );
    let tight = composed(
        &chain(2048),
        &with_sram((buffers + h + y - 1) / 1024 * 1024),
    );
    assert_ne!(
        space_of(&tight, "y"),
        TensorSpace::Sram,
        "one byte short, the intermediate used last spills"
    );
    assert_eq!(
        space_of(&tight, "h"),
        TensorSpace::Sram,
        "and the one that dies first stays"
    );
    let none = composed(&chain(2048), &with_sram(buffers / 1024 * 1024));
    for name in ["h", "y"] {
        assert_ne!(
            space_of(&none, name),
            TensorSpace::Sram,
            "with no room past the buffers `{name}` is off chip"
        );
    }
}

// ───────────────────────────── 4. a program lowered whole is its own deployment ─────────────────────────────

#[test]
fn a_program_lowered_whole_places_its_state_as_its_own_deployment() {
    let fits = composed(&attention_step(), &sn50());
    let states: Vec<_> = fits
        .tensors
        .iter()
        .filter(|t| t.role == TensorRole::State)
        .map(|t| t.name.clone())
        .collect();
    assert!(!states.is_empty(), "the step carries its caches");
    for name in &states {
        assert_eq!(
            space_of(&fits, name),
            TensorSpace::Sram,
            "`{name}`: the step's own caches fit beside its footprint"
        );
    }
    let reserve: u64 = logicaffeine_compile::spatial::residency::sram_footprint(&fits)
        .values()
        .copied()
        .max()
        .unwrap_or(0);
    let tight = composed(&attention_step(), &with_sram(reserve.div_ceil(1024) * 1024));
    for name in &states {
        assert_ne!(
            space_of(&tight, name),
            TensorSpace::Sram,
            "`{name}`: no room beside the program's own footprint, so off chip"
        );
    }
    let shared = composed(&attention_step(), &SpatialTarget::sn50_rdu());
    assert!(
        logicaffeine_compile::spatial::residency::holds_tensors_on_chip(&SpatialTarget::sn50_rdu()),
        "the SN50 preset reaches its SRAM across the crossbar"
    );
    for name in &states {
        assert_eq!(
            space_of(&shared, name),
            space_of(&fits, name),
            "`{name}`: the preset and the facts' target agree"
        );
    }
}

// ───────────────────────────── 5. the plan ─────────────────────────────

fn class(tensor: &str, chip: u16, instances: usize, bytes: u64, moved: u64) -> StateClass {
    StateClass {
        program: "kind".into(),
        tensor: tensor.into(),
        chip,
        instances,
        bytes,
        moved,
    }
}

#[test]
fn the_plan_keeps_the_state_that_moves_most_per_byte_on_chip_first() {
    const MB: u64 = 1 << 20;
    let classes = vec![
        class("kv", 0, 16, 18 * MB, 18 * MB), // 288 MB, read whole once a step
        class("memory", 0, 48, 2 * MB, 4 * MB), // 96 MB, read and written every step
        class("ring", 0, 48, MB, 3 * MB / 2), // 48 MB
        class("small", 0, 16, MB / 2, MB / 2), // 8 MB, dense 1 like kv
        class("other chip", 1, 48, 2 * MB, 4 * MB),
    ];
    let reserve: BTreeMap<u16, u64> = [(0, 100 * MB), (1, 100 * MB)].into_iter().collect();
    let plan = plan_states(classes, 432 * MB, &reserve);
    let tier = |t: &str| {
        plan.placed
            .iter()
            .find(|(c, _)| c.tensor == t)
            .map(|(_, s)| *s)
            .unwrap()
    };
    assert_eq!(tier("memory"), TensorSpace::Sram, "the densest state first");
    assert_eq!(tier("ring"), TensorSpace::Sram, "then the next");
    assert_ne!(
        tier("kv"),
        TensorSpace::Sram,
        "a class past what is left stays off chip"
    );
    assert_eq!(
        tier("small"),
        TensorSpace::Sram,
        "and a smaller one of the same worth still takes what is left"
    );
    assert_eq!(
        tier("other chip"),
        TensorSpace::Sram,
        "each chip plans its own SRAM"
    );
    assert_eq!(
        plan.on_chip(0),
        96 * MB + 48 * MB + 8 * MB,
        "chip 0 holds exactly what was placed"
    );
    for chip in [0u16, 1] {
        assert!(
            plan.on_chip(chip) + reserve[&chip] <= 432 * MB,
            "chip {chip}: the plan fits beside the reserve"
        );
    }
    // Worth, not size: 100 MB of room, a memory of 80 MB moved twice a step against two caches of 30 MB moved once.
    // Smallest-first would keep the two caches (60 MB a step kept on chip); densest-first keeps the memory (160 MB).
    let plan = plan_states(
        vec![
            class("memory", 0, 1, 80 * MB, 160 * MB),
            class("cache a", 0, 1, 30 * MB, 30 * MB),
            class("cache b", 0, 1, 30 * MB, 30 * MB),
        ],
        100 * MB,
        &BTreeMap::new(),
    );
    let tier = |t: &str| {
        plan.placed
            .iter()
            .find(|(c, _)| c.tensor == t)
            .map(|(_, s)| *s)
            .unwrap()
    };
    assert_eq!(
        (
            tier("memory"),
            tier("cache a") == TensorSpace::Sram,
            tier("cache b") == TensorSpace::Sram
        ),
        (TensorSpace::Sram, false, false),
        "the plan keeps the bytes a step moves most, not the most classes"
    );
}

// ───────────────────────────── 6. a real-width deployment ─────────────────────────────

/// Qwen3.8 at real width, four layers (three Gated-DeltaNet, one attention) and a small vocabulary: every layer kind
/// the model has, at the widths it has.
fn real_width_layers() -> Qwen38Cfg {
    let mut cfg = Qwen38Cfg::real();
    cfg.layers = 4;
    cfg.vocab = 512;
    cfg
}

/// The bundle's plan, checked against its programs: every CARRIED state of every layer kind planned once per layer of
/// the kind, at the tensor's bytes, with the tier its program carries, the whole within the SRAM beside the reserve —
/// and no table a step assembles whole for itself among them.
fn plan_holds(m: &logicaffeine_compile::spatial::streamed::StreamedModel, what: &str) {
    let plan = &m.residency;
    let mut seen = BTreeSet::new();
    for kind in &m.kinds {
        let instances = m
            .layers
            .iter()
            .filter(|l| m.kinds[l.kind].name == kind.name)
            .count();
        for t in kind
            .program
            .tensors
            .iter()
            .filter(|t| t.role == TensorRole::State && !kind.carried.contains(&t.name))
        {
            assert!(
                plan.tier_of(&kind.name, &t.name).is_none(),
                "{what}: `{}` is the {} step's own table, not the deployment's to keep",
                t.name,
                kind.name
            );
        }
        for t in kind
            .program
            .tensors
            .iter()
            .filter(|t| t.role == TensorRole::State && kind.carried.contains(&t.name))
        {
            let (c, tier) = plan
                .placed
                .iter()
                .find(|(c, _)| c.program == kind.name && c.tensor == t.name)
                .unwrap_or_else(|| {
                    panic!("{what}: `{}` of the {} kind is planned", t.name, kind.name)
                });
            assert_eq!(
                c.instances, instances,
                "{what}: `{}` is carried once per {} layer",
                t.name, kind.name
            );
            assert_eq!(
                c.bytes,
                tensor_bytes(&kind.program, t),
                "{what}: `{}`'s bytes are the tensor's",
                t.name
            );
            assert_eq!(
                t.space, *tier,
                "{what}: the {} program carries the plan's tier for `{}`",
                kind.name, t.name
            );
            seen.insert((kind.name.clone(), t.name.clone()));
        }
    }
    assert_eq!(
        seen.len(),
        plan.placed.len(),
        "{what}: the plan covers exactly the carried states"
    );
    assert!(
        plan.on_chip(0) + plan.reserve[&0] <= plan.sram_bytes,
        "{what}: the plan fits the SRAM beside the largest program's own footprint"
    );
    println!(
        "{what}: reserve {} KiB, on chip {} KiB: {:?}",
        plan.reserve[&0] >> 10,
        plan.on_chip(0) >> 10,
        plan.placed
            .iter()
            .map(|(c, s)| format!("{} ×{} {:?}", c.tensor, c.instances, s))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_real_width_decode_bundle_plans_every_carried_state_within_the_sram() {
    let cfg = real_width_layers();
    let roomy = lower_decode_streamed(&cfg.spec(), Storage::AllQ8, 4096, &sn50(), |s| s.to_string())
        .unwrap_or_else(|e| panic!("the bundle lowers: {e}"));
    plan_holds(&roomy, "the SN50");
    assert!(
        roomy
            .residency
            .placed
            .iter()
            .all(|(_, s)| *s == TensorSpace::Sram),
        "four layers' state at batch 1 all fits on the SN50"
    );
    // An SRAM with room for the recurrent state and a quarter of the caches — less than either cache: the memory and
    // the rings, moved more per byte than a cache, stay; both caches stream.
    let total = |prefix: &[&str]| {
        roomy
            .residency
            .placed
            .iter()
            .filter(|(c, _)| prefix.iter().any(|p| c.tensor.starts_with(p)))
            .map(|(c, _)| c.bytes * c.instances as u64)
            .sum::<u64>()
    };
    let (recurrent, caches) = (total(&["St", "Cs"]), total(&["Kc", "Vc"]));
    assert!(
        recurrent > 0 && caches > 0,
        "the model carries recurrent state and caches"
    );
    let tight =
        with_sram((roomy.residency.reserve[&0] + recurrent + caches / 4).div_ceil(1024) * 1024);
    let m = lower_decode_streamed(&cfg.spec(), Storage::AllQ8, 4096, &tight, |s| s.to_string())
        .unwrap_or_else(|e| panic!("the bundle lowers on a smaller SRAM: {e}"));
    plan_holds(&m, "a smaller SRAM");
    for (c, tier) in &m.residency.placed {
        let recurrent = c.tensor.starts_with("St") || c.tensor.starts_with("Cs");
        assert_eq!(
            *tier == TensorSpace::Sram,
            recurrent,
            "a smaller SRAM: `{}` is {tier:?}",
            c.tensor
        );
    }
}

// ───────────────────────────── 7. the manifest tells the host ─────────────────────────────

/// The bundle's manifest names where the host allocates every carried state: each state tensor of each program, its
/// instances, bytes and tier, and the SRAM each chip keeps for them fits beside the reserve.
#[test]
fn the_bundle_manifest_tells_the_host_where_each_state_lives() {
    use logicaffeine_compile::compile::{compile_model_to_rdu_streamed, StepShape};
    let cfg = Qwen38Cfg {
        d: 64,
        dff: 256,
        hd: 64,
        rope_dim: 64,
        gdn_head_dim: 32,
        vocab: 64,
        ..Qwen38Cfg::mini()
    };
    let bundle = compile_model_to_rdu_streamed(&RduModel::qwen38(&cfg),
        RduChip::Sn50,
        "/w",
        Some(StepShape {
            ctx: 64,
            batch: 1,
            tp: 1,
            prefill: None,
        }),
        |_| Ok(()),
    )
    .unwrap_or_else(|e| panic!("the bundle lowers: {e}"));
    let m: serde_json::Value =
        serde_json::from_str(&bundle.manifest).expect("the manifest is JSON");
    let r = &m["residency"];
    let states = r["states"]
        .as_array()
        .expect("the manifest lists the planned states");
    let carried: BTreeSet<String> = m["layers"]
        .as_array()
        .expect("layers")
        .iter()
        .flat_map(|l| l["tensors"].as_array().unwrap().iter())
        .filter(|t| t["role"] == "state")
        .map(|t| t["program"].as_str().unwrap().to_string())
        .collect();
    assert!(!carried.is_empty(), "the decode bundle carries state");
    let planned: BTreeSet<String> = states
        .iter()
        .map(|s| s["tensor"].as_str().unwrap().to_string())
        .collect();
    assert!(
        carried.is_subset(&planned),
        "every carried state is planned: {carried:?} ⊄ {planned:?}"
    );
    let sram = r["sram_bytes_per_chip"]
        .as_u64()
        .expect("the SRAM per chip");
    let reserve = r["reserve_bytes"]["0"].as_u64().expect("chip 0's reserve");
    let on_chip: u64 = states
        .iter()
        .filter(|s| s["tier"] == "sram")
        .map(|s| s["bytes"].as_u64().unwrap() * s["instances"].as_u64().unwrap())
        .sum();
    assert!(
        states
            .iter()
            .all(|s| ["sram", "hbm", "ddr"].contains(&s["tier"].as_str().unwrap_or(""))),
        "a tier is the SRAM, the HBM or the DDR"
    );
    let off: Vec<_> = r["off_chip"]
        .as_array()
        .expect("the manifest lists where every off-chip tensor lives")
        .iter()
        .collect();
    assert!(
        off.iter()
            .any(|t| t["role"] == "weight" && t["tier"] == "hbm"),
        "the weights live in the HBM"
    );
    assert_eq!(
        r["hbm_bytes_per_chip"].as_u64(),
        Some(64 << 30),
        "the SN50's 64 GiB of HBM"
    );
    assert!(
        on_chip + reserve <= sram,
        "the SRAM kept for state fits beside the reserve: {on_chip} + {reserve} > {sram}"
    );
    assert_eq!(r["provenance"], "modeled", "the plan is modeled");
}

// ───────────────────────────── 8. off the chip: the HBM, then the DDR ─────────────────────────────

/// The off-chip side is two tiers on an RDU: its HBM, and the DDR behind it. What leaves the SRAM goes to the HBM while
/// it holds it — the tensor that moves the most per byte first — and the rest to the DDR.
#[test]
fn off_the_chip_the_hbm_holds_what_moves_most_and_the_ddr_the_rest() {
    use logicaffeine_compile::spatial::residency::plan_off_chip;
    const MB: u64 = 1 << 20;
    // 100 MB of HBM: a memory of 70 MB moved twice a step against two 40 MB tables moved once. Smallest-first would
    // keep both tables (80 MB kept off the DDR a step); densest-first keeps the memory (140 MB).
    let classes = vec![
        class("table a", 0, 1, 40 * MB, 40 * MB),
        class("table b", 0, 1, 40 * MB, 40 * MB),
        class("memory", 0, 1, 70 * MB, 140 * MB),
        class("small", 0, 1, 20 * MB, 20 * MB),
        class("other chip", 1, 1, 90 * MB, 90 * MB),
    ];
    let tiers = plan_off_chip(&classes, 100 * MB);
    let tier = |t: &str| tiers[classes.iter().position(|c| c.tensor == t).unwrap()];
    assert_eq!(tier("memory"), TensorSpace::Hbm, "the densest first");
    assert_eq!(
        tier("small"),
        TensorSpace::Hbm,
        "then, of equal worth, the smaller while it fits"
    );
    assert_eq!(
        (tier("table a"), tier("table b")),
        (TensorSpace::Dram, TensorSpace::Dram),
        "past what the HBM holds, the DDR"
    );
    assert_eq!(
        tier("other chip"),
        TensorSpace::Hbm,
        "each chip its own HBM"
    );
    let none = plan_off_chip(&classes, 0);
    assert!(
        none.iter().all(|t| *t == TensorSpace::Dram),
        "a chip with no HBM keeps everything in its DRAM"
    );
}

#[test]
fn a_real_width_bundle_keeps_its_weights_in_the_hbm_and_overflows_to_the_ddr() {
    let cfg = real_width_layers();
    let m = lower_decode_streamed(&cfg.spec(), Storage::AllQ8, 4096, &sn50(), |s| s.to_string())
        .unwrap_or_else(|e| panic!("the bundle lowers: {e}"));
    let programs: Vec<&SpatialProgram> = m
        .embed
        .iter()
        .chain(m.kinds.iter().map(|k| &k.program))
        .chain(std::iter::once(&m.head))
        .collect();
    for p in &programs {
        for t in &p.tensors {
            assert_ne!(
                t.space,
                TensorSpace::Dram,
                "`{}` of `{}`: four real-width layers fit the SN50's 64 GiB of HBM",
                t.name,
                p.name
            );
        }
    }
    // A chip with no HBM keeps everything off the SRAM in its DRAM.
    let mut small = sn50();
    small.dram.hbm_gib = 0;
    let m = lower_decode_streamed(&cfg.spec(), Storage::AllQ8, 4096, &small, |s| s.to_string())
        .unwrap_or_else(|e| panic!("the bundle lowers without HBM: {e}"));
    let tiers: Vec<TensorSpace> = m
        .kinds
        .iter()
        .flat_map(|k| k.program.tensors.iter().map(|t| t.space))
        .collect();
    assert!(
        !tiers.contains(&TensorSpace::Hbm),
        "no HBM, no tensor in it"
    );
}
