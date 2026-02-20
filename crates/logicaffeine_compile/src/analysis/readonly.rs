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
