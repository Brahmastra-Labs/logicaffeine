//! Datalog: function-free Horn rules evaluated bottom-up to their least model.
//!
//! A `## Theory` whose axioms are ground facts (`VP0(A, H1)`) and universally quantified Horn rules
//! (`for all v1 v2 h, if Assign(v1, v2) and VP(v2, h) then VP(v1, h)`) is a Datalog program. Its least model is every
//! fact the rules derive, and it is reached bottom-up: fire the rules on what is known until nothing new appears.
//!
//! [`evaluate`] does that semi-naively. A round joins each rule once for every recursive body literal, with that
//! literal ranging over only the facts the previous round derived (its delta) and the others over everything known, so
//! a derivation is found in the round after its newest premise appears and no join repeats a whole previous round.
//! Rules run stratum by stratum: the predicates are grouped into the strongly connected components of their
//! dependency graph and evaluated in dependency order, so a negated literal (`not Reach(x)`) reads a relation that is
//! already complete. A negation inside a cycle has no such order and is refused ([`DatalogError::Unstratifiable`]).
//! [`evaluate_naive`] re-fires every rule over the whole model each round: the same model, by the definition.
//!
//! A rule's condition may also compare values its positive atoms bind ([`Guard`]): `x ≤ y`, `x < y`, `x ≥ y`, `x > y`
//! between numbers, read as exact rationals (`107.1`, `-10`, `3/4`), and `x = y`, `x ≠ y` between any two constants,
//! numbers by value. A guard filters a rule's instances and is no premise of them. An ordering of a constant that is not a
//! number is refused ([`DatalogError::NotANumber`]). A fact that rests on a guard or a negation reads the closed world (that
//! two names differ, that a number lies below another, that nothing else holds), which the axioms alone do not state
//! ([`Model::closed_world`]).
//!
//! Every fact carries the rule and the premises that first derived it ([`Model::derivation`]), so each answer can be
//! replayed as a proof. A relation is a list of interned tuples in derivation order with a hash set for membership and,
//! per column, an index from value to tuples, built the first time a join looks a column up and grown as tuples
//! arrive; a round's delta is the range of tuples the round before appended.
//!
//! [`evaluate_bdd`] reaches the same model with every relation a binary decision diagram, bddbddb's representation,
//! and [`evaluate_bdd_with_contexts`] adds bddbddb's context sensitivity: [`Contexts`] numbers the call paths of the
//! program's call graph, and the rules read the numbering as two relations computed from it, so a method cloned once per
//! call path costs diagram nodes polynomial in the call graph however many paths there are.

use crate::{ProofExpr, ProofTerm};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

/// A term of a Datalog atom: a variable a rule quantifies, or a constant.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Term {
    /// A variable of the rule it appears in.
    Var(String),
    /// A constant symbol.
    Const(String),
}

impl Term {
    /// The variable `name`.
    pub fn var(name: &str) -> Self {
        Term::Var(name.to_string())
    }

    /// The constant `name`.
    pub fn constant(name: &str) -> Self {
        Term::Const(name.to_string())
    }
}

/// An atom `predicate(term, …)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Atom {
    /// The predicate's name.
    pub predicate: String,
    /// Its arguments.
    pub terms: Vec<Term>,
}

impl Atom {
    /// The atom `predicate(terms…)`.
    pub fn new(predicate: &str, terms: Vec<Term>) -> Self {
        Atom { predicate: predicate.to_string(), terms }
    }

    /// The atom's constants, when it has no variable.
    pub fn ground(&self) -> Option<Vec<String>> {
        self.terms
            .iter()
            .map(|t| match t {
                Term::Const(c) => Some(c.clone()),
                Term::Var(_) => None,
            })
            .collect()
    }
}

/// A body literal: an atom that must hold, or one that must not.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Literal {
    /// The atom holds.
    Pos(Atom),
    /// The atom does not hold (negation as failure, over a lower stratum).
    Neg(Atom),
}

impl Literal {
    fn atom(&self) -> &Atom {
        match self {
            Literal::Pos(a) | Literal::Neg(a) => a,
        }
    }
}

/// How a guard compares its two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Comparison {
    /// `<`
    Lt,
    /// `≤`
    Le,
    /// `>`
    Gt,
    /// `≥`
    Ge,
    /// `=`: the same constant, or numbers of the same value.
    Eq,
    /// `≠`
    Neq,
}

impl Comparison {
    /// The comparison a theory's relation name stands for (`Lt`, `Le`, `Gt`, `Ge`, `Neq`), as the formula parser
    /// names `<`, `≤`, `>`, `≥`, `≠`.
    pub fn named(name: &str) -> Option<Comparison> {
        Some(match name {
            "Lt" => Comparison::Lt,
            "Le" => Comparison::Le,
            "Gt" => Comparison::Gt,
            "Ge" => Comparison::Ge,
            "Neq" => Comparison::Neq,
            _ => return None,
        })
    }

    /// The comparison that holds exactly where this one does not.
    pub fn negated(self) -> Comparison {
        match self {
            Comparison::Lt => Comparison::Ge,
            Comparison::Le => Comparison::Gt,
            Comparison::Gt => Comparison::Le,
            Comparison::Ge => Comparison::Lt,
            Comparison::Eq => Comparison::Neq,
            Comparison::Neq => Comparison::Eq,
        }
    }

    /// Whether it orders numbers, rather than telling constants apart.
    pub fn orders(self) -> bool {
        !matches!(self, Comparison::Eq | Comparison::Neq)
    }

    /// The symbol it is written with.
    pub fn symbol(self) -> &'static str {
        match self {
            Comparison::Lt => "<",
            Comparison::Le => "≤",
            Comparison::Gt => ">",
            Comparison::Ge => "≥",
            Comparison::Eq => "=",
            Comparison::Neq => "≠",
        }
    }

    /// Whether `left` and `right` stand in this comparison: `Err` names a value an ordering finds is not a number.
    pub fn holds<'v>(self, left: &'v str, right: &'v str) -> Result<bool, &'v str> {
        let (a, b) = (number(left), number(right));
        if self.orders() {
            let a = a.ok_or(left)?;
            let b = b.ok_or(right)?;
            return Ok(match self {
                Comparison::Lt => a < b,
                Comparison::Le => a <= b,
                Comparison::Gt => a > b,
                _ => a >= b,
            });
        }
        let same = match (a, b) {
            (Some(a), Some(b)) => a == b,
            _ => left == right,
        };
        Ok(same == (self == Comparison::Eq))
    }
}

/// A constant's value as an exact rational, when it is a numeral: an integer (`145`, `-10`), a decimal (`107.1`,
/// `-0.5`) or a fraction (`3/4`).
pub fn number(constant: &str) -> Option<logicaffeine_base::Rational> {
    logicaffeine_base::Rational::parse(constant)
        .or_else(|| logicaffeine_base::Decimal::parse(constant).map(|d| d.to_rational()))
}

/// A comparison a rule's condition makes between two of its values: `left op right`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Guard {
    /// How the values compare.
    pub op: Comparison,
    /// The value on the left.
    pub left: Term,
    /// The value on the right.
    pub right: Term,
}

/// A Horn rule `head :- body`, named for the derivations it makes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Rule {
    /// The rule's name (the axiom's, when it came from a theory).
    pub name: String,
    /// The atom it derives.
    pub head: Atom,
    /// What must hold (and not hold) for it to fire.
    pub body: Vec<Literal>,
    /// The comparisons its values must pass.
    pub guards: Vec<Guard>,
}

impl Rule {
    /// The rule `name: head :- body`.
    pub fn new(name: impl Into<String>, head: Atom, body: Vec<Literal>) -> Self {
        Rule { name: name.into(), head, body, guards: Vec::new() }
    }

    /// The rule `name: head :- body, guards`.
    pub fn guarded(name: impl Into<String>, head: Atom, body: Vec<Literal>, guards: Vec<Guard>) -> Self {
        Rule { name: name.into(), head, body, guards }
    }

    /// Whether a fact it derives reads the closed world: it rests on a negation or a comparison.
    pub fn reads_closed_world(&self) -> bool {
        !self.guards.is_empty() || self.body.iter().any(|l| matches!(l, Literal::Neg(_)))
    }

    /// Every variable the rule names.
    pub fn variables(&self) -> BTreeSet<String> {
        std::iter::once(&self.head)
            .chain(self.body.iter().map(Literal::atom))
            .flat_map(|a| a.terms.iter())
            .chain(self.guards.iter().flat_map(|g| [&g.left, &g.right]))
            .filter_map(|t| match t {
                Term::Var(v) => Some(v.clone()),
                Term::Const(_) => None,
            })
            .collect()
    }
}

/// The call graph the context numbering reads: `Calls(caller, site, callee)`, given as facts or discovered by rules
/// that do not read the contexts numbered over it.
pub const CALLS: &str = "Calls";

/// `Context(method, c)`: `c` is one of the method's contexts, a number below the paths that reach it. Computed by the
/// context numbering.
pub const CONTEXT: &str = "Context";

/// `CallContext(c, site, d, callee)`: the call at `site`, made in context `c` of its caller, enters the callee in
/// its context `d`. Computed by the context numbering.
pub const CALL_CONTEXT: &str = "CallContext";

/// Why a theory is not a Datalog program, or a program has no stratified least model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatalogError {
    /// An axiom is neither a ground fact nor a function-free Horn rule.
    NotDatalog {
        /// The axiom's name.
        axiom: String,
        /// What it holds instead.
        reason: String,
    },
    /// A variable of a rule's head, of a negated literal or of a comparison appears in no positive body literal, so the
    /// rule would range over a domain nothing bounds.
    Unsafe {
        /// The rule's name.
        rule: String,
        /// The unbound variable.
        variable: String,
    },
    /// These predicates depend on one another through a negation: no stratum can be finished before another reads it.
    Unstratifiable {
        /// The predicates of the cycle.
        predicates: Vec<String>,
    },
    /// A predicate is used with two arities.
    Arity {
        /// The predicate.
        predicate: String,
        /// The arity it was first used with.
        expected: usize,
        /// The arity it was later used with.
        found: usize,
    },
    /// The program states or derives a relation the context numbering computes.
    Computed {
        /// [`CONTEXT`] or [`CALL_CONTEXT`].
        predicate: String,
    },
    /// A rule derives the call graph from the contexts the numbering builds over it.
    NotGiven {
        /// [`CALLS`].
        predicate: String,
        /// The rule that derives it.
        rule: String,
    },
    /// A rule joins an argument that holds constants to one that holds context numbers.
    Sort {
        /// The predicate whose argument holds constants.
        predicate: String,
        /// The argument, counted from 0.
        position: usize,
    },
    /// More call paths reach a method than 64 bits number.
    ContextOverflow {
        /// The method.
        method: String,
    },
    /// A rule orders a constant that is not a number.
    NotANumber {
        /// The rule's name.
        rule: String,
        /// The constant.
        value: String,
    },
    /// The diagram evaluation met a rule that compares values: a diagram encodes constants as codes with no order.
    Guarded {
        /// The rule's name.
        rule: String,
    },
}

impl fmt::Display for DatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DatalogError::NotDatalog { axiom, reason } => write!(f, "axiom `{axiom}` is not a Datalog rule: {reason}"),
            DatalogError::Unsafe { rule, variable } => {
                write!(f, "rule `{rule}`: `{variable}` appears in no positive body literal, so nothing bounds it")
            }
            DatalogError::Unstratifiable { predicates } => {
                write!(f, "{} depend on one another through a negation", predicates.join(", "))
            }
            DatalogError::Arity { predicate, expected, found } => {
                write!(f, "`{predicate}` is used with {expected} arguments and with {found}")
            }
            DatalogError::Computed { predicate } => write!(
                f,
                "`{predicate}` is computed from `{CALLS}` by the context numbering; the program may not state or derive it"
            ),
            DatalogError::NotGiven { predicate, rule } => write!(
                f,
                "rule `{rule}` derives `{predicate}` from the contexts numbered over it: the call graph must be complete \
                 before its contexts are"
            ),
            DatalogError::Sort { predicate, position } => write!(
                f,
                "argument {} of `{predicate}` holds constants, and a rule joins it to a context number",
                position + 1
            ),
            DatalogError::ContextOverflow { method } => {
                write!(f, "more call paths reach `{method}` than 64 bits number")
            }
            DatalogError::NotANumber { rule, value } => {
                write!(f, "rule `{rule}` orders `{value}`, which is not a number: only numbers are ordered")
            }
            DatalogError::Guarded { rule } => write!(
                f,
                "rule `{rule}` compares values, which a decision diagram cannot: evaluate it without diagrams"
            ),
        }
    }
}

impl std::error::Error for DatalogError {}

/// A Datalog program: ground facts and Horn rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    facts: Vec<Atom>,
    fact_names: Vec<String>,
    rules: Vec<Rule>,
}

impl Program {
    /// The program with these facts and rules.
    pub fn new(facts: Vec<Atom>, rules: Vec<Rule>) -> Self {
        let fact_names = (0..facts.len()).map(|i| format!("fact {i}")).collect();
        Program { facts, fact_names, rules }
    }

    /// The program a theory's axioms state: each ground atom a fact, each `for all …, if … then …` with a conjunction
    /// of atoms, negated atoms and comparisons for its condition and one atom for its conclusion a rule. A name the axiom does not
    /// quantify is a constant, as it is to the prover. The program is checked for safety and consistent arities.
    pub fn from_axioms(axioms: &[(String, ProofExpr)]) -> Result<Self, DatalogError> {
        let mut facts = Vec::new();
        let mut fact_names = Vec::new();
        let mut rules = Vec::new();
        for (name, expr) in axioms {
            let not_datalog = |reason: &str| DatalogError::NotDatalog { axiom: name.clone(), reason: reason.to_string() };
            let mut bound = Vec::new();
            let mut body = expr;
            while let ProofExpr::ForAll { variable, body: inner } = body {
                bound.push(variable.clone());
                body = inner;
            }
            match body {
                ProofExpr::Implies(condition, conclusion) => {
                    let head = atom_of(conclusion, &bound).map_err(|r| not_datalog(&r))?;
                    let mut literals = Vec::new();
                    let mut guards = Vec::new();
                    conjuncts(condition, &bound, &mut literals, &mut guards).map_err(|r| not_datalog(&r))?;
                    rules.push(Rule::guarded(name.clone(), head, literals, guards));
                }
                _ => {
                    let atom = atom_of(body, &bound).map_err(|r| not_datalog(&r))?;
                    if atom.ground().is_some() {
                        facts.push(atom);
                        fact_names.push(name.clone());
                    } else {
                        rules.push(Rule::new(name.clone(), atom, Vec::new()));
                    }
                }
            }
        }
        let program = Program { facts, fact_names, rules };
        program.validate()?;
        Ok(program)
    }

    /// The ground facts.
    pub fn facts(&self) -> &[Atom] {
        &self.facts
    }

    /// The rules.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Every constant the program names.
    pub fn constants(&self) -> BTreeSet<String> {
        self.facts
            .iter()
            .chain(self.rules.iter().flat_map(|r| std::iter::once(&r.head).chain(r.body.iter().map(Literal::atom))))
            .flat_map(|a| a.terms.iter())
            .chain(self.rules.iter().flat_map(|r| r.guards.iter().flat_map(|g| [&g.left, &g.right])))
            .filter_map(|t| match t {
                Term::Const(c) => Some(c.clone()),
                Term::Var(_) => None,
            })
            .collect()
    }

    /// Safety of every rule and one arity per predicate.
    fn validate(&self) -> Result<(), DatalogError> {
        let mut arity: HashMap<&str, usize> = HashMap::new();
        let atoms = self
            .facts
            .iter()
            .chain(self.rules.iter().flat_map(|r| std::iter::once(&r.head).chain(r.body.iter().map(Literal::atom))));
        for a in atoms {
            let expected = *arity.entry(a.predicate.as_str()).or_insert(a.terms.len());
            if expected != a.terms.len() {
                return Err(DatalogError::Arity { predicate: a.predicate.clone(), expected, found: a.terms.len() });
            }
        }
        for rule in &self.rules {
            let positive: HashSet<&str> = rule
                .body
                .iter()
                .filter_map(|l| match l {
                    Literal::Pos(a) => Some(a),
                    Literal::Neg(_) => None,
                })
                .flat_map(|a| a.terms.iter())
                .filter_map(|t| match t {
                    Term::Var(v) => Some(v.as_str()),
                    Term::Const(_) => None,
                })
                .collect();
            let must_bind = std::iter::once(&rule.head)
                .chain(rule.body.iter().filter_map(|l| match l {
                    Literal::Neg(a) => Some(a),
                    Literal::Pos(_) => None,
                }))
                .flat_map(|a| a.terms.iter())
                .chain(rule.guards.iter().flat_map(|g| [&g.left, &g.right]));
            for t in must_bind {
                if let Term::Var(v) = t {
                    if !positive.contains(v.as_str()) {
                        return Err(DatalogError::Unsafe { rule: rule.name.clone(), variable: v.clone() });
                    }
                }
            }
        }
        Ok(())
    }

    /// The rules grouped into strata in dependency order: each stratum is one strongly connected component of the
    /// predicate graph (body predicate → head predicate) with the rules that derive into it.
    fn strata(&self) -> Result<Vec<Vec<usize>>, DatalogError> {
        let mut preds: Vec<&str> = Vec::new();
        let mut index: HashMap<&str, usize> = HashMap::new();
        for rule in &self.rules {
            for a in std::iter::once(&rule.head).chain(rule.body.iter().map(Literal::atom)) {
                if !index.contains_key(a.predicate.as_str()) {
                    index.insert(a.predicate.as_str(), preds.len());
                    preds.push(a.predicate.as_str());
                }
            }
        }
        let n = preds.len();
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        for rule in &self.rules {
            let h = index[rule.head.predicate.as_str()];
            for l in &rule.body {
                edges[index[l.atom().predicate.as_str()]].push(h);
            }
        }
        let components = tarjan(&edges);
        let mut component_of = vec![0usize; n];
        for (c, members) in components.iter().enumerate() {
            for &m in members {
                component_of[m] = c;
            }
        }
        for rule in &self.rules {
            let h = component_of[index[rule.head.predicate.as_str()]];
            for l in &rule.body {
                if let Literal::Neg(a) = l {
                    if component_of[index[a.predicate.as_str()]] == h {
                        let mut predicates: Vec<String> =
                            components[h].iter().map(|&m| preds[m].to_string()).collect();
                        predicates.sort();
                        return Err(DatalogError::Unstratifiable { predicates });
                    }
                }
            }
        }
        // Tarjan emits a component only after every component it reaches, so dependencies come out last: reverse.
        let mut strata = vec![Vec::new(); components.len()];
        for (r, rule) in self.rules.iter().enumerate() {
            strata[component_of[index[rule.head.predicate.as_str()]]].push(r);
        }
        strata.reverse();
        Ok(strata.into_iter().filter(|s| !s.is_empty()).collect())
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let term = |t: &Term| match t {
            Term::Var(v) | Term::Const(v) => v.clone(),
        };
        let atom = |a: &Atom| format!("{}({})", a.predicate, a.terms.iter().map(term).collect::<Vec<_>>().join(", "));
        for fact in &self.facts {
            writeln!(f, "{}.", atom(fact))?;
        }
        for rule in &self.rules {
            let body: Vec<String> = rule
                .body
                .iter()
                .map(|l| match l {
                    Literal::Pos(a) => atom(a),
                    Literal::Neg(a) => format!("not {}", atom(a)),
                })
                .chain(rule.guards.iter().map(|g| format!("{} {} {}", term(&g.left), g.op.symbol(), term(&g.right))))
                .collect();
            writeln!(f, "{} :- {}.   % {}", atom(&rule.head), body.join(", "), rule.name)?;
        }
        Ok(())
    }
}

fn atom_of(expr: &ProofExpr, bound: &[String]) -> Result<Atom, String> {
    match expr {
        ProofExpr::Predicate { name, args, world: None } => {
            let terms = args.iter().map(|t| term_of(t, bound)).collect::<Result<Vec<_>, _>>()?;
            Ok(Atom { predicate: name.clone(), terms })
        }
        ProofExpr::Predicate { world: Some(w), .. } => Err(format!("an atom at world `{w}`")),
        ProofExpr::Or(..) => Err("a disjunction".to_string()),
        ProofExpr::Exists { .. } => Err("an existential".to_string()),
        ProofExpr::Identity(..) => Err("an equation".to_string()),
        ProofExpr::Iff(..) => Err("a biconditional".to_string()),
        ProofExpr::Not(_) => Err("a negated conclusion".to_string()),
        ProofExpr::ForAll { .. } => Err("a quantifier inside a rule".to_string()),
        other => Err(format!("{other:?}")),
    }
}

fn conjuncts(expr: &ProofExpr, bound: &[String], out: &mut Vec<Literal>, guards: &mut Vec<Guard>) -> Result<(), String> {
    match expr {
        ProofExpr::And(a, b) => {
            conjuncts(a, bound, out, guards)?;
            conjuncts(b, bound, out, guards)
        }
        ProofExpr::Not(inner) => match guard_of(inner, bound)? {
            Some(g) => {
                guards.push(Guard { op: g.op.negated(), ..g });
                Ok(())
            }
            None => {
                out.push(Literal::Neg(atom_of(inner, bound)?));
                Ok(())
            }
        },
        other => match guard_of(other, bound)? {
            Some(g) => {
                guards.push(g);
                Ok(())
            }
            None => {
                out.push(Literal::Pos(atom_of(other, bound)?));
                Ok(())
            }
        },
    }
}

/// The comparison `expr` makes, when it makes one: an equation `a = b`, or a relation the formula parser names for a
/// comparison (`Lt(a, b)` for `a < b`, …).
fn guard_of(expr: &ProofExpr, bound: &[String]) -> Result<Option<Guard>, String> {
    let (op, left, right) = match expr {
        ProofExpr::Identity(l, r) => (Comparison::Eq, l, r),
        ProofExpr::Predicate { name, args, world: None } if args.len() == 2 => match Comparison::named(name) {
            Some(op) => (op, &args[0], &args[1]),
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(Some(Guard { op, left: term_of(left, bound)?, right: term_of(right, bound)? }))
}

fn term_of(t: &ProofTerm, bound: &[String]) -> Result<Term, String> {
    match t {
        ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) if bound.contains(v) => Ok(Term::Var(v.clone())),
        ProofTerm::Variable(v) | ProofTerm::BoundVarRef(v) | ProofTerm::Constant(v) => Ok(Term::Const(v.clone())),
        ProofTerm::Function(f, _) => Err(format!("`{f}(…)` is a function symbol")),
        ProofTerm::Group(_) => Err("a tuple term".to_string()),
        #[allow(unreachable_patterns)]
        other => Err(format!("the term {other:?}")),
    }
}

/// Tarjan's strongly connected components of a graph given as adjacency lists.
fn tarjan(edges: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct State<'a> {
        edges: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    fn visit(s: &mut State<'_>, v: usize) {
        s.index[v] = Some(s.next);
        s.low[v] = s.next;
        s.next += 1;
        s.stack.push(v);
        s.on_stack[v] = true;
        for &w in &s.edges[v] {
            match s.index[w] {
                None => {
                    visit(s, w);
                    s.low[v] = s.low[v].min(s.low[w]);
                }
                Some(iw) if s.on_stack[w] => s.low[v] = s.low[v].min(iw),
                Some(_) => {}
            }
        }
        if Some(s.low[v]) == s.index[v] {
            let mut component = Vec::new();
            loop {
                let w = s.stack.pop().expect("v is on the stack");
                s.on_stack[w] = false;
                component.push(w);
                if w == v {
                    break;
                }
            }
            s.out.push(component);
        }
    }
    let n = edges.len();
    let mut s = State {
        edges,
        index: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        next: 0,
        out: Vec::new(),
    };
    for v in 0..n {
        if s.index[v].is_none() {
            visit(&mut s, v);
        }
    }
    s.out
}

// ── evaluation ──────────────────────────────────────────────────────────────────────────────────────────────────

type Sym = u32;

/// How a tuple came to hold.
#[derive(Debug, Clone)]
enum Origin {
    /// A fact of the program, by its index.
    Given(usize),
    /// A rule, by its index, from these premises (predicate index, tuple index), one per positive body literal.
    Derived(usize, Vec<(usize, usize)>),
}

#[derive(Default)]
struct Relation {
    tuples: Vec<Box<[Sym]>>,
    origins: Vec<Origin>,
    members: HashMap<Box<[Sym]>, usize>,
    /// Per column, value → the tuples holding it; `None` until a join first looks the column up.
    columns: Vec<Option<HashMap<Sym, Vec<usize>>>>,
}

impl Relation {
    fn push(&mut self, tuple: Box<[Sym]>, origin: Origin) {
        let id = self.tuples.len();
        for (c, index) in self.columns.iter_mut().enumerate() {
            if let Some(index) = index {
                index.entry(tuple[c]).or_default().push(id);
            }
        }
        self.members.insert(tuple.clone(), id);
        self.tuples.push(tuple);
        self.origins.push(origin);
    }

    fn column(&mut self, c: usize) -> &HashMap<Sym, Vec<usize>> {
        if self.columns.len() <= c {
            self.columns.resize_with(c + 1, || None);
        }
        if self.columns[c].is_none() {
            let mut index: HashMap<Sym, Vec<usize>> = HashMap::new();
            for (id, t) in self.tuples.iter().enumerate() {
                index.entry(t[c]).or_default().push(id);
            }
            self.columns[c] = Some(index);
        }
        self.columns[c].as_ref().expect("built above")
    }
}

/// An argument slot of a compiled atom.
#[derive(Clone, Copy)]
enum Slot {
    Var(usize),
    Const(Sym),
}

struct CompiledAtom {
    pred: usize,
    slots: Vec<Slot>,
}

struct CompiledRule {
    name: String,
    head: CompiledAtom,
    positive: Vec<CompiledAtom>,
    negative: Vec<CompiledAtom>,
    guards: Vec<(Comparison, Slot, Slot)>,
    vars: usize,
}

struct Engine {
    symbols: Vec<String>,
    symbol_ids: HashMap<String, Sym>,
    preds: Vec<String>,
    pred_ids: HashMap<String, usize>,
    relations: Vec<Relation>,
    work: u64,
    /// The first ordering of a constant that is not a number, which stops the evaluation.
    refused: Option<DatalogError>,
}

impl Engine {
    fn symbol(&mut self, s: &str) -> Sym {
        if let Some(&id) = self.symbol_ids.get(s) {
            return id;
        }
        let id = self.symbols.len() as Sym;
        self.symbols.push(s.to_string());
        self.symbol_ids.insert(s.to_string(), id);
        id
    }

    fn pred(&mut self, p: &str) -> usize {
        if let Some(&id) = self.pred_ids.get(p) {
            return id;
        }
        let id = self.preds.len();
        self.preds.push(p.to_string());
        self.pred_ids.insert(p.to_string(), id);
        self.relations.push(Relation::default());
        id
    }

    fn compile_atom(&mut self, a: &Atom, vars: &mut Vec<String>) -> CompiledAtom {
        let pred = self.pred(&a.predicate);
        let slots = a
            .terms
            .iter()
            .map(|t| match t {
                Term::Const(c) => Slot::Const(self.symbol(c)),
                Term::Var(v) => Slot::Var(match vars.iter().position(|x| x == v) {
                    Some(i) => i,
                    None => {
                        vars.push(v.clone());
                        vars.len() - 1
                    }
                }),
            })
            .collect();
        CompiledAtom { pred, slots }
    }

    fn compile_rule(&mut self, rule: &Rule) -> CompiledRule {
        let mut vars = Vec::new();
        let positive: Vec<CompiledAtom> = rule
            .body
            .iter()
            .filter_map(|l| match l {
                Literal::Pos(a) => Some(a),
                Literal::Neg(_) => None,
            })
            .map(|a| self.compile_atom(a, &mut vars))
            .collect();
        let negative: Vec<CompiledAtom> = rule
            .body
            .iter()
            .filter_map(|l| match l {
                Literal::Neg(a) => Some(a),
                Literal::Pos(_) => None,
            })
            .map(|a| self.compile_atom(a, &mut vars))
            .collect();
        let head = self.compile_atom(&rule.head, &mut vars);
        let slot = |engine: &mut Engine, t: &Term| match t {
            Term::Const(c) => Slot::Const(engine.symbol(c)),
            Term::Var(v) => Slot::Var(vars.iter().position(|x| x == v).expect("a safe rule binds its guards")),
        };
        let guards = rule.guards.iter().map(|g| (g.op, slot(self, &g.left), slot(self, &g.right))).collect();
        CompiledRule { name: rule.name.clone(), head, positive, negative, guards, vars: vars.len() }
    }

    /// Every instance of `rule` over the relations as they stand, the positive literal at `delta` (if any) ranging
    /// over the tuples `ranges[pred]` names and the others over every tuple below `limits[pred]`. Each instance is the
    /// head tuple and the premises it was joined from.
    fn instances(
        &mut self,
        rule: &CompiledRule,
        delta: Option<usize>,
        ranges: &[(usize, usize)],
        limits: &[usize],
    ) -> Vec<(Box<[Sym]>, Vec<(usize, usize)>)> {
        let mut order: Vec<usize> = (0..rule.positive.len()).collect();
        if let Some(d) = delta {
            order.retain(|&i| i != d);
            order.insert(0, d);
        }
        let mut out = Vec::new();
        let mut binding: Vec<Option<Sym>> = vec![None; rule.vars];
        let mut premises: Vec<(usize, usize)> = vec![(0, 0); rule.positive.len()];
        self.join(rule, &order, 0, delta, ranges, limits, &mut binding, &mut premises, &mut out);
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn join(
        &mut self,
        rule: &CompiledRule,
        order: &[usize],
        depth: usize,
        delta: Option<usize>,
        ranges: &[(usize, usize)],
        limits: &[usize],
        binding: &mut Vec<Option<Sym>>,
        premises: &mut Vec<(usize, usize)>,
        out: &mut Vec<(Box<[Sym]>, Vec<(usize, usize)>)>,
    ) {
        if depth == order.len() {
            self.work += 1;
            for &(op, l, r) in &rule.guards {
                let (l, r) = (value(l, binding), value(r, binding));
                match op.holds(&self.symbols[l as usize], &self.symbols[r as usize]) {
                    Ok(true) => {}
                    Ok(false) => return,
                    Err(value) => {
                        if self.refused.is_none() {
                            self.refused =
                                Some(DatalogError::NotANumber { rule: rule.name.clone(), value: value.to_string() });
                        }
                        return;
                    }
                }
            }
            for neg in &rule.negative {
                let tuple: Box<[Sym]> = neg.slots.iter().map(|s| value(*s, binding)).collect();
                if self.relations[neg.pred].members.contains_key(&tuple) {
                    return;
                }
            }
            let head: Box<[Sym]> = rule.head.slots.iter().map(|s| value(*s, binding)).collect();
            out.push((head, premises.clone()));
            return;
        }
        let li = order[depth];
        let atom = &rule.positive[li];
        let (lo, hi) = if delta == Some(li) { ranges[atom.pred] } else { (0, limits[atom.pred]) };
        // Look up the first bound column through its index; otherwise scan the range.
        let probe = atom.slots.iter().enumerate().find_map(|(c, s)| match *s {
            Slot::Const(k) => Some((c, k)),
            Slot::Var(v) => binding[v].map(|k| (c, k)),
        });
        let candidates: Vec<usize> = match probe {
            Some((c, k)) => self.relations[atom.pred]
                .column(c)
                .get(&k)
                .map(|ids| ids.iter().copied().filter(|&id| id >= lo && id < hi).collect())
                .unwrap_or_default(),
            None => (lo..hi).collect(),
        };
        for id in candidates {
            let mut bound_here = Vec::new();
            let mut fits = true;
            for (c, s) in atom.slots.iter().enumerate() {
                let have = self.relations[atom.pred].tuples[id][c];
                match *s {
                    Slot::Const(k) => fits = k == have,
                    Slot::Var(v) => match binding[v] {
                        Some(b) => fits = b == have,
                        None => {
                            binding[v] = Some(have);
                            bound_here.push(v);
                        }
                    },
                }
                if !fits {
                    break;
                }
            }
            if fits {
                premises[li] = (atom.pred, id);
                self.join(rule, order, depth + 1, delta, ranges, limits, binding, premises, out);
            }
            for v in bound_here {
                binding[v] = None;
            }
        }
    }
}

fn value(slot: Slot, binding: &[Option<Sym>]) -> Sym {
    match slot {
        Slot::Const(k) => k,
        Slot::Var(v) => binding[v].expect("a safe rule binds every variable it reads"),
    }
}

/// The least model of a program: every fact it derives, with how.
pub struct Model {
    symbols: Vec<String>,
    symbol_ids: HashMap<String, Sym>,
    preds: Vec<String>,
    pred_ids: HashMap<String, usize>,
    relations: Vec<Relation>,
    rule_names: Vec<String>,
    /// Per rule, whether it reads the closed world ([`Rule::reads_closed_world`]).
    rule_closed: Vec<bool>,
    fact_names: Vec<String>,
    work: u64,
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut m = f.debug_struct("Model");
        for (p, name) in self.preds.iter().enumerate() {
            m.field(name, &self.relations[p].tuples.len());
        }
        m.field("work", &self.work).finish()
    }
}

/// How one fact of a model holds: the rule that derived it and its premises, or nothing for a given fact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derivation {
    /// The rule that derived the fact; `None` for a fact the program states.
    pub rule: Option<String>,
    /// The facts the rule fired on, one for each positive body literal.
    pub premises: Vec<(String, Vec<String>)>,
    /// For a given fact, the name of the axiom that states it.
    pub given: Option<String>,
}

impl Model {
    /// Every tuple of `predicate`.
    pub fn relation(&self, predicate: &str) -> BTreeSet<Vec<String>> {
        let Some(&p) = self.pred_ids.get(predicate) else {
            return BTreeSet::new();
        };
        self.relations[p].tuples.iter().map(|t| self.names(t)).collect()
    }

    /// Whether `predicate(args…)` holds.
    pub fn holds(&self, predicate: &str, args: &[&str]) -> bool {
        self.tuple_id(predicate, args.iter().copied()).is_some()
    }

    /// Every fact, ordered by predicate and arguments.
    pub fn facts(&self) -> Vec<(String, Vec<String>)> {
        let mut out: Vec<(String, Vec<String>)> = self
            .relations
            .iter()
            .enumerate()
            .flat_map(|(p, r)| r.tuples.iter().map(move |t| (p, t)))
            .map(|(p, t)| (self.preds[p].clone(), self.names(t)))
            .collect();
        out.sort();
        out
    }

    /// How `predicate(args…)` holds, when it does.
    pub fn derivation(&self, predicate: &str, args: &[String]) -> Option<Derivation> {
        let p = *self.pred_ids.get(predicate)?;
        let id = self.tuple_id(predicate, args.iter().map(String::as_str))?;
        Some(match &self.relations[p].origins[id] {
            Origin::Given(f) => Derivation { rule: None, premises: Vec::new(), given: Some(self.fact_names[*f].clone()) },
            Origin::Derived(r, premises) => Derivation {
                rule: Some(self.rule_names[*r].clone()),
                premises: premises
                    .iter()
                    .map(|&(q, t)| (self.preds[q].clone(), self.names(&self.relations[q].tuples[t])))
                    .collect(),
                given: None,
            },
        })
    }

    /// Whether `predicate(args…)` rests on the closed world: a rule that derived it, or one that derived a fact it rests
    /// on, has a negation or a comparison in its condition. Such a fact holds in the least model, but the axioms alone
    /// do not entail it: that two names differ, or that one number lies below another, is no axiom. `false` for a fact
    /// that does not hold.
    pub fn closed_world(&self, predicate: &str, args: &[String]) -> bool {
        let (Some(&p), Some(id)) = (self.pred_ids.get(predicate), self.tuple_id(predicate, args.iter().map(String::as_str)))
        else {
            return false;
        };
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        let mut stack = vec![(p, id)];
        while let Some((q, t)) = stack.pop() {
            if !seen.insert((q, t)) {
                continue;
            }
            if let Origin::Derived(r, premises) = &self.relations[q].origins[t] {
                if self.rule_closed[*r] {
                    return true;
                }
                stack.extend(premises.iter().copied());
            }
        }
        false
    }

    /// The rule instances the evaluation tried: every assignment that satisfied a rule's positive body.
    pub fn work(&self) -> u64 {
        self.work
    }

    fn names(&self, t: &[Sym]) -> Vec<String> {
        t.iter().map(|&s| self.symbols[s as usize].clone()).collect()
    }

    fn tuple_id<'a>(&self, predicate: &str, args: impl Iterator<Item = &'a str>) -> Option<usize> {
        let p = *self.pred_ids.get(predicate)?;
        let tuple: Option<Box<[Sym]>> = args.map(|a| self.symbol_ids.get(a).copied()).collect();
        self.relations[p].members.get(&tuple?).copied()
    }
}

/// The least model, evaluated semi-naively, stratum by stratum.
pub fn evaluate(program: &Program) -> Result<Model, DatalogError> {
    run(program, true)
}

/// The least model, evaluated naively: every rule re-fired over the whole model until nothing changes.
pub fn evaluate_naive(program: &Program) -> Result<Model, DatalogError> {
    run(program, false)
}

fn run(program: &Program, semi_naive: bool) -> Result<Model, DatalogError> {
    program.validate()?;
    let strata = program.strata()?;
    let mut engine = Engine {
        symbols: Vec::new(),
        symbol_ids: HashMap::new(),
        preds: Vec::new(),
        pred_ids: HashMap::new(),
        relations: Vec::new(),
        work: 0,
        refused: None,
    };
    for (i, fact) in program.facts.iter().enumerate() {
        let p = engine.pred(&fact.predicate);
        let tuple: Box<[Sym]> = fact.ground().expect("a fact is ground").iter().map(|c| engine.symbol(c)).collect();
        if !engine.relations[p].members.contains_key(&tuple) {
            engine.relations[p].push(tuple, Origin::Given(i));
        }
    }
    let compiled: Vec<CompiledRule> = program.rules.iter().map(|r| engine.compile_rule(r)).collect();
    for stratum in strata {
        let heads: BTreeSet<usize> = stratum.iter().map(|&r| compiled[r].head.pred).collect();
        // Round 0 joins every rule over everything known; later rounds join each recursive literal over the delta.
        let mut ranges: Vec<(usize, usize)> = engine.relations.iter().map(|r| (0, r.tuples.len())).collect();
        let mut first = true;
        loop {
            let limits: Vec<usize> = engine.relations.iter().map(|r| r.tuples.len()).collect();
            let mut found: Vec<(usize, usize, Box<[Sym]>, Vec<(usize, usize)>)> = Vec::new();
            for &r in &stratum {
                let rule = &compiled[r];
                let deltas: Vec<Option<usize>> = if first || !semi_naive {
                    vec![None]
                } else {
                    rule.positive
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| heads.contains(&a.pred))
                        .map(|(i, _)| Some(i))
                        .collect()
                };
                for delta in deltas {
                    for (head, premises) in engine.instances(rule, delta, &ranges, &limits) {
                        found.push((r, rule.head.pred, head, premises));
                    }
                    if let Some(refused) = engine.refused.take() {
                        return Err(refused);
                    }
                }
            }
            let before: Vec<usize> = engine.relations.iter().map(|r| r.tuples.len()).collect();
            for (r, pred, head, premises) in found {
                if !engine.relations[pred].members.contains_key(&head) {
                    engine.relations[pred].push(head, Origin::Derived(r, premises));
                }
            }
            let grew = heads.iter().any(|&p| engine.relations[p].tuples.len() > before[p]);
            if !grew {
                break;
            }
            ranges = engine.relations.iter().enumerate().map(|(p, r)| (before[p], r.tuples.len())).collect();
            first = false;
        }
    }
    Ok(Model {
        symbols: engine.symbols,
        symbol_ids: engine.symbol_ids,
        preds: engine.preds,
        pred_ids: engine.pred_ids,
        relations: engine.relations,
        rule_names: program.rules.iter().map(|r| r.name.clone()).collect(),
        rule_closed: program.rules.iter().map(Rule::reads_closed_world).collect(),
        fact_names: program.fact_names.clone(),
        work: engine.work,
    })
}

// ── context numbering ──────────────────────────────────────────────────────────────────────────────────────────────

/// The call paths of a program's call graph, numbered as Whaley and Lam number them (PLDI 2004, §4).
///
/// Read from the program's [`CALLS`] relation: its facts, or the call graph its rules discover (Whaley and Lam's
/// Algorithm 6, a virtual call's targets from its receiver's context-insensitive points-to set), evaluated to completion
/// first. Stated calls come in the order the program states them, discovered ones after them in sorted order. The
/// graph's strongly connected components are numbered callers first. A
/// component no call enters from outside is a root, with one context. Any other has one context per path entering it:
/// each call into it from another component takes the next range of its numbers, as many as its caller has contexts,
/// in the order the program states the calls, and maps its caller's context `c` to `c + offset`. A call inside a
/// component keeps its context, so the methods of a recursive cycle share their contexts and the paths around the
/// cycle are not told apart. A method no call names has no contexts.
///
/// Every call path to a method then has its own number below the method's count, and the numbers a path's calls
/// compose to are exactly its own: the numbering is a bijection from paths to `0..count`. Since each call adds a
/// constant to a range, the relations it defines are diagrams linear in the bits however many paths there are.
#[derive(Debug, Clone)]
pub struct Contexts {
    methods: Vec<String>,
    index: HashMap<String, usize>,
    count: Vec<u64>,
    calls: Vec<CallEdge>,
    bits: u32,
}

#[derive(Debug, Clone)]
struct CallEdge {
    caller: usize,
    site: String,
    callee: usize,
    /// The offset a call entering the callee's component adds; `None` for a call inside one.
    offset: Option<u64>,
}

impl Contexts {
    /// Number the call paths of `program`'s call graph.
    pub fn number(program: &Program) -> Result<Self, DatalogError> {
        let call_graph = call_graph(program)?;
        let mut methods: Vec<String> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut method = |name: &str, methods: &mut Vec<String>| -> usize {
            *index.entry(name.to_string()).or_insert_with(|| {
                methods.push(name.to_string());
                methods.len() - 1
            })
        };
        let mut seen: HashSet<Vec<String>> = HashSet::new();
        let mut calls: Vec<CallEdge> = Vec::new();
        for g in call_graph {
            if !seen.insert(g.clone()) {
                continue;
            }
            let caller = method(&g[0], &mut methods);
            let callee = method(&g[2], &mut methods);
            calls.push(CallEdge { caller, site: g[1].clone(), callee, offset: None });
        }
        let n = methods.len();
        let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
        for call in &calls {
            edges[call.caller].push(call.callee);
        }
        let components = tarjan(&edges);
        let mut component_of = vec![0usize; n];
        for (c, members) in components.iter().enumerate() {
            for &m in members {
                component_of[m] = c;
            }
        }
        // Tarjan emits a component after every component it reaches: reversed, callers come first.
        let mut count = vec![0u64; n];
        for c in (0..components.len()).rev() {
            let mut next: Option<u64> = None;
            for call in calls.iter_mut() {
                if component_of[call.callee] == c && component_of[call.caller] != c {
                    let offset = next.unwrap_or(0);
                    call.offset = Some(offset);
                    next = Some(offset.checked_add(count[call.caller]).ok_or_else(|| DatalogError::ContextOverflow {
                        method: methods[call.callee].clone(),
                    })?);
                }
            }
            let here = next.unwrap_or(1);
            for &m in &components[c] {
                count[m] = here;
            }
        }
        let largest = count.iter().copied().max().unwrap_or(1).max(1);
        let bits = (64 - (largest - 1).leading_zeros()).max(1);
        Ok(Contexts { methods, index, count, calls, bits })
    }

    /// How many contexts `method` has: the call paths that reach it, or 0 for a method no call names.
    pub fn of(&self, method: &str) -> u64 {
        self.index.get(method).map_or(0, |&m| self.count[m])
    }

    /// Every method the call graph names, with its number of contexts, in the order the program first names them.
    pub fn methods(&self) -> impl Iterator<Item = (&str, u64)> {
        self.methods.iter().map(String::as_str).zip(self.count.iter().copied())
    }

    /// The contexts of every method together.
    pub fn total(&self) -> u128 {
        self.count.iter().map(|&c| u128::from(c)).sum()
    }

    /// The bits a context number takes.
    pub fn bits(&self) -> u32 {
        self.bits
    }

    /// [`CONTEXT`] and [`CALL_CONTEXT`] written out as facts, one per context: what the diagrams hold, for a program
    /// small enough to list it (the hash engine's input, or a check of the diagrams).
    pub fn facts(&self) -> Vec<Atom> {
        let mut out = Vec::new();
        for (m, &n) in self.methods.iter().zip(&self.count) {
            for c in 0..n {
                out.push(Atom::new(CONTEXT, vec![Term::constant(m), Term::constant(&c.to_string())]));
            }
        }
        for call in &self.calls {
            for c in 0..self.count[call.caller] {
                let d = call.offset.map_or(c, |k| c + k);
                out.push(Atom::new(
                    CALL_CONTEXT,
                    vec![
                        Term::constant(&c.to_string()),
                        Term::constant(&call.site),
                        Term::constant(&d.to_string()),
                        Term::constant(&self.methods[call.callee]),
                    ],
                ));
            }
        }
        out
    }
}

/// The program's call graph: its [`CALLS`] facts in the order stated, then the calls its rules discover, evaluated to
/// completion over the rules `Calls` depends on, in sorted order. Refused when that derivation reads [`CONTEXT`] or
/// [`CALL_CONTEXT`]: the contexts are numbered over the call graph, so it cannot depend on them.
fn call_graph(program: &Program) -> Result<Vec<Vec<String>>, DatalogError> {
    let mut graph = Vec::new();
    for fact in program.facts.iter().filter(|f| f.predicate == CALLS) {
        if fact.terms.len() != 3 {
            return Err(DatalogError::Arity { predicate: CALLS.to_string(), expected: 3, found: fact.terms.len() });
        }
        graph.push(fact.ground().expect("a fact is ground"));
    }
    if !program.rules.iter().any(|r| r.head.predicate == CALLS) {
        return Ok(graph);
    }
    let mut needed: BTreeSet<&str> = BTreeSet::from([CALLS]);
    loop {
        let before = needed.len();
        let read: Vec<&str> = program
            .rules
            .iter()
            .filter(|r| needed.contains(r.head.predicate.as_str()))
            .flat_map(|r| r.body.iter().map(|l| l.atom().predicate.as_str()))
            .collect();
        needed.extend(read);
        if needed.len() == before {
            break;
        }
    }
    let rules: Vec<Rule> = program
        .rules
        .iter()
        .filter(|r| needed.contains(r.head.predicate.as_str()))
        .cloned()
        .collect();
    if let Some(rule) = rules
        .iter()
        .find(|r| r.body.iter().any(|l| matches!(l.atom().predicate.as_str(), CONTEXT | CALL_CONTEXT)))
    {
        return Err(DatalogError::NotGiven { predicate: CALLS.to_string(), rule: rule.name.clone() });
    }
    let discovered = evaluate(&Program::new(program.facts.clone(), rules))?;
    let stated: HashSet<Vec<String>> = graph.iter().cloned().collect();
    graph.extend(discovered.relation(CALLS).into_iter().filter(|t| !stated.contains(t)));
    Ok(graph)
}

/// The argument positions that hold context numbers: those a rule's variables join, directly or through other rules,
/// to [`CONTEXT`]'s second argument or [`CALL_CONTEXT`]'s first or third. An argument that holds a constant, or a
/// method or site of the computed relations, holds no number.
fn number_positions(program: &Program) -> Result<HashSet<(String, usize)>, DatalogError> {
    let mut ids: HashMap<(String, usize), usize> = HashMap::new();
    let mut parent: Vec<usize> = Vec::new();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut id = |key: (String, usize), parent: &mut Vec<usize>| -> usize {
        *ids.entry(key).or_insert_with(|| {
            parent.push(parent.len());
            parent.len() - 1
        })
    };
    let mut held: Vec<(bool, String, usize)> = vec![
        (true, CONTEXT.to_string(), 0),
        (true, CALL_CONTEXT.to_string(), 1),
        (true, CALL_CONTEXT.to_string(), 3),
    ];
    let numbers = [(CONTEXT, 1usize), (CALL_CONTEXT, 0), (CALL_CONTEXT, 2)]
        .map(|(p, i)| id((p.to_string(), i), &mut parent));
    let computed = |p: &str| p == CONTEXT || p == CALL_CONTEXT;
    for fact in &program.facts {
        for i in 0..fact.terms.len() {
            held.push((computed(&fact.predicate), fact.predicate.clone(), i));
        }
    }
    for rule in &program.rules {
        let mut first: HashMap<&str, usize> = HashMap::new();
        for a in std::iter::once(&rule.head).chain(rule.body.iter().map(Literal::atom)) {
            for (i, t) in a.terms.iter().enumerate() {
                let here = id((a.predicate.clone(), i), &mut parent);
                match t {
                    Term::Const(_) => held.push((computed(&a.predicate), a.predicate.clone(), i)),
                    Term::Var(v) => {
                        let there = *first.entry(v.as_str()).or_insert(here);
                        let (x, y) = (find(&mut parent, here), find(&mut parent, there));
                        parent[x] = y;
                    }
                }
            }
        }
    }
    let roots: HashSet<usize> = numbers.iter().map(|&n| find(&mut parent, n)).collect();
    held.sort();
    held.dedup();
    for (_, predicate, position) in held {
        let Some(&here) = ids.get(&(predicate.clone(), position)) else {
            continue;
        };
        if roots.contains(&find(&mut parent, here)) {
            return Err(DatalogError::Sort { predicate, position });
        }
    }
    let keys: Vec<(String, usize)> = ids.keys().cloned().collect();
    Ok(keys
        .into_iter()
        .filter(|k| {
            let here = ids[k];
            roots.contains(&find(&mut parent, here))
        })
        .collect())
}

// ── relations as binary decision diagrams ──────────────────────────────────────────────────────────────────────────

/// The least model with every relation a binary decision diagram: bddbddb's representation.
///
/// Each constant is a binary code (in order of first appearance, so a chain's nodes count up). The diagram's
/// variables come in blocks of one code each: a block per argument position of a relation, then a block per variable
/// of a rule. Blocks are interleaved bit by bit, most significant bit first, so relating two codes (equality, order)
/// costs nodes linear in the bits rather than in the codes. A relation is the characteristic function of its tuples
/// over the position blocks. A rule joins its body atoms as relational products against equality constraints that
/// move each atom's positions onto the rule's variable blocks, then projects the result onto the head's positions the
/// same way; evaluation is semi-naive per stratum, as [`evaluate`]'s is, over diagrams instead of tuples.
///
/// Evaluated with [`Contexts`], an argument that holds context numbers holds the number itself in its block, and the
/// blocks are as wide as the larger of the constants' codes and the context numbers. [`BddModel::relation`] writes a
/// context in decimal, and [`BddModel::count`] counts a relation's tuples without listing them.
pub struct BddModel {
    bdd: crate::bdd::Bdd,
    constants: Vec<String>,
    codes: HashMap<String, u64>,
    bits: u32,
    blocks: u32,
    relations: HashMap<String, (crate::bdd::NodeId, usize)>,
    numbers: HashSet<(String, usize)>,
}

impl BddModel {
    /// Every tuple of `predicate`.
    pub fn relation(&self, predicate: &str) -> BTreeSet<Vec<String>> {
        let Some(&(root, arity)) = self.relations.get(predicate) else {
            return BTreeSet::new();
        };
        let vars: Vec<u32> = (0..arity as u32).flat_map(|p| self.block_vars(p)).collect();
        self.bdd
            .assignments(root, &vars)
            .into_iter()
            .map(|bits| {
                bits.chunks(self.bits as usize)
                    .enumerate()
                    .map(|(position, code)| {
                        let n = code.iter().fold(0u64, |acc, b| acc << 1 | u64::from(*b));
                        if self.numbers.contains(&(predicate.to_string(), position)) {
                            n.to_string()
                        } else {
                            self.constants[n as usize].clone()
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// Whether an argument of `predicate` holds context numbers: a relation with as many tuples as contexts, to be
    /// counted rather than listed.
    pub fn over_contexts(&self, predicate: &str) -> bool {
        self.numbers.iter().any(|(p, _)| p == predicate)
    }

    /// How many tuples `predicate` holds, counted on its diagram rather than listed.
    pub fn count(&self, predicate: &str) -> u128 {
        let Some(&(root, arity)) = self.relations.get(predicate) else {
            return 0;
        };
        let vars: Vec<u32> = (0..arity as u32).flat_map(|p| self.block_vars(p)).collect();
        self.bdd.sat_count_over(root, &vars)
    }

    /// Whether `predicate(args…)` holds.
    pub fn holds(&self, predicate: &str, args: &[&str]) -> bool {
        let Some(&(root, arity)) = self.relations.get(predicate) else {
            return false;
        };
        if args.len() != arity {
            return false;
        }
        let code = |(position, a): (usize, &&str)| -> Option<u64> {
            if self.numbers.contains(&(predicate.to_string(), position)) {
                a.parse::<u64>().ok().filter(|&n| u128::from(n) < 1u128 << self.bits)
            } else {
                self.codes.get(*a).copied()
            }
        };
        let Some(codes) = args.iter().enumerate().map(code).collect::<Option<Vec<u64>>>() else {
            return false;
        };
        let bits = self.bits;
        let blocks = self.blocks;
        self.bdd.eval(root, |v| {
            let (bit, block) = (v / blocks, v % blocks);
            (block as usize) < codes.len() && codes[block as usize] >> (bits - 1 - bit) & 1 == 1
        })
    }

    /// The decision nodes of `predicate`'s diagram.
    pub fn nodes(&self, predicate: &str) -> usize {
        self.relations.get(predicate).map_or(0, |&(root, _)| self.bdd.node_count(root))
    }

    /// Every node the evaluation built, shared across relations.
    pub fn total_nodes(&self) -> usize {
        self.bdd.size()
    }

    /// The variables of block `block`, most significant bit first.
    fn block_vars(&self, block: u32) -> Vec<u32> {
        (0..self.bits).map(|b| b * self.blocks + block).collect()
    }
}

struct DiagramEngine {
    bdd: crate::bdd::Bdd,
    bits: u32,
    blocks: u32,
    positions: u32,
}

impl DiagramEngine {
    fn var(&self, block: u32, bit: u32) -> u32 {
        bit * self.blocks + block
    }

    fn block_vars(&self, block: u32) -> Vec<u32> {
        (0..self.bits).map(|b| self.var(block, b)).collect()
    }

    /// The block's code is `code`.
    fn encode(&mut self, block: u32, code: u64) -> crate::bdd::NodeId {
        let mut f = self.bdd.constant(true);
        for b in (0..self.bits).rev() {
            let x = self.bdd.var(self.var(block, b));
            let literal = if code >> (self.bits - 1 - b) & 1 == 1 { x } else { self.bdd.not(x) };
            f = self.bdd.and(literal, f);
        }
        f
    }

    /// The two blocks hold the same code.
    fn equal(&mut self, a: u32, b: u32) -> crate::bdd::NodeId {
        let mut f = self.bdd.constant(true);
        for bit in (0..self.bits).rev() {
            let x = self.bdd.var(self.var(a, bit));
            let y = self.bdd.var(self.var(b, bit));
            let same = self.bdd.xor(x, y);
            let same = self.bdd.not(same);
            f = self.bdd.and(same, f);
        }
        f
    }

    /// The constraint tying an atom's argument positions to rule variable blocks or constant codes.
    fn binding(&mut self, atom: &Atom, var_block: &HashMap<&str, u32>, codes: &HashMap<String, u64>) -> crate::bdd::NodeId {
        let mut f = self.bdd.constant(true);
        for (i, t) in atom.terms.iter().enumerate() {
            let c = match t {
                Term::Var(v) => self.equal(i as u32, var_block[v.as_str()]),
                Term::Const(k) => self.encode(i as u32, codes[k]),
            };
            f = self.bdd.and(f, c);
        }
        f
    }

    /// The block holds a number below `bound`.
    fn below(&mut self, block: u32, bound: u64) -> crate::bdd::NodeId {
        if u128::from(bound) >= 1u128 << self.bits {
            return self.bdd.constant(true);
        }
        // From the least significant bit up: whether the bits so far are below `bound`'s.
        let mut f = self.bdd.constant(false);
        for significance in 0..self.bits {
            let x = self.bdd.var(self.var(block, self.bits - 1 - significance));
            f = if bound >> significance & 1 == 1 {
                let t = self.bdd.constant(true);
                self.bdd.ite(x, f, t)
            } else {
                let e = self.bdd.constant(false);
                self.bdd.ite(x, e, f)
            };
        }
        f
    }

    /// Block `to` holds block `from`'s number plus `k`, with no carry out of the top bit.
    fn plus(&mut self, from: u32, to: u32, k: u64) -> crate::bdd::NodeId {
        let (no, yes) = (self.bdd.constant(false), self.bdd.constant(true));
        // `carry[c]`: the bits below this one add up, carrying `c` into it.
        let mut carry = [yes, no];
        for significance in 0..self.bits {
            let level = self.bits - 1 - significance;
            let x = self.bdd.var(self.var(from, level));
            let y = self.bdd.var(self.var(to, level));
            let k_bit = (k >> significance & 1) as usize;
            let mut next = [no, no];
            for c_in in 0..2 {
                if carry[c_in] == no {
                    continue;
                }
                let differ = self.bdd.xor(x, y);
                let sum = if (k_bit ^ c_in) == 1 { differ } else { self.bdd.not(differ) };
                for (c_out, slot) in next.iter_mut().enumerate() {
                    let carries = match (k_bit + c_in, c_out) {
                        (0, 0) | (2, 1) => yes,
                        (0, _) | (2, _) => no,
                        (_, 1) => x,
                        _ => self.bdd.not(x),
                    };
                    let both = self.bdd.and(sum, carries);
                    let term = self.bdd.and(both, carry[c_in]);
                    *slot = self.bdd.or(*slot, term);
                }
            }
            carry = next;
        }
        carry[0]
    }

    /// The atom's relation over the rule's variable blocks.
    fn atom_over_variables(
        &mut self,
        relation: crate::bdd::NodeId,
        atom: &Atom,
        var_block: &HashMap<&str, u32>,
        codes: &HashMap<String, u64>,
    ) -> crate::bdd::NodeId {
        let tie = self.binding(atom, var_block, codes);
        let positions: Vec<u32> = (0..atom.terms.len() as u32).flat_map(|p| self.block_vars(p)).collect();
        self.bdd.rel_prod(relation, tie, &positions)
    }
}

/// The least model over binary decision diagrams (see [`BddModel`]).
pub fn evaluate_bdd(program: &Program) -> Result<BddModel, DatalogError> {
    run_bdd(program, None)
}

/// The least model over binary decision diagrams, with the rules reading [`CONTEXT`] and [`CALL_CONTEXT`] as
/// `contexts` numbers the program's call graph: bddbddb's context-sensitive evaluation.
pub fn evaluate_bdd_with_contexts(program: &Program, contexts: &Contexts) -> Result<BddModel, DatalogError> {
    run_bdd(program, Some(contexts))
}

fn run_bdd(program: &Program, contexts: Option<&Contexts>) -> Result<BddModel, DatalogError> {
    use crate::bdd::{Bdd, NodeId};
    program.validate()?;
    if let Some(rule) = program.rules.iter().find(|r| !r.guards.is_empty()) {
        return Err(DatalogError::Guarded { rule: rule.name.clone() });
    }
    let strata = program.strata()?;
    let numbers = match contexts {
        None => HashSet::new(),
        Some(_) => {
            let heads = program.rules.iter().map(|r| &r.head);
            if let Some(a) = program.facts.iter().chain(heads).find(|a| a.predicate == CONTEXT || a.predicate == CALL_CONTEXT) {
                return Err(DatalogError::Computed { predicate: a.predicate.clone() });
            }
            for rule in &program.rules {
                for a in rule.body.iter().map(Literal::atom) {
                    let expected = match a.predicate.as_str() {
                        CONTEXT => 2,
                        CALL_CONTEXT => 4,
                        _ => continue,
                    };
                    if a.terms.len() != expected {
                        return Err(DatalogError::Arity { predicate: a.predicate.clone(), expected, found: a.terms.len() });
                    }
                }
            }
            number_positions(program)?
        }
    };
    let mut constants: Vec<String> = Vec::new();
    let mut codes: HashMap<String, u64> = HashMap::new();
    let atoms = program
        .facts
        .iter()
        .chain(program.rules.iter().flat_map(|r| std::iter::once(&r.head).chain(r.body.iter().map(Literal::atom))));
    let mut arity: HashMap<String, usize> = HashMap::new();
    for a in atoms {
        arity.insert(a.predicate.clone(), a.terms.len());
        for t in &a.terms {
            if let Term::Const(c) = t {
                if !codes.contains_key(c) {
                    codes.insert(c.clone(), constants.len() as u64);
                    constants.push(c.clone());
                }
            }
        }
    }
    if let Some(cx) = contexts {
        let named = cx.methods.iter().chain(cx.calls.iter().map(|c| &c.site));
        for name in named {
            if !codes.contains_key(name) {
                codes.insert(name.clone(), constants.len() as u64);
                constants.push(name.clone());
            }
        }
        arity.insert(CONTEXT.to_string(), 2);
        arity.insert(CALL_CONTEXT.to_string(), 4);
    }
    let symbol_bits = (64 - (constants.len().max(2) as u64 - 1).leading_zeros()).max(1);
    let bits = symbol_bits.max(contexts.map_or(0, Contexts::bits));
    let positions = arity.values().copied().max().unwrap_or(0) as u32;
    let rule_vars = program.rules.iter().map(|r| r.variables().len()).max().unwrap_or(0) as u32;
    let blocks = (positions + rule_vars).max(1);
    let mut e = DiagramEngine { bdd: Bdd::new(bits * blocks), bits, blocks, positions };
    let mut relations: HashMap<String, NodeId> = arity.keys().map(|p| (p.clone(), e.bdd.constant(false))).collect();
    for fact in &program.facts {
        let mut tuple = e.bdd.constant(true);
        for (i, c) in fact.ground().expect("a fact is ground").iter().enumerate() {
            let code = e.encode(i as u32, codes[c]);
            tuple = e.bdd.and(tuple, code);
        }
        let r = relations[&fact.predicate];
        let r = e.bdd.or(r, tuple);
        relations.insert(fact.predicate.clone(), r);
    }
    if let Some(cx) = contexts {
        let mut context = e.bdd.constant(false);
        for (m, &n) in cx.methods.iter().zip(&cx.count) {
            let method = e.encode(0, codes[m]);
            let below = e.below(1, n);
            let tuples = e.bdd.and(method, below);
            context = e.bdd.or(context, tuples);
        }
        relations.insert(CONTEXT.to_string(), context);
        let mut call_context = e.bdd.constant(false);
        for call in &cx.calls {
            let site = e.encode(1, codes[&call.site]);
            let callee = e.encode(3, codes[&cx.methods[call.callee]]);
            let range = e.below(0, cx.count[call.caller]);
            let step = match call.offset {
                Some(k) => e.plus(0, 2, k),
                None => e.equal(0, 2),
            };
            let named = e.bdd.and(site, callee);
            let ranged = e.bdd.and(range, step);
            let tuples = e.bdd.and(named, ranged);
            call_context = e.bdd.or(call_context, tuples);
        }
        relations.insert(CALL_CONTEXT.to_string(), call_context);
    }
    for stratum in strata {
        let heads: BTreeSet<&str> = stratum.iter().map(|&r| program.rules[r].head.predicate.as_str()).collect();
        let mut delta: HashMap<String, NodeId> = HashMap::new();
        let mut first = true;
        loop {
            let mut derived: HashMap<String, NodeId> = HashMap::new();
            for &r in &stratum {
                let rule = &program.rules[r];
                let names: Vec<String> = rule.variables().into_iter().collect();
                let var_block: HashMap<&str, u32> =
                    names.iter().enumerate().map(|(i, v)| (v.as_str(), e.positions + i as u32)).collect();
                let positive: Vec<&Atom> = rule
                    .body
                    .iter()
                    .filter_map(|l| match l {
                        Literal::Pos(a) => Some(a),
                        Literal::Neg(_) => None,
                    })
                    .collect();
                let recursive: Vec<Option<usize>> = if first {
                    vec![None]
                } else {
                    positive
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| heads.contains(a.predicate.as_str()))
                        .map(|(i, _)| Some(i))
                        .collect()
                };
                for on_delta in recursive {
                    let mut body = e.bdd.constant(true);
                    for (i, atom) in positive.iter().enumerate() {
                        let source = if on_delta == Some(i) {
                            delta.get(&atom.predicate).copied().unwrap_or(e.bdd.constant(false))
                        } else {
                            relations[&atom.predicate]
                        };
                        let over_vars = e.atom_over_variables(source, atom, &var_block, &codes);
                        body = e.bdd.and(body, over_vars);
                    }
                    for literal in &rule.body {
                        if let Literal::Neg(atom) = literal {
                            let over_vars = e.atom_over_variables(relations[&atom.predicate], atom, &var_block, &codes);
                            body = e.bdd.diff(body, over_vars);
                        }
                    }
                    let tie = e.binding(&rule.head, &var_block, &codes);
                    let rule_vars: Vec<u32> = var_block.values().flat_map(|&b| e.block_vars(b)).collect();
                    let head = e.bdd.rel_prod(body, tie, &rule_vars);
                    let so_far = derived.get(&rule.head.predicate).copied().unwrap_or(e.bdd.constant(false));
                    let joined = e.bdd.or(so_far, head);
                    derived.insert(rule.head.predicate.clone(), joined);
                }
            }
            delta.clear();
            let mut grew = false;
            for (pred, new) in derived {
                let old = relations[&pred];
                let fresh = e.bdd.diff(new, old);
                if fresh != e.bdd.constant(false) {
                    grew = true;
                    let all = e.bdd.or(old, fresh);
                    relations.insert(pred.clone(), all);
                    delta.insert(pred, fresh);
                }
            }
            if !grew {
                break;
            }
            first = false;
        }
    }
    Ok(BddModel {
        bdd: e.bdd,
        constants,
        codes,
        bits,
        blocks,
        relations: relations.into_iter().map(|(p, root)| (p.clone(), (root, arity[&p]))).collect(),
        numbers,
    })
}
