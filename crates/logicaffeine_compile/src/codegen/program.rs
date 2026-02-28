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
use crate::analysis::liveness::LivenessResult;
use crate::analysis::readonly::{ReadonlyParams, MutableBorrowParams};

use super::detection::{
    requires_async, requires_vfs, collect_mutable_vars,
    collect_crdt_register_fields, collect_boxed_fields, collect_async_functions,
    collect_pure_functions, count_self_calls, is_hashable_type, is_copy_type_expr,
    should_memoize, body_contains_self_call, should_inline,
    collect_pipe_sender_params, collect_pipe_vars,
    collect_mutable_vars_stmt, is_result_type,
    vec_to_slice_type, vec_to_mut_slice_type, collect_give_arg_indices,
    collect_single_char_text_vars,
    detect_double_recursion_closed_form,
};
use super::expr::{codegen_expr, codegen_expr_with_async};
use super::ffi::{
    has_wasm_exports, has_c_exports, has_c_exports_with_text,
    codegen_logos_runtime_preamble, collect_c_export_reference_types,
    collect_c_export_value_type_structs,
};
use super::marshal::{is_text_type, codegen_c_export_with_marshaling};
use super::policy::codegen_policy_impls;
use super::stmt::codegen_stmt;
use super::tce::{
    is_tail_recursive, detect_accumulator_pattern, codegen_stmt_acc,
    detect_mutual_tce_pairs, codegen_mutual_tce_pair, codegen_stmt_tce,
};
use super::types::{
    codegen_type_expr, infer_return_type_from_body,
    codegen_struct_def, codegen_enum_def,
};
use super::{escape_rust_ident, is_rust_keyword};
use super::{
    collect_c_export_ref_structs, codegen_c_accessors,
    try_emit_vec_fill_pattern, try_emit_for_range_pattern, try_emit_swap_pattern,
    try_emit_seq_copy_pattern, try_emit_seq_from_slice_pattern,
    try_emit_bare_slice_push_pattern,
    try_emit_vec_with_capacity_pattern, try_emit_merge_capacity_pattern,
    try_emit_string_with_capacity_pattern,
    try_emit_rotate_left_pattern,
    try_emit_buffer_reuse_while,
    classify_type_for_c_abi, CAbiClass,
};

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
pub fn codegen_program(stmts: &[Stmt], registry: &TypeRegistry, policies: &PolicyRegistry, interner: &Interner, type_env: &crate::analysis::types::TypeEnv) -> String {
    let mut output = String::new();

    // Prelude
    // Use extracted crates instead of logos_core
    writeln!(output, "#[allow(unused_imports)]").unwrap();
    writeln!(output, "use std::fmt::Write as _;").unwrap();
    writeln!(output, "use logicaffeine_data::*;").unwrap();
    writeln!(output, "use logicaffeine_system::*;\n").unwrap();

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
            if is_tail_recursive(*name, body) {
                continue;
            }
            if detect_accumulator_pattern(*name, body).is_some() {
                continue;
            }
            let indices: HashSet<usize> = params.iter().enumerate()
                .filter(|(_, (sym, _))| readonly_params.is_readonly(*name, *sym))
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
            // Skip if already has readonly borrow params (readonly takes precedence)
            let readonly_indices = borrow_params_map.get(name).cloned().unwrap_or_default();
            let indices: HashSet<usize> = params.iter().enumerate()
                .filter(|(i, (sym, _))| {
                    mutable_borrow_params.is_mutable_borrow(*name, *sym)
                        && !readonly_indices.contains(i)
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
                output.push_str(&codegen_function_def(*name, generics, params, body, return_type.as_ref().copied(), *is_native, *native_path, *is_exported, *export_target, interner, &lww_fields, &mv_fields, &async_functions, &boxed_fields, registry, &pure_functions, type_env, &borrow_params_map, &mut_borrow_params_map, &liveness, opt_flags));
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
    }
    // Phase 53: Inject VFS when file operations or persistence is used
    if requires_vfs(stmts) {
        writeln!(output, "    let vfs: std::sync::Arc<dyn logicaffeine_system::fs::Vfs + Send + Sync> = std::sync::Arc::from(logicaffeine_system::fs::get_platform_vfs());").unwrap();
    }
    let mut main_ctx = RefinementContext::from_type_env(type_env);
    // OPT: Register single-char text vars as u8 type for optimized codegen
    for sym in &single_char_vars {
        main_ctx.register_variable_type(*sym, "__single_char_u8".to_string());
    }
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
            // Peephole: seq-from-slice pattern (push-copy loop → slice.to_vec()) — check before vec_with_capacity since it's more specific
            if let Some((code, skip)) = try_emit_seq_from_slice_pattern(&stmt_refs, i, interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mv_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars, &boxed_fields, registry, type_env) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: Vec fill pattern (more specific, must run before with_capacity)
            if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, i, interner, 1, &mut main_ctx) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: Bare slice push pattern (extend_from_slice for bare While copy loops)
            if let Some((code, skip)) = try_emit_bare_slice_push_pattern(&stmt_refs, i, interner, 1, main_ctx.get_variable_types()) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: Vec with_capacity pattern optimization
            if let Some((code, skip)) = try_emit_vec_with_capacity_pattern(&stmt_refs, i, interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mv_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars, &boxed_fields, registry, type_env) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: Merge Vec capacity pattern (capacity from source Vec lengths)
            if let Some((code, skip)) = try_emit_merge_capacity_pattern(&stmt_refs, i, interner, 1, &mut main_ctx) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: String with_capacity pattern optimization
            if let Some((code, skip)) = try_emit_string_with_capacity_pattern(&stmt_refs, i, interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mv_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars, &boxed_fields, registry, type_env) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: Buffer reuse (hoist inner buffer, clear+swap instead of alloc+move)
            if let Some((code, skip)) = try_emit_buffer_reuse_while(&stmt_refs, i, interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mv_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars, &boxed_fields, registry, type_env) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: For-range loop optimization
            if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, i, interner, 1, &main_mutable_vars, &mut main_ctx, &lww_fields, &mv_fields, &mut main_synced_vars, &main_var_caps, &async_functions, &main_pipe_vars, &boxed_fields, registry, type_env) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: swap pattern optimization
            if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, i, interner, 1, main_ctx.get_variable_types()) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: seq-copy pattern (push loop → .to_vec())
            if let Some((code, skip)) = try_emit_seq_copy_pattern(&stmt_refs, i, interner, 1, &mut main_ctx) {
                output.push_str(&code);
                i += 1 + skip;
                continue;
            }
            // Peephole: rotate-left pattern (shift loop → .rotate_left(1))
            if let Some((code, skip)) = try_emit_rotate_left_pattern(&stmt_refs, i, interner, 1, main_ctx.get_variable_types()) {
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
) -> String {
    let mut output = String::new();
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
    let is_tce = !is_native && !is_c_export_early && !no_tco && is_tail_recursive(name, body);
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
        has_text_param || has_text_return || has_ref_param || has_ref_return
            || has_result_return || has_refinement_param
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

        // Wrap exported C functions in catch_unwind for panic safety
        let wrap_catch_unwind = is_c_export;
        if wrap_catch_unwind {
            writeln!(output, "    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {{").unwrap();
        }

        let mut func_ctx = RefinementContext::new();
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

        // Phase 54: Functions receive pipe senders as parameters, no local pipe declarations
        let func_pipe_vars = HashSet::new();

        if is_tce {
            // TCE: Wrap body in loop, use TCE-aware statement emitter
            writeln!(output, "    loop {{").unwrap();
            let stmt_refs: Vec<&Stmt> = body.iter().collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if !no_peephole {
                    if let Some((code, skip)) = try_emit_seq_from_slice_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 2, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_bare_slice_push_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_vec_with_capacity_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_merge_capacity_pattern(&stmt_refs, si, interner, 2, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_string_with_capacity_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_buffer_reuse_while(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_seq_copy_pattern(&stmt_refs, si, interner, 2, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_rotate_left_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
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
            let stmt_refs: Vec<&Stmt> = body.iter().collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if !no_peephole {
                    if let Some((code, skip)) = try_emit_seq_from_slice_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 2, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_bare_slice_push_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_vec_with_capacity_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_merge_capacity_pattern(&stmt_refs, si, interner, 2, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_string_with_capacity_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_buffer_reuse_while(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_seq_copy_pattern(&stmt_refs, si, interner, 2, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_rotate_left_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
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
            if cf.k == 0 {
                writeln!(output, "    ({}i64 << {})", base_plus_k, param_name).unwrap();
            } else {
                writeln!(output, "    ({}i64 << {}) - {}", base_plus_k, param_name, cf.k).unwrap();
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
            let stmt_refs: Vec<&Stmt> = body.iter().collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if !no_peephole {
                    if let Some((code, skip)) = try_emit_seq_from_slice_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 2, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_bare_slice_push_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_vec_with_capacity_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_merge_capacity_pattern(&stmt_refs, si, interner, 2, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_string_with_capacity_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_buffer_reuse_while(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 2, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_seq_copy_pattern(&stmt_refs, si, interner, 2, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_rotate_left_pattern(&stmt_refs, si, interner, 2, func_ctx.get_variable_types()) {
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
            let stmt_refs: Vec<&Stmt> = body.iter().collect();
            let mut si = 0;
            while si < stmt_refs.len() {
                if !no_peephole {
                    if let Some((code, skip)) = try_emit_seq_from_slice_pattern(&stmt_refs, si, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_vec_fill_pattern(&stmt_refs, si, interner, 1, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_bare_slice_push_pattern(&stmt_refs, si, interner, 1, func_ctx.get_variable_types()) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_vec_with_capacity_pattern(&stmt_refs, si, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_merge_capacity_pattern(&stmt_refs, si, interner, 1, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_string_with_capacity_pattern(&stmt_refs, si, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_buffer_reuse_while(&stmt_refs, si, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_for_range_pattern(&stmt_refs, si, interner, 1, &func_mutable_vars, &mut func_ctx, lww_fields, mv_fields, &mut func_synced_vars, &func_var_caps, async_functions, &func_pipe_vars, boxed_fields, registry, type_env) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_swap_pattern(&stmt_refs, si, interner, 1, func_ctx.get_variable_types()) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_seq_copy_pattern(&stmt_refs, si, interner, 1, &mut func_ctx) {
                        output.push_str(&code);
                        si += 1 + skip;
                        continue;
                    }
                    if let Some((code, skip)) = try_emit_rotate_left_pattern(&stmt_refs, si, interner, 1, func_ctx.get_variable_types()) {
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
        "format" => Some(("fmt", "format")),
        _ => None,
    }
}
