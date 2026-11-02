//! Operands in the order LOGOS evaluates them, and the places a `mutable` parameter lends.
//!
//! **Order.** LOGOS evaluates an operator's operands and a call's arguments left to right; C leaves that order
//! unspecified. Once an operand can change a binding a sibling reads, the order shows. A call changes the program
//! bindings its function writes (file-scope statics in C, [`CallEffects`]): in `counter + next()`, where `next` sets
//! `counter`, the interpreter reads the old count and C may read the new one. A call also changes what it lends to a
//! `mutable` parameter, a local as much as a program binding (the optimizer's effect walk, [`super::WriteSet`]): in
//! `a + bump(a) + a` the first `a` is read before `bump` writes it and the last after. [`sequence_operands`] binds
//! such an expression's operands in order to fresh names ahead of its statement: `Return f(n - 1) + f(n - 2)`, where
//! `f` counts its calls in a program binding, becomes `Let a be f(n - 1)`, `Let b be f(n - 2)`, `Return a + b`. A
//! sibling evaluated before a bound one is bound too, so nothing moves ahead of an operand LOGOS evaluates first.
//!
//! **Lent places.** `bump(item 2 of xs)` and `bump(p's y)` lend an element or a field to a `mutable` parameter. The
//! VM captures the place's keys at the argument's position, lends the value there, and when the call returns stores
//! the parameter's final value back through the same keys. A backend that passes a parameter by reference has only
//! variables to point at, so [`lower_lent_places`] (and [`sequence_operands`], which includes it) spells that out:
//! each key bound once in order, the value copied into a fresh `mutable` binding the call lends, the call bound to a
//! fresh name, and the binding stored back into the place, before anything after the call runs. Every argument
//! evaluated before a captured key is bound before it too. A lent binding is the caller's variable itself and is
//! never bound to another name.
//!
//! **Observed loans.** For as long as the call holds what it was lent, the VM leaves `nothing` in the place: a callee
//! that reads `n` while `n` is lent to it reads `nothing`, and an argument after the lent one that reads the place
//! reads `nothing`. A lent value something can see is lowered with that spelled out: after its copy, `nothing` is
//! stored into the place (a lent binding is then copied too, and passed as the copy), and the store after the call
//! puts the parameter's value back. Something can see it when another argument of the call names the place's root,
//! unless it is a place of the same root that provably differs from it (an item at another literal key, another
//! field), or when the function reaches the root as a program binding ([`CallEffects`]). A backend types what a
//! stored `nothing` can reach as able to hold it; a loan nothing can see is lowered as before.
//!
//! Only a statement that evaluates its expression once, before it acts, is rewritten (`Let`, `Set`, `Return`, `Show`,
//! a call, `Push`'s value, an `If`'s condition, a `Repeat`'s iterable), through the forms that evaluate every operand
//! (operators other than `and`/`or`, calls, `item … of`, `length of`, `not`). A `While` condition runs again each
//! pass and is left as it is. Names are compared as spelled: the parser may give one variable a distinct symbol per
//! occurrence.

use std::collections::{HashMap, HashSet};

use crate::analysis::callgraph::{install_lending_parameters, lent_positions};
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt};
use crate::ast_map::Arenas;
use crate::intern::{Interner, Symbol};

use super::mentions::Mentions;
use super::writes::mutation_root;
use super::WriteSet;

/// What each function's calls read and write of the program's bindings, beyond what they lend.
pub(crate) struct CallEffects {
    pub(crate) reads: HashMap<Symbol, HashSet<Symbol>>,
    pub(crate) writes: HashMap<Symbol, HashSet<Symbol>>,
}

/// Bind, in order, the operands of every expression of `stmts` whose operands write and read one binding, and lower
/// every lent place (see the module doc). For a backend that fixes no operand order.
pub(crate) fn sequence_operands<'a>(
    stmts: Vec<Stmt<'a>>,
    arenas: Arenas<'a>,
    interner: &mut Interner,
    effects: CallEffects,
) -> Vec<Stmt<'a>> {
    install_lending_parameters(&stmts);
    let lends = stmts
        .iter()
        .any(|s| matches!(s, Stmt::FunctionDef { name, .. } if !lent_positions(*name).is_empty()));
    if effects.reads.is_empty() && effects.writes.is_empty() && !lends {
        return stmts;
    }
    Sequencer {
        order: Some(effects),
        arenas,
        interner,
        next: 0,
    }
    .block_vec(stmts)
}

/// Lower every lent place of `stmts` (see the module doc). For a backend that evaluates operands left to right.
pub(crate) fn lower_lent_places<'a>(
    stmts: Vec<Stmt<'a>>,
    arenas: Arenas<'a>,
    interner: &mut Interner,
) -> Vec<Stmt<'a>> {
    install_lending_parameters(&stmts);
    if !lends_a_place(&stmts, interner) {
        return stmts;
    }
    Sequencer {
        order: None,
        arenas,
        interner,
        next: 0,
    }
    .block_vec(stmts)
}

/// Whether a call of `stmts` lends an element or a field to a `mutable` parameter, or a binding another of its
/// arguments names, as the program last installed ([`install_lending_parameters`]) declares them.
pub(crate) fn lends_a_place(stmts: &[Stmt], interner: &Interner) -> bool {
    let mut found = false;
    crate::analysis::callgraph::for_each_call_site(stmts, &mut |function, args| {
        found |= lent_positions(function).iter().any(|&i| {
            args.get(i).is_some_and(|a| {
                is_place(a) || (matches!(a, Expr::Identifier(_)) && seen_by_another(args, i, interner))
            })
        });
    });
    found
}

/// Whether another of `args` can read or hold what argument `at` lends while the call holds it: it names the lent
/// value's root, and is not a place of that root that provably differs from it. The record of program bindings a
/// tier lends every function that reaches one (`program_bindings`) is no value of the program's: what it lends is
/// never empty, though a binding of it lent beside it is.
fn seen_by_another(args: &[&Expr], at: usize, interner: &Interner) -> bool {
    let Some(root) = args.get(at).and_then(|a| mutation_root(a)) else {
        return false;
    };
    if matches!(args[at], Expr::Identifier(_))
        && interner.resolve(root) == crate::program_bindings::PROGRAM_BINDING
    {
        return false;
    }
    let name = interner.resolve(root);
    let lent = path(args[at], interner);
    args.iter().enumerate().any(|(j, other)| {
        j != at
            && Mentions::of_expr(other).touches(name, interner)
            && !matches!((&lent, path(other, interner)), (Some(a), Some(b)) if disjoint(a, &b))
    })
}

/// One step of a place's path from its root: a field by name, or an item by a literal key (`None` for any other key).
#[derive(PartialEq)]
enum Step<'e> {
    Field(&'e str),
    Item(Option<&'e crate::ast::stmt::Literal>),
}

/// `e` as its root's name and the steps from it, when `e` is a binding, an element or a field.
fn path<'e>(e: &'e Expr, interner: &'e Interner) -> Option<(&'e str, Vec<Step<'e>>)> {
    match e {
        Expr::Identifier(sym) => Some((interner.resolve(*sym), Vec::new())),
        Expr::FieldAccess { object, field } => {
            let (root, mut steps) = path(object, interner)?;
            steps.push(Step::Field(interner.resolve(*field)));
            Some((root, steps))
        }
        Expr::Index { collection, index } => {
            let (root, mut steps) = path(collection, interner)?;
            steps.push(Step::Item(match index {
                Expr::Literal(key) => Some(key),
                _ => None,
            }));
            Some((root, steps))
        }
        _ => None,
    }
}

/// Whether two places of one root can never be one another or nest: at the first step where they part, both name a
/// field or both an item by a literal key, and the two differ.
fn disjoint(a: &(&str, Vec<Step>), b: &(&str, Vec<Step>)) -> bool {
    a.0 == b.0
        && a.1.iter().zip(&b.1).find(|(x, y)| x != y).is_some_and(|step| match step {
            (Step::Field(_), Step::Field(_)) => true,
            (Step::Item(Some(x)), Step::Item(Some(y))) => x != y,
            _ => false,
        })
}

/// An element or a field of a binding: what a `mutable` parameter writes through when it is not a binding.
fn is_place(e: &Expr) -> bool {
    !matches!(e, Expr::Identifier(_))
        && mutation_root(e).is_some()
        && !matches!(e, Expr::Slice { .. })
}

struct Sequencer<'a, 'i> {
    /// The call effects operands are ordered by; `None` lowers lent places only.
    order: Option<CallEffects>,
    arenas: Arenas<'a>,
    interner: &'i mut Interner,
    next: u32,
}

impl<'a> Sequencer<'a, '_> {
    fn fresh(&mut self) -> Symbol {
        loop {
            let name = format!("__seq_{}", self.next);
            self.next += 1;
            if self.interner.lookup(&name).is_none() {
                return self.interner.intern(&name);
            }
        }
    }

    fn block_vec(&mut self, stmts: Vec<Stmt<'a>>) -> Vec<Stmt<'a>> {
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            self.stmt(s, &mut out);
        }
        out
    }

    fn block(&mut self, stmts: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        let out = self.block_vec(stmts.to_vec());
        self.arenas.stmts.alloc_slice(out)
    }

    /// `stmt` into `out`, after the bindings its expression's operands need.
    fn stmt(&mut self, stmt: Stmt<'a>, out: &mut Vec<Stmt<'a>>) {
        let s = match stmt {
            Stmt::Let {
                var,
                ty,
                value,
                mutable,
            } => Stmt::Let {
                var,
                ty,
                value: self.expr(value, out),
                mutable,
            },
            Stmt::Set { target, value } => Stmt::Set {
                target,
                value: self.expr(value, out),
            },
            Stmt::Return { value: Some(value) } => Stmt::Return {
                value: Some(self.expr(value, out)),
            },
            Stmt::Show { object, recipient } => Stmt::Show {
                object: self.expr(object, out),
                recipient,
            },
            Stmt::Push { value, collection } => Stmt::Push {
                value: self.expr(value, out),
                collection,
            },
            Stmt::Call { function, args } => {
                if self.lends_place(function, &args) {
                    self.lend(function, &args, out, false);
                    return;
                }
                let call = self.arenas.exprs.alloc(Expr::Call {
                    function,
                    args: args.clone(),
                });
                match self.expr(call, out) {
                    Expr::Call { function, args } => Stmt::Call {
                        function: *function,
                        args: args.clone(),
                    },
                    _ => Stmt::Call { function, args },
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => Stmt::If {
                cond: self.expr(cond, out),
                then_block: self.block(then_block),
                else_block: else_block.map(|b| self.block(b)),
            },
            Stmt::While {
                cond,
                body,
                decreasing,
            } => Stmt::While {
                cond,
                body: self.block(body),
                decreasing,
            },
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => Stmt::Repeat {
                pattern,
                iterable: self.expr(iterable, out),
                body: self.block(body),
            },
            Stmt::FunctionDef {
                name,
                params,
                generics,
                body,
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            } => Stmt::FunctionDef {
                name,
                params,
                generics,
                body: self.block(body),
                return_type,
                is_native,
                native_path,
                is_exported,
                export_target,
                opt_flags,
            },
            other => other,
        };
        out.push(s);
    }

    /// Whether a call of `function` with `args` lends an element or a field, or a binding something can see while the
    /// call holds it.
    fn lends_place(&self, function: Symbol, args: &[&Expr]) -> bool {
        lent_positions(function).iter().any(|&i| {
            args.get(i).is_some_and(|a| {
                is_place(a) || (matches!(a, Expr::Identifier(_)) && self.observed(function, args, i))
            })
        })
    }

    /// Whether what argument `at` of a call of `function` lends can be read or written while the call holds it
    /// (see the module doc).
    fn observed(&self, function: Symbol, args: &[&Expr], at: usize) -> bool {
        if seen_by_another(args, at, self.interner) {
            return true;
        }
        let (Some(order), Some(root)) = (&self.order, args.get(at).and_then(|a| mutation_root(a))) else {
            return false;
        };
        let name = self.interner.resolve(root);
        [order.reads.get(&function), order.writes.get(&function)]
            .into_iter()
            .flatten()
            .flatten()
            .any(|s| self.interner.resolve(*s) == name)
    }

    /// The `nothing` a place holds while the call it is lent to holds its value.
    fn nothing(&self) -> &'a Expr<'a> {
        let nothing = self.arenas.exprs.alloc(Expr::Literal(crate::ast::stmt::Literal::Nothing));
        super::facts::mark_expr(super::facts::Fact::Vacated, nothing);
        nothing
    }

    /// `value` copied into a fresh `mutable` binding, as that binding.
    fn hold(&mut self, value: &'a Expr<'a>, out: &mut Vec<Stmt<'a>>) -> &'a Expr<'a> {
        let held = self.fresh();
        out.push(Stmt::Let {
            var: held,
            ty: None,
            value,
            mutable: true,
        });
        self.arenas.exprs.alloc(Expr::Identifier(held))
    }

    /// A call that lends places, lowered into `out` (see the module doc): its value as a fresh binding when
    /// `value` is wanted, else the call as a statement.
    fn lend(
        &mut self,
        function: Symbol,
        args: &[&'a Expr<'a>],
        out: &mut Vec<Stmt<'a>>,
        value: bool,
    ) -> Option<&'a Expr<'a>> {
        let lent = lent_positions(function);
        let mut passed = Vec::with_capacity(args.len());
        let mut stores = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            if lent.contains(&i) {
                let observed = self.observed(function, args, i);
                if let Expr::Identifier(root) = arg {
                    if !observed {
                        passed.push(*arg);
                        continue;
                    }
                    let held = self.hold(arg, out);
                    out.push(Stmt::Set {
                        target: *root,
                        value: self.nothing(),
                    });
                    passed.push(held);
                    stores.push(Stmt::Set {
                        target: *root,
                        value: held,
                    });
                    continue;
                }
                if is_place(arg) {
                    let place = self.capture(arg, out);
                    let held = self.hold(place, out);
                    if observed {
                        out.push(Self::store(place, self.nothing()));
                    }
                    passed.push(held);
                    stores.push(Self::store(place, held));
                    continue;
                }
            }
            let mut pre = Vec::new();
            let v = self.expr(arg, &mut pre);
            out.extend(pre);
            passed.push(self.bind(v, lent.contains(&i), out));
        }
        let args = passed;
        let result = if value {
            let r = self.fresh();
            let call = self.arenas.exprs.alloc(Expr::Call { function, args });
            out.push(Stmt::Let {
                var: r,
                ty: None,
                value: call,
                mutable: false,
            });
            Some(&*self.arenas.exprs.alloc(Expr::Identifier(r)))
        } else {
            out.push(Stmt::Call { function, args });
            None
        };
        out.extend(stores);
        result
    }

    /// `place` with each key it reads evaluated and bound in order, so the store after the call reaches the very
    /// element or field the call was lent.
    fn capture(&mut self, place: &'a Expr<'a>, out: &mut Vec<Stmt<'a>>) -> &'a Expr<'a> {
        match place {
            Expr::Index { collection, index } => {
                let collection = self.capture(collection, out);
                let mut pre = Vec::new();
                let key = self.expr(index, &mut pre);
                out.extend(pre);
                let index = self.bind(key, false, out);
                self.arenas.exprs.alloc(Expr::Index { collection, index })
            }
            Expr::FieldAccess { object, field } => {
                let object = self.capture(object, out);
                self.arenas.exprs.alloc(Expr::FieldAccess {
                    object,
                    field: *field,
                })
            }
            _ => place,
        }
    }

    /// The statement that stores `value` into the captured `place`.
    fn store(place: &'a Expr<'a>, value: &'a Expr<'a>) -> Stmt<'a> {
        match place {
            Expr::Index { collection, index } => Stmt::SetIndex {
                collection,
                index,
                value,
            },
            Expr::FieldAccess { object, field } => Stmt::SetField {
                object,
                field: *field,
                value,
            },
            _ => unreachable!("a captured place is an element or a field"),
        }
    }

    /// `v` as an operand evaluated now: a literal as it is, anything else bound to a fresh name, `mutable` for a
    /// temporary a `mutable` parameter writes.
    fn bind(&mut self, v: &'a Expr<'a>, mutable: bool, out: &mut Vec<Stmt<'a>>) -> &'a Expr<'a> {
        if matches!(v, Expr::Literal(_)) && !mutable {
            return v;
        }
        let var = self.fresh();
        out.push(Stmt::Let {
            var,
            ty: None,
            value: v,
            mutable,
        });
        self.arenas.exprs.alloc(Expr::Identifier(var))
    }

    /// The names evaluating `e` reads and writes, as spelled: every name it reads and the program bindings its calls
    /// reach; the program bindings its calls write and what they lend to `mutable` parameters.
    fn effects(&self, order: &CallEffects, e: &Expr) -> (HashSet<String>, HashSet<String>) {
        let (mut r, mut w) = (HashSet::new(), HashSet::new());
        self.reach_and_reads(order, e, &mut r, &mut w);
        w.extend(
            WriteSet::of_expr(e)
                .symbols()
                .map(|s| self.interner.resolve(s).to_string()),
        );
        (r, w)
    }

    fn reach_and_reads(
        &self,
        order: &CallEffects,
        e: &Expr,
        r: &mut HashSet<String>,
        w: &mut HashSet<String>,
    ) {
        match e {
            Expr::Identifier(s) => {
                r.insert(self.interner.resolve(*s).to_string());
            }
            Expr::Call { function, .. } => {
                r.extend(
                    order
                        .reads
                        .get(function)
                        .into_iter()
                        .flatten()
                        .map(|s| self.interner.resolve(*s).to_string()),
                );
                w.extend(
                    order
                        .writes
                        .get(function)
                        .into_iter()
                        .flatten()
                        .map(|s| self.interner.resolve(*s).to_string()),
                );
            }
            _ => {}
        }
        for child in logicaffeine_language::ast_depth::expr_children(e) {
            self.reach_and_reads(order, child, r, w);
        }
    }

    /// Whether one of `kids` writes a name another reads (never, when only lent places are lowered).
    fn conflict(&self, kids: &[&Expr]) -> bool {
        let Some(order) = &self.order else {
            return false;
        };
        let fx: Vec<_> = kids.iter().map(|k| self.effects(order, k)).collect();
        fx.iter().enumerate().any(|(i, (_, wi))| {
            fx.iter()
                .enumerate()
                .any(|(j, (rj, _))| i != j && !wi.is_disjoint(rj))
        })
    }

    /// `e` with every conflicting operand bound in `out`, in evaluation order, and every lent place lowered.
    fn expr(&mut self, e: &'a Expr<'a>, out: &mut Vec<Stmt<'a>>) -> &'a Expr<'a> {
        match e {
            Expr::BinaryOp { op, left, right }
                if !matches!(op, BinaryOpKind::And | BinaryOpKind::Or) =>
            {
                let op = *op;
                let kids = self.operands(vec![*left, *right], &[], out);
                self.arenas.exprs.alloc(Expr::BinaryOp {
                    op,
                    left: kids[0],
                    right: kids[1],
                })
            }
            Expr::Call { function, args } if self.lends_place(*function, args) => self
                .lend(*function, args, out, true)
                .expect("a call lowered for its value has one"),
            Expr::Call { function, args } => {
                let function = *function;
                let lent = lent_positions(function);
                let kids = self.operands(args.clone(), &lent, out);
                self.arenas.exprs.alloc(Expr::Call {
                    function,
                    args: kids,
                })
            }
            Expr::Index { collection, index } => {
                let kids = self.operands(vec![*collection, *index], &[], out);
                self.arenas.exprs.alloc(Expr::Index {
                    collection: kids[0],
                    index: kids[1],
                })
            }
            Expr::Not { operand } => {
                let kids = self.operands(vec![*operand], &[], out);
                self.arenas.exprs.alloc(Expr::Not { operand: kids[0] })
            }
            Expr::Length { collection } => {
                let kids = self.operands(vec![*collection], &[], out);
                self.arenas.exprs.alloc(Expr::Length {
                    collection: kids[0],
                })
            }
            _ => e,
        }
    }

    /// The operands of one node, in order: each sequenced within itself, and bound to a fresh name ahead of the
    /// statement when the node's operands conflict, or when a later operand needed bindings of its own (so it cannot
    /// run ahead of this one). An argument at a `lent` position is the binding a `mutable` parameter writes, never a
    /// value to bind: it stays where it is.
    fn operands(
        &mut self,
        kids: Vec<&'a Expr<'a>>,
        lent: &[usize],
        out: &mut Vec<Stmt<'a>>,
    ) -> Vec<&'a Expr<'a>> {
        let conflicting = self.conflict(&kids);
        let mut staged: Vec<(Vec<Stmt<'a>>, &'a Expr<'a>)> = Vec::with_capacity(kids.len());
        for kid in kids {
            let mut pre = Vec::new();
            let value = self.expr(kid, &mut pre);
            staged.push((pre, value));
        }
        let bound_through = if conflicting {
            staged.len()
        } else {
            staged
                .iter()
                .rposition(|(pre, _)| !pre.is_empty())
                .unwrap_or(0)
        };
        let mut values = Vec::with_capacity(staged.len());
        for (i, (pre, value)) in staged.into_iter().enumerate() {
            out.extend(pre);
            if i < bound_through && !lent.contains(&i) {
                values.push(self.bind(value, false, out));
            } else {
                values.push(value);
            }
        }
        values
    }
}
