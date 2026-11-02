//! Sweep the half-sync accumulator window over many shapes, on every silicon preset and both numerics
//! policies — offline, with no simulator.
//!
//! `spatial_chip_facts_lock::no_emitted_compute_window_packs_more_tiles_than_the_half_sync_accumulator`
//! holds four shapes to this, which is enough to catch a preset that claims the wrong accumulator. It is
//! not enough to prove the PLANNER never exceeds the window for some other reason — a sub-block rule, a
//! spill path, a fused epilogue — and the defect this guards against (a kernel that computes the problem
//! and writes the answer nowhere) is invisible to every structural check the compiler already runs.
//!
//! The L1 finding came from sweeping 2,197 shapes after eight hand-picked ones showed nothing, so this
//! sweeps rather than samples. A violation here is a wrong answer waiting for someone to measure it.
//!
//!   cargo run -q --release -p logicaffeine-compile --features codegen --example probe_acc_window_sweep

use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::lower::{
    tensix::TensixBackend, SpatialArtifact, SpatialBackend,
};
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_source_with};
use logicaffeine_compile::spatial::target::SpatialTarget;

fn gemm(m: usize, n: usize, k: usize) -> String {
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

fn balanced_args(after_open: &str) -> String {
    let (mut depth, mut out) = (1i32, String::new());
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

fn loop_bound_before_pack(body: &str) -> Option<usize> {
    let at = body.find("pack_tile")?;
    let head = &body[..at];
    let open = head.rfind("for (uint32_t")?;
    let cond = head[open..].split('<').nth(1)?.trim_start();
    cond.chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

fn main() {
    // Both parts have a 16-tile DST with an 8-tile half-sync window (their own `tensix_types.h`).
    const HALF: usize = 8;
    let tiles = [1usize, 2, 3, 4, 5, 6, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64];
    let mut checked = 0usize;
    let mut violations = 0usize;
    let mut rects: std::collections::BTreeSet<(usize, usize)> = Default::default();
    let mut windows: std::collections::BTreeSet<usize> = Default::default();

    for target in [SpatialTarget::wormhole(), SpatialTarget::blackhole()] {
        for policy in [NumericsPolicy::Exact, NumericsPolicy::AllowSpillRounding] {
            for &mt in &tiles {
                for &nt in &tiles {
                    for &kt in &[4usize, 16, 64] {
                        let (m, n, k) = (mt * 32, nt * 32, kt * 32);
                        let Ok(tm) = template_from_source_with(&gemm(m, n, k), &target, policy)
                        else {
                            continue;
                        };
                        let Ok(p) = run_passes(tm, &target) else {
                            continue;
                        };
                        let Ok(SpatialArtifact::Tensix(a)) =
                            TensixBackend::new(target.clone()).lower(&p)
                        else {
                            continue;
                        };
                        for kern in a
                            .kernels
                            .iter()
                            .filter(|k| k.source.contains("tile_regs_acquire"))
                        {
                            for window in kern.source.split("tile_regs_acquire").skip(1) {
                                let body =
                                    window.split("tile_regs_release").next().unwrap_or(window);
                                for call in body.split("matmul_block(").skip(1) {
                                    let nums: Vec<usize> = top_level_args(&balanced_args(call))
                                        .into_iter()
                                        .filter_map(|x| {
                                            let x = x.trim();
                                            if !x.starts_with(|c: char| c.is_ascii_digit()) {
                                                return None;
                                            }
                                            x.chars()
                                                .take_while(|c| c.is_ascii_digit())
                                                .collect::<String>()
                                                .parse()
                                                .ok()
                                        })
                                        .collect();
                                    if nums.len() >= 3 {
                                        let (ct, rt) = (nums[nums.len() - 3], nums[nums.len() - 2]);
                                        rects.insert((ct, rt));
                                        if ct * rt > HALF {
                                            violations += 1;
                                            println!("★ FILL {} {m}x{n}x{k} {policy:?}: matmul_block {ct}x{rt} = {} > {HALF}",
                                                     target.name, ct * rt);
                                        }
                                    }
                                }
                                if body.contains("pack_tile") {
                                    let packed = body
                                        .split("pack_tile")
                                        .count()
                                        .saturating_sub(1)
                                        .max(loop_bound_before_pack(body).unwrap_or(0));
                                    windows.insert(packed);
                                    checked += 1;
                                    if packed > HALF {
                                        violations += 1;
                                        println!("★ PACK {} {m}x{n}x{k} {policy:?}: {packed} tiles > {HALF}", target.name, m = m, n = n, k = k);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    println!("\n{checked} compute windows over 2 chips x 2 policies; {violations} violate the half-sync window.");
    println!("window sizes observed: {windows:?}");
    println!("matmul_block rectangles observed: {rects:?}");
}
