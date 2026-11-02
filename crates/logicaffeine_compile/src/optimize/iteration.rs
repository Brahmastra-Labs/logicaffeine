//! How a `Repeat` may iterate, proven once for every backend.
//!
//! **A counted range.** `Repeat for v from a to b` evaluates its bounds once and gives `v` each Int from `a` up to `b`.
//! When both bounds are Ints wherever they are evaluated ([`super::kinds`]), the loop is a counter and a comparison:
//! nothing need build the range, and a backend lowers it as the counted loop it is ([`Fact::IntRange`]). A bound that
//! may be something else keeps the range value, which refuses a non-Int bound in the VM's words.
//!
//! **Lent iteration.** `Repeat for v in xs` iterates a snapshot of `xs` on every tier. When `xs` holds a sequence on
//! every binding ([`super::kinds`]) and nothing the loop runs can write through the name `xs` (its body's `Set`s,
//! in-place changes and lends to `mutable` parameters, a callee's write to a global of that name, foreign code), then
//! under value semantics every other handle to the sequence copies before it writes, so the sequence itself is the
//! snapshot for the whole loop. A backend may then walk `xs` by position: the VM without an iterator, which leaves the
//! plain counted loop its native tier compiles, and the Rust backend through a shared borrow instead of a copy.
//!
//! Both proofs run on exactly the statements a backend is handed, after the last rewrite, and mark each `Repeat` they
//! hold for ([`Fact::IntRange`], [`Fact::LentIteration`]); a backend asks the fact. Writes come from the optimizer's
//! exhaustive write walk ([`super::WriteSet`]) and are matched by NAME, since the parser may give one variable a
//! distinct symbol per occurrence. `## No borrow` turns lent iteration off, for the program or for one function; a
//! counted range is how a range loop runs, not an optimization, so no toggle reaches it.

use crate::analysis::types::TypeEnv;
use crate::ast::stmt::{Expr, Pattern, Stmt};
use crate::intern::{Interner, Symbol};
use crate::optimization::Opt;
use logicaffeine_language::ast_depth::{for_each_node, Visited};

use super::facts::{self, Fact};
use super::writes::{CalleeGlobals, WriteSet};

/// Whether `Repeat for … in xs` with this `body` may walk `xs` in place: value semantics hold, and nothing the body
/// runs, the globals its callees write included, can write through the name `xs`.
pub(crate) fn seq_iteration_lendable(
    body: &[Stmt],
    xs: Symbol,
    interner: &Interner,
    globals: &CalleeGlobals,
) -> bool {
    if !crate::semantics::collections::value_semantics_enabled() {
        return false;
    }
    let name = interner.resolve(xs);
    let writes = WriteSet::of_block(body).with_callee_globals(globals);
    !writes.is_opaque() && !writes.symbols().any(|w| interner.resolve(w) == name)
}

/// Mark every `Repeat` in `stmts` that counts through an Int range, or may walk its sequence in place. `stmts` must be
/// the statements a backend lowers: the marks are keyed by the nodes themselves.
pub fn prove_iteration_facts(stmts: &[Stmt], interner: &Interner) {
    // A call that passes a binding to a `mutable` parameter writes it; the write walk learns those positions from the
    // program installed, so this proof installs its own.
    crate::analysis::callgraph::install_lending_parameters(stmts);
    let program = super::active_config();
    let env = TypeEnv::new();
    let kinds = super::kinds::of_program(stmts, &env, interner);
    let globals = CalleeGlobals::of_program(stmts);
    for stmt in stmts {
        let (block, lend) = match stmt {
            Stmt::FunctionDef {
                body, opt_flags, ..
            } => (*body, program.merged(opt_flags).is_on(Opt::Borrow)),
            _ => (std::slice::from_ref(stmt), program.is_on(Opt::Borrow)),
        };
        for_each_node(block, |node| {
            let Visited::Stmt(repeat) = node else { return };
            let Stmt::Repeat {
                pattern: Pattern::Identifier(_),
                iterable,
                body,
            } = repeat
            else {
                return;
            };
            match iterable {
                Expr::Range { start, end } => {
                    if kinds.int_valued(start, &env) && kinds.int_valued(end, &env) {
                        facts::mark_stmt(Fact::IntRange, repeat);
                    }
                }
                Expr::Identifier(xs) => {
                    if lend
                        && kinds.seq(*xs)
                        && seq_iteration_lendable(body, *xs, interner, &globals)
                    {
                        facts::mark_stmt(Fact::LentIteration, repeat);
                    }
                }
                _ => {}
            }
        });
    }
}
