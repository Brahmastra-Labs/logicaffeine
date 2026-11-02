//! ═══════════════════════════════════════════════════════════════════════════════════════════════
//! THE FABRIC FIFTY-MILLION-DOLLAR TEST — the SPATIAL-FABRIC analog of `fifty_million_dollar_test.rs`.
//! ═══════════════════════════════════════════════════════════════════════════════════════════════
//!
//! WHY THIS FILE EXISTS ($50M rationale)
//! ─────────────────────────────────────
//! The SOFT-CORE has `fifty_million_dollar_test.rs`: a coverage RATCHET proving every opcode the encoder
//! emits is DECODED in the real `soft_core.v`/`alu.v` AND EXERCISED by an iverilog tb. Building it caught 3
//! real "$50M silent-zero" holes (Word32Add/Sub/Mul, decoded on the model but falling through to `alu.v`'s
//! `default: y = 32'd0`). The SPATIAL FABRIC had NO equivalent. Its existing locks are DIFFERENTIALS, not
//! coverage ratchets:
//!   • `fabric_coverage_lock` certifies the MODEL (`fabric_tile` classifier eval ≡ VM, 115/115 ops), but
//!     entirely at the eval level — it never emits a single line of fabric Verilog.
//!   • `fabric_soc_lock` / `fabric_whole_program_lock` prove the fabric RTL ≡ VM under iverilog, but ONLY
//!     for the specific `(cfg, inputs)` datapaths their CORPUS tests exercise.
//! So the fabric could carry the SAME latent class as Word32Add: a compute tile-type the MODEL realizes
//! (and `fabric_coverage` COUNTS toward its 115/115 claim) that the RTL emitter PANICS on, never emits, or
//! that no iverilog testbench ever runs on real emitted Verilog. On tape-out silicon such a tile is a
//! catastrophic hole: the model says "realized", the chip computes nothing.
//!
//! THIS ALREADY EXISTED (caught + fixed by this file). `fabric_coverage` counted the compute Func tiles
//! `Pow`, `MulOvfS`, `ApproxEqF`, `FFloorF` as realized (each an eval≡VM witness), yet `FuncUnit::verilog`
//! PANICKED on all four — NO real emit path produced iverilog-runnable RTL for them, and NO lock iverilog'd
//! them. The MODEL outran the SILICON. This guard, written RED-first, listed exactly those four; the fix
//! added their real RTL (combinational for MulOvfS/ApproxEqF/FFloorF, the sequenced `pow_*` datapath for
//! Pow) + iverilog corpora that run them ≡ the VM, closing the gap.
//!
//! THE INVARIANT (strict — never weaken it to go green)
//! ─────────────────────────────────────────────────────
//! Every DISTINCT compute `FabricTile` the model produces for a silicon-realized op must:
//!   (A) be EMITTABLE to iverilog-RUNNABLE fabric Verilog by SOME real emit path (the emit_* RTL path, NOT
//!       the SAT-only `grid_to_bounded` path — SAT correctly DECLINES division/float/heap since QF_BV has no
//!       `bvsdiv`/IEEE-754/mutable-heap; those are proven by eval+iverilog instead), AND
//!   (B) be EXERCISED by an iverilog testbench that runs it ≡ the VM on the emitted fabric RTL.
//!
//!   • GUARD A (EMIT coverage) — for every distinct compute tile, a fabric using it EMITS valid fabric
//!     Verilog through a real path WITHOUT panicking, and iverilog COMPILES that Verilog. A hole is a tile
//!     that panics / emits nothing on ALL real emit paths (a genuine no-RTL hole).
//!   • GUARD B (IVERILOG-EXERCISE coverage) — every distinct compute tile is in at least one iverilog tb
//!     corpus a fabric lock (or this file's own corpus) actually runs on emitted RTL ≡ VM. EXERCISED is
//!     reconstructed from the `fabric_soc_lock` + `fabric_whole_program_lock` CORPORA the same way the
//!     soft-core Guard B reconstructs from its `verify-*.sh` corpora, UNIONed with the newly-realized-unit
//!     corpus this file iverilogs itself.
//!
//! Both fail LOUD with the tile-type list + a reproducer. Non-vacuous: the covered set is asserted non-empty
//! and to include a representative HARD tile (the `DivS` Func division tile), so the guard cannot pass by
//! watching nothing.
//!
//! ⚠️  DO NOT weaken this file to make a RED pass. A RED means the fabric is missing an RTL emit path (fix
//! `FuncUnit::verilog` / `fabric_grid` / a sequenced datapath emitter) or an exercise corpus (add an
//! iverilog'd program). The whole point is that the invariant is STRICT — that is what makes the fabric as
//! airtight as the soft-core. NEVER demote `fabric_coverage`'s claim to hide a tile with no RTL path. ⚠️

use std::collections::{BTreeMap, BTreeSet};

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, decision_fabric_has_state, decision_fabric_is_float_arith,
    decision_fabric_is_pow, emit_decision_fabric_testbench_dummy, emit_decision_fabric_verilog,
    emit_float_datapath_verilog, emit_pow_datapath_testbench, emit_pow_datapath_verilog,
    emit_state_plane_verilog, eval_decision_fabric, AluOp, CmpKind, DecisionFabric, FuncUnit,
    LoopCell, Src, STATE_PLANE_HEAP_WORDS,
};
use logicaffeine_compile::codegen_sva::fabric_tile::{fabric_tile, FabricTile};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::vm::Op;

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// THE COMPUTE-TILE SURFACE — enumerated IN CODE from `fabric_tile` (never a stale hardcoded list).
//
// `fabric_tile(op)` is the single source of truth `fabric_coverage_lock` keys off. We walk every ISA op,
// classify it, and collect every DISTINCT compute tile (`Alu`/`Cmp`/`Func`/`Mem`). A `Debug`-string key
// canonicalizes each tile's discriminant (the enums are small `Copy` value types). Plus the five FLOAT-ARITH
// Func units (`FAddF/FSubF/FMulF/FDivF/FSqrtF`) — these do not fall out of `fabric_tile(op)` (a float `+`/`*`
// is `FabricTile::Alu` at the OP level, becoming a float `LoopCell::Func` only INSIDE a decision fabric once
// the operands are floats), yet they are genuine compute tiles the emitter produces and must run on silicon.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// A canonical string key for a compute tile-type (its discriminant), used as a set element.
fn tile_key(t: &FabricTile) -> String {
    match t {
        FabricTile::Alu(op) => format!("Alu::{op:?}"),
        FabricTile::Cmp(k) => format!("Cmp::{k:?}"),
        FabricTile::Func(u) => format!("Func::{u:?}"),
        FabricTile::Mem => "Mem".to_string(),
        // Structural tiles carry no compute datapath — never in the surface (filtered by is_compute).
        FabricTile::Wire | FabricTile::Control => unreachable!("structural tile is not compute"),
    }
}

/// A canonical key for a float-arith Func unit (the cell-level units not surfaced by `fabric_tile(op)`).
fn func_key(u: FuncUnit) -> String {
    format!("Func::{u:?}")
}

/// EMITTED = the union of DISTINCT compute tile-types over the whole ISA (each op → `fabric_tile` → filtered
/// to compute), PLUS the five float-arith cell units. Returns tile-key → a short human origin, so a RED can
/// name a concrete reproducer. This is the ground truth of what the fabric MUST be able to run on silicon.
fn emitted_compute_tiles() -> BTreeMap<String, String> {
    let mut origin: BTreeMap<String, String> = BTreeMap::new();
    for op in Op::all_variants() {
        if let Some(t) = fabric_tile(&op) {
            if t.is_compute() {
                origin
                    .entry(tile_key(&t))
                    .or_insert_with(|| format!("op {}", op.name()));
            }
        }
    }
    // The float-arith lane: produced inside float decision fabrics (see the block comment). Their origin is
    // the float kernels the float lock iverilogs.
    for u in [
        FuncUnit::FAddF,
        FuncUnit::FSubF,
        FuncUnit::FMulF,
        FuncUnit::FDivF,
        FuncUnit::FSqrtF,
    ] {
        origin
            .entry(func_key(u))
            .or_insert_with(|| "float decision fabric (a * b, sqrt, …)".to_string());
    }
    origin
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// EMIT-OK — the tiles a REAL emit path lowers to iverilog-RUNNABLE fabric Verilog without panicking. For
// each distinct compute tile we build the MINIMAL fabric carrying exactly that tile, drive it through the
// correct real emitter (never the SAT `grid_to_bounded` path), and iverilog-COMPILE the result. A tile whose
// every real emit path PANICS / emits nothing is a genuine no-RTL hole (the $50M class).
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// The real emit path for a tile: which emitter lowers it, and the Verilog it produces (or a panic message).
/// `catch_unwind` around the emit turns a `FuncUnit::verilog` panic into a caught hole rather than a crash.
fn emit_tile_rtl(key: &str) -> Result<String, String> {
    // Two dummy inputs for a binary tile; a float bit-pattern for the float→int floor bridge.
    let bin = |cell: LoopCell| DecisionFabric {
        n_in: 2,
        cells: vec![cell],
        result: Src::Val(0),
        escaped_first_return: false,
    };
    let df: DecisionFabric = if let Some(op) = alu_of(key) {
        bin(LoopCell::Arith {
            op,
            a: Src::In(0),
            b: Src::In(1),
        })
    } else if let Some(cmp) = cmp_of(key) {
        // A comparison tile realizes as a predicated Select (its combinational form on the fabric).
        bin(LoopCell::Select {
            cmp,
            ca: Src::In(0),
            cb: Src::In(1),
            t: Src::Const(1),
            e: Src::Const(0),
        })
    } else if let Some(u) = func_of(key) {
        bin(LoopCell::Func {
            unit: u,
            a: Src::In(0),
            b: Src::In(1),
        })
    } else if key == "Mem" {
        // A heap read/write/alloc chain — a state-plane fabric (`emit_state_plane_verilog`).
        DecisionFabric {
            n_in: 1,
            cells: vec![
                LoopCell::HeapAlloc {
                    size: Src::Const(2),
                }, // ptr = alloc 2
                LoopCell::HeapWrite {
                    addr: Src::Val(0),
                    val: Src::In(0),
                }, // heap[ptr] = in0
                LoopCell::HeapRead { addr: Src::Val(0) }, // read it back
            ],
            result: Src::Val(2),
            escaped_first_return: false,
        }
    } else {
        return Err(format!(
            "unknown tile key `{key}` — the surface enumerator produced a tile with no emit route"
        ));
    };

    // Select the real emit path exactly as the production locks do, and CATCH any panic (a no-RTL hole).
    let name = key.replace([':', '<', '>', ' '], "_").to_lowercase();
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if decision_fabric_has_state(&df) {
            emit_state_plane_verilog(&name, &df, STATE_PLANE_HEAP_WORDS)
        } else if decision_fabric_is_pow(&df) {
            emit_pow_datapath_verilog(&name, &df)
        } else if decision_fabric_is_float_arith(&df) {
            emit_float_datapath_verilog(&name, &df)
        } else {
            emit_decision_fabric_verilog(&name, &df)
        }
    }));
    out.map_err(|e| {
        let msg = e
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "panicked (non-string payload)".to_string());
        format!("emit PANICKED: {}", msg.lines().next().unwrap_or("panic"))
    })
}

fn alu_of(key: &str) -> Option<AluOp> {
    for op in [
        AluOp::Add,
        AluOp::Sub,
        AluOp::Mul,
        AluOp::And,
        AluOp::Or,
        AluOp::Xor,
        AluOp::Shl,
        AluOp::Shr,
    ] {
        if key == format!("Alu::{op:?}") {
            return Some(op);
        }
    }
    None
}
fn cmp_of(key: &str) -> Option<CmpKind> {
    for k in [
        CmpKind::Lt,
        CmpKind::Gt,
        CmpKind::Le,
        CmpKind::Ge,
        CmpKind::Eq,
        CmpKind::Ne,
    ] {
        if key == format!("Cmp::{k:?}") {
            return Some(k);
        }
    }
    None
}
fn func_of(key: &str) -> Option<FuncUnit> {
    for u in [
        FuncUnit::DivS,
        FuncUnit::ModS,
        FuncUnit::FloorDivS,
        FuncUnit::Pow,
        FuncUnit::MulHiU,
        FuncUnit::ApproxEqF,
        FuncUnit::FAddF,
        FuncUnit::FSubF,
        FuncUnit::FMulF,
        FuncUnit::FDivF,
        FuncUnit::FSqrtF,
        FuncUnit::FFloorF,
        FuncUnit::MulOvfS,
    ] {
        if key == format!("Func::{u:?}") {
            return Some(u);
        }
    }
    None
}

/// True iff `iverilog`+`vvp` are on PATH — the RTL compile/exercise checks are skipped gracefully otherwise
/// (the eval-level `fabric_coverage_lock` covers that case; CI has iverilog).
fn iverilog_available() -> bool {
    common::tool_available("iverilog") && common::tool_available("vvp")
}

/// iverilog-COMPILE a self-contained fabric Verilog snippet (Guard A's "runnable" proof — it elaborates a
/// real gate-level netlist, catching syntax / width / undeclared-net errors a mere string check would miss).
/// The float lane additionally needs `fpu.v`; supply it via `extra`. Returns Ok(()) or the iverilog stderr.
fn iverilog_compiles(name: &str, rtl: &str, extra: &[(&str, String)]) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("logos_fab50_emit_{name}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let srcv = dir.join(format!("{name}.v"));
    std::fs::write(&srcv, rtl).map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new("iverilog");
    cmd.arg("-g2012")
        .arg("-o")
        .arg(dir.join("sim.vvp"))
        .arg(&srcv);
    for (fname, body) in extra {
        let p = dir.join(fname);
        std::fs::write(&p, body).map_err(|e| e.to_string())?;
        cmd.arg(&p);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .take(6)
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// EXERCISED — the compute tiles that a fabric lock actually RUNS on emitted RTL ≡ VM under iverilog:
//   • fabric_soc_lock:  FABRIC_SOURCES (affine/clamp/sign/min2/ratio) via emit_config_grid, STATE_SOC (grade)
//                       via emit_state_plane, FLOAT_SOC (fhypf) via emit_float_datapath.
//   • fabric_whole_program_lock: STATE_KERNELS (fab_new/fab_set/fab_sum) via emit_state_plane.
//   • THIS FILE: the newly-realized-unit corpus (Pow/MulOvfS/ApproxEqF/FFloorF), iverilog'd ≡ VM below.
// We RECONSTRUCT the lock corpora's tiles the same way the soft-core Guard B reconstructs from its corpora:
// compile each program to its fabric and collect the compute-tile discriminants it places.
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// The `(src, fn_name)` programs the two existing fabric locks iverilog. Kept in lockstep with those locks'
/// corpora (they are `const`/private in sibling binaries and cannot be imported).
const LOCK_IVERILOG_CORPUS: &[(&str, &str)] = &[
    // fabric_soc_lock::FABRIC_SOURCES — the config-grid corpus (Arith / Select / DivS).
    ("## To affine (x: Int) -> Int:\n    Return 3 * x + 7.\n", "affine"),
    ("## To clamp (x: Int, lo: Int, hi: Int) -> Int:\n    Let result be x.\n    If x is less than lo:\n        Set result to lo.\n    If x is greater than hi:\n        Set result to hi.\n    Return result.\n", "clamp"),
    ("## To sign (n: Int) -> Int:\n    Let result be 0.\n    If n is less than 0:\n        Set result to 0 - 1.\n    If n is greater than 0:\n        Set result to 1.\n    Return result.\n", "sign"),
    ("## To min2 (a: Int, b: Int) -> Int:\n    Let result be b.\n    If a is less than b:\n        Set result to a.\n    Return result.\n", "min2"),
    ("## To ratio (a: Int, b: Int) -> Int:\n    Return a / b.\n", "ratio"),
    // fabric_soc_lock::STATE_SOC_SRC — the state plane (Mem + Arith + Select).
    ("## To grade (a: Int, b: Int, c: Int) -> Int:\n    Let xs be [a, b, c].\n    Let total be item 1 of xs + item 2 of xs + item 3 of xs.\n    Let result be total.\n    If total is greater than 100:\n        Set result to 100.\n    If total is less than 0:\n        Set result to 0.\n    Return result.\n", "grade"),
    // fabric_soc_lock::FLOAT_SOC_SRC — the float lane (FMulF ×2 + FAddF + FSqrtF).
    ("## To fhypf (a: Float, b: Float) -> Real:\n    Return sqrt(a * a + b * b).\n", "fhypf"),
    // fabric_whole_program_lock::STATE_KERNELS — more state-plane heap fabrics (Mem + Arith).
    ("## To fab_new (a: Int, b: Int, c: Int, i: Int) -> Int:\n    Let ys be [a, b, c].\n    Return item i of ys.\n", "fab_new"),
    ("## To fab_set (a: Int, b: Int, c: Int, i: Int, v: Int) -> Int:\n    Let mutable ys be [a, b, c].\n    Set item i of ys to v.\n    Return item i of ys.\n", "fab_set"),
    ("## To fab_sum (a: Int, b: Int, c: Int) -> Int:\n    Let ys be [a, b, c].\n    Return item 1 of ys + item 2 of ys + item 3 of ys.\n", "fab_sum"),
];

/// Collect the compute-tile keys a decision/state fabric PLACES (the discriminant of every `LoopCell` that is
/// a compute tile). Float-arith cells contribute their float Func key; a `Func::FFloorF` also surfaces here.
fn tiles_of_fabric(df: &DecisionFabric) -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    for c in &df.cells {
        match c {
            LoopCell::Arith { op, .. } => {
                s.insert(format!("Alu::{op:?}"));
            }
            LoopCell::Select { cmp, .. } => {
                s.insert(format!("Cmp::{cmp:?}"));
            }
            LoopCell::Func { unit, .. } => {
                s.insert(func_key(*unit));
            }
            LoopCell::HeapRead { .. }
            | LoopCell::HeapWrite { .. }
            | LoopCell::HeapAlloc { .. }
            | LoopCell::MemRead { .. } => {
                s.insert("Mem".to_string());
            }
            LoopCell::VectorArith { op, ty, .. } => {
                s.insert(format!("Simd::{op:?}::{ty:?}"));
            }
        }
    }
    s
}

/// EXERCISED-from-locks = the tiles the existing locks' iverilog corpora place (reconstructed by compiling
/// each corpus program to its fabric). This is the tile set an existing `verify`/`lock` run genuinely
/// iverilogs on emitted RTL ≡ VM.
fn exercised_by_existing_locks() -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for (src, name) in LOCK_IVERILOG_CORPUS {
        let df = decision_fabric_from_source(src, name);
        set.extend(tiles_of_fabric(&df));
    }
    set
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// THE NEWLY-REALIZED-UNIT CORPUS — the tiles the two existing locks do NOT iverilog (Pow / MulOvfS /
// ApproxEqF / FFloorF). This file iverilogs each ≡ the VM on the emitted fabric RTL, so it is a genuine
// exercise corpus (not a claim). Each entry: a canonical tile key, the fabric that carries it, a fuzz
// battery, and the VM oracle. `run_new_unit_corpus` returns the set it PROVED (∅ if iverilog is absent).
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// The VM oracle for a hand-built unit over a single `(a, b)` — via `eval_decision_fabric` on the SAME cells,
/// which `fabric_coverage_lock` already pins ≡ the VM. (A hand-bytecode tile like `MulOvfS` has no surface
/// syntax, so the eval — proven ≡ VM in the coverage lock — is the reference the RTL must match.)
fn unit_oracle(df: &DecisionFabric, a: i64, b: i64) -> i64 {
    eval_decision_fabric(df, &[a, b])
}

/// Run ONE combinational decision-fabric cell (`Arith` / `Select` / `Func`) through
/// `emit_decision_fabric_verilog` + a self-checking tb over a battery, asserting the emitted RTL ≡
/// `eval_decision_fabric` (which `fabric_coverage_lock` pins ≡ the VM) under iverilog. This is the ONE genuine
/// iverilog-exercise path every single-cell tile shares — the combinational Alu tiles, the comparison Select
/// tiles, and the sequential-in-name-only Func tiles all flow through it, so closing an exercise gap is a
/// battery + a call, never a new untested harness. Returns true iff it PASSED (panics on a genuine mismatch).
fn exercise_single_cell(name: &str, cell: LoopCell, battery: &[(i64, i64)]) -> bool {
    let df = DecisionFabric {
        n_in: 2,
        cells: vec![cell],
        result: Src::Val(0),
        escaped_first_return: false,
    };
    let rtl = emit_decision_fabric_verilog(name, &df);
    let tb = emit_decision_fabric_testbench_dummy(name, 2, battery.len());
    let dir = std::env::temp_dir().join(format!("logos_fab50_ex_{name}"));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut inhex = String::new();
    let mut outhex = String::new();
    for (a, b) in battery {
        inhex.push_str(&format!("{:016x}\n{:016x}\n", *a as u64, *b as u64));
        outhex.push_str(&format!("{:016x}\n", unit_oracle(&df, *a, *b) as u64));
    }
    std::fs::write(dir.join(format!("dec_{name}.v")), &rtl).expect("rtl");
    std::fs::write(dir.join(format!("tb_dec_{name}.v")), &tb).expect("tb");
    std::fs::write(dir.join(format!("dec_{name}_in.hex")), &inhex).expect("in");
    std::fs::write(dir.join(format!("dec_{name}_out.hex")), &outhex).expect("out");
    let sim = dir.join("sim.vvp");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(dir.join(format!("dec_{name}.v")))
        .arg(dir.join(format!("tb_dec_{name}.v")))
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile the {name} decision fabric:\n{}\n--- RTL ---\n{rtl}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "the {name} decision tile RTL != eval(≡VM) under iverilog:\n{stdout}\n--- RTL ---\n{rtl}"
    );
    true
}

/// Exercise a single combinational Func tile (`ModS`/`FloorDivS`/`MulHiU`/`MulOvfS`/`ApproxEqF`/`FFloorF`).
fn exercise_combinational_func(name: &str, unit: FuncUnit, battery: &[(i64, i64)]) -> bool {
    exercise_single_cell(
        name,
        LoopCell::Func {
            unit,
            a: Src::In(0),
            b: Src::In(1),
        },
        battery,
    )
}

/// Exercise a single combinational Alu tile (`Sub`/`And`/`Or`/`Xor`/`Shl`/`Shr` — the `Add`/`Mul` twins are
/// already exercised by the `affine`/`grade` corpus programs, so these six close the arithmetic-lane gap).
fn exercise_arith(name: &str, op: AluOp, battery: &[(i64, i64)]) -> bool {
    exercise_single_cell(
        name,
        LoopCell::Arith {
            op,
            a: Src::In(0),
            b: Src::In(1),
        },
        battery,
    )
}

/// Exercise a single comparison tile (`Le`/`Ge`/`Eq`/`Ne` — the `Lt`/`Gt` twins are already exercised by the
/// `clamp`/`sign`/`min2` corpus, so these four close the comparison-lane gap). A comparison realizes on the
/// fabric EXACTLY as `emit_tile_rtl` classifies it: a predicated `Select` yielding `1`/`0`.
fn exercise_cmp(name: &str, cmp: CmpKind, battery: &[(i64, i64)]) -> bool {
    exercise_single_cell(
        name,
        LoopCell::Select {
            cmp,
            ca: Src::In(0),
            cb: Src::In(1),
            t: Src::Const(1),
            e: Src::Const(0),
        },
        battery,
    )
}

/// Run the sequenced `pow_*` datapath ≡ the VM under iverilog over an exponentiation battery.
fn exercise_pow() -> bool {
    // `pw(a, b) = a ** b` as a straight-line LOGOS function → a single `Func::Pow` decision-fabric cell.
    let src = "## To pw (a: Int, b: Int) -> Int:\n    Return a to the power of b.\n";
    let df = decision_fabric_from_source(src, "pw");
    assert!(
        decision_fabric_is_pow(&df),
        "the pw fabric must be a pure Pow datapath, got {:?}",
        df.cells
    );
    // NON-NEGATIVE exponents only, and results kept inside i64 (the VM's integer power is EXACT and promotes
    // to BigInt on overflow / errors on a negative exponent — the fabric wraps, so equality holds only in the
    // i64-fitting, non-negative-exponent domain, exactly the FuncUnit::Pow witness domain).
    let battery: &[(i64, i64)] = &[
        (2, 10),
        (3, 4),
        (5, 0),
        (-2, 3),
        (-2, 4),
        (10, 3),
        (7, 2),
        (1, 63),
        (-1, 5),
        (2, 30),
    ];
    let rtl = emit_pow_datapath_verilog("pw", &df);
    let tb = emit_pow_datapath_testbench("pw", 2, battery.len(), 300);
    let dir = std::env::temp_dir().join("logos_fab50_pow");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut inhex = String::new();
    let mut outhex = String::new();
    for (a, b) in battery {
        inhex.push_str(&format!("{:016x}\n{:016x}\n", *a as u64, *b as u64));
        let want = vm_call2(src, "pw", *a, *b);
        outhex.push_str(&format!("{:016x}\n", want as u64));
    }
    std::fs::write(dir.join("pow_pw.v"), &rtl).expect("rtl");
    std::fs::write(dir.join("tb_pow_pw.v"), &tb).expect("tb");
    std::fs::write(dir.join("pow_pw_in.hex"), &inhex).expect("in");
    std::fs::write(dir.join("pow_pw_out.hex"), &outhex).expect("out");
    let sim = dir.join("sim.vvp");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(dir.join("pow_pw.v"))
        .arg(dir.join("tb_pow_pw.v"))
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile the pow datapath:\n{}\n--- RTL ---\n{rtl}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "the pow datapath RTL != VM under iverilog:\n{stdout}\n--- RTL ---\n{rtl}"
    );
    true
}

/// The `(a, b)` batteries for the combinational units, spanning the corners each unit's eval distinguishes.
fn mulovf_battery() -> Vec<(i64, i64)> {
    vec![
        (6_000_000_000, 6_000_000_000),
        (3_000_000_000, 3_000_000_000),
        (-3_000_000_000, 3_000_000_000),
        (100, 200),
        (i64::MAX, 2),
        (i64::MIN, -1),
        (0, 12345),
        (1, 1),
    ]
}
fn approx_battery() -> Vec<(i64, i64)> {
    vec![
        (1, 1),
        (1, 2),
        (2_000_000_000, 2_000_000_001),
        (1_000_000_000, 1_000_000_002),
        (0, 0),
        (-5, -5),
        (-1_000_000_000, -1_000_000_001),
        (7, 9),
    ]
}
fn ffloor_battery() -> Vec<(i64, i64)> {
    // FFloorF's operand is a float BIT-PATTERN (in0); in1 is a dummy 0.
    [
        5.0_f64, 2.5, -2.5, 1.5, -1.5, 0.0, 0.5, -0.5, 100.0, -100.3, 1e10, -1e10, 1.0, -1.0,
    ]
    .iter()
    .map(|f| (f.to_bits() as i64, 0))
    .collect()
}

/// A `(a, b)` battery spanning the corners the ALU/comparison lanes distinguish: sign combinations, zero,
/// bit-pattern overlap for the bitwise lane, shift counts up to and past the mod-64 mask, and the 64-bit
/// wrapping edges (`i64::MIN`/`i64::MAX`). ONE battery serves every single-cell Alu/Cmp tile — each `op.eval`
/// is total over it, and the RTL must agree bit-for-bit.
fn alu_cmp_battery() -> Vec<(i64, i64)> {
    vec![
        (0, 0),
        (1, 1),
        (1, 2),
        (2, 1),
        (-1, 1),
        (1, -1),
        (-7, -3),
        (-3, -7),
        (12345, 67890),
        (0xF0F0_F0F0, 0x0F0F_0F0F),
        (
            0xFFFF_FFFF_FFFF_FFFFu64 as i64,
            0x5555_5555_5555_5555u64 as i64,
        ),
        (5, 3),
        (5, 65),
        (-8, 2),
        (1, 63),
        (1, 64),
        (i64::MIN, -1),
        (i64::MAX, 1),
        (i64::MAX, i64::MIN),
        (-100, 100),
    ]
}

/// Run a SINGLE-CELL float-arith unit (`FAddF`/`FSubF`/`FMulF`/`FDivF`/`FSqrtF`) through the SEQUENCED
/// `emit_float_datapath_verilog` lane (binding `fpu.v`) ≡ the eval (which `fabric_coverage_lock` pins ≡ VM and
/// `float_soc_lock` ties to Rust f64), over a battery of f64 bit-patterns. `fhypf` (the existing float lock)
/// only exercises FMulF/FAddF/FSqrtF — FSubF and FDivF have NO existing iverilog run, so this corpus closes
/// that exercise gap. Returns true iff the datapath's f64 result bits match the eval under iverilog.
fn exercise_float_unit(name: &str, unit: FuncUnit, battery: &[(f64, f64)]) -> bool {
    let df = DecisionFabric {
        n_in: 2,
        cells: vec![LoopCell::Func {
            unit,
            a: Src::In(0),
            b: Src::In(1),
        }],
        result: Src::Val(0),
        escaped_first_return: false,
    };
    assert!(
        decision_fabric_is_float_arith(&df),
        "{name} must be a pure float-arith datapath"
    );
    let rtl = emit_float_datapath_verilog(name, &df);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fpu = std::fs::read_to_string(root.join("benchmarks/silicon/fpu.v")).expect("read fpu.v");
    let dir = std::env::temp_dir().join(format!("logos_fab50_float_{name}"));
    std::fs::create_dir_all(&dir).expect("temp dir");
    // A clocked self-checking tb (the float lane is multi-cycle): drive each (a,b) bit-pattern, run to `done`,
    // compare the f64 result bits against the eval oracle.
    let mut runs = String::new();
    for (a, b) in battery {
        let want = eval_decision_fabric(&df, &[a.to_bits() as i64, b.to_bits() as i64]) as u64;
        runs.push_str(&format!(
            "    run(64'h{:016x}, 64'h{:016x}, 64'h{:016x});\n",
            a.to_bits(),
            b.to_bits(),
            want
        ));
    }
    let tb = format!(
        "`timescale 1ns/1ps\nmodule tb_float_{name};\n  reg clk=0, rst; reg [63:0] in0, in1; wire [63:0] result; wire done;\n  integer c, errors=0;\n  float_{name} dut(.clk(clk), .rst(rst), .in0(in0), .in1(in1), .result(result), .done(done));\n  always #5 clk=~clk;\n  task run(input [63:0] a, input [63:0] b, input [63:0] want); begin\n    @(negedge clk); in0=a; in1=b; rst=1; @(negedge clk); rst=0;\n    c=0; while(!done && c<5000) begin @(negedge clk); c=c+1; end\n    if (result !== want) begin errors=errors+1; $display(\"FAIL %h != %h\", result, want); end\n  end endtask\n  initial begin\n{runs}    if (errors==0) $display(\"PASS: {n} float-datapath results match the eval (fpu.v lane)\"); else $display(\"FAIL: %0d\", errors);\n    $finish;\n  end\nendmodule\n",
        n = battery.len(),
    );
    std::fs::write(dir.join(format!("float_{name}.v")), &rtl).expect("rtl");
    std::fs::write(dir.join("fpu.v"), &fpu).expect("fpu");
    std::fs::write(dir.join(format!("tb_float_{name}.v")), &tb).expect("tb");
    let sim = dir.join("sim.vvp");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(dir.join(format!("float_{name}.v")))
        .arg(dir.join("fpu.v"))
        .arg(dir.join(format!("tb_float_{name}.v")))
        .current_dir(&dir)
        .output()
        .expect("iverilog");
    assert!(
        build.status.success(),
        "iverilog failed to compile the {name} float datapath:\n{}\n--- RTL ---\n{rtl}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .expect("vvp");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        stdout.contains("PASS:"),
        "the {name} float datapath RTL != eval under iverilog:\n{stdout}\n--- RTL ---\n{rtl}"
    );
    true
}

/// Run every newly-realized-unit corpus program under iverilog ≡ VM/eval, returning the set of tile keys PROVEN.
fn exercised_by_new_unit_corpus() -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    if !iverilog_available() {
        return set;
    }
    if exercise_combinational_func("mulovf", FuncUnit::MulOvfS, &mulovf_battery()) {
        set.insert(func_key(FuncUnit::MulOvfS));
    }
    if exercise_combinational_func("approx", FuncUnit::ApproxEqF, &approx_battery()) {
        set.insert(func_key(FuncUnit::ApproxEqF));
    }
    if exercise_combinational_func("ffloor", FuncUnit::FFloorF, &ffloor_battery()) {
        set.insert(func_key(FuncUnit::FFloorF));
    }
    if exercise_pow() {
        set.insert(func_key(FuncUnit::Pow));
    }
    // The float lane — every single-cell fpu.v unit. `fhypf` (the existing float lock) covers FMulF/FAddF/
    // FSqrtF; this corpus additionally runs FSubF and FDivF (no existing iverilog run), and re-covers the
    // rest so the whole float lane is exercised by an owned corpus, not a fragile reconstruction of fhypf.
    let float_batt: &[(f64, f64)] = &[
        (3.5, 2.0),
        (7.0, 2.5),
        (10.0, 4.0),
        (0.0, 5.0),
        (-3.5, 1.25),
        (9.0, 3.0),
    ];
    let sqrt_batt: &[(f64, f64)] = &[
        (4.0, 0.0),
        (2.0, 0.0),
        (20.25, 0.0),
        (0.0, 0.0),
        (100.0, 0.0),
    ];
    for (name, unit) in [
        ("fadd", FuncUnit::FAddF),
        ("fsub", FuncUnit::FSubF),
        ("fmul", FuncUnit::FMulF),
        ("fdiv", FuncUnit::FDivF),
    ] {
        if exercise_float_unit(name, unit, float_batt) {
            set.insert(func_key(unit));
        }
    }
    if exercise_float_unit("fsqrt", FuncUnit::FSqrtF, sqrt_batt) {
        set.insert(func_key(FuncUnit::FSqrtF));
    }
    // The ARITHMETIC lane — the six combinational Alu tiles NO existing lock iverilogs (`Sub`/bitwise/shift;
    // `Add`/`Mul` are covered by the `affine`/`grade` corpus). Each runs on emitted decision-fabric RTL ≡ eval.
    let alu_cmp = alu_cmp_battery();
    for (name, op) in [
        ("alu_sub", AluOp::Sub),
        ("alu_and", AluOp::And),
        ("alu_or", AluOp::Or),
        ("alu_xor", AluOp::Xor),
        ("alu_shl", AluOp::Shl),
        ("alu_shr", AluOp::Shr),
    ] {
        if exercise_arith(name, op, &alu_cmp) {
            set.insert(format!("Alu::{op:?}"));
        }
    }
    // The COMPARISON lane — the four comparison tiles NO existing lock iverilogs (`Le`/`Ge`/`Eq`/`Ne`; `Lt`/`Gt`
    // are covered by the `clamp`/`sign`/`min2` corpus). Each realizes as a predicated Select yielding 1/0 ≡ eval.
    for (name, cmp) in [
        ("cmp_le", CmpKind::Le),
        ("cmp_ge", CmpKind::Ge),
        ("cmp_eq", CmpKind::Eq),
        ("cmp_ne", CmpKind::Ne),
    ] {
        if exercise_cmp(name, cmp, &alu_cmp) {
            set.insert(format!("Cmp::{cmp:?}"));
        }
    }
    // The DIVISION-CLASS Func tiles NO existing lock iverilogs (`ratio` covers only `DivS`): the remainder
    // (`ModS`), the floored quotient (`FloorDivS`), and the magic-reciprocal high-half multiply (`MulHiU`).
    // Batteries span sign combinations, the `b == 0 → 0` trap, and the full-width unsigned product.
    if exercise_combinational_func("mods", FuncUnit::ModS, &alu_cmp) {
        set.insert(func_key(FuncUnit::ModS));
    }
    if exercise_combinational_func("floordivs", FuncUnit::FloorDivS, &alu_cmp) {
        set.insert(func_key(FuncUnit::FloorDivS));
    }
    if exercise_combinational_func("mulhiu", FuncUnit::MulHiU, &alu_cmp) {
        set.insert(func_key(FuncUnit::MulHiU));
    }
    set
}

// ── VM oracle helper (shared) ────────────────────────────────────────────────────────────────────────
fn arg(n: i64) -> String {
    if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    }
}
fn vm_call2(fn_src: &str, name: &str, a: i64, b: i64) -> i64 {
    let src = format!(
        "{fn_src}## Main\n    Show {name}({}, {}).\n",
        arg(a),
        arg(b)
    );
    let out = vm_outcome(&src);
    assert!(
        out.error.is_none(),
        "VM error on {name}({a}, {b}): {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

// ─────────────────────────────────────────────────────────────────────────────────────────────────
// THE GUARDS
// ─────────────────────────────────────────────────────────────────────────────────────────────────

/// ★ GUARD A — EMIT COVERAGE (the killer guard) ★
/// Every distinct compute tile the model produces MUST emit iverilog-runnable fabric Verilog through some
/// REAL path (never the SAT path) without panicking, and iverilog must COMPILE it. A hole is a tile that
/// panics / emits nothing on ALL real paths — the fabric analog of the Word32 silent-zero: model-realized,
/// silicon-absent. Fails LOUD listing every such tile + a reproducer.
#[test]
fn every_compute_tile_emits_iverilog_runnable_fabric_verilog() {
    let emitted = emitted_compute_tiles();

    println!("── EMITTED compute-tile surface (fabric_tile → compute) ──");
    let mut holes: Vec<(String, String, String)> = Vec::new();
    let iv = iverilog_available();
    let mut compiled = 0usize;
    for (key, origin) in &emitted {
        match emit_tile_rtl(key) {
            Ok(rtl) => {
                // The float lane needs fpu.v to elaborate; the rest are self-contained.
                let extra: Vec<(&str, String)> =
                    if key.starts_with("Func::F") && key != "Func::FFloorF" {
                        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
                        vec![(
                            "fpu.v",
                            std::fs::read_to_string(root.join("benchmarks/silicon/fpu.v"))
                                .expect("read fpu.v"),
                        )]
                    } else {
                        Vec::new()
                    };
                if iv {
                    let name = key.replace([':', ' '], "_").to_lowercase();
                    match iverilog_compiles(&name, &rtl, &extra) {
                        Ok(()) => {
                            compiled += 1;
                            println!("  ✓ {key:<16} emits + iverilog-compiles  ({origin})");
                        }
                        Err(e) => {
                            holes.push((
                                key.clone(),
                                origin.clone(),
                                format!("iverilog rejected the emitted RTL:\n{e}"),
                            ));
                        }
                    }
                } else {
                    println!(
                        "  ✓ {key:<16} emits (iverilog unavailable — compile skipped)  ({origin})"
                    );
                }
            }
            Err(e) => holes.push((key.clone(), origin.clone(), e)),
        }
    }
    println!(
        "  ({} distinct compute tiles; {compiled} iverilog-compiled)",
        emitted.len()
    );

    assert!(
        holes.is_empty(),
        "\n★ $50M FABRIC EMIT HOLE ★ — {} compute tile-type(s) the model REALIZES (and `fabric_coverage` \
         counts) have NO real emit path to iverilog-runnable fabric Verilog. On tape-out silicon such a tile \
         computes nothing — the fabric analog of Word32Add's silent zero: model-proven eval ≡ VM, yet the RTL \
         emitter PANICS / emits nothing. Fix the FABRIC (add the tile's RTL emission in FuncUnit::verilog / \
         fabric_grid / a sequenced datapath emitter), NEVER weaken this guard or demote fabric_coverage.\n\
         Holes (tile  origin  why):\n{}",
        holes.len(),
        holes.iter().map(|(k, o, e)| format!("    {k:<16} {o:<44} {e}")).collect::<Vec<_>>().join("\n"),
    );
}

/// ★ GUARD B — IVERILOG-EXERCISE COVERAGE ★
/// Every distinct compute tile MUST be in at least one iverilog tb corpus a fabric lock actually runs on
/// emitted RTL ≡ VM — so a tile's RTL is not merely EMITTABLE but PROVEN to run correctly. EXERCISED is the
/// union of the existing locks' corpora (reconstructed) and this file's newly-realized-unit corpus (iverilog'd
/// here). Fails LOUD listing every emitted-but-unexercised tile + a reproducer. Skipped gracefully without
/// iverilog (the eval-level `fabric_coverage_lock` covers that case).
#[test]
fn every_compute_tile_is_exercised_by_an_iverilog_testbench() {
    if !common::require_tool("iverilog") {
        return;
    }
    let emitted = emitted_compute_tiles();
    let mut exercised = exercised_by_existing_locks();
    exercised.extend(exercised_by_new_unit_corpus());

    println!("── EXERCISED compute-tile surface (fabric locks' iverilog corpora + the new-unit corpus) ──");
    for k in &exercised {
        println!("  {k}");
    }
    println!(
        "  ({} distinct exercised on emitted RTL ≡ VM)",
        exercised.len()
    );

    let unexercised: Vec<(String, String)> = emitted
        .iter()
        .filter(|(k, _)| !exercised.contains(*k))
        .map(|(k, o)| (k.clone(), o.clone()))
        .collect();

    assert!(
        unexercised.is_empty(),
        "\n★ FABRIC IVERILOG-EXERCISE GAP ★ — {} compute tile-type(s) the model EMITS are in NO iverilog tb \
         corpus a fabric lock runs on emitted RTL ≡ VM. Each may be model-proven (eval ≡ VM) and even \
         EMITTABLE, but no differential actually RUNS it on real fabric Verilog ≡ VM — an RTL bug in its \
         datapath would go uncaught. Add a program exercising it to a fabric lock's iverilog corpus (or this \
         file's new-unit corpus). NEVER weaken this guard.\n\
         Unexercised tiles (tile  origin):\n{}",
        unexercised.len(),
        unexercised.iter().map(|(k, o)| format!("    {k:<16} {o}")).collect::<Vec<_>>().join("\n"),
    );
}

/// A sanity floor so the guards can never go vacuous by an enumerator / corpus regression that empties a set:
/// the EMITTED surface is broad by construction, and it MUST include the representative HARD tile `DivS` (a
/// sequential Func division — the first non-Arith op-class, the tile a pure combinational guard would miss),
/// so the guards keep watching exactly the layered (non-SAT) op-classes that are the fabric's risk frontier.
#[test]
fn the_guarded_sets_are_non_vacuous_and_watch_the_division_class() {
    let emitted = emitted_compute_tiles();
    assert!(
        emitted.len() >= 20,
        "EMITTED shrank to {} distinct compute tiles — the surface enumerator regressed; the guards would be \
         vacuous",
        emitted.len()
    );
    // The four originally-missing Func tiles (the fabric's Word32 class) MUST remain in the surface — if a
    // future refactor drops one from `fabric_tile`, the guard stops watching exactly the class that bit us.
    for u in [
        FuncUnit::DivS,
        FuncUnit::Pow,
        FuncUnit::MulOvfS,
        FuncUnit::ApproxEqF,
        FuncUnit::FFloorF,
    ] {
        assert!(
            emitted.contains_key(&func_key(u)),
            "the Func tile {u:?} — a layered (eval+iverilog, not SAT) op-class — is no longer in EMITTED; the \
             guard stopped watching it. Restore its `fabric_tile` classification.",
        );
    }
    // The representative HARD tile emits + iverilog-compiles (a division functional unit, not a wire).
    let rtl = emit_tile_rtl(&func_key(FuncUnit::DivS)).expect("DivS must emit real RTL");
    assert!(
        rtl.contains("module"),
        "the DivS emit must be a real Verilog module"
    );
    if iverilog_available() {
        iverilog_compiles("divs_nonvacuous", &rtl, &[])
            .expect("the DivS division tile must iverilog-compile");
    }
}
