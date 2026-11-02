//! A pass that binds a name it has already used. `Let x be …` in a loop's body makes `x` a new name
//! for the rest of the pass, while the guard and the statements before it — the binding's own
//! value among them — read and change the program's `x`. The strategies read a pass by its names, so
//! before one reads it, each such binding and every later mention of its name are renamed apart to a
//! fresh name: the same program, one binding to a name.
//!
//! The renaming reaches the names an expression reads and the names `Let` and `Set` bind and change.
//! Every renamed statement is checked against the walk of every name it touches ([`Mentions`]): a
//! form that binds or reads the name where the renaming does not reach — a closure's parameter, a
//! pattern — would leave the two bindings mixed, so the pass is refused with the reason instead.

use std::collections::HashSet;

use logicaffeine_base::{Interner, Symbol};

use super::build::Build;
use crate::ast::stmt::{Expr, Stmt};
use crate::ast_map::{walk_expr, walk_stmt, Arenas, AstMap};
use crate::optimize::mentions::Mentions;

/// The pass `body` with every binding of a name the guard (`heads`) or an earlier statement of the
/// pass already used renamed apart, and the names so renamed; `Ok(None)` when no binding needs it,
/// and the reason when one cannot be kept apart.
pub(super) fn distinct<'a>(
    heads: &[&'a Expr<'a>],
    body: &'a [Stmt<'a>],
    b: &mut Build<'a, '_>,
) -> Result<Option<(&'a [Stmt<'a>], Vec<String>)>, String> {
    let names = |m: Mentions, interner: &Interner| -> HashSet<String> {
        m.symbols().map(|s| interner.resolve(s).to_string()).collect()
    };
    let mut used: HashSet<String> = HashSet::new();
    for head in heads {
        used.extend(names(Mentions::of_expr(head), b.interner));
    }
    let mut pass: Vec<Stmt<'a>> = body.to_vec();
    let mut renamed: Vec<String> = Vec::new();
    for at in 0..pass.len() {
        if let Stmt::Let { var, value, .. } = pass[at] {
            let name = b.interner.resolve(var).to_string();
            if used.contains(&name) || names(Mentions::of_expr(value), b.interner).contains(&name) {
                let fresh = b.fresh(&format!("pass_{name}"));
                let mut rename = Rename {
                    arenas: b.arenas,
                    interner: b.interner,
                    from: &name,
                    to: fresh,
                };
                if let Stmt::Let { var, .. } = &mut pass[at] {
                    *var = fresh;
                }
                for later in &mut pass[at + 1..] {
                    *later = rename.map_stmt(later);
                    if names(Mentions::of_stmt(later), rename.interner).contains(&name) {
                        return Err(format!(
                            "its pass binds `{name}` again where the engine cannot keep the two apart"
                        ));
                    }
                }
                renamed.push(name);
            }
        }
        used.extend(names(Mentions::of_stmt(&pass[at]), b.interner));
    }
    Ok((!renamed.is_empty()).then(|| (&*b.arenas.stmts.alloc_slice(pass), renamed)))
}

/// Renames the name `from` to `to` wherever an expression reads it and wherever `Let` binds it or
/// `Set` changes it.
struct Rename<'a, 'n> {
    arenas: Arenas<'a>,
    interner: &'n Interner,
    from: &'n str,
    to: Symbol,
}

impl Rename<'_, '_> {
    fn renamed(&self, s: Symbol) -> Symbol {
        if self.interner.resolve(s) == self.from {
            self.to
        } else {
            s
        }
    }
}

impl<'a> AstMap<'a> for Rename<'a, '_> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        match e {
            Expr::Identifier(s) if self.renamed(*s) != *s => self.arenas.exprs.alloc(Expr::Identifier(self.to)),
            _ => walk_expr(self, e),
        }
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        let mut out = walk_stmt(self, s);
        match &mut out {
            Stmt::Let { var, .. } => *var = self.renamed(*var),
            Stmt::Set { target, .. } => *target = self.renamed(*target),
            _ => {}
        }
        out
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/supertask/scope/tests.rs"]
mod tests;
