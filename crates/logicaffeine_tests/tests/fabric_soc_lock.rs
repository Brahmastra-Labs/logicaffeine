//! fabric_soc_lock — Phase E: the flashable reconfigurable-fabric SoC ≡ the VM, under iverilog.
//!
//! `fabric_soc` (Phase E) wraps the Phase-D reconfigurable array in registered I/O + a cycle counter, so a
//! host presents `(cfg, inputs)` and reads `(result, cycles)`. This lock EMITS the config-grid RTL, the SoC,
//! and its self-checking testbench, runs them under a real gate-level simulator (iverilog/vvp), and checks —
//! for every `(cfg, inputs)` vector — that the SoC's `result` equals the VM's answer for that datapath and
//! that `cycles == 1` (a spatial decision settles in ONE clock, the honest cycles/op). Skipped gracefully
//! where iverilog is absent; the Phase-D eval + SAT locks are simulator-free and cover that case.

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_soc::{
    emit_config_grid, emit_fabric_soc, emit_fabric_soc_tb, emit_state_soc, emit_state_soc_module,
    emit_state_soc_tb, state_soc_vectors, vectors, FABRIC_SOURCES, STATE_SOC_NAME, STATE_SOC_SRC,
};
use logicaffeine_compile::compile::vm_outcome;

/// Call `FABRIC_SOURCES[cfg]`'s function with `inputs` (only the function's arity) on the VM — the oracle.
fn vm_call(cfg: usize, inputs: &[i64]) -> i64 {
    let (src, name) = FABRIC_SOURCES[cfg];
    // Each parameter is `name: Int` (contributes a ": Int"); the return type is `-> Int` (colon AFTER Int,
    // so it does NOT match ": Int"). Hence the parameter count == the number of ": Int" occurrences.
    let arity = src.matches(": Int").count().max(1);
    let arg = |n: i64| {
        if n < 0 {
            format!("(0 - {})", -n)
        } else {
            n.to_string()
        }
    };
    let arglist = inputs[..arity]
        .iter()
        .map(|a| arg(*a))
        .collect::<Vec<_>>()
        .join(", ");
    let prog = format!("{src}## Main\n    Show {name}({arglist}).\n");
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error calling {name}: {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

#[test]
fn fabric_soc_rtl_matches_vm_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }

    let dir = std::env::temp_dir().join("logos_fabric_soc_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let cfgv = dir.join("config_fabric.v");
    let socv = dir.join("fabric_soc.v");
    let tbv = dir.join("tb_fabric_soc.v");
    let sim = dir.join("sim.vvp");
    std::fs::write(&cfgv, emit_config_grid()).expect("write config grid");
    std::fs::write(&socv, emit_fabric_soc()).expect("write soc");
    std::fs::write(&tbv, emit_fabric_soc_tb()).expect("write tb");

    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&cfgv)
        .arg(&socv)
        .arg(&tbv)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile fabric_soc: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);

    // Each result line is "cfg result cycles". Parse them in order and match against the vector list.
    let lines: Vec<(i64, i64, i64)> = stdout
        .lines()
        .map(str::trim)
        .filter_map(|l| {
            let ns: Vec<i64> = l
                .split_whitespace()
                .filter_map(|w| w.parse::<i64>().ok())
                .collect();
            if ns.len() == 3 {
                Some((ns[0], ns[1], ns[2]))
            } else {
                None
            }
        })
        .collect();

    let vs = vectors();
    assert_eq!(
        lines.len(),
        vs.len(),
        "expected one result line per vector ({}), got {}: {stdout:?}",
        vs.len(),
        lines.len()
    );

    for ((cfg, inputs), (rcfg, result, cycles)) in vs.iter().zip(lines.iter()) {
        assert_eq!(*rcfg, *cfg as i64, "config word echoed back wrong");
        let want = vm_call(*cfg as usize, inputs);
        assert_eq!(
            *result, want,
            "fabric_soc result {result} != VM {want} for cfg {cfg} inputs {inputs:?}"
        );
        assert_eq!(
            *cycles, 1,
            "a spatial decision must settle in ONE clock (cycles/op = 1), got {cycles} for cfg {cfg}"
        );
    }

    println!(
        "Phase E: fabric_soc ran {} vectors across {} reconfigured datapaths under iverilog — all ≡ VM, 1 cycle/op",
        vs.len(),
        FABRIC_SOURCES.len()
    );
}

/// Phase F LOCK — the reconfigurable fabric on REAL Artix-7 silicon. BOARD-GATED: runs only when
/// `LOGOS_FPGA_BOARD=1` (a Nexys A7-100T on Board B, `/dev/ttyUSB3`, sudo libusb access). It flashes
/// `nexys_fabric_soc.bit` and reads every datapath back over UART, asserting each result ≡ the VM on
/// hardware. Skipped gracefully off-board (the iverilog lock above + the Phase-D/G locks cover CI); this is
/// the on-silicon capstone. Reproduce with:
///   `LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test fabric_soc_lock -- --nocapture`
#[test]
fn fabric_runs_on_the_artix7_matching_the_vm() {
    if std::env::var("LOGOS_FPGA_BOARD").as_deref() != Ok("1") {
        eprintln!(
            "LOGOS_FPGA_BOARD != 1 — skipping the physical Artix-7 fabric proof (needs a Nexys A7 on Board B). \
             Reproduce: LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test fabric_soc_lock"
        );
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = std::process::Command::new("bash")
        .arg("scripts/validate-on-nexys-fabric.sh")
        .current_dir(&root)
        .status()
        .expect("failed to launch scripts/validate-on-nexys-fabric.sh");
    assert!(
        status.success(),
        "the reconfigurable fabric did not match the VM on the Artix-7 (see validate-on-nexys-fabric.sh output)"
    );
}

/// Call the flashed state-plane kernel `grade(a,b,c)` on the VM — the oracle for the state SoC.
fn vm_grade(a: i64, b: i64, c: i64) -> i64 {
    let arg = |n: i64| {
        if n < 0 {
            format!("(0 - {})", -n)
        } else {
            n.to_string()
        }
    };
    let prog = format!(
        "{STATE_SOC_SRC}## Main\n    Show {STATE_SOC_NAME}({}, {}, {}).\n",
        arg(a),
        arg(b),
        arg(c)
    );
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error calling {STATE_SOC_NAME}: {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

/// STAGE X2c iverilog LOCK — the flashable STATE-PLANE SoC ≡ the VM under a real gate-level simulator. The
/// flashed kernel `grade` is a WHOLE MULTI-OP-CLASS program (collections + arithmetic + control flow). Emits
/// the sequenced state-plane datapath (`state_grade`), the SoC wrapper with its cycle counter (`state_soc`),
/// and the self-checking testbench; runs them under iverilog/vvp; and checks — for every `(a,b,c)` vector —
/// that the SoC's `result` equals the VM's clamped sum AND that `cycles > 1` (a sequenced heap program is a
/// MULTI-cycle datapath, not a 1-clock combinational decision — the honest state-plane cost). This is the
/// board-independent proof that the flashable state-plane RTL is correct; the physical Artix-7 run is below.
#[test]
fn state_soc_rtl_matches_vm_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_state_soc_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let modv = dir.join("state_grade.v");
    let socv = dir.join("state_soc.v");
    let tbv = dir.join("tb_state_soc.v");
    let sim = dir.join("sim.vvp");
    std::fs::write(&modv, emit_state_soc_module()).expect("write state module");
    std::fs::write(&socv, emit_state_soc()).expect("write state soc");
    std::fs::write(&tbv, emit_state_soc_tb()).expect("write state tb");

    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&modv)
        .arg(&socv)
        .arg(&tbv)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile state_soc: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let lines: Vec<(i64, i64)> = stdout
        .lines()
        .map(str::trim)
        .filter_map(|l| {
            let ns: Vec<i64> = l
                .split_whitespace()
                .filter_map(|w| w.parse::<i64>().ok())
                .collect();
            if ns.len() == 2 {
                Some((ns[0], ns[1]))
            } else {
                None
            }
        })
        .collect();

    let vs = state_soc_vectors();
    assert_eq!(
        lines.len(),
        vs.len(),
        "expected one (result, cycles) line per vector ({}), got {}: {stdout:?}",
        vs.len(),
        lines.len()
    );
    for (v, (result, cycles)) in vs.iter().zip(lines.iter()) {
        let want = vm_grade(v[0], v[1], v[2]);
        assert_eq!(
            *result, want,
            "state_soc result {result} != VM {want} for inputs {v:?}"
        );
        assert!(
            *cycles > 1,
            "a sequenced state-plane program must take MORE than one clock (got {cycles} for {v:?}) — the \
             honest heap-datapath latency, not a 1-clock combinational decision"
        );
    }

    println!(
        "Stage X2c: the flashable state-plane SoC ran {} grade vectors (collections + arithmetic + control flow) under iverilog — every result ≡ VM, each a multi-cycle sequenced heap datapath",
        vs.len()
    );
}

/// STAGE X2c BOARD LOCK — the STATE PLANE (a whole multi-op-class program) on REAL Artix-7 silicon. BOARD-GATED
/// (`LOGOS_FPGA_BOARD=1`, a Nexys A7-100T on Board B). Flashes `nexys_state_soc.bit` and reads every clamped
/// `grade` back over UART, asserting each result ≡ the VM on hardware. Skipped gracefully off-board (the
/// iverilog lock above covers CI); this is the on-silicon capstone for the state plane. Reproduce:
///   `LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test fabric_soc_lock state_plane_runs_on`
#[test]
fn state_plane_runs_on_the_artix7_matching_the_vm() {
    if std::env::var("LOGOS_FPGA_BOARD").as_deref() != Ok("1") {
        eprintln!(
            "LOGOS_FPGA_BOARD != 1 — skipping the physical Artix-7 STATE-PLANE proof (needs a Nexys A7 on \
             Board B). Reproduce: LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test fabric_soc_lock"
        );
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = std::process::Command::new("bash")
        .arg("scripts/validate-on-nexys-state.sh")
        .current_dir(&root)
        .status()
        .expect("failed to launch scripts/validate-on-nexys-state.sh");
    assert!(
        status.success(),
        "the state plane did not match the VM on the Artix-7 (see validate-on-nexys-state.sh output)"
    );
}

/// STAGE X3b iverilog LOCK — the flashable FLOAT-LANE SoC ≡ the f64 reference under a real gate-level
/// simulator. Emits the sequenced float datapath (`float_fhypf`, binding `fpu.v`), the SoC wrapper with its
/// cycle counter (`float_soc`), and the testbench; compiles them WITH `fpu.v` under iverilog/vvp; and checks —
/// for every `(a,b)` float vector — that the SoC's f64 result BITS equal BOTH the model (`eval_grid` over the
/// f64 bit-patterns) AND Rust `f64` (`(a*a+b*b).sqrt()`), and that `cycles > 1` (a multi-cycle FPU pipeline).
/// The `fpu.v` units are already bit-exact to the VM (`float_heap_matches_vm`), so board == model == Rust ties
/// the silicon to the interpreter's float semantics. This is the board-independent proof; the Artix-7 run is
/// below.
#[test]
fn float_soc_rtl_matches_reference_under_iverilog() {
    use logicaffeine_compile::codegen_sva::fabric_grid::{eval_grid, route_loop_body};
    use logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source;
    use logicaffeine_compile::codegen_sva::fabric_soc::{
        emit_float_soc, emit_float_soc_module, emit_float_soc_tb, float_soc_vectors,
        FLOAT_SOC_NAME, FLOAT_SOC_SRC,
    };

    if !common::require_tool("iverilog") {
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fpu = std::fs::read_to_string(root.join("benchmarks/silicon/fpu.v")).expect("read fpu.v");

    let dir = std::env::temp_dir().join("logos_float_soc_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let modv = dir.join("float_fhypf.v");
    let fpuv = dir.join("fpu.v");
    let socv = dir.join("float_soc.v");
    let tbv = dir.join("tb_float_soc.v");
    let sim = dir.join("sim.vvp");
    std::fs::write(&modv, emit_float_soc_module()).expect("write float module");
    std::fs::write(&fpuv, &fpu).expect("write fpu.v");
    std::fs::write(&socv, emit_float_soc()).expect("write float soc");
    std::fs::write(&tbv, emit_float_soc_tb()).expect("write float tb");

    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&modv)
        .arg(&fpuv)
        .arg(&socv)
        .arg(&tbv)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile float_soc: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .output()
        .expect("run vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let lines: Vec<(u64, u64)> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let bits = u64::from_str_radix(it.next()?, 16).ok()?;
            let cyc = it.next()?.parse::<u64>().ok()?;
            Some((bits, cyc))
        })
        .collect();

    // The MODEL grid — the oracle the silicon is tied to (eval_grid over the SAME cells the datapath emits).
    let df = decision_fabric_from_source(FLOAT_SOC_SRC, FLOAT_SOC_NAME);
    let g = route_loop_body(&df.cells, df.n_in, &[df.result]);

    let vs = float_soc_vectors();
    assert_eq!(
        lines.len(),
        vs.len(),
        "one (result, cycles) line per vector: {stdout:?}"
    );
    for ((a, b), (bits, cycles)) in vs.iter().zip(lines.iter()) {
        let model = eval_grid(&g, &[a.to_bits() as i64, b.to_bits() as i64]) as u64;
        let rust = (a * a + b * b).sqrt().to_bits();
        assert_eq!(
            *bits, model,
            "float_soc {a},{b}: board bits != model (eval_grid)"
        );
        assert_eq!(
            *bits, rust,
            "float_soc {a},{b}: board bits != Rust f64 sqrt(a*a+b*b)"
        );
        assert!(
            *cycles > 1,
            "the float lane is a multi-cycle FPU pipeline — cycles must be > 1 (got {cycles})"
        );
    }

    println!(
        "Stage X3b: the flashable float-lane SoC ran {} hypotenuses under iverilog (fpu.v) — every f64 result bit-exact to the model AND Rust, a multi-cycle FPU pipeline",
        vs.len()
    );
}

/// STAGE X3b BOARD LOCK — the FLOAT LANE on REAL Artix-7 silicon. BOARD-GATED (`LOGOS_FPGA_BOARD=1`). Flashes
/// `nexys_float_soc.bit` and reads every hypotenuse back over UART (as f64 bits, floored by the host to the
/// integer hypotenuse), asserting each ≡ the reference on hardware. Skipped gracefully off-board. Reproduce:
///   `LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test fabric_soc_lock float_lane_runs_on`
#[test]
fn float_lane_runs_on_the_artix7_matching_the_reference() {
    if std::env::var("LOGOS_FPGA_BOARD").as_deref() != Ok("1") {
        eprintln!(
            "LOGOS_FPGA_BOARD != 1 — skipping the physical Artix-7 FLOAT-LANE proof (needs a Nexys A7 on \
             Board B). Reproduce: LOGOS_FPGA_BOARD=1 cargo test -p logicaffeine-tests --test fabric_soc_lock"
        );
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let status = std::process::Command::new("bash")
        .arg("scripts/validate-on-nexys-float.sh")
        .current_dir(&root)
        .status()
        .expect("failed to launch scripts/validate-on-nexys-float.sh");
    assert!(
        status.success(),
        "the float lane did not match the reference on the Artix-7 (see validate-on-nexys-float.sh output)"
    );
}
