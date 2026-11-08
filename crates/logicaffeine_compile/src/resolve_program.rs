//! The semantic resolutions every program goes through before any tier runs it, in order: a
//! program that defines a real number by a function that cannot define one is refused
//! ([`crate::analysis::defined_real`]); then each exact context to its exact reading — `/` exact or floored, and `sqrt`, `exp`, `ln`, `pi`, `e` and
//! `omega` exact where an exact type is declared ([`crate::resolve_division`]) — and every other bare `pi`, `π` and
//! `e` to the `Real` constant ([`crate::resolve_division::resolve_real_constants`]), then each supertask
//! to the statements that compute its end state ([`crate::supertask`]), then each `Pop` behind the
//! check that its list has an item ([`crate::resolve_pop`]). Every seam calls this one function, so
//! no tier sees a program the others did not.

use logicaffeine_base::Interner;

use crate::ast::stmt::{Expr, Literal, Stmt};
use crate::ast_map::Arenas;

/// The resolved program; `None` when no resolution changed anything. `library` names the program's
/// functions that are the standard library's own definitions, read from the text the program was
/// parsed from ([`crate::loader::library_owned_functions`]). A refused program resolves to the one
/// statement that fails with the refusal, so every tier refuses it in the same words before it does
/// anything else.
pub(crate) fn resolve_program<'a>(
    stmts: &[Stmt<'a>],
    arenas: Arenas<'a>,
    interner: &mut Interner,
    fold_constants: bool,
    library: &std::collections::HashSet<String>,
) -> Option<&'a [Stmt<'a>]> {
    if let Err(refusal) = crate::analysis::defined_real::check(stmts, interner) {
        let reason = arenas.exprs.alloc(Expr::Literal(Literal::Text(interner.intern(&refusal))));
        return Some(arenas.stmts.alloc_slice(vec![Stmt::Fail { reason }]));
    }
    let divided = crate::resolve_division::resolve_divisions(
        stmts,
        arenas.stmts,
        arenas.exprs,
        interner,
        fold_constants,
        library,
    );
    let base = divided.unwrap_or(stmts);
    let real = crate::resolve_division::resolve_real_constants(base, arenas, interner, library).or(divided);
    let base = real.unwrap_or(stmts);
    let resolved = crate::supertask::resolve_supertasks(base, arenas, interner).or(real);
    let base = resolved.unwrap_or(stmts);
    crate::resolve_pop::resolve_pops(base, arenas, interner).or(resolved)
}
