//! Binding-Time Analysis (BTA)
//!
//! Classifies every variable and expression as Static (S) — value known at compile
//! time — or Dynamic (D) — value depends on runtime input. This classification
//! drives function specialization in the partial evaluator.
//!
//! BTA is polyvariant: the same function analyzed at different call sites with
//! different argument divisions produces different results.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use crate::analysis::DiscoveryPass;
use crate::arena::Arena;
use crate::arena_ctx::AstContext;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, Stmt, TypeExpr};
use crate::drs::WorldState;
use crate::error::ParseError;
use crate::intern::{Interner, Symbol};
use crate::lexer::Lexer;
use crate::parser::Parser;

// =============================================================================
// Core Types
// =============================================================================

#[derive(Debug, Clone)]
pub enum BindingTime {
    Static(Literal),
    Dynamic,
}

impl PartialEq for BindingTime {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (BindingTime::Static(a), BindingTime::Static(b)) => match (a, b) {
                (Literal::Float(x), Literal::Float(y)) => x.to_bits() == y.to_bits(),
                _ => a == b,
            },
            (BindingTime::Dynamic, BindingTime::Dynamic) => true,
            _ => false,
        }
    }
}

impl Eq for BindingTime {}

impl Hash for BindingTime {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            BindingTime::Static(lit) => hash_literal(lit, state),
            BindingTime::Dynamic => {}
        }
    }
}

fn hash_literal<H: Hasher>(lit: &Literal, state: &mut H) {
    std::mem::discriminant(lit).hash(state);
    match lit {
        Literal::Number(n) => n.hash(state),
        Literal::Float(f) => f.to_bits().hash(state),
        Literal::Text(s) => s.hash(state),
        Literal::Boolean(b) => b.hash(state),
        Literal::Nothing => {}
        Literal::Char(c) => c.hash(state),
        Literal::Duration(d) => d.hash(state),
        Literal::Date(d) => d.hash(state),
        Literal::Moment(m) => m.hash(state),
        Literal::Span { months, days } => {
            months.hash(state);
            days.hash(state);
        }
        Literal::Time(t) => t.hash(state),
    }
}

impl BindingTime {
    pub fn is_static(&self) -> bool {
        matches!(self, BindingTime::Static(_))
    }

    pub fn is_dynamic(&self) -> bool {
        matches!(self, BindingTime::Dynamic)
    }
}

pub type Division = HashMap<Symbol, BindingTime>;

#[derive(Debug, Clone, PartialEq)]
pub struct BtaResult {
    pub division: Division,
    pub return_bt: BindingTime,
}

pub type BtaCache = HashMap<(Symbol, Vec<BindingTime>), BtaResult>;

// =============================================================================
// Expression Analysis
// =============================================================================

pub fn analyze_expr(expr: &Expr, division: &Division) -> BindingTime {
    match expr {
        Expr::Literal(lit) => BindingTime::Static(lit.clone()),

        Expr::Identifier(sym) => {
            division.get(sym).cloned().unwrap_or(BindingTime::Dynamic)
        }

        Expr::BinaryOp { op, left, right } => {
            let bt_left = analyze_expr(left, division);
            let bt_right = analyze_expr(right, division);
            match (&bt_left, &bt_right) {
                (BindingTime::Static(l), BindingTime::Static(r)) => {
                    match eval_literal_binop(*op, l, r) {
                        Some(result) => BindingTime::Static(result),
                        None => BindingTime::Dynamic,
                    }
                }
                _ => BindingTime::Dynamic,
            }
        }

        Expr::Not { operand } => {
            match analyze_expr(operand, division) {
                BindingTime::Static(Literal::Boolean(b)) => {
                    BindingTime::Static(Literal::Boolean(!b))
                }
                _ => BindingTime::Dynamic,
            }
        }

        Expr::Length { .. } => BindingTime::Dynamic,
        Expr::Index { .. } => BindingTime::Dynamic,

        Expr::Call { .. } => BindingTime::Dynamic,

        _ => BindingTime::Dynamic,
    }
}

fn eval_literal_binop(op: BinaryOpKind, left: &Literal, right: &Literal) -> Option<Literal> {
    match (op, left, right) {
        (BinaryOpKind::Add, Literal::Number(a), Literal::Number(b)) => {
            Some(Literal::Number(a.wrapping_add(*b)))
        }
        (BinaryOpKind::Subtract, Literal::Number(a), Literal::Number(b)) => {
            Some(Literal::Number(a.wrapping_sub(*b)))
        }
        (BinaryOpKind::Multiply, Literal::Number(a), Literal::Number(b)) => {
            Some(Literal::Number(a.wrapping_mul(*b)))
        }
        (BinaryOpKind::Divide, Literal::Number(a), Literal::Number(b)) if *b != 0 => {
            Some(Literal::Number(a / b))
        }
        (BinaryOpKind::Modulo, Literal::Number(a), Literal::Number(b)) if *b != 0 => {
            Some(Literal::Number(a % b))
        }
        (BinaryOpKind::Eq, Literal::Number(a), Literal::Number(b)) => {
            Some(Literal::Boolean(a == b))
        }
        (BinaryOpKind::NotEq, Literal::Number(a), Literal::Number(b)) => {
            Some(Literal::Boolean(a != b))
        }
        (BinaryOpKind::Lt, Literal::Number(a), Literal::Number(b)) => {
            Some(Literal::Boolean(a < b))
        }
        (BinaryOpKind::Gt, Literal::Number(a), Literal::Number(b)) => {
            Some(Literal::Boolean(a > b))
        }
        (BinaryOpKind::LtEq, Literal::Number(a), Literal::Number(b)) => {
            Some(Literal::Boolean(a <= b))
        }
        (BinaryOpKind::GtEq, Literal::Number(a), Literal::Number(b)) => {
            Some(Literal::Boolean(a >= b))
        }
        (BinaryOpKind::Add, Literal::Float(a), Literal::Float(b)) => {
            Some(Literal::Float(a + b))
        }
        (BinaryOpKind::Subtract, Literal::Float(a), Literal::Float(b)) => {
            Some(Literal::Float(a - b))
        }
        (BinaryOpKind::Multiply, Literal::Float(a), Literal::Float(b)) => {
            Some(Literal::Float(a * b))
        }
        (BinaryOpKind::Divide, Literal::Float(a), Literal::Float(b)) if *b != 0.0 => {
            Some(Literal::Float(a / b))
        }
        (BinaryOpKind::And, Literal::Boolean(a), Literal::Boolean(b)) => {
            Some(Literal::Boolean(*a && *b))
        }
        (BinaryOpKind::Or, Literal::Boolean(a), Literal::Boolean(b)) => {
            Some(Literal::Boolean(*a || *b))
        }
        _ => None,
    }
}

// =============================================================================
// Statement / Block Analysis
// =============================================================================

fn analyze_block<'a>(
    stmts: &[Stmt<'a>],
    division: &mut Division,
    funcs: &HashMap<Symbol, FuncDef<'a>>,
    cache: &mut BtaCache,
) -> BindingTime {
    let mut return_bt = BindingTime::Dynamic;
    for stmt in stmts {
        if let Some(bt) = analyze_stmt(stmt, division, funcs, cache) {
            return_bt = bt;
        }
    }
    return_bt
}

fn analyze_stmt<'a>(
    stmt: &Stmt<'a>,
    division: &mut Division,
    funcs: &HashMap<Symbol, FuncDef<'a>>,
    cache: &mut BtaCache,
) -> Option<BindingTime> {
    match stmt {
        Stmt::Let { var, value, .. } => {
            let bt = analyze_expr(value, division);
            division.insert(*var, bt);
            None
        }
        Stmt::Set { target, value } => {
            let bt = analyze_expr(value, division);
            division.insert(*target, bt);
            None
        }
        Stmt::Return { value } => {
            let bt = match value {
                Some(expr) => analyze_expr(expr, division),
                None => BindingTime::Static(Literal::Nothing),
            };
            Some(bt)
        }
        Stmt::Show { .. } => None,
        Stmt::Call { function, args } => {
            for arg in args {
                analyze_expr(arg, division);
            }
            None
        }
        _ => None,
    }
}

// =============================================================================
// Function Definition Storage
// =============================================================================

struct FuncDef<'a> {
    params: Vec<(Symbol, &'a TypeExpr<'a>)>,
    body: Block<'a>,
}

pub fn analyze_function_bt<'a>(
    func_name: Symbol,
    params: &[(Symbol, &'a TypeExpr<'a>)],
    body: &[Stmt<'a>],
    arg_bts: &[BindingTime],
    funcs: &HashMap<Symbol, FuncDef<'a>>,
    cache: &mut BtaCache,
) -> BtaResult {
    let cache_key = (func_name, arg_bts.to_vec());
    if let Some(cached) = cache.get(&cache_key) {
        return cached.clone();
    }

    let mut division = Division::new();
    for (i, (param_sym, _)) in params.iter().enumerate() {
        if let Some(bt) = arg_bts.get(i) {
            division.insert(*param_sym, bt.clone());
        } else {
            division.insert(*param_sym, BindingTime::Dynamic);
        }
    }

    let return_bt = analyze_block(body, &mut division, funcs, cache);

    let result = BtaResult {
        division,
        return_bt,
    };
    cache.insert(cache_key, result.clone());
    result
}

// =============================================================================
// SCC-Ordered Analysis
// =============================================================================

pub fn analyze_with_sccs<'a>(
    stmts: &[Stmt<'a>],
    interner: &Interner,
) -> BtaCache {
    use crate::analysis::callgraph::CallGraph;

    let cg = CallGraph::build(stmts, interner);

    let mut funcs: HashMap<Symbol, FuncDef<'a>> = HashMap::new();
    for stmt in stmts {
        if let Stmt::FunctionDef { name, params, body, is_native: false, .. } = stmt {
            funcs.insert(*name, FuncDef { params: params.clone(), body });
        }
    }

    let mut cache = BtaCache::new();

    // Process SCCs in topological order (sccs are already in reverse topo order from Kosaraju)
    for scc in cg.sccs.iter().rev() {
        if scc.len() == 1 {
            // Non-recursive (or self-recursive): analyze once
            let sym = scc[0];
            if let Some(func_def) = funcs.get(&sym) {
                let arg_bts: Vec<BindingTime> = func_def.params.iter()
                    .map(|_| BindingTime::Dynamic)
                    .collect();
                analyze_function_bt(sym, &func_def.params, func_def.body, &arg_bts, &funcs, &mut cache);
            }
        } else {
            // Mutually recursive SCC: iterate to fixed point
            for _iteration in 0..10 {
                let mut changed = false;
                for &sym in scc {
                    if let Some(func_def) = funcs.get(&sym) {
                        let arg_bts: Vec<BindingTime> = func_def.params.iter()
                            .map(|_| BindingTime::Dynamic)
                            .collect();
                        let key = (sym, arg_bts.clone());
                        let old = cache.get(&key).cloned();
                        let result = analyze_function_bt(sym, &func_def.params, func_def.body, &arg_bts, &funcs, &mut cache);
                        if old.as_ref() != Some(&result) {
                            changed = true;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
        }
    }

    cache
}

// =============================================================================
// Test-Friendly Wrapper
// =============================================================================

pub struct BtaEnv {
    interner: Interner,
    main_stmts: Vec<BtaStmt>,
    functions: Vec<BtaFunc>,
}

#[derive(Clone)]
struct BtaStmt {
    kind: BtaStmtKind,
}

#[derive(Clone)]
enum BtaStmtKind {
    Let { var: Symbol, value: BtaExpr },
    Set { target: Symbol, value: BtaExpr },
    Return { value: Option<BtaExpr> },
    Show,
    Call { function: Symbol, args: Vec<BtaExpr> },
    If { cond: BtaExpr, then_block: Vec<BtaStmt>, else_block: Option<Vec<BtaStmt>> },
    While { cond: BtaExpr, body: Vec<BtaStmt> },
    Other,
}

#[derive(Clone)]
enum BtaExpr {
    Literal(Literal),
    Identifier(Symbol),
    BinaryOp { op: BinaryOpKind, left: Box<BtaExpr>, right: Box<BtaExpr> },
    Not { operand: Box<BtaExpr> },
    Length { collection: Box<BtaExpr> },
    Index { collection: Box<BtaExpr>, index: Box<BtaExpr> },
    Call { function: Symbol, args: Vec<BtaExpr> },
    Other,
}

#[derive(Clone)]
struct BtaFunc {
    name: Symbol,
    params: Vec<(Symbol, bool)>, // (name, is_collection_type)
    body: Vec<BtaStmt>,
}

struct BtaContext {
    cache: BtaCache,
    on_stack: HashSet<(Symbol, Vec<BindingTime>)>,
}

fn convert_expr(expr: &Expr) -> BtaExpr {
    match expr {
        Expr::Literal(lit) => BtaExpr::Literal(lit.clone()),
        Expr::Identifier(sym) => BtaExpr::Identifier(*sym),
        Expr::BinaryOp { op, left, right } => BtaExpr::BinaryOp {
            op: *op,
            left: Box::new(convert_expr(left)),
            right: Box::new(convert_expr(right)),
        },
        Expr::Not { operand } => BtaExpr::Not {
            operand: Box::new(convert_expr(operand)),
        },
        Expr::Length { collection } => BtaExpr::Length {
            collection: Box::new(convert_expr(collection)),
        },
        Expr::Index { collection, index } => BtaExpr::Index {
            collection: Box::new(convert_expr(collection)),
            index: Box::new(convert_expr(index)),
        },
        Expr::Call { function, args } => BtaExpr::Call {
            function: *function,
            args: args.iter().map(|a| convert_expr(a)).collect(),
        },
        _ => BtaExpr::Other,
    }
}

fn convert_stmt(stmt: &Stmt) -> BtaStmt {
    let kind = match stmt {
        Stmt::Let { var, value, .. } => BtaStmtKind::Let {
            var: *var,
            value: convert_expr(value),
        },
        Stmt::Set { target, value } => BtaStmtKind::Set {
            target: *target,
            value: convert_expr(value),
        },
        Stmt::Return { value } => BtaStmtKind::Return {
            value: value.map(|v| convert_expr(v)),
        },
        Stmt::Show { .. } => BtaStmtKind::Show,
        Stmt::Call { function, args } => BtaStmtKind::Call {
            function: *function,
            args: args.iter().map(|a| convert_expr(a)).collect(),
        },
        Stmt::If { cond, then_block, else_block } => BtaStmtKind::If {
            cond: convert_expr(cond),
            then_block: then_block.iter().map(|s| convert_stmt(s)).collect(),
            else_block: else_block.map(|eb| eb.iter().map(|s| convert_stmt(s)).collect()),
        },
        Stmt::While { cond, body, .. } => BtaStmtKind::While {
            cond: convert_expr(cond),
            body: body.iter().map(|s| convert_stmt(s)).collect(),
        },
        _ => BtaStmtKind::Other,
    };
    BtaStmt { kind }
}

fn convert_block(block: &[Stmt]) -> Vec<BtaStmt> {
    block.iter().map(|s| convert_stmt(s)).collect()
}

fn is_collection_type(ty: &TypeExpr) -> bool {
    match ty {
        TypeExpr::Generic { .. } => true,
        _ => false,
    }
}

fn analyze_bta_expr(
    expr: &BtaExpr,
    division: &Division,
    functions: &[BtaFunc],
    ctx: &mut BtaContext,
) -> BindingTime {
    match expr {
        BtaExpr::Literal(lit) => BindingTime::Static(lit.clone()),
        BtaExpr::Identifier(sym) => {
            division.get(sym).cloned().unwrap_or(BindingTime::Dynamic)
        }
        BtaExpr::BinaryOp { op, left, right } => {
            let bt_left = analyze_bta_expr(left, division, functions, ctx);
            let bt_right = analyze_bta_expr(right, division, functions, ctx);
            match (&bt_left, &bt_right) {
                (BindingTime::Static(l), BindingTime::Static(r)) => {
                    match eval_literal_binop(*op, l, r) {
                        Some(result) => BindingTime::Static(result),
                        None => BindingTime::Dynamic,
                    }
                }
                _ => BindingTime::Dynamic,
            }
        }
        BtaExpr::Not { operand } => {
            match analyze_bta_expr(operand, division, functions, ctx) {
                BindingTime::Static(Literal::Boolean(b)) => {
                    BindingTime::Static(Literal::Boolean(!b))
                }
                _ => BindingTime::Dynamic,
            }
        }
        BtaExpr::Length { .. } => BindingTime::Dynamic,
        BtaExpr::Index { .. } => BindingTime::Dynamic,
        BtaExpr::Call { function, args } => {
            let arg_bts: Vec<BindingTime> = args.iter()
                .map(|a| analyze_bta_expr(a, division, functions, ctx))
                .collect();

            let cache_key = (*function, arg_bts.clone());

            if let Some(cached) = ctx.cache.get(&cache_key) {
                return cached.return_bt.clone();
            }

            if ctx.on_stack.contains(&cache_key) {
                return BindingTime::Dynamic;
            }

            let func = match functions.iter().find(|f| f.name == *function) {
                Some(f) => f.clone(),
                None => return BindingTime::Dynamic,
            };

            ctx.on_stack.insert(cache_key.clone());

            let mut func_div = Division::new();
            for (i, (param_sym, is_collection)) in func.params.iter().enumerate() {
                if *is_collection {
                    func_div.insert(*param_sym, BindingTime::Dynamic);
                } else if let Some(bt) = arg_bts.get(i) {
                    func_div.insert(*param_sym, bt.clone());
                } else {
                    func_div.insert(*param_sym, BindingTime::Dynamic);
                }
            }

            let return_bt = analyze_bta_block(&func.body, &mut func_div, functions, ctx);

            ctx.on_stack.remove(&cache_key);

            let result = BtaResult {
                division: func_div,
                return_bt: return_bt.clone(),
            };
            ctx.cache.insert(cache_key, result);

            return_bt
        }
        BtaExpr::Other => BindingTime::Dynamic,
    }
}

fn analyze_bta_block(
    stmts: &[BtaStmt],
    division: &mut Division,
    functions: &[BtaFunc],
    ctx: &mut BtaContext,
) -> BindingTime {
    analyze_bta_block_opt(stmts, division, functions, ctx)
        .unwrap_or(BindingTime::Dynamic)
}

fn analyze_bta_block_opt(
    stmts: &[BtaStmt],
    division: &mut Division,
    functions: &[BtaFunc],
    ctx: &mut BtaContext,
) -> Option<BindingTime> {
    for stmt in stmts {
        if let Some(bt) = analyze_bta_stmt(stmt, division, functions, ctx) {
            return Some(bt);
        }
    }
    None
}

fn join_bt(a: &BindingTime, b: &BindingTime) -> BindingTime {
    match (a, b) {
        (BindingTime::Static(l1), BindingTime::Static(l2)) => {
            if l1 == l2 {
                a.clone()
            } else {
                BindingTime::Dynamic
            }
        }
        _ => BindingTime::Dynamic,
    }
}

fn join_divisions(a: &Division, b: &Division) -> Division {
    let mut joined = Division::new();
    for (sym, bt_a) in a {
        if let Some(bt_b) = b.get(sym) {
            joined.insert(*sym, join_bt(bt_a, bt_b));
        }
    }
    // Variables only in one branch get their value from that branch,
    // but since the other branch doesn't define them, treat as Dynamic
    for (sym, _) in b {
        if !a.contains_key(sym) {
            joined.insert(*sym, BindingTime::Dynamic);
        }
    }
    for (sym, _) in a {
        if !b.contains_key(sym) {
            joined.insert(*sym, BindingTime::Dynamic);
        }
    }
    joined
}

fn analyze_bta_stmt(
    stmt: &BtaStmt,
    division: &mut Division,
    functions: &[BtaFunc],
    ctx: &mut BtaContext,
) -> Option<BindingTime> {
    match &stmt.kind {
        BtaStmtKind::Let { var, value } => {
            let bt = analyze_bta_expr(value, division, functions, ctx);
            division.insert(*var, bt);
            None
        }
        BtaStmtKind::Set { target, value } => {
            let bt = analyze_bta_expr(value, division, functions, ctx);
            division.insert(*target, bt);
            None
        }
        BtaStmtKind::Return { value } => {
            let bt = match value {
                Some(expr) => analyze_bta_expr(expr, division, functions, ctx),
                None => BindingTime::Static(Literal::Nothing),
            };
            Some(bt)
        }
        BtaStmtKind::Show => None,
        BtaStmtKind::Call { .. } => None,
        BtaStmtKind::If { cond, then_block, else_block } => {
            let cond_bt = analyze_bta_expr(cond, division, functions, ctx);
            match cond_bt {
                BindingTime::Static(Literal::Boolean(true)) => {
                    analyze_bta_block_opt(then_block, division, functions, ctx)
                }
                BindingTime::Static(Literal::Boolean(false)) => {
                    if let Some(else_b) = else_block {
                        analyze_bta_block_opt(else_b, division, functions, ctx)
                    } else {
                        None
                    }
                }
                _ => {
                    let snapshot = division.clone();

                    let then_ret = analyze_bta_block_opt(then_block, division, functions, ctx);
                    let then_div = division.clone();

                    *division = snapshot;
                    let else_ret = if let Some(else_b) = else_block {
                        analyze_bta_block_opt(else_b, division, functions, ctx)
                    } else {
                        None
                    };
                    let else_div = division.clone();

                    *division = join_divisions(&then_div, &else_div);
                    match (then_ret, else_ret) {
                        (Some(t), Some(e)) => Some(join_bt(&t, &e)),
                        _ => None,
                    }
                }
            }
        }
        BtaStmtKind::While { cond, body } => {
            let max_iterations = 256;
            for _ in 0..max_iterations {
                let cond_bt = analyze_bta_expr(cond, division, functions, ctx);

                match cond_bt {
                    BindingTime::Static(Literal::Boolean(false)) => break,
                    BindingTime::Static(Literal::Boolean(true)) => {
                        // Static true: unroll this iteration directly
                        analyze_bta_block(body, division, functions, ctx);
                    }
                    _ => {
                        // Dynamic condition: fixed-point with join
                        let snapshot = division.clone();
                        let mut body_div = snapshot.clone();
                        analyze_bta_block(body, &mut body_div, functions, ctx);

                        let joined = join_divisions(&snapshot, &body_div);
                        if joined == *division {
                            break;
                        }
                        *division = joined;
                    }
                }
            }
            None
        }
        BtaStmtKind::Other => None,
    }
}

impl BtaEnv {
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

        let mut main_stmts = Vec::new();
        let mut functions = Vec::new();

        for stmt in &stmts {
            match stmt {
                Stmt::FunctionDef { name, params, body, is_native, .. } => {
                    if !is_native {
                        let param_info: Vec<(Symbol, bool)> = params.iter().map(|(sym, ty)| {
                            (*sym, is_collection_type(ty))
                        }).collect();
                        functions.push(BtaFunc {
                            name: *name,
                            params: param_info,
                            body: convert_block(body),
                        });
                    }
                }
                _ => {
                    main_stmts.push(convert_stmt(stmt));
                }
            }
        }

        Ok(BtaEnv {
            interner,
            main_stmts,
            functions,
        })
    }

    pub fn analyze_main(&mut self) -> BtaResult {
        let mut division = Division::new();
        let mut ctx = BtaContext {
            cache: BtaCache::new(),
            on_stack: HashSet::new(),
        };
        let return_bt = analyze_bta_block(&self.main_stmts, &mut division, &self.functions, &mut ctx);
        BtaResult { division, return_bt }
    }

    pub fn analyze_function(&mut self, func_name: &str, arg_bts: Vec<BindingTime>) -> BtaResult {
        let func_sym = self.interner.lookup(func_name)
            .unwrap_or_else(|| panic!("Function '{}' not found in interner", func_name));

        let func = self.functions.iter()
            .find(|f| f.name == func_sym)
            .unwrap_or_else(|| panic!("Function '{}' not found", func_name))
            .clone();

        let mut division = Division::new();
        for (i, (param_sym, is_collection)) in func.params.iter().enumerate() {
            if *is_collection {
                division.insert(*param_sym, BindingTime::Dynamic);
            } else if let Some(bt) = arg_bts.get(i) {
                division.insert(*param_sym, bt.clone());
            } else {
                division.insert(*param_sym, BindingTime::Dynamic);
            }
        }

        let mut ctx = BtaContext {
            cache: BtaCache::new(),
            on_stack: HashSet::new(),
        };
        let return_bt = analyze_bta_block(&func.body, &mut division, &self.functions, &mut ctx);
        BtaResult { division, return_bt }
    }

    pub fn lookup(&self, name: &str) -> Option<Symbol> {
        self.interner.lookup(name)
    }
}
