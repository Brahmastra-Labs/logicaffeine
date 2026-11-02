//! A local `Seq of Bool` held one bit per item (`Opt::BitPack`).
//!
//! A `Seq of Bool` is a `Vec<bool>`, a byte per item. A sequence of flags that only one function body uses, through
//! item reads, item stores, pushes and its length, can instead be a [`logicaffeine_data::LogosBits`]: a bit per item,
//! an eighth of the memory. A sieve's table of 10^8 flags is 12.5 MB where it was 100 MB, small enough for the cache
//! its passes used to stream through. Every item reads and stores as it did, counted from 1, failing with the same
//! sentence past the end.
//!
//! A sequence qualifies when it is declared `a new Seq of Bool` (or `List`), is uniquely owned (de-Rc), and every
//! mention of it in the body is one of those four uses: no rebinding, copy, return, argument, iteration, exchange,
//! pop or removal, which would hand the bits to code that expects a `Vec<bool>`. Sequences another representation
//! has claimed (a stack array, a worklist) keep it.

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use logicaffeine_base::intern::{Interner, Symbol};
use std::collections::{HashMap, HashSet};

use super::worklist::ok_read_only;

/// The `Let c be a new Seq of Bool` declarations in `body` whose every use is an item read, an item store, a push or
/// its length, among the uniquely owned sequences `de_rc`.
pub(crate) fn detect_bitpacked(
    body: &[Stmt],
    de_rc: &HashSet<Symbol>,
    interner: &Interner,
) -> HashSet<Symbol> {
    let mut declared: Vec<Symbol> = Vec::new();
    let mut once: HashSet<Symbol> = HashSet::new();
    collect_bool_seq_lets(body, interner, &mut declared);
    for &c in &declared {
        if declared.iter().filter(|&&d| d == c).count() == 1 {
            once.insert(c);
        }
    }
    once.into_iter()
        .filter(|c| de_rc.contains(c) && body.iter().all(|s| use_is_bitwise(s, *c, interner)))
        .collect()
}

/// Every name a `Let` in `stmts` (nested blocks included) binds to an empty new sequence of `Bool`.
fn collect_bool_seq_lets(stmts: &[Stmt], interner: &Interner, out: &mut Vec<Symbol>) {
    for s in stmts {
        match s {
            Stmt::Let { var, value, .. } if is_new_bool_seq(value, interner) => out.push(*var),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_bool_seq_lets(then_block, interner, out);
                if let Some(eb) = else_block {
                    collect_bool_seq_lets(eb, interner, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_bool_seq_lets(body, interner, out)
            }
            _ => {}
        }
    }
}

/// `a new Seq of Bool` (or `List`), with no initial fields, optionally with a capacity.
fn is_new_bool_seq(value: &Expr, interner: &Interner) -> bool {
    match value {
        Expr::WithCapacity { value, .. } => is_new_bool_seq(value, interner),
        Expr::New {
            type_name,
            type_args,
            init_fields,
        } => {
            init_fields.is_empty()
                && matches!(interner.resolve(*type_name), "Seq" | "List")
                && matches!(type_args.as_slice(), [TypeExpr::Primitive(t) | TypeExpr::Named(t)]
                    if interner.resolve(*t) == "Bool")
        }
        _ => false,
    }
}

/// Whether every mention of `c` in `s` (its nested blocks included) is an item read, an item store, a push, its
/// length, or its own declaration.
fn use_is_bitwise(s: &Stmt, c: Symbol, interner: &Interner) -> bool {
    let all = |b: &[Stmt]| b.iter().all(|x| use_is_bitwise(x, c, interner));
    match s {
        Stmt::Let { var, value, .. } => {
            if *var == c {
                is_new_bool_seq(value, interner)
            } else {
                ok_read_only(value, c)
            }
        }
        Stmt::Set { target, value } => *target != c && ok_read_only(value, c),
        Stmt::Push { collection, value } => {
            (names(collection, c) || ok_read_only(collection, c)) && ok_read_only(value, c)
        }
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            (names(collection, c) || ok_read_only(collection, c))
                && ok_read_only(index, c)
                && ok_read_only(value, c)
        }
        Stmt::SetField { object, value, .. } => ok_read_only(object, c) && ok_read_only(value, c),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => ok_read_only(cond, c) && all(then_block) && else_block.map_or(true, |eb| all(eb)),
        Stmt::While { cond, body, .. } => ok_read_only(cond, c) && all(body),
        Stmt::Repeat { iterable, body, .. } => {
            !crate::optimize::WriteSet::of_stmt_shallow(s)
                .declared()
                .contains(&c)
                && ok_read_only(iterable, c)
                && all(body)
        }
        Stmt::Return { value } => value.map_or(true, |v| ok_read_only(v, c)),
        Stmt::Show { object, recipient } => ok_read_only(object, c) && ok_read_only(recipient, c),
        Stmt::Call { args, .. } => args.iter().all(|a| ok_read_only(a, c)),
        Stmt::RuntimeAssert { condition, .. } => ok_read_only(condition, c),
        _ => !names_anywhere(s, c),
    }
}

fn names(e: &Expr, c: Symbol) -> bool {
    matches!(e, Expr::Identifier(s) if *s == c)
}

/// `c` is named anywhere inside `s`, its nested blocks included: in an expression, or as a name the statement binds,
/// assigns or mutates. Both walks are exhaustive over the AST.
fn names_anywhere(s: &Stmt, c: Symbol) -> bool {
    use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};
    let mut found = false;
    walk_nodes(std::slice::from_ref(s), |node| {
        if matches!(node, Visited::Expr(Expr::Identifier(sym)) if *sym == c) {
            found = true;
        }
        Walk::Into
    });
    found || crate::optimize::WriteSet::of_stmt(s).may_write(c)
}

/// `While j is at most bound` (or `less than`) over `Set item index of bits to value. Set j to j + step.`, where `bits`
/// is bit-packed, `index` is `j` plus something the loop does not change, `value` and `step` are things it does not
/// change, every store is proven to name an item and `step` is proven at least 1. Its stores are the progression from
/// `j`'s value on entry, `step` apart, up to the bound, which
/// [`logicaffeine_data::LogosBits::fill_stride_through_unchecked`] makes a word at a time.
pub(crate) struct StrideFill<'a> {
    pub bits: Symbol,
    pub counter: Symbol,
    pub bound: &'a Expr<'a>,
    /// `less than`: the last value stored at is below the bound, not at it.
    pub strict: bool,
    pub index: &'a Expr<'a>,
    pub value: &'a Expr<'a>,
    pub step: &'a Expr<'a>,
    /// `Set j to j + step`, emitted once after the fill, from the last value stored at.
    pub increment: &'a Stmt<'a>,
}

/// The [`StrideFill`] a `While` with this condition and body is, if it is one.
pub(crate) fn match_stride_fill<'a>(
    cond: &'a Expr<'a>,
    body: &'a [Stmt<'a>],
    types: &HashMap<Symbol, String>,
    oracle: &crate::optimize::OracleFacts,
    interner: &Interner,
) -> Option<StrideFill<'a>> {
    let Expr::BinaryOp {
        op: op @ (BinaryOpKind::LtEq | BinaryOpKind::Lt),
        left: Expr::Identifier(counter),
        right: bound,
    } = cond
    else {
        return None;
    };
    let [store, increment] = body else {
        return None;
    };
    let Stmt::SetIndex {
        collection: collection @ Expr::Identifier(bits),
        index,
        value,
    } = store
    else {
        return None;
    };
    let Stmt::Set {
        target,
        value: Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        },
    } = increment
    else {
        return None;
    };
    let step: &'a Expr<'a> = match (*left, *right) {
        (Expr::Identifier(s), step) | (step, Expr::Identifier(s)) if s == counter => step,
        _ => return None,
    };
    let int = |e: &Expr| super::types::infer_numeric_type(e, interner, types) == "i64";
    let held = |e: &Expr| {
        let mut read = Vec::new();
        super::peephole::collect_expr_symbols(e, &mut read);
        super::peephole::is_simple_expr(e) && !read.contains(counter)
    };
    let is_bits = types
        .get(bits)
        .is_some_and(|t| t.split("|__hl:").next() == Some("LogosBits"));
    let unit_offset = match index {
        Expr::Identifier(s) => s == counter,
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => match (*left, *right) {
            (Expr::Identifier(s), offset) | (offset, Expr::Identifier(s)) if s == counter => held(offset),
            _ => false,
        },
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left: Expr::Identifier(s),
            right: offset,
        } => s == counter && held(offset),
        _ => false,
    };
    let steps_forward = match step {
        Expr::Literal(Literal::Number(n)) => *n >= 1,
        _ => oracle.expr_proven_at_least(step, 1),
    };
    let held_value = match value {
        Expr::Literal(Literal::Boolean(_)) => true,
        Expr::Identifier(v) => v != counter && v != bits,
        _ => false,
    };
    let fills = is_bits
        && target == counter
        && types.get(counter).map(String::as_str) == Some("i64")
        && held(bound)
        && int(bound)
        && held(step)
        && int(step)
        && steps_forward
        && unit_offset
        && held_value
        && oracle.index_provably_in_bounds(collection, index)
        && oracle.index_positivity_guards(index).is_empty();
    fills.then_some(StrideFill {
        bits: *bits,
        counter: *counter,
        bound: *bound,
        strict: matches!(op, BinaryOpKind::Lt),
        index: *index,
        value: *value,
        step,
        increment,
    })
}
