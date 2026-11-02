//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! spatial_chip_facts_lock — a chip preset is a claim about silicon, and the vendor publishes the answer.
//!
//! `SpatialTarget::blackhole()` is written as `{ name, sites, acc_tiles, write_ordering, ..wormhole() }`.
//! Struct-update syntax is the right way to say "the same part, three fields different" — and it is also
//! how a second chip silently inherits the first one's PHYSICS. Blackhole took Wormhole's 1464 KiB of L1,
//! its 12 GiB of DRAM and its 64-site SRAM total, none of which are Blackhole's.
//!
//! L1 size is not decoration: `size_buffers` shrinks a program to fit `local_mem_bytes`, and `tile.rs`
//! blocks against it. Under-reporting Blackhole's L1 by 72 KiB makes the compiler choose smaller blocks
//! than the chip can hold, forever, on a part we claim to target — a pessimization nothing else in the
//! suite can see, because every ttsim row ever measured is Wormhole.
//!
//! So the presets are held to the vendor's own SoC descriptors. Two tiers, because the descriptors live
//! in the tt-metal tree and are not on every machine:
//!
//!   1. `benchmarks/silicon/tt_soc_facts.json` — extracted from those yaml files by
//!      `scripts/accel/tt-soc-facts.py`, carrying each descriptor's path and sha256. Read on EVERY build.
//!   2. When the SDK is present, `scripts/accel/tt-soc-facts.py --check` re-extracts and must still agree, so
//!      the repo's copy of a vendor fact cannot rot while the tests stay green.
//!
//! ★ The one thing an extractor must not do is guess. The first version of that script hand-rolled a
//! regex over the yaml and reported FIVE DRAM channels where there are six — a wrong fact, checked in,
//! that would then have been locked. It parses the file with a YAML parser now.
//!
//!   cargo nextest run -p logicaffeine-tests -E 'binary(spatial_chip_facts_lock)'
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::target::SpatialTarget;
use std::path::{Path, PathBuf};

mod common;

const FACTS: &str = "benchmarks/silicon/tt_soc_facts.json";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn facts() -> serde_json::Value {
    let p = repo_root().join(FACTS);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "{}: {e} — regenerate with `python3 scripts/accel/tt-soc-facts.py`",
            p.display()
        )
    });
    serde_json::from_str(&s).expect("tt_soc_facts.json is JSON")
}

/// The presets that claim to BE a Tenstorrent part. `wormhole_ttsim()` is deliberately absent: it is the
/// simulator's calibrated twin, not a claim about silicon.
fn silicon_presets() -> Vec<SpatialTarget> {
    vec![SpatialTarget::wormhole(), SpatialTarget::blackhole()]
}

/// The GEMM the measurement harness compiles, so the plan under test is the plan that ships.
fn gemm_source(m: usize, n: usize, k: usize) -> String {
    format!(
        r#"## Main
Let rows be {m}.
Let cols be {n}.
Let mid be {k}.
Let mutable a be a new Seq of Int.
Let mutable b be a new Seq of Int.
Let mutable c be a new Seq of Int.
Let mutable p be 0.
While p is less than rows * mid:
    Push (p % 7) - 3 to a.
    Set p to p + 1.
Set p to 0.
While p is less than mid * cols:
    Push (p % 11) - 5 to b.
    Set p to p + 1.
Set p to 0.
While p is less than rows * cols:
    Push 0 to c.
    Set p to p + 1.
Let mutable i be 0.
While i is less than rows:
    Let mutable k be 0.
    While k is less than mid:
        Let mutable j be 0.
        While j is less than cols:
            Let idx be i * cols + j + 1.
            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.
            Set j to j + 1.
        Set k to k + 1.
    Set i to i + 1.
Let mutable checksum be 0.
Let mutable q be 1.
While q is at most rows * cols:
    Set checksum to checksum + item q of c.
    Set q to q + 1.
Show checksum.
"#
    )
}

fn u(v: &serde_json::Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or_else(|| {
        panic!("tt_soc_facts.json: no `{k}` (regenerate with scripts/accel/tt-soc-facts.py)")
    })
}

/// Every field a preset shares with the descriptor is the descriptor's.
///
/// NON-VACUITY: this is the test that found the defect it exists to prevent — `blackhole()` failed three
/// of the four properties on its first run (L1 1,499,136 vs 1,572,864; SRAM 93,696 KiB vs 199,680;
/// DRAM 12 GiB vs 31), while `wormhole()` passed all four. A property that only one of two presets
/// satisfies is not a tautology, and the one that satisfies it shows the property is the right one.
#[test]
fn every_silicon_preset_agrees_with_the_vendor_soc_descriptor() {
    let f = facts();
    for t in silicon_presets() {
        let c = &f["chips"][t.name];
        assert!(
            !c.is_null(),
            "no vendor facts for preset `{}` — add it to CHIPS in scripts/accel/tt-soc-facts.py",
            t.name
        );

        // 1. L1 per worker: a physical per-core fact, with no harvesting ambiguity to hide behind.
        assert_eq!(
            t.site.local_mem_bytes as u64,
            u(c, "worker_l1_bytes"),
            "{}: the preset budgets {} B of L1 per site; {} says {} B. `size_buffers` shrinks to this \
             number and `tile.rs` blocks against it, so a wrong one is a permanent pessimization (or an \
             overflow) on every program compiled for this chip.",
            t.name, t.site.local_mem_bytes, c["source"], u(c, "worker_l1_bytes")
        );

        // 2. A preset may model FEWER sites than the part has (harvesting, dispatch reservation), never more.
        let sites = (t.sites.x * t.sites.y) as u64;
        assert!(
            sites <= u(c, "functional_workers"),
            "{}: the preset claims {sites} sites; the part has {} functional workers",
            t.name,
            u(c, "functional_workers")
        );

        // 3. Total on-chip SRAM is the mesh we actually model, at this chip's L1 size — not the previous
        //    chip's. This is the property Blackhole's `..wormhole()` broke while looking correct.
        assert_eq!(
            t.dram.sram_kib as u64,
            sites * u(c, "worker_l1_bytes") / 1024,
            "{}: sram_kib must be the modelled site count ({sites}) times this chip's L1 ({} B)",
            t.name,
            u(c, "worker_l1_bytes")
        );

        // 4. ★ THE ADDRESSABLE ACCUMULATOR IS THE HALF-SYNC WINDOW, NOT THE REGISTER FILE.
        //
        // `DEST_NUM_TILES_FP16` is the whole DST — 16 tiles on BOTH parts, which have identical
        // 16x16 dest faces. Our compute kernels bracket their math with `tile_regs_acquire` /
        // `tile_regs_commit`, and that is `DstSync::SyncHalf`: the packer drains one half while math
        // fills the other, so a single acquire reaches only eight.
        //
        // Wormhole's preset had always said 8. Blackhole's said 16, and nothing noticed until a shape
        // appeared whose plan could USE more than eight — at 2048x1024x512 the planner picked a 4x4
        // sub-block, the kernel packed sixteen tiles through an eight-tile window, and the program
        // returned PCC 0.872 having retired EXACTLY the right arithmetic. 640^3 had passed on the same
        // chip because its sub-block was four tiles. A capacity a preset claims and the part does not
        // offer is not a pessimization like the L1 one above; it is a wrong answer.
        assert_eq!(
            t.site.acc_tiles as u64,
            u(c, "dest_num_tiles_fp16_half"),
            "{}: acc_tiles must be the half-sync window ({} of the {}-tile DST in {}), because the \
             emitted kernels use tile_regs_acquire/commit",
            t.name, u(c, "dest_num_tiles_fp16_half"), u(c, "dest_num_tiles_fp16"), c["dest_types_source"]
        );

        // 5. DRAM capacity is the descriptor's channel count times its bank size.
        assert_eq!(
            t.dram.ddr_gib as u64,
            u(c, "dram_total_gib"),
            "{}: the preset claims {} GiB of DRAM; {} channels x {} B = {} GiB",
            t.name,
            t.dram.ddr_gib,
            u(c, "dram_channels"),
            u(c, "dram_bank_size_bytes"),
            u(c, "dram_total_gib")
        );
    }
}

/// Every chip with vendor facts has a preset, and every preset is covered — so the table cannot be
/// satisfied by shrinking it, and a new chip cannot be added without its facts.
#[test]
fn the_fact_table_and_the_preset_list_cover_each_other() {
    let f = facts();
    let chips: Vec<String> = f["chips"]
        .as_object()
        .expect("chips")
        .keys()
        .cloned()
        .collect();
    assert!(
        chips.len() >= 2,
        "at least Wormhole and Blackhole: {chips:?}"
    );
    let presets: Vec<&str> = silicon_presets().iter().map(|t| t.name).collect();
    for c in &chips {
        assert!(
            presets.contains(&c.as_str()),
            "vendor facts for `{c}` with no SpatialTarget preset"
        );
    }
    for p in &presets {
        assert!(
            chips.iter().any(|c| c == p),
            "preset `{p}` has no vendor facts"
        );
    }
    // Provenance, not just numbers: a fact with no traceable source is a guess with a filename.
    for c in &chips {
        let e = &f["chips"][c];
        assert!(
            e["source"]
                .as_str()
                .is_some_and(|s| s.ends_with("_arch.yaml")),
            "{c}: source must name the descriptor"
        );
        assert_eq!(
            e["sha256"].as_str().map(str::len),
            Some(64),
            "{c}: the descriptor's sha256"
        );
        assert!(
            e["arch_name"].as_str().is_some_and(|s| !s.is_empty()),
            "{c}: the descriptor's own arch_name"
        );
    }
}

/// When the SDK is here, the checked-in facts are re-derived from the vendor descriptors and must still
/// be true. This is what stops tier 1 from locking a stale copy of someone else's file.
#[test]
fn the_checked_in_facts_are_still_true_of_the_vendor_descriptors() {
    // ★ THE HONEST SKIP CHANNEL, not `eprintln!("skipping…"); return;`. A test that prints SKIP and
    // returns reports PASS to cargo though it never ran — `skip_budget_lock` counts those as dishonest,
    // and it is right to: the tier-1 facts above ARE still locked, but THIS test's claim (that the
    // checked-in facts still match the vendor descriptors) went unverified, and a silent PASS says
    // otherwise. `require_tool` emits the greppable `LOGOS_TOOL_SKIP: ttsim` marker and, under
    // `LOGOS_REQUIRE_TOOLS=ttsim`, panics instead — so the release gate can demand this ran.
    if !common::require_tool("ttsim") {
        return;
    }
    let out = std::process::Command::new("python3")
        .arg(repo_root().join("scripts/accel/tt-soc-facts.py"))
        .arg("--check")
        .current_dir(repo_root())
        .output()
        .expect("run scripts/accel/tt-soc-facts.py");
    assert!(
        out.status.success(),
        "{FACTS} no longer matches the vendor descriptors:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// ★ THE 72 KiB IS WORTH 8x THE DRAM TRAFFIC — the correction is not bookkeeping.
///
/// A plan is pruned when its footprint exceeds `local_mem_bytes`, so the budget decides which plans the
/// search may even rank. At 1280x2048x1280 the plan Blackhole actually wants occupies **1,556,480 B**,
/// inside the chip's 1536 KiB L1 and 57,344 B past Wormhole's 1464 KiB, so under the inherited budget the
/// search cannot rank it and ships a plan that moves 110,100,480 DRAM bytes where 13,762,560 would do.
///
/// Swept over {512, 640, 768, 1024, 1280, 1536, 2048}³ (343 shapes), sixteen choose a different plan on the
/// corrected budget; thirteen of those are plans the inherited budget prunes, and every one moves less DRAM —
/// median -85.06%, best -87.50%, at this shape. This test pins the sharpest so the constant cannot quietly
/// regress to Wormhole's.
///
/// NON-VACUITY: the first two assertions are what make the third mean anything — they say the chosen
/// plan is one the inherited budget WOULD HAVE REJECTED. Raising `local_mem_bytes` back to 1464 KiB
/// fails all three.
#[test]
fn blackholes_own_l1_is_load_bearing_not_decoration() {
    use logicaffeine_compile::spatial::ir::NumericsPolicy;
    use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
    use logicaffeine_compile::spatial::passes::tile;
    use logicaffeine_compile::spatial::pipeline::template_from_spec;
    use logicaffeine_compile::spatial::work::Objective;
    use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};

    let bh = SpatialTarget::blackhole();
    let mut inherited = bh.clone();
    inherited.site.local_mem_bytes = 1464 * 1024;

    // External data: an exact reduction over operands of unknown range keeps its order, so the plan holds the whole
    // K panel in one block and its footprint is what the local memory has to carry. (Integer operands with a known
    // bound split K in place instead, and fit either budget.)
    let spec = MatmulSpec {
        m: 1280,
        n: 2048,
        k: 1280,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let names = MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    };
    let pick = |t: &SpatialTarget| {
        let p = template_from_spec(
            &spec,
            &names,
            (&Fill::Unknown, &Fill::Unknown),
            (None, None),
            NumericsPolicy::Exact,
            t,
        )
        .expect("the GEMM template");
        tile::best_candidate_for(&p, t, Objective::default()).expect("a legal plan")
    };
    let (small, real) = (pick(&inherited), pick(&bh));

    assert!(
        real.footprint_bytes > inherited.site.local_mem_bytes,
        "the plan Blackhole chooses ({} B) must be one Wormhole's budget ({} B) would have pruned — \
         otherwise this test proves nothing about the budget",
        real.footprint_bytes, inherited.site.local_mem_bytes
    );
    assert!(
        real.footprint_bytes <= bh.site.local_mem_bytes,
        "and it must fit the real chip: {} B in {} B",
        real.footprint_bytes,
        bh.site.local_mem_bytes
    );
    assert!(
        real.work.dram_bytes < small.work.dram_bytes,
        "the plan the correct budget unlocks must move LESS through the shared resource: {} vs {} bytes",
        real.work.dram_bytes, small.work.dram_bytes
    );
}

/// ★ AND THE EMITTED KERNEL IS HELD TO THE SAME VENDOR FACT, INDEPENDENTLY OF THE PRESET.
///
/// The assertion above stops a preset from claiming an accumulator the part does not have. This one
/// stops the consequence even if that assertion is ever wrong or removed: whatever any descriptor says,
/// a compute kernel bracketed by `tile_regs_acquire` / `tile_regs_release` may not pack more tiles in
/// one window than `DEST_NUM_TILES_FP16_HALF`, because the packer is draining the other half.
///
/// Two independent paths to the same number matter here because a single one already failed. Blackhole's
/// preset said 16, the planner believed it, the printer printed `pack_tile` sixteen times between one
/// acquire and its release, nvcc and the tt-metal build were perfectly happy, and the program returned
/// PCC 0.872 on the simulator — the FIRST thing in the whole toolchain to object. A wrong constant should
/// not need a simulator run to be caught.
///
/// NON-VACUITY: the sweep must find shapes whose plans differ in sub-block size (asserted below), and
/// this test fails on the Blackhole preset as it stood before the fix — that program packed 16.
#[test]
fn no_emitted_compute_window_packs_more_tiles_than_the_half_sync_accumulator() {
    use logicaffeine_compile::spatial::ir::NumericsPolicy;
    use logicaffeine_compile::spatial::lower::{
        tensix::TensixBackend, SpatialArtifact, SpatialBackend,
    };
    use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_source_with};

    let f = facts();
    let mut seen: Vec<(String, String, String, usize)> = Vec::new();
    // ★ The math-side assertion parses `matmul_block`'s argument list. A parse that silently matches
    // nothing asserts nothing, so the rectangles it found are collected and checked below — the same
    // vacuity guard as the windows, which has already refused two degenerate sweeps of my own.
    let mut rects: std::collections::BTreeSet<(usize, usize)> = Default::default();
    let mut skipped: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for t in silicon_presets() {
        let half = u(&f["chips"][t.name], "dest_num_tiles_fp16_half") as usize;
        // ★ BOTH NUMERICS POLICIES. Exact fp32 HALVES the accumulator (partials must survive packing),
        // so sweeping only the default hid every full-width window behind the halved one — all eight
        // shapes reported four tiles and the guard below refused the result, correctly. The measured
        // rows in the ledger are `--fp32 0`, which is the relaxed path, so the exact-only sweep was not
        // even looking at the plans that ship.
        for ((m, n, k), numerics) in [
            (640usize, 640usize, 640usize),
            (2048, 1024, 512),
            (512, 512, 512),
            (2048, 2048, 512),
        ]
        .into_iter()
        .flat_map(|shape| {
            [
                (shape, NumericsPolicy::Exact),
                (shape, NumericsPolicy::AllowSpillRounding),
            ]
        }) {
            let Ok(tm) = template_from_source_with(&gemm_source(m, n, k), &t, numerics) else {
                skipped.push(format!("{} {m}x{n}x{k}: template", t.name));
                continue;
            };
            let Ok(p) = run_passes(tm, &t) else {
                skipped.push(format!("{} {m}x{n}x{k}: passes", t.name));
                continue;
            };
            let Ok(SpatialArtifact::Tensix(a)) = TensixBackend::new(t.clone()).lower(&p) else {
                skipped.push(format!("{} {m}x{n}x{k}: lowering", t.name));
                continue;
            };
            for kern in a
                .kernels
                .iter()
                .filter(|kern| kern.source.contains("tile_regs_acquire"))
            {
                for window in kern.source.split("tile_regs_acquire").skip(1) {
                    let body = window.split("tile_regs_release").next().unwrap_or(window);
                    // ★ THE MATH SIDE FIRST, because it is where the tiles are WRITTEN. `matmul_block`'s
                    // `rt_dim x ct_dim` is the accumulator rectangle it fills, and a kernel could fill
                    // sixteen and pack eight — wrong in the same way and invisible to the check above,
                    // which counts only what leaves. Both ends are held to the one vendor number.
                    for call in body.split("matmul_block(").skip(1) {
                        // ★ BALANCED PARENS, not the first `)`. The operand indices are expressions —
                        // `matmul_block(CB_IN0, CB_IN1, (sh * 4u) * 16u + kk, ...)` — so splitting at the
                        // first `)` truncates the argument list after the third argument and finds no
                        // dimensions at all. This assertion silently matched NOTHING and passed; the
                        // rectangle-coverage guard below is the only reason that was visible.
                        let args = balanced_args(call);
                        let nums: Vec<usize> = top_level_args(&args)
                            .into_iter()
                            .filter_map(|a| {
                                let a = a.trim();
                                if !a.starts_with(|c: char| c.is_ascii_digit()) {
                                    return None;
                                }
                                let d: String =
                                    a.chars().take_while(|c| c.is_ascii_digit()).collect();
                                d.parse().ok()
                            })
                            .collect();
                        // `matmul_block(in0, in1, a_idx, b_idx, dst, transpose, ct_dim, rt_dim, kt_dim)`
                        if nums.len() >= 3 {
                            let (ct, rt) = (nums[nums.len() - 3], nums[nums.len() - 2]);
                            rects.insert((ct, rt));
                            assert!(
                                ct * rt <= half,
                                "{} at {m}x{n}x{k} ({numerics:?}), kernel `{}`: matmul_block fills a \
                                 {ct}x{rt} = {} tile accumulator rectangle, but the half-sync window is \
                                 {half}. The math side overflows DST even when the pack side does not.",
                                t.name, kern.name, ct * rt
                            );
                        }
                    }
                    // `pack_tile(i, CB)` inside a `for (uint32_t i = 0; i < N; ++i)` packs N tiles.
                    let packed = match body.find("pack_tile") {
                        None => continue,
                        Some(_) => body
                            .split("pack_tile")
                            .count()
                            .saturating_sub(1)
                            .max(loop_bound_before_pack(body).unwrap_or(0)),
                    };
                    assert!(
                        packed <= half,
                        "{} at {m}x{n}x{k} ({numerics:?}), kernel `{}`: {packed} tiles packed between one \
                         `tile_regs_acquire` and its release, but {} says the half-sync accumulator holds \
                         {half}. The packer drains one half while math fills the other, so a window wider \
                         than the half writes the answer nowhere.",
                        t.name, kern.name, f["chips"][t.name]["dest_types_source"]
                    );
                    seen.push((
                        t.name.to_string(),
                        format!("{m}x{n}x{k} {numerics:?}"),
                        kern.name.clone(),
                        packed,
                    ));
                    checked += 1;
                }
            }
        }
    }
    assert!(
        checked >= 4,
        "the sweep must actually lower compute windows: {checked} (skipped: {skipped:?})"
    );
    assert!(
        rects.len() >= 2,
        "the matmul_block argument parse found {} distinct accumulator rectangles {rects:?} — a parse \
         that matches nothing asserts nothing",
        rects.len()
    );
    let windows: std::collections::BTreeSet<usize> = seen.iter().map(|(_, _, _, w)| *w).collect();
    assert!(
        windows.len() >= 2,
        "the shapes must differ in window size or this proves nothing. Observed {windows:?} over:\n{}\nskipped: {skipped:?}",
        seen.iter().map(|(c, s, k, w)| format!("  {c:12} {s:16} {k:22} {w} tiles")).collect::<Vec<_>>().join("\n")
    );
}

/// The text of a call's argument list, up to the `)` that balances its opening paren.
fn balanced_args(after_open: &str) -> String {
    let mut depth = 1i32;
    let mut out = String::new();
    for c in after_open.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        out.push(c);
    }
    out
}

/// Split an argument list on the commas that are not inside nested parens.
fn top_level_args(args: &str) -> Vec<String> {
    let (mut depth, mut cur, mut out) = (0i32, String::new(), Vec::new());
    for c in args.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

/// The trip count of the `for` loop immediately preceding a `pack_tile`, when the printer emits one.
///
/// ★ The first version took the condition's LEADING digits by stripping non-digits from the END of the
/// remaining string — which runs past the bound and into the rest of the kernel. Every window then
/// measured the same size and the test's own non-vacuity guard refused it, which is what that guard is
/// for: a measurement that cannot vary is not a measurement.
fn loop_bound_before_pack(body: &str) -> Option<usize> {
    let at = body.find("pack_tile")?;
    let head = &body[..at];
    let open = head.rfind("for (uint32_t")?;
    let cond = head[open..].split('<').nth(1)?.trim_start();
    let digits: String = cond.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}
