use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::registry::{FieldDef, TypeDef, TypeRegistry};
use crate::analysis::policy::PolicyRegistry;
use crate::analysis::types::RustNames;
use crate::ast::stmt::{Expr, OptFlag, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::registry::SymbolRegistry;

use super::context::{RefinementContext, VariableCapabilities, analyze_variable_capabilities};
use crate::analysis::callgraph::CallGraph;

/// Does the program call `count_ones` (inserted by `optimize::popcount_leaf`)?
/// Gates emission of the `count_ones` helper so programs that don't use it have
/// byte-identical codegen (keeps the codegen snapshot tests stable).
fn program_uses_count_ones(stmts: &[Stmt], interner: &Interner) -> bool {
    fn in_expr(e: &Expr, it: &Interner) -> bool {
        match e {
            Expr::Call { function, args } => {
                it.resolve(*function) == "count_ones" || args.iter().any(|a| in_expr(a, it))
            }
            Expr::BinaryOp { left, right, .. } => in_expr(left, it) || in_expr(right, it),
            Expr::Not { operand } => in_expr(operand, it),
            Expr::Index { collection, index } => in_expr(collection, it) || in_expr(index, it),
            Expr::Length { collection } => in_expr(collection, it),
            _ => false,
        }
    }
    fn in_block(b: &[Stmt], it: &Interner) -> bool {
        b.iter().any(|s| match s {
            Stmt::Let { value, .. } | Stmt::Set { value, .. } => in_expr(value, it),
            Stmt::Return { value } => value.map_or(false, |e| in_expr(e, it)),
            Stmt::If { cond, then_block, else_block } => {
                in_expr(cond, it)
                    || in_block(then_block, it)
                    || else_block.map_or(false, |b| in_block(b, it))
            }
            Stmt::While { cond, body, .. } => in_expr(cond, it) || in_block(body, it),
            Stmt::FunctionDef { body, .. } => in_block(body, it),
            _ => false,
        })
    }
    in_block(stmts, interner)
}
use crate::analysis::liveness::LivenessResult;
use crate::analysis::readonly::{ReadonlyParams, MutableBorrowParams};

use super::detection::{
    requires_async, requires_vfs, collect_mutable_vars,
    collect_crdt_register_fields, collect_boxed_fields, collect_async_functions,
    collect_pure_functions, count_self_calls, is_hashable_type, is_copy_type_expr,
    should_memoize, body_contains_self_call, should_inline,
    collect_pipe_sender_params, collect_pipe_vars,
    collect_mutable_vars_stmt, is_result_type,
    vec_to_slice_type, vec_to_mut_slice_type, collect_give_arg_indices, is_vec_type_expr,
    collect_single_char_text_vars, collect_escaping_collection_vars,
    collect_scalarizable_seqs, collect_interleaved_groups, collect_de_rc_seqs, collect_vec_return_fns,
    detect_double_recursion_closed_form,
};
use super::expr::{codegen_expr, codegen_expr_with_async};
use super::ffi::{
    has_wasm_exports, has_c_exports, has_c_exports_with_text,
    codegen_logos_runtime_preamble, collect_c_export_reference_types,
    collect_c_export_value_type_structs,
};
use super::marshal::{is_text_type, is_char_type, codegen_c_export_with_marshaling};
use super::policy::codegen_policy_impls;
use super::stmt::codegen_stmt;
use super::tce::{
    is_tail_recursive, body_has_top_level_tail_pair, codegen_tce_loopback,
    codegen_stmt_acc,
    detect_mutual_tce_pairs, codegen_mutual_tce_pair, codegen_stmt_tce,
};
use crate::tail_call::detect_accumulator_pattern;
use super::types::{
    codegen_type_expr, infer_return_type_from_body,
    codegen_struct_def, codegen_enum_def,
};
use super::{escape_rust_ident, is_rust_keyword};
use super::{
    collect_c_export_ref_structs, codegen_c_accessors,
    try_emit_vec_fill_pattern, try_emit_for_range_pattern, try_emit_swap_pattern,
    try_emit_prefix_reverse,
    try_emit_seq_copy_pattern, try_emit_seq_from_slice_pattern,
    try_emit_bare_slice_push_pattern,
    try_emit_vec_with_capacity_pattern, try_emit_merge_capacity_pattern,
    try_emit_string_with_capacity_pattern,
    try_emit_rotate_left_pattern,
    try_emit_buffer_reuse_while,
    classify_type_for_c_abi, CAbiClass,
};

/// Check if a function body contains escape blocks (raw Rust code).
/// Functions with escape blocks should not have their param types changed
/// by borrow optimization, since the escape code may depend on specific types.
fn body_contains_escape(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| stmt_contains_escape(stmt))
}

fn stmt_contains_escape(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Escape { .. } => true,
        Stmt::Let { value, .. } => expr_contains_escape(value),
        Stmt::Set { value, .. } => expr_contains_escape(value),
        Stmt::If { then_block, else_block, .. } => {
            body_contains_escape(then_block)
                || else_block.as_ref().map_or(false, |eb| body_contains_escape(eb))
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
    if std::env::var_os("LOGOS_NO_NARROW").is_some() {
        return HashMap::new();
    }
    let mut n = super::narrow::detect_narrowable(body, de_rc, interner);
    n.retain(|sym, _| !affine.contains_key(sym) && !worklists.contains_key(sym));
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

pub fn codegen_program(stmts: &[Stmt], registry: &TypeRegistry, policies: &PolicyRegistry, interner: &Interner, type_env: &crate::analysis::types::TypeEnv) -> String {
    let mut output = String::new();

    // Prelude
    // Use extracted crates instead of logos_core
    writeln!(output, "#[allow(unused_imports)]").unwrap();
    writeln!(output, "use std::fmt::Write as _;").unwrap();
    writeln!(output, "use logicaffeine_data::*;").unwrap();
    writeln!(output, "use logicaffeine_system::*;\n").unwrap();
    // Population-count intrinsic — emitted ONLY when the program calls it (so
    // unrelated codegen is byte-identical). A free function so every codegen
    // path renders `count_ones(x)` as a plain call; two's-complement faithful,
    // matching the tree-walker/VM builtin (optimize::popcount_leaf inserts it).
    if program_uses_count_ones(stmts, interner) {
        writeln!(
            output,
            "#[inline(always)]\nfn count_ones(x: i64) -> i64 {{ (x as u64).count_ones() as i64 }}\n"
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

    // Phase 49: Collect CRDT register fields for special SetField handling
    // LWW fields need timestamp, MV fields don't
    let (lww_fields, mv_fields) = collect_crdt_register_fields(registry, interner);

    // Phase 54: Collect async functions for Launch codegen
    let async_functions = collect_async_functions(stmts);

    // Purity analysis for memoization
    let pure_functions = collect_pure_functions(stmts);

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
    let structs: Vec<_> = registry.iter_types()
        .filter_map(|(name, def)| {
            if let TypeDef::Struct { fields, generics, is_portable, is_shared } = def {
                if !fields.is_empty() || !generics.is_empty() {
                    Some((*name, fields.clone(), generics.clone(), *is_portable, *is_shared))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Phase 33/34: Collect user-defined enums from registry (generics, Phase 47: is_portable, Phase 49: is_shared)
    let enums: Vec<_> = registry.iter_types()
        .filter_map(|(name, def)| {
            if let TypeDef::Enum { variants, generics, is_portable, is_shared } = def {
                if !variants.is_empty() || !generics.is_empty() {
                    Some((*name, variants.clone(), generics.clone(), *is_portable, *is_shared))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();

    // Emit struct and enum definitions in user_types module if any exist
    if !structs.is_empty() || !enums.is_empty() {
        writeln!(output, "pub mod user_types {{").unwrap();
        writeln!(output, "    use super::*;\n").unwrap();

        for (name, fields, generics, is_portable, is_shared) in &structs {
            output.push_str(&codegen_struct_def(*name, fields, generics, *is_portable, *is_shared, interner, 4, &c_abi_value_structs, &c_abi_ref_structs));
        }

        for (name, variants, generics, is_portable, is_shared) in &enums {
            output.push_str(&codegen_enum_def(*name, variants, generics, *is_portable, *is_shared, interner, 4));
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

    let mut borrow_params_map: HashMap<Symbol, HashSet<usize>> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, is_native, is_exported, opt_flags, .. } = stmt {
            // Skip native, exported, TCE, accumulator, and mutual TCE functions
            if *is_native || *is_exported || mutual_tce_members.contains(name) {
                continue;
            }
            // Respect ## No Borrow / ## No Optimize annotations
            if opt_flags.contains(&OptFlag::NoBorrow) || opt_flags.contains(&OptFlag::NoOptimize) {
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
            let indices: HashSet<usize> = params.iter().enumerate()
                .filter(|(_, (sym, param_type))| {
                    readonly_params.is_readonly(*name, *sym)
                        && is_vec_type_expr(param_type, interner)
                })
                .map(|(i, _)| i)
                .collect();
            if !indices.is_empty() {
                let give_indices = collect_give_arg_indices(*name, stmts);
                let filtered: HashSet<usize> = indices.difference(&give_indices).copied().collect();
                if !filtered.is_empty() {
                    borrow_params_map.insert(*name, filtered);
                }
            }
        }
    }

    // Mutable borrow analysis: detect Seq params that are only mutated via SetIndex
    // (element-only, no Push/Pop) and returned. These get &mut [T] instead of owned Vec<T>.
    let mutable_borrow_params = MutableBorrowParams::analyze(stmts, &callgraph, type_env);

    let mut mut_borrow_params_map: HashMap<Symbol, HashSet<usize>> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, is_native, is_exported, opt_flags, .. } = stmt {
            if *is_native || *is_exported || mutual_tce_members.contains(name) {
                continue;
            }
            // Respect ## No Borrow / ## No Optimize annotations
            if opt_flags.contains(&OptFlag::NoBorrow) || opt_flags.contains(&OptFlag::NoOptimize) {
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
            let indices: HashSet<usize> = params.iter().enumerate()
                .filter(|(i, (sym, param_type))| {
                    mutable_borrow_params.is_mutable_borrow(*name, *sym)
                        && !readonly_indices.contains(i)
                        && is_vec_type_expr(param_type, interner)
                })
                .map(|(i, _)| i)
                .collect();
            if !indices.is_empty() {
                mut_borrow_params_map.insert(*name, indices);
            }
        }
    }

    // Pass 2: Compute liveness for all user-defined functions (backward dataflow).
    // Used by codegen_function_def to enable last-use move optimization (OPT-1C).
    let liveness = LivenessResult::analyze(stmts);

    // De-Rc Phase 4: functions whose every Return is a uniquely-owned fresh Seq
    // return `Vec<T>` instead of `LogosSeq<T>` — removing the per-call Rc
    // clone/borrow and unblocking de-Rc on the locals that capture the result
    // (`Set left to mergeSort(left)`). The mergesort allocation keystone.
    let vec_return_fns = collect_vec_return_fns(stmts, interner, &borrow_params_map, &mut_borrow_params_map);

    // Build function return type map for variable type inference at call sites.
    // Phase 4: a return-type-de-Rc'd function returns `Vec<T>`, so callers infer
    // the result var as `Vec` (not `LogosSeq`) — keeping its uses Rc-free.
    let fn_returns_map: HashMap<Symbol, String> = stmts.iter().filter_map(|s| {
        if let Stmt::FunctionDef { name, return_type: Some(rt), .. } = s {
            let ty = codegen_type_expr(rt, interner);
            let ty = if vec_return_fns.contains(name) {
                ty.replacen("LogosSeq<", "Vec<", 1)
            } else {
                ty
            };
            Some((*name, ty))
        } else {
            None
        }
    }).collect();

    // O1 borrow hoisting: analyze the oracle ONCE on this exact statement
    // slice (loop alias snapshots are keyed by Stmt address; codegen walks
    // the same Stmts, so the pointers match). One forward pass with bounded
    // per-loop fixpoints; gated for very large programs and by LOGOS_HOIST=0.
    const MAX_HOIST_ORACLE_STMTS: usize = 5_000;
    let oracle: Option<std::rc::Rc<crate::optimize::OracleFacts>> =
        if super::hoist::hoisting_disabled() || stmts.len() > MAX_HOIST_ORACLE_STMTS {
            None
        } else {
            Some(std::rc::Rc::new(crate::optimize::oracle_analyze_with_entry_guards(stmts, interner)))
        };

    // Phase 32/38: Emit function definitions before main
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } = stmt {
            if mutual_tce_members.contains(name) {
                // Part of a mutual pair — emit merged function when we see the first member
                if !mutual_tce_emitted.contains(name) {
                    // Find the pair this function belongs to
                    if let Some((a, b)) = mutual_tce_pairs.iter().find(|(a, b)| *a == *name || *b == *name) {
                        output.push_str(&codegen_mutual_tce_pair(*a, *b, stmts, interner, &lww_fields, &mv_fields, &async_functions, &boxed_fields, registry, type_env));
                        mutual_tce_emitted.insert(*a);
                        mutual_tce_emitted.insert(*b);
                    }
                }
                // Skip individual emission — already emitted as part of merged pair
            } else {
                output.push_str(&codegen_function_def(*name, generics, params, body, return_type.as_ref().copied(), *is_native, *native_path, *is_exported, *export_target, interner, &lww_fields, &mv_fields, &async_functions, &boxed_fields, registry, &pure_functions, type_env, &borrow_params_map, &mut_borrow_params_map, &liveness, opt_flags, &fn_returns_map, &vec_return_fns, oracle.as_ref()));
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
    let main_stmts: Vec<&Stmt> = stmts.iter()
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
    if requires_async(stmts) {
        writeln!(output, "#[tokio::main]").unwrap();
        writeln!(output, "async fn main() {{").unwrap();
    } else {
        writeln!(output, "fn main() {{").unwrap();
        writeln!(output, "    std::thread::Builder::new()").unwrap();
        writeln!(output, "        .stack_size(67_108_864)").unwrap();
        writeln!(output, "        .spawn(_logos_main)").unwrap();
        writeln!(output, "        .unwrap().join().unwrap();").unwrap();
        writeln!(output, "}}").unwrap();
        writeln!(output, "fn _logos_main() {{").unwrap();
    }
    // Phase 53: Inject VFS when file operations or persistence is used
    if requires_vfs(stmts) {
        writeln!(output, "    let vfs: std::sync::Arc<dyn logicaffeine_system::fs::Vfs + Send + Sync> = std::sync::Arc::from(logicaffeine_system::fs::get_platform_vfs());").unwrap();
    }
    let mut main_ctx = RefinementContext::from_type_env(type_env);
    // O1 borrow hoisting: seed the oracle (pointer-keyed loop alias
    // snapshots must match the Stmts codegen walks).
    if let Some(o) = &oracle {
        main_ctx.set_oracle(o.clone());
    }
    // Local Vec optimization: detect which collection vars escape main
    let main_escaping = collect_escaping_collection_vars(stmts, interner);
    main_ctx.set_escaping_vars(main_escaping);
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
                format!("__aos:{}:{}:{}:{}:{}", backing, col, width, group.len, group.elem_ty),
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
    // O2 de-Rc: Seqs proven to never need reference semantics → plain Vec<T>
    // (no Rc/RefCell). A var already scalarized to `[T; N]` (O3) wins — exclude it.
    let mut de_rc_seqs = collect_de_rc_seqs(stmts, interner, &borrow_params_map, &mut_borrow_params_map, &vec_return_fns, false);
    for (sym, _) in &scalarizable_seqs {
        de_rc_seqs.remove(sym);
    }
    // Append-only worklist → pre-sized buffer + register tail (BFS frontier).
    let main_worklists = super::worklist::detect_worklists(stmts, &de_rc_seqs, interner);
    // Affine read-only array → delete it, substitute the closed form at reads
    // (CSR offset array `adjStarts[v] == v*5` becomes C's `v*5` shift). An O3
    // scalarized (`[T;N]`) or worklist symbol is already claimed — exclude it.
    let mut main_affine = super::affine_array::detect_affine_arrays(stmts, &de_rc_seqs, interner);
    main_affine.retain(|sym, _| {
        !scalarizable_seqs.contains_key(sym) && !main_worklists.contains_key(sym)
    });
    for (sym, info) in &main_affine {
        main_ctx.register_variable_type(
            *sym,
            format!("__affine_array:{}:{}:{}", info.coeff, info.offset, info.trip),
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
    main_ctx.set_de_rc_vars(de_rc_seqs);
    main_ctx.set_worklists(main_worklists);
    main_ctx.set_affine_arrays(main_affine);
    main_ctx.set_narrowed(main_narrowed);
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
    main_ctx.set_fast_div(super::fast_div::detect_fast_div(stmts, oracle.as_deref(), interner));
    // Register function borrow info on main context for call-site optimization
    for (fn_sym, indices) in &borrow_params_map {
        let indices_str = indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        main_ctx.register_variable_type(*fn_sym, format!("fn_borrow:{}", indices_str));
    }
    // Register mutable borrow info for call-site transformation
    for (fn_sym, indices) in &mut_borrow_params_map {
        let indices_str = indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        main_ctx.register_variable_type(*fn_sym, format!("fn_mut_borrow:{}", indices_str));
    }
    // Register function return types for variable type inference at call sites
    for (fn_sym, rt_str) in &fn_returns_map {
        main_ctx.register_fn_return(*fn_sym, rt_str.clone());
    }
    let mut main_synced_vars = HashSet::new();  // Phase 52: Track synced variables in main
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
            if let Some((code, skip)) = super::peephole::try_block_peepholes(&stmt_refs, i, interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mv_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars, &boxed_fields, registry, type_env) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            output.push_str(&codegen_stmt(stmt_refs[i], interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mv_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars, &boxed_fields, registry, type_env));
            i += 1;
        }
    }
    writeln!(output, "}}").unwrap();
    output
}

/// Phase 32/38: Generate a function definition.
/// Phase 38: Updated for native functions and TypeExpr types.
/// Phase 49: Accepts lww_fields for LWWRegister SetField handling.
/// Phase 103: Accepts registry for polymorphic enum type inference.
fn codegen_function_def(
    name: Symbol,
    generics: &[Symbol],
    params: &[(Symbol, &TypeExpr)],
    body: &[Stmt],
    return_type: Option<&TypeExpr>,
    is_native: bool,
    native_path: Option<Symbol>,
    is_exported: bool,
    export_target: Option<Symbol>,
    interner: &Interner,
    lww_fields: &HashSet<(String, String)>,
    mv_fields: &HashSet<(String, String)>,  // Phase 49b: MVRegister fields
    async_functions: &HashSet<Symbol>,  // Phase 54
    boxed_fields: &HashSet<(String, String, String)>,  // Phase 102
    registry: &TypeRegistry,  // Phase 103
    pure_functions: &HashSet<Symbol>,
    type_env: &crate::analysis::types::TypeEnv,
    borrow_params_map: &HashMap<Symbol, HashSet<usize>>,
    mut_borrow_params_map: &HashMap<Symbol, HashSet<usize>>,
    liveness: &LivenessResult,
    opt_flags: &HashSet<OptFlag>,
    fn_returns_map: &HashMap<Symbol, String>,
    vec_return_fns: &HashSet<Symbol>,
    oracle: Option<&std::rc::Rc<crate::optimize::OracleFacts>>,
) -> String {
    let mut output = String::new();
    // De-Rc Phase 4: this function returns an owned `Vec<T>` (every Return is a
    // uniquely-owned fresh Seq) instead of `LogosSeq<T>`.
    let returns_vec = vec_return_fns.contains(&name);
    let names = RustNames::new(interner);
    let raw_name = names.raw(name);
    let func_name = names.ident(name);
    let export_target_lower = export_target.map(|s| interner.resolve(s).to_lowercase());

    // Phase 54: Detect which parameters are used as pipe senders
    let pipe_sender_params = collect_pipe_sender_params(body);

    // FFI: Exported functions need special signatures
    let is_c_export_early = is_exported && matches!(export_target_lower.as_deref(), None | Some("c"));

    // TCE: Detect tail recursion eligibility (respects ## No TCO / ## No Optimize)
    let no_tco = opt_flags.contains(&OptFlag::NoTCO) || opt_flags.contains(&OptFlag::NoOptimize);
    // A direct `Return self(args)` always TCE's. The `Set/Let x = self(args);
    // Return x` pair TCE's too — UNLESS the function is borrow/mut-borrow-eligible
    // (an in-place array recursion like quicksort), where that rewrite is the
    // better lowering and pair-TCE would force an owned, cloned parameter. Such
    // functions recurse only O(log n) deep, so the constant-stack guarantee is
    // moot for them anyway.
    let pair_tce_ok = body_has_top_level_tail_pair(name, body, params.len())
        && !borrow_params_map.contains_key(&name)
        && !mut_borrow_params_map.contains_key(&name);
    let is_tce = !is_native && !is_c_export_early && !no_tco
        && (is_tail_recursive(name, body) || pair_tce_ok);
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
    let closed_form_info = if !is_tce && !is_acc && !is_native && !is_c_export_early {
        detect_double_recursion_closed_form(name, params, body, interner)
    } else {
        None
    };
    let is_closed_form = closed_form_info.is_some();

    // Memoization: Detect pure multi-call recursive functions with hashable params
    // Respects ## No Memo / ## No Optimize annotations
    let no_memo = opt_flags.contains(&OptFlag::NoMemo) || opt_flags.contains(&OptFlag::NoOptimize);
    let is_memo = !is_tce && !is_acc && !is_closed_form && !is_native && !is_c_export_early && !no_memo
        && should_memoize(name, body, params, return_type, pure_functions.contains(&name), interner);

    let needs_mut_params = is_tce || is_acc;

    // Peephole: respect ## No Peephole / ## No Optimize annotations
    let no_peephole = opt_flags.contains(&OptFlag::NoPeephole) || opt_flags.contains(&OptFlag::NoOptimize);

    // Get borrow indices for this function (empty set if none)
    let borrow_indices = borrow_params_map.get(&name).cloned().unwrap_or_default();

    // Get mutable borrow indices (element-only mutation, return same param)
    let mut_borrow_indices = mut_borrow_params_map.get(&name).cloned().unwrap_or_default();

    // Compute mutable vars early for param mutability detection
    let func_mutable_vars = collect_mutable_vars(body);

    // Build parameter list using TypeExpr
    let params_str: Vec<String> = params.iter().enumerate()
        .map(|(i, (param_name, param_type))| {
            let pname = names.ident(*param_name);
            let ty = codegen_type_expr(param_type, interner);
            // Phase 54: If param is used as a pipe sender, wrap type in Sender<T>
            if pipe_sender_params.contains(param_name) {
                format!("{}: tokio::sync::mpsc::Sender<{}>", pname, ty)
            } else if borrow_indices.contains(&i) {
                // Read-only Vec param → borrow as &[T]
                let slice_ty = vec_to_slice_type(&ty);
                format!("{}: {}", pname, slice_ty)
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
    let return_type_str = if has_mut_borrow {
        None // No return type — mutation is in-place via &mut [T]
    } else if returns_vec {
        // Phase 4: `LogosSeq<T>` → owned `Vec<T>`. Replace the `LogosSeq<` head
        // of the codegen'd type (`LogosSeq<i64>` → `Vec<i64>`).
        return_type
            .map(|t| codegen_type_expr(t, interner).replacen("LogosSeq<", "Vec<", 1))
    } else {
        return_type
            .map(|t| codegen_type_expr(t, interner))
            .or_else(|| infer_return_type_from_body(body, interner))
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
        let has_refinement_param = params.iter().any(|(_, ty)| {
            matches!(ty, TypeExpr::Refinement { .. })
        });
        // Char crosses the C ABI as uint32_t; route through the marshal path so
        // the wrapper validates it via char::from_u32 instead of exposing a raw
        // Rust `char` (an out-of-range u32 from C is UB).
        let has_char_param = params.iter().any(|(_, ty)| is_char_type(ty, interner));
        let has_char_return = return_type.map_or(false, |ty| is_char_type(ty, interner));
        has_text_param || has_text_return || has_ref_param || has_ref_return
            || has_result_return || has_refinement_param || has_char_param || has_char_return
    };

    if needs_c_marshaling {
        // Generate two-function pattern: inner function + C ABI wrapper
        return codegen_c_export_with_marshaling(
            name, params, body, return_type, interner,
            lww_fields, mv_fields, async_functions, boxed_fields, registry, type_env,
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

    // Generic functions: "fn identity<T>" — resolve each generic Symbol to its name
    let generics_str = if generics.is_empty() {
        String::new()
    } else {
        let params_list: Vec<&str> = generics.iter()
            .map(|sym| interner.resolve(*sym))
            .collect();
        format!("<{}>", params_list.join(", "))
    };

    let signature = if let Some(ref ret_ty) = return_type_str {
        if ret_ty != "()" {
            format!("{}{}{} {}{}({}) -> {}", vis_prefix, abi_prefix, fn_keyword, func_name, generics_str, params_str.join(", "), ret_ty)
        } else {
            format!("{}{}{} {}{}({})", vis_prefix, abi_prefix, fn_keyword, func_name, generics_str, params_str.join(", "))
        }
    } else {
        format!("{}{}{} {}{}({})", vis_prefix, abi_prefix, fn_keyword, func_name, generics_str, params_str.join(", "))
    };

    // Emit #[inline] for small non-recursive, non-exported functions
    // Closed-form functions are tiny (2 lines) and should always inline.
    if is_closed_form || (!is_tce && !is_acc && should_inline(name, body, is_native, is_exported, is_async)) {
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
        let arg_names: Vec<&str> = params.iter()
            .map(|(n, _)| interner.resolve(*n))
            .collect();

        if let Some(path_sym) = native_path {
            // User-defined native path: call the Rust path directly
            let path = interner.resolve(path_sym);
            // Validate path looks like a valid Rust path (identifiers separated by ::)
            let is_valid_path = !path.is_empty() && path.split("::").all(|seg| {
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
                writeln!(output, "    logicaffeine_system::{}::{}({})", module, core_fn, arg_names.join(", ")).unwrap();
                writeln!(output, "}}\n").unwrap();
            } else {
                writeln!(output, "{} {{", signature).unwrap();
                writeln!(output, "    compile_error!(\"Unknown system native function: '{}'. Use `is native \\\"crate::path\\\"` syntax for user-defined native functions.\")", raw_name).unwrap();
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
        // abort is equivalent to the out-of-range access it pre-empts.
        if !is_tce && !is_acc {
            if let Some(g) = super::entry_guard::detect_entry_guard(params, body, interner) {
                let arr = names.ident(g.arr);
                let lo = names.ident(g.lo);
                let hi = names.ident(g.hi);
                writeln!(
                    output,
                    "    if ({lo}) < ({hi}) {{ let __{arr}_glen = {arr}.len() as i64; assert!(({lo}) >= 1 && ({hi}) <= __{arr}_glen, \"LOGOS precondition guard: 1-based index range\"); }}"
                )
                .unwrap();
            }
        }

        // Wrap exported C functions in catch_unwind for panic safety
        let wrap_catch_unwind = is_c_export;
        if wrap_catch_unwind {
            writeln!(output, "    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {{").unwrap();
        }

        let mut func_ctx = RefinementContext::new();
        // O1 borrow hoisting: same oracle, keyed by the loop Stmts in this
        // body (sub-slice of the program codegen analyzed).
        if let Some(o) = oracle {
            func_ctx.set_oracle(o.clone());
        }
        // Local Vec optimization: detect which collection vars escape this function
        let func_escaping = collect_escaping_collection_vars(body, interner);
        func_ctx.set_escaping_vars(func_escaping);
        // O2 de-Rc: function-LOCAL Seqs that never need reference semantics →
        // plain Vec<T>. The use-scan disqualifies any returned/escaping/aliased
        // handle, so only genuinely-local buffers (sieve flags, scratch arrays)
        // de-Rc. Parameters are not candidates (Phase 3 handles those).
        let func_de_rc = collect_de_rc_seqs(body, interner, borrow_params_map, mut_borrow_params_map, vec_return_fns, returns_vec);
        let func_worklists = super::worklist::detect_worklists(body, &func_de_rc, interner);
        let mut func_affine = super::affine_array::detect_affine_arrays(body, &func_de_rc, interner);
        func_affine.retain(|sym, _| !func_worklists.contains_key(sym));
        for (sym, info) in &func_affine {
            func_ctx.register_variable_type(
                *sym,
                format!("__affine_array:{}:{}:{}", info.coeff, info.offset, info.trip),
            );
        }
        let func_narrowed = narrow_seqs(body, &func_de_rc, &func_affine, &func_worklists, interner);
        for sym in func_narrowed.keys() {
            func_ctx.register_variable_type(*sym, "Vec<i32>".to_string());
        }
        func_ctx.set_de_rc_vars(func_de_rc);
        func_ctx.set_worklists(func_worklists);
        func_ctx.set_affine_arrays(func_affine);
        func_ctx.set_narrowed(func_narrowed);
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
        func_presize.retain(|sym, _| func_ctx.affine_array(*sym).is_none());
        func_ctx.set_vec_presize(func_presize);
        func_ctx.set_fast_div(super::fast_div::detect_fast_div(body, oracle.map(|o| o.as_ref()), interner));
        func_ctx.set_returns_vec(returns_vec);
        // OPT: Detect single-char text vars in function body
        let func_single_char_vars = collect_single_char_text_vars(body, interner);
        for sym in &func_single_char_vars {
            func_ctx.register_variable_type(*sym, "__single_char_u8".to_string());
        }
        let mut func_synced_vars = HashSet::new();  // Phase 52: Track synced variables in function
        // Phase 56: Pre-scan for Mount+Sync combinations in function body
        let func_var_caps = analyze_variable_capabilities(body, interner);

        // Phase 50: Register parameter types for capability Check resolution
        // Borrow-optimized params get &[T] or &mut [T] type so downstream codegen handles them correctly
        for (i, (param_name, param_type)) in params.iter().enumerate() {
            let type_name = codegen_type_expr(param_type, interner);
            if borrow_indices.contains(&i) {
                func_ctx.register_variable_type(*param_name, vec_to_slice_type(&type_name));
            } else if mut_borrow_indices.contains(&i) {
                func_ctx.register_variable_type(*param_name, vec_to_mut_slice_type(&type_name));
            } else {
                func_ctx.register_variable_type(*param_name, type_name);
            }
        }

        // Register function borrow info on func context for call-site optimization
        for (fn_sym, indices) in borrow_params_map {
            let indices_str = indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            func_ctx.register_variable_type(*fn_sym, format!("fn_borrow:{}", indices_str));
        }
        for (fn_sym, indices) in mut_borrow_params_map {
            let indices_str = indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            func_ctx.register_variable_type(*fn_sym, format!("fn_mut_borrow:{}", indices_str));
        }
        // Register function return types for variable type inference at call sites
        for (fn_sym, rt_str) in fn_returns_map {
            func_ctx.register_fn_return(*fn_sym, rt_str.clone());
        }

        // Phase 54: Functions receive pipe senders as parameters, no local pipe declarations
        let func_pipe_vars = HashSet::new();

        if is_tce {
            // TCE: Wrap body in loop, use TCE-aware statement emitter
            writeln!(output, "    loop {{").unwrap();
            let stmt_refs: Vec<&Stmt> = body.iter().filter(|s| !is_affine_array_decl(s, &func_ctx)).collect();
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
                    if let Some((code, skip)) = super::peephole::try_block_peepholes(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                }
                output.push_str(&codegen_stmt_tce(stmt_refs[si], name, &param_syms, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env));
                si += 1;
            }
            writeln!(output, "    }}").unwrap();
        } else if let Some(ref acc) = acc_info {
            // Accumulator Introduction: Wrap body in loop with accumulator variable
            writeln!(output, "    let mut __acc: i64 = {};", acc.identity).unwrap();
            writeln!(output, "    loop {{").unwrap();
            let stmt_refs: Vec<&Stmt> = body.iter().filter(|s| !is_affine_array_decl(s, &func_ctx)).collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if !no_peephole {
                    if let Some((code, skip)) = super::peephole::try_block_peepholes(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                }
                output.push_str(&codegen_stmt_acc(stmt_refs[si], name, &param_syms, acc, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env));
                si += 1;
            }
            writeln!(output, "    }}").unwrap();
        } else if let Some(ref cf) = closed_form_info {
            // Closed-form: f(d) = ((base + k) << d) - k
            let param_name = names.ident(params[0].0);
            let base_plus_k = cf.base + cf.k;
            writeln!(output, "    if {} == 0 {{ return {}; }}", param_name, cf.base).unwrap();
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
                writeln!(output, "    ({}i64.wrapping_mul({})).wrapping_sub({})", base_plus_k, pow2d, cf.k).unwrap();
            }
        } else if is_memo {
            // Memoization: Wrap body in closure with thread-local cache
            let ret_ty = return_type_str.as_deref().unwrap_or("i64");
            let memo_name = format!("__MEMO_{}", func_name.to_uppercase());

            // Build key type and key expression
            let (key_type, key_expr, copy_method) = if params.len() == 1 {
                let ty = codegen_type_expr(params[0].1, interner);
                let pname = interner.resolve(params[0].0).to_string();
                let copy = if is_copy_type_expr(params[0].1, interner) { "copied" } else { "cloned" };
                (ty, pname, copy)
            } else {
                let types: Vec<String> = params.iter().map(|(_, t)| codegen_type_expr(t, interner)).collect();
                let names: Vec<String> = params.iter().map(|(n, _)| interner.resolve(*n).to_string()).collect();
                let copy = if params.iter().all(|(_, t)| is_copy_type_expr(t, interner)) { "copied" } else { "cloned" };
                (format!("({})", types.join(", ")), format!("({})", names.join(", ")), copy)
            };

            writeln!(output, "    use std::cell::RefCell;").unwrap();
            writeln!(output, "    thread_local! {{").unwrap();
            writeln!(output, "        static {}: RefCell<FxHashMap<{}, {}>> = RefCell::new(FxHashMap::default());", memo_name, key_type, ret_ty).unwrap();
            writeln!(output, "    }}").unwrap();
            writeln!(output, "    if let Some(__v) = {}.with(|c| c.borrow().get(&{}).{}()) {{", memo_name, key_expr, copy_method).unwrap();
            writeln!(output, "        return __v;").unwrap();
            writeln!(output, "    }}").unwrap();
            writeln!(output, "    let __memo_result = (|| -> {} {{", ret_ty).unwrap();
            let stmt_refs: Vec<&Stmt> = body.iter().filter(|s| !is_affine_array_decl(s, &func_ctx)).collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if !no_peephole {
                    if let Some((code, skip)) = super::peephole::try_block_peepholes(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                }
                output.push_str(&codegen_stmt(stmt_refs[si], interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env));
                si += 1;
            }
            writeln!(output, "    }})();").unwrap();
            writeln!(output, "    {}.with(|c| c.borrow_mut().insert({}, __memo_result));", memo_name, key_expr).unwrap();
            writeln!(output, "    __memo_result").unwrap();
        } else {
            let stmt_refs: Vec<&Stmt> = body.iter().filter(|s| !is_affine_array_decl(s, &func_ctx)).collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if !no_peephole {
                    if let Some((code, skip)) = super::peephole::try_block_peepholes(&stmt_refs, si, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                }
                // OPT-1C: Set liveness for this statement so codegen_stmt can move
                // dead non-Copy args instead of cloning them.
                func_ctx.set_live_vars_after(liveness.live_after(name, si).clone());
                output.push_str(&codegen_stmt(stmt_refs[si], interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env));
                si += 1;
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
    }

    output
}

/// Phase 38: Map native function names to logicaffeine_system module paths.
/// For system functions only — user-defined native paths bypass this entirely.
/// Returns None for unknown functions (caller emits compile_error!).
fn map_native_function(name: &str) -> Option<(&'static str, &'static str)> {
    match name {
        "read" => Some(("file", "read")),
        "write" => Some(("file", "write")),
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
