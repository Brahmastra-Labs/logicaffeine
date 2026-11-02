//! The supertask engine: a loop that runs infinitely many times, resolved to the statements that
//! compute its end state, before any tier runs the program.
//!
//! Two loops run infinitely often. A loop driven by a clock stepped by an infinitesimal (`While t <
//! 1: … Set t to t + dt.`) runs `⌈1/dt⌉` times — an infinite natural — and [`clock`] writes the
//! state it ends in. A search over every natural (`Repeat for every natural n:`) has no last pass,
//! and [`search`] writes its outcome. Both write ordinary statements over a few run-time operations
//! ([`runtime`]), so every tier receives the same resolved program and runs it the same way.
//!
//! `At the limit, take <rule>.` sets the rule the supertasks after it in its block — and in the
//! blocks inside those — apply to the residue-dependent values they leave.
//!
//! A program with no supertask comes through untouched. Only a program that declares an
//! infinitesimal can step a clock by one, so only such a program has its clock loops rewritten, and
//! a clock stepped by a constant is never rewritten; a search exists only where the program writes
//! one. Every other program is handed on exactly as it was parsed.

mod build;
mod clock;
mod hybrid;
mod linear;
pub(crate) mod runtime;
mod scope;
mod search;
mod stage;

use logicaffeine_base::{Interner, Symbol};

use crate::analysis::tower::{Scope, TowerTypes};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt};
use crate::ast_map::{walk_stmt, Arenas, AstMap};
use build::Build;

/// Resolve every supertask in `stmts`; `None` when there is none.
pub(crate) fn resolve_supertasks<'a>(
    stmts: &[Stmt<'a>],
    arenas: Arenas<'a>,
    interner: &mut Interner,
) -> Option<&'a [Stmt<'a>]> {
    let infinitesimal = interner.lookup("infinitesimal");
    let every_natural = interner.lookup("everyNatural");
    if infinitesimal.is_none() && every_natural.is_none() {
        return None;
    }
    let calls = crate::analysis::known_calls::harvest_calls(stmts).calls;
    let calls_to = |name: Option<Symbol>| name.is_some_and(|f| calls.iter().any(|c| c.function == f));
    let clocks = calls_to(infinitesimal);
    let searches = calls_to(every_natural);
    if !clocks && !searches {
        return None;
    }
    let tower = TowerTypes::of_program(stmts, interner);
    let every_natural = interner.intern("everyNatural");
    let at_the_limit = interner.intern("atTheLimit");
    let mut engine = Engine {
        arenas,
        interner,
        every_natural,
        at_the_limit,
        clocks,
        tower,
        scope: None,
        rule: None,
        serial: 0,
        changed: false,
    };
    let out = engine.block(stmts);
    engine.changed.then(|| arenas.stmts.alloc_slice(out))
}

struct Engine<'a, 'i> {
    arenas: Arenas<'a>,
    interner: &'i mut Interner,
    every_natural: Symbol,
    at_the_limit: Symbol,
    /// Whether the program declares an infinitesimal, the only way a clock can be stepped by one.
    clocks: bool,
    /// The values the exact extension reaches, which can put a loop's bound beyond every natural.
    tower: Option<TowerTypes>,
    /// The function whose body the pass is in; `None` in Main.
    scope: Scope,
    /// The limit rule in force where the pass is.
    rule: Option<String>,
    serial: usize,
    changed: bool,
}

impl<'a, 'i> Engine<'a, 'i> {
    fn builder(&mut self) -> Build<'a, '_> {
        self.serial += 1;
        Build {
            arenas: self.arenas,
            interner: self.interner,
            serial: self.serial,
        }
    }

    /// The limit rule an `At the limit, take <rule>.` statement names.
    fn rule_of(&self, s: &Stmt<'a>) -> Option<String> {
        match s {
            Stmt::Call { function, args } if *function == self.at_the_limit => match args.as_slice() {
                [Expr::Literal(Literal::Text(rule))] => Some(self.interner.resolve(*rule).to_string()),
                _ => None,
            },
            _ => None,
        }
    }

    fn block(&mut self, block: &[Stmt<'a>]) -> Vec<Stmt<'a>> {
        let outer_rule = self.rule.clone();
        let mut out = Vec::with_capacity(block.len());
        let mut after_search: Option<(Symbol, i64)> = None;
        for s in block {
            if let Some(rule) = self.rule_of(s) {
                self.rule = Some(rule);
            }
            let mut mapped = self.map_stmt(s);
            if let Some((proved, cases)) = after_search {
                let mut taint = Taint {
                    arenas: self.arenas,
                    interner: self.interner,
                    proved,
                    cases,
                };
                mapped = taint.map_stmt(&mapped);
            }
            match self.resolve(mapped) {
                Ok((resolved, search)) => {
                    self.changed = true;
                    out.push(resolved);
                    if search.is_some() {
                        after_search = search;
                    }
                }
                Err(unchanged) => out.push(unchanged),
            }
        }
        self.rule = outer_rule;
        out
    }

    /// The statement that replaces a supertask, with the settled flag and case count of a search;
    /// the statement itself back when it is not one.
    fn resolve(&mut self, s: Stmt<'a>) -> Result<(Stmt<'a>, Option<(Symbol, i64)>), Stmt<'a>> {
        match &s {
            Stmt::While {
                cond,
                body,
                decreasing: None,
            } if self.clocks => {
                let (cond, body) = (*cond, *body);
                let rule = self.rule.clone();
                let (tower, scope) = (self.tower.as_ref(), self.scope);
                let reached = |e: &Expr<'_>| reaches(tower, scope, e);
                self.serial += 1;
                let mut b = Build {
                    arenas: self.arenas,
                    interner: self.interner,
                    serial: self.serial,
                };
                let pass = scope::distinct(&[cond], body, &mut b);
                let (read, renamed) = match &pass {
                    Ok(Some((renamed, names))) => (*renamed, names.as_slice()),
                    _ => (body, &[][..]),
                };
                if let Some(c) = clock::recognize(cond, read, &reached, &mut b) {
                    let pass = pass.as_ref().map(|_| read).map_err(String::clone);
                    return Ok((clock::resolve(c, s.clone(), pass, rule.as_deref(), &mut b), None));
                }
                let Some(c) = clock::recognize(cond, body, &reached, &mut b) else {
                    return Err(s);
                };
                let t = b.interner.resolve(c.var).to_string();
                if !renamed.contains(&t) {
                    return Err(s);
                }
                let reason = format!(
                    "its pass binds its clock `{t}` again, so the `{t}` its guard reads is not the one the pass steps"
                );
                Ok((clock::resolve(c, s.clone(), Err(reason), rule.as_deref(), &mut b), None))
            }
            Stmt::Repeat {
                pattern: Pattern::Identifier(n),
                iterable,
                body,
            } => match search::cases_of(iterable, self.every_natural) {
                Some(cases) => {
                    let (n, body) = (*n, *body);
                    let mut b = self.builder();
                    let (resolved, proved) = search::resolve(n, cases, body, &mut b);
                    Ok((resolved, Some((proved, cases))))
                }
                None => Err(s),
            },
            _ => Err(s),
        }
    }
}

impl<'a, 'i> AstMap<'a> for Engine<'a, 'i> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        let Stmt::FunctionDef { name, .. } = s else {
            return walk_stmt(self, s);
        };
        let outer = self.scope.replace(*name);
        let mapped = walk_stmt(self, s);
        self.scope = outer;
        mapped
    }

    fn map_block(&mut self, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        let out = self.block(block);
        self.arenas.stmts.alloc_slice(out)
    }
}

/// Whether a condition compares a value the exact extension reaches; `and`, `or` and `not` are
/// read through to the comparisons under them.
fn reaches(tower: Option<&TowerTypes>, scope: Scope, e: &Expr<'_>) -> bool {
    match e {
        Expr::Not { operand } => reaches(tower, scope, operand),
        Expr::BinaryOp {
            op: BinaryOpKind::And | BinaryOpKind::Or,
            left,
            right,
        } => reaches(tower, scope, left) || reaches(tower, scope, right),
        _ => tower.is_some_and(|t| t.expr(scope, e)),
    }
}

/// The statements after a search: an answer returned once the search may have run out of cases is
/// provisional — `If proved: Return v. Otherwise: Return provisional(v, cases, 0).` A function
/// defined there is its own scope and is left alone.
struct Taint<'a, 'i> {
    arenas: Arenas<'a>,
    interner: &'i mut Interner,
    proved: Symbol,
    cases: i64,
}

impl<'a, 'i> AstMap<'a> for Taint<'a, 'i> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        match s {
            Stmt::Return { value: Some(v) } => {
                let mut b = Build {
                    arenas: self.arenas,
                    interner: self.interner,
                    serial: 0,
                };
                let provisional = b.call("provisional", vec![v, b.number(self.cases), b.number(0)]);
                let settled = Stmt::Return { value: Some(v) };
                let unsettled = Stmt::Return {
                    value: Some(provisional),
                };
                b.if_(b.ident(self.proved), vec![settled], Some(vec![unsettled]))
            }
            Stmt::FunctionDef { .. } => s.clone(),
            _ => walk_stmt(self, s),
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/supertask/tests.rs"]
mod tests;
