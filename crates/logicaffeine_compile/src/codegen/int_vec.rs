//! Which `Seq of Int` lists the generated Rust holds as an [`IntVec`](logicaffeine_data::IntVec): machine words until a
//! value past the word is stored, then, once, exact integers.
//!
//! The decision is the C tier's (`codegen_c::exact`), taken over the same flow classes
//! ([`crate::analysis::seq_flow`]): a class is promotable when some store into it can outgrow a word and no member
//! reaches a reader or a source that knows only machine words. Whether a store can outgrow a word is the shared
//! exactness decision's ([`crate::analysis::exactness`]), told what this tier knows: its promoted returns, fields and
//! locals ([`super::bigint_promote`]) and its range oracle. A store the oracle proves stays in a word never promotes
//! anything, so a list a benchmark fills within the word keeps every fast form this tier gives a `Vec<i64>`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

use super::bigint_promote::{const_overflows_i64, mentions_promotable, promotable_int_vars, PromotionFacts};
use crate::analysis::seq_flow::{address, record_seq_flow, SeqFlow, SeqSlot};
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::{Interner, Symbol};

/// The Rust type of a list held as an `IntVec`.
pub(crate) const INT_VEC: &str = "logicaffeine_data::IntVec";

/// A new, empty `IntVec` with room for `capacity` (a `usize`) words.
pub(crate) fn with_room(capacity: &str) -> String {
    format!("{INT_VEC}::with_capacity({capacity})")
}

/// An `IntVec` of `count` (a `usize`) copies of the word `fill`.
pub(crate) fn filled(fill: &str, count: &str) -> String {
    format!("{INT_VEC}::from_words(vec![{fill}; {count}])")
}

/// The lists of one program the generated Rust holds as an `IntVec`.
#[derive(Default)]
pub(crate) struct Promoting {
    /// Each body's promotable `Seq of Int` locals (its parameters among them); `None` is Main.
    locals: HashSet<(Option<Symbol>, Symbol)>,
    /// Functions whose `Seq of Int` return is an `IntVec`.
    returns: HashSet<Symbol>,
    /// The list literals built as an `IntVec` holding an exact integer, by node address.
    literals: HashSet<usize>,
    /// Each user function's parameters, in order.
    params: HashMap<Symbol, Vec<Symbol>>,
}

thread_local! {
    /// The decision of each compile in progress, innermost last.
    static PROMOTING: RefCell<Vec<Rc<Promoting>>> = const { RefCell::new(Vec::new()) };
    /// The body being emitted: `Some(f)` inside function `f`, `None` in Main.
    static OWNER: std::cell::Cell<Option<Symbol>> = const { std::cell::Cell::new(None) };
    /// Whether the expression being emitted is read by an exact sink, which reads an item of an `IntVec` as the exact
    /// integer it is. Everywhere else an item is the machine word it is held as.
    static EXACT_VIEW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// The `IntVec` binding a store into it holds the words of ([`StoreForm::OnWords`]): the expression being emitted
    /// reads its items and length through that view, [`WORDS`].
    static WORDS_VIEW: std::cell::Cell<Option<Symbol>> = const { std::cell::Cell::new(None) };
    /// The views of their lists' words the `While` loops being emitted on words hold ([`versioned_while`]), innermost
    /// last: each loop's label, with its lists and the name each list's words are read through.
    static LOOP_VIEWS: RefCell<Vec<(String, Vec<(Symbol, String)>)>> = const { RefCell::new(Vec::new()) };
    /// The versioned loops of this compile so far, which number their labels.
    static LOOP_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How deep the emission is in a continuation, a pass finished on the exact integers ([`versioned_while`]): cold
    /// code, whose loops are not versioned again.
    static CONTINUING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Whether the expression being emitted is the word form of an expression guarded on its lists' forms
    /// ([`arithmetic`], [`comparison`]): every list it reads holds words there, so it reads them as words unguarded.
    static GUARDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `f` emitted as the word form of an expression guarded on its lists' forms.
fn guarded<R>(f: impl FnOnce() -> R) -> R {
    let outer = GUARDED.with(|g| g.replace(true));
    let answer = f();
    GUARDED.with(|g| g.set(outer));
    answer
}

/// `e`, a sum, difference, product, quotient or remainder reading an item of an `IntVec`, where its result is a word:
/// its words while every list it reads holds words; computed on the exact integers once one has promoted, and
/// narrowed at the result, loudly only where the result itself is past the word. An item past the word is never
/// narrowed on the way (`(s + item i of d) % 1000`). `None` for any other expression, for one an exact sink or an
/// index reads (each reads its own way), and inside a guarded expression's word form.
pub(crate) fn arithmetic(e: &Expr, ecx: &super::expr::ExprCtx) -> Option<String> {
    use crate::ast::stmt::BinaryOpKind;
    let Expr::BinaryOp { op, .. } = e else { return None };
    let arithmetic = matches!(
        op,
        BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply | BinaryOpKind::Divide | BinaryOpKind::Modulo
    );
    if !arithmetic || ecx.int_index_context || ecx.int_exact_tolerant || exact_view() || GUARDED.with(|g| g.get()) {
        return None;
    }
    let types = ecx.variable_types;
    if super::types::infer_numeric_type(e, ecx.interner, types) != "i64" || super::expr::mentions_bigint_var(e, types) {
        return None;
    }
    let promoted: Vec<String> = item_lists(e, types)?
        .into_iter()
        .filter(|l| loop_view(*l).is_none())
        .map(|l| format!("{}.is_exact()", place(l, ecx)))
        .collect();
    if promoted.is_empty() {
        return None;
    }
    let words = guarded(|| super::expr::codegen_expr_ctx(e, ecx));
    let exactly = method_receiver(&exact_value(e, ecx));
    Some(format!("(if {} {{ {exactly}.expect_i64(\"Int\") }} else {{ ({words}) }})", promoted.join(" || ")))
}

/// The view of its words a store into an `IntVec` holds, a `&mut Vec<i64>`.
const WORDS: &str = "__w";

/// `f` emitted with `list`'s items and length read through the view of its words a store holds.
fn through_words<R>(list: Symbol, f: impl FnOnce() -> R) -> R {
    let outer = WORDS_VIEW.with(|v| v.replace(Some(list)));
    let answer = f();
    WORDS_VIEW.with(|v| v.set(outer));
    answer
}

/// The name a versioned loop being emitted on words reads `list`'s words through.
pub(crate) fn loop_view(list: Symbol) -> Option<String> {
    LOOP_VIEWS.with(|v| {
        v.borrow().iter().rev().find_map(|(_, lists)| lists.iter().find(|(l, _)| *l == list).map(|(_, n)| n.clone()))
    })
}

/// The name `collection`'s words are read through where a view of them is held: the store's own ([`WORDS`]) or an
/// enclosing loop's.
fn viewed(collection: &Expr) -> Option<String> {
    let Expr::Identifier(list) = collection else { return None };
    if WORDS_VIEW.with(|v| v.get()) == Some(*list) {
        return Some(WORDS.to_string());
    }
    loop_view(*list)
}

/// Whether an item of an `IntVec` is read as the exact integer it is (an exact sink's view) rather than as a word.
pub(crate) fn exact_view() -> bool {
    EXACT_VIEW.with(|v| v.get())
}

/// `f`, an emission in which an item of an `IntVec` is read as the exact integer it is.
pub(crate) fn with_exact_view<R>(f: impl FnOnce() -> R) -> R {
    let outer = EXACT_VIEW.with(|v| v.replace(true));
    let answer = f();
    EXACT_VIEW.with(|v| v.set(outer));
    answer
}

/// The `IntVec` bindings whose items `e` reads: `Some` with each binding (none when it reads no item of one), `None`
/// when it reads an item of an `IntVec` some other expression gives (a call's result, a literal).
pub(crate) fn item_lists(e: &Expr, variable_types: &HashMap<Symbol, String>) -> Option<Vec<Symbol>> {
    let mut lists = Vec::new();
    let mut work = vec![e];
    while let Some(e) = work.pop() {
        if let Expr::Index { collection, .. } = e {
            if is_int_vec(collection, variable_types) {
                match collection {
                    Expr::Identifier(s) => {
                        if !lists.contains(s) {
                            lists.push(*s);
                        }
                    }
                    _ => return None,
                }
            }
        }
        if !matches!(e, Expr::Closure { .. }) {
            work.extend(logicaffeine_language::ast_depth::expr_children(e));
        }
    }
    Some(lists)
}

/// Holds the program's decision installed until the compile ends.
pub(crate) struct Scope {
    depth: usize,
}

impl Drop for Scope {
    fn drop(&mut self) {
        PROMOTING.with(|p| p.borrow_mut().truncate(self.depth - 1));
    }
}

/// The body being emitted, restored when the guard drops.
pub(crate) struct Body {
    outer: Option<Symbol>,
}

impl Drop for Body {
    fn drop(&mut self) {
        OWNER.with(|o| o.set(self.outer));
    }
}

/// Enter the body of `owner` (`None` for Main) for its emission.
pub(crate) fn enter_body(owner: Option<Symbol>) -> Body {
    Body { outer: OWNER.with(|o| o.replace(owner)) }
}

fn with<R>(f: impl FnOnce(&Promoting) -> R) -> Option<R> {
    PROMOTING.with(|p| p.borrow().last().map(|d| f(d)))
}

/// Whether the local `name` of the body `owner` names (`None` for Main) is an `IntVec`.
pub(crate) fn promotes(owner: Option<Symbol>, name: Symbol) -> bool {
    with(|d| d.locals.contains(&(owner, name))).unwrap_or(false)
}

/// Whether the local `name` of the body being emitted is an `IntVec`.
pub(crate) fn promotes_here(name: Symbol) -> bool {
    promotes(OWNER.with(|o| o.get()), name)
}

/// The locals of the body `owner` names (`None` is Main) that the decision holds as `IntVec`s.
pub(crate) fn locals_of(owner: Option<Symbol>) -> Vec<Symbol> {
    with(|d| d.locals.iter().filter(|(o, _)| *o == owner).map(|(_, name)| *name).collect()).unwrap_or_default()
}

/// Whether a call to `function` returns an `IntVec`.
pub(crate) fn promotes_return(function: Symbol) -> bool {
    with(|d| d.returns.contains(&function)).unwrap_or(false)
}

/// Whether any of `stmts` reads or writes a list the body being emitted holds as an `IntVec`: the statement-window
/// rewrites are written for machine-word lists, so such statements are emitted one by one.
pub(crate) fn touches(stmts: &[&Stmt]) -> bool {
    let owner = OWNER.with(|o| o.get());
    with(|d| {
        d.locals
            .iter()
            .filter(|(o, _)| *o == owner)
            .any(|(_, list)| stmts.iter().any(|s| crate::optimize::stmt_touches(s, *list)))
    })
    .unwrap_or(false)
}

/// Whether parameter `i` of `function` is an `IntVec`.
pub(crate) fn promotes_param(function: Symbol, i: usize) -> bool {
    with(|d| d.params.get(&function).and_then(|ps| ps.get(i)).is_some_and(|p| d.locals.contains(&(Some(function), *p))))
        .unwrap_or(false)
}

/// Whether the list literal `literal` is built as an `IntVec` holding an exact integer.
pub(crate) fn literal_promoted(literal: &Expr) -> bool {
    with(|d| d.literals.contains(&address(literal))).unwrap_or(false)
}

/// Whether a binding of Rust type `ty` holds a list of machine words: the forms a list of `Int` takes when it is not
/// an `IntVec`, owned or lent, whole, scalarized or fused.
pub(crate) fn is_word_list(ty: &str) -> bool {
    let ty = ty.split("|__hl:").next().unwrap_or(ty);
    let held = ty.strip_prefix("&mut ").or_else(|| ty.strip_prefix('&')).unwrap_or(ty);
    matches!(held, "LogosSeq<i64>" | "LogosSeq<i32>" | "Vec<i64>" | "Vec<i32>" | "[i64]" | "[i32]")
        || held.starts_with("[i64; ")
        || held.starts_with("[i32; ")
        || held.starts_with("__aos:")
        || held.starts_with("__affine_array:")
        || held == "logicaffeine_data::LogosBits"
}

/// Whether a binding of Rust type `ty` holds an `IntVec`, its own or lent.
pub(crate) fn holds_int_vec(ty: &str) -> bool {
    let ty = ty.split("|__hl:").next().unwrap_or(ty);
    let held = ty.strip_prefix("&mut ").or_else(|| ty.strip_prefix('&')).unwrap_or(ty);
    held == INT_VEC
}

/// The Rust type a parameter held as an `IntVec` is taken as: lent to be written in place, lent to be read, or the
/// callee's own.
pub(crate) fn param_type(lent: bool, written_in_place: bool) -> String {
    if written_in_place {
        format!("&mut {INT_VEC}")
    } else if lent {
        format!("&{INT_VEC}")
    } else {
        INT_VEC.to_string()
    }
}

/// Whether `e` is an `IntVec`: a binding that holds one, a call returning one, a `copy of` or a give of either, or a
/// list literal built as one.
pub(crate) fn is_int_vec(e: &Expr, variable_types: &HashMap<Symbol, String>) -> bool {
    match e {
        Expr::Identifier(s) => variable_types.get(s).is_some_and(|t| holds_int_vec(t)),
        Expr::Call { function, .. } => promotes_return(*function),
        Expr::Copy { expr } => is_int_vec(expr, variable_types),
        Expr::Give { value } => is_int_vec(value, variable_types),
        Expr::List(_) => literal_promoted(e),
        _ => false,
    }
}

/// What this tier knows that the shared exactness decision does not.

/// The literal `value`, stored at `index` of `list`, advances the item there by: `(item index of list) + c` (`c + (item
/// index of list)`) or `(item index of list) - c`.
fn step_in_place(list: Symbol, index: &Expr, value: &Expr) -> Option<i64> {
    use crate::ast::stmt::{BinaryOpKind, Literal};
    let item = |e: &Expr| matches!(e, Expr::Index { collection: Expr::Identifier(l), index: k }
        if *l == list && super::peephole::exprs_equal(k, index));
    match value {
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right: Expr::Literal(Literal::Number(c)) } if item(left) => Some(*c),
        Expr::BinaryOp { op: BinaryOpKind::Add, left: Expr::Literal(Literal::Number(c)), right } if item(right) => Some(*c),
        Expr::BinaryOp { op: BinaryOpKind::Subtract, left, right: Expr::Literal(Literal::Number(c)) } if item(left) => {
            c.checked_neg()
        }
        _ => None,
    }
}

/// Whether `value` stored at `index` of `list` is `(item index of list) + 1`.
fn is_increment(list: Symbol, index: &Expr, value: &Expr) -> bool {
    use crate::ast::stmt::{BinaryOpKind, Literal};
    let item = |e: &Expr| matches!(e, Expr::Index { collection: Expr::Identifier(l), index: k }
        if *l == list && super::peephole::exprs_equal(k, index));
    matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
        if (item(left) && matches!(right, Expr::Literal(Literal::Number(1))))
            || (item(right) && matches!(left, Expr::Literal(Literal::Number(1)))))
}

/// Whether `body` advances `counter` by one exactly once a pass, directly, and assigns it nowhere else.
pub(super) fn advances_once(body: &[Stmt], counter: Symbol) -> bool {
    use crate::ast::stmt::{BinaryOpKind, Literal};
    let mut direct = 0;
    for s in body {
        if let Stmt::Set { target, value } = s {
            if *target == counter {
                let plus_one = matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
                    if matches!(left, Expr::Identifier(l) if *l == counter)
                        && matches!(right, Expr::Literal(Literal::Number(1))));
                if !plus_one {
                    return false;
                }
                direct += 1;
                continue;
            }
        }
        if crate::optimize::stmt_touches(s, counter) && writes(s, counter) {
            return false;
        }
    }
    direct == 1
}

/// Whether `stmt` assigns `name` anywhere inside it.
pub(super) fn writes(stmt: &Stmt, name: Symbol) -> bool {
    let mut written = false;
    walk_nodes(std::slice::from_ref(stmt), |node| {
        if let Visited::Stmt(Stmt::Set { target, .. } | Stmt::Let { var: target, .. }) = node {
            written |= *target == name;
        }
        Walk::Into
    });
    written
}

/// The literal a counter holds on entry to the statement at the innermost of `path`: its last binding before that
/// statement, searched in that statement's block and then outward, through the blocks that hold it (each with the
/// position of the statement that holds the next).
pub(super) fn entry_literal(path: &[(&[Stmt], usize)], counter: Symbol) -> Option<i64> {
    use crate::ast::stmt::Literal;
    path.iter().rev().flat_map(|(block, at)| block[..*at].iter().rev()).find_map(|s| match s {
        Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } if *var == counter => Some(match value {
            Expr::Literal(Literal::Number(n)) => Some(*n),
            _ => None,
        }),
        s if writes(s, counter) => Some(None),
        _ => None,
    })?
}

/// Whether the loop at the innermost of `path` runs at most `i64::MAX` passes: over a collection (no longer than
/// memory holds), over a range from a literal of at least one, or `While i < bound` (`<=` from at least one) with `i`
/// entering at a literal that is not negative, advanced by one once a pass, and `bound` a word.
pub(super) fn counted_loop(path: &[(&[Stmt], usize)], words: &dyn Fn(&Expr) -> bool) -> bool {
    use crate::ast::stmt::{BinaryOpKind, Literal};
    let Some((block, at)) = path.last() else { return false };
    match &block[*at] {
        Stmt::Repeat { iterable: Expr::Range { start: Expr::Literal(Literal::Number(n)), .. }, .. } => *n >= 1,
        Stmt::Repeat { iterable: Expr::Range { .. }, .. } => false,
        Stmt::Repeat { .. } => true,
        Stmt::While { cond: Expr::BinaryOp { op, left: Expr::Identifier(counter), right }, body, .. } => {
            let Some(entry) = entry_literal(path, *counter) else { return false };
            let bounded = match op {
                BinaryOpKind::Lt => entry >= 0,
                BinaryOpKind::LtEq => entry >= 1,
                _ => false,
            };
            bounded && words(right) && advances_once(body, *counter)
        }
        _ => false,
    }
}

/// The stores a body makes into its `Seq of Int` locals, for the counting certificate ([`counting_increments`]).
#[derive(Default)]
struct Stores {
    /// Every local stored into.
    stored: HashSet<SeqSlot>,
    /// The locals some store into is neither `Push 0` nor an increment a counting loop bounds.
    other: HashSet<SeqSlot>,
    /// The stores the walk classified, by address.
    seen: HashSet<usize>,
}

/// Every `Push` and `Set item` into a body's `Seq of Int` locals the walk of [`count_stores`] (or of [`once_stores`])
/// never classified, a store inside a statement it does not descend into, marked as one no certificate bounds: a
/// certificate speaks for every store into its list, or for none.
fn unclassified_stores(body: &[Stmt], owner: Option<Symbol>, seen: &HashSet<usize>, mut mark: impl FnMut(SeqSlot)) {
    walk_nodes(body, |node| {
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => return Walk::Past,
            Visited::Stmt(
                s @ (Stmt::Push { collection: Expr::Identifier(list), .. }
                | Stmt::SetIndex { collection: Expr::Identifier(list), .. }),
            ) if !seen.contains(&(s as *const Stmt as usize)) => mark(SeqSlot::Local(owner, *list)),
            _ => {}
        }
        Walk::Into
    });
}

/// Whether `value` stored at `index` of `list`, the statement at `at` of `block`, is the item read a few statements
/// before plus one: `Let c be item index of list.` earlier in `block`, then `Set item index of list to c + 1.`, with no
/// store to `list` between them and neither `c` nor what `index` reads assigned there.
fn increments_through(block: &[Stmt], at: usize, list: Symbol, index: &Expr, value: &Expr) -> bool {
    use crate::ast::stmt::{BinaryOpKind, Literal};
    let temp = match value {
        Expr::BinaryOp { op: BinaryOpKind::Add, left: Expr::Identifier(c), right: Expr::Literal(Literal::Number(1)) }
        | Expr::BinaryOp { op: BinaryOpKind::Add, left: Expr::Literal(Literal::Number(1)), right: Expr::Identifier(c) } => *c,
        _ => return false,
    };
    let reads = crate::optimize::Mentions::of_expr(index);
    for s in block[..at].iter().rev() {
        if let Stmt::Let { var, value: Expr::Index { collection: Expr::Identifier(l), index: k }, .. } = s {
            if *var == temp {
                return *l == list && super::peephole::exprs_equal(k, index);
            }
        }
        let stores_list = matches!(s, Stmt::SetIndex { collection: Expr::Identifier(l), .. }
            | Stmt::Push { collection: Expr::Identifier(l), .. } if *l == list);
        if stores_list || writes(s, temp) || reads.symbols().any(|r| writes(s, r)) {
            return false;
        }
    }
    false
}

/// The most items a list holds: a list of `Int` stores each in at least a word (eight bytes) unless every value it
/// holds is proven to fit in less, and no allocation passes `isize::MAX` bytes.
const MOST_ITEMS: i128 = 1 << 60;

/// The bounds the counter of each list-bounded loop keeps through its body, by the address of every expression in
/// that body: `(counter, least, greatest)`. A loop is list-bounded when its counter enters at a literal and advances by
/// one exactly once a pass ([`advances_once`]), and either every pass, at the body's top level, pushes onto a list or
/// writes the list's item at the counter plus or minus a literal (a list the body sees only through items, `length of`
/// and stores), or the loop runs while the counter is at most (below) the length of a held collection. No collection
/// holds more than [`MOST_ITEMS`]: a loop that pushes once a pass runs no more passes than that (the push past it fails
/// first), one that writes the item its counter names fails once the counter names none, and one its condition holds
/// within a length stops there. So `k * 7` in `While k < n: Push k * 7 to xs` stays in the word whatever `n` is.
pub(super) fn list_bounded_counters(body: &[Stmt]) -> HashMap<usize, Vec<(Symbol, i128, i128)>> {
    fn sees_only_items(body: &[Stmt], list: Symbol) -> bool {
        let (mut all, mut allowed, mut refused) = (0usize, 0usize, false);
        walk_nodes(body, |node| {
            match node {
                Visited::Expr(e @ Expr::Closure { .. }) => {
                    refused |= crate::optimize::expr_touches(e, list);
                    return Walk::Past;
                }
                Visited::Stmt(
                    Stmt::Push { collection: Expr::Identifier(l), .. } | Stmt::SetIndex { collection: Expr::Identifier(l), .. },
                ) if *l == list => allowed += 1,
                Visited::Expr(Expr::Identifier(l)) if *l == list => all += 1,
                Visited::Expr(Expr::Index { collection: Expr::Identifier(l), .. } | Expr::Length { collection: Expr::Identifier(l) })
                    if *l == list =>
                {
                    allowed += 1
                }
                _ => {}
            }
            Walk::Into
        });
        !refused && all == allowed
    }
    /// The literal `c` of an index `counter + c` (`c + counter`, `counter - c` as `-c`, `counter` as 0).
    fn offset(index: &Expr, counter: Symbol) -> Option<i128> {
        use crate::ast::stmt::{BinaryOpKind, Literal};
        match index {
            Expr::Identifier(c) if *c == counter => Some(0),
            Expr::BinaryOp { op: BinaryOpKind::Add, left: Expr::Identifier(c), right: Expr::Literal(Literal::Number(k)) }
            | Expr::BinaryOp { op: BinaryOpKind::Add, left: Expr::Literal(Literal::Number(k)), right: Expr::Identifier(c) }
                if *c == counter =>
            {
                Some(i128::from(*k))
            }
            Expr::BinaryOp { op: BinaryOpKind::Subtract, left: Expr::Identifier(c), right: Expr::Literal(Literal::Number(k)) }
                if *c == counter =>
            {
                Some(-i128::from(*k))
            }
            _ => None,
        }
    }
    fn walk<'s>(block: &'s [Stmt], path: &[(&'s [Stmt], usize)], out: &mut HashMap<usize, Vec<(Symbol, i128, i128)>>) {
        for (at, s) in block.iter().enumerate() {
            let mut here = path.to_vec();
            here.push((block, at));
            match s {
                Stmt::While { body, .. } => {
                    let bound = counter_of(s).and_then(|counter| {
                        let entry = i128::from(entry_literal(&here, counter)?);
                        if !advances_once(body, counter) {
                            return None;
                        }
                        let most = body.iter().filter_map(|b| match b {
                            Stmt::Push { collection: Expr::Identifier(l), .. } if sees_only_items(body, *l) => {
                                Some(entry + MOST_ITEMS)
                            }
                            Stmt::SetIndex { collection: Expr::Identifier(l), index, .. } if sees_only_items(body, *l) => {
                                offset(index, counter).map(|c| entry.max(MOST_ITEMS - c + 2))
                            }
                            _ => None,
                        });
                        // `While counter <= length of xs` (or `<`): every pass the body begins sees the counter at most
                        // the length of a held collection, whatever the body does to it.
                        let by_length = match s {
                            Stmt::While {
                                cond:
                                    Expr::BinaryOp {
                                        op: crate::ast::stmt::BinaryOpKind::LtEq | crate::ast::stmt::BinaryOpKind::Lt,
                                        left: Expr::Identifier(c),
                                        right: Expr::Length { collection: Expr::Identifier(_) },
                                    },
                                ..
                            } if *c == counter => Some(entry.max(MOST_ITEMS)),
                            _ => None,
                        };
                        most.chain(by_length).min().map(|most| (counter, entry, most))
                    });
                    if let Some(bound) = bound {
                        walk_nodes(body, |node| {
                            if let Visited::Expr(e) = node {
                                out.entry(address(e)).or_default().push(bound);
                            }
                            Walk::Into
                        });
                    }
                    walk(body, &[], out);
                }
                Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => walk(body, &[], out),
                Stmt::If { then_block, else_block, .. } => {
                    walk(then_block, &here, out);
                    if let Some(else_block) = else_block {
                        walk(else_block, &here, out);
                    }
                }
                Stmt::Inspect { arms, .. } => {
                    for arm in arms {
                        walk(arm.body, &here, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    walk(body, &[], &mut out);
    out
}

/// Whether `index` names a different item for each value of `counter`: `counter`, or `counter` plus or minus a
/// literal.
fn injective_in(index: &Expr, counter: Symbol) -> bool {
    use crate::ast::stmt::{BinaryOpKind, Literal};
    match index {
        Expr::Identifier(c) => *c == counter,
        Expr::BinaryOp { op: BinaryOpKind::Add | BinaryOpKind::Subtract, left: Expr::Identifier(c), right: Expr::Literal(Literal::Number(_)) } => *c == counter,
        Expr::BinaryOp { op: BinaryOpKind::Add, left: Expr::Literal(Literal::Number(_)), right: Expr::Identifier(c) } => *c == counter,
        _ => false,
    }
}

/// The counter of the loop `l`, a `While counter < bound` or `While counter <= bound`.
fn counter_of(l: &Stmt) -> Option<Symbol> {
    match l {
        Stmt::While { cond: Expr::BinaryOp { left: Expr::Identifier(c), .. }, .. } => Some(*c),
        _ => None,
    }
}

/// Along any one path through `block`, per local, how many increments it runs: at the top of a body (`depth` 0),
/// the counting loops whose passes increment the local, each counted once, since its trip count bounds its increments;
/// in such a loop's body (`depth` 1, `counted`), the increments one pass makes, an inner counting loop that
/// increments items at an index that is its counter plus a literal counted once (`inject`: its counter names each
/// item at most once a run). Branches are alternatives: a path takes one of them. `path` holds the blocks enclosing
/// `block` within one pass, with the position in each of the statement holding the next, where a counter's entry value
/// is looked for. Every other store is recorded in `stores.other`.
#[allow(clippy::too_many_arguments)]
fn count_stores<'s, 'a>(
    block: &'s [Stmt<'a>],
    path: &[(&'s [Stmt<'a>], usize)],
    owner: Option<Symbol>,
    depth: usize,
    counted: bool,
    inject: Option<Symbol>,
    words: &dyn Fn(&Expr) -> bool,
    stores: &mut Stores,
) -> HashMap<SeqSlot, u32> {
    use crate::ast::stmt::Literal;
    let mut runs: HashMap<SeqSlot, u32> = HashMap::new();
    let then = |runs: &mut HashMap<SeqSlot, u32>, more: HashMap<SeqSlot, u32>| {
        for (slot, n) in more {
            *runs.entry(slot).or_default() += n;
        }
    };
    let alternatives = |arms: Vec<HashMap<SeqSlot, u32>>| {
        let mut most: HashMap<SeqSlot, u32> = HashMap::new();
        for arm in arms {
            for (slot, n) in arm {
                let m = most.entry(slot).or_default();
                *m = (*m).max(n);
            }
        }
        most
    };
    for (at, s) in block.iter().enumerate() {
        let here = || {
            let mut inner = path.to_vec();
            inner.push((block, at));
            inner
        };
        match s {
            Stmt::Push { value, collection: Expr::Identifier(list) } => {
                let slot = SeqSlot::Local(owner, *list);
                stores.stored.insert(slot);
                stores.seen.insert(s as *const Stmt as usize);
                if !matches!(value, Expr::Literal(Literal::Number(0))) {
                    stores.other.insert(slot);
                }
            }
            Stmt::SetIndex { collection: Expr::Identifier(list), index, value } => {
                let slot = SeqSlot::Local(owner, *list);
                stores.stored.insert(slot);
                stores.seen.insert(s as *const Stmt as usize);
                let increments =
                    is_increment(*list, index, value) || increments_through(block, at, *list, index, value);
                let bounded = match inject {
                    Some(counter) => injective_in(index, counter),
                    None => counted && depth == 1,
                };
                if increments && bounded {
                    *runs.entry(slot).or_default() += 1;
                } else {
                    stores.other.insert(slot);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                let inner_counter = (depth == 1 && counted && inject.is_none() && counted_loop(&here(), words))
                    .then(|| counter_of(s))
                    .flatten();
                let counting = depth == 0 && counted_loop(&here(), words);
                let pass = count_stores(body, &[], owner, depth + 1, counting, inner_counter, words, stores);
                let mut bounded = HashMap::new();
                for (slot, n) in pass {
                    if n > 1 {
                        stores.other.insert(slot);
                    } else if n == 1 && (counting || inner_counter.is_some()) {
                        bounded.insert(slot, 1);
                    } else if n == 1 {
                        stores.other.insert(slot);
                    }
                }
                then(&mut runs, bounded);
            }
            Stmt::If { then_block, else_block, .. } => {
                let inner = here();
                let mut arms = vec![count_stores(then_block, &inner, owner, depth, counted, inject, words, stores)];
                if let Some(else_block) = else_block {
                    arms.push(count_stores(else_block, &inner, owner, depth, counted, inject, words, stores));
                }
                then(&mut runs, alternatives(arms));
            }
            Stmt::Inspect { arms, .. } => {
                let inner = here();
                let arms = arms
                    .iter()
                    .map(|arm| count_stores(arm.body, &inner, owner, depth, counted, inject, words, stores));
                then(&mut runs, alternatives(arms.collect()));
            }
            Stmt::Zone { body, .. } => {
                let inner = here();
                then(&mut runs, count_stores(body, &inner, owner, depth, counted, inject, words, stores));
            }
            _ => {}
        }
    }
    runs
}

/// The flow classes whose item increments cannot leave the word, by root. Every item starts at zero: the class holds
/// no parameter of the body that stores into it, so each run of that body builds its list afresh, from empty lists
/// and literals of zeros. Its one stored local takes pushes of zero and increments, and along any path through its
/// body one counting loop runs them, at most one a pass. No item then passes the loop's trip count, which fits a word.
/// The certificate is the bounded count accumulator's (`stmt::collect_bounded_count_accumulators_with_purity`) carried
/// to a list's items.
fn counting_increments(
    bodies: &[(Option<Symbol>, &[Stmt], &[Symbol])],
    words: &dyn Fn(&Expr) -> bool,
    flow: &mut SeqFlow,
) -> HashSet<SeqSlot> {
    use crate::ast::stmt::Literal;
    let mut stores = Stores::default();
    let mut runs: HashMap<SeqSlot, u32> = HashMap::new();
    for (owner, body, _) in bodies {
        runs.extend(count_stores(body, &[], *owner, 0, false, None, words, &mut stores));
    }
    for (owner, body, _) in bodies {
        let mut missed = Vec::new();
        unclassified_stores(body, *owner, &stores.seen, |slot| missed.push(slot));
        for slot in missed {
            stores.stored.insert(slot);
            stores.other.insert(slot);
        }
    }
    for slot in &stores.stored {
        flow.find(*slot);
    }
    let mut members: HashMap<SeqSlot, Vec<SeqSlot>> = HashMap::new();
    for slot in flow.slots() {
        members.entry(flow.find(slot)).or_default().push(slot);
    }
    let zeros = |literal: &Expr| {
        matches!(literal, Expr::List(items) if items.iter().all(|i| matches!(i, Expr::Literal(Literal::Number(0)))))
    };
    let mut born_zero: HashMap<SeqSlot, bool> = HashMap::new();
    for (literal, dst, _) in flow.stored_literals.clone() {
        let root = flow.find(dst);
        *born_zero.entry(root).or_insert(true) &= zeros(literal);
    }
    let mut counting = HashSet::new();
    for (root, slots) in &members {
        let stored: Vec<SeqSlot> = slots.iter().copied().filter(|s| stores.stored.contains(s)).collect();
        let [slot @ SeqSlot::Local(owner, _)] = stored.as_slice() else { continue };
        let params: &[Symbol] = bodies.iter().find(|(o, _, _)| o == owner).map_or(&[], |(_, _, p)| *p);
        let arrives = slots.iter().any(|s| matches!(s, SeqSlot::Local(o, p) if o == owner && params.contains(p)));
        if !arrives
            && born_zero.get(root).copied().unwrap_or(true)
            && !stores.other.contains(slot)
            && runs.get(slot) == Some(&1)
        {
            counting.insert(*root);
        }
    }
    counting
}

/// The integer `e` is where it is a literal, or a sum or difference of literals (`0 - 1`).
fn literal_int(e: &Expr) -> Option<i64> {
    use crate::ast::stmt::{BinaryOpKind, Literal};
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } => literal_int(left)?.checked_add(literal_int(right)?),
        Expr::BinaryOp { op: BinaryOpKind::Subtract, left, right } => literal_int(left)?.checked_sub(literal_int(right)?),
        _ => None,
    }
}

/// The guard `If item index of list equals fill:` (`fill equals item index of list`), `fill` a literal: the list,
/// the index and the fill.
fn fill_guard<'e, 'a>(cond: &'e Expr<'a>) -> Option<(Symbol, &'e Expr<'a>, i64)> {
    use crate::ast::stmt::BinaryOpKind;
    let Expr::BinaryOp { op: BinaryOpKind::Eq, left, right } = cond else { return None };
    let item = |e: &'e Expr<'a>| match e {
        Expr::Index { collection: Expr::Identifier(list), index } => Some((*list, &**index)),
        _ => None,
    };
    match (item(left), item(right)) {
        (Some((list, index)), _) => Some((list, index, literal_int(right)?)),
        (_, Some((list, index))) => Some((list, index, literal_int(left)?)),
        _ => None,
    }
}

/// The literal `c` of a step `(item j of list) + c` (`c + (item j of list)`).
fn step_of(list: Symbol, value: &Expr) -> Option<i64> {
    use crate::ast::stmt::BinaryOpKind;
    let item = |e: &Expr| matches!(e, Expr::Index { collection: Expr::Identifier(l), .. } if *l == list);
    match value {
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } if item(left) => literal_int(right),
        Expr::BinaryOp { op: BinaryOpKind::Add, left, right } if item(right) => literal_int(left),
        _ => None,
    }
}

/// The stores into one `Seq of Int` local, for the write-once certificate ([`write_once_steps`]).
#[derive(Default)]
struct OnceStores {
    /// The literals pushed onto it: the values its items are born with.
    births: Vec<i64>,
    /// The literals stored into an item.
    literals: Vec<i64>,
    /// Each guarded step: the fill its guard tests, and the literal it adds.
    steps: Vec<(i64, i64)>,
    /// Whether some store is none of these.
    other: bool,
}

/// The stores `block` makes into its body's `Seq of Int` locals, classified for the write-once certificate: a push
/// of a literal, a store of a literal, a guarded step, or anything else. A step is guarded where it is a statement of
/// the `then` block of `If item index of list equals fill:` (`guard`), at that very index, and nothing before it in
/// that block writes the list, what the index reads, or runs a call.
fn once_stores<'a>(
    block: &[Stmt<'a>],
    owner: Option<Symbol>,
    guard: Option<(Symbol, &Expr<'a>, i64)>,
    out: &mut HashMap<SeqSlot, OnceStores>,
    seen: &mut HashSet<usize>,
) {
    for (at, s) in block.iter().enumerate() {
        match s {
            Stmt::Push { collection: Expr::Identifier(list), value } => {
                seen.insert(s as *const Stmt as usize);
                let stores = out.entry(SeqSlot::Local(owner, *list)).or_default();
                match literal_int(value) {
                    Some(v) => stores.births.push(v),
                    None => stores.other = true,
                }
            }
            Stmt::SetIndex { collection: Expr::Identifier(list), index, value } => {
                seen.insert(s as *const Stmt as usize);
                if let Some(v) = literal_int(value) {
                    out.entry(SeqSlot::Local(owner, *list)).or_default().literals.push(v);
                    continue;
                }
                let guarded = guard.filter(|(guarded, at_index, _)| {
                    *guarded == *list && super::peephole::exprs_equal(at_index, index) && {
                        let before = crate::optimize::WriteSet::of_block(&block[..at]);
                        let reads = crate::optimize::Mentions::of_expr(index);
                        !before.has_calls()
                            && !before.is_opaque()
                            && !before.may_write(*list)
                            && !block[..at].iter().any(|b| crate::optimize::stmt_touches(b, *list))
                            && !reads.symbols().any(|r| before.may_write(r))
                    }
                });
                let stores = out.entry(SeqSlot::Local(owner, *list)).or_default();
                match guarded.zip(step_of(*list, value)) {
                    Some(((_, _, fill), step)) => stores.steps.push((fill, step)),
                    None => stores.other = true,
                }
            }
            Stmt::If { cond, then_block, else_block } => {
                once_stores(then_block, owner, fill_guard(cond), out, seen);
                if let Some(else_block) = else_block {
                    once_stores(else_block, owner, None, out, seen);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => {
                once_stores(body, owner, None, out, seen)
            }
            Stmt::Inspect { arms, .. } => {
                for arm in arms {
                    once_stores(arm.body, owner, None, out, seen);
                }
            }
            _ => {}
        }
    }
}

/// The flow classes whose items stay in the word because each is written once from another, by root. Every item is
/// born the fill `F`, the least literal ever pushed onto the list (the class holds no parameter of the body that
/// stores into it, so each run of that body builds its list afresh); a store of a literal writes one above `F`; and
/// the one other store is a step `item k = (item j) + c`, `c` in 1 and 2, under the guard that item `k` is still `F`.
/// No store writes `F` again, so a step finds each item at `F` once: there are no more steps than pushes, at most
/// 2^60 (a list's length). Every item then lies between `F` and the largest literal plus 2^61, which fits a word
/// while `F` is at least -2^62 and every literal at most 2^61. Breadth-first distances and the visited marks of a
/// graph walk keep their words.
fn write_once_steps(bodies: &[(Option<Symbol>, &[Stmt], &[Symbol])], flow: &mut SeqFlow) -> HashSet<SeqSlot> {
    let mut stores: HashMap<SeqSlot, OnceStores> = HashMap::new();
    let mut seen = HashSet::new();
    for (owner, body, _) in bodies {
        once_stores(body, *owner, None, &mut stores, &mut seen);
    }
    for (owner, body, _) in bodies {
        unclassified_stores(body, *owner, &seen, |slot| stores.entry(slot).or_default().other = true);
    }
    for slot in stores.keys() {
        flow.find(*slot);
    }
    let mut members: HashMap<SeqSlot, Vec<SeqSlot>> = HashMap::new();
    for slot in flow.slots() {
        members.entry(flow.find(slot)).or_default().push(slot);
    }
    let mut born: HashMap<SeqSlot, Option<Vec<i64>>> = HashMap::new();
    for (literal, dst, _) in flow.stored_literals.clone() {
        let items = match literal {
            Expr::List(items) => items.iter().map(|i| literal_int(i)).collect::<Option<Vec<i64>>>(),
            _ => None,
        };
        let entry = born.entry(flow.find(dst)).or_insert_with(|| Some(Vec::new()));
        *entry = entry.take().zip(items).map(|(mut all, more)| {
            all.extend(more);
            all
        });
    }
    let mut once = HashSet::new();
    for (root, slots) in &members {
        let stored: Vec<SeqSlot> = slots.iter().copied().filter(|s| stores.contains_key(s)).collect();
        let [slot @ SeqSlot::Local(owner, _)] = stored.as_slice() else { continue };
        let params: &[Symbol] = bodies.iter().find(|(o, _, _)| o == owner).map_or(&[], |(_, _, p)| *p);
        if slots.iter().any(|s| matches!(s, SeqSlot::Local(o, p) if o == owner && params.contains(p))) {
            continue;
        }
        let list = &stores[slot];
        let Some(literal_births) = born.get(root).cloned().unwrap_or(Some(Vec::new())) else { continue };
        let births: Vec<i64> = list.births.iter().copied().chain(literal_births).collect();
        let Some(&fill) = births.iter().min() else { continue };
        let largest = births.iter().chain(&list.literals).copied().max().unwrap_or(fill);
        if !list.other
            && !list.steps.is_empty()
            && fill >= -(1i64 << 62)
            && largest <= 1i64 << 61
            && list.literals.iter().all(|v| *v > fill)
            && list.steps.iter().all(|(guarded, step)| *guarded == fill && (1..=2).contains(step))
        {
            once.insert(*root);
        }
    }
    once
}

/// Number the versioned loops of the program about to be emitted from the first.
pub(crate) fn reset_loop_labels() {
    LOOP_COUNT.with(|c| c.set(0));
}

/// Decide the program's promotable lists and install the decision for this compile.
pub(super) fn install(
    stmts: &[Stmt],
    interner: &Interner,
    promo: &PromotionFacts,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> Scope {
    let decision = decide(stmts, interner, promo, oracle);
    if std::env::var_os("LOGOS_INTVEC_TRACE").is_some() {
        let body = |owner: Option<Symbol>| owner.map_or("Main".to_string(), |f| interner.resolve(f).to_string());
        let mut lists: Vec<String> = decision
            .locals
            .iter()
            .map(|(owner, name)| format!("{}::{}", body(*owner), interner.resolve(*name)))
            .chain(decision.returns.iter().map(|f| format!("{} returns", interner.resolve(*f))))
            .collect();
        lists.sort();
        for list in lists {
            eprintln!("LOGOS_INTVEC: `{list}` → IntVec");
        }
    }
    let depth = PROMOTING.with(|p| {
        let mut p = p.borrow_mut();
        p.push(Rc::new(decision));
        p.len()
    });
    Scope { depth }
}

fn decide(
    stmts: &[Stmt],
    interner: &Interner,
    promo: &PromotionFacts,
    oracle: Option<&crate::optimize::OracleFacts>,
) -> Promoting {
    // A word twin holds its lists as machine words (`word_twin`): none of its lists is an `IntVec`.
    let fns: Vec<(Symbol, Vec<Symbol>, &[Stmt])> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef { name, params, body, is_native: false, .. } if !super::word_twin::is_twin(*name) => {
                Some((*name, params.iter().map(|(p, _)| *p).collect(), *body))
            }
            _ => None,
        })
        .collect();
    let fn_params: HashMap<Symbol, Vec<Symbol>> = fns.iter().map(|(f, ps, _)| (*f, ps.clone())).collect();
    let mut flow = SeqFlow::default();
    for (f, params, body) in &fns {
        record_seq_flow(body, Some(*f), params, &fn_params, interner, &mut flow);
    }
    record_seq_flow(stmts, None, &[], &fn_params, interner, &mut flow);

    let counted_bodies: Vec<(Option<Symbol>, &[Stmt], &[Symbol])> = fns
        .iter()
        .map(|(f, params, body)| (Some(*f), *body, params.as_slice()))
        .chain(std::iter::once((None, stmts, &[][..])))
        .collect();
    // A bound held as a `LogosInt` in any body may pass the word, and the loop's count with it.
    let promoted: HashSet<Symbol> = counted_bodies
        .iter()
        .flat_map(|(_, body, _)| promotable_int_vars(body, &promo.bigint_fns, &promo.bigint_fields, &HashSet::new()))
        .collect();
    let word_bound = |e: &Expr| match e {
        Expr::Literal(crate::ast::stmt::Literal::Number(_)) | Expr::Length { .. } => true,
        Expr::Identifier(s) => !promoted.contains(s),
        _ => false,
    };
    let mut counting = counting_increments(&counted_bodies, &word_bound, &mut flow);
    counting.extend(write_once_steps(&counted_bodies, &mut flow));
    let mut roots: Vec<SeqSlot> = Vec::new();
    let bodies = fns
        .iter()
        .map(|(f, params, body)| {
            let seeds: HashSet<Symbol> = if promo.exact_hofs.contains(f) { params.iter().copied().collect() } else { HashSet::new() };
            (Some(*f), *body, seeds)
        })
        .chain(std::iter::once((None, stmts, HashSet::new())));
    for (owner, body, seeds) in bodies {
        let arriving = promotable_int_vars(body, &promo.bigint_fns, &promo.bigint_fields, &seeds);
        let (none, counters) = (HashSet::new(), list_bounded_counters(body));
        let host = super::exact_scalars::ExactHost {
            bigint_fns: &promo.bigint_fns,
            bigint_fields: &promo.bigint_fields,
            arriving: &arriving,
            ints: None,
            bounded: &none,
            counters: &counters,
            oracle,
            pow: interner.lookup("pow"),
            abs: interner.lookup("abs"),
        };
        let decision = crate::analysis::exactness::body_exactness(body, owner, &host);
        let outgrows = |value: &Expr| {
            decision.exprs.contains(&address(value))
                || const_overflows_i64(value)
                || mentions_promotable(value, &arriving)
                || matches!(value, Expr::Call { function, .. } if promo.bigint_fns.contains(function))
        };
        walk_nodes(body, |node| {
            match node {
                Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => return Walk::Past,
                // A certified list's stores never leave the word, the certificate's steps and literals alike: no root.
                Visited::Stmt(
                    Stmt::Push { collection: collection @ Expr::Identifier(t), .. }
                    | Stmt::SetIndex { collection: collection @ Expr::Identifier(t), .. },
                ) if super::instantiation::checked_int_seq(collection)
                    && counting.contains(&flow.find(SeqSlot::Local(owner, *t))) => {}
                Visited::Stmt(
                    Stmt::Push { collection: collection @ Expr::Identifier(t), value }
                    | Stmt::SetIndex { collection: collection @ Expr::Identifier(t), value, .. },
                ) if super::instantiation::checked_int_seq(collection) && outgrows(value) => {
                    roots.push(SeqSlot::Local(owner, *t))
                }
                _ => {}
            }
            Walk::Into
        });
    }
    let big_item = |literal: &Expr| {
        super::instantiation::checked_int_seq(literal)
            && matches!(literal, Expr::List(items) if items.iter().any(|i| const_overflows_i64(i)))
    };
    for (literal, dst, _) in flow.stored_literals.clone() {
        if big_item(literal) {
            roots.push(dst);
        }
    }

    let promoted_roots: HashSet<SeqSlot> = roots.into_iter().map(|r| flow.find(r)).collect();
    let mut raw_roots: HashSet<SeqSlot> = HashSet::new();
    // A list the exact extension holds is a `LogosValue`, and so is every list it moves between.
    let extension_holds = |slot: SeqSlot| match slot {
        SeqSlot::Local(owner, name) => super::tower::binding_in(owner, name),
        SeqSlot::Return(f) => super::tower::returns(f),
        SeqSlot::Field(_) => false,
    };
    for slot in flow.slots() {
        if flow.raw.contains(&slot) || matches!(slot, SeqSlot::Field(_)) || extension_holds(slot) {
            raw_roots.insert(flow.find(slot));
        }
    }
    let mut decision = Promoting { params: fn_params.clone(), ..Promoting::default() };
    for slot in flow.slots() {
        let root = flow.find(slot);
        if !promoted_roots.contains(&root) || raw_roots.contains(&root) {
            continue;
        }
        match slot {
            SeqSlot::Local(owner, name) => {
                decision.locals.insert((owner, name));
            }
            SeqSlot::Return(f) => {
                decision.returns.insert(f);
            }
            SeqSlot::Field(_) => {}
        }
    }
    for (literal, dst, _) in flow.stored_literals.clone() {
        let root = flow.find(dst);
        if big_item(literal) && promoted_roots.contains(&root) && !raw_roots.contains(&root) {
            decision.literals.insert(address(literal));
        }
    }
    for (literal, _) in flow.read_literals.clone() {
        if big_item(literal) {
            decision.literals.insert(address(literal));
        }
    }
    decision
}

/// The place an `IntVec` binding is read through: the binding, or the list a parameter lends.
fn place(sym: Symbol, ecx: &super::expr::ExprCtx) -> String {
    let name = crate::analysis::types::RustNames::new(ecx.interner).ident(sym);
    let lent = ecx.variable_types.get(&sym).is_some_and(|t| t.starts_with('&'));
    if lent {
        format!("(*{name})")
    } else {
        name
    }
}

/// `e`, an `IntVec`, as an expression a method can be called on without moving it out of a binding.
pub(crate) fn receiver(e: &Expr, ecx: &super::expr::ExprCtx) -> String {
    match e {
        Expr::Identifier(sym) => place(*sym, ecx),
        Expr::Copy { expr } | Expr::Give { value: expr } => receiver(expr, ecx),
        _ => format!("({})", owned(e, ecx)),
    }
}

/// `e`, an `IntVec`, as a value the receiving slot owns: a binding's list moved at its last use or copied, a call's
/// result, a `copy of`'s, a literal built as one, a new empty list.
pub(crate) fn owned(e: &Expr, ecx: &super::expr::ExprCtx) -> String {
    match e {
        Expr::Identifier(sym) => {
            let own = ecx.variable_types.get(sym).is_some_and(|t| !t.starts_with('&'));
            if own && super::expr::is_movable_last_use(*sym) {
                place(*sym, ecx)
            } else {
                format!("{}.clone()", place(*sym, ecx))
            }
        }
        Expr::Copy { expr } => format!("{}.clone()", receiver(expr, ecx)),
        Expr::Give { value } => owned(value, ecx),
        Expr::List(items) => literal(items, ecx),
        Expr::New { .. } => format!("{INT_VEC}::new()"),
        _ => super::expr::codegen_expr_ctx(e, ecx),
    }
}

/// A list literal as an `IntVec`: words while every item is one, exact integers once one is past the word.
pub(crate) fn literal(items: &[&Expr], ecx: &super::expr::ExprCtx) -> String {
    let words: Option<Vec<String>> = items
        .iter()
        .map(|e| match e {
            Expr::Literal(crate::ast::stmt::Literal::Number(n)) => Some(format!("{n}_i64")),
            _ => None,
        })
        .collect();
    if let Some(words) = words {
        return format!("{INT_VEC}::from_words(vec![{}])", words.join(", "));
    }
    let exact: Vec<String> = items.iter().map(|e| exact_value(e, ecx)).collect();
    format!("{INT_VEC}::from_ints(vec![{}])", exact.join(", "))
}

/// `value` as the exact `LogosInt` it is, whatever form the emission gives it.
pub(crate) fn exact_value(value: &Expr, ecx: &super::expr::ExprCtx) -> String {
    let (code, exact) = exact_sink_form(value, ecx);
    if exact {
        return code;
    }
    format!("logicaffeine_data::LogosInt::from({})", super::precedence::standalone(&code))
}

/// `value` as an exact sink reads it, and whether that is a `LogosInt` (an item of a list that may hold integers past
/// the word, an `Int` operation on one) rather than the word it is otherwise.
fn exact_sink_form(value: &Expr, ecx: &super::expr::ExprCtx) -> (String, bool) {
    let tolerant = super::expr::ExprCtx { int_exact_tolerant: true, ..*ecx };
    let (code, exact) = with_exact_view(|| {
        (super::expr::codegen_expr_ctx(value, &tolerant), super::expr::emits_logos_int(value, &tolerant))
    });
    (super::precedence::standalone(&code).to_string(), exact && !ecx.int_index_context)
}

/// `code` as the receiver of a method call written where a statement may begin: a block is parenthesized, or it would
/// end there as a statement of its own.
fn method_receiver(code: &str) -> String {
    let receiver = super::precedence::operand(code, super::precedence::Under::Postfix);
    if receiver.starts_with('{') {
        format!("({receiver})")
    } else {
        receiver.into_owned()
    }
}

/// `item index of collection` for an `IntVec`: the exact integer where the context holds one, the word where a word
/// holds it (a value past the word stops the run loudly there).
pub(crate) fn item(collection: &Expr, index: &Expr, ecx: &super::expr::ExprCtx) -> String {
    let exact = exact_view() && !ecx.int_index_context;
    let view = viewed(collection);
    // A held view's word, the exact integer it is where an exact sink reads it.
    let of_view = |read: String| if exact { format!("logicaffeine_data::LogosInt::from({read})") } else { read };
    if super::expr::oracle_proves_index(ecx, collection, index) || super::expr::range_proves_index(collection, index) {
        let offset = super::expr::proven_offset_str(index, ecx, ecx.variable_types, ecx.interner);
        let at = super::precedence::standalone(&offset);
        return match view {
            Some(words) => of_view(format!("unsafe {{ *{words}.get_unchecked({at}) }}")),
            None if exact => format!("{}.get_int({at})", receiver(collection, ecx)),
            // The index the oracle proves is the proof a `Vec<i64>` read takes unchecked on.
            None => format!("unsafe {{ {}.get_word_unchecked({at}) }}", receiver(collection, ecx)),
        };
    }
    let at = super::expr::logos_index_str(index, ecx, ecx.variable_types, ecx.interner);
    if let Some(words) = view {
        return of_view(format!("*{words}.logos_item({})", super::precedence::standalone(&at)));
    }
    let read = if exact { "item_int" } else { "item_word" };
    format!("{}.{read}({})", receiver(collection, ecx), super::precedence::standalone(&at))
}

/// `left op right` (`<`, `<=`, `>`, `>=`, `==`, `!=`) where a side reads an item of an `IntVec`, compared exactly:
/// while every list it reads holds words, their words, each side in 128 bits where it is arithmetic that may pass the
/// word; once one has promoted, the exact integers. An item is never narrowed into the word to be compared. `None` for
/// any other comparison, and inside an exact sink, where an item is read as its exact integer already.
pub(crate) fn comparison(
    op: crate::ast::stmt::BinaryOpKind,
    left: &Expr,
    right: &Expr,
    ecx: &super::expr::ExprCtx,
) -> Option<String> {
    use crate::ast::stmt::BinaryOpKind;
    let op_str = match op {
        BinaryOpKind::Lt => "<",
        BinaryOpKind::LtEq => "<=",
        BinaryOpKind::Gt => ">",
        BinaryOpKind::GtEq => ">=",
        BinaryOpKind::Eq => "==",
        BinaryOpKind::NotEq => "!=",
        _ => return None,
    };
    if ecx.int_index_context || exact_view() || GUARDED.with(|g| g.get()) {
        return None;
    }
    let types = ecx.variable_types;
    let int = |e: &Expr| {
        super::types::infer_numeric_type(e, ecx.interner, types) == "i64" && !super::expr::mentions_bigint_var(e, types)
    };
    if !int(left) || !int(right) {
        return None;
    }
    let mut lists = item_lists(left, types)?;
    for list in item_lists(right, types)? {
        if !lists.contains(&list) {
            lists.push(list);
        }
    }
    if lists.is_empty() {
        return None;
    }
    let plain = super::expr::ExprCtx { int_exact_tolerant: false, ..*ecx };
    // A bare counter a range rebased to zero reads one below its value, as in every other comparison.
    let zero_based = |e: &Expr| matches!(e, Expr::Identifier(s) if types.get(s).is_some_and(|t| t == "__zero_based_i64"));
    let word = |e: &Expr| {
        let code = super::expr::codegen_expr_ctx(e, &plain);
        if zero_based(e) {
            format!("({code} + 1)")
        } else {
            super::precedence::standalone(&code).to_string()
        }
    };
    // Each side as an exact sink reads it: a `LogosInt` orders against a word, so a side that is a word stays one.
    let exact = |e: &Expr, side: super::precedence::Side| {
        let code = if zero_based(e) {
            format!("{} + 1", super::expr::codegen_expr_ctx(e, &plain))
        } else {
            exact_sink_form(e, ecx).0
        };
        super::precedence::operand(&code, super::precedence::Under::Binary(op_str, side)).into_owned()
    };
    let exactly =
        format!("{} {op_str} {}", exact(left, super::precedence::Side::Left), exact(right, super::precedence::Side::Right));
    let raw = |e: &Expr| matches!(e, Expr::Literal(_) | Expr::Identifier(_) | Expr::Index { .. }) || super::expr::stays_word(e, &plain);
    let words = guarded(|| if raw(left) && raw(right) {
        Some(format!("{} {op_str} {}", word(left), word(right)))
    } else {
        let wide = |e: &Expr| if raw(e) { Some(format!("i128::from({})", word(e))) } else { super::expr::wide_chain(e, &plain) };
        match (wide(left), wide(right)) {
            (Some(l), Some(r)) => Some(format!("{} {op_str} {}", super::precedence::standalone(&l), super::precedence::standalone(&r))),
            _ => None,
        }
    });
    let Some(words) = words else { return Some(format!("({exactly})")) };
    // A list a loop runs on the words of holds words throughout.
    let promoted: Vec<String> =
        lists.iter().filter(|l| loop_view(**l).is_none()).map(|l| format!("{}.is_exact()", place(*l, ecx))).collect();
    if promoted.is_empty() {
        return Some(format!("({words})"));
    }
    // The exact comparison where a list holds exact integers, the word comparison where every list holds words: each
    // side is evaluated only for its own form.
    let promoted = promoted.join(" || ");
    Some(format!("((({promoted}) && {exactly}) || (!({promoted}) && {words}))"))
}

/// `length of collection` for an `IntVec`.
pub(crate) fn length(collection: &Expr, ecx: &super::expr::ExprCtx) -> String {
    if let Some(words) = viewed(collection) {
        return format!("({words}.len() as i64)");
    }
    format!("({}.len() as i64)", receiver(collection, ecx))
}

/// `collection contains value` for an `IntVec`, compared by value.
pub(crate) fn contains(collection: &Expr, value: &Expr, ecx: &super::expr::ExprCtx) -> String {
    format!("{}.contains_int(&{})", receiver(collection, ecx), exact_value(value, ecx))
}

/// How a value is stored into an `IntVec`.
pub(crate) enum StoreForm {
    /// A machine word that cannot leave the word.
    Word(String),
    /// The 128-bit value a width-bounded chain computes.
    Wide(String),
    /// The exact integer.
    Exact(String),
    /// A chain over items of `IntVec`s: its 128-bit value while every one of `lists` holds words, else the exact
    /// integer it is.
    Guarded { lists: Vec<String>, wide: String, exact: String },
    /// A chain on the words of the list it is stored into, through one view of them ([`WORDS`]) while it and every
    /// one of `guards`, the other lists the chain reads, hold words: `binds` binding its leaves, `checked` its value or
    /// `None` where a step leaves the word, `past` its exact value over the bound leaves then; `exact` its value where
    /// a list has promoted.
    OnWords { guards: Vec<String>, binds: String, checked: String, past: String, exact: String },
}

/// Whether `value` names the binding `list` only as the list of an item read or a `length of`, which the view of its
/// words a store holds answers.
fn read_as_words(value: &Expr, list: Symbol) -> bool {
    match value {
        Expr::Index { collection: Expr::Identifier(l), index } if *l == list => read_as_words(index, list),
        Expr::Length { collection: Expr::Identifier(l) } if *l == list => true,
        Expr::Identifier(l) => *l != list,
        Expr::Closure { .. } => !crate::optimize::expr_touches(value, list),
        _ => logicaffeine_language::ast_depth::expr_children(value).into_iter().all(|c| read_as_words(c, list)),
    }
}

/// `value`, an item of a list or an arithmetic chain, stored into `target` on its words ([`StoreForm::OnWords`]), the
/// other lists of `lists` (those whose items it reads) guarded: `None` where the value names `target` other than as a
/// list it reads items of, or is no chain checked steps compute.
fn on_words(value: &Expr, target: Symbol, lists: &[Symbol], ecx: &super::expr::ExprCtx) -> Option<StoreForm> {
    if !read_as_words(value, target) {
        return None;
    }
    let chain = || match value {
        Expr::Index { .. } => {
            let word = super::expr::ExprCtx { int_exact_tolerant: false, ..*ecx };
            (super::types::infer_numeric_type(value, ecx.interner, ecx.variable_types) == "i64").then(|| {
                let read = super::expr::codegen_expr_ctx(value, &word);
                super::expr::CheckedChain {
                    binds: format!("let __chain0: i64 = {}; ", super::precedence::standalone(&read)),
                    checked: "Some(__chain0)".to_string(),
                    exact: "__chain0".to_string(),
                }
            })
        }
        _ => super::expr::checked_chain_parts(value, ecx, "__chain"),
    };
    let chain = if loop_view(target).is_some() { chain() } else { through_words(target, chain) }?;
    Some(StoreForm::OnWords {
        guards: lists.iter().filter(|l| **l != target && loop_view(**l).is_none()).map(|l| place(*l, ecx)).collect(),
        binds: chain.binds,
        checked: chain.checked,
        past: format!("logicaffeine_data::LogosInt::from({})", super::precedence::standalone(&chain.exact)),
        exact: exact_value(value, ecx),
    })
}

/// How `value` is stored into an `IntVec`. A value the emission gives as a machine word that cannot leave it is stored
/// as that word; a width-bounded chain as the 128-bit value it computes; a chain over items of lists that may have
/// promoted as that 128-bit value while every one of them holds words, else exactly; anything else as the exact integer
/// it is.
pub(crate) fn store(value: &Expr, target: Symbol, ecx: &super::expr::ExprCtx) -> StoreForm {
    use crate::ast::stmt::BinaryOpKind;
    let word = super::expr::ExprCtx { int_exact_tolerant: false, ..*ecx };
    let standalone = |code: String| super::precedence::standalone(&code).to_string();
    match item_lists(value, ecx.variable_types) {
        None => return StoreForm::Exact(exact_value(value, ecx)),
        Some(lists) if !lists.is_empty() => {
            if let Some(form) = on_words(value, target, &lists, ecx) {
                return form;
            }
            let fast = match value {
                Expr::Index { .. } => {
                    Some(format!("i128::from({})", standalone(super::expr::codegen_expr_ctx(value, &word))))
                }
                _ => super::expr::wide_chain(value, ecx),
            };
            let exact = exact_value(value, ecx);
            return match fast {
                Some(wide) => StoreForm::Guarded {
                    lists: lists.iter().filter(|l| loop_view(**l).is_none()).map(|l| place(*l, ecx)).collect(),
                    wide: standalone(wide),
                    exact,
                },
                None => StoreForm::Exact(exact),
            };
        }
        Some(_) => {}
    }
    let int = |e: &Expr| super::types::infer_numeric_type(e, ecx.interner, ecx.variable_types) == "i64";
    let stays_word = match value {
        Expr::Literal(crate::ast::stmt::Literal::Number(_)) => true,
        Expr::Identifier(sym) => {
            int(value) && !ecx.variable_types.get(sym).is_some_and(|t| t.contains("__bigint"))
        }
        // Checked arithmetic the oracle keeps in the word, and a bit operation on words, which never leaves one. A
        // power is neither: `2 ** 70` is past the word.
        Expr::BinaryOp {
            op:
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Divide
                | BinaryOpKind::Modulo
                | BinaryOpKind::BitAnd
                | BinaryOpKind::BitOr
                | BinaryOpKind::BitXor
                | BinaryOpKind::Shl
                | BinaryOpKind::Shr,
            ..
        } => int(value) && super::expr::stays_word(value, ecx),
        _ => false,
    };
    if stays_word {
        StoreForm::Word(standalone(super::expr::codegen_expr_ctx(value, &word)))
    } else if let Some(form) = on_words(value, target, &[], ecx) {
        form
    } else if let Some(wide) = super::expr::wide_chain(value, ecx) {
        StoreForm::Wide(standalone(wide))
    } else {
        StoreForm::Exact(exact_value(value, ecx))
    }
}

/// `form` stored by `method` (`push` or `set_item`) into `target`, with `at` its index where the method takes one.
fn stored(form: StoreForm, target: &str, method: &str, at: Option<&str>) -> String {
    // `set` writes at an index the oracle proves names an item, the proof a `Vec<i64>` write takes unchecked on.
    let call = |kind: &str, v: &str| match at {
        Some(at) if method == "set" => format!("unsafe {{ {target}.set_{kind}_unchecked({at}, {v}) }}"),
        Some(at) => format!("{target}.{method}_{kind}({at}, {v})"),
        None => format!("{target}.{method}_{kind}({v})"),
    };
    match form {
        StoreForm::Word(v) => format!("{{ let __stored: i64 = {v}; {}; }}", call("word", "__stored")),
        StoreForm::Wide(v) => format!("{{ let __stored: i128 = {v}; {}; }}", call("i128", "__stored")),
        StoreForm::Exact(v) => format!("{{ let __stored: logicaffeine_data::LogosInt = {v}; {}; }}", call("int", "__stored")),
        StoreForm::Guarded { lists, wide, exact } => {
            let words: Vec<String> = lists.iter().map(|l| format!("!{l}.is_exact()")).collect();
            format!(
                "if {} {{ let __stored: i128 = {wide}; {}; }} else {{ let __stored: logicaffeine_data::LogosInt = {exact}; {}; }}",
                words.join(" && "),
                call("i128", "__stored"),
                call("int", "__stored")
            )
        }
        StoreForm::OnWords { guards, binds, checked, past, exact } => {
            let held = match (method, at) {
                ("push", _) => format!("{WORDS}.push(__v)"),
                ("set", Some(at)) => format!("unsafe {{ *{WORDS}.get_unchecked_mut({at}) = __v }}"),
                (_, at) => format!("*{WORDS}.logos_item_mut({}) = __v", at.expect("a `Set item` names its item")),
            };
            let words: Vec<String> = guards.iter().map(|g| format!("!{g}.is_exact()")).collect();
            let all_words = if words.is_empty() { String::new() } else { format!(" if {}", words.join(" && ")) };
            // A step past the word runs out of line: inlined, its big-integer code crowds the loop it stores in.
            format!(
                "{{ let __past: Option<logicaffeine_data::LogosInt> = match {target}.words_mut() {{ \
                 Some({WORDS}){all_words} => {{ {binds}match {checked} {{ Some(__v) => {{ {held}; None }} \
                 None => Some(logos_cold_path(move || {past})) }} }} _ => Some({exact}) }}; \
                 if let Some(__x) = __past {{ {}; }} }}",
                call("int", "__x")
            )
        }
    }
}

/// Where a store into a list a versioned loop runs on the words of finds a value past the word: the pass is finished
/// on the exact integers ([`finish_pass`] writes it).
const PASS_ON_EXACT: &str = "/*@intvec-finish-pass*/";

/// `form`, or the literal `step` of an item advanced in place, stored by `method` (`push`, `set` or `set_item`, `at`
/// its index) into `target` while a versioned loop holds `words`, the view of its words. A word goes in through the
/// view; a value past the word is stored exactly into `target`, and the pass finishes on the exact integers
/// ([`PASS_ON_EXACT`]): the view is not read again.
fn on_loop_words(
    form: StoreForm,
    step: Option<i64>,
    words: &str,
    target: &str,
    method: &str,
    at: Option<&str>,
) -> String {
    let put = |v: &str| match (method, at) {
        ("push", _) => format!("{words}.push({v})"),
        ("set", Some(at)) => format!("unsafe {{ *{words}.get_unchecked_mut({at}) = {v} }}"),
        (_, at) => format!("*{words}.logos_item_mut({}) = {v}", at.expect("a `Set item` names its item")),
    };
    let exactly = match (method, at) {
        ("push", _) => format!("{target}.push_int(__x)"),
        ("set", Some(at)) => format!("unsafe {{ {target}.set_int_unchecked({at}, __x) }}"),
        (_, at) => format!("{target}.set_item_int({}, __x)", at.expect("a `Set item` names its item")),
    };
    let finish = |past: String| {
        format!("{{ let __past: Option<logicaffeine_data::LogosInt> = {past}; if let Some(__x) = __past {{ {exactly}; {PASS_ON_EXACT} }} }}")
    };
    let exact_or_put = |exact: &str| {
        format!("{{ let __x: logicaffeine_data::LogosInt = {exact}; match __x.to_i64() {{ Some(__v) => {{ {}; None }} None => Some(__x) }} }}", put("__v"))
    };
    if let Some(step) = step {
        let slot = match (method, at) {
            ("set", Some(at)) => format!("unsafe {{ {words}.get_unchecked_mut({at}) }}"),
            (_, at) => format!("{words}.logos_item_mut({})", at.expect("a `Set item` names its item")),
        };
        return finish(format!(
            "{{ let __s: &mut i64 = {slot}; match __s.checked_add({step}i64) {{ Some(__v) => {{ *__s = __v; None }} None => Some(logicaffeine_data::LogosInt::from_i128(i128::from(*__s) + {step}i128)) }} }}"
        ));
    }
    match form {
        StoreForm::Word(v) => format!("{{ let __stored: i64 = {v}; {}; }}", put("__stored")),
        StoreForm::OnWords { guards, binds, checked, past, exact } => {
            let words_path = format!(
                "{{ {binds}match {checked} {{ Some(__v) => {{ {}; None }} None => Some(logos_cold_path(move || {past})) }} }}",
                put("__v")
            );
            if guards.is_empty() {
                finish(words_path)
            } else {
                let held: Vec<String> = guards.iter().map(|g| format!("!{g}.is_exact()")).collect();
                finish(format!("if {} {words_path} else {}", held.join(" && "), exact_or_put(&exact)))
            }
        }
        StoreForm::Wide(wide) => finish(format!(
            "{{ let __wide: i128 = {wide}; match i64::try_from(__wide) {{ Ok(__v) => {{ {}; None }} Err(_) => Some(logicaffeine_data::LogosInt::from_i128(__wide)) }} }}",
            put("__v")
        )),
        StoreForm::Guarded { lists, wide, exact } => {
            let unviewed: Vec<String> = lists.iter().map(|l| format!("!{l}.is_exact()")).collect();
            let wide_path = format!(
                "{{ let __wide: i128 = {wide}; match i64::try_from(__wide) {{ Ok(__v) => {{ {}; None }} Err(_) => Some(logicaffeine_data::LogosInt::from_i128(__wide)) }} }}",
                put("__v")
            );
            if unviewed.is_empty() {
                finish(wide_path)
            } else {
                finish(format!("if {} {wide_path} else {}", unviewed.join(" && "), exact_or_put(&exact)))
            }
        }
        StoreForm::Exact(exact) => finish(exact_or_put(&exact)),
    }
}

/// A `While` loop the emission runs on the words of the `IntVec`s it touches while they hold them ([`versioned_while`]).
pub(crate) struct IntVecLoop<'a> {
    /// The loop, alive for the whole emission of its versions.
    stmt: *const Stmt<'a>,
    /// The label its words version leaves by.
    label: String,
    /// The lists it runs on the words of.
    lists: Vec<Symbol>,
    /// Each store into one of them, by address: the statements that finish the pass after it, innermost block first.
    rests: HashMap<usize, Vec<&'a [Stmt<'a>]>>,
}

/// `code`, a store into `list` emitted while a versioned loop runs on its words, with the pass finished on the exact
/// integers where it finds a value past the word: the statements after the store in the pass, then the loop again,
/// each read on its exact integers, and the words version left.
fn finish_pass(code: String, store: &Stmt, list: Symbol, cx: &mut StmtCx) -> Option<String> {
    if !code.contains(PASS_ON_EXACT) {
        return Some(code);
    }
    let address = store as *const Stmt as usize;
    let at = cx.ctx.intvec_loops().iter().rposition(|l| l.lists.contains(&list))?;
    let (stmt, label, rests) = {
        let l = &cx.ctx.intvec_loops()[at];
        (l.stmt, l.label.clone(), l.rests.get(&address)?.clone())
    };
    let frame = LOOP_VIEWS.with(|v| {
        let mut v = v.borrow_mut();
        let at = v.iter().rposition(|(l, _)| *l == label)?;
        Some((at, v.remove(at)))
    })?;
    CONTINUING.with(|c| c.set(c.get() + 1));
    let indent = cx.indent_level + 1;
    let mut pass = String::from("\n");
    for rest in &rests {
        for s in rest.iter() {
            pass.push_str(&super::codegen_stmt(
                s,
                cx.interner,
                indent,
                cx.mutable_vars,
                cx.ctx,
                cx.lww_fields,
                cx.mv_fields,
                cx.synced_vars,
                cx.var_caps,
                cx.async_functions,
                cx.pipe_vars,
                cx.boxed_fields,
                cx.registry,
                cx.type_env,
            ));
        }
    }
    // SAFETY: the loop's emission is under way below this store's: the loop outlives it.
    let whole = unsafe { &*stmt };
    pass.push_str(&super::codegen_stmt(
        whole,
        cx.interner,
        indent,
        cx.mutable_vars,
        cx.ctx,
        cx.lww_fields,
        cx.mv_fields,
        cx.synced_vars,
        cx.var_caps,
        cx.async_functions,
        cx.pipe_vars,
        cx.boxed_fields,
        cx.registry,
        cx.type_env,
    ));
    pass.push_str(&format!("{}break {label};\n{}", "    ".repeat(indent), "    ".repeat(cx.indent_level)));
    CONTINUING.with(|c| c.set(c.get() - 1));
    LOOP_VIEWS.with(|v| v.borrow_mut().insert(frame.0, frame.1));
    Some(code.replace(PASS_ON_EXACT, &pass))
}

/// The `IntVec`s held at its entry that a `While` loop with `cond` and `body` touches, if it can run on their words:
/// each with whether it stores into it. Every mention of such a list is an item read, a `length of`, or a store; no loop nested in the body
/// and no closure mentions one; and every store into one is reached through the body's blocks and `If`s alone, so the
/// rest of its pass is known. `None` where it touches none, or one otherwise.
fn loop_lists(cond: &Expr, body: &[Stmt], types: &HashMap<Symbol, String>) -> Option<Vec<(Symbol, bool)>> {
    // A list the body binds is no list the loop is entered with: a view is taken of the one held at entry.
    let mut bound: Vec<Symbol> = Vec::new();
    walk_nodes(body, |node| {
        if let Visited::Stmt(Stmt::Let { var, .. }) = node {
            bound.push(*var);
        }
        Walk::Into
    });
    let held = |s: &Symbol| !bound.contains(s) && types.get(s).is_some_and(|t| holds_int_vec(t));
    let mut mentioned: Vec<Symbol> = Vec::new();
    let mut mention = |s: Symbol| {
        if held(&s) && !mentioned.contains(&s) {
            mentioned.push(s);
        }
    };
    walk_expr_nodes(cond, &mut |e| {
        if let Expr::Identifier(s) = e {
            mention(*s)
        }
    });
    walk_nodes(body, |node| {
        if let Visited::Expr(Expr::Identifier(s)) = node {
            mention(*s);
        }
        Walk::Into
    });
    let mut lists = Vec::new();
    for list in mentioned {
        // Each mention of the list, and those an item read, a `length of` or a store makes of it.
        let (mut all, mut allowed, mut stores, mut refused) = (0usize, 0usize, 0usize, false);
        let mut count = |e: &Expr| match e {
            Expr::Identifier(s) if *s == list => all += 1,
            Expr::Index { collection: Expr::Identifier(s), .. } | Expr::Length { collection: Expr::Identifier(s) }
                if *s == list =>
            {
                allowed += 1
            }
            _ => {}
        };
        walk_expr_nodes(cond, &mut count);
        walk_nodes(body, |node| {
            match node {
                Visited::Expr(e @ Expr::Closure { .. }) => {
                    refused |= crate::optimize::expr_touches(e, list);
                    return Walk::Past;
                }
                Visited::Stmt(s @ (Stmt::While { .. } | Stmt::Repeat { .. } | Stmt::FunctionDef { .. })) => {
                    refused |= crate::optimize::stmt_touches(s, list);
                    return Walk::Past;
                }
                // A list the body sets anew is no list a view of it can hold.
                Visited::Stmt(Stmt::Set { target, .. }) if *target == list => refused = true,
                Visited::Stmt(Stmt::Swap { a, b }) if *a == list || *b == list => refused = true,
                Visited::Stmt(
                    Stmt::Push { collection: Expr::Identifier(s), .. } | Stmt::SetIndex { collection: Expr::Identifier(s), .. },
                ) if *s == list => stores += 1,
                Visited::Expr(e) => count(e),
                _ => {}
            }
            Walk::Into
        });
        if refused || all != allowed + stores {
            return None;
        }
        lists.push((list, stores > 0));
    }
    if lists.is_empty() {
        return None;
    }
    lists.sort_by_key(|(l, _)| l.index());
    Some(lists)
}

/// Every node of `e`, outermost first.
fn walk_expr_nodes(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    for child in logicaffeine_language::ast_depth::expr_children(e) {
        walk_expr_nodes(child, f);
    }
}

/// For each store into one of `lists` in `body`, by address, the statements that finish its pass: the rest of its
/// block, then of each block that holds it, out to the body. `None` where a store sits where the blocks and `If`s of the
/// body do not reach (the rest of its pass is not known).
fn rests_of<'a>(body: &'a [Stmt<'a>], lists: &[Symbol]) -> Option<HashMap<usize, Vec<&'a [Stmt<'a>]>>> {
    fn walk<'a>(
        block: &'a [Stmt<'a>],
        outer: &[&'a [Stmt<'a>]],
        lists: &[Symbol],
        out: &mut HashMap<usize, Vec<&'a [Stmt<'a>]>>,
    ) {
        for (at, s) in block.iter().enumerate() {
            let mut rest = vec![&block[at + 1..]];
            rest.extend_from_slice(outer);
            match s {
                Stmt::Push { collection: Expr::Identifier(l), .. } | Stmt::SetIndex { collection: Expr::Identifier(l), .. }
                    if lists.contains(l) =>
                {
                    out.insert(s as *const Stmt as usize, rest);
                }
                Stmt::If { then_block, else_block, .. } => {
                    walk(then_block, &rest, lists, out);
                    if let Some(else_block) = else_block {
                        walk(else_block, &rest, lists, out);
                    }
                }
                Stmt::Inspect { arms, .. } => {
                    for arm in arms {
                        walk(arm.body, &rest, lists, out);
                    }
                }
                Stmt::Zone { body, .. } => walk(body, &rest, lists, out),
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    walk(body, &[], lists, &mut out);
    let mut every = 0;
    walk_nodes(body, |node| {
        if let Visited::Stmt(
            Stmt::Push { collection: Expr::Identifier(l), .. } | Stmt::SetIndex { collection: Expr::Identifier(l), .. },
        ) = node
        {
            every += lists.contains(l) as usize;
        }
        Walk::Into
    });
    (every == out.len()).then_some(out)
}

/// The most stores a versioned loop finishes its pass after: each writes the loop once more, on the exact integers.
const MOST_STORES: usize = 8;

/// The `While` statement `stmt` run on the words of the `IntVec`s it touches while they hold words: a words version, each
/// list read and written through a view of its words (one check of each list's form for the whole loop, where each
/// access made its own), and the loop as written for a list already past the word. A store that finds a value past
/// the word stores it exactly and finishes its pass, and the loop, on the exact integers ([`finish_pass`]). `emit`
/// writes a statement at an indent. `None` where the loop cannot run on words ([`loop_lists`]), and for the loop's
/// own versions, a continuation's cold code included.
pub(crate) fn versioned_while<'a>(
    stmt: &Stmt<'a>,
    indent: usize,
    ctx: &mut super::context::RefinementContext<'a>,
    interner: &Interner,
    emit: &mut dyn FnMut(&Stmt<'a>, usize, &mut super::context::RefinementContext<'a>) -> String,
) -> Option<String> {
    let Stmt::While { cond, body, .. } = stmt else { return None };
    let address = stmt as *const Stmt as *const Stmt<'a>;
    if CONTINUING.with(|c| c.get()) > 0 || ctx.intvec_loops().iter().any(|l| std::ptr::eq(l.stmt, address)) {
        return None;
    }
    let types = ctx.get_variable_types();
    let lists = loop_lists(cond, body, types)?;
    let names = crate::analysis::types::RustNames::new(interner);
    let place_of = |l: Symbol| {
        let name = names.ident(l);
        if types.get(&l).is_some_and(|t| t.starts_with('&')) { format!("(*{name})") } else { name }
    };
    let held: Vec<Symbol> = lists.iter().map(|(l, _)| *l).collect();
    let rests = rests_of(body, &held)?;
    if rests.len() > MOST_STORES {
        return None;
    }
    let views: Vec<String> =
        lists.iter().map(|(l, stores)| format!("{}.{}()", place_of(*l), if *stores { "words_mut" } else { "words" })).collect();
    let n = LOOP_COUNT.with(|c| {
        let n = c.get();
        c.set(n + 1);
        n
    });
    let label = format!("'__words{n}");
    let frame: Vec<(Symbol, String)> =
        held.iter().map(|l| (*l, format!("__wv{n}_{}", interner.resolve(*l)))).collect();
    let (pattern, scrutinee) = match frame.as_slice() {
        [(_, view)] => (format!("Some({view})"), views[0].clone()),
        _ => (
            format!("({})", frame.iter().map(|(_, v)| format!("Some({v})")).collect::<Vec<_>>().join(", ")),
            format!("({})", views.join(", ")),
        ),
    };
    // SAFETY of `stmt` in the record: it is read only while this emission, below which the loop's stores are written,
    // is under way.
    ctx.intvec_loops().push(IntVecLoop { stmt: address, label: label.clone(), lists: held, rests });
    let hints = ctx.hints();
    LOOP_VIEWS.with(|v| v.borrow_mut().push((label.clone(), frame)));
    let words = emit(stmt, indent + 2, ctx);
    LOOP_VIEWS.with(|v| v.borrow_mut().pop());
    ctx.restore_hints(hints.clone());
    let exact = emit(stmt, indent + 1, ctx);
    ctx.restore_hints(hints);
    ctx.intvec_loops().pop();
    let ind = "    ".repeat(indent);
    Some(format!(
        "{ind}{label}: {{\n{ind}    if let {pattern} = {scrutinee} {{\n{words}{ind}        break {label};\n{ind}    }}\n{exact}{ind}}}\n"
    ))
}

/// The counted loop at `idx` of `stmts` (its counter's `Let` or `Set`, then the `While`), which the for-range pattern
/// writes, run on the words of the `IntVec`s it reads while they hold words: a words version reading each through a
/// shared view of its words (bounds hints and all, as a list of words has), and the loop as written for a list past
/// the word. A loop that stores into one is left to its per-access forms; so is one whose counter, a `Let` of the
/// window's own, is read after it (both versions sit in one block). `emit` writes the window at an indent.
pub(crate) fn versioned_window<'a>(
    stmts: &[&Stmt<'a>],
    idx: usize,
    counter: Symbol,
    ctx: &mut super::context::RefinementContext<'a>,
    interner: &Interner,
    indent: usize,
    emit: &mut dyn FnMut(usize, &mut super::context::RefinementContext<'a>) -> Option<(String, usize)>,
) -> Option<String> {
    let w = *stmts.get(idx + 1)?;
    let Stmt::While { cond, body, .. } = w else { return None };
    let address = w as *const Stmt as *const Stmt<'a>;
    if CONTINUING.with(|c| c.get()) > 0 || ctx.intvec_loops().iter().any(|l| std::ptr::eq(l.stmt, address)) {
        return None;
    }
    let types = ctx.get_variable_types();
    let lists = loop_lists(cond, body, types)?;
    if lists.iter().any(|(_, stores)| *stores) {
        return None;
    }
    if matches!(stmts[idx], Stmt::Let { .. }) && stmts[idx + 2..].iter().any(|s| crate::optimize::stmt_touches(s, counter)) {
        return None;
    }
    let names = crate::analysis::types::RustNames::new(interner);
    let place_of = |l: Symbol| {
        let name = names.ident(l);
        if types.get(&l).is_some_and(|t| t.starts_with('&')) { format!("(*{name})") } else { name }
    };
    let views: Vec<String> = lists.iter().map(|(l, _)| format!("{}.words()", place_of(*l))).collect();
    let held: Vec<Symbol> = lists.iter().map(|(l, _)| *l).collect();
    let n = LOOP_COUNT.with(|c| {
        let n = c.get();
        c.set(n + 1);
        n
    });
    let label = format!("'__words{n}");
    let frame: Vec<(Symbol, String)> = held.iter().map(|l| (*l, format!("__wv{n}_{}", interner.resolve(*l)))).collect();
    let (pattern, scrutinee) = match frame.as_slice() {
        [(_, view)] => (format!("Some({view})"), views[0].clone()),
        _ => (
            format!("({})", frame.iter().map(|(_, v)| format!("Some({v})")).collect::<Vec<_>>().join(", ")),
            format!("({})", views.join(", ")),
        ),
    };
    ctx.intvec_loops().push(IntVecLoop { stmt: address, label: label.clone(), lists: held, rests: HashMap::new() });
    let hints = ctx.hints();
    LOOP_VIEWS.with(|v| v.borrow_mut().push((label.clone(), frame)));
    let words = emit(indent + 2, ctx);
    LOOP_VIEWS.with(|v| v.borrow_mut().pop());
    ctx.restore_hints(hints.clone());
    let exact = words.as_ref().and_then(|_| emit(indent + 1, ctx));
    ctx.restore_hints(hints);
    ctx.intvec_loops().pop();
    let ((words, 1), (exact, 1)) = (words?, exact?) else { return None };
    let ind = "    ".repeat(indent);
    Some(format!(
        "{ind}{label}: {{\n{ind}    if let {pattern} = {scrutinee} {{\n{words}{ind}        break {label};\n{ind}    }}\n{exact}{ind}}}\n"
    ))
}

/// What a statement over an `IntVec` is emitted with.
pub(crate) struct StmtCx<'c, 'a> {
    pub interner: &'c Interner,
    pub indent: &'c str,
    pub indent_level: usize,
    pub mutable_vars: &'c HashSet<Symbol>,
    pub ctx: &'c mut super::context::RefinementContext<'a>,
    pub lww_fields: &'c HashSet<(String, String)>,
    pub mv_fields: &'c HashSet<(String, String)>,
    pub synced_vars: &'c mut HashSet<Symbol>,
    pub var_caps: &'c HashMap<Symbol, super::context::VariableCapabilities>,
    pub async_functions: &'c HashSet<Symbol>,
    pub pipe_vars: &'c HashSet<Symbol>,
    pub boxed_fields: &'c HashSet<(String, String, String)>,
    pub registry: &'c crate::analysis::TypeRegistry,
    pub type_env: &'c crate::analysis::types::TypeEnv,
}

/// Whether the binding `sym` holds an `IntVec` in the body being emitted.
fn held(sym: Symbol, variable_types: &HashMap<Symbol, String>) -> bool {
    variable_types.get(&sym).is_some_and(|t| holds_int_vec(t))
}

/// The statement `stmt` where it reads or writes an `IntVec` as a whole list: a binding, an assignment, a `Push`, a
/// `Set item`, a `Swap`, a `Show` of one. `None` for any other statement.
pub(crate) fn emit_stmt(stmt: &Stmt, cx: &mut StmtCx) -> Option<String> {
    let indent = cx.indent;
    let names = crate::analysis::types::RustNames::new(cx.interner);
    let types = cx.ctx.get_variable_types();
    let ecx = super::expr::ExprCtx {
        boxed_fields: cx.boxed_fields,
        registry: cx.registry,
        async_functions: cx.async_functions,
        string_vars: cx.ctx.get_string_vars(),
        variable_types: types,
        fast_div: cx.ctx.get_fast_div(),
        oracle: cx.ctx.oracle(),
        ..super::expr::ExprCtx::bare(cx.interner, &*cx.synced_vars)
    };
    let target_place = |sym: Symbol| place(sym, &ecx);
    let code = match stmt {
        // A binding of a list a parameter lends is the lend itself, as a lent machine-word list's is: the body changes
        // the caller's list in place.
        Stmt::Let { var, value: Expr::Identifier(lent), .. }
            if promotes_here(*var) && types.get(lent).is_some_and(|t| t.starts_with('&') && holds_int_vec(t)) =>
        {
            let ty = types[lent].clone();
            let line = format!("{indent}let {}: {ty} = {};\n", names.ident(*var), names.ident(*lent));
            cx.ctx.register_variable_type(*var, ty);
            line
        }
        Stmt::Let { var, value, mutable, .. } if promotes_here(*var) => {
            let init = match value {
                Expr::New { .. } => match cx.ctx.get_vec_presize(*var) {
                    Some(capacity) => with_room(capacity),
                    None => format!("{INT_VEC}::new()"),
                },
                _ => owned(value, &ecx),
            };
            let kw = if *mutable || cx.mutable_vars.contains(var) {
                format!("let {}", super::tentative::MAYBE_MUT.trim_end())
            } else {
                "let".to_string()
            };
            let line = format!("{indent}{kw} {}: {INT_VEC} = {init};\n", names.ident(*var));
            let bound = *var;
            cx.ctx.register_variable_type(bound, INT_VEC.to_string());
            line
        }
        Stmt::Set { target, value } if held(*target, types) => {
            if let Expr::Call { function, args } = value {
                let in_place = super::fn_role_indices(*function, super::FnRole::MutBorrow);
                if args.iter().enumerate().any(|(i, a)| in_place.contains(&i) && matches!(a, Expr::Identifier(s) if s == target)) {
                    return Some(format!("{indent}{};\n", super::expr::codegen_expr_ctx(value, &ecx)));
                }
            }
            format!("{indent}{} = {};\n", target_place(*target), owned(value, &ecx))
        }
        Stmt::Push { value, collection: Expr::Identifier(t) } if held(*t, types) && loop_view(*t).is_some() => {
            let words = loop_view(*t).expect("a loop holds the view");
            let form = store(value, *t, &ecx);
            let code = on_loop_words(form, None, &words, &target_place(*t), "push", None);
            return finish_pass(format!("{indent}{code}\n"), stmt, *t, cx);
        }
        Stmt::Push { value, collection: Expr::Identifier(t) } if held(*t, types) => {
            format!("{indent}{}\n", stored(store(value, *t, &ecx), &target_place(*t), "push", None))
        }
        Stmt::SetIndex { collection: collection @ Expr::Identifier(t), index, value } if held(*t, types) => {
            let at_ecx = super::expr::ExprCtx { int_index_context: true, ..ecx };
            let proven = super::expr::oracle_proves_index(&ecx, collection, index)
                || super::expr::range_proves_index(collection, index);
            let (at, at_ty, method) = if proven {
                (super::expr::proven_offset_str(index, &at_ecx, types, cx.interner), "usize", "set")
            } else {
                (super::expr::logos_index_str(index, &at_ecx, types, cx.interner), "i64", "set_item")
            };
            if let Some(words) = loop_view(*t) {
                let form = store(value, *t, &ecx);
                let step = step_in_place(*t, index, value);
                let code = on_loop_words(form, step, &words, &target_place(*t), method, Some("__at"));
                return finish_pass(
                    format!("{indent}{{ let __at: {at_ty} = {}; {code} }}\n", super::precedence::standalone(&at)),
                    stmt,
                    *t,
                    cx,
                );
            }
            // An item advanced by a literal is updated in place: its word while the list holds words, the 128-bit
            // value where the step leaves the word, the exact integer once the list has promoted.
            if let Some(step) = step_in_place(*t, index, value) {
                let l = target_place(*t);
                let (slot, exact_set, wide_set) = if proven {
                    (
                        format!("unsafe {{ {l}.word_mut_unchecked(__at) }}"),
                        format!("unsafe {{ {l}.set_int_unchecked(__at, __stored) }}"),
                        format!("unsafe {{ {l}.set_i128_unchecked(__at, __x) }}"),
                    )
                } else {
                    (
                        format!("{l}.item_word_mut(__at)"),
                        format!("{l}.set_item_int(__at, __stored)"),
                        format!("{l}.set_item_i128(__at, __x)"),
                    )
                };
                return Some(format!(
                    "{indent}{{ let __at: {at_ty} = {}; let __past: Option<i128> = match {slot} {{ Some(__w) => match __w.checked_add({step}i64) {{ Some(__v) => {{ *__w = __v; None }} None => Some(i128::from(*__w) + {step}i128) }}, None => {{ let __stored: logicaffeine_data::LogosInt = {}; {exact_set}; None }} }}; if let Some(__x) = __past {{ {wide_set}; }} }}\n",
                    super::precedence::standalone(&at),
                    exact_value(value, &ecx)
                ));
            }
            let form = store(value, *t, &ecx);
            format!(
                "{indent}{{ let __at: {at_ty} = {}; {} }}\n",
                super::precedence::standalone(&at),
                stored(form, &target_place(*t), method, Some("__at"))
            )
        }
        Stmt::Swap { a, b } if held(*a, types) && held(*b, types) => {
            let lend = |sym: Symbol| {
                if types.get(&sym).is_some_and(|t| t.starts_with("&mut ")) {
                    names.ident(sym)
                } else {
                    format!("&mut {}", names.ident(sym))
                }
            };
            format!("{indent}std::mem::swap({}, {});\n", lend(*a), lend(*b))
        }
        Stmt::Show { object, recipient: Expr::Identifier(show) }
            if cx.interner.resolve(*show) == "show" && is_int_vec(object, types) =>
        {
            format!("{indent}show(&{});\n", receiver(object, &ecx))
        }
        // An `Int` computed from items of lists that may hold integers past the word is shown as the exact integer it
        // is.
        Stmt::Show { object, recipient: Expr::Identifier(show) }
            if cx.interner.resolve(*show) == "show"
                && item_lists(object, types).is_none_or(|lists| !lists.is_empty())
                && super::types::infer_numeric_type(object, cx.interner, types) == "i64" =>
        {
            format!("{indent}show(&{});\n", exact_value(object, &ecx))
        }
        _ => return None,
    };
    Some(code)
}

/// The value a function whose result is an `IntVec` returns: its own list moved out, any other the list it owns.
/// `None` where the body being emitted returns no `IntVec`, or lends its result in place.
pub(crate) fn returned(v: &Expr, ecx: &super::expr::ExprCtx) -> Option<String> {
    let owner = OWNER.with(|o| o.get())?;
    if !promotes_return(owner) || !super::fn_role_indices(owner, super::FnRole::MutBorrow).is_empty() {
        return None;
    }
    Some(match v {
        Expr::Identifier(sym) if ecx.variable_types.get(sym).is_some_and(|t| t == INT_VEC) => {
            crate::analysis::types::RustNames::new(ecx.interner).ident(*sym)
        }
        _ => owned(v, ecx),
    })
}

/// The argument `a` handed to parameter `i` of `function`, an `IntVec` parameter: lent to be written in place, lent
/// to be read, or given as the callee's own.
pub(crate) fn argument(function: Symbol, i: usize, a: &Expr, ecx: &super::expr::ExprCtx) -> String {
    let in_place = super::fn_role_indices(function, super::FnRole::MutBorrow).contains(&i)
        || super::fn_role_indices(function, super::FnRole::ValueMutable).contains(&i);
    let lent = super::fn_role_indices(function, super::FnRole::Borrow).contains(&i);
    let names = crate::analysis::types::RustNames::new(ecx.interner);
    match a {
        Expr::Identifier(sym) if in_place || lent => {
            let ty = ecx.variable_types.get(sym).map(String::as_str).unwrap_or("");
            match (in_place, ty.starts_with('&')) {
                (true, true) => format!("&mut *{}", names.ident(*sym)),
                (true, false) => format!("&mut {}", names.ident(*sym)),
                (false, true) => format!("&*{}", names.ident(*sym)),
                (false, false) => format!("&{}", names.ident(*sym)),
            }
        }
        _ if in_place => format!("&mut {}", owned(a, ecx)),
        _ if lent => format!("&{}", super::precedence::operand(&owned(a, ecx), super::precedence::Under::Prefix)),
        _ => owned(a, ecx),
    }
}
