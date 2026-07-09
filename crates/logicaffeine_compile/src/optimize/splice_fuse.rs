//! Fuse the parser's place-write desugar (`Stmt::Splice`) back into a direct
//! nested write when copy-on-write can never be observed.
//!
//! The parser lowers `Set item j of (item i of grid) to v` into a
//! read → element write → write-back sequence so each engine's binding-level
//! copy-on-write preserves value semantics (`desugar_place_set_index`). Under
//! REFERENCE semantics (`LOGOS_VALUE_SEMANTICS=0`) no engine ever copies, so
//! that sequence is observationally identical to the direct nested write it
//! desugared from — and the direct form is what the borrow-hoist and
//! bounds-elision codegen machinery pattern-match (`phase_borrow_hoist`).
//! This pass restores it, statement-for-statement.
//!
//! Value-semantics mode keeps the desugar (the write-back IS the semantics);
//! the alias-oracle fast path for that mode lands separately.

use crate::arena::Arena;
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::{Interner, Symbol};

pub fn fuse_place_splices<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Vec<Stmt<'a>> {
    // Under REFERENCE semantics both the Set- and Push-place desugars fuse to their direct nested
    // forms (no copy is ever observable). Under VALUE semantics the Set-shape ALSO fuses — to the
    // nested `SetIndex` the codegen lowers as a value-semantic THROUGH-WRITE (`grid.cow();
    // grid.set_nested(k, i, v)`), which cow's the row only if it is shared, avoiding the desugar's
    // unconditional full-row clone. The Push-shape keeps its desugar under value semantics (its
    // value-semantic fast path is not implemented here).
    stmts
        .into_iter()
        .map(|s| fuse_stmt(s, expr_arena, stmt_arena, interner))
        .collect()
}

fn fuse_block<'a>(
    block: &'a [Stmt<'a>],
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> &'a [Stmt<'a>] {
    let fused: Vec<Stmt<'a>> = block
        .iter()
        .cloned()
        .map(|s| fuse_stmt(s, expr_arena, stmt_arena, interner))
        .collect();
    stmt_arena.alloc_slice(fused)
}

fn fuse_stmt<'a>(
    stmt: Stmt<'a>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &Interner,
) -> Stmt<'a> {
    match stmt {
        Stmt::Splice { body } => {
            if let Some(direct) = try_fuse_place_splice(body, expr_arena, interner) {
                return direct;
            }
            // Not the place-write shape (e.g. a multi-push) — keep the
            // Splice, fusing anything nested inside it.
            Stmt::Splice { body: fuse_block(body, expr_arena, stmt_arena, interner) }
        }
        Stmt::If { cond, then_block, else_block } => Stmt::If {
            cond,
            then_block: fuse_block(then_block, expr_arena, stmt_arena, interner),
            else_block: else_block.map(|b| fuse_block(b, expr_arena, stmt_arena, interner)),
        },
        Stmt::While { cond, body, decreasing } => Stmt::While {
            cond,
            body: fuse_block(body, expr_arena, stmt_arena, interner),
            decreasing,
        },
        Stmt::Repeat { pattern, iterable, body } => Stmt::Repeat {
            pattern,
            iterable,
            body: fuse_block(body, expr_arena, stmt_arena, interner),
        },
        Stmt::FunctionDef { name, params, generics, body, return_type, is_native, native_path, is_exported, export_target, opt_flags } => Stmt::FunctionDef {
            name,
            params,
            generics,
            body: fuse_block(body, expr_arena, stmt_arena, interner),
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        },
        Stmt::Inspect { target, arms, has_otherwise } => Stmt::Inspect {
            target,
            arms: arms
                .into_iter()
                .map(|arm| crate::ast::stmt::MatchArm {
                    enum_name: arm.enum_name,
                    variant: arm.variant,
                    bindings: arm.bindings,
                    body: fuse_block(arm.body, expr_arena, stmt_arena, interner),
                })
                .collect(),
            has_otherwise,
        },
        Stmt::Zone { name, capacity, source_file, body } => Stmt::Zone {
            name,
            capacity,
            source_file,
            body: fuse_block(body, expr_arena, stmt_arena, interner),
        },
        Stmt::Concurrent { tasks } => Stmt::Concurrent {
            tasks: fuse_block(tasks, expr_arena, stmt_arena, interner),
        },
        Stmt::Parallel { tasks } => Stmt::Parallel {
            tasks: fuse_block(tasks, expr_arena, stmt_arena, interner),
        },
        other => other,
    }
}

/// True when `sym`'s name is one of the parser's place-desugar temporaries
/// with the given tag (`__place_i_17`, `__place_t_20`, …).
fn is_place_temp(interner: &Interner, sym: Symbol, tag: &str) -> bool {
    interner.resolve(sym).starts_with(tag)
}

/// Match the EXACT output shape of `desugar_place_set_index` /
/// `desugar_place_push` and rebuild the direct nested statement from the
/// original sub-expressions held by the temp `Let`s. The write-back chain
/// (statement 5 / 4) is dropped wholesale: under reference semantics the
/// direct write already lands in the outer collection's storage.
fn try_fuse_place_splice<'a>(
    body: &'a [Stmt<'a>],
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &Interner,
) -> Option<Stmt<'a>> {
    match body {
        // Set-shape: Let __place_i / __place_v / __place_k / __place_t, the
        // element write, then the write-back.
        [Stmt::Let { var: ti, value: index_expr, .. },
         Stmt::Let { var: tv, value: value_expr, .. },
         Stmt::Let { var: tk, value: key_expr, .. },
         Stmt::Let { var: tt, value: Expr::Index { collection: base, index: Expr::Identifier(read_k) }, .. },
         Stmt::SetIndex {
             collection: Expr::Identifier(w_t),
             index: Expr::Identifier(w_i),
             value: Expr::Identifier(w_v),
         },
         _write_back]
            if is_place_temp(interner, *ti, "__place_i")
                && is_place_temp(interner, *tv, "__place_v")
                && is_place_temp(interner, *tk, "__place_k")
                && is_place_temp(interner, *tt, "__place_t")
                && read_k == tk
                && w_t == tt
                && w_i == ti
                && w_v == tv =>
        {
            let place = expr_arena.alloc(Expr::Index { collection: *base, index: *key_expr });
            Some(Stmt::SetIndex { collection: place, index: *index_expr, value: *value_expr })
        }
        // Push-shape: Let __place_v / __place_k / __place_t, the push, then
        // the write-back. Only fused under REFERENCE semantics (its value-semantic
        // through-write is not implemented); under value semantics the desugar stays.
        [Stmt::Let { var: tv, value: value_expr, .. },
         Stmt::Let { var: tk, value: key_expr, .. },
         Stmt::Let { var: tt, value: Expr::Index { collection: base, index: Expr::Identifier(read_k) }, .. },
         Stmt::Push { value: Expr::Identifier(w_v), collection: Expr::Identifier(w_t) },
         _write_back]
            if !crate::semantics::collections::value_semantics_enabled()
                && is_place_temp(interner, *tv, "__place_v")
                && is_place_temp(interner, *tk, "__place_k")
                && is_place_temp(interner, *tt, "__place_t")
                && read_k == tk
                && w_v == tv
                && w_t == tt =>
        {
            let place = expr_arena.alloc(Expr::Index { collection: *base, index: *key_expr });
            Some(Stmt::Push { value: *value_expr, collection: place })
        }
        _ => None,
    }
}
