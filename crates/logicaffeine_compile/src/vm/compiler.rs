//! AST → bytecode compiler.
//!
//! Layout: Main's bytecode is emitted first (entry pc 0) and ends with `Halt`;
//! each function body is appended after it and reached via `Call` (an absolute
//! jump to its `entry_pc`). Function names are registered in a first pass so
//! forward references and (mutual) recursion resolve.
//!
//! Locals are assigned registers as they are first bound; expression temporaries
//! get fresh registers above them, so a call's argument block never overlaps a
//! live caller local.

use std::collections::HashMap;

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};

use super::instruction::{CompiledFunction, CompiledProgram, ConstIdx, Constant, FuncIdx, Op, Reg};
use super::{MAX_EXPR_DEPTH, MAX_REGISTERS_PER_FRAME};

/// Whether the W24 constant-divisor magic-reciprocal lowering is enabled.
/// Default ON (it is bit-identical to `Op::Div`/`Op::Mod` for the proven
/// non-negative dividend it gates on, validated by the corpus differential).
/// `LOGOS_MAGIC_DIV=0` is the kill-switch — also the A/B handle for measuring
/// the lever's effect on a quiet box. Read once.
pub fn magic_div_enabled() -> bool {
    crate::optimize::active_config().is_on(crate::optimization::Opt::FastDiv)
}

/// Narrow a `new Seq of Int` whose elements provably fit `i32` into half-width
/// (`Vec<i32>`) VM storage — the interpreter analogue of the AOT `Vec<i64>`→
/// `Vec<i32>` narrowing, halving the array footprint and cache pressure. The JIT
/// region path is i32-array aware (the hot region still tiers — 0 bails). PROMOTED
/// 2026-06-21: default ON (kill-switch LOGOS_NARROW_VM=0) — graph_bfs -27.6% and
/// array_fill -16.7% on the faithful interleaved A/B, all 33 benchmarks
/// bit-identical, full suite 10341/10341 green, no regression.
///
/// Read fresh on each body-compile (not memoized) so a process can compile both
/// regimes — the A/B differential harness and the in-process tests rely on this.
/// The cost is one env lookup per function body, never per op.
pub fn narrow_vm_enabled() -> bool {
    crate::optimize::active_config().is_on(crate::optimization::Opt::NarrowVm)
}

/// Precompute the unsigned magic-reciprocal constants `(magic, more)` for a
/// constant divisor `c` (`c != 0`), reusing the single canonical
/// [`logicaffeine_data::LogosDivU64`] generator — the same algorithm the AOT
/// `compile_to_rust` path emits, exhaustively proven against hardware `/`/`%`.
/// The `more` byte encoding (low 6 bits = shift, `0x40` = the 65-bit add-marker
/// path, `0x80` = the pure-shift power-of-two path) is exactly what
/// [`magic_eval`] consumes.
pub fn magic_u64_gen(c: u64) -> (u64, u8) {
    logicaffeine_data::LogosDivU64::new(c).parts()
}

/// Evaluate the magic reciprocal: `x / c` when `mul_back == 0`, else `x % c`
/// (`mul_back == c`). `x` is the dividend reinterpreted as `u64` — sound because
/// the op is emitted ONLY for a proven non-negative `x`, where the i64 bit
/// pattern equals the mathematical value and the unsigned quotient/remainder
/// equal the signed truncating ones. The remainder is `x - q*c` computed in
/// wrapping i64 (the low 64 bits agree with the u64 difference), bit-exact with
/// the kernel's `wrapping_rem` for non-negative `x`.
pub fn magic_eval(x: i64, magic: u64, more: u8, mul_back: i64) -> i64 {
    const SHIFT_MASK: u8 = 0x3F;
    const ADD_MARKER: u8 = 0x40;
    const SHIFT_PATH: u8 = 0x80;
    let n = x as u64;
    let q = if more & SHIFT_PATH != 0 {
        n >> (more & SHIFT_MASK)
    } else {
        let hi = (((magic as u128) * (n as u128)) >> 64) as u64;
        if more & ADD_MARKER != 0 {
            let t = (n.wrapping_sub(hi) >> 1).wrapping_add(hi);
            t >> (more & SHIFT_MASK)
        } else {
            hi >> (more & SHIFT_MASK)
        }
    };
    if mul_back == 0 {
        q as i64
    } else {
        x.wrapping_sub((q as i64).wrapping_mul(mul_back))
    }
}

/// Constant-pool dedup key. Floats are keyed by bit pattern so that distinct
/// NaN payloads and -0.0/0.0 stay distinct pool entries.
#[derive(Clone, PartialEq, Eq, Hash)]
enum ConstKey {
    Int(i64),
    FloatBits(u64),
    Bool(bool),
    Text(String),
    Char(char),
    Nothing,
    Duration(i64),
    Date(i32),
    Moment(i64),
    Span { months: i32, days: i32 },
    Time(i64),
}

fn const_key(c: &Constant) -> ConstKey {
    match c {
        Constant::Int(n) => ConstKey::Int(*n),
        Constant::Float(f) => ConstKey::FloatBits(f.to_bits()),
        Constant::Bool(b) => ConstKey::Bool(*b),
        Constant::Text(s) => ConstKey::Text(s.clone()),
        Constant::Char(c) => ConstKey::Char(*c),
        Constant::Nothing => ConstKey::Nothing,
        Constant::Duration(n) => ConstKey::Duration(*n),
        Constant::Date(d) => ConstKey::Date(*d),
        Constant::Moment(n) => ConstKey::Moment(*n),
        Constant::Span { months, days } => ConstKey::Span { months: *months, days: *days },
        Constant::Time(n) => ConstKey::Time(*n),
    }
}

/// Which i64-slot kind a DECLARED scalar type compiles to, if any.
fn slot_kind_of_type(
    ty: &crate::ast::stmt::TypeExpr,
    interner: &Interner,
) -> Option<super::native_tier::SlotKind> {
    use super::native_tier::SlotKind;
    use crate::ast::stmt::TypeExpr;
    match ty {
        // Float reaches the AST as Named (the Primitive set is Int, Nat,
        // Text, Bool) — accept the scalar names through either variant.
        TypeExpr::Primitive(sym) | TypeExpr::Named(sym) => match interner.resolve(*sym) {
            "Int" | "Nat" => Some(SlotKind::Int),
            "Float" => Some(SlotKind::Float),
            "Bool" => Some(SlotKind::Bool),
            _ => None,
        },
        TypeExpr::Refinement { base, .. } => slot_kind_of_type(base, interner),
        _ => None,
    }
}

/// A DECLARED parameter's native representation: scalar slot or pinned
/// `Seq of <scalar>`.
fn param_kind_of_type(
    ty: &crate::ast::stmt::TypeExpr,
    interner: &Interner,
) -> Option<super::native_tier::ParamKind> {
    use super::native_tier::{ParamKind, PinElem, SlotKind};
    use crate::ast::stmt::TypeExpr;
    if let Some(k) = slot_kind_of_type(ty, interner) {
        return Some(ParamKind::Scalar(k));
    }
    if let TypeExpr::Generic { base, params } = ty {
        if interner.resolve(*base) == "Seq" && params.len() == 1 {
            return match slot_kind_of_type(&params[0], interner) {
                Some(SlotKind::Int) => Some(ParamKind::List(PinElem::Int)),
                Some(SlotKind::Float) => Some(ParamKind::List(PinElem::Float)),
                Some(SlotKind::Bool) => Some(ParamKind::List(PinElem::Bool)),
                None => None,
            };
        }
    }
    if let TypeExpr::Refinement { base, .. } = ty {
        return param_kind_of_type(base, interner);
    }
    None
}

pub struct Compiler<'i> {
    interner: &'i Interner,
    code: Vec<Op>,
    constants: Vec<Constant>,
    const_map: HashMap<ConstKey, ConstIdx>,
    functions: Vec<CompiledFunction>,
    fn_index: HashMap<Symbol, FuncIdx>,
    // Struct definitions: field (name, declared-type, is_public) per struct —
    // from `Stmt::StructDef` and the discovery-pass type registry. Drives
    // default-fill on construction, like the tree-walker's struct_defs.
    struct_defs: HashMap<Symbol, Vec<(Symbol, Symbol, bool)>>,
    // Lexical block scopes for the frame currently being compiled (Main or one
    // function body). scopes[0] is the frame root; If/While/Repeat bodies push
    // a child scope whose registers are recycled on exit — mirroring the
    // tree-walker's execute_block undo-scope.
    scopes: Vec<HashMap<Symbol, Reg>>,
    next_reg: Reg,
    /// DEBUG-ONLY: when set (the debugger's compile path), capture source variable
    /// names so the debug drawer can show `x` rather than `R0`. Off in production.
    debug_names: bool,
    /// DEBUG-ONLY: (register, name-symbol) bound in the CURRENT frame; cleared per
    /// frame in `begin_scope`, resolved into `CompiledProgram::reg_names` for Main.
    dbg_names: Vec<(Reg, Symbol)>,
    /// Registers bound to a NAME (Let targets, params, loop variables,
    /// captures) in any frame compiled so far. Everything else is a
    /// statement-local scratch — dead at every statement boundary by the
    /// allocator's recycling discipline. Snapshotted for Main into
    /// `CompiledProgram::named_regs`, where the region JIT uses it to skip
    /// write-back (and pre-state preservation) for scratch slots.
    named: Vec<bool>,
    /// Loop heads (back-edge target pcs) currently being compiled, outermost
    /// first. Every `mark_named` while this is non-empty records the register as
    /// a LOOP-LOCAL of each enclosing loop — bound inside a loop body, so it is
    /// lexically dead the instant the loop exits (the scope is popped).
    loop_stack: Vec<usize>,
    /// `loop_locals[head]` = the registers bound inside the loop whose region
    /// head is `head` (absolute pc). The region JIT subtracts these from the
    /// write-back set: a slot dead at the loop exit must not be written back,
    /// which is what lets copy-prop/CSE/fusion treat it as true scratch. Keyed
    /// by absolute pc (Main and every function share one code array), valued by
    /// the OWNING frame's register indices.
    loop_locals: std::collections::HashMap<usize, Vec<bool>>,
    /// High-water mark of `next_reg` for the current frame — block scopes
    /// recycle registers, so the frame's true size is the maximum ever
    /// reached, not the final `next_reg`.
    max_reg: Reg,
    expr_depth: usize,
    // Enclosing control-flow contexts (innermost last). Loops collect Break
    // jumps; Zones (and Concurrent/Parallel tasks) SWALLOW Break/Return — the
    // tree-walker's execute path discards their ControlFlow.
    flow_stack: Vec<FlowCtx>,
    // Whether we are compiling a function body (false ⇒ Main, where a
    // top-level Break/Return halts the program).
    in_function: bool,
    /// Self-tail-call optimization context for the function currently being
    /// compiled: `(name, entry_pc, param_count)`. A self-call in TAIL position
    /// (`Return f(args)`, or a `Set/Let x = f(args)` immediately followed by
    /// `Return x`) is lowered to "reassign params 0..param_count, jump to
    /// entry_pc" instead of Call+Return — turning self-tail-recursion into a
    /// loop (constant stack, no per-call frame round-trip). `None` outside any
    /// function and for non-self / non-arity-matching calls.
    tail_fn: Option<(Symbol, usize, u16)>,
    // Promoted globals: Main TOP-LEVEL Let names referenced from function or
    // closure bodies, with their slot in the runtime globals table.
    promoted: HashMap<Symbol, u16>,
    // When compiling a closure body: per promoted capture, its (value, flag)
    // frame registers — a captured-global access reads the snapshot when the
    // flag is set and the LIVE global otherwise (tree-walker fall-through).
    closure_ctx: Option<HashMap<Symbol, (Reg, Reg)>>,
    /// Oracle range-analysis facts (M9) for the program being compiled.
    /// Present only on the live run path; when set, an `Index` whose
    /// `index_provably_in_bounds` holds is emitted as `IndexUnchecked`
    /// (bounds-check elimination, V8/LLVM-style). Keyed on the AST `Expr`
    /// pointers of THESE stmts, so the facts must be analyzed on the exact
    /// snapshot being compiled.
    oracle: Option<crate::optimize::OracleFacts>,
    /// Per-enclosing-loop sets of arrays whose region-entry `RegionBoundsGuard`
    /// was successfully emitted. A speculative access elides ONLY when its
    /// array is in some enclosing frame — the compiler's half of the
    /// elision ⟺ guard ⟺ VM-check invariant.
    hoist_enabled: Vec<std::collections::HashSet<Symbol>>,
    /// `new Seq of Int` declarations in the body CURRENTLY being compiled whose
    /// every element provably fits `i32` (the `codegen::narrow` proof). Their
    /// empty-constructor lowers to [`Op::NewEmptyListI32`] (half-width storage),
    /// observably identical to the i64 form. Populated per-body only when
    /// `LOGOS_NARROW_VM=1`; empty otherwise (default OFF).
    narrowable_seqs: std::collections::HashSet<Symbol>,
}

/// How a name resolves at a point in compilation.
enum NameRef {
    Local(Reg),
    /// A closure capture whose name is also a promoted global: snapshot when
    /// present, live global otherwise.
    CaptureOrGlobal { value: Reg, flag: Reg, global: u16 },
    Global(u16),
    Unbound,
}

enum FlowCtx {
    Loop {
        breaks: Vec<usize>,
        /// Repeat loops own a live iterator that must be IterPop'd when a
        /// Return jumps out across them into a Zone.
        is_repeat: bool,
    },
    Zone {
        exits: Vec<usize>,
    },
}

impl<'i> Compiler<'i> {
    /// Compile a statement block to a runnable program.
    pub fn compile(stmts: &[Stmt], interner: &'i Interner) -> Result<CompiledProgram, String> {
        Self::compile_with_types(stmts, interner, None)
    }

    /// Compile with the discovery-pass type registry (struct definitions that
    /// never appear as `Stmt::StructDef`).
    pub fn compile_with_types(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
    ) -> Result<CompiledProgram, String> {
        Self::compile_with_oracle(stmts, interner, types, None)
    }

    /// Like [`compile_with_types`], plus the Oracle's range-analysis facts
    /// (M9) for bounds-check elimination. The live run path computes these
    /// on the exact (optimized) snapshot it then compiles, so an `Index`
    /// proven in-bounds becomes `IndexUnchecked`. `None` ⇒ every index
    /// stays checked (the codegen / test paths).
    pub fn compile_with_oracle(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
        oracle: Option<crate::optimize::OracleFacts>,
    ) -> Result<CompiledProgram, String> {
        Self::compile_inner(stmts, interner, types, oracle, false)
    }

    /// Like [`compile_with_types`] but also captures source variable names into
    /// `CompiledProgram::reg_names` for the Studio debugger. Used only by the
    /// debugger's compile path; production callers pay nothing for it.
    pub fn compile_for_debug(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
    ) -> Result<CompiledProgram, String> {
        Self::compile_inner(stmts, interner, types, None, true)
    }

    fn compile_inner(
        stmts: &[Stmt],
        interner: &'i Interner,
        types: Option<&crate::analysis::TypeRegistry>,
        oracle: Option<crate::optimize::OracleFacts>,
        debug_names: bool,
    ) -> Result<CompiledProgram, String> {
        let mut c = Compiler {
            interner,
            debug_names,
            dbg_names: Vec::new(),
            code: Vec::new(),
            constants: Vec::new(),
            const_map: HashMap::new(),
            functions: Vec::new(),
            fn_index: HashMap::new(),
            struct_defs: HashMap::new(),
            scopes: vec![HashMap::new()],
            next_reg: 0,
            named: Vec::new(),
            loop_stack: Vec::new(),
            loop_locals: std::collections::HashMap::new(),
            max_reg: 0,
            expr_depth: 0,
            flow_stack: Vec::new(),
            tail_fn: None,
            hoist_enabled: Vec::new(),
            in_function: false,
            promoted: HashMap::new(),
            closure_ctx: None,
            oracle,
            narrowable_seqs: std::collections::HashSet::new(),
        };

        // Struct definitions from the type registry (mirrors the tree-walker's
        // with_type_registry) and from StructDef statements (pass 1 below).
        if let Some(registry) = types {
            use crate::analysis::registry::{FieldType, TypeDef};
            for (name_sym, type_def) in registry.iter_types() {
                if let TypeDef::Struct { fields, .. } = type_def {
                    let field_defs: Vec<(Symbol, Symbol, bool)> = fields
                        .iter()
                        .map(|f| {
                            let type_sym = match &f.ty {
                                FieldType::Primitive(s)
                                | FieldType::Named(s)
                                | FieldType::TypeParam(s) => *s,
                                FieldType::Generic { base, .. } => *base,
                            };
                            (f.name, type_sym, f.is_public)
                        })
                        .collect();
                    c.struct_defs.insert(*name_sym, field_defs);
                }
            }
        }

        // Pass 1: register every function name → index (entry_pc filled later)
        // and every struct definition.
        for s in stmts {
            if let Stmt::StructDef { name, fields, .. } = s {
                c.struct_defs.insert(*name, fields.clone());
            }
            if let Stmt::FunctionDef { name, params, return_type, .. } = s {
                if c.fn_index.contains_key(name) {
                    return Err(format!("vm: function '{}' defined twice", interner.resolve(*name)));
                }
                let idx = c.functions.len() as FuncIdx;
                c.fn_index.insert(*name, idx);
                c.functions.push(CompiledFunction {
                    name: *name,
                    entry_pc: 0,
                    param_count: u16::try_from(params.len())
                        .map_err(|_| "vm: too many parameters".to_string())?,
                    register_count: 0,
                    captures: Vec::new(),
                    named_regs: Vec::new(),
                    param_kinds: params
                        .iter()
                        .map(|(_, ty)| param_kind_of_type(ty, interner))
                        .collect(),
                    ret_kind: return_type.and_then(|ty| slot_kind_of_type(ty, interner)),
                });
            }
        }

        // Pass 1.5: promote Main TOP-LEVEL Let names referenced from any
        // function or closure body to globals (lexically visible everywhere).
        let mut nonlocal_idents: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
        for s in stmts {
            collect_nonlocal_idents_stmt(s, true, &mut nonlocal_idents);
        }
        let mut global_names: Vec<String> = Vec::new();
        for s in stmts {
            if let Stmt::Let { var, .. } = s {
                if nonlocal_idents.contains(var) && !c.promoted.contains_key(var) {
                    let idx = u16::try_from(c.promoted.len())
                        .map_err(|_| "vm: too many globals".to_string())?;
                    c.promoted.insert(*var, idx);
                    global_names.push(interner.resolve(*var).to_string());
                }
            }
        }

        // Pass 2a: compile Main (every non-FunctionDef top-level statement).
        // Behind `LOGOS_NARROW_VM`: certify which top-level `new Seq of Int`
        // declarations fit `i32` (the AOT narrowing proof), so their empty
        // constructor lowers to the half-width `NewEmptyListI32`. The proof's
        // `walk`/`escapes` ignore `FunctionDef` statements (none of the write
        // forms it tracks), and the runtime widen-on-overflow is the ultimate
        // safety net, so feeding the full statement list is sound.
        if narrow_vm_enabled() {
            c.narrowable_seqs = crate::codegen::narrow::narrowable_seqs_for_body(stmts, interner);
            if !c.narrowable_seqs.is_empty() {
                crate::optimize::mark_fired(crate::optimization::Opt::NarrowVm);
            }
        }
        c.begin_scope();
        for s in stmts {
            if !matches!(s, Stmt::FunctionDef { .. }) {
                c.compile_stmt(s)?;
            }
        }
        c.narrowable_seqs.clear();
        c.emit(Op::Halt);
        let main_regs = c.max_reg as usize;
        let mut named_regs = c.named.clone();
        named_regs.resize(main_regs, false);
        // Snapshot Main's variable names NOW — the per-function `begin_scope` below
        // clears `dbg_names`. Empty unless the debugger enabled name capture.
        let reg_names: Vec<(u16, String)> = c
            .dbg_names
            .iter()
            .map(|(r, sym)| (*r, c.interner.resolve(*sym).to_string()))
            .collect();

        // Pass 2b: compile each function body, recording its entry point.
        for s in stmts {
            if let Stmt::FunctionDef { name, params, body, .. } = s {
                let idx = c.fn_index[name];
                let entry_pc = c.code.len();
                c.begin_scope();
                c.in_function = true;
                c.named.clear();
                for (i, (psym, _ty)) in params.iter().enumerate() {
                    c.scopes.last_mut().unwrap().insert(*psym, i as Reg);
                    c.mark_named(i as Reg);
                }
                c.next_reg = params.len() as Reg;
                // Parameters occupy registers 0..n even when the body
                // allocates no temps — register_count must cover them or
                // the native frame's limit slot aliases a parameter.
                c.max_reg = c.max_reg.max(c.next_reg);
                debug_assert!(params.len() <= MAX_REGISTERS_PER_FRAME);
                c.tail_fn = Some((*name, entry_pc, params.len() as u16));
                let body_slice: &[Stmt] = body;
                let mut bk = 0;
                while bk < body_slice.len() {
                    // TAIL-CALL PAIR: `Set/Let x = self(args)` immediately
                    // followed by `Return x` is a self-tail-call (x is returned,
                    // never read after) — lower it to the loop-back instead of
                    // call+return. A direct `Return self(args)` is handled in the
                    // Return arm of compile_stmt (a Return is inherently tail).
                    if bk + 1 < body_slice.len() {
                        if let Some(args) = crate::tail_call::tail_pair_args(
                            &body_slice[bk],
                            &body_slice[bk + 1],
                            *name,
                            params.len(),
                        ) {
                            c.emit_tail_call(args, entry_pc, params.len() as u16)?;
                            bk += 2;
                            continue;
                        }
                    }
                    c.compile_stmt(&body_slice[bk])?;
                    bk += 1;
                }
                c.tail_fn = None;
                // Fall off the end → return nothing.
                c.emit(Op::ReturnNothing);
                let f = &mut c.functions[idx as usize];
                f.entry_pc = entry_pc;
                f.register_count = c.max_reg as usize;
                let mut fnamed = c.named.clone();
                fnamed.resize(f.register_count, false);
                f.named_regs = fnamed;
            }
        }

        Ok(CompiledProgram {
            constants: c.constants,
            code: c.code,
            register_count: main_regs,
            functions: c.functions,
            fn_index: c.fn_index,
            globals: global_names,
            named_regs,
            loop_locals: c.loop_locals,
            reg_names,
        })
    }

    fn begin_scope(&mut self) {
        self.scopes = vec![HashMap::new()];
        self.next_reg = 0;
        self.max_reg = 0;
        self.in_function = false;
        self.dbg_names.clear();
    }

    /// Enter a child block scope (If/While/Repeat body).
    fn enter_block(&mut self) -> Reg {
        self.scopes.push(HashMap::new());
        self.next_reg
    }

    /// Leave a block scope, recycling every register it allocated (its values
    /// are dead — the tree-walker's pop_scope undoes the bindings too).
    fn exit_block(&mut self, mark: Reg) {
        self.scopes.pop();
        self.next_reg = mark;
    }

    /// Resolve a name through the enclosing block scopes (innermost first).
    fn lookup_local(&self, sym: Symbol) -> Option<Reg> {
        self.scopes.iter().rev().find_map(|s| s.get(&sym).copied())
    }

    /// Is `sym`'s region-entry bounds guard active in some enclosing loop?
    fn is_hoist_enabled(&self, sym: Symbol) -> bool {
        self.hoist_enabled.iter().any(|s| s.contains(&sym))
    }

    /// Should `item index of collection` be emitted UNCHECKED? Either the
    /// Oracle proved it statically, or it is speculatively in bounds AND its
    /// array's region-entry guard was emitted for an enclosing loop (so the VM
    /// verifies it before any unchecked native access runs).
    fn index_in_bounds(&self, collection: &Expr, index: &Expr) -> bool {
        let Some(o) = self.oracle.as_ref() else { return false };
        if o.index_provably_in_bounds(collection, index) {
            // A symbolic `% m` element-bound elision carries a `m >= 1`
            // precondition the AOT discharges with a preheader `assert!`. The VM
            // emits no such guard, so it KEEPS the runtime check (it is the safe
            // reference, and `largo run` is not the benchmark path) rather than
            // risk an unchecked native access when `m <= 0`.
            return o.index_positivity_guards(index).is_empty();
        }
        o.index_is_speculative(index)
            && matches!(collection, Expr::Identifier(s) if self.is_hoist_enabled(*s))
    }

    /// `x / 2^k` where the divisor is a literal power of two and the Oracle
    /// proves the dividend is `Int` → the shift amount `k` (`1..=62`), else
    /// `None`. Type-safety is the crux: `Op::Div` dispatches int/float at
    /// runtime on the dividend, so lowering to a shift is sound ONLY for an
    /// integer dividend. The literal divisor is visible here pre-hoist, so this
    /// fires for loop-invariant divisors the JIT's in-region detector misses.
    fn divpow2_shift(&self, left: &Expr, right: &Expr) -> Option<u32> {
        let Expr::Literal(Literal::Number(d)) = right else { return None };
        let d = *d;
        if d < 2 || (d & (d - 1)) != 0 {
            return None;
        }
        let k = d.trailing_zeros();
        if !(1..=62).contains(&k) {
            return None;
        }
        if self.oracle.as_ref()?.expr_scalar(left)? != crate::optimize::ScalarKind::Int {
            return None;
        }
        Some(k)
    }

    /// `x % 2^k` where the divisor is a literal power of two AND the Oracle
    /// proves the dividend is `Int` and NON-NEGATIVE → the low-bit mask
    /// `2^k - 1`, else `None`. On a two's-complement i64 the truncated
    /// remainder agrees with `x & (2^k - 1)` only when `x >= 0` (the sign of
    /// `x % m` follows the dividend, so `(-1) % 8 == -1 != 7 == (-1) & 7`); the
    /// non-negativity gate is the SOUNDNESS condition — exactly the gate the AOT
    /// e-graph's `mod-pow2-and` rule applies. The literal divisor is visible
    /// here pre-hoist, so this fires for loop-invariant moduli the JIT's
    /// in-region detector misses (histogram's `% 2^31` LCG feedback). The win:
    /// a register-form `AndEager` (1-cycle AND, which the JIT lowers to a
    /// `BitAnd` stencil) instead of `Op::Mod`'s idiv on BOTH the VM and JIT.
    fn modpow2_mask(&self, left: &Expr, right: &Expr) -> Option<i64> {
        let Expr::Literal(Literal::Number(d)) = right else { return None };
        let d = *d;
        if d < 2 || (d & (d - 1)) != 0 {
            return None;
        }
        let k = d.trailing_zeros();
        if !(1..=62).contains(&k) {
            return None;
        }
        let oracle = self.oracle.as_ref()?;
        if oracle.expr_scalar(left)? != crate::optimize::ScalarKind::Int {
            return None;
        }
        if !oracle.expr_proven_nonneg(left) {
            return None;
        }
        Some(d - 1)
    }

    /// `x / c` or `x % c` where the divisor is a literal constant `c > 0` that is
    /// NOT a power of two (W5's `divpow2_shift`/`modpow2_mask` own powers of two)
    /// AND the Oracle proves `x` is `Int` and NON-NEGATIVE → the precomputed
    /// `(magic, more)` for the unsigned magic-reciprocal sequence, else `None`.
    ///
    /// SOUNDNESS — the unsigned magic equals the signed truncating `/`/`%` only
    /// for a non-negative dividend: `x.wrapping_div(c)` / `x.wrapping_rem(c)`
    /// equal `(x as u64) / c` / `(x as u64) % c` exactly when `x >= 0` and
    /// `c > 0` (both operands non-negative, no overflow edge — `c` is a literal
    /// `> 0` so `MIN/-1` is impossible). For `x < 0` the signs disagree, so the
    /// non-negativity gate is the soundness condition — exactly the gate
    /// `modpow2_mask` and the AOT `fast_div` lever apply. The literal divisor is
    /// visible here pre-hoist, so this fires for loop-invariant moduli the JIT's
    /// in-region detector misses (matrix's `% 1000000007`, histogram's `% 1000`).
    fn magic_div_const(&self, left: &Expr, right: &Expr) -> Option<(u64, u8)> {
        if !magic_div_enabled() {
            return None;
        }
        let Expr::Literal(Literal::Number(d)) = right else { return None };
        let d = *d;
        // `c > 0` and NOT a power of two (pow2 is W5's AND/shift territory).
        if d < 2 || (d & (d - 1)) == 0 {
            return None;
        }
        let oracle = self.oracle.as_ref()?;
        if oracle.expr_scalar(left)? != crate::optimize::ScalarKind::Int {
            return None;
        }
        if !oracle.expr_proven_nonneg(left) {
            return None;
        }
        crate::optimize::mark_fired(crate::optimization::Opt::FastDiv);
        Some(magic_u64_gen(d as u64))
    }

    /// Emit `RegionBoundsGuard` for each hoist descriptor the Oracle recorded
    /// for the loop `stmt`, returning the set of arrays whose guard was
    /// emitted (only those are then eligible for speculative elision). All
    /// three symbols must resolve to local registers, and the loop must read
    /// the array's length nowhere stale — guaranteed by the Oracle's analysis.
    fn emit_hoist_guards(&mut self, stmt: &Stmt) -> std::collections::HashSet<Symbol> {
        let mut enabled = std::collections::HashSet::new();
        let Some(oracle) = self.oracle.as_ref() else { return enabled };
        let descs = oracle
            .hoist_descs_for(stmt as *const Stmt as usize)
            .to_vec();
        for d in descs {
            let (Some(array), Some(bound), Some(iv)) = (
                self.lookup_local(d.array),
                self.lookup_local(d.bound),
                self.lookup_local(d.iv),
            ) else {
                continue;
            };
            self.emit(Op::RegionBoundsGuard {
                array,
                bound,
                iv,
                add_max: d.add_max,
                add_min: d.add_min,
            });
            enabled.insert(d.array);
        }
        enabled
    }

    /// Emit the runtime "Undefined variable" failure — unbound names are a
    /// RUNTIME error in the tree-walker, so dead branches stay free.
    fn emit_unbound(&mut self, sym: Symbol) -> Result<(), String> {
        let msg = format!("Undefined variable: {}", self.interner.resolve(sym));
        let idx = self.add_const(Constant::Text(msg))?;
        self.emit(Op::FailWith { msg: idx });
        Ok(())
    }

    /// Emit an unconditional runtime failure with a fixed message — the
    /// pattern for statements the interpreter spec rejects WHEN EXECUTED.
    fn emit_fail(&mut self, msg: &str) -> Result<(), String> {
        let idx = self.add_const(Constant::Text(msg.to_string()))?;
        self.emit(Op::FailWith { msg: idx });
        Ok(())
    }

    /// Resolve a name: block locals (and closure captures) shadow promoted
    /// globals; a promoted capture in a closure body needs the
    /// snapshot-or-live-global split.
    fn resolve_name(&self, sym: Symbol) -> NameRef {
        if let Some(r) = self.lookup_local(sym) {
            if let Some(ctx) = &self.closure_ctx {
                if let Some(&(value, flag)) = ctx.get(&sym) {
                    // Only when the local resolution IS the capture slot (a
                    // body-local `Let` shadows the capture entirely).
                    if r == value {
                        if let Some(&global) = self.promoted.get(&sym) {
                            return NameRef::CaptureOrGlobal { value, flag, global };
                        }
                    }
                }
            }
            NameRef::Local(r)
        } else if let Some(&g) = self.promoted.get(&sym) {
            NameRef::Global(g)
        } else {
            NameRef::Unbound
        }
    }

    /// Emit a read of `sym` into `dst`.
    fn emit_read(&mut self, sym: Symbol, dst: Reg) -> Result<(), String> {
        match self.resolve_name(sym) {
            NameRef::Local(src) => {
                if src != dst {
                    self.emit(Op::Move { dst, src });
                }
                Ok(())
            }
            NameRef::CaptureOrGlobal { value, flag, global } => {
                let jg = self.emit_placeholder_jump_if_false(flag);
                self.emit(Op::Move { dst, src: value });
                let jend = self.emit_placeholder_jump();
                self.patch_jump_target(jg, self.current_pc())?;
                self.emit(Op::GlobalGet { dst, idx: global });
                self.patch_jump_target(jend, self.current_pc())?;
                Ok(())
            }
            NameRef::Global(idx) => {
                self.emit(Op::GlobalGet { dst, idx });
                Ok(())
            }
            NameRef::Unbound => self.emit_unbound(sym),
        }
    }

    /// Emit a write of `R[src]` to `sym` (Set semantics: the binding must
    /// already exist somewhere).
    fn emit_write(&mut self, sym: Symbol, src: Reg) -> Result<(), String> {
        match self.resolve_name(sym) {
            NameRef::Local(dst) => {
                if src != dst {
                    self.emit(Op::Move { dst, src });
                }
                Ok(())
            }
            NameRef::CaptureOrGlobal { value, flag, global } => {
                let jg = self.emit_placeholder_jump_if_false(flag);
                self.emit(Op::Move { dst: value, src });
                let jend = self.emit_placeholder_jump();
                self.patch_jump_target(jg, self.current_pc())?;
                self.emit(Op::GlobalSet { idx: global, src });
                self.patch_jump_target(jend, self.current_pc())?;
                Ok(())
            }
            NameRef::Global(idx) => {
                self.emit(Op::GlobalSet { idx, src });
                Ok(())
            }
            NameRef::Unbound => self.emit_unbound(sym),
        }
    }

    fn alloc_reg(&mut self) -> Result<Reg, String> {
        self.reserve_regs(1)
    }

    /// Emit a self-tail-call as a LOOP-BACK (TCO): evaluate every argument into a
    /// FRESH temp first — so a parameter read by a later argument is never
    /// clobbered by an earlier parameter store — then move the temps into the
    /// parameter slots `0..param_count` and jump to the function entry. This
    /// turns self-tail-recursion into constant-stack iteration (no per-call frame
    /// round-trip, no `MAX_CALL_DEPTH` ceiling for tail calls).
    fn emit_tail_call(
        &mut self,
        args: &[&Expr],
        entry_pc: usize,
        param_count: u16,
    ) -> Result<(), String> {
        let mut temps = Vec::with_capacity(args.len());
        for a in args {
            let t = self.alloc_reg()?;
            self.compile_expr_into(a, t)?;
            temps.push(t);
        }
        for (i, t) in temps.into_iter().enumerate() {
            if t != i as Reg {
                self.emit(Op::Move { dst: i as Reg, src: t });
            }
        }
        self.emit(Op::Jump { target: entry_pc });
        Ok(())
    }

    /// Reserve `n` consecutive registers, returning the first.
    fn reserve_regs(&mut self, n: u16) -> Result<Reg, String> {
        let start = self.next_reg;
        let next = (self.next_reg as usize) + n as usize;
        if next > MAX_REGISTERS_PER_FRAME {
            return Err(format!(
                "vm: out of registers (frame limit is {})",
                MAX_REGISTERS_PER_FRAME
            ));
        }
        self.next_reg = next as Reg;
        if self.next_reg > self.max_reg {
            self.max_reg = self.next_reg;
        }
        // Scratch reserved inside a loop is a per-iteration temp, dead at the
        // loop exit — record it as a loop-local so the region JIT treats it as
        // true scratch even when it recycled a (now-dead) named variable's slot
        // (the fannkuch `perm[hi]` transfer temp that blocked swap fusion).
        if !self.loop_stack.is_empty() {
            for r in start..(next as Reg) {
                self.record_loop_local(r);
            }
        }
        Ok(start)
    }

    /// Resolve a mutation-target collection (`Push … to xs`) to its name
    /// reference. The caller materializes it (Rc-backed collections mutate
    /// through any clone) or emits the unbound failure AFTER the value's side
    /// effects, matching the tree-walker.
    fn resolve_collection(&mut self, e: &Expr) -> Result<(Symbol, NameRef), String> {
        match e {
            Expr::Identifier(sym) => Ok((*sym, self.resolve_name(*sym))),
            _ => Err("vm: expected an identifier collection".to_string()),
        }
    }

    /// Materialize a resolved collection into a register (None = unbound).
    fn collection_reg(&mut self, sym: Symbol, nr: &NameRef) -> Result<Option<Reg>, String> {
        match nr {
            NameRef::Local(r) => Ok(Some(*r)),
            NameRef::Unbound => Ok(None),
            _ => {
                let scratch = self.alloc_reg()?;
                self.emit_read(sym, scratch)?;
                Ok(Some(scratch))
            }
        }
    }

    /// Bind `sym` for a `Let`: reuse its register when the innermost scope
    /// already has it (re-Let overwrites), otherwise allocate a fresh one in
    /// the innermost scope (shadowing any outer binding).
    fn let_reg(&mut self, sym: Symbol) -> Result<Reg, String> {
        let r = if let Some(&r) = self.scopes.last().unwrap().get(&sym) {
            self.mark_named(r);
            r
        } else {
            let r = self.alloc_reg()?;
            self.scopes.last_mut().unwrap().insert(sym, r);
            self.mark_named(r);
            r
        };
        self.record_dbg_name(sym, r);
        Ok(r)
    }

    /// DEBUG-ONLY: remember that register `r` holds source variable `sym` in the
    /// current frame (last name wins after block-scope register reuse). A no-op
    /// unless the debugger's compile path enabled name capture.
    fn record_dbg_name(&mut self, sym: Symbol, r: Reg) {
        if !self.debug_names {
            return;
        }
        match self.dbg_names.iter_mut().find(|(rr, _)| *rr == r) {
            Some(slot) => slot.1 = sym,
            None => self.dbg_names.push((r, sym)),
        }
    }

    /// Record that `r` carries a user-visible name in the current frame.
    fn mark_named(&mut self, r: Reg) {
        let i = r as usize;
        if self.named.len() <= i {
            self.named.resize(i + 1, false);
        }
        self.named[i] = true;
        self.record_loop_local(r);
    }

    /// Record register `r` as LOCAL to every enclosing loop — lexically dead at
    /// each loop's exit — so the region JIT drops it from the write-back set.
    /// Called for named bindings (`mark_named`) AND for anonymous scratch
    /// reserved inside a loop: a per-iteration temp is dead at the loop exit too,
    /// and may have recycled a now-dead named variable's slot (block-scope reuse).
    /// Without this, such a temp keeps that slot's stale `named` flag and the
    /// region JIT treats it as live — blocking fusions like the swap idiom
    /// (fannkuch's `perm[hi]` transfer temp).
    fn record_loop_local(&mut self, r: Reg) {
        let i = r as usize;
        for &head in &self.loop_stack {
            let m = self.loop_locals.entry(head).or_default();
            if m.len() <= i {
                m.resize(i + 1, false);
            }
            m[i] = true;
        }
    }

    fn add_const(&mut self, c: Constant) -> Result<ConstIdx, String> {
        let key = const_key(&c);
        if let Some(&idx) = self.const_map.get(&key) {
            return Ok(idx);
        }
        let idx = ConstIdx::try_from(self.constants.len())
            .map_err(|_| "vm: constant pool overflow".to_string())?;
        self.constants.push(c);
        self.const_map.insert(key, idx);
        Ok(idx)
    }

    fn emit(&mut self, op: Op) {
        self.code.push(op);
    }

    fn compile_stmt(&mut self, s: &Stmt) -> Result<(), String> {
        match s {
            Stmt::Let { var, value, .. } => {
                // A promoted Main TOP-LEVEL Let defines the global; everywhere
                // else (function bodies, Main blocks — which shadow) a
                // register binding.
                if !self.in_function && self.scopes.len() == 1 && self.promoted.contains_key(var) {
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(value, scratch)?;
                    let idx = self.promoted[var];
                    self.emit(Op::GlobalSet { idx, src: scratch });
                    return Ok(());
                }
                // `Let var be a new Seq/Set/Map` (a fresh EMPTY collection):
                // bind first and construct directly into the var's register.
                // An empty constructor reads nothing, so the
                // evaluate-in-old-environment rule below is moot — and writing
                // into the var's stable register lets the loop-carried reuse
                // path (NewEmptyList over a sole-owned buffer) fire across
                // iterations instead of allocating fresh every pass.
                if let Expr::New { type_name, init_fields, .. } = value {
                    if init_fields.is_empty()
                        && matches!(
                            self.interner.resolve(*type_name),
                            "Seq" | "List" | "Set" | "HashSet" | "Map" | "HashMap"
                        )
                    {
                        let dst = self.let_reg(*var)?;
                        // A narrowing-proven `new Seq of Int` allocates the
                        // half-width (`Vec<i32>`) buffer directly into the var's
                        // stable register (so the loop-carried reuse path fires).
                        if self.narrowable_seqs.contains(var)
                            && matches!(self.interner.resolve(*type_name), "Seq" | "List")
                        {
                            self.emit(Op::NewEmptyListI32 { dst });
                        } else {
                            self.compile_expr_into(value, dst)?;
                        }
                        return Ok(());
                    }
                }
                // The value is evaluated BEFORE the new binding exists (the
                // tree-walker evaluates in the old environment): `Let x be
                // x + 1` in a block reads the OUTER x. Compile into a scratch
                // register first, then bind.
                let scratch = self.alloc_reg()?;
                self.compile_expr_into(value, scratch)?;
                let dst = self.let_reg(*var)?;
                self.emit(Op::Move { dst, src: scratch });
                Ok(())
            }
            Stmt::Set { target, value } => {
                match self.resolve_name(*target) {
                    NameRef::Local(dst) => {
                        // `Set x to x + e1 + … + ek`: lower to an AddAssign
                        // chain (in-place Text append; identical fold for
                        // every other type). Only when each later term
                        // provably never observes x — see add_assign_chain.
                        if let Some(terms) = add_assign_chain(*target, value) {
                            for term in terms {
                                let scratch = self.alloc_reg()?;
                                self.compile_expr_into(term, scratch)?;
                                self.emit(Op::AddAssign { dst, src: scratch });
                            }
                            return Ok(());
                        }
                        // Never compile directly into the live target: a
                        // multi-op value (interpolation accumulation, branch
                        // joins) would clobber the register before reading it
                        // — `Set result to "{result}{s}"` must accumulate.
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(value, scratch)?;
                        self.emit(Op::Move { dst, src: scratch });
                        Ok(())
                    }
                    NameRef::Unbound => {
                        // The tree-walker evaluates the value FIRST (its side
                        // effects happen), then fails the assignment — and only
                        // when the statement actually executes.
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(value, scratch)?;
                        self.emit_unbound(*target)
                    }
                    _ => {
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(value, scratch)?;
                        self.emit_write(*target, scratch)
                    }
                }
            }
            Stmt::Return { value } => {
                // A Zone swallows Return (the tree-walker discards the zone
                // body's ControlFlow): jump to the zone's end instead of
                // returning, popping the iterator of every Repeat crossed.
                let zone_pos = self
                    .flow_stack
                    .iter()
                    .rposition(|c| matches!(c, FlowCtx::Zone { .. }));
                if let Some(pos) = zone_pos {
                    if let Some(e) = value {
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(e, scratch)?;
                    }
                    let crossed_repeats = self.flow_stack[pos + 1..]
                        .iter()
                        .filter(|c| matches!(c, FlowCtx::Loop { is_repeat: true, .. }))
                        .count();
                    for _ in 0..crossed_repeats {
                        self.emit(Op::IterPop);
                    }
                    let j = self.code.len();
                    self.emit(Op::Jump { target: usize::MAX });
                    if let FlowCtx::Zone { exits } = &mut self.flow_stack[pos] {
                        exits.push(j);
                    }
                } else if self.in_function {
                    match value {
                        Some(e) => {
                            // TCO: a direct self-tail-call `Return self(args)` is
                            // inherently in tail position — lower it to the
                            // loop-back (reassign params + jump to entry) instead
                            // of a real call + return.
                            if let Some((tf, entry_pc, pc)) = self.tail_fn {
                                // Do NOT TCO across a `Repeat` (for-in): the
                                // loop-back jump would skip its `IterPop`,
                                // stacking abandoned iterators. (A `While`
                                // holds no runtime state, so jumping out of it
                                // is fine; `Zone`s are handled above.)
                                let in_repeat = self.flow_stack.iter().any(|c| {
                                    matches!(c, FlowCtx::Loop { is_repeat: true, .. })
                                });
                                if !in_repeat {
                                    if let Some(args) =
                                        crate::tail_call::direct_self_tail_args(*e, tf, pc as usize)
                                    {
                                        self.emit_tail_call(args, entry_pc, pc)?;
                                        return Ok(());
                                    }
                                }
                            }
                            let src = self.compile_expr(e)?;
                            self.emit(Op::Return { src });
                        }
                        None => self.emit(Op::ReturnNothing),
                    }
                } else {
                    // Return at Main top level = stop the program (the value's
                    // side effects still happen first).
                    if let Some(e) = value {
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(e, scratch)?;
                    }
                    self.emit(Op::Halt);
                }
                Ok(())
            }
            Stmt::Break => {
                // The innermost Loop or Zone catches it, whichever is nearer.
                match self.flow_stack.last_mut() {
                    Some(FlowCtx::Loop { breaks, .. }) => {
                        let j = self.code.len();
                        breaks.push(j);
                        self.emit(Op::Jump { target: usize::MAX });
                    }
                    Some(FlowCtx::Zone { .. }) => {
                        let j = self.code.len();
                        self.emit(Op::Jump { target: usize::MAX });
                        if let Some(FlowCtx::Zone { exits }) = self.flow_stack.last_mut() {
                            exits.push(j);
                        }
                    }
                    None => {
                        if self.in_function {
                            // A Break with no enclosing loop ends the function
                            // body (the tree-walker's call loop treats Break
                            // as "stop executing the body").
                            self.emit(Op::ReturnNothing);
                        } else {
                            // At Main top level it stops the program.
                            self.emit(Op::Halt);
                        }
                    }
                }
                Ok(())
            }
            Stmt::Call { function, args } => {
                // A call used for its effect; the result register is discarded.
                let dst = self.alloc_reg()?;
                self.compile_call(*function, args, dst)
            }
            Stmt::Push { value, collection } => match collection {
                Expr::Identifier(_) => {
                    let (sym, nr) = self.resolve_collection(collection)?;
                    match self.collection_reg(sym, &nr)? {
                        Some(list) => {
                            let val = self.compile_expr(value)?;
                            self.emit(Op::ListPush { list, value: val });
                            Ok(())
                        }
                        None => {
                            let scratch = self.alloc_reg()?;
                            self.compile_expr_into(value, scratch)?;
                            self.emit_unbound(sym)
                        }
                    }
                }
                Expr::FieldAccess { object: Expr::Identifier(obj_sym), field } => {
                    // The value is evaluated BEFORE the object lookup.
                    let val = self.compile_expr(value)?;
                    let obj = self.alloc_reg()?;
                    self.emit_read(*obj_sym, obj)?;
                    let name = self.interner.resolve(*field).to_string();
                    let fidx = self.add_const(Constant::Text(name))?;
                    self.emit(Op::ListPushField { obj, field: fidx, src: val });
                    Ok(())
                }
                Expr::FieldAccess { .. } => {
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(value, scratch)?;
                    self.emit_fail("Push to nested field access not supported")
                }
                _ => {
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(value, scratch)?;
                    self.emit_fail("Push collection must be an identifier or field access")
                }
            },
            Stmt::Add { value, collection } => {
                if !matches!(collection, Expr::Identifier(_)) {
                    // A field-access (or other) collection — `Add "Alice" to p's guests`.
                    // Compile it to a register: a collection/CRDT field reads as a shallow
                    // `Rc`, so the in-place `SetAdd` mutates the value stored in the struct.
                    // Mirrors the tree-walker's generalized `Add` (value evaluated first).
                    let val = self.compile_expr(value)?;
                    let set = self.compile_expr(collection)?;
                    self.emit(Op::SetAdd { set, value: val });
                    return Ok(());
                }
                let (sym, nr) = self.resolve_collection(collection)?;
                match self.collection_reg(sym, &nr)? {
                    Some(set) => {
                        let val = self.compile_expr(value)?;
                        self.emit(Op::SetAdd { set, value: val });
                        Ok(())
                    }
                    None => {
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(value, scratch)?;
                        self.emit_unbound(sym)
                    }
                }
            }
            Stmt::Remove { value, collection } => {
                if !matches!(collection, Expr::Identifier(_)) {
                    // Field-access collection — mirrors the generalized `Add` above.
                    let val = self.compile_expr(value)?;
                    let coll = self.compile_expr(collection)?;
                    self.emit(Op::RemoveFrom { collection: coll, value: val });
                    return Ok(());
                }
                let (sym, nr) = self.resolve_collection(collection)?;
                match self.collection_reg(sym, &nr)? {
                    Some(coll) => {
                        let val = self.compile_expr(value)?;
                        self.emit(Op::RemoveFrom { collection: coll, value: val });
                        Ok(())
                    }
                    None => {
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(value, scratch)?;
                        self.emit_unbound(sym)
                    }
                }
            }
            Stmt::SetIndex { collection, index, value } => {
                if !matches!(collection, Expr::Identifier(_)) {
                    // Index and value evaluate (in that order) before the
                    // collection-shape failure.
                    let s1 = self.alloc_reg()?;
                    self.compile_expr_into(index, s1)?;
                    let s2 = self.alloc_reg()?;
                    self.compile_expr_into(value, s2)?;
                    return self.emit_fail("SetIndex collection must be an identifier");
                }
                let proven = self.index_in_bounds(collection, index);
                let (sym, nr) = self.resolve_collection(collection)?;
                match self.collection_reg(sym, &nr)? {
                    Some(coll) => {
                        let idx = self.compile_expr(index)?;
                        let val = self.compile_expr(value)?;
                        if proven {
                            self.emit(Op::SetIndexUnchecked { collection: coll, index: idx, value: val });
                        } else {
                            self.emit(Op::SetIndex { collection: coll, index: idx, value: val });
                        }
                        // Structs are VALUE types: the struct-field form of
                        // SetIndex rewrites the register, so a promoted/global
                        // or captured name needs the write-back (a no-op for
                        // Rc-shared collections — same allocation).
                        if !matches!(nr, NameRef::Local(_)) {
                            self.emit_write(sym, coll)?;
                        }
                        Ok(())
                    }
                    None => {
                        // Tree-walker order: index, then value, then lookup.
                        let s1 = self.alloc_reg()?;
                        self.compile_expr_into(index, s1)?;
                        let s2 = self.alloc_reg()?;
                        self.compile_expr_into(value, s2)?;
                        self.emit_unbound(sym)
                    }
                }
            }
            Stmt::Show { object, recipient } => {
                if let Expr::Identifier(sym) = recipient {
                    self.compile_recipient_call(*sym, object)
                } else {
                    // Non-identifier recipient: the tree-walker still evaluates
                    // the object (side effects), then silently moves on.
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(object, scratch)?;
                    Ok(())
                }
            }
            Stmt::Give { object, recipient } => {
                if let Expr::Identifier(sym) = recipient {
                    self.compile_recipient_call(*sym, object)
                } else {
                    // Non-identifier recipient: the tree-walker evaluates the
                    // object (side effects) and silently moves on.
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(object, scratch)?;
                    Ok(())
                }
            }
            Stmt::If { cond, then_block, else_block } => {
                let c = self.compile_expr(cond)?;
                let jif = self.emit_placeholder_jump_if_false(c);
                let mark = self.enter_block();
                for st in *then_block {
                    self.compile_stmt(st)?;
                }
                self.exit_block(mark);
                match else_block {
                    Some(eb) => {
                        let jend = self.emit_placeholder_jump();
                        self.patch_jump_target(jif, self.current_pc())?;
                        let mark = self.enter_block();
                        for st in *eb {
                            self.compile_stmt(st)?;
                        }
                        self.exit_block(mark);
                        self.patch_jump_target(jend, self.current_pc())?;
                    }
                    None => {
                        self.patch_jump_target(jif, self.current_pc())?;
                    }
                }
                Ok(())
            }
            Stmt::While { cond, body, .. } => {
                // Region-entry bounds hoists go at the loop head (the back-edge
                // target), so they fall inside any region formed for the loop.
                let loop_start = self.current_pc();
                // Track names bound inside this loop body (the region head is
                // `loop_start`, the back-edge target) so the JIT can treat them
                // as dead-at-exit scratch. Pushed before the cond (re-evaluated
                // each iteration, so cond-bound names are loop-local too).
                self.loop_stack.push(loop_start);
                let enabled = self.emit_hoist_guards(s);
                self.hoist_enabled.push(enabled);
                let c = self.compile_expr(cond)?;
                let jexit = self.emit_placeholder_jump_if_false(c);
                self.flow_stack.push(FlowCtx::Loop { breaks: Vec::new(), is_repeat: false });
                let mark = self.enter_block();
                for st in *body {
                    self.compile_stmt(st)?;
                }
                self.exit_block(mark);
                self.hoist_enabled.pop();
                self.loop_stack.pop();
                self.emit(Op::Jump { target: loop_start });
                let exit_pc = self.current_pc();
                self.patch_jump_target(jexit, exit_pc)?;
                if let Some(FlowCtx::Loop { breaks, .. }) = self.flow_stack.pop() {
                    for j in breaks {
                        self.patch_jump_target(j, exit_pc)?;
                    }
                }
                Ok(())
            }
            Stmt::Repeat { pattern, iterable, body } => {
                self.compile_repeat(pattern, iterable, body)
            }
            Stmt::Pop { collection, into } => {
                if !matches!(collection, Expr::Identifier(_)) {
                    return self.emit_fail("Pop collection must be an identifier");
                }
                let (sym, nr) = self.resolve_collection(collection)?;
                match self.collection_reg(sym, &nr)? {
                    Some(list) => {
                        let dst = match into {
                            Some(var) => self.let_reg(*var)?,
                            None => self.alloc_reg()?,
                        };
                        self.emit(Op::ListPop { list, dst });
                        Ok(())
                    }
                    None => self.emit_unbound(sym),
                }
            }
            Stmt::RuntimeAssert { condition, .. } => {
                let c = self.compile_expr(condition)?;
                let jok = self.code.len();
                self.emit(Op::JumpIfTrue { cond: c, target: usize::MAX });
                let idx = self.add_const(Constant::Text("Assertion failed".to_string()))?;
                self.emit(Op::FailWith { msg: idx });
                self.patch_jump_target(jok, self.current_pc())?;
                Ok(())
            }
            Stmt::Zone { name, body, .. } => self.compile_zone(*name, body),
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                // Sequential in the interpreter spec. Each task runs WITHOUT a
                // block scope (its Lets persist), and its Break/Return is
                // swallowed — the tree-walker discards each task's ControlFlow.
                for task in tasks.iter() {
                    self.flow_stack.push(FlowCtx::Zone { exits: Vec::new() });
                    self.compile_stmt(task)?;
                    let end_pc = self.current_pc();
                    if let Some(FlowCtx::Zone { exits }) = self.flow_stack.pop() {
                        for j in exits {
                            self.patch_jump_target(j, end_pc)?;
                        }
                    }
                }
                Ok(())
            }
            Stmt::Sleep { milliseconds } => {
                let duration = self.compile_expr(milliseconds)?;
                self.emit(Op::Sleep { duration });
                Ok(())
            }
            // Verification-only / declaration statements: no runtime effect.
            Stmt::Assert { .. } | Stmt::Trust { .. } | Stmt::Require { .. }
            | Stmt::Theorem(_) | Stmt::Definition(_) => {
                Ok(())
            }
            // Definitions were registered in pass 1.
            Stmt::StructDef { .. } => Ok(()),
            Stmt::SetField { object, field, value } => {
                let field_name = self.interner.resolve(*field).to_string();
                let fidx = self.add_const(Constant::Text(field_name))?;
                if let Expr::Identifier(obj_sym) = object {
                    // Tree-walker order: value first, then the target lookup.
                    match self.resolve_name(*obj_sym) {
                        NameRef::Local(obj) => {
                            let v = self.compile_expr(value)?;
                            self.emit(Op::StructInsert { obj, field: fidx, value: v });
                            Ok(())
                        }
                        NameRef::Unbound => {
                            let scratch = self.alloc_reg()?;
                            self.compile_expr_into(value, scratch)?;
                            self.emit_unbound(*obj_sym)
                        }
                        _ => {
                            // Structs are VALUES: load the global's copy,
                            // mutate it, store it back.
                            let v = self.compile_expr(value)?;
                            let obj = self.alloc_reg()?;
                            self.emit_read(*obj_sym, obj)?;
                            self.emit(Op::StructInsert { obj, field: fidx, value: v });
                            self.emit_write(*obj_sym, obj)
                        }
                    }
                } else {
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(value, scratch)?;
                    let idx = self
                        .add_const(Constant::Text("SetField target must be an identifier".to_string()))?;
                    self.emit(Op::FailWith { msg: idx });
                    Ok(())
                }
            }
            Stmt::Inspect { target, arms, .. } => self.compile_inspect(target, arms),
            Stmt::IncreaseCrdt { object, field, amount }
            | Stmt::DecreaseCrdt { object, field, amount } => {
                let negate = matches!(s, Stmt::DecreaseCrdt { .. });
                let amt = self.compile_expr(amount)?;
                let fname = self.interner.resolve(*field).to_string();
                let fidx = self.add_const(Constant::Text(fname))?;
                if let Expr::Identifier(obj_sym) = object {
                    match self.resolve_name(*obj_sym) {
                        NameRef::Local(obj) => {
                            self.emit(Op::CrdtBump { obj, field: fidx, amount: amt, negate });
                            Ok(())
                        }
                        NameRef::Unbound => self.emit_unbound(*obj_sym),
                        _ => {
                            let obj = self.alloc_reg()?;
                            self.emit_read(*obj_sym, obj)?;
                            self.emit(Op::CrdtBump { obj, field: fidx, amount: amt, negate });
                            self.emit_write(*obj_sym, obj)
                        }
                    }
                } else {
                    let msg = if negate {
                        "DecreaseCrdt target must be an identifier"
                    } else {
                        "IncreaseCrdt target must be an identifier"
                    };
                    let idx = self.add_const(Constant::Text(msg.to_string()))?;
                    self.emit(Op::FailWith { msg: idx });
                    Ok(())
                }
            }
            Stmt::MergeCrdt { source, target } => {
                let src = self.compile_expr(source)?;
                if let Expr::Identifier(target_sym) = target {
                    match self.resolve_name(*target_sym) {
                        NameRef::Local(tgt) => {
                            self.emit(Op::CrdtMerge { target: tgt, source: src });
                            Ok(())
                        }
                        NameRef::Unbound => self.emit_unbound(*target_sym),
                        _ => {
                            let tgt = self.alloc_reg()?;
                            self.emit_read(*target_sym, tgt)?;
                            self.emit(Op::CrdtMerge { target: tgt, source: src });
                            self.emit_write(*target_sym, tgt)
                        }
                    }
                } else {
                    let idx = self
                        .add_const(Constant::Text("Merge target must be an identifier".to_string()))?;
                    self.emit(Op::FailWith { msg: idx });
                    Ok(())
                }
            }
            Stmt::ReadFrom { var, source } => {
                use crate::ast::stmt::ReadSource;
                match source {
                    ReadSource::Console => {
                        // Console reads yield empty Text in the interpreter.
                        let dst = self.let_reg(*var)?;
                        let idx = self.add_const(Constant::Text(String::new()))?;
                        self.emit(Op::LoadConst { dst, idx });
                        Ok(())
                    }
                    ReadSource::File(path_expr) => {
                        // The VM has no VFS (yet): the path's side effects run,
                        // then the tree-walker's exact no-VFS error.
                        let scratch = self.alloc_reg()?;
                        self.compile_expr_into(path_expr, scratch)?;
                        let idx = self.add_const(Constant::Text(
                            "VFS not initialized. Use Interpreter::with_vfs()".to_string(),
                        ))?;
                        self.emit(Op::FailWith { msg: idx });
                        Ok(())
                    }
                }
            }
            Stmt::WriteFile { content, path } => {
                let s1 = self.alloc_reg()?;
                self.compile_expr_into(content, s1)?;
                let s2 = self.alloc_reg()?;
                self.compile_expr_into(path, s2)?;
                let idx = self.add_const(Constant::Text(
                    "VFS not initialized. Use Interpreter::with_vfs()".to_string(),
                ))?;
                self.emit(Op::FailWith { msg: idx });
                Ok(())
            }
            Stmt::Mount { path, .. } => {
                let scratch = self.alloc_reg()?;
                self.compile_expr_into(path, scratch)?;
                let idx = self.add_const(Constant::Text(
                    "VFS not initialized. Use Interpreter::with_vfs()".to_string(),
                ))?;
                self.emit(Op::FailWith { msg: idx });
                Ok(())
            }
            Stmt::Spawn { name, .. } => {
                // Agents do not run in the interpreter: the handle is Nothing.
                let dst = self.let_reg(*name)?;
                let idx = self.add_const(Constant::Nothing)?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            Stmt::SendMessage { .. } => Ok(()),
            Stmt::AwaitMessage { into, .. } => {
                let dst = self.let_reg(*into)?;
                let idx = self.add_const(Constant::Nothing)?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            Stmt::Check { subject, predicate, is_capability, object, source_text, .. } => {
                let subj = self.alloc_reg()?;
                self.emit_read(*subject, subj)?;
                // The object is only looked up for capability checks.
                let obj = if *is_capability {
                    match object {
                        Some(obj_sym) => {
                            let r = self.alloc_reg()?;
                            self.emit_read(*obj_sym, r)?;
                            r
                        }
                        None => Reg::MAX,
                    }
                } else {
                    Reg::MAX
                };
                let st = self.add_const(Constant::Text(source_text.clone()))?;
                self.emit(Op::CheckPolicy {
                    subject: subj,
                    predicate: *predicate,
                    is_capability: *is_capability,
                    object: obj,
                    source_text: st,
                });
                Ok(())
            }
            Stmt::AppendToSequence { sequence, value } => {
                // The RGA is always behind a shared `Rc` (whether a bare variable or a
                // struct field), so appending in place propagates — no write-back needed.
                // Value compiled first, matching the tree-walker's evaluation order.
                let val = self.compile_expr(value)?;
                let seq = self.compile_expr(sequence)?;
                self.emit(Op::CrdtAppend { seq, value: val });
                Ok(())
            }
            Stmt::ResolveConflict { object, field, value } => {
                let val = self.compile_expr(value)?;
                let fname = self.interner.resolve(*field).to_string();
                let fidx = self.add_const(Constant::Text(fname))?;
                // Mirrors `IncreaseCrdt`: a register field resolves in place via its shared
                // `Rc`; a plain field is overwritten, so it needs the read/write-back path
                // (structs have value semantics).
                if let Expr::Identifier(obj_sym) = object {
                    match self.resolve_name(*obj_sym) {
                        NameRef::Local(obj) => {
                            self.emit(Op::CrdtResolve { obj, field: fidx, value: val });
                            Ok(())
                        }
                        NameRef::Unbound => self.emit_unbound(*obj_sym),
                        _ => {
                            let obj = self.alloc_reg()?;
                            self.emit_read(*obj_sym, obj)?;
                            self.emit(Op::CrdtResolve { obj, field: fidx, value: val });
                            self.emit_write(*obj_sym, obj)
                        }
                    }
                } else {
                    let idx = self.add_const(Constant::Text(
                        "Resolve target must be a struct field".to_string(),
                    ))?;
                    self.emit(Op::FailWith { msg: idx });
                    Ok(())
                }
            }
            // Relay networking is async-tier: it runs on the tree-walker's async executor
            // (the `needs_async` routing diverts these before the VM ever sees them). The VM
            // tier owns compute + channel concurrency and defers networking — these arms are
            // the defensive diagnostic if a networking statement ever reaches VM compilation.
            Stmt::Listen { .. } => {
                self.emit_fail("Listen requires the async execution path")
            }
            Stmt::ConnectTo { .. } => {
                self.emit_fail("Connect requires the async execution path")
            }
            Stmt::LetPeerAgent { .. } => {
                self.emit_fail("PeerAgent messaging requires the async execution path")
            }
            Stmt::Sync { .. } => {
                self.emit_fail("Sync requires the async execution path")
            }
            // Go-like concurrency (T11d). The VM emits the scheduler opcodes; a
            // concurrent program is then driven through `run_until_block` by a
            // `VmTask`. (Concurrent programs currently route to the tree-walker
            // before the VM, so these are exercised via the dedicated VM-scheduler
            // entry; the default routing is unchanged.)
            Stmt::CreatePipe { var, capacity, .. } => {
                let dst = self.let_reg(*var)?;
                let cap = capacity.map(|c| c as i32).unwrap_or(-1);
                self.emit(Op::ChanNew { dst, cap });
                Ok(())
            }
            Stmt::SendPipe { value, pipe } => {
                let val = self.compile_expr(value)?;
                let chan = self.compile_expr(pipe)?;
                self.emit(Op::ChanSend { chan, val });
                Ok(())
            }
            Stmt::ReceivePipe { var, pipe } => {
                let chan = self.compile_expr(pipe)?;
                let dst = self.let_reg(*var)?;
                self.emit(Op::ChanRecv { dst, chan });
                Ok(())
            }
            Stmt::TrySendPipe { value, pipe, result } => {
                let val = self.compile_expr(value)?;
                let chan = self.compile_expr(pipe)?;
                let dst = match result {
                    Some(sym) => self.let_reg(*sym)?,
                    None => self.alloc_reg()?,
                };
                self.emit(Op::ChanTrySend { dst, chan, val });
                Ok(())
            }
            Stmt::TryReceivePipe { var, pipe } => {
                let chan = self.compile_expr(pipe)?;
                let dst = self.let_reg(*var)?;
                self.emit(Op::ChanTryRecv { dst, chan });
                Ok(())
            }
            Stmt::LaunchTask { function, args } => self.compile_spawn(*function, args, None),
            Stmt::LaunchTaskWithHandle { handle, function, args } => {
                self.compile_spawn(*function, args, Some(*handle))
            }
            Stmt::StopTask { handle } => {
                let h = self.compile_expr(handle)?;
                self.emit(Op::TaskAbort { handle: h });
                Ok(())
            }
            Stmt::Select { branches } => {
                use crate::ast::stmt::SelectBranch;
                // Register each arm. A recv arm allocates and binds its var
                // register up front, so the winning branch body reads the value
                // `deliver_select` writes there.
                for branch in branches.iter() {
                    match branch {
                        SelectBranch::Receive { var, pipe, .. } => {
                            let chan = self.compile_expr(pipe)?;
                            let var_reg = self.let_reg(*var)?;
                            self.emit(Op::SelectArmRecv { chan, var: var_reg });
                        }
                        SelectBranch::Timeout { milliseconds, .. } => {
                            let ticks = self.compile_expr(milliseconds)?;
                            self.emit(Op::SelectArmTimeout { ticks });
                        }
                    }
                }
                let dst_arm = self.alloc_reg()?;
                self.emit(Op::SelectWait { dst_arm });
                // Dispatch on the winning arm index: run that branch's body.
                let mut end_jumps = Vec::new();
                for (i, branch) in branches.iter().enumerate() {
                    let lit = self.alloc_reg()?;
                    let idx = self.add_const(Constant::Int(i as i64))?;
                    self.emit(Op::LoadConst { dst: lit, idx });
                    let cmp = self.alloc_reg()?;
                    self.emit(Op::Eq { dst: cmp, lhs: dst_arm, rhs: lit });
                    let jskip = self.emit_placeholder_jump_if_false(cmp);
                    let body = match branch {
                        SelectBranch::Receive { body, .. } | SelectBranch::Timeout { body, .. } => {
                            *body
                        }
                    };
                    let mark = self.enter_block();
                    for st in body {
                        self.compile_stmt(st)?;
                    }
                    self.exit_block(mark);
                    let jend = self.emit_placeholder_jump();
                    end_jumps.push(jend);
                    self.patch_jump_target(jskip, self.current_pc())?;
                }
                for j in end_jumps {
                    self.patch_jump_target(j, self.current_pc())?;
                }
                Ok(())
            }
            Stmt::Escape { .. } => self.emit_fail(
                "Escape blocks contain raw Rust code and cannot be interpreted. \
                     Use `largo build` or `largo run` to compile and run this program.",
            ),
            Stmt::FunctionDef { .. } => {
                // Nested function definitions are unreachable from the parser
                // (## To headers are top level); statically registering one
                // here cannot model the tree-walker's conditional runtime
                // definition, so it is a compile error.
                Err("vm: nested function definitions are not supported".to_string())
            }
        }
    }

    fn current_pc(&self) -> usize {
        self.code.len()
    }

    fn emit_placeholder_jump(&mut self) -> usize {
        let idx = self.code.len();
        self.emit(Op::Jump { target: usize::MAX });
        idx
    }

    fn emit_placeholder_jump_if_false(&mut self, cond: Reg) -> usize {
        let idx = self.code.len();
        self.emit(Op::JumpIfFalse { cond, target: usize::MAX });
        idx
    }

    fn patch_jump_target(&mut self, idx: usize, target: usize) -> Result<(), String> {
        patch_jump(&mut self.code, idx, target)
    }

    /// Compile `e`, returning the register that holds its value. An unbound
    /// identifier becomes a runtime failure at this point in the instruction
    /// stream (never a compile error — dead branches stay free).
    fn compile_expr(&mut self, e: &Expr) -> Result<Reg, String> {
        match e {
            Expr::Identifier(sym) => match self.interner.resolve(*sym) {
                "today" | "now" => {
                    let scratch = self.alloc_reg()?;
                    self.compile_expr_into(e, scratch)?;
                    Ok(scratch)
                }
                _ => match self.resolve_name(*sym) {
                    NameRef::Local(r) => Ok(r),
                    _ => {
                        let scratch = self.alloc_reg()?;
                        self.emit_read(*sym, scratch)?;
                        Ok(scratch)
                    }
                },
            },
            _ => {
                let dst = self.alloc_reg()?;
                self.compile_expr_into(e, dst)?;
                Ok(dst)
            }
        }
    }

    /// Compile `e`, placing its value into `dst`. Depth-guarded so adversarially
    /// nested expressions fail with an error instead of exhausting the native
    /// stack.
    fn compile_expr_into(&mut self, e: &Expr, dst: Reg) -> Result<(), String> {
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            self.expr_depth -= 1;
            return Err("vm: expression too deeply nested".to_string());
        }
        let result = self.compile_expr_into_inner(e, dst);
        self.expr_depth -= 1;
        result
    }

    fn compile_expr_into_inner(&mut self, e: &Expr, dst: Reg) -> Result<(), String> {
        match e {
            Expr::Literal(Literal::Text(sym)) => {
                let idx = self.add_const(Constant::Text(self.interner.resolve(*sym).to_string()))?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            Expr::Literal(lit) => {
                let idx = self.add_const(literal_const(lit)?)?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            Expr::Identifier(sym) => {
                // Temporal builtins: the NAME wins even when shadowed
                // (tree-walker checks before the env lookup).
                match self.interner.resolve(*sym) {
                    "today" => {
                        self.emit(Op::LoadToday { dst });
                        Ok(())
                    }
                    "now" => {
                        self.emit(Op::LoadNow { dst });
                        Ok(())
                    }
                    _ => self.emit_read(*sym, dst),
                }
            }
            Expr::BinaryOp { op: BinaryOpKind::And, left, right } => {
                self.compile_short_circuit(true, left, right, dst)
            }
            Expr::BinaryOp { op: BinaryOpKind::Or, left, right } => {
                self.compile_short_circuit(false, left, right, dst)
            }
            Expr::BinaryOp { op, left, right } => {
                // Int `x / 2^k` → single `DivPow2` op (the JIT lowers it to the
                // 1-op shift stencil): replaces idiv, fires for loop-invariant
                // divisors, no scratch-register pressure.
                if matches!(op, BinaryOpKind::Divide) {
                    if let Some(k) = self.divpow2_shift(left, right) {
                        let lhs = self.compile_expr(left)?;
                        self.emit(Op::DivPow2 { dst, lhs, k: k as u8 });
                        return Ok(());
                    }
                    // Proven-non-negative Int `x / c` (c a constant non-pow2) →
                    // the magic reciprocal (mul-high + shift) instead of idiv.
                    if let Some((magic, more)) = self.magic_div_const(left, right) {
                        let lhs = self.compile_expr(left)?;
                        self.emit(Op::MagicDivU { dst, lhs, magic, more, mul_back: 0 });
                        return Ok(());
                    }
                }
                // Proven-non-negative Int `x % 2^k` → `x & (2^k - 1)`, emitted
                // as the existing register-form `AndEager` (Int×Int is bitwise
                // AND): replaces idiv with a 1-cycle AND, which the JIT lowers to
                // a `BitAnd` stencil. Gated on Oracle-proven non-negativity —
                // unsound for negative dividends.
                if matches!(op, BinaryOpKind::Modulo) {
                    if let Some(mask) = self.modpow2_mask(left, right) {
                        let lhs = self.compile_expr(left)?;
                        let rhs = self.alloc_reg()?;
                        let idx = self.add_const(Constant::Int(mask))?;
                        self.emit(Op::LoadConst { dst: rhs, idx });
                        self.emit(Op::AndEager { dst, lhs, rhs });
                        return Ok(());
                    }
                    // Proven-non-negative Int `x % c` (c a constant non-pow2) →
                    // the magic reciprocal: `x - (x/c)*c` via mul-high + shift,
                    // instead of idiv. `mul_back == c` selects the remainder.
                    if let Expr::Literal(Literal::Number(c)) = right {
                        if let Some((magic, more)) = self.magic_div_const(left, right) {
                            let lhs = self.compile_expr(left)?;
                            self.emit(Op::MagicDivU { dst, lhs, magic, more, mul_back: *c });
                            return Ok(());
                        }
                    }
                }
                let lhs = self.compile_expr(left)?;
                let rhs = self.compile_expr(right)?;
                self.emit(binop_op(*op, dst, lhs, rhs)?);
                Ok(())
            }
            Expr::Not { operand } => {
                let src = self.compile_expr(operand)?;
                self.emit(Op::Not { dst, src });
                Ok(())
            }
            Expr::Call { function, args } => self.compile_call(*function, args, dst),
            Expr::List(items) => {
                let count = u16::try_from(items.len())
                    .map_err(|_| "vm: list literal too long (max 65535 elements)".to_string())?;
                let start = self.reserve_regs(count)?;
                for (i, item) in items.iter().enumerate() {
                    self.compile_expr_into(item, start + i as Reg)?;
                }
                self.emit(Op::NewList { dst, start, count });
                Ok(())
            }
            Expr::New { type_name, init_fields, .. } => {
                self.compile_new(*type_name, init_fields, dst)
            }
            Expr::FieldAccess { object, field } => {
                let obj = self.compile_expr(object)?;
                let fname = self.interner.resolve(*field).to_string();
                let fidx = self.add_const(Constant::Text(fname))?;
                self.emit(Op::GetField { dst, obj, field: fidx });
                Ok(())
            }
            Expr::NewVariant { enum_name, variant, fields } => {
                let tname = self.interner.resolve(*enum_name).to_string();
                let cname = self.interner.resolve(*variant).to_string();
                let tidx = self.add_const(Constant::Text(tname))?;
                let cidx = self.add_const(Constant::Text(cname))?;
                let count = u16::try_from(fields.len())
                    .map_err(|_| "vm: too many variant fields".to_string())?;
                let args_start = self.reserve_regs(count)?;
                for (i, (_, field_expr)) in fields.iter().enumerate() {
                    self.compile_expr_into(field_expr, args_start + i as Reg)?;
                }
                self.emit(Op::NewInductive { dst, type_name: tidx, ctor: cidx, args_start, count });
                Ok(())
            }
            Expr::Range { start, end } => {
                let s = self.compile_expr(start)?;
                let e = self.compile_expr(end)?;
                self.emit(Op::NewRange { dst, start: s, end: e });
                Ok(())
            }
            Expr::Length { collection } => {
                let c = self.compile_expr(collection)?;
                self.emit(Op::Length { dst, collection: c });
                Ok(())
            }
            Expr::Index { collection, index } => {
                // Bounds-check elimination: if the Oracle proved this index
                // in `[1, length]` (range analysis, M9), emit the unchecked
                // form — the JIT drops the bounds branch. Checked otherwise.
                let proven = self.index_in_bounds(collection, index);
                let c = self.compile_expr(collection)?;
                let i = self.compile_expr(index)?;
                if proven {
                    self.emit(Op::IndexUnchecked { dst, collection: c, index: i });
                } else {
                    self.emit(Op::Index { dst, collection: c, index: i });
                }
                Ok(())
            }
            Expr::Contains { collection, value } => {
                let c = self.compile_expr(collection)?;
                let v = self.compile_expr(value)?;
                self.emit(Op::Contains { dst, collection: c, value: v });
                Ok(())
            }
            Expr::InterpolatedString(parts) => self.compile_interpolation(parts, dst),
            Expr::Slice { collection, start, end } => {
                let c = self.compile_expr(collection)?;
                let st = self.compile_expr(start)?;
                let en = self.compile_expr(end)?;
                self.emit(Op::SliceOp { dst, collection: c, start: st, end: en });
                Ok(())
            }
            Expr::Copy { expr } => {
                let src = self.compile_expr(expr)?;
                self.emit(Op::DeepClone { dst, src });
                Ok(())
            }
            Expr::Give { value } => self.compile_expr_into(value, dst),
            Expr::Tuple(items) => {
                let count = u16::try_from(items.len())
                    .map_err(|_| "vm: tuple literal too long".to_string())?;
                let start = self.reserve_regs(count)?;
                for (i, item) in items.iter().enumerate() {
                    self.compile_expr_into(item, start + i as Reg)?;
                }
                self.emit(Op::NewTuple { dst, start, count });
                Ok(())
            }
            Expr::Union { left, right } => {
                let l = self.compile_expr(left)?;
                let r = self.compile_expr(right)?;
                self.emit(Op::UnionOp { dst, lhs: l, rhs: r });
                Ok(())
            }
            Expr::Intersection { left, right } => {
                let l = self.compile_expr(left)?;
                let r = self.compile_expr(right)?;
                self.emit(Op::IntersectOp { dst, lhs: l, rhs: r });
                Ok(())
            }
            Expr::OptionSome { value } => self.compile_expr_into(value, dst),
            Expr::OptionNone => {
                let idx = self.add_const(Constant::Nothing)?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            Expr::WithCapacity { value, .. } => self.compile_expr_into(value, dst),
            Expr::ManifestOf { .. } => {
                self.emit(Op::NewEmptyList { dst });
                Ok(())
            }
            Expr::ChunkAt { .. } => {
                let idx = self.add_const(Constant::Nothing)?;
                self.emit(Op::LoadConst { dst, idx });
                Ok(())
            }
            Expr::Escape { .. } => {
                let idx = self.add_const(Constant::Text(
                    "Escape expressions contain raw Rust code and cannot be interpreted. \
                     Use `largo build` or `largo run` to compile and run this program."
                        .to_string(),
                ))?;
                self.emit(Op::FailWith { msg: idx });
                Ok(())
            }
            Expr::Closure { params, body, .. } => self.compile_closure(params, body, dst),
            Expr::CallExpr { callee, args } => {
                let c = self.compile_expr(callee)?;
                let arg_count =
                    u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
                let args_start = self.reserve_regs(arg_count)?;
                for (i, arg) in args.iter().enumerate() {
                    self.compile_expr_into(arg, args_start + i as Reg)?;
                }
                self.emit(Op::CallValue {
                    dst,
                    callee: c,
                    args_start,
                    arg_count,
                    name_for_err: u32::MAX,
                });
                Ok(())
            }
            _ => Err("vm: unsupported expression".to_string()),
        }
    }

    /// Compile a `Repeat` loop (extracted: keeps `compile_stmt`'s recursion
    /// frame small in debug builds).
    fn compile_repeat(
        &mut self,
        pattern: &crate::ast::stmt::Pattern,
        iterable: &Expr,
        body: &[Stmt],
    ) -> Result<(), String> {
        use crate::ast::stmt::Pattern;

        let it_reg = self.compile_expr(iterable)?;
        self.emit(Op::IterPrepare { iterable: it_reg });

        // The loop variable lives in a scope spanning the whole loop
        // (the tree-walker pushes ONE scope outside the iteration).
        let outer_mark = self.enter_block();
        let loop_start;
        let next_idx;
        match pattern {
            Pattern::Identifier(sym) => {
                let dst = self.let_reg(*sym)?;
                loop_start = self.current_pc();
                next_idx = self.code.len();
                self.emit(Op::IterNext { dst, exit: usize::MAX });
            }
            Pattern::Tuple(syms) => {
                let tmp = self.alloc_reg()?;
                let count = u16::try_from(syms.len())
                    .map_err(|_| "vm: tuple pattern too long".to_string())?;
                let start = self.reserve_regs(count)?;
                for (i, sym) in syms.iter().enumerate() {
                    self.scopes.last_mut().unwrap().insert(*sym, start + i as Reg);
                    self.mark_named(start + i as Reg);
                }
                loop_start = self.current_pc();
                next_idx = self.code.len();
                self.emit(Op::IterNext { dst: tmp, exit: usize::MAX });
                self.emit(Op::DestructureTuple { src: tmp, start, count });
            }
        }

        self.flow_stack.push(FlowCtx::Loop { breaks: Vec::new(), is_repeat: true });
        let body_mark = self.enter_block();
        for st in body {
            self.compile_stmt(st)?;
        }
        self.exit_block(body_mark);
        self.emit(Op::Jump { target: loop_start });

        let exit_pc = self.current_pc();
        self.patch_jump_target(next_idx, exit_pc)?;
        if let Some(FlowCtx::Loop { breaks, .. }) = self.flow_stack.pop() {
            for j in breaks {
                self.patch_jump_target(j, exit_pc)?;
            }
        }
        self.emit(Op::IterPop);
        self.exit_block(outer_mark);
        Ok(())
    }

    /// Compile a `Zone`: the name is bound to Nothing for the body's duration,
    /// and the zone SWALLOWS Break/Return escaping the body.
    fn compile_zone(&mut self, name: Symbol, body: &[Stmt]) -> Result<(), String> {
        let outer_mark = self.enter_block();
        let name_reg = self.let_reg(name)?;
        let idx = self.add_const(Constant::Nothing)?;
        self.emit(Op::LoadConst { dst: name_reg, idx });
        self.flow_stack.push(FlowCtx::Zone { exits: Vec::new() });
        let body_mark = self.enter_block();
        for st in body {
            self.compile_stmt(st)?;
        }
        self.exit_block(body_mark);
        let end_pc = self.current_pc();
        if let Some(FlowCtx::Zone { exits }) = self.flow_stack.pop() {
            for j in exits {
                self.patch_jump_target(j, end_pc)?;
            }
        }
        self.exit_block(outer_mark);
        Ok(())
    }

    /// Compile `Inspect` as a chain of TestArm/JumpIfFalse arms (the
    /// tree-walker's linear arm scan).
    fn compile_inspect(
        &mut self,
        target: &Expr,
        arms: &[crate::ast::stmt::MatchArm],
    ) -> Result<(), String> {
        let t = self.compile_expr(target)?;
        let mut end_jumps: Vec<usize> = Vec::new();
        for arm in arms.iter() {
            match arm.variant {
                None => {
                    // Otherwise: unconditional, and no arm after it runs.
                    let mark = self.enter_block();
                    for st in arm.body {
                        self.compile_stmt(st)?;
                    }
                    self.exit_block(mark);
                    break;
                }
                Some(variant) => {
                    let vname = self.interner.resolve(variant).to_string();
                    let vidx = self.add_const(Constant::Text(vname))?;
                    let flag = self.alloc_reg()?;
                    self.emit(Op::TestArm { dst: flag, target: t, variant: vidx });
                    let jnext = self.emit_placeholder_jump_if_false(flag);

                    let mark = self.enter_block();
                    // Struct arms bind by field NAME, inductive arms by
                    // POSITION; the flavor is only known at runtime, so
                    // BindArm carries both and dispatches there.
                    for (i, (field_name, binding_name)) in arm.bindings.iter().enumerate() {
                        let dst = self.let_reg(*binding_name)?;
                        let fname = self.interner.resolve(*field_name).to_string();
                        let fidx = self.add_const(Constant::Text(fname))?;
                        self.emit(Op::BindArm {
                            dst,
                            target: t,
                            field: fidx,
                            index: u16::try_from(i)
                                .map_err(|_| "vm: too many arm bindings".to_string())?,
                        });
                    }
                    for st in arm.body {
                        self.compile_stmt(st)?;
                    }
                    self.exit_block(mark);
                    let j = self.emit_placeholder_jump();
                    end_jumps.push(j);
                    self.patch_jump_target(jnext, self.current_pc())?;
                }
            }
        }
        let end_pc = self.current_pc();
        for j in end_jumps {
            self.patch_jump_target(j, end_pc)?;
        }
        Ok(())
    }

    /// Compile `a new T …` — collections, or a struct with default-fill.
    fn compile_new(
        &mut self,
        type_name: Symbol,
        init_fields: &[(Symbol, &Expr)],
        dst: Reg,
    ) -> Result<(), String> {
        // Collection names win unconditionally (tree-walker order).
        match self.interner.resolve(type_name) {
            "Seq" | "List" => { self.emit(Op::NewEmptyList { dst }); return Ok(()); }
            "Set" | "HashSet" => { self.emit(Op::NewEmptySet { dst }); return Ok(()); }
            "Map" | "HashMap" => { self.emit(Op::NewEmptyMap { dst }); return Ok(()); }
            _ => {}
        }
        let name = self.interner.resolve(type_name).to_string();
        let name_idx = self.add_const(Constant::Text(name))?;
        self.emit(Op::NewStruct { dst, type_name: name_idx });

        let mut provided: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
        for (field_sym, field_expr) in init_fields {
            provided.insert(*field_sym);
            let fname = self.interner.resolve(*field_sym).to_string();
            let fidx = self.add_const(Constant::Text(fname))?;
            let v = self.compile_expr(field_expr)?;
            self.emit(Op::StructInsert { obj: dst, field: fidx, value: v });
        }

        // Default-fill the declared fields not provided (tree-walker defaults
        // by declared type name).
        if let Some(def) = self.struct_defs.get(&type_name).cloned() {
            for (field_sym, type_sym, _) in def {
                if provided.contains(&field_sym) {
                    continue;
                }
                let fname = self.interner.resolve(field_sym).to_string();
                let fidx = self.add_const(Constant::Text(fname))?;
                let v = self.alloc_reg()?;
                match self.interner.resolve(type_sym) {
                    "Int" | "Byte" => {
                        let i = self.add_const(Constant::Int(0))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Float" => {
                        let i = self.add_const(Constant::Float(0.0))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Bool" => {
                        let i = self.add_const(Constant::Bool(false))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Text" | "String" => {
                        let i = self.add_const(Constant::Text(String::new()))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Char" => {
                        let i = self.add_const(Constant::Char('\0'))?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                    "Seq" | "List" => self.emit(Op::NewEmptyList { dst: v }),
                    "Set" | "HashSet" => self.emit(Op::NewEmptySet { dst: v }),
                    "Map" | "HashMap" => self.emit(Op::NewEmptyMap { dst: v }),
                    // A `Shared` struct's CRDT collection fields default to an empty live
                    // CRDT, mirroring the tree-walker's `new`-struct init.
                    "SharedSet" | "ORSet" | "SharedSet_AddWins" => {
                        self.emit(Op::NewCrdt { dst: v, kind: 0 })
                    }
                    "SharedSet_RemoveWins" => self.emit(Op::NewCrdt { dst: v, kind: 3 }),
                    "SharedSequence" | "RGA" | "SharedSequence_YATA" | "CollaborativeSequence" => {
                        self.emit(Op::NewCrdt { dst: v, kind: 1 })
                    }
                    _ => {
                        let i = self.add_const(Constant::Nothing)?;
                        self.emit(Op::LoadConst { dst: v, idx: i });
                    }
                }
                self.emit(Op::StructInsert { obj: dst, field: fidx, value: v });
            }
        }
        Ok(())
    }

    /// Compile `and`/`or` with the tree-walker's exact evaluation order: an Int
    /// left operand always evaluates the right (bitwise/eager); a non-Int left
    /// short-circuits on truthiness. The right operand is compiled ONCE.
    ///
    /// ```text
    ///   rL = eval(left)
    ///   JumpIfInt   rL → eval         ; Int ⇒ eager path
    ///   JumpIfTrue  rL → eval  (and)  ; or: JumpIfFalse rL → eval
    ///   dst = false            (and)  ; or: dst = true   — short-circuit
    ///   Jump → end
    /// eval:
    ///   rR = eval(right)
    ///   dst = AndEager(rL, rR)        ; or: OrEager
    /// end:
    /// ```
    fn compile_short_circuit(
        &mut self,
        is_and: bool,
        left: &Expr,
        right: &Expr,
        dst: Reg,
    ) -> Result<(), String> {
        let l = self.compile_expr(left)?;

        let j_int = self.current_pc();
        self.emit(Op::JumpIfInt { cond: l, target: usize::MAX });
        let j_eval = self.current_pc();
        if is_and {
            self.emit(Op::JumpIfTrue { cond: l, target: usize::MAX });
        } else {
            self.emit(Op::JumpIfFalse { cond: l, target: usize::MAX });
        }

        // Short-circuit result: `and` → false, `or` → true.
        let idx = self.add_const(Constant::Bool(!is_and))?;
        self.emit(Op::LoadConst { dst, idx });
        let j_end = self.emit_placeholder_jump();

        let eval_pc = self.current_pc();
        self.patch_jump_target(j_int, eval_pc)?;
        self.patch_jump_target(j_eval, eval_pc)?;
        let r = self.compile_expr(right)?;
        if is_and {
            self.emit(Op::AndEager { dst, lhs: l, rhs: r });
        } else {
            self.emit(Op::OrEager { dst, lhs: l, rhs: r });
        }
        self.patch_jump_target(j_end, self.current_pc())?;
        Ok(())
    }

    /// Compile a call, writing the result to `dst`. Dispatch order mirrors the
    /// tree-walker exactly: `show` → kernel builtins → user functions →
    /// runtime "Unknown function". Arity errors fire at RUNTIME, BEFORE the
    /// arguments are evaluated.
    /// Compile `Launch a task to f with args` (and the handle-binding variant).
    /// Mirrors [`Self::compile_call`]'s function resolution + contiguous arg
    /// window, then emits `Spawn`/`SpawnHandle`.
    fn compile_spawn(
        &mut self,
        function: Symbol,
        args: &[&Expr],
        handle: Option<Symbol>,
    ) -> Result<(), String> {
        let func = match self.fn_index.get(&function) {
            Some(&f) => f,
            None => {
                return self.emit_fail(&format!(
                    "Unknown task function: {}",
                    self.interner.resolve(function)
                ))
            }
        };
        let arg_count =
            u16::try_from(args.len()).map_err(|_| "vm: too many task arguments".to_string())?;
        let args_start = self.reserve_regs(arg_count)?;
        for (i, arg) in args.iter().enumerate() {
            self.compile_expr_into(arg, args_start + i as Reg)?;
        }
        match handle {
            Some(h) => {
                let dst = self.let_reg(h)?;
                self.emit(Op::SpawnHandle { dst, func, args_start, arg_count });
            }
            None => self.emit(Op::Spawn { func, args_start, arg_count }),
        }
        Ok(())
    }

    fn compile_call(&mut self, function: Symbol, args: &[&Expr], dst: Reg) -> Result<(), String> {
        use crate::semantics::builtins::{builtin_from_name, check_arity, BuiltinId};

        let name = self.interner.resolve(function);
        if name == "show" {
            // show(a, b, …) emits each argument; result is Nothing.
            for arg in args {
                let src = self.compile_expr(arg)?;
                self.emit(Op::Show { src });
            }
            let idx = self.add_const(Constant::Nothing)?;
            self.emit(Op::LoadConst { dst, idx });
            return Ok(());
        }

        // `args()` is the program-argument system native (declared `## To
        // native args ()`); it resolves to the VM's stored argv, like the
        // compiled binary's `env::args()`. Intercepted by name BEFORE the
        // empty native-decl body would be reached, mirroring `show`.
        if name == "args" {
            self.emit(Op::Args { dst });
            return Ok(());
        }

        if let Some(id) = builtin_from_name(name) {
            if let Err(msg) = check_arity(id, args.len()) {
                let idx = self.add_const(Constant::Text(msg))?;
                self.emit(Op::FailWith { msg: idx });
                return Ok(());
            }
            // `format` evaluates only its first argument (tree-walker laziness).
            let used: &[&Expr] = if id == BuiltinId::Format && !args.is_empty() {
                &args[..1]
            } else {
                args
            };
            let arg_count =
                u16::try_from(used.len()).map_err(|_| "vm: too many arguments".to_string())?;
            let args_start = self.reserve_regs(arg_count)?;
            for (i, arg) in used.iter().enumerate() {
                self.compile_expr_into(arg, args_start + i as Reg)?;
            }
            self.emit(Op::CallBuiltin { dst, builtin: id, args_start, arg_count });
            return Ok(());
        }

        if let Some(&func) = self.fn_index.get(&function) {
            let param_count = self.functions[func as usize].param_count as usize;
            if args.len() != param_count {
                let msg = format!(
                    "Function {} expects {} arguments, got {}",
                    name,
                    param_count,
                    args.len()
                );
                let idx = self.add_const(Constant::Text(msg))?;
                self.emit(Op::FailWith { msg: idx });
                return Ok(());
            }
            let arg_count =
                u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
            let args_start = self.reserve_regs(arg_count)?;
            for (i, arg) in args.iter().enumerate() {
                self.compile_expr_into(arg, args_start + i as Reg)?;
            }
            self.emit(Op::Call { dst, func, args_start, arg_count });
            return Ok(());
        }

        // A variable holding a closure: call it by name. A bound non-Function
        // value errors "Unknown function: {name}" at runtime — the
        // tree-walker's by-name fallback.
        if !matches!(self.resolve_name(function), NameRef::Unbound) {
            let callee = self.compile_expr(&Expr::Identifier(function))?;
            let arg_count =
                u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
            let args_start = self.reserve_regs(arg_count)?;
            for (i, arg) in args.iter().enumerate() {
                self.compile_expr_into(arg, args_start + i as Reg)?;
            }
            let name_idx = self.add_const(Constant::Text(name.to_string()))?;
            self.emit(Op::CallValue { dst, callee, args_start, arg_count, name_for_err: name_idx });
            return Ok(());
        }

        let msg = format!("Unknown function: {}", name);
        let idx = self.add_const(Constant::Text(msg))?;
        self.emit(Op::FailWith { msg: idx });
        Ok(())
    }

    /// Compile `Show x to f` / `Give x to f` — the value goes to a FUNCTION
    /// recipient via the tree-walker's `call_function_with_values` dispatch,
    /// which knows only `show`, user functions, and closures (NOT the other
    /// builtins).
    fn compile_recipient_call(&mut self, recipient: Symbol, object: &Expr) -> Result<(), String> {
        let name = self.interner.resolve(recipient);
        if name == "show" {
            let src = self.compile_expr(object)?;
            self.emit(Op::Show { src });
            return Ok(());
        }
        if let Some(&func) = self.fn_index.get(&recipient) {
            let param_count = self.functions[func as usize].param_count as usize;
            let args_start = self.reserve_regs(1)?;
            self.compile_expr_into(object, args_start)?;
            if param_count != 1 {
                let msg = format!(
                    "Function {} expects {} arguments, got 1",
                    name, param_count
                );
                let idx = self.add_const(Constant::Text(msg))?;
                self.emit(Op::FailWith { msg: idx });
                return Ok(());
            }
            let dst = self.alloc_reg()?;
            self.emit(Op::Call { dst, func, args_start, arg_count: 1 });
            return Ok(());
        }
        // A variable holding a closure (the with_values closure fallback).
        if !matches!(self.resolve_name(recipient), NameRef::Unbound) {
            let args_start = self.reserve_regs(1)?;
            self.compile_expr_into(object, args_start)?;
            let callee = self.compile_expr(&Expr::Identifier(recipient))?;
            let dst = self.alloc_reg()?;
            let name_idx = self.add_const(Constant::Text(name.to_string()))?;
            self.emit(Op::CallValue { dst, callee, args_start, arg_count: 1, name_for_err: name_idx });
            return Ok(());
        }
        // The object's side effects happen before the dispatch failure.
        let scratch = self.alloc_reg()?;
        self.compile_expr_into(object, scratch)?;
        let msg = format!("Unknown function: {}", name);
        let idx = self.add_const(Constant::Text(msg))?;
        self.emit(Op::FailWith { msg: idx });
        Ok(())
    }

    /// Compile an interpolated string: parts accumulate into a Text register
    /// via Concat (Text+Text concatenation — identical to the tree-walker's
    /// push_str building).
    fn compile_interpolation(
        &mut self,
        parts: &[crate::ast::stmt::StringPart],
        dst: Reg,
    ) -> Result<(), String> {
        use crate::ast::stmt::StringPart;

        let empty = self.add_const(Constant::Text(String::new()))?;
        self.emit(Op::LoadConst { dst, idx: empty });
        for part in parts {
            match part {
                StringPart::Literal(sym) => {
                    let idx =
                        self.add_const(Constant::Text(self.interner.resolve(*sym).to_string()))?;
                    let lit = self.alloc_reg()?;
                    self.emit(Op::LoadConst { dst: lit, idx });
                    self.emit(Op::Concat { dst, lhs: dst, rhs: lit });
                }
                StringPart::Expr { value, format_spec, debug } => {
                    let v = self.compile_expr(value)?;
                    let needs_format = format_spec.is_some() || *debug;
                    let piece = if needs_format {
                        let spec = match format_spec {
                            Some(sym) => self.add_const(Constant::Text(
                                self.interner.resolve(*sym).to_string(),
                            ))?,
                            None => u32::MAX,
                        };
                        let debug_prefix = if *debug {
                            let prefix = match value {
                                Expr::Identifier(sym) => {
                                    self.interner.resolve(*sym).to_string()
                                }
                                _ => "expr".to_string(),
                            };
                            self.add_const(Constant::Text(prefix))?
                        } else {
                            u32::MAX
                        };
                        let formatted = self.alloc_reg()?;
                        self.emit(Op::FormatValue { dst: formatted, src: v, spec, debug_prefix });
                        formatted
                    } else {
                        v
                    };
                    self.emit(Op::Concat { dst, lhs: dst, rhs: piece });
                }
            }
        }
        Ok(())
    }

    /// Compile a closure literal: the body becomes an anonymous function
    /// emitted INLINE (jumped over at the creation site); local captures are
    /// snapshotted from a register window, global captures from the globals
    /// table at creation time. Frame layout:
    /// `[params…, capture values…, capture-present flags…]`.
    fn compile_closure(
        &mut self,
        params: &[(Symbol, &crate::ast::stmt::TypeExpr)],
        body: &crate::ast::stmt::ClosureBody,
        dst: Reg,
    ) -> Result<(), String> {
        use crate::ast::stmt::ClosureBody;
        use crate::interpreter::Interpreter;

        let free = Interpreter::free_vars_in_closure(params, body);
        let mut captures: Vec<(Symbol, Option<u16>)> = Vec::new();
        let mut local_sources: Vec<Reg> = Vec::new();
        for sym in free {
            match self.resolve_name(sym) {
                NameRef::Local(r) => {
                    captures.push((sym, None));
                    local_sources.push(r);
                }
                NameRef::CaptureOrGlobal { value, global, .. } => {
                    // Closure-in-closure over a promoted capture: snapshot the
                    // capture slot (the live-global fall-through nests with
                    // the outer flag at creation time only).
                    let _ = global;
                    captures.push((sym, None));
                    local_sources.push(value);
                }
                NameRef::Global(g) => captures.push((sym, Some(g))),
                NameRef::Unbound => {}
            }
        }

        // Move local capture sources into a contiguous window for MakeClosure.
        let local_count =
            u16::try_from(local_sources.len()).map_err(|_| "vm: too many captures".to_string())?;
        let locals_start = self.reserve_regs(local_count)?;
        for (i, src) in local_sources.iter().enumerate() {
            let w = locals_start + i as Reg;
            if *src != w {
                self.emit(Op::Move { dst: w, src: *src });
            }
        }

        // Register the body as a function and compile it inline, jumped over.
        let func_idx = u16::try_from(self.functions.len())
            .map_err(|_| "vm: too many functions".to_string())?;
        let param_count =
            u16::try_from(params.len()).map_err(|_| "vm: too many parameters".to_string())?;
        let jover = self.emit_placeholder_jump();
        self.functions.push(CompiledFunction {
            name: Symbol::default(),
            entry_pc: self.code.len(),
            param_count,
            register_count: 0,
            captures: captures.clone(),
            named_regs: Vec::new(),
            // Closures carry no declarations — all-Int is the historical
            // entry-guard contract.
            param_kinds: vec![
                Some(super::native_tier::ParamKind::Scalar(
                    super::native_tier::SlotKind::Int
                ));
                param_count as usize
            ],
            ret_kind: None,
        });

        // Shelve the enclosing frame's compilation state.
        let saved_scopes = std::mem::replace(&mut self.scopes, vec![HashMap::new()]);
        let saved_next = self.next_reg;
        let saved_max = self.max_reg;
        let saved_flow = std::mem::take(&mut self.flow_stack);
        let saved_in_fn = self.in_function;
        let saved_ctx = self.closure_ctx.take();
        let saved_named = std::mem::take(&mut self.named);

        self.in_function = true;
        let p = param_count;
        let cap_n = captures.len() as Reg;
        for (i, (psym, _)) in params.iter().enumerate() {
            self.scopes.last_mut().unwrap().insert(*psym, i as Reg);
            self.mark_named(i as Reg);
        }
        let mut ctx: HashMap<Symbol, (Reg, Reg)> = HashMap::new();
        for (k, (sym, global)) in captures.iter().enumerate() {
            let value = p + k as Reg;
            self.scopes.last_mut().unwrap().insert(*sym, value);
            self.mark_named(value);
            if global.is_some() {
                ctx.insert(*sym, (value, p + cap_n + k as Reg));
            }
        }
        self.next_reg = p + 2 * cap_n;
        self.max_reg = self.next_reg;
        self.closure_ctx = Some(ctx);

        let body_result = (|| -> Result<(), String> {
            match body {
                ClosureBody::Expression(e) => {
                    let r = self.compile_expr(e)?;
                    self.emit(Op::Return { src: r });
                }
                ClosureBody::Block(block) => {
                    for st in block.iter() {
                        self.compile_stmt(st)?;
                    }
                    self.emit(Op::ReturnNothing);
                }
            }
            Ok(())
        })();
        self.functions[func_idx as usize].register_count = self.max_reg as usize;
        let mut cnamed = std::mem::replace(&mut self.named, saved_named);
        cnamed.resize(self.functions[func_idx as usize].register_count, false);
        self.functions[func_idx as usize].named_regs = cnamed;

        // Restore the enclosing frame's state (even when the body failed).
        self.scopes = saved_scopes;
        self.next_reg = saved_next;
        self.max_reg = saved_max.max(self.next_reg);
        self.flow_stack = saved_flow;
        self.in_function = saved_in_fn;
        self.closure_ctx = saved_ctx;
        body_result?;

        self.patch_jump_target(jover, self.current_pc())?;
        self.emit(Op::MakeClosure { dst, func: func_idx, locals_start });
        Ok(())
    }
}

/// Collect identifiers that appear inside function or closure bodies — the
/// names that, when bound at Main top level, must live in the globals table.
/// `at_main` is true while walking Main-level statements (whose own
/// identifiers are frame-local); inside a FunctionDef body or a Closure body
/// every identifier counts.
fn collect_nonlocal_idents_stmt(
    s: &Stmt,
    at_main: bool,
    out: &mut std::collections::HashSet<Symbol>,
) {
    use crate::ast::stmt::ClosureBody;
    use crate::interpreter::Interpreter;

    if let Stmt::FunctionDef { params, body, .. } = s {
        // Everything free in a function body is a nonlocal reference.
        let free = Interpreter::free_vars_in_closure(params, &ClosureBody::Block(body));
        out.extend(free);
        return;
    }
    // Walk this statement's expressions for Closure literals (whose free vars
    // are nonlocal) and recurse into nested statements.
    visit_stmt_exprs(s, &mut |e| {
        if let Expr::Closure { params, body, .. } = e {
            let free = Interpreter::free_vars_in_closure(params, body);
            out.extend(free);
        }
    });
    for_each_child_block(s, &mut |block| {
        for st in block {
            collect_nonlocal_idents_stmt(st, at_main, out);
        }
    });
}

/// Apply `f` to every expression directly held by `s` (and, via the walker in
/// `f` itself when needed, their subexpressions are reached by the free-vars
/// collector — Closure bodies included).
fn visit_stmt_exprs(s: &Stmt, f: &mut dyn FnMut(&Expr)) {
    // Iterative walk: expressions can be arbitrarily deep (degenerate inputs),
    // so no native recursion here.
    fn walk_expr<'a>(root: &'a Expr<'a>, f: &mut dyn FnMut(&Expr)) {
        let mut stack: Vec<&Expr> = vec![root];
        while let Some(e) = stack.pop() {
            f(e);
            match e {
                Expr::BinaryOp { left, right, .. } => {
                    stack.push(left);
                    stack.push(right);
                }
                Expr::Not { operand } => stack.push(operand),
                Expr::Call { args, .. } => stack.extend(args.iter().copied()),
                Expr::CallExpr { callee, args } => {
                    stack.push(callee);
                    stack.extend(args.iter().copied());
                }
                Expr::Index { collection, index } => {
                    stack.push(collection);
                    stack.push(index);
                }
                Expr::Slice { collection, start, end } => {
                    stack.push(collection);
                    stack.push(start);
                    stack.push(end);
                }
                Expr::Copy { expr } => stack.push(expr),
                Expr::Give { value } => stack.push(value),
                Expr::Length { collection } => stack.push(collection),
                Expr::Contains { collection, value } => {
                    stack.push(collection);
                    stack.push(value);
                }
                Expr::Union { left, right } | Expr::Intersection { left, right } => {
                    stack.push(left);
                    stack.push(right);
                }
                Expr::List(items) | Expr::Tuple(items) => stack.extend(items.iter().copied()),
                Expr::Range { start, end } => {
                    stack.push(start);
                    stack.push(end);
                }
                Expr::FieldAccess { object, .. } => stack.push(object),
                Expr::New { init_fields, .. } => {
                    stack.extend(init_fields.iter().map(|(_, fe)| *fe));
                }
                Expr::NewVariant { fields, .. } => {
                    stack.extend(fields.iter().map(|(_, fe)| *fe));
                }
                Expr::OptionSome { value } => stack.push(value),
                Expr::WithCapacity { value, .. } => stack.push(value),
                _ => {}
            }
        }
    }

    use crate::ast::stmt::ReadSource;
    match s {
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => walk_expr(value, f),
        Stmt::Return { value: Some(e) } => walk_expr(e, f),
        Stmt::Call { args, .. } => {
            for a in args {
                walk_expr(a, f);
            }
        }
        Stmt::If { cond, .. } | Stmt::While { cond, .. } => walk_expr(cond, f),
        Stmt::Repeat { iterable, .. } => walk_expr(iterable, f),
        Stmt::Show { object, .. } | Stmt::Give { object, .. } => walk_expr(object, f),
        Stmt::Push { value, collection }
        | Stmt::Add { value, collection }
        | Stmt::Remove { value, collection } => {
            walk_expr(value, f);
            walk_expr(collection, f);
        }
        Stmt::SetIndex { collection, index, value } => {
            walk_expr(collection, f);
            walk_expr(index, f);
            walk_expr(value, f);
        }
        Stmt::SetField { object, value, .. } => {
            walk_expr(object, f);
            walk_expr(value, f);
        }
        Stmt::Inspect { target, .. } => walk_expr(target, f),
        Stmt::RuntimeAssert { condition, .. } => walk_expr(condition, f),
        Stmt::Sleep { milliseconds } => walk_expr(milliseconds, f),
        Stmt::IncreaseCrdt { amount, .. } | Stmt::DecreaseCrdt { amount, .. } => {
            walk_expr(amount, f)
        }
        Stmt::MergeCrdt { source, .. } => walk_expr(source, f),
        Stmt::WriteFile { content, path } => {
            walk_expr(content, f);
            walk_expr(path, f);
        }
        Stmt::ReadFrom { source: ReadSource::File(p), .. } => walk_expr(p, f),
        _ => {}
    }
}

/// Apply `f` to every nested statement block of `s`.
fn for_each_child_block<'a>(s: &Stmt<'a>, f: &mut dyn FnMut(&[Stmt<'a>])) {
    match s {
        Stmt::If { then_block, else_block, .. } => {
            f(then_block);
            if let Some(eb) = else_block {
                f(eb);
            }
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => f(body),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => f(tasks),
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                f(arm.body);
            }
        }
        _ => {}
    }
}

/// Rewrite the placeholder target of the jump at `idx`. Total: a non-jump op or
/// an out-of-bounds index is a compiler-internal bug surfaced as `Err`.
pub(super) fn patch_jump(code: &mut [Op], idx: usize, target: usize) -> Result<(), String> {
    match code.get_mut(idx) {
        Some(Op::Jump { target: t })
        | Some(Op::JumpIfFalse { target: t, .. })
        | Some(Op::JumpIfTrue { target: t, .. })
        | Some(Op::JumpIfInt { target: t, .. })
        | Some(Op::IterNext { exit: t, .. }) => {
            *t = target;
            Ok(())
        }
        Some(other) => Err(format!("vm: patch_jump on non-jump op {:?}", other)),
        None => Err(format!("vm: patch_jump index {} out of bounds", idx)),
    }
}

fn literal_const(lit: &Literal) -> Result<Constant, String> {
    match lit {
        Literal::Number(n) => Ok(Constant::Int(*n)),
        Literal::Float(f) => Ok(Constant::Float(*f)),
        Literal::Boolean(b) => Ok(Constant::Bool(*b)),
        Literal::Nothing => Ok(Constant::Nothing),
        Literal::Char(c) => Ok(Constant::Char(*c)),
        Literal::Duration(nanos) => Ok(Constant::Duration(*nanos)),
        Literal::Date(days) => Ok(Constant::Date(*days)),
        Literal::Moment(nanos) => Ok(Constant::Moment(*nanos)),
        Literal::Span { months, days } => Ok(Constant::Span { months: *months, days: *days }),
        Literal::Time(nanos) => Ok(Constant::Time(*nanos)),
        Literal::Text(_) => unreachable!("Text literals are interned and handled by the caller"),
    }
}

/// Match `value` as the left-spine `target + e1 + … + ek` (k ≥ 1), the
/// accumulate shape `Set x to x + …` lowers to an [`Op::AddAssign`] chain —
/// the exact left-fold the expression denotes: same kernel calls, same
/// evaluation order, same error points, but folding into the target register
/// at each step so a sole-owner Text can extend in place.
///
/// Soundness: the first term evaluates before any fold touches the register,
/// so it may mention the target freely. Every LATER term runs after a fold —
/// it must provably never observe the target ([`expr_avoids`]), or the chain
/// would read a partially-folded value the original expression never exposes.
fn add_assign_chain<'a>(target: Symbol, value: &'a Expr<'a>) -> Option<Vec<&'a Expr<'a>>> {
    let mut terms: Vec<&Expr> = Vec::new();
    let mut cur = value;
    loop {
        match cur {
            Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => {
                terms.push(right);
                cur = left;
            }
            Expr::Identifier(sym) if *sym == target => break,
            _ => return None,
        }
    }
    if terms.is_empty() {
        return None;
    }
    terms.reverse();
    if terms[1..].iter().all(|t| expr_avoids(t, target)) {
        Some(terms)
    } else {
        None
    }
}

/// True when `e` provably never observes `sym`: every node comes from a
/// closed set of value-only forms with no `Identifier(sym)` anywhere.
/// Closures are conservatively "may observe" — they snapshot captures at
/// evaluation, and under the AddAssign rewrite that snapshot would see the
/// partially-folded register. Named calls cannot read the caller's locals
/// (and the rewrite only fires for locals), so only their argument
/// expressions need scanning. Anything unrecognized fails closed.
fn expr_avoids(e: &Expr, sym: Symbol) -> bool {
    use crate::ast::stmt::StringPart;
    let mut stack: Vec<&Expr> = vec![e];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Identifier(s) => {
                if *s == sym {
                    return false;
                }
            }
            Expr::Literal(_) | Expr::OptionNone => {}
            Expr::BinaryOp { left, right, .. }
            | Expr::Union { left, right }
            | Expr::Intersection { left, right }
            | Expr::Range { start: left, end: right } => {
                stack.push(left);
                stack.push(right);
            }
            Expr::Not { operand } => stack.push(operand),
            Expr::Call { args, .. } => stack.extend(args.iter().copied()),
            Expr::CallExpr { callee, args } => {
                stack.push(callee);
                stack.extend(args.iter().copied());
            }
            Expr::Index { collection, index } => {
                stack.push(collection);
                stack.push(index);
            }
            Expr::Slice { collection, start, end } => {
                stack.push(collection);
                stack.push(start);
                stack.push(end);
            }
            Expr::Copy { expr } => stack.push(expr),
            Expr::Length { collection } => stack.push(collection),
            Expr::Contains { collection, value } => {
                stack.push(collection);
                stack.push(value);
            }
            Expr::List(items) | Expr::Tuple(items) => stack.extend(items.iter().copied()),
            Expr::FieldAccess { object, .. } => stack.push(object),
            Expr::New { init_fields, .. } => {
                stack.extend(init_fields.iter().map(|(_, fe)| *fe));
            }
            Expr::NewVariant { fields, .. } => {
                stack.extend(fields.iter().map(|(_, fe)| *fe));
            }
            Expr::OptionSome { value } => stack.push(value),
            Expr::WithCapacity { value, capacity } => {
                stack.push(value);
                stack.push(capacity);
            }
            Expr::InterpolatedString(parts) => {
                for p in parts {
                    if let StringPart::Expr { value, .. } = p {
                        stack.push(value);
                    }
                }
            }
            _ => return false,
        }
    }
    true
}

fn binop_op(op: BinaryOpKind, dst: Reg, lhs: Reg, rhs: Reg) -> Result<Op, String> {
    use BinaryOpKind::*;
    Ok(match op {
        Add => Op::Add { dst, lhs, rhs },
        Subtract => Op::Sub { dst, lhs, rhs },
        Multiply => Op::Mul { dst, lhs, rhs },
        Divide => Op::Div { dst, lhs, rhs },
        ExactDivide => Op::ExactDiv { dst, lhs, rhs },
        Modulo => Op::Mod { dst, lhs, rhs },
        Lt => Op::Lt { dst, lhs, rhs },
        Gt => Op::Gt { dst, lhs, rhs },
        LtEq => Op::LtEq { dst, lhs, rhs },
        GtEq => Op::GtEq { dst, lhs, rhs },
        Eq => Op::Eq { dst, lhs, rhs },
        NotEq => Op::NotEq { dst, lhs, rhs },
        Concat => Op::Concat { dst, lhs, rhs },
        BitXor => Op::BitXor { dst, lhs, rhs },
        Shl => Op::Shl { dst, lhs, rhs },
        Shr => Op::Shr { dst, lhs, rhs },
        // And/Or are compiled by `compile_short_circuit`, never through here.
        And | Or => return Err("vm: internal — And/Or must use compile_short_circuit".to_string()),
    })
}
