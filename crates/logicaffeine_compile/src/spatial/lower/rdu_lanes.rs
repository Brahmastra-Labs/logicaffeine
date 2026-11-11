//! The RDU front door's three lanes — how ANY LOGOS program reaches the SambaNova descriptor.
//!
//! LOGOS already lowers to RISC-V, C, WASM, CUDA, Tensix, the spatial fabric (the 117-op ISA) and
//! Plasticine. `largo emit rdu` lowers a program onto the SN40L / SN50 descriptor through ONE of three
//! lanes, chosen by the program's SHAPE ([`lane_of`]) and printed by name:
//!
//! * **tensor** — the program declares a tensor or the tensor recognizer reads a matmul / elementwise /
//!   swiglu / fused-row loop nest: the spatial IR front end (`pipeline::compile_to_spatial`), placed and
//!   scheduled on the mesh, the certified neutral placement (`lower::rdu`).
//! * **loop** ([`lower_loop_lane`]) — the first function is a counted range loop with scalar carried state:
//!   the Plasticine lane on the RDU mesh descriptor (`PlasticineTarget::rdu`) — metapipeline II under the
//!   Tungsten-calibrated PCU model, checkerboard placement + static routing, the SAT certificate of the
//!   routed body, the VM anchor, Spatial source for the public PIR chain.
//! * **whole-program** ([`lower_whole_program_lane`]) — everything else the fabric's decision datapath
//!   covers: the first function (if/else, collections on the state plane, structs, enums, helper calls
//!   inlined) as a `Scalar` stage applied lane-wise over an input vector (`y[i] = f(x_0[i], …)`), or a
//!   `## Main` of `Show`s as the tape-select stage (lane i emits Show i) — verified, certified, and handed
//!   to `RduSpatialBackend` as the neutral placement.
//!
//! What the dataflow lanes REFUSE, by name (never a wrong stage): recursion (the decision front end would
//! otherwise take the first-return path only — a spatial call stack is what it needs; the whole-program
//! call-stack engine runs it ≡VM meanwhile), loops outside the loop lane's single-stage counted shape, early
//! returns, a `Show` of Text on the tape, and every op on the fabric frontier (`fabric_tile::frontier_reason`).

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::super::ir::*;
use super::super::passes::{SpatialReject, LV_ITEM_X, LV_ITEM_Y};
use super::super::pipeline::run_passes;
use super::super::rdu_chip::RduChip;
use super::super::target::SpatialTarget;
use crate::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, whole_program_fabric_from_source, CmpKind, DecisionFabric,
    LoopCell, Src,
};
use crate::codegen_sva::fabric_target::Precision;
use crate::codegen_sva::fabric_tile::{fabric_tile, frontier_reason};
use crate::codegen_sva::plasticine::PlasticineTarget;
use crate::codegen_sva::plasticine_report::{
    counted_loop_fabric, lower_counted_loop, PlasticineArtifact, CERTIFICATE_WIDTH,
};
use crate::tensor_recognize::{
    recognize_elementwise, recognize_fused_row, recognize_matmul, recognize_swiglu,
};
use crate::vm::{CompiledProgram, Compiler, Op};
use logicaffeine_language::ast::stmt::Stmt;

/// Which lane a program takes — a function of its shape, printed by `largo emit rdu` as `lane: <name>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RduLane {
    Tensor,
    Loop,
    WholeProgram,
}

impl RduLane {
    pub const ALL: [RduLane; 3] = [RduLane::Tensor, RduLane::Loop, RduLane::WholeProgram];

    /// The printed spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            RduLane::Tensor => "tensor",
            RduLane::Loop => "loop",
            RduLane::WholeProgram => "whole-program",
        }
    }

    pub fn parse(s: &str) -> Option<RduLane> {
        RduLane::ALL.into_iter().find(|l| l.as_str() == s.trim())
    }
}

/// The lanes an input vector of the whole-program lane carries: four PCUs' worth of the unit model's
/// sixteen SIMD lanes — enough to exercise every path of a decision body at the VM anchor's sample inputs.
pub const WHOLE_PROGRAM_LANES: usize = 64;

/// The VM anchor's sample inputs for a function of `arity` parameters: small, distinct, positive — the
/// same samples `compile_to_plasticine` anchors on.
pub fn anchor_params(arity: usize) -> Vec<i64> {
    (0..arity).map(|i| 8 + i as i64).collect()
}

/// The function names `source` ITSELF declares (`## To [native] [private] <name>`), read off the text.
///
/// The parsed statement list is NOT the program's own: `loader::apply_prelude` demand-imports every stdlib
/// module the program references and splices its definitions AHEAD of the user's, so the first parsed
/// `FunctionDef` of `Let c be tcpConnect(host, port).` is tcp.lg's `tcpConnect`, not the function the
/// program is about. The lane's unit is the program's own first function; this set is what says which.
fn own_function_names(source: &str) -> BTreeSet<String> {
    source
        .lines()
        .filter_map(|line| {
            let mut rest = line.trim().strip_prefix("## To ")?;
            loop {
                match rest
                    .strip_prefix("native ")
                    .or_else(|| rest.strip_prefix("private "))
                {
                    Some(r) => rest = r,
                    None => break,
                }
            }
            let name = rest.split(|c: char| c == '(' || c.is_whitespace()).next()?;
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

/// The first function `source` itself declares — the unit the loop and whole-program lanes lower — as
/// `(name, arity)`; `None` for a `## Main`-only program or one that does not parse. A prelude module
/// spliced in by demand (`own_function_names`) is never the unit.
pub fn first_function(source: &str) -> Option<(String, usize)> {
    let own = own_function_names(source);
    if own.is_empty() {
        return None;
    }
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.ok()?;
        stmts.iter().find_map(|s| match s {
            Stmt::FunctionDef { name, params, .. } => {
                let name = interner.resolve(*name).to_string();
                own.contains(&name).then_some((name, params.len()))
            }
            _ => None,
        })
    })
}

/// Whether `source` declares a tensor — `Let X be a R by C tensor of …`, the tensor sugar's own opening
/// (the parser's `try_parse_tensor_new`), read off the text so the lane is known before any op is recognized.
pub fn has_tensor_sugar(source: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim();
        if line.starts_with('#') || !line.starts_with("Let ") || !line.contains(" be a ") {
            return false;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        words.windows(4).any(|w| {
            w[0] == "by"
                && w[1].parse::<u64>().is_ok()
                && w[2] == "tensor"
                && w[3].starts_with("of")
        })
    })
}

/// Whether the tensor recognizer reads a matmul / elementwise / swiglu / fused-row loop nest in `source`
/// (the plain-loop tensor programs the spatial front end lowers without sugar).
pub fn recognizes_a_tensor_op(source: &str) -> bool {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let Ok((stmts, _, _)) = parsed else {
            return false;
        };
        recognize_matmul(stmts, interner).is_ok()
            || recognize_elementwise(stmts, interner).is_ok()
            || recognize_swiglu(stmts, interner).is_ok()
            || recognize_fused_row(stmts, interner).is_ok()
    })
}

/// The lane `source` takes: tensor when it declares a tensor or a tensor op is recognized; loop when its
/// first function is the counted range loop the metapipeline model takes; whole-program otherwise (a
/// source that does not parse falls there too — its lowering reports the parse error typed).
pub fn lane_of(source: &str) -> RduLane {
    if has_tensor_sugar(source) || recognizes_a_tensor_op(source) {
        return RduLane::Tensor;
    }
    if let Some((name, _)) = first_function(source) {
        if counted_loop_fabric(source, &name).is_ok() {
            return RduLane::Loop;
        }
    }
    RduLane::WholeProgram
}

// ── The loop lane ────────────────────────────────────────────────────────────────────────────────

/// What the loop lane hands `largo emit rdu`: the Plasticine artifact of the first function on the RDU
/// mesh descriptor, with its pieces rendered — the placement, the modeled report row, the Spatial source,
/// the certificate summary bound to the certificate by its digest, and the text an engineer reads first.
#[derive(Debug)]
pub struct LoopLaneArtifact {
    /// The checkerboard placement (pretty JSON).
    pub placement_json: String,
    /// The modeled report row: II, latencies, utilization, the VM anchor, the certificate row (pretty JSON).
    pub report_json: String,
    /// Spatial source for the counted loop (a standalone `SpatialApp`).
    pub spatial_source: String,
    /// The SAT certificate summary — kind, what it proves, steps, variables, width, rechecked, digest — or
    /// the reason it was omitted.
    pub certificate_json: String,
    /// One screen of text: the lane, the placement, the II, the anchor, the certificate.
    pub pretty: String,
    /// The Plasticine artifact itself (the loop fabric, the metapipeline, the placement object, the
    /// certificate object, the regression-harness Spatial app).
    pub plasticine: PlasticineArtifact,
}

/// Lower the first function of `source` — a counted range loop — onto `chip`'s mesh descriptor through the
/// Plasticine lane, anchored to the VM at [`anchor_params`]. A program whose first function is not that
/// loop is refused with the lane's own reason.
pub fn lower_loop_lane(source: &str, chip: RduChip) -> Result<LoopLaneArtifact, String> {
    let (fn_name, arity) = first_function(source).ok_or_else(|| {
        "no region function found (the loop lane's unit is a counted-loop function)".to_string()
    })?;
    let params = anchor_params(arity);
    let target = PlasticineTarget::rdu(chip);
    let art = lower_counted_loop(source, &fn_name, &params, &target)?;
    let certificate = match (
        &art.certificate,
        &art.row.certificate,
        &art.row.certificate_omitted,
    ) {
        (Some(pc), Some(row), _) => json!({
            "kind": "sat-placement",
            "proves": row.proves,
            "steps": row.steps,
            "vars": row.vars,
            "width": CERTIFICATE_WIDTH,
            "rechecked": row.rechecked,
            "formula": row.proof.as_ref().map(|p| p.formula.clone()),
            "digest": format!("{:016x}", pc.digest()),
        }),
        (_, _, Some(why)) => json!({ "kind": "omitted", "reason": why }),
        _ => json!({ "kind": "omitted", "reason": "no certificate was produced" }),
    };
    let certificate_json = format!(
        "{}\n",
        serde_json::to_string_pretty(&certificate).expect("a certificate summary serializes")
    );
    let row = &art.row;
    let mut pretty = String::new();
    pretty.push_str("lane: loop\n");
    pretty.push_str(&format!(
        "program: `{fn_name}` ({arity} parameter{}) on {}\n",
        if arity == 1 { "" } else { "s" },
        target.name
    ));
    pretty.push_str(&format!(
        "placement: pcu {}/{} pmu {}/{} ({}x{} checkerboard) router {} routed statically {}\n",
        row.pcu_used,
        row.pcu_total,
        row.pmu_used,
        row.pmu_total,
        target.pcu_rows,
        target.pcu_cols,
        row.router,
        row.routed_statically
    ));
    for s in &row.stages {
        pretty.push_str(&format!(
            "stage `{}`: II {} (resource {} recurrence {} memory {}) latency {} cells {} pcus {}\n",
            s.name, s.ii, s.resource_ii, s.recurrence_ii, s.memory_ii, s.latency, s.cells, s.pcus
        ));
    }
    pretty.push_str(&format!(
        "metapipeline: II {} iterations {} fill {} compute {} modeled cycles {} (provenance {})\n",
        row.pipeline_ii,
        row.iterations,
        row.fill_cycles,
        row.compute_cycles,
        row.modeled_cycles,
        row.provenance
    ));
    if let Some(a) = &row.vm_anchor {
        pretty.push_str(&format!(
            "vm anchor: {fn_name}({}) = {} (VM) = {} (fabric) = {} (pipelined at II {}, hazards {})\n",
            a.params.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "),
            a.vm,
            a.fabric,
            a.pipelined_at_ii,
            row.pipeline_ii,
            a.hazards_at_ii
        ));
    }
    match (&art.certificate, &row.certificate) {
        (Some(pc), Some(c)) => pretty.push_str(&format!(
            "certificate: SAT (PR/RUP) {} steps {} vars at {} bits, rechecked {}, digest {:016x}\n",
            c.steps,
            c.vars,
            CERTIFICATE_WIDTH,
            c.rechecked,
            pc.digest()
        )),
        _ => pretty.push_str(&format!(
            "certificate: omitted — {}\n",
            row.certificate_omitted
                .as_deref()
                .unwrap_or("no certificate")
        )),
    }
    pretty.push_str(&format!("spatial: Logos_{fn_name} (SpatialApp) + the Logos_{fn_name} regression app for the public PIR chain\n"));
    Ok(LoopLaneArtifact {
        placement_json: art.placement_json(),
        report_json: art.row_json(),
        spatial_source: art.spatial.clone(),
        certificate_json,
        pretty,
        plasticine: art,
    })
}

// ── The whole-program lane ──────────────────────────────────────────────────────────────────────

/// Run `f`, turning a panic into an `Err` carrying its message, with the default panic printer silenced —
/// the fabric front ends panic on shapes outside their family (their honest boundary); here that becomes a
/// typed refusal.
fn quietly<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(prev);
    r.map_err(|p| {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "the lowering panicked".to_string())
    })
}

/// The unit the whole-program lane lowers, with the bytecode facts the gates need.
enum Unit {
    /// The first function: its name, arity, and the ops of its body plus every function it reaches.
    Function { name: String, arity: usize },
    /// A `## Main` of `Show`s.
    Main,
}

struct Scan {
    unit: Unit,
    /// A back-edge (a loop) in the unit or a function it reaches, with where.
    loop_in: Option<String>,
    /// A cycle in the call graph reachable from the unit, with the function that closes it.
    recursion: Option<String>,
    /// A branch at the top level of `Main` (a conditional `Show` has no tape form).
    branch_in_main: bool,
    /// The first op with no fabric tile, with its frontier reason.
    frontier: Option<&'static str>,
}

/// The range of `f`'s ops: its entry up to the next function's entry (or the end of the code).
fn function_range(program: &CompiledProgram, entry: usize) -> std::ops::Range<usize> {
    let end = program
        .functions
        .iter()
        .map(|g| g.entry_pc)
        .filter(|&e| e > entry)
        .min()
        .unwrap_or(program.code.len());
    entry..end
}

/// The ops of Main: the code before the first function (or the whole code), up to its `Halt`.
fn main_range(program: &CompiledProgram) -> std::ops::Range<usize> {
    let first_fn = program
        .functions
        .iter()
        .map(|f| f.entry_pc)
        .min()
        .unwrap_or(program.code.len());
    let halt = program.code[..first_fn]
        .iter()
        .position(|op| matches!(op, Op::Halt))
        .map(|i| i + 1)
        .unwrap_or(first_fn);
    0..halt
}

fn callees(program: &CompiledProgram, range: std::ops::Range<usize>) -> Vec<usize> {
    program.code[range]
        .iter()
        .filter_map(|op| match op {
            Op::Call { func, .. }
            | Op::Spawn { func, .. }
            | Op::SpawnHandle { func, .. }
            | Op::MakeClosure { func, .. } => Some(*func as usize),
            _ => None,
        })
        .collect()
}

fn has_back_edge(program: &CompiledProgram, range: std::ops::Range<usize>) -> bool {
    let start = range.start;
    program.code[range]
        .iter()
        .enumerate()
        .any(|(i, op)| match op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. } => *target <= start + i,
            _ => false,
        })
}

/// Compile `source` and read the facts the lane's gates decide on.
fn scan(source: &str) -> Result<Scan, SpatialReject> {
    let own = own_function_names(source);
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, types, _policies) = parsed.map_err(SpatialReject::Parse)?;
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .map_err(SpatialReject::Parse)?;
        let first = stmts.iter().find_map(|s| match s {
            Stmt::FunctionDef { name, params, .. } => {
                let name = interner.resolve(*name).to_string();
                own.contains(&name).then_some((name, params.len()))
            }
            _ => None,
        });
        let name_of = |idx: usize| {
            program
                .functions
                .get(idx)
                .map(|f| interner.resolve(f.name).to_string())
                .unwrap_or_else(|| format!("#{idx}"))
        };
        // The unit's own range, and the roots of its call graph.
        let (unit, own, roots): (Unit, std::ops::Range<usize>, Vec<usize>) = match &first {
            Some((name, arity)) => {
                let idx = program
                    .functions
                    .iter()
                    .position(|f| interner.resolve(f.name) == name)
                    .expect("the first function compiles");
                let range = function_range(&program, program.functions[idx].entry_pc);
                (
                    Unit::Function {
                        name: name.clone(),
                        arity: *arity,
                    },
                    range,
                    vec![idx],
                )
            }
            None => {
                let range = main_range(&program);
                let roots = callees(&program, range.clone());
                (Unit::Main, range, roots)
            }
        };
        let branch_in_main = matches!(unit, Unit::Main)
            && program.code[own.clone()].iter().any(|op| {
                matches!(
                    op,
                    Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. }
                )
            });
        // Loops: in the unit itself, then in every function it reaches.
        let mut loop_in =
            if matches!(unit, Unit::Function { .. }) && has_back_edge(&program, own.clone()) {
                match &unit {
                    Unit::Function { name, .. } => Some(name.clone()),
                    Unit::Main => None,
                }
            } else {
                None
            };
        // Recursion: a cycle in the call graph reachable from the roots (DFS with the active path).
        let mut recursion = None;
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        let mut on_path: Vec<usize> = Vec::new();
        fn dfs(
            program: &CompiledProgram,
            f: usize,
            visited: &mut BTreeSet<usize>,
            on_path: &mut Vec<usize>,
            recursion: &mut Option<String>,
            loop_in: &mut Option<String>,
            name_of: &dyn Fn(usize) -> String,
        ) {
            if on_path.contains(&f) {
                if recursion.is_none() {
                    *recursion = Some(name_of(f));
                }
                return;
            }
            if !visited.insert(f) {
                return;
            }
            let Some(func) = program.functions.get(f) else {
                return;
            };
            let range = function_range(program, func.entry_pc);
            if loop_in.is_none() && has_back_edge(program, range.clone()) {
                *loop_in = Some(name_of(f));
            }
            on_path.push(f);
            for c in callees(program, range) {
                dfs(program, c, visited, on_path, recursion, loop_in, name_of);
            }
            on_path.pop();
        }
        for r in roots {
            dfs(
                &program,
                r,
                &mut visited,
                &mut on_path,
                &mut recursion,
                &mut loop_in,
                &name_of,
            );
        }
        // The frontier: any op of the unit or a reached function with no fabric tile.
        let mut ranges = vec![own];
        for f in &visited {
            ranges.push(function_range(&program, program.functions[*f].entry_pc));
        }
        let frontier = ranges
            .into_iter()
            .flat_map(|r| program.code[r].iter())
            .find_map(|op| {
                if fabric_tile(op).is_none() {
                    Some(
                        frontier_reason(op)
                            .unwrap_or("an op with no fabric tile and no stated frontier reason"),
                    )
                } else {
                    None
                }
            });
        Ok(Scan {
            unit,
            loop_in,
            recursion,
            branch_in_main,
            frontier,
        })
    })
}

/// A decision fabric as one `Scalar` stage over `n`-lane input vectors — one Input tensor, load and FIFO
/// per parameter, the body, one Output tensor (the general form of `from_scalar_body`'s single input).
fn scalar_program(
    df: &DecisionFabric,
    n: usize,
    target: &SpatialTarget,
    name: String,
) -> Result<SpatialProgram, SpatialReject> {
    if target.scalar_target().is_none() {
        return Err(SpatialReject::MissingCompute {
            needed: "scalar datapath",
            target: target.name,
        });
    }
    let k = df.n_in;
    // The stage's result is a cell value: an identity function or a constant tape lane drives its result
    // straight from an input or a constant, which the per-PE certificate has no output cell to bind to.
    let mut cells = df.cells.clone();
    let result = match df.result {
        Src::Val(_) => df.result,
        other => {
            cells.push(LoopCell::Arith {
                op: crate::codegen_sva::fabric_rtl::AluOp::Add,
                a: other,
                b: Src::Const(0),
            });
            Src::Val(cells.len() - 1)
        }
    };
    let local = Handshake::Fifo;
    let tile = target.tile;
    let grid = |rows: usize, cols: usize| TileGrid {
        rows_t: rows.div_ceil(tile.rows),
        cols_t: cols.div_ceil(tile.cols),
    };
    let decl = |id: usize, name: String, role: TensorRole, init: TensorInit| TensorDecl {
        id: TensorId::new(id),
        name,
        role,
        elems: ElemShape { rows: 1, cols: n },
        tiles: grid(1, n),
        dtype: Precision::Int64,
        layout: Layout::RowMajorTiles,
        space: TensorSpace::Dram,
        init,
        range: None,
        chip: None,
    };
    let mut tensors: Vec<TensorDecl> = (0..k)
        .map(|i| decl(i, format!("x{i}"), TensorRole::Input, TensorInit::External))
        .collect();
    tensors.push(decl(k, "y".into(), TensorRole::Output, TensorInit::Zero));
    let nt = tensors[k].tiles.cols_t;
    // ★ ONE PAGE CARRIES THE WHOLE VECTOR. The decision body is ONE dataflow program over a 1xN lane
    // vector; splitting it per tile made `place` clone the entire Load/Scalar/Store triple per tile, so a
    // 64-lane vector over 32-wide tiles reported TWO identical 75-cell Scalar stages for one program. The
    // page spans the tensor's tiles, the block spans them, and there is one work item — so the program
    // says what it is: one body, applied across the lanes.
    let fifo = |id: usize, name: String| Buffer {
        id: BufferId::new(id),
        name,
        kind: BufferKind::Fifo {
            depth_pages: 2,
            page_tiles: nt,
        },
        dtype: Precision::Int64,
        space: MemSpace::Local,
        site: None,
        origin: None,
        alias_of: None,
    };
    let mut buffers: Vec<Buffer> = (0..k).map(|i| fifo(i, format!("in{i}"))).collect();
    buffers.push(fifo(k, "out".into()));
    let pattern = || AccessPattern {
        origin: TileAddr {
            base: 0,
            steps: vec![(LV_ITEM_Y, nt), (LV_ITEM_X, nt)],
            indirect: None,
        },
        block_h: 1,
        block_w: nt,
        stride_w: 1,
        stride_h: nt,
    };
    let mut stages: Vec<Stage> = (0..k)
        .map(|i| Stage {
            id: StageId::new(i),
            name: format!("load_x{i}"),
            kind: StageKind::Load {
                tensor: TensorId::new(i),
                pattern: pattern(),
                num_blocks: 1,
                multicast: None,
            },
            role: SiteRole::Movement,
            ins: vec![],
            outs: vec![BufferId::new(i)],
            site: None,
            item: None,
            origin: None,
        })
        .collect();
    let body = StageId::new(k);
    stages.push(Stage {
        id: body,
        name: "body".into(),
        kind: StageKind::Scalar(ScalarBody {
            n_in: df.n_in,
            cells,
            result,
        }),
        role: SiteRole::Compute,
        ins: (0..k).map(BufferId::new).collect(),
        outs: vec![BufferId::new(k)],
        site: None,
        item: None,
        origin: None,
    });
    let store = StageId::new(k + 1);
    stages.push(Stage {
        id: store,
        name: "store_y".into(),
        kind: StageKind::Store {
            tensor: TensorId::new(k),
            pattern: pattern(),
            sub_blocks: (1, 1),
        },
        role: SiteRole::Movement,
        ins: vec![BufferId::new(k)],
        outs: vec![],
        site: None,
        item: None,
        origin: None,
    });
    let mut edges: Vec<Edge> = (0..k)
        .map(|i| Edge {
            id: EdgeId::new(i),
            from: StageId::new(i),
            buffer: BufferId::new(i),
            to: vec![body],
            handshake: local.clone(),
        })
        .collect();
    edges.push(Edge {
        id: EdgeId::new(k),
        from: body,
        buffer: BufferId::new(k),
        to: vec![store],
        handshake: local.clone(),
    });
    let items = vec![WorkItem {
        id: WorkItemId::new(0),
        coord: (0, 0),
        out: TileRect {
            row0_t: 0,
            col0_t: 0,
            h_t: 1,
            w_t: nt,
        },
    }];
    let mut requires = Requirements::default();
    requires.handshakes.insert(local.kind());
    let p = SpatialProgram {
        name,
        tile,
        tensors,
        stages,
        buffers,
        edges,
        semaphores: Vec::new(),
        sections: Vec::new(),
        trips: Vec::new(),
        links: Vec::new(),
        work: WorkList {
            items,
            k_tiles: 1,
            plan: None,
            k_trip: None,
        },
        numerics: NumericsPolicy::Exact,
        claim: NumericsClaim::Exact,
        acc_tiles_used: target.site.acc_tiles.max(1),
        mem_banks: target.site.mem_banks,
        requires,
        placement: None,
        memory: None,
        schedule: None,
        certificate: None,
    };
    p.validate()?;
    Ok(p)
}

/// Lower `source` onto `chip` through the whole-program lane: the first function as a `Scalar` stage over
/// [`WHOLE_PROGRAM_LANES`]-lane input vectors, or a `## Main` of `Show`s as the tape-select stage; the
/// program is run through the pinned passes to `Verified` (with its choreography certificate and the
/// per-PE certificate where the body is ALU + select). Every shape the dataflow lane cannot carry is
/// refused by name.
pub fn lower_whole_program_lane(
    source: &str,
    chip: RduChip,
) -> Result<SpatialProgram, SpatialReject> {
    let target = chip.facts().spatial_target();
    let scan = scan(source)?;
    if let Some(reason) = scan.frontier {
        return Err(SpatialReject::MissingCompute {
            needed: reason,
            target: target.name,
        });
    }
    if scan.recursion.is_some() {
        return Err(SpatialReject::NotLowered(
            "recursion: the first function reaches itself through the call graph — the decision front end inlines calls and would take the first-return path only; a spatial call stack is what a data-dependent recursion needs (the whole-program call-stack engine, `program_calls_fabric`, runs it ≡VM; a sequenced-program stage in the spatial IR is the expansion that lands it on the mesh)",
        ));
    }
    if scan.loop_in.is_some() {
        return Err(match &scan.unit {
            Unit::Function { name, .. } if counted_loop_fabric(source, name).is_ok() => SpatialReject::NotLowered(
                "a counted range loop is the loop lane's unit (`lower_loop_lane`: the Plasticine metapipeline on the RDU mesh descriptor), not a Scalar stage",
            ),
            _ => SpatialReject::NotLowered(
                "a loop outside the loop lane's single-stage counted shape (a `While`, a nested loop, a loop over a sequence, a loop in a called function): the loop fabrics model it and have no spatial-IR stage form yet",
            ),
        });
    }
    if scan.branch_in_main {
        return Err(SpatialReject::NotLowered("a branch at the top level of `Main` would make a `Show` conditional — the tape has no conditional lane; move the decision into a function"));
    }
    match scan.unit {
        Unit::Function { name, arity } => {
            if arity == 0 {
                return Err(SpatialReject::NotLowered("a parameterless first function has no input vector to stream lane-wise — write its value as a `Show` in `Main` (the tape form) or give it a parameter"));
            }
            let df = quietly(|| decision_fabric_from_source(source, &name))
                .map_err(SpatialReject::ScalarFrontEnd)?;
            if df.escaped_first_return {
                return Err(SpatialReject::NotLowered(
                    "an early return: a branch jumps past the first `Return`, and the decision front end would keep the first-return path only — write the function with one final `Return` and conditional `Set`s",
                ));
            }
            let p = scalar_program(
                &df,
                WHOLE_PROGRAM_LANES,
                &target,
                format!("rdu_whole_program:{name}"),
            )?;
            run_passes(p, &target)
        }
        Unit::Main => {
            let wpf = quietly(|| whole_program_fabric_from_source(source))
                .map_err(SpatialReject::ScalarFrontEnd)?;
            if wpf.outputs.is_empty() {
                return Err(SpatialReject::NotLowered(
                    "a `Main` without a `Show` has no tape to lower",
                ));
            }
            if wpf.outputs.iter().any(|(_, is_text, _)| *is_text) {
                return Err(SpatialReject::NotLowered("a `Show` of Text has no integer lane on the tape — the tape-select stage carries Int values"));
            }
            if wpf.outputs.iter().any(|(_, _, guard)| guard.is_some()) {
                return Err(SpatialReject::NotLowered("a policy `Check` guards the tape (a denied check halts it) — the tape-select stage has no conditional lane"));
            }
            // The tape-select body: lane i reads `In(0) == i ? show_i : …`, the program's cells shared.
            let mut cells = wpf.cells.clone();
            let k = wpf.outputs.len();
            let mut acc = wpf.outputs[k - 1].0;
            for i in (0..k - 1).rev() {
                let idx = cells.len();
                cells.push(LoopCell::Select {
                    cmp: CmpKind::Eq,
                    ca: Src::In(0),
                    cb: Src::Const(i as i64),
                    t: wpf.outputs[i].0,
                    e: acc,
                });
                acc = Src::Val(idx);
            }
            let df = DecisionFabric {
                n_in: 1,
                cells,
                result: acc,
                escaped_first_return: false,
            };
            let p = scalar_program(&df, k, &target, "rdu_whole_program:main".into())?;
            run_passes(p, &target)
        }
    }
}

/// The lane's input vectors for a program the whole-program lane produced, so a caller can run it: for a
/// function unit, [`anchor_params`]-seeded samples per parameter; for the tape, the Show indices.
pub fn whole_program_inputs(p: &SpatialProgram) -> super::super::eval::Inputs {
    use super::super::eval::{Inputs, Value};
    let mut inputs = Inputs::default();
    let n = p
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .map(|t| t.elems.cols)
        .unwrap_or(0);
    let is_tape = p.name == "rdu_whole_program:main";
    for (k, t) in p
        .tensors
        .iter()
        .filter(|t| t.role == TensorRole::Input)
        .enumerate()
    {
        let xs: Vec<i64> = if is_tape {
            (0..n as i64).collect()
        } else {
            (0..n).map(|i| sample(k, i)).collect()
        };
        inputs.tensors.insert(t.id, Value::Int(xs));
    }
    inputs
}

/// Lane `i` of parameter `k`'s sample vector: the anchor value on lane 0, then a deterministic spread of
/// small signed integers around it (every path of a decision body sees both signs and a run of magnitudes).
pub fn sample(k: usize, i: usize) -> i64 {
    let base = anchor_params(k + 1)[k];
    if i == 0 {
        return base;
    }
    let mut x = (i as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(k as u64 + 1);
    x ^= x >> 29;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 32;
    (x % 401) as i64 - 200
}

/// The lane's VM anchor for a function unit: `fn_name(args)` shown by the VM, `Main` replaced.
pub fn vm_anchor(source: &str, fn_name: &str, args: &[i64]) -> Result<i64, String> {
    let call = args
        .iter()
        .map(|p| {
            if *p < 0 {
                format!("(0 - {})", -p)
            } else {
                p.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let funcs = match source.find("## Main") {
        Some(idx) => &source[..idx],
        None => source,
    };
    let prog = format!("{funcs}## Main\n    Show {fn_name}({call}).\n");
    let out = crate::compile::vm_outcome(&prog);
    if let Some(e) = out.error {
        return Err(format!("the VM errored running `{fn_name}({call})`: {e}"));
    }
    out.output
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| {
            format!(
                "the VM printed no integer for `{fn_name}({call})`: {:?}",
                out.output
            )
        })
}
