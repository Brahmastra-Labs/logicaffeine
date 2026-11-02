use std::collections::{HashMap, HashSet};

use logicaffeine_base::Symbol;
use logicaffeine_language::ast::stmt::Pattern;
use logicaffeine_language::ast::{Expr, Stmt};

use crate::optimize::Mentions;

/// Liveness analysis result for a whole program.
///
/// For every statement of every user-defined function and of the top-level (`## Main`) statement list — nested
/// statements included — the set of variables live immediately *after* it. Each function also keeps the table of its
/// top-level statements by position (`live_after[i]`).
///
/// Used by codegen to decide when a value can be *moved* (into a call, into a constructed record) instead of cloned:
/// if a variable is not live after the statement, the old value is never read again and ownership can be
/// transferred. A move on a variable wrongly thought dead breaks the program, so the analysis errs toward "live": a
/// statement kind it has no case for uses every name it touches (the optimizer's one exhaustive walk,
/// `crate::optimize::Mentions`), foreign code uses every name, and a loop body's end sees what the next iteration
/// needs.
pub struct LivenessResult {
    functions: HashMap<Symbol, FunctionLiveness>,
    /// Every analysed statement's live-after set, keyed by the statement's address in the arena it was analysed in.
    by_stmt: HashMap<usize, HashSet<Symbol>>,
}

/// Per-function liveness table.
pub struct FunctionLiveness {
    /// `live_after[i]` = variables live after top-level statement `i`.
    pub live_after: Vec<HashSet<Symbol>>,
}

impl LivenessResult {
    /// Compute liveness for every `FunctionDef` in `stmts`, and for `stmts` themselves as the `## Main` body.
    ///
    /// Algorithm: backward dataflow over each body. `Return` is treated as a terminator — variables used in
    /// subsequent (dead-code) statements do not affect liveness before the `Return`.
    pub fn analyze(stmts: &[Stmt<'_>]) -> Self {
        let mut functions = HashMap::new();
        let mut by_stmt = HashMap::new();
        let none_pinned = HashSet::new();
        for stmt in stmts {
            if let Stmt::FunctionDef { name, body, .. } = stmt {
                functions.insert(*name, analyze_body(body, &none_pinned, &mut by_stmt));
            }
        }
        // A function reaches Main's variables by name (they are its globals), so a call can read one no Main
        // statement names: every name a function body speaks of stays live throughout Main.
        let reached: HashSet<Symbol> = stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { body, .. } => {
                    Some(Mentions::of_block(body).symbols().collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();
        analyze_body(stmts, &reached, &mut by_stmt);
        Self { functions, by_stmt }
    }

    /// The variables live after `stmt`, when the analysis saw it (a statement the compiler synthesized later has
    /// no entry).
    pub fn after(&self, stmt: &Stmt<'_>) -> Option<&HashSet<Symbol>> {
        self.by_stmt.get(&stmt_key(stmt))
    }

    /// Every analysed statement's live-after set, by [`stmt_key`].
    pub fn into_statement_table(self) -> HashMap<usize, HashSet<Symbol>> {
        self.by_stmt
    }

    /// Returns `true` if `var` is live immediately after top-level statement
    /// `stmt_idx` in function `fn_sym`.
    ///
    /// Returns `false` for unknown functions, out-of-bounds indices, or when
    /// the variable is definitely dead.
    pub fn is_live_after(&self, fn_sym: Symbol, stmt_idx: usize, var: Symbol) -> bool {
        self.functions
            .get(&fn_sym)
            .and_then(|fl| fl.live_after.get(stmt_idx))
            .map(|s| s.contains(&var))
            .unwrap_or(false)
    }

    /// Returns the live-after set for statement `stmt_idx` in `fn_sym`.
    ///
    /// Returns a reference to an empty set when the function or index is
    /// unknown.
    pub fn live_after(&self, fn_sym: Symbol, stmt_idx: usize) -> &HashSet<Symbol> {
        static EMPTY: std::sync::OnceLock<HashSet<Symbol>> = std::sync::OnceLock::new();
        self.functions
            .get(&fn_sym)
            .and_then(|fl| fl.live_after.get(stmt_idx))
            .unwrap_or_else(|| EMPTY.get_or_init(HashSet::new))
    }
}

/// The key a statement's liveness is recorded under: its address in the arena the analysis walked.
pub fn stmt_key(stmt: &Stmt<'_>) -> usize {
    stmt as *const Stmt<'_> as *const () as usize
}

// =============================================================================
// Per-body analysis
// =============================================================================

/// The backward walk over one body: what foreign code may read, the names live throughout it, and where each
/// statement's live-after set is written.
struct Walk<'r> {
    /// Every name the body speaks of — what foreign code may read.
    every: HashSet<Symbol>,
    /// Names live at every point of the body (Main's variables that a function reads by name).
    pinned: &'r HashSet<Symbol>,
    /// What is live where a `Break` goes: after the nearest loop or zone.
    break_exit: Option<HashSet<Symbol>>,
    /// What is live where a `Return` goes when a zone encloses it: after the nearest zone, which the `Return` leaves.
    return_exit: Option<HashSet<Symbol>>,
    by_stmt: &'r mut HashMap<usize, HashSet<Symbol>>,
}

fn analyze_body(
    body: &[Stmt<'_>],
    pinned: &HashSet<Symbol>,
    by_stmt: &mut HashMap<usize, HashSet<Symbol>>,
) -> FunctionLiveness {
    let mut walk = Walk {
        every: Mentions::of_block(body).symbols().collect(),
        pinned,
        break_exit: None,
        return_exit: None,
        by_stmt,
    };
    let n = body.len();
    let mut live_after = vec![HashSet::<Symbol>::new(); n];
    let mut current: HashSet<Symbol> = pinned.clone();

    for i in (0..n).rev() {
        if is_terminator(&body[i]) {
            // Return (or similar terminator): nothing is live after it,
            // and dead code that follows does not influence pre-Return liveness.
            live_after[i] = pinned.clone();
            walk.record(&body[i], &live_after[i]);
            current = walk.gen_stmt(&body[i]);
        } else {
            live_after[i] = current.clone();
            current = walk.live_before_stmt(&body[i], &current);
        }
    }

    FunctionLiveness { live_after }
}

fn is_terminator(stmt: &Stmt<'_>) -> bool {
    matches!(stmt, Stmt::Return { .. })
}

impl Walk<'_> {
    /// Record `live_out` as what is live after `stmt`. A loop body is walked once per fixpoint round; the last
    /// round, at the fixpoint, writes last.
    fn record(&mut self, stmt: &Stmt<'_>, live_out: &HashSet<Symbol>) {
        let mut live = live_out.clone();
        live.extend(self.pinned.iter().copied());
        self.by_stmt.insert(stmt_key(stmt), live);
    }

    /// Variables generated (used) by a terminator.
    fn gen_stmt(&self, stmt: &Stmt<'_>) -> HashSet<Symbol> {
        let mut out = self.pinned.clone();
        if let Stmt::Return { value: Some(v) } = stmt {
            self.gen_expr(v, &mut out);
        }
        out
    }

    /// Compute the live-before set for a single statement given what is live after it, recording what is live
    /// after it and after every statement nested in it.
    fn live_before_stmt(&mut self, stmt: &Stmt<'_>, live_out: &HashSet<Symbol>) -> HashSet<Symbol> {
        // A `Break` goes to the nearest loop's or zone's exit, whatever follows it; one with no exit known keeps every
        // name live.
        if matches!(stmt, Stmt::Break) {
            let exit = self.break_exit.clone().unwrap_or_else(|| self.every.clone());
            self.record(stmt, &exit);
            return exit;
        }
        // A `Return` inside a zone leaves the zone and goes on after it.
        if let (Stmt::Return { value }, Some(exit)) = (stmt, self.return_exit.clone()) {
            self.record(stmt, &exit);
            let mut before = exit;
            if let Some(value) = value {
                self.gen_expr(value, &mut before);
            }
            return before;
        }
        self.record(stmt, live_out);
        match stmt {
            Stmt::Return { .. } => self.gen_stmt(stmt),

            Stmt::Let { var, value, .. } => {
                let mut result = live_out.clone();
                result.remove(var);
                self.gen_expr(value, &mut result);
                result
            }

            Stmt::Set { target, value } => {
                let mut result = live_out.clone();
                result.remove(target);
                self.gen_expr(value, &mut result);
                result
            }

            Stmt::Call { function, args } => {
                let mut result = live_out.clone();
                // A callee named by a binding (a closure held in a variable) is a use of it.
                result.insert(*function);
                for a in args.iter() {
                    self.gen_expr(a, &mut result);
                }
                result
            }

            Stmt::Push { value, collection } => {
                let mut result = live_out.clone();
                self.gen_expr(value, &mut result);
                self.gen_expr(collection, &mut result);
                result
            }

            Stmt::Pop { collection, into } => {
                let mut result = live_out.clone();
                if let Some(v) = into {
                    result.remove(v);
                }
                self.gen_expr(collection, &mut result);
                result
            }

            Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
                let mut result = live_out.clone();
                self.gen_expr(value, &mut result);
                self.gen_expr(collection, &mut result);
                result
            }

            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                let mut result = live_out.clone();
                self.gen_expr(collection, &mut result);
                self.gen_expr(index, &mut result);
                self.gen_expr(value, &mut result);
                result
            }

            Stmt::SetField { object, value, .. } => {
                let mut result = live_out.clone();
                self.gen_expr(object, &mut result);
                self.gen_expr(value, &mut result);
                result
            }

            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let then_lb = self.live_before_block(then_block, live_out);
                let else_lb = match else_block {
                    Some(b) => self.live_before_block(b, live_out),
                    None => live_out.clone(),
                };
                let mut result = HashSet::new();
                self.gen_expr(cond, &mut result);
                result.extend(then_lb);
                result.extend(else_lb);
                result
            }

            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                // Fixed-point: loop_live = live_out ∪ gen(cond, decreasing) ∪ body_live_before(loop_live)
                let mut loop_live: HashSet<Symbol> = live_out.clone();
                self.gen_expr(cond, &mut loop_live);
                if let Some(d) = decreasing {
                    self.gen_expr(d, &mut loop_live);
                }
                let enclosing = self.break_exit.replace(live_out.clone());
                loop {
                    let body_before = self.live_before_block(body, &loop_live);
                    let mut new_live = live_out.clone();
                    self.gen_expr(cond, &mut new_live);
                    if let Some(d) = decreasing {
                        self.gen_expr(d, &mut new_live);
                    }
                    new_live.extend(body_before);
                    if new_live == loop_live {
                        break;
                    }
                    loop_live = new_live;
                }
                self.break_exit = enclosing;
                loop_live
            }

            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                // Fixed-point: what the top of an iteration needs is live at the end of the one before it; the
                // pattern's names are bound afresh at each top.
                let pattern_syms: HashSet<Symbol> = match pattern {
                    Pattern::Identifier(s) => [*s].into_iter().collect(),
                    Pattern::Tuple(syms) => syms.iter().copied().collect(),
                };
                let mut loop_live: HashSet<Symbol> = live_out.clone();
                let enclosing = self.break_exit.replace(live_out.clone());
                loop {
                    let body_before = self.live_before_block(body, &loop_live);
                    let mut new_live = live_out.clone();
                    new_live.extend(
                        body_before
                            .into_iter()
                            .filter(|s| !pattern_syms.contains(s)),
                    );
                    if new_live == loop_live {
                        break;
                    }
                    loop_live = new_live;
                }
                self.break_exit = enclosing;
                let mut result = loop_live;
                self.gen_expr(iterable, &mut result);
                result
            }

            Stmt::Inspect { target, arms, .. } => {
                let mut result = HashSet::new();
                for arm in arms.iter() {
                    let arm_lb = self.live_before_block(arm.body, live_out);
                    result.extend(arm_lb);
                }
                self.gen_expr(target, &mut result);
                result
            }

            Stmt::Zone { body, .. } => {
                let enclosing_break = self.break_exit.replace(live_out.clone());
                let enclosing_return = self.return_exit.replace(live_out.clone());
                let before = self.live_before_block(body, live_out);
                self.break_exit = enclosing_break;
                self.return_exit = enclosing_return;
                before
            }

            // Any other statement uses every name it touches (a `Swap` reads both names; a `Show`, a `Give`, a
            // splice, a select arm, a concurrent task reads what it names), and foreign code uses every name.
            // Nothing nested in it is recorded, so nothing inside it moves.
            _ => {
                let touched = Mentions::of_stmt(stmt);
                let mut result = live_out.clone();
                result.extend(touched.symbols());
                if touched.is_opaque() {
                    result.extend(self.every.iter().copied());
                }
                result
            }
        }
    }

    /// Compute live-before for a block of statements.
    fn live_before_block(
        &mut self,
        stmts: &[Stmt<'_>],
        live_out: &HashSet<Symbol>,
    ) -> HashSet<Symbol> {
        let mut current = live_out.clone();
        for stmt in stmts.iter().rev() {
            if is_terminator(stmt) && self.return_exit.is_none() {
                let after = self.pinned.clone();
                self.record(stmt, &after);
                current = self.gen_stmt(stmt);
            } else {
                current = self.live_before_stmt(stmt, &current);
            }
        }
        current
    }

    /// Every variable an expression uses: each identifier it reads, what a closure in it captures, and — for foreign
    /// code — every name the body speaks of. From the optimizer's one exhaustive walk, so no expression form is
    /// overlooked.
    fn gen_expr(&self, expr: &Expr<'_>, out: &mut HashSet<Symbol>) {
        let touched = Mentions::of_expr(expr);
        out.extend(touched.symbols());
        if touched.is_opaque() {
            out.extend(self.every.iter().copied());
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/liveness/tests.rs"]
mod tests;
