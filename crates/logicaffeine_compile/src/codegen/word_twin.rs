//! The word twin of a pure function: the same body with every `Int` a machine word, which the function runs first.
//!
//! A pure function whose locals, parameters or result the exactness decision makes exact (`exact_scalars`) would pay
//! an exact operation at every step of every call. Its twin `f__w` is the same body on machine words, whose
//! arithmetic stops at the canonical overflow (`logicaffeine_data::ops::raise_overflow`); `f` runs the twin inside
//! `logicaffeine_system::failure::speculate` with its arguments as words, and returns what the twin returns. Where an
//! argument is no word, or the twin meets a value past the word anywhere in its calls, `f` runs its own exact body:
//! a pure call run again is the same call. A twin calls the twins of the functions it calls, so a recursion stays on
//! words until a value leaves the word, and a twin is never itself exact: the promotion facts skip it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use crate::ast::stmt::{Expr, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use logicaffeine_language::ast_map::{walk_expr, walk_stmt, Arenas, AstMap};

thread_local! {
    static TWIN_FIRST: RefCell<HashSet<Symbol>> = RefCell::new(HashSet::new());
    static TWINS: RefCell<HashMap<Symbol, Symbol>> = RefCell::new(HashMap::new());
    static TWIN_NAMES: RefCell<HashSet<Symbol>> = RefCell::new(HashSet::new());
    static TWIN_BODIES: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

/// The program's twins, installed for one compile; dropping it removes them.
pub(crate) struct Twins;

impl Drop for Twins {
    fn drop(&mut self) {
        TWINS.with(|t| t.borrow_mut().clear());
        TWIN_NAMES.with(|t| t.borrow_mut().clear());
        TWIN_BODIES.with(|t| t.borrow_mut().clear());
        TWIN_FIRST.with(|t| t.borrow_mut().clear());
    }
}

/// Record the twinned functions that hold an exact local, parameter or result: each runs its twin first, and only
/// their twins are emitted.
pub(crate) fn set_needed(functions: HashSet<Symbol>) {
    TWIN_FIRST.with(|n| *n.borrow_mut() = functions);
}

/// Whether `function` runs its twin first.
pub(crate) fn needed(function: Symbol) -> bool {
    TWIN_FIRST.with(|n| n.borrow().contains(&function))
}

/// Whether the twin `twin` is emitted: some function runs a twin, and twins call one another's, so every twin is
/// (a twin whose function holds nothing exact is that function on words, and is emitted unused where nothing calls it).
pub(crate) fn twin_emitted(twin: Symbol) -> bool {
    is_twin(twin) && TWIN_FIRST.with(|n| !n.borrow().is_empty())
}

/// The twin of `function`, where it has one.
pub(crate) fn twin_of(function: Symbol) -> Option<Symbol> {
    TWINS.with(|t| t.borrow().get(&function).copied())
}

/// Whether `function` is a twin.
pub(crate) fn is_twin(function: Symbol) -> bool {
    TWIN_NAMES.with(|t| t.borrow().contains(&function))
}

/// The function `twin` is the twin of.
pub(crate) fn origin_of(twin: Symbol) -> Option<Symbol> {
    TWINS.with(|t| t.borrow().iter().find(|(_, w)| **w == twin).map(|(f, _)| *f))
}

/// Whether a parameter of type `ty` is a machine value a twin takes as its function does: an `Int` (a word in the
/// twin), a `Bool`, a `Real`, a byte or a character.
fn scalar(ty: &TypeExpr, interner: &Interner) -> bool {
    matches!(ty, TypeExpr::Primitive(s) | TypeExpr::Named(s)
        if matches!(interner.resolve(*s), "Int" | "Nat" | "Bool" | "Real" | "Byte" | "Char"))
}

/// Whether `ty` is a `Seq of Int`: a list the twin takes and gives as machine words, its function as it holds it.
pub(super) fn int_list(ty: &TypeExpr, interner: &Interner) -> bool {
    matches!(ty, TypeExpr::Generic { base, params }
        if matches!(interner.resolve(*base), "Seq" | "List") && params.len() == 1
            && matches!(&params[0], TypeExpr::Primitive(s) | TypeExpr::Named(s) if interner.resolve(*s) == "Int"))
}

/// The functions of `stmts` that get a twin: user functions, not generic, every parameter a scalar or a `Seq of Int`
/// the body only reads, returning an `Int` or a `Seq of Int`, and pure (`detection::collect_pure_functions`). A twin
/// run again must see the lists it first saw, so it reads the lists it is handed; a function writing one holds it as
/// its `IntVec`, word-first already (`int_vec`). A function with a list among its parameters or its result calls no
/// user function but those that get a twin: its twin hands its lists on as words, which only a twin takes.
fn twinned(stmts: &[Stmt], interner: &Interner, holds_nothing: &HashSet<Symbol>) -> Vec<Symbol> {
    let pure = crate::optimize::purity::Purity::of_program_making_fresh_values(stmts, interner).into_set();
    let user: HashSet<Symbol> = stmts.iter().filter_map(|s| match s {
        Stmt::FunctionDef { name, is_native: false, .. } => Some(*name),
        _ => None,
    }).collect();
    let eligible = |s: &Stmt| match s {
        Stmt::FunctionDef { name, generics, params, body, return_type: Some(rt), is_native: false, is_exported: false, .. }
            if generics.is_empty()
                && pure.contains(name)
                && !holds_nothing.contains(name)
                && (matches!(rt, TypeExpr::Primitive(s) | TypeExpr::Named(s) if interner.resolve(*s) == "Int")
                    || int_list(rt, interner))
                && params.iter().all(|(_, t)| scalar(t, interner) || int_list(t, interner))
                && {
                    let written = super::detection::collect_mutable_vars(body);
                    params.iter().all(|(p, t)| !int_list(t, interner) || !written.contains(p))
                } =>
        {
            Some((*name, int_list(rt, interner) || params.iter().any(|(_, t)| int_list(t, interner))))
        }
        _ => None,
    };
    let candidates: Vec<(Symbol, bool)> = stmts.iter().filter_map(eligible).collect();
    // Dropping a function drops every function with lists that calls it, until none is dropped.
    let mut twinned: HashSet<Symbol> = candidates.iter().map(|(f, _)| *f).collect();
    loop {
        let kept: HashSet<Symbol> = candidates
            .iter()
            .filter(|(f, lists)| {
                twinned.contains(f)
                    && (!*lists
                        || stmts.iter().any(|s| matches!(s, Stmt::FunctionDef { name, body, .. } if name == f
                            && calls(body).iter().all(|c| !user.contains(c) || twinned.contains(c)))))
            })
            .map(|(f, _)| *f)
            .collect();
        if kept.len() == twinned.len() {
            break;
        }
        twinned = kept;
    }
    candidates.iter().map(|(f, _)| *f).filter(|f| twinned.contains(f)).collect()
}

/// Every function `body` calls, nested blocks and expressions included.
fn calls(body: &[Stmt]) -> Vec<Symbol> {
    use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};
    let mut out = Vec::new();
    walk_nodes(body, |node| {
        match node {
            Visited::Expr(Expr::Call { function, .. }) | Visited::Stmt(Stmt::Call { function, .. }) => out.push(*function),
            _ => {}
        }
        Walk::Into
    });
    out
}

/// A twin's body: every call to a twinned function a call to its twin.
struct Rename<'a> {
    arenas: Arenas<'a>,
    twins: HashMap<Symbol, Symbol>,
}

impl<'a> AstMap<'a> for Rename<'a> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        let mapped = walk_expr(self, e);
        match mapped {
            Expr::Call { function, args } if self.twins.contains_key(function) => {
                self.arenas.exprs.alloc(Expr::Call { function: self.twins[function], args: args.clone() })
            }
            _ => mapped,
        }
    }

    fn map_stmt(&mut self, s: &Stmt<'a>) -> Stmt<'a> {
        match walk_stmt(self, s) {
            Stmt::Call { function, args } if self.twins.contains_key(&function) => {
                Stmt::Call { function: self.twins[&function], args }
            }
            other => other,
        }
    }
}

/// Whether `stmts` has a function that gets a twin.
pub(crate) fn wants_twins(stmts: &[Stmt], interner: &Interner, holds_nothing: &HashSet<Symbol>) -> bool {
    !twinned(stmts, interner, holds_nothing).is_empty()
}

/// `stmts` with the twin of each pure function that gets one, after it; the twins installed for this compile.
pub(crate) fn add_twins<'a>(
    stmts: Vec<Stmt<'a>>,
    arenas: Arenas<'a>,
    interner: &mut Interner,
    holds_nothing: &HashSet<Symbol>,
) -> (Vec<Stmt<'a>>, Twins) {
    let names = twinned(&stmts, interner, holds_nothing);
    let twins: HashMap<Symbol, Symbol> =
        names.iter().map(|f| (*f, interner.intern(&format!("{}__w", interner.resolve(*f))))).collect();
    TWINS.with(|t| *t.borrow_mut() = twins.clone());
    TWIN_NAMES.with(|t| *t.borrow_mut() = twins.values().copied().collect());
    let mut rename = Rename { arenas, twins: twins.clone() };
    let mut out = Vec::with_capacity(stmts.len() + twins.len());
    for s in stmts {
        let twin = match &s {
            Stmt::FunctionDef { name, .. } if twins.contains_key(name) => match rename.map_stmt(&s) {
                Stmt::FunctionDef { generics, params, body, return_type, is_native, native_path, is_exported, export_target, opt_flags, .. } => {
                    TWIN_BODIES.with(|t| t.borrow_mut().insert(body.as_ptr() as usize));
                    Some(Stmt::FunctionDef {
                        name: twins[name],
                        generics,
                        params,
                        body,
                        return_type,
                        is_native,
                        native_path,
                        is_exported,
                        export_target,
                        opt_flags,
                    })
                }
                _ => None,
            },
            _ => None,
        };
        out.push(s);
        out.extend(twin);
    }
    (out, Twins)
}

/// Whether `body` is a twin's: its `Int`s are machine words, never exact.
pub(crate) fn is_twin_body(body: &[Stmt]) -> bool {
    TWIN_BODIES.with(|t| t.borrow().contains(&(body.as_ptr() as usize)))
}

/// Whether a parameter its function holds as the Rust type `held` reaches the twin by one of its conventions: an exact
/// `Int` the prologue unboxes, a list of `Int` in a [`WordList`] form, or a machine scalar handed on as it is. A value
/// of the exact extension, a value-or-nothing or anything else keeps the function on its own body.
pub(super) fn held_for_a_twin(held: &str, exact: bool, list: bool) -> bool {
    if exact {
        return held == "logicaffeine_data::LogosInt";
    }
    if list {
        let words = held.strip_prefix("&'a ").or_else(|| held.strip_prefix('&')).unwrap_or(held);
        let int_vec = held.trim_start_matches("&mut ").trim_start_matches('&') == super::int_vec::INT_VEC;
        return int_vec
            || matches!(words, "[i64]" | "LogosSeq<i64>" | "Vec<i64>")
            || (words.starts_with("[i64; ") && held.starts_with('&'));
    }
    matches!(held, "i64" | "i32" | "u64" | "u8" | "f64" | "bool" | "char")
}

/// The form a function of the Rust tier holds a `Seq of Int` parameter in, or gives a `Seq of Int` result in: what a
/// function's prologue converts between, handing its lists to its twin and taking the twin's list back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum WordList {
    /// An `IntVec`, owned or lent: words while every item is one, exact past that.
    Exact,
    /// Lent words: `&[i64]`, `&[i64; N]` or `&'a [i64]`.
    Lent,
    /// A shared `LogosSeq<i64>`.
    Shared,
    /// An owned `Vec<i64>`, a result.
    Owned,
    /// A stack array `[i64; N]`, a result.
    Fixed,
    /// Words or an `IntVec` written in place (`&mut`): the caller's own list, which a run of the twin cut short would
    /// leave part-written, so no twin is run on it.
    Written,
}

impl WordList {
    /// The argument a twin taking its list in form `to` is handed for the parameter `n` its function holds in form
    /// `from`, an `Exact` one unboxed to its words as `__w_n`; none where either writes the list in place.
    pub(super) fn hand(from: WordList, to: WordList, n: &str) -> Option<String> {
        let words = if from == WordList::Exact { format!("__w_{}", n.trim_start_matches("r#")) } else { n.to_string() };
        Some(match (from, to) {
            (WordList::Written, _) | (_, WordList::Written) => return None,
            (WordList::Shared, WordList::Shared) => format!("{n}.clone()"),
            (WordList::Shared, WordList::Lent) => format!("&{n}.borrow()[..]"),
            (_, WordList::Lent) => format!("&{words}[..]"),
            _ => format!("logicaffeine_data::LogosSeq::from_vec({words}.to_vec())"),
        })
    }

    /// The result in form `to` for the list `__w` a twin gives in form `from`, where one form becomes the other.
    pub(super) fn give(from: WordList, to: WordList) -> Option<String> {
        let words = match from {
            WordList::Owned => "__w".to_string(),
            WordList::Fixed => "Vec::from(__w)".to_string(),
            WordList::Lent => "__w.to_vec()".to_string(),
            WordList::Shared => "__w.into_vec()".to_string(),
            WordList::Exact | WordList::Written => return None,
        };
        match to {
            _ if to == from => Some("__w".to_string()),
            WordList::Exact => Some(format!("{}::from_words({words})", super::int_vec::INT_VEC)),
            WordList::Owned => Some(words),
            WordList::Shared => Some(format!("logicaffeine_data::LogosSeq::from_vec({words})")),
            WordList::Lent | WordList::Fixed | WordList::Written => None,
        }
    }
}

/// The program's lowering decisions a [`WordList`] is read from, as `program::codegen_function_def` applies them.
pub(super) struct WordListForms<'m> {
    pub(super) borrow: &'m HashMap<Symbol, HashSet<usize>>,
    pub(super) mut_borrow: &'m HashMap<Symbol, HashSet<usize>>,
    pub(super) vec_return: &'m HashSet<Symbol>,
    pub(super) byte_slice_return: &'m HashMap<Symbol, usize>,
    pub(super) array_return: &'m HashMap<Symbol, super::affine_array::ArrayReturnInfo>,
}

impl WordListForms<'_> {
    /// The form `function` takes its `Seq of Int` parameter `index` in.
    pub(super) fn param(&self, function: Symbol, index: usize) -> WordList {
        if self.mut_borrow.get(&function).is_some_and(|at| at.contains(&index)) {
            WordList::Written
        } else if super::int_vec::promotes_param(function, index) {
            WordList::Exact
        } else if self.borrow.get(&function).is_some_and(|at| at.contains(&index))
            || self.byte_slice_return.get(&function) == Some(&index)
        {
            WordList::Lent
        } else {
            WordList::Shared
        }
    }

    /// The form `function` gives its `Seq of Int` result in: a function writing a list in place gives it back there.
    pub(super) fn result(&self, function: Symbol) -> WordList {
        if self.mut_borrow.contains_key(&function) {
            WordList::Written
        } else if self.byte_slice_return.contains_key(&function) {
            WordList::Lent
        } else if super::int_vec::promotes_return(function) {
            WordList::Exact
        } else if self.array_return.contains_key(&function) {
            WordList::Fixed
        } else if self.vec_return.contains(&function) {
            WordList::Owned
        } else {
            WordList::Shared
        }
    }
}
