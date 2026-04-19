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

    pub fn is_pure(&self) -> bool {
        self.reads.is_empty()
            && self.writes.is_empty()
            && !self.allocates
            && !self.io
            && !self.security_check
            && !self.diverges
            && !self.unknown
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
        Stmt::RuntimeAssert { condition } => analyze_expr_effects_core(condition, known_fns),
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
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => analyze_block_effects_with(tasks, known_fns),
        Stmt::WriteFile { content, path } => {
            let mut effects = analyze_expr_effects_core(content, known_fns);
            effects.join(&analyze_expr_effects_core(path, known_fns));
            effects.io = true;
            effects
        }
        Stmt::SendMessage { message, destination } => {
            let mut effects = analyze_expr_effects_core(message, known_fns);
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
            effects
        }
        Stmt::TrySendPipe { value, pipe, .. } => {
            let mut effects = analyze_expr_effects_core(value, known_fns);
            effects.join(&analyze_expr_effects_core(pipe, known_fns));
            effects.io = true;
            effects
        }
        Stmt::ReceivePipe { pipe, var, .. } => {
            let mut effects = analyze_expr_effects_core(pipe, known_fns);
            effects.writes.insert(*var);
            effects.io = true;
            effects.diverges = true;
            effects
        }
        Stmt::ReadFrom { var, .. } => {
            let mut effects = EffectSet::default();
            effects.writes.insert(*var);
            effects.io = true;
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
        Expr::UnaryOp { .. } => {
            unreachable!("HW-Spec UnaryOp not emitted outside ## Hardware/Property blocks (in effects analysis)")
        }
        Expr::BitSelect { .. } => {
            unreachable!("HW-Spec BitSelect not emitted outside ## Hardware/Property blocks (in effects analysis)")
        }
        Expr::PartSelect { .. } => {
            unreachable!("HW-Spec PartSelect not emitted outside ## Hardware/Property blocks (in effects analysis)")
        }
        Expr::HwConcat { .. } => {
            unreachable!("HW-Spec HwConcat not emitted outside ## Hardware/Property blocks (in effects analysis)")
        }
    }
}

/// Classify a native function's effects by name.
fn classify_native_function(sym: Symbol, interner: &Interner) -> EffectSet {
    let name = interner.resolve(sym);
    match name {
        // Pure functions
        "parseInt" | "parseFloat" | "abs" | "min" | "max" | "sqrt" | "floor"
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
        "newSeq" | "newMap" | "newSet" | "copy" | "clone" => {
            EffectSet::alloc()
        }
        // Default: IO (conservative for unknown natives)
        _ => EffectSet::io(),
    }
}
