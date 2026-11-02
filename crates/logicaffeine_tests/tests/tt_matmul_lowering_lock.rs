//! tt_matmul_lowering_lock — proves the Tensix matrix-engine kernel is GENUINELY COMPILED from a LOGOS
//! program, not a hand-written template. The compiler recognizes an ordinary triple-loop matmul (affine
//! index analysis), extracts its dimensions, and lowers to a tiled `matmul_tiles` kernel whose bounds are
//! the *source program's* dimensions. The genuineness contract, enforced here:
//!   • a real LOGOS matmul is RECOGNIZED and its (M,N,K) extracted from the source;
//!   • the emitted kernel is a FUNCTION of the source — change the dims, the kernel's tile bounds change;
//!   • a non-matmul program, and a triple-loop with a NON-GEMM index layout, are both REFUSED (typed);
//!   • the recognized program actually runs on the VM (the numeric anchor).
//! A template can satisfy none of these. Run:
//!   `cargo nextest run -p logicaffeine-compile -E 'binary(tt_matmul_lowering_lock)'`

#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_tt::matmul_lower::{
    choose_strategy, choose_tiling, dram_read_tiles, emit_matmul_vendor_dispatch, tune_tiling,
    tune_tiling_with, ChipKnobs, ComputeKnobs, MathFidelity, MatmulReject, MatmulStrategy,
    TileConfig, MAX_DST_TILES, MAX_GRID,
};
use logicaffeine_compile::compile::{compile_to_tenstorrent_matmul, vm_outcome};
use logicaffeine_compile::spatial::ir::{ComputeOp, StageKind, TensorRole};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::MatmulSpec;

/// A row-major GEMM `C(M×N) = A(M×K)·B(K×N)` as an ordinary LOGOS program (distinct dims, deterministic
/// fill, checksum) — the kind of code a user writes, with no matmul primitive.
fn gemm_program(m: usize, n: usize, k: usize) -> String {
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
    Push (p % 5) - 2 to b.
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

#[test]
fn recognizes_a_logos_matmul_and_extracts_its_dimensions_from_the_source() {
    // Distinct M, N, K so a template that hardcodes one shape could not fake this.
    let (spec, _kernel) = compile_to_tenstorrent_matmul(&gemm_program(64, 128, 96))
        .expect("a genuine triple-loop matmul must be recognized");
    assert_eq!(
        (spec.m, spec.n, spec.k),
        (64, 128, 96),
        "dims must come from the source"
    );
    // Tile counts (32×32 tiles) derived from the recognized dims.
    assert_eq!((spec.mt(), spec.nt(), spec.kt()), (2, 4, 3));
}

#[test]
fn the_emitted_kernel_is_a_function_of_the_source_not_a_fixed_string() {
    let (_s1, k_small) = compile_to_tenstorrent_matmul(&gemm_program(32, 32, 32)).unwrap();
    let (_s2, k_big) = compile_to_tenstorrent_matmul(&gemm_program(128, 96, 64)).unwrap();
    // Different source dimensions ⇒ different emitted kernel. A template cannot do this.
    assert_ne!(
        k_small, k_big,
        "the kernel must change with the source dims"
    );
    // ★ THE TILE COUNTS COME FROM THE PROGRAM, NOT FROM THE KERNEL'S TEXT. These were
    // `k_small.contains("Mt = 1")` — template-era markers that the IR printer does not emit, and that a
    // comment could satisfy anyway. The claim they were making ("the tile geometry is the source's")
    // is a structural fact, so it is now read structurally: the lowered program's own tensor tile
    // grids. `assert_ne!(k_small, k_big)` above still holds the other half — that the kernel TEXT
    // genuinely varies with the shape.
    for (src, want) in [
        (gemm_program(32, 32, 32), (1usize, 1usize)),
        (gemm_program(128, 96, 64), (4, 3)),
    ] {
        let p = compile_to_spatial(&src, &SpatialTarget::wormhole()).expect("the GEMM must lower");
        let out = p
            .tensors
            .iter()
            .find(|t| t.role == TensorRole::Output)
            .expect("an output tensor");
        assert_eq!(
            (out.tiles.rows_t, out.tiles.cols_t),
            want,
            "the output tile grid must be the source's dimensions in whole tiles"
        );
    }
    // ★ IT DRIVES THE MATRIX ENGINE — ASKED OF THE PROGRAM, NOT OF THE TEXT. This used to require the
    // substring `matmul_tiles`, which pinned ONE spelling of the instruction: the IR picks
    // `matmul_block` where the target advertises the sub-block API (Wormhole does), issuing one call
    // per sub-block instead of `osh·osw·ibw` separate ones. That is the better instruction, and the old
    // assertion would have rejected it — a test forbidding an improvement because it named a mnemonic.
    let p = compile_to_spatial(&gemm_program(128, 96, 64), &SpatialTarget::wormhole())
        .expect("the GEMM must lower");
    assert!(
        p.stages
            .iter()
            .any(|s| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. }))),
        "the lowered program must carry the matrix-engine intrinsic"
    );
    // These needles are genuinely ABOUT the emitted text — the register-window protocol every Tensix
    // compute kernel must open, close and pack through, whatever op it runs.
    for needle in [
        "tile_regs_acquire",
        "pack_tile",
        "compute_kernel_hw_startup",
    ] {
        assert!(k_big.contains(needle), "lowered kernel missing `{needle}`");
    }
    assert!(
        k_big.contains("matmul_block") || k_big.contains("matmul_tiles"),
        "the kernel must issue the matrix-engine op in one of its two spellings: {k_big}"
    );
}

#[test]
fn a_non_matmul_program_is_refused_typed() {
    // No loop nest at all.
    let src = "## Main\n    Let x be 5.\n    Show x + 3.\n";
    let err = compile_to_tenstorrent_matmul(src).expect_err("a non-matmul must be refused");
    assert!(
        err.contains("not a tile-matmul"),
        "refusal must be typed + explained, got: {err}"
    );
}

#[test]
fn a_triple_loop_with_a_non_gemm_index_layout_is_refused() {
    // Break B's index from the row-major GEMM layout `k*cols + j` to `i*cols + j` (wrong contraction axis).
    let good = gemm_program(64, 64, 64);
    let bad = good.replace(
        "item (k * cols + j + 1) of b",
        "item (i * cols + j + 1) of b",
    );
    assert_ne!(good, bad, "the test must actually perturb the layout");
    let err =
        compile_to_tenstorrent_matmul(&bad).expect_err("a non-GEMM index layout must be refused");
    assert!(err.contains("not a tile-matmul"), "got: {err}");
    // And specifically because the layout check failed.
    let via_api = logicaffeine_compile::compile::compile_to_tenstorrent_matmul(&bad);
    if let Err(msg) = via_api {
        assert!(
            msg.contains(&format!("{:?}", MatmulReject::NotGemmLayout)),
            "expected NotGemmLayout, got: {msg}"
        );
    }
}

#[test]
/// **The autotuner's config must be legal for the VENDOR op at every shape, not just the one we measured.**
///
/// `ttnn.matmul` refuses a `MatmulMultiCoreReuseMultiCastProgramConfig` whose output-block count along an
/// axis exceeds the compute grid on that axis (`matmul_device_operation.cpp`: `num_blocks_x <= grid.x`). The
/// tuner satisfies that by construction — it sets the grid to tiles divided by the per-core block — and this
/// holds it there across a sweep of shapes, because a config the vendor rejects is a config we cannot
/// compare against. (Found the hard way: the measurement harness once defaulted the dispatch config to one
/// shape's tiling and produced an illegal 8-blocks-on-a-5-column-grid request at another.)
#[test]
fn the_autotuned_config_is_legal_for_the_vendor_op_at_every_shape() {
    let chip = ChipKnobs::wormhole();
    let knobs = ComputeKnobs::default();
    let mut checked = 0usize;
    for &(m, n, k) in &[
        (640usize, 640usize, 640usize),
        (2048, 1024, 512),
        (1024, 1024, 1024),
        (512, 512, 512),
        (2048, 2048, 1024),
        (256, 1024, 512),
        (1024, 256, 2048),
        (320, 640, 960),
        (1536, 768, 384),
    ] {
        let spec = MatmulSpec {
            m,
            n,
            k,
            a: Default::default(),
            b: Default::default(),
            c: Default::default(),
        };
        let cfg = tune_tiling_with(&spec, &chip, &knobs);
        let (mt, nt) = (m / 32, n / 32);
        assert!(
            cfg.per_core_m >= 1 && cfg.per_core_n >= 1,
            "{m}x{n}x{k}: a per-core block is at least one tile"
        );
        // The vendor's constraint, both axes, exactly as its device op states it.
        assert_eq!(
            nt / cfg.per_core_n,
            cfg.grid_x,
            "{m}x{n}x{k}: output blocks along x must equal the grid columns"
        );
        assert_eq!(
            mt / cfg.per_core_m,
            cfg.grid_y,
            "{m}x{n}x{k}: output blocks along y must equal the grid rows"
        );
        assert!(
            nt / cfg.per_core_n <= cfg.grid_x,
            "{m}x{n}x{k}: num_blocks_x <= grid.x"
        );
        assert!(
            mt / cfg.per_core_m <= cfg.grid_y,
            "{m}x{n}x{k}: num_blocks_y <= grid.y"
        );
        // The blocking must actually tile the shape, and fit the chip.
        assert_eq!(
            nt % cfg.per_core_n,
            0,
            "{m}x{n}x{k}: the per-core block tiles N"
        );
        assert_eq!(
            mt % cfg.per_core_m,
            0,
            "{m}x{n}x{k}: the per-core block tiles M"
        );
        assert!(
            cfg.grid_x <= chip.grid_x && cfg.grid_y <= chip.grid_y,
            "{m}x{n}x{k}: the grid fits the chip"
        );
        assert!(
            cfg.dst_tiles() <= chip.max_dst_tiles,
            "{m}x{n}x{k}: the sub-block fits DST"
        );
        assert_eq!(
            cfg.per_core_m % cfg.out_subblock_h,
            0,
            "{m}x{n}x{k}: the sub-block tiles the per-core block"
        );
        assert_eq!(
            cfg.per_core_n % cfg.out_subblock_w,
            0,
            "{m}x{n}x{k}: the sub-block tiles the per-core block"
        );
        checked += 1;
    }
    assert!(
        checked >= 9,
        "the sweep covers square, tall, wide and non-square shapes"
    );
}

#[test]
fn the_autotuner_chooses_a_legal_tiling_from_the_recognized_shape() {
    // The compiler auto-tunes the blocking from the recognized dims (what the vendor hand-tunes).
    let (spec, _) = compile_to_tenstorrent_matmul(&gemm_program(640, 640, 640)).unwrap();
    let cfg = choose_tiling(&spec);
    // Legal on the hardware: DST holds ≤ 8 output tiles; the grid is ≤ 8×8.
    assert!(
        cfg.dst_tiles() <= MAX_DST_TILES,
        "sub-block {} exceeds DST capacity",
        cfg.dst_tiles()
    );
    assert!(
        cfg.grid_x <= MAX_GRID && cfg.grid_y <= MAX_GRID,
        "grid {}×{} too large",
        cfg.grid_x,
        cfg.grid_y
    );
    // Tiles the shape exactly (no remainder) so every core owns a whole divisor block.
    assert_eq!(spec.mt() % cfg.per_core_m, 0, "per_core_m must divide Mt");
    assert_eq!(spec.nt() % cfg.per_core_n, 0, "per_core_n must divide Nt");
    assert_eq!(
        cfg.per_core_m % cfg.out_subblock_h,
        0,
        "sub-block must divide the per-core block"
    );
    assert_eq!(cfg.per_core_n % cfg.out_subblock_w, 0);
    assert_eq!(spec.kt() % cfg.in0_block_w, 0, "in0_block_w must divide Kt");
    // It actually uses the parallelism (more than one core) and fills the DST for max reuse.
    assert!(
        cfg.num_cores() > 1,
        "autotuner should spread across cores, got {}",
        cfg.num_cores()
    );
    assert_eq!(
        cfg.dst_tiles(),
        MAX_DST_TILES,
        "for a big square GEMM the autotuner should fill the DST"
    );
}

#[test]
fn the_autotuner_adapts_to_the_chip_knobs() {
    // The SAME matmul tunes to a DIFFERENT, faster tiling on a chip with more cores / a bigger DST — the
    // "knobs let us be fastest per chip" property.
    let (spec, _) = compile_to_tenstorrent_matmul(&gemm_program(640, 640, 640)).unwrap();
    let wh = tune_tiling(&spec, &ChipKnobs::wormhole());
    let bh = tune_tiling(&spec, &ChipKnobs::blackhole());
    assert_ne!(
        (
            wh.per_core_m,
            wh.per_core_n,
            wh.out_subblock_h,
            wh.out_subblock_w
        ),
        (
            bh.per_core_m,
            bh.per_core_n,
            bh.out_subblock_h,
            bh.out_subblock_w
        ),
        "different chips must yield different tilings"
    );
    // Blackhole's bigger 13×10 grid lets the smart lattice spread across MORE cores (less per-core work).
    assert!(
        bh.num_cores() > wh.num_cores(),
        "bigger grid ⇒ more parallelism: bh {} vs wh {}",
        bh.num_cores(),
        wh.num_cores()
    );
    // Each is legal on ITS chip: DST holds the sub-block, grid fits.
    for (cfg, chip) in [(wh, ChipKnobs::wormhole()), (bh, ChipKnobs::blackhole())] {
        assert!(
            cfg.dst_tiles() <= chip.max_dst_tiles,
            "{}: subblock exceeds DST",
            chip.name
        );
        assert!(
            cfg.grid_x <= chip.grid_x && cfg.grid_y <= chip.grid_y,
            "{}: grid too large",
            chip.name
        );
    }
}

#[test]
fn the_compiler_chooses_the_program_strategy_by_shape() {
    use logicaffeine_compile::codegen_tt::matmul_lower::{choose_strategy, MatmulStrategy};
    // 640³: each core owns a small (4×4) output block → streaming Simple wins (measured 9,735 < 12,097).
    let (small, _) = compile_to_tenstorrent_matmul(&gemm_program(640, 640, 640)).unwrap();
    assert_eq!(
        choose_strategy(&small, &ChipKnobs::wormhole()),
        MatmulStrategy::Simple
    );
    // A large GEMM (4096³ → 128×128×128 tiles): big per-core blocks on a genuine 2-D grid → multicast
    // reads each operand block from DRAM once (the vendor's win) → ReuseMcast strictly dominates BlockReuse.
    let (big, _) = compile_to_tenstorrent_matmul(&gemm_program(4096, 4096, 4096)).unwrap();
    assert_eq!(
        choose_strategy(&big, &ChipKnobs::wormhole()),
        MatmulStrategy::ReuseMcast
    );
}

/// Every kernel FILE NAME the host names, from the string literals it builds its paths out of.
///
/// Read as literals rather than by walking `CreateKernel(` calls, because a path is assembled from
/// a directory variable and a literal — `kdir + "logos_mm_reader.cpp"` — so the file name is the
/// literal and the directory is not in it. A parser that demanded a `/` in the literal saw only
/// the one call that spells its directory inline, and reported an empty census as a pass.
fn kernels_named(host: &str) -> Vec<String> {
    let mut out = Vec::new();
    for piece in host.split('"').skip(1).step_by(2) {
        if piece.ends_with(".cpp") {
            out.push(piece.rsplit('/').next().unwrap_or(piece).to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn the_compiler_dispatches_a_recognized_matmul_to_the_vendor_op_with_its_own_tuning() {
    // The "recognize the matmul → dispatch to BLAS" path: the compiler recognizes the GEMM, auto-tunes the
    // config, and routes the primitive to the vendor's optimal op (ttnn.matmul) with THAT config. Measured:
    // this compiler-generated dispatch runs 640³ at 6154 device cycles — beating ttnn's own default (6745).
    let (spec, _) = compile_to_tenstorrent_matmul(&gemm_program(640, 640, 640)).unwrap();
    let cfg = tune_tiling(&spec, &ChipKnobs::wormhole());
    let dispatch = emit_matmul_vendor_dispatch(&spec, &cfg);

    // It's a real ttnn.matmul dispatch, carrying the RECOGNIZED dims + the compiler's tuned grid/per-core.
    assert!(
        dispatch.contains("ttnn.matmul"),
        "dispatch must route to the vendor op"
    );
    assert!(
        dispatch.contains("MatmulMultiCoreReuseMultiCastProgramConfig"),
        "must pass a program config"
    );
    assert!(
        dispatch.contains(&format!("[{}, {}]", spec.m, spec.k)),
        "A operand carries recognized M×K"
    );
    assert!(
        dispatch.contains(&format!("[{}, {}]", spec.k, spec.n)),
        "B operand carries recognized K×N"
    );
    assert!(
        dispatch.contains(&format!(
            "compute_with_storage_grid_size=({}, {})",
            cfg.grid_x, cfg.grid_y
        )),
        "dispatch must pass the compiler's auto-tuned grid"
    );
    assert!(
        dispatch.contains(&format!("per_core_M={}", cfg.per_core_m)),
        "dispatch must pass the compiler's auto-tuned per-core block"
    );
    // The overlap-favoring inner block (measured sweet spot in0_block_w=4 → 6154 cyc).
    assert!(
        dispatch.contains("in0_block_w=4"),
        "for 640³ the tuned inner block is 4 (the 6154-cyc point)"
    );
    // Genuinely a function of the source: a different shape ⇒ a different dispatch.
    let (spec2, _) = compile_to_tenstorrent_matmul(&gemm_program(320, 320, 320)).unwrap();
    let dispatch2 =
        emit_matmul_vendor_dispatch(&spec2, &tune_tiling(&spec2, &ChipKnobs::wormhole()));
    assert_ne!(
        dispatch, dispatch2,
        "the dispatch must depend on the recognized dimensions"
    );
}

#[test]
fn the_recognized_program_actually_runs_on_the_vm_the_numeric_anchor() {
    // The genuine lowering targets a program the VM also executes — the ≡VM anchor for the numerics.
    let out = vm_outcome(&gemm_program(32, 32, 32));
    assert!(
        out.error.is_none(),
        "the matmul program must run on the VM: {:?}",
        out.error
    );
    let checksum: i64 = out
        .output
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    // Deterministic fill ⇒ a fixed checksum; recompute the reference in Rust and require exact agreement.
    let (m, n, k) = (32usize, 32usize, 32usize);
    let a: Vec<i64> = (0..m * k).map(|p| (p as i64 % 7) - 3).collect();
    let b: Vec<i64> = (0..k * n).map(|p| (p as i64 % 5) - 2).collect();
    let mut c = vec![0i64; m * n];
    for i in 0..m {
        for kk in 0..k {
            for j in 0..n {
                c[i * n + j] += a[i * k + kk] * b[kk * n + j];
            }
        }
    }
    let reference: i64 = c.iter().sum();
    assert_eq!(
        checksum, reference,
        "VM matmul numerics must match the Rust reference"
    );
}

// ── Multicast-reuse: closing the gap to the vendor's hand-optimized op ────────────────────────────────
// The vendor's fully-optimized matmul beats our auto-generated kernel by reading each operand block from
// DRAM ONCE and NOC-multicasting it across a row/column. These locks prove the compiler now (1) MODELS
// that traffic win, (2) EMITS a structurally-correct multicast host as a function of (spec,cfg), and
// (3) never ships the strictly-dominated plain block-reuse on a 2-D grid. The device-cycle confirmation
// (does it hit the vendor's ~6,745?) is a ttsim run — traffic is necessary, not sufficient.

#[test]
fn multicast_reuse_models_the_fewest_dram_reads() {
    let (spec, _) = compile_to_tenstorrent_matmul(&gemm_program(640, 640, 640)).unwrap();
    // Only the worker-grid dims drive the traffic model; a 2×2 grid is enough to exercise row+column reuse.
    let cfg = TileConfig {
        grid_x: 2,
        grid_y: 2,
        per_core_m: 10,
        per_core_n: 10,
        in0_block_w: 2,
        out_subblock_h: 2,
        out_subblock_w: 2,
    };
    let simple = dram_read_tiles(&spec, &cfg, MatmulStrategy::Simple);
    let reuse = dram_read_tiles(&spec, &cfg, MatmulStrategy::BlockReuse);
    let mcast = dram_read_tiles(&spec, &cfg, MatmulStrategy::ReuseMcast);
    assert!(
        mcast < reuse,
        "multicast must read each operand tile once: {mcast} !< {reuse}"
    );
    assert!(
        reuse < simple,
        "block reuse must beat no-reuse streaming: {reuse} !< {simple}"
    );
    // Multicast reads each A and B tile from DRAM exactly once ⇒ (Mt+Nt)·Kt.
    assert_eq!(
        mcast,
        ((spec.mt() + spec.nt()) * spec.kt()) as u64,
        "mcast = one read per operand tile"
    );
}

#[test]
fn the_compiler_never_ships_dominated_plain_block_reuse_on_a_2d_grid() {
    // ReuseMcast strictly dominates plain BlockReuse (same L1 reuse, fewer DRAM reads), so on a genuine
    // 2-D worker grid the compiler must never pick BlockReuse. Also REPORTS which sizes reach multicast so
    // the autotuner's mcast-awareness is visible (not asserted — that is the ttsim-validated next step).
    let chip = ChipKnobs::wormhole();
    let mut reached = Vec::new();
    for s in [128usize, 256, 512, 640, 1024, 2048] {
        let (spec, _) = compile_to_tenstorrent_matmul(&gemm_program(s, s, s)).unwrap();
        let cfg = tune_tiling(&spec, &chip);
        let strat = choose_strategy(&spec, &chip);
        if cfg.grid_x > 1 && cfg.grid_y > 1 {
            assert_ne!(
                strat, MatmulStrategy::BlockReuse,
                "on a 2-D grid ({}×{}) the compiler must pick ReuseMcast, never redundant-read BlockReuse (size {s})",
                cfg.grid_x, cfg.grid_y
            );
        }
        if strat == MatmulStrategy::ReuseMcast {
            reached.push(s);
        }
    }
    eprintln!("multicast-reuse reached at GEMM sizes: {reached:?}");
}

/// ★ PORTED FROM THE RETIRED EMITTER'S KNOB TEST — the one assertion in that test the spatial IR path
/// did NOT already cover, kept because it has already caught a real defect once.
///
/// ttnn's own bf16 default numerics are HiFi2 with bf16 (not fp32) dest-accumulate. Ours match them so a
/// head-to-head against the vendor is matched BY CONSTRUCTION rather than by remembering to pass a flag.
/// Defaulting to HiFi4 would silently compare our four-pass arithmetic against the vendor's two-pass and
/// report the difference as a compiler result.
///
/// This is not hypothetical: when `ComputeKnobs` was salvaged into [`NumericsKnobs`] the default became
/// HiFi4, and nothing else in the suite noticed. The "every host prints them" half of the original test
/// retired with the emitters — `spatial_tensix_lowering_lock::the_host_never_prints_a_knob_it_does_not_run`
/// holds that on the IR path.
#[test]
fn the_default_numerics_are_ttnns_own_bf16_defaults() {
    use logicaffeine_compile::spatial::target::{Fidelity, NumericsKnobs};
    let k = NumericsKnobs::default();
    assert_eq!(
        k.fidelity,
        Fidelity::HiFi2,
        "ttnn's bf16 default is HiFi2; a head-to-head must be matched by construction"
    );
    assert!(
        !k.fp32_dest_acc,
        "bf16 dest-accumulate by default — fp32 is a knob, not a default"
    );
    assert!(
        !k.packer_l1_acc,
        "packer L1 accumulate is a knob, not a default"
    );
    assert!(
        (k.pcc_floor - 0.99).abs() < 1e-12,
        "the PCC floor the emitted program requires of itself"
    );
}

#[test]
fn compute_knobs_parse_from_the_harness_environment() {
    // scripts/tt-matmul-measure.sh hands the emitter LOGOS_TT_FIDELITY / LOGOS_TT_FP32 / LOGOS_TT_L1ACC; the
    // ledger then refuses the row if the program printed anything else — so parsing must be exact.
    let k = ComputeKnobs::from_vars(Some("HiFi4"), Some("1"), Some("0")).unwrap();
    assert_eq!(k.fidelity, MathFidelity::HiFi4);
    assert!(k.fp32_dest_acc && !k.packer_l1_acc);
    let d = ComputeKnobs::from_vars(None, None, None).unwrap();
    assert_eq!(
        d,
        ComputeKnobs::default(),
        "unset variables mean the defaults"
    );
    assert!(
        ComputeKnobs::from_vars(Some("HiFi9"), None, None).is_err(),
        "an unknown fidelity is refused"
    );
    assert!(
        ComputeKnobs::from_vars(None, Some("yes"), None).is_err(),
        "fp32 must be 0|1"
    );
    for (f, passes) in [
        (MathFidelity::LoFi, 1),
        (MathFidelity::HiFi2, 2),
        (MathFidelity::HiFi3, 3),
        (MathFidelity::HiFi4, 4),
    ] {
        assert_eq!(f.fpu_passes(), passes);
        assert_eq!(MathFidelity::parse(f.as_str()), Some(f));
    }
}
