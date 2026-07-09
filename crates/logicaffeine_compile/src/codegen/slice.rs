//! Per-function codegen slicing (HOTSWAP Axis-3 / P14a).
//!
//! The AOT-native tier compiles ONE annotated function into a loadable artifact, so it
//! needs that function plus everything it transitively calls — and nothing else.
//! [`function_slice`] reduces a program's statements to exactly that closure: the
//! target `FunctionDef` and every function reachable from it through the call graph,
//! dropping `Main` and unrelated functions. Type definitions live in the
//! `TypeRegistry` (threaded to codegen separately), so only `FunctionDef`s are
//! filtered here. The reachable set comes from the shared [`CallGraph`].

use logicaffeine_base::{Interner, Symbol};
use logicaffeine_language::ast::Stmt;

use crate::analysis::callgraph::CallGraph;

/// The statements needed to compile `target` standalone: the `target` function plus
/// every function reachable from it through the call graph (transitively). `Main` and
/// top-level functions unreachable from `target` are dropped; definition order is
/// preserved.
pub fn function_slice<'a>(stmts: &[Stmt<'a>], target: Symbol, interner: &Interner) -> Vec<Stmt<'a>> {
    let cg = CallGraph::build(stmts, interner);
    let mut keep = cg.reachable_from(target);
    keep.insert(target);
    stmts
        .iter()
        .filter(|s| matches!(s, Stmt::FunctionDef { name, .. } if keep.contains(name)))
        .cloned()
        .collect()
}
