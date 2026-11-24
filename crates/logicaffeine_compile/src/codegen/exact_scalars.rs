//! Which `Int` locals of a body outgrow the word on AOT-Rust: the shared exactness decision
//! ([`crate::analysis::exactness`]), told what this tier proves.
//!
//! An `Int` local the decision makes exact is a `LogosInt`; every other one is a machine word whose arithmetic the
//! oracle proves stays in it. The syntactic promotion rules (`bigint_promote`) name the locals a constant past the
//! word, a growing product or a doubling sum make exact; this decision adds every one some arithmetic without a
//! proven bound can carry past the word: a counter climbing from a value near the word, a sum of unbounded terms.
//! [`ExactHost`] is what the tier knows that the decision does not, and the decision over a list's stores
//! (`int_vec`) asks the same host, so a value and the list it is stored into agree.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::{Interner, Symbol};
use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

/// What the scalar decision needs from the compile it serves: the range oracle over the statements being emitted,
/// the integer power builtin's name, and the functions the program's purity analysis finds pure.
pub(super) struct Scope {
    oracle: Option<Rc<crate::optimize::OracleFacts>>,
    pow: Option<Symbol>,
    abs: Option<Symbol>,
    pure_calls: HashSet<Symbol>,
}

thread_local! {
    static SCOPES: RefCell<Vec<Rc<Scope>>> = const { RefCell::new(Vec::new()) };
}

/// The decision installed for one compile; dropping it removes it.
pub(super) struct Installed {
    depth: usize,
}

impl Drop for Installed {
    fn drop(&mut self) {
        SCOPES.with(|s| s.borrow_mut().truncate(self.depth - 1));
    }
}

/// Install the scalar decision for the compile of `oracle`'s statements.
pub(super) fn install(
    oracle: Option<Rc<crate::optimize::OracleFacts>>,
    interner: &Interner,
    pure_calls: HashSet<Symbol>,
) -> Installed {
    let scope = Rc::new(Scope { oracle, pow: interner.lookup("pow"), abs: interner.lookup("abs"), pure_calls });
    let depth = SCOPES.with(|s| {
        let mut s = s.borrow_mut();
        s.push(scope);
        s.len()
    });
    Installed { depth }
}

fn scope() -> Option<Rc<Scope>> {
    SCOPES.with(|s| s.borrow().last().cloned())
}

/// What AOT-Rust knows that the shared decision does not.
pub(super) struct ExactHost<'h> {
    /// The functions whose `Int` result is a `LogosInt`.
    pub(super) bigint_fns: &'h HashSet<Symbol>,
    /// The `(type, field)` pairs a `LogosInt` is stored in.
    pub(super) bigint_fields: &'h HashSet<(Symbol, Symbol)>,
    /// The locals that arrive exact: an exactified function's parameters, locals the syntactic rules promote.
    pub(super) arriving: &'h HashSet<Symbol>,
    /// The locals bound to an `Int`, where the decision is asked of scalars; `None` for the decision over a list's
    /// stores, which reads every local.
    pub(super) ints: Option<&'h HashSet<Symbol>>,
    /// The updates a counting certificate keeps in the word, by node address.
    pub(super) bounded: &'h HashSet<usize>,
    /// The bounds the counters of list-bounded loops keep, by the address of each expression in such a loop's body
    /// (`int_vec::list_bounded_counters`).
    pub(super) counters: &'h HashMap<usize, Vec<(Symbol, i128, i128)>>,
    pub(super) oracle: Option<&'h crate::optimize::OracleFacts>,
    pub(super) pow: Option<Symbol>,
    pub(super) abs: Option<Symbol>,
}

impl crate::analysis::exactness::AstHost for ExactHost<'_> {
    fn holds_no_int(&self, var: Symbol) -> bool {
        self.ints.is_some_and(|ints| !ints.contains(&var))
    }

    fn var_arrives_exact(&self, var: Symbol) -> bool {
        self.arriving.contains(&var)
    }

    fn call_arrives_exact(&self, function: Symbol) -> bool {
        self.bigint_fns.contains(&function)
    }

    fn value_call_arrives_exact(&self) -> bool {
        true
    }

    fn field_arrives_exact(&self, field: Symbol) -> bool {
        super::bigint_promote::field_name_is_exact(self.bigint_fields, field)
    }

    fn component_arrives_exact(&self, _collection: &Expr, _k: i64) -> bool {
        false
    }

    fn arm_field_arrives_exact(&self, _enum_name: Option<Symbol>, variant: Symbol, field: Symbol) -> bool {
        self.bigint_fields.contains(&(variant, field))
    }

    fn is_power(&self, function: Symbol) -> bool {
        self.pow == Some(function)
    }

    fn is_magnitude(&self, function: Symbol) -> bool {
        self.abs == Some(function)
    }

    fn fits_word(&self, e: &Expr) -> bool {
        // A quotient leaves the word only as `i64::MIN / -1`: never by a divisor that is no `-1`, nor of a dividend
        // the oracle keeps above `i64::MIN`.
        if let Expr::BinaryOp { op: BinaryOpKind::Divide | BinaryOpKind::FloorDivide, left, right } = e {
            if matches!(right, Expr::Literal(Literal::Number(d)) if *d != -1) {
                return true;
            }
            if let Some(o) = self.oracle {
                let above_min = o.expr_lower_bound(left).is_some_and(|lo| lo > i64::MIN);
                let no_minus_one = o.expr_lower_bound(right).is_some_and(|lo| lo >= 0)
                    || o.expr_upper_bound(right).is_some_and(|hi| hi <= -2);
                if above_min || no_minus_one {
                    return true;
                }
            }
        }
        let guarded_raw = matches!(
            e,
            Expr::BinaryOp { op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply, .. }
        ) && crate::optimize::facts::holds_expr(crate::optimize::facts::Fact::RawIntOp, e);
        self.bounded.contains(&(e as *const Expr as usize))
            || guarded_raw
            || self.oracle.is_some_and(|o| super::expr::oracle_proves_in_word(o, e) || self.words_keep_it(o, e))
    }
}

impl ExactHost<'_> {
    /// Whether a sum, difference or product of operands the oracle bounds on the side that matters stays in the word,
    /// a binding this tier holds as a machine word lying in the word whatever bound the oracle leaves open: every
    /// value it is given is checked into it. `i - 1` with `i` at least one never leaves the word.
    fn words_keep_it(&self, o: &crate::optimize::OracleFacts, e: &Expr) -> bool {
        let Expr::BinaryOp { op, left, right } = e else { return false };
        let range = |x: &Expr| -> Option<(i128, i128)> {
            match x {
                Expr::Literal(Literal::Number(n)) => Some((*n as i128, *n as i128)),
                Expr::Identifier(s) if !self.arriving.contains(s) => {
                    let (mut lo, mut hi) =
                        (o.expr_lower_bound(x).unwrap_or(i64::MIN) as i128, o.expr_upper_bound(x).unwrap_or(i64::MAX) as i128);
                    let held =
                        self.counters.get(&(e as *const Expr as usize)).into_iter().flatten().find(|(counter, _, _)| counter == s);
                    if let Some((_, from, to)) = held {
                        (lo, hi) = (lo.max(*from), hi.min(*to));
                    }
                    Some((lo, hi))
                }
                _ => o.expr_int_range(x).map(|(lo, hi)| (lo as i128, hi as i128)),
            }
        };
        let (Some((a, b)), Some((c, d))) = (range(left), range(right)) else { return false };
        let (lo, hi) = match op {
            BinaryOpKind::Add => (a + c, b + d),
            BinaryOpKind::Subtract => (a - d, b - c),
            BinaryOpKind::Multiply => {
                let corners = [a * c, a * d, b * c, b * d];
                (*corners.iter().min().expect("four corners"), *corners.iter().max().expect("four corners"))
            }
            _ => return false,
        };
        lo >= i64::MIN as i128 && hi <= i64::MAX as i128
    }
}

/// The locals of `body` bound to an `Int`: a binding whose value the checker resolves to one, or an integer literal.
pub(super) fn int_locals(body: &[Stmt]) -> HashSet<Symbol> {
    let is_int =
        |e: &Expr| matches!(e, Expr::Literal(Literal::Number(_))) || super::instantiation::checked_int(e);
    let mut ints = HashSet::new();
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Let { var, value, .. } | Stmt::Set { target: var, value }) => {
            if is_int(value) {
                ints.insert(*var);
            }
            Walk::Into
        }
        _ => Walk::Into,
    });
    ints
}

/// Whether `cond` is `counter < bound` or `counter <= bound`, by `op`.
fn counts_by(cond: &Expr, counter: Symbol) -> Option<BinaryOpKind> {
    match cond {
        Expr::BinaryOp { op: op @ (BinaryOpKind::Lt | BinaryOpKind::LtEq), left: Expr::Identifier(c), .. }
            if *c == counter =>
        {
            Some(*op)
        }
        _ => None,
    }
}

/// Whether `counter` is read anywhere in `block` outside the counted loops over it and the binding each starts from
/// (`Set i to 0.` right before `While i < n:`): the value a loop leaves it with reaches nothing.
fn read_outside_its_loops(block: &[Stmt], counter: Symbol) -> bool {
    let touches = |e: &Expr| crate::optimize::expr_touches(e, counter);
    block.iter().enumerate().any(|(i, s)| match s {
        Stmt::FunctionDef { .. } => false,
        Stmt::While { cond, .. } if counts_by(cond, counter).is_some() => false,
        Stmt::Let { var, value, .. } | Stmt::Set { target: var, value }
            if *var == counter
                && !touches(value)
                && matches!(block.get(i + 1), Some(Stmt::While { cond, .. }) if counts_by(cond, counter).is_some()) =>
        {
            false
        }
        Stmt::If { cond, then_block, else_block } => {
            touches(cond)
                || read_outside_its_loops(then_block, counter)
                || else_block.is_some_and(|b| read_outside_its_loops(b, counter))
        }
        Stmt::While { cond, body, .. } => touches(cond) || read_outside_its_loops(body, counter),
        Stmt::Repeat { iterable, body, .. } => touches(iterable) || read_outside_its_loops(body, counter),
        Stmt::Zone { body, .. } => read_outside_its_loops(body, counter),
        other => crate::optimize::stmt_touches(other, counter),
    })
}

/// The increments of loop counters that stay in the word, by the address of the value each stores: `Set i to i + 1`,
/// the one write of `i` in the body of `While i < bound`, leaves `i` at most `bound`, a word. Under `While i <= bound`
/// it leaves `i` one past `bound` on the last pass, which stays in the word unless `bound` is the largest word; where
/// the loop is the counted shape the emitter lowers to a range (`loop_shape::extract_counted_while`), which never
/// computes that value, and nothing reads the counter after its loops, no value past the word is ever made. A bound
/// the decision makes exact vouches for nothing.
fn counter_steps(body: &[Stmt], decided: &crate::analysis::exactness::BodyDecision) -> HashSet<usize> {
    fn blocks<'b, 'a>(block: &'b [Stmt<'a>], out: &mut Vec<&'b [Stmt<'a>]>) {
        out.push(block);
        for s in block {
            match s {
                Stmt::If { then_block, else_block, .. } => {
                    blocks(then_block, out);
                    if let Some(b) = else_block {
                        blocks(b, out);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Zone { body, .. } => blocks(body, out),
                Stmt::Inspect { arms, .. } => {
                    for arm in arms.iter() {
                        blocks(arm.body, out);
                    }
                }
                _ => {}
            }
        }
    }
    let word_bound = |bound: &Expr| {
        let mut word = true;
        let mut stack = vec![bound];
        while let Some(e) = stack.pop() {
            match e {
                Expr::Identifier(s) => word &= !decided.vars.contains(s),
                Expr::Call { .. } | Expr::FieldAccess { .. } | Expr::Index { .. } => word = false,
                _ => {}
            }
            word &= !decided.exprs.contains(&(e as *const Expr as usize));
            stack.extend(logicaffeine_language::ast_depth::expr_children(e));
        }
        word
    };
    let mut all = Vec::new();
    blocks(body, &mut all);
    let mut steps = HashSet::new();
    for block in all {
        let refs: Vec<&Stmt> = block.iter().collect();
        for (at, s) in block.iter().enumerate() {
            let Stmt::While { cond: cond @ Expr::BinaryOp { right: bound, .. }, body: loop_body, .. } = s else {
                continue;
            };
            let Expr::BinaryOp { left: Expr::Identifier(counter), .. } = cond else { continue };
            let Some(op) = counts_by(cond, *counter) else { continue };
            if !super::int_vec::advances_once(loop_body, *counter) || !word_bound(bound) {
                continue;
            }
            let steps_safely = match op {
                BinaryOpKind::Lt => true,
                _ => {
                    at > 0
                        && crate::loop_shape::extract_counted_while(&refs, at - 1)
                            .is_some_and(|(counted, _)| counted.var == *counter)
                        && !read_outside_its_loops(body, *counter)
                }
            };
            if !steps_safely {
                continue;
            }
            for t in loop_body.iter() {
                if let Stmt::Set { target, value } = t {
                    if target == counter {
                        steps.insert(*value as *const Expr as usize);
                    }
                }
            }
        }
    }
    steps
}

/// The increments of counts that stay in the word, by the address of the value each stores: a local bound to zero at
/// the top of the body, then advanced only by `Set c to c + 1`, at most once a pass along any path through one
/// counting loop that no other loop holds, is at most that loop's trip count, a word. Branches are alternatives, as
/// for a list's counts (`int_vec::counting_increments`); a bound the decision makes exact vouches for nothing.
fn count_steps(body: &[Stmt], decided: &crate::analysis::exactness::BodyDecision) -> HashSet<usize> {
    #[derive(Default)]
    struct Writes {
        born_zero: HashMap<Symbol, u32>,
        other: HashSet<Symbol>,
        steps: HashMap<Symbol, Vec<usize>>,
    }
    fn plus_one(value: &Expr, c: Symbol) -> bool {
        matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left, right }
            if (matches!(left, Expr::Identifier(l) if *l == c) && matches!(right, Expr::Literal(Literal::Number(1))))
                || (matches!(right, Expr::Identifier(r) if *r == c) && matches!(left, Expr::Literal(Literal::Number(1)))))
    }
    fn walk<'s, 'a>(
        block: &'s [Stmt<'a>],
        path: &[(&'s [Stmt<'a>], usize)],
        depth: usize,
        counted: bool,
        words: &dyn Fn(&Expr) -> bool,
        w: &mut Writes,
    ) -> HashMap<Symbol, u32> {
        let mut runs: HashMap<Symbol, u32> = HashMap::new();
        let add = |runs: &mut HashMap<Symbol, u32>, more: HashMap<Symbol, u32>| {
            for (c, n) in more {
                *runs.entry(c).or_default() += n;
            }
        };
        let most = |arms: Vec<HashMap<Symbol, u32>>| {
            let mut most: HashMap<Symbol, u32> = HashMap::new();
            for arm in arms {
                for (c, n) in arm {
                    let m = most.entry(c).or_default();
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
                Stmt::Let { var, value, .. } => {
                    if depth == 0 && matches!(value, Expr::Literal(Literal::Number(0))) {
                        *w.born_zero.entry(*var).or_default() += 1;
                    } else {
                        w.other.insert(*var);
                    }
                }
                Stmt::Set { target, value } => {
                    if counted && depth == 1 && plus_one(value, *target) {
                        *runs.entry(*target).or_default() += 1;
                        w.steps.entry(*target).or_default().push(*value as *const Expr as usize);
                    } else {
                        w.other.insert(*target);
                    }
                }
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                    let counting = depth == 0 && super::int_vec::counted_loop(&here(), words);
                    let pass = walk(body, &[], depth + 1, counting, words, w);
                    let mut bounded = HashMap::new();
                    for (c, n) in pass {
                        if n > 1 {
                            w.other.insert(c);
                        } else if n == 1 {
                            bounded.insert(c, 1);
                        }
                    }
                    add(&mut runs, bounded);
                }
                Stmt::If { then_block, else_block, .. } => {
                    let inner = if depth == 0 { here() } else { Vec::new() };
                    let mut arms = vec![walk(then_block, &inner, depth, counted, words, w)];
                    if let Some(b) = else_block {
                        arms.push(walk(b, &inner, depth, counted, words, w));
                    }
                    add(&mut runs, most(arms));
                }
                Stmt::Inspect { arms, .. } => {
                    let inner = if depth == 0 { here() } else { Vec::new() };
                    let arms = arms.iter().map(|arm| walk(arm.body, &inner, depth, counted, words, w)).collect();
                    add(&mut runs, most(arms));
                }
                Stmt::Zone { body, .. } => {
                    let inner = if depth == 0 { here() } else { Vec::new() };
                    add(&mut runs, walk(body, &inner, depth, counted, words, w));
                }
                Stmt::Pop { into: Some(x), .. } => {
                    w.other.insert(*x);
                }
                _ => {}
            }
        }
        runs
    }
    let words = |e: &Expr| match e {
        Expr::Literal(Literal::Number(_)) | Expr::Length { .. } => true,
        Expr::Identifier(s) => !decided.vars.contains(s),
        _ => false,
    };
    let mut w = Writes::default();
    let units = walk(body, &[], 0, false, &words, &mut w);
    let mut steps = HashSet::new();
    for (c, increments) in &w.steps {
        if w.born_zero.get(c) == Some(&1) && !w.other.contains(c) && units.get(c).is_some_and(|n| *n <= 1) {
            steps.extend(increments.iter().copied());
        }
    }
    steps
}

/// Whether no operation in `e` can leave the word with every local of `exact` exact and every other local a machine
/// word: each sum, difference, product, power, left shift and division whose divisor is no nonzero literal is proven
/// in the word by the compile's oracle, and `e` reads nothing exact: no local of `exact`, no call to a function whose
/// result is exact, no exact field, no item of a list that may hold integers past the word.
pub(super) fn stays_word(
    e: &Expr,
    exact: &HashSet<Symbol>,
    bigint_fns: &HashSet<Symbol>,
    bigint_fields: &HashSet<(Symbol, Symbol)>,
    exact_list: &dyn Fn(Symbol) -> bool,
) -> bool {
    let Some(scope) = scope() else { return false };
    let (none, counters) = (HashSet::new(), HashMap::new());
    let host = ExactHost {
        bigint_fns,
        bigint_fields,
        arriving: exact,
        ints: None,
        bounded: &none,
        counters: &counters,
        oracle: scope.oracle.as_deref(),
        pow: scope.pow,
        abs: scope.abs,
    };
    let mut word = true;
    let mut stack = vec![e];
    while let Some(e) = stack.pop() {
        match e {
            Expr::Identifier(s) => word &= !exact.contains(s),
            Expr::Call { function, .. } => word &= !bigint_fns.contains(function),
            Expr::FieldAccess { field, .. } => word &= !super::bigint_promote::field_name_is_exact(bigint_fields, *field),
            Expr::Index { collection: Expr::Identifier(list), .. } => word &= !exact_list(*list),
            Expr::BinaryOp { op, right, .. } => {
                let may_leave = match op {
                    BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply | BinaryOpKind::Pow => true,
                    BinaryOpKind::Shl => true,
                    BinaryOpKind::Divide | BinaryOpKind::FloorDivide | BinaryOpKind::Modulo => {
                        !matches!(right, Expr::Literal(Literal::Number(d)) if *d != 0 && *d != -1)
                    }
                    _ => false,
                };
                if may_leave {
                    use crate::analysis::exactness::AstHost;
                    word &= host.fits_word(e);
                }
            }
            Expr::Closure { .. } => word = false,
            _ => {}
        }
        if !word {
            return false;
        }
        stack.extend(logicaffeine_language::ast_depth::expr_children(e));
    }
    word
}

/// Whether AOT-Rust takes the shared decision over its scalars. It is held off until a loop that writes a list runs
/// its exact locals on machine words first, as word twins and word regions run the rest: with it on, a sort's counters
/// and fannkuch's flips run as `LogosInt`s and those benchmarks run several times slower. Off, an `Int` is exact where
/// the syntactic rules promote it (`bigint_promote`) and a checked word everywhere else.
pub(super) const DECIDES_SCALARS: bool = false;

/// What the shared decision says of a body's `Int` locals and its returns.
pub(super) struct Exact {
    /// The locals it makes exact.
    pub(super) locals: HashSet<Symbol>,
    /// Whether a value the body returns may be past the word.
    pub(super) returns: bool,
    /// The sums, differences, products and quotients that may leave the word, by node address.
    pub(super) exprs: HashSet<usize>,
}

/// The shared decision over `body`'s `Int` locals, with `arriving` exact already, under the compile's installed
/// scope; `None` where no scope is installed (a compile that is not AOT-Rust's). A counting certificate
/// (`stmt::collect_bounded_count_accumulators_with_purity`) keeps a counter only one bounded loop advances in the
/// word, and is asked only of what the decision without it leaves exact, so a bound never relies on the count it
/// bounds.
pub(super) fn decide(
    body: &[Stmt],
    arriving: &HashSet<Symbol>,
    bigint_fns: &HashSet<Symbol>,
    bigint_fields: &HashSet<(Symbol, Symbol)>,
) -> Option<Exact> {
    if !DECIDES_SCALARS {
        return None;
    }
    let scope = scope()?;
    let ints = int_locals(body);
    let (none, counters) = (HashSet::new(), super::int_vec::list_bounded_counters(body));
    let host = ExactHost {
        bigint_fns,
        bigint_fields,
        arriving,
        ints: Some(&ints),
        bounded: &none,
        counters: &counters,
        oracle: scope.oracle.as_deref(),
        pow: scope.pow,
        abs: scope.abs,
    };
    let baseline = crate::analysis::exactness::body_exactness(body, None, &host);
    let counts =
        crate::codegen::stmt::collect_bounded_count_accumulators_with_purity(body, &baseline.vars, &scope.pure_calls);
    let mut bounded = counter_steps(body, &baseline);
    bounded.extend(count_steps(body, &baseline));
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Set { target, value }) if counts.contains(target) && !arriving.contains(target) => {
            bounded.insert(*value as *const Expr as usize);
            Walk::Into
        }
        _ => Walk::Into,
    });
    let decision = if bounded.is_empty() {
        baseline
    } else {
        crate::analysis::exactness::body_exactness(body, None, &ExactHost { bounded: &bounded, ..host })
    };
    Some(Exact {
        locals: decision.vars.into_iter().filter(|v| ints.contains(v) || arriving.contains(v)).collect(),
        returns: decision.returns,
        exprs: decision.exprs,
    })
}

/// Whether `stmts` reach an `Int` held as a `LogosInt`: a local the exactness decision makes exact (bound, assigned
/// or read), or a call to a function whose result is exact. Nested functions are their own bodies.
pub(super) fn mentions_exact(stmts: &[&Stmt], ctx: &super::context::RefinementContext) -> bool {
    let exact = |sym: Symbol| {
        ctx.is_promotable_candidate(sym) || ctx.get_variable_types().get(&sym).is_some_and(|t| t.contains("__bigint"))
    };
    let mut found = false;
    for s in stmts {
        walk_nodes(std::slice::from_ref(*s), |node| {
            if found {
                return Walk::Past;
            }
            match node {
                Visited::Stmt(Stmt::FunctionDef { .. }) => return Walk::Past,
                Visited::Expr(Expr::Identifier(sym)) => found = exact(*sym),
                Visited::Stmt(Stmt::Let { var: sym, .. } | Stmt::Set { target: sym, .. }) => found = exact(*sym),
                Visited::Expr(Expr::Call { function, .. }) | Visited::Stmt(Stmt::Call { function, .. }) => {
                    found = super::expr::is_bigint_returning_call(*function)
                }
                _ => {}
            }
            Walk::Into
        });
        if found {
            return true;
        }
    }
    false
}
