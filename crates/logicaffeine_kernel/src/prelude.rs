//! Standard Library for the Kernel.
//!
//! Defines fundamental types and logical connectives:
//! - Entity: domain of individuals (for FOL)
//! - Nat: natural numbers
//! - True, False: propositional constants
//! - Eq: propositional equality
//! - And, Or: logical connectives

use crate::context::Context;
use crate::term::{Literal, Term, Universe};

/// Standard library definitions.
pub struct StandardLibrary;

impl StandardLibrary {
    /// Register all standard library definitions in the context.
    pub fn register(ctx: &mut Context) {
        Self::register_entity(ctx);
        Self::register_nat(ctx);
        Self::register_monolist(ctx);
        Self::register_bool(ctx);
        Self::register_tlist(ctx);
        Self::register_true(ctx);
        Self::register_false(ctx);
        Self::register_not(ctx);
        Self::register_classical(ctx);
        Self::register_eq(ctx);
        Self::register_and(ctx);
        Self::register_or(ctx);
        Self::register_ex(ctx);
        Self::register_decidable(ctx);
        Self::register_of_decide(ctx);
        Self::register_dec_eq_bool(ctx);
        Self::register_dec_eq_nat(ctx);
        Self::register_native_decide(ctx);
        Self::register_quot(ctx);
        Self::register_acc(ctx);
        Self::register_primitives(ctx);
        Self::register_int_ring_axioms(ctx);
        Self::register_int_order_axioms(ctx);
        Self::register_reflection(ctx);
        Self::register_hardware(ctx);
    }

    /// Classical logic: double-negation elimination `dne : Π(P:Prop). ¬¬P → P`,
    /// i.e. `Π(P:Prop). ((P → False) → False) → P`. This makes the logic CLASSICAL —
    /// exactly as Lean/Coq's Mathlib are classical via `Classical.em`/`choice` — which
    /// ordinary mathematics (and proof by contradiction: Euclid I.6/I.7/I.27 …)
    /// requires. It is an EXPLICIT, type-checked axiom in the trusted base, recorded
    /// in the certificate's axiom-set version; the de Bruijn criterion is preserved
    /// (every classical proof still elaborates to a kernel `Term` re-checked here).
    fn register_classical(ctx: &mut Context) {
        let false_t = || Term::Global("False".to_string());
        let p = || Term::Var("P".to_string());
        // ¬P = P → False
        let not_p = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(p()),
            body_type: Box::new(false_t()),
        };
        // ¬¬P = (P → False) → False
        let not_not_p = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(not_p),
            body_type: Box::new(false_t()),
        };
        // dne : Π(P:Prop). ¬¬P → P
        let dne_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(not_not_p),
                body_type: Box::new(p()),
            }),
        };
        ctx.add_declaration("dne", dne_type);
    }

    /// The commutative-ring axioms for `Int` — the ENTIRE trusted arithmetic base.
    ///
    /// `Int` is an opaque type (no constructors, `register_primitives`), so these
    /// standard ring laws are not derivable in-kernel; they are the axiomatization
    /// of `(Int, add, mul, 0, 1)` as a commutative ring — the textbook foundation,
    /// not a soundness shortcut. The proof-producing arithmetic procedure builds
    /// every arithmetic proof from these (closed/literal goals need none — they
    /// hold by `add`/`mul` computation + `refl`). They are the *only* arithmetic
    /// things trusted beyond the kernel; `phase98`'s TCB-inventory test locks the
    /// exact set so it can never silently grow. Replaceable later (without any
    /// downstream churn) by defining `Int` from `Nat` and proving them.
    fn register_int_ring_axioms(ctx: &mut Context) {
        let int = || Term::Global("Int".to_string());
        let lit = |n: i64| Term::Lit(Literal::Int(n));
        // (op a b)
        let bin = |op: &str, a: Term, b: Term| {
            Term::App(
                Box::new(Term::App(Box::new(Term::Global(op.to_string())), Box::new(a))),
                Box::new(b),
            )
        };
        // Eq Int l r
        let eq_int = |l: Term, r: Term| {
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::Global("Eq".to_string())),
                        Box::new(Term::Global("Int".to_string())),
                    )),
                    Box::new(l),
                )),
                Box::new(r),
            )
        };
        // Π over the given Int-typed parameter names, body = eq.
        let forall_ints = |names: &[&str], body: Term| {
            names.iter().rev().fold(body, |acc, name| Term::Pi {
                param: name.to_string(),
                param_type: Box::new(int()),
                body_type: Box::new(acc),
            })
        };
        let v = |s: &str| Term::Var(s.to_string());

        // add_comm : Π a b. Eq Int (add a b) (add b a)
        ctx.add_declaration(
            "add_comm",
            forall_ints(
                &["a", "b"],
                eq_int(bin("add", v("a"), v("b")), bin("add", v("b"), v("a"))),
            ),
        );
        // add_assoc : Π a b c. Eq Int (add (add a b) c) (add a (add b c))
        ctx.add_declaration(
            "add_assoc",
            forall_ints(
                &["a", "b", "c"],
                eq_int(
                    bin("add", bin("add", v("a"), v("b")), v("c")),
                    bin("add", v("a"), bin("add", v("b"), v("c"))),
                ),
            ),
        );
        // add_zero : Π a. Eq Int (add a 0) a
        ctx.add_declaration(
            "add_zero",
            forall_ints(&["a"], eq_int(bin("add", v("a"), lit(0)), v("a"))),
        );
        // mul_comm : Π a b. Eq Int (mul a b) (mul b a)
        ctx.add_declaration(
            "mul_comm",
            forall_ints(
                &["a", "b"],
                eq_int(bin("mul", v("a"), v("b")), bin("mul", v("b"), v("a"))),
            ),
        );
        // mul_assoc : Π a b c. Eq Int (mul (mul a b) c) (mul a (mul b c))
        ctx.add_declaration(
            "mul_assoc",
            forall_ints(
                &["a", "b", "c"],
                eq_int(
                    bin("mul", bin("mul", v("a"), v("b")), v("c")),
                    bin("mul", v("a"), bin("mul", v("b"), v("c"))),
                ),
            ),
        );
        // mul_one : Π a. Eq Int (mul a 1) a
        ctx.add_declaration(
            "mul_one",
            forall_ints(&["a"], eq_int(bin("mul", v("a"), lit(1)), v("a"))),
        );
        // mul_zero : Π a. Eq Int (mul a 0) 0. Not derivable from the others without
        // an additive-inverse axiom; needed so the normalizer can drop a monomial
        // whose coefficient cancels to zero (e.g. in a Farkas combination).
        ctx.add_declaration(
            "mul_zero",
            forall_ints(&["a"], eq_int(bin("mul", v("a"), lit(0)), lit(0))),
        );
        // mul_distrib_add : Π a b c. Eq Int (mul a (add b c)) (add (mul a b) (mul a c))
        ctx.add_declaration(
            "mul_distrib_add",
            forall_ints(
                &["a", "b", "c"],
                eq_int(
                    bin("mul", v("a"), bin("add", v("b"), v("c"))),
                    bin("add", bin("mul", v("a"), v("b")), bin("mul", v("a"), v("c"))),
                ),
            ),
        );
    }

    /// The standard order axioms for `Int`, completing the ordered-ring trusted
    /// base. `Int` is opaque, so just as the ring laws are axioms here (Coq `Int63`
    /// / Lean primitives), the order laws are too: reflexivity, transitivity,
    /// monotone addition, and scaling by a non-negative factor — the primitives a
    /// Farkas linear-arithmetic certificate is reconstructed from. `m ≤ n` is the
    /// shallow `Eq Bool (le m n) true` (decidable by computation on literals, see
    /// `reduction.rs`); these axioms extend it to the symbolic case. Replaceable
    /// later (no downstream churn) by defining `≤` from `Nat` and proving them.
    /// Locked by `phase98`'s TCB inventory.
    fn register_int_order_axioms(ctx: &mut Context) {
        let int = || Term::Global("Int".to_string());
        let v = |s: &str| Term::Var(s.to_string());
        let g = |s: &str| Term::Global(s.to_string());
        let lit = |n: i64| Term::Lit(Literal::Int(n));
        let bin = |op: &str, a: Term, b: Term| {
            Term::App(
                Box::new(Term::App(Box::new(Term::Global(op.to_string())), Box::new(a))),
                Box::new(b),
            )
        };
        // le_prop a b = Eq Bool (le a b) true  — the shallow `a ≤ b`.
        let le_prop = |a: Term, b: Term| {
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(Box::new(g("Eq")), Box::new(g("Bool")))),
                    Box::new(bin("le", a, b)),
                )),
                Box::new(g("true")),
            )
        };
        let arrow = |dom: Term, cod: Term| Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(dom),
            body_type: Box::new(cod),
        };
        let forall_ints = |names: &[&str], body: Term| {
            names.iter().rev().fold(body, |acc, name| Term::Pi {
                param: name.to_string(),
                param_type: Box::new(int()),
                body_type: Box::new(acc),
            })
        };

        // le_refl : Π a. a ≤ a
        ctx.add_declaration("le_refl", forall_ints(&["a"], le_prop(v("a"), v("a"))));
        // le_trans : Π a b c. a ≤ b → b ≤ c → a ≤ c
        ctx.add_declaration(
            "le_trans",
            forall_ints(
                &["a", "b", "c"],
                arrow(
                    le_prop(v("a"), v("b")),
                    arrow(le_prop(v("b"), v("c")), le_prop(v("a"), v("c"))),
                ),
            ),
        );
        // le_add_mono : Π a b c d. a ≤ b → c ≤ d → (a + c) ≤ (b + d)
        ctx.add_declaration(
            "le_add_mono",
            forall_ints(
                &["a", "b", "c", "d"],
                arrow(
                    le_prop(v("a"), v("b")),
                    arrow(
                        le_prop(v("c"), v("d")),
                        le_prop(bin("add", v("a"), v("c")), bin("add", v("b"), v("d"))),
                    ),
                ),
            ),
        );
        // le_mul_nonneg : Π k a b. 0 ≤ k → a ≤ b → (k * a) ≤ (k * b)
        ctx.add_declaration(
            "le_mul_nonneg",
            forall_ints(
                &["k", "a", "b"],
                arrow(
                    le_prop(lit(0), v("k")),
                    arrow(
                        le_prop(v("a"), v("b")),
                        le_prop(bin("mul", v("k"), v("a")), bin("mul", v("k"), v("b"))),
                    ),
                ),
            ),
        );
        // le_sub : Π a b. a ≤ b → 0 ≤ b + (-1)·a   (the Farkas "move to one side").
        // Written with add/mul (not sub) so the ring oracle, whose axioms are over
        // add/mul, can prove the combined-term equalities during reconstruction.
        ctx.add_declaration(
            "le_sub",
            forall_ints(
                &["a", "b"],
                arrow(
                    le_prop(v("a"), v("b")),
                    le_prop(lit(0), bin("add", v("b"), bin("mul", lit(-1), v("a")))),
                ),
            ),
        );

        // lt_prop a b = Eq Bool (lt a b) true  — the shallow `a < b`.
        let lt_prop = |a: Term, b: Term| {
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::App(Box::new(g("Eq")), Box::new(g("Bool")))),
                    Box::new(bin("lt", a, b)),
                )),
                Box::new(g("true")),
            )
        };
        // lt_succ_le : Π a b. a < b → (a + 1) ≤ b.  Integer DISCRETENESS — the fact
        // rational Fourier-Motzkin lacks: over ℤ a strict `<` is a `≤` shifted by one,
        // so nothing lives strictly between `a` and `a+1`. This single axiom is what
        // lets `omega` refute strict systems the rational solver reports satisfiable
        // (`x < y ∧ y < x+1`), and it is discharged by ordinary Farkas afterward.
        ctx.add_declaration(
            "lt_succ_le",
            forall_ints(
                &["a", "b"],
                arrow(lt_prop(v("a"), v("b")), le_prop(bin("add", v("a"), lit(1)), v("b"))),
            ),
        );
        // lt_add1_le : Π a b. a < (b + 1) → a ≤ b.  The upper-side companion of
        // `lt_succ_le` (`a < b+1 ⟺ a ≤ b` over ℤ): when the strict bound already has
        // the shape `b + 1`, this yields the CANCELLED `a ≤ b` directly instead of the
        // constant-laden `a + 1 ≤ b + 1`, keeping the reconstructed Farkas terms small.
        ctx.add_declaration(
            "lt_add1_le",
            forall_ints(
                &["a", "b"],
                arrow(lt_prop(v("a"), bin("add", v("b"), lit(1))), le_prop(v("a"), v("b"))),
            ),
        );
        // le_total : Π a b. (a ≤ b) ∨ (b ≤ a).  Linear order totality — the case-split
        // seam for disequality (`a ≠ b` ⟹ split `a < b ∨ b < a`) and for reducing a
        // positive `≤` goal to a refutation of its negation.
        let or = |p: Term, q: Term| {
            Term::App(
                Box::new(Term::App(Box::new(g("Or")), Box::new(p))),
                Box::new(q),
            )
        };
        ctx.add_declaration(
            "le_total",
            forall_ints(
                &["a", "b"],
                or(le_prop(v("a"), v("b")), le_prop(v("b"), v("a"))),
            ),
        );
    }

    /// Primitive types and operations.
    ///
    /// Int : Type 0 (64-bit signed integer)
    /// Float : Type 0 (64-bit floating point)
    /// Text : Type 0 (UTF-8 string)
    /// Duration : Type 0 (nanoseconds, i64 - physical time)
    /// Date : Type 0 (days since epoch, i32 - calendar date)
    /// Moment : Type 0 (nanoseconds since epoch, i64 - instant in UTC)
    ///
    /// add, sub, mul, div, mod : Int -> Int -> Int
    fn register_primitives(ctx: &mut Context) {
        // Opaque types (no constructors, cannot be pattern-matched)
        ctx.add_inductive("Int", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Float", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Text", Term::Sort(Universe::Type(0)));

        // Temporal types (opaque, no constructors)
        ctx.add_inductive("Duration", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Date", Term::Sort(Universe::Type(0)));
        ctx.add_inductive("Moment", Term::Sort(Universe::Type(0)));

        let int = Term::Global("Int".to_string());

        // Binary Int -> Int -> Int type
        let bin_int_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(int.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(int.clone()),
            }),
        };

        // Register arithmetic builtins as declarations (axioms with computational behavior)
        ctx.add_declaration("add", bin_int_type.clone());
        ctx.add_declaration("sub", bin_int_type.clone());
        ctx.add_declaration("mul", bin_int_type.clone());
        ctx.add_declaration("div", bin_int_type.clone());
        ctx.add_declaration("mod", bin_int_type);

        // Comparison builtins: Int -> Int -> Bool, decided by computation on
        // literals (see `reduction.rs`). `Eq Bool (le m n) true` is the shallow
        // encoding of `m ≤ n`, provable by `refl` exactly when it holds.
        let bin_int_bool_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(int.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(Term::Global("Bool".to_string())),
            }),
        };
        ctx.add_declaration("le", bin_int_bool_type.clone());
        ctx.add_declaration("lt", bin_int_bool_type.clone());
        ctx.add_declaration("ge", bin_int_bool_type.clone());
        ctx.add_declaration("gt", bin_int_bool_type);

        // Temporal operations
        let duration = Term::Global("Duration".to_string());
        let date = Term::Global("Date".to_string());
        let moment = Term::Global("Moment".to_string());
        let bool_type = Term::Global("Bool".to_string());

        // Duration -> Duration -> Duration
        let bin_duration_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(duration.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(duration.clone()),
                body_type: Box::new(duration.clone()),
            }),
        };

        // Duration -> Int -> Duration
        let duration_int_duration_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(duration.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(duration.clone()),
            }),
        };

        // Duration arithmetic: add, sub, mul, div
        ctx.add_declaration("add_duration", bin_duration_type.clone());
        ctx.add_declaration("sub_duration", bin_duration_type.clone());
        ctx.add_declaration("mul_duration", duration_int_duration_type.clone());
        ctx.add_declaration("div_duration", duration_int_duration_type);

        // Date -> Int -> Date (add days offset)
        let date_int_date_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(date.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(date.clone()),
            }),
        };
        ctx.add_declaration("date_add_days", date_int_date_type);

        // Date -> Date -> Int (difference in days)
        let date_date_int_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(date.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(date.clone()),
                body_type: Box::new(int.clone()),
            }),
        };
        ctx.add_declaration("date_sub_date", date_date_int_type);

        // Moment -> Duration -> Moment
        let moment_duration_moment_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(moment.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(duration.clone()),
                body_type: Box::new(moment.clone()),
            }),
        };
        ctx.add_declaration("moment_add_duration", moment_duration_moment_type);

        // Moment -> Moment -> Duration
        let moment_moment_duration_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(moment.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(moment.clone()),
                body_type: Box::new(duration.clone()),
            }),
        };
        ctx.add_declaration("moment_sub_moment", moment_moment_duration_type);

        // Comparison operations: X -> X -> Bool
        let date_date_bool_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(date.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(date),
                body_type: Box::new(bool_type.clone()),
            }),
        };
        ctx.add_declaration("date_lt", date_date_bool_type);

        let moment_moment_bool_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(moment.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(moment),
                body_type: Box::new(bool_type.clone()),
            }),
        };
        ctx.add_declaration("moment_lt", moment_moment_bool_type);

        let duration_duration_bool_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(duration.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(duration),
                body_type: Box::new(bool_type),
            }),
        };
        ctx.add_declaration("duration_lt", duration_duration_bool_type);
    }

    /// Entity : Type 0
    ///
    /// The domain of individuals for first-order logic.
    /// Proper names like "Socrates" are declared as Entity.
    /// Predicates like "Man" are declared as Entity → Prop.
    fn register_entity(ctx: &mut Context) {
        ctx.add_inductive("Entity", Term::Sort(Universe::Type(0)));
    }

    /// Nat : Type 0
    /// Zero : Nat
    /// Succ : Nat → Nat
    fn register_nat(ctx: &mut Context) {
        let nat = Term::Global("Nat".to_string());

        // Nat : Type 0
        ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));

        // Zero : Nat
        ctx.add_constructor("Zero", "Nat", nat.clone());

        // Succ : Nat → Nat
        ctx.add_constructor(
            "Succ",
            "Nat",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(nat.clone()),
                body_type: Box::new(nat),
            },
        );
    }

    /// Monomorphic `EList` (a list of `Entity`) — the concrete inductive that
    /// structural induction runs over. The kernel's parametric `TList A` carries a
    /// type argument, so its constructors are `Π(A:Type)…`; the generic
    /// `InductionScheme` eliminator builds a `match` over a *bare* `Global(ind_type)`,
    /// which needs a non-parametric type. `EList = μX. ENil | ECons Entity X` is that
    /// target: it gives the `induction` tactic an `ENil`/`ECons` recursor that
    /// certifies — `fix rec. λl:EList. match l { ENil => …, ECons h t => … }`. The
    /// `E`-prefixed names deliberately avoid the user-space `List`/`Nil`/`Cons` a REPL
    /// program defines (typically a parametric `List A`), so registering this in the
    /// prelude never shadows a user inductive.
    ///
    /// `EList : Type 0`
    /// `ENil  : EList`
    /// `ECons : Entity → EList → EList`
    fn register_monolist(ctx: &mut Context) {
        let list = Term::Global("EList".to_string());
        let entity = Term::Global("Entity".to_string());

        // EList : Type 0
        ctx.add_inductive("EList", Term::Sort(Universe::Type(0)));

        // ENil : EList
        ctx.add_constructor("ENil", "EList", list.clone());

        // ECons : Entity → EList → EList
        ctx.add_constructor(
            "ECons",
            "EList",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(entity),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(list.clone()),
                    body_type: Box::new(list),
                }),
            },
        );
    }

    /// Bool : Type 0
    /// true : Bool
    /// false : Bool
    fn register_bool(ctx: &mut Context) {
        let bool_type = Term::Global("Bool".to_string());

        // Bool : Type 0
        ctx.add_inductive("Bool", Term::Sort(Universe::Type(0)));

        // true : Bool
        ctx.add_constructor("true", "Bool", bool_type.clone());

        // false : Bool
        ctx.add_constructor("false", "Bool", bool_type);
    }

    /// TList : Type 0 -> Type 0
    /// TNil : Π(A : Type 0). TList A
    /// TCons : Π(A : Type 0). A -> TList A -> TList A
    fn register_tlist(ctx: &mut Context) {
        let type0 = Term::Sort(Universe::Type(0));
        let a = Term::Var("A".to_string());

        // TList : Type 0 -> Type 0
        let tlist_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(type0.clone()),
            body_type: Box::new(type0.clone()),
        };
        ctx.add_inductive("TList", tlist_type);

        // TList A
        let tlist_a = Term::App(
            Box::new(Term::Global("TList".to_string())),
            Box::new(a.clone()),
        );

        // TNil : Π(A : Type 0). TList A
        let tnil_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(type0.clone()),
            body_type: Box::new(tlist_a.clone()),
        };
        ctx.add_constructor("TNil", "TList", tnil_type);

        // TCons : Π(A : Type 0). A -> TList A -> TList A
        let tcons_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(type0),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(a.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(tlist_a.clone()),
                    body_type: Box::new(tlist_a),
                }),
            }),
        };
        ctx.add_constructor("TCons", "TList", tcons_type);

        // Convenience aliases for tactic lists (Syntax -> Derivation)
        Self::register_tactic_list_helpers(ctx);
    }

    /// TTactics : Type 0 = TList (Syntax -> Derivation)
    /// TacNil : TTactics
    /// TacCons : (Syntax -> Derivation) -> TTactics -> TTactics
    ///
    /// Convenience wrappers to avoid explicit type arguments for tactic lists.
    fn register_tactic_list_helpers(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // TTactics = TList (Syntax -> Derivation)
        let ttactics = Term::App(
            Box::new(Term::Global("TList".to_string())),
            Box::new(tactic_type.clone()),
        );

        // TTactics : Type 0
        ctx.add_definition("TTactics".to_string(), Term::Sort(Universe::Type(0)), ttactics.clone());

        // TacNil : TTactics = TNil (Syntax -> Derivation)
        let tac_nil_body = Term::App(
            Box::new(Term::Global("TNil".to_string())),
            Box::new(tactic_type.clone()),
        );
        ctx.add_definition("TacNil".to_string(), ttactics.clone(), tac_nil_body);

        // TacCons : (Syntax -> Derivation) -> TTactics -> TTactics
        let tac_cons_type = Term::Pi {
            param: "t".to_string(),
            param_type: Box::new(tactic_type.clone()),
            body_type: Box::new(Term::Pi {
                param: "ts".to_string(),
                param_type: Box::new(ttactics.clone()),
                body_type: Box::new(ttactics.clone()),
            }),
        };

        // TacCons t ts = TCons (Syntax -> Derivation) t ts
        let tac_cons_body = Term::Lambda {
            param: "t".to_string(),
            param_type: Box::new(tactic_type.clone()),
            body: Box::new(Term::Lambda {
                param: "ts".to_string(),
                param_type: Box::new(ttactics.clone()),
                body: Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("TCons".to_string())),
                            Box::new(tactic_type),
                        )),
                        Box::new(Term::Var("t".to_string())),
                    )),
                    Box::new(Term::Var("ts".to_string())),
                )),
            }),
        };
        ctx.add_definition("TacCons".to_string(), tac_cons_type, tac_cons_body);
    }

    /// True : Prop
    /// I : True
    fn register_true(ctx: &mut Context) {
        ctx.add_inductive("True", Term::Sort(Universe::Prop));
        ctx.add_constructor("I", "True", Term::Global("True".to_string()));
    }

    /// False : Prop
    /// (no constructors)
    fn register_false(ctx: &mut Context) {
        ctx.add_inductive("False", Term::Sort(Universe::Prop));
    }

    /// Not : Prop -> Prop
    /// Not P := P -> False
    fn register_not(ctx: &mut Context) {
        // Type: Π(P : Prop). Prop
        let not_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Sort(Universe::Prop)),
        };

        // Body: λ(P : Prop). Π(_ : P). False
        let not_body = Term::Lambda {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(Term::Var("P".to_string())),
                body_type: Box::new(Term::Global("False".to_string())),
            }),
        };

        ctx.add_definition("Not".to_string(), not_type, not_body);
    }

    /// `Decidable (p:Prop) : Type` — the data of a decision procedure for `p`, with
    /// `isTrue : p → Decidable p` and `isFalse : ¬p → Decidable p`. Type-valued (so it may
    /// be eliminated into `Type` to compute a `Bool`), auto-derives its recursor, and
    /// defines `decide : Π(p). Decidable p → Bool` (isTrue ↝ true, isFalse ↝ false).
    fn register_decidable(ctx: &mut Context) {
        let prop = || Term::Sort(Universe::Prop);
        let type0 = || Term::Sort(Universe::Type(0));
        let g = |s: &str| Term::Global(s.to_string());
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let lm = |p: &str, t: Term, b: Term| Term::Lambda {
            param: p.to_string(),
            param_type: Box::new(t),
            body: Box::new(b),
        };
        let pi = |p: &str, t: Term, b: Term| Term::Pi {
            param: p.to_string(),
            param_type: Box::new(t),
            body_type: Box::new(b),
        };
        let dec = |p: Term| ap(g("Decidable"), p);
        let not = |p: Term| ap(g("Not"), p);

        // Decidable : Π(p:Prop). Type 0   (one uniform parameter, Type-valued)
        ctx.add_indexed_inductive("Decidable", pi("p", prop(), type0()), 1);
        // isTrue  : Π(p:Prop). p → Decidable p
        ctx.add_constructor("isTrue", "Decidable", pi("p", prop(), pi("_", v("p"), dec(v("p")))));
        // isFalse : Π(p:Prop). Not p → Decidable p
        ctx.add_constructor(
            "isFalse",
            "Decidable",
            pi("p", prop(), pi("_", not(v("p")), dec(v("p")))),
        );

        // Decidable_rec — auto-derived dependent eliminator, kernel-checked (not an axiom).
        let (rec_ty, rec_body) = crate::recursor::derive_recursor(ctx, "Decidable")
            .expect("Decidable's eliminator must derive");
        ctx.add_definition("Decidable_rec".to_string(), rec_ty, rec_body);

        // decide : Π(p:Prop). Decidable p → Bool
        //   := λp inst. Decidable_rec p (λ_. Bool) (λh. true) (λh. false) inst
        let decide_ty = pi("p", prop(), pi("_", dec(v("p")), g("Bool")));
        let decide_body = lm(
            "p",
            prop(),
            lm(
                "inst",
                dec(v("p")),
                ap(
                    ap(
                        ap(
                            ap(ap(g("Decidable_rec"), v("p")), lm("_", dec(v("p")), g("Bool"))),
                            lm("_", v("p"), g("true")),
                        ),
                        lm("_", not(v("p")), g("false")),
                    ),
                    v("inst"),
                ),
            ),
        );
        ctx.add_definition("decide".to_string(), decide_ty, decide_body);
    }

    /// `of_decide_eq_true : Π(p:Prop). Π(inst:Decidable p). Eq Bool (decide p inst) true → p`
    /// — the bridge that turns a computed `decide` into a proof, PROVEN from `Decidable_rec`
    /// and Bool no-confusion (via `Eq_rec_dep`), NOT axiomatized. This is what makes the
    /// `decide` tactic sound with zero additions to the trusted base.
    fn register_of_decide(ctx: &mut Context) {
        let prop = || Term::Sort(Universe::Prop);
        let g = |s: &str| Term::Global(s.to_string());
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let lm = |p: &str, t: Term, b: Term| Term::Lambda {
            param: p.to_string(),
            param_type: Box::new(t),
            body: Box::new(b),
        };
        let pi = |p: &str, t: Term, b: Term| Term::Pi {
            param: p.to_string(),
            param_type: Box::new(t),
            body_type: Box::new(b),
        };
        let dec = |p: Term| ap(g("Decidable"), p);
        let not = |p: Term| ap(g("Not"), p);
        // Eq Bool x y
        let eqb = |x: Term, y: Term| ap(ap(ap(g("Eq"), g("Bool")), x), y);
        // decide p inst
        let decide = |p: Term, inst: Term| ap(ap(g("decide"), p), inst);
        let is_true = |p: Term, h: Term| ap(ap(g("isTrue"), p), h);
        let is_false = |p: Term, h: Term| ap(ap(g("isFalse"), p), h);

        // of_decide_eq_true : Π(p:Prop). Π(inst:Decidable p). Eq Bool (decide p inst) true → p
        let ty = pi(
            "p",
            prop(),
            pi(
                "inst",
                dec(v("p")),
                pi("_", eqb(decide(v("p"), v("inst")), g("true")), v("p")),
            ),
        );

        // Motive for the recursion on `inst`: λinst. Eq Bool (decide p inst) true → p.
        let motive = lm(
            "inst",
            dec(v("p")),
            pi("_", eqb(decide(v("p"), v("inst")), g("true")), v("p")),
        );
        // isTrue branch: `decide p (isTrue p hp) ≡ true`, so just return the witness `hp`.
        let f_istrue = lm(
            "hp",
            v("p"),
            lm(
                "_",
                eqb(decide(v("p"), is_true(v("p"), v("hp"))), g("true")),
                v("hp"),
            ),
        );
        // isFalse branch: `decide p (isFalse p hnp) ≡ false`, so the hypothesis is
        // `false = true`; Bool no-confusion turns it into `p`. Transport `discr` along the
        // equality, where `discr b := match b with true => p | false => True`, taking the
        // inhabitant `I : True` at `false` to a proof of `p` at `true`.
        let discr = Term::Match {
            discriminant: Box::new(v("b")),
            motive: Box::new(lm("_", g("Bool"), prop())),
            cases: vec![v("p"), g("True")], // Bool constructor order: [true, false]
        };
        let bool_motive = lm("b", g("Bool"), lm("_", eqb(g("false"), v("b")), discr));
        // Eq_rec_dep Bool false bool_motive I true h  :  p
        let noconf = ap(
            ap(
                ap(ap(ap(ap(g("Eq_rec_dep"), g("Bool")), g("false")), bool_motive), g("I")),
                g("true"),
            ),
            v("h"),
        );
        let f_isfalse = lm(
            "hnp",
            not(v("p")),
            lm(
                "h",
                eqb(decide(v("p"), is_false(v("p"), v("hnp"))), g("true")),
                noconf,
            ),
        );

        // λp inst h. Decidable_rec p motive f_istrue f_isfalse inst h
        let rec_app = ap(
            ap(
                ap(ap(ap(ap(g("Decidable_rec"), v("p")), motive), f_istrue), f_isfalse),
                v("inst"),
            ),
            v("h"),
        );
        let body = lm(
            "p",
            prop(),
            lm(
                "inst",
                dec(v("p")),
                lm("h", eqb(decide(v("p"), v("inst")), g("true")), rec_app),
            ),
        );
        ctx.add_definition("of_decide_eq_true".to_string(), ty, body);
    }

    /// Decidable equality of `Bool` (`decEqBool : Π(a b:Bool). Decidable (Eq Bool a b)`),
    /// with the two Bool no-confusion lemmas it needs — all derived. This is a concrete
    /// `Decidable` INSTANCE, so `decide` can actually discharge `Eq Bool _ _` goals.
    fn register_dec_eq_bool(ctx: &mut Context) {
        let prop = || Term::Sort(Universe::Prop);
        let g = |s: &str| Term::Global(s.to_string());
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let lm = |p: &str, t: Term, b: Term| Term::Lambda {
            param: p.to_string(),
            param_type: Box::new(t),
            body: Box::new(b),
        };
        let eqb = |x: Term, y: Term| ap(ap(ap(g("Eq"), g("Bool")), x), y);
        let dec = |p: Term| ap(g("Decidable"), p);
        let is_true = |p: Term, h: Term| ap(ap(g("isTrue"), p), h);
        let is_false = |p: Term, h: Term| ap(ap(g("isFalse"), p), h);
        let refl = |x: Term| ap(ap(g("refl"), g("Bool")), x);
        // `match b return (λ_:Bool. Prop) with { <true-case>, <false-case> }`.
        let bool_match = |t_case: Term, f_case: Term| Term::Match {
            discriminant: Box::new(v("b")),
            motive: Box::new(lm("_", g("Bool"), prop())),
            cases: vec![t_case, f_case],
        };

        // bool_tf_ne : Not (Eq Bool true false)  — transport True@true along h to False@false.
        let tf_discr = bool_match(g("True"), g("False"));
        let tf_motive = lm("b", g("Bool"), lm("_", eqb(g("true"), v("b")), tf_discr));
        let bool_tf_ne = lm(
            "h",
            eqb(g("true"), g("false")),
            ap(
                ap(ap(ap(ap(ap(g("Eq_rec_dep"), g("Bool")), g("true")), tf_motive), g("I")), g("false")),
                v("h"),
            ),
        );
        // bool_ft_ne : Not (Eq Bool false true)  — transport True@false along h to False@true.
        let ft_discr = bool_match(g("False"), g("True"));
        let ft_motive = lm("b", g("Bool"), lm("_", eqb(g("false"), v("b")), ft_discr));
        let bool_ft_ne = lm(
            "h",
            eqb(g("false"), g("true")),
            ap(
                ap(ap(ap(ap(ap(g("Eq_rec_dep"), g("Bool")), g("false")), ft_motive), g("I")), g("true")),
                v("h"),
            ),
        );

        // decEqBool : Π(a b:Bool). Decidable (Eq Bool a b)
        let de_ty = Term::Pi {
            param: "a".to_string(),
            param_type: Box::new(g("Bool")),
            body_type: Box::new(Term::Pi {
                param: "b".to_string(),
                param_type: Box::new(g("Bool")),
                body_type: Box::new(dec(eqb(v("a"), v("b")))),
            }),
        };
        // Inner match on `b`, with the outer `a` fixed to a constructor `af`.
        let inner = |af: Term, both_same: Term, ne_proof: Term, same_first: bool| {
            let motive = lm("b'", g("Bool"), dec(eqb(af.clone(), v("b'"))));
            // cases in Bool order [true, false]; `same` is when b matches af.
            let (t_case, f_case) = if same_first {
                // af == true
                (
                    is_true(eqb(af.clone(), g("true")), both_same),
                    is_false(eqb(af.clone(), g("false")), ne_proof),
                )
            } else {
                // af == false
                (
                    is_false(eqb(af.clone(), g("true")), ne_proof),
                    is_true(eqb(af.clone(), g("false")), both_same),
                )
            };
            Term::Match {
                discriminant: Box::new(v("b")),
                motive: Box::new(motive),
                cases: vec![t_case, f_case],
            }
        };
        let a_true = inner(g("true"), refl(g("true")), bool_tf_ne, true);
        let a_false = inner(g("false"), refl(g("false")), bool_ft_ne, false);
        let outer_motive = lm("a'", g("Bool"), dec(eqb(v("a'"), v("b"))));
        let de_body = lm(
            "a",
            g("Bool"),
            lm(
                "b",
                g("Bool"),
                Term::Match {
                    discriminant: Box::new(v("a")),
                    motive: Box::new(outer_motive),
                    cases: vec![a_true, a_false],
                },
            ),
        );
        ctx.add_definition("decEqBool".to_string(), de_ty, de_body);
    }

    /// Decidable equality of `Nat` (`decEqNat : Π(a b:Nat). Decidable (Eq Nat a b)`) — the
    /// flagship: it lets `decide` discharge arithmetic (in)equalities. Built by structural
    /// recursion on `a`, using derived Nat no-confusion (`Zero ≠ Succ`), `Succ` congruence
    /// (via J), and `Succ` injectivity (via a `pred` congruence). All derived — no axioms.
    fn register_dec_eq_nat(ctx: &mut Context) {
        let prop = || Term::Sort(Universe::Prop);
        let nat = || Term::Global("Nat".to_string());
        let g = |s: &str| Term::Global(s.to_string());
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let lm = |p: &str, t: Term, b: Term| Term::Lambda {
            param: p.to_string(),
            param_type: Box::new(t),
            body: Box::new(b),
        };
        let pi = |p: &str, t: Term, b: Term| Term::Pi {
            param: p.to_string(),
            param_type: Box::new(t),
            body_type: Box::new(b),
        };
        let succ = |n: Term| ap(g("Succ"), n);
        let eqn = |x: Term, y: Term| ap(ap(ap(g("Eq"), nat()), x), y);
        let not = |p: Term| ap(g("Not"), p);
        let dec = |p: Term| ap(g("Decidable"), p);
        let is_true = |p: Term, h: Term| ap(ap(g("isTrue"), p), h);
        let is_false = |p: Term, h: Term| ap(ap(g("isFalse"), p), h);
        let refln = |x: Term| ap(ap(g("refl"), nat()), x);
        // `match k return (λ_:Nat. R) with { <Zero-case>, <Succ-case> }` (Nat order Zero,Succ).
        let nat_match = |ret: Term, zero_case: Term, succ_case: Term| Term::Match {
            discriminant: Box::new(v("k")),
            motive: Box::new(lm("_", nat(), ret)),
            cases: vec![zero_case, succ_case],
        };
        // Transport `Eq_rec_dep Nat x (λk.λ_:Eq Nat x k. discr) base y h`.
        let transport = |x: Term, discr_motive: Term, base: Term, y: Term, h: Term| {
            ap(ap(ap(ap(ap(ap(g("Eq_rec_dep"), nat()), x), discr_motive), base), y), h)
        };

        // nat_zs_ne : Π(n:Nat). Not (Eq Nat Zero (Succ n))  — Zero ≠ Succ n.
        let d_zs = nat_match(prop(), g("True"), lm("_", nat(), g("False")));
        let zs_motive = lm("k", nat(), lm("_", eqn(g("Zero"), v("k")), d_zs));
        let nat_zs_ne = lm(
            "n",
            nat(),
            lm(
                "h",
                eqn(g("Zero"), succ(v("n"))),
                transport(g("Zero"), zs_motive, g("I"), succ(v("n")), v("h")),
            ),
        );
        ctx.add_definition("nat_zs_ne".to_string(), pi("n", nat(), not(eqn(g("Zero"), succ(v("n"))))), nat_zs_ne);

        // nat_sz_ne : Π(n:Nat). Not (Eq Nat (Succ n) Zero)  — Succ n ≠ Zero.
        let d_sz = nat_match(prop(), g("False"), lm("_", nat(), g("True")));
        let sz_motive = lm("k", nat(), lm("_", eqn(succ(v("n")), v("k")), d_sz));
        let nat_sz_ne = lm(
            "n",
            nat(),
            lm(
                "h",
                eqn(succ(v("n")), g("Zero")),
                transport(succ(v("n")), sz_motive, g("I"), g("Zero"), v("h")),
            ),
        );
        ctx.add_definition("nat_sz_ne".to_string(), pi("n", nat(), not(eqn(succ(v("n")), g("Zero")))), nat_sz_ne);

        // succ_cong : Π(a b:Nat). Eq Nat a b → Eq Nat (Succ a) (Succ b).
        let sc_motive = lm("b'", nat(), lm("_", eqn(v("a"), v("b'")), eqn(succ(v("a")), succ(v("b'")))));
        let succ_cong_body = lm(
            "a",
            nat(),
            lm(
                "b",
                nat(),
                lm(
                    "h",
                    eqn(v("a"), v("b")),
                    ap(ap(ap(ap(ap(ap(g("Eq_rec_dep"), nat()), v("a")), sc_motive), refln(succ(v("a")))), v("b")), v("h")),
                ),
            ),
        );
        let succ_cong_ty = pi("a", nat(), pi("b", nat(), pi("_", eqn(v("a"), v("b")), eqn(succ(v("a")), succ(v("b"))))));
        ctx.add_definition("succ_cong".to_string(), succ_cong_ty, succ_cong_body);

        // succ_inj : Π(a b:Nat). Eq Nat (Succ a) (Succ b) → Eq Nat a b.
        // `pred y := match y with Zero => Zero | Succ p => p`; transport `Eq Nat a (pred y)`
        // along `h`, base `refl Nat a` (since `pred (Succ a) ≡ a`), result `Eq Nat a b`.
        let pred_y = Term::Match {
            discriminant: Box::new(v("y'")),
            motive: Box::new(lm("_", nat(), nat())),
            cases: vec![g("Zero"), lm("p", nat(), v("p"))],
        };
        let si_motive = lm("y'", nat(), lm("_", eqn(succ(v("a")), v("y'")), eqn(v("a"), pred_y)));
        let succ_inj_body = lm(
            "a",
            nat(),
            lm(
                "b",
                nat(),
                lm(
                    "h",
                    eqn(succ(v("a")), succ(v("b"))),
                    ap(ap(ap(ap(ap(ap(g("Eq_rec_dep"), nat()), succ(v("a"))), si_motive), refln(v("a"))), succ(v("b"))), v("h")),
                ),
            ),
        );
        let succ_inj_ty = pi("a", nat(), pi("b", nat(), pi("_", eqn(succ(v("a")), succ(v("b"))), eqn(v("a"), v("b")))));
        ctx.add_definition("succ_inj".to_string(), succ_inj_ty, succ_inj_body);

        // decEqNat : Π(a b:Nat). Decidable (Eq Nat a b)  — structural recursion on `a`.
        // Inner match on the recursive result `rec a' b'`.
        let rec_call = ap(ap(v("rec"), v("a'")), v("b'"));
        let true_case = lm(
            "hp",
            eqn(v("a'"), v("b'")),
            is_true(eqn(succ(v("a'")), succ(v("b'"))), ap(ap(ap(g("succ_cong"), v("a'")), v("b'")), v("hp"))),
        );
        let false_case = lm(
            "hnp",
            not(eqn(v("a'"), v("b'"))),
            is_false(
                eqn(succ(v("a'")), succ(v("b'"))),
                lm(
                    "hs",
                    eqn(succ(v("a'")), succ(v("b'"))),
                    ap(v("hnp"), ap(ap(ap(g("succ_inj"), v("a'")), v("b'")), v("hs"))),
                ),
            ),
        );
        let inner_rec_match = Term::Match {
            discriminant: Box::new(rec_call),
            motive: Box::new(lm("_", dec(eqn(v("a'"), v("b'"))), dec(eqn(succ(v("a'")), succ(v("b'")))))),
            cases: vec![true_case, false_case],
        };
        // b-match in the `Succ a'` branch: motive λb'. Decidable (Eq Nat (Succ a') b').
        let succ_b_match = Term::Match {
            discriminant: Box::new(v("b")),
            motive: Box::new(lm("b'", nat(), dec(eqn(succ(v("a'")), v("b'"))))),
            cases: vec![
                is_false(eqn(succ(v("a'")), g("Zero")), ap(g("nat_sz_ne"), v("a'"))),
                lm("b'", nat(), inner_rec_match),
            ],
        };
        // b-match in the `Zero` branch: motive λb'. Decidable (Eq Nat Zero b').
        let zero_b_match = Term::Match {
            discriminant: Box::new(v("b")),
            motive: Box::new(lm("b'", nat(), dec(eqn(g("Zero"), v("b'"))))),
            cases: vec![
                is_true(eqn(g("Zero"), g("Zero")), refln(g("Zero"))),
                lm("b'", nat(), is_false(eqn(g("Zero"), succ(v("b'"))), ap(g("nat_zs_ne"), v("b'")))),
            ],
        };
        // Outer match on `a`: motive λa'. Decidable (Eq Nat a' b).
        let a_match = Term::Match {
            discriminant: Box::new(v("a")),
            motive: Box::new(lm("a'", nat(), dec(eqn(v("a'"), v("b"))))),
            cases: vec![zero_b_match, lm("a'", nat(), succ_b_match)],
        };
        let deceqnat_body = Term::Fix {
            name: "rec".to_string(),
            body: Box::new(lm("a", nat(), lm("b", nat(), a_match))),
        };
        let deceqnat_ty = pi("a", nat(), pi("b", nat(), dec(eqn(v("a"), v("b")))));
        ctx.add_definition("decEqNat".to_string(), deceqnat_ty, deceqnat_body);
    }

    /// The `native_decide` trust boundary (TCB additions, exactly as Lean's `native_decide`
    /// adds `ofReduceBool` + its compiler): `reduceBool : Bool → Bool` is the identity, but
    /// the KERNEL reduces `reduceBool t` by running the fast [`crate::eval`] evaluator (a
    /// reduction hook), and `ofReduceBool` turns `reduceBool a = b` into `a = b`. So a
    /// `native_decide` proof discharges `decide p inst = true` by native evaluation instead
    /// of the kernel re-normalizing the decision procedure.
    fn register_native_decide(ctx: &mut Context) {
        let g = |s: &str| Term::Global(s.to_string());
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let pi = |p: &str, t: Term, b: Term| Term::Pi {
            param: p.to_string(),
            param_type: Box::new(t),
            body_type: Box::new(b),
        };
        let eqb = |x: Term, y: Term| ap(ap(ap(g("Eq"), g("Bool")), x), y);
        // reduceBool : Bool → Bool
        ctx.add_declaration("reduceBool", pi("_", g("Bool"), g("Bool")));
        // ofReduceBool : Π(a:Bool). Π(b:Bool). Eq Bool (reduceBool a) b → Eq Bool a b
        ctx.add_declaration(
            "ofReduceBool",
            pi(
                "a",
                g("Bool"),
                pi(
                    "b",
                    g("Bool"),
                    pi("_", eqb(ap(g("reduceBool"), v("a")), v("b")), eqb(v("a"), v("b"))),
                ),
            ),
        );
    }

    /// QUOTIENT TYPES — CIC primitives (as in Lean). `Quot A r` is the quotient of `A` by
    /// the relation `r`; `Quot_mk` forms classes; `Quot_lift` lifts a relation-respecting
    /// function (with the definitional computation rule `Quot_lift … (Quot_mk a) ≡ f a`
    /// implemented in `reduction.rs`); `Quot_ind` is the induction principle; and the
    /// `Quot_sound` AXIOM identifies related representatives — the propositional content that
    /// makes `Quot` a genuine quotient. `Quot A r` is OPAQUE (not an inductive), so it cannot
    /// be pattern-matched — only `Quot_lift`/`Quot_ind` eliminate it, which is what keeps the
    /// identification consistent. Unblocks ℤ/ℚ/ℝ, `Multiset`, setoids.
    fn register_quot(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let prop = || Term::Sort(Universe::Prop);
        let g = |s: &str| Term::Global(s.to_string());
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let pi = |p: &str, t: Term, b: Term| Term::Pi {
            param: p.to_string(),
            param_type: Box::new(t),
            body_type: Box::new(b),
        };
        let arrow = |t: Term, b: Term| pi("_", t, b);
        // r : A → A → Prop
        let rel = || arrow(v("A"), arrow(v("A"), prop()));
        let quot = |a: Term, r: Term| ap(ap(g("Quot"), a), r);
        let mk = |a: Term, r: Term, x: Term| ap(ap(ap(g("Quot_mk"), a), r), x);
        let eq3 = |t: Term, x: Term, y: Term| ap(ap(ap(g("Eq"), t), x), y);

        // Quot : Π(A:Type). (A → A → Prop) → Type
        ctx.add_declaration("Quot", pi("A", type0(), arrow(rel(), type0())));
        // Quot_mk : Π(A:Type). Π(r:A→A→Prop). A → Quot A r
        ctx.add_declaration(
            "Quot_mk",
            pi("A", type0(), pi("r", rel(), arrow(v("A"), quot(v("A"), v("r"))))),
        );
        // Quot_lift : Π(A). Π(r). Π(B:Type). Π(f:A→B).
        //   Π(h: Π(a b:A). r a b → Eq B (f a) (f b)). Quot A r → B
        let resp = pi(
            "a",
            v("A"),
            pi(
                "b",
                v("A"),
                arrow(
                    ap(ap(v("r"), v("a")), v("b")),
                    eq3(v("B"), ap(v("f"), v("a")), ap(v("f"), v("b"))),
                ),
            ),
        );
        ctx.add_declaration(
            "Quot_lift",
            pi(
                "A",
                type0(),
                pi(
                    "r",
                    rel(),
                    pi(
                        "B",
                        type0(),
                        pi(
                            "f",
                            arrow(v("A"), v("B")),
                            pi("h", resp, arrow(quot(v("A"), v("r")), v("B"))),
                        ),
                    ),
                ),
            ),
        );
        // Quot_sound : Π(A). Π(r). Π(a b:A). r a b → Eq (Quot A r) (Quot_mk A r a) (Quot_mk A r b)
        ctx.add_declaration(
            "Quot_sound",
            pi(
                "A",
                type0(),
                pi(
                    "r",
                    rel(),
                    pi(
                        "a",
                        v("A"),
                        pi(
                            "b",
                            v("A"),
                            arrow(
                                ap(ap(v("r"), v("a")), v("b")),
                                eq3(
                                    quot(v("A"), v("r")),
                                    mk(v("A"), v("r"), v("a")),
                                    mk(v("A"), v("r"), v("b")),
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        );
        // Quot_ind : Π(A). Π(r). Π(P: Quot A r → Prop).
        //   (Π(a:A). P (Quot_mk A r a)) → Π(q: Quot A r). P q
        ctx.add_declaration(
            "Quot_ind",
            pi(
                "A",
                type0(),
                pi(
                    "r",
                    rel(),
                    pi(
                        "P",
                        arrow(quot(v("A"), v("r")), prop()),
                        arrow(
                            pi("a", v("A"), ap(v("P"), mk(v("A"), v("r"), v("a")))),
                            pi("q", quot(v("A"), v("r")), ap(v("P"), v("q"))),
                        ),
                    ),
                ),
            ),
        );
    }

    /// Eq : Π(A : Type 0). A → A → Prop
    /// refl : Π(A : Type 0). Π(x : A). Eq A x x
    /// Well-founded recursion. `Acc (A)(R) : A → Prop` is the accessibility
    /// predicate — `x` is accessible under `R` when every `R`-predecessor of `x` is
    /// accessible — with the single constructor
    /// `Acc_intro : Π(A)(R)(x). (Π(y:A). R y x → Acc A R y) → Acc A R x`. Its
    /// recursive field is FUNCTIONAL (the accessibility of all predecessors), a
    /// strictly-positive occurrence under a `Π`; the auto-derived eliminator
    /// `Acc_rec` carries the matching functional induction hypothesis, and recursion
    /// over an `Acc` proof terminates by the applied-smaller guard. `WellFounded A R`
    /// abbreviates `Π(x:A). Acc A R x`. Together these are the substrate for
    /// definitions by well-founded recursion (gcd, division, strong induction) that
    /// structural recursion cannot express — the parity item Lean's kernel has and
    /// ours did not.
    fn register_acc(ctx: &mut Context) {
        fn pi(p: &str, t: Term, b: Term) -> Term {
            Term::Pi { param: p.to_string(), param_type: Box::new(t), body_type: Box::new(b) }
        }
        fn arrow(a: Term, b: Term) -> Term {
            pi("_", a, b)
        }
        fn v(n: &str) -> Term {
            Term::Var(n.to_string())
        }
        fn g(n: &str) -> Term {
            Term::Global(n.to_string())
        }
        fn apps(f: Term, xs: &[Term]) -> Term {
            xs.iter().fold(f, |a, x| Term::App(Box::new(a), Box::new(x.clone())))
        }
        let type0 = || Term::Sort(Universe::Type(0));
        let prop = || Term::Sort(Universe::Prop);
        // R : A → A → Prop
        let rel = || arrow(v("A"), arrow(v("A"), prop()));
        // Acc A R x
        let acc = |x: Term| apps(g("Acc"), &[v("A"), v("R"), x]);

        // Acc : Π(A:Type). Π(R:A→A→Prop). A → Prop  — two params (A, R), one index (x).
        let acc_ty = pi("A", type0(), pi("R", rel(), arrow(v("A"), prop())));
        ctx.add_indexed_inductive("Acc", acc_ty, 2);

        // Acc_intro : Π(A)(R)(x). (Π(y:A). R y x → Acc A R y) → Acc A R x
        let intro_ty = pi(
            "A",
            type0(),
            pi(
                "R",
                rel(),
                pi(
                    "x",
                    v("A"),
                    arrow(
                        pi("y", v("A"), arrow(apps(v("R"), &[v("y"), v("x")]), acc(v("y")))),
                        acc(v("x")),
                    ),
                ),
            ),
        );
        ctx.add_constructor("Acc_intro", "Acc", intro_ty);

        // Acc_rec — the AUTO-DERIVED dependent eliminator carrying the functional
        // induction hypothesis (two-kernel verified in tests/well_founded.rs). A
        // kernel-checked definition, not an axiom.
        let (rec_ty, rec_body) = crate::recursor::derive_recursor(ctx, "Acc")
            .expect("Acc's dependent eliminator must derive");
        ctx.add_definition("Acc_rec".to_string(), rec_ty, rec_body);

        // WellFounded A R := Π(x:A). Acc A R x  — `R` is well-founded iff every
        // element is accessible. A plain definition (abbreviation), no new inductive.
        let wf_ty = pi("A", type0(), arrow(rel(), prop()));
        let wf_body = Term::Lambda {
            param: "A".to_string(),
            param_type: Box::new(type0()),
            body: Box::new(Term::Lambda {
                param: "R".to_string(),
                param_type: Box::new(rel()),
                body: Box::new(pi("x", v("A"), acc(v("x")))),
            }),
        };
        ctx.add_definition("WellFounded".to_string(), wf_ty, wf_body);
    }

    fn register_eq(ctx: &mut Context) {
        // Eq : Π(A : Type 0). A → A → Prop
        let eq_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(Term::Var("A".to_string())),
                body_type: Box::new(Term::Pi {
                    param: "y".to_string(),
                    param_type: Box::new(Term::Var("A".to_string())),
                    body_type: Box::new(Term::Sort(Universe::Prop)),
                }),
            }),
        };
        // `Eq (A) (x) : A → Prop` — the leading `A` and `x` are uniform parameters
        // (`refl`'s result `Eq A x x` repeats them), the trailing slot is the index.
        ctx.add_indexed_inductive("Eq", eq_type, 2);

        // refl : Π(A : Type 0). Π(x : A). Eq A x x
        let refl_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(Term::Var("A".to_string())),
                body_type: Box::new(Term::App(
                    Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("Eq".to_string())),
                            Box::new(Term::Var("A".to_string())),
                        )),
                        Box::new(Term::Var("x".to_string())),
                    )),
                    Box::new(Term::Var("x".to_string())),
                )),
            }),
        };
        ctx.add_constructor("refl", "Eq", refl_type);

        // `Eq_rec_dep` — the AUTO-DERIVED full dependent eliminator (Paulin-Mohring J):
        // `Π(A). Π(x:A). Π(P:Π(y:A). Eq A x y → Type). P x (refl A x) → Π(y). Π(h:Eq A x y).
        // P y h`. A kernel-CHECKED definition (two-kernel verified), not an axiom — so the
        // equality eliminator and its lemmas below leave the trusted base.
        Self::register_eq_recursor(ctx);

        // Eq_rec : Π(A:Type). Π(x:A). Π(P:A→Prop). P x → Π(y:A). Eq A x y → P y
        // The (proof-irrelevant) substitution eliminator — now DERIVED from J.
        Self::register_eq_rec(ctx);

        // Eq_sym : Π(A:Type). Π(x:A). Π(y:A). Eq A x y → Eq A y x — DERIVED from J.
        Self::register_eq_sym(ctx);

        // Eq_trans : Π(A:Type). Π(x:A). Π(y:A). Π(z:A). Eq A x y → Eq A y z → Eq A x z — DERIVED.
        Self::register_eq_trans(ctx);
    }

    /// Register the auto-derived dependent equality eliminator `Eq_rec_dep` (full J) as a
    /// kernel-checked definition. `Eq` and `refl` must already be registered.
    fn register_eq_recursor(ctx: &mut Context) {
        let (ty, body) = crate::recursor::derive_recursor(ctx, "Eq")
            .expect("Eq's dependent eliminator (J) must derive");
        ctx.add_definition("Eq_rec_dep".to_string(), ty, body);
    }

    /// Eq_rec : Π(A:Type). Π(x:A). Π(P:A→Prop). P x → Π(y:A). Eq A x y → P y
    /// The eliminator for equality - Leibniz's Law / Substitution of Equals
    fn register_eq_rec(ctx: &mut Context) {
        let a = Term::Var("A".to_string());
        let x = Term::Var("x".to_string());
        let y = Term::Var("y".to_string());
        let p = Term::Var("P".to_string());

        // P x = P applied to x
        let p_x = Term::App(Box::new(p.clone()), Box::new(x.clone()));

        // P y = P applied to y
        let p_y = Term::App(Box::new(p.clone()), Box::new(y.clone()));

        // Eq A x y
        let eq_a_x_y = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(x.clone()),
            )),
            Box::new(y.clone()),
        );

        // P : A → Prop
        let p_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(a.clone()),
            body_type: Box::new(Term::Sort(Universe::Prop)),
        };

        // Full type: Π(A:Type). Π(x:A). Π(P:A→Prop). P x → Π(y:A). Eq A x y → P y
        let eq_rec_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(a.clone()),
                body_type: Box::new(Term::Pi {
                    param: "P".to_string(),
                    param_type: Box::new(p_type),
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(p_x),
                        body_type: Box::new(Term::Pi {
                            param: "y".to_string(),
                            param_type: Box::new(a.clone()),
                            body_type: Box::new(Term::Pi {
                                param: "_".to_string(),
                                param_type: Box::new(eq_a_x_y),
                                body_type: Box::new(p_y),
                            }),
                        }),
                    }),
                }),
            }),
        };

        // Body — the substitution eliminator DERIVED from J by discarding the proof in the
        // motive: `λA x P base y h. Eq_rec_dep A x (λy'. λ_:Eq A x y'. P y') base y h`.
        let gl = |s: &str| Term::Global(s.to_string());
        let vr = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let lm = |p: &str, t: Term, b: Term| Term::Lambda {
            param: p.to_string(),
            param_type: Box::new(t),
            body: Box::new(b),
        };
        let eq3 = |a: Term, x: Term, y: Term| ap(ap(ap(gl("Eq"), a), x), y);
        let dep_motive = lm(
            "y'",
            vr("A"),
            lm("_", eq3(vr("A"), vr("x"), vr("y'")), ap(vr("P"), vr("y'"))),
        );
        let call = ap(
            ap(
                ap(ap(ap(ap(gl("Eq_rec_dep"), vr("A")), vr("x")), dep_motive), vr("base")),
                vr("y"),
            ),
            vr("h"),
        );
        let eq_rec_body = lm(
            "A",
            Term::Sort(Universe::Type(0)),
            lm(
                "x",
                vr("A"),
                lm(
                    "P",
                    Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(vr("A")),
                        body_type: Box::new(Term::Sort(Universe::Prop)),
                    },
                    lm(
                        "base",
                        ap(vr("P"), vr("x")),
                        lm("y", vr("A"), lm("h", eq3(vr("A"), vr("x"), vr("y")), call)),
                    ),
                ),
            ),
        );

        ctx.add_definition("Eq_rec".to_string(), eq_rec_type, eq_rec_body);
    }

    /// Eq_sym : Π(A:Type). Π(x:A). Π(y:A). Eq A x y → Eq A y x
    /// Symmetry of equality
    fn register_eq_sym(ctx: &mut Context) {
        let a = Term::Var("A".to_string());
        let x = Term::Var("x".to_string());
        let y = Term::Var("y".to_string());

        // Eq A x y
        let eq_a_x_y = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(x.clone()),
            )),
            Box::new(y.clone()),
        );

        // Eq A y x
        let eq_a_y_x = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(y.clone()),
            )),
            Box::new(x.clone()),
        );

        // Π(A:Type). Π(x:A). Π(y:A). Eq A x y → Eq A y x
        let eq_sym_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(a.clone()),
                body_type: Box::new(Term::Pi {
                    param: "y".to_string(),
                    param_type: Box::new(a),
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(eq_a_x_y),
                        body_type: Box::new(eq_a_y_x),
                    }),
                }),
            }),
        };

        // Body — symmetry DERIVED from J: transport `Eq A x _` along `h` with base `refl`.
        // `λA x y h. Eq_rec_dep A x (λy'. λ_:Eq A x y'. Eq A y' x) (refl A x) y h`.
        let gl = |s: &str| Term::Global(s.to_string());
        let vr = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let lm = |p: &str, t: Term, b: Term| Term::Lambda {
            param: p.to_string(),
            param_type: Box::new(t),
            body: Box::new(b),
        };
        let eq3 = |a: Term, x: Term, y: Term| ap(ap(ap(gl("Eq"), a), x), y);
        let motive = lm(
            "y'",
            vr("A"),
            lm("_", eq3(vr("A"), vr("x"), vr("y'")), eq3(vr("A"), vr("y'"), vr("x"))),
        );
        let refl_a_x = ap(ap(gl("refl"), vr("A")), vr("x"));
        let call = ap(
            ap(
                ap(ap(ap(ap(gl("Eq_rec_dep"), vr("A")), vr("x")), motive), refl_a_x),
                vr("y"),
            ),
            vr("h"),
        );
        let eq_sym_body = lm(
            "A",
            Term::Sort(Universe::Type(0)),
            lm(
                "x",
                vr("A"),
                lm("y", vr("A"), lm("h", eq3(vr("A"), vr("x"), vr("y")), call)),
            ),
        );

        ctx.add_definition("Eq_sym".to_string(), eq_sym_type, eq_sym_body);
    }

    /// Eq_trans : Π(A:Type). Π(x:A). Π(y:A). Π(z:A). Eq A x y → Eq A y z → Eq A x z
    /// Transitivity of equality
    fn register_eq_trans(ctx: &mut Context) {
        let a = Term::Var("A".to_string());
        let x = Term::Var("x".to_string());
        let y = Term::Var("y".to_string());
        let z = Term::Var("z".to_string());

        // Eq A x y
        let eq_a_x_y = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(x.clone()),
            )),
            Box::new(y.clone()),
        );

        // Eq A y z
        let eq_a_y_z = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(y.clone()),
            )),
            Box::new(z.clone()),
        );

        // Eq A x z
        let eq_a_x_z = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Eq".to_string())),
                    Box::new(a.clone()),
                )),
                Box::new(x.clone()),
            )),
            Box::new(z.clone()),
        );

        // Π(A:Type). Π(x:A). Π(y:A). Π(z:A). Eq A x y → Eq A y z → Eq A x z
        let eq_trans_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "x".to_string(),
                param_type: Box::new(a.clone()),
                body_type: Box::new(Term::Pi {
                    param: "y".to_string(),
                    param_type: Box::new(a.clone()),
                    body_type: Box::new(Term::Pi {
                        param: "z".to_string(),
                        param_type: Box::new(a),
                        body_type: Box::new(Term::Pi {
                            param: "_".to_string(),
                            param_type: Box::new(eq_a_x_y),
                            body_type: Box::new(Term::Pi {
                                param: "_".to_string(),
                                param_type: Box::new(eq_a_y_z),
                                body_type: Box::new(eq_a_x_z),
                            }),
                        }),
                    }),
                }),
            }),
        };

        // Body — transitivity DERIVED from J: transport `Eq A x _` along `hyz` (relating
        // `y` and `z`) with base `hxy`.
        // `λA x y z hxy hyz. Eq_rec_dep A y (λz'. λ_:Eq A y z'. Eq A x z') hxy z hyz`.
        let gl = |s: &str| Term::Global(s.to_string());
        let vr = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let lm = |p: &str, t: Term, b: Term| Term::Lambda {
            param: p.to_string(),
            param_type: Box::new(t),
            body: Box::new(b),
        };
        let eq3 = |a: Term, x: Term, y: Term| ap(ap(ap(gl("Eq"), a), x), y);
        let motive = lm(
            "z'",
            vr("A"),
            lm("_", eq3(vr("A"), vr("y"), vr("z'")), eq3(vr("A"), vr("x"), vr("z'"))),
        );
        let call = ap(
            ap(
                ap(ap(ap(ap(gl("Eq_rec_dep"), vr("A")), vr("y")), motive), vr("hxy")),
                vr("z"),
            ),
            vr("hyz"),
        );
        let eq_trans_body = lm(
            "A",
            Term::Sort(Universe::Type(0)),
            lm(
                "x",
                vr("A"),
                lm(
                    "y",
                    vr("A"),
                    lm(
                        "z",
                        vr("A"),
                        lm(
                            "hxy",
                            eq3(vr("A"), vr("x"), vr("y")),
                            lm("hyz", eq3(vr("A"), vr("y"), vr("z")), call),
                        ),
                    ),
                ),
            ),
        );

        ctx.add_definition("Eq_trans".to_string(), eq_trans_type, eq_trans_body);
    }

    /// And : Prop → Prop → Prop
    /// conj : Π(P : Prop). Π(Q : Prop). P → Q → And P Q
    fn register_and(ctx: &mut Context) {
        // And : Prop → Prop → Prop
        let and_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Sort(Universe::Prop)),
            }),
        };
        ctx.add_inductive("And", and_type);

        // conj : Π(P : Prop). Π(Q : Prop). P → Q → And P Q
        let conj_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Pi {
                    param: "p".to_string(),
                    param_type: Box::new(Term::Var("P".to_string())),
                    body_type: Box::new(Term::Pi {
                        param: "q".to_string(),
                        param_type: Box::new(Term::Var("Q".to_string())),
                        body_type: Box::new(Term::App(
                            Box::new(Term::App(
                                Box::new(Term::Global("And".to_string())),
                                Box::new(Term::Var("P".to_string())),
                            )),
                            Box::new(Term::Var("Q".to_string())),
                        )),
                    }),
                }),
            }),
        };
        ctx.add_constructor("conj", "And", conj_type);
    }

    /// Or : Prop → Prop → Prop
    /// left : Π(P : Prop). Π(Q : Prop). P → Or P Q
    /// right : Π(P : Prop). Π(Q : Prop). Q → Or P Q
    fn register_or(ctx: &mut Context) {
        // Or : Prop → Prop → Prop
        let or_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Sort(Universe::Prop)),
            }),
        };
        ctx.add_inductive("Or", or_type);

        // left : Π(P : Prop). Π(Q : Prop). P → Or P Q
        let left_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Pi {
                    param: "p".to_string(),
                    param_type: Box::new(Term::Var("P".to_string())),
                    body_type: Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("Or".to_string())),
                            Box::new(Term::Var("P".to_string())),
                        )),
                        Box::new(Term::Var("Q".to_string())),
                    )),
                }),
            }),
        };
        ctx.add_constructor("left", "Or", left_type);

        // right : Π(P : Prop). Π(Q : Prop). Q → Or P Q
        let right_type = Term::Pi {
            param: "P".to_string(),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(Term::Pi {
                param: "Q".to_string(),
                param_type: Box::new(Term::Sort(Universe::Prop)),
                body_type: Box::new(Term::Pi {
                    param: "q".to_string(),
                    param_type: Box::new(Term::Var("Q".to_string())),
                    body_type: Box::new(Term::App(
                        Box::new(Term::App(
                            Box::new(Term::Global("Or".to_string())),
                            Box::new(Term::Var("P".to_string())),
                        )),
                        Box::new(Term::Var("Q".to_string())),
                    )),
                }),
            }),
        };
        ctx.add_constructor("right", "Or", right_type);
    }

    /// Ex : Π(A : Type 0). (A → Prop) → Prop
    /// witness : Π(A : Type 0). Π(P : A → Prop). Π(x : A). P x → Ex A P
    ///
    /// Ex is the existential type for propositions.
    /// Ex A P means "there exists an x:A such that P(x)"
    fn register_ex(ctx: &mut Context) {
        // Ex : Π(A : Type 0). (A → Prop) → Prop
        let ex_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "P".to_string(),
                param_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(Term::Var("A".to_string())),
                    body_type: Box::new(Term::Sort(Universe::Prop)),
                }),
                body_type: Box::new(Term::Sort(Universe::Prop)),
            }),
        };
        ctx.add_inductive("Ex", ex_type);

        // witness : Π(A : Type 0). Π(P : A → Prop). Π(x : A). P x → Ex A P
        //
        // To construct an existential proof, provide:
        // - A: the type being quantified over
        // - P: the predicate
        // - x: the witness (an element of A)
        // - proof: evidence that P(x) holds
        let a = Term::Var("A".to_string());
        let p = Term::Var("P".to_string());
        let x = Term::Var("x".to_string());

        // P x = P applied to x
        let p_x = Term::App(Box::new(p.clone()), Box::new(x.clone()));

        // Ex A P = Ex applied to A, then to P
        let ex_a_p = Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("Ex".to_string())),
                Box::new(a.clone()),
            )),
            Box::new(p.clone()),
        );

        let witness_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Type(0))),
            body_type: Box::new(Term::Pi {
                param: "P".to_string(),
                param_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(a.clone()),
                    body_type: Box::new(Term::Sort(Universe::Prop)),
                }),
                body_type: Box::new(Term::Pi {
                    param: "x".to_string(),
                    param_type: Box::new(a),
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(p_x),
                        body_type: Box::new(ex_a_p),
                    }),
                }),
            }),
        };
        ctx.add_constructor("witness", "Ex", witness_type);
    }

    // -------------------------------------------------------------------------
    // Reflection System (Deep Embedding)
    // -------------------------------------------------------------------------

    /// Reflection types for deep embedding.
    ///
    /// Univ : Type 0 (representation of universes)
    /// Syntax : Type 0 (representation of terms with De Bruijn indices)
    /// syn_size : Syntax -> Int (compute size of syntax tree)
    /// syn_max_var : Syntax -> Int (compute max free variable index)
    fn register_reflection(ctx: &mut Context) {
        Self::register_univ(ctx);
        Self::register_syntax(ctx);
        Self::register_syn_size(ctx);
        Self::register_syn_max_var(ctx);
        Self::register_syn_lift(ctx);
        Self::register_syn_subst(ctx);
        Self::register_syn_beta(ctx);
        Self::register_syn_step(ctx);
        Self::register_syn_eval(ctx);
        Self::register_syn_quote(ctx);
        Self::register_syn_diag(ctx);
        Self::register_derivation(ctx);
        Self::register_concludes(ctx);
        Self::register_try_refl(ctx);
        Self::register_try_compute(ctx);
        Self::register_try_cong(ctx);
        Self::register_tact_fail(ctx);
        Self::register_tact_orelse(ctx);
        Self::register_tact_try(ctx);
        Self::register_tact_repeat(ctx);
        Self::register_tact_then(ctx);
        Self::register_tact_first(ctx);
        Self::register_tact_solve(ctx);
        Self::register_try_ring(ctx);
        Self::register_try_lia(ctx);
        Self::register_try_cc(ctx);
        Self::register_try_simp(ctx);
        Self::register_try_omega(ctx);
        Self::register_try_auto(ctx);
        Self::register_try_induction(ctx);
        Self::register_induction_helpers(ctx);
        Self::register_try_inversion_tactic(ctx);
        Self::register_operator_tactics(ctx);
        Self::register_hw_tactics(ctx);
    }

    /// Univ : Type 0 (representation of universes)
    /// UProp : Univ
    /// UType : Int -> Univ
    fn register_univ(ctx: &mut Context) {
        let univ = Term::Global("Univ".to_string());
        let int = Term::Global("Int".to_string());

        // Univ : Type 0
        ctx.add_inductive("Univ", Term::Sort(Universe::Type(0)));

        // UProp : Univ
        ctx.add_constructor("UProp", "Univ", univ.clone());

        // UType : Int -> Univ
        ctx.add_constructor(
            "UType",
            "Univ",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int),
                body_type: Box::new(univ),
            },
        );
    }

    /// Syntax : Type 0 (representation of terms with De Bruijn indices)
    ///
    /// SVar : Int -> Syntax (De Bruijn variable index)
    /// SGlobal : Int -> Syntax (global reference by ID)
    /// SSort : Univ -> Syntax (universe)
    /// SApp : Syntax -> Syntax -> Syntax (application)
    /// SLam : Syntax -> Syntax -> Syntax (lambda: param_type, body)
    /// SPi : Syntax -> Syntax -> Syntax (pi: param_type, body_type)
    fn register_syntax(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());
        let univ = Term::Global("Univ".to_string());

        // Syntax : Type 0
        ctx.add_inductive("Syntax", Term::Sort(Universe::Type(0)));

        // SVar : Int -> Syntax (De Bruijn index)
        ctx.add_constructor(
            "SVar",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(syntax.clone()),
            },
        );

        // SGlobal : Int -> Syntax (global reference by ID)
        ctx.add_constructor(
            "SGlobal",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int.clone()),
                body_type: Box::new(syntax.clone()),
            },
        );

        // SSort : Univ -> Syntax
        ctx.add_constructor(
            "SSort",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(univ),
                body_type: Box::new(syntax.clone()),
            },
        );

        // SApp : Syntax -> Syntax -> Syntax
        ctx.add_constructor(
            "SApp",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax.clone()),
                }),
            },
        );

        // SLam : Syntax -> Syntax -> Syntax (param_type, body)
        ctx.add_constructor(
            "SLam",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax.clone()),
                }),
            },
        );

        // SPi : Syntax -> Syntax -> Syntax (param_type, body_type)
        ctx.add_constructor(
            "SPi",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax.clone()),
                }),
            },
        );

        // SLit : Int -> Syntax (integer literal in quoted syntax)
        ctx.add_constructor(
            "SLit",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int),
                body_type: Box::new(syntax.clone()),
            },
        );

        // SName : Text -> Syntax (named global reference)
        let text = Term::Global("Text".to_string());
        ctx.add_constructor(
            "SName",
            "Syntax",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(text),
                body_type: Box::new(syntax),
            },
        );
    }

    /// syn_size : Syntax -> Int
    ///
    /// Computes the number of nodes in a syntax tree.
    /// Computational behavior defined in reduction.rs.
    fn register_syn_size(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        // syn_size : Syntax -> Int
        let syn_size_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(int),
        };

        ctx.add_declaration("syn_size", syn_size_type);
    }

    /// syn_max_var : Syntax -> Int
    ///
    /// Returns the maximum free variable index in a syntax term.
    /// Returns -1 if the term is closed (all variables bound).
    /// Computational behavior defined in reduction.rs.
    fn register_syn_max_var(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        let syn_max_var_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(int),
        };

        ctx.add_declaration("syn_max_var", syn_max_var_type);
    }

    // -------------------------------------------------------------------------
    // De Bruijn Operations (Substitution)
    // -------------------------------------------------------------------------

    /// syn_lift : Int -> Int -> Syntax -> Syntax
    ///
    /// Shifts free variables (index >= cutoff) by amount.
    /// - syn_lift amount cutoff term
    /// - Variables with index < cutoff are bound -> unchanged
    /// - Variables with index >= cutoff are free -> add amount
    /// Computational behavior defined in reduction.rs.
    fn register_syn_lift(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        // syn_lift : Int -> Int -> Syntax -> Syntax
        let syn_lift_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(int.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax),
                }),
            }),
        };

        ctx.add_declaration("syn_lift", syn_lift_type);
    }

    /// syn_subst : Syntax -> Int -> Syntax -> Syntax
    ///
    /// Substitutes replacement for variable at index in term.
    /// - syn_subst replacement index term
    /// - If term is SVar k and k == index, return replacement
    /// - If term is SVar k and k != index, return term unchanged
    /// - For binders, increment index and lift replacement
    /// Computational behavior defined in reduction.rs.
    fn register_syn_subst(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        // syn_subst : Syntax -> Int -> Syntax -> Syntax
        let syn_subst_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(int),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax),
                }),
            }),
        };

        ctx.add_declaration("syn_subst", syn_subst_type);
    }

    // -------------------------------------------------------------------------
    // Beta Reduction (Computation)
    // -------------------------------------------------------------------------

    /// syn_beta : Syntax -> Syntax -> Syntax
    ///
    /// Beta reduction: substitute arg for variable 0 in body, then decrement the
    /// surviving free variables (the λ binder is removed, so references past it
    /// drop one de Bruijn level). `syn_beta body arg = syn_subst arg 0 body`
    /// composed with that shift — a FAITHFUL reflection of real beta (unlike the
    /// raw, non-shifting `syn_subst` primitive).
    /// Computational behavior defined in reduction.rs.
    fn register_syn_beta(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());

        // syn_beta : Syntax -> Syntax -> Syntax
        let syn_beta_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(syntax),
            }),
        };

        ctx.add_declaration("syn_beta", syn_beta_type);
    }

    /// syn_step : Syntax -> Syntax
    ///
    /// Single-step head reduction. Finds first beta redex and reduces.
    /// - If SApp (SLam T body) arg: perform beta reduction
    /// - If SApp f x where f is reducible: reduce f
    /// - Otherwise: return unchanged (stuck or value)
    /// Computational behavior defined in reduction.rs.
    fn register_syn_step(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());

        // syn_step : Syntax -> Syntax
        let syn_step_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(syntax),
        };

        ctx.add_declaration("syn_step", syn_step_type);
    }

    // -------------------------------------------------------------------------
    // Bounded Evaluation
    // -------------------------------------------------------------------------

    /// syn_eval : Int -> Syntax -> Syntax
    ///
    /// Bounded evaluation: reduce for up to N steps.
    /// - syn_eval fuel term
    /// - If fuel <= 0: return term unchanged
    /// - Otherwise: step and repeat until normal form or fuel exhausted
    /// Computational behavior defined in reduction.rs.
    fn register_syn_eval(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let int = Term::Global("Int".to_string());

        // syn_eval : Int -> Syntax -> Syntax
        let syn_eval_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(int),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(syntax),
            }),
        };

        ctx.add_declaration("syn_eval", syn_eval_type);
    }

    // -------------------------------------------------------------------------
    // Reification (Quote)
    // -------------------------------------------------------------------------

    /// syn_quote : Syntax -> Syntax
    ///
    /// Converts a Syntax value to Syntax code that constructs it.
    /// Computational behavior defined in reduction.rs.
    fn register_syn_quote(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());

        // syn_quote : Syntax -> Syntax
        let syn_quote_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(syntax),
        };

        ctx.add_declaration("syn_quote", syn_quote_type);
    }

    // -------------------------------------------------------------------------
    // Diagonalization (Self-Reference)
    // -------------------------------------------------------------------------

    /// syn_diag : Syntax -> Syntax
    ///
    /// The diagonal function: syn_diag x = syn_subst (syn_quote x) 0 x
    /// This is the key construction for self-reference and the diagonal lemma.
    fn register_syn_diag(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());

        let syn_diag_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(syntax),
        };

        ctx.add_declaration("syn_diag", syn_diag_type);
    }

    // -------------------------------------------------------------------------
    // Inference Rules (Proof Trees)
    // -------------------------------------------------------------------------

    /// Derivation : Type 0 (deep embedding of proof trees)
    ///
    /// DAxiom : Syntax -> Derivation (introduce an axiom)
    /// DModusPonens : Derivation -> Derivation -> Derivation (A->B, A |- B)
    /// DUnivIntro : Derivation -> Derivation (P(x) |- forall x. P(x))
    /// DUnivElim : Derivation -> Syntax -> Derivation (forall x. P(x), t |- P(t))
    fn register_derivation(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Derivation : Type 0
        ctx.add_inductive("Derivation", Term::Sort(Universe::Type(0)));

        // DAxiom : Syntax -> Derivation
        ctx.add_constructor(
            "DAxiom",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DModusPonens : Derivation -> Derivation -> Derivation
        ctx.add_constructor(
            "DModusPonens",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()),
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // DUnivIntro : Derivation -> Derivation
        ctx.add_constructor(
            "DUnivIntro",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DUnivElim : Derivation -> Syntax -> Derivation
        ctx.add_constructor(
            "DUnivElim",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // DRefl : Syntax -> Syntax -> Derivation
        // DRefl T a proves (Eq T a a)
        ctx.add_constructor(
            "DRefl",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // DInduction : Syntax -> Derivation -> Derivation -> Derivation
        // DInduction motive base step proves (∀n:Nat. motive n) via induction
        // - motive: λn:Nat. P(n) as Syntax
        // - base: proof of P(Zero)
        // - step: proof of ∀k:Nat. P(k) → P(Succ k)
        ctx.add_constructor(
            "DInduction",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // motive
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // base proof
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(derivation.clone()), // step proof
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // DCompute : Syntax -> Derivation
        // DCompute goal proves goal by computation
        // - Validates that goal is (Eq T A B) and eval(A) == eval(B)
        ctx.add_constructor(
            "DCompute",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DCong : Syntax -> Derivation -> Derivation
        // DCong context eq_proof proves congruence
        // - context: SLam T body (function with hole at SVar 0)
        // - eq_proof: proof of (Eq T a b)
        // - Result: proof of (Eq T (body[0:=a]) (body[0:=b]))
        ctx.add_constructor(
            "DCong",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // context
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // eq_proof
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // DCase : Derivation -> Derivation -> Derivation
        // Cons cell for case proofs in DElim
        ctx.add_constructor(
            "DCase",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // head (case proof)
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // tail (rest of cases)
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // DCaseEnd : Derivation
        // Nil for case proofs in DElim
        ctx.add_constructor("DCaseEnd", "Derivation", derivation.clone());

        // DElim : Syntax -> Syntax -> Derivation -> Derivation
        // Generic elimination for any inductive type
        // - ind_type: the inductive type (e.g., SName "Nat" or SApp (SName "List") A)
        // - motive: SLam param_type body (the property to prove)
        // - cases: DCase chain with one proof per constructor
        ctx.add_constructor(
            "DElim",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // ind_type
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // motive
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(derivation.clone()), // cases
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // DInversion : Syntax -> Derivation
        // Proves False when no constructor can build the given hypothesis.
        // - hyp_type: the Syntax representation of the hypothesis (e.g., SApp (SName "Even") three)
        // - Returns proof of False if verified that all constructors are impossible
        ctx.add_constructor(
            "DInversion",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DRewrite : Derivation -> Syntax -> Syntax -> Derivation
        // Stores: eq_proof, original_goal, new_goal
        // Given eq_proof : Eq A x y, rewrites goal by replacing x with y
        ctx.add_constructor(
            "DRewrite",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // eq_proof
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // original_goal
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(syntax.clone()), // new_goal
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // DDestruct : Syntax -> Syntax -> Derivation -> Derivation
        // Case analysis without induction hypotheses
        // - ind_type: the inductive type
        // - motive: the property to prove
        // - cases: DCase chain with proofs for each constructor
        ctx.add_constructor(
            "DDestruct",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // ind_type
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // motive
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(derivation.clone()), // cases
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // DApply : Syntax -> Derivation -> Syntax -> Syntax -> Derivation
        // Manual backward chaining
        // - hyp_name: name of hypothesis
        // - hyp_proof: proof of the hypothesis
        // - original_goal: the goal we started with
        // - new_goal: the antecedent we need to prove
        ctx.add_constructor(
            "DApply",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // hyp_name
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // hyp_proof
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(syntax.clone()), // original_goal
                        body_type: Box::new(Term::Pi {
                            param: "_".to_string(),
                            param_type: Box::new(syntax), // new_goal
                            body_type: Box::new(derivation),
                        }),
                    }),
                }),
            },
        );
    }

    /// concludes : Derivation -> Syntax
    ///
    /// Extracts the conclusion from a derivation.
    /// Computational behavior defined in reduction.rs.
    fn register_concludes(ctx: &mut Context) {
        let derivation = Term::Global("Derivation".to_string());
        let syntax = Term::Global("Syntax".to_string());

        // concludes : Derivation -> Syntax
        let concludes_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(derivation),
            body_type: Box::new(syntax),
        };

        ctx.add_declaration("concludes", concludes_type);
    }

    // -------------------------------------------------------------------------
    // Core Tactics
    // -------------------------------------------------------------------------

    /// try_refl : Syntax -> Derivation
    ///
    /// Reflexivity tactic: given a goal, try to prove it by reflexivity.
    /// - If goal matches (Eq T a b) and a == b, returns DRefl T a
    /// - Otherwise returns DAxiom (SName "Error")
    /// Computational behavior defined in reduction.rs.
    fn register_try_refl(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_refl : Syntax -> Derivation
        let try_refl_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_refl", try_refl_type);
    }

    /// try_compute : Syntax -> Derivation
    ///
    /// Computation tactic: given a goal (Eq T A B), proves it by evaluating
    /// both sides and checking equality.
    /// Returns DCompute goal.
    /// Computational behavior defined in reduction.rs.
    fn register_try_compute(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_compute : Syntax -> Derivation
        let try_compute_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_compute", try_compute_type);
    }

    /// try_cong : Syntax -> Derivation -> Derivation
    ///
    /// Congruence tactic: given a context (SLam T body) and proof of (Eq T a b),
    /// produces proof of (Eq T (body[0:=a]) (body[0:=b])).
    /// Returns DCong context eq_proof.
    /// Computational behavior defined in reduction.rs.
    fn register_try_cong(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_cong : Syntax -> Derivation -> Derivation
        let try_cong_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax), // context
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // eq_proof
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("try_cong", try_cong_type);
    }

    // -------------------------------------------------------------------------
    // Tactic Combinators
    // -------------------------------------------------------------------------

    /// tact_fail : Syntax -> Derivation
    ///
    /// A tactic that always fails by returning DAxiom (SName "Error").
    /// Useful for testing combinators and as a base case.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_fail(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // tact_fail : Syntax -> Derivation
        let tact_fail_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("tact_fail", tact_fail_type);
    }

    /// tact_orelse : (Syntax -> Derivation) -> (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: try first tactic, if it fails (concludes to Error) try second.
    /// Enables composition of tactics with fallback behavior.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_orelse(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_orelse : (Syntax -> Derivation) -> (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_orelse_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type.clone()), // t1
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(tactic_type), // t2
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax), // goal
                    body_type: Box::new(derivation),
                }),
            }),
        };

        ctx.add_declaration("tact_orelse", tact_orelse_type);
    }

    /// tact_try : (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: try the tactic, but never fail.
    /// If the tactic fails (concludes to Error), return identity (DAxiom goal).
    /// Computational behavior defined in reduction.rs.
    fn register_tact_try(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_try : (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_try_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type), // t
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("tact_try", tact_try_type);
    }

    /// tact_repeat : (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: apply tactic repeatedly until it fails or makes no progress.
    /// Returns identity if tactic fails immediately.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_repeat(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_repeat : (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_repeat_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type), // t
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("tact_repeat", tact_repeat_type);
    }

    /// tact_then : (Syntax -> Derivation) -> (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: sequence two tactics (the ";" operator).
    /// Apply t1 to goal, then apply t2 to the result.
    /// Fails if either tactic fails.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_then(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_then : (Syntax -> Derivation) -> (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_then_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type.clone()), // t1
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(tactic_type), // t2
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax),
                    body_type: Box::new(derivation),
                }),
            }),
        };

        ctx.add_declaration("tact_then", tact_then_type);
    }

    /// tact_first : TList (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: try tactics from a list until one succeeds.
    /// Returns Error if all tactics fail or list is empty.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_first(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // TList (Syntax -> Derivation)
        let tactic_list_type = Term::App(
            Box::new(Term::Global("TList".to_string())),
            Box::new(tactic_type),
        );

        // tact_first : TList (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_first_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_list_type), // tactics
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("tact_first", tact_first_type);
    }

    /// tact_solve : (Syntax -> Derivation) -> Syntax -> Derivation
    ///
    /// Tactic combinator: tactic MUST completely solve the goal.
    /// If tactic returns Error, returns Error.
    /// If tactic returns a proof, returns that proof.
    /// Computational behavior defined in reduction.rs.
    fn register_tact_solve(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // Tactic type: Syntax -> Derivation
        let tactic_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()),
            body_type: Box::new(derivation.clone()),
        };

        // tact_solve : (Syntax -> Derivation) -> Syntax -> Derivation
        let tact_solve_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(tactic_type), // t
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            }),
        };

        ctx.add_declaration("tact_solve", tact_solve_type);
    }

    // -------------------------------------------------------------------------
    // Ring Tactic (Polynomial Equality)
    // -------------------------------------------------------------------------

    /// DRingSolve : Syntax -> Derivation
    /// try_ring : Syntax -> Derivation
    ///
    /// Ring tactic: proves polynomial equalities by normalization.
    /// Computational behavior defined in reduction.rs.
    fn register_try_ring(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DRingSolve : Syntax -> Derivation
        // Proof constructor for ring-solved equalities
        ctx.add_constructor(
            "DRingSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_ring : Syntax -> Derivation
        // Ring tactic: given a goal, try to prove it by polynomial normalization
        let try_ring_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_ring", try_ring_type);
    }

    // -------------------------------------------------------------------------
    // LIA Tactic (Linear Integer Arithmetic)
    // -------------------------------------------------------------------------

    /// DLiaSolve : Syntax -> Derivation
    /// try_lia : Syntax -> Derivation
    ///
    /// LIA tactic: proves linear inequalities by Fourier-Motzkin elimination.
    /// Computational behavior defined in reduction.rs.
    fn register_try_lia(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DLiaSolve : Syntax -> Derivation
        // Proof constructor for LIA-solved inequalities
        ctx.add_constructor(
            "DLiaSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_lia : Syntax -> Derivation
        // LIA tactic: given a goal, try to prove it by linear arithmetic
        let try_lia_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_lia", try_lia_type);
    }

    /// DccSolve : Syntax -> Derivation
    /// try_cc : Syntax -> Derivation
    ///
    /// Congruence Closure tactic: proves equalities over uninterpreted functions.
    /// Handles hypotheses via implications: (implies (Eq x y) (Eq (f x) (f y)))
    /// Computational behavior defined in reduction.rs.
    fn register_try_cc(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DccSolve : Syntax -> Derivation
        // Proof constructor for congruence closure proofs
        ctx.add_constructor(
            "DccSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_cc : Syntax -> Derivation
        // CC tactic: given a goal, try to prove it by congruence closure
        let try_cc_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_cc", try_cc_type);
    }

    /// DSimpSolve : Syntax -> Derivation
    /// try_simp : Syntax -> Derivation
    ///
    /// Simplifier tactic: proves equalities by term rewriting.
    /// Uses bottom-up rewriting with arithmetic evaluation and hypothesis substitution.
    /// Handles: reflexivity, constant folding (2+3=5), and hypothesis-based substitution.
    /// Computational behavior defined in reduction.rs.
    fn register_try_simp(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DSimpSolve : Syntax -> Derivation
        // Proof constructor for simplifier proofs
        ctx.add_constructor(
            "DSimpSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_simp : Syntax -> Derivation
        // Simp tactic: given a goal, try to prove it by simplification
        let try_simp_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_simp", try_simp_type);
    }

    /// DOmegaSolve : Syntax -> Derivation
    /// try_omega : Syntax -> Derivation
    ///
    /// Omega tactic: proves linear integer arithmetic with proper floor/ceil rounding.
    /// Unlike lia (which uses rationals), omega handles integers correctly:
    /// - x > 1 means x >= 2 (strict-to-nonstrict conversion)
    /// - 3x <= 10 means x <= 3 (floor division)
    /// Computational behavior defined in reduction.rs.
    fn register_try_omega(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DOmegaSolve : Syntax -> Derivation
        // Proof constructor for omega-solved inequalities
        ctx.add_constructor(
            "DOmegaSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_omega : Syntax -> Derivation
        // Omega tactic: given a goal, try to prove it by integer arithmetic
        let try_omega_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_omega", try_omega_type);
    }

    /// DAutoSolve : Syntax -> Derivation
    /// try_auto : Syntax -> Derivation
    ///
    /// Auto tactic: tries all decision procedures in sequence.
    /// Order: simp → ring → cc → omega → lia
    /// Returns the first successful derivation, or error if all fail.
    /// Computational behavior defined in reduction.rs.
    fn register_try_auto(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DAutoSolve : Syntax -> Derivation
        // Proof constructor for auto-solved goals
        ctx.add_constructor(
            "DAutoSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_auto : Syntax -> Derivation
        // Auto tactic: given a goal, try all tactics in sequence
        let try_auto_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax),
            body_type: Box::new(derivation),
        };

        ctx.add_declaration("try_auto", try_auto_type);
    }

    /// try_induction : Syntax -> Syntax -> Derivation -> Derivation
    ///
    /// Generic induction tactic for any inductive type.
    /// Arguments:
    /// - ind_type: The inductive type (SName "Nat" or SApp (SName "List") A)
    /// - motive: The property to prove (SLam param_type body)
    /// - cases: DCase chain with one derivation per constructor
    ///
    /// Returns a DElim derivation if verification passes.
    fn register_try_induction(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_induction : Syntax -> Syntax -> Derivation -> Derivation
        let try_induction_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(syntax.clone()), // ind_type
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // motive
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // cases
                    body_type: Box::new(derivation),
                }),
            }),
        };

        ctx.add_declaration("try_induction", try_induction_type);
    }

    /// Helper functions for building induction goals.
    ///
    /// These functions help construct the subgoals for induction:
    /// - induction_base_goal: Computes the base case goal
    /// - induction_step_goal: Computes the step case goal for a constructor
    /// - induction_num_cases: Returns number of constructors for an inductive
    fn register_induction_helpers(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let nat = Term::Global("Nat".to_string());

        // induction_base_goal : Syntax -> Syntax -> Syntax
        // Given ind_type and motive, returns the base case goal (first constructor)
        ctx.add_declaration(
            "induction_base_goal",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(syntax.clone()),
                }),
            },
        );

        // induction_step_goal : Syntax -> Syntax -> Nat -> Syntax
        // Given ind_type, motive, constructor index, returns the case goal
        ctx.add_declaration(
            "induction_step_goal",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()),
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(nat),
                        body_type: Box::new(syntax.clone()),
                    }),
                }),
            },
        );

        // induction_num_cases : Syntax -> Nat
        // Returns number of constructors for an inductive type
        ctx.add_declaration(
            "induction_num_cases",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(Term::Global("Nat".to_string())),
            },
        );
    }

    // -------------------------------------------------------------------------
    // Inversion Tactic
    // -------------------------------------------------------------------------

    /// try_inversion : Syntax -> Derivation
    ///
    /// Inversion tactic: given a hypothesis type, derives False if no constructor
    /// can possibly build that type.
    ///
    /// Example: try_inversion (SApp (SName "Even") three) proves False because
    /// neither even_zero (requires 0) nor even_succ (requires Even 1) can build Even 3.
    ///
    /// Computational behavior defined in reduction.rs.
    fn register_try_inversion_tactic(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_inversion : Syntax -> Derivation
        ctx.add_declaration(
            "try_inversion",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            },
        );
    }

    // -------------------------------------------------------------------------
    // Operator Tactics (rewrite, destruct, apply)
    // -------------------------------------------------------------------------

    /// Operator tactics for manual proof control.
    ///
    /// try_rewrite : Derivation -> Syntax -> Derivation
    /// try_rewrite_rev : Derivation -> Syntax -> Derivation
    /// try_destruct : Syntax -> Syntax -> Derivation -> Derivation
    /// try_apply : Syntax -> Derivation -> Syntax -> Derivation
    fn register_operator_tactics(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // try_rewrite : Derivation -> Syntax -> Derivation
        // Given eq_proof (concluding Eq A x y) and goal, replaces x with y in goal
        ctx.add_declaration(
            "try_rewrite",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // eq_proof
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // goal
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // try_rewrite_rev : Derivation -> Syntax -> Derivation
        // Given eq_proof (concluding Eq A x y) and goal, replaces y with x in goal (reverse direction)
        ctx.add_declaration(
            "try_rewrite_rev",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(derivation.clone()), // eq_proof
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // goal
                    body_type: Box::new(derivation.clone()),
                }),
            },
        );

        // try_destruct : Syntax -> Syntax -> Derivation -> Derivation
        // Case analysis without induction hypotheses
        ctx.add_declaration(
            "try_destruct",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // ind_type
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax.clone()), // motive
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(derivation.clone()), // cases
                        body_type: Box::new(derivation.clone()),
                    }),
                }),
            },
        );

        // try_apply : Syntax -> Derivation -> Syntax -> Derivation
        // Manual backward chaining - applies hypothesis to transform goal
        ctx.add_declaration(
            "try_apply",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()), // hyp_name
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(derivation.clone()), // hyp_proof
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(syntax), // goal
                        body_type: Box::new(derivation),
                    }),
                }),
            },
        );
    }

    // =========================================================================
    // HARDWARE TACTICS: try_bitblast, try_tabulate, try_hw_auto
    // =========================================================================

    fn register_hw_tactics(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());

        // DBitblastSolve : Syntax -> Derivation
        ctx.add_constructor(
            "DBitblastSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_bitblast : Syntax -> Derivation
        ctx.add_declaration(
            "try_bitblast",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DTabulateSolve : Syntax -> Derivation
        ctx.add_constructor(
            "DTabulateSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_tabulate : Syntax -> Derivation
        ctx.add_declaration(
            "try_tabulate",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // DHwAutoSolve : Syntax -> Derivation
        ctx.add_constructor(
            "DHwAutoSolve",
            "Derivation",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation.clone()),
            },
        );

        // try_hw_auto : Syntax -> Derivation
        ctx.add_declaration(
            "try_hw_auto",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(derivation),
            },
        );
    }

    // =========================================================================
    // HARDWARE TYPES: Bit, Unit, BVec, gate operations, Circuit, BVec ops
    // =========================================================================

    /// Register all hardware-related types and operations.
    fn register_hardware(ctx: &mut Context) {
        Self::register_bit(ctx);
        Self::register_hw_unit(ctx);
        Self::register_bvec(ctx);
        Self::register_gate_ops(ctx);
        Self::register_circuit(ctx);
        Self::register_bvec_ops(ctx);
    }

    /// Bit : Type 0 — 1-bit logic value
    /// B0 : Bit — logic low
    /// B1 : Bit — logic high
    fn register_bit(ctx: &mut Context) {
        let bit = Term::Global("Bit".to_string());
        ctx.add_inductive("Bit", Term::Sort(Universe::Type(0)));
        ctx.add_constructor("B0", "Bit", bit.clone());
        ctx.add_constructor("B1", "Bit", bit);
    }

    /// Unit : Type 0 — trivial type for stateless circuits
    /// Tt : Unit — sole inhabitant
    fn register_hw_unit(ctx: &mut Context) {
        let unit = Term::Global("Unit".to_string());
        ctx.add_inductive("Unit", Term::Sort(Universe::Type(0)));
        ctx.add_constructor("Tt", "Unit", unit);
    }

    /// BVec : Nat -> Type 0 — length-indexed bitvector
    /// BVNil : BVec Zero
    /// BVCons : Bit -> Π(n:Nat). BVec n -> BVec (Succ n)
    fn register_bvec(ctx: &mut Context) {
        let type0 = Term::Sort(Universe::Type(0));
        let nat = Term::Global("Nat".to_string());
        let n = Term::Var("n".to_string());

        // BVec : Nat -> Type 0
        let bvec_type = Term::Pi {
            param: "n".to_string(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(type0),
        };
        ctx.add_inductive("BVec", bvec_type);

        // BVNil : BVec Zero
        let bvec_zero = Term::App(
            Box::new(Term::Global("BVec".to_string())),
            Box::new(Term::Global("Zero".to_string())),
        );
        ctx.add_constructor("BVNil", "BVec", bvec_zero);

        // BVCons : Bit -> Π(n:Nat). BVec n -> BVec (Succ n)
        let bit = Term::Global("Bit".to_string());
        let bvec_n = Term::App(
            Box::new(Term::Global("BVec".to_string())),
            Box::new(n.clone()),
        );
        let bvec_succ_n = Term::App(
            Box::new(Term::Global("BVec".to_string())),
            Box::new(Term::App(
                Box::new(Term::Global("Succ".to_string())),
                Box::new(n.clone()),
            )),
        );
        let bvcons_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(bit),
            body_type: Box::new(Term::Pi {
                param: "n".to_string(),
                param_type: Box::new(nat),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(bvec_n),
                    body_type: Box::new(bvec_succ_n),
                }),
            }),
        };
        ctx.add_constructor("BVCons", "BVec", bvcons_type);
    }

    /// Gate operations as transparent definitions (unfold via iota reduction).
    ///
    /// bit_and : Bit -> Bit -> Bit — match a: B0 -> B0, B1 -> b
    /// bit_or  : Bit -> Bit -> Bit — match a: B0 -> b,  B1 -> B1
    /// bit_not : Bit -> Bit        — match a: B0 -> B1, B1 -> B0
    /// bit_xor : Bit -> Bit -> Bit — match a: B0 -> bit_not b, B1 -> b
    /// bit_mux : Bit -> Bit -> Bit -> Bit — match sel: B0 -> else, B1 -> then
    fn register_gate_ops(ctx: &mut Context) {
        let bit = Term::Global("Bit".to_string());

        // Bit -> Bit -> Bit
        let bit2_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(bit.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(bit.clone()),
                body_type: Box::new(bit.clone()),
            }),
        };

        // Bit -> Bit
        let bit1_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(bit.clone()),
            body_type: Box::new(bit.clone()),
        };

        // Bit -> Bit -> Bit -> Bit
        let bit3_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(bit.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(bit.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(bit.clone()),
                    body_type: Box::new(bit.clone()),
                }),
            }),
        };

        // Motive for match on Bit: λ(_:Bit). Bit
        let motive = Term::Lambda {
            param: "_".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(bit.clone()),
        };

        // bit_and := λ(a:Bit). λ(b:Bit). match a return (λ_:Bit. Bit) with [B0, b]
        let bit_and_body = Term::Lambda {
            param: "a".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Lambda {
                param: "b".to_string(),
                param_type: Box::new(bit.clone()),
                body: Box::new(Term::Match {
                    discriminant: Box::new(Term::Var("a".to_string())),
                    motive: Box::new(motive.clone()),
                    cases: vec![
                        Term::Global("B0".to_string()), // B0 case: return B0
                        Term::Var("b".to_string()),      // B1 case: return b
                    ],
                }),
            }),
        };
        ctx.add_definition("bit_and".to_string(), bit2_type.clone(), bit_and_body);

        // bit_or := λ(a:Bit). λ(b:Bit). match a return (λ_:Bit. Bit) with [b, B1]
        let bit_or_body = Term::Lambda {
            param: "a".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Lambda {
                param: "b".to_string(),
                param_type: Box::new(bit.clone()),
                body: Box::new(Term::Match {
                    discriminant: Box::new(Term::Var("a".to_string())),
                    motive: Box::new(motive.clone()),
                    cases: vec![
                        Term::Var("b".to_string()),      // B0 case: return b
                        Term::Global("B1".to_string()), // B1 case: return B1
                    ],
                }),
            }),
        };
        ctx.add_definition("bit_or".to_string(), bit2_type.clone(), bit_or_body);

        // bit_not := λ(a:Bit). match a return (λ_:Bit. Bit) with [B1, B0]
        let bit_not_body = Term::Lambda {
            param: "a".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Match {
                discriminant: Box::new(Term::Var("a".to_string())),
                motive: Box::new(motive.clone()),
                cases: vec![
                    Term::Global("B1".to_string()), // B0 case: return B1
                    Term::Global("B0".to_string()), // B1 case: return B0
                ],
            }),
        };
        ctx.add_definition("bit_not".to_string(), bit1_type, bit_not_body);

        // bit_xor := λ(a:Bit). λ(b:Bit). match a return (λ_:Bit. Bit) with [b, bit_not b]
        // XOR truth table: 0^0=0, 0^1=1, 1^0=1, 1^1=0
        // When a=B0: result = b (0^0=0, 0^1=1)
        // When a=B1: result = bit_not b (1^0=1, 1^1=0)
        let bit_xor_body = Term::Lambda {
            param: "a".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Lambda {
                param: "b".to_string(),
                param_type: Box::new(bit.clone()),
                body: Box::new(Term::Match {
                    discriminant: Box::new(Term::Var("a".to_string())),
                    motive: Box::new(motive.clone()),
                    cases: vec![
                        // B0 case: b
                        Term::Var("b".to_string()),
                        // B1 case: bit_not b
                        Term::App(
                            Box::new(Term::Global("bit_not".to_string())),
                            Box::new(Term::Var("b".to_string())),
                        ),
                    ],
                }),
            }),
        };
        ctx.add_definition("bit_xor".to_string(), bit2_type, bit_xor_body);

        // bit_mux := λ(sel:Bit). λ(then_v:Bit). λ(else_v:Bit).
        //            match sel return (λ_:Bit. Bit) with [else_v, then_v]
        let bit_mux_body = Term::Lambda {
            param: "sel".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Lambda {
                param: "then_v".to_string(),
                param_type: Box::new(bit.clone()),
                body: Box::new(Term::Lambda {
                    param: "else_v".to_string(),
                    param_type: Box::new(bit.clone()),
                    body: Box::new(Term::Match {
                        discriminant: Box::new(Term::Var("sel".to_string())),
                        motive: Box::new(motive),
                        cases: vec![
                            Term::Var("else_v".to_string()), // B0 case: else
                            Term::Var("then_v".to_string()), // B1 case: then
                        ],
                    }),
                }),
            }),
        };
        ctx.add_definition("bit_mux".to_string(), bit3_type, bit_mux_body);
    }

    /// Circuit : Type 0 -> Type 0 -> Type 0 -> Type 0
    /// MkCircuit : Π(S:Type 0). Π(I:Type 0). Π(O:Type 0).
    ///             (S -> I -> S) -> (S -> I -> O) -> S -> Circuit S I O
    fn register_circuit(ctx: &mut Context) {
        let type0 = Term::Sort(Universe::Type(0));
        let s = Term::Var("S".to_string());
        let i = Term::Var("I".to_string());
        let o = Term::Var("O".to_string());

        // Circuit : Type 0 -> Type 0 -> Type 0 -> Type 0
        let circuit_type = Term::Pi {
            param: "S".to_string(),
            param_type: Box::new(type0.clone()),
            body_type: Box::new(Term::Pi {
                param: "I".to_string(),
                param_type: Box::new(type0.clone()),
                body_type: Box::new(Term::Pi {
                    param: "O".to_string(),
                    param_type: Box::new(type0.clone()),
                    body_type: Box::new(type0.clone()),
                }),
            }),
        };
        ctx.add_inductive("Circuit", circuit_type);

        // Circuit S I O
        let circuit_s_i_o = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("Circuit".to_string())),
                    Box::new(s.clone()),
                )),
                Box::new(i.clone()),
            )),
            Box::new(o.clone()),
        );

        // S -> I -> S (transition function type)
        let trans_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(s.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(i.clone()),
                body_type: Box::new(s.clone()),
            }),
        };

        // S -> I -> O (output function type)
        let out_type = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(s.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(i.clone()),
                body_type: Box::new(o.clone()),
            }),
        };

        // MkCircuit : Π(S:Type0). Π(I:Type0). Π(O:Type0).
        //             (S->I->S) -> (S->I->O) -> S -> Circuit S I O
        let mkcircuit_type = Term::Pi {
            param: "S".to_string(),
            param_type: Box::new(type0.clone()),
            body_type: Box::new(Term::Pi {
                param: "I".to_string(),
                param_type: Box::new(type0.clone()),
                body_type: Box::new(Term::Pi {
                    param: "O".to_string(),
                    param_type: Box::new(type0),
                    body_type: Box::new(Term::Pi {
                        param: "_".to_string(),
                        param_type: Box::new(trans_type),
                        body_type: Box::new(Term::Pi {
                            param: "_".to_string(),
                            param_type: Box::new(out_type),
                            body_type: Box::new(Term::Pi {
                                param: "_".to_string(),
                                param_type: Box::new(s),
                                body_type: Box::new(circuit_s_i_o),
                            }),
                        }),
                    }),
                }),
            }),
        };
        ctx.add_constructor("MkCircuit", "Circuit", mkcircuit_type);
    }

    /// Bitvector operations as recursive Fix definitions.
    /// bv_and, bv_or, bv_not, bv_xor : Π(n:Nat). BVec n -> BVec n -> BVec n
    fn register_bvec_ops(ctx: &mut Context) {
        let nat = Term::Global("Nat".to_string());
        let n = Term::Var("n".to_string());
        let bvec_n = Term::App(
            Box::new(Term::Global("BVec".to_string())),
            Box::new(n.clone()),
        );

        // Π(n:Nat). BVec n -> BVec n -> BVec n
        let bv_binop_type = Term::Pi {
            param: "n".to_string(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(bvec_n.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(bvec_n.clone()),
                    body_type: Box::new(bvec_n.clone()),
                }),
            }),
        };

        // Π(n:Nat). BVec n -> BVec n
        let bv_unop_type = Term::Pi {
            param: "n".to_string(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(bvec_n.clone()),
                body_type: Box::new(bvec_n.clone()),
            }),
        };

        // Helper: build BVec m
        let bvec_of = |m: Term| -> Term {
            Term::App(Box::new(Term::Global("BVec".to_string())), Box::new(m))
        };

        // Motive for Match on BVec n: λ(_:BVec n). BVec n
        // (In practice, the motive parameter is unused, so a simple identity type works)
        let motive_n = Term::Lambda {
            param: "_".to_string(),
            param_type: Box::new(bvec_n.clone()),
            body: Box::new(bvec_n.clone()),
        };

        // --- bv_and ---
        // fix bv_and_rec. λ(n:Nat). λ(v1:BVec n). λ(v2:BVec n).
        //   match v1 with
        //   | BVNil => BVNil
        //   | BVCons => λ(b1:Bit). λ(m:Nat). λ(tail1:BVec m).
        //       match v2 with
        //       | BVNil => BVNil
        //       | BVCons => λ(b2:Bit). λ(_:Nat). λ(tail2:BVec m).
        //           BVCons (bit_and b1 b2) m (bv_and_rec m tail1 tail2)
        let bit = Term::Global("Bit".to_string());

        let bv_and_body = Self::make_bvec_binop_fix(
            "bv_and_rec", "bit_and", &nat, &bvec_n, &bit, &motive_n,
        );
        ctx.add_definition("bv_and".to_string(), bv_binop_type.clone(), bv_and_body);

        // --- bv_or ---
        let bv_or_body = Self::make_bvec_binop_fix(
            "bv_or_rec", "bit_or", &nat, &bvec_n, &bit, &motive_n,
        );
        ctx.add_definition("bv_or".to_string(), bv_binop_type.clone(), bv_or_body);

        // --- bv_xor ---
        let bv_xor_body = Self::make_bvec_binop_fix(
            "bv_xor_rec", "bit_xor", &nat, &bvec_n, &bit, &motive_n,
        );
        ctx.add_definition("bv_xor".to_string(), bv_binop_type, bv_xor_body);

        // --- bv_not ---
        // fix bv_not_rec. λ(n:Nat). λ(v:BVec n).
        //   match v with
        //   | BVNil => BVNil
        //   | BVCons => λ(b:Bit). λ(m:Nat). λ(tail:BVec m).
        //       BVCons (bit_not b) m (bv_not_rec m tail)
        let bv_not_body = Self::make_bvec_unop_fix(
            "bv_not_rec", "bit_not", &nat, &bvec_n, &bit, &motive_n,
        );
        ctx.add_definition("bv_not".to_string(), bv_unop_type, bv_not_body);
    }

    /// Build a Fix term for a binary BVec operation (bv_and, bv_or, bv_xor).
    ///
    /// Pattern: fix rec. λn. λv1. λv2. match v1 { BVNil => BVNil, BVCons b1 m t1 => match v2 { BVNil => BVNil, BVCons b2 _ t2 => BVCons (bit_op b1 b2) m (rec m t1 t2) } }
    fn make_bvec_binop_fix(
        rec_name: &str,
        bit_op: &str,
        nat: &Term,
        bvec_n: &Term,
        bit: &Term,
        motive: &Term,
    ) -> Term {
        let m_var = Term::Var("m".to_string());
        let bvec_m = Term::App(Box::new(Term::Global("BVec".to_string())), Box::new(m_var.clone()));
        let motive_m = Term::Lambda {
            param: "_".to_string(),
            param_type: Box::new(bvec_m.clone()),
            body: Box::new(bvec_m.clone()),
        };

        // Innermost: BVCons (bit_op b1 b2) m (rec m tail1 tail2)
        let bit_op_applied = Term::App(
            Box::new(Term::App(
                Box::new(Term::Global(bit_op.to_string())),
                Box::new(Term::Var("b1".to_string())),
            )),
            Box::new(Term::Var("b2".to_string())),
        );
        let rec_call = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Var(rec_name.to_string())),
                    Box::new(m_var.clone()),
                )),
                Box::new(Term::Var("tail1".to_string())),
            )),
            Box::new(Term::Var("tail2".to_string())),
        );
        let bvcons_result = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("BVCons".to_string())),
                    Box::new(bit_op_applied),
                )),
                Box::new(m_var.clone()),
            )),
            Box::new(rec_call),
        );

        // Inner match on v2 (BVCons case for v1)
        let inner_bvcons_case = Term::Lambda {
            param: "b2".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Lambda {
                param: "_m2".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::Lambda {
                    param: "tail2".to_string(),
                    param_type: Box::new(bvec_m.clone()),
                    body: Box::new(bvcons_result),
                }),
            }),
        };

        let inner_match = Term::Match {
            discriminant: Box::new(Term::Var("v2".to_string())),
            motive: Box::new(motive_m.clone()),
            cases: vec![
                Term::Global("BVNil".to_string()), // BVNil case
                inner_bvcons_case,                  // BVCons case
            ],
        };

        // Outer BVCons case for v1
        let outer_bvcons_case = Term::Lambda {
            param: "b1".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Lambda {
                param: "m".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::Lambda {
                    param: "tail1".to_string(),
                    param_type: Box::new(bvec_m.clone()),
                    body: Box::new(inner_match),
                }),
            }),
        };

        // Outer match on v1
        let outer_match = Term::Match {
            discriminant: Box::new(Term::Var("v1".to_string())),
            motive: Box::new(motive.clone()),
            cases: vec![
                Term::Global("BVNil".to_string()), // BVNil case
                outer_bvcons_case,                  // BVCons case
            ],
        };

        // Fix + lambdas
        Term::Fix {
            name: rec_name.to_string(),
            body: Box::new(Term::Lambda {
                param: "n".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::Lambda {
                    param: "v1".to_string(),
                    param_type: Box::new(bvec_n.clone()),
                    body: Box::new(Term::Lambda {
                        param: "v2".to_string(),
                        param_type: Box::new(bvec_n.clone()),
                        body: Box::new(outer_match),
                    }),
                }),
            }),
        }
    }

    /// Build a Fix term for a unary BVec operation (bv_not).
    ///
    /// Pattern: fix rec. λn. λv. match v { BVNil => BVNil, BVCons b m t => BVCons (bit_op b) m (rec m t) }
    fn make_bvec_unop_fix(
        rec_name: &str,
        bit_op: &str,
        nat: &Term,
        bvec_n: &Term,
        bit: &Term,
        motive: &Term,
    ) -> Term {
        let m_var = Term::Var("m".to_string());
        let bvec_m = Term::App(Box::new(Term::Global("BVec".to_string())), Box::new(m_var.clone()));

        // BVCons (bit_op b) m (rec m tail)
        let bit_op_applied = Term::App(
            Box::new(Term::Global(bit_op.to_string())),
            Box::new(Term::Var("b".to_string())),
        );
        let rec_call = Term::App(
            Box::new(Term::App(
                Box::new(Term::Var(rec_name.to_string())),
                Box::new(m_var.clone()),
            )),
            Box::new(Term::Var("tail".to_string())),
        );
        let bvcons_result = Term::App(
            Box::new(Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global("BVCons".to_string())),
                    Box::new(bit_op_applied),
                )),
                Box::new(m_var.clone()),
            )),
            Box::new(rec_call),
        );

        // BVCons case
        let bvcons_case = Term::Lambda {
            param: "b".to_string(),
            param_type: Box::new(bit.clone()),
            body: Box::new(Term::Lambda {
                param: "m".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::Lambda {
                    param: "tail".to_string(),
                    param_type: Box::new(bvec_m),
                    body: Box::new(bvcons_result),
                }),
            }),
        };

        // Match on v
        let match_expr = Term::Match {
            discriminant: Box::new(Term::Var("v".to_string())),
            motive: Box::new(motive.clone()),
            cases: vec![
                Term::Global("BVNil".to_string()), // BVNil case
                bvcons_case,                        // BVCons case
            ],
        };

        // Fix + lambdas
        Term::Fix {
            name: rec_name.to_string(),
            body: Box::new(Term::Lambda {
                param: "n".to_string(),
                param_type: Box::new(nat.clone()),
                body: Box::new(Term::Lambda {
                    param: "v".to_string(),
                    param_type: Box::new(bvec_n.clone()),
                    body: Box::new(match_expr),
                }),
            }),
        }
    }
}
