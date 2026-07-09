use std::collections::{HashMap, HashSet};

use crate::analysis::DiscoveryPass;
use crate::arena::Arena;
use crate::arena_ctx::AstContext;
use crate::ast::stmt::{ClosureBody, Expr, Literal, Pattern, Stmt};
use crate::drs::WorldState;
use crate::error::ParseError;
use crate::intern::{Interner, Symbol};
use crate::lexer::Lexer;
use crate::parser::Parser;

use crate::analysis::callgraph::CallGraph;

/// Effect classification for expressions, statements, and functions.
///
/// Effects form a lattice: Pure ⊂ Read ⊂ Write ⊂ IO ⊂ Unknown.
/// Each EffectSet tracks the precise set of variables read/written,
/// plus boolean flags for allocation, IO, security checks, divergence,
/// and unknown (escape blocks).
#[derive(Debug, Clone, Default)]
pub struct EffectSet {
    pub reads: HashSet<Symbol>,
    pub writes: HashSet<Symbol>,
    pub allocates: bool,
    pub io: bool,
    pub security_check: bool,
    pub diverges: bool,
    pub unknown: bool,
    /// Nondeterministic choice — `Select`, `Try*` (depends on instantaneous buffer
    /// state), and cooperative `StopTask`. The outcome is not a function of the inputs,
    /// so a function carrying one must never be specialized on a static argument.
    pub nondet: bool,
    /// Spawns or communicates across a concurrency boundary — `LaunchTask`, `CreatePipe`,
    /// `Spawn`, and the networking statements. Opaque to the partial evaluator.
    pub concurrent: bool,
}

impl EffectSet {
    pub fn pure() -> Self {
        Self::default()
    }

    pub fn read(sym: Symbol) -> Self {
        let mut s = Self::default();
        s.reads.insert(sym);
        s
    }

    pub fn write(sym: Symbol) -> Self {
        let mut s = Self::default();
        s.writes.insert(sym);
        s
    }

    pub fn io() -> Self {
        let mut s = Self::default();
        s.io = true;
        s
    }

    pub fn alloc() -> Self {
        let mut s = Self::default();
        s.allocates = true;
        s
    }

    pub fn unknown() -> Self {
        let mut s = Self::default();
        s.unknown = true;
        s
    }

    pub fn nondet() -> Self {
        let mut s = Self::default();
        s.nondet = true;
        s
    }

    pub fn concurrent() -> Self {
        let mut s = Self::default();
        s.concurrent = true;
        s
    }

    pub fn is_pure(&self) -> bool {
        self.reads.is_empty()
            && self.writes.is_empty()
            && !self.allocates
            && !self.io
            && !self.security_check
            && !self.diverges
            && !self.unknown
            && !self.nondet
            && !self.concurrent
    }

    /// Join two EffectSets (union of all effects).
    pub fn join(&mut self, other: &EffectSet) {
        self.reads.extend(&other.reads);
        self.writes.extend(&other.writes);
        self.allocates |= other.allocates;
        self.io |= other.io;
        self.security_check |= other.security_check;
        self.diverges |= other.diverges;
        self.unknown |= other.unknown;
        self.nondet |= other.nondet;
        self.concurrent |= other.concurrent;
    }
}

/// Environment tracking effects for variables and functions.
///
/// For each variable binding (`Let x be <expr>`), tracks the EffectSet
/// of the bound expression. For each function, tracks the aggregate
/// EffectSet of its body.
pub struct EffectEnv {
    /// Binding-level effects: var_sym → effect of the bound expression
    bindings: HashMap<Symbol, EffectSet>,
    /// Statement-level aggregate effects (joined across all statements)
    aggregate: EffectSet,
    /// Function-level effects: fn_sym → aggregate effect of body
    functions: HashMap<Symbol, EffectSet>,
    /// Interner for name resolution in test queries
    interner: Interner,
}

impl EffectEnv {
    /// Analyze already-parsed statements and return the EffectEnv.
    pub fn from_stmts(stmts: &[Stmt<'_>], interner: &Interner) -> Self {
        let env = analyze_program(stmts, interner);
        Self {
            bindings: env.bindings,
            aggregate: env.aggregate,
            functions: env.functions,
            interner: interner.clone(),
        }
    }

    /// Analyze a complete LOGOS source and return the EffectEnv.
    pub fn analyze_source(source: &str) -> Result<Self, ParseError> {
        let mut interner = Interner::new();
        let mut lexer = Lexer::new(source, &mut interner);
        let tokens = lexer.tokenize();

        let type_registry = {
            let mut discovery = DiscoveryPass::new(&tokens, &mut interner);
            let result = discovery.run_full();
            result.types
        };

        let mut world_state = WorldState::new();
        let expr_arena = Arena::new();
        let term_arena = Arena::new();
        let np_arena = Arena::new();
        let sym_arena = Arena::new();
        let role_arena = Arena::new();
        let pp_arena = Arena::new();
        let stmt_arena = Arena::new();
        let imperative_expr_arena = Arena::new();
        let type_expr_arena = Arena::new();

        let ast_ctx = AstContext::with_types(
            &expr_arena,
            &term_arena,
            &np_arena,
            &sym_arena,
            &role_arena,
            &pp_arena,
            &stmt_arena,
            &imperative_expr_arena,
            &type_expr_arena,
        );

        let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ast_ctx, type_registry);
        let stmts = parser.parse_program()?;

        let env = analyze_program(&stmts, &interner);

        Ok(Self {
            bindings: env.bindings,
            aggregate: env.aggregate,
            functions: env.functions,
            interner,
        })
    }

    fn resolve_name(&self, name: &str) -> Option<Symbol> {
        // Try to find the symbol in the interner
        self.interner.lookup(name)
    }

    pub fn is_binding_pure(&self, var_name: &str) -> bool {
        if let Some(sym) = self.resolve_name(var_name) {
            self.bindings.get(&sym).map(|e| e.is_pure()).unwrap_or(true)
        } else {
            true
        }
    }

    pub fn binding_reads(&self, var_name: &str, read_var: &str) -> bool {
        let var_sym = match self.resolve_name(var_name) {
            Some(s) => s,
            None => return false,
        };
        let read_sym = match self.resolve_name(read_var) {
            Some(s) => s,
            None => return false,
        };
        self.bindings
            .get(&var_sym)
            .map(|e| e.reads.contains(&read_sym))
            .unwrap_or(false)
    }

    pub fn binding_allocates(&self, var_name: &str) -> bool {
        if let Some(sym) = self.resolve_name(var_name) {
            self.bindings.get(&sym).map(|e| e.allocates).unwrap_or(false)
        } else {
            false
        }
    }

    pub fn has_write_to(&self, var_name: &str) -> bool {
        if let Some(sym) = self.resolve_name(var_name) {
            self.aggregate.writes.contains(&sym)
        } else {
            false
        }
    }

    pub fn has_io(&self) -> bool {
        self.aggregate.io
    }

    pub fn has_unknown(&self) -> bool {
        self.aggregate.unknown
    }

    pub fn has_security_check(&self) -> bool {
        self.aggregate.security_check
    }

    pub fn may_diverge(&self) -> bool {
        self.aggregate.diverges
    }

    pub fn function_is_pure(&self, fn_name: &str) -> bool {
        if let Some(sym) = self.resolve_name(fn_name) {
            self.functions.get(&sym).map(|e| e.is_pure()).unwrap_or(true)
        } else {
            true
        }
    }

    pub fn function_has_io(&self, fn_name: &str) -> bool {
        if let Some(sym) = self.resolve_name(fn_name) {
            self.functions.get(&sym).map(|e| e.io).unwrap_or(false)
        } else {
            false
        }
    }

    /// Whether the partial evaluator may specialize a call to `fn_name` on a static
    /// argument. Reads, writes, allocation, and divergence are ordinary PE targets and
    /// stay safe. IO already blocks specialization (the historical gate). Nondeterminism
    /// and concurrency are the new exclusions: a concurrency / networking statement carries
    /// expression arguments (a timeout, spawn args, a branch body) that reference the
    /// static parameter, and the specializer's substitution does not enter those
    /// statements — folding across one would drop the parameter and leave a dangling
    /// reference. A `Check` (security_check) is deliberately *not* excluded: it is preserved
    /// verbatim and never references a foldable parameter, so specializing across it is
    /// sound (see `pe_effect_env_check_is_not_io`). Escape blocks (`unknown`) are handled by
    /// the caller's separate `body_has_escape` guard.
    pub fn function_is_specialization_safe(&self, fn_name: &str) -> bool {
        if let Some(sym) = self.resolve_name(fn_name) {
            match self.functions.get(&sym) {
                Some(e) => !e.io && !e.nondet && !e.concurrent,
                None => false,
            }
        } else {
            false
        }
    }
}

// =============================================================================
// Internal analysis
// =============================================================================

struct AnalysisResult {
    bindings: HashMap<Symbol, EffectSet>,
    aggregate: EffectSet,
    functions: HashMap<Symbol, EffectSet>,
}

fn analyze_program(stmts: &[Stmt<'_>], interner: &Interner) -> AnalysisResult {
    let callgraph = CallGraph::build(stmts, interner);

    // Phase 1: Analyze function bodies
    let mut functions: HashMap<Symbol, EffectSet> = HashMap::new();

    // Collect all known function names first
    let mut known_fns: HashSet<Symbol> = HashSet::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, .. } = stmt {
            known_fns.insert(*name);
        }
    }

    // Initialize all functions with their direct effects
    for stmt in stmts {
        if let Stmt::FunctionDef { name, body, is_native, params, .. } = stmt {
            if *is_native {
                // Native functions: classify by name
                let effects = classify_native_function(*name, interner);
                functions.insert(*name, effects);
            } else {
                let mut effects = analyze_block_effects_with(body, &known_fns);
                // Remove parameter reads — reading a parameter is not a side effect
                for (param_sym, _) in params {
                    effects.reads.remove(param_sym);
                }
                functions.insert(*name, effects);
            }
        }
    }

    // Fixed-point: propagate effects through call graph
    let mut changed = true;
    while changed {
        changed = false;
        for scc in &callgraph.sccs {
            // For each SCC, join effects of all callees
            for &fn_sym in scc {
                if callgraph.native_fns.contains(&fn_sym) {
                    continue;
                }
                let callees = callgraph.edges.get(&fn_sym).cloned().unwrap_or_default();
                for callee in &callees {
                    if let Some(callee_effects) = functions.get(callee).cloned() {
                        if let Some(fn_effects) = functions.get_mut(&fn_sym) {
                            let before = format!("{:?}", fn_effects);
                            fn_effects.join(&callee_effects);
                            let after = format!("{:?}", fn_effects);
                            if before != after {
                                changed = true;
                            }
                        }
                    }
                }
            }

            // Recursive SCCs always diverge
            if scc.len() > 1 {
                for &fn_sym in scc {
                    if let Some(fn_effects) = functions.get_mut(&fn_sym) {
                        if !fn_effects.diverges {
                            fn_effects.diverges = true;
                            changed = true;
                        }
                    }
                }
            } else if scc.len() == 1 {
                let fn_sym = scc[0];
                if callgraph.edges.get(&fn_sym).map(|e| e.contains(&fn_sym)).unwrap_or(false) {
                    if let Some(fn_effects) = functions.get_mut(&fn_sym) {
                        if !fn_effects.diverges {
                            fn_effects.diverges = true;
                            changed = true;
                        }
                    }
                }
            }
        }
    }

    // Phase 2: Analyze main body and bindings
    let mut bindings: HashMap<Symbol, EffectSet> = HashMap::new();
    let mut aggregate = EffectSet::default();

    for stmt in stmts {
        let effects = analyze_stmt_effects(stmt, &functions);
        aggregate.join(&effects);

        // Track per-binding effects
        if let Stmt::Let { var, value, .. } = stmt {
            let expr_effects = analyze_expr_effects(value);
            bindings.insert(*var, expr_effects);
        }
    }

    AnalysisResult {
        bindings,
        aggregate,
        functions,
    }
}

fn analyze_block_effects(stmts: &[Stmt<'_>]) -> EffectSet {
    analyze_block_effects_with(stmts, &HashSet::new())
}

fn analyze_block_effects_with(stmts: &[Stmt<'_>], known_fns: &HashSet<Symbol>) -> EffectSet {
    let mut result = EffectSet::default();
    for stmt in stmts {
        let effects = analyze_stmt_effects_initial(stmt, known_fns);
        result.join(&effects);
    }
    result
}

/// Analyze statement effects during initial function body analysis.
/// known_fns tracks which functions are defined in the program so calls to them
/// don't get marked as unknown (the fixed-point iteration handles transitive effects).
fn analyze_stmt_effects_initial(stmt: &Stmt<'_>, known_fns: &HashSet<Symbol>) -> EffectSet {
    analyze_stmt_effects_core(stmt, None, known_fns)
}

fn analyze_stmt_effects(stmt: &Stmt<'_>, functions: &HashMap<Symbol, EffectSet>) -> EffectSet {
    analyze_stmt_effects_core(stmt, Some(functions), &HashSet::new())
}

fn analyze_stmt_effects_core(
    stmt: &Stmt<'_>,
    functions: Option<&HashMap<Symbol, EffectSet>>,
    known_fns: &HashSet<Symbol>,
) -> EffectSet {
    match stmt {
        Stmt::Let { var, value, .. } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            effects.writes.insert(*var);
            effects
        }
        Stmt::Set { target, value } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            effects.writes.insert(*target);
            effects
        }
        Stmt::Call { function, args } => {
            let mut effects = EffectSet::default();
            for arg in args {
                effects.join(&analyze_expr_effects_core(arg, known_fns));
            }
            if let Some(fns) = functions {
                if let Some(fn_effects) = fns.get(function) {
                    effects.join(fn_effects);
                } else if !known_fns.contains(function) {
                    effects.unknown = true;
                }
            } else if !known_fns.contains(function) {
                effects.unknown = true;
            }
            effects
        }
        Stmt::If { cond, then_block, else_block } => {
            let mut effects = analyze_expr_effects_core(cond, known_fns);
            let then_effects = analyze_block_effects_with(then_block, known_fns);
            effects.join(&then_effects);
            if let Some(else_b) = else_block {
                let else_effects = analyze_block_effects_with(else_b, known_fns);
                effects.join(&else_effects);
            }
            effects
        }
        Stmt::While { cond, body, .. } => {
            let mut effects = analyze_expr_effects_core(cond, known_fns);
            let body_effects = analyze_block_effects_with(body, known_fns);
            effects.join(&body_effects);
            effects.diverges = true;
            effects
        }
        Stmt::Repeat { iterable, body, .. } => {
            let mut effects = analyze_expr_effects_core(iterable, known_fns);
            let body_effects = analyze_block_effects_with(body, known_fns);
            effects.join(&body_effects);
            effects
        }
        Stmt::Return { value } => {
            if let Some(v) = value {
                analyze_expr_effects_core(v, known_fns)
            } else {
                EffectSet::pure()
            }
        }
        Stmt::Show { object, .. } => {
            let mut effects = analyze_expr_effects_core(object, known_fns);
            effects.io = true;
            effects
        }
        Stmt::Push { value, collection } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            effects.join(&analyze_expr_effects_core(collection, known_fns));
            if let Expr::Identifier(sym) = collection {
                effects.writes.insert(*sym);
            }
            effects
        }
        Stmt::Pop { collection, into } => {
            let mut effects = analyze_expr_effects_core(collection, known_fns);
            if let Expr::Identifier(sym) = collection {
                effects.writes.insert(*sym);
            }
            if let Some(into_sym) = into {
                effects.writes.insert(*into_sym);
            }
            effects
        }
        Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            effects.join(&analyze_expr_effects_core(collection, known_fns));
            if let Expr::Identifier(sym) = collection {
                effects.writes.insert(*sym);
            }
            effects
        }
        Stmt::SetIndex { collection, index, value } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            effects.join(&analyze_expr_effects_core(index, known_fns));
            if let Expr::Identifier(sym) = collection {
                effects.writes.insert(*sym);
            }
            effects
        }
        Stmt::SetField { object, value, .. } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            if let Expr::Identifier(sym) = object {
                effects.writes.insert(*sym);
            }
            effects
        }
        Stmt::Give { object, recipient } => {
            let mut effects = analyze_expr_effects_core(object, known_fns);
            effects.join(&analyze_expr_effects_core(recipient, known_fns));
            effects
        }
        Stmt::Escape { .. } => EffectSet::unknown(),
        Stmt::Check { .. } => {
            let mut effects = EffectSet::default();
            effects.security_check = true;
            effects
        }
        Stmt::RuntimeAssert { condition, .. } => analyze_expr_effects_core(condition, known_fns),
        Stmt::FunctionDef { .. } | Stmt::StructDef { .. } => EffectSet::pure(),
        Stmt::Inspect { target, arms, .. } => {
            let mut effects = analyze_expr_effects_core(target, known_fns);
            for arm in arms {
                let arm_effects = analyze_block_effects_with(arm.body, known_fns);
                effects.join(&arm_effects);
            }
            effects
        }
        Stmt::Zone { body, .. } => analyze_block_effects_with(body, known_fns),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            let mut effects = analyze_block_effects_with(tasks, known_fns);
            effects.concurrent = true;
            effects
        }
        Stmt::WriteFile { content, path } => {
            let mut effects = analyze_expr_effects_core(content, known_fns);
            effects.join(&analyze_expr_effects_core(path, known_fns));
            effects.io = true;
            effects
        }
        Stmt::SendMessage { message, destination, .. } => {
            let mut effects = analyze_expr_effects_core(message, known_fns);
            effects.join(&analyze_expr_effects_core(destination, known_fns));
            effects.io = true;
            effects
        }
        Stmt::StreamMessage { values, destination } => {
            let mut effects = analyze_expr_effects_core(values, known_fns);
            effects.join(&analyze_expr_effects_core(destination, known_fns));
            effects.io = true;
            effects
        }
        Stmt::IncreaseCrdt { object, amount, .. } | Stmt::DecreaseCrdt { object, amount, .. } => {
            let mut effects = analyze_expr_effects_core(object, known_fns);
            effects.join(&analyze_expr_effects_core(amount, known_fns));
            if let Expr::Identifier(sym) = object {
                effects.writes.insert(*sym);
            }
            effects
        }
        Stmt::MergeCrdt { source, target } => {
            let mut effects = analyze_expr_effects_core(source, known_fns);
            effects.join(&analyze_expr_effects_core(target, known_fns));
            if let Expr::Identifier(sym) = target {
                effects.writes.insert(*sym);
            }
            effects
        }
        Stmt::Sleep { milliseconds } => {
            let mut effects = analyze_expr_effects_core(milliseconds, known_fns);
            effects.io = true;
            effects
        }
        Stmt::SendPipe { value, pipe } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            effects.join(&analyze_expr_effects_core(pipe, known_fns));
            effects.io = true;
            effects.diverges = true;
            effects.concurrent = true;
            effects
        }
        Stmt::TrySendPipe { value, pipe, result } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            effects.join(&analyze_expr_effects_core(pipe, known_fns));
            effects.io = true;
            effects.concurrent = true;
            // Success depends on instantaneous buffer state — nondeterministic.
            effects.nondet = true;
            if let Some(result_sym) = result {
                effects.writes.insert(*result_sym);
            }
            effects
        }
        Stmt::ReceivePipe { pipe, var, .. } => {
            let mut effects = analyze_expr_effects_core(pipe, known_fns);
            effects.writes.insert(*var);
            effects.io = true;
            effects.diverges = true;
            effects.concurrent = true;
            effects
        }
        Stmt::ReadFrom { var, .. } => {
            let mut effects = EffectSet::default();
            effects.writes.insert(*var);
            effects.io = true;
            effects
        }
        // ----- Go-like concurrency: opaque effectful boundaries -----
        Stmt::Select { branches } => {
            // A Select is a nondeterministic choice over its ready branches. Its outcome
            // is not a function of the inputs, and each branch body is part of the effect.
            let mut effects = EffectSet::nondet();
            for branch in branches {
                match branch {
                    crate::ast::stmt::SelectBranch::Receive { var, pipe, body } => {
                        effects.join(&analyze_expr_effects_core(pipe, known_fns));
                        effects.writes.insert(*var);
                        effects.join(&analyze_block_effects_with(body, known_fns));
                    }
                    crate::ast::stmt::SelectBranch::Timeout { milliseconds, body } => {
                        effects.join(&analyze_expr_effects_core(milliseconds, known_fns));
                        effects.join(&analyze_block_effects_with(body, known_fns));
                    }
                }
            }
            effects.concurrent = true;
            effects
        }
        Stmt::LaunchTask { args, .. } => {
            let mut effects = EffectSet::concurrent();
            for arg in args {
                effects.join(&analyze_expr_effects_core(arg, known_fns));
            }
            effects
        }
        Stmt::LaunchTaskWithHandle { handle, args, .. } => {
            let mut effects = EffectSet::concurrent();
            effects.writes.insert(*handle);
            for arg in args {
                effects.join(&analyze_expr_effects_core(arg, known_fns));
            }
            effects
        }
        Stmt::CreatePipe { var, .. } => {
            let mut effects = EffectSet::concurrent();
            effects.allocates = true;
            effects.writes.insert(*var);
            effects
        }
        Stmt::TryReceivePipe { var, pipe } => {
            // Non-blocking receive: returns value-or-Nothing depending on instantaneous
            // buffer state — nondeterministic, and it binds `var`.
            let mut effects = analyze_expr_effects_core(pipe, known_fns);
            effects.writes.insert(*var);
            effects.io = true;
            effects.nondet = true;
            effects.concurrent = true;
            effects
        }
        Stmt::StopTask { handle } => {
            // Cooperative cancellation — scheduling-dependent.
            let mut effects = analyze_expr_effects_core(handle, known_fns);
            effects.nondet = true;
            effects.concurrent = true;
            effects
        }
        Stmt::Spawn { name, .. } => {
            let mut effects = EffectSet::concurrent();
            effects.writes.insert(*name);
            effects
        }
        // ----- Networking over the relay: I/O boundaries -----
        Stmt::AwaitMessage { source, into, view: _, stream: _ } => {
            let mut effects = analyze_expr_effects_core(source, known_fns);
            effects.writes.insert(*into);
            effects.io = true;
            effects.concurrent = true;
            effects
        }
        Stmt::Listen { address, secure } | Stmt::ConnectTo { address, secure } => {
            let mut effects = analyze_expr_effects_core(address, known_fns);
            if let Some(bind) = secure {
                // The pad path is evaluated too; fold in its (read-only) effects for completeness.
                let pad_fx = analyze_expr_effects_core(bind.pad, known_fns);
                effects.io |= pad_fx.io;
                effects.concurrent |= pad_fx.concurrent;
                effects.writes.extend(pad_fx.writes);
            }
            effects.io = true;
            effects.concurrent = true;
            effects
        }
        Stmt::LetPeerAgent { var, address } => {
            let mut effects = analyze_expr_effects_core(address, known_fns);
            effects.writes.insert(*var);
            effects.io = true;
            effects.concurrent = true;
            effects
        }
        Stmt::Sync { var, topic } => {
            // Subscribe + auto-publish-on-mutation + auto-merge-on-receive: reads and
            // writes the synced CRDT and talks to the relay.
            let mut effects = analyze_expr_effects_core(topic, known_fns);
            effects.reads.insert(*var);
            effects.writes.insert(*var);
            effects.io = true;
            effects.concurrent = true;
            effects
        }
        Stmt::Mount { var, path } => {
            let mut effects = analyze_expr_effects_core(path, known_fns);
            effects.writes.insert(*var);
            effects.io = true;
            effects
        }
        // ----- CRDT structural mutations not covered above -----
        Stmt::AppendToSequence { sequence, value } => {
            let mut effects = analyze_expr_effects_core(sequence, known_fns);
            effects.join(&analyze_expr_effects_core(value, known_fns));
            if let Expr::Identifier(sym) = sequence {
                effects.writes.insert(*sym);
            }
            effects
        }
        Stmt::ResolveConflict { object, value, .. } => {
            let mut effects = analyze_expr_effects_core(object, known_fns);
            effects.join(&analyze_expr_effects_core(value, known_fns));
            if let Expr::Identifier(sym) = object {
                effects.writes.insert(*sym);
            }
            effects
        }
        _ => EffectSet::default(),
    }
}

fn analyze_expr_effects(expr: &Expr<'_>) -> EffectSet {
    analyze_expr_effects_core(expr, &HashSet::new())
}

fn analyze_expr_effects_core(expr: &Expr<'_>, known_fns: &HashSet<Symbol>) -> EffectSet {
    match expr {
        Expr::Literal(_) | Expr::OptionNone => EffectSet::pure(),
        Expr::Identifier(sym) => EffectSet::read(*sym),
        Expr::BinaryOp { left, right, .. } => {
            let mut effects = analyze_expr_effects_core(left, known_fns);
            effects.join(&analyze_expr_effects_core(right, known_fns));
            effects
        }
        Expr::Not { operand } => analyze_expr_effects_core(operand, known_fns),
        Expr::Call { function, args, .. } => {
            let mut effects = EffectSet::default();
            for arg in args {
                effects.join(&analyze_expr_effects_core(arg, known_fns));
            }
            // Only mark as unknown if the callee is not a known function
            if !known_fns.contains(function) {
                effects.unknown = true;
            }
            effects
        }
        Expr::CallExpr { callee, args } => {
            let mut effects = analyze_expr_effects_core(callee, known_fns);
            for arg in args {
                effects.join(&analyze_expr_effects_core(arg, known_fns));
            }
            effects.unknown = true;
            effects
        }
        Expr::Index { collection, index } => {
            let mut effects = analyze_expr_effects_core(collection, known_fns);
            effects.join(&analyze_expr_effects_core(index, known_fns));
            effects
        }
        Expr::Slice { collection, start, end } => {
            let mut effects = analyze_expr_effects_core(collection, known_fns);
            effects.join(&analyze_expr_effects_core(start, known_fns));
            effects.join(&analyze_expr_effects_core(end, known_fns));
            effects
        }
        Expr::Length { collection } => analyze_expr_effects_core(collection, known_fns),
        Expr::Contains { collection, value } => {
            let mut effects = analyze_expr_effects_core(collection, known_fns);
            effects.join(&analyze_expr_effects_core(value, known_fns));
            effects
        }
        Expr::Union { left, right } | Expr::Intersection { left, right }
        | Expr::Range { start: left, end: right } => {
            let mut effects = analyze_expr_effects_core(left, known_fns);
            effects.join(&analyze_expr_effects_core(right, known_fns));
            effects
        }
        Expr::Copy { expr } => {
            let mut effects = analyze_expr_effects_core(expr, known_fns);
            effects.allocates = true;
            effects
        }
        Expr::Give { value } => analyze_expr_effects_core(value, known_fns),
        Expr::FieldAccess { object, .. } => analyze_expr_effects_core(object, known_fns),
        Expr::OptionSome { value } => analyze_expr_effects_core(value, known_fns),
        Expr::ManifestOf { zone } => analyze_expr_effects_core(zone, known_fns),
        Expr::ChunkAt { index, zone } => {
            let mut effects = analyze_expr_effects_core(index, known_fns);
            effects.join(&analyze_expr_effects_core(zone, known_fns));
            effects
        }
        Expr::New { init_fields, .. } => {
            let mut effects = EffectSet::default();
            effects.allocates = true;
            for (_, val) in init_fields {
                effects.join(&analyze_expr_effects_core(val, known_fns));
            }
            effects
        }
        Expr::NewVariant { fields, .. } => {
            let mut effects = EffectSet::default();
            for (_, val) in fields {
                effects.join(&analyze_expr_effects_core(val, known_fns));
            }
            effects
        }
        Expr::WithCapacity { value, capacity } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            effects.join(&analyze_expr_effects_core(capacity, known_fns));
            effects.allocates = true;
            effects
        }
        Expr::List(elems) | Expr::Tuple(elems) => {
            let mut effects = EffectSet::default();
            for elem in elems {
                effects.join(&analyze_expr_effects_core(elem, known_fns));
            }
            effects
        }
        Expr::Closure { .. } => EffectSet::pure(),
        Expr::Escape { .. } => EffectSet::unknown(),
        Expr::InterpolatedString(parts) => {
            let mut effects = EffectSet::default();
            for part in parts {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = part {
                    effects.join(&analyze_expr_effects_core(value, known_fns));
                }
            }
            effects
        }
    }
}

/// Classify a native function's effects by name.
fn classify_native_function(sym: Symbol, interner: &Interner) -> EffectSet {
    let name = interner.resolve(sym);
    match name {
        // Pure functions
        "parseInt" | "parseFloat" | "decimal" | "complex" | "modular" | "quantity" | "convert" | "money"
        | "uuid" | "uuid_nil" | "uuid_max" | "uuid_v3" | "uuid_v5" | "uuid_version"
        | "uuid_dns" | "uuid_url" | "uuid_oid" | "uuid_x500"
        | "md5" | "sha1" | "text_bytes" | "uuid_bytes" | "uuid_from_bytes"
        | "parse_timestamp" | "format_timestamp" | "year_of" | "month_of" | "day_of" | "weekday_of"
        | "hour_of" | "minute_of" | "second_of" | "week_of" | "quarter_of" | "date_of" | "time_of"
        | "seconds_between" | "months_between" | "years_between" | "add_seconds" | "in_zone"
        | "local_instant" | "abs" | "min" | "max" | "sqrt" | "floor"
        | "ceil" | "round" | "pow" | "log" | "sin" | "cos" | "tan"
        | "toString" | "toInt" | "toFloat" | "trim" | "uppercase" | "lowercase"
        | "split" | "join" | "replace" | "startsWith" | "endsWith"
        | "substring" | "charAt" | "indexOf" | "lastIndexOf" | "repeat" => {
            EffectSet::pure()
        }
        // IO functions
        "args" | "show" | "readLine" | "sleep" | "readFile" | "writeFile"
        | "print" | "println" | "eprintln" | "exit" => {
            EffectSet::io()
        }
        // Alloc functions
        "newSeq" | "newMap" | "newSet" | "copy" | "clone" | "mapOf" | "setOf" | "repeatSeq" => {
            EffectSet::alloc()
        }
        // Default: IO (conservative for unknown natives)
        _ => EffectSet::io(),
    }
}

#[cfg(test)]
mod concurrency_effect_tests {
    //! A concurrency / networking construct is an effectful, often nondeterministic
    //! boundary. The partial evaluator's specialization gate (`partial_eval.rs`) keys
    //! off these effects; if any of them are misclassified as Pure, a function holding
    //! one can be specialized and a static parameter flowing into it dropped, leaving a
    //! dangling free variable. These tests pin the classification at the source.

    use super::EffectEnv;

    #[test]
    fn effects_select_is_not_pure() {
        // A `Select` is a nondeterministic choice — never Pure. The classifier must also
        // see *into* the branch bodies (the `Show` here is an effect of the function).
        let src = "\
## To sel (ch: Int):
    Await the first of:
        Receive x from ch:
            Show x.
        After 1 seconds:
            Show 0.

## Main
    Let ch be a Pipe of Int.
    Launch a task to sel with ch.
";
        let env = EffectEnv::analyze_source(src).expect("parse");
        assert!(
            !env.function_is_pure("sel"),
            "a function whose body is a Select must not be classified Pure"
        );
    }

    #[test]
    fn effects_launch_task_not_pure() {
        let src = "\
## To helper (ch: Int):
    Receive y from ch.
    Show y.

## To launcher (ch: Int):
    Launch a task to helper with ch.

## Main
    Let ch be a Pipe of Int.
    Launch a task to launcher with ch.
";
        let env = EffectEnv::analyze_source(src).expect("parse");
        assert!(
            !env.function_is_pure("launcher"),
            "a function that launches a task is concurrent, not Pure"
        );
    }

    #[test]
    fn effects_try_receive_nondet() {
        let src = "\
## To tryer (ch: Int):
    Try to receive v from ch.

## Main
    Let ch be a Pipe of Int.
    Launch a task to tryer with ch.
";
        let env = EffectEnv::analyze_source(src).expect("parse");
        assert!(
            !env.function_is_pure("tryer"),
            "a non-blocking receive depends on instantaneous buffer state — not Pure"
        );
    }

    #[test]
    fn effects_mount_writes_var() {
        let src = "\
## Main
    Mount counter at \"data/counter.journal\".
    Show counter.
";
        let env = EffectEnv::analyze_source(src).expect("parse");
        assert!(
            env.has_write_to("counter"),
            "Mount binds its variable — the write must be recorded"
        );
    }
}
