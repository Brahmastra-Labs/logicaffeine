//! wprog_engine_rtl — the WHOLE-PROGRAM spatial engine: run an entire control-flow program on the
//! reconfigurable fabric, streamed over the wire, not just one straight-line block. Where the block engine
//! (`block_engine_rtl`) runs a single straight-line residual, this runs a whole `ProgramCfgFabric` — multiple
//! basic blocks with branch conditions (`Select` cells), a register file that survives block reconfiguration,
//! and a block-walk terminator (Jump / Branch / Return) — the RTL of `run_program_cfg_on_grid` driven by a
//! flat config, so ANY acyclic branchy program runs on ONE physical array by loading its config.
//!
//! This module is the reference + config layer (pure Rust, provable without a simulator): `serialize_program`
//! flattens a `ProgramCfgFabric` into a `WProgConfig`, and `eval_wprog_config` interprets that flat config
//! exactly as the RTL will — proven bit-identical to `run_program_cfg_on_grid` (≡ the VM) by
//! `wprog_engine_lock`. The RTL emitter + the over-the-wire SoC lower this same flat config to silicon.
//!
//! Correct-first scope: Arith + Select cells (arithmetic + branch decisions) — the whole acyclic branchy
//! control-flow family. The state plane (`Mem` cells → heap BRAM), loops (the loop fabric), and function
//! calls (a call stack) are the follow-ons that extend it toward the whole language.

use super::chip_protocol::{
    fill_verilog, IMAGE_GO, IMAGE_HEAP, TAG_BOOL, TAG_FLOAT, TAG_INT, TAG_TEXT, TAG_WORD32,
};
use super::fabric_rtl::{
    AluOp, CfgBlock, CmpKind, DecisionFabric, FuncUnit, LoopCell, ProgramCfgFabric, Src, Term,
};
use super::wpe_floorplan::ArrayRole;

/// An operand source in the flat config: a live-in register (`In`), a constant, or an earlier cell's value.
#[derive(Clone, Copy, Debug)]
pub struct WSrc {
    pub mode: u8, // 0 = In(ref), 1 = Const(konst), 2 = Val(ref)
    pub reference: u16,
    pub konst: i64,
}

/// One cell. `kind`: 0 = Arith `op(ops[0], ops[1])`, 1 = Select `cmp(ops[0],ops[1]) ? ops[2] : ops[3]`,
/// 3 = HeapRead `heap[ops[0]]`, 4 = HeapWrite `heap[ops[0]] = ops[1]` (drives `ops[1]`), 5 = HeapAlloc (grow the
/// heap by `ops[0]` words, drive the new base pointer) — the state plane, mirroring `eval_grid_with_heap`.
#[derive(Clone, Copy, Debug)]
pub struct WCell {
    pub kind: u8,
    pub code: u8, // AluOp code (Arith) or CmpKind code (Select); unused otherwise
    pub ops: [WSrc; 4],
}

/// A block's terminator in the flat config.
#[derive(Clone, Debug)]
pub enum WTerm {
    Jump(usize),
    Branch {
        cond: WSrc,
        then_blk: usize,
        else_blk: usize,
    },
    Return(WSrc),
    /// A function CALL: run the callee (its entry block `callee_block`) with `args` bound to its parameter
    /// registers `0..args.len()`, then resume at `ret_block` with the return value written to `dst_reg`. Runs
    /// on a FRAME STACK (each frame saves the caller's register file), so recursion of any depth works — the
    /// spatial call mechanism the whole-program fabric needs for the full language.
    Call {
        callee_block: usize,
        args: Vec<WSrc>,
        dst_reg: u16,
        ret_block: usize,
    },
}

/// One basic block: live-in registers, cells, register writes, terminator.
#[derive(Clone, Debug)]
pub struct WBlock {
    pub livein: Vec<u16>,
    pub cells: Vec<WCell>,
    pub writes: Vec<(u16, WSrc)>,
    pub term: WTerm,
}

/// A whole program flattened for the fabric.
#[derive(Clone, Debug)]
pub struct WProgConfig {
    pub n_params: usize,
    pub blocks: Vec<WBlock>,
}

fn alu_code(op: AluOp) -> u8 {
    match op {
        AluOp::Add => 0,
        AluOp::Sub => 1,
        AluOp::Mul => 2,
        AluOp::And => 3,
        AluOp::Or => 4,
        AluOp::Xor => 5,
        AluOp::Shl => 6,
        AluOp::Shr => 7,
        AluOp::UShr => 8,
    }
}

fn cmp_code(c: CmpKind) -> u8 {
    match c {
        CmpKind::Lt => 0,
        CmpKind::Gt => 1,
        CmpKind::Le => 2,
        CmpKind::Ge => 3,
        CmpKind::Eq => 4,
        CmpKind::Ne => 5,
    }
}

/// Encode a FUNCTIONAL UNIT to the `WCell.code` byte — integer division/mod/floor-div/power/magic-high-mul
/// AND the FLOAT LANE (`f64`-bit-pattern add/sub/mul/div/sqrt/floor + approximate-equality). `eval_wprog_config`
/// evaluates every one of these via the same `FuncUnit::eval` the CFG model uses, so the float benchmarks run.
fn func_code(u: FuncUnit) -> u8 {
    match u {
        FuncUnit::DivS => 0,
        FuncUnit::ModS => 1,
        FuncUnit::FloorDivS => 2,
        FuncUnit::Pow => 3,
        FuncUnit::MulHiU => 4,
        FuncUnit::FAddF => 5,
        FuncUnit::FSubF => 6,
        FuncUnit::FMulF => 7,
        FuncUnit::FDivF => 8,
        FuncUnit::FSqrtF => 9,
        FuncUnit::FFloorF => 10,
        FuncUnit::ApproxEqF => 11,
        FuncUnit::MulOvfS => 12,
        FuncUnit::IntToF => 13,
        FuncUnit::FLtF => 14,
        FuncUnit::FGtF => 15,
        FuncUnit::FLeF => 16,
        FuncUnit::FGeF => 17,
        FuncUnit::FEqF => 18,
        FuncUnit::FNeF => 19,
        FuncUnit::PopCount => 20,
    }
}

/// Does a `Func` cell with this code run on the FPU lane (`WITH_FPU`)? The float units (add/sub/mul/div/sqrt/
/// floor, the IEEE compares, int→float, approx-eq) exist only when the engine is built with the FPU, and so does
/// popcount (it rides the FPU's combinational lane; the FPU-less `MUL_FUNC_F` answers 0 for it). An image built
/// `WITH_FPU(0)` has no datapath for any of these and must not be handed such a program. The integer units —
/// div/mod/floor-div/pow and the DSP multiplies — run on every image.
pub fn func_code_needs_fpu(code: u8) -> bool {
    matches!(code, 5..=11 | 13..=20)
}

fn func_from_code(c: u8) -> FuncUnit {
    match c {
        0 => FuncUnit::DivS,
        1 => FuncUnit::ModS,
        2 => FuncUnit::FloorDivS,
        3 => FuncUnit::Pow,
        4 => FuncUnit::MulHiU,
        5 => FuncUnit::FAddF,
        6 => FuncUnit::FSubF,
        7 => FuncUnit::FMulF,
        8 => FuncUnit::FDivF,
        9 => FuncUnit::FSqrtF,
        10 => FuncUnit::FFloorF,
        11 => FuncUnit::ApproxEqF,
        12 => FuncUnit::MulOvfS,
        13 => FuncUnit::IntToF,
        14 => FuncUnit::FLtF,
        15 => FuncUnit::FGtF,
        16 => FuncUnit::FLeF,
        17 => FuncUnit::FGeF,
        18 => FuncUnit::FEqF,
        19 => FuncUnit::FNeF,
        _ => FuncUnit::PopCount,
    }
}

/// The Verilog `func_f` — the whole-program engine's FUNCTIONAL-UNIT evaluator, shared by the specialized
/// (`emit_wprog_verilog`) and the generic config-driven (`emit_wprog_engine_verilog`) emitters so the two can
/// never drift. Integer division/mod/floor-div/power are pure `i64`; the FLOAT LANE (codes 5–19) is a
/// BEHAVIORAL FPU using iverilog's real support (`$bitstoreal`/`$realtobits`/`$sqrt`/`$floor`) — bit-identical
/// to [`FuncUnit::eval`] in simulation, so a float program's image runs on the engine under iverilog with a real
/// cycle count. (A SYNTHESIZABLE multi-cycle FPU — the `fpu.v` datapath the decision-fabric float lane already
/// binds — is the hardware follow-on; this behavioral model proves the sequencing + the result, not the gates.)
/// The `op` selector is 8 bits (float-compare codes reach 19), so callers must pass the full code, not `[3:0]`.
fn func_f_verilog() -> &'static str {
    r#"  function signed [63:0] func_f(input [7:0] op, input signed [63:0] a, input signed [63:0] x);
    real ra, rx; integer pk;
    begin
      ra = $bitstoreal(a); rx = $bitstoreal(x);
      case (op)
        8'd0: func_f = (x==0) ? 64'sd0 : (a / x);
        8'd1: func_f = (x==0) ? 64'sd0 : (a % x);
        8'd2: func_f = (x==0) ? 64'sd0 : (((a % x) != 0) && ((a < 0) != (x < 0)) ? (a / x) - 64'sd1 : (a / x));
        8'd3: begin func_f = 64'sd1; for (pk=0; pk<x; pk=pk+1) func_f = func_f * a; end
        8'd5:  func_f = $realtobits(ra + rx);
        8'd6:  func_f = $realtobits(ra - rx);
        8'd7:  func_f = $realtobits(ra * rx);
        8'd8:  func_f = $realtobits(ra / rx);
        8'd9:  func_f = $realtobits($sqrt(ra));
        8'd10: func_f = $floor(ra);
        8'd13: begin ra = a; func_f = $realtobits(ra); end
        8'd14: func_f = (ra <  rx) ? 64'sd1 : 64'sd0;
        8'd15: func_f = (ra >  rx) ? 64'sd1 : 64'sd0;
        8'd16: func_f = (ra <= rx) ? 64'sd1 : 64'sd0;
        8'd17: func_f = (ra >= rx) ? 64'sd1 : 64'sd0;
        8'd18: func_f = (ra == rx) ? 64'sd1 : 64'sd0;
        8'd19: func_f = (ra != rx) ? 64'sd1 : 64'sd0;
        8'd20: func_f = $countones(a);
        default: func_f = 64'sd0;
      endcase
    end
  endfunction
"#
}

/// The INT→FLOAT promotion of a float op's operand for the CALL-STACK fabric (WCell form): a float op with an
/// integer operand promotes it through the `IntToF` functional unit (code 13) first, mirroring `program_cfg_fabric`'s
/// `promote_float_operand`. Returns the operand unchanged when it already carries an `f64` bit pattern.
fn wfpromote(cells: &mut Vec<WCell>, s: WSrc, is_float: bool) -> WSrc {
    if is_float {
        return s;
    }
    let zero = WSrc {
        mode: 1,
        reference: 0,
        konst: 0,
    };
    let k = cells.len();
    cells.push(WCell {
        kind: 2,
        code: func_code(FuncUnit::IntToF),
        ops: [s, zero, zero, zero],
    });
    WSrc {
        mode: 2,
        reference: k as u16,
        konst: 0,
    }
}

/// Mask a value to 32 bits (`& 0xFFFFFFFF`) — the WORD32 wrapping into ℤ/2³² after arithmetic. An `And` ALU cell.
fn wmask32(cells: &mut Vec<WCell>, s: WSrc) -> WSrc {
    let zero = WSrc {
        mode: 1,
        reference: 0,
        konst: 0,
    };
    let mask = WSrc {
        mode: 1,
        reference: 0,
        konst: 0xFFFF_FFFF,
    };
    let k = cells.len();
    cells.push(WCell {
        kind: 0,
        code: 3,
        ops: [s, mask, zero, zero],
    }); // code 3 = AluOp::And
    WSrc {
        mode: 2,
        reference: k as u16,
        konst: 0,
    }
}

fn enc_src(s: Src) -> WSrc {
    match s {
        Src::In(k) => WSrc {
            mode: 0,
            reference: k as u16,
            konst: 0,
        },
        Src::Const(c) => WSrc {
            mode: 1,
            reference: 0,
            konst: c,
        },
        Src::Val(j) => WSrc {
            mode: 2,
            reference: j as u16,
            konst: 0,
        },
    }
}

const ZERO_SRC: WSrc = WSrc {
    mode: 1,
    reference: 0,
    konst: 0,
};

fn enc_cell(c: &LoopCell) -> WCell {
    match *c {
        LoopCell::Arith { op, a, b } => {
            WCell { kind: 0, code: alu_code(op), ops: [enc_src(a), enc_src(b), ZERO_SRC, ZERO_SRC] }
        }
        LoopCell::Select { cmp, ca, cb, t, e } => WCell {
            kind: 1,
            code: cmp_code(cmp),
            ops: [enc_src(ca), enc_src(cb), enc_src(t), enc_src(e)],
        },
        LoopCell::HeapRead { addr } => {
            WCell { kind: 3, code: 0, ops: [enc_src(addr), ZERO_SRC, ZERO_SRC, ZERO_SRC] }
        }
        LoopCell::HeapWrite { addr, val } => {
            WCell { kind: 4, code: 0, ops: [enc_src(addr), enc_src(val), ZERO_SRC, ZERO_SRC] }
        }
        LoopCell::HeapAlloc { size } => {
            WCell { kind: 5, code: 0, ops: [enc_src(size), ZERO_SRC, ZERO_SRC, ZERO_SRC] }
        }
        LoopCell::Func { unit, a, b } => {
            WCell { kind: 2, code: func_code(unit), ops: [enc_src(a), enc_src(b), ZERO_SRC, ZERO_SRC] }
        }
        ref other => panic!(
            "wprog: cell {other:?} is outside the current scope (Arith + Select + Func + heap state plane) — \
             float-lane and loop-sequence cells are the follow-on"
        ),
    }
}

fn enc_block(b: &CfgBlock) -> WBlock {
    WBlock {
        livein: b.inputs.clone(),
        cells: b.body.iter().map(enc_cell).collect(),
        writes: b.writes.iter().map(|(r, s)| (*r, enc_src(*s))).collect(),
        term: match &b.term {
            Term::Jump(t) => WTerm::Jump(*t),
            Term::Branch {
                cond,
                then_blk,
                else_blk,
            } => WTerm::Branch {
                cond: enc_src(*cond),
                then_blk: *then_blk,
                else_blk: *else_blk,
            },
            Term::Return(s) => WTerm::Return(enc_src(*s)),
        },
    }
}

/// Flatten a whole-function CFG fabric into the fabric's flat config.
pub fn serialize_program(pf: &ProgramCfgFabric) -> WProgConfig {
    WProgConfig {
        n_params: pf.n_params,
        blocks: pf.blocks.iter().map(enc_block).collect(),
    }
}

/// Wrap a single-block if-converted DECISION FABRIC as a one-block flat config, so its cells run on the
/// CROSSBAR whole-program engine (`eval_wprog_config`). The crossbar datapath has full interconnect — a cell
/// may read any earlier cell's value directly — so a COMPUTED comparison predicate needs no spatial routing.
/// This runs constructs the nearest-neighbour placer (`route_loop_body`) cannot place, e.g. a `Set`'s dedup
/// scan, on the fabric ≡ the VM (the decision fabric's lowering is proven; only the *placement* was the limit).
pub fn wprog_from_decision_fabric(df: &DecisionFabric) -> WProgConfig {
    let block = WBlock {
        livein: (0..df.n_in as u16).collect(),
        cells: df.cells.iter().map(enc_cell).collect(),
        writes: Vec::new(),
        term: WTerm::Return(enc_src(df.result)),
    };
    WProgConfig {
        n_params: df.n_in,
        blocks: vec![block],
    }
}

/// Lift a WHOLE multi-function program (with function CALLS and recursion) to a `WProgConfig` carrying `Call`
/// terminators — the source→config half of recursion (the frame-stack `eval_wprog_config` is the eval half).
/// `fn_name` is lifted FIRST so its entry is block 0 (the config's entry); every function shares one global
/// block index space, and a `Call` targets the callee function's entry block. Correct-first scope: integer
/// bodies + calls (LoadConst-int / Move / Add·Sub·Mul / AddAssign / Div·Mod·FloorDiv / comparisons / Call /
/// Return / Jump family) — enough for fib/sumdown/gcd-style recursion; collections-in-recursion is the
/// follow-on (the heap arms would be added the same way `program_cfg_fabric` has them).
pub fn program_calls_fabric(src: &str, fn_name: &str) -> WProgConfig {
    lift_program(src, Some(fn_name))
}

/// Lift a WHOLE LOGOS PROGRAM from its `## Main`: the code region the compiler emits before the first function
/// body (terminated by `Halt`), plus every function it transitively calls. Main takes no parameters and its
/// `Halt` is a top-level `Return 0`; its `Show`s become SHOW cells (kind 10) so the program's OUTPUT — not just a
/// return value — runs on the engine, ≡ `vm_outcome(src).output` line for line (`program_output`).
pub fn program_main_fabric(src: &str) -> WProgConfig {
    lift_program(src, None)
}

fn lift_program(src: &str, fn_name: Option<&str>) -> WProgConfig {
    use crate::vm::{Compiler, Constant, Op};
    use std::collections::{BTreeSet, HashMap};

    if let Some(refusal) = crate::compile::exact_refusal(src, "fabric", "the fabric") {
        panic!("{refusal}");
    }

    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _t, _p) = parsed.expect("parse failed");
        let program = Compiler::compile(stmts, interner).expect("compile failed");
        let code = &program.code;

        // The entry REGION: a named function, or `## Main` — the code the compiler emits before the first
        // function body (pass 2a), terminated by `Halt`. Main is not a `CompiledFunction`, so it is described by
        // its pc range directly; `None` in the function-index slots below means "the Main region".
        let main_end = program
            .functions
            .iter()
            .map(|f| f.entry_pc)
            .min()
            .unwrap_or(code.len());
        let entry_fi: Option<usize> = fn_name.map(|name| {
            program
                .functions
                .iter()
                .position(|f| interner.resolve(f.name) == name)
                .unwrap_or_else(|| panic!("function `{name}` not found"))
        });
        // REACHABILITY: lift only functions transitively CALLED from the entry. Dead prelude/stdlib functions
        // (e.g. an unused math tower — `sqrt`/`complex`/`parseFloat`) must NOT be lowered, or their unsupported
        // ops would falsely refuse a program that never calls them (the fannkuch phantom-`Sqrt`). `Op::Call`'s
        // `func` is the callee index directly, so the closure is a plain worklist.
        let fn_end = |fi: usize| -> usize {
            let s = program.functions[fi].entry_pc;
            program
                .functions
                .iter()
                .map(|f| f.entry_pc)
                .filter(|&e| e > s)
                .min()
                .unwrap_or(code.len())
        };
        let region_of = |fi: Option<usize>| -> (usize, usize) {
            match fi {
                Some(fi) => (program.functions[fi].entry_pc, fn_end(fi)),
                None => (0, main_end),
            }
        };
        let mut reachable: BTreeSet<usize> = BTreeSet::new();
        let mut work: Vec<Option<usize>> = vec![entry_fi];
        let mut main_scanned = false;
        while let Some(fi) = work.pop() {
            match fi {
                Some(i) if !reachable.insert(i) => continue,
                None if main_scanned => continue,
                None => main_scanned = true,
                _ => {}
            }
            let (s, e) = region_of(fi);
            for pc in s..e {
                if let Op::Call { func, .. } = code[pc] {
                    work.push(Some(func as usize));
                }
            }
        }
        // Lift the entry region FIRST (so its entry becomes global block 0), then the reachable rest by entry_pc.
        let mut order: Vec<Option<usize>> = vec![entry_fi];
        let mut rest: Vec<usize> = reachable
            .iter()
            .copied()
            .filter(|&i| Some(i) != entry_fi)
            .collect();
        rest.sort_by_key(|&fi| program.functions[fi].entry_pc);
        order.extend(rest.into_iter().map(Some));

        // Per-function block leaders (absolute pcs): the entry, every jump target, the op after a
        // jump/return, and the op after a CALL (calls end their block, the return point starts a new one).
        struct Lift {
            fi: Option<usize>,
            end_pc: usize,
            leaders: Vec<usize>,
        }
        let mut lifts: Vec<Lift> = Vec::new();
        for &fi in order.iter() {
            // Bound this region's code range against ALL functions' entry_pcs (not just the lifted ones), so a
            // reachable function never swallows a skipped dead function's code that sits between them.
            let (entry_pc, end_pc) = region_of(fi);
            let mut leaders: BTreeSet<usize> = BTreeSet::new();
            leaders.insert(entry_pc);
            for pc in entry_pc..end_pc {
                match code[pc] {
                    Op::Jump { target }
                    | Op::JumpIfFalse { target, .. }
                    | Op::JumpIfTrue { target, .. } => {
                        leaders.insert(target);
                        if pc + 1 < end_pc {
                            leaders.insert(pc + 1);
                        }
                    }
                    // `IterNext` is a CONDITIONAL BRANCH (next value → fall through; exhausted → `exit`), so it
                    // ends its block and both successors are leaders — exactly like a `JumpIfFalse`.
                    Op::IterNext { exit, .. } => {
                        leaders.insert(exit);
                        if pc + 1 < end_pc {
                            leaders.insert(pc + 1);
                        }
                    }
                    Op::Call { .. } | Op::Return { .. } | Op::ReturnNothing | Op::Halt => {
                        if pc + 1 < end_pc {
                            leaders.insert(pc + 1);
                        }
                    }
                    _ => {}
                }
            }
            let leaders: Vec<usize> = leaders
                .into_iter()
                .filter(|&l| l >= entry_pc && l < end_pc)
                .collect();
            lifts.push(Lift {
                fi,
                end_pc,
                leaders,
            });
        }

        // Global block offsets + per-function entry block.
        let mut offset: Vec<usize> = Vec::new();
        let mut total = 0usize;
        for l in &lifts {
            offset.push(total);
            total += l.leaders.len();
        }
        let mut fn_entry: HashMap<usize, usize> = HashMap::new();
        for (li, l) in lifts.iter().enumerate() {
            if let Some(fi) = l.fi {
                fn_entry.insert(fi, offset[li]);
            }
        }
        // Global block index of the block containing `pc` in lift `li`.
        let block_of = |li: usize, pc: usize| -> usize {
            let ls = &lifts[li].leaders;
            offset[li] + (ls.partition_point(|&l| l <= pc) - 1)
        };

        let get = |reg: &mut HashMap<u16, WSrc>, inputs: &mut Vec<u16>, r: u16| -> WSrc {
            if let Some(s) = reg.get(&r) {
                return *s;
            }
            let k = inputs.len();
            inputs.push(r);
            let s = WSrc {
                mode: 0,
                reference: k as u16,
                konst: 0,
            };
            reg.insert(r, s);
            s
        };
        let record = |wo: &mut Vec<u16>, dst: u16| {
            if !wo.contains(&dst) {
                wo.push(dst);
            }
        };

        use crate::vm::BoundaryType;
        let mut blocks: Vec<WBlock> = Vec::new();
        for (li, l) in lifts.iter().enumerate() {
            // Per-function FLOAT tracking (registers are per-function under the frame-stack model). Seed it from
            // the DECLARED param types — a `Float` param is a float register, a `Seq of Float` a float collection
            // — so float-ness flows across recursive calls WITHOUT propagation (the callee re-derives it). Float
            // arithmetic then routes through the float functional units, exactly like `program_cfg_fabric`.
            let mut float_regs: std::collections::HashSet<u16> = std::collections::HashSet::new();
            let mut float_colls: std::collections::HashSet<u16> = std::collections::HashSet::new();
            // WORD32 (ℤ/2³²) tracking, mirroring the float lane: a `word32` value in an i64 slot, so arithmetic
            // must WRAP mod 2³² (mask to 32 bits). Seeded from `Word32` / `Seq of Word32` params. What sha256 needs.
            let mut w32_regs: std::collections::HashSet<u16> = std::collections::HashSet::new();
            let mut w32_colls: std::collections::HashSet<u16> = std::collections::HashSet::new();
            // Main has no parameters; a function's declared types seed the lanes below.
            let param_types: &[Option<BoundaryType>] = match l.fi {
                Some(fi) => &program.functions[fi].param_types,
                None => &[],
            };
            for (r, pt) in param_types.iter().enumerate() {
                match pt {
                    Some(BoundaryType::Float) => {
                        float_regs.insert(r as u16);
                    }
                    Some(BoundaryType::Word32) => {
                        w32_regs.insert(r as u16);
                    }
                    Some(BoundaryType::Seq(inner)) if matches!(**inner, BoundaryType::Float) => {
                        float_colls.insert(r as u16);
                    }
                    Some(BoundaryType::Seq(inner)) if matches!(**inner, BoundaryType::Word32) => {
                        w32_colls.insert(r as u16);
                    }
                    _ => {}
                }
            }
            // PARAMETER-COLLECTION tracking (value semantics across a call boundary). A `Move` of a collection
            // that came in as a PARAMETER must DEEP-COPY its heap block (the VM's copy-on-write) — else
            // `Let mutable next be queens; Push v to next` grows into the CALLER's shared reserved region and
            // corrupts it (the nqueens divergence). A LOCAL builder Seq is NOT copied on Move: LOGOS keeps such
            // locals aliased (the COW-exempt in-place-write path heap_sort/mergesort rely on), matching the VM.
            let mut param_colls: std::collections::HashSet<u16> = std::collections::HashSet::new();
            for (r, pt) in param_types.iter().enumerate() {
                if matches!(pt, Some(BoundaryType::Seq(_))) {
                    param_colls.insert(r as u16);
                }
            }
            // TEXT tracking: registers holding a Text (a char-list `[len, byte0..]` on the heap). Set at a Text
            // `LoadConst`, at a text `Add` (concat), and at a text `item i of` (char-at); propagated by `Move`.
            // Routes `Add`→concat, `Index`→char-at, `Eq`→text-equality — mirroring the decision-fabric text lane.
            let mut text_regs: std::collections::HashSet<u16> = std::collections::HashSet::new();
            // BOOL tracking: registers holding a 0/1 predicate — a `Bool` literal or a comparison result — so a
            // `Show` renders `true`/`false` rather than `1`/`0`. Cleared wherever a register takes a non-Bool
            // value (alongside the float clears below); propagated by `Move`. Only `Show` reads it: the cells a
            // Bool feeds are unchanged (it is a 0/1 Int on the datapath, exactly as before).
            let mut bool_regs: std::collections::HashSet<u16> = std::collections::HashSet::new();
            // ITERATOR (range / for-each) tracking. A range iterator is a heap object `[cursor, limit]`; a Seq
            // iterator is `[index, coll_ptr]`. Loops are STRUCTURED, so the active iterator for each `IterNext`
            // is known at lowering — a lowering-time stack (pushed at `IterPrepare`, popped at `IterPop`). Range
            // iterators reuse the `NewRange` dst register; Seq iterators get a fresh register (their object ptr).
            let mut range_regs: std::collections::HashSet<u16> = std::collections::HashSet::new();
            // (is_seq, iterator-object REGISTER).
            let mut iter_stack: Vec<(bool, u16)> = Vec::new();
            // A fresh register base for synthesized Seq-iterator object pointers (for-each). Well above any real
            // register a function uses; each function has its own slot space under the frame stack, so per-function
            // is fine. (Model-only — the RTL's fixed NREG rejects it via the capacity guard, and no benchmark hits
            // it since range/for-each loops are otherwise While-shaped.)
            let mut fresh_reg: u16 = 60000;
            // A register an op WRITES without READING holds nothing it held before: every lane mark on it is
            // dropped before the op is lowered, so the op alone decides its kind — a register the VM reuses for
            // an Int after a Text or a comparison no longer reads as either. A register both read and written
            // keeps its marks for the op to read.
            macro_rules! forget_overwritten {
                ($op:expr) => {{
                    let mut read = Vec::new();
                    $op.for_each_read_reg(
                        |func| {
                            program
                                .functions
                                .get(func as usize)
                                .map_or(0, |f| f.captures.len())
                        },
                        |r| read.push(r),
                    );
                    let mut written = Vec::new();
                    $op.for_each_written_reg(|r| written.push(r));
                    for r in written.into_iter().filter(|r| !read.contains(r)) {
                        float_regs.remove(&r);
                        float_colls.remove(&r);
                        w32_regs.remove(&r);
                        w32_colls.remove(&r);
                        param_colls.remove(&r);
                        text_regs.remove(&r);
                        bool_regs.remove(&r);
                        range_regs.remove(&r);
                    }
                }};
            }
            for (bk, &bstart) in l.leaders.iter().enumerate() {
                let bend = if bk + 1 < l.leaders.len() {
                    l.leaders[bk + 1]
                } else {
                    l.end_pc
                };
                let ops = &code[bstart..bend];
                let (body_ops, term_op): (&[Op], Option<Op>) = match ops.last() {
                    Some(
                        t @ (Op::Jump { .. }
                        | Op::JumpIfFalse { .. }
                        | Op::JumpIfTrue { .. }
                        | Op::Return { .. }
                        | Op::ReturnNothing
                        | Op::Call { .. }
                        | Op::IterNext { .. }
                        | Op::Halt),
                    ) => (&ops[..ops.len() - 1], Some(*t)),
                    _ => (ops, None),
                };
                let mut reg: HashMap<u16, WSrc> = HashMap::new();
                let mut inputs: Vec<u16> = Vec::new();
                let mut cells: Vec<WCell> = Vec::new();
                let mut wo: Vec<u16> = Vec::new();
                let z = WSrc {
                    mode: 1,
                    reference: 0,
                    konst: 0,
                };
                // A prior cell's value (mode 2) / a signed constant (mode 1) — for the heap-cell sequences below.
                let val = |k: usize| WSrc {
                    mode: 2,
                    reference: k as u16,
                    konst: 0,
                };
                let con = |c: i64| WSrc {
                    mode: 1,
                    reference: 0,
                    konst: c,
                };
                for op in body_ops {
                    forget_overwritten!(op);
                    match op {
                        Op::LoadConst { dst, idx } => match &program.constants[*idx as usize] {
                            Constant::Int(n) => {
                                reg.insert(*dst, WSrc { mode: 1, reference: 0, konst: *n });
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                float_colls.remove(dst);
                                w32_regs.remove(dst);
                                w32_colls.remove(dst);
                                text_regs.remove(dst); // register REUSE: a former Text reg now holds an Int
                                record(&mut wo, *dst);
                            }
                            // A Bool rides a scalar slot as 0/1 (exactly as `exec_ops` lowers it).
                            Constant::Bool(b) => {
                                reg.insert(*dst, WSrc { mode: 1, reference: 0, konst: i64::from(*b) });
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                float_colls.remove(dst);
                                w32_regs.remove(dst);
                                w32_colls.remove(dst);
                                text_regs.remove(dst);
                                bool_regs.insert(*dst);
                                record(&mut wo, *dst);
                            }
                            // A Float literal → its f64 BIT PATTERN, with `dst` marked float so downstream
                            // arithmetic routes to the float functional units.
                            Constant::Float(f) => {
                                reg.insert(*dst, WSrc { mode: 1, reference: 0, konst: f.to_bits() as i64 });
                                float_regs.insert(*dst);
                                float_colls.remove(dst);
                                text_regs.remove(dst);
                                record(&mut wo, *dst);
                            }
                            // A Text literal → a char-list `[len, byte0, byte1, ...]` on the heap (UTF-8 bytes,
                            // so `length of` = byte count, matching the VM), and `dst` marked TEXT.
                            Constant::Text(s) => {
                                let bytes = s.as_bytes();
                                let n = bytes.len();
                                let ka = cells.len();
                                cells.push(WCell { kind: 5, code: 0, ops: [con(n as i64 + 1), z, z, z] });
                                let ptr = val(ka);
                                cells.push(WCell { kind: 4, code: 0, ops: [ptr, con(n as i64), z, z] });
                                for (i, b) in bytes.iter().enumerate() {
                                    let kadd = cells.len();
                                    cells.push(WCell { kind: 0, code: 0, ops: [ptr, con(i as i64 + 1), z, z] });
                                    cells.push(WCell { kind: 4, code: 0, ops: [val(kadd), con(*b as i64), z, z] });
                                }
                                reg.insert(*dst, ptr);
                                text_regs.insert(*dst);
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                w32_regs.remove(dst);
                                record(&mut wo, *dst);
                            }
                            c => panic!("calls fabric: constant {c:?} unsupported (integer-recursion scope)"),
                        },
                        Op::Move { dst, src } => {
                            let s = get(&mut reg, &mut inputs, *src);
                            // VALUE SEMANTICS: a `Move` of a PARAMETER collection deep-copies its heap block (the
                            // VM's COW), so a later `Push`/`Set item` on the copy never mutates the caller's Seq.
                            // A scalar or local-collection Move is a plain register copy (aliasing a local is the
                            // COW-exempt path the in-place sorts rely on).
                            if param_colls.contains(src) {
                                let k = cells.len();
                                cells.push(WCell { kind: 6, code: 0, ops: [s, z, z, z] }); // deep-copy collection
                                reg.insert(*dst, val(k));
                            } else {
                                reg.insert(*dst, s);
                            }
                            // A copy carries float-ness / word32-ness (scalar and collection-element).
                            if float_regs.contains(src) { float_regs.insert(*dst); } else { float_regs.remove(dst); }
                            if float_colls.contains(src) { float_colls.insert(*dst); } else { float_colls.remove(dst); }
                            if w32_regs.contains(src) { w32_regs.insert(*dst); } else { w32_regs.remove(dst); }
                            if bool_regs.contains(src) { bool_regs.insert(*dst); } else { bool_regs.remove(dst); }
                            if w32_colls.contains(src) { w32_colls.insert(*dst); } else { w32_colls.remove(dst); }
                            if text_regs.contains(src) { text_regs.insert(*dst); } else { text_regs.remove(dst); }
                            record(&mut wo, *dst);
                        }
                        // `RevSwap a, b` exchanges two registers in place: each value, and everything the tracking
                        // knows of it, moves to the other register. No copy — a parameter collection stays one
                        // value under its new name. Block writes commit together, so the exchange is simultaneous.
                        Op::RevSwap { a, b } => {
                            let va = get(&mut reg, &mut inputs, *a);
                            let vb = get(&mut reg, &mut inputs, *b);
                            reg.insert(*a, vb);
                            reg.insert(*b, va);
                            for set in [
                                &mut float_regs,
                                &mut float_colls,
                                &mut w32_regs,
                                &mut w32_colls,
                                &mut param_colls,
                                &mut text_regs,
                                &mut bool_regs,
                                &mut range_regs,
                            ] {
                                let (ha, hb) = (set.contains(a), set.contains(b));
                                if ha != hb {
                                    let (from, to) = if ha { (*a, *b) } else { (*b, *a) };
                                    set.remove(&from);
                                    set.insert(to);
                                }
                            }
                            record(&mut wo, *a);
                            record(&mut wo, *b);
                        }
                        Op::Add { dst, lhs, rhs }
                        | Op::Sub { dst, lhs, rhs }
                        | Op::Mul { dst, lhs, rhs } => {
                            let a = get(&mut reg, &mut inputs, *lhs);
                            let b = get(&mut reg, &mut inputs, *rhs);
                            let (lt, rt) = (text_regs.contains(lhs), text_regs.contains(rhs));
                            // TEXT CONCAT: `text + x` / `x + text` (only `Add`) builds a fresh char-list, rendering
                            // a non-text operand to its decimal bytes (the VM's `value + text` coercion).
                            if (lt || rt) && matches!(op, Op::Add { .. }) {
                                let code = (u8::from(lt)) | (u8::from(rt) << 1);
                                let k = cells.len();
                                cells.push(WCell { kind: 7, code, ops: [a, b, z, z] });
                                reg.insert(*dst, val(k));
                                text_regs.insert(*dst);
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                w32_regs.remove(dst);
                                record(&mut wo, *dst);
                                continue;
                            }
                            let (lf, rf) = (float_regs.contains(lhs), float_regs.contains(rhs));
                            let (lw, rw) = (w32_regs.contains(lhs), w32_regs.contains(rhs));
                            let cc = match op {
                                Op::Add { .. } => 0,
                                Op::Sub { .. } => 1,
                                _ => 2,
                            };
                            if lf || rf {
                                let a2 = wfpromote(&mut cells, a, lf);
                                let b2 = wfpromote(&mut cells, b, rf);
                                let fc = func_code(match op {
                                    Op::Add { .. } => FuncUnit::FAddF,
                                    Op::Sub { .. } => FuncUnit::FSubF,
                                    _ => FuncUnit::FMulF,
                                });
                                let k = cells.len();
                                cells.push(WCell { kind: 2, code: fc, ops: [a2, b2, z, z] });
                                reg.insert(*dst, WSrc { mode: 2, reference: k as u16, konst: 0 });
                                float_regs.insert(*dst);
                                float_colls.remove(dst);
                                w32_regs.remove(dst);
                            } else if lw || rw {
                                // WORD32 arithmetic wraps mod 2³²: the raw ALU op, then a 32-bit mask.
                                let k = cells.len();
                                cells.push(WCell { kind: 0, code: cc, ops: [a, b, z, z] });
                                let masked = wmask32(&mut cells, WSrc { mode: 2, reference: k as u16, konst: 0 });
                                reg.insert(*dst, masked);
                                w32_regs.insert(*dst);
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                float_colls.remove(dst);
                                w32_colls.remove(dst);
                            } else {
                                let k = cells.len();
                                cells.push(WCell { kind: 0, code: cc, ops: [a, b, z, z] });
                                reg.insert(*dst, WSrc { mode: 2, reference: k as u16, konst: 0 });
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                float_colls.remove(dst);
                                w32_regs.remove(dst);
                            }
                            text_regs.remove(dst); // an int/float/word32 arithmetic result is not Text
                            record(&mut wo, *dst);
                        }
                        Op::AddAssign { dst, src } => {
                            let a = get(&mut reg, &mut inputs, *dst);
                            let b = get(&mut reg, &mut inputs, *src);
                            let (dt, st) = (text_regs.contains(dst), text_regs.contains(src));
                            // TEXT CONCAT in place: `result += x` where `result` is Text (the `Set result to result
                            // + …` string builder) — append `x`'s chars (a non-text `x` is rendered to decimal).
                            if dt || st {
                                let code = (u8::from(dt)) | (u8::from(st) << 1);
                                let k = cells.len();
                                cells.push(WCell { kind: 7, code, ops: [a, b, z, z] });
                                reg.insert(*dst, val(k));
                                text_regs.insert(*dst);
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                w32_regs.remove(dst);
                                record(&mut wo, *dst);
                                continue;
                            }
                            let (df, sf) = (float_regs.contains(dst), float_regs.contains(src));
                            if df || sf {
                                let a2 = wfpromote(&mut cells, a, df);
                                let b2 = wfpromote(&mut cells, b, sf);
                                let k = cells.len();
                                cells.push(WCell { kind: 2, code: func_code(FuncUnit::FAddF), ops: [a2, b2, z, z] });
                                reg.insert(*dst, WSrc { mode: 2, reference: k as u16, konst: 0 });
                                float_regs.insert(*dst);
                                float_colls.remove(dst);
                            } else {
                                let k = cells.len();
                                cells.push(WCell { kind: 0, code: 0, ops: [a, b, z, z] });
                                reg.insert(*dst, WSrc { mode: 2, reference: k as u16, konst: 0 });
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                float_colls.remove(dst);
                            }
                            text_regs.remove(dst); // an int/float `+=` result is not Text
                            record(&mut wo, *dst);
                        }
                        Op::Div { dst, lhs, rhs }
                        | Op::Mod { dst, lhs, rhs }
                        | Op::FloorDiv { dst, lhs, rhs } => {
                            let a = get(&mut reg, &mut inputs, *lhs);
                            let b = get(&mut reg, &mut inputs, *rhs);
                            let (lf, rf) = (float_regs.contains(lhs), float_regs.contains(rhs));
                            if lf || rf {
                                let a2 = wfpromote(&mut cells, a, lf);
                                let b2 = wfpromote(&mut cells, b, rf);
                                let k = cells.len();
                                cells.push(WCell { kind: 2, code: func_code(FuncUnit::FDivF), ops: [a2, b2, z, z] });
                                reg.insert(*dst, WSrc { mode: 2, reference: k as u16, konst: 0 });
                                float_regs.insert(*dst);
                                float_colls.remove(dst);
                            } else {
                                let cc = match op {
                                    Op::Mod { .. } => 1,
                                    Op::FloorDiv { .. } => 2,
                                    _ => 0,
                                };
                                let k = cells.len();
                                cells.push(WCell { kind: 2, code: cc, ops: [a, b, z, z] });
                                reg.insert(*dst, WSrc { mode: 2, reference: k as u16, konst: 0 });
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                float_colls.remove(dst);
                            }
                            record(&mut wo, *dst);
                        }
                        Op::Lt { dst, lhs, rhs }
                        | Op::Gt { dst, lhs, rhs }
                        | Op::LtEq { dst, lhs, rhs }
                        | Op::GtEq { dst, lhs, rhs }
                        | Op::Eq { dst, lhs, rhs }
                        | Op::NotEq { dst, lhs, rhs } => {
                            let a = get(&mut reg, &mut inputs, *lhs);
                            let b = get(&mut reg, &mut inputs, *rhs);
                            // TEXT EQUALITY: `text == text` / `text != text` compares char-lists byte-for-byte
                            // (only `Eq`/`NotEq` apply to Text). Yields the 0/1 Int predicate.
                            if (text_regs.contains(lhs) || text_regs.contains(rhs))
                                && matches!(op, Op::Eq { .. } | Op::NotEq { .. })
                            {
                                let code = if matches!(op, Op::Eq { .. }) { 0 } else { 1 };
                                let k = cells.len();
                                cells.push(WCell { kind: 9, code, ops: [a, b, z, z] });
                                reg.insert(*dst, val(k));
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                w32_regs.remove(dst);
                                text_regs.remove(dst);
                                bool_regs.insert(*dst);
                                record(&mut wo, *dst);
                                continue;
                            }
                            let (lf, rf) = (float_regs.contains(lhs), float_regs.contains(rhs));
                            if lf || rf {
                                // A genuine IEEE-754 float comparison (float-compare functional units), promoting
                                // an integer operand first. Yields the 0/1 predicate (an Int).
                                let a2 = wfpromote(&mut cells, a, lf);
                                let b2 = wfpromote(&mut cells, b, rf);
                                let fc = func_code(match op {
                                    Op::Lt { .. } => FuncUnit::FLtF,
                                    Op::Gt { .. } => FuncUnit::FGtF,
                                    Op::LtEq { .. } => FuncUnit::FLeF,
                                    Op::GtEq { .. } => FuncUnit::FGeF,
                                    Op::Eq { .. } => FuncUnit::FEqF,
                                    _ => FuncUnit::FNeF,
                                });
                                let k = cells.len();
                                cells.push(WCell { kind: 2, code: fc, ops: [a2, b2, z, z] });
                                reg.insert(*dst, WSrc { mode: 2, reference: k as u16, konst: 0 });
                            } else {
                                let cc = match op {
                                    Op::Lt { .. } => 0,
                                    Op::Gt { .. } => 1,
                                    Op::LtEq { .. } => 2,
                                    Op::GtEq { .. } => 3,
                                    Op::Eq { .. } => 4,
                                    _ => 5,
                                };
                                let k = cells.len();
                                let one = WSrc { mode: 1, reference: 0, konst: 1 };
                                cells.push(WCell { kind: 1, code: cc, ops: [a, b, one, z] });
                                reg.insert(*dst, WSrc { mode: 2, reference: k as u16, konst: 0 });
                            }
                            float_regs.remove(dst); // a 0/1 predicate is an Int on the datapath …
                            text_regs.remove(dst);
                            float_colls.remove(dst);
                            w32_regs.remove(dst);
                            w32_colls.remove(dst);
                            bool_regs.insert(*dst); // … and a Bool to `Show`
                            record(&mut wo, *dst);
                        }
                        // bitwise / shift → the same combinational ALU codes (And=3/Or=4/Xor=5/Shl=6/Shr=7);
                        // `~x` already lowers to `x ^ -1` upstream, so no separate NOT is needed. This is what
                        // `nqueens`'s bitmask recursion needs.
                        Op::BitAnd { dst, lhs, rhs }
                        | Op::BitOr { dst, lhs, rhs }
                        | Op::BitXor { dst, lhs, rhs }
                        | Op::Shl { dst, lhs, rhs }
                        | Op::Shr { dst, lhs, rhs } => {
                            let a = get(&mut reg, &mut inputs, *lhs);
                            let b = get(&mut reg, &mut inputs, *rhs);
                            let cc = match op {
                                Op::BitAnd { .. } => 3,
                                Op::BitOr { .. } => 4,
                                Op::BitXor { .. } => 5,
                                Op::Shl { .. } => 6,
                                _ => 7,
                            };
                            let w32 = w32_regs.contains(lhs) || w32_regs.contains(rhs);
                            let k = cells.len();
                            cells.push(WCell { kind: 0, code: cc, ops: [a, b, z, z] });
                            // WORD32 propagation: xor/and/or of 32-bit values stay 32-bit; a left shift can carry
                            // past bit 31, so it re-masks.
                            let out = if w32 && matches!(op, Op::Shl { .. }) {
                                wmask32(&mut cells, WSrc { mode: 2, reference: k as u16, konst: 0 })
                            } else {
                                WSrc { mode: 2, reference: k as u16, konst: 0 }
                            };
                            reg.insert(*dst, out);
                            float_regs.remove(dst);
                            bool_regs.remove(dst);
                            float_colls.remove(dst);
                            if w32 { w32_regs.insert(*dst); } else { w32_regs.remove(dst); }
                            record(&mut wo, *dst);
                        }
                        // ── collections on the shared heap. The heap is GLOBAL across call frames (matching the
                        // VM's single heap), so an array built before a recursive call and mutated IN PLACE
                        // through it (quicksort/heap_sort — one array threaded by pointer) runs. Same `[len,
                        // e1..eN]` layout (1-based) as the straight-line fabric. Allocating a FRESH collection
                        // inside the recursion (mergesort's left/right/result) hits the multi-grow-Seq heap
                        // limit — that stays the follow-on.
                        Op::Length { dst, collection } => {
                            let ptr = get(&mut reg, &mut inputs, *collection);
                            let k = cells.len();
                            cells.push(WCell { kind: 3, code: 0, ops: [ptr, z, z, z] });
                            reg.insert(*dst, val(k));
                            float_regs.remove(dst); // a length is an Int
                            text_regs.remove(dst);
                            float_colls.remove(dst);
                            w32_regs.remove(dst);
                            w32_colls.remove(dst);
                            record(&mut wo, *dst);
                        }
                        Op::Index { dst, collection, index }
                        | Op::IndexUnchecked { dst, collection, index } => {
                            let ptr = get(&mut reg, &mut inputs, *collection);
                            let idx = get(&mut reg, &mut inputs, *index);
                            // `item i of TEXT` → a fresh 1-char Text (the VM returns a 1-char string, compared by
                            // text-equality), NOT the raw byte.
                            if text_regs.contains(collection) {
                                let k = cells.len();
                                cells.push(WCell { kind: 8, code: 0, ops: [ptr, idx, z, z] });
                                reg.insert(*dst, val(k));
                                text_regs.insert(*dst);
                                float_regs.remove(dst);
                                bool_regs.remove(dst);
                                w32_regs.remove(dst);
                                record(&mut wo, *dst);
                                continue;
                            }
                            let ka = cells.len();
                            cells.push(WCell { kind: 0, code: 0, ops: [ptr, idx, z, z] });
                            let kr = cells.len();
                            cells.push(WCell { kind: 3, code: 0, ops: [val(ka), z, z, z] });
                            reg.insert(*dst, val(kr));
                            // `item i of v` off a float / word32 Seq carries that element type.
                            if float_colls.contains(collection) { float_regs.insert(*dst); } else { float_regs.remove(dst); }
                            if w32_colls.contains(collection) { w32_regs.insert(*dst); } else { w32_regs.remove(dst); }
                            float_colls.remove(dst);
                            w32_colls.remove(dst);
                            record(&mut wo, *dst);
                        }
                        Op::SetIndex { collection, index, value }
                        | Op::SetIndexUnchecked { collection, index, value } => {
                            if float_regs.contains(value) { float_colls.insert(*collection); }
                            if w32_regs.contains(value) { w32_colls.insert(*collection); }
                            let ptr = get(&mut reg, &mut inputs, *collection);
                            let idx = get(&mut reg, &mut inputs, *index);
                            let v = get(&mut reg, &mut inputs, *value);
                            let ka = cells.len();
                            cells.push(WCell { kind: 0, code: 0, ops: [ptr, idx, z, z] });
                            cells.push(WCell { kind: 4, code: 0, ops: [val(ka), v, z, z] });
                        }
                        // `args()`: argv, program name first, as on every tier (`semantics::argv_with_program_name`).
                        // The engine has no host argv, so the list is the program name alone — `[1, name]`, the name
                        // a heap Text laid out as a Text literal is, in a collection block with its growth reserve.
                        Op::Args { dst } => {
                            let bytes = crate::semantics::PROGRAM_NAME.as_bytes();
                            let kt = cells.len();
                            cells.push(WCell { kind: 5, code: 0, ops: [con(bytes.len() as i64 + 1), z, z, z] });
                            let name = val(kt);
                            cells.push(WCell { kind: 4, code: 0, ops: [name, con(bytes.len() as i64), z, z] });
                            for (i, b) in bytes.iter().enumerate() {
                                let kadd = cells.len();
                                cells.push(WCell { kind: 0, code: 0, ops: [name, con(i as i64 + 1), z, z] });
                                cells.push(WCell { kind: 4, code: 0, ops: [val(kadd), con(*b as i64), z, z] });
                            }
                            let ka = cells.len();
                            cells.push(WCell {
                                kind: 5,
                                code: 0,
                                ops: [con(1 + crate::codegen_sva::fabric_rtl::SEQ_RESERVE), z, z, z],
                            });
                            let ptr = val(ka);
                            cells.push(WCell { kind: 4, code: 0, ops: [ptr, con(1), z, z] });
                            let kadd = cells.len();
                            cells.push(WCell { kind: 0, code: 0, ops: [ptr, con(1), z, z] });
                            cells.push(WCell { kind: 4, code: 0, ops: [val(kadd), name, z, z] });
                            reg.insert(*dst, ptr);
                            text_regs.remove(dst);
                            float_regs.remove(dst);
                            float_colls.remove(dst);
                            bool_regs.remove(dst);
                            w32_regs.remove(dst);
                            record(&mut wo, *dst);
                        }
                        Op::NewEmptyList { dst }
                        | Op::NewEmptyListWithCapacity { dst, .. }
                        | Op::NewEmptyListFixed { dst, .. }
                        | Op::NewEmptyListI32 { dst }
                        | Op::NewEmptySet { dst }
                        | Op::NewEmptyMap { dst }
                        | Op::NewEmptyMapWithCapacity { dst, .. } => {
                            // Reserve the collection's own capacity block (see `SEQ_RESERVE`), so an array built
                            // and grown across recursive frames doesn't collide with a sibling collection.
                            let ka = cells.len();
                            cells.push(WCell {
                                kind: 5,
                                code: 0,
                                ops: [con(1 + crate::codegen_sva::fabric_rtl::SEQ_RESERVE), z, z, z],
                            });
                            let ptr = val(ka);
                            cells.push(WCell { kind: 4, code: 0, ops: [ptr, con(0), z, z] });
                            reg.insert(*dst, ptr);
                            record(&mut wo, *dst);
                        }
                        Op::NewList { dst, start, count } | Op::NewTuple { dst, start, count } => {
                            let n = *count;
                            if (0..n).any(|j| float_regs.contains(&(*start + j))) {
                                float_colls.insert(*dst);
                            } else {
                                float_colls.remove(dst);
                            }
                            if (0..n).any(|j| w32_regs.contains(&(*start + j))) {
                                w32_colls.insert(*dst);
                            } else {
                                w32_colls.remove(dst);
                            }
                            w32_regs.remove(dst);
                            let ka = cells.len();
                            cells.push(WCell { kind: 5, code: 0, ops: [con(n as i64 + 1), z, z, z] });
                            let ptr = val(ka);
                            cells.push(WCell { kind: 4, code: 0, ops: [ptr, con(n as i64), z, z] });
                            for j in 0..n {
                                let elem = get(&mut reg, &mut inputs, *start + j);
                                let kadd = cells.len();
                                cells.push(WCell { kind: 0, code: 0, ops: [ptr, con(j as i64 + 1), z, z] });
                                cells.push(WCell { kind: 4, code: 0, ops: [val(kadd), elem, z, z] });
                            }
                            reg.insert(*dst, ptr);
                            float_regs.remove(dst);
                            bool_regs.remove(dst);
                            record(&mut wo, *dst);
                        }
                        Op::DestructureTuple { src, start, count } => {
                            // The inverse of `NewTuple`: spread a tuple `heap[ptr] = len; heap[ptr+1..=ptr+len]`
                            // into consecutive result registers `start..start+count` — element j is
                            // `heap[ptr + j + 1]`. The source surface is `Repeat for (a, b) in list`.
                            let ptr = get(&mut reg, &mut inputs, *src);
                            for j in 0..*count {
                                let kadd = cells.len();
                                cells.push(WCell { kind: 0, code: 0, ops: [ptr, con(j as i64 + 1), z, z] });
                                let kr = cells.len();
                                cells.push(WCell { kind: 3, code: 0, ops: [val(kadd), z, z, z] });
                                reg.insert(*start + j, val(kr));
                                float_regs.remove(&(*start + j));
                                w32_regs.remove(&(*start + j));
                                record(&mut wo, *start + j);
                            }
                        }
                        Op::ListPush { list, value } => {
                            if float_regs.contains(value) { float_colls.insert(*list); }
                            if w32_regs.contains(value) { w32_colls.insert(*list); }
                            let ptr = get(&mut reg, &mut inputs, *list);
                            let v = get(&mut reg, &mut inputs, *value);
                            let klen = cells.len();
                            cells.push(WCell { kind: 3, code: 0, ops: [ptr, z, z, z] });
                            let len = val(klen);
                            let kp1 = cells.len();
                            cells.push(WCell { kind: 0, code: 0, ops: [ptr, con(1), z, z] });
                            let kaddr = cells.len();
                            cells.push(WCell { kind: 0, code: 0, ops: [val(kp1), len, z, z] });
                            cells.push(WCell { kind: 4, code: 0, ops: [val(kaddr), v, z, z] });
                            let knl = cells.len();
                            cells.push(WCell { kind: 0, code: 0, ops: [len, con(1), z, z] });
                            cells.push(WCell { kind: 4, code: 0, ops: [ptr, val(knl), z, z] });
                        }
                        Op::ListPop { list, dst } => {
                            let ptr = get(&mut reg, &mut inputs, *list);
                            let klen = cells.len();
                            cells.push(WCell { kind: 3, code: 0, ops: [ptr, z, z, z] });
                            let len = val(klen);
                            let kaddr = cells.len();
                            cells.push(WCell { kind: 0, code: 0, ops: [ptr, len, z, z] });
                            let kr = cells.len();
                            cells.push(WCell { kind: 3, code: 0, ops: [val(kaddr), z, z, z] });
                            let kdec = cells.len();
                            cells.push(WCell { kind: 0, code: 1, ops: [len, con(1), z, z] });
                            cells.push(WCell { kind: 4, code: 0, ops: [ptr, val(kdec), z, z] });
                            reg.insert(*dst, val(kr));
                            if float_colls.contains(list) { float_regs.insert(*dst); } else { float_regs.remove(dst); }
                            if w32_colls.contains(list) { w32_regs.insert(*dst); } else { w32_regs.remove(dst); }
                            float_colls.remove(dst);
                            w32_colls.remove(dst);
                            record(&mut wo, *dst);
                        }
                        // `floor`/`sqrt` over a float register run on the float functional units (the float→int
                        // bridge for `floor`, float→float for `sqrt`) — what spectral_norm/nbody-in-recursion need.
                        Op::CallBuiltin { dst, builtin, args_start, .. } => {
                            use crate::semantics::builtins::BuiltinId;
                            let a0 = get(&mut reg, &mut inputs, *args_start);
                            let is_f = float_regs.contains(args_start);
                            let mut mk_w32 = false; // does this builtin PRODUCE a word32?
                            match builtin {
                                BuiltinId::Sqrt if is_f => {
                                    let k = cells.len();
                                    cells.push(WCell { kind: 2, code: func_code(FuncUnit::FSqrtF), ops: [a0, z, z, z] });
                                    reg.insert(*dst, val(k));
                                    float_regs.insert(*dst);
                                }
                                BuiltinId::Floor if is_f => {
                                    let k = cells.len();
                                    cells.push(WCell { kind: 2, code: func_code(FuncUnit::FFloorF), ops: [a0, z, z, z] });
                                    reg.insert(*dst, val(k));
                                    float_regs.remove(dst);
                                    bool_regs.remove(dst);
                                }
                                BuiltinId::Floor => {
                                    reg.insert(*dst, a0);
                                    float_regs.remove(dst);
                                    bool_regs.remove(dst);
                                }
                                // WORD32 construction: `word32(n) = n & 0xFFFFFFFF`.
                                BuiltinId::Word32 => {
                                    let m = wmask32(&mut cells, a0);
                                    reg.insert(*dst, m);
                                    mk_w32 = true;
                                }
                                // `rotr(w, n) = ((w >> n) | (w << (32 - n))) & 0xFFFFFFFF` (logical right-rotate).
                                BuiltinId::Rotr => {
                                    let n = get(&mut reg, &mut inputs, *args_start + 1);
                                    let ksh1 = cells.len();
                                    cells.push(WCell { kind: 0, code: 7, ops: [a0, n, z, z] }); // w >> n
                                    let kcomp = cells.len();
                                    cells.push(WCell { kind: 0, code: 1, ops: [con(32), n, z, z] }); // 32 - n
                                    let ksh2 = cells.len();
                                    cells.push(WCell { kind: 0, code: 6, ops: [a0, val(kcomp), z, z] }); // w << (32-n)
                                    let kor = cells.len();
                                    cells.push(WCell { kind: 0, code: 4, ops: [val(ksh1), val(ksh2), z, z] }); // |
                                    let m = wmask32(&mut cells, val(kor));
                                    reg.insert(*dst, m);
                                    mk_w32 = true;
                                }
                                // `word32Shr(w, n) = w >> n` (w is a 32-bit non-negative value → logical == signed).
                                BuiltinId::Word32Shr => {
                                    let n = get(&mut reg, &mut inputs, *args_start + 1);
                                    let k = cells.len();
                                    cells.push(WCell { kind: 0, code: 7, ops: [a0, n, z, z] });
                                    reg.insert(*dst, val(k));
                                    mk_w32 = true;
                                }
                                // `intOfWord32(w) = w` — the unsigned value as an Int (identity; `dst` is an Int).
                                BuiltinId::IntOfWord32 => {
                                    reg.insert(*dst, a0);
                                }
                                // WORD32 bitwise: `word_and`/`word_or` stay 32-bit (no re-mask); `word_not = ~a &
                                // 0xFFFFFFFF` (the 32-bit complement).
                                BuiltinId::Wand => {
                                    let b1 = get(&mut reg, &mut inputs, *args_start + 1);
                                    let k = cells.len();
                                    cells.push(WCell { kind: 0, code: 3, ops: [a0, b1, z, z] });
                                    reg.insert(*dst, val(k));
                                    mk_w32 = true;
                                }
                                BuiltinId::Wor => {
                                    let b1 = get(&mut reg, &mut inputs, *args_start + 1);
                                    let k = cells.len();
                                    cells.push(WCell { kind: 0, code: 4, ops: [a0, b1, z, z] });
                                    reg.insert(*dst, val(k));
                                    mk_w32 = true;
                                }
                                BuiltinId::Wnot => {
                                    let kx = cells.len();
                                    cells.push(WCell { kind: 0, code: 5, ops: [a0, con(-1), z, z] }); // a ^ -1
                                    let m = wmask32(&mut cells, val(kx));
                                    reg.insert(*dst, m);
                                    mk_w32 = true;
                                }
                                // FLOAT abs: clear the IEEE-754 sign bit (`a & 0x7FFF_FFFF_FFFF_FFFF`).
                                BuiltinId::Abs if is_f => {
                                    let k = cells.len();
                                    cells.push(WCell { kind: 0, code: 3, ops: [a0, con(0x7FFF_FFFF_FFFF_FFFF), z, z] });
                                    reg.insert(*dst, val(k));
                                    float_regs.insert(*dst);
                                }
                                // INTEGER abs: `a < 0 ? (0 - a) : a` — a negate cell feeding a select.
                                BuiltinId::Abs => {
                                    let kneg = cells.len();
                                    cells.push(WCell { kind: 0, code: 1, ops: [con(0), a0, z, z] }); // 0 - a
                                    let ksel = cells.len();
                                    cells.push(WCell { kind: 1, code: 0, ops: [a0, con(0), val(kneg), a0] }); // a<0 ? -a : a
                                    reg.insert(*dst, val(ksel));
                                    float_regs.remove(dst);
                                    bool_regs.remove(dst);
                                }
                                other => panic!(
                                    "calls fabric: builtin `{other:?}` unsupported (float/word32 lanes wired: \
                                     sqrt/floor/word32/rotr/word32Shr/intOfWord32)"
                                ),
                            }
                            float_colls.remove(dst);
                            w32_colls.remove(dst);
                            if mk_w32 { w32_regs.insert(*dst); } else { w32_regs.remove(dst); }
                            record(&mut wo, *dst);
                        }
                        // ── native range / for-each iterators (the `Repeat for …` loops). NewRange builds a heap
                        // range object `[cursor, limit]`; IterPrepare pushes the active iterator (a Seq iterable
                        // gets a fresh `[index, coll_ptr]` object); IterNext (the terminator, above) steps it.
                        Op::NewRange { dst, start, end } => {
                            let s = get(&mut reg, &mut inputs, *start);
                            let e = get(&mut reg, &mut inputs, *end);
                            let ka = cells.len();
                            cells.push(WCell { kind: 5, code: 0, ops: [con(2), z, z, z] }); // alloc [cursor, limit]
                            let ptr = val(ka);
                            cells.push(WCell { kind: 4, code: 0, ops: [ptr, s, z, z] }); // heap[ptr] = start
                            let kp1 = cells.len();
                            cells.push(WCell { kind: 0, code: 0, ops: [ptr, con(1), z, z] });
                            cells.push(WCell { kind: 4, code: 0, ops: [val(kp1), e, z, z] }); // heap[ptr+1] = end
                            reg.insert(*dst, ptr);
                            range_regs.insert(*dst);
                            record(&mut wo, *dst);
                        }
                        Op::IterPrepare { iterable } => {
                            if range_regs.contains(iterable) {
                                iter_stack.push((false, *iterable)); // the NewRange dst IS the range object
                            } else {
                                // A Seq iterable: build a fresh iterator object `[index=0, coll_ptr]`.
                                let coll = get(&mut reg, &mut inputs, *iterable);
                                let ka = cells.len();
                                cells.push(WCell { kind: 5, code: 0, ops: [con(2), z, z, z] });
                                let ptr = val(ka);
                                cells.push(WCell { kind: 4, code: 0, ops: [ptr, con(0), z, z] }); // index = 0
                                let kp1 = cells.len();
                                cells.push(WCell { kind: 0, code: 0, ops: [ptr, con(1), z, z] });
                                cells.push(WCell { kind: 4, code: 0, ops: [val(kp1), coll, z, z] }); // coll_ptr
                                let ir = fresh_reg;
                                fresh_reg += 1;
                                reg.insert(ir, ptr);
                                record(&mut wo, ir);
                                iter_stack.push((true, ir));
                            }
                        }
                        Op::IterPop => {
                            iter_stack.pop();
                        }
                        // `Show v` — a SHOW cell (kind 10): the value flows on as the cell's result, and the engine
                        // records/streams it with the type tag the tracking sets assign — the exact per-register
                        // type at this point, so a reused register shows as what it holds now.
                        Op::Show { src } => {
                            let s = get(&mut reg, &mut inputs, *src);
                            let tag = if text_regs.contains(src) {
                                TAG_TEXT
                            } else if float_regs.contains(src) {
                                TAG_FLOAT
                            } else if w32_regs.contains(src) {
                                TAG_WORD32
                            } else if bool_regs.contains(src) {
                                TAG_BOOL
                            } else {
                                TAG_INT
                            };
                            cells.push(WCell { kind: 10, code: tag, ops: [s, z, z, z] });
                        }
                        other => panic!(
                            "calls fabric: op `{}` unsupported (integer-recursion scope — collections-in-\
                             recursion is the follow-on)",
                            other.name()
                        ),
                    }
                }
                if let Some(t) = &term_op {
                    forget_overwritten!(t);
                }
                let term = match term_op {
                    Some(Op::Return { src }) => WTerm::Return(get(&mut reg, &mut inputs, src)),
                    Some(Op::ReturnNothing) => WTerm::Return(WSrc {
                        mode: 1,
                        reference: 0,
                        konst: 0,
                    }),
                    // `Halt` ends Main: a top-level return of 0 (the frame stack is empty, so the engine finishes).
                    Some(Op::Halt) => WTerm::Return(WSrc {
                        mode: 1,
                        reference: 0,
                        konst: 0,
                    }),
                    Some(Op::Jump { target }) => WTerm::Jump(block_of(li, target)),
                    Some(Op::JumpIfFalse { cond, target }) => {
                        let c = get(&mut reg, &mut inputs, cond);
                        WTerm::Branch {
                            cond: c,
                            then_blk: block_of(li, bend),
                            else_blk: block_of(li, target),
                        }
                    }
                    Some(Op::JumpIfTrue { cond, target }) => {
                        let c = get(&mut reg, &mut inputs, cond);
                        WTerm::Branch {
                            cond: c,
                            then_blk: block_of(li, target),
                            else_blk: block_of(li, bend),
                        }
                    }
                    Some(Op::Call {
                        dst,
                        func,
                        args_start,
                        arg_count,
                    }) => {
                        let args: Vec<WSrc> = (0..arg_count)
                            .map(|i| get(&mut reg, &mut inputs, args_start + i))
                            .collect();
                        // The return value's type flows from the callee's DECLARED return type: mark `dst` float /
                        // float-collection so the continuation block reads it through the right lane (e.g.
                        // `Set v to mulAtav(...)` returning a `Seq of Float`, or `aVal(..)` returning a `Float`).
                        match &program.functions[func as usize].return_type {
                            Some(BoundaryType::Float) => {
                                float_regs.insert(dst);
                                float_colls.remove(&dst);
                                w32_regs.remove(&dst);
                                w32_colls.remove(&dst);
                                text_regs.remove(&dst);
                            }
                            Some(BoundaryType::Text) => {
                                text_regs.insert(dst);
                                float_regs.remove(&dst);
                                bool_regs.remove(&dst);
                                float_colls.remove(&dst);
                                w32_regs.remove(&dst);
                                w32_colls.remove(&dst);
                            }
                            Some(BoundaryType::Word32) => {
                                w32_regs.insert(dst);
                                w32_colls.remove(&dst);
                                float_regs.remove(&dst);
                                bool_regs.remove(&dst);
                                float_colls.remove(&dst);
                                text_regs.remove(&dst);
                            }
                            Some(BoundaryType::Seq(inner))
                                if matches!(**inner, BoundaryType::Float) =>
                            {
                                float_colls.insert(dst);
                                float_regs.remove(&dst);
                                bool_regs.remove(&dst);
                                w32_regs.remove(&dst);
                                w32_colls.remove(&dst);
                                text_regs.remove(&dst);
                            }
                            Some(BoundaryType::Seq(inner))
                                if matches!(**inner, BoundaryType::Word32) =>
                            {
                                w32_colls.insert(dst);
                                w32_regs.remove(&dst);
                                float_regs.remove(&dst);
                                bool_regs.remove(&dst);
                                float_colls.remove(&dst);
                                text_regs.remove(&dst);
                            }
                            _ => {
                                float_regs.remove(&dst);
                                bool_regs.remove(&dst);
                                float_colls.remove(&dst);
                                w32_regs.remove(&dst);
                                w32_colls.remove(&dst);
                                text_regs.remove(&dst);
                            }
                        }
                        WTerm::Call {
                            callee_block: fn_entry[&(func as usize)],
                            args,
                            dst_reg: dst,
                            ret_block: block_of(li, bend),
                        }
                    }
                    // `IterNext { dst, exit }` — the loop step of a `Repeat for …` range/for-each. The active
                    // iterator (top of the lowering-time stack) is a heap object: RANGE `[cursor, limit]` or SEQ
                    // `[index, coll_ptr]`. Read its state, produce `dst`, advance the cursor in the heap, then
                    // branch: has-next → the body block (fall-through); exhausted → `exit`. The unconditional
                    // `dst`/advance are harmless when exhausted (the body doesn't run and the iterator is popped).
                    Some(Op::IterNext { dst, exit }) => {
                        let (is_seq, iter_reg) =
                            *iter_stack.last().expect("IterNext with no active iterator");
                        let iptr = get(&mut reg, &mut inputs, iter_reg);
                        let kcur = cells.len();
                        cells.push(WCell {
                            kind: 3,
                            code: 0,
                            ops: [iptr, z, z, z],
                        }); // cursor/index = heap[iptr]
                        let cursor = val(kcur);
                        let kp1 = cells.len();
                        cells.push(WCell {
                            kind: 0,
                            code: 0,
                            ops: [iptr, con(1), z, z],
                        }); // iptr + 1
                        let ksec = cells.len();
                        cells.push(WCell {
                            kind: 3,
                            code: 0,
                            ops: [val(kp1), z, z, z],
                        }); // limit / coll_ptr
                        let (has_next, value) = if is_seq {
                            // SEQ: len = heap[coll]; has_next = index < len; value = heap[coll + index + 1].
                            let klen = cells.len();
                            cells.push(WCell {
                                kind: 3,
                                code: 0,
                                ops: [val(ksec), z, z, z],
                            });
                            let khn = cells.len();
                            cells.push(WCell {
                                kind: 1,
                                code: 0,
                                ops: [cursor, val(klen), con(1), con(0)],
                            }); // <
                            let kidx = cells.len();
                            cells.push(WCell {
                                kind: 0,
                                code: 0,
                                ops: [val(ksec), cursor, z, z],
                            }); // coll + index
                            let kaddr = cells.len();
                            cells.push(WCell {
                                kind: 0,
                                code: 0,
                                ops: [val(kidx), con(1), z, z],
                            }); // + 1
                            let kval = cells.len();
                            cells.push(WCell {
                                kind: 3,
                                code: 0,
                                ops: [val(kaddr), z, z, z],
                            });
                            (val(khn), val(kval))
                        } else {
                            // RANGE: has_next = cursor <= limit; value = cursor.
                            let khn = cells.len();
                            cells.push(WCell {
                                kind: 1,
                                code: 2,
                                ops: [cursor, val(ksec), con(1), con(0)],
                            }); // <=
                            (val(khn), cursor)
                        };
                        reg.insert(dst, value);
                        // whichever lane the elements are, the read value inherits it (Seq iterator over a float/
                        // word32 collection). Cleared here; the collection-typed tracking is a follow-on for typed
                        // for-each — the range case (the common one) is an Int.
                        float_regs.remove(&dst);
                        bool_regs.remove(&dst);
                        w32_regs.remove(&dst);
                        record(&mut wo, dst);
                        let kinc = cells.len();
                        cells.push(WCell {
                            kind: 0,
                            code: 0,
                            ops: [cursor, con(1), z, z],
                        }); // cursor + 1
                        cells.push(WCell {
                            kind: 4,
                            code: 0,
                            ops: [iptr, val(kinc), z, z],
                        }); // heap[iptr] = cursor+1
                        WTerm::Branch {
                            cond: has_next,
                            then_blk: block_of(li, bend),
                            else_blk: block_of(li, exit),
                        }
                    }
                    None => WTerm::Jump(block_of(li, bend)),
                    _ => unreachable!(),
                };
                let writes: Vec<(u16, WSrc)> = wo.iter().map(|r| (*r, reg[r])).collect();
                blocks.push(WBlock {
                    livein: inputs,
                    cells,
                    writes,
                    term,
                });
            }
        }

        WProgConfig {
            n_params: entry_fi.map_or(0, |fi| program.functions[fi].param_count as usize),
            blocks,
        }
    })
}

/// One `Show` the engine executed: the value tag the lowering assigned in the SHOW cell's `code` (from its exact
/// per-register type tracking, so a reused register is shown as what it holds now) and the raw 64-bit value. A
/// Text needs the heap to render — see [`render_show`]. This is exactly the record the RTL streams over the byte
/// channel ([`REC_SHOW`](super::chip_protocol::REC_SHOW), tag, payload) the moment the cell runs, so the tape the
/// model builds is the byte stream the host reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WShow {
    pub tag: u8,
    pub value: i64,
}

/// Everything one engine run produces: the entry's return value, the block-step count, the final heap, the peak
/// call-frame depth, and the output tape (every `Show`, in program order).
#[derive(Clone, Debug)]
pub struct WRun {
    pub result: i64,
    pub steps: u64,
    pub heap: Vec<i64>,
    pub peak_depth: usize,
    pub shows: Vec<WShow>,
    pub work: WWork,
}

/// What a run did, in the units the engine's cycle count is made of: every executed cell's cycles with its
/// data-dependent part priced from the data (a divide's 64 steps, a power's multiplies, a text's or a copy's
/// length), and the blocks, register writes, calls and returns around them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WWork {
    pub cell_cycles: u64,
    pub blocks: u64,
    pub writes: u64,
    pub calls: u64,
    pub returns: u64,
    /// Bytes of Show records streamed, which the engine's run-cycle count leaves out.
    pub shown_bytes: u64,
}

impl WWork {
    /// An upper bound on the engine's counted run cycles for `nreg` registers and `maxarg` call arguments,
    /// under every memory backend: each block's fetch and terminator settle, each write's read-before-write
    /// pass, and each call's frame save and argument gather and each return's restore, which walk every
    /// register of the frame.
    pub fn rtl_cycle_bound(&self, nreg: usize, maxarg: usize) -> u64 {
        let (nreg, maxarg) = (nreg as u64, maxarg as u64);
        self.cell_cycles
            + 16 * self.blocks
            + 8 * self.writes
            + (3 * nreg + 3 * maxarg + 16) * self.calls
            + (3 * nreg + 16) * self.returns
    }
}

/// Cycles one cell may take outside its data-dependent part: its operands fetched one at a time through the
/// registered config and value ports, the compute, and the settle before the next cell.
const CELL_CYCLES: u64 = 16;
/// Cycles the sequential integer divider takes: one per dividend bit, and the sign fix.
const DIVIDE_CYCLES: u64 = 70;
/// Cycles a multiply spends on the multiplier unit after its issue: the tree's three stages and the write
/// (ST_MU..ST_MUW). A power's step adds the loop state that issues it.
const MUL_CYCLES: u64 = 4;
/// Cycles the widest floating-point unit takes (the iterative divide and square root).
const FLOAT_CYCLES: u64 = 256;
/// Cycles the decimal conversion of one Int takes: a double-dabble step per bit, then a byte per digit.
const DECIMAL_CYCLES: u64 = 96;

/// Render one shown value the way the VM prints it: signed Int, `fmt_f64` Float, the Text's bytes, unsigned
/// Word32, `true`/`false`.
pub fn render_show(s: WShow, heap: &[i64]) -> String {
    match s.tag {
        TAG_FLOAT => logicaffeine_data::fmt::fmt_f64(f64::from_bits(s.value as u64)),
        TAG_TEXT => {
            let ptr = s.value.max(0) as usize;
            let len = heap.get(ptr).copied().unwrap_or(0).max(0) as usize;
            let bytes: Vec<u8> = (0..len)
                .map(|i| heap.get(ptr + 1 + i).copied().unwrap_or(0) as u8)
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        }
        TAG_WORD32 => (s.value as u64 & 0xFFFF_FFFF).to_string(),
        TAG_BOOL => (s.value != 0).to_string(),
        _ => s.value.to_string(),
    }
}

/// A whole program's OUTPUT on the engine model — every `Show` rendered, one line each — the form that compares
/// directly against `vm_outcome(src).output`.
pub fn program_output(cfg: &WProgConfig, params: &[i64]) -> Vec<String> {
    let run = eval_wprog_config_run(cfg, params, &[]);
    run.shows
        .iter()
        .map(|s| render_show(*s, &run.heap))
        .collect()
}

/// Interpret the flat config exactly as the RTL will: a register file seeded with `params`, a block-walk over
/// the terminator graph, each block computing its cells (Arith/Select) over its live-in registers + prior
/// cell values, committing its register writes, and branching on its terminator. Proven bit-identical to
/// `run_program_cfg_on_grid` (≡ the VM) — the reference the whole-program RTL is checked against.
pub fn eval_wprog_config(cfg: &WProgConfig, params: &[i64]) -> i64 {
    eval_wprog_config_counted(cfg, params).0
}

/// Like [`eval_wprog_config`] but also returns the number of BLOCK-STEPS executed. Each block is one spatial
/// tile group evaluated in a single fabric cycle, so this count IS the whole-program fabric's cycle count.
/// ONE authoritative evaluator shared by the result path and any cycle measurement — no drifting hand-copy.
pub fn eval_wprog_config_counted(cfg: &WProgConfig, params: &[i64]) -> (i64, u64) {
    let (r, c, _heap) = eval_wprog_config_counted_heap(cfg, params);
    (r, c)
}

/// As [`eval_wprog_config_counted`] but also returns the final HEAP, so a caller can render a Text result (a
/// char-list `[len, byte0..]` at the returned pointer) back to a string for verification.
pub fn eval_wprog_config_counted_heap(cfg: &WProgConfig, params: &[i64]) -> (i64, u64, Vec<i64>) {
    eval_wprog_config_counted_heap_preloaded(cfg, params, &[])
}

/// As [`eval_wprog_config_counted_heap`] but the heap starts PRE-LOADED with `heap_init` — the DATA-segment
/// model of a chip that receives its weights streamed into memory separately from the kernel. `loaded()`
/// lowers to heap pointer 0, so `item k of loaded()` reads `heap_init[k]` (`heap_init[0]` = the blob length,
/// `heap_init[1..]` = the weights). The bump allocator is simply `heap.len()`, so runtime collections
/// allocate PAST the pre-loaded region automatically — no separate `hp` seeding needed. This is what lets the
/// streamed transformer run ≡VM on the engine without Push-building its ~3,136 weights into the config.
pub fn eval_wprog_config_counted_heap_preloaded(
    cfg: &WProgConfig,
    params: &[i64],
    heap_init: &[i64],
) -> (i64, u64, Vec<i64>) {
    let (result, steps, heap, _peak_depth) = eval_wprog_config_probed(cfg, params, heap_init);
    (result, steps, heap)
}

/// As [`eval_wprog_config_counted_heap_preloaded`] but also returns the PEAK call-frame depth the run reached.
///
/// Every other config dimension (blocks, cells, registers, live-ins, konsts) is a STATIC property the fit-check
/// reads straight off the config. Frame depth is not: it depends on the arguments, so the only way to size the
/// engine's `DEPTH` against a program is to run the program. Without this the sizing oracle is blind in exactly
/// the dimension the hardware corrupts silently — the engine advances `sp` past `DEPTH` with no bound check, so
/// an over-deep call returns a plausible wrong number rather than failing.
pub fn eval_wprog_config_probed(
    cfg: &WProgConfig,
    params: &[i64],
    heap_init: &[i64],
) -> (i64, u64, Vec<i64>, usize) {
    let r = eval_wprog_config_run(cfg, params, heap_init);
    (r.result, r.steps, r.heap, r.peak_depth)
}

/// THE golden interpreter of the flat config — the one body every result / step / heap / depth / output-tape
/// query is a view of. Returns everything a run produces ([`WRun`]), including the `Show` tape.
pub fn eval_wprog_config_run(cfg: &WProgConfig, params: &[i64], heap_init: &[i64]) -> WRun {
    use std::collections::HashMap;
    let mut shows: Vec<WShow> = Vec::new();
    let mut file: HashMap<u16, i64> = (0..cfg.n_params).map(|p| (p as u16, params[p])).collect();
    // ONE heap threaded across every block (the state plane) — collections persist across reconfiguration.
    let mut heap: Vec<i64> = heap_init.to_vec();
    let mut blk = 0usize;
    // The CALL FRAME STACK: each frame is (return block, destination register in the caller, the caller's
    // saved register file). The heap is GLOBAL (not per-frame), matching the VM's single heap.
    let mut stack: Vec<(usize, u16, HashMap<u16, i64>)> = Vec::new();
    // The deepest this run drives the frame stack — the engine's `DEPTH` cap must cover it.
    let mut peak_depth = 0usize;

    let read = |src: &WSrc, li: &[i64], vals: &[i64]| -> i64 {
        match src.mode {
            0 => li[src.reference as usize],
            1 => src.konst,
            _ => vals[src.reference as usize],
        }
    };

    let mut steps = 0u64;
    let mut work = WWork::default();
    for _ in 0..10_000_000 {
        steps += 1;
        let b = &cfg.blocks[blk];
        work.blocks += 1;
        work.writes += b.writes.len() as u64;
        let li: Vec<i64> = b
            .livein
            .iter()
            .map(|r| *file.get(r).unwrap_or(&0))
            .collect();
        let mut vals: Vec<i64> = Vec::with_capacity(b.cells.len());
        for c in &b.cells {
            work.cell_cycles += CELL_CYCLES;
            let v = match c.kind {
                0 => {
                    let a = read(&c.ops[0], &li, &vals);
                    let b2 = read(&c.ops[1], &li, &vals);
                    if c.code == 2 {
                        work.cell_cycles += MUL_CYCLES;
                    }
                    match c.code {
                        0 => a.wrapping_add(b2),
                        1 => a.wrapping_sub(b2),
                        2 => a.wrapping_mul(b2),
                        3 => a & b2,
                        4 => a | b2,
                        5 => a ^ b2,
                        6 => ((a as u64) << (b2 as u64 & 63)) as i64,
                        _ => a >> (b2 as u64 & 63),
                    }
                }
                1 => {
                    let s0 = read(&c.ops[0], &li, &vals);
                    let s1 = read(&c.ops[1], &li, &vals);
                    let hit = match c.code {
                        0 => s0 < s1,
                        1 => s0 > s1,
                        2 => s0 <= s1,
                        3 => s0 >= s1,
                        4 => s0 == s1,
                        _ => s0 != s1,
                    };
                    if hit {
                        read(&c.ops[2], &li, &vals)
                    } else {
                        read(&c.ops[3], &li, &vals)
                    }
                }
                3 => {
                    let a = read(&c.ops[0], &li, &vals);
                    heap.get(a.max(0) as usize).copied().unwrap_or(0)
                }
                4 => {
                    let a = read(&c.ops[0], &li, &vals);
                    let bv = read(&c.ops[1], &li, &vals);
                    let ai = a.max(0) as usize;
                    if ai >= heap.len() {
                        heap.resize(ai + 1, 0);
                    }
                    heap[ai] = bv;
                    bv
                }
                5 => {
                    let a = read(&c.ops[0], &li, &vals);
                    let ptr = heap.len() as i64;
                    heap.resize(heap.len() + a.max(0) as usize, 0);
                    ptr
                }
                6 => {
                    // DEEP-COPY a collection (value semantics / COW): read `len = heap[src]`, allocate a fresh
                    // block `[len, e0..e_{len-1}]` plus reserve headroom, copy the elements, return the new ptr.
                    let src = read(&c.ops[0], &li, &vals).max(0) as usize;
                    let len = heap.get(src).copied().unwrap_or(0).max(0) as usize;
                    work.cell_cycles += 3 * (len as u64 + 1);
                    let newptr = heap.len() as i64;
                    let reserve = crate::codegen_sva::fabric_rtl::seq_reserve() as usize;
                    heap.resize(heap.len() + 1 + len + reserve, 0);
                    for j in 0..=len {
                        let v = heap.get(src + j).copied().unwrap_or(0);
                        let dst_idx = newptr as usize + j;
                        heap[dst_idx] = v;
                    }
                    newptr
                }
                7 => {
                    // TEXT CONCAT: build a fresh char-list `[len, bytes…]` = chars(a) ++ chars(b). Each operand is
                    // either TEXT (its char-list, `code` bit set) or an INT (rendered to its decimal bytes — the
                    // VM's `value + text` coercion). Immutable, so no reserve headroom.
                    let mut chars: Vec<i64> = Vec::new();
                    for (slot, bit) in [(0usize, 1u8), (1usize, 2u8)] {
                        let v = read(&c.ops[slot], &li, &vals);
                        if c.code & bit != 0 {
                            let p = v.max(0) as usize;
                            let ln = heap.get(p).copied().unwrap_or(0).max(0) as usize;
                            for j in 0..ln {
                                chars.push(heap.get(p + 1 + j).copied().unwrap_or(0));
                            }
                        } else {
                            for byte in v.to_string().into_bytes() {
                                chars.push(byte as i64);
                            }
                        }
                    }
                    work.cell_cycles += 3 * chars.len() as u64 + 2 * DECIMAL_CYCLES;
                    let ptr = heap.len() as i64;
                    heap.push(chars.len() as i64);
                    heap.extend(chars);
                    ptr
                }
                8 => {
                    // TEXT CHAR-AT: `item i of text` → a fresh 1-char Text `[1, byte]` (the VM returns a 1-char
                    // string, so it compares by text-equality). `idx` is 1-based over the char-list body.
                    let p = read(&c.ops[0], &li, &vals).max(0) as usize;
                    let idx = read(&c.ops[1], &li, &vals).max(0) as usize;
                    let ch = heap.get(p + idx).copied().unwrap_or(0);
                    let ptr = heap.len() as i64;
                    heap.push(1);
                    heap.push(ch);
                    ptr
                }
                9 => {
                    // TEXT EQUALITY: compare two char-lists byte-for-byte → 0/1 (`code` 0 = eq, 1 = ne).
                    let a = read(&c.ops[0], &li, &vals).max(0) as usize;
                    let b = read(&c.ops[1], &li, &vals).max(0) as usize;
                    let la = heap.get(a).copied().unwrap_or(0);
                    let lb = heap.get(b).copied().unwrap_or(0);
                    work.cell_cycles += 6 * la.max(0) as u64 + 8;
                    let eq = la == lb
                        && (0..la as usize).all(|j| heap.get(a + 1 + j) == heap.get(b + 1 + j));
                    i64::from(if c.code == 0 { eq } else { !eq })
                }
                2 => {
                    // Functional unit (division / mod / floor-div / power / magic-high-mul) — the VM-exact
                    // closed form, reusing the same `FuncUnit::eval` the CFG model uses.
                    let a = read(&c.ops[0], &li, &vals);
                    let b2 = read(&c.ops[1], &li, &vals);
                    work.cell_cycles += match c.code {
                        0..=2 => DIVIDE_CYCLES,
                        3 => (MUL_CYCLES + 1) * b2.max(0) as u64 + 4,
                        4 | 12 => MUL_CYCLES,
                        _ => FLOAT_CYCLES,
                    };
                    func_from_code(c.code).eval(a, b2)
                }
                10 => {
                    // SHOW: record the value on the output tape with the type tag the lowering assigned; the
                    // value itself flows on as the cell's result. The RTL streams the same record over the wire.
                    let v = read(&c.ops[0], &li, &vals);
                    // The record: sync, tag, then eight value bytes — or, for Text, four length bytes and the text.
                    work.shown_bytes += 2 + if c.code == TAG_TEXT {
                        4 + heap.get(v.max(0) as usize).copied().unwrap_or(0).max(0) as u64
                    } else {
                        8
                    };
                    shows.push(WShow {
                        tag: c.code,
                        value: v,
                    });
                    v
                }
                _ => panic!("wprog eval: unknown cell kind {}", c.kind),
            };
            vals.push(v);
        }
        for (r, s) in &b.writes {
            file.insert(*r, read(s, &li, &vals));
        }
        match &b.term {
            WTerm::Jump(t) => blk = *t,
            WTerm::Branch {
                cond,
                then_blk,
                else_blk,
            } => {
                blk = if read(cond, &li, &vals) != 0 {
                    *then_blk
                } else {
                    *else_blk
                }
            }
            WTerm::Call {
                callee_block,
                args,
                dst_reg,
                ret_block,
            } => {
                // Bind the args (read in the CALLER's context) to the callee's parameter registers 0..k, push
                // a frame, and jump to the callee's entry — a fresh register file for the callee.
                let arg_vals: Vec<i64> = args.iter().map(|a| read(a, &li, &vals)).collect();
                work.calls += 1;
                stack.push((*ret_block, *dst_reg, std::mem::take(&mut file)));
                peak_depth = peak_depth.max(stack.len());
                file = arg_vals
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i as u16, *v))
                    .collect();
                blk = *callee_block;
            }
            WTerm::Return(s) => {
                let ret_val = read(s, &li, &vals);
                work.returns += 1;
                match stack.pop() {
                    None => {
                        return WRun {
                            result: ret_val,
                            steps,
                            heap,
                            peak_depth,
                            shows,
                            work,
                        }
                    } // top-level return
                    Some((ret_block, dst_reg, saved_file)) => {
                        file = saved_file;
                        file.insert(dst_reg, ret_val);
                        blk = ret_block;
                    }
                }
            }
        }
    }
    panic!("wprog: did not return within the step budget (unbounded recursion or a non-terminating loop)")
}

/// A hardware fault model injected into the golden interpreter. Running a program through the model under each
/// fault and comparing the result against a value observed on silicon localizes a synthesis/place-and-route
/// divergence to one state memory + one address bit in pure Rust: `eval` is the ground truth, so a fault whose
/// result equals the observed value characterizes the hardware defect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WFault {
    /// No fault — byte-identical to `eval_wprog_config_counted_heap_preloaded`.
    None,
    /// Register file `file` address decode is `bits` wide: logical index `r` reads/writes physical
    /// `r & ((1<<bits)-1)` (both ports share the decode, so two logical registers alias onto one cell).
    RegDecodeBits(u32),
    /// Register file high half is dead: a read of `r >= thresh` yields 0.
    RegZeroAbove(u16),
    /// Heap load/store address decode is `bits` wide: `heap[addr]` accesses physical `addr & ((1<<bits)-1)`.
    /// The allocator's length counter is a register and stays exact; only the RAM addressing folds.
    HeapDecodeBits(u32),
    /// Heap high half is dead: a load from `addr >= thresh` yields 0.
    HeapZeroAbove(usize),
    /// Cell-value plane `v` address decode is `bits` wide: an earlier-cell reference `ci` reads physical
    /// `ci & ((1<<bits)-1)`.
    VDecodeBits(u32),
    /// A `heap[X] = v` (kind-4) store becomes visible to heap reads only after `steps` further block-steps,
    /// modelling a registered read/write pipeline. Allocations (bump-pointer) commit immediately so addressing
    /// stays valid; only explicit stores lag.
    HeapStoreLatency(u32),
}

impl WFault {
    #[inline]
    fn reg(self, r: u16) -> u16 {
        match self {
            WFault::RegDecodeBits(b) => r & ((1u16 << b) - 1),
            _ => r,
        }
    }
    #[inline]
    fn reg_read_zero(self, r: u16) -> bool {
        matches!(self, WFault::RegZeroAbove(t) if r >= t)
    }
    #[inline]
    fn heap(self, a: usize) -> usize {
        match self {
            WFault::HeapDecodeBits(b) => a & ((1usize << b) - 1),
            _ => a,
        }
    }
    #[inline]
    fn heap_read_zero(self, a: usize) -> bool {
        matches!(self, WFault::HeapZeroAbove(t) if a >= t)
    }
    #[inline]
    fn vref(self, ci: usize) -> usize {
        match self {
            WFault::VDecodeBits(b) => ci & ((1usize << b) - 1),
            _ => ci,
        }
    }
}

// Fold a whole fault set over each access point (compound faults compose: every decode mask is applied, and a
// zero-read fires if any variant matches). An empty slice is the identity — byte-identical to the golden model.
#[inline]
fn f_reg(faults: &[WFault], r: u16) -> u16 {
    faults.iter().fold(r, |r, f| f.reg(r))
}
#[inline]
fn f_reg_zero(faults: &[WFault], r: u16) -> bool {
    faults.iter().any(|f| f.reg_read_zero(r))
}
#[inline]
fn f_heap(faults: &[WFault], a: usize) -> usize {
    faults.iter().fold(a, |a, f| f.heap(a))
}
#[inline]
fn f_heap_zero(faults: &[WFault], a: usize) -> bool {
    faults.iter().any(|f| f.heap_read_zero(a))
}
#[inline]
fn f_vref(faults: &[WFault], ci: usize) -> usize {
    faults.iter().fold(ci, |ci, f| f.vref(ci))
}
#[inline]
fn f_store_latency(faults: &[WFault]) -> usize {
    faults
        .iter()
        .filter_map(|f| {
            if let WFault::HeapStoreLatency(n) = f {
                Some(*n as usize)
            } else {
                None
            }
        })
        .max()
        .unwrap_or(0)
}

/// The golden interpreter under an injected hardware fault (see [`WFault`]). An empty fault set is
/// byte-identical to [`eval_wprog_config_counted_heap_preloaded`].
pub fn eval_wprog_config_faulted(
    cfg: &WProgConfig,
    params: &[i64],
    heap_init: &[i64],
    faults: &[WFault],
) -> (i64, u64) {
    use std::collections::HashMap;
    let mut file: HashMap<u16, i64> = (0..cfg.n_params)
        .map(|p| (f_reg(faults, p as u16), params[p]))
        .collect();
    let mut heap: Vec<i64> = heap_init.to_vec();
    let mut blk = 0usize;
    let mut stack: Vec<(usize, u16, HashMap<u16, i64>)> = Vec::new();

    // Read an operand. Cell-value references (mode 2) route through the faulty `v` decode; live-in reads
    // already went through the faulty register decode when `li` was built.
    let read = |src: &WSrc, li: &[i64], vals: &[i64]| -> i64 {
        match src.mode {
            0 => li[src.reference as usize],
            1 => src.konst,
            _ => vals[f_vref(faults, src.reference as usize).min(vals.len().saturating_sub(1))],
        }
    };

    // Heap stores held in flight, made visible to reads only after `store_lat` steps.
    let store_lat = f_store_latency(faults);
    let mut store_pipe: std::collections::VecDeque<Vec<(usize, i64)>> =
        std::iter::repeat_with(Vec::new).take(store_lat).collect();

    let mut steps = 0u64;
    for _ in 0..10_000_000 {
        steps += 1;
        // Commit the stores that have aged `store_lat` steps, then open a fresh in-flight bucket for this step.
        if store_lat > 0 {
            if let Some(commit) = store_pipe.pop_front() {
                for (a, v) in commit {
                    if a < heap.len() {
                        heap[a] = v;
                    }
                }
            }
            store_pipe.push_back(Vec::new());
        }
        let b = &cfg.blocks[blk];
        let li: Vec<i64> = b
            .livein
            .iter()
            .map(|r| {
                if f_reg_zero(faults, *r) {
                    0
                } else {
                    *file.get(&f_reg(faults, *r)).unwrap_or(&0)
                }
            })
            .collect();
        let mut vals: Vec<i64> = Vec::with_capacity(b.cells.len());
        for c in &b.cells {
            let v = match c.kind {
                0 => {
                    let a = read(&c.ops[0], &li, &vals);
                    let b2 = read(&c.ops[1], &li, &vals);
                    match c.code {
                        0 => a.wrapping_add(b2),
                        1 => a.wrapping_sub(b2),
                        2 => a.wrapping_mul(b2),
                        3 => a & b2,
                        4 => a | b2,
                        5 => a ^ b2,
                        6 => ((a as u64) << (b2 as u64 & 63)) as i64,
                        _ => a >> (b2 as u64 & 63),
                    }
                }
                1 => {
                    let s0 = read(&c.ops[0], &li, &vals);
                    let s1 = read(&c.ops[1], &li, &vals);
                    let hit = match c.code {
                        0 => s0 < s1,
                        1 => s0 > s1,
                        2 => s0 <= s1,
                        3 => s0 >= s1,
                        4 => s0 == s1,
                        _ => s0 != s1,
                    };
                    if hit {
                        read(&c.ops[2], &li, &vals)
                    } else {
                        read(&c.ops[3], &li, &vals)
                    }
                }
                3 => {
                    let a = read(&c.ops[0], &li, &vals).max(0) as usize;
                    if f_heap_zero(faults, a) {
                        0
                    } else {
                        heap.get(f_heap(faults, a)).copied().unwrap_or(0)
                    }
                }
                4 => {
                    let a = read(&c.ops[0], &li, &vals);
                    let bv = read(&c.ops[1], &li, &vals);
                    let ai = a.max(0) as usize;
                    if ai >= heap.len() {
                        heap.resize(ai + 1, 0);
                    }
                    let wi = f_heap(faults, ai).min(heap.len() - 1);
                    if store_lat > 0 {
                        store_pipe.back_mut().unwrap().push((wi, bv));
                    } else {
                        heap[wi] = bv;
                    }
                    bv
                }
                5 => {
                    let a = read(&c.ops[0], &li, &vals);
                    let ptr = heap.len() as i64;
                    heap.resize(heap.len() + a.max(0) as usize, 0);
                    ptr
                }
                6 => {
                    let src = read(&c.ops[0], &li, &vals).max(0) as usize;
                    let len = heap.get(src).copied().unwrap_or(0).max(0) as usize;
                    let newptr = heap.len() as i64;
                    let reserve = crate::codegen_sva::fabric_rtl::seq_reserve() as usize;
                    heap.resize(heap.len() + 1 + len + reserve, 0);
                    for j in 0..=len {
                        let v = heap.get(src + j).copied().unwrap_or(0);
                        let dst_idx = newptr as usize + j;
                        heap[dst_idx] = v;
                    }
                    newptr
                }
                7 => {
                    let mut chars: Vec<i64> = Vec::new();
                    for (slot, bit) in [(0usize, 1u8), (1usize, 2u8)] {
                        let v = read(&c.ops[slot], &li, &vals);
                        if c.code & bit != 0 {
                            let p = v.max(0) as usize;
                            let ln = heap.get(p).copied().unwrap_or(0).max(0) as usize;
                            for j in 0..ln {
                                chars.push(heap.get(p + 1 + j).copied().unwrap_or(0));
                            }
                        } else {
                            for byte in v.to_string().into_bytes() {
                                chars.push(byte as i64);
                            }
                        }
                    }
                    let ptr = heap.len() as i64;
                    heap.push(chars.len() as i64);
                    heap.extend(chars);
                    ptr
                }
                8 => {
                    let p = read(&c.ops[0], &li, &vals).max(0) as usize;
                    let idx = read(&c.ops[1], &li, &vals).max(0) as usize;
                    let ch = heap.get(p + idx).copied().unwrap_or(0);
                    let ptr = heap.len() as i64;
                    heap.push(1);
                    heap.push(ch);
                    ptr
                }
                9 => {
                    let a = read(&c.ops[0], &li, &vals).max(0) as usize;
                    let b = read(&c.ops[1], &li, &vals).max(0) as usize;
                    let la = heap.get(a).copied().unwrap_or(0);
                    let lb = heap.get(b).copied().unwrap_or(0);
                    let eq = la == lb
                        && (0..la as usize).all(|j| heap.get(a + 1 + j) == heap.get(b + 1 + j));
                    i64::from(if c.code == 0 { eq } else { !eq })
                }
                2 => {
                    let a = read(&c.ops[0], &li, &vals);
                    let b2 = read(&c.ops[1], &li, &vals);
                    func_from_code(c.code).eval(a, b2)
                }
                10 => read(&c.ops[0], &li, &vals),
                _ => panic!("wprog eval: unknown cell kind {}", c.kind),
            };
            vals.push(v);
        }
        for (r, s) in &b.writes {
            file.insert(f_reg(faults, *r), read(s, &li, &vals));
        }
        match &b.term {
            WTerm::Jump(t) => blk = *t,
            WTerm::Branch {
                cond,
                then_blk,
                else_blk,
            } => {
                blk = if read(cond, &li, &vals) != 0 {
                    *then_blk
                } else {
                    *else_blk
                }
            }
            WTerm::Call {
                callee_block,
                args,
                dst_reg,
                ret_block,
            } => {
                let arg_vals: Vec<i64> = args.iter().map(|a| read(a, &li, &vals)).collect();
                stack.push((*ret_block, *dst_reg, std::mem::take(&mut file)));
                file = arg_vals
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (f_reg(faults, i as u16), *v))
                    .collect();
                blk = *callee_block;
            }
            WTerm::Return(s) => {
                let ret_val = read(s, &li, &vals);
                match stack.pop() {
                    None => return (ret_val, steps),
                    Some((ret_block, dst_reg, saved_file)) => {
                        file = saved_file;
                        file.insert(f_reg(faults, dst_reg), ret_val);
                        blk = ret_block;
                    }
                }
            }
        }
    }
    panic!("wprog: did not return within the step budget (unbounded recursion or a non-terminating loop)")
}

/// The register-file width for a whole-program config: one 64-bit word per distinct register touched.
pub fn wprog_nregs(cfg: &WProgConfig) -> usize {
    let mut m = 0u16;
    for b in &cfg.blocks {
        for r in &b.livein {
            m = m.max(*r);
        }
        for (r, _) in &b.writes {
            m = m.max(*r);
        }
        if let WTerm::Call { dst_reg, .. } = &b.term {
            m = m.max(*dst_reg);
        }
    }
    (m as usize + 1).max(cfg.n_params).max(1)
}

// ── The GENERIC config-driven engine's fixed capacities ──────────────────────────────────────────────────────
// One netlist runs EVERY program that fits these bounds (recursion/collections/division/floats included). They
// are shared between the emitter (`emit_wprog_engine_verilog`) and the loud capacity guard
// (`wprog_config_fits_engine`) so a program too big for the netlist is REJECTED at image build, never silently
// mis-run — an over-`NREG` register access hits a nonexistent file slot and reads 0 (exactly the failure the
// Mandelbrot kernel first exposed). Sized to hold the real benchmark kernels while still targeting the Nexys A7.
// The generic (sim/iverilog) engine caps. Widened to hold a whole int8 transformer PROJECTION (matmul +
// requant + argmax) — the RDU's core workload — end to end on the config-driven engine (proven ≡VM by
// wpe_transformer_lock). These size only the generic engine (emit_wprog_engine_verilog) used under iverilog;
// the deployed silicon bitstream uses the separate WPE_HW_* profile, so this widening does not bloat the chip.
/// Register-file depth of the generic config-driven whole-program engine (per-register, `file[0:NREG-1]`).
pub const WPE_NREG: usize = 64;
/// Max basic blocks per program (`blk_*[0:MAXBLK-1]`, indexed by the block PC).
pub const WPE_MAXBLK: usize = 32;
/// Max TOTAL cells across the whole program — the flat `cell_*` config memory (indexed by `blk_cell_ptr[b]+ci`,
/// an accumulating pointer) AND the per-block value array `v[0:MAXCELL-1]`.
pub const WPE_MAXCELL: usize = 128;
/// Max TOTAL register writes across the whole program — the flat `wr_*` config memory (accumulating pointer).
pub const WPE_MAXWR: usize = 128;
/// Max TOTAL call arguments across the whole program — the flat `arg_op` config memory.
pub const WPE_MAXARG: usize = 16;
/// Max TOTAL live-in registers across the whole program — the flat `liv_reg` config memory.
pub const WPE_MAXLIV: usize = 96;
/// Max distinct constants in the program's konst table (the flat `konst` memory).
pub const WPE_MAXK: usize = 128;
/// Heap state-plane depth (words) — a projection builds a weight Seq + activation Seqs.
pub const WPE_HEAP: usize = 1024;
/// Frame-stack depth (max recursion).
pub const WPE_DEPTH: usize = 24;

/// Does `cfg` fit the fixed generic engine? Returns `Err` naming the first exceeded dimension. A program that
/// overflows any bound CANNOT run on the one-size netlist, so its image must be rejected LOUDLY here rather than
/// producing silently-wrong results (a flat-array overflow corrupts later blocks' cells/writes; an over-`NREG`
/// register indexes a nonexistent file slot → 0 — the two failure modes the Mandelbrot kernel first exposed).
/// The `cell_*`/`wr_*`/`liv_reg`/`arg_op`/`konst` memories are FLAT with accumulating per-block pointers, so
/// their bounds are program TOTALS, not per-block. Called by `wprog_config_image` — every image is checked.
pub fn wprog_config_fits_engine(cfg: &WProgConfig) -> Result<(), String> {
    if cfg.blocks.len() > WPE_MAXBLK {
        return Err(format!(
            "{} blocks > engine MAXBLK={WPE_MAXBLK}",
            cfg.blocks.len()
        ));
    }
    let nreg = wprog_nregs(cfg);
    if nreg > WPE_NREG {
        return Err(format!(
            "program needs {nreg} registers > engine NREG={WPE_NREG}"
        ));
    }
    let total_cells: usize = cfg.blocks.iter().map(|b| b.cells.len()).sum();
    if total_cells > WPE_MAXCELL {
        return Err(format!(
            "{total_cells} total cells > engine MAXCELL={WPE_MAXCELL}"
        ));
    }
    let total_writes: usize = cfg.blocks.iter().map(|b| b.writes.len()).sum();
    if total_writes > WPE_MAXWR {
        return Err(format!(
            "{total_writes} total writes > engine MAXWR={WPE_MAXWR}"
        ));
    }
    let total_liveins: usize = cfg.blocks.iter().map(|b| b.livein.len()).sum();
    if total_liveins > WPE_MAXLIV {
        return Err(format!(
            "{total_liveins} total live-ins > engine MAXLIV={WPE_MAXLIV}"
        ));
    }
    let total_args: usize = cfg
        .blocks
        .iter()
        .map(|b| {
            if let WTerm::Call { args, .. } = &b.term {
                args.len()
            } else {
                0
            }
        })
        .sum();
    if total_args > WPE_MAXARG {
        return Err(format!(
            "{total_args} total call args > engine MAXARG={WPE_MAXARG}"
        ));
    }
    Ok(())
}

fn wclog2(n: usize) -> usize {
    let mut b = 1;
    while (1usize << b) < n {
        b += 1;
    }
    b
}

/// The Verilog expression for a `WSrc` in the SEQUENTIAL per-cell engine (block live-ins are `livein`). Mode 0
/// (`In`) is a POSITION into the block's live-in list — exactly as `eval_wprog_config`'s `li[reference]` indexes
/// `livein` — so it is the register-file net `fr_{livein[reference]}`; mode 1 is a signed constant; mode 2
/// (`Val`) is an earlier cell's REGISTERED value `v[reference]`, valid because cells evaluate one per clock.
fn op_expr(livein: &[u16], s: &WSrc) -> String {
    match s.mode {
        0 => format!("fr_{}", livein[s.reference as usize]),
        1 => format!("$signed(64'h{:016x})", s.konst as u64),
        _ => format!("v[{}]", s.reference),
    }
}

/// Emit the WHOLE-PROGRAM ENGINE as synthesizable RTL: a register file + block-PC + a SEQUENTIAL per-cell
/// datapath (one cell per clock into a cell-value array `v`, so the ordered HEAP STATE PLANE — HeapRead /
/// HeapWrite / HeapAlloc against a heap BRAM — threads exactly as the model does) + a terminator FSM
/// (Jump/Branch/Return/CALL) + a FRAME STACK that saves the caller's register file on every `Call` and
/// restores it on `Return`. So the WHOLE language family — branchy control flow, loops, recursion, division,
/// and collections/strings/structs on the heap — runs on silicon, proven bit-identical to `eval_wprog_config`
/// (≡ the VM) under iverilog. Cells: Arith / Select / Func (divider) / HeapRead / HeapWrite / HeapAlloc.
pub fn emit_wprog_verilog(name: &str, cfg: &WProgConfig) -> String {
    let nb = cfg.blocks.len();
    let nreg = wprog_nregs(cfg);
    let pcw = wclog2(nb);
    let depth = 64usize; // frame-stack depth bound for the differential
    let spw = wclog2(depth + 1);
    let dstw = wclog2(nreg).max(1);
    let max_cells = cfg.blocks.iter().map(|b| b.cells.len()).max().unwrap_or(0);
    let vcap = max_cells.max(1);
    let ciw = wclog2(max_cells + 1);
    const HSIZE: usize = 1024; // heap BRAM depth (words) — sized for the differential's collections

    let mut b = String::new();
    let mut ports: Vec<String> = vec!["input clk".into(), "input rst".into()];
    for p in 0..cfg.n_params {
        ports.push(format!("input signed [63:0] p{p}"));
    }
    ports.push("output [63:0] result".into());
    ports.push("output done".into());
    ports.push("output [63:0] cycles".into());

    b.push_str(&format!(
        "  localparam NREG = {nreg};\n  localparam HSIZE = {HSIZE};\n"
    ));
    b.push_str(&format!("  reg signed [63:0] file [0:{}];\n", nreg - 1));
    b.push_str(&format!(
        "  reg signed [63:0] sfile [0:{}];\n",
        depth * nreg - 1
    ));
    b.push_str(&format!("  reg [{}:0] sret [0:{}];\n", pcw - 1, depth - 1));
    b.push_str(&format!("  reg [{}:0] sdst [0:{}];\n", dstw - 1, depth - 1));
    b.push_str(&format!("  reg [{}:0] sp;\n", spw - 1));
    b.push_str(&format!("  reg [{}:0] pc;\n", pcw - 1));
    b.push_str(&format!("  reg signed [63:0] v [0:{}];\n", vcap - 1)); // this block's cell values
    b.push_str(&format!("  reg [{}:0] ci;\n", ciw - 1)); // cell index within the current block
    b.push_str("  reg signed [63:0] heap [0:HSIZE-1];\n  reg signed [63:0] hp;\n"); // shared heap + length
    b.push_str("  reg done_r; reg [63:0] result_r; reg [63:0] cyc_r;\n");
    b.push_str("  assign done = done_r; assign result = result_r; assign cycles = cyc_r;\n");
    b.push_str("  integer gi, hj;\n\n");

    for r in 0..nreg {
        b.push_str(&format!("  wire signed [63:0] fr_{r} = file[{r}];\n"));
    }
    b.push('\n');

    b.push_str("  function signed [63:0] alu_f(input [3:0] op, input signed [63:0] a, input signed [63:0] x);\n");
    b.push_str("    case (op)\n");
    b.push_str("      4'd0: alu_f=a+x; 4'd1: alu_f=a-x; 4'd2: alu_f=a*x;\n");
    b.push_str("      4'd3: alu_f=a&x; 4'd4: alu_f=a|x; 4'd5: alu_f=a^x;\n");
    b.push_str("      4'd6: alu_f=a<<<x; 4'd7: alu_f=a>>>x; 4'd8: alu_f=$unsigned(a)>>x; default: alu_f=64'sd0;\n");
    b.push_str("    endcase\n  endfunction\n");
    b.push_str("  function cmp_f(input [3:0] op, input signed [63:0] a, input signed [63:0] x);\n");
    b.push_str("    case (op)\n");
    b.push_str("      4'd0: cmp_f=(a<x); 4'd1: cmp_f=(a>x); 4'd2: cmp_f=(a<=x);\n");
    b.push_str("      4'd3: cmp_f=(a>=x); 4'd4: cmp_f=(a==x); default: cmp_f=(a!=x);\n");
    b.push_str("    endcase\n  endfunction\n");
    b.push_str(func_f_verilog());
    b.push('\n');

    // The FSM: sequence each block's cells one per clock (into `v`), then commit its writes + terminator.
    b.push_str("  always @(posedge clk) begin\n    if (rst) begin\n");
    b.push_str(&format!(
        "      pc <= {pcw}'d0; done_r <= 1'b0; result_r <= 64'd0; cyc_r <= 64'd0; sp <= {spw}'d0; ci <= {ciw}'d0; hp <= 64'sd0;\n"
    ));
    b.push_str("      for (hj=0; hj<HSIZE; hj=hj+1) heap[hj] <= 64'sd0;\n");
    for r in 0..nreg {
        if r < cfg.n_params {
            b.push_str(&format!("      file[{r}] <= p{r};\n"));
        } else {
            b.push_str(&format!("      file[{r}] <= 64'sd0;\n"));
        }
    }
    b.push_str("    end else if (!done_r) begin\n      cyc_r <= cyc_r + 64'd1;\n      case (pc)\n");
    for (k, blk) in cfg.blocks.iter().enumerate() {
        let li = &blk.livein;
        b.push_str(&format!("        {pcw}'d{k}: begin\n"));
        let commit = |b: &mut String| {
            // Commit register writes, then follow the terminator. Reset the cell index for the next block.
            b.push_str(&format!("            ci <= {ciw}'d0;\n"));
            for (r, s) in &blk.writes {
                b.push_str(&format!("            file[{r}] <= {};\n", op_expr(li, s)));
            }
            match &blk.term {
                WTerm::Jump(t) => b.push_str(&format!("            pc <= {pcw}'d{t};\n")),
                WTerm::Branch {
                    cond,
                    then_blk,
                    else_blk,
                } => b.push_str(&format!(
                    "            pc <= (({}) != 64'sd0) ? {pcw}'d{then_blk} : {pcw}'d{else_blk};\n",
                    op_expr(li, cond)
                )),
                WTerm::Return(s) => {
                    b.push_str(&format!(
                        "            if (sp == {spw}'d0) begin result_r <= {}; done_r <= 1'b1; end\n",
                        op_expr(li, s)
                    ));
                    b.push_str("            else begin\n");
                    b.push_str("              for (gi=0; gi<NREG; gi=gi+1) file[gi] <= sfile[(sp-1)*NREG+gi];\n");
                    b.push_str(&format!(
                        "              file[sdst[sp-1]] <= {};\n",
                        op_expr(li, s)
                    ));
                    b.push_str(&format!(
                        "              pc <= sret[sp-1]; sp <= sp - {spw}'d1;\n"
                    ));
                    b.push_str("            end\n");
                }
                WTerm::Call {
                    callee_block,
                    args,
                    dst_reg,
                    ret_block,
                } => {
                    b.push_str(
                        "            for (gi=0; gi<NREG; gi=gi+1) sfile[sp*NREG+gi] <= file[gi];\n",
                    );
                    // Registers WRITTEN in this Call block are live across the call, but the loop above snapshots
                    // the PRE-write file (non-blocking `<=` reads the old value). Override those slots with the NEW
                    // value (last-write-wins) so a value computed in the call block — e.g. `n*4` in `(n*4)+f(n-1)` —
                    // survives the call + its post-Return restore instead of collapsing to the stale pre-block value.
                    for (r, s) in &blk.writes {
                        b.push_str(&format!(
                            "            sfile[sp*NREG+{r}] <= {};\n",
                            op_expr(li, s)
                        ));
                    }
                    b.push_str(&format!(
                        "            sret[sp] <= {pcw}'d{ret_block}; sdst[sp] <= {dstw}'d{dst_reg}; sp <= sp + {spw}'d1;\n"
                    ));
                    for (a, s) in args.iter().enumerate() {
                        b.push_str(&format!("            file[{a}] <= {};\n", op_expr(li, s)));
                    }
                    for a in args.len()..nreg {
                        b.push_str(&format!("            file[{a}] <= 64'sd0;\n"));
                    }
                    b.push_str(&format!("            pc <= {pcw}'d{callee_block};\n"));
                }
            }
        };
        if blk.cells.is_empty() {
            commit(&mut b);
        } else {
            b.push_str(&format!(
                "          if (ci < {ciw}'d{}) begin\n",
                blk.cells.len()
            ));
            b.push_str("            case (ci)\n");
            for (i, c) in blk.cells.iter().enumerate() {
                b.push_str(&format!("              {ciw}'d{i}: begin\n"));
                match c.kind {
                    0 => b.push_str(&format!(
                        "                v[{i}] <= alu_f(4'd{}, {}, {});\n",
                        c.code, op_expr(li, &c.ops[0]), op_expr(li, &c.ops[1])
                    )),
                    1 => b.push_str(&format!(
                        "                v[{i}] <= cmp_f(4'd{}, {}, {}) ? ({}) : ({});\n",
                        c.code, op_expr(li, &c.ops[0]), op_expr(li, &c.ops[1]),
                        op_expr(li, &c.ops[2]), op_expr(li, &c.ops[3])
                    )),
                    2 => b.push_str(&format!(
                        "                v[{i}] <= func_f(8'd{}, {}, {});\n",
                        c.code, op_expr(li, &c.ops[0]), op_expr(li, &c.ops[1])
                    )),
                    // HeapRead: v[i] <= heap[addr]. The heap is reset-zeroed, so an unwritten / unallocated
                    // slot reads 0 — matching `eval_wprog_config`'s `heap.get(ai).unwrap_or(0)`.
                    3 => b.push_str(&format!(
                        "                v[{i}] <= heap[{}];\n",
                        op_expr(li, &c.ops[0])
                    )),
                    // HeapWrite: store, grow the length pointer if the address is at/beyond it, drive the value.
                    4 => {
                        let addr = op_expr(li, &c.ops[0]);
                        let val = op_expr(li, &c.ops[1]);
                        b.push_str(&format!("                heap[{addr}] <= {val};\n"));
                        b.push_str(&format!("                if (({addr}) >= hp) hp <= ({addr}) + 64'sd1;\n"));
                        b.push_str(&format!("                v[{i}] <= {val};\n"));
                    }
                    // HeapAlloc: return the base pointer (current length), grow the heap by `size` (zeroed region).
                    5 => {
                        let sz = op_expr(li, &c.ops[0]);
                        b.push_str(&format!("                v[{i}] <= hp; hp <= hp + ({sz});\n"));
                    }
                    10 => panic!(
                        "wprog RTL: a Show cell streams over the SoC's byte channel — the per-program netlist \
                         has no channel; run Show programs on the SoC engine (emit_wprog_engine_soc_verilog)"
                    ),
                    other => panic!("wprog RTL: unknown cell kind {other}"),
                }
                b.push_str("              end\n");
            }
            b.push_str("            endcase\n");
            b.push_str(&format!("            ci <= ci + {ciw}'d1;\n"));
            b.push_str("          end else begin\n");
            commit(&mut b);
            b.push_str("          end\n");
        }
        b.push_str("        end\n");
    }
    b.push_str("        default: done_r <= 1'b1;\n      endcase\n    end\n  end\n");

    format!(
        "// wprog_{name} — the WHOLE-PROGRAM ENGINE in RTL: register file + block-PC + SEQUENTIAL per-cell\n\
         // datapath (Arith/Select/Func + a heap state plane: HeapRead/Write/Alloc on a heap BRAM) + terminator\n\
         // FSM (Jump/Branch/Return/Call) + a FRAME STACK, so recursion + collections run on silicon. == eval.\n\
         module wprog_{name}(\n    {}\n);\n{b}endmodule\n",
        ports.join(",\n    "),
    )
}

/// A self-checking testbench for the whole-program engine: for each param vector, pulse reset, run the FSM to
/// `done`, and compare `result` to the `eval_wprog_config` oracle in `wprog_{name}_out.hex` (params in
/// `wprog_{name}_in.hex`). Proves the RTL — register file + block walk + FRAME STACK — matches the model.
pub fn emit_wprog_testbench(name: &str, n_params: usize, n_vectors: usize) -> String {
    let mut decls = String::new();
    if n_params > 0 {
        decls.push_str(&format!(
            "  reg signed [63:0] pin [0:{}];\n",
            n_params * n_vectors - 1
        ));
    }
    decls.push_str(&format!(
        "  reg signed [63:0] oout [0:{}];\n",
        n_vectors - 1
    ));
    for p in 0..n_params {
        decls.push_str(&format!("  reg signed [63:0] p{p};\n"));
    }
    let mut dut_ports: Vec<String> = vec![".clk(clk)".into(), ".rst(rst)".into()];
    for p in 0..n_params {
        dut_ports.push(format!(".p{p}(p{p})"));
    }
    dut_ports.push(".result(result)".into());
    dut_ports.push(".done(done)".into());
    dut_ports.push(".cycles(cycles)".into());
    let readin = if n_params > 0 {
        format!("    $readmemh(\"wprog_{name}_in.hex\", pin);\n")
    } else {
        String::new()
    };
    let mut load = String::new();
    for p in 0..n_params {
        load.push_str(&format!("      p{p} = pin[i*{n_params}+{p}];\n"));
    }
    format!(
        r#"// tb_wprog_{name} — the whole-program engine (regfile + block-walk + frame stack) vs eval_wprog_config.
`timescale 1ns/1ps
module tb_wprog_{name};
  reg clk, rst;
{decls}  wire [63:0] result, cycles; wire done;
  integer i, errors, guard;
  wprog_{name} dut({ports});
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin
    errors = 0;
{readin}    $readmemh("wprog_{name}_out.hex", oout);
    for (i = 0; i < {n_vectors}; i = i + 1) begin
{load}      rst = 1; @(posedge clk); #1; rst = 0;
      guard = 0;
      while (!done && guard < 2000000) begin @(posedge clk); #1; guard = guard + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: {n_vectors} whole-program (recursion) runs match eval_wprog_config");
    else $display("FAIL: %0d whole-program mismatches", errors);
    $finish;
  end
endmodule
"#,
        ports = dut_ports.join(", "),
    )
}

// ── Config-driven whole-program processor: the config IMAGE (loaded over the wire) + the GENERIC engine that
// interprets it. Where `emit_wprog_verilog` specializes the RTL to ONE program, this pair is ONE fixed engine
// that runs ANY `WProgConfig` by loading its image into on-chip memories — the self-synthesizing bytecode XPU.
// The image is a set of PARALLEL per-field memories (no bit-packing) so the engine indexes fields directly.

/// A 32-bit operand word: `mode` in bits [31:30], `ref` in [15:0]. `In` → ref is the live-in POSITION; `Const`
/// → ref indexes the 64-bit constant pool; `Val` → ref is the earlier cell index.
fn pack_op(s: &WSrc, konst_pool: &mut Vec<i64>) -> u32 {
    let (mode, r) = match s.mode {
        0 => (0u32, s.reference as u32),
        1 => {
            // Intern the constant: identical values share one konst slot, so the table holds only the DISTINCT
            // constants (keeping it under the engine's fixed `MAXK`). Safe — a konst index is by-value, and the
            // model reads `WSrc.konst` inline, so deduping the packed image changes nothing the engine computes.
            let idx = konst_pool
                .iter()
                .position(|&k| k == s.konst)
                .unwrap_or_else(|| {
                    konst_pool.push(s.konst);
                    konst_pool.len() - 1
                }) as u32;
            (1u32, idx)
        }
        _ => (2u32, s.reference as u32),
    };
    (mode << 30) | (r & 0xffff)
}

/// The flat, loadable image of a whole program: parallel per-field arrays the generic engine reads directly.
/// Block `b`'s fields are `blk_*[b]`; its cells are `cell_*[blk_cell_ptr[b] .. + blk_n_cells[b]]`; and so on.
#[derive(Default)]
pub struct WImage {
    pub n_blocks: usize,
    pub n_params: usize,
    pub nreg: usize,
    pub blk_livein_ptr: Vec<u32>,
    pub blk_n_livein: Vec<u32>,
    pub blk_cell_ptr: Vec<u32>,
    pub blk_n_cells: Vec<u32>,
    pub blk_write_ptr: Vec<u32>,
    pub blk_n_writes: Vec<u32>,
    pub blk_term_kind: Vec<u32>, // 0 Jump, 1 Branch, 2 Return, 3 Call
    pub blk_t0: Vec<u32>,        // Jump target / Branch then / Call callee
    pub blk_t1: Vec<u32>,        // Branch else / Call ret_block
    pub blk_top: Vec<u32>,       // Branch cond op / Return op (32-bit operand word)
    pub blk_cdst: Vec<u32>,      // Call dst register
    pub blk_cargc: Vec<u32>,     // Call arg count
    pub blk_cargp: Vec<u32>,     // Call arg pointer
    pub cell_kind: Vec<u32>,
    pub cell_code: Vec<u32>,
    pub cell_op0: Vec<u32>,
    pub cell_op1: Vec<u32>,
    pub cell_op2: Vec<u32>,
    pub cell_op3: Vec<u32>,
    pub wr_reg: Vec<u32>,
    pub wr_op: Vec<u32>,
    pub arg_op: Vec<u32>,
    pub liv_reg: Vec<u32>,
    pub konst: Vec<i64>,
}

/// Flatten a `WProgConfig` into its loadable image — the exact tables the generic engine walks: block records
/// with base pointers into flat cell / write / arg / live-in pools, and a constant pool for `Const` operands.
pub fn wprog_config_image(cfg: &WProgConfig) -> WImage {
    // A program that overflows the fixed netlist cannot run on it — reject it LOUDLY here instead of emitting an
    // image the engine would silently mis-run (the Mandelbrot-over-NREG bug).
    if let Err(e) = wprog_config_fits_engine(cfg) {
        panic!("wprog_config_image: program does not fit the generic config-driven engine ({e})");
    }
    wprog_config_image_unchecked(cfg)
}

/// Build the flat wire image WITHOUT the generic-engine fit check — for streaming into a BIG-cap engine (the
/// backend/scale work: the pipelined BRAM engine sized to run every benchmark) whose caps exceed the generic
/// netlist. The caller guarantees the program fits the target cap profile; only the `MAXK` konst-table bound
/// (shared by every profile) is still asserted here.
pub fn wprog_config_image_unchecked(cfg: &WProgConfig) -> WImage {
    let nreg = wprog_nregs(cfg);
    let mut im = WImage {
        n_blocks: cfg.blocks.len(),
        n_params: cfg.n_params,
        nreg,
        blk_livein_ptr: vec![],
        blk_n_livein: vec![],
        blk_cell_ptr: vec![],
        blk_n_cells: vec![],
        blk_write_ptr: vec![],
        blk_n_writes: vec![],
        blk_term_kind: vec![],
        blk_t0: vec![],
        blk_t1: vec![],
        blk_top: vec![],
        blk_cdst: vec![],
        blk_cargc: vec![],
        blk_cargp: vec![],
        cell_kind: vec![],
        cell_code: vec![],
        cell_op0: vec![],
        cell_op1: vec![],
        cell_op2: vec![],
        cell_op3: vec![],
        wr_reg: vec![],
        wr_op: vec![],
        arg_op: vec![],
        liv_reg: vec![],
        konst: vec![],
    };
    for blk in &cfg.blocks {
        im.blk_livein_ptr.push(im.liv_reg.len() as u32);
        im.blk_n_livein.push(blk.livein.len() as u32);
        for r in &blk.livein {
            im.liv_reg.push(*r as u32);
        }
        im.blk_cell_ptr.push(im.cell_kind.len() as u32);
        im.blk_n_cells.push(blk.cells.len() as u32);
        for c in &blk.cells {
            im.cell_kind.push(c.kind as u32);
            im.cell_code.push(c.code as u32);
            im.cell_op0.push(pack_op(&c.ops[0], &mut im.konst));
            im.cell_op1.push(pack_op(&c.ops[1], &mut im.konst));
            im.cell_op2.push(pack_op(&c.ops[2], &mut im.konst));
            im.cell_op3.push(pack_op(&c.ops[3], &mut im.konst));
        }
        im.blk_write_ptr.push(im.wr_reg.len() as u32);
        im.blk_n_writes.push(blk.writes.len() as u32);
        for (r, s) in &blk.writes {
            im.wr_reg.push(*r as u32);
            let op = pack_op(s, &mut im.konst);
            im.wr_op.push(op);
        }
        let (tk, t0, t1, top, cdst, cargc, cargp) = match &blk.term {
            WTerm::Jump(t) => (0u32, *t as u32, 0, 0u32, 0u32, 0u32, 0u32),
            WTerm::Branch {
                cond,
                then_blk,
                else_blk,
            } => (
                1u32,
                *then_blk as u32,
                *else_blk as u32,
                pack_op(cond, &mut im.konst),
                0,
                0,
                0,
            ),
            WTerm::Return(s) => (2u32, 0, 0, pack_op(s, &mut im.konst), 0, 0, 0),
            WTerm::Call {
                callee_block,
                args,
                dst_reg,
                ret_block,
            } => {
                let argp = im.arg_op.len() as u32;
                let ops: Vec<u32> = args.iter().map(|a| pack_op(a, &mut im.konst)).collect();
                for op in ops {
                    im.arg_op.push(op);
                }
                (
                    3u32,
                    *callee_block as u32,
                    *ret_block as u32,
                    0,
                    *dst_reg as u32,
                    args.len() as u32,
                    argp,
                )
            }
        };
        im.blk_term_kind.push(tk);
        im.blk_t0.push(t0);
        im.blk_t1.push(t1);
        im.blk_top.push(top);
        im.blk_cdst.push(cdst);
        im.blk_cargc.push(cargc);
        im.blk_cargp.push(cargp);
    }
    // The konst table is built during packing, so its final size is checked here (the structural bounds were
    // checked up front). An over-`MAXK` program would read a nonexistent constant slot.
    assert!(
        im.konst.len() <= WPE_MAXK,
        "wprog_config_image: {} distinct constants > engine MAXK={WPE_MAXK}",
        im.konst.len()
    );
    im
}

impl WImage {
    /// The `$readmemh` payloads for every field memory, as `(memory-name, hex-lines)` pairs. An EMPTY memory
    /// still emits one `0` line so `$readmemh` has a file to read (the engine never indexes an empty pool).
    pub fn hex_files(&self) -> Vec<(String, String)> {
        fn h32(v: &[u32]) -> String {
            if v.is_empty() {
                return "00000000\n".to_string();
            }
            v.iter().map(|x| format!("{x:08x}\n")).collect()
        }
        fn h64(v: &[i64]) -> String {
            if v.is_empty() {
                return "0000000000000000\n".to_string();
            }
            v.iter().map(|x| format!("{:016x}\n", *x as u64)).collect()
        }
        vec![
            ("blk_livein_ptr".into(), h32(&self.blk_livein_ptr)),
            ("blk_n_livein".into(), h32(&self.blk_n_livein)),
            ("blk_cell_ptr".into(), h32(&self.blk_cell_ptr)),
            ("blk_n_cells".into(), h32(&self.blk_n_cells)),
            ("blk_write_ptr".into(), h32(&self.blk_write_ptr)),
            ("blk_n_writes".into(), h32(&self.blk_n_writes)),
            ("blk_term_kind".into(), h32(&self.blk_term_kind)),
            ("blk_t0".into(), h32(&self.blk_t0)),
            ("blk_t1".into(), h32(&self.blk_t1)),
            ("blk_top".into(), h32(&self.blk_top)),
            ("blk_cdst".into(), h32(&self.blk_cdst)),
            ("blk_cargc".into(), h32(&self.blk_cargc)),
            ("blk_cargp".into(), h32(&self.blk_cargp)),
            ("cell_kind".into(), h32(&self.cell_kind)),
            ("cell_code".into(), h32(&self.cell_code)),
            ("cell_op0".into(), h32(&self.cell_op0)),
            ("cell_op1".into(), h32(&self.cell_op1)),
            ("cell_op2".into(), h32(&self.cell_op2)),
            ("cell_op3".into(), h32(&self.cell_op3)),
            ("wr_reg".into(), h32(&self.wr_reg)),
            ("wr_op".into(), h32(&self.wr_op)),
            ("arg_op".into(), h32(&self.arg_op)),
            ("liv_reg".into(), h32(&self.liv_reg)),
            ("konst".into(), h64(&self.konst)),
        ]
    }
}

/// Emit the GENERIC config-driven whole-program processor: ONE fixed microcoded engine that runs ANY program
/// by loading its `WImage` into on-chip config memories (via `$readmemh`) and interpreting it — fetch a block
/// record, evaluate its cells one per clock (operands resolved from the register file / cell-value array /
/// constant pool through the live-in table), commit its register writes, then follow the terminator, with a
/// heap state plane and a frame stack for `Call`/`Return`. Proven ≡ `eval_wprog_config` (≡ the VM) under
/// iverilog for any loaded image. `n_params` sizes only the input ports; every other bound is a fixed generic
/// capacity, so the SAME netlist runs every program that fits.
pub fn emit_wprog_engine_verilog(name: &str, n_params: usize) -> String {
    // Fixed generic capacities (one netlist for all programs that fit these bounds) — shared with the loud
    // capacity guard (`wprog_config_fits_engine`) so a program too big for the netlist is REJECTED at image
    // build, never silently mis-run (an over-`NREG` register access would hit a nonexistent file slot → 0).
    const NREG: usize = WPE_NREG;
    const MAXBLK: usize = WPE_MAXBLK;
    const MAXCELL: usize = WPE_MAXCELL;
    const MAXWR: usize = WPE_MAXWR;
    const MAXARG: usize = WPE_MAXARG;
    const MAXLIV: usize = WPE_MAXLIV;
    const MAXK: usize = WPE_MAXK;
    const HEAP: usize = WPE_HEAP;
    const DEPTH: usize = WPE_DEPTH;

    let mut ports: Vec<String> = vec!["input clk".into(), "input rst".into()];
    for p in 0..n_params {
        ports.push(format!("input signed [63:0] p{p}"));
    }
    ports.push("output [63:0] result".into());
    ports.push("output done".into());

    let mut file_init = String::new();
    for r in 0..NREG {
        if r < n_params {
            file_init.push_str(&format!("      file[{r}] <= p{r};\n"));
        } else {
            file_init.push_str(&format!("      file[{r}] <= 64'sd0;\n"));
        }
    }

    // Config memories + their $readmemh loads.
    let cfg_mems = [
        ("blk_livein_ptr", MAXBLK, 32),
        ("blk_n_livein", MAXBLK, 32),
        ("blk_cell_ptr", MAXBLK, 32),
        ("blk_n_cells", MAXBLK, 32),
        ("blk_write_ptr", MAXBLK, 32),
        ("blk_n_writes", MAXBLK, 32),
        ("blk_term_kind", MAXBLK, 32),
        ("blk_t0", MAXBLK, 32),
        ("blk_t1", MAXBLK, 32),
        ("blk_top", MAXBLK, 32),
        ("blk_cdst", MAXBLK, 32),
        ("blk_cargc", MAXBLK, 32),
        ("blk_cargp", MAXBLK, 32),
        ("cell_kind", MAXCELL, 32),
        ("cell_code", MAXCELL, 32),
        ("cell_op0", MAXCELL, 32),
        ("cell_op1", MAXCELL, 32),
        ("cell_op2", MAXCELL, 32),
        ("cell_op3", MAXCELL, 32),
        ("wr_reg", MAXWR, 32),
        ("wr_op", MAXWR, 32),
        ("arg_op", MAXARG, 32),
        ("liv_reg", MAXLIV, 32),
    ];
    let mut decls = String::new();
    let mut loads = String::new();
    for (nm, depth, w) in cfg_mems {
        decls.push_str(&format!("  reg [{}:0] {nm} [0:{}];\n", w - 1, depth - 1));
        loads.push_str(&format!("    $readmemh(\"wpe_{name}_{nm}.hex\", {nm});\n"));
    }
    decls.push_str(&format!("  reg signed [63:0] konst [0:{}];\n", MAXK - 1));
    loads.push_str(&format!(
        "    $readmemh(\"wpe_{name}_konst.hex\", konst);\n"
    ));

    format!(
        r#"// wpe_{name} — the GENERIC config-driven WHOLE-PROGRAM PROCESSOR. One fixed microcoded engine runs ANY
// program by loading its image ($readmemh) and interpreting it: fetch block -> eval cells (1/clk) -> commit
// writes (1/clk) -> terminator, over a register file + cell-value array + heap state plane + a frame stack.
// == eval_wprog_config for every loaded image.
`timescale 1ns/1ps
module wpe_{name}(
    {ports}
);
  localparam NREG={NREG}, HEAP={HEAP}, DEPTH={DEPTH};
{decls}
  reg signed [63:0] file [0:NREG-1];
  reg signed [63:0] v [0:{MAXCELL_M1}];
  reg signed [63:0] heap [0:HEAP-1];
  reg signed [63:0] hp;
  reg signed [63:0] sfile [0:DEPTH*NREG-1];
  reg [15:0] sret [0:DEPTH-1];
  reg [15:0] sdst [0:DEPTH-1];
  reg [15:0] sp, pc, ci, wi, gi, ai;
  reg [2:0] st;
  reg signed [63:0] retval;
  reg done_r; reg signed [63:0] result_r;
  assign done = done_r; assign result = result_r;
  integer hj;
  initial begin
{loads}  end

  // Current block record (combinational, indexed by pc).
  wire [31:0] c_livptr = blk_livein_ptr[pc]; wire [31:0] c_cellptr = blk_cell_ptr[pc];
  wire [31:0] c_ncells = blk_n_cells[pc];    wire [31:0] c_wrptr  = blk_write_ptr[pc];
  wire [31:0] c_nwr = blk_n_writes[pc];      wire [31:0] c_tk = blk_term_kind[pc];
  wire [31:0] c_t0 = blk_t0[pc]; wire [31:0] c_t1 = blk_t1[pc]; wire [31:0] c_top = blk_top[pc];
  wire [31:0] c_cdst = blk_cdst[pc]; wire [31:0] c_cargc = blk_cargc[pc]; wire [31:0] c_cargp = blk_cargp[pc];
  // Current cell (combinational, indexed by cellptr+ci).
  wire [31:0] cc_kind = cell_kind[c_cellptr + ci]; wire [31:0] cc_code = cell_code[c_cellptr + ci];
  wire [31:0] cc_op0 = cell_op0[c_cellptr + ci]; wire [31:0] cc_op1 = cell_op1[c_cellptr + ci];
  wire [31:0] cc_op2 = cell_op2[c_cellptr + ci]; wire [31:0] cc_op3 = cell_op3[c_cellptr + ci];

  function signed [63:0] alu_f(input [3:0] op, input signed [63:0] a, input signed [63:0] x);
    case (op)
      4'd0: alu_f=a+x; 4'd1: alu_f=a-x; 4'd2: alu_f=a*x; 4'd3: alu_f=a&x;
      4'd4: alu_f=a|x; 4'd5: alu_f=a^x; 4'd6: alu_f=a<<<x; 4'd7: alu_f=a>>>x; default: alu_f=64'sd0;
    endcase
  endfunction
  function cmp_f(input [3:0] op, input signed [63:0] a, input signed [63:0] x);
    case (op)
      4'd0: cmp_f=(a<x); 4'd1: cmp_f=(a>x); 4'd2: cmp_f=(a<=x);
      4'd3: cmp_f=(a>=x); 4'd4: cmp_f=(a==x); default: cmp_f=(a!=x);
    endcase
  endfunction
{func_f_def}  // Resolve an operand against the LIVE register file: In (mode 0) -> file[liv_reg[lptr+ref]], Const -> konst,
  // Val -> v[ref].
  function signed [63:0] resolve(input [31:0] op, input [31:0] lptr);
    case (op[31:30])
      2'd0: resolve = file[liv_reg[lptr + op[15:0]]];
      2'd1: resolve = konst[op[15:0]];
      default: resolve = v[op[15:0]];
    endcase
  endfunction
  // Resolve a CALL argument against the SAVED caller frame (sfile[(sp-1)*NREG+reg]) — sp is already bumped —
  // so binding args does not corrupt the operands still being read. Val still reads this block's cells `v`.
  function signed [63:0] resolve_arg(input [31:0] idx);
    reg [31:0] op;
    begin
      op = arg_op[c_cargp + idx];
      case (op[31:30])
        2'd0: resolve_arg = sfile[(sp-1)*NREG + liv_reg[c_livptr + op[15:0]]];
        2'd1: resolve_arg = konst[op[15:0]];
        default: resolve_arg = v[op[15:0]];
      endcase
    end
  endfunction

  // Operand resolution reads the LIVE register file, so it must be recomputed inside the clocked block each
  // step (a continuous-assign `wire = resolve(...)` would NOT re-trigger when `file` changes, since `file` is
  // read inside the function, not passed as an argument — the classic function-in-continuous-assign staleness).
  reg signed [63:0] r0, r1, r2, r3, cellval;

  always @(posedge clk) begin
    if (rst) begin
      pc<=16'd0; sp<=16'd0; ci<=16'd0; wi<=16'd0; gi<=16'd0; ai<=16'd0; st<=3'd0;
      done_r<=1'b0; result_r<=64'sd0; hp<=64'sd0;
      for (hj=0; hj<HEAP; hj=hj+1) heap[hj] <= 64'sd0;
{file_init}    end else if (!done_r) begin
      case (st)
        3'd0: begin // EVAL cells — resolve operands FRESH (blocking) against the live file, then latch the value
          if (ci < c_ncells[15:0]) begin
            r0 = resolve(cc_op0, c_livptr);
            r1 = resolve(cc_op1, c_livptr);
            r2 = resolve(cc_op2, c_livptr);
            r3 = resolve(cc_op3, c_livptr);
            cellval = (cc_kind==32'd0) ? alu_f(cc_code[3:0], r0, r1) :
                      (cc_kind==32'd1) ? (cmp_f(cc_code[3:0], r0, r1) ? r2 : r3) :
                      (cc_kind==32'd2) ? func_f(cc_code[7:0], r0, r1) :
                      (cc_kind==32'd3) ? heap[r0] :
                      (cc_kind==32'd4) ? r1 : hp;
            v[ci] <= cellval;
            if (cc_kind==32'd4) begin heap[r0] <= r1; if (r0 >= hp) hp <= r0 + 64'sd1; end
            if (cc_kind==32'd5) hp <= hp + r0;
            ci <= ci + 16'd1;
          end else begin wi <= 16'd0; st <= 3'd1; end
        end
        3'd1: begin // COMMIT writes
          if (wi < c_nwr[15:0]) begin
            file[wr_reg[c_wrptr + wi][15:0]] <= resolve(wr_op[c_wrptr + wi], c_livptr);
            wi <= wi + 16'd1;
          end else st <= 3'd2;
        end
        3'd2: begin // TERMINATOR
          ci <= 16'd0;
          case (c_tk)
            32'd0: begin pc <= c_t0[15:0]; st <= 3'd0; end
            32'd1: begin pc <= (resolve(c_top, c_livptr) != 64'sd0) ? c_t0[15:0] : c_t1[15:0]; st <= 3'd0; end
            32'd2: begin
              if (sp == 16'd0) begin result_r <= resolve(c_top, c_livptr); done_r <= 1'b1; end
              else begin retval <= resolve(c_top, c_livptr); gi <= 16'd0; st <= 3'd5; end
            end
            default: begin gi <= 16'd0; st <= 3'd3; end
          endcase
        end
        3'd3: begin // SAVE caller frame
          if (gi < NREG) begin sfile[sp*NREG + gi] <= file[gi]; gi <= gi + 16'd1; end
          else begin sret[sp] <= c_t1[15:0]; sdst[sp] <= c_cdst[15:0]; sp <= sp + 16'd1; ai <= 16'd0; st <= 3'd4; end
        end
        3'd4: begin // BIND args into the callee's parameter registers
          if (ai < NREG) begin
            file[ai] <= (ai < c_cargc[15:0]) ? resolve_arg(ai) : 64'sd0;
            ai <= ai + 16'd1;
          end else begin pc <= c_t0[15:0]; st <= 3'd0; end
        end
        3'd5: begin // RESTORE caller frame on return
          if (gi < NREG) begin file[gi] <= sfile[(sp-1)*NREG + gi]; gi <= gi + 16'd1; end
          else begin file[sdst[sp-1]] <= retval; pc <= sret[sp-1]; sp <= sp - 16'd1; st <= 3'd0; end
        end
      endcase
    end
  end
endmodule
"#,
        ports = ports.join(",\n    "),
        func_f_def = func_f_verilog(),
        MAXCELL_M1 = MAXCELL - 1,
    )
}

/// A self-checking testbench for the generic config-driven engine: it loads the params, pulses reset, runs to
/// `done`, and compares `result` to the `eval_wprog_config` oracle in `wpe_{name}_out.hex` (params in
/// `wpe_{name}_in.hex`; the program image in the `wpe_{name}_*.hex` config memories the engine loads).
pub fn emit_wprog_engine_testbench(name: &str, n_params: usize, n_vectors: usize) -> String {
    let mut decls = String::new();
    if n_params > 0 {
        decls.push_str(&format!(
            "  reg signed [63:0] pin [0:{}];\n",
            n_params * n_vectors - 1
        ));
    }
    decls.push_str(&format!(
        "  reg signed [63:0] oout [0:{}];\n",
        n_vectors - 1
    ));
    for p in 0..n_params {
        decls.push_str(&format!("  reg signed [63:0] p{p};\n"));
    }
    let mut dut_ports: Vec<String> = vec![".clk(clk)".into(), ".rst(rst)".into()];
    for p in 0..n_params {
        dut_ports.push(format!(".p{p}(p{p})"));
    }
    dut_ports.push(".result(result)".into());
    dut_ports.push(".done(done)".into());
    let readin = if n_params > 0 {
        format!("    $readmemh(\"wpe_{name}_in.hex\", pin);\n")
    } else {
        String::new()
    };
    let mut load = String::new();
    for p in 0..n_params {
        load.push_str(&format!("      p{p} = pin[i*{n_params}+{p}];\n"));
    }
    format!(
        r#"// tb_wpe_{name} — the GENERIC config-driven whole-program engine vs eval_wprog_config.
`timescale 1ns/1ps
module tb_wpe_{name};
  reg clk, rst;
{decls}  wire [63:0] result; wire done;
  integer i, errors, guard;
  wpe_{name} dut({ports});
  initial begin clk = 0; forever #5 clk = ~clk; end
  initial begin
    errors = 0;
{readin}    $readmemh("wpe_{name}_out.hex", oout);
    for (i = 0; i < {n_vectors}; i = i + 1) begin
{load}      // Drive reset and sample `done` on the NEGEDGE, away from the engine's posedge writes (no race).
      @(negedge clk); rst = 1;
      @(negedge clk); @(negedge clk); rst = 0;
      guard = 0;
      while (done !== 1'b1 && guard < 4000000) begin @(negedge clk); guard = guard + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: {n_vectors} config-driven whole-program runs match eval_wprog_config");
    else $display("FAIL: %0d config-driven mismatches", errors);
    $finish;
  end
endmodule
"#,
        ports = dut_ports.join(", "),
    )
}

impl WImage {
    /// The exact byte stream a host sends OVER THE WIRE to the `wpe_soc` engine: for each of the 24 config
    /// memories (the `hex_files` order) a 2-byte big-endian entry count then that many big-endian words (4 bytes
    /// for the 23 u32 memories, 8 for `konst`), then a 2-byte param count + that many 8-byte params, then the
    /// [`IMAGE_GO`] marker. The on-chip loader FSM reproduces the identical config in on-chip memory — so the chip
    /// SYNTHESIZES the program it is handed. Single source of truth for both the iverilog host and the FPGA host.
    pub fn wire_bytes(&self, params: &[i64]) -> Vec<u8> {
        self.wire_bytes_preloaded(params, &[])
    }

    /// As [`Self::wire_bytes`] but with an optional HEAP PRE-LOAD segment — the streamed-weights path. `heap_init` is
    /// loaded into `heap[0..heap_init.len()]` before the run (the DATA-segment model), so `loaded()` (heap
    /// pointer 0) reads it; the engine seeds its bump allocator past the pre-load and clears only the runtime
    /// region. The segment is BACKWARD-COMPATIBLE and marker-delimited: when `heap_init` is empty the stream is
    /// byte-identical to `wire_bytes` (…params + [`IMAGE_GO`]); when non-empty, an [`IMAGE_HEAP`] marker after the
    /// params introduces a u16 word-count then that many u64 words, then the [`IMAGE_GO`] byte. So a non-streamed
    /// program's wire format is unchanged (the loader sees [`IMAGE_GO`] right after the params) and only streamed
    /// programs carry the extra segment.
    pub fn wire_bytes_preloaded(&self, params: &[i64], heap_init: &[i64]) -> Vec<u8> {
        let mut out = Vec::new();
        let u16be = |out: &mut Vec<u8>, x: u16| out.extend_from_slice(&x.to_be_bytes());
        let mems32: [&Vec<u32>; 23] = [
            &self.blk_livein_ptr,
            &self.blk_n_livein,
            &self.blk_cell_ptr,
            &self.blk_n_cells,
            &self.blk_write_ptr,
            &self.blk_n_writes,
            &self.blk_term_kind,
            &self.blk_t0,
            &self.blk_t1,
            &self.blk_top,
            &self.blk_cdst,
            &self.blk_cargc,
            &self.blk_cargp,
            &self.cell_kind,
            &self.cell_code,
            &self.cell_op0,
            &self.cell_op1,
            &self.cell_op2,
            &self.cell_op3,
            &self.wr_reg,
            &self.wr_op,
            &self.arg_op,
            &self.liv_reg,
        ];
        for m in mems32 {
            u16be(&mut out, m.len() as u16);
            for &w in m {
                out.extend_from_slice(&w.to_be_bytes());
            }
        }
        u16be(&mut out, self.konst.len() as u16);
        for &k in &self.konst {
            out.extend_from_slice(&(k as u64).to_be_bytes());
        }
        u16be(&mut out, params.len() as u16);
        for &p in params {
            out.extend_from_slice(&(p as u64).to_be_bytes());
        }
        if !heap_init.is_empty() {
            out.push(IMAGE_HEAP);
            u16be(&mut out, heap_init.len() as u16);
            for &h in heap_init {
                out.extend_from_slice(&(h as u64).to_be_bytes());
            }
        }
        out.push(IMAGE_GO);
        out
    }
}

/// The combinational `func_f` for the over-the-wire SoC: no integer func code computes here. DivS/ModS/FloorDivS/Pow
/// (codes 0–3) run on the engine's sequential datapath (`wpe_soc_template.v` st==6, a magnitude-restoring divider and
/// a repeat-multiply), MulHiU (4) and MulOvfS (12) on its multiplier unit (ST_MU..ST_MUW), and the float lane on the
/// FPU, so no multiply or divide shares a cycle with the cell walk. Shared by all three SoC engines
/// (behavioral/synth/hw) so they agree byte-for-byte; the sequencing and results are proven ≡ `FuncUnit::eval`
/// (== the VM) under iverilog.
pub const MUL_FUNC_F: &str = r#"  // The integer multiplies (MulHiU, MulOvfS) run on the engine's multiplier unit (ST_MU..ST_MUW); every other
  // integer func code that reaches here answers 0.
  function signed [63:0] func_f(input [7:0] op, input signed [63:0] a, input signed [63:0] x);
    begin
      func_f = 64'sd0;
    end
  endfunction
"#;

/// The proven bit-exact sequential IEEE-754 f64 FPU (`fpu.v` beside this module; `benchmarks/silicon/fpu.v` is
/// its golden copy for the FPGA flows) — `fp_addsub`/`fp_mul`/`fp_div`/
/// `fp_sqrt` (start/done handshake) + combinational `fp_cmp`. Appended verbatim after the engine module so the
/// over-the-wire SoC can instantiate the float units; the engine's 64-bit register file / heap already carry f64
/// bit patterns, so the FPU is the only float hardware the engine needs. Bit-exact vs Rust `f64` over 41,521
/// vectors (its own `verify-fpu.sh`), and the exponent path is timing-safe (see the `fp_div` pipeline lock).
pub const FPU_V: &str = include_str!("fpu.v");

/// Emit the UNIVERSAL over-the-wire whole-program engine SoC: one fixed netlist that RECEIVES any fitting
/// program's flat image + params over a UART, fills its on-chip config memories (the loader FSM), seeds the
/// register file from the params (INIT), runs the interpret st-machine (== `eval_wprog_config` == the VM), and
/// streams the 64-bit result + run-cycle count back over the UART — then loops for the next program. The engine
/// core (block-record wires, `resolve`/`resolve_arg`, the `case(st)` walk) is the SAME logic proven in
/// `emit_wprog_engine_verilog`; only the config source (wire vs `$readmemh`) and the result path (UART vs port)
/// differ. Integer functional units (synthesizable — the sequential div/mod/floordiv/pow datapath lives in the
/// template; `func_f` is the DSP multiplies only). `CLKS_PER_BIT` sizes the UART bit period.
pub fn emit_wprog_engine_soc_verilog(name: &str) -> String {
    let template = engine_template(false);
    let engine = template
        .replace("__NAME__", name)
        .replace("__INTEGER_FUNC_F__", MUL_FUNC_F);
    // Excise the DEBUG_READBACK-only regions (this raw-replace path never declares the `DEBUG_READBACK`
    // parameter, so the readback FSM's `DEBUG_READBACK` references would fail elaboration otherwise). With them
    // gone the emitted RTL is byte-identical to before the readback feature. The CFGBRAM regions stay (their
    // `CFG_BRAM` parameter IS declared with default 0 and folds away) — matching this path's prior behaviour.
    let engine = strip_debug_readback_regions(&engine, false);
    // Excise the RDW_DETECT-only collision-detector regions too, so this path stays byte-identical to before the
    // detector feature (the `RDW_DETECT` parameter folds to 0 and its region body is a dead diagnostic cone).
    let engine = strip_marker_regions(
        &engine,
        "//__RDW_DETECT_ONLY_BEGIN__",
        "//__RDW_DETECT_ONLY_END__",
        false,
    );
    let engine = select_memory_regions(&engine, &[]);
    format!("{engine}\n{FPU_V}")
}

/// A self-checking testbench that acts as the OVER-THE-WIRE HOST: it `$readmemh`s the concatenated wire bytes
/// (`wire_<name>.hex`), per-vector byte lengths (`lens_<name>.hex`), and the `eval_wprog_config` oracle
/// (`oracle_<name>.hex`), then for each vector bit-bangs its bytes onto `uart_rx`, decodes the returned frame
/// (`0xA5` sync + `result[63:0]` + `cycles[31:0]` + `0x5A`), and checks the result == the oracle. Proves the WHOLE
/// wire→synthesize→run→wire path ≡ the VM under iverilog.
pub fn emit_wprog_engine_soc_testbench(
    name: &str,
    n_vectors: usize,
    clks_per_bit: usize,
    max_bytes: usize,
) -> String {
    emit_wprog_engine_soc_testbench_board(name, n_vectors, clks_per_bit, max_bytes, false)
}

/// [`emit_wprog_engine_soc_testbench`] on a board with the PSRAM (`psram`, for `wpe_offchip`), whose
/// testbench also wires the part's pads, watches every received byte and leaves `__HALF_NS__` to fill.
pub(crate) fn emit_wprog_engine_soc_testbench_board(
    name: &str,
    n_vectors: usize,
    clks_per_bit: usize,
    max_bytes: usize,
    psram: bool,
) -> String {
    let template = fill_verilog(include_str!("tb_wpe_soc_template.v"));
    let template = strip_marker_regions(
        &template,
        "//__OFFCHIP_ANY_BEGIN__",
        "//__OFFCHIP_ANY_END__",
        psram,
    );
    strip_marker_regions(
        &template,
        "//__ONCHIP_ALL_BEGIN__",
        "//__ONCHIP_ALL_END__",
        !psram,
    )
    .replace("__NAME__", name)
    .replace("__NVEC__", &n_vectors.to_string())
    .replace("__CLKS__", &clks_per_bit.to_string())
    .replace("__MAXBYTES__", &max_bytes.to_string())
}

// ── The SYNTHESIZABLE over-the-wire engine (real Nexys A7 bitstream) ────────────────────────────────────────
// Two adjustments vs the iverilog engine make it FPGA-buildable through the open flow (yosys→nextpnr-xilinx):
//  1. `func_f` is `MUL_FUNC_F` — the DSP multiplies (MulHiU/MulOvfS) only. The combinational 64-bit divider
//     (`a/x`, `a%x` — which yosys can't map in bounded time) and the runtime-bounded `Pow` loop are GONE;
//     Div/Mod/FloorDiv/Pow run instead on the template's SEQUENTIAL datapath (st==6, a magnitude-restoring
//     divider + repeat-multiply), so the full integer arithmetic surface synthesizes. The float lane is the FPU
//     hardware follow-on (fitting rejects float/popcount func codes).
//  2. A smaller cap profile (`WPE_SYNTH_*`) so every UART-written config memory can be forced to REGISTERS
//     (`ram_style="registers"`) — the open flow silently drops distributed-RAM writes, so the streamed config
//     must live in flip-flops. Full-cap registers would exceed the device; this profile fits (~23k FF / ~31k LUT).

pub const WPE_SYNTH_NREG: usize = 16;
pub const WPE_SYNTH_MAXBLK: usize = 8;
pub const WPE_SYNTH_MAXCELL: usize = 32;
pub const WPE_SYNTH_MAXWR: usize = 32;
pub const WPE_SYNTH_MAXARG: usize = 8;
pub const WPE_SYNTH_MAXLIV: usize = 16;
pub const WPE_SYNTH_MAXK: usize = 32;
pub const WPE_SYNTH_HEAP: usize = 64;
pub const WPE_SYNTH_DEPTH: usize = 8;

// The DEPLOYED Nexys A7-100T cap profile — VERIFIED on real silicon. One hard synthesis constraint shapes
// MAXCELL: yosys `synth_xilinx` MIS-SYNTHESIZES the loop's register-vs-register compare below MAXCELL=32 (at
// MAXCELL=16 the loop silently corrupts once the real `func_f` DSP logic is present, even though a func_f-stubbed
// gate-sim passes), so MAXCELL stays 32. HEAP=64/DEPTH=8 is the FULL profile: the BRAM redesign (heap/`sfile` →
// block RAM, ~11.8k FF + 2 RAMB36, ≡VM, locked by `the_big_engine_fits_the_hold_safe_ff_budget_via_block_ram`)
// moves the FF-heavy state off flip-flops so the big engine both HOLDS (under the ~12.7k threshold) AND ROUTES on
// the open flow — proven on Board B streaming depth-8 recursion (f(8)=36) + loops/collatz over the wire
// (`wpe_soc_big.bit`, coreclk 16-24 MHz PASS). The deployed hw engine sets bram=true. (The earlier "does not
// route" was a mis-diagnosis: a bloated 92-RAMB build variant + a good build killed early — the correct 2-RAMB /
// 30%-LUT netlist routes given the full nextpnr grind.)
pub const WPE_HW_NREG: usize = 16;
pub const WPE_HW_MAXBLK: usize = 8;
pub const WPE_HW_MAXCELL: usize = 32;
pub const WPE_HW_MAXWR: usize = 32;
pub const WPE_HW_MAXARG: usize = 8;
pub const WPE_HW_MAXLIV: usize = 16;
pub const WPE_HW_MAXK: usize = 32;
pub const WPE_HW_HEAP: usize = 64; // block RAM (routes + holds on silicon — verified)
pub const WPE_HW_DEPTH: usize = 8; // block RAM (depth-8 recursion runs on the board — verified)

/// Returns true iff `cfg` fits the DEPLOYED hold-safe hardware engine's cap profile (`WPE_HW_*`) — the tighter
/// bounds the real bitstream can hold. Programs must satisfy these to run over the wire on silicon.
pub fn wprog_config_fits_hw_engine(cfg: &WProgConfig) -> Result<(), String> {
    if cfg.blocks.len() > WPE_HW_MAXBLK {
        return Err(format!(
            "{} blocks > hw MAXBLK={WPE_HW_MAXBLK}",
            cfg.blocks.len()
        ));
    }
    if wprog_nregs(cfg) > WPE_HW_NREG {
        return Err(format!(
            "{} registers > hw NREG={WPE_HW_NREG}",
            wprog_nregs(cfg)
        ));
    }
    let total_cells: usize = cfg.blocks.iter().map(|b| b.cells.len()).sum();
    if total_cells > WPE_HW_MAXCELL {
        return Err(format!("{total_cells} cells > hw MAXCELL={WPE_HW_MAXCELL}"));
    }
    // The hw ENGINE carries the IEEE-754 FPU (the sim netlist proves float programs on it); whether a given BOARD
    // image has it is a wrapper decision (`nexys_wpe_soc.v` WITH_FPU) recorded in the bitstream's recipe, and the
    // host refuses float programs for an `FPU=0` image from that record (`emit_wpe_soc --caps FPU=0`).
    for b in &cfg.blocks {
        for c in &b.cells {
            if c.kind == 2 && !matches!(c.code, 0..=10 | 12..=20) {
                return Err(format!(
                    "Func cell code {} (ApproxEqF/unknown) is the remaining float follow-on; the hw engine has integer + \
                     the IEEE-754 FPU (add/sub/mul/div/sqrt/floor/compare/convert)",
                    c.code
                ));
            }
        }
    }
    Ok(())
}

// ── WPE_STREAM: the whole STREAMED-WEIGHTS transformer's cap profile ─────────────────────────────────────
// The whole baby transformer runs on ONE chip when its ~3,136 weights are STREAMED (pre-loaded into the heap
// via `loaded()`, the DATA-segment model) instead of Push-built into the config. Streaming removes the giant
// 18,934-cell weight-load block, so the config collapses to flip-flop scale (measured on the generated net:
// 158 blocks / 124 registers / 556 cells / 644 writes / 258 live-ins / 0 call-args / 41 constants) and only
// the runtime heap (right-sized to ~18,300 words by `with_seq_reserve(8)`) needs block RAM. Most caps size
// the emitted RTL's config memories with a SMALL headroom above those measured counts — MAXBLK 160 (actual
// 158), MAXWR 704 (actual 644), MAXLIV 288 (actual 258) — tightened so the still-combinational `wr_*`/`blk_*`/
// `liv` read muxes shrink, easing open-flow placement; the heap is the block-RAM state plane (`MemBackend::Bram`).
// Proven ≡VM as the engine model by `wpe_transformer_lock`'s streamed leg.
//
// SIZED FOR THE WHOLE FAMILY, NOT JUST THE TRANSFORMER. The engine is the ONE netlist that runs EVERY program
// fitting these bounds — so the register/const/per-block caps carry generous headroom above the transformer's
// own counts. A straight-line block N deep (e.g. N heap reads then an N-term running sum) lowers register→SSA
// to ~5N/2 SSA values, cells, and writes and ~N constants — with NO register coalescing (one FU per SSA value
// is the dataflow model), so those dimensions grow FAR past the transformer's. The `postsynth_high_register_
// index_matches_vm` lock's 90-value sum is such a program: 453 registers / 270 per-block cells / 452 per-block
// writes / 91 constants. `NREG` (register file `file` + write buffer `wbuf` depth), `MAXK` (const table), and
// `MAXV` (per-block value scratch `v`) are therefore sized to 512/128/512 — a clean power-of-two register file
// (proven to round-trip all indices distinctly at the synthesized netlist level, RAMB18 SDP-36, no mis-wire)
// that covers such programs. The `unchecked` image builder (used by `emit_transformer_wire` and the postsynth
// locks) does NOT clamp to these, so a program that OVERFLOWS a cap silently aliases its own registers/cells —
// which is exactly the corruption `postsynth_high_register_index_matches_vm` caught (453 > the old NREG 128).
pub const WPE_STREAM_NREG: usize = 512;
pub const WPE_STREAM_MAXBLK: usize = 160;
pub const WPE_STREAM_MAXCELL: usize = 1024;
pub const WPE_STREAM_MAXWR: usize = 704;
pub const WPE_STREAM_MAXARG: usize = 24;
pub const WPE_STREAM_MAXLIV: usize = 288;
/// Const-table depth (`konst[0:MAXK-1]`). The transformer uses 41; a wider straight-line program (the postsynth
/// high-register lock's 90-value sum) needs 91 distinct constants, so MAXK=128 covers the family. The `unchecked`
/// image builder skips the `konst.len() <= MAXK` guard, so an over-MAXK program would silently read a nonexistent
/// const slot — 91 > the old 64 was one of the four caps the high-register lock overflowed.
pub const WPE_STREAM_MAXK: usize = 128;
/// Heap depth (words) for the streamed engine. Sized to `2^15 = 32768` — a POWER OF TWO that maps to exactly ONE
/// RAMB36E1 depth-bank at ×1 width (a single RAMB36 is 32768×1), so the whole 15-bit `heap_ra` addresses one clean
/// tile with NO depth-cascade. This dodges a yosys 0.33 `synth_xilinx` mis-map: for a heap whose depth is NOT a
/// clean single-tile fit (e.g. the old 20480), `memory_libmap` picks a wide tile (×9 → 4096 deep) and then FAILS
/// to depth-cascade — it wires every tile to the SAME 12-bit address (`heap_ra[11:0]`) and drops the top address
/// bits (`heap_ra[14:12]`), so on silicon (and in post-synth gate-level sim) any heap read/write to an address
/// >= 4096 aliases into the first 4096-word tile and returns garbage. The whole-program transformer's heap grows
/// past 4096, so it read uninitialized cells and returned the wrong result on the XC7A200T. 32768 >= the model's
/// working-set (was 20480), fits the BRAM budget, and — being a single-tile power of two — maps with the full
/// 15-bit address on every port. Proven at the netlist level (the synthesized 20480 heap wires `heap_ra[11:0]`
/// and drops the top bits; the 32768 heap wires the full `heap_ra`) and by a post-synth gate-level RAMB sim.
///
/// DEPTH-BANKING (silicon fix below the netlist). A 32768-deep heap half at ×1 mapped to a RAMB36E1 in ×1-width,
/// 32768-deep mode — whose INTERNAL two-16K-half depth-cascade (address bit 15 tied high on the upper tiles) is
/// the least-fuzzed RAMB corner. prjxray/openXC7 mis-encodes it on the XC7A200T (a small read-only probe reads
/// wrong by ≈1.5·N) even though the gate-level netlist is ≡VM — the defect is BELOW the netlist. The template
/// therefore DEPTH-BANKS the heap into two 16384-deep banks (`heap_b0_*`/`heap_b1_*`, selected by heap-address
/// bit 14; in-bank index = bits `[13:0]`) so each maps to a 14-bit-indexed tile with NO 32K depth-cascade — a
/// better-fuzzed primitive. Behaviorally IDENTICAL to a flat 32768 heap. HEAP stays 32768 (two 16384 banks).
pub const WPE_STREAM_HEAP: usize = 32768;
/// Call-frame depth. Sized from the MEASURED peak across the benchmark suite (26, driven by `fib`), rounded to
/// a power of two — not from the transformer, which is call-free and needs none. This was 4 while the smaller
/// XC7A100T profile was 8, and because the wire image carries no record of the profile it was built for, a
/// depth-8 program streamed to the depth-4 engine ran and returned a plausible wrong number: on silicon
/// `bench_fib` gave 8 instead of 21, `wpe_rec8` 20 instead of 36, `wpe_recursion` 11 instead of 15. Both
/// `wpe_cap_profile_lock` (no dimension may shrink between profiles) and
/// `the_stream_engine_profile_covers_every_benchmark_in_the_suite` (the suite's measured ceiling) now pin it.
pub const WPE_STREAM_DEPTH: usize = 32;
/// Depth of the per-block cell-value scratch array `v` (`v[0:MAXV-1]`). `v` is indexed ONLY by a cell's position
/// WITHIN ITS BLOCK (mode-2 operands + the `v[ci]` write), never by the flat `blk_cell_ptr + ci`, so it needs to
/// hold only the MAX cells in any SINGLE block — not the whole-program `MAXCELL` total. The streamed transformer's
/// largest block is 116 cells; a wider straight-line block (the postsynth high-register lock's 90-value sum lowers
/// to a single 270-cell block) needs more, so MAXV=512 covers the family. Under `ConfigBram` (the streamed engine
/// + every postsynth lock) `v` is BLOCK RAM with a registered read (`v_ra`/`v_rd`), so the depth costs a RAMB tile,
/// not a combinational mux; the flip-flop-backed combinational `resolve`/`resolve0` reads only exist under the
/// `Registers`/`Bram` backends, which use the SMALL `WPE_SYNTH_*` profile — untouched by this. A block deeper than
/// MAXV aliases its own cells (the corruption `postsynth_high_register_index_matches_vm` caught: 270 > the old 128).
pub const WPE_STREAM_MAXV: usize = 512;

/// The `WPE_STREAM_*` profile as the `[usize; 10]` caps array `emit_wprog_engine_soc_verilog_backend` expects
/// (order: NREG, MAXBLK, MAXCELL, MAXWR, MAXARG, MAXLIV, MAXK, HEAP, DEPTH, MAXV).
pub const WPE_STREAM_CAPS: [usize; 10] = [
    WPE_STREAM_NREG,
    WPE_STREAM_MAXBLK,
    WPE_STREAM_MAXCELL,
    WPE_STREAM_MAXWR,
    WPE_STREAM_MAXARG,
    WPE_STREAM_MAXLIV,
    WPE_STREAM_MAXK,
    WPE_STREAM_HEAP,
    WPE_STREAM_DEPTH,
    WPE_STREAM_MAXV,
];

/// Returns `Ok` iff `cfg` fits the `WPE_STREAM_*` profile (the whole streamed transformer's engine). Every
/// dimension is a program TOTAL (the flat config memories accumulate across blocks). Named-loudly on overflow
/// so a growing model is caught before it silently corrupts the flat config image.
pub fn wprog_config_fits_stream_engine(cfg: &WProgConfig) -> Result<(), String> {
    let checks: [(usize, usize, &str); 5] = [
        (cfg.blocks.len(), WPE_STREAM_MAXBLK, "blocks"),
        (wprog_nregs(cfg), WPE_STREAM_NREG, "registers"),
        (
            cfg.blocks.iter().map(|b| b.cells.len()).sum(),
            WPE_STREAM_MAXCELL,
            "cells",
        ),
        (
            cfg.blocks.iter().map(|b| b.writes.len()).sum(),
            WPE_STREAM_MAXWR,
            "writes",
        ),
        (
            cfg.blocks.iter().map(|b| b.livein.len()).sum(),
            WPE_STREAM_MAXLIV,
            "live-ins",
        ),
    ];
    for (got, cap, what) in checks {
        if got > cap {
            return Err(format!("{got} {what} > stream engine cap {cap}"));
        }
    }
    // The per-block value array `v` (depth MAXV) is indexed WITHIN a block, so the binding dimension is the MAX
    // cells in any single block, not the whole-program total. A block deeper than MAXV would alias its own cells
    // and corrupt the walk — reject it loudly here, before it can silently mis-run.
    let max_block_cells = cfg.blocks.iter().map(|b| b.cells.len()).max().unwrap_or(0);
    if max_block_cells > WPE_STREAM_MAXV {
        return Err(format!(
            "a block has {max_block_cells} cells > stream engine per-block cap MAXV={WPE_STREAM_MAXV}"
        ));
    }
    Ok(())
}

/// Returns true iff `cfg` fits the SYNTHESIZABLE engine's smaller cap profile (a superset check of the generic
/// engine's fit): every over-the-wire program synthesized to hardware must satisfy these or it would overflow
/// the register-backed config memories. The integer func units — Div/Mod/FloorDiv/Pow (sequential datapath) and
/// MulHiU/MulOvfS (DSP) — all synthesize; only the float/popcount codes are rejected (the FPU follow-on).
pub fn wprog_config_fits_synth_engine(cfg: &WProgConfig) -> Result<(), String> {
    if cfg.blocks.len() > WPE_SYNTH_MAXBLK {
        return Err(format!(
            "{} blocks > synth MAXBLK={WPE_SYNTH_MAXBLK}",
            cfg.blocks.len()
        ));
    }
    if wprog_nregs(cfg) > WPE_SYNTH_NREG {
        return Err(format!(
            "{} registers > synth NREG={WPE_SYNTH_NREG}",
            wprog_nregs(cfg)
        ));
    }
    let total_cells: usize = cfg.blocks.iter().map(|b| b.cells.len()).sum();
    if total_cells > WPE_SYNTH_MAXCELL {
        return Err(format!(
            "{total_cells} cells > synth MAXCELL={WPE_SYNTH_MAXCELL}"
        ));
    }
    for b in &cfg.blocks {
        for c in &b.cells {
            if c.kind == 2 && !matches!(c.code, 0..=10 | 12..=20) {
                return Err(format!("Func cell code {} (ApproxEqF/unknown) is the remaining float follow-on; the synth engine has integer + the IEEE-754 FPU (add/sub/mul/div/sqrt/floor/int→float/compares/popcount)", c.code));
            }
        }
    }
    Ok(())
}

/// Emit the SYNTHESIZABLE over-the-wire engine SoC (real Nexys A7 bitstream): the same universal loader/INIT/
/// interpret/TX logic as [`emit_wprog_engine_soc_verilog`], but with `SYNTH_FUNC_F`, the `WPE_SYNTH_*` cap
/// profile, and `(* ram_style="registers" *)` forced on every config/state memory so the open flow keeps the
/// UART-streamed config in flip-flops. Proven ≡ `eval_wprog_config` for fitting programs under iverilog, then
/// synthesized through yosys→nextpnr-xilinx to a bitstream.
pub fn emit_wprog_engine_soc_verilog_synth(name: &str) -> String {
    // BRAM: the SYNTH engine's big HEAP=64/DEPTH=8 heap/sfile map to block RAM (11.8k FF, hold-safe in synthesis).
    emit_wprog_engine_soc_with_caps(
        name,
        [
            WPE_SYNTH_NREG,
            WPE_SYNTH_MAXBLK,
            WPE_SYNTH_MAXCELL,
            WPE_SYNTH_MAXWR,
            WPE_SYNTH_MAXARG,
            WPE_SYNTH_MAXLIV,
            WPE_SYNTH_MAXK,
            WPE_SYNTH_HEAP,
            WPE_SYNTH_DEPTH,
            WPE_SYNTH_MAXCELL,
        ],
        true,
    )
}

/// Emit the DEPLOYED hold-safe over-the-wire engine SoC (the real bitstream): same logic as the synth engine,
/// but with the `WPE_HW_*` cap profile that both synthesizes the loop's compare correctly under yosys AND stays
/// under the board's hold-safe FF threshold. Runs loops/branches/writes, recursion (depth ≤3), and small
/// collections over the wire on real silicon; proven ≡ `eval_wprog_config` (RTL) + loop gate-sim ≡VM.
pub fn emit_wprog_engine_soc_verilog_hw(name: &str) -> String {
    // BRAM: the deployed engine keeps heap/`sfile` in BLOCK RAM (HEAP=64/DEPTH=8) so it both holds (~11.8k FF) AND
    // routes on the open flow — proven on Board B (depth-8 recursion + loops streamed over the wire). Same profile
    // as the synth engine now: the BRAM redesign made the full-corpus engine deployable on silicon.
    emit_wprog_engine_soc_with_caps(
        name,
        [
            WPE_HW_NREG,
            WPE_HW_MAXBLK,
            WPE_HW_MAXCELL,
            WPE_HW_MAXWR,
            WPE_HW_MAXARG,
            WPE_HW_MAXLIV,
            WPE_HW_MAXK,
            WPE_HW_HEAP,
            WPE_HW_DEPTH,
            WPE_HW_MAXCELL,
        ],
        true,
    )
}

/// The MEMORY BACKEND for the config-driven engine's on-chip memories — a first-class, selectable choice of
/// substrate. The whole-program engine holds two kinds of memory: the STATE PLANE (the shared `heap` + the
/// recursion `sfile` frame stack) and the CONFIG memories (the UART-streamed program image: `blk_*`, `cell_*`,
/// live-ins, writes, constants). Each can live in LUT flip-flops or in block RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemBackend {
    /// Everything in LUT flip-flops: combinational reads, smallest latency, but the capacity is bounded by the
    /// FF budget (the `WPE_SYNTH_*` profile). The open-flow-safe default for tiny programs.
    Registers,
    /// The STATE PLANE (`heap` + `sfile`) in BLOCK RAM (registered reads, 1-cycle latency absorbed by the FSM's
    /// wait-states); config stays in flip-flops. Moves the FF-heavy state off flip-flops so a bigger program
    /// both HOLDS and ROUTES on the open flow — the DEPLOYED silicon backend (`wpe_soc_big.bit`, proven Board B).
    Bram,
    /// State plane AND the config image in BLOCK RAM — the large-capacity backend that lets the whole 158-block
    /// transformer fit one chip's BRAM. Requires registered CONFIG reads (block/cell fetch wait-states) — the
    /// interpret-FSM extension that is the remaining build; the BRAM budget already fits (4 Mb of 4.9 Mb).
    ConfigBram,
}

/// Shared body: instantiate the engine template with `MUL_FUNC_F` (DSP multiplies; div/mod/floordiv/pow are the
/// template's sequential datapath), a cap profile `[NREG, MAXBLK, MAXCELL, MAXWR, MAXARG, MAXLIV, MAXK, HEAP,
/// DEPTH, MAXV]`, and the chosen memory backend (which config/state memories map to block RAM vs flip-flops).
///
/// A caller that wants an explicit backend uses [`emit_wprog_engine_soc_verilog_backend`]. The `bram: bool`
/// overload is kept as a thin shim (`true` = [`MemBackend::Bram`], `false` = [`MemBackend::Registers`]).
fn emit_wprog_engine_soc_with_caps(name: &str, caps: [usize; 10], bram: bool) -> String {
    emit_wprog_engine_soc_with_backend(
        name,
        caps,
        if bram {
            MemBackend::Bram
        } else {
            MemBackend::Registers
        },
        false,
        false,
        &[],
    )
}

/// Emit the over-the-wire engine with an EXPLICIT memory backend — the first-class "choose the substrate" entry
/// point. `Registers`/`Bram` are the proven paths; `ConfigBram` additionally maps the config image to block RAM
/// (its registered-read FSM is the in-progress large-capacity build). `caps[9]` (MAXV) sizes the per-block
/// value array `v` independently of the whole-program `MAXCELL` total.
pub fn emit_wprog_engine_soc_verilog_backend(
    name: &str,
    caps: [usize; 10],
    backend: MemBackend,
) -> String {
    emit_wprog_engine_soc_with_backend(name, caps, backend, false, false, &[])
}

/// Emit the over-the-wire engine with an EXPLICIT backend AND the SIM-ONLY read-during-write collision detectors
/// active (`RDW_DETECT=1`). The detectors compare each memory's registered-read address (`heap_ra`/`sf_ra`/
/// `file_ra`/`v_ra`) against every same-cycle write address during the interpret phase and `$display` +
/// count a `RDW-COLLISION` whenever they coincide — a cross-port read-during-write that a real RAMB36 returns
/// UNDEFINED for but this behavioral sim idealizes as READ_FIRST (old data). This is a DIAGNOSTIC build only;
/// `rdw_detect=false` (every other entry point) EXCISES the whole detector region so the shipped netlist is
/// BYTE-IDENTICAL to before this feature (the `RDW_DETECT` parameter is not even injected into the header).
pub fn emit_wprog_engine_soc_verilog_backend_rdw(
    name: &str,
    caps: [usize; 10],
    backend: MemBackend,
) -> String {
    emit_wprog_engine_soc_with_backend(name, caps, backend, false, true, &[])
}

/// Emit the over-the-wire engine with an EXPLICIT memory backend AND an on-chip READBACK debug mode. When
/// `debug_readback` is true the engine, at the load→run boundary (P_CLR done), FIRST streams a DEBUG FRAME back
/// over the SAME UART tx — a fixed diagnostic slice of each major memory — then hands off to the interpret FSM,
/// so real silicon can be compared against the known config-image/heap values byte-for-byte. The DEBUG FRAME is:
/// sync `0xD8`, then for each memory in order {cell_kind, cell_op0, cell_op1, blk_term_kind, heap} entries
/// `[0..8)`, each as a 64-bit BIG-ENDIAN word (8 bytes MSB-first, matching the result frame's byte order), then
/// end byte `0x8D` — 1 + 5*8*8 + 1 = 322 bytes. The debug FSM walks each address through a REGISTERED BRAM read
/// (issue → wait the latency → capture) and reuses the existing tx byte-sender. `debug_readback=false` emits RTL
/// BYTE-IDENTICAL to [`emit_wprog_engine_soc_verilog_backend`] (the whole debug region is excised + the parameter
/// is not injected). Readback is only meaningful on the `ConfigBram` streamed engine — asserted here.
pub fn emit_wprog_engine_soc_verilog_backend_debug(
    name: &str,
    caps: [usize; 10],
    backend: MemBackend,
    debug_readback: bool,
) -> String {
    assert!(
        !debug_readback || matches!(backend, MemBackend::ConfigBram),
        "DEBUG_READBACK is a diagnostic for the ConfigBram streamed engine (its config/heap memories are the \
         block RAMs a synthesis mis-map corrupts); it is not defined for the flip-flop backends"
    );
    emit_wprog_engine_soc_with_backend(name, caps, backend, debug_readback, false, &[])
}

/// The engine with an explicit backend and the memories in `off_chip` behind the PSRAM edge, for
/// `wpe_offchip`, which fills the `//__OFFCHIP_GLUE__` line and appends the modules it names.
pub(crate) fn emit_wprog_engine_soc_offchip_body(
    name: &str,
    caps: [usize; 10],
    backend: MemBackend,
    off_chip: &[ArrayRole],
) -> String {
    emit_wprog_engine_soc_with_backend(name, caps, backend, false, false, off_chip)
}

fn emit_wprog_engine_soc_with_backend(
    name: &str,
    caps: [usize; 10],
    backend: MemBackend,
    debug_readback: bool,
    rdw_detect: bool,
    off_chip: &[ArrayRole],
) -> String {
    emit_wprog_engine_soc_with_probes(name, caps, backend, debug_readback, rdw_detect, off_chip, false)
}

/// The engine template with its protocol bytes filled, and its `PROBE_ONLY` regions — the observer ports
/// `probe_{running, st, pc, cell, sp, hp, cycles}` and their assignments — kept or excised. Excised, the engine is
/// byte-identical to one emitted before the probes existed.
fn engine_template(probes: bool) -> String {
    strip_marker_regions(
        &fill_verilog(include_str!("wpe_soc_template.v")),
        "//__PROBE_ONLY_BEGIN__",
        "//__PROBE_ONLY_END__",
        probes,
    )
}

/// The deployed hold-safe engine ([`emit_wprog_engine_soc_verilog_hw`]) with its observer ports: the same logic and
/// caps, plus `probe_{running, st, pc, cell, sp, hp, cycles}` outputs wired straight from the engine's own registers, for
/// the Fabric Terminal's activity panel.
pub fn emit_wprog_engine_soc_verilog_hw_probed(name: &str) -> String {
    emit_wprog_engine_soc_with_probes(
        name,
        [
            WPE_HW_NREG,
            WPE_HW_MAXBLK,
            WPE_HW_MAXCELL,
            WPE_HW_MAXWR,
            WPE_HW_MAXARG,
            WPE_HW_MAXLIV,
            WPE_HW_MAXK,
            WPE_HW_HEAP,
            WPE_HW_DEPTH,
            WPE_HW_MAXCELL,
        ],
        MemBackend::Bram,
        false,
        false,
        &[],
        true,
    )
}

fn emit_wprog_engine_soc_with_probes(
    name: &str,
    caps: [usize; 10],
    backend: MemBackend,
    debug_readback: bool,
    rdw_detect: bool,
    off_chip: &[ArrayRole],
    probes: bool,
) -> String {
    let [nreg, maxblk, maxcell, maxwr, maxarg, maxliv, maxk, heap, depth, maxv] = caps;
    let base = engine_template(probes);
    let mut s = base
        .replace("__NAME__", name)
        .replace("__INTEGER_FUNC_F__", MUL_FUNC_F)
        .replace(
            "localparam NREG=48, MAXBLK=16, MAXCELL=64, MAXWR=64, MAXARG=16, MAXLIV=48, MAXK=64, HEAP=128, DEPTH=24, MAXV=64;",
            &format!(
                "localparam NREG={nreg}, MAXBLK={maxblk}, MAXCELL={maxcell}, MAXWR={maxwr}, MAXARG={maxarg}, MAXLIV={maxliv}, MAXK={maxk}, HEAP={heap}, DEPTH={depth}, MAXV={maxv};"
            ),
        );
    // Under `ConfigBram` the config image (`cell_*`) is block RAM with a REGISTERED read, so the interpret FSM
    // must route the cell walk through the one-cycle cell-fetch wait-state (`ST_CF`). That path is guarded by the
    // module parameter `CFG_BRAM`; bake its DEFAULT to 1 so every instantiation (incl. the testbench, which does
    // not override it) takes the registered-fetch path. `Registers`/`Bram` leave the default 0 → identical to today.
    if matches!(backend, MemBackend::ConfigBram) {
        s = s.replace("parameter CFG_BRAM = 0", "parameter CFG_BRAM = 1");
    }
    // DEBUG_READBACK: inject the compile-time parameter into the module header (mirroring how the ConfigBram
    // default is flipped by `.replace`), so a normal build's header stays byte-identical (no injection). Baked to
    // 1 as the default so a testbench that does not override it still takes the readback path. Only ConfigBram
    // reaches here (asserted in the public entry), so the injection anchors on the CFG_BRAM=1 header text.
    if debug_readback {
        s = s.replace(
            "parameter WITH_FPU = 1, parameter CFG_BRAM = 1, parameter RDW_DETECT = 0) (",
            "parameter WITH_FPU = 1, parameter CFG_BRAM = 1, parameter RDW_DETECT = 0, parameter DEBUG_READBACK = 1) (",
        );
    }
    // RDW_DETECT: flip the sim-only read-during-write collision-detector parameter's default to 1 for a
    // diagnostic build (so a testbench that does not override it takes the detector path). A normal build leaves
    // it 0 and — crucially — has the whole detector region EXCISED below, so its header + body stay byte-identical.
    if rdw_detect {
        s = s.replace("parameter RDW_DETECT = 0", "parameter RDW_DETECT = 1");
    }
    // Excise the RDW_DETECT-only regions (the collision-detector shadow regs, counters, per-site shadow sets, and
    // the bottom-of-always check) unless this is an RDW diagnostic build — keeping the shipped netlist
    // byte-identical to before this feature (and never referencing `RDW_DETECT`).
    let s = strip_marker_regions(
        &s,
        "//__RDW_DETECT_ONLY_BEGIN__",
        "//__RDW_DETECT_ONLY_END__",
        rdw_detect,
    );
    // Keep one home for each memory that can leave the die: its on-chip array, or the port to its cache
    // behind the PSRAM edge, frozen with the rest of the FSM by `mstall`.
    let s = select_memory_regions(&s, off_chip);
    // Excise the DEBUG_READBACK-only regions (the debug read ports, state regs, P_DBG phase, and P_CLR detour)
    // unless this is a debug build — keeping the normal netlist byte-identical to before this feature.
    let s = strip_debug_readback_regions(&s, debug_readback);
    // The register-file/cell-value BRAM machinery (the `file_ra`/`file_rd`/`v_ra`/`v_rd` read ports, the `wbuf`
    // read-before-write buffer, the registered `file_rd`/`v_rd` reads, and the ST_OP.../ST_AG. fetch wait-states)
    // is used ONLY under CFG_BRAM=1. Under the other backends every one of those states is unreachable and every
    // one of those regs is dead — but leaving the DECL + logic in place gives the open synthesis flow's tech
    // mapper (ABC) a large dead cone to chew on before DCE removes it, blowing the CFG_BRAM=0 synth from ~2 min
    // to 10+ min. So we physically EXCISE the `//__CFGBRAM_ONLY_BEGIN__`…`//__CFGBRAM_ONLY_END__` regions from
    // the emitted RTL for non-ConfigBram backends — making the CFG_BRAM=0 netlist byte-identical to before this
    // feature. Under ConfigBram we keep the region bodies and only drop the marker comment lines. The shared FSM's
    // `if (CFG_BRAM) st <= ST_*` branches reference these states, but those localparams are OUTSIDE the excised
    // regions (kept) and the branches themselves fold away under CFG_BRAM=0, so the excision leaves valid RTL.
    let config_bram = matches!(backend, MemBackend::ConfigBram);
    let s = strip_cfgbram_regions(&s, config_bram);
    // Choose each memory array's substrate from the backend — no hard-wiring. The STATE PLANE (`heap`/`sfile`)
    // reads through registered ports (`heap_rd`/`sf_rd`) whose 1-cycle latency the FSM already absorbs, so it
    // maps to BLOCK RAM under `Bram`/`ConfigBram`. The CONFIG image (`blk_*`/`cell_*`/live-ins/writes/consts) is
    // registered-read only under `ConfigBram` (the block/cell fetch wait-states in the template) — under the
    // other backends it stays flip-flops so its combinational reads are sound.
    let state_plane_bram = matches!(backend, MemBackend::Bram | MemBackend::ConfigBram);
    let engine = s
        .lines()
        .map(|line| {
            let t = line.trim_start();
            if t.starts_with("reg ") && line.contains("[0:") && line.trim_end().ends_with("];") {
                let indent = &line[..line.len() - t.len()];
                // The array NAME is the identifier just before `[0:` (declarations are column-padded, so match
                // the token, not a fixed-space substring).
                let arr = t[..t.find("[0:").unwrap()]
                    .split_whitespace()
                    .last()
                    .unwrap_or("");
                // `heap` and `sfile` are each split into two 32-bit halves (`sfile_lo`/`sfile_hi`; heap into
                // `heap_b*_lo`/`heap_b*_hi`) so neither maps to a single 64-bit RAMB36 in SDP-72 mode (whose
                // parity-lane write yosys 0.33 mis-wires — heap reproduced on real Artix-7 silicon, sfile at
                // gate level). The heap is ADDITIONALLY depth-banked into two 16384-deep banks (`heap_b0_*`/
                // `heap_b1_*`, selected by heap-address bit 14) so it never maps to a RAMB36E1 in ×1-width,
                // 32768-deep mode — the internal two-16K-half depth-cascade that prjxray/openXC7 mis-encodes on
                // the XC7A200T (correct at gate level, wrong on silicon). Every half of every bank is the SAME
                // state-plane substrate the un-split array was.
                let is_state_plane = arr == "heap"
                    || arr == "heap_lo"
                    || arr == "heap_hi"
                    || arr == "heap_b0_lo"
                    || arr == "heap_b0_hi"
                    || arr == "heap_b1_lo"
                    || arr == "heap_b1_hi"
                    || arr == "sfile"
                    || arr == "sfile_lo"
                    || arr == "sfile_hi";
                // `cell_*` (the `ST_CF` cell-fetch wait-state), `wr_*` (the `ST_WF` write-fetch wait-state), and the
                // 13 `blk_*` block-record arrays (the `ST_BF` block-fetch wait-state) each have a REGISTERED read,
                // so they may become block RAM under ConfigBram. The remaining config memories (`arg_op`/`liv_reg`/
                // `konst`) are still read COMBINATIONALLY — a block-tagged combinational-read array has no valid
                // RAMB mapping (yosys errors), and `liv_reg`/`konst` deliberately stay flip-flops to keep `resolve`
                // unchanged. They fit in FF once `cell_*`/`wr_*`/`blk_*` (the dominant config memories) are off
                // flip-flops; a registered read for `arg_op` is the remaining follow-on.
                // The register file `file` and per-block cell-value array `v` are the STATE-PLANE tail of this
                // group: under ConfigBram they carry a REGISTERED read (file_rd/v_rd, consumed a cycle later by
                // the ST_OP/ST_WR/ST_TR/ST_FS/ST_AG fetch wait-states), so they map to block RAM — dissolving the
                // 128:1×64 operand-mux hub the open router could not legalize. Under Registers/Bram they stay
                // flip-flops (their reads are combinational there, and `file0` — dead under ConfigBram — always
                // stays flip-flops since nothing reads it there).
                // `file`/`v` are each split into two 32-bit halves (`file_lo`/`file_hi`, `v_lo`/`v_hi`) so
                // neither maps to a single 64-bit RAMB36 in SDP-72 mode (whose parity-lane write yosys 0.33
                // mis-wires — gate-level reproduced). Both halves get the SAME registered-config substrate the
                // un-split `file`/`v` had (block RAM under ConfigBram, flip-flops otherwise).
                let is_registered_config = matches!(
                    arr,
                    "file_lo"
                        | "file_hi"
                        | "v_lo"
                        | "v_hi"
                        | "cell_kind"
                        | "cell_code"
                        | "cell_op0"
                        | "cell_op1"
                        | "cell_op2"
                        | "cell_op3"
                        | "wr_reg"
                        | "wr_op"
                        | "blk_livein_ptr"
                        | "blk_n_livein"
                        | "blk_cell_ptr"
                        | "blk_n_cells"
                        | "blk_write_ptr"
                        | "blk_n_writes"
                        | "blk_term_kind"
                        | "blk_t0"
                        | "blk_t1"
                        | "blk_top"
                        | "blk_cdst"
                        | "blk_cargc"
                        | "blk_cargp"
                );
                let to_block = if is_state_plane {
                    state_plane_bram
                } else if is_registered_config {
                    config_bram
                } else {
                    false
                };
                let style = if to_block { "block" } else { "registers" };
                format!("{indent}(* ram_style = \"{style}\" *) {t}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    // Append the FPU modules raw (the ram_style transform is only for the engine's UART-streamed config memories).
    format!("{engine}\n{FPU_V}\n")
}

/// Excise (or, under ConfigBram, unwrap) the `//__CFGBRAM_ONLY_BEGIN__`…`//__CFGBRAM_ONLY_END__` regions of the
/// template. These regions hold the register-file/cell-value BRAM read machinery that only the ConfigBram backend
/// exercises. For non-ConfigBram backends we DROP the region bodies entirely (the states are unreachable and the
/// regs are dead there) so the CFG_BRAM=0 netlist is byte-identical to before this feature and its open-flow
/// synthesis stays fast; for ConfigBram we keep the bodies and drop only the two marker comment lines. The
/// markers must nest zero-deep (each BEGIN closed by its END before the next BEGIN) — asserted here so a
/// mis-paired marker fails loudly at emit time rather than silently emitting broken RTL.
fn strip_cfgbram_regions(src: &str, config_bram: bool) -> String {
    strip_marker_regions(
        src,
        "//__CFGBRAM_ONLY_BEGIN__",
        "//__CFGBRAM_ONLY_END__",
        config_bram,
    )
}

/// Excise (or, when enabled, unwrap) the `//__DEBUG_READBACK_ONLY_BEGIN__`…`//__DEBUG_READBACK_ONLY_END__`
/// regions of the template. These regions hold the on-chip memory-readback debug FSM (the `DEBUG_READBACK`
/// parameter's registered debug read ports, its state registers, the `P_DBG` phase, and the P_CLR→P_DBG detour)
/// which only a `DEBUG_READBACK=1` diagnostic build exercises. For a normal build we DROP the region bodies
/// entirely so the emitted RTL is BYTE-IDENTICAL to before this feature (and never references `DEBUG_READBACK`);
/// for a debug build we keep the bodies and drop only the marker comment lines — the SAME discipline as the
/// CFG_BRAM gating. The parameter itself is injected into the module header by a `.replace` (mirroring how the
/// ConfigBram default is flipped), so a normal build's header is unchanged too.
fn strip_debug_readback_regions(src: &str, debug_readback: bool) -> String {
    strip_marker_regions(
        src,
        "//__DEBUG_READBACK_ONLY_BEGIN__",
        "//__DEBUG_READBACK_ONLY_END__",
        debug_readback,
    )
}

/// The template's region tag for each memory that can leave the die.
const OFFCHIP_MEMORIES: [(ArrayRole, &str); 3] = [
    (ArrayRole::Heap, "HEAP"),
    (ArrayRole::FrameStack, "FRAMES"),
    (ArrayRole::ConfigImage, "CELLS"),
];

/// Keep, for each memory that can leave the die, its `OFFCHIP_<TAG>` regions when `off_chip` names its
/// role and its `ONCHIP_<TAG>` regions otherwise; `OFFCHIP_ANY` (the PSRAM pins, the held UART byte, the
/// stall gate, the glue site) when anything is off-chip and `ONCHIP_ALL` when nothing is.
fn select_memory_regions(src: &str, off_chip: &[ArrayRole]) -> String {
    assert!(
        off_chip
            .iter()
            .all(|r| OFFCHIP_MEMORIES.iter().any(|(m, _)| m == r)),
        "only the heap, the frame stack and the config image can leave the die: {off_chip:?}"
    );
    let mut s = src.to_string();
    for (role, tag) in OFFCHIP_MEMORIES {
        let off = off_chip.contains(&role);
        s = strip_marker_regions(
            &s,
            &format!("//__OFFCHIP_{tag}_BEGIN__"),
            &format!("//__OFFCHIP_{tag}_END__"),
            off,
        );
        s = strip_marker_regions(
            &s,
            &format!("//__ONCHIP_{tag}_BEGIN__"),
            &format!("//__ONCHIP_{tag}_END__"),
            !off,
        );
    }
    let any = !off_chip.is_empty();
    let s = strip_marker_regions(&s, "//__OFFCHIP_ANY_BEGIN__", "//__OFFCHIP_ANY_END__", any);
    strip_marker_regions(&s, "//__ONCHIP_ALL_BEGIN__", "//__ONCHIP_ALL_END__", !any)
}

/// Shared marker-region excision: DROP every line strictly between a `begin`/`end` marker pair when `keep` is
/// false (the marker lines themselves are never emitted); when `keep` is true, unwrap the bodies and drop only
/// the two marker comment lines. Markers must nest zero-deep (each begin closed by its end before the next
/// begin) — asserted so a mis-paired marker fails loudly at emit time rather than silently emitting broken RTL.
fn strip_marker_regions(src: &str, begin: &str, end: &str, keep: bool) -> String {
    let mut out = Vec::new();
    let mut in_region = false;
    for line in src.lines() {
        let trimmed = line.trim();
        if trimmed == begin {
            assert!(!in_region, "nested {begin} marker in wpe_soc_template.v");
            in_region = true;
            continue; // never emit the marker line itself
        }
        if trimmed == end {
            assert!(in_region, "unmatched {end} marker in wpe_soc_template.v");
            in_region = false;
            continue;
        }
        if in_region && !keep {
            continue; // drop the gated body when the feature is off
        }
        out.push(line);
    }
    assert!(
        !in_region,
        "unterminated {begin} region in wpe_soc_template.v"
    );
    out.join("\n")
}
