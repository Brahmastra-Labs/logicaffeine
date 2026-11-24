//! Points-to facts of an imperative program, and the analysis Whaley and Lam write in four rules (PLDI 2004) run over
//! them by [`logicaffeine_proof::datalog`].
//!
//! [`points_to_facts`] walks the program once, flow-insensitively, and states what each statement does to the heap as
//! Datalog facts over names: a variable is `function.name` (a compiler temporary `function.%n`), an allocation site
//! `Hn`, a call site `Sn`, an argument position `Zk`, a collection's elements the field `Elem`:
//!
//! - `VP0(v, h)`: `v` is bound to the object site `h` allocates (`a new Point`, a list or tuple literal, `Some(x)`, a
//!   variant, a copy, a slice, the result of a builtin, taken as fresh), and `Declares(f, v)` for every variable
//!   function `f` binds;
//! - `Assign(v1, v2)`: `v1` takes the object `v2` holds (`Let`, `Set`, a moved value);
//! - `Store(v1, f, v2)` / `Load(v1, f, v2)`: a field or element write or read (`Set field`, `Push`, `Set item`, `Add`;
//!   a field access, `item i of`, `Pop … into`, a `Repeat` variable, an `Inspect` binding);
//! - `Calls(f, s, g)`, `Actual(s, Zk, v)`, `Formal(g, Zk, p)`, `Returns(g, v)`, `Receives(s, r)`: a call of a function
//!   the program defines, its arguments, the callee's parameters, what it returns and where the result goes.
//!
//! A value that is not an object (a number, a truth value, a Text, which is immutable) states nothing, and a closure's
//! body and a call through a closure are not read. [`points_to`] evaluates the facts under [`CONTEXT_INSENSITIVE`] or,
//! numbering every call path as its own context, [`CONTEXT_SENSITIVE`], and answers what each variable may point to.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use logicaffeine_base::Interner;
use logicaffeine_language::ast::stmt::{Expr, MatchArm, Pattern, Stmt};
use logicaffeine_proof::datalog::{evaluate, evaluate_bdd_with_contexts, Atom, Contexts, Program, Term};

/// Whaley and Lam's four rules over the facts, extended to calls: a call passes each argument's objects to the
/// callee's parameter and the callee's returned objects to the call's result.
pub const CONTEXT_INSENSITIVE: &str = "\
Axiom alloc: for all v h, if VP0(v, h) then VP(v, h).
Axiom assign: for all v1 v2 h, if Assign(v1, v2) and VP(v2, h) then VP(v1, h).
Axiom pass: for all m i n z a f h, if Calls(m, i, n) and Actual(i, z, a) and Formal(n, z, f) and VP(a, h) then VP(f, h).
Axiom result: for all m i n v r h, if Calls(m, i, n) and Returns(n, v) and Receives(i, r) and VP(v, h) then VP(r, h).
Axiom store: for all v1 f v2 h1 h2, if Store(v1, f, v2) and VP(v1, h1) and VP(v2, h2) then HP(h1, f, h2).
Axiom load: for all v1 f v2 h1 h2, if Load(v1, f, v2) and VP(v1, h1) and HP(h1, f, h2) then VP(v2, h2).
";

/// The same analysis with every call path its own context (PLDI 2004 §4): `VPC(c, v, h)` holds in context `c` of
/// `v`'s function, and `VP` is its projection.
pub const CONTEXT_SENSITIVE: &str = "\
Axiom alloc: for all v h m c, if VP0(v, h) and Declares(m, v) and Context(m, c) then VPC(c, v, h).
Axiom assign: for all c v1 v2 h, if Assign(v1, v2) and VPC(c, v2, h) then VPC(c, v1, h).
Axiom pass: for all c i d n z a f h, if CallContext(c, i, d, n) and Actual(i, z, a) and Formal(n, z, f) and VPC(c, a, h) then VPC(d, f, h).
Axiom result: for all c i d n v r h, if CallContext(c, i, d, n) and Returns(n, v) and Receives(i, r) and VPC(d, v, h) then VPC(c, r, h).
Axiom store: for all c v1 f v2 h1 h2, if Store(v1, f, v2) and VPC(c, v1, h1) and VPC(c, v2, h2) then HP(h1, f, h2).
Axiom load: for all c v1 f v2 h1 h2, if Load(v1, f, v2) and VPC(c, v1, h1) and HP(h1, f, h2) then VPC(c, v2, h2).
Axiom project: for all c v h, if VPC(c, v, h) then VP(v, h).
";

/// A program's points-to facts, and what each allocation site is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PointsToFacts {
    /// Every fact, in the order the walk states it.
    pub facts: Vec<Atom>,
    /// Each allocation site with the function it is in and what it allocates.
    pub sites: Vec<(String, String, String)>,
}

/// What each variable may point to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PointsTo {
    /// `function.name` → the allocation sites it may hold.
    pub variables: BTreeMap<String, BTreeSet<String>>,
    /// `(site, field, site)`: an object's field (`Elem` for a collection's elements) may hold another.
    pub heap: BTreeSet<(String, String, String)>,
    /// The call paths numbered, when the analysis ran in contexts.
    pub contexts: Option<u128>,
    /// The allocation sites, as [`PointsToFacts::sites`].
    pub sites: Vec<(String, String, String)>,
}

/// The points-to facts of `source` (see the module doc).
pub fn points_to_facts(source: &str) -> Result<PointsToFacts, String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed?;
        let mut x = Extractor::new(interner);
        x.functions = stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, .. } => Some(interner.resolve(*name).to_string()),
                _ => None,
            })
            .collect();
        for stmt in stmts {
            if let Stmt::FunctionDef { name, params, body, .. } = stmt {
                x.current = interner.resolve(*name).to_string();
                for (k, (param, _)) in params.iter().enumerate() {
                    let p = x.local(*param);
                    x.declare(&p);
                    let f = x.current.clone();
                    x.fact("Formal", &[&f, &format!("Z{k}"), &p]);
                }
                x.block(body);
            }
        }
        x.current = "Main".to_string();
        for stmt in stmts.iter().filter(|s| !matches!(s, Stmt::FunctionDef { .. })) {
            x.stmt(stmt);
        }
        Ok(PointsToFacts { facts: x.facts, sites: x.sites })
    })
}

/// What each variable of `source` may point to, every call path its own context when `contexts`.
pub fn points_to(source: &str, contexts: bool) -> Result<PointsTo, String> {
    let extracted = points_to_facts(source)?;
    let rules = if contexts { CONTEXT_SENSITIVE } else { CONTEXT_INSENSITIVE };
    let development = logicaffeine_proof::development::parse_development(rules).map_err(|e| e.to_string())?;
    let rules = Program::from_axioms(&development.axioms).map_err(|e| e.to_string())?;
    let mut facts = extracted.facts;
    // The numbering knows a function from the calls that name it, and `Main` is called by none: a call of itself, which
    // keeps its context, makes it the root with its one context even in a program that calls nothing.
    if contexts {
        facts.push(Atom::new("Calls", ["Main", "S_main", "Main"].iter().map(|a| Term::constant(a)).collect()));
    }
    let program = Program::new(facts, rules.rules().to_vec());
    let (relation, heap, numbered): (BTreeSet<Vec<String>>, BTreeSet<Vec<String>>, Option<u128>) = if contexts {
        let numbering = Contexts::number(&program).map_err(|e| e.to_string())?;
        let model = evaluate_bdd_with_contexts(&program, &numbering).map_err(|e| e.to_string())?;
        (model.relation("VP"), model.relation("HP"), Some(numbering.total()))
    } else {
        let model = evaluate(&program).map_err(|e| e.to_string())?;
        (model.relation("VP"), model.relation("HP"), None)
    };
    let mut variables: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for t in relation {
        variables.entry(t[0].clone()).or_default().insert(t[1].clone());
    }
    Ok(PointsTo {
        variables,
        heap: heap.into_iter().map(|t| (t[0].clone(), t[1].clone(), t[2].clone())).collect(),
        contexts: numbered,
        sites: extracted.sites,
    })
}

struct Extractor<'i> {
    interner: &'i Interner,
    facts: Vec<Atom>,
    sites: Vec<(String, String, String)>,
    declared: HashSet<(String, String)>,
    functions: HashSet<String>,
    current: String,
    calls: usize,
    temps: usize,
}

impl<'i> Extractor<'i> {
    fn new(interner: &'i Interner) -> Self {
        Extractor {
            interner,
            facts: Vec::new(),
            sites: Vec::new(),
            declared: HashSet::new(),
            functions: HashSet::new(),
            current: "Main".to_string(),
            calls: 0,
            temps: 0,
        }
    }

    fn fact(&mut self, predicate: &str, args: &[&str]) {
        self.facts.push(Atom::new(predicate, args.iter().map(|a| Term::constant(a)).collect()));
    }

    fn local(&self, name: logicaffeine_base::Symbol) -> String {
        format!("{}.{}", self.current, self.interner.resolve(name))
    }

    fn declare(&mut self, var: &str) {
        if self.declared.insert((self.current.clone(), var.to_string())) {
            let f = self.current.clone();
            self.fact("Declares", &[&f, var]);
        }
    }

    fn temp(&mut self) -> String {
        let t = format!("{}.%{}", self.current, self.temps);
        self.temps += 1;
        self.declare(&t);
        t
    }

    /// A fresh object allocated here, held by a new temporary.
    fn allocate(&mut self, what: &str) -> String {
        let site = format!("H{}", self.sites.len());
        self.sites.push((site.clone(), self.current.clone(), what.to_string()));
        let t = self.temp();
        self.fact("VP0", &[&t, &site]);
        t
    }

    fn block(&mut self, stmts: &[Stmt<'_>]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    /// A call of `function` with `args` from the current function: its site's facts, when the program defines the
    /// callee; the site.
    fn call(&mut self, function: logicaffeine_base::Symbol, args: &[&Expr<'_>]) -> Option<String> {
        let values: Vec<Option<String>> = args.iter().map(|a| self.value(a)).collect();
        let callee = self.interner.resolve(function).to_string();
        if !self.functions.contains(&callee) {
            return None;
        }
        let site = format!("S{}", self.calls);
        self.calls += 1;
        let caller = self.current.clone();
        self.fact("Calls", &[&caller, &site, &callee]);
        for (k, v) in values.iter().enumerate() {
            if let Some(v) = v {
                self.fact("Actual", &[&site, &format!("Z{k}"), v]);
            }
        }
        Some(site)
    }

    /// The variable holding the object `expr` evaluates to, when it is one; every fact the expression's parts state
    /// is stated on the way.
    fn value(&mut self, expr: &Expr<'_>) -> Option<String> {
        match expr {
            Expr::Identifier(name) => Some(self.local(*name)),
            Expr::Call { function, args } => {
                let args: Vec<&Expr<'_>> = args.iter().copied().collect();
                match self.call(*function, &args) {
                    Some(site) => {
                        let r = self.temp();
                        self.fact("Receives", &[&site, &r]);
                        Some(r)
                    }
                    // A builtin's result is taken as fresh: so a list `split` returns is an object, and two
                    // variables that copy it share it.
                    None => {
                        let what = format!("the result of {}", self.interner.resolve(*function));
                        Some(self.allocate(&what))
                    }
                }
            }
            Expr::Index { collection, index } => {
                self.value(index);
                let c = self.value(collection)?;
                let t = self.temp();
                self.fact("Load", &[&c, "Elem", &t]);
                Some(t)
            }
            Expr::FieldAccess { object, field } => {
                let o = self.value(object)?;
                let t = self.temp();
                let f = self.interner.resolve(*field).to_string();
                self.fact("Load", &[&o, &f, &t]);
                Some(t)
            }
            Expr::New { type_name, init_fields, .. } => {
                let what = format!("a new {}", self.interner.resolve(*type_name));
                let fields: Vec<(String, Option<String>)> = init_fields
                    .iter()
                    .map(|(f, e)| (self.interner.resolve(*f).to_string(), self.value(e)))
                    .collect();
                let t = self.allocate(&what);
                for (f, v) in fields {
                    if let Some(v) = v {
                        self.fact("Store", &[&t, &f, &v]);
                    }
                }
                Some(t)
            }
            Expr::NewVariant { variant, fields, .. } => {
                let what = format!("a {}", self.interner.resolve(*variant));
                let fields: Vec<(String, Option<String>)> = fields
                    .iter()
                    .map(|(f, e)| (self.interner.resolve(*f).to_string(), self.value(e)))
                    .collect();
                let t = self.allocate(&what);
                for (f, v) in fields {
                    if let Some(v) = v {
                        self.fact("Store", &[&t, &f, &v]);
                    }
                }
                Some(t)
            }
            Expr::List(items) | Expr::Tuple(items) => {
                let what = if matches!(expr, Expr::List(_)) { "a list" } else { "a tuple" };
                let values: Vec<Option<String>> = items.iter().map(|e| self.value(e)).collect();
                let t = self.allocate(what);
                for v in values.into_iter().flatten() {
                    self.fact("Store", &[&t, "Elem", &v]);
                }
                Some(t)
            }
            Expr::OptionSome { value } => {
                let v = self.value(value);
                let t = self.allocate("a Some");
                if let Some(v) = v {
                    self.fact("Store", &[&t, "Some", &v]);
                }
                Some(t)
            }
            Expr::Copy { expr } => {
                self.value(expr);
                Some(self.allocate("a copy"))
            }
            Expr::Give { value } => self.value(value),
            Expr::WithCapacity { value, capacity } => {
                self.value(capacity);
                self.value(value)
            }
            Expr::BinaryOp { left, right, .. } => {
                self.value(left);
                self.value(right);
                None
            }
            Expr::Not { operand } => {
                self.value(operand);
                None
            }
            Expr::Length { collection } => {
                self.value(collection);
                None
            }
            Expr::Contains { collection, value } => {
                self.value(collection);
                self.value(value);
                None
            }
            Expr::Slice { collection, start, end } => {
                self.value(start);
                self.value(end);
                let c = self.value(collection);
                let t = self.allocate("a slice");
                if let Some(c) = c {
                    let e = self.temp();
                    self.fact("Load", &[&c, "Elem", &e]);
                    self.fact("Store", &[&t, "Elem", &e]);
                }
                Some(t)
            }
            _ => None,
        }
    }

    fn stmt(&mut self, stmt: &Stmt<'_>) {
        match stmt {
            Stmt::Let { var, value, .. } => {
                let v = self.local(*var);
                self.declare(&v);
                if let Some(x) = self.value(value) {
                    self.fact("Assign", &[&v, &x]);
                }
            }
            Stmt::Set { target, value } => {
                let v = self.local(*target);
                self.declare(&v);
                if let Some(x) = self.value(value) {
                    self.fact("Assign", &[&v, &x]);
                }
            }
            Stmt::SetField { object, field, value } => {
                let x = self.value(value);
                if let (Some(o), Some(x)) = (self.value(object), x) {
                    let f = self.interner.resolve(*field).to_string();
                    self.fact("Store", &[&o, &f, &x]);
                }
            }
            Stmt::Push { value, collection } | Stmt::Add { value, collection } => {
                let x = self.value(value);
                if let (Some(c), Some(x)) = (self.value(collection), x) {
                    self.fact("Store", &[&c, "Elem", &x]);
                }
            }
            Stmt::SetIndex { collection, index, value } => {
                self.value(index);
                let x = self.value(value);
                if let (Some(c), Some(x)) = (self.value(collection), x) {
                    self.fact("Store", &[&c, "Elem", &x]);
                }
            }
            Stmt::Pop { collection, into } => {
                let c = self.value(collection);
                if let (Some(c), Some(into)) = (c, into) {
                    let v = self.local(*into);
                    self.declare(&v);
                    self.fact("Load", &[&c, "Elem", &v]);
                }
            }
            Stmt::Call { function, args } => {
                let args: Vec<&Expr<'_>> = args.iter().copied().collect();
                self.call(*function, &args);
            }
            Stmt::Return { value } => {
                if let Some(v) = value.and_then(|e| self.value(e)) {
                    let f = self.current.clone();
                    self.fact("Returns", &[&f, &v]);
                }
            }
            Stmt::Show { object, recipient } | Stmt::Give { object, recipient } => {
                self.value(object);
                self.value(recipient);
            }
            Stmt::If { cond, then_block, else_block } => {
                self.value(cond);
                self.block(then_block);
                if let Some(b) = else_block {
                    self.block(b);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.value(cond);
                self.block(body);
            }
            Stmt::Repeat { pattern, iterable, body } => {
                let it = self.value(iterable);
                let names: Vec<logicaffeine_base::Symbol> = match pattern {
                    Pattern::Identifier(x) => vec![*x],
                    Pattern::Tuple(xs) => xs.clone(),
                };
                for x in names {
                    let v = self.local(x);
                    self.declare(&v);
                    if let Some(it) = &it {
                        self.fact("Load", &[it, "Elem", &v]);
                    }
                }
                self.block(body);
            }
            Stmt::Inspect { target, arms, .. } => {
                let t = self.value(target);
                for MatchArm { bindings, body, .. } in arms {
                    for (field, binding) in bindings {
                        let v = self.local(*binding);
                        self.declare(&v);
                        if let Some(t) = &t {
                            let f = self.interner.resolve(*field).to_string();
                            self.fact("Load", &[t, &f, &v]);
                        }
                    }
                    self.block(body);
                }
            }
            Stmt::Zone { body, .. } => self.block(body),
            _ => {}
        }
    }
}
