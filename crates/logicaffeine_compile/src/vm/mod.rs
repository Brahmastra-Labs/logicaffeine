//! Register bytecode VM (VM_PLAN.md).
//!
//! A fast, portable interpreter that runs the same `RuntimeValue` semantics as
//! the tree-walker. It is the browser/WASM execution engine and the substrate
//! the native copy-and-patch JIT tiers up from. Built incrementally, RED-first,
//! differential-tested against the tree-walker.

pub mod compiler;
// Off-thread native compilation (HOTSWAP §6). Native-only: it needs std::thread and
// the forge backend, neither of which exists on wasm (which never installs a tier).
#[cfg(not(target_arch = "wasm32"))]
mod bg_compile;
// AOT-native tier loader (HOTSWAP §Axis-3): dlopen a rustc-built cdylib. Native-only.
#[cfg(not(target_arch = "wasm32"))]
pub mod aot_tier;
// Background AOT-native compilation (HOTSWAP §Axis-3 / P18): build+load off-thread.
#[cfg(not(target_arch = "wasm32"))]
pub mod bg_aot;
// Relocatable per-function bytecode units (HOTSWAP §7) — the producer behind the
// Axis-1 warm side-table and the OPFS tier cache. Pure data; available on all targets.
pub mod fn_bytecode;
// LOGOS_TIER_TRACE observability (HOTSWAP §P13): one stderr line per tier hot-swap.
pub mod tier_trace;
// Tier cache (HOTSWAP §P12): persist a compiled FnBytecode keyed by source+config+tier.
pub mod tier_cache;
#[cfg(test)]
mod fuzz;
/// Bytecode disassembler — readable text for the Studio debug drawer and any
/// future `largo --disasm`. Pure, headless, no VM state.
pub mod disasm;
mod instruction;
mod machine;
/// WS-F representation overhaul: the narrow (8-byte) NaN-boxed register-file
/// value (see the module docs). Under `feature = "narrow-value"` it backs
/// [`value::Value`] and the VM register file; the default build keeps the fat
/// `RuntimeValue` path, so the scalar accessors/arith here are exercised only by
/// the in-module tests then. `allow(dead_code)` keeps the default build quiet
/// about the methods only the narrow path calls.
#[cfg_attr(not(feature = "narrow-value"), allow(dead_code))]
mod nanbox;
mod native_tier;
mod value;

/// WS6 (Phase 13): the browser WASM-JIT backend (emits a WebAssembly module per hot
/// region). Default-off; builds for and runs on wasm32, unlike the native x86 forge JIT.
#[cfg(feature = "wasm-jit")]
pub mod wasm_jit;

pub use compiler::Compiler;
pub use disasm::{disassemble, format_constant, op_io, DisasmLine, OpIo};
pub use instruction::{CompiledProgram, Constant, Op};
pub use machine::Vm;
pub(crate) use machine::{DebugFrameView, DebugView, DebugVmState, HeapObjView};
pub(crate) use machine::{VmBlock, VmStep};
pub use native_tier::{
    install_native_tier, installed_native_tier, ArrayPin, CalleeSig, FnTable, HoistGuard, NativeCtx,
    NativeFn,
    NativeFrame, NativeOutcome, NativeRet, NativeTier, ObservedKind, ParamKind, PinElem,
    RegBox, RegionFn, RegionOutcome, RegionReturn, RegionReturnKind, SlotKind,
    NATIVE_TIER_THRESHOLD, REGION_TIER_THRESHOLD,
};
pub use value::Value;

use crate::ast::stmt::Stmt;
use crate::intern::Interner;

/// Hard cap on registers a single frame may claim. Generous for real programs;
/// prevents a pathological frame from claiming the whole register file.
pub const MAX_REGISTERS_PER_FRAME: usize = 16_384;
/// Hard cap on the total register file across all live frames. Deep recursion
/// hits this and errors instead of consuming unbounded memory.
pub const MAX_REGISTER_FILE: usize = 1 << 20;
/// Maximum expression nesting depth the compiler will descend before erroring
/// (guards the compiler's own recursion against adversarial inputs). Debug
/// builds allocate every match arm's locals in `compile_expr_into_inner`'s
/// frame — measured in the tens of KiB per nesting level as the compiler
/// grows — so 64 keeps the worst case well inside a default 2 MiB
/// test-thread stack while still exceeding any realistic expression depth.
/// (An explicit-work-stack compiler would remove the native-stack coupling
/// entirely; tracked as future work.)
pub const MAX_EXPR_DEPTH: usize = 64;

/// Shannon entropy of a two-outcome branch profile, in bits (EXODIA 3.3).
/// 0.0 = perfectly predictable (the hardware branch predictor wins; Tier-1
/// code is fine), 1.0 = a coin flip (Tier-2 should restructure the branch
/// away). Zero samples are zero entropy.
pub fn branch_entropy(taken: u64, not_taken: u64) -> f64 {
    let total = taken + not_taken;
    if total == 0 || taken == 0 || not_taken == 0 {
        return 0.0;
    }
    let p = taken as f64 / total as f64;
    -(p * p.log2() + (1.0 - p) * (1.0 - p).log2())
}

/// Compile a statement block to bytecode and run it, returning captured output.
pub fn compile_and_run(stmts: &[Stmt], interner: &Interner) -> Result<String, String> {
    let program = Compiler::compile(stmts, interner)?;
    let mut vm = Vm::new(&program);
    if let Some(tier) = installed_native_tier() {
        vm = vm.with_native_tier(tier);
    }
    vm.run()?;
    Ok(vm.output().to_string())
}

/// Compile and run, preserving partial output alongside any error — the shape
/// the differential harness compares against the tree-walker (which also keeps
/// the lines emitted before a runtime error). `types` carries the discovery
/// pass's struct definitions (for default-fill on construction).
pub fn run_to_outcome(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    policies: Option<&crate::analysis::PolicyRegistry>,
) -> (String, Option<String>) {
    let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
    let program = match Compiler::compile_with_oracle(stmts, interner, types, Some(oracle)) {
        Ok(p) => p,
        Err(e) => return (String::new(), Some(e)),
    };
    let mut vm = Vm::new(&program);
    if let Some(tier) = installed_native_tier() {
        vm = vm.with_native_tier(tier);
    }
    if let Some(registry) = policies {
        vm = vm.with_policy_ctx(registry, interner);
    }
    match vm.run() {
        Ok(()) => (vm.output().to_string(), None),
        Err(e) => (vm.output().to_string(), Some(e)),
    }
}

/// [`run_to_outcome`] with the program argument vector for the `args()`
/// system native (full argv; index 0 is the program name) and an optional
/// caller-supplied native tier. A `Some(tier)` overrides the process-wide
/// installed tier — differential suites use a private tier per program so
/// its compile counters are isolated from every other test in the binary.
pub fn run_to_outcome_with_args(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    policies: Option<&crate::analysis::PolicyRegistry>,
    program_args: &[String],
    tier: Option<&dyn NativeTier>,
) -> (String, Option<String>) {
    let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
    let program = match Compiler::compile_with_oracle(stmts, interner, types, Some(oracle)) {
        Ok(p) => p,
        Err(e) => return (String::new(), Some(e)),
    };
    let mut vm = Vm::new(&program).with_program_args(program_args.to_vec());
    let chosen: Option<&dyn NativeTier> = match tier {
        Some(t) => Some(t),
        None => installed_native_tier().map(|t| t as &dyn NativeTier),
    };
    if let Some(t) = chosen {
        vm = vm.with_native_tier(t);
    }
    if let Some(registry) = policies {
        vm = vm.with_policy_ctx(registry, interner);
    }
    match vm.run() {
        Ok(()) => (vm.output().to_string(), None),
        Err(e) => (vm.output().to_string(), Some(e)),
    }
}

/// [`run_to_outcome_with_args`] with BACKGROUND native compilation (HOTSWAP §6): hot
/// functions compile on a worker thread while the interpreter keeps running bytecode,
/// then their chains are drained + published. Requires a process-installed tier (the
/// worker needs a `&'static` backend); with none installed it runs pure bytecode.
/// Drains outstanding compiles before returning so the native tier engages
/// deterministically for the differential gates.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_to_outcome_bg(
    stmts: &[Stmt],
    interner: &Interner,
    types: Option<&crate::analysis::TypeRegistry>,
    policies: Option<&crate::analysis::PolicyRegistry>,
    program_args: &[String],
) -> (String, Option<String>) {
    let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
    let program = match Compiler::compile_with_oracle(stmts, interner, types, Some(oracle)) {
        Ok(p) => p,
        Err(e) => return (String::new(), Some(e)),
    };
    let mut vm = Vm::new(&program).with_program_args(program_args.to_vec());
    if let Some(t) = installed_native_tier() {
        vm = vm.with_bg_native_tier(t);
    }
    if let Some(registry) = policies {
        vm = vm.with_policy_ctx(registry, interner);
    }
    let result = match vm.run() {
        Ok(()) => (vm.output().to_string(), None),
        Err(e) => (vm.output().to_string(), Some(e)),
    };
    vm.drain_pending_compiles();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Arena;
    use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
    use crate::intern::Interner;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Run a program through the production tree-walker, capturing its output —
    /// the independent oracle every VM result is checked against.
    fn run_treewalk(stmts: &[Stmt], interner: &Interner) -> Result<String, String> {
        use crate::interpreter::{Interpreter, OutputCallback};
        let buf = Rc::new(RefCell::new(String::new()));
        let sink = buf.clone();
        let cb: OutputCallback = Rc::new(RefCell::new(move |s: String| {
            sink.borrow_mut().push_str(&s);
            sink.borrow_mut().push('\n');
        }));
        let mut interp = Interpreter::new(interner).with_output_callback(cb);
        interp.run_sync(stmts)?;
        let out = buf.borrow().clone();
        Ok(out)
    }

    fn normalize(s: &str) -> String {
        s.lines()
            .map(|l| l.trim_end())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The differential gate: the VM's observable output must equal the
    /// tree-walker's for the same program.
    fn assert_vm_eq_treewalk(stmts: &[Stmt], interner: &Interner) {
        let vm_out = compile_and_run(stmts, interner).expect("vm run failed");
        let tw_out = run_treewalk(stmts, interner).expect("tree-walker run failed");
        assert_eq!(
            normalize(&vm_out),
            normalize(&tw_out),
            "VM output diverged from the tree-walker oracle"
        );
    }

    // ---- AST builder helpers (keep the function tests readable) ---------------

    use crate::ast::stmt::TypeExpr;
    use crate::intern::Symbol;

    fn num<'a>(ea: &'a Arena<Expr<'a>>, n: i64) -> &'a Expr<'a> {
        ea.alloc(Expr::Literal(Literal::Number(n)))
    }
    fn idref<'a>(ea: &'a Arena<Expr<'a>>, s: Symbol) -> &'a Expr<'a> {
        ea.alloc(Expr::Identifier(s))
    }
    fn bin<'a>(ea: &'a Arena<Expr<'a>>, op: BinaryOpKind, l: &'a Expr<'a>, r: &'a Expr<'a>) -> &'a Expr<'a> {
        ea.alloc(Expr::BinaryOp { op, left: l, right: r })
    }
    fn calle<'a>(ea: &'a Arena<Expr<'a>>, f: Symbol, args: Vec<&'a Expr<'a>>) -> &'a Expr<'a> {
        ea.alloc(Expr::Call { function: f, args })
    }
    fn letb<'a>(var: Symbol, value: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Let { var, ty: None, value, mutable: false }
    }
    fn ret<'a>(value: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Return { value: Some(value) }
    }
    fn show<'a>(ea: &'a Arena<Expr<'a>>, show_sym: Symbol, obj: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Show { object: obj, recipient: idref(ea, show_sym) }
    }
    fn fndef<'a>(
        name: Symbol,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>,
        body: &'a [Stmt<'a>],
    ) -> Stmt<'a> {
        Stmt::FunctionDef {
            name,
            generics: vec![],
            params,
            body,
            return_type: None,
            is_native: false,
            native_path: None,
            is_exported: false,
            export_target: None,
            opt_flags: Default::default(),
        }
    }

    #[test]
    fn vm_function_single_param() {
        // ## To double (n: Int) -> Int: Return n * 2.
        // Main: Let r be double(21). Show r.   → 42
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let double = it.intern("double");
        let n = it.intern("n");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let body: &[Stmt] = sa.alloc_slice(vec![ret(bin(&ea, BinaryOpKind::Multiply, idref(&ea, n), num(&ea, 2)))]);
        let main_call = calle(&ea, double, vec![num(&ea, 21)]);

        let stmts = vec![
            fndef(double, vec![(n, int_ty)], body),
            letb(r, main_call),
            show(&ea, show_s, idref(&ea, r)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "42");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_function_two_params() {
        // ## To add (a, b): Return a + b.  Main: Show add(3, 4).   → 7
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let add = it.intern("add");
        let a = it.intern("a");
        let b = it.intern("b");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let body: &[Stmt] = sa.alloc_slice(vec![ret(bin(&ea, BinaryOpKind::Add, idref(&ea, a), idref(&ea, b)))]);
        let stmts = vec![
            fndef(add, vec![(a, int_ty), (b, int_ty)], body),
            letb(r, calle(&ea, add, vec![num(&ea, 3), num(&ea, 4)])),
            show(&ea, show_s, idref(&ea, r)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "7");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_function_recursion_factorial() {
        // ## To factorial (n: Int): If n <= 1: Return 1. Return n * factorial(n - 1).
        // Main: Show factorial(5).   → 120
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let factorial = it.intern("factorial");
        let n = it.intern("n");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let then_blk: &[Stmt] = sa.alloc_slice(vec![ret(num(&ea, 1))]);
        let cond = bin(&ea, BinaryOpKind::LtEq, idref(&ea, n), num(&ea, 1));
        let rec = calle(&ea, factorial, vec![bin(&ea, BinaryOpKind::Subtract, idref(&ea, n), num(&ea, 1))]);
        let tail = ret(bin(&ea, BinaryOpKind::Multiply, idref(&ea, n), rec));
        let body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::If { cond, then_block: then_blk, else_block: None },
            tail,
        ]);

        let stmts = vec![
            fndef(factorial, vec![(n, int_ty)], body),
            letb(r, calle(&ea, factorial, vec![num(&ea, 5)])),
            show(&ea, show_s, idref(&ea, r)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "120");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_function_nested_calls() {
        // triple(n)=n*3, double(n)=n*2.  Main: Show double(triple(2)).   → 12
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let triple = it.intern("triple");
        let double = it.intern("double");
        let n = it.intern("n");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let triple_body: &[Stmt] = sa.alloc_slice(vec![ret(bin(&ea, BinaryOpKind::Multiply, idref(&ea, n), num(&ea, 3)))]);
        let double_body: &[Stmt] = sa.alloc_slice(vec![ret(bin(&ea, BinaryOpKind::Multiply, idref(&ea, n), num(&ea, 2)))]);
        let inner = calle(&ea, triple, vec![num(&ea, 2)]);
        let outer = calle(&ea, double, vec![inner]);
        let stmts = vec![
            fndef(triple, vec![(n, int_ty)], triple_body),
            fndef(double, vec![(n, int_ty)], double_body),
            letb(r, outer),
            show(&ea, show_s, idref(&ea, r)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "12");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_function_with_local_variable() {
        // ## To compute (x): Let y be x * x. Return y + 1.  Main: Show compute(4).   → 17
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let compute = it.intern("compute");
        let x = it.intern("x");
        let y = it.intern("y");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let body: &[Stmt] = sa.alloc_slice(vec![
            letb(y, bin(&ea, BinaryOpKind::Multiply, idref(&ea, x), idref(&ea, x))),
            ret(bin(&ea, BinaryOpKind::Add, idref(&ea, y), num(&ea, 1))),
        ]);
        let stmts = vec![
            fndef(compute, vec![(x, int_ty)], body),
            letb(r, calle(&ea, compute, vec![num(&ea, 4)])),
            show(&ea, show_s, idref(&ea, r)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "17");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_function_conditional_return_fallthrough() {
        // ## To sign (n): If n > 0: Return 1. Return 0.
        // Main: Show sign(5). Show sign(-3).   → 1, 0
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let sign = it.intern("sign");
        let n = it.intern("n");
        let p = it.intern("p");
        let q = it.intern("q");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let then_blk: &[Stmt] = sa.alloc_slice(vec![ret(num(&ea, 1))]);
        let body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::If { cond: bin(&ea, BinaryOpKind::Gt, idref(&ea, n), num(&ea, 0)), then_block: then_blk, else_block: None },
            ret(num(&ea, 0)),
        ]);
        let stmts = vec![
            fndef(sign, vec![(n, int_ty)], body),
            letb(p, calle(&ea, sign, vec![num(&ea, 5)])),
            show(&ea, show_s, idref(&ea, p)),
            letb(q, calle(&ea, sign, vec![num(&ea, -3)])),
            show(&ea, show_s, idref(&ea, q)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "1");
        assert_eq!(lines[1], "0");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    // ---- Collection builder helpers ----------------------------------------

    fn list_lit<'a>(ea: &'a Arena<Expr<'a>>, items: Vec<&'a Expr<'a>>) -> &'a Expr<'a> {
        ea.alloc(Expr::List(items))
    }
    fn new_coll<'a>(ea: &'a Arena<Expr<'a>>, type_name: Symbol) -> &'a Expr<'a> {
        ea.alloc(Expr::New { type_name, type_args: vec![], init_fields: vec![] })
    }
    fn length_of<'a>(ea: &'a Arena<Expr<'a>>, c: &'a Expr<'a>) -> &'a Expr<'a> {
        ea.alloc(Expr::Length { collection: c })
    }
    fn index_at<'a>(ea: &'a Arena<Expr<'a>>, c: &'a Expr<'a>, i: &'a Expr<'a>) -> &'a Expr<'a> {
        ea.alloc(Expr::Index { collection: c, index: i })
    }
    fn contains_e<'a>(ea: &'a Arena<Expr<'a>>, c: &'a Expr<'a>, v: &'a Expr<'a>) -> &'a Expr<'a> {
        ea.alloc(Expr::Contains { collection: c, value: v })
    }
    fn range_e<'a>(ea: &'a Arena<Expr<'a>>, s: &'a Expr<'a>, e: &'a Expr<'a>) -> &'a Expr<'a> {
        ea.alloc(Expr::Range { start: s, end: e })
    }
    fn push_to<'a>(value: &'a Expr<'a>, collection: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Push { value, collection }
    }

    #[test]
    fn vm_list_literal_and_length() {
        // Let xs be [10, 20, 30]. Show length of xs.   → 3
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let show_s = it.intern("show");
        let lst = list_lit(&ea, vec![num(&ea, 10), num(&ea, 20), num(&ea, 30)]);
        let stmts = vec![
            letb(xs, lst),
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "3");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_new_seq_push_length() {
        // Let xs be a new Seq of Int. Push 10 to xs. Push 20 to xs. Show length of xs.   → 2
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let seq = it.intern("Seq");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: new_coll(&ea, seq), mutable: true },
            push_to(num(&ea, 10), idref(&ea, xs)),
            push_to(num(&ea, 20), idref(&ea, xs)),
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "2");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_empty_seq_length_is_zero() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let seq = it.intern("Seq");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: new_coll(&ea, seq), mutable: true },
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "0");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_list_index_one_based() {
        // Let xs be [5, 6, 7]. Show xs at 2.   → 6
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let show_s = it.intern("show");
        let lst = list_lit(&ea, vec![num(&ea, 5), num(&ea, 6), num(&ea, 7)]);
        let stmts = vec![
            letb(xs, lst),
            show(&ea, show_s, index_at(&ea, idref(&ea, xs), num(&ea, 2))),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "6");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_range_to_list() {
        // Let xs be 1 to 5. Show length of xs. Show xs at 3.   → 5, 3
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let show_s = it.intern("show");
        let stmts = vec![
            letb(xs, range_e(&ea, num(&ea, 1), num(&ea, 5))),
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
            show(&ea, show_s, index_at(&ea, idref(&ea, xs), num(&ea, 3))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "5");
        assert_eq!(lines[1], "3");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_list_contains() {
        // Let xs be [1, 2, 3]. Show xs contains 2. Show xs contains 5.   → true, false
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let show_s = it.intern("show");
        let lst = list_lit(&ea, vec![num(&ea, 1), num(&ea, 2), num(&ea, 3)]);
        let stmts = vec![
            letb(xs, lst),
            show(&ea, show_s, contains_e(&ea, idref(&ea, xs), num(&ea, 2))),
            show(&ea, show_s, contains_e(&ea, idref(&ea, xs), num(&ea, 5))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "true");
        assert_eq!(lines[1], "false");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_push_then_index_sum_loop() {
        // Build [2,4,6] by pushing; sum via 1-based indexing in a While loop.   → 12
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let seq = it.intern("Seq");
        let total = it.intern("total");
        let i = it.intern("i");
        let show_s = it.intern("show");

        let cond = bin(&ea, BinaryOpKind::LtEq, idref(&ea, i), length_of(&ea, idref(&ea, xs)));
        let elem = index_at(&ea, idref(&ea, xs), idref(&ea, i));
        let body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::Set { target: total, value: bin(&ea, BinaryOpKind::Add, idref(&ea, total), elem) },
            Stmt::Set { target: i, value: bin(&ea, BinaryOpKind::Add, idref(&ea, i), num(&ea, 1)) },
        ]);

        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: new_coll(&ea, seq), mutable: true },
            push_to(num(&ea, 2), idref(&ea, xs)),
            push_to(num(&ea, 4), idref(&ea, xs)),
            push_to(num(&ea, 6), idref(&ea, xs)),
            Stmt::Let { var: total, ty: None, value: num(&ea, 0), mutable: true },
            Stmt::Let { var: i, ty: None, value: num(&ea, 1), mutable: true },
            Stmt::While { cond, body, decreasing: None },
            show(&ea, show_s, idref(&ea, total)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "12");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_index_out_of_bounds_errors_like_treewalk() {
        // Let xs be [1, 2]. Show xs at 5.   → both VM and tree-walker error.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let show_s = it.intern("show");
        let lst = list_lit(&ea, vec![num(&ea, 1), num(&ea, 2)]);
        let stmts = vec![
            letb(xs, lst),
            show(&ea, show_s, index_at(&ea, idref(&ea, xs), num(&ea, 5))),
        ];
        assert!(compile_and_run(&stmts, &it).is_err(), "VM should error on OOB index");
        assert!(run_treewalk(&stmts, &it).is_err(), "tree-walker should error on OOB index");
    }

    fn text<'a>(ea: &'a Arena<Expr<'a>>, sym: Symbol) -> &'a Expr<'a> {
        ea.alloc(Expr::Literal(Literal::Text(sym)))
    }

    #[test]
    fn vm_set_add_dedups() {
        // new Set; Add 1; Add 2; Add 1 (dup). Show length.   → 2
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let s = it.intern("s");
        let set_ty = it.intern("Set");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: s, ty: None, value: new_coll(&ea, set_ty), mutable: true },
            Stmt::Add { value: num(&ea, 1), collection: idref(&ea, s) },
            Stmt::Add { value: num(&ea, 2), collection: idref(&ea, s) },
            Stmt::Add { value: num(&ea, 1), collection: idref(&ea, s) },
            show(&ea, show_s, length_of(&ea, idref(&ea, s))),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "2");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_set_contains_and_remove() {
        // new Set; Add 1,2,3; Remove 2; Show length; Show s contains 2; Show s contains 3.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let s = it.intern("s");
        let set_ty = it.intern("Set");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: s, ty: None, value: new_coll(&ea, set_ty), mutable: true },
            Stmt::Add { value: num(&ea, 1), collection: idref(&ea, s) },
            Stmt::Add { value: num(&ea, 2), collection: idref(&ea, s) },
            Stmt::Add { value: num(&ea, 3), collection: idref(&ea, s) },
            Stmt::Remove { value: num(&ea, 2), collection: idref(&ea, s) },
            show(&ea, show_s, length_of(&ea, idref(&ea, s))),
            show(&ea, show_s, contains_e(&ea, idref(&ea, s), num(&ea, 2))),
            show(&ea, show_s, contains_e(&ea, idref(&ea, s), num(&ea, 3))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, vec!["2", "false", "true"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_map_set_get_length() {
        // new Map; Set item "a" of m to 10; Set item "b" of m to 20.
        // Show item "a" of m; Show length of m.   → 10, 2
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let m = it.intern("m");
        let map_ty = it.intern("Map");
        let key_a = it.intern("a");
        let key_b = it.intern("b");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: m, ty: None, value: new_coll(&ea, map_ty), mutable: true },
            Stmt::SetIndex { collection: idref(&ea, m), index: text(&ea, key_a), value: num(&ea, 10) },
            Stmt::SetIndex { collection: idref(&ea, m), index: text(&ea, key_b), value: num(&ea, 20) },
            show(&ea, show_s, index_at(&ea, idref(&ea, m), text(&ea, key_a))),
            show(&ea, show_s, length_of(&ea, idref(&ea, m))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "10");
        assert_eq!(lines[1], "2");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_map_overwrite_and_remove() {
        // new Map; Set "a"→1; Set "a"→9 (overwrite); Set "b"→2; Remove "a".
        // Show length; Show item "b" of m.   → 1, 2
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let m = it.intern("m");
        let map_ty = it.intern("Map");
        let key_a = it.intern("a");
        let key_b = it.intern("b");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: m, ty: None, value: new_coll(&ea, map_ty), mutable: true },
            Stmt::SetIndex { collection: idref(&ea, m), index: text(&ea, key_a), value: num(&ea, 1) },
            Stmt::SetIndex { collection: idref(&ea, m), index: text(&ea, key_a), value: num(&ea, 9) },
            Stmt::SetIndex { collection: idref(&ea, m), index: text(&ea, key_b), value: num(&ea, 2) },
            Stmt::Remove { value: text(&ea, key_a), collection: idref(&ea, m) },
            show(&ea, show_s, length_of(&ea, idref(&ea, m))),
            show(&ea, show_s, index_at(&ea, idref(&ea, m), text(&ea, key_b))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "1");
        assert_eq!(lines[1], "2");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_list_set_index_one_based() {
        // Let xs be [1,2,3]. Set item 2 of xs to 99. Show xs at 2.   → 99
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let show_s = it.intern("show");
        let lst = list_lit(&ea, vec![num(&ea, 1), num(&ea, 2), num(&ea, 3)]);
        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: lst, mutable: true },
            Stmt::SetIndex { collection: idref(&ea, xs), index: num(&ea, 2), value: num(&ea, 99) },
            show(&ea, show_s, index_at(&ea, idref(&ea, xs), num(&ea, 2))),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "99");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    // ---- Generative differential fuzzer (VM vs tree-walker) ------------------
    //
    // Generates random total (overflow-free) programs over Int variables with
    // arithmetic, comparisons, conditionals, and bounded loops, then asserts the
    // VM's behavior is identical to the tree-walker's. Any divergence is a real
    // translation bug. Deterministic (SplitMix64) so failures reproduce by seed.

    struct SplitMix64 {
        state: u64,
    }
    impl SplitMix64 {
        fn new(seed: u64) -> Self {
            SplitMix64 { state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15) }
        }
        fn next_u64(&mut self) -> u64 {
            self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }
    }

    fn gen_atom<'a>(ea: &'a Arena<Expr<'a>>, rng: &mut SplitMix64, vars: &[Symbol]) -> &'a Expr<'a> {
        if rng.below(2) == 0 {
            num(ea, rng.below(6) as i64)
        } else {
            idref(ea, vars[rng.below(vars.len() as u64) as usize])
        }
    }

    // depth-1 arithmetic over small atoms. +,- keep values tiny; * uses a small
    // multiplier and / % use a non-zero literal divisor — all provably total and
    // overflow-free for the bounded value range.
    fn gen_arith<'a>(ea: &'a Arena<Expr<'a>>, rng: &mut SplitMix64, vars: &[Symbol]) -> &'a Expr<'a> {
        match rng.below(8) {
            0 | 1 | 2 => {
                let l = gen_atom(ea, rng, vars);
                let r = gen_atom(ea, rng, vars);
                bin(ea, BinaryOpKind::Add, l, r)
            }
            3 | 4 => {
                let l = gen_atom(ea, rng, vars);
                let r = gen_atom(ea, rng, vars);
                bin(ea, BinaryOpKind::Subtract, l, r)
            }
            5 => {
                let l = gen_atom(ea, rng, vars);
                bin(ea, BinaryOpKind::Multiply, l, num(ea, rng.below(4) as i64))
            }
            6 => {
                let l = gen_atom(ea, rng, vars);
                bin(ea, BinaryOpKind::Divide, l, num(ea, 1 + rng.below(5) as i64))
            }
            _ => {
                let l = gen_atom(ea, rng, vars);
                bin(ea, BinaryOpKind::Modulo, l, num(ea, 1 + rng.below(5) as i64))
            }
        }
    }

    fn gen_cmp<'a>(ea: &'a Arena<Expr<'a>>, rng: &mut SplitMix64, vars: &[Symbol]) -> &'a Expr<'a> {
        let l = idref(ea, vars[rng.below(vars.len() as u64) as usize]);
        let r = num(ea, rng.below(6) as i64);
        let op = match rng.below(6) {
            0 => BinaryOpKind::Lt,
            1 => BinaryOpKind::Gt,
            2 => BinaryOpKind::LtEq,
            3 => BinaryOpKind::GtEq,
            4 => BinaryOpKind::Eq,
            _ => BinaryOpKind::NotEq,
        };
        bin(ea, op, l, r)
    }

    fn gen_program<'a>(
        seed: u64,
        ea: &'a Arena<Expr<'a>>,
        sa: &'a Arena<Stmt<'a>>,
        vars: &[Symbol],
        show_s: Symbol,
    ) -> Vec<Stmt<'a>> {
        let mut rng = SplitMix64::new(seed);
        let mut stmts: Vec<Stmt> = Vec::new();
        for &v in vars {
            stmts.push(Stmt::Let { var: v, ty: None, value: num(ea, rng.below(6) as i64), mutable: true });
        }
        let m = 4 + rng.below(8);
        for _ in 0..m {
            match rng.below(10) {
                0..=5 => {
                    let v = vars[rng.below(vars.len() as u64) as usize];
                    stmts.push(Stmt::Set { target: v, value: gen_arith(ea, &mut rng, vars) });
                }
                6..=7 => {
                    let cond = gen_cmp(ea, &mut rng, vars);
                    let v = vars[rng.below(vars.len() as u64) as usize];
                    let then_blk: &[Stmt] =
                        sa.alloc_slice(vec![Stmt::Set { target: v, value: gen_arith(ea, &mut rng, vars) }]);
                    if rng.below(2) == 0 {
                        let v2 = vars[rng.below(vars.len() as u64) as usize];
                        let else_blk: &[Stmt] =
                            sa.alloc_slice(vec![Stmt::Set { target: v2, value: gen_arith(ea, &mut rng, vars) }]);
                        stmts.push(Stmt::If { cond, then_block: then_blk, else_block: Some(else_blk) });
                    } else {
                        stmts.push(Stmt::If { cond, then_block: then_blk, else_block: None });
                    }
                }
                _ => {
                    // A terminating bounded loop: the loop variable is mutated ONLY
                    // by its +1 increment, so it strictly increases to the bound.
                    let li = rng.below(vars.len() as u64) as usize;
                    let loop_var = vars[li];
                    let other = vars[(li + 1 + rng.below(vars.len() as u64 - 1) as usize) % vars.len()];
                    let bound = 2 + rng.below(4);
                    let cond = bin(ea, BinaryOpKind::Lt, idref(ea, loop_var), num(ea, bound as i64));
                    let body: &[Stmt] = sa.alloc_slice(vec![
                        Stmt::Set { target: other, value: gen_arith(ea, &mut rng, vars) },
                        Stmt::Set {
                            target: loop_var,
                            value: bin(ea, BinaryOpKind::Add, idref(ea, loop_var), num(ea, 1)),
                        },
                    ]);
                    stmts.push(Stmt::While { cond, body, decreasing: None });
                }
            }
        }
        for &v in vars {
            stmts.push(show(ea, show_s, idref(ea, v)));
        }
        stmts
    }

    #[test]
    fn vm_differential_fuzz_300_programs() {
        for seed in 0..300u64 {
            let ea: Arena<Expr> = Arena::new();
            let sa: Arena<Stmt> = Arena::new();
            let mut it = Interner::new();
            let show_s = it.intern("show");
            let vars: Vec<Symbol> = (0..4u32).map(|k| it.intern(&format!("v{k}"))).collect();
            let stmts = gen_program(seed, &ea, &sa, &vars, show_s);

            let vm = compile_and_run(&stmts, &it);
            let tw = run_treewalk(&stmts, &it);
            match (vm, tw) {
                (Ok(a), Ok(b)) => assert_eq!(
                    normalize(&a),
                    normalize(&b),
                    "seed {} diverged:\nVM:\n{}\nTREE-WALKER:\n{}",
                    seed, a, b
                ),
                (Err(_), Err(_)) => {}
                (a, b) => panic!("seed {} one engine errored: vm={:?} tw={:?}", seed, a, b),
            }
        }
    }

    #[test]
    fn vm_differential_fuzz_functions() {
        for seed in 0..150u64 {
            let ea: Arena<Expr> = Arena::new();
            let sa: Arena<Stmt> = Arena::new();
            let ta: Arena<TypeExpr> = Arena::new();
            let mut it = Interner::new();
            let show_s = it.intern("show");
            let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
            let mut rng = SplitMix64::new(seed ^ 0xF00D);

            let f0 = it.intern("f0");
            let f1 = it.intern("f1");
            let pa = it.intern("pa");
            let pb = it.intern("pb");
            let params = [pa, pb];
            // Function bodies are arithmetic over the parameters (divisors are
            // non-zero literals, so no division-by-zero from arguments).
            let body0: &[Stmt] = sa.alloc_slice(vec![ret(gen_arith(&ea, &mut rng, &params))]);
            let body1: &[Stmt] = sa.alloc_slice(vec![ret(gen_arith(&ea, &mut rng, &params))]);
            let mut stmts = vec![
                fndef(f0, vec![(pa, int_ty), (pb, int_ty)], body0),
                fndef(f1, vec![(pa, int_ty), (pb, int_ty)], body1),
            ];
            let calls = 3 + rng.below(4);
            for _ in 0..calls {
                let f = if rng.below(2) == 0 { f0 } else { f1 };
                let a = num(&ea, rng.below(6) as i64);
                let b = num(&ea, 1 + rng.below(5) as i64);
                stmts.push(show(&ea, show_s, calle(&ea, f, vec![a, b])));
            }

            let vm = compile_and_run(&stmts, &it);
            let tw = run_treewalk(&stmts, &it);
            match (vm, tw) {
                (Ok(a), Ok(b)) => assert_eq!(normalize(&a), normalize(&b), "fn seed {} diverged", seed),
                (Err(_), Err(_)) => {}
                (a, b) => panic!("fn seed {} one engine errored: vm={:?} tw={:?}", seed, a, b),
            }
        }
    }

    fn gen_float_lit<'a>(ea: &'a Arena<Expr<'a>>, rng: &mut SplitMix64) -> &'a Expr<'a> {
        ea.alloc(Expr::Literal(Literal::Float(rng.below(10) as f64 * 0.5)))
    }
    fn gen_float_atom<'a>(ea: &'a Arena<Expr<'a>>, rng: &mut SplitMix64, vars: &[Symbol]) -> &'a Expr<'a> {
        if rng.below(2) == 0 {
            gen_float_lit(ea, rng)
        } else {
            idref(ea, vars[rng.below(vars.len() as u64) as usize])
        }
    }
    // Float arithmetic uses only +,-,* (no division → no float div-by-zero),
    // which keeps every program total; any NaN/Inf displays identically in both.
    fn gen_float_arith<'a>(ea: &'a Arena<Expr<'a>>, rng: &mut SplitMix64, vars: &[Symbol]) -> &'a Expr<'a> {
        let l = gen_float_atom(ea, rng, vars);
        let r = gen_float_atom(ea, rng, vars);
        let op = match rng.below(3) {
            0 => BinaryOpKind::Add,
            1 => BinaryOpKind::Subtract,
            _ => BinaryOpKind::Multiply,
        };
        bin(ea, op, l, r)
    }

    #[test]
    fn vm_differential_fuzz_floats() {
        for seed in 0..200u64 {
            let ea: Arena<Expr> = Arena::new();
            let sa: Arena<Stmt> = Arena::new();
            let mut it = Interner::new();
            let show_s = it.intern("show");
            let vars: Vec<Symbol> = (0..3u32).map(|k| it.intern(&format!("f{k}"))).collect();
            let mut rng = SplitMix64::new(seed ^ 0xBEEF);

            let mut stmts: Vec<Stmt> = Vec::new();
            for &v in &vars {
                stmts.push(Stmt::Let { var: v, ty: None, value: gen_float_lit(&ea, &mut rng), mutable: true });
            }
            let m = 4 + rng.below(6);
            for _ in 0..m {
                if rng.below(8) < 5 {
                    let v = vars[rng.below(vars.len() as u64) as usize];
                    stmts.push(Stmt::Set { target: v, value: gen_float_arith(&ea, &mut rng, &vars) });
                } else {
                    let l = idref(&ea, vars[rng.below(vars.len() as u64) as usize]);
                    let r = idref(&ea, vars[rng.below(vars.len() as u64) as usize]);
                    let op = match rng.below(4) {
                        0 => BinaryOpKind::Lt,
                        1 => BinaryOpKind::Gt,
                        2 => BinaryOpKind::LtEq,
                        _ => BinaryOpKind::GtEq,
                    };
                    let cond = bin(&ea, op, l, r);
                    let v = vars[rng.below(vars.len() as u64) as usize];
                    let then_blk: &[Stmt] =
                        sa.alloc_slice(vec![Stmt::Set { target: v, value: gen_float_arith(&ea, &mut rng, &vars) }]);
                    stmts.push(Stmt::If { cond, then_block: then_blk, else_block: None });
                }
            }
            for &v in &vars {
                stmts.push(show(&ea, show_s, idref(&ea, v)));
            }

            let vm = compile_and_run(&stmts, &it);
            let tw = run_treewalk(&stmts, &it);
            match (vm, tw) {
                (Ok(a), Ok(b)) => assert_eq!(normalize(&a), normalize(&b), "float seed {} diverged", seed),
                (Err(_), Err(_)) => {}
                (a, b) => panic!("float seed {} one engine errored: vm={:?} tw={:?}", seed, a, b),
            }
        }
    }

    #[test]
    fn vm_function_mutual_recursion() {
        // isEven(n): If n == 0: Return 1. Return isOdd(n - 1).
        // isOdd(n):  If n == 0: Return 0. Return isEven(n - 1).
        // Main: Show isEven(4).   → 1   (forward reference to isOdd)
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let is_even = it.intern("isEven");
        let is_odd = it.intern("isOdd");
        let n = it.intern("n");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let even_then: &[Stmt] = sa.alloc_slice(vec![ret(num(&ea, 1))]);
        let even_body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::If { cond: bin(&ea, BinaryOpKind::Eq, idref(&ea, n), num(&ea, 0)), then_block: even_then, else_block: None },
            ret(calle(&ea, is_odd, vec![bin(&ea, BinaryOpKind::Subtract, idref(&ea, n), num(&ea, 1))])),
        ]);
        let odd_then: &[Stmt] = sa.alloc_slice(vec![ret(num(&ea, 0))]);
        let odd_body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::If { cond: bin(&ea, BinaryOpKind::Eq, idref(&ea, n), num(&ea, 0)), then_block: odd_then, else_block: None },
            ret(calle(&ea, is_even, vec![bin(&ea, BinaryOpKind::Subtract, idref(&ea, n), num(&ea, 1))])),
        ]);
        let stmts = vec![
            fndef(is_even, vec![(n, int_ty)], even_body),
            fndef(is_odd, vec![(n, int_ty)], odd_body),
            letb(r, calle(&ea, is_even, vec![num(&ea, 4)])),
            show(&ea, show_s, idref(&ea, r)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "1");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_runs_arithmetic_let_show() {
        // Let x be 5. Let y be x + 7. Show y.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let y = it.intern("y");
        let console = it.intern("show");

        let five = ea.alloc(Expr::Literal(Literal::Number(5)));
        let xref = ea.alloc(Expr::Identifier(x));
        let seven = ea.alloc(Expr::Literal(Literal::Number(7)));
        let sum = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: xref, right: seven });
        let yref = ea.alloc(Expr::Identifier(y));
        let console_ref = ea.alloc(Expr::Identifier(console));

        let stmts = vec![
            Stmt::Let { var: x, ty: None, value: five, mutable: false },
            Stmt::Let { var: y, ty: None, value: sum, mutable: false },
            Stmt::Show { object: yref, recipient: console_ref },
        ];

        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.trim(), "12");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_arithmetic_and_comparison_chain() {
        // Let a be 6. Let b be a * 4. Let c be b - 2. Show c.   → 22
        // Let d be c > 20. Show d.                              → true
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let a = it.intern("a");
        let b = it.intern("b");
        let c = it.intern("c");
        let d = it.intern("d");
        let console = it.intern("show");

        let six = ea.alloc(Expr::Literal(Literal::Number(6)));
        let aref = ea.alloc(Expr::Identifier(a));
        let four = ea.alloc(Expr::Literal(Literal::Number(4)));
        let mul = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::Multiply, left: aref, right: four });
        let bref = ea.alloc(Expr::Identifier(b));
        let two = ea.alloc(Expr::Literal(Literal::Number(2)));
        let subx = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::Subtract, left: bref, right: two });
        let cref1 = ea.alloc(Expr::Identifier(c));
        let cref2 = ea.alloc(Expr::Identifier(c));
        let twenty = ea.alloc(Expr::Literal(Literal::Number(20)));
        let gt = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::Gt, left: cref2, right: twenty });
        let dref = ea.alloc(Expr::Identifier(d));
        let console_ref1 = ea.alloc(Expr::Identifier(console));
        let console_ref2 = ea.alloc(Expr::Identifier(console));

        let stmts = vec![
            Stmt::Let { var: a, ty: None, value: six, mutable: false },
            Stmt::Let { var: b, ty: None, value: mul, mutable: false },
            Stmt::Let { var: c, ty: None, value: subx, mutable: false },
            Stmt::Show { object: cref1, recipient: console_ref1 },
            Stmt::Let { var: d, ty: None, value: gt, mutable: false },
            Stmt::Show { object: dref, recipient: console_ref2 },
        ];

        let out = compile_and_run(&stmts, &it).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "22");
        assert_eq!(lines[1], "true");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_while_loop_sums_to_15() {
        // Let total be 0. Let i be 1.
        // While i <= 5: Set total to total + i. Set i to i + 1.
        // Show total.   → 15
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let total = it.intern("total");
        let i = it.intern("i");
        let console = it.intern("show");

        let zero = ea.alloc(Expr::Literal(Literal::Number(0)));
        let one_init = ea.alloc(Expr::Literal(Literal::Number(1)));

        // cond: i <= 5
        let i_c = ea.alloc(Expr::Identifier(i));
        let five = ea.alloc(Expr::Literal(Literal::Number(5)));
        let cond = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::LtEq, left: i_c, right: five });

        // body: Set total to total + i.
        let total_l = ea.alloc(Expr::Identifier(total));
        let i_r = ea.alloc(Expr::Identifier(i));
        let tplus = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: total_l, right: i_r });
        // body: Set i to i + 1.
        let i_l = ea.alloc(Expr::Identifier(i));
        let one = ea.alloc(Expr::Literal(Literal::Number(1)));
        let iplus = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::Add, left: i_l, right: one });
        let body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::Set { target: total, value: tplus },
            Stmt::Set { target: i, value: iplus },
        ]);

        let total_show = ea.alloc(Expr::Identifier(total));
        let console_ref = ea.alloc(Expr::Identifier(console));

        let stmts = vec![
            Stmt::Let { var: total, ty: None, value: zero, mutable: true },
            Stmt::Let { var: i, ty: None, value: one_init, mutable: true },
            Stmt::While { cond, body, decreasing: None },
            Stmt::Show { object: total_show, recipient: console_ref },
        ];

        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.trim(), "15");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    // ---- Sprint 1: error-string parity with the tree-walker -------------------
    //
    // The Err string IS part of the spec. Each test runs the same program
    // through both engines and asserts the error text is identical.

    fn assert_err_parity(stmts: &[Stmt], interner: &Interner) {
        let vm_err = compile_and_run(stmts, interner)
            .expect_err("VM should error");
        let tw_err = run_treewalk(stmts, interner)
            .expect_err("tree-walker should error");
        assert_eq!(vm_err, tw_err, "error strings diverged");
    }

    fn boolean<'a>(ea: &'a Arena<Expr<'a>>, b: bool) -> &'a Expr<'a> {
        ea.alloc(Expr::Literal(Literal::Boolean(b)))
    }
    fn nothing_lit<'a>(ea: &'a Arena<Expr<'a>>) -> &'a Expr<'a> {
        ea.alloc(Expr::Literal(Literal::Nothing))
    }

    #[test]
    fn vm_add_type_error_matches_treewalk() {
        // true + nothing — "Cannot add Bool and Nothing" (capital C).
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let stmts = vec![letb(x, bin(&ea, BinaryOpKind::Add, boolean(&ea, true), nothing_lit(&ea)))];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_subtract_type_error_matches_treewalk() {
        // true - nothing — TW phrases this "Cannot subtract Nothing from Bool".
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let stmts = vec![letb(x, bin(&ea, BinaryOpKind::Subtract, boolean(&ea, true), nothing_lit(&ea)))];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_multiply_type_error_matches_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let stmts = vec![letb(x, bin(&ea, BinaryOpKind::Multiply, boolean(&ea, true), nothing_lit(&ea)))];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_divide_and_modulo_zero_messages_match_treewalk() {
        // Division by zero / Modulo by zero — exact text both engines.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let stmts = vec![letb(x, bin(&ea, BinaryOpKind::Divide, num(&ea, 1), num(&ea, 0)))];
        assert_err_parity(&stmts, &it);

        let y = it.intern("y");
        let stmts = vec![letb(y, bin(&ea, BinaryOpKind::Modulo, num(&ea, 1), num(&ea, 0)))];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_comparison_type_error_matches_treewalk() {
        // true < 1 — TW says "Cannot compare Bool and Int".
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let stmts = vec![letb(x, bin(&ea, BinaryOpKind::Lt, boolean(&ea, true), num(&ea, 1)))];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_index_and_length_type_errors_match_treewalk() {
        // Indexing an Int / length of a Bool — kernel message parity.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let stmts = vec![letb(x, index_at(&ea, num(&ea, 1), num(&ea, 1)))];
        assert_err_parity(&stmts, &it);

        let y = it.intern("y");
        let stmts = vec![letb(y, length_of(&ea, boolean(&ea, true)))];
        assert_err_parity(&stmts, &it);
    }

    // ---- Sprint 11: the generative differential fuzzer v2 ----------------------

    /// Run one generated program through both engines, asserting outcome
    /// equality (output AND error). Returns the divergence message if any.
    fn fuzz_one(seed: u64, features: super::fuzz::FeatureSet) -> Option<String> {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let generated = super::fuzz::generate(seed, features, &ea, &sa, &ta, &mut it);

        let (vm_out, vm_err) = run_to_outcome(&generated.stmts, &it, None, None);
        let (tw_out, tw_err) = run_treewalk_outcome(&generated.stmts, &it);
        if normalize(&vm_out) != normalize(&tw_out) || vm_err != tw_err {
            return Some(format!(
                "SEED={} FEATURES={:#x}\nvm out:\n{}\nvm err: {:?}\ntw out:\n{}\ntw err: {:?}",
                seed, features.0, vm_out, vm_err, tw_out, tw_err
            ));
        }
        None
    }

    #[test]
    fn fuzz_generator_is_deterministic() {
        // Same seed → identical program → identical outcome both times.
        for seed in [0u64, 42, 1234] {
            let f = super::fuzz::FeatureSet::all_supported();
            let run = |seed| {
                let ea: Arena<Expr> = Arena::new();
                let sa: Arena<Stmt> = Arena::new();
                let ta: Arena<TypeExpr> = Arena::new();
                let mut it = Interner::new();
                let g = super::fuzz::generate(seed, f, &ea, &sa, &ta, &mut it);
                run_to_outcome(&g.stmts, &it, None, None)
            };
            assert_eq!(run(seed), run(seed), "seed {seed} not deterministic");
        }
    }

    #[test]
    fn vm_fuzz_full_feature_differential() {
        // The standing CI gate: 1500 seeds over the full supported surface.
        let features = super::fuzz::FeatureSet::all_supported();
        for seed in 0..1500u64 {
            if let Some(divergence) = fuzz_one(seed, features) {
                panic!("fuzz divergence:\n{divergence}");
            }
        }
    }

    #[test]
    fn vm_fuzz_error_injection_differential() {
        // 500 seeds with one trap planted per program: the engines must agree
        // on partial output AND the exact error string.
        let features = super::fuzz::FeatureSet(
            super::fuzz::FeatureSet::all_supported().0 | super::fuzz::FeatureSet::ERROR_INJECTION,
        );
        for seed in 0..500u64 {
            if let Some(divergence) = fuzz_one(seed, features) {
                panic!("fuzz (error-injection) divergence:\n{divergence}");
            }
        }
    }

    /// Overnight soak: `cargo test vm_fuzz_overnight -- --ignored`.
    #[test]
    #[ignore]
    fn vm_fuzz_overnight() {
        let features = super::fuzz::FeatureSet::all_supported();
        let with_errors =
            super::fuzz::FeatureSet(features.0 | super::fuzz::FeatureSet::ERROR_INJECTION);
        for seed in 0..500_000u64 {
            if let Some(d) = fuzz_one(seed, features) {
                panic!("fuzz divergence:\n{d}");
            }
            if let Some(d) = fuzz_one(seed, with_errors) {
                panic!("fuzz divergence:\n{d}");
            }
        }
    }

    // ---- Sprint 9: strings, slices, options, temporal --------------------------

    #[test]
    fn vm_interpolated_string_spec_matrix() {
        use crate::ast::stmt::StringPart;
        // "{n} {f$} {f.1} {n>5} {n<5} {n^5} {n=}" over n=42, f=2.5.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let n = it.intern("n");
        let fv = it.intern("fv");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let sep = it.intern(" | ");
        let spec_cur = it.intern("$");
        let spec_p1 = it.intern(".1");
        let spec_r5 = it.intern(">5");
        let spec_l5 = it.intern("<5");
        let spec_c5 = it.intern("^5");

        let parts = vec![
            StringPart::Expr { value: idref(&ea, n), format_spec: None, debug: false },
            StringPart::Literal(sep),
            StringPart::Expr { value: idref(&ea, fv), format_spec: Some(spec_cur), debug: false },
            StringPart::Literal(sep),
            StringPart::Expr { value: idref(&ea, fv), format_spec: Some(spec_p1), debug: false },
            StringPart::Literal(sep),
            StringPart::Expr { value: idref(&ea, n), format_spec: Some(spec_r5), debug: false },
            StringPart::Literal(sep),
            StringPart::Expr { value: idref(&ea, n), format_spec: Some(spec_l5), debug: false },
            StringPart::Literal(sep),
            StringPart::Expr { value: idref(&ea, n), format_spec: Some(spec_c5), debug: false },
            StringPart::Literal(sep),
            StringPart::Expr { value: idref(&ea, n), format_spec: None, debug: true },
        ];
        let interp = ea.alloc(Expr::InterpolatedString(parts));
        let f25 = ea.alloc(Expr::Literal(Literal::Float(2.5)));
        let stmts = vec![
            letb(n, num(&ea, 42)),
            letb(fv, f25),
            letb(r, interp),
            show(&ea, show_s, idref(&ea, r)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.trim_end(), "42 | $2.50 | 2.5 |    42 | 42    |  42   | n=42");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_slice_copy_tuple_union_intersection() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let part = it.intern("part");
        let cp = it.intern("cp");
        let tup = it.intern("tup");
        let s1 = it.intern("s1");
        let s2 = it.intern("s2");
        let u = it.intern("u");
        let i_ = it.intern("i_");
        let set_ty = it.intern("Set");
        let show_s = it.intern("show");

        let slice_e = ea.alloc(Expr::Slice {
            collection: idref(&ea, xs),
            start: num(&ea, 2),
            end: num(&ea, 4),
        });
        let copy_e = ea.alloc(Expr::Copy { expr: idref(&ea, xs) });
        let tuple_e = ea.alloc(Expr::Tuple(vec![num(&ea, 7), num(&ea, 8)]));
        let union_e = ea.alloc(Expr::Union { left: idref(&ea, s1), right: idref(&ea, s2) });
        let inter_e = ea.alloc(Expr::Intersection { left: idref(&ea, s1), right: idref(&ea, s2) });

        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: list_lit(&ea, (1..=5).map(|k| num(&ea, k)).collect()), mutable: true },
            letb(part, slice_e),
            show(&ea, show_s, length_of(&ea, idref(&ea, part))),
            show(&ea, show_s, index_at(&ea, idref(&ea, part), num(&ea, 1))),
            letb(cp, copy_e),
            push_to(num(&ea, 9), idref(&ea, xs)),
            show(&ea, show_s, length_of(&ea, idref(&ea, cp))),
            letb(tup, tuple_e),
            show(&ea, show_s, index_at(&ea, idref(&ea, tup), num(&ea, 2))),
            show(&ea, show_s, length_of(&ea, idref(&ea, tup))),
            Stmt::Let { var: s1, ty: None, value: new_coll(&ea, set_ty), mutable: true },
            Stmt::Let { var: s2, ty: None, value: new_coll(&ea, set_ty), mutable: true },
            Stmt::Add { value: num(&ea, 1), collection: idref(&ea, s1) },
            Stmt::Add { value: num(&ea, 2), collection: idref(&ea, s1) },
            Stmt::Add { value: num(&ea, 2), collection: idref(&ea, s2) },
            Stmt::Add { value: num(&ea, 3), collection: idref(&ea, s2) },
            letb(u, union_e),
            show(&ea, show_s, length_of(&ea, idref(&ea, u))),
            letb(i_, inter_e),
            show(&ea, show_s, length_of(&ea, idref(&ea, i_))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["3", "2", "5", "8", "2", "3", "1"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_option_some_none_match_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let a = it.intern("a");
        let b = it.intern("b");
        let show_s = it.intern("show");
        let some_e = ea.alloc(Expr::OptionSome { value: num(&ea, 5) });
        let none_e = ea.alloc(Expr::OptionNone);
        let stmts = vec![
            letb(a, some_e),
            show(&ea, show_s, idref(&ea, a)),
            letb(b, none_e),
            show(&ea, show_s, idref(&ea, b)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["5", "nothing"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_today_now_with_fixed_clock_match_treewalk() {
        // Pin the clock so both engines see the same instant; also pin the
        // shadowing quirk — the NAME `today` resolves to the builtin even
        // after `Let today be 5`.
        crate::semantics::temporal::set_fixed_clock(19753, 1_700_000_000_000_000_000);
        let result = std::panic::catch_unwind(|| {
            let ea: Arena<Expr> = Arena::new();
            let mut it = Interner::new();
            let today_sym = it.intern("today");
            let a = it.intern("a");
            let b = it.intern("b");
            let show_s = it.intern("show");
            let stmts = vec![
                letb(a, idref(&ea, today_sym)),
                show(&ea, show_s, idref(&ea, a)),
                letb(b, idref(&ea, it.intern("now"))),
                show(&ea, show_s, idref(&ea, b)),
                letb(today_sym, num(&ea, 5)),
                show(&ea, show_s, idref(&ea, today_sym)),
            ];
            assert_vm_eq_treewalk(&stmts, &it);
        });
        crate::semantics::temporal::clear_fixed_clock();
        result.unwrap();
    }

    #[test]
    fn vm_escape_expr_errors_like_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let lang = it.intern("Rust");
        let code = it.intern("raw");
        let esc = ea.alloc(Expr::Escape { language: lang, code });
        let stmts = vec![letb(x, esc)];
        assert_err_parity(&stmts, &it);
    }

    // ---- Sprint 8: closures, first-class calls, globals ------------------------

    use crate::ast::stmt::ClosureBody;

    fn closure_expr<'a>(
        ea: &'a Arena<Expr<'a>>,
        params: Vec<(Symbol, &'a TypeExpr<'a>)>,
        body: ClosureBody<'a>,
    ) -> &'a Expr<'a> {
        ea.alloc(Expr::Closure { params, body, return_type: None })
    }
    fn call_expr<'a>(
        ea: &'a Arena<Expr<'a>>,
        callee: &'a Expr<'a>,
        args: Vec<&'a Expr<'a>>,
    ) -> &'a Expr<'a> {
        ea.alloc(Expr::CallExpr { callee, args })
    }

    #[test]
    fn vm_closure_capture_is_snapshot() {
        // Mirrors the pinned e2e test: `Let x be 10. Let getX be () -> x.
        // Set x to 999. Show getX().` → 10.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let get_x = it.intern("getX");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: x, ty: None, value: num(&ea, 10), mutable: true },
            letb(get_x, closure_expr(&ea, vec![], ClosureBody::Expression(idref(&ea, x)))),
            Stmt::Set { target: x, value: num(&ea, 999) },
            show(&ea, show_s, call_expr(&ea, idref(&ea, get_x), vec![])),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "10");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_closure_capture_recloned_each_call() {
        // The closure body mutates its captured list — each call gets a fresh
        // deep clone (the tree-walker re-clones per invocation).
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let f = it.intern("f");
        let show_s = it.intern("show");
        let body: &[Stmt] = sa.alloc_slice(vec![
            push_to(num(&ea, 9), idref(&ea, xs)),
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
            ret(num(&ea, 0)),
        ]);
        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: list_lit(&ea, vec![num(&ea, 1)]), mutable: true },
            letb(f, closure_expr(&ea, vec![], ClosureBody::Block(body))),
            Stmt::Call { function: f, args: vec![] },
            Stmt::Call { function: f, args: vec![] },
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        // Each call: clone has 1 element, push → 2. The original stays at 1.
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["2", "2", "1"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_closure_sees_live_global_when_created_before_definition() {
        // The closure references `g` before `Let g` runs: nothing to capture,
        // so the body falls through to the LIVE global.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let g = it.intern("g");
        let f = it.intern("f");
        let show_s = it.intern("show");
        let stmts = vec![
            letb(f, closure_expr(&ea, vec![], ClosureBody::Expression(idref(&ea, g)))),
            Stmt::Let { var: g, ty: None, value: num(&ea, 5), mutable: true },
            show(&ea, show_s, call_expr(&ea, idref(&ea, f), vec![])),
            Stmt::Set { target: g, value: num(&ea, 7) },
            show(&ea, show_s, call_expr(&ea, idref(&ea, f), vec![])),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["5", "7"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_function_reads_and_writes_global_live() {
        // Functions see Main top-level bindings LIVE (lexical globals).
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let g = it.intern("g");
        let reader = it.intern("reader");
        let writer = it.intern("writer");
        let show_s = it.intern("show");
        let reader_body: &[Stmt] = sa.alloc_slice(vec![ret(idref(&ea, g))]);
        let writer_body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::Set { target: g, value: num(&ea, 42) },
            ret(num(&ea, 0)),
        ]);
        let stmts = vec![
            fndef(reader, vec![], reader_body),
            fndef(writer, vec![], writer_body),
            Stmt::Let { var: g, ty: None, value: num(&ea, 1), mutable: true },
            show(&ea, show_s, calle(&ea, reader, vec![])),
            Stmt::Set { target: g, value: num(&ea, 2) },
            show(&ea, show_s, calle(&ea, reader, vec![])),
            Stmt::Call { function: writer, args: vec![] },
            show(&ea, show_s, idref(&ea, g)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["1", "2", "42"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_main_block_var_invisible_to_function() {
        // Lexical scoping: a Let inside a Main If-block is NOT a global; the
        // function fails with "Undefined variable" — in both engines, with the
        // partial output intact.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let t = it.intern("t");
        let f = it.intern("f");
        let show_s = it.intern("show");
        let f_body: &[Stmt] = sa.alloc_slice(vec![ret(idref(&ea, t))]);
        let then_blk: &[Stmt] = sa.alloc_slice(vec![
            letb(t, num(&ea, 5)),
            show(&ea, show_s, calle(&ea, f, vec![])),
        ]);
        let stmts = vec![
            fndef(f, vec![], f_body),
            show(&ea, show_s, num(&ea, 1)),
            Stmt::If { cond: boolean(&ea, true), then_block: then_blk, else_block: None },
        ];
        assert_outcome_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_closure_param_shadows_capture() {
        let ea: Arena<Expr> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let f = it.intern("f");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
        let stmts = vec![
            Stmt::Let { var: x, ty: None, value: num(&ea, 10), mutable: true },
            letb(
                f,
                closure_expr(&ea, vec![(x, int_ty)], ClosureBody::Expression(idref(&ea, x))),
            ),
            show(&ea, show_s, call_expr(&ea, idref(&ea, f), vec![num(&ea, 77)])),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "77");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_callexpr_on_non_function_and_arity_errors_match() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let f = it.intern("f");

        // Cannot call value of type Int.
        let stmts = vec![letb(x, call_expr(&ea, num(&ea, 5), vec![]))];
        assert_err_parity(&stmts, &it);

        // Closure arity mismatch.
        let stmts = vec![
            letb(f, closure_expr(&ea, vec![], ClosureBody::Expression(num(&ea, 1)))),
            letb(x, call_expr(&ea, idref(&ea, f), vec![num(&ea, 9)])),
        ];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_closure_passed_to_function_and_called_via_param() {
        // Higher-order: apply(f, x) = f(x).
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let apply = it.intern("apply");
        let fp = it.intern("fp");
        let xp = it.intern("xp");
        let n = it.intern("n");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let apply_body: &[Stmt] =
            sa.alloc_slice(vec![ret(call_expr(&ea, idref(&ea, fp), vec![idref(&ea, xp)]))]);
        let doubler = closure_expr(
            &ea,
            vec![(n, int_ty)],
            ClosureBody::Expression(bin(&ea, BinaryOpKind::Multiply, idref(&ea, n), num(&ea, 2))),
        );
        let stmts = vec![
            fndef(apply, vec![(fp, int_ty), (xp, int_ty)], apply_body),
            show(&ea, show_s, calle(&ea, apply, vec![doubler, num(&ea, 21)])),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "42");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    // ---- Sprint 7: structs, enums, Inspect, CRDT -------------------------------

    fn struct_def<'a>(name: Symbol, fields: Vec<(Symbol, Symbol, bool)>) -> Stmt<'a> {
        Stmt::StructDef { name, fields, is_portable: false }
    }
    fn new_struct<'a>(
        ea: &'a Arena<Expr<'a>>,
        type_name: Symbol,
        init_fields: Vec<(Symbol, &'a Expr<'a>)>,
    ) -> &'a Expr<'a> {
        ea.alloc(Expr::New { type_name, type_args: vec![], init_fields })
    }
    fn field_access<'a>(ea: &'a Arena<Expr<'a>>, object: &'a Expr<'a>, field: Symbol) -> &'a Expr<'a> {
        ea.alloc(Expr::FieldAccess { object, field })
    }

    #[test]
    fn vm_struct_new_with_defaults_and_field_access() {
        // Point { x: Int, y: Int }; new Point with x 3 — y defaults to 0.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let point = it.intern("Point");
        let x = it.intern("x");
        let y = it.intern("y");
        let int_s = it.intern("Int");
        let p = it.intern("p");
        let show_s = it.intern("show");
        let stmts = vec![
            struct_def(point, vec![(x, int_s, true), (y, int_s, true)]),
            letb(p, new_struct(&ea, point, vec![(x, num(&ea, 3))])),
            show(&ea, show_s, field_access(&ea, idref(&ea, p), x)),
            show(&ea, show_s, field_access(&ea, idref(&ea, p), y)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["3", "0"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_struct_value_semantics_no_aliasing() {
        // `Let b be a` copies the struct: SetField through b is invisible via a.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let point = it.intern("Point");
        let x = it.intern("x");
        let int_s = it.intern("Int");
        let a = it.intern("a");
        let b = it.intern("b");
        let show_s = it.intern("show");
        let stmts = vec![
            struct_def(point, vec![(x, int_s, true)]),
            Stmt::Let { var: a, ty: None, value: new_struct(&ea, point, vec![(x, num(&ea, 1))]), mutable: true },
            Stmt::Let { var: b, ty: None, value: idref(&ea, a), mutable: true },
            Stmt::SetField { object: idref(&ea, b), field: x, value: num(&ea, 99) },
            show(&ea, show_s, field_access(&ea, idref(&ea, a), x)),
            show(&ea, show_s, field_access(&ea, idref(&ea, b), x)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["1", "99"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_field_errors_match_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let point = it.intern("Point");
        let x = it.intern("x");
        let ghost = it.intern("ghost");
        let int_s = it.intern("Int");
        let p = it.intern("p");
        let v = it.intern("v");

        // Missing field read.
        let stmts = vec![
            struct_def(point, vec![(x, int_s, true)]),
            letb(p, new_struct(&ea, point, vec![])),
            letb(v, field_access(&ea, idref(&ea, p), ghost)),
        ];
        assert_err_parity(&stmts, &it);

        // Field access on a non-struct.
        let stmts = vec![letb(v, field_access(&ea, num(&ea, 5), x))];
        assert_err_parity(&stmts, &it);

        // SetField on a non-struct.
        let stmts = vec![
            letb(p, num(&ea, 5)),
            Stmt::SetField { object: idref(&ea, p), field: x, value: num(&ea, 1) },
        ];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_inspect_struct_arms_and_otherwise() {
        use crate::ast::stmt::MatchArm;
        // Inspect p (a Point): the Point arm binds x; a Circle arm is skipped;
        // then inspect an Int hits Otherwise.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let point = it.intern("Point");
        let circle = it.intern("Circle");
        let x = it.intern("x");
        let bx = it.intern("bx");
        let int_s = it.intern("Int");
        let p = it.intern("p");
        let show_s = it.intern("show");

        let point_body: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, idref(&ea, bx))]);
        let circle_body: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, num(&ea, 111))]);
        let otherwise_body: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, num(&ea, 222))]);

        let stmts = vec![
            struct_def(point, vec![(x, int_s, true)]),
            letb(p, new_struct(&ea, point, vec![(x, num(&ea, 7))])),
            Stmt::Inspect {
                target: idref(&ea, p),
                arms: vec![
                    MatchArm { enum_name: None, variant: Some(circle), bindings: vec![], body: circle_body },
                    MatchArm { enum_name: None, variant: Some(point), bindings: vec![(x, bx)], body: point_body },
                    MatchArm { enum_name: None, variant: None, bindings: vec![], body: otherwise_body },
                ],
                has_otherwise: true,
            },
            Stmt::Inspect {
                target: num(&ea, 5),
                arms: vec![
                    MatchArm { enum_name: None, variant: Some(point), bindings: vec![], body: circle_body },
                    MatchArm { enum_name: None, variant: None, bindings: vec![], body: otherwise_body },
                ],
                has_otherwise: true,
            },
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["7", "222"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_inspect_inductive_positional_bindings_and_no_match_falls_through() {
        use crate::ast::stmt::MatchArm;
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let shape = it.intern("Shape");
        let circle = it.intern("Circle");
        let square = it.intern("Square");
        let r_field = it.intern("radius");
        let r_bind = it.intern("r");
        let c = it.intern("c");
        let show_s = it.intern("show");

        let circle_expr = ea.alloc(Expr::NewVariant {
            enum_name: shape,
            variant: circle,
            fields: vec![(r_field, num(&ea, 10))],
        });
        let circle_body: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, idref(&ea, r_bind))]);
        let square_body: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, num(&ea, 333))]);

        let stmts = vec![
            letb(c, circle_expr),
            // Matching arm binds positionally.
            Stmt::Inspect {
                target: idref(&ea, c),
                arms: vec![MatchArm {
                    enum_name: None,
                    variant: Some(circle),
                    bindings: vec![(r_field, r_bind)],
                    body: circle_body,
                }],
                has_otherwise: false,
            },
            // No arm matches and no Otherwise: execution continues silently.
            Stmt::Inspect {
                target: idref(&ea, c),
                arms: vec![MatchArm { enum_name: None, variant: Some(square), bindings: vec![], body: square_body }],
                has_otherwise: false,
            },
            show(&ea, show_s, num(&ea, 1)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["10", "1"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_inductive_equality_is_structural() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let shape = it.intern("Shape");
        let circle = it.intern("Circle");
        let r_field = it.intern("radius");
        let a = it.intern("a");
        let b = it.intern("b");
        let show_s = it.intern("show");
        let c1 = ea.alloc(Expr::NewVariant {
            enum_name: shape,
            variant: circle,
            fields: vec![(r_field, num(&ea, 5))],
        });
        let c2 = ea.alloc(Expr::NewVariant {
            enum_name: shape,
            variant: circle,
            fields: vec![(r_field, num(&ea, 5))],
        });
        let stmts = vec![
            letb(a, c1),
            letb(b, c2),
            show(&ea, show_s, bin(&ea, BinaryOpKind::Eq, idref(&ea, a), idref(&ea, b))),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "true");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_crdt_increase_decrease_merge_match_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let counter = it.intern("Counter");
        let n_field = it.intern("n");
        let int_s = it.intern("Int");
        let a = it.intern("a");
        let b = it.intern("b");
        let show_s = it.intern("show");
        let stmts = vec![
            struct_def(counter, vec![(n_field, int_s, true)]),
            Stmt::Let { var: a, ty: None, value: new_struct(&ea, counter, vec![(n_field, num(&ea, 1))]), mutable: true },
            Stmt::Let { var: b, ty: None, value: new_struct(&ea, counter, vec![(n_field, num(&ea, 10))]), mutable: true },
            Stmt::IncreaseCrdt { object: idref(&ea, a), field: n_field, amount: num(&ea, 5) },
            Stmt::DecreaseCrdt { object: idref(&ea, a), field: n_field, amount: num(&ea, 2) },
            Stmt::MergeCrdt { source: idref(&ea, b), target: idref(&ea, a) },
            show(&ea, show_s, field_access(&ea, idref(&ea, a), n_field)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.trim(), "14");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    // ---- Sprint 6: builtins, call edges, MAX_CALL_DEPTH ------------------------

    #[test]
    fn vm_builtins_match_treewalk() {
        // One Show per builtin, covering Int/Float coercions.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let show_s = it.intern("show");
        let f25 = ea.alloc(Expr::Literal(Literal::Float(2.5)));
        let f29 = ea.alloc(Expr::Literal(Literal::Float(2.9)));
        let neg = bin(&ea, BinaryOpKind::Subtract, num(&ea, 0), num(&ea, 7));
        let calls: Vec<&Expr> = vec![
            calle(&ea, it.intern("abs"), vec![neg]),
            calle(&ea, it.intern("sqrt"), vec![num(&ea, 9)]),
            calle(&ea, it.intern("min"), vec![num(&ea, 3), f25]),
            calle(&ea, it.intern("max"), vec![num(&ea, 3), f25]),
            calle(&ea, it.intern("floor"), vec![f29]),
            calle(&ea, it.intern("ceil"), vec![f29]),
            calle(&ea, it.intern("round"), vec![f29]),
            calle(&ea, it.intern("pow"), vec![num(&ea, 2), num(&ea, 10)]),
            calle(&ea, it.intern("chr"), vec![num(&ea, 65)]),
            calle(&ea, it.intern("length"), vec![text(&ea, it.intern("hello"))]),
            calle(&ea, it.intern("format"), vec![num(&ea, 42)]),
        ];
        let mut stmts = Vec::new();
        for (k, c) in calls.into_iter().enumerate() {
            let v = it.intern(&format!("b{k}"));
            stmts.push(letb(v, c));
            stmts.push(show(&ea, show_s, idref(&ea, v)));
        }
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_parse_int_and_float_match_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let show_s = it.intern("show");
        let a = it.intern("a");
        let b = it.intern("b");
        let s42 = it.intern(" 42 ");
        let s25 = it.intern("2.5");
        let stmts = vec![
            letb(a, calle(&ea, it.intern("parseInt"), vec![text(&ea, s42)])),
            show(&ea, show_s, idref(&ea, a)),
            letb(b, calle(&ea, it.intern("parseFloat"), vec![text(&ea, s25)])),
            show(&ea, show_s, idref(&ea, b)),
        ];
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_builtin_error_messages_match_treewalk() {
        // parseInt("zz"), chr(-1), sqrt("x") arity/type errors — exact strings.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let zz = it.intern("zz");

        let stmts = vec![letb(x, calle(&ea, it.intern("parseInt"), vec![text(&ea, zz)]))];
        assert_err_parity(&stmts, &it);

        let neg1 = bin(&ea, BinaryOpKind::Subtract, num(&ea, 0), num(&ea, 1));
        let stmts = vec![letb(x, calle(&ea, it.intern("chr"), vec![neg1]))];
        assert_err_parity(&stmts, &it);

        // Wrong arity: the arity error fires BEFORE evaluating arguments.
        let stmts = vec![letb(x, calle(&ea, it.intern("abs"), vec![num(&ea, 1), num(&ea, 2)]))];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_copy_builtin_is_deep() {
        // copy(xs) then mutate xs — the copy must be unaffected (both engines).
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let ys = it.intern("ys");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: list_lit(&ea, vec![num(&ea, 1)]), mutable: true },
            letb(ys, calle(&ea, it.intern("copy"), vec![idref(&ea, xs)])),
            push_to(num(&ea, 2), idref(&ea, xs)),
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
            show(&ea, show_s, length_of(&ea, idref(&ea, ys))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["2", "1"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_alias_vs_copy_semantics() {
        // `Let a be xs` aliases (mutation visible through both names).
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let a = it.intern("a");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: list_lit(&ea, vec![num(&ea, 1)]), mutable: true },
            Stmt::Let { var: a, ty: None, value: idref(&ea, xs), mutable: true },
            push_to(num(&ea, 2), idref(&ea, a)),
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "2");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_unknown_function_error_matches_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let stmts = vec![letb(x, calle(&ea, it.intern("frobnicate"), vec![num(&ea, 1)]))];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_user_fn_arity_mismatch_matches_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let f = it.intern("f");
        let n = it.intern("n");
        let x = it.intern("x");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
        let body: &[Stmt] = sa.alloc_slice(vec![ret(idref(&ea, n))]);
        let stmts = vec![
            fndef(f, vec![(n, int_ty)], body),
            letb(x, calle(&ea, f, vec![num(&ea, 1), num(&ea, 2)])),
        ];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_infinite_recursion_hits_call_depth_limit_like_treewalk() {
        // Both engines must report the canonical stack-overflow error instead
        // of crashing the host. The tree-walker's SYNC path burns many native
        // frames per LOGOS call, so this runs on a big-stack thread (debug
        // frames are huge; release frames fit a normal stack at depth 1000).
        //
        // The recursion is deliberately NON-tail (`spin(n+1) - 1`): the call's
        // result feeds a subtraction, so it is not in tail position and is NOT
        // tail-call-optimized on any tier. (A direct `return spin(n+1)` is a
        // self-tail-call and would run in constant stack forever — TCO is a
        // language semantic here, covered by the `tco` differential tests.)
        // Subtraction also dodges the AOT accumulator transform, which only
        // strength-reduces `+`/`*`, so this stays unbounded recursion everywhere.
        std::thread::Builder::new()
            .stack_size(256 * 1024 * 1024)
            .spawn(|| {
                let ea: Arena<Expr> = Arena::new();
                let sa: Arena<Stmt> = Arena::new();
                let ta: Arena<TypeExpr> = Arena::new();
                let mut it = Interner::new();
                let spin = it.intern("spin");
                let n = it.intern("n");
                let x = it.intern("x");
                let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
                let body: &[Stmt] = sa.alloc_slice(vec![ret(bin(
                    &ea,
                    BinaryOpKind::Subtract,
                    calle(
                        &ea,
                        spin,
                        vec![bin(&ea, BinaryOpKind::Add, idref(&ea, n), num(&ea, 1))],
                    ),
                    num(&ea, 1),
                ))]);
                let stmts = vec![
                    fndef(spin, vec![(n, int_ty)], body),
                    letb(x, calle(&ea, spin, vec![num(&ea, 0)])),
                ];
                let vm_err = compile_and_run(&stmts, &it).unwrap_err();
                assert_eq!(vm_err, "Stack overflow: maximum call depth exceeded");
                let (_, tw_err) = run_treewalk_outcome(&stmts, &it);
                assert_eq!(
                    tw_err.as_deref(),
                    Some("Stack overflow: maximum call depth exceeded")
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn vm_depth_900_recursion_still_fine() {
        // The limit is 1000; 900 must work in both engines.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let down = it.intern("down");
        let n = it.intern("n");
        let x = it.intern("x");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
        let base: &[Stmt] = sa.alloc_slice(vec![ret(num(&ea, 0))]);
        let body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::If {
                cond: bin(&ea, BinaryOpKind::LtEq, idref(&ea, n), num(&ea, 0)),
                then_block: base,
                else_block: None,
            },
            ret(calle(&ea, down, vec![bin(&ea, BinaryOpKind::Subtract, idref(&ea, n), num(&ea, 1))])),
        ]);
        let stmts = vec![
            fndef(down, vec![(n, int_ty)], body),
            letb(x, calle(&ea, down, vec![num(&ea, 900)])),
            show(&ea, show_s, idref(&ea, x)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "0");
    }

    #[test]
    fn vm_show_to_function_recipient_calls_it() {
        // `Show x to f` calls f(x); `Give x to f` likewise.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let sink = it.intern("sink");
        let v = it.intern("v");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
        let body: &[Stmt] = sa.alloc_slice(vec![show(
            &ea,
            show_s,
            bin(&ea, BinaryOpKind::Multiply, idref(&ea, v), num(&ea, 2)),
        )]);
        let stmts = vec![
            fndef(sink, vec![(v, int_ty)], body),
            Stmt::Show { object: num(&ea, 21), recipient: idref(&ea, sink) },
            Stmt::Give { object: num(&ea, 5), recipient: idref(&ea, sink) },
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["42", "10"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    // ---- Sprint 5: block scoping, Break, Repeat --------------------------------

    /// Full-outcome differential: output AND error must both match the oracle.
    fn assert_outcome_eq_treewalk(stmts: &[Stmt], interner: &Interner) {
        let (vm_out, vm_err) = run_to_outcome(stmts, interner, None, None);
        let (tw_out, tw_err) = match run_treewalk_outcome(stmts, interner) {
            (o, e) => (o, e),
        };
        assert_eq!(
            normalize(&vm_out),
            normalize(&tw_out),
            "partial output diverged (vm err {:?}, tw err {:?})",
            vm_err,
            tw_err
        );
        assert_eq!(vm_err, tw_err, "error diverged");
    }

    /// Tree-walker outcome with partial output preserved.
    fn run_treewalk_outcome(stmts: &[Stmt], interner: &Interner) -> (String, Option<String>) {
        use crate::interpreter::{Interpreter, OutputCallback};
        let buf = Rc::new(RefCell::new(String::new()));
        let sink = buf.clone();
        let cb: OutputCallback = Rc::new(RefCell::new(move |s: String| {
            sink.borrow_mut().push_str(&s);
            sink.borrow_mut().push('\n');
        }));
        let mut interp = Interpreter::new(interner).with_output_callback(cb);
        let err = interp.run_sync(stmts).err();
        let out = buf.borrow().clone();
        (out, err)
    }

    fn brk<'a>() -> Stmt<'a> {
        Stmt::Break
    }
    fn repeat_ident<'a>(var: Symbol, iterable: &'a Expr<'a>, body: &'a [Stmt<'a>]) -> Stmt<'a> {
        use crate::ast::stmt::Pattern;
        Stmt::Repeat { pattern: Pattern::Identifier(var), iterable, body }
    }

    #[test]
    fn vm_break_exits_innermost_while() {
        // i counts 0..; If i >= 3: Break — output 0,1,2.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let i = it.intern("i");
        let show_s = it.intern("show");
        let break_blk: &[Stmt] = sa.alloc_slice(vec![brk()]);
        let body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::If {
                cond: bin(&ea, BinaryOpKind::GtEq, idref(&ea, i), num(&ea, 3)),
                then_block: break_blk,
                else_block: None,
            },
            show(&ea, show_s, idref(&ea, i)),
            Stmt::Set { target: i, value: bin(&ea, BinaryOpKind::Add, idref(&ea, i), num(&ea, 1)) },
        ]);
        let stmts = vec![
            Stmt::Let { var: i, ty: None, value: num(&ea, 0), mutable: true },
            Stmt::While { cond: bin(&ea, BinaryOpKind::Lt, idref(&ea, i), num(&ea, 100)), body, decreasing: None },
            show(&ea, show_s, idref(&ea, i)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["0", "1", "2", "3"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_break_only_exits_inner_loop() {
        // Outer runs 2 iterations; inner breaks immediately each time.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let i = it.intern("i");
        let j = it.intern("j");
        let show_s = it.intern("show");
        let inner_body: &[Stmt] = sa.alloc_slice(vec![brk()]);
        let outer_body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::Let { var: j, ty: None, value: num(&ea, 0), mutable: true },
            Stmt::While { cond: bin(&ea, BinaryOpKind::Lt, idref(&ea, j), num(&ea, 5)), body: inner_body, decreasing: None },
            show(&ea, show_s, idref(&ea, i)),
            Stmt::Set { target: i, value: bin(&ea, BinaryOpKind::Add, idref(&ea, i), num(&ea, 1)) },
        ]);
        let stmts = vec![
            Stmt::Let { var: i, ty: None, value: num(&ea, 0), mutable: true },
            Stmt::While { cond: bin(&ea, BinaryOpKind::Lt, idref(&ea, i), num(&ea, 2)), body: outer_body, decreasing: None },
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["0", "1"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_break_at_top_level_halts_program() {
        // The tree-walker's run loop treats Break at Main top level as stop.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let show_s = it.intern("show");
        let stmts = vec![
            show(&ea, show_s, num(&ea, 1)),
            brk(),
            show(&ea, show_s, num(&ea, 2)),
        ];
        assert_outcome_eq_treewalk(&stmts, &it);
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "1");
    }

    #[test]
    fn vm_return_at_top_level_halts_program() {
        // Return in Main = stop, not "return with no caller".
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let show_s = it.intern("show");
        let stmts = vec![
            show(&ea, show_s, num(&ea, 1)),
            ret(num(&ea, 99)),
            show(&ea, show_s, num(&ea, 2)),
        ];
        assert_outcome_eq_treewalk(&stmts, &it);
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "1");
    }

    #[test]
    fn vm_block_let_does_not_leak_scope() {
        // Let inside an If-block is undone after the block (tree-walker
        // execute_block scoping). The use after the block fails AT RUNTIME with
        // the partial output preserved.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let t = it.intern("t");
        let show_s = it.intern("show");
        let then_blk: &[Stmt] = sa.alloc_slice(vec![
            letb(t, num(&ea, 5)),
            show(&ea, show_s, idref(&ea, t)),
        ]);
        let stmts = vec![
            Stmt::If { cond: boolean(&ea, true), then_block: then_blk, else_block: None },
            show(&ea, show_s, idref(&ea, t)),
        ];
        assert_outcome_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_unbound_identifier_in_dead_branch_is_free() {
        // `Show ghost` inside `If false:` must not fail in either engine —
        // unbound names are a RUNTIME error, not a compile error.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let ghost = it.intern("ghost");
        let show_s = it.intern("show");
        let dead: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, idref(&ea, ghost))]);
        let stmts = vec![
            Stmt::If { cond: boolean(&ea, false), then_block: dead, else_block: None },
            show(&ea, show_s, num(&ea, 7)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "7");
        assert_outcome_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_repeat_list_snapshot_semantics() {
        // Pushing to the list inside the loop must NOT extend the iteration:
        // the tree-walker snapshots the collection before looping.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let x = it.intern("x");
        let show_s = it.intern("show");
        let body: &[Stmt] = sa.alloc_slice(vec![
            push_to(num(&ea, 9), idref(&ea, xs)),
            show(&ea, show_s, idref(&ea, x)),
        ]);
        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: list_lit(&ea, vec![num(&ea, 1), num(&ea, 2)]), mutable: true },
            repeat_ident(x, idref(&ea, xs), body),
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["1", "2", "4"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_repeat_over_text_iterates_chars() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let s = it.intern("s");
        let c = it.intern("c");
        let show_s = it.intern("show");
        let abc = it.intern("abc");
        let body: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, idref(&ea, c))]);
        let stmts = vec![
            letb(s, text(&ea, abc)),
            repeat_ident(c, idref(&ea, s), body),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["a", "b", "c"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_repeat_map_single_entry_tuple_pattern() {
        use crate::ast::stmt::Pattern;
        // One-entry map (iteration order is nondeterministic for larger maps).
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let m = it.intern("m");
        let k = it.intern("k");
        let v = it.intern("v");
        let map_ty = it.intern("Map");
        let key_a = it.intern("a");
        let show_s = it.intern("show");
        let body: &[Stmt] = sa.alloc_slice(vec![
            show(&ea, show_s, idref(&ea, k)),
            show(&ea, show_s, idref(&ea, v)),
        ]);
        let stmts = vec![
            Stmt::Let { var: m, ty: None, value: new_coll(&ea, map_ty), mutable: true },
            Stmt::SetIndex { collection: idref(&ea, m), index: text(&ea, key_a), value: num(&ea, 10) },
            Stmt::Repeat { pattern: Pattern::Tuple(vec![k, v]), iterable: idref(&ea, m), body },
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["a", "10"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_repeat_tuple_pattern_on_non_tuple_errors_like_treewalk() {
        use crate::ast::stmt::Pattern;
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let k = it.intern("k");
        let v = it.intern("v");
        let show_s = it.intern("show");
        let body: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, idref(&ea, k))]);
        let stmts = vec![Stmt::Repeat {
            pattern: Pattern::Tuple(vec![k, v]),
            iterable: list_lit(&ea, vec![num(&ea, 1)]),
            body,
        }];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_repeat_break_exits_and_loop_var_scoped() {
        // Break exits the Repeat; the loop variable is gone after the loop.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let x = it.intern("x");
        let show_s = it.intern("show");
        let break_blk: &[Stmt] = sa.alloc_slice(vec![brk()]);
        let body: &[Stmt] = sa.alloc_slice(vec![
            show(&ea, show_s, idref(&ea, x)),
            Stmt::If {
                cond: bin(&ea, BinaryOpKind::GtEq, idref(&ea, x), num(&ea, 2)),
                then_block: break_blk,
                else_block: None,
            },
        ]);
        let stmts = vec![
            letb(xs, list_lit(&ea, vec![num(&ea, 1), num(&ea, 2), num(&ea, 3)])),
            repeat_ident(x, idref(&ea, xs), body),
            show(&ea, show_s, idref(&ea, x)),
        ];
        // Both engines: print 1, 2, then `x` is out of scope after the loop.
        assert_outcome_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_repeat_over_int_errors_like_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let show_s = it.intern("show");
        let body: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, idref(&ea, x))]);
        let stmts = vec![repeat_ident(x, num(&ea, 42), body)];
        assert_err_parity(&stmts, &it);
    }

    // ---- Sprint 5 (statement core): Pop, RuntimeAssert, Zone, Concurrent ------

    #[test]
    fn vm_pop_into_binds_and_empty_pop_is_nothing() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let v = it.intern("v");
        let w = it.intern("w");
        let show_s = it.intern("show");
        let stmts = vec![
            Stmt::Let { var: xs, ty: None, value: list_lit(&ea, vec![num(&ea, 1), num(&ea, 2)]), mutable: true },
            Stmt::Pop { collection: idref(&ea, xs), into: Some(v) },
            show(&ea, show_s, idref(&ea, v)),
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
            Stmt::Pop { collection: idref(&ea, xs), into: None },
            Stmt::Pop { collection: idref(&ea, xs), into: Some(w) },
            show(&ea, show_s, idref(&ea, w)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["2", "1", "nothing"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_runtime_assert_passes_and_fails_like_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let show_s = it.intern("show");
        let ok_stmts = vec![
            Stmt::RuntimeAssert { condition: boolean(&ea, true) , hard: false },
            show(&ea, show_s, num(&ea, 1)),
        ];
        assert_eq!(compile_and_run(&ok_stmts, &it).unwrap().trim(), "1");
        assert_vm_eq_treewalk(&ok_stmts, &it);

        let fail_stmts = vec![
            show(&ea, show_s, num(&ea, 1)),
            Stmt::RuntimeAssert { condition: boolean(&ea, false) , hard: false },
            show(&ea, show_s, num(&ea, 2)),
        ];
        assert_outcome_eq_treewalk(&fail_stmts, &it);
    }

    #[test]
    fn vm_zone_swallows_return_inside_function() {
        // The tree-walker DISCARDS a zone body's ControlFlow: a Return inside
        // a Zone does not return from the function.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let f = it.intern("f");
        let z = it.intern("z");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));
        let _ = int_ty;

        let zone_body: &[Stmt] = sa.alloc_slice(vec![ret(num(&ea, 5))]);
        let f_body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::Zone { name: z, capacity: None, source_file: None, body: zone_body },
            show(&ea, show_s, num(&ea, 77)),
            ret(num(&ea, 1)),
        ]);
        let stmts = vec![
            fndef(f, vec![], f_body),
            letb(r, calle(&ea, f, vec![])),
            show(&ea, show_s, idref(&ea, r)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["77", "1"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_zone_swallows_break_in_loop() {
        // `While …: Zone: Break` — the zone catches the Break before the loop
        // sees it, so the loop runs to its own condition.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let i = it.intern("i");
        let z = it.intern("z");
        let show_s = it.intern("show");
        let zone_body: &[Stmt] = sa.alloc_slice(vec![brk()]);
        let body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::Zone { name: z, capacity: None, source_file: None, body: zone_body },
            show(&ea, show_s, idref(&ea, i)),
            Stmt::Set { target: i, value: bin(&ea, BinaryOpKind::Add, idref(&ea, i), num(&ea, 1)) },
        ]);
        let stmts = vec![
            Stmt::Let { var: i, ty: None, value: num(&ea, 0), mutable: true },
            Stmt::While { cond: bin(&ea, BinaryOpKind::Lt, idref(&ea, i), num(&ea, 2)), body, decreasing: None },
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["0", "1"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_return_inside_repeat_inside_zone_unwinds_iterator() {
        // The Return jumps out across a live Repeat into the zone end; the
        // iterator must be unwound so a LATER Repeat still works.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let z = it.intern("z");
        let x = it.intern("x");
        let y = it.intern("y");
        let show_s = it.intern("show");
        let rep_body: &[Stmt] = sa.alloc_slice(vec![ret(idref(&ea, x))]);
        let zone_body: &[Stmt] = sa.alloc_slice(vec![repeat_ident(
            x,
            list_lit(&ea, vec![num(&ea, 1), num(&ea, 2), num(&ea, 3)]),
            rep_body,
        )]);
        let second_body: &[Stmt] = sa.alloc_slice(vec![show(&ea, show_s, idref(&ea, y))]);
        let stmts = vec![
            Stmt::Zone { name: z, capacity: None, source_file: None, body: zone_body },
            show(&ea, show_s, num(&ea, 0)),
            repeat_ident(y, list_lit(&ea, vec![num(&ea, 7), num(&ea, 8)]), second_body),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["0", "7", "8"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_concurrent_tasks_run_sequentially_share_scope_and_swallow_flow() {
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let a = it.intern("a");
        let show_s = it.intern("show");
        let tasks: &[Stmt] = sa.alloc_slice(vec![
            letb(a, num(&ea, 1)),
            show(&ea, show_s, idref(&ea, a)),
            ret(num(&ea, 9)),
            show(&ea, show_s, num(&ea, 3)),
        ]);
        let stmts = vec![
            Stmt::Concurrent { tasks },
            // The task's Let persists (tasks execute in the enclosing scope),
            // and the Return was swallowed.
            show(&ea, show_s, idref(&ea, a)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["1", "3", "1"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    // ---- Sprint 4: short-circuit And/Or, Concat/BitXor/Shl/Shr, Not, literals --

    fn not_e<'a>(ea: &'a Arena<Expr<'a>>, operand: &'a Expr<'a>) -> &'a Expr<'a> {
        ea.alloc(Expr::Not { operand })
    }

    /// `## To noisy: Show 9. Return 1.` — its output is the side-effect probe.
    fn noisy_fn<'a>(
        ea: &'a Arena<Expr<'a>>,
        sa: &'a Arena<Stmt<'a>>,
        noisy: Symbol,
        show_s: Symbol,
    ) -> Stmt<'a> {
        let body: &[Stmt] = sa.alloc_slice(vec![
            show(ea, show_s, num(ea, 9)),
            ret(num(ea, 1)),
        ]);
        fndef(noisy, vec![], body)
    }

    #[test]
    fn vm_and_short_circuits_side_effects() {
        // false and noisy() — noisy must NOT run; output is just "false".
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let noisy = it.intern("noisy");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let stmts = vec![
            noisy_fn(&ea, &sa, noisy, show_s),
            letb(r, bin(&ea, BinaryOpKind::And, boolean(&ea, false), calle(&ea, noisy, vec![]))),
            show(&ea, show_s, idref(&ea, r)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "false");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_or_short_circuits_side_effects() {
        // true or noisy() — noisy must NOT run; output is just "true".
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let noisy = it.intern("noisy");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let stmts = vec![
            noisy_fn(&ea, &sa, noisy, show_s),
            letb(r, bin(&ea, BinaryOpKind::Or, boolean(&ea, true), calle(&ea, noisy, vec![]))),
            show(&ea, show_s, idref(&ea, r)),
        ];
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "true");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_and_int_is_eager_bitwise() {
        // 6 and 3 → 2 (bitwise). 6 and noisy() → noisy RUNS (prints 9),
        // returns 1, and 6 & 1 == 0.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let noisy = it.intern("noisy");
        let a = it.intern("a");
        let b = it.intern("b");
        let show_s = it.intern("show");
        let stmts = vec![
            noisy_fn(&ea, &sa, noisy, show_s),
            letb(a, bin(&ea, BinaryOpKind::And, num(&ea, 6), num(&ea, 3))),
            show(&ea, show_s, idref(&ea, a)),
            letb(b, bin(&ea, BinaryOpKind::And, num(&ea, 6), calle(&ea, noisy, vec![]))),
            show(&ea, show_s, idref(&ea, b)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["2", "9", "0"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_and_or_mixed_int_bool_uses_truthiness() {
        // Int left forces the eager path; a non-Int right falls to truthiness.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let a = it.intern("a");
        let b = it.intern("b");
        let show_s = it.intern("show");
        let stmts = vec![
            letb(a, bin(&ea, BinaryOpKind::And, num(&ea, 1), boolean(&ea, false))),
            show(&ea, show_s, idref(&ea, a)),
            letb(b, bin(&ea, BinaryOpKind::Or, num(&ea, 0), boolean(&ea, true))),
            show(&ea, show_s, idref(&ea, b)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["false", "true"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_not_is_logical_for_bool_bitwise_for_int() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let a = it.intern("a");
        let b = it.intern("b");
        let show_s = it.intern("show");
        let stmts = vec![
            letb(a, not_e(&ea, num(&ea, 6))),
            show(&ea, show_s, idref(&ea, a)),
            letb(b, not_e(&ea, boolean(&ea, true))),
            show(&ea, show_s, idref(&ea, b)),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["-7", "false"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_not_text_error_matches_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let hi = it.intern("hi");
        let stmts = vec![letb(x, not_e(&ea, text(&ea, hi)))];
        assert_err_parity(&stmts, &it);
    }

    #[test]
    fn vm_concat_bitxor_shl_shr_match_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let show_s = it.intern("show");
        let names: Vec<Symbol> = (0..4).map(|k| it.intern(&format!("o{k}"))).collect();
        let stmts = vec![
            letb(names[0], bin(&ea, BinaryOpKind::Concat, num(&ea, 1), num(&ea, 2))),
            show(&ea, show_s, idref(&ea, names[0])),
            letb(names[1], bin(&ea, BinaryOpKind::BitXor, num(&ea, 6), num(&ea, 3))),
            show(&ea, show_s, idref(&ea, names[1])),
            letb(names[2], bin(&ea, BinaryOpKind::Shl, num(&ea, 1), num(&ea, 3))),
            show(&ea, show_s, idref(&ea, names[2])),
            letb(names[3], bin(&ea, BinaryOpKind::Shr, num(&ea, 16), num(&ea, 2))),
            show(&ea, show_s, idref(&ea, names[3])),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.lines().collect::<Vec<_>>(), vec!["12", "5", "8", "4"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_char_nothing_and_temporal_literals_display_like_treewalk() {
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let show_s = it.intern("show");
        let lits: Vec<&Expr> = vec![
            ea.alloc(Expr::Literal(Literal::Char('x'))),
            ea.alloc(Expr::Literal(Literal::Nothing)),
            ea.alloc(Expr::Literal(Literal::Duration(1_500_000_000))),
            ea.alloc(Expr::Literal(Literal::Date(19753))),
            ea.alloc(Expr::Literal(Literal::Moment(86_400_000_000_000))),
            ea.alloc(Expr::Literal(Literal::Span { months: 2, days: 3 })),
            ea.alloc(Expr::Literal(Literal::Time(3_600_000_000_000))),
        ];
        let mut stmts = Vec::new();
        for (k, lit) in lits.into_iter().enumerate() {
            let v = it.intern(&format!("lit{k}"));
            stmts.push(letb(v, lit));
            stmts.push(show(&ea, show_s, idref(&ea, v)));
        }
        assert_vm_eq_treewalk(&stmts, &it);
    }

    // ---- Sprint 0: encoding widening + compiler hardening ---------------------

    #[test]
    fn vm_compiles_300_element_list_literal() {
        // 300 > u8::MAX — list literals must not be capped at 255 elements.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let show_s = it.intern("show");
        let items: Vec<&Expr> = (0..300).map(|k| num(&ea, k)).collect();
        let stmts = vec![
            letb(xs, list_lit(&ea, items)),
            show(&ea, show_s, length_of(&ea, idref(&ea, xs))),
            show(&ea, show_s, index_at(&ea, idref(&ea, xs), num(&ea, 300))),
        ];
        let out = compile_and_run(&stmts, &it).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, vec!["300", "299"]);
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_compiles_main_with_400_locals() {
        // 400 locals exceed a u8 register index — frames must address more
        // than 256 registers.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let show_s = it.intern("show");
        let mut stmts = Vec::new();
        let mut last = None;
        for k in 0..400i64 {
            let v = it.intern(&format!("v{k}"));
            stmts.push(letb(v, num(&ea, k)));
            last = Some(v);
        }
        stmts.push(show(&ea, show_s, idref(&ea, last.unwrap())));
        assert_eq!(compile_and_run(&stmts, &it).unwrap().trim(), "399");
        assert_vm_eq_treewalk(&stmts, &it);
    }

    #[test]
    fn vm_const_pool_dedups_identical_literals() {
        // The same literal used many times must occupy one constant-pool slot.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let mut stmts = Vec::new();
        for k in 0..50 {
            let v = it.intern(&format!("c{k}"));
            stmts.push(letb(v, num(&ea, 7)));
        }
        let program = Compiler::compile(&stmts, &it).unwrap();
        assert_eq!(
            program.constants.len(),
            1,
            "identical Int literals must dedup to a single pool entry"
        );
    }

    #[test]
    fn vm_patch_jump_is_total() {
        // Backpatching a non-jump instruction is a compiler bug that must
        // surface as Err, never a panic.
        let mut code = vec![Op::Halt];
        assert!(super::compiler::patch_jump(&mut code, 0, 5).is_err());

        let mut code = vec![Op::Jump { target: usize::MAX }];
        assert!(super::compiler::patch_jump(&mut code, 0, 5).is_ok());
        assert!(matches!(code[0], Op::Jump { target: 5 }));

        // Out-of-bounds instruction index must also be an Err, not a panic.
        let mut code = vec![Op::Halt];
        assert!(super::compiler::patch_jump(&mut code, 9, 5).is_err());
    }

    #[test]
    fn vm_deeply_nested_expr_errors_not_overflows() {
        // A 100k-deep expression must produce a compile error, not blow the
        // native stack.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let mut e = num(&ea, 1);
        for _ in 0..100_000 {
            e = bin(&ea, BinaryOpKind::Add, e, num(&ea, 1));
        }
        let stmts = vec![letb(x, e)];
        let err = Compiler::compile(&stmts, &it).unwrap_err();
        assert!(err.contains("too deeply nested"), "got: {err}");
    }

    #[test]
    fn vm_range_with_float_bound_errors_not_panics() {
        // `1 to 2.5` — the tree-walker errors with "Range requires Int bounds";
        // the VM must return the same Err, never panic.
        let ea: Arena<Expr> = Arena::new();
        let mut it = Interner::new();
        let xs = it.intern("xs");
        let f = ea.alloc(Expr::Literal(Literal::Float(2.5)));
        let stmts = vec![letb(xs, range_e(&ea, num(&ea, 1), f))];
        let vm_err = compile_and_run(&stmts, &it).unwrap_err();
        let tw_err = run_treewalk(&stmts, &it).unwrap_err();
        assert_eq!(vm_err, tw_err);
    }

    #[test]
    fn vm_register_file_growth_is_capped() {
        // WIDE frames recursing: each frame claims ~3000 registers (a 3000-
        // element list literal), so the register file hits MAX_REGISTER_FILE
        // (1M) near depth ~350 — well before the 1000 call-depth limit. The
        // VM must error, not consume unbounded memory. (VM-only: the
        // tree-walker would blow the native stack on this program.)
        //
        // The recursion is NON-tail (`spin(n+1) - 1`): a direct `return spin(n+1)`
        // is a self-tail-call and TCO loops it in constant stack (one frame, no
        // register growth), so it would never hit the cap. The `- 1` keeps the
        // call out of tail position so real frames accumulate.
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let ta: Arena<TypeExpr> = Arena::new();
        let mut it = Interner::new();
        let spin = it.intern("spin");
        let n = it.intern("n");
        let big = it.intern("big");
        let r = it.intern("r");
        let show_s = it.intern("show");
        let int_ty = ta.alloc(TypeExpr::Primitive(it.intern("Int")));

        let then_blk: &[Stmt] = sa.alloc_slice(vec![ret(num(&ea, 0))]);
        let cond = bin(&ea, BinaryOpKind::GtEq, idref(&ea, n), num(&ea, 900));
        let wide: Vec<&Expr> = (0..3000).map(|_| num(&ea, 0)).collect();
        let rec = calle(&ea, spin, vec![bin(&ea, BinaryOpKind::Add, idref(&ea, n), num(&ea, 1))]);
        let rec_nontail = bin(&ea, BinaryOpKind::Subtract, rec, num(&ea, 1));
        let body: &[Stmt] = sa.alloc_slice(vec![
            Stmt::If { cond, then_block: then_blk, else_block: None },
            letb(big, list_lit(&ea, wide)),
            ret(rec_nontail),
        ]);
        let stmts = vec![
            fndef(spin, vec![(n, int_ty)], body),
            letb(r, calle(&ea, spin, vec![num(&ea, 0)])),
            show(&ea, show_s, idref(&ea, r)),
        ];
        let err = compile_and_run(&stmts, &it).unwrap_err();
        assert!(err.contains("register file"), "got: {err}");
    }

    #[test]
    fn vm_if_else_takes_correct_branch() {
        // Let x be 3. If x > 5: Show 100. Otherwise: Show 200.   → 200
        let ea: Arena<Expr> = Arena::new();
        let sa: Arena<Stmt> = Arena::new();
        let mut it = Interner::new();
        let x = it.intern("x");
        let console = it.intern("show");

        let three = ea.alloc(Expr::Literal(Literal::Number(3)));
        let x_c = ea.alloc(Expr::Identifier(x));
        let five = ea.alloc(Expr::Literal(Literal::Number(5)));
        let cond = ea.alloc(Expr::BinaryOp { op: BinaryOpKind::Gt, left: x_c, right: five });

        let hundred = ea.alloc(Expr::Literal(Literal::Number(100)));
        let two_hundred = ea.alloc(Expr::Literal(Literal::Number(200)));
        let cref1 = ea.alloc(Expr::Identifier(console));
        let cref2 = ea.alloc(Expr::Identifier(console));
        let then_block: &[Stmt] = sa.alloc_slice(vec![Stmt::Show { object: hundred, recipient: cref1 }]);
        let else_block: &[Stmt] = sa.alloc_slice(vec![Stmt::Show { object: two_hundred, recipient: cref2 }]);

        let stmts = vec![
            Stmt::Let { var: x, ty: None, value: three, mutable: false },
            Stmt::If { cond, then_block, else_block: Some(else_block) },
        ];

        let out = compile_and_run(&stmts, &it).unwrap();
        assert_eq!(out.trim(), "200");
        assert_vm_eq_treewalk(&stmts, &it);
    }
}
