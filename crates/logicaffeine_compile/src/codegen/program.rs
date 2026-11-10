use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::rc::Rc;

use crate::analysis::policy::PolicyRegistry;
use crate::analysis::registry::{TypeDef, TypeRegistry};
use crate::analysis::types::RustNames;
use crate::ast::stmt::{Expr, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::optimization::{Opt, OptimizationConfig};
use crate::sourcemap::{OwnershipRole, SourceMap, SourceMapBuilder};
use crate::token::Span as LogosSpan;

use super::context::{analyze_variable_capabilities, RefinementContext};
use crate::analysis::callgraph::CallGraph;

/// Does the program call a bit-intrinsic helper `name` (`count_ones` /
/// `leading_zeros` / `trailing_zeros`, inserted by `optimize::popcount_leaf`)?
/// Gates emission of that helper so programs that don't use it have
/// byte-identical codegen (keeps the codegen snapshot tests stable).
fn program_calls_named(stmts: &[Stmt], interner: &Interner, name: &str) -> bool {
    fn in_expr(e: &Expr, it: &Interner, name: &str) -> bool {
        match e {
            Expr::Call { function, args } => {
                it.resolve(*function) == name || args.iter().any(|a| in_expr(a, it, name))
            }
            Expr::BinaryOp { left, right, .. } => {
                in_expr(left, it, name) || in_expr(right, it, name)
            }
            Expr::Not { operand } => in_expr(operand, it, name),
            Expr::Index { collection, index } => {
                in_expr(collection, it, name) || in_expr(index, it, name)
            }
            Expr::Length { collection } => in_expr(collection, it, name),
            _ => false,
        }
    }
    fn in_block(b: &[Stmt], it: &Interner, name: &str) -> bool {
        b.iter().any(|s| match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, it, name),
            Stmt::Return { value } => value.map_or(false, |e| in_expr(e, it, name)),
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                in_expr(cond, it, name)
                    || in_block(then_block, it, name)
                    || else_block.map_or(false, |b| in_block(b, it, name))
            }
            Stmt::While { cond, body, .. } => in_expr(cond, it, name) || in_block(body, it, name),
            Stmt::FunctionDef { body, .. } => in_block(body, it, name),
            _ => false,
        })
    }
    in_block(stmts, interner, name)
}
use crate::analysis::liveness::LivenessResult;
use crate::analysis::readonly::{MutableBorrowParams, ReadonlyParams};

use super::detection::{
    collect_async_functions, collect_boxed_fields, collect_crdt_register_fields,
    collect_de_rc_seqs, collect_escaping_collection_vars, collect_give_args,
    collect_interleaved_groups, collect_mutable_vars, collect_mutable_vars_stmt,
    collect_pipe_sender_params, collect_pipe_vars, collect_pure_functions,
    collect_scalarizable_seqs, collect_single_char_text_vars, collect_vec_return_fns,
    detect_double_recursion_closed_form, is_copy_type_expr, is_result_type, is_vec_type_expr,
    requires_async, requires_vfs, serves, should_inline, should_memoize, vec_to_mut_slice_type,
    vec_to_slice_type,
};
use super::escape_rust_ident;
use super::ffi::{
    codegen_logos_runtime_preamble, collect_c_export_reference_types,
    collect_c_export_value_type_structs, has_c_exports, has_c_exports_with_text, has_wasm_exports,
};
use super::marshal::{codegen_c_export_with_marshaling, is_char_type, is_text_type};
use super::policy::codegen_policy_impls;
use super::stmt::codegen_stmt;
use super::tce::{
    body_has_top_level_tail_pair, codegen_mutual_tce_pair, codegen_stmt_acc, codegen_stmt_tce,
    codegen_tce_loopback, detect_mutual_tce_pairs, is_tail_recursive,
};
use super::types::{
    codegen_enum_def, codegen_struct_def, codegen_type_expr, infer_return_type_from_body,
    logos_return_type_to_rust, rust_type_ident,
};
use crate::tail_call::detect_accumulator_pattern;

/// Emit `pub type <Bare><generics> = <Alias__Name><generics>;` for a bare type
/// whose canonical namespaced struct/enum is emitted elsewhere. Preserves the
/// type's generics (so a generic imported type stays generic) and escapes a
/// keyword name (`type`/`match`/…), so construction through the bare spelling
/// resolves to the one canonical Rust type.
fn emit_bare_type_alias(
    output: &mut String,
    simple: &str,
    twin: &str,
    generics: &[logicaffeine_base::Symbol],
    interner: &Interner,
) {
    use std::fmt::Write as _;
    let gstr = if generics.is_empty() {
        String::new()
    } else {
        let params: Vec<&str> = generics.iter().map(|g| interner.resolve(*g)).collect();
        format!("<{}>", params.join(", "))
    };
    let _ = writeln!(
        output,
        "    pub type {}{gstr} = {}{gstr};",
        escape_rust_ident(simple),
        rust_type_ident(twin)
    );
}

use super::{
    classify_type_for_c_abi, codegen_c_accessors, collect_c_export_ref_structs, CAbiClass,
};

/// Check if a function body contains escape blocks (raw Rust code).
/// Functions with escape blocks should not have their param types changed
/// by borrow optimization, since the escape code may depend on specific types.
pub(crate) fn body_contains_escape(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| stmt_contains_escape(stmt))
}

fn stmt_contains_escape(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Escape { .. } => true,
        Stmt::Let { value, .. } => expr_contains_escape(value),
        Stmt::Set { value, .. } => expr_contains_escape(value),
        Stmt::If {
            then_block,
            else_block,
            ..
        } => {
            body_contains_escape(then_block)
                || else_block
                    .as_ref()
                    .map_or(false, |eb| body_contains_escape(eb))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => body_contains_escape(body),
        Stmt::Inspect { arms, .. } => arms.iter().any(|arm| body_contains_escape(arm.body)),
        _ => false,
    }
}

fn expr_contains_escape(expr: &Expr) -> bool {
    matches!(expr, Expr::Escape { .. })
}

/// - Uses `Distributed<T>` when both Mount and Sync detected
/// - Boxes recursive enum fields
/// Generates a complete Rust program from LOGOS statements.
///
/// This is the main entry point for code generation. It produces a full Rust
/// program including:
/// - Prelude imports (`use logicaffeine_data::*;`)
/// - Type definitions (structs, enums, inductive types)
/// - Policy structs with capability methods
/// - Main function with async runtime if needed
/// - VFS initialization for file operations
///
/// # Arguments
///
/// * `stmts` - The parsed LOGOS statements to compile
/// * `registry` - Type definitions discovered during parsing
/// * `policies` - Policy definitions for access control
/// * `interner` - Symbol interner for resolving names
///
/// # Returns
///
/// A complete Rust source code string ready for compilation.
/// An affine read-only array's declaration (`Let A be a new Seq …` where `A` was
/// recognized as an affine read-only array). Codegen deletes such arrays — the
/// build push is suppressed and every read substitutes the closed form — so the
/// decl is filtered out of the statement stream before peephole dispatch, where
/// a `vec_with_capacity`/`vec_fill` pattern would otherwise re-materialize it.
fn is_affine_array_decl(stmt: &Stmt, ctx: &RefinementContext) -> bool {
    matches!(stmt, Stmt::Let { var, value, .. }
        if matches!(value, Expr::New { .. }) && ctx.affine_array(*var).is_some())
}

/// A statement that BUILDS a constant-table local (its `Let` or one of its constant `Push`es). Filtered
/// out of the body: the table is emitted once as a stack array `[T; N]` at the top of the function, so
/// its original heap-`Vec` build (`Seq::default()` + `push` × N) is dropped.
fn is_const_table_stmt(stmt: &Stmt, ctx: &RefinementContext) -> bool {
    match stmt {
        Stmt::Let { var, .. } => ctx.const_table(*var).is_some(),
        Stmt::Push {
            collection: Expr::Identifier(c),
            ..
        } => ctx.const_table(*c).is_some(),
        _ => false,
    }
}

/// Recursively collect every symbol in `stmts` (INCLUDING nested loop / if / match blocks) whose interned
/// name equals `name`. Needed because the parser mints distinct symbols per occurrence of an identifier,
/// so a constant-table's `[T; N]` type must be registered for the USE-SITE symbol (e.g. a call argument
/// inside a loop), not only the `Let`-binding symbol — `variable_types` is symbol-keyed.
fn collect_named_syms(
    stmts: &[Stmt],
    name: &str,
    interner: &Interner,
    out: &mut std::collections::HashSet<Symbol>,
) {
    for s in stmts {
        super::worklist::for_each_stmt_expr(s, &mut |e| {
            super::worklist::visit_idents(e, &mut |sym| {
                if name == interner.resolve(sym) {
                    out.insert(sym);
                }
            });
        });
        match s {
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_named_syms(then_block, name, interner, out);
                if let Some(eb) = else_block {
                    collect_named_syms(eb, name, interner, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_named_syms(body, name, interner, out);
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    collect_named_syms(arm.body, name, interner, out);
                }
            }
            _ => {}
        }
    }
}

/// Run i64→i32 narrowing detection (on by default). `LOGOS_NO_NARROW` is the
/// kill-switch (A/B and debugging), matching the other codegen toggles. Excludes
/// deleted-affine and worklist sequences. `LOGOS_NARROW_TRACE` reports what
/// narrowed and why.
fn narrow_seqs<'a>(
    body: &'a [Stmt<'a>],
    de_rc: &std::collections::HashSet<Symbol>,
    affine: &HashMap<Symbol, super::affine_array::AffineArrayInfo>,
    worklists: &HashMap<Symbol, super::worklist::WorklistInfo>,
    interner: &Interner,
) -> HashMap<Symbol, super::narrow::NarrowInfo> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Narrow) {
        return HashMap::new();
    }
    let mut n = super::narrow::detect_narrowable(body, de_rc, interner);
    n.retain(|sym, _| {
        // A deleted affine array has no plain `Vec` to narrow — Affine preempts Narrow.
        if affine.contains_key(sym) {
            crate::optimize::mark_preempted(
                crate::optimization::Opt::Affine,
                crate::optimization::Opt::Narrow,
            );
            return false;
        }
        !worklists.contains_key(sym)
    });
    if !n.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::Narrow);
    }
    if std::env::var_os("LOGOS_NARROW_TRACE").is_some() {
        for (sym, info) in &n {
            eprintln!(
                "LOGOS_NARROW: `{}` → Vec<i32>{}",
                interner.resolve(*sym),
                if info.guards.is_empty() {
                    " (static range)".to_string()
                } else {
                    format!(" (guards: {})", info.guards.join(" && "))
                }
            );
        }
    }
    n
}

/// `Opt::BitPack` (on by default): the `Seq of Bool` locals of `body` held one bit per item
/// (`codegen::bits`), among the uniquely owned sequences no other representation has claimed.
fn bitpack_seqs(
    body: &[Stmt],
    de_rc: &std::collections::HashSet<Symbol>,
    claimed: &dyn Fn(Symbol) -> bool,
    interner: &Interner,
) -> std::collections::HashSet<Symbol> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::BitPack) {
        return std::collections::HashSet::new();
    }
    let mut packed = super::bits::detect_bitpacked(body, de_rc, interner);
    packed.retain(|s| !claimed(*s));
    if !packed.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::BitPack);
    }
    packed
}

/// The fixed length of every local that codegen emits as a `[T; N]` stack array in `body`, keyed by NAME
/// (the parser mints distinct symbols per occurrence; a call argument's symbol differs from the decl's).
/// Computed from the SAME detections codegen uses, so it never over-approximates (a size here ⟹ a real
/// `[T; N]`), which keeps the derived `&[T; N]` parameter type sound.
fn scope_array_sizes(
    body: &[Stmt],
    all_stmts: &[Stmt],
    is_main: bool,
    returns_vec: bool,
    this_ret: Option<&super::affine_array::ArrayReturnInfo>,
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    mut_borrow_params: &HashMap<Symbol, HashSet<usize>>,
    vec_return_fns: &HashSet<Symbol>,
    array_return_fns: &HashMap<Symbol, super::affine_array::ArrayReturnInfo>,
    interner: &Interner,
) -> HashMap<String, usize> {
    let de_rc = collect_de_rc_seqs(
        body,
        interner,
        borrow_params,
        mut_borrow_params,
        vec_return_fns,
        returns_vec,
    );
    let mut m: HashMap<String, usize> = HashMap::new();
    let mut put = |sym: Symbol, n: usize| {
        m.insert(interner.resolve(sym).to_string(), n);
    };
    if is_main {
        // Main scalarizes ONLY via `collect_scalarizable_seqs` (indexed/const O3 walk that disqualifies
        // call-arg appearances) + straight-line buffers (borrow-aware) + fn-return arrays. The scratch /
        // indexed / const-table passes are function-body-only, so including them here would over-report.
        for (v, info) in collect_scalarizable_seqs(body, interner) {
            put(v, info.len);
        }
    } else {
        for (v, info) in super::affine_array::detect_const_tables(
            body,
            all_stmts,
            &de_rc,
            borrow_params,
            interner,
        ) {
            put(v, info.values.len());
        }
        for (v, info) in
            super::affine_array::detect_scratch_buffers(body, &de_rc, borrow_params, interner)
        {
            put(v, info.len);
        }
        for (v, info) in super::affine_array::detect_indexed_buffers(body, &de_rc, interner) {
            put(v, info.len);
        }
    }
    for (v, (_, len)) in
        super::affine_array::detect_straightline_buffers(body, &de_rc, borrow_params, interner)
    {
        put(v, len);
    }
    for (v, ty) in super::affine_array::array_var_types(body, this_ret, array_return_fns) {
        if let Some(n) = ty
            .strip_prefix('[')
            .and_then(|s| s.rsplit_once("; "))
            .and_then(|(_, n)| n.strip_suffix(']'))
            .and_then(|n| n.parse::<usize>().ok())
        {
            put(v, n);
        }
    }
    m
}

/// Record the fixed array length of every read-borrow argument of every `Call` in `e` (recursing into
/// sub-expressions). `None` = a non-fixed argument at a borrow position (that param can't be `&[T; N]`).
fn record_call_arg_sizes(
    e: &Expr,
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    sizes: &HashMap<String, usize>,
    interner: &Interner,
    agg: &mut HashMap<(Symbol, usize), Option<usize>>,
) {
    match e {
        Expr::Call { function, args } => {
            if let Some(bset) = borrow_params.get(function) {
                for (i, a) in args.iter().enumerate() {
                    if bset.contains(&i) {
                        let val = if let Expr::Identifier(v) = a {
                            sizes.get(interner.resolve(*v)).copied()
                        } else {
                            None
                        };
                        agg.entry((*function, i))
                            .and_modify(|cur| {
                                if *cur != val {
                                    *cur = None;
                                }
                            })
                            .or_insert(val);
                    }
                }
            }
            for a in args {
                record_call_arg_sizes(a, borrow_params, sizes, interner, agg);
            }
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        } => {
            record_call_arg_sizes(left, borrow_params, sizes, interner, agg);
            record_call_arg_sizes(right, borrow_params, sizes, interner, agg);
        }
        Expr::Index { collection, index } => {
            record_call_arg_sizes(collection, borrow_params, sizes, interner, agg);
            record_call_arg_sizes(index, borrow_params, sizes, interner, agg);
        }
        Expr::Not { operand } => {
            record_call_arg_sizes(operand, borrow_params, sizes, interner, agg)
        }
        Expr::Length { collection }
        | Expr::Copy { expr: collection }
        | Expr::Give { value: collection }
        | Expr::OptionSome { value: collection }
        | Expr::FieldAccess {
            object: collection, ..
        } => record_call_arg_sizes(collection, borrow_params, sizes, interner, agg),
        Expr::CallExpr { callee, args } => {
            record_call_arg_sizes(callee, borrow_params, sizes, interner, agg);
            for a in args {
                record_call_arg_sizes(a, borrow_params, sizes, interner, agg);
            }
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for it in items {
                record_call_arg_sizes(it, borrow_params, sizes, interner, agg);
            }
        }
        _ => {}
    }
}

fn walk_call_sizes<'a>(
    scope: &[Stmt<'a>],
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    sizes: &HashMap<String, usize>,
    interner: &Interner,
    agg: &mut HashMap<(Symbol, usize), Option<usize>>,
) {
    for s in scope {
        super::worklist::for_each_stmt_expr(s, &mut |e| {
            record_call_arg_sizes(e, borrow_params, sizes, interner, agg)
        });
        match s {
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                walk_call_sizes(then_block, borrow_params, sizes, interner, agg);
                if let Some(eb) = else_block {
                    walk_call_sizes(eb, borrow_params, sizes, interner, agg);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                walk_call_sizes(body, borrow_params, sizes, interner, agg)
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    walk_call_sizes(arm.body, borrow_params, sizes, interner, agg);
                }
            }
            _ => {}
        }
    }
}

/// Fixed-array borrow-parameter propagation: `(fn, param_idx) → N` when EVERY call site passes a fixed
/// `[T; N]` array of the same N at that read-borrow position. The parameter then becomes `&[T; N]`, so
/// LLVM elides the constant-index bounds checks. Value-safe: `&arr` coerces to either form.
fn fixed_array_params(
    stmts: &[Stmt],
    borrow_params: &HashMap<Symbol, HashSet<usize>>,
    mut_borrow_params: &HashMap<Symbol, HashSet<usize>>,
    vec_return_fns: &HashSet<Symbol>,
    array_return_fns: &HashMap<Symbol, super::affine_array::ArrayReturnInfo>,
    interner: &Interner,
) -> HashMap<(Symbol, usize), usize> {
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::Scalarize) {
        return HashMap::new();
    }
    let mut agg: HashMap<(Symbol, usize), Option<usize>> = HashMap::new();
    let main_sizes = scope_array_sizes(
        stmts,
        stmts,
        true,
        false,
        None,
        borrow_params,
        mut_borrow_params,
        vec_return_fns,
        array_return_fns,
        interner,
    );
    walk_call_sizes(stmts, borrow_params, &main_sizes, interner, &mut agg);
    for s in stmts {
        if let Stmt::FunctionDef {
            name,
            body,
            is_native: false,
            ..
        } = s
        {
            let rv = vec_return_fns.contains(name);
            let sizes = scope_array_sizes(
                body,
                stmts,
                false,
                rv,
                array_return_fns.get(name),
                borrow_params,
                mut_borrow_params,
                vec_return_fns,
                array_return_fns,
                interner,
            );
            walk_call_sizes(body, borrow_params, &sizes, interner, &mut agg);
        }
    }
    agg.into_iter()
        .filter_map(|(k, v)| v.map(|n| (k, n)))
        .collect()
}

pub fn codegen_program(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    policies: &PolicyRegistry,
    interner: &Interner,
    type_env: &crate::analysis::types::TypeEnv,
    cfg: &OptimizationConfig,
) -> String {
    codegen_program_with_proven(
        stmts, registry, policies, interner, type_env, cfg, "proven", None,
    )
}

/// Like [`codegen_program`], but bundles an extracted math/logic module into the
/// output. `proven` is the body of a Rust module — functions, `check_*` property
/// fns, a `World`/`holds` model-checker — produced by the Forge's extraction with
/// NO `fn main`. It is emitted as `pub mod <module_name> { … }` right after the
/// prelude (before `user_types`), followed by `use <module_name>::*;` so a bare
/// call in the imperative program below — e.g. `double(21)` — resolves into it.
/// Naming the module (rather than dumping items at crate root) keeps multiple
/// proven modules reachable and avoids polluting the imperative namespace. When
/// `proven` is `None`/blank the output is byte-identical to [`codegen_program`].
pub fn codegen_program_with_proven(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    policies: &PolicyRegistry,
    interner: &Interner,
    type_env: &crate::analysis::types::TypeEnv,
    cfg: &OptimizationConfig,
    module_name: &str,
    proven: Option<&str>,
) -> String {
    codegen_program_inner(
        stmts,
        registry,
        policies,
        interner,
        type_env,
        cfg,
        module_name,
        proven,
        None,
    )
    .0
}

/// Like [`codegen_program`], but also builds the rustc→LOGOS [`SourceMap`]:
/// every generated line maps to the top-level statement that produced it
/// (`stmt_spans` is the parser's side-table, 1:1 with `stmts`; zero-width
/// spans mark prelude statements and are skipped), and variable origins carry
/// ownership roles for the diagnostic bridge. This is the flycheck substrate.
pub fn codegen_program_mapped(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    policies: &PolicyRegistry,
    interner: &Interner,
    type_env: &crate::analysis::types::TypeEnv,
    cfg: &OptimizationConfig,
    stmt_spans: &[LogosSpan],
    logos_source: &str,
) -> (String, SourceMap) {
    let (code, map) = codegen_program_inner(
        stmts,
        registry,
        policies,
        interner,
        type_env,
        cfg,
        "proven",
        None,
        Some((stmt_spans, logos_source)),
    );
    (code, map.expect("mapping was requested"))
}

#[allow(clippy::too_many_arguments)]
/// Emit the panic hook that makes a LOGOS runtime error look the same on the AOT tier as on every
/// other one: the message alone on stderr, then exit status 1.
///
/// Without it the AOT binary reported a runtime error as a Rust panic — the default hook's
/// `thread '<unnamed>' panicked at …/ops.rs:168` header plus a backtrace note, and exit 101 — while
/// the tree-walker and the VM printed `Division by zero` and exited 1. Same program, same input,
/// two different observable failures depending on how it was run. The runtime already carries the
/// right words; only the presentation was wrong.
fn emit_logos_panic_hook(output: &mut String, exit_immediately: bool) {
    // ★ FULLY QUALIFIED, because `use user_types::*;` sits above this and a LOGOS record may
    // legitimately be called `Box`. A shipping box, a text box, a box of parts — an ordinary
    // domain noun that shadowed `std::boxed::Box` and made the generated program fail to COMPILE,
    // with an error pointing at scaffolding the author never wrote. `String` has the same
    // problem for the same reason; both are spelled out in full wherever the compiler emits them.
    writeln!(
        output,
        "    std::panic::set_hook(::std::boxed::Box::new(|_logos_info| {{"
    )
    .unwrap();
    writeln!(output, "        logicaffeine_system::io::flush_stdout();").unwrap();
    writeln!(output, "        let _logos_msg = _logos_info.payload().downcast_ref::<&str>().map(|s| s.to_string())").unwrap();
    writeln!(
        output,
        "            .or_else(|| _logos_info.payload().downcast_ref::<String>().cloned())"
    )
    .unwrap();
    writeln!(
        output,
        "            .unwrap_or_else(|| \"LOGOS runtime error\".to_string());"
    )
    .unwrap();
    writeln!(output, "        eprintln!(\"{{}}\", _logos_msg);").unwrap();
    // A failure ends the program, under either panic strategy, unless a runtime boundary catches it: a served
    // request answers `500` and the server keeps serving. Replay tasks always unwind into the scheduler, which lets
    // their peers finish and then returns the failed run to main.
    if exit_immediately {
        writeln!(
            output,
            "        if !logicaffeine_system::failure::is_caught() {{ std::process::exit(1); }}"
        )
        .unwrap();
    }
    writeln!(output, "    }}));").unwrap();
}

fn codegen_program_inner(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    policies: &PolicyRegistry,
    interner: &Interner,
    type_env: &crate::analysis::types::TypeEnv,
    cfg: &OptimizationConfig,
    module_name: &str,
    proven: Option<&str>,
    mapping: Option<(&[LogosSpan], &str)>,
) -> (String, Option<SourceMap>) {
    // A nest the dependence test lets run in tile order is versioned here, once, for every entry to the emitter
    // (`optimize::version`): its clone lives in these arenas and what its guard proves in this facts scope, both
    // for exactly as long as the emission.
    let (exprs, stmt_arena, types) = (
        crate::arena::Arena::new(),
        crate::arena::Arena::new(),
        crate::arena::Arena::new(),
    );
    let _version_facts = crate::optimize::facts::scope();
    let arenas = crate::ast_map::Arenas {
        exprs: &exprs,
        stmts: &stmt_arena,
        types: &types,
    };
    let versioned =
        crate::optimize::version::version_stmts(stmts.to_vec(), arenas, type_env, interner, cfg);
    // A lent element or field becomes a binding the call borrows, stored back after it (`optimize::sequence`); its
    // fresh names live in this emission's own copy of the interner.
    crate::analysis::callgraph::install_lending_parameters(&versioned);
    let mut own = None;
    let versioned = if crate::optimize::sequence::lends_a_place(&versioned, interner) {
        crate::optimize::sequence::lower_lent_places(versioned, arenas, own.insert(interner.clone()))
    } else {
        versioned
    };
    // Where a stored `nothing` reaches, on exactly what this emission lowers (`absent`).
    let absence = super::absent::of_program(&versioned, registry, own.as_ref().unwrap_or(interner));
    if absence.is_some() {
        super::absent::intern_operands(own.get_or_insert_with(|| interner.clone()));
    }
    let interner = own.as_ref().unwrap_or(interner);
    let stmts: &[Stmt] = &versioned;
    let _absence = super::absent::begin_program(absence, stmts, interner);
    // How each `Repeat` may iterate: the shared optimizer's proofs, on exactly what this emission lowers.
    crate::optimize::iteration::prove_iteration_facts(stmts, interner);
    // Mode B replays the interpreter's scheduler, so it lowers exactly the programs the interpreter
    // runs on one; any other program compiles as it does in Mode A.
    let replay = super::deterministic_replay_enabled() && crate::concurrency::uses_scheduler(stmts);
    super::with_replay_mode(replay, || {
        codegen_program_body(
            stmts,
            registry,
            policies,
            interner,
            type_env,
            cfg,
            module_name,
            proven,
            mapping,
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn codegen_program_body(
    stmts: &[Stmt],
    registry: &TypeRegistry,
    policies: &PolicyRegistry,
    interner: &Interner,
    type_env: &crate::analysis::types::TypeEnv,
    cfg: &OptimizationConfig,
    module_name: &str,
    proven: Option<&str>,
    mapping: Option<(&[LogosSpan], &str)>,
) -> (String, Option<SourceMap>) {
    let _control = super::control::function();
    crate::optimize::set_active_config(*cfg);
    // ★ WHICH REPRESENTATION EACH `Int` SLOT GETS — decided ONCE, here, before a single line is
    // emitted. LOGOS has one integer type; this tier has two machine forms of it, and a struct
    // field typed from one answer while the value filling it was typed from another is a program
    // rustc rejects on a type with nothing in it that a LOGOS author wrote. The facts used to be
    // computed twice from different seeds, and the mutual recursion between fields, returns and
    // arguments was run twice rather than to a fixpoint. See `bigint_promote::promotion_facts`.
    super::expr::begin_program_emission();
    let _tower = super::tower::begin_program(stmts, interner);
    // The program's own types, which of them show, and each record's layout — read by every field
    // type the emitter writes, starting with the type definitions themselves.
    super::types::set_program_types(stmts, registry, interner);
    let route_refusals: Vec<(Symbol, Symbol, String)> =
        logicaffeine_language::parser::lgx_lower::route_refusals(stmts, interner, &|sym| {
            matches!(
                registry.get(sym),
                Some(crate::analysis::registry::TypeDef::Struct { .. })
            )
        })
        .into_iter()
        .map(|(method, path, why)| (method, path, why.message))
        .collect();
    super::expr::set_route_refusals(&route_refusals);
    super::stmt::set_serve_facts(stmts, interner);
    let promo = super::bigint_promote::promotion_facts(stmts, interner);
    let mut output = String::new();
    let replay = requires_async(stmts) && super::deterministic_replay_enabled();
    if replay || serves(stmts) {
        output.push_str(super::UNWIND_PROFILE_HEADER);
    }
    // (rust line, LOGOS span) records; materialized into the SourceMap at the
    // end. `lines.of(&output)` before/after an emission brackets its lines.
    let mut line_records: Vec<(u32, LogosSpan)> = Vec::new();
    let mut lines = LineCount::default();

    // Prelude
    // Use extracted crates instead of logos_core
    writeln!(output, "#[allow(unused_imports)]").unwrap();
    writeln!(output, "use std::fmt::Write as _;").unwrap();
    writeln!(output, "use logicaffeine_data::*;").unwrap();
    writeln!(output, "use logicaffeine_system::*;\n").unwrap();
    // Bit-count intrinsics — each emitted ONLY when the program calls it (so
    // unrelated codegen is byte-identical). Free functions so every codegen path
    // renders the call plainly; two's-complement faithful, matching the
    // tree-walker/VM builtins (optimize::popcount_leaf inserts them).
    if program_calls_named(stmts, interner, "count_ones") {
        writeln!(
            output,
            "#[inline(always)]\nfn count_ones(x: i64) -> i64 {{ (x as u64).count_ones() as i64 }}\n"
        )
        .unwrap();
    }
    if program_calls_named(stmts, interner, "leading_zeros") {
        writeln!(
            output,
            "#[inline(always)]\nfn leading_zeros(x: i64) -> i64 {{ (x as u64).leading_zeros() as i64 }}\n"
        )
        .unwrap();
    }
    if program_calls_named(stmts, interner, "trailing_zeros") {
        writeln!(
            output,
            "#[inline(always)]\nfn trailing_zeros(x: i64) -> i64 {{ (x as u64).trailing_zeros() as i64 }}\n"
        )
        .unwrap();
    }
    // SIMD substring-count kernel — emitted ONLY when the program contains a
    // recognized naive-search nest (unrelated codegen stays byte-identical). The
    // recognizer (peephole::try_emit_naive_search) lowers the nest to a call
    // into this kernel; the generated binary cannot link the compiler, so the
    // kernel travels with it, like the emitted `fn args()` wrapper.
    if super::peephole::stmts_contain_naive_search(stmts, interner) {
        writeln!(output, "{}", super::strsearch::RUNTIME_SRC).unwrap();
    }

    // FFI: Emit wasm_bindgen preamble if any function is exported for WASM
    if has_wasm_exports(stmts, interner) {
        writeln!(output, "use wasm_bindgen::prelude::*;\n").unwrap();
    }

    // FFI: Emit CStr/CString imports if any C export uses Text types
    if has_c_exports_with_text(stmts, interner) {
        writeln!(output, "use std::ffi::{{CStr, CString}};\n").unwrap();
    }

    // Universal ABI: Emit LogosStatus runtime preamble if any C exports exist
    let c_exports_exist = has_c_exports(stmts, interner);
    if c_exports_exist {
        output.push_str(&codegen_logos_runtime_preamble());
    }

    // Bundled proven module: the extracted math/logic objects (functions, `check_*`
    // property fns, `World`/`holds`) the imperative program calls into. Named so
    // multiple proven modules stay reachable; `use super::*;` gives it the crate
    // prelude and `use <name>::*;` brings its public items into imperative scope so
    // a bare `double(21)` resolves into it. Emitted only when the mixed compile
    // supplies it — otherwise the output is byte-identical to the bare imperative path.
    if let Some(proven) = proven {
        if !proven.trim().is_empty() {
            writeln!(output, "pub mod {module_name} {{").unwrap();
            writeln!(output, "    #![allow(dead_code, unused, non_snake_case)]").unwrap();
            writeln!(output, "    use super::*;").unwrap();
            output.push_str(proven);
            if !proven.ends_with('\n') {
                writeln!(output).unwrap();
            }
            writeln!(output, "}}").unwrap();
            writeln!(output, "use {module_name}::*;\n").unwrap();
        }
    }

    // Phase 49: Collect CRDT register fields for special SetField handling
    // LWW fields need timestamp, MV fields don't
    let (lww_fields, mv_fields) = collect_crdt_register_fields(registry, interner);

    // Phase 54: Collect async functions for Launch codegen
    let async_functions = collect_async_functions(stmts);
    let call_graph = crate::analysis::callgraph::CallGraph::build(stmts, interner);
    super::expr::set_recursive_async(
        async_functions.iter().copied().filter(|f| call_graph.is_recursive(*f)).collect(),
    );

    // Purity analysis for memoization
    let pure_functions = collect_pure_functions(stmts, interner);

    // Phase 54: Collect pipe declarations (variables with _tx/_rx suffixes)
    let main_pipe_vars = collect_pipe_vars(stmts);

    // Phase 102: Collect boxed fields for recursive enum handling
    let boxed_fields = collect_boxed_fields(registry, interner);

    // Collect value-type struct names used in C exports (need #[repr(C)])
    let c_abi_value_structs: HashSet<Symbol> = if c_exports_exist {
        collect_c_export_value_type_structs(stmts, interner, registry)
    } else {
        HashSet::new()
    };

    // Collect reference-type struct names used in C exports (need serde derives for from_json/to_json)
    let c_abi_ref_structs: HashSet<Symbol> = if c_exports_exist {
        collect_c_export_ref_structs(stmts, interner, registry)
    } else {
        HashSet::new()
    };

    // Collect user-defined structs from registry (Phase 34: generics, Phase 47: is_portable, Phase 49: is_shared)
    let mut structs: Vec<_> = registry
        .iter_types()
        .filter(|(sym, def)| super::types::emits_user_type(**sym, def))
        .filter_map(|(name, def)| match def {
            TypeDef::Struct {
                fields,
                generics,
                is_portable,
                is_shared,
            } => Some((
                *name,
                fields.clone(),
                generics.clone(),
                *is_portable,
                *is_shared,
            )),
            _ => None,
        })
        .collect();
    // DETERMINISM: `iter_types` walks a HashMap (random per-run order). Sort by
    // name so the emitted Rust is byte-identical across recompiles (Rust resolves
    // top-level types order-independently, so this can't affect correctness).
    structs.sort_by(|a, b| interner.resolve(a.0).cmp(interner.resolve(b.0)));

    // Phase 33/34: Collect user-defined enums from registry (generics, Phase 47: is_portable, Phase 49: is_shared)
    let mut enums: Vec<_> = registry
        .iter_types()
        .filter(|(sym, def)| super::types::emits_user_type(**sym, def))
        .filter_map(|(name, def)| match def {
            TypeDef::Enum {
                variants,
                generics,
                is_portable,
                is_shared,
            } => Some((
                *name,
                variants.clone(),
                generics.clone(),
                *is_portable,
                *is_shared,
            )),
            _ => None,
        })
        .collect();
    enums.sort_by(|a, b| interner.resolve(a.0).cmp(interner.resolve(b.0)));

    // Emit struct and enum definitions in user_types module if any exist
    if !structs.is_empty() || !enums.is_empty() {
        writeln!(output, "pub mod user_types {{").unwrap();
        writeln!(output, "    use super::*;\n").unwrap();

        // An imported type reaches the registry under BOTH its bare simple name
        // (from the inline splice) and its canonical `Alias::Name` (from import
        // discovery). Emit the canonical struct ONCE and make the bare spelling a
        // `type` alias to it, rather than a dead duplicate struct — Rust resolves
        // struct construction through a type alias, so `a new Point` still works.
        // A struct field fed a bignum-promoted value (json's `JNum { n: m }`) must be stored as
        // `LogosInt`, not `i64`. Record which `(struct, field)` pairs BEFORE emitting struct defs —
        // the def below reads this thread-local to type such a field. `bigint_fns` is a pure fn of
        // `stmts` (recomputed here so the record is available this early; the later copy is identical).
        super::expr::set_bigint_struct_fields(&promo.bigint_fields, registry);

        let twins = registry.namespaced_twins(interner);

        for (name, fields, generics, is_portable, is_shared) in &structs {
            if let Some(twin) = twins.get(name) {
                emit_bare_type_alias(
                    &mut output,
                    interner.resolve(*name),
                    interner.resolve(*twin),
                    generics,
                    interner,
                );
                continue;
            }
            output.push_str(&codegen_struct_def(
                *name,
                fields,
                generics,
                *is_portable,
                *is_shared,
                interner,
                4,
                &c_abi_value_structs,
                &c_abi_ref_structs,
            ));
        }

        // Same single-emit for imported enums: canonical `Alias__Enum` once, with
        // a `type Bare = Alias__Enum;` alias for the bare spelling.
        for (name, variants, generics, is_portable, is_shared) in &enums {
            if let Some(twin) = twins.get(name) {
                emit_bare_type_alias(
                    &mut output,
                    interner.resolve(*name),
                    interner.resolve(*twin),
                    generics,
                    interner,
                );
                continue;
            }
            output.push_str(&codegen_enum_def(
                *name,
                variants,
                generics,
                *is_portable,
                *is_shared,
                interner,
                4,
            ));
        }

        writeln!(output, "}}\n").unwrap();
        writeln!(output, "use user_types::*;\n").unwrap();
    }

    // Phase 50: Generate policy impl blocks with predicate and capability methods
    output.push_str(&codegen_policy_impls(policies, interner));

    // Mutual TCO: Detect pairs of mutually tail-calling functions
    let mutual_tce_pairs = detect_mutual_tce_pairs(stmts, interner);
    let mut mutual_tce_members: HashSet<Symbol> = HashSet::new();
    for (a, b) in &mutual_tce_pairs {
        mutual_tce_members.insert(*a);
        mutual_tce_members.insert(*b);
    }
    let mut mutual_tce_emitted: HashSet<Symbol> = HashSet::new();

    // Pre-pass: Build borrow_params_map — identifies which function params
    // can be borrowed as &[T] instead of owned Vec<T>.
    // Uses whole-program transitive readonly analysis (ReadonlyParams) instead of
    // local-body-only detection so that params passed to mutating callees are
    // correctly excluded from borrow optimization.
    let callgraph = CallGraph::build(stmts, interner);
    let readonly_params = ReadonlyParams::analyze(stmts, &callgraph, type_env);
    let give_args = collect_give_args(stmts);

    let mut borrow_params_map: HashMap<Symbol, HashSet<usize>> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            is_native,
            is_exported,
            opt_flags,
            ..
        } = stmt
        {
            // Native wrappers pass each Seq param straight to the underlying kernel (which takes
            // `&[i64]`), reading but never mutating it — so every Seq param is readonly-borrowed.
            // This makes the whole native call path zero-copy: a caller's `&[i64]` Seq param flows
            // through the wrapper to the kernel with no `LogosSeq` materialization.
            if *is_native {
                if crate::optimize::active_config()
                    .merged(opt_flags)
                    .is_on(Opt::Borrow)
                {
                    let indices: HashSet<usize> = params
                        .iter()
                        .enumerate()
                        .filter(|(_, (_, param_type))| is_vec_type_expr(param_type, interner))
                        .map(|(i, _)| i)
                        .collect();
                    if !indices.is_empty() {
                        borrow_params_map.insert(*name, indices);
                    }
                }
                continue;
            }
            // Skip exported, TCE, accumulator, and mutual TCE functions
            if *is_exported || mutual_tce_members.contains(name) {
                continue;
            }
            // Respect ## No Borrow / ## No Optimize annotations
            if !crate::optimize::active_config()
                .merged(opt_flags)
                .is_on(Opt::Borrow)
            {
                continue;
            }
            // Only DIRECT tail recursion conflicts with a borrowed param (TCE
            // reassigns it). A `Set/Let x = self(args); Return x` pair can keep
            // the borrow — pair-TCE yields to it in the `is_tce` gate below.
            if is_tail_recursive(*name, body) {
                continue;
            }
            if detect_accumulator_pattern(*name, body).is_some() {
                continue;
            }
            // Skip functions with escape blocks — raw Rust code may assume specific param types
            if body_contains_escape(body) {
                continue;
            }
            let indices: HashSet<usize> = params
                .iter()
                .enumerate()
                .filter(|(_, (sym, param_type))| {
                    readonly_params.is_readonly(*name, *sym)
                        && (is_vec_type_expr(param_type, interner)
                            || super::marshal::is_text_type(param_type, interner))
                })
                .map(|(i, _)| i)
                .collect();
            if !indices.is_empty() {
                let filtered: HashSet<usize> = match give_args.get(name) {
                    Some(given) => indices.difference(given).copied().collect(),
                    None => indices,
                };
                if !filtered.is_empty() {
                    borrow_params_map.insert(*name, filtered);
                    crate::optimize::mark_fired(Opt::Borrow);
                }
            }
        }
    }

    // Enum params that are only ever inspected or lent on are passed as `&T` (`codegen::lent`), under the same
    // gates as the sequence borrow above.
    let enum_eligible = |stmt: &Stmt| match stmt {
        Stmt::FunctionDef {
            name,
            body,
            is_native,
            is_exported,
            opt_flags,
            ..
        } => {
            !*is_native
                && !*is_exported
                && !mutual_tce_members.contains(name)
                && crate::optimize::active_config()
                    .merged(opt_flags)
                    .is_on(Opt::Borrow)
                && !is_tail_recursive(*name, body)
                && detect_accumulator_pattern(*name, body).is_none()
                && !body_contains_escape(body)
        }
        _ => false,
    };
    for (name, slots) in super::lent::enum_borrow_params(stmts, registry, interner, &enum_eligible)
    {
        borrow_params_map.entry(name).or_default().extend(slots);
        crate::optimize::mark_fired(Opt::Borrow);
    }

    // Mutable borrow analysis: detect Seq params that are only mutated via SetIndex
    // (element-only, no Push/Pop) and returned. These get &mut [T] instead of owned Vec<T>.
    let mutable_borrow_params = MutableBorrowParams::analyze(stmts, &callgraph, type_env);

    let mut mut_borrow_params_map: HashMap<Symbol, HashSet<usize>> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            body,
            is_native,
            is_exported,
            opt_flags,
            ..
        } = stmt
        {
            if *is_native || *is_exported || mutual_tce_members.contains(name) {
                continue;
            }
            // Respect ## No Borrow / ## No Optimize annotations
            if !crate::optimize::active_config()
                .merged(opt_flags)
                .is_on(Opt::Borrow)
            {
                continue;
            }
            if is_tail_recursive(*name, body) || detect_accumulator_pattern(*name, body).is_some() {
                continue;
            }
            if body_contains_escape(body) {
                continue;
            }
            // Skip if already has readonly borrow params (readonly takes precedence)
            let readonly_indices = borrow_params_map.get(name).cloned().unwrap_or_default();
            let indices: HashSet<usize> = params
                .iter()
                .enumerate()
                .filter(|(i, (sym, param_type))| {
                    // Under value semantics this in-place lowering stays EXACT:
                    // every call site is the consuming `Set x to f(x, ...)` shape
                    // (enforced by `collect_incompatible_mut_borrow_callsites`)
                    // and the call site `cow()`s the handle first, so the
                    // callee's element writes land on a buffer no other live
                    // handle can observe.
                    mutable_borrow_params.is_mutable_borrow(*name, *sym)
                        && !readonly_indices.contains(i)
                        && is_vec_type_expr(param_type, interner)
                })
                .map(|(i, _)| i)
                .collect();
            if !indices.is_empty() {
                mut_borrow_params_map.insert(*name, indices);
                crate::optimize::mark_fired(Opt::Borrow);
            }
        }
    }

    // `mutable` params, on both semantics: the caller's own value is lent for the call — a list or a map by
    // shared `&LogosSeq`/`&LogosMap` (its writes go through its own `RefCell`), anything else by `&mut` — so
    // every write the callee makes is the caller's. Distinct from the `&mut [T]` borrow opt (which has no
    // `.push()`).
    let mut value_mutable_params_map: HashMap<Symbol, HashSet<usize>> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            is_native: false,
            ..
        } = stmt
        {
            let idx: HashSet<usize> = params
                .iter()
                .enumerate()
                .filter(|(_, (_, ty))| matches!(ty, crate::ast::stmt::TypeExpr::Mutable { .. }))
                .map(|(i, _)| i)
                .collect();
            if !idx.is_empty() {
                value_mutable_params_map.insert(*name, idx);
            }
        }
    }
    crate::analysis::callgraph::install_lending_parameters(stmts);

    // Pass 2: what is live after every statement of every body (backward dataflow), installed for the whole
    // emission so a statement may move a value it uses for the last time (OPT-1C, `flow`).
    let _liveness = super::flow::install(LivenessResult::analyze(stmts).into_statement_table());

    // De-Rc Phase 4: functions whose every Return is a uniquely-owned fresh Seq
    // return `Vec<T>` instead of `LogosSeq<T>` — removing the per-call Rc
    // clone/borrow and unblocking de-Rc on the locals that capture the result
    // (`Set left to mergeSort(left)`). The mergesort allocation keystone.
    let mut vec_return_fns =
        collect_vec_return_fns(stmts, interner, &borrow_params_map, &mut_borrow_params_map);
    // Opt::TextSliceReturn — fns returning a contiguous sub-slice of a Text param lower to a borrowed
    // `&'a str` return (the trim family), when no caller lets the slice escape. The slice-builder shape
    // provably only READS the source param, so this pass DRIVES borrowing it: inject each slice fn's
    // source-param index into borrow_params_map so the signature takes `&'a str` and call sites pass
    // `&s` (no clone). Scoped to the recognized slice fns — no program-wide Text-borrow blast radius.
    let str_slice_return_fns =
        super::peephole::collect_str_slice_return_fns(stmts, interner, &borrow_params_map);
    // Opt::AsciiClassMask: pure Int->Bool classification predicates → their 256-bit membership masks.
    let ascii_masks = crate::optimize::ascii_class_masks(stmts, interner);
    for (f, &si) in &str_slice_return_fns {
        borrow_params_map.entry(*f).or_default().insert(si);
    }
    // Opt::ByteSliceReturn — fns returning a contiguous sub-slice of a `Seq of Int` param lower to a
    // borrowed `&'a [i64]` return (the bytes-trim family), same borrow-driving as TextSliceReturn.
    // Exclude them from vec-return so the borrowed-slice path (not an owned `Vec`) is authoritative.
    let byte_slice_return_fns =
        super::peephole::collect_byte_slice_return_fns(stmts, interner, &borrow_params_map);
    for (f, &si) in &byte_slice_return_fns {
        borrow_params_map.entry(*f).or_default().insert(si);
    }
    vec_return_fns.retain(|f| !byte_slice_return_fns.contains_key(f));
    // Both slice families return a BORROW, and a caller marshalling an argument at a borrow
    // position has to know that — otherwise it wraps an already-borrowed `&[T]` in `.borrow()`.
    super::expr::set_borrowed_return_fns(
        &str_slice_return_fns
            .keys()
            .chain(byte_slice_return_fns.keys())
            .copied()
            .collect(),
    );
    // Step 3b: functions returning a fixed-size buffer return `[T; N]` by value (a stack array). Disjoint
    // from vec-return (the array is the stricter, zero-heap representation) — exclude them from vec-return
    // so the array path is authoritative.
    let array_return_fns =
        super::affine_array::collect_array_return_fns(stmts, &borrow_params_map, interner);
    vec_return_fns.retain(|f| !array_return_fns.contains_key(f));
    // Fixed-array borrow-parameter propagation: a read-borrow param passed only fixed `[T; N]` arrays
    // becomes `&[T; N]` (LLVM elides its constant-index bounds checks).
    let fixed_array_param_map = fixed_array_params(
        stmts,
        &borrow_params_map,
        &mut_borrow_params_map,
        &vec_return_fns,
        &array_return_fns,
        interner,
    );

    // Build function return type map for variable type inference at call sites.
    // Phase 4: a return-type-de-Rc'd function returns `Vec<T>`, so callers infer
    // the result var as `Vec` (not `LogosSeq`) — keeping its uses Rc-free. Step 3b:
    // an array-return function's callers infer the result var as `[T; N]`.
    let mut fn_returns_map: HashMap<Symbol, String> = stmts
        .iter()
        .filter_map(|s| {
            if let Stmt::FunctionDef {
                name,
                return_type: Some(rt),
                ..
            } = s
            {
                let ty = codegen_type_expr(rt, interner);
                let ty = if let Some(info) = array_return_fns.get(name) {
                    format!("[{}; {}]", info.elem_ty, info.len)
                } else if vec_return_fns.contains(name) {
                    ty.replacen("LogosSeq<", "Vec<", 1)
                } else {
                    ty
                };
                Some((*name, ty))
            } else {
                None
            }
        })
        .collect();

    // Overflow-promoting return: the whole-program set of `Int`-returning functions whose value can
    // exceed i64 (a running product/power, or a call to another such function). Their signature
    // becomes `-> LogosInt`, and a caller's binding of the result is itself promotable. Record the
    // `|__bigint` sentinel so a call result classifies as Int (exact-arith path) yet stores LogosInt.
    // Selective higher-order-overflow analysis: a SCALAR exact-HOF (`apply(f, x)` — Int/Nat return,
    // a `fn`-typed param, no seq param) exactifies its fn-param + int-params + return to the promoting
    // `LogosInt`, so an overflowing closure body promotes to a bignum == the VM. Folded into
    // `bigint_fns` (its return becomes `-> LogosInt`) AND threaded to expr codegen for the call-site
    // half (exact closure args + integer-arg promotion). Seq-HOFs are excluded, so `map`/`fold`/
    // `reduceSeq` closures stay `i64` — no regression. Mirrors AST-C's `is_exact_hof`. The set is
    // call-site-refined (see `exact_hof_fns`): only HOFs receiving inline closures at every site
    // exactify, and a HOF whose every site is provably within `i64` keeps plain `i64` signatures.
    let exact_hofs = promo.exact_hofs.clone();
    let bigint_fns = promo.bigint_fns.clone();
    let bigint_fields = promo.bigint_fields.clone();
    let exact_result_fns = promo.exact_result_fns.clone();
    let _ = &bigint_fields;
    for f in &bigint_fns {
        fn_returns_map.insert(*f, "i64|__bigint".to_string());
    }
    // A function the exact extension reaches returns the compiled exact value, whatever its
    // declared type.
    for f in fn_returns_map.keys().copied().collect::<Vec<_>>() {
        if super::tower::returns(f) {
            fn_returns_map.insert(f, super::tower::VALUE.to_string());
        }
    }
    // Record the bignum-returning set for the expression codegen, so an INLINE call to such a
    // function is detected as a promoted `LogosInt` operand and routed through the exact helper
    // (a bare `LogosInt <op> LogosInt` has no operator impl — see `mentions_bigint_var`).
    // A program's OWN function shadows a stdlib name of the same spelling: install its declared
    // return types BEFORE anything is emitted, so a call's result type comes from the program's
    // declaration rather than math.lg's `-> Real` name list.
    let fn_returns_map = Rc::new(fn_returns_map);
    super::expr::set_fn_return_types(&fn_returns_map);
    // A function's calling roles are the same at every call site: one table for the program, read by every
    // function's codegen.
    super::set_fn_roles(&borrow_params_map, &mut_borrow_params_map, &value_mutable_params_map);
    // The SIMD-reduce lifter inlines liftable transcendentals from the program's function bodies, in Main and in
    // every function: one table for the program.
    let simd_fns = Rc::new(super::simd_reduce::build_fn_map(stmts));
    super::expr::set_bigint_returning_fns(&bigint_fns);
    super::expr::set_exact_hofs(&exact_hofs);
    super::instantiation::install(stmts, registry, interner);

    // Whole-program function → parameter Rust types, so a call passing an integer to an `f64`
    // (`Real`) parameter coerces with `as f64` at the emit — matching the interpreter/VM, which
    // coerce int→Real at the call boundary. Without it a Real-parameter function called with an
    // integer literal (`sin(0)`, `cConj(3, 4)`) fails to type-check on the AOT tier while passing
    // on tree-walker/VM, a cross-tier divergence. Native functions are excluded (their params bind
    // to hand-written host signatures, not this coercion path). An exact-HOF's parameter types are
    // EXACTIFIED (`i64` → `LogosInt`, inside `impl Fn(..)` too) so the call-site half promotes a
    // closure / integer argument to the exactified parameter.
    let fn_param_types_map: HashMap<Symbol, Vec<String>> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef {
                name,
                params,
                is_native: false,
                ..
            } => {
                let is_exact = exact_hofs.contains(name);
                Some((
                    *name,
                    params
                        .iter()
                        .map(|(_, ty)| {
                            let t = codegen_type_expr(ty, interner);
                            if is_exact {
                                exactify_rust_int_type(&t)
                            } else {
                                t
                            }
                        })
                        .collect(),
                ))
            }
            _ => None,
        })
        .collect();
    super::expr::set_fn_param_types(&fn_param_types_map);

    // O1 borrow hoisting: analyze the oracle ONCE on this exact statement
    // slice (loop alias snapshots are keyed by Stmt address; codegen walks
    // the same Stmts, so the pointers match). One forward pass with bounded
    // per-loop fixpoints; gated for very large programs and by LOGOS_HOIST=0.
    const MAX_HOIST_ORACLE_STMTS: usize = 5_000;
    let oracle: Option<std::rc::Rc<crate::optimize::OracleFacts>> =
        if super::hoist::hoisting_disabled() || stmts.len() > MAX_HOIST_ORACLE_STMTS {
            None
        } else {
            Some(std::rc::Rc::new(
                crate::optimize::oracle_analyze_with_entry_guards(stmts, interner),
            ))
        };

    // The range oracle: the whole program's range facts, computed on this exact
    // statement slice — the same one codegen walks, so expression addresses match
    // — or the borrow-hoist `oracle` above when that exists. It answers whether an
    // Int op stays in i64 (`oracle_proves_int_op_in_range`) and whether an access
    // names an item (`range_proves_index`), with what the emitted program's own
    // guards establish recorded in it (`mark_index_in_range`). Borrow-hoisting /
    // BCE / fast-div read the `ExprCtx` oracle. Installed for this compile and
    // removed when it ends, so a freed arena's reused addresses never meet it.
    let range_oracle: Option<std::rc::Rc<crate::optimize::OracleFacts>> =
        oracle.clone().or_else(|| {
            (stmts.len() <= MAX_HOIST_ORACLE_STMTS).then(|| {
                std::rc::Rc::new(crate::optimize::oracle_analyze_with_entry_guards(
                    stmts, interner,
                ))
            })
        });
    let _range_oracle_scope = super::expr::install_range_oracle(range_oracle);
    let _emission_facts = crate::optimize::facts::scope();

    let aot_compiled = crate::analysis::callgraph::aot_compiled_functions(stmts, interner);
    // Phase 32/38: Emit function definitions before main
    for (top_idx, stmt) in stmts.iter().enumerate() {
        let fn_emit_start = lines.of(&output) + 1;
        if let Stmt::FunctionDef {
            name,
            params,
            generics,
            body,
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        } = stmt
        {
            // ★ A SERVER BINARY CARRIES NO DOCUMENT CALLS. The browser half of a page's lowering
            // (`__render_`, `__act_`, `__mount_`, …) and every function that calls into it or into a
            // browser-only builtin need a live document, which this tier does not have; a server
            // render (`__serve_<C>`, `__html_<C>`) and the vdom.lg it draws with are ordinary code
            // and stay (`callgraph::aot_compiled_functions`).
            if !aot_compiled.contains(name) {
                continue;
            }
            if mutual_tce_members.contains(name) {
                // Part of a mutual pair — emit merged function when we see the first member
                if !mutual_tce_emitted.contains(name) {
                    // Find the pair this function belongs to
                    if let Some((a, b)) = mutual_tce_pairs
                        .iter()
                        .find(|(a, b)| *a == *name || *b == *name)
                    {
                        output.push_str(&codegen_mutual_tce_pair(
                            *a,
                            *b,
                            stmts,
                            interner,
                            &lww_fields,
                            &mv_fields,
                            &async_functions,
                            &boxed_fields,
                            registry,
                            type_env,
                        ));
                        mutual_tce_emitted.insert(*a);
                        mutual_tce_emitted.insert(*b);
                    }
                }
                // Skip individual emission — already emitted as part of merged pair
            } else {
                output.push_str(&codegen_function_def(
                    *name,
                    generics,
                    params,
                    body,
                    stmts,
                    return_type.as_ref().copied(),
                    *is_native,
                    *native_path,
                    *is_exported,
                    *export_target,
                    interner,
                    &lww_fields,
                    &mv_fields,
                    &async_functions,
                    &boxed_fields,
                    registry,
                    &pure_functions,
                    type_env,
                    &borrow_params_map,
                    &mut_borrow_params_map,
                    &value_mutable_params_map,
                    opt_flags,
                    &fn_returns_map,
                    &vec_return_fns,
                    &str_slice_return_fns,
                    &byte_slice_return_fns,
                    &array_return_fns,
                    &fixed_array_param_map,
                    oracle.as_ref(),
                    &bigint_fns,
                    &exact_hofs,
                    &exact_result_fns,
                    &ascii_masks,
                    &simd_fns,
                ));
            }
            if let Some((spans, _)) = mapping {
                record_emitted_lines(
                    &output,
                    &mut lines,
                    fn_emit_start,
                    spans.get(top_idx).copied(),
                    &mut line_records,
                );
            }
        }
    }

    // Universal ABI: Emit accessor/free functions for reference types in C exports
    if c_exports_exist {
        let ref_types = collect_c_export_reference_types(stmts, interner, registry);
        for ref_ty in &ref_types {
            output.push_str(&codegen_c_accessors(ref_ty, interner, registry));
        }
    }

    // Grand Challenge: Collect variables that need to be mutable
    let main_stmts: Vec<&Stmt> = stmts
        .iter()
        .filter(|s| !matches!(s, Stmt::FunctionDef { .. }))
        .collect();
    let mut main_mutable_vars = HashSet::new();
    for stmt in &main_stmts {
        collect_mutable_vars_stmt(stmt, &mut main_mutable_vars);
    }

    // OPT: Detect single-char text variables (emit u8 instead of String)
    let single_char_vars = collect_single_char_text_vars(stmts, interner);

    // Main function
    // Phase 51: Use async main when async operations are present
    if replay {
        // Mode B: the main task and every task it launches run on the replay executor, under a
        // current-thread host reactor for any real I/O a task awaits.
        writeln!(output, "fn main() {{").unwrap();
        emit_logos_panic_hook(&mut output, false);
        writeln!(
            output,
            "    let _logos_outcome = std::thread::Builder::new()"
        )
        .unwrap();
        writeln!(output, "        .stack_size(67_108_864)").unwrap();
        writeln!(output, "        .spawn(|| {{").unwrap();
        writeln!(
            output,
            "            tokio::runtime::Builder::new_current_thread()"
        )
        .unwrap();
        writeln!(output, "                .enable_all()").unwrap();
        writeln!(output, "                .build()").unwrap();
        writeln!(
            output,
            "                .expect(\"the LOGOS runtime starts\")"
        )
        .unwrap();
        writeln!(
            output,
            "                .block_on(logicaffeine_system::replay::run(_logos_main()))"
        )
        .unwrap();
        writeln!(output, "        }})").unwrap();
        writeln!(output, "        .unwrap().join();").unwrap();
        writeln!(output, "    logicaffeine_system::io::flush_stdout();").unwrap();
        writeln!(output, "    match _logos_outcome {{").unwrap();
        writeln!(output, "        Ok(Ok(())) => {{}}").unwrap();
        writeln!(output, "        Ok(Err(failure)) => {{").unwrap();
        writeln!(output, "            if failure == logicaffeine_system::replay::Failure::Deadlock {{ eprintln!(\"{{}}\", failure); }}").unwrap();
        writeln!(output, "            std::process::exit(1);").unwrap();
        writeln!(output, "        }}").unwrap();
        writeln!(output, "        Err(_) => std::process::exit(1),").unwrap();
        writeln!(output, "    }}").unwrap();
        writeln!(output, "}}").unwrap();
        writeln!(output, "async fn _logos_main() {{").unwrap();
    } else if requires_async(stmts) {
        writeln!(output, "#[tokio::main]").unwrap();
        writeln!(output, "async fn main() {{").unwrap();
        emit_logos_panic_hook(&mut output, true);
    } else {
        // `Main` runs on a deep stack of its own, switched to on the thread that started the program: a waiting
        // second thread would put every `malloc` and `free` on the C library's locked path. A runtime failure ends
        // the program in the hook, one message and exit status 1, as on every tier.
        writeln!(output, "fn main() {{").unwrap();
        emit_logos_panic_hook(&mut output, true);
        writeln!(output, "    logicaffeine_system::stack::run_main(_logos_main);").unwrap();
        writeln!(output, "}}").unwrap();
        writeln!(output, "fn _logos_main() {{").unwrap();
    }
    // Phase 53: Inject VFS when file operations or persistence is used
    if requires_vfs(stmts) {
        writeln!(output, "    let vfs: std::sync::Arc<dyn logicaffeine_system::fs::Vfs + Send + Sync> = std::sync::Arc::from(logicaffeine_system::fs::get_platform_vfs());").unwrap();
    }
    let mut main_ctx = RefinementContext::from_type_env(type_env);
    // Opt::StrengthReducePow2: the Main body's always-non-negative variables (walking `stmts` skips
    // FunctionDef bodies, so this is exactly Main's set). Per-function registration does not cover Main.
    super::expr::set_nonneg_vars(super::expr::collect_always_nonneg(stmts));
    main_ctx.set_simd_fns(Rc::clone(&simd_fns));
    // O1 borrow hoisting: seed the oracle (pointer-keyed loop alias
    // snapshots must match the Stmts codegen walks).
    if let Some(o) = &oracle {
        main_ctx.set_oracle(o.clone());
    }
    // Local Vec optimization: detect which collection vars escape main
    let main_escaping = collect_escaping_collection_vars(stmts);
    main_ctx.set_escaping_vars(main_escaping);
    // Overflow-promoting Int bindings: which integer vars must store `LogosInt`.
    // A LOCAL and a FIELD must be decided from the SAME answer to "can this exceed i64", or a
    // decoder's local is a `LogosInt` while the record it fills holds an `i64`. `promo_fns` is the
    // fixpoint's own seed — the set `bigint_struct_fields` was computed against.
    let promotable_vars = crate::codegen::bigint_promote::promotable_int_vars(
        stmts,
        &promo.promo_fns,
        &promo.bigint_fields,
        &HashSet::new(),
    );
    // Opt::WideAccumulator: Main's bounded counting accumulators safe to widen to i128. A qualifying
    // loop bound must be i64, so pass the promotable/bignum set to reject a bignum-bounded loop.
    super::expr::set_wide_acc(crate::codegen::stmt::collect_wide_accumulators(
        stmts,
        &promotable_vars,
        &pure_functions,
    ));
    main_ctx.set_promotable_candidates(promotable_vars);
    let main_joins =
        super::rational::joins(
            stmts,
            &HashSet::new(),
            super::rational::Typing { interner, variable_types: main_ctx.get_variable_types(), registry },
        );
    main_ctx.set_rational_joins(main_joins);
    // OPT: Register single-char text vars as u8 type for optimized codegen
    for sym in &single_char_vars {
        main_ctx.register_variable_type(*sym, "__single_char_u8".to_string());
    }
    // O3: scalarize fixed-size, non-escaping Seqs to `[T; N]` arrays (Main only).
    let scalarizable_seqs = collect_scalarizable_seqs(stmts, interner);
    // AoS interleaving: fuse co-indexed same-type groups (round-robin pushes)
    // into one `[[T; W]; N]` backing array reusing the first member's symbol, so
    // per-entity fields are memory-adjacent (C's struct-array layout) and LLVM
    // packs them instead of gathering separate arrays with shuffles. Members are
    // registered as AoS columns INSTEAD of plain `[T; N]` scalarized arrays.
    let aos_groups = collect_interleaved_groups(stmts, &scalarizable_seqs, interner);
    let aos_names = RustNames::new(interner);
    let mut aos_member_syms: HashSet<Symbol> = HashSet::new();
    for group in &aos_groups {
        let width = group.members.len();
        // The fused backing array reuses the first member's emitted identifier.
        // RustNames is a deterministic resolve+sanitize, so this matches the name
        // every access site produces for that symbol.
        let backing = aos_names.ident(group.members[0]);
        for (col, m) in group.members.iter().enumerate() {
            main_ctx.register_variable_type(
                *m,
                format!(
                    "__aos:{}:{}:{}:{}:{}",
                    backing, col, width, group.len, group.elem_ty
                ),
            );
            main_ctx.init_array_fill(*m);
            aos_member_syms.insert(*m);
        }
    }
    for (sym, info) in &scalarizable_seqs {
        if aos_member_syms.contains(sym) {
            continue; // handled as an AoS column above
        }
        main_ctx.register_variable_type(*sym, format!("[{}; {}]", info.elem_ty, info.len));
        main_ctx.init_array_fill(*sym);
    }
    // Step 3b: register the `[T; N]` type for Main's result variables bound to an array-return fn (the
    // per-function registration in `codegen_function_def` does not cover the top-level Main body).
    for (sym, ty) in super::affine_array::array_var_types(stmts, None, &array_return_fns) {
        let name = interner.resolve(sym).to_string();
        let mut syms: HashSet<Symbol> = HashSet::new();
        collect_named_syms(stmts, &name, interner, &mut syms);
        for s in syms {
            main_ctx.register_variable_type(s, ty.clone());
        }
    }
    // O2 de-Rc: Seqs proven to never need reference semantics → plain Vec<T>
    // (no Rc/RefCell). A var already scalarized to `[T; N]` (O3) wins — exclude it.
    let mut de_rc_seqs = collect_de_rc_seqs(
        stmts,
        interner,
        &borrow_params_map,
        &mut_borrow_params_map,
        &vec_return_fns,
        false,
    );
    for (sym, _) in &scalarizable_seqs {
        // A `[T; N]`-scalarized var already gets the win — it preempts de-Rc here.
        if de_rc_seqs.remove(sym) {
            crate::optimize::mark_preempted(Opt::Scalarize, Opt::Unbox);
        }
    }
    // Straight-line-push fixed buffers in Main → `[T; K]` (borrow-aware — the scalarizer disqualifies any
    // call-arg, even a read-only borrow, so Main's small input buffers stayed heap Vecs). The O3 `Let`/
    // `Push` handlers emit the stack array + `buf[k]=expr` fills.
    let main_sl = super::affine_array::detect_straightline_buffers(
        stmts,
        &de_rc_seqs,
        &borrow_params_map,
        interner,
    );
    for (sym, (elem_ty, len)) in &main_sl {
        let ty = format!("[{}; {}]", elem_ty, len);
        de_rc_seqs.remove(sym);
        let name = interner.resolve(*sym).to_string();
        let mut syms: HashSet<Symbol> = HashSet::new();
        collect_named_syms(stmts, &name, interner, &mut syms);
        for s in syms {
            main_ctx.register_variable_type(s, ty.clone());
        }
    }
    // Append-only worklist → pre-sized buffer + register tail (BFS frontier).
    let main_worklists = super::worklist::detect_worklists(stmts, &de_rc_seqs, interner);
    // Affine read-only array → delete it, substitute the closed form at reads
    // (CSR offset array `adjStarts[v] == v*5` becomes C's `v*5` shift). An O3
    // scalarized (`[T;N]`) or worklist symbol is already claimed — exclude it.
    let mut main_affine = super::affine_array::detect_affine_arrays(stmts, &de_rc_seqs, interner);
    main_affine.retain(|sym, _| {
        // A scalarized sequence is already claimed — Scalarize preempts Affine.
        if scalarizable_seqs.contains_key(sym) {
            crate::optimize::mark_preempted(Opt::Scalarize, Opt::Affine);
            return false;
        }
        !main_worklists.contains_key(sym)
    });
    for (sym, info) in &main_affine {
        main_ctx.register_variable_type(
            *sym,
            format!(
                "__affine_array:{}:{}:{}",
                info.coeff, info.offset, info.trip
            ),
        );
    }
    // i64→i32 element-width narrowing (gated by LOGOS_NARROW, default off): a
    // `Seq of Int` whose every value provably fits i32 is stored as `Vec<i32>`.
    // Exclude deleted (affine) and worklist sequences — they are not plain Vecs.
    let main_narrowed = narrow_seqs(stmts, &de_rc_seqs, &main_affine, &main_worklists, interner);
    for sym in main_narrowed.keys() {
        // Register the i32 element type so the borrow-hoist and indexed-read
        // dispatch treat the buffer as `&[i32]`/`Vec<i32>`; the decl paths emit
        // the matching `Vec<i32>` and the access sites convert.
        main_ctx.register_variable_type(*sym, "Vec<i32>".to_string());
    }
    let main_bits = bitpack_seqs(
        stmts,
        &de_rc_seqs,
        &|s| main_worklists.contains_key(&s) || main_affine.contains_key(&s),
        interner,
    );
    for sym in &main_bits {
        main_ctx.register_variable_type(*sym, "LogosBits".to_string());
    }
    main_ctx.set_de_rc_vars(de_rc_seqs);
    main_ctx.set_worklists(main_worklists);
    main_ctx.set_affine_arrays(main_affine);
    main_ctx.set_narrowed(main_narrowed);
    main_ctx.set_bitpacked(main_bits);
    // Non-aliased local `Map of Int to Int` → specialized `LogosI64Map`, or the
    // keys-only `LogosI64Set` when the value is never read.
    let main_i64 = super::i64_map::detect_i64_maps(stmts, interner);
    // Dense tier: maps whose key domain the oracle proved bounded within their
    // capacity hint lower to a direct-addressed flat array (no hashing/probing).
    if let Some(o) = oracle.as_deref() {
        let dense = super::i64_map::detect_dense_i64_maps(&main_i64, o, interner);
        let (i32m, i32s) = super::i64_map::detect_i32_maps(&main_i64, &dense, o);
        main_ctx.set_dense_i64(dense);
        main_ctx.set_i32_maps(i32m, i32s);
    }
    main_ctx.set_i64_sets(main_i64.sets);
    main_ctx.set_i64_maps(main_i64.maps);
    // Pre-size push-built Vecs that a later counted loop index-reads to a bound.
    // A deleted affine array must not also be pre-sized (its declaration is gone).
    let mut main_presize = super::peephole::detect_vec_presize(stmts, interner);
    main_presize.retain(|sym, _| main_ctx.affine_array(*sym).is_none());
    main_ctx.set_vec_presize(main_presize);
    // Loop-invariant positive divisors → precomputed `LogosDivU64` magic multiply.
    main_ctx.set_fast_div(super::fast_div::detect_fast_div(
        stmts,
        oracle.as_deref(),
        interner,
    ));
    main_ctx.share_fn_returns(Rc::clone(&fn_returns_map));
    let mut main_synced_vars = HashSet::new(); // Phase 52: Track synced variables in main
                                               // Phase 56: Pre-scan for Mount+Sync combinations
    let main_var_caps = analyze_variable_capabilities(stmts, interner);
    {
        let stmt_refs: Vec<&Stmt> = stmts.iter().collect();
        let mut i = 0;
        while i < stmt_refs.len() {
            // Skip function definitions - they're already emitted above
            if matches!(stmt_refs[i], Stmt::FunctionDef { .. }) {
                i += 1;
                continue;
            }
            // An affine read-only array is deleted entirely — emit no declaration.
            // (Its build push is suppressed and every read substitutes the closed
            // form.) Skip the decl here, before any peephole pattern can
            // re-materialize it as a pre-sized/with_capacity Vec.
            if let Stmt::Let { var, value, .. } = stmt_refs[i] {
                if matches!(value, Expr::New { .. }) && main_ctx.affine_array(*var).is_some() {
                    i += 1;
                    continue;
                }
            }
            // Statement-sequence peepholes (naive-search, cascade fold, presize,
            // for-range, swap, …) — one shared chain for every block context.
            if let Some((code, skip)) = super::peephole::try_block_peepholes(
                &stmt_refs,
                i,
                interner,
                1,
                &main_mutable_vars,
                &mut main_ctx,
                &lww_fields,
                &mv_fields,
                &mut main_synced_vars,
                &main_var_caps,
                &async_functions,
                &main_pipe_vars,
                &boxed_fields,
                registry,
                type_env,
            ) {
                let emit_start = lines.of(&output) + 1;
                output.push_str(&code);
                if let Some((spans, _)) = mapping {
                    // A peephole fuses statements i..=i+skip: map its lines to
                    // the merged span of everything it consumed.
                    let merged = match (spans.get(i), spans.get(i + skip)) {
                        (Some(a), Some(b)) if a.start < b.end => {
                            Some(LogosSpan::new(a.start, b.end))
                        }
                        (Some(a), _) => Some(*a),
                        _ => None,
                    };
                    record_emitted_lines(&output, &mut lines, emit_start, merged, &mut line_records);
                }
                i += 1 + skip;
                continue;
            }
            let emit_start = lines.of(&output) + 1;
            output.push_str(&codegen_stmt(
                stmt_refs[i],
                interner,
                1,
                &main_mutable_vars,
                &mut main_ctx,
                &lww_fields,
                &mv_fields,
                &mut main_synced_vars,
                &main_var_caps,
                &async_functions,
                &main_pipe_vars,
                &boxed_fields,
                registry,
                type_env,
            ));
            if let Some((spans, _)) = mapping {
                record_emitted_lines(
                    &output,
                    &mut lines,
                    emit_start,
                    spans.get(i).copied(),
                    &mut line_records,
                );
            }
            i += 1;
        }
    }
    writeln!(output, "}}").unwrap();

    let map = mapping.map(|(spans, logos_source)| {
        let mut builder = SourceMapBuilder::new(logos_source);
        for (line, span) in line_records {
            builder.record_line_at(line, span);
        }
        record_var_origins(stmts, spans, interner, &mut builder);
        builder.build()
    });
    (output, map)
}

/// Newlines emitted so far — brackets each statement's generated line range.
/// The lines of an output that only grows, each newline read once: a count reads only the text appended since the
/// last.
#[derive(Default)]
struct LineCount {
    read: usize,
    lines: u32,
}

impl LineCount {
    /// The lines `output` holds; `output` extends the text this count last read.
    fn of(&mut self, output: &str) -> u32 {
        self.lines += output.as_bytes()[self.read..].iter().filter(|&&b| b == b'\n').count() as u32;
        self.read = output.len();
        self.lines
    }
}

/// Map every line an emission produced (`start..=` the current line) to the
/// statement's span. Zero-width spans are prelude sentinels — skipped, so
/// prelude-generated code never claims user-source positions.
fn record_emitted_lines(
    output: &str,
    lines: &mut LineCount,
    start_line: u32,
    span: Option<LogosSpan>,
    records: &mut Vec<(u32, LogosSpan)>,
) {
    let Some(span) = span else { return };
    if span.start >= span.end {
        return;
    }
    let mut end_line = lines.of(output);
    if !output.ends_with('\n') {
        end_line += 1;
    }
    for line in start_line..=end_line.max(start_line) {
        records.push((line, span));
    }
}

/// Ownership-role priority: when one variable plays several roles, the move
/// is what a borrow-check error will be about.
fn role_rank(role: OwnershipRole) -> u8 {
    match role {
        // The move itself: E0382 phrasing hangs off it.
        OwnershipRole::GiveObject => 7,
        // Zone membership is structural — E0597 escape phrasing needs it even
        // when the variable is also shown or set inside the zone.
        OwnershipRole::ZoneLocal => 6,
        OwnershipRole::ShowObject => 5,
        OwnershipRole::SetTarget => 4,
        OwnershipRole::GiveRecipient => 3,
        OwnershipRole::ShowRecipient => 2,
        OwnershipRole::LetBinding => 1,
    }
}

/// Walk the program recording each variable's Rust name → LOGOS origin.
/// Nested statements attribute to their containing top-level statement's
/// span; higher-priority roles win when a variable plays several.
fn record_var_origins(
    stmts: &[Stmt],
    stmt_spans: &[LogosSpan],
    interner: &Interner,
    builder: &mut SourceMapBuilder,
) {
    use std::collections::HashMap as Map;
    let mut best: Map<String, (u8, Symbol, LogosSpan, OwnershipRole)> = Map::new();

    fn ident_of(expr: &Expr, interner: &Interner) -> Option<(String, Symbol)> {
        if let Expr::Identifier(sym) = expr {
            Some((super::escape_rust_ident(interner.resolve(*sym)), *sym))
        } else {
            None
        }
    }

    fn note(
        best: &mut Map<String, (u8, Symbol, LogosSpan, OwnershipRole)>,
        name: String,
        sym: Symbol,
        span: LogosSpan,
        role: OwnershipRole,
    ) {
        let rank = role_rank(role);
        match best.get(&name) {
            Some((existing, ..)) if *existing >= rank => {}
            _ => {
                best.insert(name, (rank, sym, span, role));
            }
        }
    }

    fn walk(
        stmts: &[Stmt],
        span: LogosSpan,
        in_zone: bool,
        interner: &Interner,
        best: &mut Map<String, (u8, Symbol, LogosSpan, OwnershipRole)>,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Let { var, .. } => {
                    let role = if in_zone {
                        OwnershipRole::ZoneLocal
                    } else {
                        OwnershipRole::LetBinding
                    };
                    let name = super::escape_rust_ident(interner.resolve(*var));
                    note(best, name, *var, span, role);
                }
                Stmt::Set { target, .. } => {
                    let name = super::escape_rust_ident(interner.resolve(*target));
                    note(best, name, *target, span, OwnershipRole::SetTarget);
                }
                Stmt::Give { object, recipient } => {
                    if let Some((name, sym)) = ident_of(object, interner) {
                        note(best, name, sym, span, OwnershipRole::GiveObject);
                    }
                    if let Some((name, sym)) = ident_of(recipient, interner) {
                        note(best, name, sym, span, OwnershipRole::GiveRecipient);
                    }
                }
                Stmt::Show { object, recipient } => {
                    if let Some((name, sym)) = ident_of(object, interner) {
                        note(best, name, sym, span, OwnershipRole::ShowObject);
                    }
                    if let Some((name, sym)) = ident_of(recipient, interner) {
                        note(best, name, sym, span, OwnershipRole::ShowRecipient);
                    }
                }
                Stmt::Zone { body, .. } => walk(body, span, true, interner, best),
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    walk(then_block, span, in_zone, interner, best);
                    if let Some(else_block) = else_block {
                        walk(else_block, span, in_zone, interner, best);
                    }
                }
                Stmt::While { body, .. } => walk(body, span, in_zone, interner, best),
                Stmt::Repeat { body, .. } => walk(body, span, in_zone, interner, best),
                Stmt::FunctionDef { body, .. } => walk(body, span, in_zone, interner, best),
                _ => {}
            }
        }
    }

    for (i, stmt) in stmts.iter().enumerate() {
        let Some(span) = stmt_spans.get(i).copied() else {
            continue;
        };
        if span.start >= span.end {
            continue;
        }
        walk(std::slice::from_ref(stmt), span, false, interner, &mut best);
    }

    for (name, (_, sym, span, role)) in best {
        builder.record_var(&name, sym, span, role);
    }
}

/// Exactify a codegen'd Rust type's integer parts to the overflow-promoting `LogosInt` — a bare
/// `i64` becomes `LogosInt`, and the `i64`s inside an `impl Fn(..) -> ..` become `LogosInt` too
/// (scalar exact-HOFs only reach here, so an `impl Fn`'s integer positions are the only `i64`s).
/// Any other type is unchanged. The call-site half of the selective higher-order-overflow analysis
/// (`exact_hof_fns`) uses this to type a closure/integer argument to the exactified parameter.
fn exactify_rust_int_type(t: &str) -> String {
    if t == "i64" {
        "logicaffeine_data::LogosInt".to_string()
    } else if t.starts_with("impl Fn") {
        t.replace("i64", "logicaffeine_data::LogosInt")
    } else if let Some(rest) = t.strip_prefix("Result<i64,") {
        // ★ AN ANSWER'S PAYLOAD IS PART OF THE ANSWER. `jsonInt(v) -> Result of Int and Text`
        // hands back a number read from a JSON document, which json.lg stores as a bignum because
        // a document may carry one — so the Ok payload has to be exactified with the value it
        // carries, or the function returns a type it cannot construct.
        format!("Result<logicaffeine_data::LogosInt,{rest}")
    } else {
        t.to_string()
    }
}

/// Phase 32/38: Generate a function definition.
/// Phase 38: Updated for native functions and TypeExpr types.
/// Phase 49: Accepts lww_fields for LWWRegister SetField handling.
/// Phase 103: Accepts registry for polymorphic enum type inference.
/// The `where` clause a generic function's type parameters need. A type parameter stands for any
/// LOGOS value, and a generic body uses it as one — copies it (value semantics), compares it,
/// shows it — so those are its bounds; every value type has them. A parameter a signature uses as
/// a map key or set element also hashes. A clause, not inline bounds, so the head stays `fn f<T>`.
fn generic_bounds(
    generics: &[Symbol],
    params: &[(Symbol, &TypeExpr)],
    return_type: Option<&TypeExpr>,
    interner: &Interner,
) -> String {
    if generics.is_empty() {
        return String::new();
    }
    fn keyed_params(
        t: &TypeExpr,
        generics: &[Symbol],
        interner: &Interner,
        keyed: &mut HashSet<Symbol>,
    ) {
        let each = |ts: &[TypeExpr], keyed: &mut HashSet<Symbol>| {
            for t in ts {
                keyed_params(t, generics, interner, keyed);
            }
        };
        match t {
            TypeExpr::Generic { base, params } => {
                if matches!(
                    interner.resolve(*base),
                    "Map" | "HashMap" | "Set" | "HashSet"
                ) {
                    if let Some(TypeExpr::Named(k) | TypeExpr::Primitive(k)) = params.first() {
                        if generics.contains(k) {
                            keyed.insert(*k);
                        }
                    }
                }
                each(params, keyed);
            }
            TypeExpr::Tuple(items) => each(items, keyed),
            TypeExpr::Record { field_types, .. } => each(field_types, keyed),
            TypeExpr::Function { inputs, output } => {
                each(inputs, keyed);
                keyed_params(output, generics, interner, keyed);
            }
            TypeExpr::Refinement { base: inner, .. }
            | TypeExpr::Persistent { inner }
            | TypeExpr::Mutable { inner } => keyed_params(inner, generics, interner, keyed),
            TypeExpr::Primitive(_) | TypeExpr::Named(_) => {}
        }
    }
    let mut keyed = HashSet::new();
    for (_, t) in params {
        keyed_params(t, generics, interner, &mut keyed);
    }
    if let Some(t) = return_type {
        keyed_params(t, generics, interner, &mut keyed);
    }
    let clauses: Vec<String> = generics
        .iter()
        .map(|g| {
            let hashes = if keyed.contains(g) { " + Eq + ::std::hash::Hash" } else { "" };
            format!(
                "{}: Clone + PartialEq + ::std::fmt::Debug + logicaffeine_system::io::Showable{hashes}",
                interner.resolve(*g)
            )
        })
        .collect();
    format!(" where {}", clauses.join(", "))
}

fn codegen_function_def<'p>(
    name: Symbol,
    generics: &[Symbol],
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    all_stmts: &[Stmt<'p>],
    return_type: Option<&TypeExpr>,
    is_native: bool,
    native_path: Option<Symbol>,
    is_exported: bool,
    export_target: Option<Symbol>,
    interner: &Interner,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>, // Phase 49b: MVRegister fields
    async_functions: &HashSet<Symbol>,     // Phase 54
    boxed_fields: &HashSet<(String, String, String)>, // Phase 102
    registry: &TypeRegistry,               // Phase 103
    pure_functions: &HashSet<Symbol>,
    type_env: &crate::analysis::types::TypeEnv,
    borrow_params_map: &HashMap<Symbol, HashSet<usize>>,
    mut_borrow_params_map: &HashMap<Symbol, HashSet<usize>>,
    value_mutable_params_map: &HashMap<Symbol, HashSet<usize>>,
    opt_flags: &OptimizationConfig,
    fn_returns_map: &Rc<HashMap<Symbol, String>>,
    vec_return_fns: &HashSet<Symbol>,
    str_slice_return_fns: &HashMap<Symbol, usize>,
    byte_slice_return_fns: &HashMap<Symbol, usize>,
    array_return_fns: &HashMap<Symbol, super::affine_array::ArrayReturnInfo>,
    fixed_array_param_map: &HashMap<(Symbol, usize), usize>,
    oracle: Option<&std::rc::Rc<crate::optimize::OracleFacts>>,
    bigint_fns: &HashSet<Symbol>,
    exact_hofs: &HashSet<Symbol>,
    // Functions whose `Result of Int and …` payload is a bignum — the SIGNATURE changes, call
    // sites do not.
    exact_result_fns: &HashSet<Symbol>,
    // Opt::AsciiClassMask: predicate name -> its precomputed 256-bit membership mask.
    ascii_masks: &HashMap<Symbol, [u64; 4]>,
    simd_fns: &Rc<super::simd_reduce::FnMap<'p>>,
) -> String {
    let mut output = String::new();
    let _tower_scope = super::tower::enter_scope(Some(name));
    let _absent_scope = super::absent::enter_function(name);
    // Overflow-promoting return: this `Int`-returning function's value can exceed i64, so it is
    // typed `-> LogosInt` and its return value is not narrowed.
    let returns_bigint = bigint_fns.contains(&name);
    // Scalar exact higher-order function (`apply(f, x)`): its `fn`-param + integer params are
    // exactified to `LogosInt` so an overflowing closure body promotes to a bignum == the VM.
    let is_exact_hof = exact_hofs.contains(&name);
    // De-Rc Phase 4: this function returns an owned `Vec<T>` (every Return is a
    // uniquely-owned fresh Seq) instead of `LogosSeq<T>`.
    let returns_vec = vec_return_fns.contains(&name);
    // Opt::TextSliceReturn: this fn returns a borrowed `&'a str` sub-slice of the param at this index.
    let str_slice_src: Option<usize> = str_slice_return_fns.get(&name).copied();
    // Opt::ByteSliceReturn: this function returns a borrowed `&'a [i64]` sub-slice of the param at this index.
    let byte_slice_src: Option<usize> = byte_slice_return_fns.get(&name).copied();
    // Step 3b: this function returns a fixed-size stack array `[T; N]` by value.
    let array_return = array_return_fns.get(&name);
    let names = RustNames::new(interner);
    let raw_name = names.raw(name);
    let func_name = names.ident(name);
    let export_target_lower = export_target.map(|s| interner.resolve(s).to_lowercase());
    // A function the exact extension reaches computes on its value model: the lowerings that rewrite
    // a body into machine-integer arithmetic (a byte table, a shift-based closed form, a memo table
    // keyed by its arguments) do not apply to it.
    let reached_by_tower = super::tower::returns(name)
        || params.iter().any(|(p, _)| super::tower::param(name, *p));

    // Opt::StrengthReducePow2: this function body's always-non-negative variables, so a `x % 2^k` /
    // `x / 2^k` on one of them can lower to `& (2^k-1)` / `>> k`. Set per-body (symbols are name-keyed,
    // so a counter `i` in another function must not leak its proof here).
    super::expr::set_nonneg_vars(super::expr::collect_always_nonneg(body));
    // Opt::WideAccumulator is Main-only in v1 — clear any set from a previous body so a function never
    // inherits Main's widened accumulators (symbols are name-keyed).
    super::expr::set_wide_acc(std::collections::HashSet::new());

    // Opt::AsciiClassMask: this predicate's body IS a 256-bit membership mask — emit the branchless
    // mask test in place of the comparison chain, so every caller (LLVM inlines this tiny fn) gets a
    // no-load classification that beats C's ctype table. Byte-identical: the mask over [0,256) is the
    // CTFE of the predicate, and the `(0<=cc<256)` guard reproduces its false-outside-range behavior.
    if let Some(mask) = ascii_masks.get(&name).filter(|_| !reached_by_tower) {
        if !is_exported
            && crate::optimize::active_config().is_on(crate::optimization::Opt::AsciiClassMask)
        {
            crate::optimize::mark_fired(crate::optimization::Opt::AsciiClassMask);
            let cparam = names.ident(params[0].0);
            // A FLAT 256-byte membership table (like C's ctype), not a compact bitmask: measured to
            // beat C (~41ms vs 49ms with branchless), whereas the bitmask's per-element shift/mask
            // arithmetic (~105ms) is slower than one hot-L1 table load. Built from the CTFE mask.
            let table: String = (0..256usize)
                .map(|c| (((mask[c >> 6] >> (c & 63)) & 1) as u8).to_string())
                .collect::<Vec<_>>()
                .join(",");
            let mut out = String::new();
            out.push_str("#[inline]\n");
            writeln!(
                out,
                "fn {}({}: i64) -> bool {{ static __T: [u8; 256] = [{}]; let __cc = {}; (0 <= __cc && __cc < 256) && __T[__cc as usize] != 0 }}",
                func_name, cparam, table, cparam
            )
            .unwrap();
            return out;
        }
    }

    // Phase 54: Detect which parameters are used as pipe senders
    let pipe_sender_params = collect_pipe_sender_params(body);
    let pipes_used = super::replay::pipes_used(body);

    // FFI: Exported functions need special signatures
    let is_c_export_early =
        is_exported && matches!(export_target_lower.as_deref(), None | Some("c"));

    // TCE: Detect tail recursion eligibility (respects ## No TCO / ## No Optimize)
    let no_tco = !crate::optimize::active_config()
        .merged(opt_flags)
        .is_on(Opt::Tco);
    // A direct `Return self(args)` always TCE's. The `Set/Let x = self(args);
    // Return x` pair TCE's too — UNLESS the function is borrow/mut-borrow-eligible
    // (an in-place array recursion like quicksort), where that rewrite is the
    // better lowering and pair-TCE would force an owned, cloned parameter. Such
    // functions recurse only O(log n) deep, so the constant-stack guarantee is
    // moot for them anyway.
    let has_tail_pair = body_has_top_level_tail_pair(name, body, params.len());
    // A borrow/mut-borrow-eligible function keeps the borrow rather than pair-TCE —
    // Borrow preempts Tco (pair) for it.
    if has_tail_pair
        && (borrow_params_map.contains_key(&name) || mut_borrow_params_map.contains_key(&name))
    {
        crate::optimize::mark_preempted(Opt::Borrow, Opt::Tco);
    }
    let pair_tce_ok = has_tail_pair
        && !borrow_params_map.contains_key(&name)
        && !mut_borrow_params_map.contains_key(&name);
    let is_tce = !is_native
        && !is_c_export_early
        && !no_tco
        && (is_tail_recursive(name, body) || pair_tce_ok);
    if is_tce {
        crate::optimize::mark_fired(Opt::Tco);
    }
    let param_syms: Vec<Symbol> = params.iter().map(|(s, _)| *s).collect();

    // Accumulator Introduction: Detect non-tail single-call + / * patterns
    let acc_info = if !is_tce && !is_native && !is_c_export_early {
        detect_accumulator_pattern(name, body)
    } else {
        None
    };
    let is_acc = acc_info.is_some();

    // Closed-form: Detect double recursion f(0)=base, f(d)=k+f(d-1)+f(d-1)
    // Emits ((base+k) << d) - k instead of memoization — matches GCC/LLVM -O2.
    let closed_form_info = if !is_tce && !is_acc && !is_native && !is_c_export_early && !reached_by_tower {
        detect_double_recursion_closed_form(name, params, body, interner)
    } else {
        None
    };
    let is_closed_form = closed_form_info.is_some();
    if is_closed_form {
        // The affine double-recursion collapse (`f(d)=k+f(d-1)+f(d-1)` → `(base+k)<<d - k`)
        // is a codegen-time closed form; record it in the firing trace so a
        // `## Require closedform` can depend on it, like the optimize-pass collapses.
        crate::optimize::mark_fired(Opt::ClosedForm);
    }

    // Memoization: Detect pure multi-call recursive functions with hashable params
    // Respects ## No Memo / ## No Optimize annotations
    let no_memo = !crate::optimize::active_config()
        .merged(opt_flags)
        .is_on(Opt::Memo);
    let is_memo = !is_tce
        && !is_acc
        && !is_closed_form
        && !reached_by_tower
        && !is_native
        && !is_c_export_early
        && !no_memo
        && should_memoize(
            name,
            body,
            params,
            return_type,
            pure_functions.contains(&name),
            interner,
        );
    if is_memo {
        crate::optimize::mark_fired(Opt::Memo);
    }
    // A tail-recursive function that WOULD otherwise memoize is claimed by TCE
    // instead — Tco preempts Memo (eval `should_memoize` only here, where `is_memo`
    // short-circuited it via `!is_tce`, so it is computed at most once).
    if is_tce
        && !no_memo
        && should_memoize(
            name,
            body,
            params,
            return_type,
            pure_functions.contains(&name),
            interner,
        )
    {
        crate::optimize::mark_preempted(Opt::Tco, Opt::Memo);
    }

    let needs_mut_params = is_tce || is_acc;

    // Peephole: respect ## No Peephole / ## No Optimize annotations
    let no_peephole = !crate::optimize::active_config()
        .merged(opt_flags)
        .is_on(Opt::Peephole);

    // Get borrow indices for this function (empty set if none)
    let borrow_indices = borrow_params_map.get(&name).cloned().unwrap_or_default();

    // Get mutable borrow indices (element-only mutation, return same param)
    let mut_borrow_indices = mut_borrow_params_map
        .get(&name)
        .cloned()
        .unwrap_or_default();

    // Compute mutable vars early for param mutability detection
    let func_mutable_vars = collect_mutable_vars(body);

    // Build parameter list using TypeExpr
    let params_str: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, (param_name, param_type))| {
            let pname = names.ident(*param_name);
            if super::tower::param(name, *param_name) {
                let mutable = needs_mut_params || func_mutable_vars.contains(param_name);
                return format!("{}{}: {}", if mutable { "mut " } else { "" }, pname, super::tower::VALUE);
            }
            let ty = codegen_type_expr(param_type, interner);
            // A parameter that can hold `nothing` holds the caller's value-or-nothing, lent or passed whole.
            if let Some(held) = super::absent::parameter(name, *param_name, &ty) {
                return if matches!(param_type, crate::ast::stmt::TypeExpr::Mutable { .. }) {
                    format!("{}: &mut {}", pname, held)
                } else if needs_mut_params || func_mutable_vars.contains(param_name) {
                    format!("mut {}: {}", pname, held)
                } else {
                    format!("{}: {}", pname, held)
                };
            }
            // Exact-HOF: exactify this parameter's integer types (`i64` → `LogosInt`, inside
            // `impl Fn(..)` too), so the fn-param is `impl Fn(LogosInt) -> LogosInt` and an integer
            // param is `LogosInt` — the closure/integer arguments the call-site promotes flow in.
            let ty = if is_exact_hof {
                exactify_rust_int_type(&ty)
            } else {
                ty
            };
            // A `mutable` param is lent: a `LogosSeq`/`LogosMap` as a shared `&` (its writes go
            // through its own `RefCell`), anything else as `&mut` — either way the callee's writes
            // are the caller's value.
            if matches!(param_type, crate::ast::stmt::TypeExpr::Mutable { .. }) {
                let lend = if super::expr::lent_by_shared_reference(&ty) {
                    "&"
                } else {
                    "&mut "
                };
                format!("{}: {}{}", pname, lend, ty)
            } else if let Some(pipe) =
                super::replay::pipe_param_type(*param_name, param_type, &pipes_used, interner)
            {
                format!("{}: {}", pname, pipe)
            } else if pipe_sender_params.contains(param_name) {
                format!("{}: tokio::sync::mpsc::Sender<{}>", pname, ty)
            } else if str_slice_src == Some(i) {
                // Opt::TextSliceReturn: this borrowed Text param is the source of the returned
                // sub-slice, so it carries the named lifetime `'a` tying the `&'a str` return to it.
                format!("{}: &'a str", pname)
            } else if byte_slice_src == Some(i) {
                // Opt::ByteSliceReturn: this borrowed Seq param is the source of the returned
                // `&'a [i64]` sub-slice, so it carries the named lifetime `'a`.
                let slice_ty = vec_to_slice_type(&ty);
                format!("{}: {}", pname, slice_ty.replacen('&', "&'a ", 1))
            } else if borrow_indices.contains(&i) && super::lent::is_user_enum(param_type, registry)
            {
                format!("{}: &{}", pname, ty)
            } else if borrow_indices.contains(&i) {
                // Read-only Vec param → borrow as `&[T]`, or `&[T; N]` when every caller passes a fixed
                // `[T; N]` array (LLVM then elides the constant-index bounds checks).
                let slice_ty = vec_to_slice_type(&ty);
                let final_ty = match fixed_array_param_map.get(&(name, i)) {
                    Some(n) => slice_ty
                        .strip_prefix("&[")
                        .and_then(|s| s.strip_suffix(']'))
                        .map(|elem| format!("&[{}; {}]", elem, n))
                        .unwrap_or(slice_ty),
                    None => slice_ty,
                };
                format!("{}: {}", pname, final_ty)
            } else if mut_borrow_indices.contains(&i) {
                // Element-only mutation Vec param → borrow as &mut [T]
                let slice_ty = vec_to_mut_slice_type(&ty);
                format!("{}: {}", pname, slice_ty)
            } else if needs_mut_params || func_mutable_vars.contains(param_name) {
                format!("mut {}: {}", pname, ty)
            } else {
                format!("{}: {}", pname, ty)
            }
        })
        .collect();

    // Get return type string from TypeExpr or infer from body
    // If this function has &mut borrow params and returns one of them,
    // suppress the return type since the mutation happens in-place.
    let has_mut_borrow = !mut_borrow_indices.is_empty();
    let return_type_str = if super::tower::returns(name) {
        Some(super::tower::VALUE.to_string())
    } else if let Some(held) = super::absent::returned(name) {
        Some(held)
    } else if exact_result_fns.contains(&name) {
        return_type.map(|t| exactify_rust_int_type(&codegen_type_expr(t, interner)))
    } else if returns_bigint {
        // Overflow-promoting: an `Int` return whose value can exceed i64 is a `LogosInt`.
        //
        // ★ AN ANSWER-SHAPED RETURN KEEPS ITS SHAPE. `jsonInt(v) -> Result of Int and Text` hands
        // back a number that a JSON document may have made huge, but it hands it back INSIDE a
        // Result — so the payload is exactified and the Result stays a Result. Replacing the
        // whole type with `LogosInt` would make the function unable to report the failure it
        // exists to report.
        match return_type.map(|t| codegen_type_expr(t, interner)) {
            Some(t) if t.starts_with("Result<i64,") => Some(exactify_rust_int_type(&t)),
            _ => Some("logicaffeine_data::LogosInt".to_string()),
        }
    } else if str_slice_src.is_some() {
        // Opt::TextSliceReturn: a borrowed sub-slice of the input Text.
        Some("&'a str".to_string())
    } else if byte_slice_src.is_some() {
        // Opt::ByteSliceReturn: a borrowed sub-slice of the input Seq (`&'a [i64]`).
        return_type
            .map(|t| vec_to_slice_type(&codegen_type_expr(t, interner)).replacen('&', "&'a ", 1))
    } else if has_mut_borrow {
        None // No return type — mutation is in-place via &mut [T]
    } else if let Some(info) = array_return {
        // Step 3b: `LogosSeq<T>` → a fixed-size stack array `[T; N]` returned by value.
        Some(format!("[{}; {}]", info.elem_ty, info.len))
    } else if returns_vec {
        // Phase 4: `LogosSeq<T>` → owned `Vec<T>`. Replace the `LogosSeq<` head
        // of the codegen'd type (`LogosSeq<i64>` → `Vec<i64>`).
        return_type.map(|t| codegen_type_expr(t, interner).replacen("LogosSeq<", "Vec<", 1))
    } else {
        return_type
            .map(|t| codegen_type_expr(t, interner))
            // An omitted return type: consume the checker's Hindley-Milner
            // inference (`type_env`, registry-aware — resolves field-access and
            // call returns) before the weaker codegen-local body heuristic. This
            // is what keeps `## To f(..):` with a field/call/struct return from
            // emitting the illegal `-> _`.
            .or_else(|| {
                type_env
                    .lookup_fn(name)
                    .and_then(|sig| logos_return_type_to_rust(&sig.return_type, interner))
            })
            .or_else(|| infer_return_type_from_body(body, params, registry, interner))
    };

    // Phase 51/54: Check if function is async (includes transitive async detection)
    let is_async = async_functions.contains(&name);
    let fn_keyword = if is_async { "async fn" } else { "fn" };

    // FFI: Exported functions need special signatures
    let is_c_export = is_c_export_early;

    // FFI: Check if C export needs type marshaling
    // Triggers for: Text params/return, reference types, Result return, refinement params
    let needs_c_marshaling = is_c_export && {
        let has_text_param = params.iter().any(|(_, ty)| is_text_type(ty, interner));
        let has_text_return = return_type.map_or(false, |ty| is_text_type(ty, interner));
        let has_ref_param = params.iter().any(|(_, ty)| {
            classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ReferenceType
        });
        let has_ref_return = return_type.map_or(false, |ty| {
            classify_type_for_c_abi(ty, interner, registry) == CAbiClass::ReferenceType
        });
        let has_result_return = return_type.map_or(false, |ty| is_result_type(ty, interner));
        let has_refinement_param = params
            .iter()
            .any(|(_, ty)| matches!(ty, TypeExpr::Refinement { .. }));
        // Char crosses the C ABI as uint32_t; route through the marshal path so
        // the wrapper validates it via char::from_u32 instead of exposing a raw
        // Rust `char` (an out-of-range u32 from C is UB).
        let has_char_param = params.iter().any(|(_, ty)| is_char_type(ty, interner));
        let has_char_return = return_type.map_or(false, |ty| is_char_type(ty, interner));
        has_text_param
            || has_text_return
            || has_ref_param
            || has_ref_return
            || has_result_return
            || has_refinement_param
            || has_char_param
            || has_char_return
    };

    if needs_c_marshaling {
        // Generate two-function pattern: inner function + C ABI wrapper
        return codegen_c_export_with_marshaling(
            name,
            params,
            body,
            return_type,
            interner,
            lww_fields,
            mv_fields,
            async_functions,
            boxed_fields,
            registry,
            type_env,
        );
    }

    // Build function signature
    let (vis_prefix, abi_prefix) = if is_exported {
        match export_target_lower.as_deref() {
            None | Some("c") => ("pub ", "extern \"C\" "),
            Some("wasm") => ("pub ", ""),
            _ => ("pub ", ""),
        }
    } else {
        ("", "")
    };

    // Generic functions: "fn identity<T>" — resolve each generic Symbol to its name.
    // Opt::TextSliceReturn prepends the borrowed-return lifetime `'a`.
    let generics_str = if generics.is_empty() {
        if str_slice_src.is_some() || byte_slice_src.is_some() {
            "<'a>".to_string()
        } else {
            String::new()
        }
    } else {
        let params_list: Vec<&str> = generics.iter().map(|sym| interner.resolve(*sym)).collect();
        if str_slice_src.is_some() || byte_slice_src.is_some() {
            format!("<'a, {}>", params_list.join(", "))
        } else {
            format!("<{}>", params_list.join(", "))
        }
    };

    let signature = if let Some(ref ret_ty) = return_type_str {
        if ret_ty != "()" {
            format!(
                "{}{}{} {}{}({}) -> {}",
                vis_prefix,
                abi_prefix,
                fn_keyword,
                func_name,
                generics_str,
                params_str.join(", "),
                ret_ty
            )
        } else {
            format!(
                "{}{}{} {}{}({})",
                vis_prefix,
                abi_prefix,
                fn_keyword,
                func_name,
                generics_str,
                params_str.join(", ")
            )
        }
    } else {
        format!(
            "{}{}{} {}{}({})",
            vis_prefix,
            abi_prefix,
            fn_keyword,
            func_name,
            generics_str,
            params_str.join(", ")
        )
    };
    let signature = format!(
        "{signature}{}",
        generic_bounds(generics, params, return_type, interner)
    );
    let _generic_params = super::expr::GenericParamsInScope::enter(
        generics
            .iter()
            .map(|g| interner.resolve(*g).to_string())
            .collect(),
    );
    let _lent_params =
        super::expr::LentParamsInScope::enter(match value_mutable_params_map.get(&name) {
            Some(lent) => params
                .iter()
                .enumerate()
                .filter(|(i, _)| lent.contains(i))
                .map(|(_, (p, _))| *p)
                .collect(),
            None => HashSet::new(),
        });

    // Emit #[inline] for small non-recursive, non-exported functions
    // Closed-form functions are tiny (2 lines) and should always inline.
    if is_closed_form
        || (!is_tce && !is_acc && should_inline(name, body, is_native, is_exported, is_async))
    {
        writeln!(output, "#[inline]").unwrap();
    }

    // FFI: Emit export attributes before the function
    if is_exported {
        match export_target_lower.as_deref() {
            None | Some("c") => {
                writeln!(output, "#[export_name = \"logos_{}\"]", raw_name).unwrap();
            }
            Some("wasm") => {
                writeln!(output, "#[wasm_bindgen]").unwrap();
            }
            _ => {}
        }
    }

    // Phase 38: Handle native functions
    if is_native {
        let arg_names: Vec<&str> = params.iter().map(|(n, _)| interner.resolve(*n)).collect();

        if let Some(path_sym) = native_path {
            // User-defined native path: call the Rust path directly
            let path = interner.resolve(path_sym);
            // Validate path looks like a valid Rust path (identifiers separated by ::)
            let is_valid_path = !path.is_empty()
                && path.split("::").all(|seg| {
                    !seg.is_empty() && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
                });
            if is_valid_path {
                writeln!(output, "{} {{", signature).unwrap();
                writeln!(output, "    {}({})", path, arg_names.join(", ")).unwrap();
                writeln!(output, "}}\n").unwrap();
            } else {
                writeln!(output, "{} {{", signature).unwrap();
                writeln!(output, "    compile_error!(\"Invalid native function path: '{}'. Path must be a valid Rust path like \\\"crate::module::function\\\".\")", path).unwrap();
                writeln!(output, "}}\n").unwrap();
            }
        } else {
            // Legacy system functions: use map_native_function()
            if let Some((module, core_fn)) = map_native_function(raw_name) {
                writeln!(output, "{} {{", signature).unwrap();
                let call = format!(
                    "logicaffeine_system::{}::{}({})",
                    module,
                    core_fn,
                    arg_names.join(", ")
                );
                // A native fn declared `-> Result of T and E` (e.g. file read/write) has a
                // runtime that returns `std::result::Result`, but the wrapper's declared type is
                // the LOGOS `user_types::Result` enum (`Ok { value }` / `Err { error }`). Convert
                // at the boundary so the primitive actually usable from LOGOS.
                if refuses_by_failing(raw_name) {
                    // A kernel whose arguments break its contract stops the program with the same
                    // report the tree-walker and the VM give (`NativeKernel::failure`).
                    writeln!(output, "    match {call} {{").unwrap();
                    writeln!(output, "        ::std::result::Result::Ok(v) => v,").unwrap();
                    writeln!(
                        output,
                        "        ::std::result::Result::Err(why) => panic!(\"{{}}(): {{}}\", {:?}, why),",
                        raw_name
                    )
                    .unwrap();
                    writeln!(output, "    }}").unwrap();
                } else if signature.contains("-> Result<") {
                    writeln!(output, "    match {call} {{").unwrap();
                    writeln!(
                        output,
                        "        ::std::result::Result::Ok(v) => Result::Ok {{ value: v }},"
                    )
                    .unwrap();
                    writeln!(
                        output,
                        "        ::std::result::Result::Err(e) => Result::Err {{ error: e }},"
                    )
                    .unwrap();
                    writeln!(output, "    }}").unwrap();
                } else if let Some(int) = ["i64", "u64"]
                    .into_iter()
                    .find(|int| signature.ends_with(&format!("-> {int}")))
                {
                    // The system function's integer width is its own; the declaration names the LOGOS
                    // one (`Int`, or `Nat`), so the wrapper answers in that.
                    writeln!(output, "    ({call}) as {int}").unwrap();
                } else {
                    writeln!(output, "    {call}").unwrap();
                }
                writeln!(output, "}}\n").unwrap();
            } else {
                writeln!(output, "{} {{", signature).unwrap();
                writeln!(
                    output,
                    "    compile_error!({:?})",
                    crate::semantics::builtins::unimplemented_native(raw_name, None)
                )
                .unwrap();
                writeln!(output, "}}\n").unwrap();
            }
        }
    } else {
        // Non-native: emit body (also used for exported functions which have bodies)
        writeln!(output, "{} {{", signature).unwrap();

        // Entry precondition guard (BCE for recursive 1-based partitions): make
        // the function's `1 <= lo` and `hi <= len` precondition explicit so LLVM
        // drops the per-access bounds checks across the hot partition loop.
        // Gated on `lo < hi` (the indexing path) so it never fires for the
        // base-case range; only emitted for pure (no-I/O) functions, so the
        // abort is equivalent to the out-of-range access it pre-empts. Emitted
        // even for a tail-call-eliminated partition (value semantics makes the
        // `Set result to qs(...)` threading TCE-able): the assert runs on the
        // INITIAL params before the loop (sound), and every in-loop access keeps
        // its own oracle `assert_unchecked` hint.
        if !is_acc {
            if let Some(g) = super::entry_guard::detect_entry_guard(params, body, interner) {
                let arr = names.ident(g.arr);
                let lo = names.ident(g.lo);
                let hi = names.ident(g.hi);
                let adj = g.hi_len_adjust;
                writeln!(
                    output,
                    "    if ({lo}) < ({hi}) {{ let __{arr}_glen = {arr}.len() as i64 + {adj}; assert!(({lo}) >= 1 && ({hi}) <= __{arr}_glen, \"LOGOS precondition guard: 1-based index range\"); }}"
                )
                .unwrap();
                // The same precondition (`hi <= arr.len()`) that licenses the
                // unchecked indexing also bounds the partition counters, so their
                // `+ 1` cannot overflow — lower it raw. Sound; see the fn's doc.
                super::entry_guard::mark_deguardable_counters(params, body, &g, interner);
            } else if let Some(g) =
                super::entry_guard::detect_heap_sift_guard(params, body, interner)
            {
                // Affine-index descent: the precondition `start >= 0 && end < arr.len()`
                // (guarded on the descent-entry `2*start+1 <= end`, the indexing path)
                // lets the oracle prove every `[0, end]` index in range. Pure fn, so the
                // abort is equivalent to the out-of-range access it pre-empts.
                let arr = names.ident(g.arr);
                let start = names.ident(g.start);
                let end = names.ident(g.end);
                writeln!(
                    output,
                    "    if (2i64.wrapping_mul({start}).wrapping_add(1)) <= ({end}) {{ let __{arr}_glen = {arr}.len() as i64; assert!(({start}) >= 0 && ({end}) < __{arr}_glen, \"LOGOS precondition guard: affine index range\"); }}"
                )
                .unwrap();
                // The same `end < arr.len()` precondition bounds every heap index in
                // the descent, so its affine index arithmetic (`2*root+1`, `child+1`,
                // …) provably fits `i64` for a wide (`>= 8` byte) element — lower it
                // raw instead of the overflow-promote form. Sound; see the fn's doc.
                super::entry_guard::mark_heap_sift_index_arith(params, body, &g, interner);
            }
        }

        // Wrap exported C functions in catch_unwind for panic safety
        let wrap_catch_unwind = is_c_export;
        if wrap_catch_unwind {
            writeln!(
                output,
                "    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {{"
            )
            .unwrap();
        }

        let mut func_ctx = RefinementContext::new();
        func_ctx.set_simd_fns(Rc::clone(simd_fns));
        // O1 borrow hoisting: same oracle, keyed by the loop Stmts in this
        // body (sub-slice of the program codegen analyzed).
        if let Some(o) = oracle {
            func_ctx.set_oracle(o.clone());
        }
        // Local Vec optimization: detect which collection vars escape this function
        let func_escaping = collect_escaping_collection_vars(body);
        func_ctx.set_escaping_vars(func_escaping);
        // Overflow-promoting Int bindings in this function body (a call to a bignum function counts
        // as a bignum-producing RHS). A promoted variable that is RETURNED makes this function
        // `-> LogosInt` (`returns_bigint`, computed by the whole-program fixpoint).
        // An exactified function's machine-word parameters hold bignums (registered `LogosInt` below),
        // so what the body sets from one is a candidate too — the same seeds the analysis used.
        let exact_params = if is_exact_hof {
            crate::codegen::bigint_promote::exact_param_seeds(params, interner)
        } else {
            HashSet::new()
        };
        func_ctx.set_promotable_candidates(crate::codegen::bigint_promote::promotable_int_vars(
            body,
            bigint_fns,
            &crate::codegen::expr::bigint_struct_fields_snapshot(),
            &exact_params,
        ));
        // O2 de-Rc: function-LOCAL Seqs that never need reference semantics →
        // plain Vec<T>. The use-scan disqualifies any returned/escaping/aliased
        // handle, so only genuinely-local buffers (sieve flags, scratch arrays)
        // de-Rc. Parameters are not candidates (Phase 3 handles those).
        let func_de_rc = collect_de_rc_seqs(
            body,
            interner,
            borrow_params_map,
            mut_borrow_params_map,
            vec_return_fns,
            returns_vec,
        );
        let func_worklists = super::worklist::detect_worklists(body, &func_de_rc, interner);
        let mut func_affine =
            super::affine_array::detect_affine_arrays(body, &func_de_rc, interner);
        func_affine.retain(|sym, _| !func_worklists.contains_key(sym));
        // Constant-table locals → stack arrays `[T; N]` (zero heap, direct index). Detected while
        // `func_de_rc` is still borrowable; excluded from affine/narrow so no other pass re-claims them,
        // and registered with the `[T; N]` type so the Index codegen emits a direct array read.
        let func_const_tables = super::affine_array::detect_const_tables(
            body,
            all_stmts,
            &func_de_rc,
            borrow_params_map,
            interner,
        );
        // Fixed-size, non-escaping scratch buffers → in-place `[T; N]` via from_fn (disjoint from constant
        // tables: those have constant values and hoist; these have per-iteration values and stay in place).
        let mut func_scratch = super::affine_array::detect_scratch_buffers(
            body,
            &func_de_rc,
            borrow_params_map,
            interner,
        );
        func_scratch.retain(|sym, _| !func_const_tables.contains_key(sym));
        // Zero-init + indexed-write fixed-size buffers → `[T; N]` stack arrays. Disjoint from scratch/const
        // (those are push-built read-only; these are indexed-written).
        let mut func_indexed =
            super::affine_array::detect_indexed_buffers(body, &func_de_rc, interner);
        func_indexed.retain(|sym, _| {
            !func_const_tables.contains_key(sym) && !func_scratch.contains_key(sym)
        });
        func_scratch.retain(|sym, _| !func_indexed.contains_key(sym));
        // Straight-line-push fixed buffers → `[T; K]` (borrow-aware; the O3 `Let`/`Push` handlers emit the
        // stack array + `buf[k]=expr` fills). Disjoint from the fixed-size passes above.
        let mut func_sl = super::affine_array::detect_straightline_buffers(
            body,
            &func_de_rc,
            borrow_params_map,
            interner,
        );
        func_sl.retain(|sym, _| {
            !func_const_tables.contains_key(sym)
                && !func_scratch.contains_key(sym)
                && !func_indexed.contains_key(sym)
        });
        // Cursor-filled fixed-size buffers → `[T; N]` + a runtime fill cursor. Reaches the
        // SELF-REFERENTIAL / multi-phase push builds the straight-line/from_fn/const-table
        // passes reject — sha256's message schedule `w[t] = f(w[t-2], …)`, built by a copy
        // loop then a self-referential compute loop. `sf_fixed_len` resolves a `for _ in coll`
        // copy phase's element count from the fixed-array params + the buffers already claimed.
        let mut sf_fixed_len: HashMap<Symbol, usize> = HashMap::new();
        for (i, (psym, _)) in params.iter().enumerate() {
            if let Some(&n) = fixed_array_param_map.get(&(name, i)) {
                sf_fixed_len.insert(*psym, n);
            }
        }
        for (s, info) in &func_const_tables {
            sf_fixed_len.insert(*s, info.values.len());
        }
        for (s, info) in &func_scratch {
            sf_fixed_len.insert(*s, info.len);
        }
        for (s, info) in &func_indexed {
            sf_fixed_len.insert(*s, info.len);
        }
        for (s, (_, n)) in &func_sl {
            sf_fixed_len.insert(*s, *n);
        }
        let mut func_seq_fill = super::affine_array::detect_seq_fill_buffers(
            body,
            &func_de_rc,
            borrow_params_map,
            &|sym| sf_fixed_len.get(&sym).copied(),
            interner,
        );
        func_seq_fill.retain(|sym, _| {
            !func_const_tables.contains_key(sym)
                && !func_scratch.contains_key(sym)
                && !func_indexed.contains_key(sym)
                && !func_sl.contains_key(sym)
        });
        func_affine.retain(|sym, _| {
            !func_const_tables.contains_key(sym)
                && !func_scratch.contains_key(sym)
                && !func_seq_fill.contains_key(sym)
        });
        for (sym, info) in &func_affine {
            func_ctx.register_variable_type(
                *sym,
                format!(
                    "__affine_array:{}:{}:{}",
                    info.coeff, info.offset, info.trip
                ),
            );
        }
        let mut func_narrowed =
            narrow_seqs(body, &func_de_rc, &func_affine, &func_worklists, interner);
        func_narrowed.retain(|sym, _| {
            !func_const_tables.contains_key(sym)
                && !func_scratch.contains_key(sym)
                && !func_indexed.contains_key(sym)
                && !func_sl.contains_key(sym)
                && !func_seq_fill.contains_key(sym)
        });
        for sym in func_narrowed.keys() {
            func_ctx.register_variable_type(*sym, "Vec<i32>".to_string());
        }
        let func_bits = bitpack_seqs(
            body,
            &func_de_rc,
            &|s| {
                func_worklists.contains_key(&s)
                    || func_affine.contains_key(&s)
                    || func_const_tables.contains_key(&s)
                    || func_scratch.contains_key(&s)
                    || func_indexed.contains_key(&s)
                    || func_sl.contains_key(&s)
                    || func_seq_fill.contains_key(&s)
            },
            interner,
        );
        for sym in &func_bits {
            func_ctx.register_variable_type(*sym, "LogosBits".to_string());
        }
        func_ctx.set_de_rc_vars(func_de_rc);
        func_ctx.set_worklists(func_worklists);
        func_ctx.set_affine_arrays(func_affine);
        func_ctx.set_narrowed(func_narrowed);
        func_ctx.set_bitpacked(func_bits);
        let func_i64 = super::i64_map::detect_i64_maps(body, interner);
        if let Some(o) = oracle {
            let dense = super::i64_map::detect_dense_i64_maps(&func_i64, o.as_ref(), interner);
            let (i32m, i32s) = super::i64_map::detect_i32_maps(&func_i64, &dense, o.as_ref());
            func_ctx.set_dense_i64(dense);
            func_ctx.set_i32_maps(i32m, i32s);
        }
        func_ctx.set_i64_sets(func_i64.sets);
        func_ctx.set_i64_maps(func_i64.maps);
        let mut func_presize = super::peephole::detect_vec_presize(body, interner);
        func_presize.retain(|sym, _| {
            func_ctx.affine_array(*sym).is_none()
                && !func_const_tables.contains_key(sym)
                && !func_scratch.contains_key(sym)
                && !func_indexed.contains_key(sym)
                && !func_sl.contains_key(sym)
                && !func_seq_fill.contains_key(sym)
        });
        func_ctx.set_vec_presize(func_presize);
        func_ctx.set_fast_div(super::fast_div::detect_fast_div(
            body,
            oracle.map(|o| o.as_ref()),
            interner,
        ));
        func_ctx.set_returns_vec(returns_vec);
        func_ctx.set_str_slice_source(str_slice_src);
        func_ctx.set_byte_slice_source(byte_slice_src);
        func_ctx.set_returns_bigint(returns_bigint);
        func_ctx.set_returns_rationals(
            fn_returns_map.get(&name).filter(|t| t.contains(super::rational::RATIONAL)).cloned(),
        );
        // OPT: Detect single-char text vars in function body
        let func_single_char_vars = collect_single_char_text_vars(body, interner);
        for sym in &func_single_char_vars {
            func_ctx.register_variable_type(*sym, "__single_char_u8".to_string());
        }
        let mut func_synced_vars = HashSet::new(); // Phase 52: Track synced variables in function
                                                   // Phase 56: Pre-scan for Mount+Sync combinations in function body
        let func_var_caps = analyze_variable_capabilities(body, interner);

        // Phase 50: Register parameter types for capability Check resolution
        // Borrow-optimized params get &[T] or &mut [T] type so downstream codegen handles them correctly
        for (i, (param_name, param_type)) in params.iter().enumerate() {
            let type_name = codegen_type_expr(param_type, interner);
            if let Some(held) = super::absent::parameter(name, *param_name, &type_name) {
                func_ctx.register_variable_type(*param_name, held);
            } else if super::tower::param(name, *param_name) {
                func_ctx.register_variable_type(*param_name, super::tower::VALUE.to_string());
            } else if let Some(pipe) =
                super::replay::pipe_param_type(*param_name, param_type, &pipes_used, interner)
            {
                func_ctx.register_variable_type(*param_name, pipe);
            } else if borrow_indices.contains(&i) && super::lent::is_user_enum(param_type, registry)
            {
                func_ctx.register_variable_type(*param_name, format!("&{}", type_name));
            } else if borrow_indices.contains(&i) {
                let slice_ty = vec_to_slice_type(&type_name);
                let reg_ty = match fixed_array_param_map.get(&(name, i)) {
                    Some(n) => slice_ty
                        .strip_prefix("&[")
                        .and_then(|s| s.strip_suffix(']'))
                        .map(|elem| format!("&[{}; {}]", elem, n))
                        .unwrap_or(slice_ty),
                    None => slice_ty,
                };
                func_ctx.register_variable_type(*param_name, reg_ty);
            } else if mut_borrow_indices.contains(&i) {
                func_ctx.register_variable_type(*param_name, vec_to_mut_slice_type(&type_name));
            } else if is_exact_hof
                && matches!(param_type, crate::ast::stmt::TypeExpr::Function { .. })
            {
                // An exactified `fn`-typed param (`impl Fn(LogosInt) -> LogosInt`). The `|__exact_fnptr`
                // sentinel tells the call emit that CALLING this param (`f(x)`) must promote its integer
                // arguments to `LogosInt` (they flow into a `LogosInt` closure param).
                func_ctx.register_variable_type(
                    *param_name,
                    format!("{}{}", exactify_rust_int_type(&type_name), "|__exact_fnptr"),
                );
            } else if is_exact_hof && type_name == "i64" {
                // An exactified integer param — promoted (`__bigint`) so the body's arithmetic on it is
                // exact and it passes into a `LogosInt`-param closure without narrowing.
                func_ctx.register_variable_type(*param_name, "i64|__bigint".to_string());
            } else {
                func_ctx.register_variable_type(*param_name, type_name);
            }
        }

        let given: HashSet<Symbol> = params.iter().map(|(p, _)| *p).collect();
        let func_joins = super::rational::joins(
            body,
            &given,
            super::rational::Typing { interner, variable_types: func_ctx.get_variable_types(), registry },
        );
        func_ctx.set_rational_joins(func_joins);

        func_ctx.share_fn_returns(Rc::clone(fn_returns_map));

        // Phase 54: Functions receive pipe senders as parameters, no local pipe declarations
        let func_pipe_vars = HashSet::new();

        // Emit the constant-table stack arrays once at the top of the body (before every emission path);
        // their original heap-`Vec` `Let`/`Push` build is filtered out (`is_const_table_stmt`), and reads
        // hit the `[T; N]` array via the registered type. Zero-alloc constant tables — MD5's shift words.
        for (sym, info) in &func_const_tables {
            writeln!(
                output,
                "    let {}: [{}; {}] = [{}];",
                names.ident(*sym),
                info.elem_ty,
                info.values.len(),
                info.values.join(", ")
            )
            .unwrap();
            // Register the `[T; N]` type for EVERY body symbol resolving to this table's name — the
            // parser mints distinct symbols for the same identifier in different positions (a bare call
            // argument vs the `Let` binding), and `variable_types` is symbol-keyed, so a single `Let`-
            // symbol registration would miss at a use site. Registered LAST so it also wins over any
            // fn-return inference of the binding. (Const-table names are unique in the function body.)
            let ty = format!("[{}; {}]", info.elem_ty, info.values.len());
            let name = interner.resolve(*sym).to_string();
            let mut syms: HashSet<Symbol> = HashSet::new();
            collect_named_syms(body, &name, interner, &mut syms);
            for s in syms {
                func_ctx.register_variable_type(s, ty.clone());
            }
        }
        func_ctx.set_const_tables(func_const_tables);

        // Register the `[T; N]` type for every use-site symbol of each scratch buffer (same reason as the
        // constant tables above: `variable_types` is symbol-keyed and the parser mints distinct symbols
        // per occurrence). The fill loop is emitted in place as `from_fn` by the `Stmt::Repeat` handler;
        // only the `new Seq` DECL is filtered (the from_fn becomes the binding).
        for (sym, info) in &func_scratch {
            let ty = format!("[{}; {}]", info.elem_ty, info.len);
            let name = interner.resolve(*sym).to_string();
            let mut syms: HashSet<Symbol> = HashSet::new();
            collect_named_syms(body, &name, interner, &mut syms);
            for s in syms {
                func_ctx.register_variable_type(s, ty.clone());
            }
        }
        func_ctx.set_scratch_buffers(func_scratch);

        // Register the `[T; N]` type for every use-site symbol of each indexed buffer, so the O3 `[T; N]`
        // path emits `let mut buf: [T; N] = [0; N]` and the SetIndex/Index codegen store/read the array.
        for (sym, info) in &func_indexed {
            let ty = format!("[{}; {}]", info.elem_ty, info.len);
            let name = interner.resolve(*sym).to_string();
            let mut syms: HashSet<Symbol> = HashSet::new();
            collect_named_syms(body, &name, interner, &mut syms);
            for s in syms {
                func_ctx.register_variable_type(s, ty.clone());
            }
        }
        func_ctx.set_indexed_buffers(func_indexed);

        // Straight-line-push buffers: register `[T; K]` so the O3 `Let`/`Push` handlers emit the stack
        // array and `buf[k]=expr` fills. (init_array_fill runs inside the O3 `Let` handler.)
        for (sym, (elem_ty, len)) in &func_sl {
            let ty = format!("[{}; {}]", elem_ty, len);
            let name = interner.resolve(*sym).to_string();
            let mut syms: HashSet<Symbol> = HashSet::new();
            collect_named_syms(body, &name, interner, &mut syms);
            for s in syms {
                func_ctx.register_variable_type(s, ty.clone());
            }
        }

        // Cursor-filled buffers: register `[T; N]` + a fill cursor for every use-site symbol, so
        // the `Let` handler emits `let mut V: [T; N] = [<zero>; N]; let mut __V_fill = 0;` and each
        // `Push x to V` lowers to `V[__V_fill] = x; __V_fill += 1;` (the loop_fill_array path the
        // array-return digest already uses). The reads hit the `[T; N]` array via the registered type.
        for (sym, info) in &func_seq_fill {
            let ty = format!("[{}; {}]", info.elem_ty, info.len);
            let bufname = interner.resolve(*sym).to_string();
            let cursor = format!("__{}_fill", names.ident(*sym));
            let mut syms: HashSet<Symbol> = HashSet::new();
            collect_named_syms(body, &bufname, interner, &mut syms);
            for s in syms {
                func_ctx.register_variable_type(s, ty.clone());
                func_ctx.set_loop_fill_array(s, cursor.clone());
            }
        }

        // Step 3b: register the `[T; N]` type for this function's own fixed-size return buffer and for every
        // caller result variable bound to an array-return fn — for every name-matching symbol, so the O3
        // `[T; N]` scalarization (stack decl + indexed-write fill) and the array-aware call/borrow/iterate
        // codegen all dispatch on it. Registered here (after const-table/scratch) so nothing overwrites it.
        for (sym, ty) in super::affine_array::array_var_types(body, array_return, array_return_fns)
        {
            let name = interner.resolve(sym).to_string();
            let mut syms: HashSet<Symbol> = HashSet::new();
            collect_named_syms(body, &name, interner, &mut syms);
            for s in syms {
                func_ctx.register_variable_type(s, ty.clone());
            }
        }
        // A LOOP-built return buffer (the digest) fills its `[T; N]` through a runtime cursor. Register the
        // return buffer (the `Return out` identifier) so the `Let` declares the cursor and each `Push`
        // becomes `out[cursor]=v; cursor+=1` instead of the compile-time O3 fill (which only counts static
        // push sites — a loop would write slot 0 repeatedly).
        if array_return.map_or(false, |i| i.loop_built) {
            if let Some(Stmt::Return {
                value: Some(Expr::Identifier(out)),
            }) = body.last()
            {
                let cursor = format!("__{}_fill", names.ident(*out));
                let name = interner.resolve(*out).to_string();
                let mut syms: HashSet<Symbol> = HashSet::new();
                collect_named_syms(body, &name, interner, &mut syms);
                for s in syms {
                    func_ctx.set_loop_fill_array(s, cursor.clone());
                }
            }
        }

        // Leaf-outlining: a self-recursive `&mut [T]` in-place function keeps its
        // hot loop embedded in the recursion, where LLVM optimizes it worse than a
        // hand-written leaf `partition`. When eligible, the plain-body branch below
        // extracts the non-recursive work region into an `#[inline(never)]` leaf
        // helper, rendered here and prepended to the wrapper at return.
        let mut outlined_helper = String::new();

        if is_tce {
            // TCE: Wrap body in loop, use TCE-aware statement emitter
            writeln!(output, "    loop {{").unwrap();
            let stmt_refs: Vec<&Stmt> = body
                .iter()
                .filter(|s| {
                    !is_affine_array_decl(s, &func_ctx) && !is_const_table_stmt(s, &func_ctx)
                })
                .collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                // Self-tail-call PAIR: `Set/Let x = self(args); Return x` lowers to
                // the same loop-back as a direct `Return self(args)` (the binding
                // flows straight into the return). Matches the VM and tree-walker.
                if si + 1 < stmt_refs.len() {
                    if let Some(call_args) = crate::tail_call::tail_pair_args(
                        stmt_refs[si],
                        stmt_refs[si + 1],
                        name,
                        param_syms.len(),
                    ) {
                        output.push_str(&codegen_tce_loopback(
                            call_args,
                            &param_syms,
                            interner,
                            2,
                            &mut func_ctx,
                            &mut func_synced_vars,
                            async_functions,
                        ));
                        si += 2;
                        continue;
                    }
                }
                if !no_peephole {
                    if let Some((code, skip)) = super::peephole::try_block_peepholes(
                        &stmt_refs,
                        si,
                        interner,
                        2,
                        &func_mutable_vars,
                        &mut func_ctx,
                        lww_fields,
                        mv_fields,
                        &mut func_synced_vars,
                        &func_var_caps,
                        async_functions,
                        &func_pipe_vars,
                        boxed_fields,
                        registry,
                        type_env,
                    ) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                }
                output.push_str(&codegen_stmt_tce(
                    stmt_refs[si],
                    name,
                    &param_syms,
                    interner,
                    2,
                    &func_mutable_vars,
                    &mut func_ctx,
                    lww_fields,
                    mv_fields,
                    &mut func_synced_vars,
                    &func_var_caps,
                    async_functions,
                    &func_pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ));
                si += 1;
            }
            writeln!(output, "    }}").unwrap();
        } else if let Some(ref acc) = acc_info {
            // Accumulator Introduction: Wrap body in loop with accumulator variable
            if super::tower::returns(name) {
                writeln!(
                    output,
                    "    let mut __acc: {} = LogosValue::Int({}_i64);",
                    super::tower::VALUE,
                    acc.identity
                )
                .unwrap();
            } else {
                writeln!(output, "    let mut __acc: i64 = {};", acc.identity).unwrap();
            }
            writeln!(output, "    loop {{").unwrap();
            let stmt_refs: Vec<&Stmt> = body
                .iter()
                .filter(|s| {
                    !is_affine_array_decl(s, &func_ctx) && !is_const_table_stmt(s, &func_ctx)
                })
                .collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if !no_peephole {
                    if let Some((code, skip)) = super::peephole::try_block_peepholes(
                        &stmt_refs,
                        si,
                        interner,
                        2,
                        &func_mutable_vars,
                        &mut func_ctx,
                        lww_fields,
                        mv_fields,
                        &mut func_synced_vars,
                        &func_var_caps,
                        async_functions,
                        &func_pipe_vars,
                        boxed_fields,
                        registry,
                        type_env,
                    ) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                }
                output.push_str(&codegen_stmt_acc(
                    stmt_refs[si],
                    name,
                    &param_syms,
                    acc,
                    interner,
                    2,
                    &func_mutable_vars,
                    &mut func_ctx,
                    lww_fields,
                    mv_fields,
                    &mut func_synced_vars,
                    &func_var_caps,
                    async_functions,
                    &func_pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ));
                si += 1;
            }
            writeln!(output, "    }}").unwrap();
        } else if let Some(ref cf) = closed_form_info {
            // Closed-form: f(d) = ((base + k) << d) - k
            let param_name = names.ident(params[0].0);
            let base_plus_k = cf.base + cf.k;
            writeln!(
                output,
                "    if {} == 0 {{ return {}; }}",
                param_name, cf.base
            )
            .unwrap();
            // Multiply by 2^d with a shift that yields 0 once d reaches the i64
            // bit width, matching the recurrence's repeated WRAPPING doubling
            // (a raw `<< d` is UB in debug and masks the count in release for
            // d >= 64, diverging from the interpreter/VM).
            let pow2d = format!(
                "(if ({p} as u64) >= 64 {{ 0i64 }} else {{ 1i64 << {p} }})",
                p = param_name
            );
            if cf.k == 0 {
                writeln!(output, "    {}i64.wrapping_mul({})", base_plus_k, pow2d).unwrap();
            } else {
                writeln!(
                    output,
                    "    ({}i64.wrapping_mul({})).wrapping_sub({})",
                    base_plus_k, pow2d, cf.k
                )
                .unwrap();
            }
        } else if is_memo {
            // Memoization: Wrap body in closure with thread-local cache
            let ret_ty = return_type_str.as_deref().unwrap_or("i64");
            let memo_name = format!("__MEMO_{}", func_name.to_uppercase());

            // Build key type and key expression
            let (key_type, key_expr, copy_method) = if params.len() == 1 {
                let ty = codegen_type_expr(params[0].1, interner);
                let pname = interner.resolve(params[0].0).to_string();
                let copy = if is_copy_type_expr(params[0].1, interner) {
                    "copied"
                } else {
                    "cloned"
                };
                (ty, pname, copy)
            } else {
                let types: Vec<String> = params
                    .iter()
                    .map(|(_, t)| codegen_type_expr(t, interner))
                    .collect();
                let names: Vec<String> = params
                    .iter()
                    .map(|(n, _)| interner.resolve(*n).to_string())
                    .collect();
                let copy = if params.iter().all(|(_, t)| is_copy_type_expr(t, interner)) {
                    "copied"
                } else {
                    "cloned"
                };
                (
                    format!("({})", types.join(", ")),
                    format!("({})", names.join(", ")),
                    copy,
                )
            };

            writeln!(output, "    use std::cell::RefCell;").unwrap();
            writeln!(output, "    thread_local! {{").unwrap();
            writeln!(output, "        static {}: RefCell<FxHashMap<{}, {}>> = RefCell::new(FxHashMap::default());", memo_name, key_type, ret_ty).unwrap();
            writeln!(output, "    }}").unwrap();
            writeln!(
                output,
                "    if let Some(__v) = {}.with(|c| c.borrow().get(&{}).{}()) {{",
                memo_name, key_expr, copy_method
            )
            .unwrap();
            writeln!(output, "        return __v;").unwrap();
            writeln!(output, "    }}").unwrap();
            writeln!(output, "    let __memo_result = (|| -> {} {{", ret_ty).unwrap();
            let stmt_refs: Vec<&Stmt> = body
                .iter()
                .filter(|s| {
                    !is_affine_array_decl(s, &func_ctx) && !is_const_table_stmt(s, &func_ctx)
                })
                .collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if !no_peephole {
                    if let Some((code, skip)) = super::peephole::try_block_peepholes(
                        &stmt_refs,
                        si,
                        interner,
                        2,
                        &func_mutable_vars,
                        &mut func_ctx,
                        lww_fields,
                        mv_fields,
                        &mut func_synced_vars,
                        &func_var_caps,
                        async_functions,
                        &func_pipe_vars,
                        boxed_fields,
                        registry,
                        type_env,
                    ) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                }
                output.push_str(&codegen_stmt(
                    stmt_refs[si],
                    interner,
                    2,
                    &func_mutable_vars,
                    &mut func_ctx,
                    lww_fields,
                    mv_fields,
                    &mut func_synced_vars,
                    &func_var_caps,
                    async_functions,
                    &func_pipe_vars,
                    boxed_fields,
                    registry,
                    type_env,
                ));
                si += 1;
            }
            writeln!(output, "    }})();").unwrap();
            writeln!(
                output,
                "    {}.with(|c| c.borrow_mut().insert({}, __memo_result));",
                memo_name, key_expr
            )
            .unwrap();
            writeln!(output, "    __memo_result").unwrap();
        } else {
            let stmt_refs: Vec<&Stmt> = body
                .iter()
                .filter(|s| {
                    !is_affine_array_decl(s, &func_ctx) && !is_const_table_stmt(s, &func_ctx)
                })
                .collect();

            // Codegen a contiguous span `[lo, hi)` of `stmt_refs` through the exact
            // peephole+statement path the plain body uses, into `$target`. A
            // sub-slice bounds the peephole look-ahead to the span.
            macro_rules! emit_span {
                ($target:expr, $lo:expr, $hi:expr) => {{
                    let sub = &stmt_refs[$lo..$hi];
                    let mut k = 0usize;
                    while k < sub.len() {
                        if !no_peephole {
                            if let Some((code, skip)) = super::peephole::try_block_peepholes(
                                sub,
                                k,
                                interner,
                                1,
                                &func_mutable_vars,
                                &mut func_ctx,
                                lww_fields,
                                mv_fields,
                                &mut func_synced_vars,
                                &func_var_caps,
                                async_functions,
                                &func_pipe_vars,
                                boxed_fields,
                                registry,
                                type_env,
                            ) {
                                $target.push_str(&code);
                                k += 1 + skip;
                                continue;
                            }
                        }
                        $target.push_str(&codegen_stmt(
                            sub[k],
                            interner,
                            1,
                            &func_mutable_vars,
                            &mut func_ctx,
                            lww_fields,
                            mv_fields,
                            &mut func_synced_vars,
                            &func_var_caps,
                            async_functions,
                            &func_pipe_vars,
                            boxed_fields,
                            registry,
                            type_env,
                        ));
                        k += 1;
                    }
                }};
            }

            // Eligibility: a Borrow-lowered (`&mut [T]`) self-recursive function
            // matching the divide-and-conquer shape.
            let outline_plan = if !mut_borrow_indices.is_empty()
                && crate::optimize::active_config()
                    .merged(opt_flags)
                    .is_on(Opt::Borrow)
                && std::env::var("LOGOS_NO_OUTLINE").is_err()
            {
                mut_borrow_indices
                    .iter()
                    .min()
                    .and_then(|&i| params.get(i))
                    .and_then(|(slice_param, _)| {
                        super::outline::plan_outline(name, &stmt_refs, *slice_param)
                    })
            } else {
                None
            };

            if let Some(plan) = outline_plan {
                crate::optimize::mark_fired(Opt::Borrow);
                // The leaf helper's body IS the work region — rendered through the
                // ordinary path, so BCE and the counter de-guard land identically.
                // Repeat the entry precondition assert at the top of the leaf: the
                // hard `hi <= arr.len()` bound (not just the per-access
                // `assert_unchecked` hints) is what lets LLVM optimize the whole
                // loop, and the leaf is a separate function the wrapper's assert
                // cannot reach into.
                let mut helper_body = String::new();
                if let Some(g) = super::entry_guard::detect_entry_guard(params, body, interner) {
                    let arr = names.ident(g.arr);
                    let lo = names.ident(g.lo);
                    let hi = names.ident(g.hi);
                    let adj = g.hi_len_adjust;
                    writeln!(
                        helper_body,
                        "    if ({lo}) < ({hi}) {{ let __{arr}_glen = {arr}.len() as i64 + {adj}; assert!(({lo}) >= 1 && ({hi}) <= __{arr}_glen, \"LOGOS precondition guard: 1-based index range\"); }}"
                    )
                    .unwrap();
                }
                emit_span!(helper_body, plan.region_start, plan.region_end);
                // The wrapper: base-case guards, the helper call, the alias, then
                // the recursive calls. `func_ctx` already learned the region's
                // locals (`i`, `result`), so the trailing calls type-check.
                emit_span!(output, 0, plan.region_start);
                let liveouts: Vec<String> =
                    plan.live_outs.iter().map(|s| names.ident(*s)).collect();
                let args: Vec<String> = params.iter().map(|(s, _)| names.ident(*s)).collect();
                let bind = if liveouts.len() == 1 {
                    liveouts[0].clone()
                } else {
                    format!("({})", liveouts.join(", "))
                };
                writeln!(
                    output,
                    "    let {} = {}__part({});",
                    bind,
                    func_name,
                    args.join(", ")
                )
                .unwrap();
                for lo in &plan.live_outs {
                    func_ctx.register_variable_type(*lo, "i64".to_string());
                }
                if let Some(ai) = plan.alias_idx {
                    emit_span!(output, ai, ai + 1);
                }
                emit_span!(output, plan.region_end, stmt_refs.len());
                // Assemble the leaf (prepended before the wrapper at return). Same
                // parameter list as the wrapper (`&mut [T]`, scalars), returning the
                // live-outs — assumed `i64` (LOGOS `Int`); a wrong assumption is a
                // loud compile error, never silent corruption.
                let ret_ty = if liveouts.len() == 1 {
                    "i64".to_string()
                } else {
                    format!("({})", vec!["i64"; liveouts.len()].join(", "))
                };
                let ret_expr = if liveouts.len() == 1 {
                    liveouts[0].clone()
                } else {
                    format!("({})", liveouts.join(", "))
                };
                outlined_helper = format!(
                    "#[inline(never)]\nfn {}__part({}) -> {} {{\n{}    {}\n}}\n\n",
                    func_name,
                    params_str.join(", "),
                    ret_ty,
                    helper_body,
                    ret_expr
                );
            } else {
                emit_span!(output, 0, stmt_refs.len());
            }
        }

        if wrap_catch_unwind {
            writeln!(output, "    }})) {{").unwrap();
            writeln!(output, "        Ok(__v) => __v,").unwrap();
            writeln!(output, "        Err(__panic) => {{").unwrap();
            writeln!(output, "            let __msg = if let Some(s) = __panic.downcast_ref::<String>() {{ s.clone() }} else if let Some(s) = __panic.downcast_ref::<&str>() {{ s.to_string() }} else {{ \"Unknown panic\".to_string() }};").unwrap();
            writeln!(output, "            logos_set_last_error(__msg);").unwrap();
            // Determine default for panic case based on return type
            if let Some(ref ret_str) = return_type_str {
                if ret_str != "()" {
                    writeln!(output, "            Default::default()").unwrap();
                }
            }
            writeln!(output, "        }}").unwrap();
            writeln!(output, "    }}").unwrap();
        }

        writeln!(output, "}}\n").unwrap();

        // Emit the outlined leaf helper before its wrapper.
        if !outlined_helper.is_empty() {
            output = format!("{}{}", outlined_helper, output);
        }
    }

    output
}

/// Whether the `logicaffeine_system` function behind native `name` answers `Result<T, String>` and
/// its `Err` is a failure of the program, not a value: crypto.lg's kernels and the FIPS-202 hashes.
fn refuses_by_failing(name: &str) -> bool {
    crate::semantics::native_kernels::NativeKernel::from_name(name).is_some()
        || crate::semantics::builtins::rust_only_native(name).is_some()
}

/// Phase 38: Map native function names to logicaffeine_system module paths.
/// For system functions only — user-defined native paths bypass this entirely.
/// Returns None for unknown functions (caller emits compile_error!).
fn map_native_function(name: &str) -> Option<(&'static str, &'static str)> {
    // crypto.lg's kernels come from the ONE table every tier reads (the tree-walker and VM call
    // the same functions through `NativeKernel::call`).
    if let Some(kernel) = crate::semantics::native_kernels::NativeKernel::from_name(name) {
        return Some(kernel.rust_path());
    }
    if let Some(linked) = crate::semantics::builtins::rust_only_native(name) {
        return Some(linked);
    }
    match name {
        "read" => Some(("file", "read")),
        "write" => Some(("file", "write")),
        "tcpConnect" => Some(("tcp", "connect")),
        "tcpConnectTls" => Some(("tcp", "connect_tls")),
        "tcpUpgradeTls" => Some(("tcp", "upgrade_tls")),
        "tcpSend" => Some(("tcp", "send")),
        "tcpRecv" => Some(("tcp", "recv")),
        "tcpRecvExact" => Some(("tcp", "recv_exact")),
        "tcpClose" => Some(("tcp", "close")),
        "tcpSetTimeout" => Some(("tcp", "set_timeout")),
        "tcpListen" => Some(("tcp", "listen")),
        "tcpAccept" => Some(("tcp", "accept")),
        "tcpListenerPort" => Some(("tcp", "listener_port")),
        "tcpShutdownWrite" => Some(("tcp", "shutdown_write")),
        "now" => Some(("time", "now")),
        "sleep" => Some(("time", "sleep")),
        "randomInt" => Some(("random", "randomInt")),
        "randomFloat" => Some(("random", "randomFloat")),
        "get" => Some(("env", "get")),
        "args" => Some(("env", "args")),
        "parseInt" => Some(("text", "parseInt")),
        "parseFloat" => Some(("text", "parseFloat")),
        "chr" => Some(("text", "chr")),
        "format" => Some(("fmt", "format")),
        _ => None,
    }
}
