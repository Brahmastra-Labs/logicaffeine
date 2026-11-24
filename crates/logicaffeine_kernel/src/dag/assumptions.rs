//! The assumption policy over an accepted export, with Lean's `#print axioms` semantics.
//!
//! Lean's kernel admits any axiom; what a result may *rest on* is a policy above it. The policy here
//! ([`Policy::Lean`]) is Lean's own: a declaration may depend only on `propext`, `Quot.sound` and
//! `Classical.choice`, each in its canonical statement. An axiom is *depended on* when a declaration
//! reaches it through its type or value (a definition's, theorem's or opaque's), its constructors'
//! types or its recursor's rules, transitively — the axioms an axiom's own type mentions do not
//! count, exactly as `#print axioms` collects them. Every other axiom a declaration depends on —
//! `sorryAx`, `Lean.ofReduceBool`, `Lean.ofReduceNat`, `Lean.trustCompiler`, `lcProof`, any custom
//! axiom — and an allowed name declared with another statement is a violation, reported with one
//! dependency path from a declaration to it. Declaring an axiom nothing depends on is no violation:
//! Lean's core declares `sorryAx`.
//!
//! The audit reads the checked environment the core built; it never re-checks a declaration.

use super::env::Kind;
use super::expr::{ExprId, Node};
use super::name::NameId;
use super::tc::Kernel;
use rustc_hash::{FxHashMap as FxMap, FxHashSet as FxSet};
use std::collections::VecDeque;

/// Which axioms a result may depend on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Any axiom the export declares (the Arena protocol's checking).
    Any,
    /// Lean's three standard axioms, in their canonical statements.
    Lean,
}

impl Policy {
    pub fn as_str(self) -> &'static str {
        match self {
            Policy::Any => "any",
            Policy::Lean => "lean",
        }
    }
}

/// Lean's standard axioms, by name.
pub const LEAN_AXIOMS: [&str; 3] = ["propext", "Quot.sound", "Classical.choice"];

/// One axiom a declaration depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub axiom: String,
    /// The declaration found depending on it, and the path from it to the axiom (inclusive).
    pub path: Vec<String>,
    /// Why the policy refuses it, when it does.
    pub refused: Option<String>,
}

/// What the export's declarations depend on, and what the policy refuses of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Audit {
    /// Every axiom some declaration depends on, sorted by name.
    pub dependencies: Vec<Dependency>,
    /// Axioms the export declares that nothing depends on.
    pub unused: Vec<String>,
}

impl Audit {
    pub fn refusals(&self) -> impl Iterator<Item = &Dependency> {
        self.dependencies.iter().filter(|d| d.refused.is_some())
    }
    pub fn admitted(&self) -> bool {
        self.refusals().next().is_none()
    }
}

/// The constants an expression mentions (constants and projections' structures), once each.
fn mentioned(k: &Kernel, root: ExprId, memo: &mut FxSet<ExprId>, out: &mut FxSet<NameId>) {
    let mut stack = vec![root];
    while let Some(e) = stack.pop() {
        if !memo.insert(e) {
            continue;
        }
        match k.s.node(e) {
            Node::Const(n, _) => {
                out.insert(n);
            }
            Node::Proj(n, _, x) => {
                out.insert(n);
                stack.push(x);
            }
            Node::App(f, a) => stack.extend([f, a]),
            Node::Lam(t, b) | Node::Pi(t, b) => stack.extend([t, b]),
            Node::Let(t, v, b) => stack.extend([t, v, b]),
            Node::Match(d, m, c) => stack.extend([d, m, c]),
            Node::Fix(b) | Node::MutualFix(b, _) => stack.push(b),
            Node::Seq(a, b) => stack.extend([a, b]),
            Node::BVar(_)
            | Node::FVar(_)
            | Node::Sort(_)
            | Node::NatLit(_)
            | Node::StrLit(_)
            | Node::Nil
            | Node::Lit(_) => {}
        }
    }
}

/// The constants a declaration reaches directly, as `#print axioms` traverses it.
fn edges(k: &Kernel, name: NameId) -> Vec<NameId> {
    let Some(c) = k.env.get(name) else {
        return Vec::new();
    };
    let mut memo = FxSet::default();
    let mut out = FxSet::default();
    match &c.kind {
        Kind::Axiom { .. } => return Vec::new(),
        Kind::Defn { value, .. } | Kind::Thm { value } | Kind::Opaque { value, .. } => {
            mentioned(k, c.ty, &mut memo, &mut out);
            mentioned(k, *value, &mut memo, &mut out);
        }
        Kind::Induct(info) => {
            mentioned(k, c.ty, &mut memo, &mut out);
            out.extend(info.ctors.iter().copied());
            out.extend(info.all.iter().copied());
        }
        Kind::Ctor(info) => {
            mentioned(k, c.ty, &mut memo, &mut out);
            out.insert(info.induct);
        }
        Kind::Rec(info) => {
            mentioned(k, c.ty, &mut memo, &mut out);
            for rule in info.rules.iter() {
                mentioned(k, rule.rhs, &mut memo, &mut out);
            }
        }
        Kind::Quot(_) => mentioned(k, c.ty, &mut memo, &mut out),
    }
    out.remove(&name);
    let mut out: Vec<NameId> = out.into_iter().collect();
    out.sort_by_key(|n| n.0);
    out
}

/// The canonical statement of a standard axiom, built in the kernel's own store, when the export
/// declares everything it names.
fn canonical(k: &mut Kernel, name: &str, lparams: &[NameId]) -> Option<ExprId> {
    let s = &mut k.s;
    let named = |s: &mut super::expr::Store, n: &str| s.names.dotted(n);
    match (name, lparams) {
        ("propext", []) => {
            let iff = named(s, "Iff");
            let eq = named(s, "Eq");
            let prop = s.prop();
            let one = s.levels.of_nat(1);
            let iff_c = s.cnst_levels(iff, &[]);
            let eq_c = s.cnst_levels(eq, &[one]);
            let (b1, b0, b2) = (s.bvar(1), s.bvar(0), s.bvar(2));
            let premise = s.mk_app_n(iff_c, &[b1, b0]);
            let b1_ = s.bvar(1);
            let conclusion = s.mk_app_n(eq_c, &[prop, b2, b1_]);
            let arrow = s.pi(premise, conclusion);
            let inner = s.pi(prop, arrow);
            Some(s.pi(prop, inner))
        }
        ("Quot.sound", [u]) => {
            let quot = named(s, "Quot");
            let mk = named(s, "Quot.mk");
            let eq = named(s, "Eq");
            let lu = s.levels.param(*u);
            let sort_u = s.sort(lu);
            let prop = s.prop();
            // {α : Sort u} {r : α → α → Prop} {a b : α}, r a b → Quot.mk r a = Quot.mk r b
            let b0 = s.bvar(0);
            let b1 = s.bvar(1);
            let r_ty_inner = s.pi(b1, prop);
            let r_ty = s.pi(b0, r_ty_inner);
            let a_ty = s.bvar(1);
            let b_ty = s.bvar(2);
            let (r, a, b) = (s.bvar(2), s.bvar(1), s.bvar(0));
            let premise = s.mk_app_n(r, &[a, b]);
            let (alpha, r4, a4, b4) = (s.bvar(4), s.bvar(3), s.bvar(2), s.bvar(1));
            let quot_c = s.cnst_levels(quot, &[lu]);
            let quot_ty = s.mk_app_n(quot_c, &[alpha, r4]);
            let mk_c = s.cnst_levels(mk, &[lu]);
            let mk_a = s.mk_app_n(mk_c, &[alpha, r4, a4]);
            let mk_b = s.mk_app_n(mk_c, &[alpha, r4, b4]);
            let eq_c = s.cnst_levels(eq, &[lu]);
            let conclusion = s.mk_app_n(eq_c, &[quot_ty, mk_a, mk_b]);
            let h = s.pi(premise, conclusion);
            let over_b = s.pi(b_ty, h);
            let over_a = s.pi(a_ty, over_b);
            let over_r = s.pi(r_ty, over_a);
            Some(s.pi(sort_u, over_r))
        }
        ("Classical.choice", [u]) => {
            let nonempty = named(s, "Nonempty");
            let lu = s.levels.param(*u);
            let sort_u = s.sort(lu);
            let nonempty_c = s.cnst_levels(nonempty, &[lu]);
            let b0 = s.bvar(0);
            let hyp = s.app(nonempty_c, b0);
            let b1 = s.bvar(1);
            let body = s.pi(hyp, b1);
            Some(s.pi(sort_u, body))
        }
        _ => None,
    }
}

/// Why `policy` refuses a dependency on the axiom `name`, or `None` when it admits it.
fn refusal(k: &mut Kernel, policy: Policy, name: NameId) -> Option<String> {
    if policy == Policy::Any {
        return None;
    }
    let shown = k.s.names.display(name);
    if !LEAN_AXIOMS.contains(&shown.as_str()) {
        return Some(match shown.as_str() {
            "sorryAx" => "an admission (`sorry`): it proves anything".to_string(),
            "Lean.ofReduceBool" | "Lean.ofReduceNat" => {
                "a native-code decision (`native_decide`): it trusts the compiled evaluator"
                    .to_string()
            }
            "Lean.trustCompiler" => "the compiler-trust axiom".to_string(),
            "lcProof" => "the code generator's proof placeholder".to_string(),
            _ => "an axiom outside Lean's standard three".to_string(),
        });
    }
    let c = k.env.get(name)?.clone();
    let expected = canonical(k, &shown, &c.lparams);
    if expected != Some(c.ty) {
        return Some(format!(
            "declared with a statement other than Lean's canonical {shown}"
        ));
    }
    None
}

/// The axioms the export's declarations depend on — all of them, or `targets`' alone — held to
/// `policy`.
pub fn audit(k: &mut Kernel, policy: Policy, targets: Option<&[NameId]>) -> Audit {
    let names: Vec<NameId> = k.env.constants().map(|c| c.name).collect();
    let axioms: FxSet<NameId> = k
        .env
        .constants()
        .filter(|c| matches!(c.kind, Kind::Axiom { .. }))
        .map(|c| c.name)
        .collect();
    let mut forward: FxMap<NameId, Vec<NameId>> = FxMap::default();
    for &n in &names {
        forward.insert(n, edges(k, n));
    }
    // From each starting declaration, breadth first, so the recorded path is a shortest one.
    let starts: Vec<NameId> = match targets {
        Some(targets) => targets.to_vec(),
        None => names
            .iter()
            .copied()
            .filter(|n| !axioms.contains(n))
            .collect(),
    };
    let mut reached: FxMap<NameId, Vec<NameId>> = FxMap::default();
    let mut parent: FxMap<NameId, NameId> = FxMap::default();
    let mut seen: FxSet<NameId> = FxSet::default();
    let mut queue: VecDeque<NameId> = VecDeque::new();
    for &start in &starts {
        if seen.insert(start) {
            queue.push_back(start);
        }
    }
    while let Some(n) = queue.pop_front() {
        if axioms.contains(&n) {
            let mut path = vec![n];
            let mut at = n;
            while let Some(&p) = parent.get(&at) {
                path.push(p);
                at = p;
            }
            path.reverse();
            reached.insert(n, path);
            continue;
        }
        for &next in forward.get(&n).map(Vec::as_slice).unwrap_or(&[]) {
            if seen.insert(next) {
                parent.insert(next, n);
                queue.push_back(next);
            }
        }
    }
    let mut dependencies: Vec<Dependency> = Vec::new();
    for (axiom, path) in reached {
        let refused = refusal(k, policy, axiom);
        dependencies.push(Dependency {
            axiom: k.s.names.display(axiom),
            path: path.iter().map(|n| k.s.names.display(*n)).collect(),
            refused,
        });
    }
    dependencies.sort_by(|a, b| a.axiom.cmp(&b.axiom));
    let used: FxSet<String> = dependencies.iter().map(|d| d.axiom.clone()).collect();
    let mut unused: Vec<String> = axioms
        .iter()
        .map(|a| k.s.names.display(*a))
        .filter(|a| !used.contains(a))
        .collect();
    unused.sort();
    Audit {
        dependencies,
        unused,
    }
}
