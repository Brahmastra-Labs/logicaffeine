//! Which names may share a buffer, which of those buffers a body changes, and which names a body reads again —
//! so the C tier copies a list, map, set or record exactly where the VM's copy-on-write must.
//!
//! The VM gives a collection value semantics by copy-on-write: a change to a buffer that another live binding
//! still holds copies the buffer first, and a buffer only one binding holds is changed in place. The C tier
//! makes the same decision when it generates the program. A binding (`Let b be a`), a loop variable, a match
//! binding or a popped value may leave two names on one buffer; the names that may share a buffer form a class
//! ([`Sharing`]), and a class is changed when any member is changed in place or handed to a function that
//! changes the parameter it arrives in ([`MutatingParams`]). A binding takes a copy of its own only when its
//! class is changed and another member is read again afterwards ([`Liveness`]).
//!
//! A function owns every parameter it changes: its caller hands it a copy when the caller still reads the
//! argument afterwards, and the argument itself when it does not — `Set arr to siftDown(arr, …)` gives the
//! function `arr`'s buffer, which nothing reads again, and takes back the buffer the function returns.

use std::collections::{HashMap, HashSet};

use logicaffeine_language::ast_depth::{expr_children, walk_nodes, Visited, Walk};

use super::values::{
    changed_place, mutated_roots, place_root, source_roots_through, stmt_lent_roots, CallShapes,
    LentParams,
};
use crate::ast::stmt::{ClosureBody, Expr, Pattern, Stmt};
use crate::intern::Symbol;

/// For each function its callers copy for, which of its parameters it may change in place.
pub(super) type MutatingParams = HashMap<Symbol, Vec<bool>>;

/// The classes of names in one body that may share a buffer, and which classes the body changes.
pub(super) struct Sharing {
    class: HashMap<Symbol, usize>,
    members: Vec<Vec<Symbol>>,
    changed: Vec<bool>,
    /// Per class, every statement of the body that changes a member's buffer in place, with that member.
    sites: Vec<Vec<(usize, Symbol)>>,
}

impl Sharing {
    /// The sharing of `body`: `also_changed` are names changed outside it (a program binding a function
    /// changes), `mutating` the parameters each function changes, `shapes` which arguments each function's
    /// result may share, `lends` each function's `mutable` parameter positions.
    pub(super) fn of(
        body: &[Stmt],
        also_changed: &HashSet<Symbol>,
        mutating: &MutatingParams,
        shapes: &CallShapes,
        lends: &LentParams,
    ) -> Sharing {
        let mut parent: HashMap<Symbol, Symbol> = HashMap::new();
        fn find(parent: &mut HashMap<Symbol, Symbol>, s: Symbol) -> Symbol {
            let mut root = s;
            while let Some(&p) = parent.get(&root) {
                if p == root {
                    break;
                }
                root = p;
            }
            let mut at = s;
            while at != root {
                let next = parent.insert(at, root).unwrap_or(root);
                at = next;
            }
            parent.entry(root).or_insert(root);
            root
        }
        let union = |parent: &mut HashMap<Symbol, Symbol>, a: Symbol, others: &HashSet<Symbol>| {
            let ra = find(parent, a);
            for &b in others {
                let rb = find(parent, b);
                if ra != rb {
                    parent.insert(rb, ra);
                }
            }
        };
        let direct = changed_in(body, also_changed, mutating, shapes, lends);
        let mut sites: Vec<(usize, Symbol)> = Vec::new();
        walk_nodes(body, |node| match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                Walk::Past
            }
            Visited::Stmt(s) => {
                let mut roots = HashSet::new();
                match s {
                    Stmt::Let { var, value, .. } => {
                        source_roots_through(value, shapes, &mut roots);
                        union(&mut parent, *var, &roots);
                    }
                    Stmt::Set { target, value } => {
                        source_roots_through(value, shapes, &mut roots);
                        union(&mut parent, *target, &roots);
                    }
                    Stmt::Repeat {
                        pattern, iterable, ..
                    } => {
                        source_roots_through(iterable, shapes, &mut roots);
                        for bound in pattern_names(pattern) {
                            union(&mut parent, bound, &roots);
                        }
                    }
                    Stmt::Inspect { target, arms, .. } => {
                        source_roots_through(target, shapes, &mut roots);
                        for arm in arms {
                            for (_, bound) in &arm.bindings {
                                union(&mut parent, *bound, &roots);
                            }
                        }
                    }
                    Stmt::Pop {
                        collection,
                        into: Some(into),
                    } => {
                        roots.extend(place_root(collection));
                        union(&mut parent, *into, &roots);
                    }
                    Stmt::Swap { a, b } => {
                        roots.insert(*b);
                        union(&mut parent, *a, &roots);
                    }
                    _ => {}
                }
                if let Some(root) = changed_place(s).and_then(place_root) {
                    sites.push((stmt_key(s), root));
                }
                let mut lent = HashSet::new();
                stmt_lent_roots(s, lends, &mut lent);
                sites.extend(lent.into_iter().map(|root| (stmt_key(s), root)));
                Walk::Into
            }
            Visited::Expr(_) => Walk::Into,
        });
        for &s in &direct {
            find(&mut parent, s);
        }
        for &(_, root) in &sites {
            find(&mut parent, root);
        }
        let names: Vec<Symbol> = parent.keys().copied().collect();
        let mut class = HashMap::new();
        let mut index: HashMap<Symbol, usize> = HashMap::new();
        let mut members: Vec<Vec<Symbol>> = Vec::new();
        for s in names {
            let root = find(&mut parent, s);
            let i = *index.entry(root).or_insert_with(|| {
                members.push(Vec::new());
                members.len() - 1
            });
            members[i].push(s);
            class.insert(s, i);
        }
        let mut changed = vec![false; members.len()];
        for s in &direct {
            if let Some(&i) = class.get(s) {
                changed[i] = true;
            }
        }
        let mut class_sites = vec![Vec::new(); members.len()];
        for (key, root) in sites {
            class_sites[class[&root]].push((key, root));
        }
        Sharing {
            class,
            members,
            changed,
            sites: class_sites,
        }
    }

    /// The statements that change a buffer of `s`'s class in place, each with the member it changes.
    pub(super) fn change_sites(&self, s: Symbol) -> &[(usize, Symbol)] {
        match self.class.get(&s) {
            Some(&i) => &self.sites[i],
            None => &[],
        }
    }

    /// Whether the buffer `s` holds may be changed in place while the body runs.
    pub(super) fn changed(&self, s: Symbol) -> bool {
        self.class.get(&s).is_some_and(|&i| self.changed[i])
    }

    /// Every name that may share a buffer with `s`, `s` among them.
    pub(super) fn class_of(&self, s: Symbol) -> Vec<Symbol> {
        match self.class.get(&s) {
            Some(&i) => self.members[i].clone(),
            None => vec![s],
        }
    }

    /// Every name that may share a buffer with any of `names`, those among them.
    pub(super) fn closure(&self, names: &HashSet<Symbol>) -> HashSet<Symbol> {
        names.iter().flat_map(|s| self.class_of(*s)).collect()
    }
}

/// Every name `body` changes in place, or hands to a parameter a function changes, with `also_changed`: the
/// names whose buffers change while the body runs.
pub(super) fn changed_in(
    body: &[Stmt],
    also_changed: &HashSet<Symbol>,
    mutating: &MutatingParams,
    shapes: &CallShapes,
    lends: &LentParams,
) -> HashSet<Symbol> {
    let mut direct: HashSet<Symbol> = mutated_roots(body, lends);
    direct.extend(also_changed.iter().copied());
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Call { function, args }) => {
            handed_to(*function, args, mutating, shapes, &mut direct);
            Walk::Into
        }
        Visited::Stmt(Stmt::Show { object, recipient } | Stmt::Give { object, recipient }) => {
            if let Expr::Identifier(f) = recipient {
                handed_to(*f, &[*object], mutating, shapes, &mut direct);
            }
            Walk::Into
        }
        Visited::Expr(Expr::Call { function, args }) => {
            handed_to(*function, args, mutating, shapes, &mut direct);
            Walk::Into
        }
        _ => Walk::Into,
    });
    direct
}

/// Adds to `changed` the roots of every argument `function` may change in place.
fn handed_to(
    function: Symbol,
    args: &[&Expr],
    mutating: &MutatingParams,
    shapes: &CallShapes,
    changed: &mut HashSet<Symbol>,
) {
    let Some(flags) = mutating.get(&function) else {
        return;
    };
    for (arg, &mutates) in args.iter().zip(flags) {
        if mutates {
            source_roots_through(arg, shapes, changed);
        }
    }
}

/// The names a loop pattern binds.
pub(super) fn pattern_names(pattern: &Pattern) -> Vec<Symbol> {
    match pattern {
        Pattern::Identifier(s) => vec![*s],
        Pattern::Tuple(names) => names.clone(),
    }
}

/// `shapes` narrowed to what each of `functions` — `(name, parameter names, body)` — can really hand back: an
/// argument its result may share is one whose parameter shares a class with a value some `Return` of the body
/// yields. Starting from the parameter and result types, each round only drops a pair no `Return` reaches, so
/// the rounds end, and what remains still holds every pair a run can produce.
pub(super) fn narrowed_call_shapes(
    functions: &[(Symbol, Vec<Symbol>, &[Stmt])],
    shapes: &CallShapes,
    lends: &LentParams,
) -> CallShapes {
    let mut shapes = shapes.clone();
    let unchanged = HashSet::new();
    let no_mutation = MutatingParams::new();
    loop {
        let mut narrowed = false;
        for (name, params, body) in functions {
            let Some(current) = shapes.get(name).cloned() else {
                continue;
            };
            if !current.contains(&true) {
                continue;
            }
            let sharing = Sharing::of(body, &unchanged, &no_mutation, &shapes, lends);
            let mut returned = HashSet::new();
            walk_nodes(body, |node| match node {
                Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                    Walk::Past
                }
                Visited::Stmt(Stmt::Return { value: Some(v) }) => {
                    source_roots_through(v, &shapes, &mut returned);
                    Walk::Into
                }
                _ => Walk::Into,
            });
            let reached = sharing.closure(&returned);
            let next: Vec<bool> = params
                .iter()
                .zip(&current)
                .map(|(p, &shared)| shared && reached.contains(p))
                .collect();
            if next != current {
                shapes.insert(*name, next);
                narrowed = true;
            }
        }
        if !narrowed {
            return shapes;
        }
    }
}

/// Which parameters of each of `functions` — `(name, parameter names, body)` — the function may change in place,
/// with `also_changed` the program bindings some function changes. A parameter is changed when its class is:
/// changed in the body, or handed to a parameter another function changes, to a fixpoint over the calls.
pub(super) fn mutating_params(
    functions: &[(Symbol, Vec<Symbol>, &[Stmt])],
    also_changed: &HashSet<Symbol>,
    shapes: &CallShapes,
    lends: &LentParams,
) -> MutatingParams {
    let mut mutating: MutatingParams = functions
        .iter()
        .map(|(name, params, _)| (*name, vec![false; params.len()]))
        .collect();
    loop {
        let mut grew = false;
        for (name, params, body) in functions {
            let sharing = Sharing::of(body, also_changed, &mutating, shapes, lends);
            for (i, p) in params.iter().enumerate() {
                if !mutating[name][i] && sharing.changed(*p) {
                    mutating.get_mut(name).expect("every function has flags")[i] = true;
                    grew = true;
                }
            }
        }
        if !grew {
            return mutating;
        }
    }
}

/// For each statement of one body, the names read after it — before being bound again — on some path to the
/// body's end, plus `always`, the names read from outside the body at any time (the program bindings functions
/// reach). A statement inside a construct the analysis does not follow has no entry, and every name is taken to
/// be read after it.
pub(super) struct Liveness {
    after: HashMap<usize, HashSet<Symbol>>,
}

impl Liveness {
    pub(super) fn of(body: &[Stmt], always: &HashSet<Symbol>) -> Liveness {
        let mut walker = LiveWalker {
            after: HashMap::new(),
            always,
        };
        let exits = Exits {
            brk: HashSet::new(),
            ret: HashSet::new(),
        };
        walker.block(body, &HashSet::new(), &exits);
        Liveness {
            after: walker.after,
        }
    }

    /// The names read after `stmt`, when the analysis followed it.
    pub(super) fn after(&self, stmt: &Stmt) -> Option<&HashSet<Symbol>> {
        self.after_key(stmt_key(stmt))
    }

    /// [`Liveness::after`] for the statement at address `key`.
    pub(super) fn after_key(&self, key: usize) -> Option<&HashSet<Symbol>> {
        self.after.get(&key)
    }
}

pub(super) fn stmt_key(stmt: &Stmt) -> usize {
    stmt as *const Stmt as *const () as usize
}

/// Where control goes on `Break` (the innermost loop's or zone's end) and on `Return` (the innermost zone's
/// end, or the body's), as the names read from there on.
struct Exits {
    brk: HashSet<Symbol>,
    ret: HashSet<Symbol>,
}

struct LiveWalker<'s> {
    after: HashMap<usize, HashSet<Symbol>>,
    always: &'s HashSet<Symbol>,
}

impl LiveWalker<'_> {
    fn record(&mut self, stmt: &Stmt, live: &HashSet<Symbol>) {
        let mut after = live.clone();
        after.extend(self.always.iter().copied());
        self.after.insert(stmt_key(stmt), after);
    }

    fn block(&mut self, stmts: &[Stmt], out: &HashSet<Symbol>, exits: &Exits) -> HashSet<Symbol> {
        let mut live = out.clone();
        for s in stmts.iter().rev() {
            self.record(s, &live);
            live = self.stmt(s, &live, exits);
        }
        live
    }

    fn stmt(&mut self, s: &Stmt, out: &HashSet<Symbol>, exits: &Exits) -> HashSet<Symbol> {
        match s {
            Stmt::Let { var, value, .. } => {
                let mut live = out.clone();
                live.remove(var);
                expr_uses(value, &mut live);
                live
            }
            Stmt::Set { target, value } => {
                let mut live = out.clone();
                live.remove(target);
                expr_uses(value, &mut live);
                live
            }
            Stmt::Return { value } => {
                let mut live = exits.ret.clone();
                if let Some(v) = value {
                    expr_uses(v, &mut live);
                }
                live
            }
            Stmt::Break => exits.brk.clone(),
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let mut live = self.block(then_block, out, exits);
                match else_block {
                    Some(b) => live.extend(self.block(b, out, exits)),
                    None => live.extend(out.iter().copied()),
                }
                expr_uses(cond, &mut live);
                live
            }
            Stmt::While { cond, body, .. } => {
                let inner = Exits {
                    brk: out.clone(),
                    ret: exits.ret.clone(),
                };
                let mut head = out.clone();
                expr_uses(cond, &mut head);
                loop {
                    let mut next = self.block(body, &head, &inner);
                    next.extend(out.iter().copied());
                    expr_uses(cond, &mut next);
                    if next == head {
                        return head;
                    }
                    head = next;
                }
            }
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                let inner = Exits {
                    brk: out.clone(),
                    ret: exits.ret.clone(),
                };
                let bound = pattern_names(pattern);
                let mut head = out.clone();
                loop {
                    let mut next: HashSet<Symbol> = self
                        .block(body, &head, &inner)
                        .into_iter()
                        .filter(|s| !bound.contains(s))
                        .collect();
                    next.extend(out.iter().copied());
                    if next == head {
                        break;
                    }
                    head = next;
                }
                expr_uses(iterable, &mut head);
                head
            }
            Stmt::Inspect { target, arms, .. } => {
                let mut live = out.clone();
                for arm in arms {
                    let bound: Vec<Symbol> = arm.bindings.iter().map(|(_, b)| *b).collect();
                    live.extend(
                        self.block(arm.body, out, exits)
                            .into_iter()
                            .filter(|s| !bound.contains(s)),
                    );
                }
                expr_uses(target, &mut live);
                live
            }
            Stmt::Zone { body, .. } => {
                let inner = Exits {
                    brk: out.clone(),
                    ret: out.clone(),
                };
                self.block(body, out, &inner)
            }
            Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
                let mut live = out.clone();
                for task in tasks.iter().rev() {
                    self.record(task, &live);
                    let inner = Exits {
                        brk: live.clone(),
                        ret: live.clone(),
                    };
                    live = self.stmt(task, &live, &inner);
                }
                live
            }
            Stmt::FunctionDef { .. } => out.clone(),
            // Each name a swap exchanges is read, then bound again.
            Stmt::Swap { a, b } => {
                let mut live = out.clone();
                live.insert(*a);
                live.insert(*b);
                live
            }
            other => {
                let mut live = out.clone();
                if may_leave(other) {
                    live.extend(exits.brk.iter().copied());
                    live.extend(exits.ret.iter().copied());
                }
                stmt_uses(other, &mut live);
                live
            }
        }
    }
}

/// Whether `stmt` holds a `Break` or a `Return` that leaves it.
pub(super) fn may_leave(stmt: &Stmt) -> bool {
    let mut leaves = false;
    walk_nodes(std::slice::from_ref(stmt), |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Break | Stmt::Return { .. }) => {
            leaves = true;
            Walk::Past
        }
        _ => Walk::Into,
    });
    leaves
}

/// Every name `expr` reads, a closure's captures among them.
pub(super) fn expr_uses(expr: &Expr, out: &mut HashSet<Symbol>) {
    let mut work = vec![expr];
    while let Some(e) = work.pop() {
        match e {
            Expr::Identifier(s) => {
                out.insert(*s);
            }
            Expr::Closure { body, .. } => match body {
                ClosureBody::Expression(inner) => work.push(inner),
                ClosureBody::Block(stmts) => stmts_uses(stmts, out),
            },
            _ => work.extend(expr_children(e)),
        }
    }
}

/// Every name `stmt` reads anywhere inside it.
fn stmt_uses(stmt: &Stmt, out: &mut HashSet<Symbol>) {
    stmts_uses(std::slice::from_ref(stmt), out);
}

fn stmts_uses(stmts: &[Stmt], out: &mut HashSet<Symbol>) {
    walk_nodes(stmts, |node| {
        match node {
            Visited::Expr(Expr::Identifier(s)) => {
                out.insert(*s);
            }
            // Names a statement holds itself rather than through an expression.
            Visited::Stmt(Stmt::Swap { a, b }) => out.extend([*a, *b]),
            Visited::Stmt(Stmt::Check {
                subject, object, ..
            }) => out.extend(std::iter::once(*subject).chain(*object)),
            Visited::Stmt(Stmt::Sync { var, .. }) => {
                out.insert(*var);
            }
            _ => {}
        }
        Walk::Into
    });
}

/// Every read of a name in `stmt`, as the address of the expression that reads it — to tell a read inside one
/// argument from a read anywhere else in the statement.
pub(super) fn name_reads(stmt: &Stmt) -> Vec<(usize, Symbol)> {
    let mut reads = Vec::new();
    walk_nodes(std::slice::from_ref(stmt), |node| {
        if let Visited::Expr(e @ Expr::Identifier(s)) = node {
            reads.push((e as *const Expr as *const () as usize, *s));
        }
        Walk::Into
    });
    reads
}

/// The addresses of every expression in `expr`.
pub(super) fn expr_addresses(expr: &Expr) -> HashSet<usize> {
    let mut out = HashSet::new();
    let mut work = vec![expr];
    while let Some(e) = work.pop() {
        out.insert(e as *const Expr as *const () as usize);
        match e {
            Expr::Closure {
                body: ClosureBody::Expression(inner),
                ..
            } => work.push(inner),
            Expr::Closure {
                body: ClosureBody::Block(stmts),
                ..
            } => {
                walk_nodes(stmts, |node| {
                    if let Visited::Expr(inner) = node {
                        out.insert(inner as *const Expr as *const () as usize);
                    }
                    Walk::Into
                });
            }
            _ => work.extend(expr_children(e)),
        }
    }
    out
}
