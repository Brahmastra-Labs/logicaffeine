//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FABRIC LOWERING LOCK — the SAME spatial IR lowers to our own fabric as synthesizable Verilog, proven
//! cycle-accurate under iverilog and clean under yosys.
//!
//! Buffers become parameterized FIFOs, valid/ready edges become wires, multicast edges become a counting
//! semaphore + a fan-out bus, barriers a join tree; tile matmuls become the proven pipelined systolic array
//! with loadable weights; loads/stores become DAE address lanes; scalar bodies the placed CGRA grid. This
//! lock proves:
//!   A. the FIFO generalization is byte-identical to the recorded channel FIFO and its bound is proven by
//!      k-induction at several depths; the handshake primitives match their models under iverilog;
//!   B. the loadable PE array equals the baked array cycle-for-cycle;
//!   C. a 2×2-site multicast GEMM, a streamed GEMM with a ragged K, a fused matmul→requant→bias→relu block
//!      and a scalar-stage program all run in RTL ≡ `eval_choreography` (≡ the systolic reference), with the
//!      cycle count equal to the IR's cycle model and above the closed-form fill/drain floor;
//!   D. the mesh top synthesizes with zero problems under yosys;
//!   E. the census of RTL instances is the census of the IR.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_fabric_lowering_lock` (iverilog / yosys gated)
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::chan_rtl::emit_chan_fifo_verilog;
use logicaffeine_compile::codegen_sva::handshake_rtl::{
    emit_barrier_verilog, emit_fifo_verilog, emit_mcast_bus_verilog, emit_semaphore_verilog,
    fifo_bound_certified,
};
use logicaffeine_compile::codegen_sva::systolic::{
    emit_systolic_pipelined_loadable_verilog, emit_systolic_pipelined_verilog_bits_acc,
    eval_systolic_pipelined_acc, Systolic, ACC_BITS_MXU,
};
use logicaffeine_compile::spatial::eval::{eval_choreography, Inputs, Value};
use logicaffeine_compile::spatial::ir::{NumericsPolicy, SpatialProgram, TensorId};
use logicaffeine_compile::spatial::lower::fabric::{
    fabric_cycle_model, FabricBackend, VerilogArtifact,
};
use logicaffeine_compile::spatial::lower::{Census, SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::from_tensor_op::{
    from_fused_block, from_scalar_body, MatmulNames,
};
use logicaffeine_compile::spatial::pipeline::{run_passes, template_from_spec};
use logicaffeine_compile::spatial::target::{MeshShape, SpatialTarget};
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};
use spatial_support::*;

fn tmp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("logos_spatial_fabric_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Compile + simulate under iverilog; returns stdout.
fn run_iverilog(dir: &std::path::Path, top: &str, sources: &[(&str, &str)]) -> String {
    let mut paths = Vec::new();
    for (name, body) in sources {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        // `.hex` files are memory images the RTL reads with `$readmemh`, not compilation units.
        if name.ends_with(".v") {
            paths.push(p);
        }
    }
    let bin = dir.join(format!("{top}.vvp"));
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&bin)
        .args(&paths)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "iverilog failed for {top}: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    // The RTL's memories `$readmemh` their images by bare name, so the simulation runs from the bundle dir.
    let run = std::process::Command::new("vvp")
        .arg(&bin)
        .current_dir(dir)
        .output()
        .expect("vvp");
    assert!(
        run.status.success(),
        "vvp failed for {top}: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).to_string()
}

fn parse_outputs(stdout: &str) -> (Vec<i64>, u64, bool) {
    let mut out = Vec::new();
    let mut cycles = 0;
    let mut pass = false;
    for line in stdout.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.first().copied() {
            // `0x…` is hex (a memory word), anything else is the decimal the testbench printed.
            Some("OUT") => out.push(match t[3].strip_prefix("0x") {
                Some(h) => i64::from_str_radix(h, 16)
                    .map(|u| u as i64)
                    .unwrap_or_else(|e| panic!("OUT hex {}: {e}", t[3])),
                None => t[3]
                    .parse()
                    .unwrap_or_else(|e| panic!("OUT value {}: {e}", t[3])),
            }),
            Some("CYCLES") => cycles = t[1].parse().unwrap(),
            Some("PASS:") => pass = true,
            _ => {}
        }
    }
    (out, cycles, pass)
}

fn lower(p: &SpatialProgram, target: &SpatialTarget) -> VerilogArtifact {
    match FabricBackend::new(target.clone()).lower(p).unwrap() {
        SpatialArtifact::Verilog(v) => v,
        other => panic!("{other:?}"),
    }
}

fn names() -> MatmulNames {
    MatmulNames {
        a: "a".into(),
        b: "b".into(),
        c: "c".into(),
    }
}

fn ints(v: &Value) -> Vec<i64> {
    match v {
        Value::Int(x) => x.clone(),
        Value::Float(x) => x.iter().map(|f| *f as i64).collect(),
    }
}

/// The fabric target as a 2×2 mesh of 4-wide systolic sites (tile 4×4) with multicast wires — the small
/// configuration the RTL locks can simulate in seconds.
fn small_mesh() -> SpatialTarget {
    let mut t = SpatialTarget::sky_fabric(16, 16);
    t.sites = MeshShape { x: 2, y: 2 };
    t.tile = logicaffeine_compile::spatial::target::TileShape { rows: 4, cols: 4 };
    t.site.acc_tiles = 4;
    t.interconnect
        .insert(logicaffeine_compile::spatial::target::Interconnect::MulticastRect);
    t.handshakes
        .insert(logicaffeine_compile::spatial::target::HandshakeKind::Semaphore);
    t.cycle = fusing_wormhole().cycle;
    t
}

#[test]
fn fifo_generalization_is_byte_identical_to_the_recorded_channel_fifo_and_bounded_at_every_depth() {
    assert_eq!(
        emit_fifo_verilog("chan_fifo", 8, 32),
        emit_chan_fifo_verilog(),
        "the channel FIFO IS the parameterized FIFO at (8, 32)"
    );
    for depth in [3usize, 8, 16] {
        assert!(
            fifo_bound_certified(depth, 32),
            "0 ≤ cnt ≤ {depth} must be proven by k-induction"
        );
        let v = emit_fifo_verilog("f", depth, 32);
        assert!(v.contains(&format!("DEPTH = {depth}")), "{v}");
    }
}

#[test]
fn handshake_primitives_match_their_models_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tmp("prims");
    // FIFO (depth 5, width 96) against a VecDeque over a scripted push/pop trace.
    let fifo = emit_fifo_verilog("fifo5", 5, 96);
    let tb = logicaffeine_compile::codegen_sva::handshake_rtl::emit_fifo_trace_testbench(
        "fifo5", 5, 96, 200, 7,
    );
    let out = run_iverilog(&dir, "tb_fifo5", &[("fifo5.v", &fifo), ("tb_fifo5.v", &tb)]);
    assert!(out.contains("PASS:"), "fifo trace: {out}");
    // Semaphore: inc / set / wait threshold trace.
    let sem = emit_semaphore_verilog("sem", 8);
    let tb =
        logicaffeine_compile::codegen_sva::handshake_rtl::emit_semaphore_trace_testbench("sem", 8);
    let out = run_iverilog(&dir, "tb_sem", &[("sem.v", &sem), ("tb_sem.v", &tb)]);
    assert!(out.contains("PASS:"), "semaphore trace: {out}");
    // Barrier joins for n = 2, 3, 9.
    for n in [2usize, 3, 9] {
        let bar = emit_barrier_verilog("bar", n);
        let tb = logicaffeine_compile::codegen_sva::handshake_rtl::emit_barrier_trace_testbench(
            "bar", n,
        );
        let out = run_iverilog(&dir, "tb_bar", &[("bar.v", &bar), ("tb_bar.v", &tb)]);
        assert!(out.contains("PASS:"), "barrier {n}: {out}");
    }
    // Multicast bus: one writer, three receivers.
    let bus = emit_mcast_bus_verilog("bus", 64, 3);
    let tb = logicaffeine_compile::codegen_sva::handshake_rtl::emit_mcast_bus_trace_testbench(
        "bus", 64, 3,
    );
    let out = run_iverilog(&dir, "tb_bus", &[("bus.v", &bus), ("tb_bus.v", &tb)]);
    assert!(out.contains("PASS:"), "mcast bus: {out}");
}

#[test]
fn the_loadable_pe_array_equals_the_baked_array_cycle_for_cycle() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tmp("pe");
    for &(k, n, m) in &[(4usize, 4usize, 6usize), (8, 4, 5), (4, 8, 7)] {
        let weights: Vec<i64> = (0..k * n).map(|i| prng(1, i, -128, 127)).collect();
        let acts: Vec<i64> = (0..m * k).map(|i| prng(2, i, -128, 127)).collect();
        let sys = Systolic::new(k, n, weights.clone());
        let baked = emit_systolic_pipelined_verilog_bits_acc("baked", &sys, 8, ACC_BITS_MXU);
        let loadable = emit_systolic_pipelined_loadable_verilog("loadable", k, n, 8, ACC_BITS_MXU);
        assert_ne!(baked, loadable);
        let (model, latency) = eval_systolic_pipelined_acc(&sys, &acts, m, 8, ACC_BITS_MXU);
        let tb = logicaffeine_compile::codegen_sva::systolic::emit_loadable_vs_baked_testbench(
            "baked",
            "loadable",
            &sys,
            &acts,
            m,
            8,
            ACC_BITS_MXU,
        );
        let out = run_iverilog(
            &dir,
            "tb_pe",
            &[
                ("baked.v", &baked),
                ("loadable.v", &loadable),
                ("tb_pe.v", &tb),
            ],
        );
        assert!(out.contains("PASS:"), "{k}x{n} m={m}: {out}");
        let (got, _, _) = parse_outputs(&out);
        assert_eq!(got, model, "{k}x{n}: RTL ≡ the cycle-exact model");
        assert!(
            out.contains(&format!("LATENCY {latency}")),
            "{k}x{n}: latency {latency}"
        );
    }
}

fn gemm_rtl_case(label: &str, m: usize, n: usize, k: usize, target: &SpatialTarget) {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tmp(label);
    let spec = MatmulSpec {
        m,
        n,
        k,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let p = template_from_spec(
        &spec,
        &names(),
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-128, 127)), Some((-128, 127))),
        NumericsPolicy::Exact,
        target,
    )
    .unwrap();
    let p = run_passes(p, target).unwrap();
    let acts: Vec<i64> = (0..m * k).map(|i| prng(11, i, -128, 127)).collect();
    let weights: Vec<i64> = (0..k * n).map(|i| prng(12, i, -128, 127)).collect();
    let mut inputs = Inputs::default();
    inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
    inputs
        .tensors
        .insert(TensorId(1), Value::Int(weights.clone()));
    let expected = ints(&eval_choreography(&p, &inputs).unwrap().tensors[&output_id(&p)]);
    let art = lower(&p, target);
    let hex = logicaffeine_compile::spatial::lower::fabric::input_hex_files(&p, &inputs);
    let mut sources: Vec<(String, String)> = vec![
        ("spatial_top.v".into(), art.rtl.clone()),
        ("tb.v".into(), art.testbench.clone()),
    ];
    for (name, body) in art.hex.iter().chain(hex.iter()) {
        sources.push((name.clone(), body.clone()));
    }
    let refs: Vec<(&str, &str)> = sources
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let out = run_iverilog(&dir, "spatial_top", &refs);
    let (got, cycles, pass) = parse_outputs(&out);
    assert!(pass, "{label}: {out}");
    assert_eq!(got, expected, "{label}: RTL != eval_choreography");
    assert_eq!(
        cycles,
        fabric_cycle_model(&p, target),
        "{label}: the RTL cycle count IS the IR's cycle model"
    );
    assert!(
        cycles >= art.cycle_floor,
        "{label}: below the closed-form fill/drain floor {}",
        art.cycle_floor
    );
    assert_eq!(
        Census::of_verilog(&art.rtl),
        Census::of(&p),
        "{label}: RTL instance census == IR census"
    );
}

#[test]
fn a_two_by_two_site_multicast_gemm_in_rtl_equals_the_choreography() {
    let t = small_mesh();
    gemm_rtl_case("mcast2x2", 8, 8, 8, &t);
}

#[test]
fn a_streamed_gemm_with_a_ragged_k_in_rtl_equals_the_choreography() {
    let mut t = small_mesh();
    t.interconnect
        .remove(&logicaffeine_compile::spatial::target::Interconnect::MulticastRect);
    gemm_rtl_case("streamed", 8, 4, 10, &t);
}

#[test]
fn a_fused_block_in_rtl_equals_eval_fused_block() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tmp("fused");
    let t = SpatialTarget::sky_fabric(16, 16);
    let (k, n, m) = (4usize, 4usize, 5usize);
    let weights: Vec<i64> = (0..k * n).map(|i| prng(3, i, -110, 110)).collect();
    let acts: Vec<i64> = (0..m * k).map(|i| prng(4, i, -110, 110)).collect();
    let bias: Vec<i64> = (0..n).map(|i| prng(5, i, -400, 400)).collect();
    let fb = logicaffeine_compile::codegen_sva::systolic::FusedBlock {
        sys: Systolic::new(k, n, weights),
        requant_m: 181,
        requant_s: 8,
        bias,
    };
    let p = run_passes(from_fused_block(&fb, m, &t).unwrap(), &t).unwrap();
    let mut inputs = Inputs::default();
    inputs.tensors.insert(TensorId(0), Value::Int(acts.clone()));
    let expected = ints(&eval_choreography(&p, &inputs).unwrap().tensors[&output_id(&p)]);
    assert_eq!(
        expected,
        logicaffeine_compile::codegen_sva::systolic::eval_fused_block(&fb, &acts, m)
    );
    let art = lower(&p, &t);
    let hex = logicaffeine_compile::spatial::lower::fabric::input_hex_files(&p, &inputs);
    let mut sources: Vec<(String, String)> = vec![
        ("spatial_top.v".into(), art.rtl.clone()),
        ("tb.v".into(), art.testbench.clone()),
    ];
    for (name, body) in art.hex.iter().chain(hex.iter()) {
        sources.push((name.clone(), body.clone()));
    }
    let refs: Vec<(&str, &str)> = sources
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let out = run_iverilog(&dir, "spatial_top", &refs);
    let (got, cycles, pass) = parse_outputs(&out);
    assert!(pass && got == expected, "{out}");
    assert_eq!(cycles, fabric_cycle_model(&p, &t));
}

#[test]
fn a_scalar_stage_program_in_rtl_equals_the_choreography() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tmp("scalar");
    let t = SpatialTarget::sky_fabric(16, 16);
    for (src, name) in [
        ("## To affine (x: Int) -> Int:\n    Return 3 * x + 7.\n", "affine"),
        ("## To absval (a: Int) -> Int:\n    Let result be a.\n    If a is less than 0:\n        Set result to 0 - a.\n    Return result.\n", "absval"),
        ("## To half (x: Int) -> Int:\n    Return x / 2.\n", "half"),
    ] {
        let df = logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source(src, name);
        let p = run_passes(from_scalar_body(&df, 8, &t).unwrap(), &t).unwrap();
        let xs: Vec<i64> = (0..8).map(|i| prng(6, i, -1000, 1000)).collect();
        let mut inputs = Inputs::default();
        inputs.tensors.insert(TensorId(0), Value::Int(xs.clone()));
        let expected = ints(&eval_choreography(&p, &inputs).unwrap().tensors[&output_id(&p)]);
        let art = lower(&p, &t);
        let hex = logicaffeine_compile::spatial::lower::fabric::input_hex_files(&p, &inputs);
        let mut sources: Vec<(String, String)> = vec![("spatial_top.v".into(), art.rtl.clone()), ("tb.v".into(), art.testbench.clone())];
        for (n, body) in art.hex.iter().chain(hex.iter()) {
            sources.push((n.clone(), body.clone()));
        }
        let refs: Vec<(&str, &str)> = sources.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let out = run_iverilog(&dir, "spatial_top", &refs);
        let (got, _, pass) = parse_outputs(&out);
        assert!(pass && got == expected, "{name}: {out}");
    }
}

#[test]
fn the_mesh_top_synthesizes_clean_under_yosys() {
    if !common::require_tool("yosys") {
        return;
    }
    let dir = tmp("yosys");
    let t = small_mesh();
    let spec = MatmulSpec {
        m: 8,
        n: 8,
        k: 8,
        a: Default::default(),
        b: Default::default(),
        c: Default::default(),
    };
    let p = run_passes(
        template_from_spec(
            &spec,
            &names(),
            (&Fill::Unknown, &Fill::Unknown),
            (Some((-128, 127)), Some((-128, 127))),
            NumericsPolicy::Exact,
            &t,
        )
        .unwrap(),
        &t,
    )
    .unwrap();
    let art = lower(&p, &t);
    let rtl = dir.join("spatial_top.v");
    std::fs::write(&rtl, &art.rtl).unwrap();
    for (name, body) in &art.hex {
        std::fs::write(dir.join(name), body).unwrap();
    }
    let script = dir.join("synth.ys");
    std::fs::write(
        &script,
        format!(
            "read_verilog -sv {}\nhierarchy -check -top {}\nsynth -top {}\ncheck -assert\n",
            rtl.display(),
            art.top,
            art.top
        ),
    )
    .unwrap();
    let out = std::process::Command::new("yosys")
        .arg("-q")
        .arg("-s")
        .arg(&script)
        .current_dir(&dir)
        .output()
        .expect("yosys");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "yosys failed: {text}");
    assert!(
        !text.contains("problems") || text.contains("Found and reported 0 problems"),
        "{text}"
    );
}
