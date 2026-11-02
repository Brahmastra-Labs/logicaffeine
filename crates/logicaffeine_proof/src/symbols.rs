//! The symbols a problem names, read in one walk by everything that must agree on them. The kernel
//! registration in [`crate::verify`] types each predicate, function and constant from it and files
//! each under its kernel name ([`KernelNames`]), and the engine and the certifier both take what
//! its definite descriptions presuppose ([`SymbolCollector::presuppositions`]) as premises.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use crate::{DerivationTree, MatchArm, ProofExpr, ProofTerm};

/// Collects predicate and constant names from a `ProofExpr`. Predicates are
/// keyed to their arity (the max seen) so they register with the right type
/// `Entity → … → Entity → Prop`, which is what lets relations like `shaves(a,b)`
/// type-check.
pub(crate) struct SymbolCollector {
    predicates: HashMap<String, usize>,
    functions: HashMap<String, usize>,
    constants: HashSet<String>,
    /// Lowercase `Constant` names — candidate individual constants. The pipeline
    /// capitalizes proper names, but a definite description ("the butler") yields
    /// a lowercase `Constant("butler")`. Such a name IS an individual constant
    /// unless the SAME name is also used as a genuine variable (a quantifier
    /// binder or a `Variable` occurrence), in which case the lowercase `Constant`
    /// is a stray variable spelling and must stay unregistered.
    weak_constants: HashSet<String>,
    /// Names that occur as a genuine variable — a quantifier binder or a
    /// `ProofTerm::Variable`. A `weak_constants` candidate colliding with one of
    /// these is NOT registered as a constant.
    variables: HashSet<String>,
    /// Atoms that occur in an arithmetic/comparison argument position, so must be
    /// typed `Int` (not the `Entity` default) — `le`/`lt`/… need `Int` operands.
    int_atoms: HashSet<String>,
    int_functions: HashSet<String>,
    /// Unary predicates applied to an inductive-type constructor (`Zero`/`Succ` →
    /// `Nat`, `Nil`/`Cons` → `List`), mapped to that inductive type's name, so they
    /// are typed `<Ind> → Prop` (not the `Entity` default) — the motive of a
    /// structural-`induction` proof is `λx:Ind. P(x)`, so `P` must accept `Ind`.
    inductive_predicates: HashMap<String, &'static str>,
    /// Temporal modality operators (`Past`, `Future`, …) seen wrapping a
    /// proposition, each registered as an opaque `Op : Prop → Prop`.
    modalities: HashSet<String>,
    /// Names bound by a quantifier. A binder is NOT a free variable, so it must
    /// not be registered — the `Pi`/`Lambda` the goal type builds binds it.
    binders: HashSet<String>,
    /// Names that occur as a `ProofTerm::Variable`/`BoundVarRef`. Those minus
    /// `binders` are the statement's FREE variables, which the certifier lowers
    /// to `Term::Var` and the kernel therefore needs as LOCAL bindings.
    var_occurrences: HashSet<String>,
    /// Which unary predicates a variable stands as the SOLE argument of — its
    /// MOTIVE positions. `P(n)` with `P : Nat → Prop` says `n : Nat`, exactly as
    /// `le(n, 3)` says `n : Int`; that half was noticed and this one was not, so a
    /// free variable in an inductive position stayed `Entity` and its own
    /// statement would not type-check.
    unary_predicate_arg: HashMap<String, std::collections::BTreeSet<String>>,
    /// Variables seen ANYWHERE ELSE — inside a function, in an equality, as one
    /// argument of several. A variable used at two different sorts keeps the
    /// `Entity` default rather than being typed by one of them.
    other_position_vars: HashSet<String>,
    /// Every equation and every application of an uninterpreted function, kept for
    /// [`SymbolCollector::settle_sorts`].
    identities: Vec<(ProofTerm, ProofTerm)>,
    applications: Vec<(String, Vec<ProofTerm>)>,
    /// The argument positions of each uninterpreted function that take an `Int`.
    int_domains: HashMap<String, std::collections::BTreeSet<usize>>,
}

impl SymbolCollector {
    pub(crate) fn new() -> Self {
        SymbolCollector {
            predicates: HashMap::new(),
            functions: HashMap::new(),
            constants: HashSet::new(),
            weak_constants: HashSet::new(),
            variables: HashSet::new(),
            int_atoms: HashSet::new(),
            int_functions: HashSet::new(),
            inductive_predicates: HashMap::new(),
            modalities: HashSet::new(),
            binders: HashSet::new(),
            var_occurrences: HashSet::new(),
            unary_predicate_arg: HashMap::new(),
            other_position_vars: HashSet::new(),
            identities: Vec::new(),
            applications: Vec::new(),
            int_domains: HashMap::new(),
        }
    }

    /// Whether a term has sort `Int` by what is known so far: a numeral, an `Int` atom,
    /// arithmetic, or an `Int`-valued function's application.
    fn is_int_sorted(&self, term: &ProofTerm) -> bool {
        match term {
            ProofTerm::Constant(s) if crate::numeral::is_numeral(s) => true,
            ProofTerm::Constant(s) | ProofTerm::Variable(s) | ProofTerm::BoundVarRef(s) => self.int_atoms.contains(s),
            ProofTerm::Function(name, _) => {
                matches!(name.as_str(), "add" | "sub" | "mul" | "div" | "mod" | "Add" | "Sub" | "Mul" | "Div")
                    || self.int_functions.contains(name)
            }
            _ => false,
        }
    }

    /// Settle the sorts of the statement's terms: the two sides of an equation share a sort,
    /// and every application of a function passes the same sort at a given position. A side or
    /// an argument known to be `Int` makes its partner `Int`, to a fixpoint, so `f(3) = 3` types
    /// `f : Int → Int` and `f(3) = f(x)` types `x : Int`, where the per-occurrence rules left
    /// `f : Entity → Entity` applied to and equated with an `Int`.
    pub(crate) fn settle_sorts(&mut self) {
        loop {
            let before = (self.int_atoms.len(), self.int_functions.len(), self.int_domain_count());
            for (l, r) in self.identities.clone() {
                match (self.is_int_sorted(&l), self.is_int_sorted(&r)) {
                    (true, false) => self.mark_int(&r),
                    (false, true) => self.mark_int(&l),
                    _ => {}
                }
            }
            for (name, args) in self.applications.clone() {
                for (i, arg) in args.iter().enumerate() {
                    if self.is_int_sorted(arg) {
                        self.int_domains.entry(name.clone()).or_default().insert(i);
                    }
                }
                let positions = self.int_domains.get(&name).cloned().unwrap_or_default();
                for i in positions {
                    if let Some(arg) = args.get(i) {
                        self.mark_int(arg);
                    }
                }
            }
            if (self.int_atoms.len(), self.int_functions.len(), self.int_domain_count()) == before {
                return;
            }
        }
    }

    fn int_domain_count(&self) -> usize {
        self.int_domains.values().map(|positions| positions.len()).sum()
    }

    /// The argument positions of `name` that take an `Int`.
    pub(crate) fn int_domain(&self, name: &str) -> std::collections::BTreeSet<usize> {
        self.int_domains.get(name).cloned().unwrap_or_default()
    }

    /// Mark a term that sits in an Int position. Atoms become `Int`; nested
    /// arithmetic propagates the Int-ness to its own operands.
    fn mark_int(&mut self, term: &ProofTerm) {
        match term {
            // Numeric literals lower to `Lit(int_lit(n))` (certifier) at ARBITRARY
            // precision — no declaration. Asking `crate::numeral` rather than
            // `parse::<i64>` is what keeps this layer's window and the certifier's
            // the same one: a numeral past `i64::MAX` used to be registered here as
            // an opaque `Int` ATOM while the kernel computes it exactly.
            ProofTerm::Constant(s) if crate::numeral::is_numeral(s) => {}
            ProofTerm::Constant(s) | ProofTerm::Variable(s) | ProofTerm::BoundVarRef(s) => {
                self.int_atoms.insert(s.clone());
            }
            ProofTerm::Function(name, args)
                if matches!(
                    name.as_str(),
                    "add" | "sub" | "mul" | "div" | "mod" | "Add" | "Sub" | "Mul" | "Div"
                ) =>
            {
                for a in args {
                    self.mark_int(a);
                }
            }
            // An UNINTERPRETED function standing in an Int position is Int-VALUED. Only its
            // atoms used to be typed, so `f` in `f(a) ≤ d` stayed `Entity → Entity` and every
            // order step over it was ill-typed at the door: `le_refl` wants `Π a:Int`, and
            // `le_refl (f a)` reported "expected Int, found Entity" about a correct proof.
            // The ARGUMENTS stay whatever they are — `f`'s domain is not the position's.
            ProofTerm::Function(name, args) => {
                self.int_functions.insert(name.clone());
                self.note_function(name, args.len());
            }
            _ => {}
        }
    }

    fn note_predicate(&mut self, name: &str, arity: usize) {
        self.predicates
            .entry(name.to_string())
            .and_modify(|a| *a = (*a).max(arity))
            .or_insert(arity);
    }

    /// A `ProofTerm::Function` is Entity-valued (a function on entities), never a
    /// proposition — only `ProofExpr::Predicate` is propositional. Registering it
    /// as `Entity → … → Entity` (not `… → Prop`) lets `F(a)` appear inside terms
    /// like `Eq Entity (F a) (F b)`, the basis for congruence.
    fn note_function(&mut self, name: &str, arity: usize) {
        self.functions
            .entry(name.to_string())
            .and_modify(|a| *a = (*a).max(arity))
            .or_insert(arity);
    }

    pub(crate) fn collect(&mut self, expr: &ProofExpr) {
        match expr {
            ProofExpr::Predicate { name, args, .. } => {
                self.note_predicate(name, args.len());
                // A unary predicate applied to an inductive-type constructor is
                // `<Ind> → Prop`, so it can be the motive of a structural induction.
                if args.len() == 1 {
                    if let Some(ind) = constructor_domain(&args[0]) {
                        self.inductive_predicates.insert(name.clone(), ind);
                    }
                }
                // A bare variable as a unary predicate's only argument is a MOTIVE position:
                // the predicate's domain is the variable's sort. Recorded rather than applied
                // here, because the constructor that reveals that domain may appear in a later
                // premise (`P(n)` can be read before `P(Zero)` is).
                match (args.len(), args.first()) {
                    (1, Some(ProofTerm::Variable(var) | ProofTerm::BoundVarRef(var))) => {
                        self.variables.insert(var.clone());
                        self.var_occurrences.insert(var.clone());
                        self.unary_predicate_arg
                            .entry(var.clone())
                            .or_default()
                            .insert(name.clone());
                    }
                    _ => {
                        for arg in args {
                            self.collect_term(arg);
                        }
                    }
                }
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                self.collect(l);
                self.collect(r);
            }
            ProofExpr::Not(inner) => self.collect(inner),
            ProofExpr::ForAll { variable, body } | ProofExpr::Exists { variable, body } => {
                self.variables.insert(variable.clone());
                self.binders.insert(variable.clone());
                self.collect(body);
            }
            // A temporal modality `Op(P)`: note the operator (registered as
            // `Op : Prop → Prop`) and recurse so `P`'s own predicates register.
            ProofExpr::Temporal { operator, body } => {
                self.modalities.insert(operator.clone());
                self.collect(body);
            }
            ProofExpr::Identity(l, r) => {
                self.identities.push((l.clone(), r.clone()));
                self.collect_term(l);
                self.collect_term(r);
            }
            // Atoms are propositional constants, not FOL predicates.
            _ => {}
        }
    }

    fn collect_term(&mut self, term: &ProofTerm) {
        match term {
            ProofTerm::Constant(name) => {
                // Proper names (capitalized) and numeric labels (a year like `2004`)
                // are unambiguously Entity constants. A lowercase name — a definite
                // description's referent ("the butler" → `Constant("butler")`) — is a
                // constant TOO, but only if it is never used as a genuine variable
                // (a binder or `Variable` occurrence); that collision is resolved in
                // `constants()`, once every symbol has been seen.
                if name
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase() || c.is_ascii_digit())
                    .unwrap_or(false)
                {
                    self.constants.insert(name.clone());
                } else {
                    self.weak_constants.insert(name.clone());
                }
            }
            ProofTerm::Variable(name) | ProofTerm::BoundVarRef(name) => {
                self.variables.insert(name.clone());
                self.var_occurrences.insert(name.clone());
                self.other_position_vars.insert(name.clone());
            }
            ProofTerm::Function(name, args) => {
                // Arithmetic / comparison builtins are prelude globals (typed
                // `Int → … → Int`/`Bool`); don't re-declare them, but type their
                // operands as `Int`. Other functions are uninterpreted (Entity).
                if matches!(
                    name.as_str(),
                    "le" | "lt" | "ge" | "gt" | "add" | "sub" | "mul" | "div" | "mod"
                        | "Add" | "Sub" | "Mul" | "Div"
                ) {
                    for arg in args {
                        self.mark_int(arg);
                    }
                } else {
                    self.note_function(name, args.len());
                    self.applications.push((name.clone(), args.clone()));
                }
                for arg in args {
                    self.collect_term(arg);
                }
            }
            _ => {}
        }
    }

    /// A definition's definiendum, a predicate of its parameters' arity whether or not the
    /// problem applies it — so the kernel names it like any other predicate.
    pub(crate) fn collect_definiendum(&mut self, name: &str, arity: usize) {
        self.note_predicate(name, arity);
    }

    pub(crate) fn predicates(&self) -> impl Iterator<Item = (&String, usize)> {
        self.predicates.iter().map(|(n, a)| (n, *a))
    }

    pub(crate) fn functions(&self) -> impl Iterator<Item = (&String, usize)> {
        self.functions.iter().map(|(n, a)| (n, *a))
    }

    pub(crate) fn int_atoms(&self) -> impl Iterator<Item = &String> {
        self.int_atoms.iter()
    }

    /// Uninterpreted functions that stand in an arithmetic or comparison position, and are
    /// therefore Int-VALUED rather than the `Entity → … → Entity` default.
    pub(crate) fn is_int_valued(&self, name: &str) -> bool {
        self.int_functions.contains(name)
    }

    /// The statement's FREE variables, each with the kernel domain it is used at.
    ///
    /// A `ProofTerm::Variable` that no quantifier binds is what a Lean `variable (n : Int)`
    /// declares: free in the theorem, and implicitly universally quantified around it. The
    /// certifier lowers it to `Term::Var(n)`, and `Context::get` reads only `bindings`, so
    /// without a LOCAL binding the goal type itself is unbound — `∃m. m = n + 1` could not be
    /// stated, let alone proved, and the error (`UnboundVariable("n")`) named the certifier's
    /// last step rather than the missing declaration. Registering them as locals rather than
    /// globals is the faithful choice: `Term::Var` is what refers to them.
    pub(crate) fn free_variables(&self) -> Vec<(&String, &'static str)> {
        self.var_occurrences
            .iter()
            .filter(|n| !self.binders.contains(n.as_str()))
            .map(|n| (n, self.free_variable_domain(n)))
            .collect()
    }

    /// The kernel domain a free variable is USED at.
    ///
    /// `Int` when it stands in an arithmetic or comparison position — the half already known.
    /// The other half: a variable that is only ever the sole argument of unary predicates, all
    /// of which some premise has already typed at the same INDUCTIVE (`P(Zero)` types
    /// `P : Nat → Prop`), is a value of that inductive. Registered as `Entity` it made its own
    /// statement ill-typed — `P n` with `P : Nat → Prop` and `n : Entity` — so `∀`-free
    /// theorems about a local of an inductive type could not be stated, let alone proved by
    /// induction on it.
    ///
    /// Deliberately conservative: one contrary occurrence anywhere else, or two predicates
    /// disagreeing about the inductive, and the `Entity` default stands. Typing a variable by
    /// one of two incompatible uses would be worse than not typing it.
    fn free_variable_domain(&self, name: &str) -> &'static str {
        if self.int_atoms.contains(name) {
            return "Int";
        }
        if self.other_position_vars.contains(name) {
            return "Entity";
        }
        let Some(predicates) = self.unary_predicate_arg.get(name) else {
            return "Entity";
        };
        let mut domain: Option<&'static str> = None;
        for predicate in predicates {
            match self.inductive_predicates.get(predicate.as_str()) {
                Some(ind) if domain.is_none() || domain == Some(*ind) => domain = Some(*ind),
                _ => return "Entity",
            }
        }
        domain.unwrap_or("Entity")
    }

    pub(crate) fn inductive_predicates(&self) -> impl Iterator<Item = (&String, &'static str)> {
        self.inductive_predicates.iter().map(|(n, d)| (n, *d))
    }

    pub(crate) fn constants(&self) -> Vec<&String> {
        // Capitalized/numeric constants always register; a lowercase candidate
        // registers only when the name is never a genuine variable.
        self.constants
            .iter()
            .chain(
                self.weak_constants
                    .iter()
                    .filter(|n| !self.variables.contains(*n)),
            )
            .collect()
    }

    pub(crate) fn modalities(&self) -> impl Iterator<Item = &String> {
        self.modalities.iter()
    }

    /// What the problem's definite descriptions presuppose, beyond the facts `known` already
    /// states. "The butler" reaches the prover as a lowercase constant spelled as its noun,
    /// `butler`, and a description asserts its noun of what it names: the butler is a butler. So
    /// every lowercase constant that is also a noun — a predicate the problem never applies to more
    /// than one argument — yields `butler(butler)`, even where the same name also binds a variable
    /// (a conditional quantifying over `butler`). A proper name (`Butler`) presupposes nothing, and
    /// neither does a noun the problem also uses as a relation ("Jeeves is Wooster's butler"): its
    /// description presupposes the relation, and the kernel types the noun as one. The facts come
    /// out sorted, so the search meets them in the same order on every run.
    pub(crate) fn presuppositions(&self, known: &[ProofExpr]) -> Vec<ProofExpr> {
        let mut described: Vec<&String> = self
            .weak_constants
            .iter()
            .filter(|name| self.predicates.get(*name) == Some(&1))
            .collect();
        described.sort();
        described
            .into_iter()
            .map(|noun| ProofExpr::Predicate {
                name: noun.clone(),
                args: vec![ProofTerm::Constant(noun.clone())],
                world: None,
            })
            .filter(|fact| !known.contains(fact))
            .collect()
    }

    /// The kernel's names for the problem's predicates (see [`KernelNames`]). A predicate the
    /// kernel also declares as an individual — one of [`Self::constants`] — is filed as
    /// `is_<name>`, with `_` appended while that spelling is `taken` (one of the kernel's own
    /// globals) or names another symbol of the problem. A predicate that is itself one of the
    /// kernel's globals keeps its name.
    pub(crate) fn kernel_names(&self, taken: impl Fn(&str) -> bool) -> KernelNames {
        let individuals: HashSet<&String> = self.constants().into_iter().collect();
        let mut shared: Vec<&String> = self
            .predicates
            .keys()
            .filter(|name| individuals.contains(name) && !taken(name))
            .collect();
        shared.sort();
        let mut renamed: HashMap<String, String> = HashMap::new();
        for name in shared {
            let mut kernel_name = format!("is_{name}");
            while taken(&kernel_name)
                || self.spells(&kernel_name)
                || renamed.values().any(|other| *other == kernel_name)
            {
                kernel_name.push('_');
            }
            renamed.insert(name.clone(), kernel_name);
        }
        KernelNames { renamed }
    }

    /// Whether some symbol of the problem is spelled `name`.
    fn spells(&self, name: &str) -> bool {
        self.predicates.contains_key(name)
            || self.functions.contains_key(name)
            || self.constants.contains(name)
            || self.weak_constants.contains(name)
            || self.variables.contains(name)
            || self.int_atoms.contains(name)
            || self.modalities.contains(name)
    }
}

/// The names the kernel files a problem's predicates under. First-order logic keeps predicates
/// and individuals apart, so the noun `butler` and "the butler" it names can share a spelling
/// (`butler(butler)`); the kernel has one namespace, where the second declaration would find the
/// first and the fact would apply the predicate to itself. So a predicate that shares its name
/// with an individual takes a name of its own there, and every other predicate keeps its own.
/// The renaming reaches every premise, the goal and the derivation alike, so it changes nothing a
/// proof proves — only where the kernel files the predicate.
pub(crate) struct KernelNames {
    renamed: HashMap<String, String>,
}

impl KernelNames {
    /// The kernel's name for the predicate `name`.
    pub(crate) fn predicate<'a>(&'a self, name: &'a str) -> &'a str {
        self.renamed.get(name).map_or(name, String::as_str)
    }

    /// `expr` as the kernel reads it — borrowed untouched when no predicate is renamed.
    pub(crate) fn expr<'e>(&self, expr: &'e ProofExpr) -> Cow<'e, ProofExpr> {
        if self.renamed.is_empty() {
            Cow::Borrowed(expr)
        } else {
            Cow::Owned(self.rename(expr))
        }
    }

    /// `tree` as the kernel reads it — borrowed untouched when no predicate is renamed. Rule
    /// payloads and substitutions name individuals and variables, never predicates, so only the
    /// conclusions change.
    pub(crate) fn tree<'t>(&self, tree: &'t DerivationTree) -> Cow<'t, DerivationTree> {
        if self.renamed.is_empty() {
            Cow::Borrowed(tree)
        } else {
            Cow::Owned(self.rename_tree(tree))
        }
    }

    fn rename_tree(&self, tree: &DerivationTree) -> DerivationTree {
        DerivationTree {
            conclusion: self.rename(&tree.conclusion),
            rule: tree.rule.clone(),
            premises: tree.premises.iter().map(|premise| self.rename_tree(premise)).collect(),
            depth: tree.depth,
            substitution: tree.substitution.clone(),
        }
    }

    fn rename(&self, expr: &ProofExpr) -> ProofExpr {
        let under = |inner: &ProofExpr| Box::new(self.rename(inner));
        match expr {
            ProofExpr::Predicate { name, args, world } => ProofExpr::Predicate {
                name: self.predicate(name).to_string(),
                args: args.clone(),
                world: world.clone(),
            },
            ProofExpr::And(l, r) => ProofExpr::And(under(l), under(r)),
            ProofExpr::Or(l, r) => ProofExpr::Or(under(l), under(r)),
            ProofExpr::Implies(l, r) => ProofExpr::Implies(under(l), under(r)),
            ProofExpr::Iff(l, r) => ProofExpr::Iff(under(l), under(r)),
            ProofExpr::Not(inner) => ProofExpr::Not(under(inner)),
            ProofExpr::ForAll { variable, body } => {
                ProofExpr::ForAll { variable: variable.clone(), body: under(body) }
            }
            ProofExpr::Exists { variable, body } => {
                ProofExpr::Exists { variable: variable.clone(), body: under(body) }
            }
            ProofExpr::Modal { domain, force, flavor, body } => ProofExpr::Modal {
                domain: domain.clone(),
                force: *force,
                flavor: flavor.clone(),
                body: under(body),
            },
            ProofExpr::Counterfactual { antecedent, consequent } => ProofExpr::Counterfactual {
                antecedent: under(antecedent),
                consequent: under(consequent),
            },
            ProofExpr::Temporal { operator, body } => {
                ProofExpr::Temporal { operator: operator.clone(), body: under(body) }
            }
            ProofExpr::TemporalBinary { operator, left, right } => ProofExpr::TemporalBinary {
                operator: operator.clone(),
                left: under(left),
                right: under(right),
            },
            ProofExpr::Lambda { variable, body } => {
                ProofExpr::Lambda { variable: variable.clone(), body: under(body) }
            }
            ProofExpr::App(function, argument) => ProofExpr::App(under(function), under(argument)),
            ProofExpr::Ctor { name, args } => ProofExpr::Ctor {
                name: name.clone(),
                args: args.iter().map(|arg| self.rename(arg)).collect(),
            },
            ProofExpr::Match { scrutinee, arms } => ProofExpr::Match {
                scrutinee: under(scrutinee),
                arms: arms
                    .iter()
                    .map(|arm| MatchArm {
                        ctor: arm.ctor.clone(),
                        bindings: arm.bindings.clone(),
                        body: self.rename(&arm.body),
                    })
                    .collect(),
            },
            ProofExpr::Fixpoint { name, body } => {
                ProofExpr::Fixpoint { name: name.clone(), body: under(body) }
            }
            // No predicate symbol: terms, atoms, holes — and events, which are abstracted to
            // predicates before anything reaches the kernel.
            ProofExpr::Identity(..)
            | ProofExpr::Atom(_)
            | ProofExpr::NeoEvent { .. }
            | ProofExpr::TypedVar { .. }
            | ProofExpr::Hole(_)
            | ProofExpr::Term(_)
            | ProofExpr::Unsupported(_) => expr.clone(),
        }
    }
}

/// The inductive type whose constructor `t` is — `Zero`/`Succ` ⇒ `Nat`,
/// `ENil`/`ECons` ⇒ `EList` — or `None` if `t` is not a recognized prelude
/// constructor. The signal that a unary predicate applied to `t` ranges over that
/// inductive type, not `Entity`, so it can serve as a structural-induction motive
/// `λx:Ind. P(x)`. (The `E`-prefixed list names are the prelude's monomorphic
/// `EList`, distinct from a user program's parametric `List`/`Nil`/`Cons`.)
fn constructor_domain(t: &ProofTerm) -> Option<&'static str> {
    let name = match t {
        ProofTerm::Constant(s) => s.as_str(),
        ProofTerm::Function(n, _) => n.as_str(),
        _ => return None,
    };
    match name {
        "Zero" | "Succ" => Some("Nat"),
        "ENil" | "ECons" => Some("EList"),
        _ => None,
    }
}
