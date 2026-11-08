//! Type-directed resolution of the exact contexts.
//!
//! A context that declares an exact type gives its arithmetic the exact reading:
//! - the parser emits an ambiguous [`BinaryOpKind::Divide`] for `/`, which floors (`7 / 2 → 3`, the
//!   integer default every existing program relies on); in an exact context it becomes
//!   [`BinaryOpKind::ExactDivide`] (`7 / 2 → 7/2`);
//! - `sqrt`, `exp` and `ln` become the exact square root, exponential and natural logarithm
//!   (`exactSqrt`, `exactExp`, `exactLn`), and the names `pi` (`π`) and `e` the exact constants
//!   (`exactPi()`, `exactE()`), unless the program defines a function or binds a name so spelled;
//! - a `**` whose exponent is itself an exact field value becomes the exact power (`exactPower`), which
//!   takes a root when the exponent is a fraction: `2 ** half`, `half` a `Rational` one half, is `√2`;
//! - in an `Ordinal` context the name `omega` (`ω`) is the first infinite ordinal (`exactOmega()`).
//!
//! The exact field types are `Rational` and the exact real types above it (`Algebraic`, `ExactReal`,
//! `Hyperreal`). Because the default stays as it was, code that declares none of them is untouched:
//! `sqrt(2)` there is still a `Real`, and `pi(digits)` still the library's digits. A bare `pi` (`π`) or `e`
//! outside an exact context is the `Real` constant, the double nearest π or e ([`resolve_real_constants`]).
//!
//! Resolution is **bidirectional**, a small forward dataflow over the variables known to hold an
//! exact field value or an ordinal:
//!   * TOP-DOWN — a declared context (a `Let x: Rational be …`, a function returning one, an
//!     assignment to such a variable, an argument at a parameter declared with one) pushes the
//!     expectation down the arithmetic spine (`+ − × ÷`, the base of `**`, and the argument of an
//!     exact function);
//!   * BOTTOM-UP — an operation with an exact operand (a known variable, an exact constructor, the
//!     infinitesimal a program declares or a standard part, a call to a function of the program
//!     declared to return an exact type) is itself exact, so the
//!     spine it stands in is an exact context, and a variable bound to it joins the known set; a
//!     comparison with an exact operand is an exact context for its other operand.
//! An index and the exponent of `**` carry their own (`Int`) context: no expectation from above makes
//! a `/` there exact, though an exact operand inside one still does. A spine with no exact operand and
//! no declared context is left alone — algorithms are safe.
//!
//! Marking these divisions `ExactDivide` also keeps them away from the integer-only
//! division strength reductions (MagicDivU / DivPow2), which would miscompile a Rational.

use std::collections::{HashMap, HashSet};
use std::f64::consts::{E, PI};

use logicaffeine_base::{Arena, Interner, Rational, Symbol};

use crate::ast::stmt::{BinaryOpKind, ClosureBody, Expr, Literal, Stmt, TypeExpr};
use crate::ast_map::{self, Arenas, AstMap};
use crate::optimize::writes::{self, Bind, EffectSink};

/// Resolve every exact context of the program. Returns the rewritten statements, or `None` if
/// nothing changed (so callers keep the original).
///
/// `fold_constants` is the [`Opt::Comptime`](crate::optimization::Opt::Comptime) toggle:
/// the always-on default collapses a constant Rational chain (`1/3 + 1/6`) to its closed
/// form at compile time; `## No comptime` turns that folding off (the chain still runs,
/// still exact). The resolution itself is semantic, never optional.
///
/// `library` names the program's functions that are the standard library's own definitions
/// ([`crate::loader::library_owned_functions`]): `exp` and `ln` are the library's, and become exact
/// in an exact context, unless the program defines its own.
pub(crate) fn resolve_divisions<'a>(
    stmts: &[Stmt<'a>],
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &mut Interner,
    fold_constants: bool,
    library: &HashSet<String>,
) -> Option<&'a [Stmt<'a>]> {
    let mut own = HashSet::new();
    let mut signatures = HashMap::new();
    for s in stmts {
        if let Stmt::FunctionDef {
            name,
            params,
            return_type,
            ..
        } = s
        {
            if !library.contains(interner.resolve(*name)) {
                own.insert(*name);
            }
            signatures.insert(
                *name,
                Signature {
                    params: params.iter().map(|(_, ty)| expect_of(Some(ty), interner)).collect(),
                    returns: expect_of(*return_type, interner),
                },
            );
        }
    }
    let globals = Scope::of(
        stmts
            .iter()
            .filter(|s| !matches!(s, Stmt::FunctionDef { .. })),
    )
    .bound;
    let names = Names::intern(interner);
    let mut cx = Cx {
        stmt_arena,
        expr_arena,
        interner,
        fold_constants,
        own,
        signatures,
        bound: globals.clone(),
        globals,
        names,
    };
    let mut known = Known::default();
    cx.block(stmts, &mut known, Expect::Plain)
}

/// Read every bare `pi` (`π`) and `e` the exact contexts left as the `Real` constant, the double nearest π or e, in
/// each scope — `## Main`, or one function, which also sees the names `## Main` binds — that binds no name so
/// spelled ([`Scope`]), unless the program defines its own function so named. Returns the rewritten statements, or
/// `None` if no scope reads one.
pub(crate) fn resolve_real_constants<'a>(
    stmts: &[Stmt<'a>],
    arenas: Arenas<'a>,
    interner: &mut Interner,
    library: &HashSet<String>,
) -> Option<&'a [Stmt<'a>]> {
    let names = Names::intern(interner);
    let doubles = [(names.pi[0], PI), (names.pi[1], PI), (names.e, E)];
    let own: HashSet<Symbol> = stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::FunctionDef { name, .. } if !library.contains(interner.resolve(*name)) => Some(*name),
            _ => None,
        })
        .collect();
    let constants = |scope: &Scope, outer: &HashSet<Symbol>| -> HashMap<Symbol, f64> {
        doubles
            .iter()
            .filter(|(sym, _)| {
                scope.read.contains(sym) && !scope.bound.contains(sym) && !outer.contains(sym) && !own.contains(sym)
            })
            .copied()
            .collect()
    };
    let main = Scope::of(stmts.iter().filter(|s| !matches!(s, Stmt::FunctionDef { .. })));
    let at_main = constants(&main, &HashSet::new());
    let mut out: Option<Vec<Stmt<'a>>> = None;
    for (i, s) in stmts.iter().enumerate() {
        let here = match s {
            Stmt::FunctionDef { .. } => constants(&Scope::of(std::iter::once(s)), &main.bound),
            _ => at_main.clone(),
        };
        let mapped = (!here.is_empty()).then(|| RealConstants { arenas, constants: here }.map_stmt(s));
        match (&mut out, mapped) {
            (Some(v), Some(m)) => v.push(m),
            (Some(v), None) => v.push(s.clone()),
            (None, Some(m)) => {
                let mut v = stmts[..i].to_vec();
                v.push(m);
                out = Some(v);
            }
            (None, None) => {}
        }
    }
    out.map(|v| arenas.stmts.alloc_slice(v))
}

/// The rewrite of one scope: each name it reads as a constant becomes its double.
struct RealConstants<'a> {
    arenas: Arenas<'a>,
    constants: HashMap<Symbol, f64>,
}

impl<'a> AstMap<'a> for RealConstants<'a> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        match e {
            Expr::Identifier(sym) => match self.constants.get(sym) {
                Some(&double) => self.arenas.exprs.alloc(Expr::Literal(Literal::Float(double))),
                None => e,
            },
            _ => ast_map::walk_expr(self, e),
        }
    }
}

/// What a context expects of a value, and what an expression is known to yield: nothing in
/// particular, an element of an exact field, or an ordinal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    Plain,
    Field,
    Ordinal,
}

impl Expect {
    /// The context two operands set together: an exact field value wins, then an ordinal.
    fn join(self, other: Expect) -> Expect {
        match (self, other) {
            (Expect::Field, _) | (_, Expect::Field) => Expect::Field,
            (Expect::Ordinal, _) | (_, Expect::Ordinal) => Expect::Ordinal,
            _ => Expect::Plain,
        }
    }
}

/// The expectation a declared type sets.
fn expect_of(ty: Option<&TypeExpr>, interner: &Interner) -> Expect {
    match ty {
        Some(TypeExpr::Primitive(s)) | Some(TypeExpr::Named(s)) => {
            let name = interner.resolve(*s);
            if ["Rational", "Algebraic", "ExactReal", "Hyperreal"]
                .iter()
                .any(|t| name.eq_ignore_ascii_case(t))
            {
                Expect::Field
            } else if name.eq_ignore_ascii_case("Ordinal") {
                Expect::Ordinal
            } else {
                Expect::Plain
            }
        }
        _ => Expect::Plain,
    }
}

/// A function of the program: what its parameters and its result are declared to be.
struct Signature {
    params: Vec<Expect>,
    returns: Expect,
}

/// The variables known to hold an exact field value or an ordinal.
#[derive(Clone, Default)]
struct Known {
    field: HashSet<Symbol>,
    ordinal: HashSet<Symbol>,
}

impl Known {
    fn of(&self, sym: Symbol) -> Expect {
        if self.field.contains(&sym) {
            Expect::Field
        } else if self.ordinal.contains(&sym) {
            Expect::Ordinal
        } else {
            Expect::Plain
        }
    }

    fn bind(&mut self, sym: Symbol, kind: Expect) {
        match kind {
            Expect::Field => {
                self.field.insert(sym);
            }
            Expect::Ordinal => {
                self.ordinal.insert(sym);
            }
            Expect::Plain => {}
        }
    }
}

/// The spellings this pass reads and writes, interned once.
struct Names {
    sqrt: Symbol,
    exp: Symbol,
    ln: Symbol,
    pi: [Symbol; 2],
    e: Symbol,
    omega: [Symbol; 2],
    exact_sqrt: Symbol,
    exact_exp: Symbol,
    exact_ln: Symbol,
    exact_pi: Symbol,
    exact_e: Symbol,
    exact_omega: Symbol,
    exact_power: Symbol,
    /// The extension's makers of a field value: an infinitesimal, declared on its own or against
    /// others, and a hyperreal's standard part.
    field_makers: [Symbol; 3],
}

impl Names {
    fn intern(interner: &mut Interner) -> Names {
        use logicaffeine_base::extension::Builtin;
        let mut i = |s: &str| interner.intern(s);
        Names {
            sqrt: i("sqrt"),
            exp: i("exp"),
            ln: i("ln"),
            pi: [i("pi"), i("π")],
            e: i("e"),
            omega: [i("omega"), i("ω")],
            exact_sqrt: i(Builtin::ExactSqrt.name()),
            exact_exp: i(Builtin::ExactExp.name()),
            exact_ln: i(Builtin::ExactLn.name()),
            exact_pi: i(Builtin::ExactPi.name()),
            exact_e: i(Builtin::ExactE.name()),
            exact_omega: i(Builtin::ExactOmega.name()),
            exact_power: i(Builtin::ExactPower.name()),
            field_makers: [
                i(Builtin::Infinitesimal.name()),
                i(Builtin::OrderedInfinitesimal.name()),
                i(Builtin::StandardPart.name()),
            ],
        }
    }

    /// The exact functions of one argument.
    fn exact_functions(&self) -> [Symbol; 3] {
        [self.exact_sqrt, self.exact_exp, self.exact_ln]
    }
}

/// What statements bind and read anywhere in them: every name a statement declares, assigns or mutates, a block's
/// header binds (a loop's pattern, an arm's bindings), a closure or a nested function takes as a parameter or binds
/// in its body, and every name an expression reads. A name bound anywhere in a scope is that binding wherever the
/// scope names it, so a bare `pi` or `e` is a constant only in a scope that binds no name so spelled.
#[derive(Default)]
struct Scope {
    bound: HashSet<Symbol>,
    read: HashSet<Symbol>,
}

impl Scope {
    fn of<'s, 'a: 's>(stmts: impl Iterator<Item = &'s Stmt<'a>>) -> Scope {
        let mut scope = Scope::default();
        for s in stmts {
            writes::walk_stmt(s, &mut scope);
        }
        scope
    }
}

impl EffectSink for Scope {
    fn bind(&mut self, sym: Symbol, _how: Bind) {
        self.bound.insert(sym);
    }
    fn mutate(&mut self, root: Symbol) {
        self.bound.insert(root);
    }
    fn call(&mut self) {}
    fn opaque(&mut self) {}
    fn read(&mut self, sym: Symbol) {
        self.read.insert(sym);
    }
    fn closure(&mut self, params: &[Symbol], body: &ClosureBody) {
        self.bound.extend(params.iter().copied());
        match body {
            ClosureBody::Expression(e) => writes::walk_expr(e, self),
            ClosureBody::Block(b) => writes::walk_block(b, self),
        }
    }
    fn function(&mut self, params: &[Symbol], body: &[Stmt]) {
        self.bound.extend(params.iter().copied());
        writes::walk_block(body, self);
    }
}

struct Cx<'a, 'i> {
    stmt_arena: &'a Arena<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    interner: &'i mut Interner,
    /// Whether the closed-form constant fold (the `Opt::Comptime` optimization) is on.
    fold_constants: bool,
    /// The functions the program defines itself, which no exact spelling replaces.
    own: HashSet<Symbol>,
    signatures: HashMap<Symbol, Signature>,
    /// The names `## Main` binds, which every function sees.
    globals: HashSet<Symbol>,
    /// The names bound where the resolution stands: the globals, and a function's own.
    bound: HashSet<Symbol>,
    names: Names,
}

impl<'a, 'i> Cx<'a, 'i> {
    /// Resolve a statement list in order, threading the known variables. `ret` is what the
    /// enclosing function's result is declared to be (so its `Return`s are that context).
    fn block(
        &mut self,
        stmts: &[Stmt<'a>],
        known: &mut Known,
        ret: Expect,
    ) -> Option<&'a [Stmt<'a>]> {
        let mut out: Option<Vec<Stmt<'a>>> = None;
        for (i, s) in stmts.iter().enumerate() {
            let rewritten = self.stmt(s, known, ret);
            match (&mut out, rewritten) {
                (Some(v), Some(ns)) => v.push(ns),
                (Some(v), None) => v.push(s.clone()),
                (None, Some(ns)) => {
                    let mut v: Vec<Stmt<'a>> = stmts[..i].to_vec();
                    v.push(ns);
                    out = Some(v);
                }
                (None, None) => {}
            }
        }
        out.map(|v| self.stmt_arena.alloc_slice(v))
    }

    fn stmt(&mut self, s: &Stmt<'a>, known: &mut Known, ret: Expect) -> Option<Stmt<'a>> {
        match s {
            Stmt::Let {
                var,
                ty,
                value,
                mutable,
            } => {
                let expected = expect_of(*ty, self.interner);
                let (nv, kind) = self.expr(value, known, expected);
                known.bind(*var, if expected == Expect::Plain { kind } else { expected });
                nv.map(|v| Stmt::Let {
                    var: *var,
                    ty: *ty,
                    value: v,
                    mutable: *mutable,
                })
            }
            Stmt::Set { target, value } => {
                let (nv, kind) = self.expr(value, known, known.of(*target));
                known.bind(*target, kind);
                nv.map(|v| Stmt::Set {
                    target: *target,
                    value: v,
                })
            }
            Stmt::Return { value: Some(v) } => {
                let (nv, _) = self.expr(v, known, ret);
                nv.map(|nv| Stmt::Return { value: Some(nv) })
            }
            Stmt::Show { object, recipient } => {
                let (nv, _) = self.expr(object, known, Expect::Plain);
                nv.map(|o| Stmt::Show {
                    object: o,
                    recipient: *recipient,
                })
            }
            Stmt::Call { function, args } => {
                let nargs = self.args(*function, args, known)?;
                Some(Stmt::Call {
                    function: *function,
                    args: nargs,
                })
            }
            Stmt::FunctionDef {
                name, body, params, ..
            } => {
                // A fresh scope: parameters declared exact seed the known sets, and the names the
                // function binds are its own.
                let mut inner = Known::default();
                for (p, ty) in params {
                    inner.bind(*p, expect_of(Some(ty), self.interner));
                }
                let mut bound = self.globals.clone();
                bound.extend(Scope::of(std::iter::once(s)).bound);
                let outer = std::mem::replace(&mut self.bound, bound);
                let returns = self
                    .signatures
                    .get(name)
                    .map_or(Expect::Plain, |sig| sig.returns);
                let nb = self.block(body, &mut inner, returns);
                self.bound = outer;
                let mut fd = s.clone();
                if let Stmt::FunctionDef { body: b, .. } = &mut fd {
                    *b = nb?;
                }
                Some(fd)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                // Branches share the outer known sets (a binding in one branch is scoped to it, but
                // the conservative shared set never makes a floor division exact, so it is sound).
                let nc = self.expr(cond, known, Expect::Plain).0;
                let nt = self.block(then_block, known, ret);
                let ne = else_block.and_then(|e| self.block(e, known, ret));
                if nc.is_none() && nt.is_none() && ne.is_none() {
                    return None;
                }
                Some(Stmt::If {
                    cond: nc.unwrap_or(cond),
                    then_block: nt.unwrap_or(then_block),
                    else_block: ne.or(*else_block),
                })
            }
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                let nc = self.expr(cond, known, Expect::Plain).0;
                let nb = self.block(body, known, ret);
                if nc.is_none() && nb.is_none() {
                    return None;
                }
                Some(Stmt::While {
                    cond: nc.unwrap_or(cond),
                    body: nb.unwrap_or(body),
                    decreasing: *decreasing,
                })
            }
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                let nb = self.block(body, known, ret)?;
                Some(Stmt::Repeat {
                    pattern: pattern.clone(),
                    iterable: *iterable,
                    body: nb,
                })
            }
            _ => None,
        }
    }

    /// The arguments of a call to `function`: one at a parameter the program declares exact is that
    /// context, and every other is resolved for what its own exact operands make it.
    fn args(
        &mut self,
        function: Symbol,
        args: &[&'a Expr<'a>],
        known: &Known,
    ) -> Option<Vec<&'a Expr<'a>>> {
        let expects: Vec<Expect> = (0..args.len())
            .map(|i| {
                self.signatures
                    .get(&function)
                    .and_then(|sig| sig.params.get(i).copied())
                    .unwrap_or(Expect::Plain)
            })
            .collect();
        self.each(args, known, expects)
    }

    /// `args` resolved, each under its own expectation; `None` when none changed.
    fn each(
        &mut self,
        args: &[&'a Expr<'a>],
        known: &Known,
        expects: Vec<Expect>,
    ) -> Option<Vec<&'a Expr<'a>>> {
        let mut changed = false;
        let out: Vec<&'a Expr<'a>> = args
            .iter()
            .zip(expects)
            .map(|(a, e)| {
                let (na, _) = self.expr(a, known, e);
                changed |= na.is_some();
                na.unwrap_or(a)
            })
            .collect();
        changed.then_some(out)
    }

    /// The exact function `function` is in an exact context, when the program does not define its
    /// own function so named.
    fn exact_function(&self, function: Symbol) -> Option<Symbol> {
        if self.own.contains(&function) {
            return None;
        }
        let n = &self.names;
        if function == n.sqrt {
            Some(n.exact_sqrt)
        } else if function == n.exp {
            Some(n.exact_exp)
        } else if function == n.ln {
            Some(n.exact_ln)
        } else {
            None
        }
    }

    /// The exact constant a bare name is in a context expecting `expect`, when nothing binds it.
    fn exact_constant(&self, sym: Symbol, expect: Expect) -> Option<Symbol> {
        if self.bound.contains(&sym) {
            return None;
        }
        let n = &self.names;
        match expect {
            Expect::Field if n.pi.contains(&sym) => Some(n.exact_pi),
            Expect::Field if sym == n.e => Some(n.exact_e),
            Expect::Ordinal if n.omega.contains(&sym) => Some(n.exact_omega),
            _ => None,
        }
    }

    /// What an expression is known to yield, before any rewriting: an exact field value, an ordinal,
    /// or nothing in particular. It flows the expectation SIDEWAYS: an operation with an exact
    /// operand is exact, so its other operand's divisions must be exact too.
    fn kind(&self, expr: &Expr, known: &Known) -> Expect {
        let n = &self.names;
        match expr {
            Expr::Identifier(sym) => known.of(*sym),
            Expr::BinaryOp {
                op: BinaryOpKind::ExactDivide,
                ..
            } => Expect::Field,
            Expr::BinaryOp {
                op:
                    BinaryOpKind::Add
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Multiply
                    | BinaryOpKind::Divide,
                left,
                right,
            } => self.kind(left, known).join(self.kind(right, known)),
            Expr::BinaryOp {
                op: BinaryOpKind::Pow,
                left,
                right,
            } => match self.kind(right, known) {
                Expect::Field => Expect::Field,
                _ => self.kind(left, known),
            },
            Expr::Call { function, .. } if self.own.contains(function) => self
                .signatures
                .get(function)
                .map_or(Expect::Plain, |sig| sig.returns),
            Expr::Call { function, args } => {
                if n.exact_functions().contains(function)
                    || n.field_makers.contains(function)
                    || *function == n.exact_pi
                    || *function == n.exact_e
                {
                    Expect::Field
                } else if *function == n.exact_omega {
                    Expect::Ordinal
                } else if self.exact_function(*function).is_some()
                    && args.iter().any(|a| self.kind(a, known) == Expect::Field)
                {
                    Expect::Field
                } else {
                    Expect::Plain
                }
            }
            _ => Expect::Plain,
        }
    }

    /// Rewrite the arithmetic spine of `expr` under `expect`. Returns `(rewritten_or_none, kind)`.
    fn expr(
        &mut self,
        expr: &'a Expr<'a>,
        known: &Known,
        expect: Expect,
    ) -> (Option<&'a Expr<'a>>, Expect) {
        match expr {
            Expr::Identifier(sym) => match self.exact_constant(*sym, expect) {
                Some(constant) => (
                    Some(self.expr_arena.alloc(Expr::Call {
                        function: constant,
                        args: Vec::new(),
                    })),
                    expect,
                ),
                None => (None, known.of(*sym)),
            },
            Expr::Call { function, args } => {
                let kind = self.kind(expr, known);
                let exact = self
                    .exact_function(*function)
                    .filter(|_| expect == Expect::Field || kind == Expect::Field);
                let callee = exact.unwrap_or(*function);
                let nargs = if self.names.exact_functions().contains(&callee)
                    && !self.own.contains(&callee)
                {
                    self.each(args, known, vec![Expect::Field; args.len()])
                } else {
                    self.args(*function, args, known)
                };
                if exact.is_none() && nargs.is_none() {
                    return (None, kind);
                }
                let call = self.expr_arena.alloc(Expr::Call {
                    function: callee,
                    args: nargs.unwrap_or_else(|| args.clone()),
                });
                (Some(call), if exact.is_some() { Expect::Field } else { kind })
            }
            Expr::BinaryOp { op, left, right } => match op {
                BinaryOpKind::Add
                | BinaryOpKind::Subtract
                | BinaryOpKind::Multiply
                | BinaryOpKind::Divide
                | BinaryOpKind::ExactDivide
                | BinaryOpKind::Pow => {
                    // The operation's context is the one above it, or else the kind of EITHER operand
                    // (of the base, for `**`): that single fact is the effective expectation for both,
                    // so a sibling Rational (`c - 1/6`) makes the other side's divisions exact too.
                    let pow = matches!(op, BinaryOpKind::Pow);
                    let sides = if pow {
                        self.kind(left, known)
                    } else {
                        self.kind(left, known).join(self.kind(right, known))
                    };
                    let effective = if expect == Expect::Plain { sides } else { expect };
                    let op_field = effective == Expect::Field;
                    // LIGHTNING CHAINING (the `Opt::Comptime` / CTFE optimization): a
                    // fully-CONSTANT Rational subtree collapses to its closed form right
                    // here, at compile time — `1/3 + 1/6 + 1/2` becomes the single constant
                    // `1`, and `1/3 + 1/6` becomes one reduced `1/2`, instead of a chain of
                    // runtime divides and adds. `## No comptime` skips it (the chain still
                    // runs, still exact).
                    if op_field && self.fold_constants && !pow {
                        if let Some(rat) = const_rational(expr) {
                            if let Some(closed) = closed_form(self.expr_arena, &rat) {
                                return (Some(closed), Expect::Field);
                            }
                        }
                    }
                    let (nl, _) = self.expr(left, known, effective);
                    let nr = if pow {
                        self.expr(right, known, Expect::Plain).0
                    } else {
                        self.expr(right, known, effective).0
                    };
                    // A power whose exponent is itself an exact field value is the exact power, which takes a
                    // root when the exponent is a fraction: `2 ** (1/2)` is `√2`. A float written as the base
                    // keeps the float power it names.
                    if pow
                        && self.kind(right, known) == Expect::Field
                        && !matches!(left, Expr::Literal(Literal::Float(_)))
                        && !self.own.contains(&self.names.exact_power)
                    {
                        let call = self.expr_arena.alloc(Expr::Call {
                            function: self.names.exact_power,
                            args: vec![nl.unwrap_or(left), nr.unwrap_or(right)],
                        });
                        return (Some(call), Expect::Field);
                    }
                    let new_op = if matches!(op, BinaryOpKind::Divide) && op_field {
                        BinaryOpKind::ExactDivide
                    } else {
                        *op
                    };
                    let kind = if matches!(new_op, BinaryOpKind::ExactDivide) {
                        Expect::Field
                    } else {
                        effective
                    };
                    let changed = nl.is_some() || nr.is_some() || new_op != *op;
                    let result = changed.then(|| {
                        &*self.expr_arena.alloc(Expr::BinaryOp {
                            op: new_op,
                            left: nl.unwrap_or(left),
                            right: nr.unwrap_or(right),
                        })
                    });
                    (result, kind)
                }
                // A comparison with an exact operand is an exact context for the other operand.
                BinaryOpKind::Lt
                | BinaryOpKind::Gt
                | BinaryOpKind::LtEq
                | BinaryOpKind::GtEq
                | BinaryOpKind::Eq
                | BinaryOpKind::NotEq => {
                    let side = self.kind(left, known).join(self.kind(right, known));
                    self.pair(*op, left, right, known, side, side)
                }
                BinaryOpKind::And | BinaryOpKind::Or => {
                    self.pair(*op, left, right, known, Expect::Plain, Expect::Plain)
                }
                // Modulo / bitwise / shift do not carry an exact expectation to an operand, and
                // their result is not an exact field value.
                _ => (None, Expect::Plain),
            },
            Expr::Not { operand } => match self.expr(operand, known, Expect::Plain).0 {
                Some(o) => (
                    Some(self.expr_arena.alloc(Expr::Not { operand: o })),
                    Expect::Plain,
                ),
                None => (None, Expect::Plain),
            },
            _ => (None, Expect::Plain),
        }
    }

    /// `left op right` with each operand resolved under its expectation; the result is no exact value.
    fn pair(
        &mut self,
        op: BinaryOpKind,
        left: &'a Expr<'a>,
        right: &'a Expr<'a>,
        known: &Known,
        left_expect: Expect,
        right_expect: Expect,
    ) -> (Option<&'a Expr<'a>>, Expect) {
        let nl = self.expr(left, known, left_expect).0;
        let nr = self.expr(right, known, right_expect).0;
        let rewritten = (nl.is_some() || nr.is_some()).then(|| {
            &*self.expr_arena.alloc(Expr::BinaryOp {
                op,
                left: nl.unwrap_or(left),
                right: nr.unwrap_or(right),
            })
        });
        (rewritten, Expect::Plain)
    }

}

/// The closed-form AST for an evaluated constant `Rational`: a bare `Number` when
/// it is whole, else a single reduced `num / den` (`ExactDivide`). `None` when a
/// term escapes i64 (then the chain is left to run, still exact, at runtime).
fn closed_form<'a>(expr_arena: &'a Arena<Expr<'a>>, rat: &Rational) -> Option<&'a Expr<'a>> {
    if let Some(n) = rat.to_i64() {
        return Some(expr_arena.alloc(Expr::Literal(Literal::Number(n))));
    }
    let num = rat.numerator().to_i64()?;
    let den = rat.denominator().to_i64()?;
    let n = expr_arena.alloc(Expr::Literal(Literal::Number(num)));
    let d = expr_arena.alloc(Expr::Literal(Literal::Number(den)));
    Some(expr_arena.alloc(Expr::BinaryOp {
        op: BinaryOpKind::ExactDivide,
        left: n,
        right: d,
    }))
}

/// Evaluate a CONSTANT arithmetic expression to its exact `Rational` — the closed form
/// of a `1/3 + 1/6 + …` chain. `None` if any leaf is non-constant (a variable, a call,
/// …) or a sub-divide is by zero. Both `Divide` and `ExactDivide` evaluate exactly here;
/// the caller only folds where it has already established the result is a `Rational`, so
/// an integer `7 / 2` (which floors) is never reached.
fn const_rational(expr: &Expr) -> Option<Rational> {
    match expr {
        Expr::Literal(Literal::Number(n)) => Some(Rational::from_i64(*n)),
        Expr::BinaryOp { op, left, right } => {
            let l = const_rational(left)?;
            let r = const_rational(right)?;
            match op {
                BinaryOpKind::Add => Some(l.add(&r)),
                BinaryOpKind::Subtract => Some(l.sub(&r)),
                BinaryOpKind::Multiply => Some(l.mul(&r)),
                BinaryOpKind::Divide | BinaryOpKind::ExactDivide => l.div(&r),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/resolve_division/tests.rs"]
mod tests;
