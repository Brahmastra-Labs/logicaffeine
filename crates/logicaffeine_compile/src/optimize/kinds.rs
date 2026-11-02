//! Which names hold an Int, which a sequence, and which a sequence of Ints, on every binding the program makes.
//!
//! A name is an Int when every value the program ever binds to it is one: an integer literal, Int arithmetic over
//! Ints (`+ - × %`; `/` is left out, since division resolution may make it exact), `length of …`, a call whose
//! declared result is Int, a parameter declared Int, a counter over a range. A name is a sequence when every value
//! bound to it is one: `a new Seq of …`, a list literal, a copy of a sequence, a call whose declared result is a
//! sequence, a parameter declared one. A sequence of Ints is the same with `Int` items: `a new Seq of Int`, a list
//! literal of Ints, a copy of one, a call or a parameter declared `Seq of Int`, whose every item the type checker
//! holds to an Int. The answer is the greatest fixpoint over all of a name's bindings, so a counter
//! stepped by `Set i to i + 1` stays an Int.
//!
//! A collection's rows are unaliased ([`Kinds::rows_unaliased`]) when no two of its rows, and no row of it and a row
//! of another collection, can be one object: every value bound to it is a new sequence, every row it receives is
//! pushed once from a binding to a new sequence that nothing mentions after the push, and the collection appears
//! nowhere else but in its length and in reads and writes of an item of one of its rows in place. Under value
//! semantics no two rows are ever one object; this is the proof reference semantics needs.
//!
//! The question is asked program-wide, scopes merged: a name bound as an Int in one function and as a Real in
//! another is neither. Every binding the program makes is counted by the exhaustive write walk
//! ([`super::writes::walk_stmt`]); a name with a binding this module cannot read is unknown. So a "yes" holds on
//! every path, and a construct this module has never seen can only turn a "yes" into "unknown".

use std::collections::{HashMap, HashSet};

use crate::analysis::types::{LogosType, TypeEnv};
use crate::ast::stmt::{BinaryOpKind, ClosureBody, Expr, Literal, Pattern, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use logicaffeine_language::ast_depth::{for_each_node, Visited};

use super::writes::{walk_block, Bind, EffectSink};

/// What a name holds on every binding.
#[derive(Default)]
pub(crate) struct Kinds {
    ints: HashSet<Symbol>,
    seqs: HashSet<Symbol>,
    int_seqs: HashSet<Symbol>,
    fresh_rows: HashSet<Symbol>,
}

impl Kinds {
    /// Whether `name` holds an Int on every binding.
    pub(crate) fn int(&self, name: Symbol) -> bool {
        self.ints.contains(&name)
    }

    /// Whether `name` holds a sequence on every binding.
    pub(crate) fn seq(&self, name: Symbol) -> bool {
        self.seqs.contains(&name)
    }

    /// Whether `name` holds a sequence of Ints on every binding.
    pub(crate) fn int_seq(&self, name: Symbol) -> bool {
        self.int_seqs.contains(&name)
    }

    /// Whether no two rows of the collection `name`, nor a row of it and a row of another collection, can be one
    /// object (see the module doc).
    pub(crate) fn rows_unaliased(&self, name: Symbol) -> bool {
        self.fresh_rows.contains(&name)
    }

    /// Whether `e` is an Int wherever it is evaluated.
    pub(crate) fn int_valued(&self, e: &Expr, env: &TypeEnv) -> bool {
        int_valued(e, &self.ints, env)
    }
}

/// One binding of a name, as far as this module reads it.
enum Binding<'a> {
    Value(&'a Expr<'a>),
    Declared(&'a TypeExpr<'a>),
    Counter,
    Unknown,
}

/// The kinds of every name in `program`. `env` answers a called function's declared result.
pub(crate) fn of_program(program: &[Stmt], env: &TypeEnv, interner: &Interner) -> Kinds {
    let mut read: HashMap<Symbol, Vec<Binding>> = HashMap::new();
    for_each_node(program, |node| match node {
        Visited::Stmt(Stmt::Let { var, value, .. })
        | Visited::Stmt(Stmt::Set { target: var, value }) => {
            read.entry(*var).or_default().push(Binding::Value(value));
        }
        Visited::Stmt(Stmt::Swap { a, b }) => {
            read.entry(*a).or_default().push(Binding::Unknown);
            read.entry(*b).or_default().push(Binding::Unknown);
        }
        Visited::Stmt(Stmt::FunctionDef { params, .. }) => {
            for (p, ty) in params {
                read.entry(*p).or_default().push(Binding::Declared(ty));
            }
        }
        Visited::Stmt(Stmt::Repeat {
            pattern, iterable, ..
        }) => match pattern {
            Pattern::Identifier(s) if matches!(iterable, Expr::Range { .. }) => {
                read.entry(*s).or_default().push(Binding::Counter);
            }
            Pattern::Identifier(s) => read.entry(*s).or_default().push(Binding::Unknown),
            Pattern::Tuple(syms) => {
                for s in syms {
                    read.entry(*s).or_default().push(Binding::Unknown);
                }
            }
        },
        Visited::Expr(Expr::Closure { params, .. }) => {
            for (p, _) in params.iter() {
                read.entry(*p).or_default().push(Binding::Unknown);
            }
        }
        _ => {}
    });
    let mut counted = Counter::default();
    walk_block(program, &mut counted);

    let complete = |s: &Symbol, bindings: &Vec<Binding>| {
        counted.binds.get(s).copied().unwrap_or(0) == bindings.len()
    };
    let mut ints: HashSet<Symbol> = read
        .iter()
        .filter(|(s, b)| complete(s, b))
        .map(|(s, _)| *s)
        .collect();
    let mut seqs = ints.clone();
    let mut int_seqs = ints.clone();
    loop {
        let before = (ints.len(), seqs.len(), int_seqs.len());
        ints = ints
            .iter()
            .filter(|s| {
                read[*s].iter().all(|b| match b {
                    Binding::Value(e) => int_valued(e, &ints, env),
                    Binding::Declared(t) => declared_is(t, interner, "Int"),
                    Binding::Counter => true,
                    Binding::Unknown => false,
                })
            })
            .copied()
            .collect();
        seqs = seqs
            .iter()
            .filter(|s| {
                read[*s].iter().all(|b| match b {
                    Binding::Value(e) => seq_valued(e, &seqs, env, interner),
                    Binding::Declared(t) => declared_seq(t, interner),
                    Binding::Counter | Binding::Unknown => false,
                })
            })
            .copied()
            .collect();
        int_seqs = int_seqs
            .iter()
            .filter(|s| {
                read[*s].iter().all(|b| match b {
                    Binding::Value(e) => int_seq_valued(e, &int_seqs, &ints, env, interner),
                    Binding::Declared(t) => declared_int_seq(t, interner),
                    Binding::Counter | Binding::Unknown => false,
                })
            })
            .copied()
            .collect();
        if (ints.len(), seqs.len(), int_seqs.len()) == before {
            break;
        }
    }
    let fresh_rows = fresh_rows(program, &read, &seqs, interner);
    Kinds {
        ints,
        seqs,
        int_seqs,
        fresh_rows,
    }
}

/// The collections whose rows are fresh objects only they hold (see the module doc).
fn fresh_rows(
    program: &[Stmt],
    read: &HashMap<Symbol, Vec<Binding>>,
    seqs: &HashSet<Symbol>,
    interner: &Interner,
) -> HashSet<Symbol> {
    let born_new = |c: &Symbol| {
        read[c].iter().all(|b| {
            matches!(b, Binding::Value(Expr::New { type_name, .. })
            if matches!(interner.resolve(*type_name), "Seq" | "List"))
        })
    };
    let mut candidates: HashSet<Symbol> = seqs.iter().filter(|c| born_new(c)).copied().collect();
    // Every mention of a candidate must be one of the allowed forms: count all mentions, count the allowed ones.
    let mut mentions: HashMap<Symbol, usize> = HashMap::new();
    let mut allowed: HashMap<Symbol, usize> = HashMap::new();
    let mut direct_index: HashMap<Symbol, usize> = HashMap::new();
    let mut nested_index: HashMap<Symbol, usize> = HashMap::new();
    for_each_node(program, |node| match node {
        Visited::Expr(Expr::Identifier(s)) => *mentions.entry(*s).or_default() += 1,
        Visited::Expr(Expr::Length {
            collection: Expr::Identifier(s),
        }) => *allowed.entry(*s).or_default() += 1,
        Visited::Expr(Expr::Index {
            collection: Expr::Identifier(s),
            ..
        }) => {
            *direct_index.entry(*s).or_default() += 1;
            *allowed.entry(*s).or_default() += 1;
        }
        Visited::Expr(Expr::Index {
            collection:
                Expr::Index {
                    collection: Expr::Identifier(s),
                    ..
                },
            ..
        })
        | Visited::Stmt(Stmt::SetIndex {
            collection:
                Expr::Index {
                    collection: Expr::Identifier(s),
                    ..
                },
            ..
        }) => {
            *nested_index.entry(*s).or_default() += 1;
        }
        Visited::Stmt(Stmt::Push {
            collection: Expr::Identifier(s),
            ..
        }) => *allowed.entry(*s).or_default() += 1,
        _ => {}
    });
    candidates.retain(|c| {
        mentions.get(c).copied().unwrap_or(0) == allowed.get(c).copied().unwrap_or(0)
            && direct_index.get(c) == nested_index.get(c)
    });
    // Every push into a candidate pushes a fresh row nothing mentions afterwards.
    let mut pushes = FreshPushes {
        candidates: &candidates,
        refused: HashSet::new(),
    };
    pushes.block(program);
    walk_block(program, &mut pushes);
    candidates.retain(|c| !pushes.refused.contains(c));
    candidates
}

/// Checks every `Push x to c` into a candidate `c`, block by block (the write walk visits every nested block).
struct FreshPushes<'c> {
    candidates: &'c HashSet<Symbol>,
    refused: HashSet<Symbol>,
}

impl FreshPushes<'_> {
    fn block(&mut self, block: &[Stmt]) {
        for (at, stmt) in block.iter().enumerate() {
            let Stmt::Push {
                collection: Expr::Identifier(c),
                value,
            } = stmt
            else {
                continue;
            };
            if !self.candidates.contains(c) {
                continue;
            }
            let fresh = match value {
                Expr::Identifier(x) => {
                    let born = block[..at].iter().rposition(
                        |s| matches!(s, Stmt::Let { var, value: Expr::New { .. }, .. } if var == x),
                    );
                    born.is_some_and(|q| {
                        block[q + 1..at].iter().all(|s| match s {
                            Stmt::Push {
                                collection: Expr::Identifier(t),
                                value,
                            } if t == x => !names(value, *x),
                            other => !mentions(other, *x),
                        }) && block[at + 1..].iter().all(|s| !mentions(s, *x))
                    })
                }
                Expr::New { .. } => true,
                _ => false,
            };
            if !fresh {
                self.refused.insert(*c);
            }
        }
    }
}

impl EffectSink for FreshPushes<'_> {
    fn bind(&mut self, _sym: Symbol, _how: Bind) {}
    fn mutate(&mut self, _root: Symbol) {}
    fn call(&mut self) {}
    fn opaque(&mut self) {}
    fn scoped(&mut self, _binders: &[Symbol], body: &[Stmt]) {
        self.block(body);
        walk_block(body, self);
    }
    fn transparent(&mut self, body: &[Stmt]) {
        self.block(body);
        walk_block(body, self);
    }
    fn closure(&mut self, _params: &[Symbol], body: &ClosureBody) {
        if let ClosureBody::Block(b) = body {
            self.block(b);
            walk_block(b, self);
        }
    }
    fn function(&mut self, _params: &[Symbol], body: &[Stmt]) {
        self.block(body);
        walk_block(body, self);
    }
}

/// `e` names `x` anywhere inside it.
fn names(e: &Expr, x: Symbol) -> bool {
    let mut stack = vec![e];
    while let Some(e) = stack.pop() {
        if matches!(e, Expr::Identifier(s) if *s == x) {
            return true;
        }
        stack.extend(logicaffeine_language::ast_depth::expr_children(e));
    }
    false
}

/// `stmt` mentions `x` anywhere, its nested blocks included: in an expression, or as a name it binds or writes.
fn mentions(stmt: &Stmt, x: Symbol) -> bool {
    let mut found = false;
    for_each_node(std::slice::from_ref(stmt), |node| {
        if matches!(node, Visited::Expr(Expr::Identifier(s)) if *s == x) {
            found = true;
        }
    });
    found || super::writes::WriteSet::of_stmt(stmt).may_write(x)
}

fn int_valued(e: &Expr, ints: &HashSet<Symbol>, env: &TypeEnv) -> bool {
    match e {
        Expr::Literal(Literal::Number(_)) => true,
        Expr::Identifier(s) => ints.contains(s),
        Expr::Length { .. } => true,
        Expr::BinaryOp {
            op:
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Modulo,
            left,
            right,
        } => int_valued(left, ints, env) && int_valued(right, ints, env),
        Expr::Call { function, .. } => env
            .lookup_fn(*function)
            .is_some_and(|sig| sig.return_type == LogosType::Int),
        _ => false,
    }
}

fn seq_valued(e: &Expr, seqs: &HashSet<Symbol>, env: &TypeEnv, interner: &Interner) -> bool {
    match e {
        Expr::New { type_name, .. } => matches!(interner.resolve(*type_name), "Seq" | "List"),
        Expr::List(_) => true,
        Expr::Copy { expr } => seq_valued(expr, seqs, env, interner),
        Expr::Identifier(s) => seqs.contains(s),
        Expr::Call { function, .. } => env
            .lookup_fn(*function)
            .is_some_and(|sig| matches!(sig.return_type, LogosType::Seq(_))),
        _ => false,
    }
}

fn int_seq_valued(
    e: &Expr,
    int_seqs: &HashSet<Symbol>,
    ints: &HashSet<Symbol>,
    env: &TypeEnv,
    interner: &Interner,
) -> bool {
    match e {
        Expr::New {
            type_name,
            type_args,
            ..
        } => {
            matches!(interner.resolve(*type_name), "Seq" | "List")
                && matches!(type_args.as_slice(), [t] if declared_is(t, interner, "Int"))
        }
        Expr::WithCapacity { value, .. } => int_seq_valued(value, int_seqs, ints, env, interner),
        Expr::List(items) => items.iter().all(|x| int_valued(x, ints, env)),
        Expr::Copy { expr } => int_seq_valued(expr, int_seqs, ints, env, interner),
        Expr::Identifier(s) => int_seqs.contains(s),
        Expr::Call { function, .. } => env.lookup_fn(*function).is_some_and(
            |sig| matches!(&sig.return_type, LogosType::Seq(item) if **item == LogosType::Int),
        ),
        _ => false,
    }
}

fn declared_is(t: &TypeExpr, interner: &Interner, name: &str) -> bool {
    matches!(t, TypeExpr::Primitive(s) | TypeExpr::Named(s) if interner.resolve(*s) == name)
}

fn declared_seq(t: &TypeExpr, interner: &Interner) -> bool {
    matches!(t, TypeExpr::Generic { base, .. } if matches!(interner.resolve(*base), "Seq" | "List"))
}

fn declared_int_seq(t: &TypeExpr, interner: &Interner) -> bool {
    matches!(t, TypeExpr::Generic { base, params } if matches!(interner.resolve(*base), "Seq" | "List")
        && matches!(params, [p] if declared_is(p, interner, "Int")))
}

/// Every binding the program makes, counted per name, through the exhaustive write walk; nested functions and
/// closures included.
#[derive(Default)]
struct Counter {
    binds: HashMap<Symbol, usize>,
}

impl EffectSink for Counter {
    fn bind(&mut self, sym: Symbol, _how: Bind) {
        *self.binds.entry(sym).or_default() += 1;
    }
    fn mutate(&mut self, _root: Symbol) {}
    fn call(&mut self) {}
    fn opaque(&mut self) {}
    fn closure(&mut self, params: &[Symbol], body: &ClosureBody) {
        for p in params {
            self.bind(*p, Bind::Declare);
        }
        if let ClosureBody::Block(b) = body {
            walk_block(b, self);
        }
    }
    fn function(&mut self, params: &[Symbol], body: &[Stmt]) {
        for p in params {
            self.bind(*p, Bind::Declare);
        }
        walk_block(body, self);
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/kinds/tests.rs"]
mod tests;
