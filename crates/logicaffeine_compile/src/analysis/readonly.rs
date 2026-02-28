use std::collections::{HashMap, HashSet};

use logicaffeine_base::Symbol;
use logicaffeine_language::ast::{Expr, Stmt};
use logicaffeine_language::ast::stmt::ClosureBody;

use super::callgraph::CallGraph;
use super::types::{LogosType, TypeEnv};

/// Readonly parameter analysis result.
///
/// Maps each function to the set of its `Seq<T>` parameters that are never
/// structurally mutated (no Push, Pop, Add, Remove, SetIndex, or reassignment)
/// either directly or transitively through callees.
///
/// Parameters in this set are eligible for `&[T]` borrow in codegen instead
/// of requiring ownership or cloning.
pub struct ReadonlyParams {
    /// fn_sym → set of param symbols that are readonly within that function.
    pub readonly: HashMap<Symbol, HashSet<Symbol>>,
}

impl ReadonlyParams {
    /// Analyze the program and compute readonly parameters.
    ///
    /// Uses fixed-point iteration: starts optimistically with all `Seq<T>`
    /// params as readonly candidates, then eliminates those that are directly
    /// mutated or transitively mutated via callee propagation.
    ///
    /// Native functions are trusted: their params remain readonly unless
    /// the LOGOS body explicitly mutates them (which is impossible since they
    /// have no body).
    pub fn analyze(stmts: &[Stmt<'_>], callgraph: &CallGraph, type_env: &TypeEnv) -> Self {
        // Build fn_params map: fn_sym → ordered list of param symbols
        let mut fn_params: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
        for stmt in stmts {
            if let Stmt::FunctionDef { name, params, .. } = stmt {
                let syms: Vec<Symbol> = params.iter().map(|(s, _)| *s).collect();
                fn_params.insert(*name, syms);
            }
        }

        // Initialize: all Seq<T> params are readonly candidates
        let mut readonly: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();
        for stmt in stmts {
            if let Stmt::FunctionDef { name, params, .. } = stmt {
                let mut candidates = HashSet::new();
                for (sym, _) in params {
                    if is_seq_type(type_env.lookup(*sym)) {
                        candidates.insert(*sym);
                    }
                }
                readonly.insert(*name, candidates);
            }
        }

        // Remove directly mutated params from non-native functions
        for stmt in stmts {
            if let Stmt::FunctionDef { name, params, body, is_native, .. } = stmt {
                if *is_native {
                    continue;
                }
                let param_set: HashSet<Symbol> = params.iter().map(|(s, _)| *s).collect();
                let mutated = collect_direct_mutations(body, &param_set);
                if let Some(candidates) = readonly.get_mut(name) {
                    for sym in &mutated {
                        candidates.remove(sym);
                    }
                }
            }
        }

        // Fixed-point: propagate non-readonly through call sites
        loop {
            let mut changed = false;

            for stmt in stmts {
                if let Stmt::FunctionDef { name: caller, body, is_native, .. } = stmt {
                    if *is_native {
                        continue;
                    }

                    // Collect all call sites in this function's body (including closures)
                    let call_sites = collect_call_sites(body);

                    for (callee, arg_syms) in &call_sites {
                        let callee_params = match fn_params.get(callee) {
                            Some(p) => p,
                            None => continue, // unknown function, skip
                        };

                        for (i, maybe_arg_sym) in arg_syms.iter().enumerate() {
                            let arg_sym = match maybe_arg_sym {
                                Some(s) => s,
                                None => continue, // arg is not a plain identifier
                            };

                            let callee_param = match callee_params.get(i) {
                                Some(p) => p,
                                None => continue,
                            };

                            // Is callee's param at position i NOT readonly?
                            let callee_param_readonly = readonly
                                .get(callee)
                                .map(|s| s.contains(callee_param))
                                .unwrap_or(true); // unknown callees are trusted

                            if !callee_param_readonly {
                                // The caller's arg is passed to a mutating position
                                if let Some(caller_readonly) = readonly.get_mut(caller) {
                                    if caller_readonly.remove(arg_sym) {
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !changed {
                break;
            }
        }

        Self { readonly }
    }

    /// Returns `true` if `param_sym` is readonly within `fn_sym`.
    pub fn is_readonly(&self, fn_sym: Symbol, param_sym: Symbol) -> bool {
        self.readonly
            .get(&fn_sym)
            .map(|s| s.contains(&param_sym))
            .unwrap_or(false)
    }
}

fn is_seq_type(ty: &LogosType) -> bool {
    matches!(ty, LogosType::Seq(_))
}

// =============================================================================
// Direct mutation detection
// =============================================================================

/// Collects param symbols that are directly mutated in the body.
///
/// Looks for Push, Pop, Add, Remove, SetIndex, SetField, and Set reassignment
/// on identifiers that appear in `param_set`. Also detects "consumed"
/// parameters: those assigned into a mutable local via `Let mutable X be param`.
/// A consumed Seq parameter should be taken by value (not borrowed) so the
/// copy becomes a move instead of a `.to_vec()` clone.
///
/// Does NOT recurse into closure bodies (closures in LOGOS capture by clone,
/// so they don't mutate the original param directly).
fn collect_direct_mutations(stmts: &[Stmt<'_>], param_set: &HashSet<Symbol>) -> HashSet<Symbol> {
    let mut mutated = HashSet::new();
    for stmt in stmts {
        collect_mutations_from_stmt(stmt, param_set, &mut mutated);
    }
    // Detect consumed parameters: `Let mutable X be param` where X is
    // subsequently mutated. Taking param by value allows a move instead
    // of a clone. We conservatively mark any param that appears as the
    // value of a `Let mutable` as consumed.
    collect_consumed_params(stmts, param_set, &mut mutated);
    mutated
}

fn collect_mutations_from_stmt(stmt: &Stmt<'_>, param_set: &HashSet<Symbol>, mutated: &mut HashSet<Symbol>) {
    match stmt {
        Stmt::Push { collection, .. } => {
            if let Expr::Identifier(sym) = **collection {
                if param_set.contains(&sym) {
                    mutated.insert(sym);
                }
            }
        }
        Stmt::Pop { collection, .. } => {
            if let Expr::Identifier(sym) = **collection {
                if param_set.contains(&sym) {
                    mutated.insert(sym);
                }
            }
        }
        Stmt::Add { collection, .. } => {
            if let Expr::Identifier(sym) = **collection {
                if param_set.contains(&sym) {
                    mutated.insert(sym);
                }
            }
        }
        Stmt::Remove { collection, .. } => {
            if let Expr::Identifier(sym) = **collection {
                if param_set.contains(&sym) {
                    mutated.insert(sym);
                }
            }
        }
        Stmt::SetIndex { collection, .. } => {
            if let Expr::Identifier(sym) = **collection {
                if param_set.contains(&sym) {
                    mutated.insert(sym);
                }
            }
        }
        Stmt::SetField { object, .. } => {
            if let Expr::Identifier(sym) = **object {
                if param_set.contains(&sym) {
                    mutated.insert(sym);
                }
            }
        }
        Stmt::Set { target, .. } => {
            if param_set.contains(target) {
                mutated.insert(*target);
            }
        }
        // Recurse into control-flow blocks (not closures)
        Stmt::If { then_block, else_block, .. } => {
            for s in *then_block {
                collect_mutations_from_stmt(s, param_set, mutated);
            }
            if let Some(else_b) = else_block {
                for s in *else_b {
                    collect_mutations_from_stmt(s, param_set, mutated);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in *body {
                collect_mutations_from_stmt(s, param_set, mutated);
            }
        }
        Stmt::Repeat { body, .. } => {
            for s in *body {
                collect_mutations_from_stmt(s, param_set, mutated);
            }
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                for s in arm.body {
                    collect_mutations_from_stmt(s, param_set, mutated);
                }
            }
        }
        _ => {}
    }
}

/// Detects consumed parameters: those copied into a mutable local via
/// `Let mutable X be param`. Recurses into control-flow blocks.
fn collect_consumed_params(stmts: &[Stmt<'_>], param_set: &HashSet<Symbol>, consumed: &mut HashSet<Symbol>) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { mutable: true, value, .. } => {
                if let Expr::Identifier(sym) = value {
                    if param_set.contains(sym) {
                        consumed.insert(*sym);
                    }
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                collect_consumed_params(then_block, param_set, consumed);
                if let Some(else_b) = else_block {
                    collect_consumed_params(else_b, param_set, consumed);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_consumed_params(body, param_set, consumed);
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    collect_consumed_params(arm.body, param_set, consumed);
                }
            }
            _ => {}
        }
    }
}

// =============================================================================
// Call site collection (for fixed-point propagation)
// =============================================================================

/// Collects all call sites in a function body, including those inside closures.
///
/// Returns `Vec<(callee, [arg0?, arg1?, ...])>` where each arg is `Some(sym)`
/// if the argument is a plain `Expr::Identifier`, otherwise `None`.
fn collect_call_sites(stmts: &[Stmt<'_>]) -> Vec<(Symbol, Vec<Option<Symbol>>)> {
    let mut sites = Vec::new();
    collect_call_sites_from_stmts(stmts, &mut sites);
    sites
}

fn collect_call_sites_from_stmts(stmts: &[Stmt<'_>], sites: &mut Vec<(Symbol, Vec<Option<Symbol>>)>) {
    for stmt in stmts {
        collect_call_sites_from_stmt(stmt, sites);
    }
}

fn collect_call_sites_from_stmt(stmt: &Stmt<'_>, sites: &mut Vec<(Symbol, Vec<Option<Symbol>>)>) {
    match stmt {
        Stmt::Call { function, args } => {
            let arg_syms = args.iter().map(|arg| {
                if let Expr::Identifier(sym) = *arg { Some(*sym) } else { None }
            }).collect();
            sites.push((*function, arg_syms));
            for arg in args {
                collect_call_sites_from_expr(arg, sites);
            }
        }
        Stmt::Let { value, .. } => collect_call_sites_from_expr(value, sites),
        Stmt::Set { value, .. } => collect_call_sites_from_expr(value, sites),
        Stmt::Return { value: Some(v) } => collect_call_sites_from_expr(v, sites),
        Stmt::If { cond, then_block, else_block } => {
            collect_call_sites_from_expr(cond, sites);
            collect_call_sites_from_stmts(then_block, sites);
            if let Some(else_b) = else_block {
                collect_call_sites_from_stmts(else_b, sites);
            }
        }
        Stmt::While { cond, body, .. } => {
            collect_call_sites_from_expr(cond, sites);
            collect_call_sites_from_stmts(body, sites);
        }
        Stmt::Repeat { iterable, body, .. } => {
            collect_call_sites_from_expr(iterable, sites);
            collect_call_sites_from_stmts(body, sites);
        }
        Stmt::Push { value, collection } => {
            collect_call_sites_from_expr(value, sites);
            collect_call_sites_from_expr(collection, sites);
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                collect_call_sites_from_stmts(arm.body, sites);
            }
        }
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            collect_call_sites_from_stmts(tasks, sites);
        }
        _ => {}
    }
}

fn collect_call_sites_from_expr(expr: &Expr<'_>, sites: &mut Vec<(Symbol, Vec<Option<Symbol>>)>) {
    match expr {
        Expr::Call { function, args } => {
            let arg_syms = args.iter().map(|arg| {
                if let Expr::Identifier(sym) = *arg { Some(*sym) } else { None }
            }).collect();
            sites.push((*function, arg_syms));
            for arg in args {
                collect_call_sites_from_expr(arg, sites);
            }
        }
        Expr::Closure { body, .. } => match body {
            ClosureBody::Expression(e) => collect_call_sites_from_expr(e, sites),
            ClosureBody::Block(stmts) => collect_call_sites_from_stmts(stmts, sites),
        },
        Expr::BinaryOp { left, right, .. } => {
            collect_call_sites_from_expr(left, sites);
            collect_call_sites_from_expr(right, sites);
        }
        Expr::Index { collection, index } => {
            collect_call_sites_from_expr(collection, sites);
            collect_call_sites_from_expr(index, sites);
        }
        Expr::Length { collection } => collect_call_sites_from_expr(collection, sites),
        Expr::Contains { collection, value } => {
            collect_call_sites_from_expr(collection, sites);
            collect_call_sites_from_expr(value, sites);
        }
        Expr::FieldAccess { object, .. } => collect_call_sites_from_expr(object, sites),
        Expr::Copy { expr } | Expr::Give { value: expr } => {
            collect_call_sites_from_expr(expr, sites);
        }
        Expr::OptionSome { value } => collect_call_sites_from_expr(value, sites),
        Expr::WithCapacity { value, capacity } => {
            collect_call_sites_from_expr(value, sites);
            collect_call_sites_from_expr(capacity, sites);
        }
        Expr::CallExpr { callee, args } => {
            collect_call_sites_from_expr(callee, sites);
            for arg in args {
                collect_call_sites_from_expr(arg, sites);
            }
        }
        _ => {}
    }
}

// =============================================================================
// Mutable Borrow Parameter Analysis
// =============================================================================

/// Mutable borrow parameter analysis result.
///
/// Identifies `Seq<T>` parameters that are only mutated via element access
/// (SetIndex) but never structurally modified (no Push, Pop, Add, Remove,
/// or reassignment). These parameters can be passed as `&mut [T]` instead
/// of by value, eliminating the move-in/move-out ownership pattern.
///
/// Additional requirement: the function must return the parameter as its
/// sole return value, so the call site can drop the assignment.
pub struct MutableBorrowParams {
    /// fn_sym → set of param symbols eligible for &mut [T] borrow.
    pub mutable_borrow: HashMap<Symbol, HashSet<Symbol>>,
}

impl MutableBorrowParams {
    /// Analyze the program and compute mutable borrow parameters.
    pub fn analyze(stmts: &[Stmt<'_>], callgraph: &CallGraph, type_env: &TypeEnv) -> Self {
        let mut fn_params: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
        for stmt in stmts {
            if let Stmt::FunctionDef { name, params, .. } = stmt {
                let syms: Vec<Symbol> = params.iter().map(|(s, _)| *s).collect();
                fn_params.insert(*name, syms);
            }
        }

        let mut mutable_borrow: HashMap<Symbol, HashSet<Symbol>> = HashMap::new();

        for stmt in stmts {
            if let Stmt::FunctionDef { name, params, body, is_native, is_exported, .. } = stmt {
                if *is_native || *is_exported {
                    continue;
                }

                let mut candidates = HashSet::new();

                for (sym, _) in params {
                    if !is_seq_type(type_env.lookup(*sym)) {
                        continue;
                    }

                    let has_set_index = has_set_index_on(body, *sym);
                    let has_structural = has_structural_mutation_on(body, *sym);
                    let has_reassign = has_reassignment_on(body, *sym);
                    let consumed = is_consumed_param(body, *sym);
                    let returned = is_sole_return_param(body, *sym);

                    if has_set_index && !has_structural && !has_reassign && !consumed && returned {
                        candidates.insert(*sym);
                    } else if consumed {
                        // Consume-alias detection: `Let mutable result be arr`
                        // where `arr` is never used after the consume and `result`
                        // satisfies all &mut [T] criteria.
                        let param_idx = params.iter().position(|(s, _)| *s == *sym).unwrap_or(usize::MAX);
                        if let Some(alias) = detect_consume_alias(body, *sym) {
                            let alias_has_set_index = has_set_index_on(body, alias);
                            let alias_has_structural = has_structural_mutation_on(body, alias);
                            let alias_returned = is_sole_return_param_or_alias(body, *sym, alias);
                            let alias_reassign_ok = reassignment_only_self_calls(body, alias, *name, param_idx);
                            let param_dead = is_param_dead_after_consume(body, *sym, alias);

                            if alias_has_set_index && !alias_has_structural && alias_returned && alias_reassign_ok && param_dead {
                                candidates.insert(*sym);
                            }
                        }
                    }
                }

                if !candidates.is_empty() {
                    mutable_borrow.insert(*name, candidates);
                }
            }
        }

        // Fixed-point: propagate through call sites.
        loop {
            let mut changed = false;
            for stmt in stmts {
                if let Stmt::FunctionDef { name: caller, body, is_native, .. } = stmt {
                    if *is_native {
                        continue;
                    }
                    let call_sites = collect_call_sites(body);
                    for (callee, arg_syms) in &call_sites {
                        let callee_params = match fn_params.get(callee) {
                            Some(p) => p,
                            None => continue,
                        };
                        for (i, maybe_arg_sym) in arg_syms.iter().enumerate() {
                            let arg_sym = match maybe_arg_sym {
                                Some(s) => s,
                                None => continue,
                            };
                            let callee_param = match callee_params.get(i) {
                                Some(p) => p,
                                None => continue,
                            };
                            let callee_is_mut_borrow = mutable_borrow
                                .get(callee)
                                .map(|s| s.contains(callee_param))
                                .unwrap_or(false);
                            if !callee_is_mut_borrow {
                                if let Some(caller_set) = mutable_borrow.get_mut(caller) {
                                    if caller_set.remove(arg_sym) {
                                        changed = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // Call-site compatibility: &mut [T] suppresses the return type, so the
        // function can only be called in void context (Stmt::Call) or in
        // `Set x to f(x, ...)` where x is at a mut_borrow position.
        // Remove functions from mutable_borrow if any call site uses the return
        // value in a way that requires it (Let, Show, Return, expression context).
        let incompatible = collect_incompatible_mut_borrow_callsites(
            stmts, &mutable_borrow, &fn_params,
        );
        for fn_sym in incompatible {
            mutable_borrow.remove(&fn_sym);
        }

        Self { mutable_borrow }
    }

    pub fn is_mutable_borrow(&self, fn_sym: Symbol, param_sym: Symbol) -> bool {
        self.mutable_borrow
            .get(&fn_sym)
            .map(|s| s.contains(&param_sym))
            .unwrap_or(false)
    }
}

fn has_set_index_on(stmts: &[Stmt<'_>], sym: Symbol) -> bool {
    stmts.iter().any(|s| check_set_index_stmt(s, sym))
}

fn check_set_index_stmt(stmt: &Stmt<'_>, sym: Symbol) -> bool {
    match stmt {
        Stmt::SetIndex { collection, .. } => {
            matches!(**collection, Expr::Identifier(s) if s == sym)
        }
        Stmt::If { then_block, else_block, .. } => {
            has_set_index_on(then_block, sym)
                || else_block.as_ref().map_or(false, |eb| has_set_index_on(eb, sym))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            has_set_index_on(body, sym)
        }
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| has_set_index_on(arm.body, sym))
        }
        _ => false,
    }
}

fn has_structural_mutation_on(stmts: &[Stmt<'_>], sym: Symbol) -> bool {
    stmts.iter().any(|s| check_structural_stmt(s, sym))
}

fn check_structural_stmt(stmt: &Stmt<'_>, sym: Symbol) -> bool {
    match stmt {
        Stmt::Push { collection, .. } | Stmt::Pop { collection, .. }
        | Stmt::Add { collection, .. } | Stmt::Remove { collection, .. } => {
            matches!(**collection, Expr::Identifier(s) if s == sym)
        }
        Stmt::If { then_block, else_block, .. } => {
            has_structural_mutation_on(then_block, sym)
                || else_block.as_ref().map_or(false, |eb| has_structural_mutation_on(eb, sym))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            has_structural_mutation_on(body, sym)
        }
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| has_structural_mutation_on(arm.body, sym))
        }
        _ => false,
    }
}

fn has_reassignment_on(stmts: &[Stmt<'_>], sym: Symbol) -> bool {
    stmts.iter().any(|s| check_reassignment_stmt(s, sym))
}

fn check_reassignment_stmt(stmt: &Stmt<'_>, sym: Symbol) -> bool {
    match stmt {
        Stmt::Set { target, .. } => *target == sym,
        Stmt::If { then_block, else_block, .. } => {
            has_reassignment_on(then_block, sym)
                || else_block.as_ref().map_or(false, |eb| has_reassignment_on(eb, sym))
        }
        Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
            has_reassignment_on(body, sym)
        }
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| has_reassignment_on(arm.body, sym))
        }
        _ => false,
    }
}

fn is_consumed_param(stmts: &[Stmt<'_>], sym: Symbol) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Let { mutable: true, value, .. } => {
                if matches!(value, Expr::Identifier(s) if *s == sym) {
                    return true;
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if is_consumed_param(then_block, sym) { return true; }
                if let Some(else_b) = else_block {
                    if is_consumed_param(else_b, sym) { return true; }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if is_consumed_param(body, sym) { return true; }
            }
            _ => {}
        }
    }
    false
}

fn is_sole_return_param(stmts: &[Stmt<'_>], sym: Symbol) -> bool {
    let mut returns = Vec::new();
    collect_returns(stmts, &mut returns);
    !returns.is_empty() && returns.iter().all(|r| *r == sym)
}

fn collect_returns(stmts: &[Stmt<'_>], returns: &mut Vec<Symbol>) {
    for stmt in stmts {
        match stmt {
            Stmt::Return { value: Some(expr) } => {
                if let Expr::Identifier(sym) = expr {
                    returns.push(*sym);
                } else {
                    // Non-identifier return — sentinel that won't match
                    returns.push(Symbol::EMPTY);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                collect_returns(then_block, returns);
                if let Some(else_b) = else_block {
                    collect_returns(else_b, returns);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_returns(body, returns);
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    collect_returns(arm.body, returns);
                }
            }
            _ => {}
        }
    }
}

// =============================================================================
// Call-site compatibility for &mut [T]
// =============================================================================

/// Collect functions in `mutable_borrow` that have incompatible call sites.
/// An incompatible call site is one where the function's return value is used
/// (e.g., in Let, Show, Return, or expression context) because &mut [T]
/// functions have void return.
fn collect_incompatible_mut_borrow_callsites(
    stmts: &[Stmt<'_>],
    mutable_borrow: &HashMap<Symbol, HashSet<Symbol>>,
    fn_params: &HashMap<Symbol, Vec<Symbol>>,
) -> HashSet<Symbol> {
    let mut incompatible = HashSet::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { body, .. } = stmt {
            check_callsite_compat_stmts(body, mutable_borrow, fn_params, &mut incompatible);
        }
    }
    // Also check main-level statements (not inside function defs)
    check_callsite_compat_stmts(stmts, mutable_borrow, fn_params, &mut incompatible);
    incompatible
}

fn check_callsite_compat_stmts(
    stmts: &[Stmt<'_>],
    mutable_borrow: &HashMap<Symbol, HashSet<Symbol>>,
    fn_params: &HashMap<Symbol, Vec<Symbol>>,
    incompatible: &mut HashSet<Symbol>,
) {
    for stmt in stmts {
        check_callsite_compat_stmt(stmt, mutable_borrow, fn_params, incompatible);
    }
}

fn check_callsite_compat_stmt(
    stmt: &Stmt<'_>,
    mutable_borrow: &HashMap<Symbol, HashSet<Symbol>>,
    fn_params: &HashMap<Symbol, Vec<Symbol>>,
    incompatible: &mut HashSet<Symbol>,
) {
    match stmt {
        // Stmt::Call → void context, always OK. But check args for nested calls.
        Stmt::Call { args, .. } => {
            for arg in args {
                check_callsite_compat_expr(arg, mutable_borrow, incompatible);
            }
        }
        // Stmt::Set → OK if target == args[mut_borrow_pos], otherwise check expr
        Stmt::Set { target, value } => {
            if let Expr::Call { function, args } = value {
                if mutable_borrow.contains_key(function) {
                    // Check that target is at a mut_borrow position
                    let mut_positions: HashSet<usize> = fn_params.get(function)
                        .map(|params| {
                            params.iter().enumerate()
                                .filter(|(_, sym)| {
                                    mutable_borrow.get(function)
                                        .map(|s| s.contains(sym))
                                        .unwrap_or(false)
                                })
                                .map(|(i, _)| i)
                                .collect()
                        })
                        .unwrap_or_default();

                    let target_at_mut_pos = args.iter().enumerate()
                        .any(|(i, a)| {
                            mut_positions.contains(&i)
                                && matches!(a, Expr::Identifier(sym) if *sym == *target)
                        });

                    if !target_at_mut_pos {
                        incompatible.insert(*function);
                    }
                }
                // Check args for nested calls
                for arg in args {
                    check_callsite_compat_expr(arg, mutable_borrow, incompatible);
                }
            } else {
                check_callsite_compat_expr(value, mutable_borrow, incompatible);
            }
        }
        // Stmt::Let → if value is a call to a mut_borrow function, it's incompatible
        Stmt::Let { value, .. } => {
            check_callsite_compat_expr(value, mutable_borrow, incompatible);
        }
        Stmt::Return { value: Some(v) } => {
            check_callsite_compat_expr(v, mutable_borrow, incompatible);
        }
        Stmt::Show { object, .. } => {
            check_callsite_compat_expr(object, mutable_borrow, incompatible);
        }
        Stmt::Push { value, collection } => {
            check_callsite_compat_expr(value, mutable_borrow, incompatible);
            check_callsite_compat_expr(collection, mutable_borrow, incompatible);
        }
        Stmt::SetIndex { collection, index, value } => {
            check_callsite_compat_expr(collection, mutable_borrow, incompatible);
            check_callsite_compat_expr(index, mutable_borrow, incompatible);
            check_callsite_compat_expr(value, mutable_borrow, incompatible);
        }
        Stmt::If { cond, then_block, else_block } => {
            check_callsite_compat_expr(cond, mutable_borrow, incompatible);
            check_callsite_compat_stmts(then_block, mutable_borrow, fn_params, incompatible);
            if let Some(else_b) = else_block {
                check_callsite_compat_stmts(else_b, mutable_borrow, fn_params, incompatible);
            }
        }
        Stmt::While { cond, body, .. } => {
            check_callsite_compat_expr(cond, mutable_borrow, incompatible);
            check_callsite_compat_stmts(body, mutable_borrow, fn_params, incompatible);
        }
        Stmt::Repeat { iterable, body, .. } => {
            check_callsite_compat_expr(iterable, mutable_borrow, incompatible);
            check_callsite_compat_stmts(body, mutable_borrow, fn_params, incompatible);
        }
        Stmt::Inspect { arms, .. } => {
            for arm in arms {
                check_callsite_compat_stmts(arm.body, mutable_borrow, fn_params, incompatible);
            }
        }
        // Skip FunctionDef — handled at top level
        _ => {}
    }
}

/// Check if an expression uses a mut_borrow function in value context (incompatible).
fn check_callsite_compat_expr(
    expr: &Expr<'_>,
    mutable_borrow: &HashMap<Symbol, HashSet<Symbol>>,
    incompatible: &mut HashSet<Symbol>,
) {
    match expr {
        Expr::Call { function, args } => {
            if mutable_borrow.contains_key(function) {
                incompatible.insert(*function);
            }
            for arg in args {
                check_callsite_compat_expr(arg, mutable_borrow, incompatible);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            check_callsite_compat_expr(left, mutable_borrow, incompatible);
            check_callsite_compat_expr(right, mutable_borrow, incompatible);
        }
        Expr::Index { collection, index } => {
            check_callsite_compat_expr(collection, mutable_borrow, incompatible);
            check_callsite_compat_expr(index, mutable_borrow, incompatible);
        }
        Expr::Length { collection } => {
            check_callsite_compat_expr(collection, mutable_borrow, incompatible);
        }
        Expr::Contains { collection, value } => {
            check_callsite_compat_expr(collection, mutable_borrow, incompatible);
            check_callsite_compat_expr(value, mutable_borrow, incompatible);
        }
        Expr::FieldAccess { object, .. } => {
            check_callsite_compat_expr(object, mutable_borrow, incompatible);
        }
        Expr::Copy { expr: inner } | Expr::Give { value: inner } | Expr::OptionSome { value: inner } => {
            check_callsite_compat_expr(inner, mutable_borrow, incompatible);
        }
        _ => {}
    }
}

// =============================================================================
// Consume-Alias Detection for &mut [T]
// =============================================================================

/// Detect consume-alias pattern: finds exactly one `Let mutable <alias> be <param>`
/// at the top level of the function body. Returns the alias symbol if found.
fn detect_consume_alias(body: &[Stmt<'_>], param_sym: Symbol) -> Option<Symbol> {
    let mut alias = None;
    for stmt in body {
        if let Stmt::Let { var, mutable: true, value, .. } = stmt {
            if matches!(value, Expr::Identifier(s) if *s == param_sym) {
                if alias.is_some() {
                    return None; // Multiple consumes — reject
                }
                alias = Some(*var);
            }
        }
    }
    alias
}

/// Check that every return in the body returns either `param_sym` or `alias_sym`.
fn is_sole_return_param_or_alias(stmts: &[Stmt<'_>], param_sym: Symbol, alias_sym: Symbol) -> bool {
    let mut returns = Vec::new();
    collect_returns(stmts, &mut returns);
    !returns.is_empty() && returns.iter().all(|r| *r == param_sym || *r == alias_sym)
}

/// Check that every `Set <alias> to <expr>` in the body is a call to `func_name`
/// with `alias` at position `param_position`. No other reassignment patterns allowed.
fn reassignment_only_self_calls(
    body: &[Stmt<'_>],
    alias: Symbol,
    func_name: Symbol,
    param_position: usize,
) -> bool {
    check_reassignment_self_calls(body, alias, func_name, param_position)
}

fn check_reassignment_self_calls(
    stmts: &[Stmt<'_>],
    alias: Symbol,
    func_name: Symbol,
    param_position: usize,
) -> bool {
    for stmt in stmts {
        match stmt {
            Stmt::Set { target, value } if *target == alias => {
                // Must be a call to func_name with alias at param_position
                match value {
                    Expr::Call { function, args } if *function == func_name => {
                        let arg_at_pos = args.get(param_position);
                        let is_alias_at_pos = arg_at_pos
                            .map(|a| matches!(a, Expr::Identifier(s) if *s == alias))
                            .unwrap_or(false);
                        if !is_alias_at_pos {
                            return false;
                        }
                    }
                    _ => return false, // Non-self-call reassignment
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                if !check_reassignment_self_calls(then_block, alias, func_name, param_position) {
                    return false;
                }
                if let Some(else_b) = else_block {
                    if !check_reassignment_self_calls(else_b, alias, func_name, param_position) {
                        return false;
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                if !check_reassignment_self_calls(body, alias, func_name, param_position) {
                    return false;
                }
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    if !check_reassignment_self_calls(arm.body, alias, func_name, param_position) {
                        return false;
                    }
                }
            }
            _ => {}
        }
    }
    true
}

/// Check that `param_sym` is dead after the consume statement
/// (`Let mutable <alias> be <param>`). Scans top-level statements:
/// before the consume, param can be used freely. After the consume,
/// param must never appear in any expression.
fn is_param_dead_after_consume(body: &[Stmt<'_>], param_sym: Symbol, alias: Symbol) -> bool {
    let mut found_consume = false;
    for stmt in body {
        if !found_consume {
            // Check if this is the consume statement
            if let Stmt::Let { var, mutable: true, value, .. } = stmt {
                if *var == alias && matches!(value, Expr::Identifier(s) if *s == param_sym) {
                    found_consume = true;
                    continue;
                }
            }
        } else {
            // After the consume: param must not appear
            if stmt_references_symbol(stmt, param_sym) {
                return false;
            }
        }
    }
    found_consume // Must have actually found the consume
}

/// Check if a statement references a given symbol anywhere (expressions, collections, etc.).
fn stmt_references_symbol(stmt: &Stmt<'_>, sym: Symbol) -> bool {
    match stmt {
        Stmt::Let { value, .. } => expr_references_symbol(value, sym),
        Stmt::Set { target, value } => *target == sym || expr_references_symbol(value, sym),
        Stmt::Call { function, args } => {
            *function == sym || args.iter().any(|a| expr_references_symbol(a, sym))
        }
        Stmt::Push { value, collection } => {
            expr_references_symbol(value, sym) || expr_references_symbol(collection, sym)
        }
        Stmt::Pop { collection, into } => {
            expr_references_symbol(collection, sym)
                || into.map_or(false, |s| s == sym)
        }
        Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            expr_references_symbol(value, sym) || expr_references_symbol(collection, sym)
        }
        Stmt::SetIndex { collection, index, value } => {
            expr_references_symbol(collection, sym)
                || expr_references_symbol(index, sym)
                || expr_references_symbol(value, sym)
        }
        Stmt::SetField { object, value, .. } => {
            expr_references_symbol(object, sym) || expr_references_symbol(value, sym)
        }
        Stmt::Return { value: Some(v) } => expr_references_symbol(v, sym),
        Stmt::Return { value: None } => false,
        Stmt::If { cond, then_block, else_block } => {
            expr_references_symbol(cond, sym)
                || then_block.iter().any(|s| stmt_references_symbol(s, sym))
                || else_block.as_ref().map_or(false, |eb| eb.iter().any(|s| stmt_references_symbol(s, sym)))
        }
        Stmt::While { cond, body, .. } => {
            expr_references_symbol(cond, sym)
                || body.iter().any(|s| stmt_references_symbol(s, sym))
        }
        Stmt::Repeat { iterable, body, .. } => {
            expr_references_symbol(iterable, sym)
                || body.iter().any(|s| stmt_references_symbol(s, sym))
        }
        Stmt::Inspect { arms, .. } => {
            arms.iter().any(|arm| arm.body.iter().any(|s| stmt_references_symbol(s, sym)))
        }
        Stmt::Show { object, .. } => expr_references_symbol(object, sym),
        _ => false,
    }
}

fn expr_references_symbol(expr: &Expr<'_>, sym: Symbol) -> bool {
    match expr {
        Expr::Identifier(s) => *s == sym,
        Expr::BinaryOp { left, right, .. } => {
            expr_references_symbol(left, sym) || expr_references_symbol(right, sym)
        }
        Expr::Not { operand } => expr_references_symbol(operand, sym),
        Expr::Call { function, args } => {
            *function == sym || args.iter().any(|a| expr_references_symbol(a, sym))
        }
        Expr::Index { collection, index } => {
            expr_references_symbol(collection, sym) || expr_references_symbol(index, sym)
        }
        Expr::Length { collection } => expr_references_symbol(collection, sym),
        Expr::Contains { collection, value } => {
            expr_references_symbol(collection, sym) || expr_references_symbol(value, sym)
        }
        Expr::FieldAccess { object, .. } => expr_references_symbol(object, sym),
        Expr::Slice { collection, start, end } => {
            expr_references_symbol(collection, sym)
                || expr_references_symbol(start, sym)
                || expr_references_symbol(end, sym)
        }
        Expr::Copy { expr: inner } | Expr::Give { value: inner } | Expr::OptionSome { value: inner } => {
            expr_references_symbol(inner, sym)
        }
        Expr::CallExpr { callee, args } => {
            expr_references_symbol(callee, sym)
                || args.iter().any(|a| expr_references_symbol(a, sym))
        }
        _ => false,
    }
}
