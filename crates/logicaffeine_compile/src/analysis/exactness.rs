//! Which integers a program can make outgrow a machine word — the decision, apart from any one form of the
//! program.
//!
//! A LOGOS `Int` is unbounded: the reference tier promotes a value past i64 to a big integer. A compiled tier keeps
//! an `Int` in a machine word wherever nothing makes it outgrow one and holds it exact wherever something can. Every
//! tier must agree on which, or one prints a big integer where another prints a wrapped word. So the rules live
//! here once, over a graph any form of the program can describe: the bytecode ([`crate::vm::exactness`], which
//! bytecode C and WebAssembly ask) and the syntax tree the AST tiers compile.
//!
//! A [`Def`] is one value the program makes, classified by the [`Operation`] that makes it; an [`Input`] is a value
//! it is computed from, with the operand position it fills and how ([`Flow`]). A *web* joins the definitions that
//! are one variable, so every definition of it agrees on the representation. A web is exact when one of its
//! definitions is:
//!
//! - arithmetic without a complete bound proving its result fits a machine word;
//! - carried beyond i64 by the program's constants;
//! - already exact where it comes from (an exact result, parameter, global, field), or computed exact;
//! - computed from an exact web — copied, added, subtracted, multiplied, raised, combined bitwise; divided
//!   when the dividend is exact; reduced when the modulus is. A constant bound (x % 7, x & 255)
//!   keeps the confined result in a word.
//!
//! A linear update is safe in a word only with a bound on its starting value and its accumulated change.
//! Unknown operands and loop-carried values cannot supply that proof. A backend can still run a checked
//! word version and transfer into the exact representation on overflow.

use std::collections::{HashMap, HashSet};

use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

use crate::analysis::{FieldType, TypeDef, TypeRegistry};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};

/// How a definition makes its value, as far as growth is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    /// An integer constant.
    Constant(i128),
    /// The value of operand 0.
    Copy,
    /// The sum or difference of operands 0 and 1; `doubles` when both are the same value (`x + x`).
    Sum { doubles: bool },
    /// The product of operands 0 and 1.
    Product,
    /// Operand 0 raised to operand 1.
    Power,
    /// A quotient of operand 0 (the dividend): no larger than it.
    Quotient,
    /// A remainder modulo operand 0 (the modulus): smaller than it.
    Remainder,
    /// A value a constant confines to its magnitude whatever its operands are (`x % c`, `x & c` for `c >= 0`): it
    /// carries no exactness from them.
    Bounded(i128),
    /// A bitwise combination of operands 0 and 1 (`^`, `|`, `&`, a shift): exact when an operand is, never growing
    /// on its own.
    Bitwise,
    /// Anything else: what it makes is no constant's concern (a parameter, an element, a call's result).
    Opaque,
}

/// How a definition's value relates to one it is computed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// An operand of a product or a power.
    Product,
    /// An operand of a sum or a difference.
    Sum,
    /// The value itself.
    Copy,
    /// The dividend of a quotient: the quotient is no larger.
    Quotient,
    /// The modulus of a remainder: the remainder is smaller.
    Remainder,
    /// A function's returned value, reaching the result of a call it makes to itself.
    Returned,
    /// A `mutable` parameter's value at a return, reaching the parameter on the caller's next lend.
    LentBack,
    /// A value stored somewhere (a global, a field), reaching every read of it.
    Stored,
}

impl Flow {
    /// Whether the flow passes through a call: a value fed back only this way grows by no more than the calls that
    /// carry it.
    fn through_a_call(self) -> bool {
        matches!(self, Flow::Returned | Flow::LentBack)
    }
}

/// One value a definition is computed from: the operand position it fills, the definition that made it, how.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Input {
    pub(crate) slot: u8,
    pub(crate) from: usize,
    pub(crate) flow: Flow,
}

/// One value the program makes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Def {
    pub(crate) operation: Operation,
    /// The form of the program computes it exact already (a value the exact tower itself makes).
    pub(crate) computed_exact: bool,
    /// It arrives exact from where it comes (an exact result, parameter, global or field).
    pub(crate) arrives_exact: bool,
}

/// The definitions of a program (or a region of one), what each is computed from, and which are one variable.
#[derive(Debug, Default)]
pub(crate) struct Graph {
    pub(crate) defs: Vec<Def>,
    pub(crate) inputs: Vec<Vec<Input>>,
    /// Pairs of definitions that are one variable.
    pub(crate) same_variable: Vec<(usize, usize)>,
}

impl Graph {
    /// Add a definition, answering its index.
    pub(crate) fn def(
        &mut self,
        operation: Operation,
        computed_exact: bool,
        arrives_exact: bool,
    ) -> usize {
        self.defs.push(Def {
            operation,
            computed_exact,
            arrives_exact,
        });
        self.inputs.push(Vec::new());
        self.defs.len() - 1
    }

    /// `to` is computed from `from` in operand position `slot`, by `flow`.
    pub(crate) fn input(&mut self, to: usize, slot: u8, from: usize, flow: Flow) {
        self.inputs[to].push(Input { slot, from, flow });
    }

    /// `a` and `b` are one variable.
    pub(crate) fn join(&mut self, a: usize, b: usize) {
        self.same_variable.push((a, b));
    }
}

/// What the decision says of each definition.
#[derive(Debug, Default)]
pub(crate) struct Decision {
    /// The definition's integer is held exact.
    pub(crate) exact: Vec<bool>,
    /// It is exact from the start — computed exact, or computed from one that is — rather than only because it can
    /// grow: a form of the program that runs on machine words until a value outgrows one ([`crate::vm::exactness`])
    /// runs these exact throughout.
    pub(crate) computed: Vec<bool>,
}

/// A disjoint-set forest over definition indices.
struct Webs {
    parent: Vec<usize>,
}

impl Webs {
    fn new(n: usize) -> Self {
        Webs {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut at = x;
        while self.parent[at] != root {
            let next = self.parent[at];
            self.parent[at] = root;
            at = next;
        }
        root
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra] = rb;
        }
    }
}

/// The largest magnitude i64 holds.
const WORD_MAX: i128 = i64::MAX as i128;

/// `base` raised to `exp`, saturating past the i64 range.
fn saturating_pow(base: i128, exp: i128) -> i128 {
    if exp <= 0 {
        return 1;
    }
    if base <= 1 {
        return base;
    }
    let mut acc: i128 = 1;
    for _ in 0..exp.min(128) {
        acc = acc.saturating_mul(base);
        if acc > WORD_MAX {
            return WORD_MAX + 1;
        }
    }
    acc
}

/// Decide which definitions of `graph` are exact.
pub(crate) fn decide(graph: &Graph) -> Decision {
    let n = graph.defs.len();
    if n == 0 {
        return Decision::default();
    }
    // The magnitudes the constants give each definition: a hint that counts what cannot be known as nothing (so
    // what it says past i64 the constants alone put there), and a bound that knows nothing of it.
    let (hint, bound) = magnitudes(graph);

    let mut grown: Vec<bool> = graph
        .defs
        .iter()
        .map(|d| d.computed_exact || d.arrives_exact)
        .collect();
    for (d, def) in graph.defs.iter().enumerate() {
        // Arithmetic with an unknown operand or a loop-carried value has no complete word bound.
        // The backend may keep a checked word version, but its exact path must exist before overflow.
        let grows = match def.operation {
            Operation::Sum { .. } | Operation::Product | Operation::Power | Operation::Quotient => {
                bound[d].is_none_or(|b| b > WORD_MAX)
            }
            _ => false,
        };
        if grows || hint[d] > WORD_MAX {
            grown[d] = true;
        }
    }

    // Close over webs and over what is computed from exact values, until nothing changes: once from the values the
    // form computes exact, once from those and the ones that arrive exact or grow.
    let mut webs = Webs::new(n);
    for &(a, b) in &graph.same_variable {
        webs.union(a, b);
    }
    let web_of: Vec<usize> = (0..n).map(|d| webs.find(d)).collect();
    let close = |mut exact: Vec<bool>| -> Vec<bool> {
        loop {
            let exact_webs: HashSet<usize> =
                (0..n).filter(|d| exact[*d]).map(|d| web_of[d]).collect();
            let mut more = false;
            for d in 0..n {
                if !exact[d]
                    && (exact_webs.contains(&web_of[d])
                        || graph.inputs[d].iter().any(|i| exact[i.from]))
                {
                    exact[d] = true;
                    more = true;
                }
            }
            if !more {
                return exact;
            }
        }
    };
    let computed = close(graph.defs.iter().map(|d| d.computed_exact).collect());
    let exact = close(grown);
    Decision { exact, computed }
}

/// Each definition's magnitude as the constants that feed it make it: `(hint, bound)`. The hint counts a value
/// nothing fixes — a parameter, a call's result, an element, a loop-carried value — as zero; the bound does not
/// know it. Both saturate past i64.
fn magnitudes(graph: &Graph) -> (Vec<i128>, Vec<Option<i128>>) {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        Visiting,
        Done,
    }
    let n = graph.defs.len();
    let mut hint = vec![0i128; n];
    let mut bound: Vec<Option<i128>> = vec![None; n];
    let mut state = vec![State::Unvisited; n];
    for root in 0..n {
        if state[root] != State::Unvisited {
            continue;
        }
        let mut stack: Vec<(usize, bool)> = vec![(root, false)];
        while let Some((d, expanded)) = stack.pop() {
            if !expanded {
                if state[d] != State::Unvisited {
                    continue;
                }
                state[d] = State::Visiting;
                stack.push((d, true));
                for i in &graph.inputs[d] {
                    if state[i.from] == State::Unvisited {
                        stack.push((i.from, false));
                    }
                }
                continue;
            }
            // A value an input is still being computed through is loop-carried: nothing fixes it.
            let slot = |s: u8| -> (i128, Option<i128>) {
                let mut h = 0i128;
                let mut b: Option<i128> = Some(0);
                let mut any = false;
                for i in graph.inputs[d].iter().filter(|i| i.slot == s) {
                    any = true;
                    let (ih, ib) = if state[i.from] == State::Done {
                        (hint[i.from], bound[i.from])
                    } else {
                        (0, None)
                    };
                    h = h.max(ih);
                    b = match (b, ib) {
                        (Some(x), Some(y)) => Some(x.max(y)),
                        _ => None,
                    };
                }
                if any {
                    (h, b)
                } else {
                    (0, None)
                }
            };
            let (h, b) = match graph.defs[d].operation {
                Operation::Constant(v) => (v.abs(), Some(v.abs())),
                Operation::Copy | Operation::Quotient => slot(0),
                Operation::Remainder => (0, slot(0).1),
                Operation::Bounded(m) => (0, Some(m.abs())),
                Operation::Sum { .. } => {
                    let ((h0, b0), (h1, b1)) = (slot(0), slot(1));
                    (
                        h0.saturating_add(h1),
                        b0.zip(b1).map(|(x, y)| x.saturating_add(y)),
                    )
                }
                Operation::Product => {
                    let ((h0, b0), (h1, b1)) = (slot(0), slot(1));
                    (
                        h0.saturating_mul(h1),
                        b0.zip(b1).map(|(x, y)| x.saturating_mul(y)),
                    )
                }
                Operation::Power => {
                    let ((h0, b0), (h1, b1)) = (slot(0), slot(1));
                    (
                        saturating_pow(h0, h1),
                        b0.zip(b1).map(|(x, y)| saturating_pow(x, y)),
                    )
                }
                Operation::Bitwise | Operation::Opaque => (0, None),
            };
            hint[d] = h.min(WORD_MAX + 1);
            bound[d] = b.map(|x| x.min(WORD_MAX + 1));
            state[d] = State::Done;
        }
    }
    (hint, bound)
}

/// What an AST tier knows that the decision does not: its own typing, and the slots it has settled so far.
pub(crate) trait AstHost {
    /// `var` holds no unbounded `Int` — a Real, a fixed-width word ([`word_vars`]): it is never exact, and nothing
    /// it holds makes another value exact.
    fn holds_no_int(&self, var: Symbol) -> bool;
    /// `var` arrives exact: an exact parameter, a program binding exact elsewhere, a variable lent to an exact
    /// `mutable` parameter.
    fn var_arrives_exact(&self, var: Symbol) -> bool;
    /// A call to `function` answers an exact integer.
    fn call_arrives_exact(&self, function: Symbol) -> bool;
    /// A call through a function value answers an exact integer.
    fn value_call_arrives_exact(&self) -> bool;
    /// A read of a field named `field` answers an exact integer.
    fn field_arrives_exact(&self, field: Symbol) -> bool;
    /// Component `k` (1-based) of what `collection` evaluates to is exact.
    fn component_arrives_exact(&self, collection: &Expr, k: i64) -> bool;
    /// The field an `Inspect` arm binds is exact.
    fn arm_field_arrives_exact(
        &self,
        enum_name: Option<Symbol>,
        variant: Symbol,
        field: Symbol,
    ) -> bool;
    /// `function` is the integer power builtin (`pow(a, b)`).
    fn is_power(&self, function: Symbol) -> bool;
    /// The value of `e` provably lies within i64 wherever the body computes it (a range the tier's oracle proves,
    /// a loop's guard bounding it among them): it never outgrows a word, whatever feeds it.
    fn fits_word(&self, e: &Expr) -> bool;
}

/// What the decision says of one body of a syntax tree: its exact variables, and whether a value it returns is.
#[derive(Debug, Default)]
pub(crate) struct BodyDecision {
    pub(crate) vars: HashSet<Symbol>,
    pub(crate) returns: bool,
}

/// A body's graph: one definition per variable, fed by every binding of it, and one per expression. A variable that
/// holds no `Int` is a definition nothing feeds.
struct BodyGraph<'h> {
    graph: Graph,
    vars: HashMap<Symbol, usize>,
    no_int: HashSet<usize>,
    returns: Vec<usize>,
    self_calls: Vec<usize>,
    function: Option<Symbol>,
    host: &'h dyn AstHost,
}

impl BodyGraph<'_> {
    fn var(&mut self, sym: Symbol) -> usize {
        if let Some(&d) = self.vars.get(&sym) {
            return d;
        }
        let d = if self.host.holds_no_int(sym) {
            let d = self.graph.def(Operation::Opaque, false, false);
            self.no_int.insert(d);
            d
        } else {
            self.graph
                .def(Operation::Copy, false, self.host.var_arrives_exact(sym))
        };
        self.vars.insert(sym, d);
        d
    }

    /// Variable `v` takes the value `from` holds.
    fn bind(&mut self, v: usize, from: usize) {
        if !self.no_int.contains(&v) {
            self.graph.input(v, 0, from, Flow::Copy);
        }
    }

    fn opaque(&mut self, arrives_exact: bool) -> usize {
        self.graph.def(Operation::Opaque, false, arrives_exact)
    }

    fn expr(&mut self, e: &Expr) -> usize {
        match e {
            Expr::Literal(Literal::Number(v)) => {
                self.graph
                    .def(Operation::Constant(*v as i128), false, false)
            }
            Expr::Identifier(sym) => self.var(*sym),
            Expr::BinaryOp { op, left, right } => {
                let d = self.binary(*op, left, right);
                if self.host.fits_word(e) {
                    self.graph.defs[d].operation = Operation::Bounded(WORD_MAX);
                    self.graph.inputs[d].clear();
                }
                d
            }
            Expr::Call { function, args } if args.len() == 2 && self.host.is_power(*function) => {
                let (base, exp) = (self.expr(args[0]), self.expr(args[1]));
                let d = self.graph.def(Operation::Power, false, false);
                self.graph.input(d, 0, base, Flow::Product);
                self.graph.input(d, 1, exp, Flow::Product);
                d
            }
            Expr::Call { function, args } => {
                for a in args.iter() {
                    self.expr(a);
                }
                let d = self.opaque(self.host.call_arrives_exact(*function));
                if Some(*function) == self.function {
                    self.self_calls.push(d);
                }
                d
            }
            Expr::CallExpr { args, .. } => {
                for a in args.iter() {
                    self.expr(a);
                }
                self.opaque(self.host.value_call_arrives_exact())
            }
            Expr::FieldAccess { field, .. } => self.opaque(self.host.field_arrives_exact(*field)),
            Expr::Index {
                collection,
                index: Expr::Literal(Literal::Number(k)),
            } => self.opaque(self.host.component_arrives_exact(collection, *k)),
            _ => self.opaque(false),
        }
    }

    fn binary(&mut self, op: BinaryOpKind, left: &Expr, right: &Expr) -> usize {
        let text = |e: &Expr| {
            matches!(
                e,
                Expr::Literal(Literal::Text(_)) | Expr::InterpolatedString(_)
            )
        };
        let literal = |e: &Expr| match e {
            Expr::Literal(Literal::Number(c)) => Some(*c as i128),
            _ => None,
        };
        let same = matches!((left, right), (Expr::Identifier(a), Expr::Identifier(b)) if a == b);
        let (l, r) = (self.expr(left), self.expr(right));
        // The operation, and the operand each slot reads, by how.
        let (operation, operands): (Operation, Vec<(usize, Flow)>) = match op {
            BinaryOpKind::Add if !text(left) && !text(right) => (
                Operation::Sum { doubles: same },
                vec![(l, Flow::Sum), (r, Flow::Sum)],
            ),
            BinaryOpKind::Subtract => (
                Operation::Sum { doubles: false },
                vec![(l, Flow::Sum), (r, Flow::Sum)],
            ),
            BinaryOpKind::Multiply => (
                Operation::Product,
                vec![(l, Flow::Product), (r, Flow::Product)],
            ),
            BinaryOpKind::Pow => (
                Operation::Power,
                vec![(l, Flow::Product), (r, Flow::Product)],
            ),
            BinaryOpKind::Divide | BinaryOpKind::FloorDivide => {
                (Operation::Quotient, vec![(l, Flow::Quotient)])
            }
            BinaryOpKind::Modulo => match literal(right) {
                Some(c) if c != 0 => (Operation::Bounded(c), Vec::new()),
                _ => (Operation::Remainder, vec![(r, Flow::Remainder)]),
            },
            BinaryOpKind::BitAnd => match literal(right).or_else(|| literal(left)) {
                Some(c) if c >= 0 => (Operation::Bounded(c), Vec::new()),
                _ => (Operation::Bitwise, vec![(l, Flow::Copy), (r, Flow::Copy)]),
            },
            BinaryOpKind::BitXor | BinaryOpKind::BitOr | BinaryOpKind::Shl | BinaryOpKind::Shr => {
                (Operation::Bitwise, vec![(l, Flow::Copy), (r, Flow::Copy)])
            }
            _ => (Operation::Opaque, Vec::new()),
        };
        let d = self.graph.def(operation, false, false);
        for (slot, (from, flow)) in operands.into_iter().enumerate() {
            self.graph.input(d, slot as u8, from, flow);
        }
        d
    }
}

/// Decide which variables of `body` — the body of `function`, or Main — are exact, and whether a value it returns
/// is: every binding feeds its variable, every expression is a definition, and a call the body makes to its own
/// function is fed by every value it returns. A nested function's body and a closure's are scopes of their own.
pub(crate) fn body_exactness(
    body: &[Stmt],
    function: Option<Symbol>,
    host: &dyn AstHost,
) -> BodyDecision {
    let mut g = BodyGraph {
        graph: Graph::default(),
        vars: HashMap::new(),
        no_int: HashSet::new(),
        returns: Vec::new(),
        self_calls: Vec::new(),
        function,
        host,
    };
    walk_nodes(body, |node| match node {
        Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => Walk::Past,
        Visited::Stmt(Stmt::Let { var, value, .. }) => {
            let (root, v) = (g.expr(value), g.var(*var));
            g.bind(v, root);
            Walk::Into
        }
        Visited::Stmt(Stmt::Set { target, value }) => {
            let (root, v) = (g.expr(value), g.var(*target));
            g.bind(v, root);
            Walk::Into
        }
        Visited::Stmt(Stmt::Swap { a, b }) => {
            let (va, vb) = (g.var(*a), g.var(*b));
            g.bind(va, vb);
            g.bind(vb, va);
            Walk::Into
        }
        Visited::Stmt(Stmt::Return { value: Some(value) }) => {
            let root = g.expr(value);
            g.returns.push(root);
            Walk::Into
        }
        Visited::Stmt(Stmt::Inspect { arms, .. }) => {
            for arm in arms.iter() {
                if let Some(variant) = arm.variant {
                    for (field, binding) in &arm.bindings {
                        let v = g.var(*binding);
                        if !g.no_int.contains(&v)
                            && g.host
                                .arm_field_arrives_exact(arm.enum_name, variant, *field)
                        {
                            g.graph.defs[v].arrives_exact = true;
                        }
                    }
                }
            }
            Walk::Into
        }
        _ => Walk::Into,
    });
    for &call in &g.self_calls {
        for &r in &g.returns {
            g.graph.input(call, 0, r, Flow::Returned);
        }
    }
    let decision = decide(&g.graph);
    BodyDecision {
        vars: g
            .vars
            .iter()
            .filter(|(_, d)| decision.exact[**d])
            .map(|(s, _)| *s)
            .collect(),
        returns: g.returns.iter().any(|r| decision.exact[*r]),
    }
}

/// A fixed-width word type's width: `Word8` … `Word64`.
fn word_width(name: &str) -> Option<u8> {
    match name {
        "Word8" => Some(8),
        "Word16" => Some(16),
        "Word32" => Some(32),
        "Word64" => Some(64),
        _ => None,
    }
}

/// What a declared type is, as far as words go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WordShape {
    /// A word of this width.
    Word(u8),
    /// A sequence of words of this width.
    Seq(u8),
}

fn is_sequence_name(name: &str) -> bool {
    matches!(name, "Seq" | "List" | "Vec")
}

/// The word shape a declared type names, if it names a word or a sequence of words.
pub(crate) fn declared_word(ty: &TypeExpr, interner: &Interner) -> Option<WordShape> {
    match ty {
        TypeExpr::Primitive(s) | TypeExpr::Named(s) => {
            word_width(interner.resolve(*s)).map(WordShape::Word)
        }
        TypeExpr::Generic { base, params } if is_sequence_name(interner.resolve(*base)) => {
            match params.first() {
                Some(TypeExpr::Primitive(s) | TypeExpr::Named(s)) => {
                    word_width(interner.resolve(*s)).map(WordShape::Seq)
                }
                _ => None,
            }
        }
        TypeExpr::Refinement { base, .. } => declared_word(base, interner),
        TypeExpr::Mutable { inner } => declared_word(inner, interner),
        _ => None,
    }
}

/// The word shape a record field's declared type names.
fn field_word(ty: &FieldType, interner: &Interner) -> Option<WordShape> {
    match ty {
        FieldType::Primitive(s) | FieldType::Named(s) => {
            word_width(interner.resolve(*s)).map(WordShape::Word)
        }
        FieldType::Generic { base, params } if is_sequence_name(interner.resolve(*base)) => {
            match params.first() {
                Some(FieldType::Primitive(s) | FieldType::Named(s)) => {
                    word_width(interner.resolve(*s)).map(WordShape::Seq)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Every user function of `stmts`, with the word shape of its declared result when that is a word or a sequence
/// of words: what a call to it answers.
pub(crate) fn word_results(
    stmts: &[Stmt],
    interner: &Interner,
) -> HashMap<Symbol, Option<WordShape>> {
    let mut results = HashMap::new();
    walk_nodes(stmts, |node| match node {
        Visited::Stmt(Stmt::FunctionDef {
            name, return_type, ..
        }) => {
            results.insert(*name, return_type.and_then(|t| declared_word(t, interner)));
            Walk::Into
        }
        _ => Walk::Into,
    });
    results
}

/// The words and the sequences of words of one body, settled to a fixpoint.
struct Words<'p> {
    results: &'p HashMap<Symbol, Option<WordShape>>,
    registry: &'p TypeRegistry,
    interner: &'p Interner,
    words: HashMap<Symbol, u8>,
    seqs: HashMap<Symbol, u8>,
}

impl Words<'_> {
    /// The shape every record field named `field` declares, when they all declare one.
    fn field(&self, field: Symbol) -> Option<WordShape> {
        let mut shapes = self.registry.iter_types().flat_map(|(_, def)| match def {
            TypeDef::Struct { fields, .. } => fields
                .iter()
                .filter(|f| f.name == field)
                .map(|f| field_word(&f.ty, self.interner))
                .collect(),
            _ => Vec::new(),
        });
        let first = shapes.next()??;
        shapes.all(|s| s == Some(first)).then_some(first)
    }

    /// The shape a call to `function` answers.
    fn call(&self, function: Symbol, args: &[&Expr]) -> Option<WordShape> {
        match self.results.get(&function) {
            Some(shape) => *shape,
            None => {
                let id =
                    crate::semantics::builtins::builtin_from_name(self.interner.resolve(function))?;
                id.word_result(args.first().and_then(|a| self.word(a)))
                    .map(WordShape::Word)
                    .or_else(|| id.word_seq_result().map(WordShape::Seq))
            }
        }
    }

    /// The width of the word `e` evaluates to, if it evaluates to a word.
    fn word(&self, e: &Expr) -> Option<u8> {
        match e {
            Expr::Identifier(s) => self.words.get(s).copied(),
            Expr::Call { function, args } => match self.call(*function, args) {
                Some(WordShape::Word(w)) => Some(w),
                _ => None,
            },
            Expr::BinaryOp { op, left, right }
                if matches!(
                    op,
                    BinaryOpKind::Add
                        | BinaryOpKind::Subtract
                        | BinaryOpKind::Multiply
                        | BinaryOpKind::Divide
                        | BinaryOpKind::FloorDivide
                        | BinaryOpKind::Modulo
                        | BinaryOpKind::BitAnd
                        | BinaryOpKind::BitOr
                        | BinaryOpKind::BitXor
                        | BinaryOpKind::Shl
                        | BinaryOpKind::Shr
                ) =>
            {
                self.word(left).or_else(|| self.word(right))
            }
            Expr::Index { collection, .. } => self.seq(collection),
            Expr::FieldAccess { field, .. } => match self.field(*field) {
                Some(WordShape::Word(w)) => Some(w),
                _ => None,
            },
            _ => None,
        }
    }

    /// The width of the words of the sequence `e` evaluates to, if it evaluates to a sequence of words.
    fn seq(&self, e: &Expr) -> Option<u8> {
        match e {
            Expr::Identifier(s) => self.seqs.get(s).copied(),
            Expr::Call { function, args } => match self.call(*function, args) {
                Some(WordShape::Seq(w)) => Some(w),
                _ => None,
            },
            Expr::New {
                type_name,
                type_args,
                ..
            } if is_sequence_name(self.interner.resolve(*type_name)) => match type_args.first() {
                Some(TypeExpr::Primitive(s) | TypeExpr::Named(s)) => {
                    word_width(self.interner.resolve(*s))
                }
                _ => None,
            },
            Expr::List(items) => items.first().and_then(|item| self.word(item)),
            Expr::FieldAccess { field, .. } => match self.field(*field) {
                Some(WordShape::Seq(w)) => Some(w),
                _ => None,
            },
            _ => None,
        }
    }

    /// Variable `var` takes a value of shape `shape`; whether that told it something new.
    fn learn(&mut self, var: Symbol, shape: Option<WordShape>) -> bool {
        match shape {
            Some(WordShape::Word(w)) => self.words.insert(var, w).is_none(),
            Some(WordShape::Seq(w)) => self.seqs.insert(var, w).is_none(),
            None => false,
        }
    }
}

/// The variables of `body` that hold a fixed-width word (`Word8` … `Word64`): a parameter or binding declared one,
/// and every variable bound a word — a word builtin's result (`word32(n)`, `rotr(w, 7)`), a call to a function
/// declared to answer one ([`word_results`]), an element of a sequence of words, a word field, arithmetic or a
/// bitwise combination with a word operand. A word's arithmetic wraps at its width, so a word is never exact,
/// however it grows. `params` are the body's declared parameters; a nested function's body and a closure's are
/// scopes of their own.
pub(crate) fn word_vars(
    body: &[Stmt],
    params: &[(Symbol, &TypeExpr)],
    results: &HashMap<Symbol, Option<WordShape>>,
    registry: &TypeRegistry,
    interner: &Interner,
) -> HashSet<Symbol> {
    let mut w = Words {
        results,
        registry,
        interner,
        words: HashMap::new(),
        seqs: HashMap::new(),
    };
    for (p, ty) in params {
        w.learn(*p, declared_word(ty, interner));
    }
    loop {
        let mut changed = false;
        walk_nodes(body, |node| match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                Walk::Past
            }
            Visited::Stmt(Stmt::Let { var, value, ty, .. }) => {
                let shape = ty
                    .and_then(|t| declared_word(t, interner))
                    .or_else(|| w.word(value).map(WordShape::Word))
                    .or_else(|| w.seq(value).map(WordShape::Seq));
                changed |= w.learn(*var, shape);
                Walk::Into
            }
            Visited::Stmt(Stmt::Set { target, value }) => {
                let shape = w
                    .word(value)
                    .map(WordShape::Word)
                    .or_else(|| w.seq(value).map(WordShape::Seq));
                changed |= w.learn(*target, shape);
                Walk::Into
            }
            Visited::Stmt(Stmt::Repeat {
                pattern: Pattern::Identifier(x),
                iterable,
                ..
            }) => {
                let shape = w.seq(iterable).map(WordShape::Word);
                changed |= w.learn(*x, shape);
                Walk::Into
            }
            Visited::Stmt(Stmt::Inspect { arms, .. }) => {
                for arm in arms.iter() {
                    let Some(variant) = arm.variant else { continue };
                    let declared = match arm.enum_name {
                        Some(e) => registry.variant_in(e, variant),
                        None => registry.find_variant(variant).map(|(_, v)| v),
                    };
                    for (field, binding) in &arm.bindings {
                        let shape = declared
                            .and_then(|v| v.fields.iter().find(|f| f.name == *field))
                            .and_then(|f| field_word(&f.ty, interner));
                        changed |= w.learn(*binding, shape);
                    }
                }
                Walk::Into
            }
            _ => Walk::Into,
        });
        if !changed {
            break;
        }
    }
    w.words.into_keys().collect()
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/analysis/exactness/tests.rs"]
mod tests;
