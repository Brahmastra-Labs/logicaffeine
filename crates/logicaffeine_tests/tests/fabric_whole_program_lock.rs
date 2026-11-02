//! fabric_whole_program_lock — Phase D capstone (Lock 3): the config-driven grid = interpreter = VM.
//!
//! Phase 5 (`fabric_reconfig_lock`) proved ONE array becomes any of THREE hand-crafted SAME-SHAPE affine
//! fabrics under a config word. Phase D removes the same-shape crutch and generalizes to a DIVERSE corpus:
//! arithmetic, branch, select, and comparison functions — different sizes, different tile mixes — are each
//! if-converted to a single-output island, PADDED to a common array shape, and placed on ONE reconfigurable
//! tile grid. This lock pins the endgame three ways over that corpus:
//!
//!   1. **grid ≡ interpreter ≡ VM** — each placed island EVALUATES exactly what the VM computes for the
//!      function, over a battery of inputs (the grid IS the interpreter, at the value level).
//!   2. **reconfiguration is semantics-preserving** — our certified SAT proves, for EACH config value `k`,
//!      `config == k ⇒ the ONE array ≡ island k`, for ALL inputs. Projecting the whole corpus onto one
//!      physical fabric loses nothing.
//!   3. **the config word does real work** — the islands are genuinely distinct datapaths (a witness input
//!      separates them), so the reconfiguration is not a vacuous mux.
//!
//! Plus a corpus↔category **set-equality** guard: the corpus must cover exactly the declared tile
//! categories, so a new category cannot be claimed without a datapath that exercises it (the pit-of-success
//! ratchet — you extend corpus + categories atomically or it goes RED).

#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_sva::fabric_config::{
    block_config, certify_config_table, compile_to_config_table, eval_config_table, ConfigWord,
};
use logicaffeine_compile::compile::vm_outcome;

/// One corpus datapath: a LOGOS function, its tile CATEGORY, and an input battery for the eval differential.
struct CfgFn {
    src: &'static str,
    name: &'static str,
    category: &'static str,
    args: &'static [&'static [i64]],
}

// A DIVERSE corpus — different input arities, different tile mixes — each a single-output island that places
// on the reconfigurable array. Written in the single-final-`Return` + conditional-`Set` style so the branches
// IF-CONVERT to select tiles (branch conditions are on inputs/constants; no division/memory tiles).
const CORPUS: &[CfgFn] = &[
    // arithmetic: a pure affine map (Mul then Add).
    CfgFn {
        name: "affine",
        category: "arith",
        src: "## To affine (x: Int) -> Int:\n    Return 3 * x + 7.\n",
        args: &[&[0], &[3], &[-5], &[100], &[-100]],
    },
    // branch: clamp to [lo, hi] — two conditional Sets that if-convert to a select chain.
    CfgFn {
        name: "clamp",
        category: "branch",
        src: "## To clamp (x: Int, lo: Int, hi: Int) -> Int:\n    Let result be x.\n    If x is less than lo:\n        Set result to lo.\n    If x is greater than hi:\n        Set result to hi.\n    Return result.\n",
        args: &[&[5, 0, 10], &[-3, 0, 10], &[20, 0, 10], &[0, 0, 10], &[10, 0, 10]],
    },
    // select: the sign function — two sign selects on the input.
    CfgFn {
        name: "sign",
        category: "select",
        src: "## To sign (n: Int) -> Int:\n    Let result be 0.\n    If n is less than 0:\n        Set result to 0 - 1.\n    If n is greater than 0:\n        Set result to 1.\n    Return result.\n",
        args: &[&[-7], &[0], &[42], &[-1], &[1]],
    },
    // arith+select: absolute value — a negate tile guarded by a sign select.
    CfgFn {
        name: "absval",
        category: "arith-select",
        src: "## To absval (a: Int) -> Int:\n    Let result be a.\n    If a is less than 0:\n        Set result to 0 - a.\n    Return result.\n",
        args: &[&[-9], &[9], &[0], &[-123], &[123]],
    },
    // comparison: the minimum of two — a single compare-select on two inputs.
    CfgFn {
        name: "min2",
        category: "cmp",
        src: "## To min2 (a: Int, b: Int) -> Int:\n    Let result be b.\n    If a is less than b:\n        Set result to a.\n    Return result.\n",
        args: &[&[3, 8], &[8, 3], &[5, 5], &[-4, -9], &[0, 7]],
    },
];

/// The tile categories the corpus is required to cover (the set-equality target).
const CATEGORIES: &[&str] = &["arith", "branch", "select", "arith-select", "cmp"];

/// The datapath / config-word widths for the SAT reconfig proof (8-bit datapath, 3-bit config indexes ≤ 8).
const WIDTH: u32 = 8;
const CFG_WIDTH: u32 = 3;

/// Call `name(args)` on the VM and return its integer result — the oracle for the eval differential.
fn vm_call(src: &str, name: &str, args: &[i64]) -> i64 {
    let arg = |n: i64| {
        if n < 0 {
            format!("(0 - {})", -n)
        } else {
            n.to_string()
        }
    };
    let arglist = args.iter().map(|a| arg(*a)).collect::<Vec<_>>().join(", ");
    let prog = format!("{src}## Main\n    Show {name}({arglist}).\n");
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error calling {name}{args:?}: {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int output")
}

#[test]
fn the_reconfigured_grid_equals_the_interpreter_equals_the_vm() {
    let sources: Vec<(&str, &str)> = CORPUS.iter().map(|f| (f.src, f.name)).collect();
    let table = compile_to_config_table(&sources, WIDTH, CFG_WIDTH);
    assert_eq!(
        table.grids.len(),
        CORPUS.len(),
        "one island per corpus datapath"
    );

    // (1) grid ≡ interpreter ≡ VM — each placed island evaluates exactly what the VM computes.
    for (k, f) in CORPUS.iter().enumerate() {
        for args in f.args {
            let got = eval_config_table(&table, k, args);
            let want = vm_call(f.src, f.name, args);
            assert_eq!(
                got, want,
                "island {} ({}) eval {got} != VM {want} on {args:?}",
                k, f.name
            );
        }
    }

    // (2) reconfiguration is semantics-preserving — config==k ⇒ the ONE array ≡ island k, for ALL inputs.
    let verdicts = certify_config_table(&table);
    assert_eq!(verdicts.len(), CORPUS.len());
    assert!(
        verdicts.iter().all(|&v| v),
        "each config value must reconfigure the ONE array to be exactly island k (config==k ⇒ array ≡ island k): {verdicts:?}"
    );

    // (3) the config word does REAL work — on a witness input the islands are genuinely distinct.
    let witness: &[&[i64]] = &[&[2], &[2, 0, 1], &[2], &[2], &[2, 5]];
    let outs: Vec<i64> = (0..CORPUS.len())
        .map(|k| eval_config_table(&table, k, witness[k]))
        .collect();
    let distinct: BTreeSet<i64> = outs.iter().copied().collect();
    assert!(
        distinct.len() >= 3,
        "the reconfigured islands must be genuinely distinct datapaths, got {outs:?}"
    );

    // (4) corpus ↔ category SET-EQUALITY — the corpus covers exactly the declared categories (extend both
    // atomically or go RED), so a coverage claim can never outrun its proof.
    let covered: BTreeSet<&str> = CORPUS.iter().map(|f| f.category).collect();
    let declared: BTreeSet<&str> = CATEGORIES.iter().copied().collect();
    assert_eq!(
        covered, declared,
        "corpus categories must equal the declared category set"
    );

    // The config-word seam: block/datapath `k` is selected by `block_config(k)`.
    for k in 0..CORPUS.len() {
        assert_eq!(block_config(k), ConfigWord(k as u64));
    }

    println!(
        "Phase D: {} diverse datapaths on ONE reconfigurable array ({}x{}), config-selected, all ≡ VM + SAT-certified",
        CORPUS.len(),
        table.rows,
        table.cols
    );
}

// ── STAGE X1 (XPU): FUNCTIONAL-UNIT tiles on the reconfigurable grid. Until now the grid placed only
// Arith/Select tiles (arithmetic + branch/compare decisions). A DIVISION-bearing decision now PLACES on the
// tile array (`route_loop_body` no longer panics on `LoopCell::Func`), EVALUATES ≡ the VM, and — via
// `cell_v` → `FuncUnit::verilog` — emits combinational divider RTL: the first non-Arith/Select op-class on
// the fabric. It is proven by eval ≡ VM here + iverilog in fabric_soc_lock (SAT cannot lower division: no
// `bvsdiv`/`bvsrem`), so `grid_is_combinational` returns false for it and `certify_config_table` refuses it.
#[test]
fn func_tiles_place_and_evaluate_matching_the_vm() {
    use logicaffeine_compile::codegen_sva::fabric_grid::{
        eval_grid, grid_is_combinational, route_loop_body,
    };
    use logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source;

    // Division/mod/floor-div kernels — each if-converts to a single Func tile over two inputs.
    let kernels: &[(&str, &str)] = &[
        (
            "## To ratio (a: Int, b: Int) -> Int:\n    Return a / b.\n",
            "ratio",
        ),
        (
            "## To rem (a: Int, b: Int) -> Int:\n    Return a mod b.\n",
            "rem",
        ),
    ];
    let vecs: &[[i64; 2]] = &[
        [10, 3],
        [100, 7],
        [-20, 4],
        [7, 2],
        [-100, -7],
        [123, 11],
        [1, 1],
    ];

    for (src, name) in kernels {
        let df = decision_fabric_from_source(src, name);
        let g = route_loop_body(&df.cells, df.n_in, &[df.result]);
        // It is genuinely a Func-bearing (non-combinational) grid — the whole point of Stage X1.
        assert!(
            !grid_is_combinational(&g),
            "{name} must place a Func tile (a division/mod functional unit) on the grid"
        );
        for v in vecs {
            let got = eval_grid(&g, v);
            let want = vm_call(src, name, v);
            assert_eq!(
                got, want,
                "{name} grid Func-tile eval {got} != VM {want} on {v:?}"
            );
        }
    }

    println!("Stage X1: division/mod Func tiles place on the grid and evaluate ≡ the VM (a non-Arith/Select op-class on the fabric)");
}

// ── STAGE X2 (XPU): the BRAM STATE PLANE — stateful ops on the reconfigurable grid. Until now the grid placed
// only Arith/Select/Func tiles (pure combinational compute over routed channels). A COLLECTION-bearing program
// now PLACES `Mem` (heap read/write/alloc) tiles on the tile array (`route_loop_body` no longer panics on
// `LoopCell::HeapRead/HeapWrite/HeapAlloc`), and — threading ONE shared heap through the tiles in row-major
// (= placement = program) order, exactly as the model's `eval_cells` — EVALUATES ≡ the VM. This is the real
// unlock: the grid becomes a dataflow-WITH-MEMORY machine, not a pure combinational array. It is proven by
// `eval_grid_with_heap ≡ the model ≡ VM` (stateful mutable heap → outside QF_BV, so NOT SAT and NOT the
// stateless `eval_grid`); `grid_is_combinational` returns false for a Mem-bearing grid and `certify_config_table`
// refuses it (the BRAM+controller RTL is the X2b flash follow-on; `cell_v` panics on a Mem tile).
#[test]
fn state_plane_mem_tiles_place_and_evaluate_matching_the_vm() {
    use logicaffeine_compile::codegen_sva::fabric_grid::{
        eval_grid_with_heap, grid_is_combinational, route_loop_body,
    };
    use logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source;

    // Collection kernels — each composes heap `Alloc`/`Write`/`Read` (state-plane `Mem`) tiles on the SHARED
    // heap, exactly the same cell stream the coverage lock proves against the linear model — now placed on the
    // GRID. Each `(src, name, args)`: the kernel, its callee, and an input battery honoring its contract
    // (`i` is a 1-based index in `1..=len`).
    let kernels: &[(&str, &str, &[&[i64]])] = &[
        // NewList: `Let ys be [a,b,c]. Return item i of ys.` — HeapAlloc + 3 element HeapWrites + HeapRead.
        (
            "## To fab_new (a: Int, b: Int, c: Int, i: Int) -> Int:\n    Let ys be [a, b, c].\n    Return item i of ys.\n",
            "fab_new",
            &[&[10, 20, 30, 1], &[10, 20, 30, 2], &[10, 20, 30, 3], &[-5, 7, 100, 2], &[0, -1, -2, 3]],
        ),
        // SetIndex: `Let mutable ys be [a,b,c]. Set item i of ys to v. Return item i of ys.` — in-place HeapWrite.
        (
            "## To fab_set (a: Int, b: Int, c: Int, i: Int, v: Int) -> Int:\n    Let mutable ys be [a, b, c].\n    Set item i of ys to v.\n    Return item i of ys.\n",
            "fab_set",
            &[&[10, 20, 30, 2, 99], &[1, 2, 3, 1, -4], &[7, 8, 9, 3, 0], &[-1, -2, -3, 2, 123]],
        ),
        // A whole-list reduction: `Return item 1 + item 2 + item 3 of ys` — 3 HeapReads folded by Arith adds,
        // exercising multiple state-plane reads feeding pure-compute tiles on the same grid.
        (
            "## To fab_sum (a: Int, b: Int, c: Int) -> Int:\n    Let ys be [a, b, c].\n    Return item 1 of ys + item 2 of ys + item 3 of ys.\n",
            "fab_sum",
            &[&[10, 20, 30], &[-5, 7, 100], &[0, 0, 0], &[1, -1, 1], &[123, 456, -789]],
        ),
    ];

    for (src, name, battery) in kernels {
        let df = decision_fabric_from_source(src, name);
        let g = route_loop_body(&df.cells, df.n_in, &[df.result]);
        // It is genuinely a state-plane (Mem-bearing, non-combinational) grid — the whole point of Stage X2.
        assert!(
            !grid_is_combinational(&g),
            "{name} must place a Mem (state-plane heap) tile on the grid"
        );
        for args in *battery {
            let got = eval_grid_with_heap(&g, args);
            let want = vm_call(src, name, args);
            assert_eq!(
                got, want,
                "{name} state-plane grid eval {got} != VM {want} on {args:?}"
            );
        }
    }

    println!("Stage X2: collections place Mem (state-plane heap) tiles on the grid and evaluate ≡ the VM via eval_grid_with_heap (a stateful op-class on the fabric — the dataflow-with-memory unlock)");
}

/// The same collection kernels the X2 grid-eval lock uses — reused by the X2b iverilog lock so the
/// synthesizable RTL is proven against EXACTLY the cell stream the grid places. `(src, callee, battery)`.
const STATE_KERNELS: &[(&str, &str, &[&[i64]])] = &[
    (
        "## To fab_new (a: Int, b: Int, c: Int, i: Int) -> Int:\n    Let ys be [a, b, c].\n    Return item i of ys.\n",
        "fab_new",
        &[&[10, 20, 30, 1], &[10, 20, 30, 2], &[10, 20, 30, 3], &[-5, 7, 100, 2], &[0, -1, -2, 3]],
    ),
    (
        "## To fab_set (a: Int, b: Int, c: Int, i: Int, v: Int) -> Int:\n    Let mutable ys be [a, b, c].\n    Set item i of ys to v.\n    Return item i of ys.\n",
        "fab_set",
        &[&[10, 20, 30, 2, 99], &[1, 2, 3, 1, -4], &[7, 8, 9, 3, 0], &[-1, -2, -3, 2, 123]],
    ),
    (
        "## To fab_sum (a: Int, b: Int, c: Int) -> Int:\n    Let ys be [a, b, c].\n    Return item 1 of ys + item 2 of ys + item 3 of ys.\n",
        "fab_sum",
        &[&[10, 20, 30], &[-5, 7, 100], &[0, 0, 0], &[1, -1, 1], &[123, 456, -789]],
    ),
];

// ── STAGE X2b (XPU): the state plane as SYNTHESIZABLE RTL, proven ≡ VM under iverilog. The X2 lock above
// proves the heap cells PLACE + EVALUATE on the grid (the model). This lock proves the SEQUENCED state-plane
// controller — one shared LUTRAM heap + a bump allocator, a hardwired micro-sequencer over the SAME cells —
// computes bit-identically to the running VM under a real gate-level simulator, over a battery of vectors per
// kernel. (Mutable heap is outside QF_BV, so this is the iverilog half of the layered proof — there is no SAT
// half for the state plane, exactly as the division Func tile is iverilog-proven, not SAT.) Skipped gracefully
// where iverilog is absent; the X2 grid-eval lock covers that case.
#[test]
fn state_plane_rtl_matches_vm_under_iverilog() {
    use logicaffeine_compile::codegen_sva::fabric_rtl::{
        decision_fabric_from_source, decision_fabric_has_state, emit_state_plane_testbench,
        emit_state_plane_verilog, STATE_PLANE_HEAP_WORDS,
    };

    if !common::require_tool("iverilog") {
        return;
    }

    let dir = std::env::temp_dir().join("logos_state_plane_iverilog");
    std::fs::create_dir_all(&dir).expect("create temp dir");

    for (src, name, battery) in STATE_KERNELS {
        let df = decision_fabric_from_source(src, name);
        assert!(
            decision_fabric_has_state(&df),
            "{name} must carry a state-plane (heap) cell — else it is not an X2b kernel"
        );
        let n_in = df.n_in;
        let m = df.cells.len();
        let rtl = emit_state_plane_verilog(name, &df, STATE_PLANE_HEAP_WORDS);
        // Generous cycle bound: one clock per cell + the rst/settle margin.
        let tb = emit_state_plane_testbench(name, n_in, battery.len(), m + 8);

        // The VM oracle + the driven inputs, as 64-bit hex (two's-complement, matching the RTL's wrapping).
        let mut inhex = String::new();
        let mut outhex = String::new();
        for args in *battery {
            assert_eq!(
                args.len(),
                n_in,
                "{name}: vector arity {} != param count {n_in}",
                args.len()
            );
            for a in *args {
                inhex.push_str(&format!("{:016x}\n", *a as u64));
            }
            let want = vm_call(src, name, args);
            outhex.push_str(&format!("{:016x}\n", want as u64));
        }

        let srcv = dir.join(format!("state_{name}.v"));
        let tbv = dir.join(format!("tb_state_{name}.v"));
        let sim = dir.join(format!("sim_{name}.vvp"));
        std::fs::write(&srcv, &rtl).expect("write state rtl");
        std::fs::write(&tbv, &tb).expect("write state tb");
        std::fs::write(dir.join(format!("state_{name}_in.hex")), &inhex).expect("write in hex");
        std::fs::write(dir.join(format!("state_{name}_out.hex")), &outhex).expect("write out hex");

        let build = std::process::Command::new("iverilog")
            .arg("-g2012")
            .arg("-o")
            .arg(&sim)
            .arg(&srcv)
            .arg(&tbv)
            .current_dir(&dir)
            .output()
            .expect("run iverilog");
        assert!(
            build.status.success(),
            "iverilog failed to compile state_{name}: {}\n--- RTL ---\n{rtl}",
            String::from_utf8_lossy(&build.stderr)
        );

        let run = std::process::Command::new("vvp")
            .arg(&sim)
            .current_dir(&dir)
            .output()
            .expect("run vvp");
        let stdout = String::from_utf8_lossy(&run.stdout);
        assert!(
            stdout.contains("PASS:"),
            "state_{name} RTL != VM under iverilog: {stdout}\n--- RTL ---\n{rtl}"
        );
    }

    println!(
        "Stage X2b: {} collection kernels lowered to a SEQUENCED state-plane controller (shared LUTRAM heap + bump allocator) — all ≡ VM under iverilog (the synthesizable state plane)",
        STATE_KERNELS.len()
    );
}

/// The FLOAT-LANE kernels — IEEE-754 `f64` arithmetic (`FAddF`/`FSubF`/`FMulF`/`FDivF`/`FSqrtF`) bridged to an
/// int result via `floor(...)` (`FFloorF`), so the output is directly VM-comparable (a decision fabric can't
/// `Show` a float, and a computed float PREDICATE won't route as a 2-channel mux). Shared by the X3 grid-eval
/// lock and the X3b iverilog lock. `(src, callee)`.
const FLOAT_KERNELS: &[(&str, &str)] = &[
    ("## To fmul () -> Int:\n    Let a be 3.5.\n    Let b be 2.0.\n    Return floor(a * b).\n", "fmul"),
    ("## To fdiv () -> Int:\n    Let a be 7.0.\n    Let b be 2.0.\n    Return floor(a / b).\n", "fdiv"),
    ("## To fadd () -> Int:\n    Let a be 3.5.\n    Let b be 2.0.\n    Return floor(a + b).\n", "fadd"),
    ("## To fsub () -> Int:\n    Let a be 7.0.\n    Let b be 2.5.\n    Return floor(a - b).\n", "fsub"),
    ("## To fsqrt () -> Int:\n    Let x be 20.0.\n    Return floor(sqrt(x)).\n", "fsqrt"),
    // The Pythagorean chain: floor(sqrt(3² + 4²)) = 5 — FMulF ×2 + FAddF + FSqrtF + FFloorF in one datapath.
    ("## To fpyth () -> Int:\n    Let a be 3.0.\n    Let b be 4.0.\n    Return floor(sqrt(a * a + b * b)).\n", "fpyth"),
];

// ── STAGE X3 (XPU): the FLOAT LANE on the reconfigurable grid. IEEE-754 `f64` operates on the bit-pattern each
// operand carries (`f64::from_bits`), so a float VALUE rides the SAME 64-bit integer channel as an int — no
// separate typed channel is needed for evaluation. Thus a float functional unit (`FuncUnit::FAddF/FSubF/FMulF/
// FDivF/FSqrtF/FFloorF`) places on the grid via EXACTLY the Stage-X1 `Func` machinery; the model already tracks
// which registers hold floats (`FLOAT_REGS`) and routes `+ - * /` through the float units, and `floor(...)`
// bridges the f64 result back to an `Int` the VM can `Show`. This lock proves those float-Func grids evaluate
// bit-identically to the running VM. (Float is outside QF_BV — proven by eval ≡ VM + iverilog in X3b, NOT SAT,
// exactly like the division Func tile; `grid_is_combinational` is false, `certify_config_table` refuses them.)
#[test]
fn float_lane_tiles_place_and_evaluate_matching_the_vm() {
    use logicaffeine_compile::codegen_sva::fabric_grid::{
        eval_grid, grid_is_combinational, route_loop_body,
    };
    use logicaffeine_compile::codegen_sva::fabric_rtl::decision_fabric_from_source;

    for (src, name) in FLOAT_KERNELS {
        let df = decision_fabric_from_source(src, name);
        let g = route_loop_body(&df.cells, df.n_in, &[df.result]);
        // A genuine float-Func-bearing (non-combinational) grid — the whole point of Stage X3.
        assert!(
            !grid_is_combinational(&g),
            "{name} must place a float functional-unit tile (FAddF/FMulF/FDivF/FSqrtF/FFloorF) on the grid"
        );
        let got = eval_grid(&g, &[]);
        let want = vm_call(src, name, &[]);
        assert_eq!(got, want, "{name} float-lane grid eval {got} != VM {want}");
    }

    println!("Stage X3: IEEE-754 float functional-unit tiles (add/sub/mul/div/sqrt→floor) place on the grid and evaluate ≡ the VM (the float lane — f64 bit-patterns ride the int channels)");
}

// ── STAGE X4 (XPU): whole-program CONTROL FLOW on the reconfigurable grid. The Phase-D config grid placed a
// SINGLE-output decision island per config word. This stage walks a whole BRANCHY function (multiple basic
// blocks, early returns) on ONE array: `run_program_cfg_on_grid` places each block's straight-line body on a
// fresh tile grid (the block a reconfiguration), reads its register writes + terminator operand off the placed
// array via `eval_grid_all`, and the block-sequencing controller walks the Jump/Branch/Return graph. So the
// whole program — not just one decision — runs as spatial dataflow, proven ≡ the CFG model
// (`eval_program_cfg_fabric`) ≡ the VM. (Counted/while/nested LOOPS on the grid are the separately-proven loop
// fabric — LoopFabric/route_loop_body; this stage is the acyclic-branch composition, the missing capstone.)
#[test]
fn branchy_whole_program_runs_on_the_grid_matching_the_vm() {
    use logicaffeine_compile::codegen_sva::fabric_grid::run_program_cfg_on_grid;
    use logicaffeine_compile::codegen_sva::fabric_rtl::{
        eval_program_cfg_fabric, program_cfg_fabric,
    };

    // A three-way clamp (four blocks, two Branches, three return paths) and a three-way sign (nested branches).
    const CLAMP: &str = "## To clamp (x: Int, lo: Int, hi: Int) -> Int:\n    If x is less than lo:\n        Return lo.\n    If x is greater than hi:\n        Return hi.\n    Return x.\n";
    const SIGN: &str = "## To sign (n: Int) -> Int:\n    If n is less than 0:\n        Return 0 - 1.\n    If n is equal to 0:\n        Return 0.\n    Return 1.\n";

    let clamp = program_cfg_fabric(CLAMP, "clamp");
    let sign = program_cfg_fabric(SIGN, "sign");
    assert!(
        clamp.blocks.len() >= 3,
        "clamp must be a genuine MULTI-block CFG, got {}",
        clamp.blocks.len()
    );
    assert!(
        sign.blocks.len() >= 3,
        "sign must be a genuine MULTI-block CFG, got {}",
        sign.blocks.len()
    );

    let clamp_vecs: &[[i64; 3]] = &[
        [5, 0, 10],
        [-3, 0, 10],
        [20, 0, 10],
        [0, 0, 10],
        [10, 0, 10],
        [7, -5, 5],
        [-100, -5, 5],
    ];
    for v in clamp_vecs {
        let grid = run_program_cfg_on_grid(&clamp, v);
        let model = eval_program_cfg_fabric(&clamp, v);
        let vm = vm_call(CLAMP, "clamp", v);
        assert_eq!(
            grid, model,
            "clamp: grid {grid} != CFG model {model} on {v:?}"
        );
        assert_eq!(grid, vm, "clamp: grid {grid} != VM {vm} on {v:?}");
    }
    for n in [-7_i64, -1, 0, 1, 42, 100, -100] {
        let grid = run_program_cfg_on_grid(&sign, &[n]);
        let model = eval_program_cfg_fabric(&sign, &[n]);
        let vm = vm_call(SIGN, "sign", &[n]);
        assert_eq!(grid, model, "sign: grid {grid} != CFG model {model} on {n}");
        assert_eq!(grid, vm, "sign: grid {grid} != VM {vm} on {n}");
    }

    println!("Stage X4: a branchy multi-block whole program runs on ONE reconfigurable grid (each block a reconfiguration, the controller walks the Jump/Branch/Return graph) ≡ the CFG model ≡ the VM");
}

/// The emitted RTL may not claim a proof it does not have.
///
/// `emit_config_grid_verilog` used to write "Certified by our SAT (certify_config_table): cfg==k ⇒
/// the array ≡ fabric k" into EVERY generated `config_*` header, unconditionally. But
/// `certify_config_table` *asserts out* on a Func-bearing table — a division or float tile has no
/// bitblast lowering — and the shipped `FABRIC_SOURCES` contains `ratio` (`a / b`), a Func tile. So
/// the one `config_fabric` we actually hardened to GDSII carried a SAT claim that had never been
/// run on it. Its real evidence is eval ≡ VM plus the iverilog differential plus 14/14 vectors on
/// both boards — strong, but a different claim.
///
/// This locks the header to the same predicate `certify_config_table` gates on, in both directions,
/// so the sentence can never again drift from the proof that backs it.
#[test]
fn the_emitted_config_header_claims_only_the_proof_it_actually_has() {
    use logicaffeine_compile::codegen_sva::fabric_grid::grid_is_combinational;
    use logicaffeine_compile::codegen_sva::fabric_soc::{emit_config_grid, table};

    let t = table();
    let sat_certifiable = t.grids.iter().all(grid_is_combinational);
    let rtl = emit_config_grid();
    let header: String = rtl
        .lines()
        .take_while(|l| l.trim_start().starts_with("//"))
        .collect();
    let claims_sat = header.contains("certify_config_table");

    assert_eq!(
        claims_sat, sat_certifiable,
        "config_fabric's header claims SAT certification = {claims_sat}, but the table is \
         SAT-certifiable = {sat_certifiable}. `certify_config_table` asserts out on a Func \
         (division/float) tile, so a Func-bearing table is proven by eval ≡ VM + iverilog, NOT by \
         the SAT reconfig cert. The emitted provenance must state whichever is true.\n\
         header was: {header}"
    );

    // The shipped table contains `ratio` (a / b) — a Func tile — so today the honest answer is the
    // differential one. If a future edit drops the divider this flips, and the assert above keeps
    // the header honest either way; this second check makes the CURRENT state explicit rather than
    // letting a vacuously-true equality pass unnoticed.
    assert!(
        !sat_certifiable,
        "FABRIC_SOURCES no longer has a Func tile — the shipped table is now SAT-certifiable. That \
         is an improvement, not a failure: update this expectation and make sure the header says so."
    );
    assert!(
        header.contains("eval") && header.contains("iverilog"),
        "a non-SAT-certifiable table's header must name the evidence it DOES have (eval ≡ VM + \
         iverilog differential). header was: {header}"
    );
}
