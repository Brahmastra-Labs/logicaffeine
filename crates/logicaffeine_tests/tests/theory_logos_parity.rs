//! LOGOS's theory in the `dag` core (`Theory::Logos`) decides what the `Term` kernel decides.
//!
//! The core is checked against the `Term` kernel as its oracle, at the kernel interface: every
//! declaration of the standard library is admitted, and each paradox of `kernel_paradox_corpus` is
//! put to both kernels as the same declaration, after the same accepted setup, and refused by both,
//! each by the rule that must refuse it. The accepted controls beside each paradox are admitted by
//! both, so a refusal is the rule's and not an incidental type error. A disagreement is a finding
//! about one of the two kernels, triaged to its cause.

use logicaffeine_kernel::dag::logos_env::{
    admit, candidate_block, candidate_definition, replay, Declaration,
};
use logicaffeine_kernel::interface::{Repl, TermParser};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{Context, MutualInductive, Term};
use std::collections::BTreeSet;

fn on_a_large_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .expect("spawns")
        .join()
        .expect("joins")
}

fn term(text: &str) -> Term {
    TermParser::parse(text).unwrap_or_else(|e| panic!("`{text}` parses: {e:?}"))
}

/// One declaration put to both kernels.
enum Candidate {
    Definition {
        name: &'static str,
        levels: &'static [&'static str],
        ty: &'static str,
        body: &'static str,
    },
    Inductive {
        name: &'static str,
        levels: &'static [&'static str],
        arity: &'static str,
        num_params: usize,
        ctors: &'static [(&'static str, &'static str)],
    },
}

impl Candidate {
    fn name(&self) -> &'static str {
        match self {
            Candidate::Definition { name, .. } | Candidate::Inductive { name, .. } => name,
        }
    }

    /// The `Term` kernel's verdict, through the checked door the declaration's kind takes.
    fn term_verdict(&self, ctx: &Context) -> Result<(), String> {
        let mut ctx = ctx.clone();
        let levels = |ls: &[&str]| ls.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        match self {
            Candidate::Definition {
                name,
                levels: ls,
                ty,
                body,
            } if ls.is_empty() => ctx.declare(name, term(ty), term(body)),
            Candidate::Definition {
                name,
                levels: ls,
                ty,
                body,
            } => ctx.declare_universe_poly(name, levels(ls), term(ty), term(body)),
            Candidate::Inductive {
                name,
                levels: ls,
                arity,
                num_params,
                ctors,
            } => {
                let block = [MutualInductive {
                    name: name.to_string(),
                    sort: term(arity),
                    num_params: *num_params,
                    constructors: ctors
                        .iter()
                        .map(|(c, t)| (c.to_string(), term(t)))
                        .collect(),
                }];
                if ls.is_empty() {
                    ctx.add_mutual_inductives(&block)
                } else {
                    ctx.add_universe_poly_inductives(&levels(ls), &block)
                }
            }
        }
        .map_err(|e| e.to_string())?;
        // A declaration the door accepts as pending (it names something not yet declared) is not
        // admitted.
        match ctx
            .pending_obligations()
            .iter()
            .find(|(name, _)| *name == self.name())
        {
            Some((name, missing)) => {
                Err(format!("`{name}` is pending on `{missing}`, not admitted"))
            }
            None => Ok(()),
        }
    }

    /// The same declaration as its door puts it to the core: its terms as the context stores
    /// them, under the telescope the door gives it.
    fn declarations(&self, ctx: &Context) -> Vec<Declaration> {
        let levels = |ls: &[&str]| ls.iter().map(|l| l.to_string()).collect::<Vec<_>>();
        match self {
            Candidate::Definition {
                name,
                levels: ls,
                ty,
                body,
            } => {
                let ls = levels(ls);
                vec![candidate_definition(
                    ctx,
                    name,
                    (!ls.is_empty()).then_some(ls.as_slice()),
                    &term(ty),
                    &term(body),
                )]
            }
            Candidate::Inductive {
                name,
                levels: ls,
                arity,
                num_params,
                ctors,
            } => {
                let ls = levels(ls);
                candidate_block(
                    ctx,
                    (!ls.is_empty()).then_some(ls.as_slice()),
                    &[MutualInductive {
                        name: name.to_string(),
                        sort: term(arity),
                        num_params: *num_params,
                        constructors: ctors
                            .iter()
                            .map(|(c, t)| (c.to_string(), term(t)))
                            .collect(),
                    }],
                )
            }
        }
    }
}

/// A paradox's refused step, or a control both kernels admit, after `setup`.
struct Case {
    label: &'static str,
    setup: &'static [&'static str],
    candidate: Candidate,
    /// `None`: admitted by both. `Some((term_rule, core_rule))`: refused by each kernel's rule.
    refused_by: Option<(&'static str, &'static str)>,
}

fn verdicts(case: &Case) -> (Result<(), String>, Result<(), String>) {
    let mut repl = Repl::new();
    for command in case.setup {
        if let Err(e) = repl.execute(command) {
            panic!("{}: setup `{command}` must be accepted: {e}", case.label);
        }
    }
    let ctx = repl.context().clone();
    let term = case.candidate.term_verdict(&ctx);
    let (mut k, replayed) = replay(&ctx);
    let refused: Vec<String> = replayed
        .iter()
        .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
        .collect();
    assert!(
        refused.is_empty(),
        "{}: the core must admit the setup as the Term kernel did: {refused:#?}",
        case.label
    );
    let core = admit(&mut k, &ctx, &case.candidate.declarations(&ctx));
    (term, core)
}

const CASES: &[Case] = &[
    Case {
        label: "Coq 8.4's propext × guard",
        setup: &[
            "Inductive Tw : Prop := tw : (False -> Tw) -> Tw.",
            "Definition tw_back : Tw -> False -> Tw := fun (t : Tw) => fun (x : False) => t.",
            "Definition heq : Eq Prop (False -> Tw) Tw := \
             propext (False -> Tw) Tw (conj ((False -> Tw) -> Tw) (Tw -> False -> Tw) tw tw_back).",
        ],
        candidate: Candidate::Definition {
            name: "con",
            levels: &[],
            ty: "Tw -> False",
            body: "fix con => fun (x : Tw) => match x return (fun (q : Tw) => False) with \
                   fun (f : False -> Tw) => con (Eq_rec Prop (False -> Tw) (fun (T : Prop) => T) f Tw heq) end",
        },
        refused_by: Some(("Termination violation", "Termination violation")),
    },
    Case {
        label: "SPEC F: a match owes no case for a constructor its indices rule out",
        setup: &["Inductive Vec (A : Type) : Nat -> Type := vnil : Vec A Zero | vcons : (n : Nat) -> A -> Vec A n -> Vec A (Succ n)."],
        candidate: Candidate::Definition {
            name: "vhead1",
            levels: &[],
            ty: "Vec Nat (Succ Zero) -> Nat",
            body: "fun (v : Vec Nat (Succ Zero)) => match v return (fun (i : Nat) (w : Vec Nat i) => Nat) with \
                   fun (n : Nat) (a : Nat) (rest : Vec Nat n) => a end",
        },
        refused_by: None,
    },
    Case {
        label: "SPEC F: an open index rules no constructor out",
        setup: &["Inductive Vec (A : Type) : Nat -> Type := vnil : Vec A Zero | vcons : (n : Nat) -> A -> Vec A n -> Vec A (Succ n)."],
        candidate: Candidate::Definition {
            name: "vhead_open",
            levels: &[],
            ty: "forall (m : Nat), Vec Nat m -> Nat",
            body: "fun (m : Nat) (v : Vec Nat m) => match v return (fun (i : Nat) (w : Vec Nat i) => Nat) with \
                   fun (n : Nat) (a : Nat) (rest : Vec Nat n) => a end",
        },
        refused_by: Some(("Wrong number of cases", "wrong number of cases")),
    },
    Case {
        label: "K: a match on a proof of `a = a` computes as on `refl a`",
        setup: &[],
        candidate: Candidate::Definition {
            name: "k_computes",
            levels: &[],
            ty: "forall (A : Type), forall (a : A), forall (h : Eq A a a), \
                 Eq A (match h return (fun (z : A) (e : Eq A a z) => A) with a end) a",
            body: "fun (A : Type) (a : A) (h : Eq A a a) => refl A a",
        },
        refused_by: None,
    },
    Case {
        label: "Girard: a constructor storing its own universe",
        setup: &[],
        candidate: Candidate::Inductive { name: "U", levels: &[], arity: "Type", num_params: 0, ctors: &[("mk", "Type -> U")] },
        refused_by: Some(("universe", "universe inconsistency")),
    },
    Case {
        label: "Girard through a function",
        setup: &[],
        candidate: Candidate::Inductive { name: "V", levels: &[], arity: "Type", num_params: 0, ctors: &[("mkv", "(Type -> Prop) -> V")] },
        refused_by: Some(("universe", "universe inconsistency")),
    },
    Case {
        label: "a constructor that does not return its family",
        setup: &[],
        candidate: Candidate::Inductive { name: "Z", levels: &[], arity: "Type", num_params: 0, ctors: &[("zmk", "Nat")] },
        refused_by: Some(("result is not the declared inductive", "must return its family")),
    },
    Case {
        label: "a constructor whose parameter is not its family's",
        setup: &[],
        candidate: Candidate::Inductive {
            name: "PBox",
            levels: &[],
            arity: "forall (A : Type), Type",
            num_params: 1,
            ctors: &[("pmk", "forall (B : Prop), B -> PBox B")],
        },
        refused_by: Some(("parameter domain differs from the parent telescope", "at a type other than its family's")),
    },
    Case {
        label: "a constructor one universe up (control)",
        setup: &[],
        candidate: Candidate::Inductive { name: "Up", levels: &[], arity: "Type1", num_params: 0, ctors: &[("mkup", "Type -> Up")] },
        refused_by: None,
    },
    Case {
        label: "a universe as its own member",
        setup: &[],
        candidate: Candidate::Definition { name: "self", levels: &[], ty: "Type", body: "Type" },
        refused_by: Some(("Type", "declaration type mismatch")),
    },
    Case {
        label: "Coquand: a proposition with data eliminated into data",
        setup: &["Inductive Bx : Prop := bmk : Type -> Bx."],
        candidate: Candidate::Definition {
            name: "unbx",
            levels: &[],
            ty: "Bx -> Type",
            body: "fun (b : Bx) => match b return (fun (q : Bx) => Type) with fun (T : Type) => T end",
        },
        refused_by: Some(("large elimination", "large elimination")),
    },
    Case {
        label: "two equal proofs computing different numbers",
        setup: &["Inductive PB : Prop := pt : PB | pf : PB."],
        candidate: Candidate::Definition {
            name: "pick",
            levels: &[],
            ty: "PB -> Nat",
            body: "fun (p : PB) => match p return (fun (q : PB) => Nat) with Zero | Succ Zero end",
        },
        refused_by: Some(("large elimination", "large elimination")),
    },
    Case {
        label: "a subsingleton proposition eliminated into data (control)",
        setup: &["Inductive PU : Prop := pu : PU."],
        candidate: Candidate::Definition {
            name: "one",
            levels: &[],
            ty: "PU -> Nat",
            body: "fun (p : PU) => match p return (fun (q : PU) => Nat) with Succ Zero end",
        },
        refused_by: None,
    },
    Case {
        label: "an SProp eliminated into data",
        setup: &["Inductive SB : SProp := st : SB | sf : SB."],
        candidate: Candidate::Definition {
            name: "spick",
            levels: &[],
            ty: "SB -> Nat",
            body: "fun (s : SB) => match s return (fun (q : SB) => Nat) with Zero | Succ Zero end",
        },
        refused_by: Some(("definitionally-irrelevant", "definitionally-irrelevant")),
    },
    Case {
        label: "an SProp eliminated into Prop",
        setup: &["Inductive SB : SProp := st : SB | sf : SB."],
        candidate: Candidate::Definition {
            name: "sprop_split",
            levels: &[],
            ty: "SB -> Prop",
            body: "fun (s : SB) => match s return (fun (q : SB) => Prop) with True | False end",
        },
        refused_by: Some(("definitionally-irrelevant", "definitionally-irrelevant")),
    },
    Case {
        label: "an SProp storing a type: SProp is impredicative (control)",
        setup: &[],
        candidate: Candidate::Inductive { name: "SBig", levels: &[], arity: "SProp", num_params: 0, ctors: &[("sbig", "Type -> SBig")] },
        refused_by: None,
    },
    Case {
        label: "a squash of any sort into SProp (control)",
        setup: &[],
        candidate: Candidate::Inductive {
            name: "SSquash",
            levels: &["u"],
            arity: "forall (A : Sort u), SProp",
            num_params: 1,
            ctors: &[("ssquash", "forall (A : Sort u) (a : A), SSquash.{u} A")],
        },
        refused_by: None,
    },
    Case {
        label: "Coquand's impredicative Set: a type stored in an SProp projected back out",
        setup: &["Inductive SBig : SProp := sbig : Type -> SBig."],
        candidate: Candidate::Definition {
            name: "carrier",
            levels: &[],
            ty: "SBig -> Type",
            body: "fun (s : SBig) => match s return (fun (q : SBig) => Type) with fun (T : Type) => T end",
        },
        refused_by: Some(("definitionally-irrelevant", "definitionally-irrelevant")),
    },
    Case {
        label: "the quotient a kernel installs computes as the standard library's does (control)",
        setup: &[
            "Inductive PEq.{u} (A : Sort u) (a : A) : A -> Prop := prefl : PEq.{u} A a a.",
            "Quotient Type.{u}.",
            "Quotient Constructor.{u}.",
            "Quotient Lift.{u, v} using PEq.",
            "Quotient Induction.{u}.",
            "Definition R : Nat -> Nat -> Prop := fun (a : Nat) (b : Nat) => True.",
            "Definition resp : forall (a : Nat) (b : Nat), R a b -> PEq.{1} Nat Zero Zero := \
             fun (a : Nat) (b : Nat) (h : R a b) => prefl.{1} Nat Zero.",
        ],
        candidate: Candidate::Definition {
            name: "lifted",
            levels: &[],
            ty: "Eq Nat (kernel.quot.lift.{1, 1} Nat R Nat (fun (a : Nat) => Zero) resp (kernel.quot.mk.{1} Nat R (Succ Zero))) Zero",
            body: "refl Nat Zero",
        },
        refused_by: None,
    },
    Case {
        label: "a family written over `Sort u` binds `u` implicitly (control)",
        setup: &["Inductive QUnit : Sort u := qunit : QUnit."],
        candidate: Candidate::Definition { name: "q1", levels: &[], ty: "QUnit", body: "qunit" },
        refused_by: None,
    },
    Case {
        label: "an implicitly polymorphic family's auxiliary at a use site (control)",
        setup: &["Inductive QUnit : Sort u := qunit : QUnit."],
        candidate: Candidate::Definition {
            name: "qb",
            levels: &[],
            ty: "Type",
            body: "QUnit_below (fun (x : QUnit) => Nat) qunit",
        },
        refused_by: None,
    },
    Case {
        label: "a bare constructor at the level of a definition around it",
        setup: &[
            "Inductive B2 : Sort u := b2t : B2 | b2f : B2.",
            "Definition P2 : Sort u := B2.",
            "Definition pirr : forall (P : Prop) (p : P) (q : P), Eq P p q := fun (P : Prop) => fun (p : P) => fun (q : P) => refl P p.",
        ],
        candidate: Candidate::Definition {
            name: "e_bare",
            levels: &[],
            ty: "Eq P2.{0} b2t b2f",
            body: "pirr P2.{0} b2t b2f",
        },
        refused_by: Some(("Type mismatch", "application type mismatch")),
    },
    Case {
        label: "the constructors equal where the family is a proposition (control)",
        setup: &[
            "Inductive B2 : Sort u := b2t : B2 | b2f : B2.",
            "Definition P2 : Sort u := B2.",
            "Definition pirr : forall (P : Prop) (p : P) (q : P), Eq P p q := fun (P : Prop) => fun (p : P) => fun (q : P) => refl P p.",
        ],
        candidate: Candidate::Definition {
            name: "e0",
            levels: &[],
            ty: "Eq P2.{0} b2t.{0} b2f.{0}",
            body: "pirr P2.{0} b2t.{0} b2f.{0}",
        },
        refused_by: None,
    },
    Case {
        label: "a derived recursor's level instantiated apart from its family's",
        setup: &[
            "Inductive B2 : Sort u := b2t : B2 | b2f : B2.",
            "Definition P2 : Sort u := B2.",
            "Definition pirr : forall (P : Prop) (p : P) (q : P), Eq P p q := fun (P : Prop) => fun (p : P) => fun (q : P) => refl P p.",
        ],
        candidate: Candidate::Definition {
            name: "D_bare",
            levels: &[],
            ty: "B2 -> Prop",
            body: "B2_rec.{1} (fun (x : B2) => Prop) True False",
        },
        refused_by: Some(("Type mismatch", "application type mismatch")),
    },
    Case {
        label: "the family eliminated into data where it is not a proposition (control)",
        setup: &[
            "Inductive B2 : Sort u := b2t : B2 | b2f : B2.",
            "Definition P2 : Sort u := B2.",
            "Definition pirr : forall (P : Prop) (p : P) (q : P), Eq P p q := fun (P : Prop) => fun (p : P) => fun (q : P) => refl P p.",
        ],
        candidate: Candidate::Definition {
            name: "D1",
            levels: &[],
            ty: "B2.{1} -> Prop",
            body: "B2_rec.{1} (fun (x : B2.{1}) => Prop) True False",
        },
        refused_by: None,
    },
    Case {
        label: "a family over `Sort u` whose constructor names it bare (control)",
        setup: &[],
        candidate: Candidate::Inductive {
            name: "QU2",
            levels: &[],
            arity: "Sort u",
            num_params: 0,
            ctors: &[("qu2", "QU2"), ("qu2s", "QU2 -> QU2")],
        },
        refused_by: None,
    },
    Case {
        label: "a body mentioning a level its type does not (control)",
        setup: &["Inductive QUnit : Sort u := qunit : QUnit."],
        candidate: Candidate::Definition {
            name: "q2",
            levels: &[],
            ty: "Nat",
            body: "(fun (x : QUnit) => Zero) qunit",
        },
        refused_by: None,
    },
    Case {
        label: "Russell through a negative occurrence",
        setup: &[],
        candidate: Candidate::Inductive { name: "Bad", levels: &[], arity: "Type", num_params: 0, ctors: &[("mk", "(Bad -> False) -> Bad")] },
        refused_by: Some(("Positivity", "Positivity")),
    },
    Case {
        label: "Russell through a container",
        setup: &[],
        candidate: Candidate::Inductive { name: "Bad2", levels: &[], arity: "Type", num_params: 0, ctors: &[("mk2", "TList (Bad2 -> False) -> Bad2")] },
        refused_by: Some(("Positivity", "Positivity")),
    },
    Case {
        label: "a strictly positive container (control)",
        setup: &[],
        candidate: Candidate::Inductive { name: "Good", levels: &[], arity: "Type", num_params: 0, ctors: &[("mkg", "TList Good -> Good")] },
        refused_by: None,
    },
    Case {
        label: "a fixpoint with no decreasing argument",
        setup: &[],
        candidate: Candidate::Definition {
            name: "spin",
            levels: &[],
            ty: "Nat -> False",
            body: "fix spin => fun (n : Nat) => match n return (fun (q : Nat) => False) with spin Zero | fun (k : Nat) => spin (Succ k) end",
        },
        refused_by: Some(("Termination violation", "Termination violation")),
    },
    Case {
        label: "a structural fixpoint (control)",
        setup: &[],
        candidate: Candidate::Definition {
            name: "down",
            levels: &[],
            ty: "Nat -> Nat",
            body: "fix down => fun (n : Nat) => match n return (fun (q : Nat) => Nat) with Zero | fun (k : Nat) => down k end",
        },
        refused_by: None,
    },
    Case {
        label: "the refutation refinement over an open decision",
        setup: &[
            "Definition pred0 : Nat -> Nat := fun (n : Nat) => match n return (fun (q : Nat) => Nat) with | Zero => Zero | Succ k => k end.",
            "Definition dz : forall (n : Nat) (e : False), Decidable (Eq Nat n Zero) := \
             fun (n : Nat) (e : False) => isFalse (Eq Nat n Zero) (fun (h : Eq Nat n Zero) => e).",
        ],
        candidate: Candidate::Definition {
            name: "spinz",
            levels: &[],
            ty: "Nat -> False -> Nat",
            body: "fix spinz => fun (n : Nat) (e : False) => \
                   match dz n e return (fun (q : Decidable (Eq Nat n Zero)) => Nat) with \
                   fun (hp : Eq Nat n Zero) => Zero | fun (hn : Not (Eq Nat n Zero)) => Succ (spinz (pred0 n) e) end",
        },
        refused_by: Some(("Termination violation", "Termination violation")),
    },
    Case {
        label: "the refutation refinement over a closed decision (control)",
        setup: &["Definition pred0 : Nat -> Nat := fun (n : Nat) => match n return (fun (q : Nat) => Nat) with | Zero => Zero | Succ k => k end."],
        candidate: Candidate::Definition {
            name: "down",
            levels: &[],
            ty: "Nat -> Nat",
            body: "fix down => fun (n : Nat) => \
                   match decEqNat n Zero return (fun (q : Decidable (Eq Nat n Zero)) => Nat) with \
                   fun (hp : Eq Nat n Zero) => Zero | fun (hn : Not (Eq Nat n Zero)) => Succ (down (pred0 n)) end",
        },
        refused_by: None,
    },
    Case {
        label: "an accumulator threaded through a convoy (control)",
        setup: &[],
        candidate: Candidate::Definition {
            name: "acc",
            levels: &[],
            ty: "Nat -> TList Nat -> Nat",
            body: "fix f => fun (n : Nat) (l : TList Nat) => \
                   (match l return (fun (q : TList Nat) => Nat -> Nat) with \
                   fun (x : Nat) => x | fun (h : Nat) (t : TList Nat) (x : Nat) => f (Succ x) t end) n",
        },
        refused_by: None,
    },
    Case {
        label: "a recursion on a convoy's threaded argument",
        setup: &[],
        candidate: Candidate::Definition {
            name: "threaded",
            levels: &[],
            ty: "TList Nat -> Nat",
            body: "fix f => fun (l : TList Nat) => \
                   (match l return (fun (q : TList Nat) => TList Nat -> Nat) with \
                   fun (m : TList Nat) => Zero | fun (h : Nat) (t : TList Nat) (m : TList Nat) => f m end) l",
        },
        refused_by: Some(("Termination violation", "Termination violation")),
    },
    Case {
        label: "a fixpoint whose type no motive states, checked at its declared type (control)",
        setup: &[],
        candidate: Candidate::Definition {
            name: "lf",
            levels: &[],
            ty: "Nat -> Nat",
            body: "fix f => fun (n : Nat) => let z : Nat := Zero in \
                   match n return (fun (q : Nat) => Nat) with z | fun (k : Nat) => f k end",
        },
        refused_by: None,
    },
    Case {
        label: "a family that may be a proposition eliminated into data",
        setup: &["Inductive Two.{u} : Sort u := t1 : Two.{u} | t2 : Two.{u}."],
        candidate: Candidate::Definition {
            name: "f",
            levels: &["u"],
            ty: "Two.{u} -> Nat",
            body: "fun (x : Two.{u}) => match x return (fun (q : Two.{u}) => Nat) with Zero | Succ Zero end",
        },
        refused_by: Some(("large elimination", "large elimination")),
    },
    Case {
        label: "a family that may be a proposition eliminated at its own level (control)",
        setup: &["Inductive Two.{u} : Sort u := t1 : Two.{u} | t2 : Two.{u}."],
        candidate: Candidate::Definition {
            name: "swap",
            levels: &["u"],
            ty: "Two.{u} -> Two.{u}",
            body: "fun (x : Two.{u}) => match x return (fun (q : Two.{u}) => Two.{u}) with t2.{u} | t1.{u} end",
        },
        refused_by: None,
    },
    Case {
        label: "an indexed family that may be a proposition eliminated into data",
        setup: &["Inductive TwoI.{u} : Nat -> Sort u := i1 : TwoI.{u} Zero | i2 : TwoI.{u} Zero."],
        candidate: Candidate::Definition {
            name: "fi",
            levels: &["u"],
            ty: "TwoI.{u} Zero -> Nat",
            body: "fun (x : TwoI.{u} Zero) => match x return (fun (n : Nat) (q : TwoI.{u} n) => Nat) with Zero | Succ Zero end",
        },
        refused_by: Some(("large elimination", "large elimination")),
    },
    Case {
        label: "a family that is never a proposition eliminated into data (control)",
        setup: &["Inductive Big.{u} : Sort (u+1) := b1 : Big.{u} | b2 : Big.{u}."],
        candidate: Candidate::Definition {
            name: "big",
            levels: &["u"],
            ty: "Big.{u} -> Nat",
            body: "fun (x : Big.{u}) => match x return (fun (q : Big.{u}) => Nat) with Zero | Succ Zero end",
        },
        refused_by: None,
    },
    Case {
        label: "a polymorphic family's auxiliary at a use site (control)",
        setup: &["Inductive Big.{u} : Sort (u+1) := b1 : Big.{u} | b2 : Big.{u}."],
        candidate: Candidate::Definition {
            name: "below_b1",
            levels: &["u"],
            ty: "Type",
            body: "Big_below.{u} (fun (x : Big.{u}) => Nat) b1.{u}",
        },
        refused_by: None,
    },
];

#[test]
fn the_core_admits_every_declaration_of_the_standard_library() {
    on_a_large_stack(|| {
        let ctx = StandardLibrary::context();
        let (_, verdicts) = replay(ctx);
        let refused: Vec<String> = verdicts
            .iter()
            .filter_map(|v| {
                v.verdict
                    .as_ref()
                    .err()
                    .map(|e| format!("{} ({}): {e}", v.name, v.kind))
            })
            .collect();
        assert!(
            refused.is_empty(),
            "the core refuses {} prelude declarations the Term kernel admitted: {refused:#?}",
            refused.len()
        );
        let declared: BTreeSet<String> = ctx
            .iter_inductives()
            .map(|(n, _)| n.to_string())
            .chain(ctx.iter_definitions().map(|(n, _, _)| n.to_string()))
            .chain(ctx.iter_universe_polys().map(|(n, _, _, _)| n.to_string()))
            .chain(ctx.iter_declarations().map(|(n, _)| n.to_string()))
            .collect();
        let replayed: BTreeSet<String> = verdicts.iter().map(|v| v.name.clone()).collect();
        assert_eq!(
            verdicts.len(),
            replayed.len(),
            "each declaration is replayed once"
        );
        assert_eq!(
            replayed, declared,
            "the replay covers exactly the declarations the context holds"
        );
        assert!(
            declared.len() > 500,
            "the prelude is the prelude: {} declarations",
            declared.len()
        );
    });
}

#[test]
fn the_core_refuses_every_paradox_the_term_kernel_refuses_by_the_same_rule() {
    on_a_large_stack(|| {
        let mut findings = Vec::new();
        for case in CASES {
            let (term, core) = verdicts(case);
            let name = case.candidate.name();
            match case.refused_by {
                Some((term_rule, core_rule)) => {
                    match &term {
                        Err(e) if e.contains(term_rule) => {}
                        other => findings.push(format!(
                            "{}: the Term kernel must refuse `{name}` by `{term_rule}`: {other:?}",
                            case.label
                        )),
                    }
                    match &core {
                        Err(e) if e.contains(core_rule) => {}
                        other => findings.push(format!(
                            "{}: the core must refuse `{name}` by `{core_rule}`: {other:?}",
                            case.label
                        )),
                    }
                }
                None => {
                    if let Err(e) = &term {
                        findings.push(format!(
                            "{}: the Term kernel must admit `{name}`: {e}",
                            case.label
                        ));
                    }
                    if let Err(e) = &core {
                        findings.push(format!("{}: the core must admit `{name}`: {e}", case.label));
                    }
                }
            }
        }
        assert!(
            findings.is_empty(),
            "the kernels disagree with the corpus:\n{}",
            findings.join("\n")
        );
        let refused = CASES.iter().filter(|c| c.refused_by.is_some()).count();
        assert!(
            refused >= 12 && CASES.len() - refused >= 6,
            "both refusals and controls are exercised"
        );
    });
}

/// Structure η: a value of a structure is its constructor applied to its projections, in both
/// kernels; the same with two fields exchanged is refused by both.
#[test]
fn a_structure_value_is_its_constructor_of_its_projections_in_both_kernels() {
    on_a_large_stack(|| {
        let mut ctx = StandardLibrary::context().clone();
        ctx.add_structure("Pt", &[], &[("x", term("Nat")), ("y", term("Nat"))]);
        ctx.add_structure(
            "Two",
            &[("A", term("Type"))],
            &[
                ("l", Term::Var("A".to_string())),
                ("r", Term::Var("A".to_string())),
            ],
        );
        for name in [
            "Pt", "Pt_mk", "Pt_x", "Pt_y", "Two", "Two_mk", "Two_l", "Two_r",
        ] {
            assert!(
                ctx.get_global(name).is_some(),
                "the setup declares `{name}`"
            );
        }
        let (mut k, replayed) = replay(&ctx);
        let refused: Vec<String> = replayed
            .iter()
            .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
            .collect();
        assert!(
            refused.is_empty(),
            "the core admits the setup: {refused:#?}"
        );
        let both =
            |k: &mut logicaffeine_kernel::dag::tc::Kernel, name: &str, ty: &str, body: &str| {
                let term_verdict = ctx
                    .clone()
                    .declare(name, term(ty), term(body))
                    .map_err(|e| e.to_string());
                let decl = Declaration::Definition {
                    name: name.to_string(),
                    params: Vec::new(),
                    ty: term(ty),
                    body: term(body),
                };
                (term_verdict, admit(k, &ctx, &[decl]))
            };
        for (name, ty, body) in [
            (
                "pt_eta",
                "forall (p : Pt), Eq Pt p (Pt_mk (Pt_x p) (Pt_y p))",
                "fun (p : Pt) => refl Pt p",
            ),
            (
                "two_eta",
                "forall (t : Two Nat), Eq (Two Nat) (Two_mk Nat (Two_l Nat t) (Two_r Nat t)) t",
                "fun (t : Two Nat) => refl (Two Nat) t",
            ),
        ] {
            let (term_verdict, core) = both(&mut k, name, ty, body);
            assert!(
                term_verdict.is_ok() && core.is_ok(),
                "`{name}` holds by η in both kernels: {term_verdict:?} / {core:?}"
            );
        }
        let (term_verdict, core) = both(
            &mut k,
            "pt_swapped",
            "forall (p : Pt), Eq Pt p (Pt_mk (Pt_y p) (Pt_x p))",
            "fun (p : Pt) => refl Pt p",
        );
        assert!(
            term_verdict.is_err() && core.is_err(),
            "exchanged fields are no η: {term_verdict:?} / {core:?}"
        );
    });
}

/// `Succ` declared again, as a definition `Nat → Bool`, is an ordinary definition to both kernels.
/// `Nat` has lost its successor, so no literal rule reads `Succ` as one: `Succ Zero` is what its
/// body makes it (`false`), never the literal `1`.
#[test]
fn a_successor_declared_again_is_an_ordinary_definition_to_both_kernels() {
    on_a_large_stack(|| {
        let mut ctx = Context::new();
        ctx.add_inductive("Nat", term("Type"));
        ctx.add_constructor("Zero", "Nat", term("Nat"));
        ctx.add_constructor("Succ", "Nat", term("Nat -> Nat"));
        ctx.add_inductive("Bool", term("Type"));
        ctx.add_constructor("false", "Bool", term("Bool"));
        ctx.add_constructor("true", "Bool", term("Bool"));
        ctx.add_mutual_inductives(&[MutualInductive {
            name: "IsFalse".to_string(),
            sort: term("Bool -> Prop"),
            num_params: 0,
            constructors: vec![("isf".to_string(), term("IsFalse false"))],
        }])
        .expect("`IsFalse` is admitted");
        ctx.declare("Succ", term("Nat -> Bool"), term("fun (n : Nat) => false"))
            .expect("the Term kernel takes `Succ` as an ordinary name");
        ctx.declare("w", term("IsFalse (Succ Zero)"), term("isf"))
            .expect("`Succ Zero` is `false` by `Succ`'s body");
        let (_, replayed) = replay(&ctx);
        let refused: Vec<String> = replayed
            .iter()
            .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
            .collect();
        assert!(
            refused.is_empty(),
            "the core admits what the Term kernel did: {refused:#?}"
        );
    })
}

/// `Nat`, `Zero`, `Succ` and `Bool`, in a context of their own.
fn peano_and_bool() -> Context {
    let mut ctx = Context::new();
    ctx.add_inductive("Nat", term("Type"));
    ctx.add_constructor("Zero", "Nat", term("Nat"));
    ctx.add_constructor("Succ", "Nat", term("Nat -> Nat"));
    ctx.add_inductive("Bool", term("Type"));
    ctx.add_constructor("false", "Bool", term("Bool"));
    ctx.add_constructor("true", "Bool", term("Bool"));
    ctx
}

/// An assumed `Succ` is no successor to either kernel: with `Nat` holding `Zero` and another
/// constructor (so no unit type, whose elements are all equal), `P 1 → P (Succ Zero)` is refused by both, and `P 0 → P Zero` admitted by both,
/// `Zero` being `Nat`'s still. A universe-polymorphic definition named `Succ` is admitted by both.
#[test]
fn a_successor_assumed_or_defined_again_reads_no_literal_in_either_kernel() {
    on_a_large_stack(|| {
        let mut ctx = peano_and_bool();
        ctx.add_constructor("Other", "Nat", term("Nat -> Nat"));
        ctx.add_declaration("Succ", term("Nat -> Nat"));
        ctx.add_declaration("P", term("Nat -> Prop"));
        assert!(
            ctx.refused_declarations().is_empty(),
            "the setup is admitted: {:?}",
            ctx.refused_declarations()
        );
        let (mut k, replayed) = replay(&ctx);
        assert!(
            replayed.iter().all(|r| r.verdict.is_ok()),
            "the core admits the setup"
        );
        for (name, ty, body, admitted) in [
            (
                "one_is_succ_zero",
                "P 1n -> P (Succ Zero)",
                "fun (x : P 1n) => x",
                false,
            ),
            (
                "zero_is_zero",
                "P 0n -> P Zero",
                "fun (x : P 0n) => x",
                true,
            ),
        ] {
            let term_verdict = ctx.clone().declare(name, term(ty), term(body));
            let core_verdict = admit(
                &mut k,
                &ctx,
                &[candidate_definition(
                    &ctx,
                    name,
                    None,
                    &term(ty),
                    &term(body),
                )],
            );
            assert_eq!(
                (term_verdict.is_ok(), core_verdict.is_ok()),
                (admitted, admitted),
                "`{name} : {ty}`: the Term kernel {term_verdict:?}, the core {core_verdict:?}"
            );
        }
        let mut ctx = peano_and_bool();
        ctx.add_constructor("Other", "Nat", term("Nat -> Nat"));
        ctx.add_declaration("P", term("Nat -> Prop"));
        let (mut k, _) = replay(&ctx);
        let (ty, body) = ("P 1n -> P (Succ Zero)", "fun (x : P 1n) => x");
        let term_verdict = ctx
            .clone()
            .declare("one_is_succ_zero", term(ty), term(body));
        let core_verdict = admit(
            &mut k,
            &ctx,
            &[candidate_definition(
                &ctx,
                "one_is_succ_zero",
                None,
                &term(ty),
                &term(body),
            )],
        );
        assert!(
            term_verdict.is_ok() && core_verdict.is_ok(),
            "`1` reads as `Succ 0` while `Succ` is `Nat`'s constructor, a third one beside it: the \
             Term kernel {term_verdict:?}, the core {core_verdict:?}"
        );
        let mut ctx = peano_and_bool();
        ctx.declare_universe_poly(
            "Succ",
            vec!["u".to_string()],
            term("Nat -> Bool"),
            term("fun (n : Nat) => false"),
        )
        .expect("the Term kernel takes `Succ.{u}` as an ordinary definition");
        let (_, replayed) = replay(&ctx);
        let refused: Vec<String> = replayed
            .iter()
            .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
            .collect();
        assert!(refused.is_empty(), "and so does the core: {refused:#?}");
    })
}

/// A declaration whose check needs structure η depends on the structure's projections, though it
/// names none: `Pt_a_rec`'s body builds `Pt_mk` from matches on `p`, and its type is stated at `p`.
/// The core admits the replayed context whole, whatever order the names sort in.
#[test]
fn a_declaration_that_builds_a_structure_value_waits_for_the_projections() {
    on_a_large_stack(|| {
        let mut ctx = StandardLibrary::context().clone();
        ctx.add_structure("Pt", &[], &[("x", term("Nat")), ("y", term("Nat"))]);
        ctx.declare(
            "Pt_a_rec",
            term("forall (motive : Pt -> Type) (f : forall (a : Nat) (b : Nat), motive (Pt_mk a b)) (p : Pt), motive p"),
            term(
                "fun (motive : Pt -> Type) (f : forall (a : Nat) (b : Nat), motive (Pt_mk a b)) (p : Pt) => \
                 f (match p return (fun (q : Pt) => Nat) with fun (a : Nat) (b : Nat) => a end) \
                   (match p return (fun (q : Pt) => Nat) with fun (a : Nat) (b : Nat) => b end)",
            ),
        )
        .expect("the Term kernel admits the recursor by structure η");
        let (_, replayed) = replay(&ctx);
        let refused: Vec<String> = replayed
            .iter()
            .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
            .collect();
        assert!(
            refused.is_empty(),
            "the core admits the replayed context: {refused:#?}"
        );
    })
}

/// A Lean import's whole context replays into the core, declaration by declaration: every one the
/// `Term` kernel admitted on import (`lean_scale/{stem}`) is admitted by the core, within a deadline.
/// The core computes Lean content's primitives natively, so a replay that runs past it is reading
/// a primitive by unfolding, and fails here rather than running on.
fn replays_whole(stem: &'static str) {
    const DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);
    let (sent, received) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || {
            let export = std::fs::read_to_string(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("tests/fixtures/lean_scale/{stem}.ndjson")),
            )
            .expect("the pinned export");
            let (ctx, _) = logicaffeine_kernel::interface::lean::import(&export).expect("imports");
            let started = std::time::Instant::now();
            let (_, replayed) = replay(&ctx);
            let refused: Vec<String> = replayed
                .iter()
                .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
                .collect();
            let _ = sent.send((replayed.len(), refused, started.elapsed()));
        })
        .expect("spawns");
    let (count, refused, took) = match received.recv_timeout(DEADLINE * 4) {
        Ok(finished) => finished,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => panic!(
            "{stem}: the import and replay did not finish in {:?}",
            DEADLINE * 4
        ),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            panic!("{stem}: the import and replay panicked (its message is above)")
        }
    };
    assert!(
        refused.is_empty(),
        "{stem}: the core admits all {count} declarations: {refused:#?}"
    );
    assert!(
        took < DEADLINE,
        "{stem}: the replay took {took:?}, past {DEADLINE:?}"
    );
}

/// The Omega originals: structure recursors that need structure η before any projection is named,
/// and `Nat.brecOn` recursions whose reducts' `match` types are read.
#[test]
fn the_core_admits_a_lean_imports_context_whole() {
    replays_whole("LG_Omega")
}

/// `Nat.beq` and `Nat.ble` at large literals: Lean content's primitives, named as the importer names
/// them in a LOGOS context, checked by their equations and computed natively.
#[test]
fn the_core_admits_the_nat_comparison_originals_whole() {
    replays_whole("LG_NatCompare")
}

/// String literals and `Char.ofNat`, whose proofs decide comparisons at large literals.
#[test]
fn the_core_admits_the_string_literal_originals_whole() {
    replays_whole("LG_StringLit")
}

/// Two programs shaped like Lean's `Nat.modCore` (`badMod`, which answers `x + 1`, and `axMod`, which
/// subtracts through an axiom): the core admits both as ordinary definitions and computes neither as
/// `%`, as the `Term` kernel does.
#[test]
fn the_core_admits_the_division_impostor_whole() {
    replays_whole("LG_DivImpostor")
}

/// Lean's own `Nat.div` and `Nat.mod` at 2^64, 2^128 and by zero: the well-founded unary programs
/// the `Term` kernel's evaluator certifies by their recurrence, computed natively by the core.
#[test]
fn the_core_admits_the_division_originals_whole() {
    replays_whole("LG_Div")
}

/// `Nat.Linear`'s theorems, whose conversions Lean's kernel decides by lazy δ-reduction over
/// polynomials it never normalizes.
#[test]
fn the_core_admits_the_linear_arithmetic_originals_whole() {
    replays_whole("LG_Linear")
}

/// `Nat.modCore_eq_mod` and `Nat.gcd_zero_right`, decided by lazy δ-reduction through the
/// well-founded modulus.
#[test]
fn the_core_admits_the_modulus_originals_whole() {
    replays_whole("LG_ModCore")
}

/// Projections whose discriminant's type reaches a structure only by unfolding its head, through a
/// definition or a recursor.
#[test]
fn the_core_admits_the_projection_originals_whole() {
    replays_whole("LG_Proj")
}
