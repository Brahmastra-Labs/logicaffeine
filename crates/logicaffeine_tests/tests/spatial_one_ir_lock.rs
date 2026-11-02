//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ONE-IR LOCK — the capstone: the SAME spatial program lowers to Tensix, our fabric, the RDU and a
//! persistent GPU kernel, every artifact is the image of the IR, and no bespoke kernel body exists
//! outside the IR printers.
//!
//!   A. One LOGOS source → per-target templates that are structurally IDENTICAL (same tensors, stage kinds,
//!      roles, edges and handshake kinds — only the native tile geometry differs) → each target's passes →
//!      each backend's lowering; every verified program evaluates to the VM's checksum.
//!   B. Every artifact's census equals its IR's census (Tensix, Verilog, CUDA, neutral).
//!   C. Where the validating tool is present the artifact is validated by it (iverilog / nvcc+GPU / the
//!      RDU certificate); absence is an honest `LOGOS_TOOL_SKIP`, never a print.
//!   D. NO BESPOKE: every low-level choreography token (circular-buffer, NOC-semaphore, multicast, cp.async,
//!      mbarrier, pipeline, FIFO, skew-register) in `codegen_{tt,sva,cuda}` is at or below its frozen count,
//!      new files start at zero, and `spatial/lower/*` (the printers) are the only exemption.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_one_ir_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorId};
use logicaffeine_compile::spatial::lower::{
    cuda::CudaPersistentBackend, fabric::FabricBackend, rdu::RduSpatialBackend,
    tensix::TensixBackend, Census, SpatialArtifact, SpatialBackend,
};
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_source};
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::*;

/// The target-independent shape of a template: tensor names/elements/roles, stage names/kinds/roles, edge
/// endpoints and handshake kinds. Tile geometry, dtypes and numerics are target facts and are excluded.
fn structural_signature(p: &SpatialProgram) -> String {
    let mut s = String::new();
    for t in &p.tensors {
        s.push_str(&format!(
            "T{}:{:?}:{}x{};",
            t.name, t.role, t.elems.rows, t.elems.cols
        ));
    }
    for st in &p.stages {
        let kind = match &st.kind {
            logicaffeine_compile::spatial::ir::StageKind::Load { tensor, .. } => {
                format!("Load(t{})", tensor.idx())
            }
            logicaffeine_compile::spatial::ir::StageKind::Store { tensor, .. } => {
                format!("Store(t{})", tensor.idx())
            }
            logicaffeine_compile::spatial::ir::StageKind::Recv { .. } => "Recv".into(),
            logicaffeine_compile::spatial::ir::StageKind::Compute(
                logicaffeine_compile::spatial::ir::ComputeOp::MatmulTiles { .. },
            ) => "Matmul".into(),
            logicaffeine_compile::spatial::ir::StageKind::Compute(op) => format!("{op:?}"),
            logicaffeine_compile::spatial::ir::StageKind::AllReduce { pool, tensor, .. } => {
                format!("AllReduce({pool:?},t{})", tensor.idx())
            }
            logicaffeine_compile::spatial::ir::StageKind::Scalar(_) => "Scalar".into(),
        };
        s.push_str(&format!(
            "S{}:{kind}:{:?}:{:?}->{:?};",
            st.name, st.role, st.ins, st.outs
        ));
    }
    for e in &p.edges {
        s.push_str(&format!(
            "E{:?}:{:?}->{:?}:{:?};",
            e.from,
            e.buffer,
            e.to,
            e.handshake.kind()
        ));
    }
    s
}

#[test]
fn the_same_program_lowers_to_all_four_backends_and_equals_the_vm() {
    let src = gemm_program(128, 64, 96);
    let vm = shown_int(&vm_outcome(&src).output);
    let targets = [
        SpatialTarget::wormhole(),
        SpatialTarget::sky_fabric(64, 64),
        SpatialTarget::sn40l_rdu(),
        SpatialTarget::rtx3060(),
    ];
    let templates: Vec<SpatialProgram> = targets
        .iter()
        .map(|t| template_from_source(&src, t).unwrap())
        .collect();
    let sig = structural_signature(&templates[0]);
    for (t, tpl) in targets.iter().zip(&templates) {
        assert_eq!(
            structural_signature(tpl),
            sig,
            "{}: the template is the same program (only the native tile differs)",
            t.name
        );
    }
    let backends: Vec<Box<dyn SpatialBackend>> = vec![
        Box::new(TensixBackend::new(targets[0].clone())),
        Box::new(FabricBackend::new(targets[1].clone())),
        Box::new(RduSpatialBackend::new()),
        Box::new(CudaPersistentBackend::new(targets[3].clone())),
    ];
    for ((t, tpl), backend) in targets.iter().zip(templates).zip(backends) {
        let p = run_passes(tpl, t).unwrap_or_else(|e| panic!("{}: {e:?}", t.name));
        let out = eval_choreography(&p, &inputs_from_fills(&p)).unwrap();
        let got = match &out.tensors[&TensorId(2)] {
            Value::Int(v) => checksum(v),
            Value::Float(v) => checksum(&v.iter().map(|x| *x as i64).collect::<Vec<_>>()),
        };
        assert_eq!(got, vm, "{}: ≡ VM", t.name);
        let art = backend
            .lower(&p)
            .unwrap_or_else(|e| panic!("{}: {e:?}", backend.name()));
        let census = Census::of(&p);
        match &art {
            SpatialArtifact::Tensix(a) => assert_eq!(a.census, census, "{}", t.name),
            SpatialArtifact::Verilog(a) => {
                assert_eq!(a.census, census, "{}", t.name);
                assert_eq!(
                    Census::of_verilog(&a.rtl),
                    census,
                    "{}: RTL instance census",
                    t.name
                );
            }
            SpatialArtifact::Cuda(a) => assert_eq!(a.census, census, "{}", t.name),
            SpatialArtifact::SdkGated(a) => {
                assert_eq!(a.census, census, "{}", t.name);
                assert!(a.certificate.recheck(&p));
            }
        }
    }
}

#[test]
fn artifacts_are_validated_by_their_tools_where_present() {
    let src = gemm_program(16, 8, 8);
    // Fabric under iverilog.
    if common::require_tool("iverilog") {
        let mut t = SpatialTarget::sky_fabric(16, 16);
        t.tile = logicaffeine_compile::spatial::target::TileShape { rows: 4, cols: 4 };
        t.site.acc_tiles = 4;
        let p = run_passes(template_from_source(&src, &t).unwrap(), &t).unwrap();
        let art = match FabricBackend::new(t.clone()).lower(&p).unwrap() {
            SpatialArtifact::Verilog(v) => v,
            other => panic!("{other:?}"),
        };
        let dir = std::env::temp_dir().join(format!("logos_one_ir_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut paths = Vec::new();
        for (name, body) in [
            ("spatial_top.v".to_string(), art.rtl.clone()),
            ("tb.v".to_string(), art.testbench.clone()),
        ]
        .iter()
        .chain(
            art.hex
                .iter()
                .map(|(a, b)| (a.clone(), b.clone()))
                .collect::<Vec<_>>()
                .iter(),
        )
        .chain(
            logicaffeine_compile::spatial::lower::fabric::input_hex_files(
                &p,
                &inputs_from_fills(&p),
            )
            .iter(),
        ) {
            let path = dir.join(name);
            std::fs::write(&path, body).unwrap();
            paths.push(path);
        }
        let bin = dir.join("top.vvp");
        // Only the Verilog is compiled. The `.hex` images are written beside it for `$readmemh` to open at
        // run time; handing them to iverilog as sources makes it parse a memory image as a module.
        let sources: Vec<&std::path::PathBuf> = paths
            .iter()
            .filter(|p| p.extension().map(|e| e == "v").unwrap_or(false))
            .collect();
        let b = std::process::Command::new("iverilog")
            .arg("-g2012")
            .arg("-o")
            .arg(&bin)
            .args(&sources)
            .output()
            .unwrap();
        assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
        let r = std::process::Command::new("vvp")
            .arg(&bin)
            .current_dir(&dir)
            .output()
            .unwrap();
        let out = String::from_utf8_lossy(&r.stdout);
        assert!(out.contains("PASS:"), "{out}");
        let expected = shown_int(&vm_outcome(&src).output);
        let got: i64 = out
            .lines()
            .filter(|l| l.starts_with("OUT "))
            .map(|l| {
                let t: Vec<&str> = l.split_whitespace().collect();
                match t[3].strip_prefix("0x") {
                    Some(h) => i64::from_str_radix(h, 16).map(|u| u as i64).unwrap(),
                    None => t[3].parse().unwrap(),
                }
            })
            .fold(0i64, |s, x| s.wrapping_add(x));
        assert_eq!(got, expected, "fabric RTL checksum ≡ VM");
    }
    // CUDA under nvcc + the GPU.
    if common::require_tool("nvcc") && common::require_gpu() {
        let t = SpatialTarget::rtx3060();
        let p = run_passes(template_from_source(&src, &t).unwrap(), &t).unwrap();
        let art = match CudaPersistentBackend::new(t).lower(&p).unwrap() {
            SpatialArtifact::Cuda(c) => c,
            other => panic!("{other:?}"),
        };
        let dir = std::env::temp_dir().join(format!("logos_one_ir_cuda_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cu = dir.join("p.cu");
        std::fs::write(&cu, &art.program_cu).unwrap();
        let bin = dir.join("p");
        let b = std::process::Command::new("nvcc")
            .args(["-O3", "-arch=sm_86", "-std=c++17", "-o"])
            .arg(&bin)
            .arg(&cu)
            .output()
            .unwrap();
        assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
        let inputs = inputs_from_fills(&p);
        let (Value::Int(a), Value::Int(bv)) =
            (&inputs.tensors[&TensorId(0)], &inputs.tensors[&TensorId(1)])
        else {
            panic!()
        };
        std::fs::write(
            dir.join("a.txt"),
            a.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("b.txt"),
            bv.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let r = std::process::Command::new(&bin)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
        let got: i64 = String::from_utf8_lossy(&r.stdout)
            .split_whitespace()
            .filter_map(|t| t.parse::<i64>().ok())
            .fold(0i64, |s, x| s.wrapping_add(x));
        assert_eq!(
            got,
            shown_int(&vm_outcome(&src).output),
            "GPU checksum ≡ VM"
        );
    }
    // Tensix under ttsim is measured by `spatial_tensix_ttsim_lock`; here the structural census stands in and
    // the simulator's absence is reported through the honest seam.
    let _ = common::require_tool("ttsim");
    let t = SpatialTarget::wormhole();
    let p = run_passes(template_from_source(&src, &t).unwrap(), &t).unwrap();
    match TensixBackend::new(t).lower(&p).unwrap() {
        SpatialArtifact::Tensix(a) => assert_eq!(a.census, Census::of(&p)),
        other => panic!("{other:?}"),
    }
}

/// Low-level choreography tokens that may only appear in the IR's printers (`spatial/lower/*`). Every
/// other file is frozen at the count below (default 0) and may only shrink.
const SPATIAL_OP_TOKENS: &[&str] = &[
    "cb_reserve_back",
    "cb_push_back",
    "cb_wait_front",
    "cb_pop_front",
    "noc_semaphore_",
    "noc_async_write_multicast",
    "__pipeline_memcpy_async",
    "__pipeline_commit",
    "__pipeline_wait_prior",
    "cuda::pipeline",
    "cuda::barrier",
    "mbarrier",
    "bar.sync",
    "chan_fifo",
    "ps_reg_",
    "deskew_",
    "sem_wait",
    "reader_bmm",
    "writer_bmm",
    "matmul_common",
    "reader_mm_output",
    "writer_unary",
    "programming_examples/matmul/",
];

/// The choreography a PRINTER must actually emit — the non-vacuity half of this lock.
///
/// ★ DELIBERATELY NOT ALL OF `SPATIAL_OP_TOKENS`. That list mixes three kinds of string and only one of
/// them is printer vocabulary:
///
///   * **choreography the compiler emits** — the circular-buffer discipline, the NOC semaphores, the
///     multicast, CUDA's `cp.async` commit groups. These belong to the printers and are listed below.
///   * **vendor kernel NAMES** (`reader_bmm`, `writer_bmm`, `matmul_common`, `reader_mm_output`,
///     `writer_unary`, `programming_examples/matmul/`) — pure PROHIBITIONS. Tensix is a LOWERING, so a
///     vendor path may appear nowhere at all, printers included. Requiring them here would demand
///     exactly what the first half of this test forbids.
///   * **frozen backend-native rungs** (`chan_fifo` / `ps_reg_` / `deskew_` in `codegen_sva`, and the CUDA
///     knob lattice's own `__pipeline_*` rungs in `codegen_cuda/gemm_tile.rs`) — hand-written on purpose,
///     carried in FROZEN. The fabric's are not printed from the IR, so a printer that lacks them is correct,
///     not vacuous; the `__pipeline_*` calls are ALSO the CUDA printer's own FIFO images, so they are listed.
const PRINTER_VOCABULARY: &[&str] = &[
    "cb_reserve_back",
    "cb_push_back",
    "cb_wait_front",
    "cb_pop_front",
    "noc_semaphore_",
    "noc_async_write_multicast",
    "__pipeline_memcpy_async",
    "__pipeline_commit",
    "__pipeline_wait_prior",
];

/// (file, token) → the count frozen on 2026-09-02. The CUDA knob lattice keeps its hand-written cp.async
/// rungs (the backend-native path); the fabric keeps its systolic skew registers and the recorded channel
/// FIFO. Everything in `codegen_tt` is zero: the Tensix templates are retired.
///
/// ★ P5 LANDED (2026-09-12) AND THIS RATCHET IS GREEN ON THE MERITS. `codegen_tt/{matrix,matmul_lower,
/// concurrency_mesh}.rs` held 130 hand-written choreography tokens between them; they now hold zero.
/// `matrix.rs` is deleted (its int8 `matmul_tiles` path measured 0/4096 correct on Wormhole AND
/// Blackhole, and the test certifying it only ever checked that the kernel contained the SUBSTRING
/// `matmul_tiles`). `matmul_lower.rs` keeps the tiling facade and the strategy, which carry no
/// choreography. The concurrency patterns lower through `spatial::passes::from_tensor_op::from_pipe`.
///
/// ★ THE ZERO IS STILL A CLAIM ABOUT WHERE THE CODE GOES, NOT A RECORD OF WHERE IT IS. It was red for
/// months as the task marker, and freezing these files at their then-current counts was tried once and
/// reverted — recording the debt reads as progress and is the opposite. Do not re-add entries for
/// `codegen_tt` here; a new bespoke body in that directory is a regression, not a new baseline.
const FROZEN: &[(&str, &str, usize)] = &[
    ("codegen_cuda/gemm_tile.rs", "__pipeline_commit", 25),
    ("codegen_cuda/gemm_tile.rs", "__pipeline_memcpy_async", 20),
    ("codegen_cuda/gemm_tile.rs", "__pipeline_wait_prior", 12),
    // The generation lane's streamed Q8_0 GEMM (`lk_blk_streamed`) lands its `cp.async` stages on mbarriers: the CUDA
    // chip's own kernel library, as the knob lattice's `__pipeline_*` rungs are, not a spatial body.
    ("codegen_cuda/lane/kernels.rs", "mbarrier", 4),
    ("codegen_sva/chan_rtl.rs", "chan_fifo", 17),
    // The parameterized FIFO's byte-identity contract with the recorded channel FIFO, stated three times.
    ("codegen_sva/handshake_rtl.rs", "chan_fifo", 3),
    ("codegen_sva/systolic.rs", "deskew_", 6),
    ("codegen_sva/systolic.rs", "ps_reg_", 6),
];

#[test]
fn no_bespoke_spatial_bodies_outside_the_printers() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let src = root.join("crates/logicaffeine_compile/src");
    let mut files = Vec::new();
    fn walk(d: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    for dir in ["codegen_tt", "codegen_sva", "codegen_cuda"] {
        walk(&src.join(dir), &mut files);
    }
    let mut violations = Vec::new();
    for f in files {
        let rel = f
            .strip_prefix(&src)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&f).unwrap();
        for tok in SPATIAL_OP_TOKENS {
            let count = text.matches(tok).count();
            let frozen = FROZEN
                .iter()
                .find(|(file, t, _)| *file == rel && t == tok)
                .map(|(_, _, n)| *n)
                .unwrap_or(0);
            if count > frozen {
                violations.push(format!("{rel}: `{tok}` ×{count} (frozen {frozen})"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "bespoke choreography outside spatial/lower/*:\n{}",
        violations.join("\n")
    );
    // ── The exemption is not vacuous ────────────────────────────────────────────────────────────────
    //
    // ★ THIS WAS `assert!(total > 40)` — every token occurrence summed over `spatial/lower/*`, as a proxy
    // for "the printers really are where the choreography lives". The proxy stopped measuring that
    // property when the printers became GENERAL: each construct is emitted from ONE site now, where the
    // retired hand-written templates repeated it dozens of times. P5 took `codegen_tt` from 130 bespoke
    // tokens to 0 and, in the same move, the printer total from >40 to 14. The migration SUCCEEDING is
    // what broke the threshold, so re-basing the number to 14 would record the debt as progress and
    // measure nothing either.
    //
    // A sum was the weaker statement in any case: 40 copies of `cb_push_back` with every other construct
    // missing would have passed it — which is precisely the vacuity this assertion is named against. So
    // assert the property directly: every construct whose ONLY permitted home is a printer must actually
    // be emitted by one.
    let printers = src.join("spatial/lower");
    let mut printer_files = Vec::new();
    walk(&printers, &mut printer_files);
    let printed: Vec<String> = printer_files
        .iter()
        .map(|f| std::fs::read_to_string(f).unwrap())
        .collect();
    for tok in PRINTER_VOCABULARY {
        assert!(
            printed.iter().any(|t| t.contains(tok)),
            "no printer under spatial/lower/* emits `{tok}`. Every other file in codegen_{{tt,sva,cuda}} is \
             forbidden from carrying it, so if the printers do not emit it either, the token is dead \
             vocabulary and this lock's exemption is vacuous."
        );
    }
}
