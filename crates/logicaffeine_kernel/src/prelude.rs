//! Standard Library for the Kernel.
//!
//! Defines fundamental types and logical connectives:
//! - Entity: domain of individuals (for FOL)
//! - Nat: natural numbers
//! - True, False: propositional constants
//! - Eq: propositional equality
//! - And, Or: logical connectives

pub(crate) mod natural_arithmetic;

use crate::context::Context;
use crate::term::{Literal, Term, Universe};

/// Standard library definitions.
pub struct StandardLibrary;

#[cfg(feature = "shadow")]
thread_local! {
    /// How deep this thread is inside [`StandardLibrary::register`].
    static REGISTERING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// This thread is registering the standard library while one lives.
#[cfg(feature = "shadow")]
struct Registering;

#[cfg(feature = "shadow")]
impl Registering {
    fn enter() -> Registering {
        REGISTERING.with(|depth| depth.set(depth.get() + 1));
        Registering
    }
}

#[cfg(feature = "shadow")]
impl Drop for Registering {
    fn drop(&mut self) {
        REGISTERING.with(|depth| depth.set(depth.get() - 1));
    }
}

/// Whether this thread is registering the standard library (shadow mode stands down meanwhile).
#[cfg(feature = "shadow")]
pub(crate) fn registering() -> bool {
    REGISTERING.with(|depth| depth.get() > 0)
}

/// The standard library's one registration ([`StandardLibrary::context`]).
static PRELUDE: std::sync::OnceLock<Context> = std::sync::OnceLock::new();

impl StandardLibrary {
    /// Register all standard library definitions in the context.
    ///
    /// A thousand checked declarations and their checked equation lemmas: production code asks
    /// the process's one registration, [`StandardLibrary::context`], instead.
    pub fn register(ctx: &mut Context) {
        #[cfg(feature = "shadow")]
        let _registering = Registering::enter();
        ctx.with_equations_suspended(Self::register_core);
        crate::equations::generate_missing_for_all(ctx);
    }

    /// The standard library's context, registered once per process.
    ///
    /// A registered `Context` is `Arc`-backed and copy-on-write throughout (every mutating
    /// accessor goes through `Arc::make_mut`), so a caller that only asks questions borrows this
    /// one, and a caller that extends it clones it: about a microsecond, sharing every table
    /// until the clone writes to it, against the ~0.3 s (optimized) of registering again. The
    /// clone is an independent value; the attestation caches it shares are keyed by the registry
    /// generation they inspected, so a clone that diverges never reads another's answer.
    /// `the_cached_prelude_is_indistinguishable_from_a_fresh_one` pins it against a fresh
    /// registration.
    pub fn context() -> &'static Context {
        PRELUDE.get_or_init(|| {
            let mut ctx = Context::new();
            Self::register(&mut ctx);
            ctx
        })
    }

    /// [`StandardLibrary::context`] if it is registered already: no context descends from the
    /// standard library before it is.
    pub(crate) fn registered() -> Option<&'static Context> {
        PRELUDE.get()
    }

    fn register_core(ctx: &mut Context) {
        Self::register_entity(ctx);
        Self::register_nat(ctx);
        Self::register_monolist(ctx);
        Self::register_bool(ctx);
        Self::register_tlist(ctx);
        Self::register_true(ctx);
        Self::register_false(ctx);
        Self::register_not(ctx);
        Self::register_eq(ctx);
        Self::register_heq(ctx);
        Self::register_and(ctx);
        Self::register_or(ctx);
        Self::register_ex(ctx);
        Self::register_decidable(ctx);
        Self::register_of_decide(ctx);
        Self::register_dec_eq_bool(ctx);
        Self::register_dec_eq_nat(ctx);
        Self::register_decidable_instances(ctx);
        Self::register_native_decide(ctx);
        Self::register_psigma(ctx);
        Self::register_quot(ctx);
        Self::register_setoid(ctx);
        Self::register_acc(ctx);
        Self::register_primitives(ctx);
        Self::register_int_ring_axioms(ctx);
        Self::register_int_order_axioms(ctx);
        Self::register_reflection(ctx);
        Self::register_hardware(ctx);
        Self::register_core_types(ctx);
        natural_arithmetic::register(ctx);
        crate::int_inductive::register_inductive_int(ctx)
            .expect("checked structural integer prelude");
        crate::int_inductive::register_public_arithmetic_aliases(ctx)
            .expect("checked public integer arithmetic aliases");
        crate::int_inductive::register_public_division_aliases(ctx)
            .expect("checked public integer division aliases");
        crate::integer_laws::register(ctx).expect("checked constructor-derived integer laws");
        Self::register_extensionality_and_choice(ctx);
    }

    /// **Lean's other core constants, and everything that derives from them.**
    ///
    /// The four constants a Lean development actually reaches for are `propext`, `funext`,
    /// `Classical.choice` and `Quot.sound` — but Lean's trusted base is only THREE of them, and
    /// that distinction is the whole design of this function. LOGOS had `Quot.sound` and `dne` and
    /// none of the rest, which is a STATEABILITY gap, not a convenience one: without `propext`
    /// there is no `Set`, no quotient extensionality and no `simp` on `↔`; without `funext` there
    /// is no `ext` lemma and no equality of `Set`/`Finset`/`Matrix`; without `Classical.choice`
    /// almost every Mathlib theorem loses its `#print axioms` line. A translator that meets one of
    /// them can only give up or forge, and forging is what rule 6 of the campaign exists to
    /// prevent.
    ///
    /// **TWO axioms, and not one more.** `funext` is a THEOREM here, proved from `Quot_sound`
    /// exactly as Lean proves it — an earlier wave assumed it, which bought at permanent cost to
    /// the trusted base something the reference implementation gets for free. Everything else in
    /// this function is likewise a DEFINITION with a kernel-checked body, so the trusted base
    /// grows by exactly the two assumptions Lean also makes and by nothing that Lean proves:
    ///
    /// | constant | kind | why |
    /// |---|---|---|
    /// | `propext` | axiom | Lean's; unprovable — `Prop` has no extensionality rule |
    /// | `Classical_choice` | axiom | Lean's; `Nonempty α → α` extracts data from a proof |
    /// | `funext` | **definition** | Lean PROVES it from `Quot.sound`; so do we (see below) |
    /// | `eq_true`, `eq_false` | definitions | `propext`'s canonical users — `simp`'s Prop normal form |
    /// | `nonemptyNat` | definition | `Nonempty_intro Nat Zero` |
    /// | `Or_rec`, `Ex_rec` | definitions | auto-derived eliminators |
    /// | `Classical_em` | definition | the standard intuitionistic derivation from `dne` |
    /// | `Classical_propDecidable` | definition | `dne` + `choice`, NO `em` needed (see below) |
    /// | `Classical_axiomOfChoice` | definition | pointwise `choice` through `Subtype` |
    ///
    /// **Why `Classical_propDecidable` does not go through `em`.** The textbook route is
    /// `em p : p ∨ ¬p`, then `Or_rec` into `Nonempty (Decidable p)`, then `choice`. The shorter
    /// route needs no disjunction at all: `Nonempty (Decidable p)` is itself a `Prop`, so `dne`
    /// applies to it directly, and the double-negation argument builds `isTrue`/`isFalse` from the
    /// SAME refutation `k`. Fewer moving parts, one fewer eliminator in the proof term, and it
    /// makes plain that the classical content is `choice` — `em` is not on the critical path.
    ///
    /// **The firewall stays shut.** `Classical_propDecidable` is what MAKES data out of a
    /// proposition, and it does so only by applying `Classical_choice`. Eliminating `p ∨ ¬p` into
    /// `Decidable p` is still refused, because `Or` has two constructors and `motive_target` keeps
    /// its motive in `Prop` — keyed on the MOTIVE'S SORT, never on a `Classical.` name prefix. So
    /// the new capability arrives as a term you apply, not as a loosened eliminator.
    fn register_extensionality_and_choice(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let prop = || Term::Sort(Universe::Prop);
        let eq3 = |t: Term, x: Term, y: Term| apps_(gl("Eq"), &[t, x, y]);

        // ---- propext : Π(p q : Prop). Iff p q → Eq Prop p q  (AXIOM) ----------------------
        //
        // The conclusion is an equation between PROPOSITIONS, and it is already expressible:
        // `Prop.succ() == Type 0`, so `Sort Prop : Type 0` and `Eq`'s `Type 0` carrier accepts it.
        // `Iff` is a DEFINITION unfolding to the conjunction, so the hypothesis is the `And` a
        // translator already has — a fresh `Iff` inductive would be a different proposition that
        // `propext` itself could not bridge back.
        ctx.add_declaration(
            "propext",
            pi_(
                "p",
                prop(),
                pi_(
                    "q",
                    prop(),
                    arrow_(
                        apps_(gl("Iff"), &[v_("p"), v_("q")]),
                        eq3(prop(), v_("p"), v_("q")),
                    ),
                ),
            ),
        );

        // ---- funext, in its DEPENDENT form — a THEOREM, proved from `Quot_sound` ----------
        //
        //   Π(A:Type). Π(B:A→Type). Π(f g : Π(x:A). B x).
        //     (Π(x:A). Eq (B x) (f x) (g x)) → Eq (Π(x:A). B x) f g
        //
        // **Lean does not assume this, and neither do we.** `funext` is one of the four constants
        // a Lean user reaches for, but it is NOT one of Lean's three axioms: Lean PROVES it from
        // `Quot.sound`. Assuming it would have bought — at a permanent cost to the trusted base —
        // something the reference implementation gets for free, and the trusted base is the first
        // number a Lean reader checks.
        //
        // THE ARGUMENT, and it is Lean's. Write `F := Π(x:A). B x`, and let `eqv` be POINTWISE
        // equality on `F`. That is a relation, not an equality, so quotienting by it is legal and
        // assumes nothing. At each point `x`, lift the single application `λu. u x` through
        // `Quot F eqv`: its respect obligation is `eqv u w → Eq (B x) (u x) (w x)`, which the
        // hypothesis supplies POINTWISE — no extensionality is needed to build it. That gives
        //
        //     ext : Quot F eqv → F        with     ext (Quot_mk F eqv u)  ≡  λx. u x  ≡  u
        //
        // — the first step by the quotient ι-rule, the second by η, which `def_eq` decides and
        // `normalize` does not (exactly the asymmetry T00-014 pins). Now `Quot_sound F eqv f g h`
        // proves `Eq (Quot F eqv) (mk f) (mk g)`, and substituting along it in the motive
        // `λq. Eq F f (ext q)` — `Eq_rec`, the Prop-motive substitution eliminator, itself derived
        // from J — carries `Eq F f (ext (mk f))`, which is `refl F f` by the conversion above, to
        // `Eq F f (ext (mk g))`, which IS `Eq F f g` by the same conversion.
        //
        // So extensionality of functions follows from the ONE thing quotients already assume: that
        // `Quot_sound` identifies related representatives. The η step is load-bearing and is why
        // this is a proof rather than a restatement — without η, `ext (mk u)` would only ever be
        // `λx. u x`, and the theorem would not close.
        //
        // It remains a theorem you APPLY, never a conversion rule the kernel silently gained:
        // `def_eq`'s `Lambda`/`Lambda` arm stays α-equivalence, so `λn. addz n` and `λn. n` are
        // still not convertible. A monomorphic `Nat → Nat` version would be strictly weaker at the
        // same length; the dependent form is the one `ext`, `Set` and `Matrix` need.
        let b_at = |x: Term| Term::App(Box::new(v_("B")), Box::new(x));
        let dep_fn = || pi_("x", v_("A"), b_at(v_("x")));
        let at = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        // `eqv := λ(u w : F). Π(pt:A). Eq (B pt) (u pt) (w pt)` — pointwise equality, as a
        // RELATION. Every binder below is uniquely named: the kernel is name-keyed, and a shared
        // spelling between this relation's binder and the point it is later instantiated at is
        // exactly the capture a named kernel has to freshen around.
        let pointwise_named = |u: &str, w: &str, pt: &str| {
            pi_(
                pt,
                v_("A"),
                eq3(b_at(v_(pt)), at(v_(u), v_(pt)), at(v_(w), v_(pt))),
            )
        };
        let eqv = || {
            lam_(
                "u",
                dep_fn(),
                lam_("w", dep_fn(), pointwise_named("u", "w", "pt")),
            )
        };
        let quot_f = || apps_(gl("Quot"), &[dep_fn(), eqv()]);
        let mk_class = |u: Term| apps_(gl("Quot_mk"), &[dep_fn(), eqv(), u]);
        // `ext := λ(q : Quot F eqv). λ(ix:A). Quot_lift F eqv (B ix) (λu2. u2 ix) (λa2 b2 hab.
        //          hab ix) q`
        let ext = || {
            lam_(
                "q",
                quot_f(),
                lam_(
                    "ix",
                    v_("A"),
                    apps_(
                        gl("Quot_lift"),
                        &[
                            dep_fn(),
                            eqv(),
                            b_at(v_("ix")),
                            lam_("u2", dep_fn(), at(v_("u2"), v_("ix"))),
                            lam_(
                                "a2",
                                dep_fn(),
                                lam_(
                                    "b2",
                                    dep_fn(),
                                    lam_(
                                        "hab",
                                        pointwise_named("a2", "b2", "pt2"),
                                        at(v_("hab"), v_("ix")),
                                    ),
                                ),
                            ),
                            v_("q"),
                        ],
                    ),
                ),
            )
        };
        // `Eq_rec (Quot F eqv) (mk f) (λq2. Eq F f (ext q2)) (refl F f) (mk g)
        //         (Quot_sound F eqv f g h)`
        let funext_body = lam_(
            "A",
            type0(),
            lam_(
                "B",
                pi_("_x", v_("A"), type0()),
                lam_(
                    "f",
                    dep_fn(),
                    lam_(
                        "g",
                        dep_fn(),
                        lam_(
                            "h",
                            pointwise_named("f", "g", "pt3"),
                            apps_(
                                gl("Eq_rec"),
                                &[
                                    quot_f(),
                                    mk_class(v_("f")),
                                    lam_(
                                        "q2",
                                        quot_f(),
                                        eq3(dep_fn(), v_("f"), at(ext(), v_("q2"))),
                                    ),
                                    apps_(gl("refl"), &[dep_fn(), v_("f")]),
                                    mk_class(v_("g")),
                                    apps_(
                                        gl("Quot_sound"),
                                        &[dep_fn(), eqv(), v_("f"), v_("g"), v_("h")],
                                    ),
                                ],
                            ),
                        ),
                    ),
                ),
            ),
        );
        ctx.add_definition(
            "funext".to_string(),
            pi_(
                "A",
                type0(),
                pi_(
                    "B",
                    pi_("_x", v_("A"), type0()),
                    pi_(
                        "f",
                        dep_fn(),
                        pi_(
                            "g",
                            dep_fn(),
                            arrow_(
                                pointwise_named("f", "g", "pt3"),
                                eq3(dep_fn(), v_("f"), v_("g")),
                            ),
                        ),
                    ),
                ),
            ),
            funext_body,
        );

        // ---- Classical_choice : Π(A:Type). Nonempty A → A  (AXIOM) ------------------------
        //
        // `Nonempty α` is a PROPOSITION and `α` is data; turning one into the other is exactly
        // the content of choice, and it is why `Nonempty` is registered at `Prop` with a
        // data-carrying constructor — that shape is NOT a subsingleton, so the kernel refuses to
        // large-eliminate it and the only route to a witness is this declared axiom. Deriving it
        // instead (by placing `Nonempty` in `Type 0`) would be choice that the axiom census could
        // never see.
        ctx.add_declaration(
            "Classical_choice",
            pi_(
                "A",
                type0(),
                arrow_(
                    Term::App(Box::new(gl("Nonempty")), Box::new(v_("A"))),
                    v_("A"),
                ),
            ),
        );

        // ---- from here on: DEFINITIONS ONLY. ---------------------------------------------

        // `eq_true` and `eq_false` — `propext`'s two canonical users, and Lean's own
        // (`eq_true`, `eq_false`). They are here because an axiom nothing cites is an axiom
        // nothing can REPORT: `axiom_dependencies` computes the closure of a term, so `propext`
        // becomes visible to `Print Axioms` only once something in the library rests on it.
        //
        // They are also the proposition normal form `simp` works in — `p` becomes `p = True` and
        // `¬p` becomes `p = False` — which is the shape that makes propositional rewriting a
        // rewrite at all. Both cost the trusted base NOTHING: each is one application of
        // `propext` to an `Iff` built from `conj`.
        //
        //   eq_true  : Π(p:Prop). p     → Eq Prop p True
        //            := λp hp. propext p True  (conj (p → True) (True → p) (λ_. I)  (λ_. hp))
        //   eq_false : Π(p:Prop). Not p → Eq Prop p False
        //            := λp hn. propext p False (conj (p → False) (False → p) hn (λh. absurd h))
        //
        // `Iff p q` is a DEFINITION unfolding to `And (p→q) (q→p)`, so `conj` at those two arrow
        // types IS the biconditional — no `Iff` constructor exists to reach for, deliberately.
        let iff_via_conj = |q: Term, fwd: Term, bwd: Term| {
            apps_(
                gl("conj"),
                &[arrow_(v_("p"), q.clone()), arrow_(q, v_("p")), fwd, bwd],
            )
        };
        ctx.add_definition(
            "eq_true".to_string(),
            pi_(
                "p",
                prop(),
                arrow_(v_("p"), eq3(prop(), v_("p"), gl("True"))),
            ),
            lam_(
                "p",
                prop(),
                lam_(
                    "hp",
                    v_("p"),
                    apps_(
                        gl("propext"),
                        &[
                            v_("p"),
                            gl("True"),
                            iff_via_conj(
                                gl("True"),
                                lam_("_", v_("p"), gl("I")),
                                lam_("_", gl("True"), v_("hp")),
                            ),
                        ],
                    ),
                ),
            ),
        );
        // `False`'s eliminator into `Prop` is a zero-case `Match`: small elimination is
        // unconditional, so no `False_rec` constant is needed to cross back.
        let from_false = lam_(
            "hf",
            gl("False"),
            Term::Match {
                discriminant: Box::new(v_("hf")),
                motive: Box::new(lam_("_", gl("False"), v_("p"))),
                cases: vec![],
            },
        );
        ctx.add_definition(
            "eq_false".to_string(),
            pi_(
                "p",
                prop(),
                arrow_(
                    Term::App(Box::new(gl("Not")), Box::new(v_("p"))),
                    eq3(prop(), v_("p"), gl("False")),
                ),
            ),
            lam_(
                "p",
                prop(),
                lam_(
                    "hn",
                    Term::App(Box::new(gl("Not")), Box::new(v_("p"))),
                    apps_(
                        gl("propext"),
                        &[
                            v_("p"),
                            gl("False"),
                            iff_via_conj(gl("False"), v_("hn"), from_false),
                        ],
                    ),
                ),
            ),
        );

        // nonemptyNat : Nonempty Nat := Nonempty_intro Nat Zero
        ctx.add_definition(
            "nonemptyNat".to_string(),
            Term::App(Box::new(gl("Nonempty")), Box::new(gl("Nat"))),
            apps_(gl("Nonempty_intro"), &[gl("Nat"), gl("Zero")]),
        );

        // The two eliminators the classical layer needs, auto-derived. Both keep their `Prop`
        // motive: `Or` has two constructors and `Ex` stores a witness that is not among its
        // indices, so neither is a subsingleton and neither may reach `Type`. That restriction is
        // the firewall, and `Classical_choice` is the declared, visible way through it.
        for ind in ["Or", "Ex"] {
            let (ty, body) = crate::recursor::derive_recursor(ctx, ind)
                .unwrap_or_else(|e| panic!("`{ind}`'s eliminator must derive: {e:?}"));
            ctx.add_definition(format!("{ind}_rec"), ty, body);
        }

        // ═══ THE CLASSICAL LAYER, DERIVED — Lean's `Init/Classical.lean`, ported ═══════════
        //
        // Lean's trusted base is THREE axioms and excluded middle is not one of them:
        // `Classical.em` is **Diaconescu's theorem**, proved from `Classical.choice`, `funext`
        // and `propext` at `Init/Classical.lean:32-63`, and `Classical.byContradiction` — our
        // `dne` — is two further steps downstream of it (`Init/Classical.lean:131`, through
        // `propDecidable` at :75). LOGOS used to ASSUME `dne`: the same classical content,
        // bought instead of proved, standing as a fourth foundational assumption exactly where
        // the reference implementation has a proof.
        //
        // It is a proof here too, and it runs in Lean's own order:
        //
        //     Classical_choice ─▶ indefiniteDescription ─▶ choose / choose_spec
        //                                  │
        //     funext + propext ────────────┴─▶ em (Diaconescu) ─▶ propDecidable ─▶ dne
        //
        // so `dne` is the LAST link rather than the first, and the FOUNDATIONAL fragment of the
        // trusted base is now exactly `propext`, `Classical_choice` and `Quot_sound` — Lean's
        // three, under LOGOS spellings, and not one more.
        //
        // The firewall is unchanged and is still structural: `Or` has two constructors, so
        // `Or_rec`'s motive stays in `Prop` and `p ∨ ¬p` still cannot be eliminated into
        // `Decidable p`. `propDecidable` crosses `Prop → Type` by APPLYING `Classical_choice`,
        // which is the one declared, visible way through, and it is now the only one: nothing
        // below reaches for a second.

        // ---- Classical.indefiniteDescription (Init/Classical.lean:15) ---------------------
        //
        //   indefiniteDescription : Π(A:Type). Π(p:A→Prop). Ex A p → Subtype A p
        //     := λA p h. Classical_choice (Subtype A p)
        //                  (Ex_rec A p (λ_. Nonempty (Subtype A p))
        //                              (λw hw. Nonempty_intro (Subtype A p) (Subtype_mk A p w hw))
        //                              h)
        //
        // `Ex_rec` at a `Prop` motive is SMALL elimination and needs no axiom; the `Prop → Type`
        // step is `Classical_choice`, applied once, in the open.
        let pred_ty = || arrow_(v_("A"), prop());
        let sub_ap = || apps_(gl("Subtype"), &[v_("A"), v_("p")]);
        let ex_ap = || apps_(gl("Ex"), &[v_("A"), v_("p")]);
        ctx.add_definition(
            "Classical_indefiniteDescription".to_string(),
            pi_("A", type0(), pi_("p", pred_ty(), arrow_(ex_ap(), sub_ap()))),
            lam_(
                "A",
                type0(),
                lam_(
                    "p",
                    pred_ty(),
                    lam_(
                        "hex",
                        ex_ap(),
                        apps_(
                            gl("Classical_choice"),
                            &[
                                sub_ap(),
                                apps_(
                                    gl("Ex_rec"),
                                    &[
                                        v_("A"),
                                        v_("p"),
                                        lam_(
                                            "_e",
                                            ex_ap(),
                                            Term::App(Box::new(gl("Nonempty")), Box::new(sub_ap())),
                                        ),
                                        lam_(
                                            "w",
                                            v_("A"),
                                            lam_(
                                                "hw",
                                                Term::App(Box::new(v_("p")), Box::new(v_("w"))),
                                                apps_(
                                                    gl("Nonempty_intro"),
                                                    &[
                                                        sub_ap(),
                                                        apps_(
                                                            gl("Subtype_mk"),
                                                            &[v_("A"), v_("p"), v_("w"), v_("hw")],
                                                        ),
                                                    ],
                                                ),
                                            ),
                                        ),
                                        v_("hex"),
                                    ],
                                ),
                            ],
                        ),
                    ),
                ),
            ),
        );

        // ---- Classical.choose / choose_spec (Init/Classical.lean:25, :28) -----------------
        //
        // The `val`/`property` projections of the subtype `indefiniteDescription` picked. They
        // COMPUTE: both are structure projections over a constructor form once the existence
        // proof is concrete, which is why `choose` is a function and not a name.
        let indef_at = || {
            apps_(
                gl("Classical_indefiniteDescription"),
                &[v_("A"), v_("p"), v_("hex")],
            )
        };
        ctx.add_definition(
            "Classical_choose".to_string(),
            pi_("A", type0(), pi_("p", pred_ty(), arrow_(ex_ap(), v_("A")))),
            lam_(
                "A",
                type0(),
                lam_(
                    "p",
                    pred_ty(),
                    lam_(
                        "hex",
                        ex_ap(),
                        apps_(gl("Subtype_val"), &[v_("A"), v_("p"), indef_at()]),
                    ),
                ),
            ),
        );
        ctx.add_definition(
            "Classical_choose_spec".to_string(),
            pi_(
                "A",
                type0(),
                pi_(
                    "p",
                    pred_ty(),
                    pi_(
                        "hex",
                        ex_ap(),
                        Term::App(
                            Box::new(v_("p")),
                            Box::new(apps_(
                                gl("Classical_choose"),
                                &[v_("A"), v_("p"), v_("hex")],
                            )),
                        ),
                    ),
                ),
            ),
            lam_(
                "A",
                type0(),
                lam_(
                    "p",
                    pred_ty(),
                    lam_(
                        "hex",
                        ex_ap(),
                        apps_(gl("Subtype_property"), &[v_("A"), v_("p"), indef_at()]),
                    ),
                ),
            ),
        );

        // ---- Classical.em — DIACONESCU'S THEOREM (Init/Classical.lean:32-63) --------------
        //
        // Lean's proof, term for term. Two predicates on `Prop`,
        //
        //     U x := (x = True)  ∨ P          V x := (x = False) ∨ P
        //
        // are each inhabited (`True` witnesses `U`, `False` witnesses `V`), so `choose` picks
        // `u := choose U` and `v := choose V` and `choose_spec` returns `U u` and `V v`.
        //
        //   * If either disjunct lands on the right, `P` holds outright.
        //   * If both land on the left then `u = True` and `v = False`, so `u ≠ v` — the one
        //     place `True ≠ False` is needed, and it is PROVED by transporting `I : True` along
        //     `Eq Prop True False` into `False`, not assumed.
        //   * And `P → u = v`: under `hp : P`, every `U x` and `V x` is inhabited by `Or.inr hp`
        //     in both directions, so `propext` makes them the SAME proposition pointwise and
        //     `funext` makes `U` and `V` the same FUNCTION. Substituting along that equality
        //     carries `choose U e₁ = choose U e₂` — which is `refl`, because the two existence
        //     proofs are proofs of one proposition and the kernel is proof-irrelevant — to
        //     `choose U exU = choose V exV`, i.e. `u = v`.
        //
        // Contraposing the two gives `¬P` in the first case and `P` in the second.
        //
        // NOTHING here is assumed. `propext` and `Classical_choice` are Lean's own axioms;
        // `funext` is the theorem proved above from `Quot_sound`.
        let eq_prop = |x: Term, y: Term| eq3(prop(), x, y);
        let p_prop = || v_("P");
        let not_p_of = || Term::App(Box::new(gl("Not")), Box::new(v_("P")));
        // U := λ(x:Prop). Or (Eq Prop x True) P   /   V := λ(x:Prop). Or (Eq Prop x False) P
        let side = |anchor: &'static str| {
            lam_(
                "x",
                prop(),
                apps_(gl("Or"), &[eq_prop(v_("x"), gl(anchor)), p_prop()]),
            )
        };
        let u_pred = || side("True");
        let v_pred = || side("False");
        // exU : Ex Prop U := witness Prop U True (left (Eq Prop True True) P (refl Prop True))
        let ex_side = |anchor: &'static str, pred: Term| {
            apps_(
                gl("witness"),
                &[
                    prop(),
                    pred,
                    gl(anchor),
                    apps_(
                        gl("left"),
                        &[
                            eq_prop(gl(anchor), gl(anchor)),
                            p_prop(),
                            apps_(gl("refl"), &[prop(), gl(anchor)]),
                        ],
                    ),
                ],
            )
        };
        let ex_u = || ex_side("True", u_pred());
        let ex_v = || ex_side("False", v_pred());
        let chosen = |pred: Term, ex: Term| apps_(gl("Classical_choose"), &[prop(), pred, ex]);
        let u_of = || chosen(u_pred(), ex_u());
        let v_of = || chosen(v_pred(), ex_v());
        let spec = |pred: Term, ex: Term| apps_(gl("Classical_choose_spec"), &[prop(), pred, ex]);
        let not_uv = || Term::App(Box::new(gl("Not")), Box::new(eq_prop(u_of(), v_of())));
        let disj = || apps_(gl("Or"), &[not_uv(), p_prop()]);

        // `True = False → False`, by transport. `Eq_rec_dep`'s motive lands in `Prop` by
        // cumulativity, and `(λ(y:Prop). λ_. y) True refl ≡ True` is the `I` slot.
        let true_ne_false = |witness_eq: Term| {
            apps_(
                gl("Eq_rec_dep"),
                &[
                    prop(),
                    gl("True"),
                    lam_(
                        "y",
                        prop(),
                        lam_("_hy", eq_prop(gl("True"), v_("y")), v_("y")),
                    ),
                    gl("I"),
                    gl("False"),
                    witness_eq,
                ],
            )
        };
        // u = True, u = v, v = False  ⟹  True = False
        let true_eq_false = apps_(
            gl("Eq_trans"),
            &[
                prop(),
                gl("True"),
                u_of(),
                gl("False"),
                apps_(gl("Eq_sym"), &[prop(), u_of(), gl("True"), v_("hut")]),
                apps_(
                    gl("Eq_trans"),
                    &[prop(), u_of(), v_of(), gl("False"), v_("huv"), v_("hvf")],
                ),
            ],
        );
        // not_uv_or_p : Or (¬(u = v)) P
        let not_uv_or_p = apps_(
            gl("Or_rec"),
            &[
                eq_prop(u_of(), gl("True")),
                p_prop(),
                lam_(
                    "_du",
                    apps_(gl("Or"), &[eq_prop(u_of(), gl("True")), p_prop()]),
                    disj(),
                ),
                lam_(
                    "hut",
                    eq_prop(u_of(), gl("True")),
                    apps_(
                        gl("Or_rec"),
                        &[
                            eq_prop(v_of(), gl("False")),
                            p_prop(),
                            lam_(
                                "_dv",
                                apps_(gl("Or"), &[eq_prop(v_of(), gl("False")), p_prop()]),
                                disj(),
                            ),
                            lam_(
                                "hvf",
                                eq_prop(v_of(), gl("False")),
                                apps_(
                                    gl("left"),
                                    &[
                                        not_uv(),
                                        p_prop(),
                                        lam_(
                                            "huv",
                                            eq_prop(u_of(), v_of()),
                                            true_ne_false(true_eq_false),
                                        ),
                                    ],
                                ),
                            ),
                            lam_(
                                "hpv",
                                p_prop(),
                                apps_(gl("right"), &[not_uv(), p_prop(), v_("hpv")]),
                            ),
                            spec(v_pred(), ex_v()),
                        ],
                    ),
                ),
                lam_(
                    "hpu",
                    p_prop(),
                    apps_(gl("right"), &[not_uv(), p_prop(), v_("hpu")]),
                ),
                spec(u_pred(), ex_u()),
            ],
        );

        // p_implies_uv : P → Eq Prop u v
        let prop_fn = || pi_("x", prop(), prop());
        let iff_both_right = apps_(
            gl("conj"),
            &[
                arrow_(
                    Term::App(Box::new(u_pred()), Box::new(v_("x"))),
                    Term::App(Box::new(v_pred()), Box::new(v_("x"))),
                ),
                arrow_(
                    Term::App(Box::new(v_pred()), Box::new(v_("x"))),
                    Term::App(Box::new(u_pred()), Box::new(v_("x"))),
                ),
                lam_(
                    "_fu",
                    Term::App(Box::new(u_pred()), Box::new(v_("x"))),
                    apps_(
                        gl("right"),
                        &[eq_prop(v_("x"), gl("False")), p_prop(), v_("hp")],
                    ),
                ),
                lam_(
                    "_fv",
                    Term::App(Box::new(v_pred()), Box::new(v_("x"))),
                    apps_(
                        gl("right"),
                        &[eq_prop(v_("x"), gl("True")), p_prop(), v_("hp")],
                    ),
                ),
            ],
        );
        // hpred : Eq (Prop → Prop) U V
        let hpred = apps_(
            gl("funext"),
            &[
                prop(),
                lam_("_bx", prop(), prop()),
                u_pred(),
                v_pred(),
                lam_(
                    "x",
                    prop(),
                    apps_(
                        gl("propext"),
                        &[
                            Term::App(Box::new(u_pred()), Box::new(v_("x"))),
                            Term::App(Box::new(v_pred()), Box::new(v_("x"))),
                            iff_both_right,
                        ],
                    ),
                ),
            ],
        );
        // The motive: for ANY W equal to U, the choice made from U and the choice made from W
        // agree — quantified over both existence proofs, because they are what the two `choose`
        // calls differ in and proof irrelevance is what closes the base case.
        let ex_of = |w: Term| apps_(gl("Ex"), &[prop(), w]);
        let agree_motive = lam_(
            "W",
            prop_fn(),
            lam_(
                "_hw",
                eq3(prop_fn(), u_pred(), v_("W")),
                pi_(
                    "e1",
                    ex_of(u_pred()),
                    pi_(
                        "e2",
                        ex_of(v_("W")),
                        eq_prop(chosen(u_pred(), v_("e1")), chosen(v_("W"), v_("e2"))),
                    ),
                ),
            ),
        );
        let agree_base = lam_(
            "e1",
            ex_of(u_pred()),
            lam_(
                "e2",
                ex_of(u_pred()),
                apps_(gl("refl"), &[prop(), chosen(u_pred(), v_("e1"))]),
            ),
        );
        let p_implies_uv = apps_(
            apps_(
                gl("Eq_rec_dep"),
                &[
                    prop_fn(),
                    u_pred(),
                    agree_motive,
                    agree_base,
                    v_pred(),
                    hpred,
                ],
            ),
            &[ex_u(), ex_v()],
        );
        let em_body = lam_(
            "P",
            prop(),
            apps_(
                gl("Or_rec"),
                &[
                    not_uv(),
                    p_prop(),
                    lam_("_dd", disj(), apps_(gl("Or"), &[p_prop(), not_p_of()])),
                    lam_(
                        "hne",
                        not_uv(),
                        apps_(
                            gl("right"),
                            &[
                                p_prop(),
                                not_p_of(),
                                lam_(
                                    "hp",
                                    p_prop(),
                                    Term::App(Box::new(v_("hne")), Box::new(p_implies_uv)),
                                ),
                            ],
                        ),
                    ),
                    lam_(
                        "hpp",
                        p_prop(),
                        apps_(gl("left"), &[p_prop(), not_p_of(), v_("hpp")]),
                    ),
                    not_uv_or_p,
                ],
            ),
        );
        ctx.add_definition(
            "Classical_em".to_string(),
            pi_("P", prop(), apps_(gl("Or"), &[p_prop(), not_p_of()])),
            em_body,
        );

        // ---- Classical.propDecidable (Init/Classical.lean:75) ------------------------------
        //
        //   propDecidable a := choice <| match em a with
        //                                | Or.inl h => ⟨isTrue h⟩
        //                                | Or.inr h => ⟨isFalse h⟩
        //
        // Lean's proof, unchanged: `Or_rec` at the `Prop` motive `Nonempty (Decidable p)` (legal
        // — `Nonempty` is a proposition), then one `Classical_choice` to cross into `Type`.
        let dec_p = || Term::App(Box::new(gl("Decidable")), Box::new(v_("p")));
        let ne_dec = || Term::App(Box::new(gl("Nonempty")), Box::new(dec_p()));
        let intro = |d: Term| apps_(gl("Nonempty_intro"), &[dec_p(), d]);
        let not_p_v = || Term::App(Box::new(gl("Not")), Box::new(v_("p")));
        let or_p_v = || apps_(gl("Or"), &[v_("p"), not_p_v()]);
        ctx.add_definition(
            "Classical_propDecidable".to_string(),
            pi_("p", prop(), dec_p()),
            lam_(
                "p",
                prop(),
                apps_(
                    gl("Classical_choice"),
                    &[
                        dec_p(),
                        apps_(
                            gl("Or_rec"),
                            &[
                                v_("p"),
                                not_p_v(),
                                lam_("_dp", or_p_v(), ne_dec()),
                                lam_(
                                    "hyes",
                                    v_("p"),
                                    intro(apps_(gl("isTrue"), &[v_("p"), v_("hyes")])),
                                ),
                                lam_(
                                    "hno",
                                    not_p_v(),
                                    intro(apps_(gl("isFalse"), &[v_("p"), v_("hno")])),
                                ),
                                Term::App(Box::new(gl("Classical_em")), Box::new(v_("p"))),
                            ],
                        ),
                    ],
                ),
            ),
        );

        // ---- dne — Lean's `Classical.byContradiction` (Init/Classical.lean:131) ------------
        //
        //   byContradiction (h : ¬p → False) : p := Decidable.byContradiction (dec := propDecidable _) h
        //
        // `dne : Π(P:Prop). ¬¬P → P` is the LOGOS spelling and it is now a THEOREM. It was the
        // prelude's one home-grown classical axiom, and it is gone from the trusted base: the
        // blanket `Decidable` instance decides `P`, the `isTrue` branch returns the proof and
        // the `isFalse` branch feeds the refutation to the double negation and eliminates the
        // resulting `False` into `P` with a zero-case `Match` (small elimination, no constant).
        let dne_p = || v_("P");
        let not_dne = || arrow_(dne_p(), gl("False"));
        let dne_body = lam_(
            "P",
            prop(),
            lam_(
                "hnn",
                arrow_(not_dne(), gl("False")),
                apps_(
                    gl("Decidable_rec"),
                    &[
                        dne_p(),
                        lam_(
                            "_dd",
                            Term::App(Box::new(gl("Decidable")), Box::new(dne_p())),
                            dne_p(),
                        ),
                        lam_("hyes", dne_p(), v_("hyes")),
                        lam_(
                            "hno",
                            Term::App(Box::new(gl("Not")), Box::new(dne_p())),
                            Term::Match {
                                discriminant: Box::new(Term::App(
                                    Box::new(v_("hnn")),
                                    Box::new(v_("hno")),
                                )),
                                motive: Box::new(lam_("_hf", gl("False"), dne_p())),
                                cases: vec![],
                            },
                        ),
                        Term::App(Box::new(gl("Classical_propDecidable")), Box::new(dne_p())),
                    ],
                ),
            ),
        );
        ctx.add_definition(
            "dne".to_string(),
            pi_("P", prop(), arrow_(arrow_(not_dne(), gl("False")), dne_p())),
            dne_body,
        );

        // Classical_axiomOfChoice — Skolemisation, in Lean's dependent shape:
        //
        //   Π(A:Type). Π(B:A→Type). Π(r : Π(x:A). B x → Prop).
        //     (Π(x:A). Ex (B x) (r x)) → Ex (Π(x:A). B x) (λf. Π(x:A). r x (f x))
        //
        // It produces a FUNCTION, which is why the `Prop`-only `dne` fragment cannot reach it: the
        // witness has to be chosen at every point at once. Proved, not assumed — each `Ex` is
        // turned into a `Nonempty (Subtype …)` by `Ex_rec` at a `Prop` motive (legal), `choice`
        // picks the subtype element (the one classical step), and its `val`/`property`
        // projections COMPUTE the function and its specification.
        let b_at = |x: Term| Term::App(Box::new(v_("B")), Box::new(x));
        let r_at = |x: Term, y: Term| apps_(v_("r"), &[x, y]);
        let sub_x = || {
            apps_(
                gl("Subtype"),
                &[
                    b_at(v_("x")),
                    Term::App(Box::new(v_("r")), Box::new(v_("x"))),
                ],
            )
        };
        let ex_at = |x: Term| {
            apps_(
                gl("Ex"),
                &[b_at(x.clone()), Term::App(Box::new(v_("r")), Box::new(x))],
            )
        };
        // ne x : Nonempty (Subtype (B x) (r x))
        let ne_x = apps_(
            gl("Ex_rec"),
            &[
                b_at(v_("x")),
                Term::App(Box::new(v_("r")), Box::new(v_("x"))),
                lam_(
                    "_",
                    ex_at(v_("x")),
                    Term::App(Box::new(gl("Nonempty")), Box::new(sub_x())),
                ),
                lam_(
                    "y",
                    b_at(v_("x")),
                    lam_(
                        "hy",
                        r_at(v_("x"), v_("y")),
                        apps_(
                            gl("Nonempty_intro"),
                            &[
                                sub_x(),
                                apps_(
                                    gl("Subtype_mk"),
                                    &[
                                        b_at(v_("x")),
                                        Term::App(Box::new(v_("r")), Box::new(v_("x"))),
                                        v_("y"),
                                        v_("hy"),
                                    ],
                                ),
                            ],
                        ),
                    ),
                ),
                Term::App(Box::new(v_("h")), Box::new(v_("x"))),
            ],
        );
        // pick x : Subtype (B x) (r x)
        let pick = apps_(gl("Classical_choice"), &[sub_x(), ne_x]);
        let proj = |field: &str| {
            apps_(
                gl(field),
                &[
                    b_at(v_("x")),
                    Term::App(Box::new(v_("r")), Box::new(v_("x"))),
                    pick.clone(),
                ],
            )
        };
        let chosen_fn = lam_("x", v_("A"), proj("Subtype_val"));
        let chosen_spec = lam_("x", v_("A"), proj("Subtype_property"));
        let fn_ty = || pi_("x", v_("A"), b_at(v_("x")));
        let spec_pred = || {
            lam_(
                "f",
                fn_ty(),
                pi_(
                    "x",
                    v_("A"),
                    r_at(v_("x"), Term::App(Box::new(v_("f")), Box::new(v_("x")))),
                ),
            )
        };
        let rel_ty = || pi_("x", v_("A"), arrow_(b_at(v_("x")), prop()));
        let hyp_ty = || pi_("x", v_("A"), ex_at(v_("x")));
        ctx.add_definition(
            "Classical_axiomOfChoice".to_string(),
            pi_(
                "A",
                type0(),
                pi_(
                    "B",
                    pi_("_x", v_("A"), type0()),
                    pi_(
                        "r",
                        rel_ty(),
                        arrow_(hyp_ty(), apps_(gl("Ex"), &[fn_ty(), spec_pred()])),
                    ),
                ),
            ),
            lam_(
                "A",
                type0(),
                lam_(
                    "B",
                    pi_("_x", v_("A"), type0()),
                    lam_(
                        "r",
                        rel_ty(),
                        lam_(
                            "h",
                            hyp_ty(),
                            apps_(
                                gl("witness"),
                                &[fn_ty(), spec_pred(), chosen_fn, chosen_spec],
                            ),
                        ),
                    ),
                ),
            ),
        );
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
                Box::new(Term::App(
                    Box::new(Term::Global(op.to_string())),
                    Box::new(a),
                )),
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
                    bin(
                        "add",
                        bin("mul", v("a"), v("b")),
                        bin("mul", v("a"), v("c")),
                    ),
                ),
            ),
        );
        // sub_elim : Π a b. Eq Int (sub a b) (add a (mul (-1) b)). Subtraction defined over the ring.
        // `sub` is a primitive with only literal reduction (`sub 5 3 ⇝ 2`), so a symbolic `sub a b`
        // stays stuck; this lets the normalizer prove identities written with `sub` by rewriting it to
        // the `add`/`mul` vocabulary it decides. Asserted like `mul_zero` (true in ℤ, not derivable from
        // the multiplicative/additive laws without it).
        ctx.add_declaration(
            "sub_elim",
            forall_ints(
                &["a", "b"],
                eq_int(
                    bin("sub", v("a"), v("b")),
                    bin("add", v("a"), bin("mul", lit(-1), v("b"))),
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
                Box::new(Term::App(
                    Box::new(Term::Global(op.to_string())),
                    Box::new(a),
                )),
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
        // lt_succ_le : Π a b. a < b → (a + 1) ≤ b, and its converse le_succ_lt.  Integer
        // DISCRETENESS — the fact rational Fourier-Motzkin lacks: over ℤ a strict `<` is a `≤`
        // shifted by one, so nothing lives strictly between `a` and `a+1`. It is what lets
        // `omega` refute strict systems the rational solver reports satisfiable
        // (`x < y ∧ y < x+1`), and it is discharged by ordinary Farkas afterward.
        //
        // BOTH DIRECTIONS ARE `λa b h. h`, and that is the whole point. `lt` is no longer an
        // opaque primitive: it is DEFINED as `le (a + 1) b` in `register_primitives`, so
        // `Eq Bool (lt a b) true` and `Eq Bool (le (a+1) b) true` are the SAME proposition up to
        // δβ and the identity inhabits each from the other. Wave 1 assumed the forward direction
        // as an axiom and the backward one was simply missing, which is why no `Int` order axiom
        // could CONCLUDE a strict `<` and `calc` could not compose a mixed `≤`/`<` chain
        // (T08-021). Both are now kernel-checked theorems and the trusted base is two smaller.
        let ident_proof = |premise: Term| Term::Lambda {
            param: "a".to_string(),
            param_type: Box::new(int()),
            body: Box::new(Term::Lambda {
                param: "b".to_string(),
                param_type: Box::new(int()),
                body: Box::new(Term::Lambda {
                    param: "h".to_string(),
                    param_type: Box::new(premise),
                    body: Box::new(v("h")),
                }),
            }),
        };
        ctx.add_definition(
            "lt_succ_le".to_string(),
            forall_ints(
                &["a", "b"],
                arrow(
                    lt_prop(v("a"), v("b")),
                    le_prop(bin("add", v("a"), lit(1)), v("b")),
                ),
            ),
            ident_proof(lt_prop(v("a"), v("b"))),
        );
        // le_succ_lt : Π a b. (a + 1) ≤ b → a < b.  The direction that CONCLUDES a `<`, which no
        // assumed order law had: `lt_succ_le` and `lt_add1_le` both take one as a premise.
        ctx.add_definition(
            "le_succ_lt".to_string(),
            forall_ints(
                &["a", "b"],
                arrow(
                    le_prop(bin("add", v("a"), lit(1)), v("b")),
                    lt_prop(v("a"), v("b")),
                ),
            ),
            ident_proof(le_prop(bin("add", v("a"), lit(1)), v("b"))),
        );
        // lt_add1_le : Π a b. a < (b + 1) → a ≤ b.  The upper-side companion of
        // `lt_succ_le` (`a < b+1 ⟺ a ≤ b` over ℤ): when the strict bound already has
        // the shape `b + 1`, this yields the CANCELLED `a ≤ b` directly instead of the
        // constant-laden `a + 1 ≤ b + 1`, keeping the reconstructed Farkas terms small.
        ctx.add_declaration(
            "lt_add1_le",
            forall_ints(
                &["a", "b"],
                arrow(
                    lt_prop(v("a"), bin("add", v("b"), lit(1))),
                    le_prop(v("a"), v("b")),
                ),
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
        //
        // `le` is the ONE opaque order primitive, and the other three are DEFINITIONS written
        // from it — which is exactly what Lean does. `Int.lt a b := a + 1 ≤ b` (integer
        // discreteness: nothing lives strictly between `a` and `a+1`), `GE.ge a b := LE.le b a`
        // and `GT.gt a b := LT.lt b a`. Assuming those three here would be assuming three facts
        // the reference implementation *proves*, and it also made `lt_succ_le` — the bridge every
        // `omega` refutation of a strict system runs through — an axiom instead of `λa b h. h`.
        //
        // A body does NOT cost the literal computation: a primitive rule fires on a NAME and both
        // kernels already refuse to let one outrank a definition of that name
        // (`reduction.rs::try_primitive_reduce`'s `get_definition_body` gate and
        // `recheck.rs::shadowed_by_a_definition`), so `lt 2 3` now δβ-unfolds to `le 3 3` and is
        // decided by `le`'s rule in BOTH kernels, from ONE specification instead of two copies.
        ctx.add_declaration("le", bin_int_bool_type.clone());

        let bin_int = |op: &str, x: Term, y: Term| {
            Term::App(
                Box::new(Term::App(
                    Box::new(Term::Global(op.to_string())),
                    Box::new(x),
                )),
                Box::new(y),
            )
        };
        let order_body = |body: Term| Term::Lambda {
            param: "a".to_string(),
            param_type: Box::new(int.clone()),
            body: Box::new(Term::Lambda {
                param: "b".to_string(),
                param_type: Box::new(int.clone()),
                body: Box::new(body),
            }),
        };
        let a = || Term::Var("a".to_string());
        let b = || Term::Var("b".to_string());

        // lt a b := le (a + 1) b
        ctx.add_definition(
            "lt".to_string(),
            bin_int_bool_type.clone(),
            order_body(bin_int(
                "le",
                bin_int("add", a(), Term::Lit(Literal::Int(1))),
                b(),
            )),
        );
        // ge a b := le b a
        ctx.add_definition(
            "ge".to_string(),
            bin_int_bool_type.clone(),
            order_body(bin_int("le", b(), a())),
        );
        // gt a b := lt b a
        ctx.add_definition(
            "gt".to_string(),
            bin_int_bool_type,
            order_body(bin_int("lt", b(), a())),
        );

        // `Float`'s arithmetic and comparison surface — OPAQUE constants, exactly as in Lean.
        //
        // Lean's are `@[extern] opaque Float.add : Float → Float → Float`: they exist, they have
        // types, they do not reduce in the kernel, and `native_decide` evaluates them. Before
        // this, `Float` was a carrier with NOTHING — not one operation, not one axiom, not one
        // reduction rule — so `def sum : Float := 1.5 + 2.5` had no target at all, which is
        // strictly worse than an axiomatised type (that at least has laws).
        //
        // They are declarations rather than definitions because `Float` has no constructors, so
        // there is no body to write; a definition with an invented body would COMPUTE a wrong
        // answer, which is the one outcome worse than not computing. The `Int` ALU builtins
        // (`add`/`le`/…) above are the same shape, and `reduction.rs` gives those literal
        // short-circuits — the same hook is what `Float` owes next, and until it lands these are
        // honestly opaque rather than quietly wrong.
        //
        // `add`/`sub`/`mul` stay `Int`-ONLY: reusing them at `Float` would make
        // `add 1.5 2.5` well-typed and silently reduce it with integer semantics.
        let float = Term::Global("Float".to_string());
        let bin_float = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(float.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(float.clone()),
                body_type: Box::new(float.clone()),
            }),
        };
        let float_cmp = Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(float.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(float.clone()),
                body_type: Box::new(Term::Global("Bool".to_string())),
            }),
        };
        for name in ["Float_add", "Float_sub", "Float_mul", "Float_div"] {
            ctx.add_declaration(name, bin_float.clone());
        }
        for name in ["Float_lt", "Float_le"] {
            ctx.add_declaration(name, float_cmp.clone());
        }

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
        ctx.add_definition(
            "TTactics".to_string(),
            Term::Sort(Universe::Type(0)),
            ttactics.clone(),
        );

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
        ctx.add_constructor(
            "isTrue",
            "Decidable",
            pi("p", prop(), pi("_", v("p"), dec(v("p")))),
        );
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
                            ap(
                                ap(g("Decidable_rec"), v("p")),
                                lm("_", dec(v("p")), g("Bool")),
                            ),
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
                ap(
                    ap(ap(ap(g("Eq_rec_dep"), g("Bool")), g("false")), bool_motive),
                    g("I"),
                ),
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
                ap(
                    ap(ap(ap(g("Decidable_rec"), v("p")), motive), f_istrue),
                    f_isfalse,
                ),
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
                ap(
                    ap(
                        ap(ap(ap(g("Eq_rec_dep"), g("Bool")), g("true")), tf_motive),
                        g("I"),
                    ),
                    g("false"),
                ),
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
                ap(
                    ap(
                        ap(ap(ap(g("Eq_rec_dep"), g("Bool")), g("false")), ft_motive),
                        g("I"),
                    ),
                    g("true"),
                ),
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
            ap(
                ap(
                    ap(ap(ap(ap(g("Eq_rec_dep"), nat()), x), discr_motive), base),
                    y,
                ),
                h,
            )
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
        ctx.add_definition(
            "nat_zs_ne".to_string(),
            pi("n", nat(), not(eqn(g("Zero"), succ(v("n"))))),
            nat_zs_ne,
        );

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
        ctx.add_definition(
            "nat_sz_ne".to_string(),
            pi("n", nat(), not(eqn(succ(v("n")), g("Zero")))),
            nat_sz_ne,
        );

        // succ_cong : Π(a b:Nat). Eq Nat a b → Eq Nat (Succ a) (Succ b).
        let sc_motive = lm(
            "b'",
            nat(),
            lm("_", eqn(v("a"), v("b'")), eqn(succ(v("a")), succ(v("b'")))),
        );
        let succ_cong_body = lm(
            "a",
            nat(),
            lm(
                "b",
                nat(),
                lm(
                    "h",
                    eqn(v("a"), v("b")),
                    ap(
                        ap(
                            ap(
                                ap(ap(ap(g("Eq_rec_dep"), nat()), v("a")), sc_motive),
                                refln(succ(v("a"))),
                            ),
                            v("b"),
                        ),
                        v("h"),
                    ),
                ),
            ),
        );
        let succ_cong_ty = pi(
            "a",
            nat(),
            pi(
                "b",
                nat(),
                pi("_", eqn(v("a"), v("b")), eqn(succ(v("a")), succ(v("b")))),
            ),
        );
        ctx.add_definition("succ_cong".to_string(), succ_cong_ty, succ_cong_body);

        // succ_inj : Π(a b:Nat). Eq Nat (Succ a) (Succ b) → Eq Nat a b.
        // `pred y := match y with Zero => Zero | Succ p => p`; transport `Eq Nat a (pred y)`
        // along `h`, base `refl Nat a` (since `pred (Succ a) ≡ a`), result `Eq Nat a b`.
        let pred_y = Term::Match {
            discriminant: Box::new(v("y'")),
            motive: Box::new(lm("_", nat(), nat())),
            cases: vec![g("Zero"), lm("p", nat(), v("p"))],
        };
        let si_motive = lm(
            "y'",
            nat(),
            lm("_", eqn(succ(v("a")), v("y'")), eqn(v("a"), pred_y)),
        );
        let succ_inj_body = lm(
            "a",
            nat(),
            lm(
                "b",
                nat(),
                lm(
                    "h",
                    eqn(succ(v("a")), succ(v("b"))),
                    ap(
                        ap(
                            ap(
                                ap(ap(ap(g("Eq_rec_dep"), nat()), succ(v("a"))), si_motive),
                                refln(v("a")),
                            ),
                            succ(v("b")),
                        ),
                        v("h"),
                    ),
                ),
            ),
        );
        let succ_inj_ty = pi(
            "a",
            nat(),
            pi(
                "b",
                nat(),
                pi("_", eqn(succ(v("a")), succ(v("b"))), eqn(v("a"), v("b"))),
            ),
        );
        ctx.add_definition("succ_inj".to_string(), succ_inj_ty, succ_inj_body);

        // decEqNat : Π(a b:Nat). Decidable (Eq Nat a b)  — structural recursion on `a`.
        // Inner match on the recursive result `rec a' b'`.
        let rec_call = ap(ap(v("rec"), v("a'")), v("b'"));
        let true_case = lm(
            "hp",
            eqn(v("a'"), v("b'")),
            is_true(
                eqn(succ(v("a'")), succ(v("b'"))),
                ap(ap(ap(g("succ_cong"), v("a'")), v("b'")), v("hp")),
            ),
        );
        let false_case = lm(
            "hnp",
            not(eqn(v("a'"), v("b'"))),
            is_false(
                eqn(succ(v("a'")), succ(v("b'"))),
                lm(
                    "hs",
                    eqn(succ(v("a'")), succ(v("b'"))),
                    ap(
                        v("hnp"),
                        ap(ap(ap(g("succ_inj"), v("a'")), v("b'")), v("hs")),
                    ),
                ),
            ),
        );
        let inner_rec_match = Term::Match {
            discriminant: Box::new(rec_call),
            motive: Box::new(lm(
                "_",
                dec(eqn(v("a'"), v("b'"))),
                dec(eqn(succ(v("a'")), succ(v("b'")))),
            )),
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
                lm(
                    "b'",
                    nat(),
                    is_false(eqn(g("Zero"), succ(v("b'"))), ap(g("nat_zs_ne"), v("b'"))),
                ),
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

    /// The `Decidable` INSTANCES — Lean's `instDecidableEqNat` and `instDecidableNot`.
    ///
    /// `decEqBool` and `decEqNat` are decision PROCEDURES; an instance is what makes one
    /// FINDABLE from a goal. Without an entry, `Decidable (Eq Nat b Zero)` is answerable only by
    /// the last-resort derived tier and `Decidable (Not p)` is not answerable at all — so an
    /// auto-param obligation `(h : b ≠ 0 := by decide)` has nothing to run, which is T05-007's
    /// remaining half.
    ///
    /// Both go in the FALLBACK tier, for the reason [`Self::register_inhabited`] states at
    /// length: `Context::instances()` is resolved first-match and is the faithful record of what
    /// a USER declared, so a prelude entry there would be entry zero and would permanently
    /// outrank every instance anyone writes for the same goal.
    ///
    /// `instDecidableNot` is a DEFINITION with a kernel-checked body and NOT an assumption — it
    /// is `Decidable_rec` with the two constructors swapped, and the `isTrue` case builds
    /// `Not (Not p)` from `hp : p` as `λhn. hn hp`. Nothing here enters the trusted base, which
    /// matters: an axiomatised `Decidable p` inhabits the class for EVERY proposition, and
    /// `of_decide_eq_true` would then turn it into a proof of anything.
    fn register_decidable_instances(ctx: &mut Context) {
        let prop = || Term::Sort(Universe::Prop);
        let g = |s: &str| Term::Global(s.to_string());
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let apps = |head: Term, args: &[Term]| {
            args.iter()
                .fold(head, |acc, a| Term::App(Box::new(acc), Box::new(a.clone())))
        };
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
        let eq_at = |ty: Term, a: Term, b: Term| apps(g("Eq"), &[ty, a, b]);

        // `instDecidableEqNat` / `instDecidableEqBool` — the prelude's own `DecidableEq`
        // instances, pointing at the procedures it already proved. No new constant: the instance
        // IS `decEqNat`, so there is exactly one decision procedure for `Eq Nat` and a goal and a
        // direct call can never come to disagree about which one decided.
        ctx.add_fallback_instance(
            pi(
                "a",
                g("Nat"),
                pi("b", g("Nat"), dec(eq_at(g("Nat"), v("a"), v("b")))),
            ),
            g("decEqNat"),
        );
        ctx.add_fallback_instance(
            pi(
                "a",
                g("Bool"),
                pi("b", g("Bool"), dec(eq_at(g("Bool"), v("a"), v("b")))),
            ),
            g("decEqBool"),
        );

        // instDecidableNot : Π(p:Prop). Decidable p → Decidable (Not p)
        //   := λp inst. Decidable_rec p (λ_. Decidable (Not p))
        //                 (λhp. isFalse (Not p) (λhn. hn hp))
        //                 (λhn. isTrue  (Not p) hn)
        //                 inst
        let not_ty = pi("p", prop(), pi("_", dec(v("p")), dec(not(v("p")))));
        let not_body = lm(
            "p",
            prop(),
            lm(
                "inst",
                dec(v("p")),
                apps(
                    g("Decidable_rec"),
                    &[
                        v("p"),
                        lm("_", dec(v("p")), dec(not(v("p")))),
                        lm(
                            "hp",
                            v("p"),
                            apps(
                                g("isFalse"),
                                &[not(v("p")), lm("hn", not(v("p")), ap(v("hn"), v("hp")))],
                            ),
                        ),
                        lm(
                            "hn",
                            not(v("p")),
                            apps(g("isTrue"), &[not(v("p")), v("hn")]),
                        ),
                        v("inst"),
                    ],
                ),
            ),
        );
        ctx.add_definition("instDecidableNot".to_string(), not_ty.clone(), not_body);
        ctx.add_fallback_instance(not_ty, g("instDecidableNot"));
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
                    pi(
                        "_",
                        eqb(ap(g("reduceBool"), v("a")), v("b")),
                        eqb(v("a"), v("b")),
                    ),
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
    /// identification consistent. The dependent eliminator `Quot_rec` is a checked DEFINITION
    /// over them, as Lean's `Quot.rec` is. Unblocks ℤ/ℚ/ℝ, `Multiset`, setoids.
    fn register_quot(ctx: &mut Context) {
        crate::quotient::register_prelude(ctx)
            .expect("checked legacy quotient primitive registration");
    }

    /// `Setoid` and `Quotient` — the layer Lean's ENTIRE quotient API is phrased in.
    ///
    /// Mathlib never writes `Quot`. `Int`, `Rat`, `ZMod`, free groups, completions and every
    /// `Quotient.lift` in the library go through `Quotient s` where `s : Setoid α` carries the
    /// relation AND its equivalence proof. Silently unfolding `Quotient s` to `Quot s.r` at
    /// translation time drops the instance that was resolved there and is no longer resolvable at
    /// check time, so the wrapper is not sugar — it is where the relation comes from.
    ///
    /// Neither costs the trusted base anything: `Setoid` is a STRUCTURE (its projections
    /// ι-reduce) and `Quotient`/`Quotient_mk` are δ-transparent DEFINITIONS over the existing
    /// `Quot` primitives, so `Quotient_mk` really is `Quot_mk` and the one computation rule
    /// quotients have still fires through it.
    fn register_setoid(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let prop = || Term::Sort(Universe::Prop);
        let rel_at = |x: Term, y: Term| apps_(v_("r"), &[x, y]);

        // structure Setoid (a : Type) where
        //   r : a → a → Prop ; refl : Π x. r x x
        //   symm : Π x y. r x y → r y x ; trans : Π x y z. r x y → r y z → r x z
        ctx.add_structure(
            "Setoid",
            &[("a", type0())],
            &[
                ("r", pi_("_x", v_("a"), pi_("_y", v_("a"), prop()))),
                ("refl", pi_("x", v_("a"), rel_at(v_("x"), v_("x")))),
                (
                    "symm",
                    pi_(
                        "x",
                        v_("a"),
                        pi_(
                            "y",
                            v_("a"),
                            arrow_(rel_at(v_("x"), v_("y")), rel_at(v_("y"), v_("x"))),
                        ),
                    ),
                ),
                (
                    "trans",
                    pi_(
                        "x",
                        v_("a"),
                        pi_(
                            "y",
                            v_("a"),
                            pi_(
                                "z",
                                v_("a"),
                                arrow_(
                                    rel_at(v_("x"), v_("y")),
                                    arrow_(rel_at(v_("y"), v_("z")), rel_at(v_("x"), v_("z"))),
                                ),
                            ),
                        ),
                    ),
                ),
            ],
        );

        // Quotient : Π(a:Type). Setoid a → Type := λa s. Quot a (Setoid_r a s)
        let setoid_a = || Term::App(Box::new(gl("Setoid")), Box::new(v_("a")));
        let carrier_rel = || apps_(gl("Setoid_r"), &[v_("a"), v_("s")]);
        ctx.add_definition(
            "Quotient".to_string(),
            pi_("a", type0(), arrow_(setoid_a(), type0())),
            lam_(
                "a",
                type0(),
                lam_(
                    "s",
                    setoid_a(),
                    apps_(gl("Quot"), &[v_("a"), carrier_rel()]),
                ),
            ),
        );

        // Quotient_mk : Π(a:Type). Π(s:Setoid a). a → Quotient a s
        //   := λa s x. Quot_mk a (Setoid_r a s) x
        ctx.add_definition(
            "Quotient_mk".to_string(),
            pi_(
                "a",
                type0(),
                pi_(
                    "s",
                    setoid_a(),
                    arrow_(v_("a"), apps_(gl("Quotient"), &[v_("a"), v_("s")])),
                ),
            ),
            lam_(
                "a",
                type0(),
                lam_(
                    "s",
                    setoid_a(),
                    lam_(
                        "x",
                        v_("a"),
                        apps_(gl("Quot_mk"), &[v_("a"), carrier_rel(), v_("x")]),
                    ),
                ),
            ),
        );

        // ---- The rest of Lean's `Quotient` API (Init/Core.lean:1678-1725) -----------------
        //
        // `Quotient.sound`, `.lift`, `.ind` and `.rec` are what Mathlib actually calls; `Quot` is
        // the layer underneath and is almost never written by hand. Each of Lean's is literally
        // its `Quot` counterpart with the relation read off the setoid
        // (`sound := Quot.sound`, `lift f := Quot.lift f`, `ind := Quot.ind`, `rec := Quot.rec`),
        // and each of ours is the same one-line body. None costs the trusted base anything: they
        // are δ-transparent definitions over the `Quot` family, so the quotient
        // computation rule still fires through them and `Quotient_lift f h (Quotient_mk s a)`
        // really does reduce to `f a`.
        let rel = || apps_(gl("Setoid_r"), &[v_("a"), v_("s")]);
        let quot_as = || apps_(gl("Quotient"), &[v_("a"), v_("s")]);
        let mk_at = |x: Term| apps_(gl("Quotient_mk"), &[v_("a"), v_("s"), x]);
        let head2 = |inner: Term| lam_("a", type0(), lam_("s", setoid_a(), inner));
        let head2_ty = |inner: Term| pi_("a", type0(), pi_("s", setoid_a(), inner));

        // Quotient_sound : Π(a)(s)(x y:a). Setoid_r a s x y → Eq (Quotient a s) (mk x) (mk y)
        ctx.add_definition(
            "Quotient_sound".to_string(),
            head2_ty(pi_(
                "x",
                v_("a"),
                pi_(
                    "y",
                    v_("a"),
                    arrow_(
                        apps_(rel(), &[v_("x"), v_("y")]),
                        apps_(gl("Eq"), &[quot_as(), mk_at(v_("x")), mk_at(v_("y"))]),
                    ),
                ),
            )),
            head2(lam_(
                "x",
                v_("a"),
                lam_(
                    "y",
                    v_("a"),
                    lam_(
                        "hxy",
                        apps_(rel(), &[v_("x"), v_("y")]),
                        apps_(
                            gl("Quot_sound"),
                            &[v_("a"), rel(), v_("x"), v_("y"), v_("hxy")],
                        ),
                    ),
                ),
            )),
        );

        // Quotient_lift : Π(a)(s)(B:Type)(f:a→B).
        //   (Π x y. Setoid_r a s x y → Eq B (f x) (f y)) → Quotient a s → B
        let respect = || {
            pi_(
                "x",
                v_("a"),
                pi_(
                    "y",
                    v_("a"),
                    arrow_(
                        apps_(rel(), &[v_("x"), v_("y")]),
                        apps_(
                            gl("Eq"),
                            &[
                                v_("B"),
                                Term::App(Box::new(v_("f")), Box::new(v_("x"))),
                                Term::App(Box::new(v_("f")), Box::new(v_("y"))),
                            ],
                        ),
                    ),
                ),
            )
        };
        ctx.add_definition(
            "Quotient_lift".to_string(),
            head2_ty(pi_(
                "B",
                type0(),
                pi_(
                    "f",
                    arrow_(v_("a"), v_("B")),
                    arrow_(respect(), arrow_(quot_as(), v_("B"))),
                ),
            )),
            head2(lam_(
                "B",
                type0(),
                lam_(
                    "f",
                    arrow_(v_("a"), v_("B")),
                    lam_(
                        "hresp",
                        respect(),
                        lam_(
                            "q",
                            quot_as(),
                            apps_(
                                gl("Quot_lift"),
                                &[v_("a"), rel(), v_("B"), v_("f"), v_("hresp"), v_("q")],
                            ),
                        ),
                    ),
                ),
            )),
        );

        // Quotient_ind : Π(a)(s)(P : Quotient a s → Prop).
        //   (Π x. P (mk x)) → Π q. P q
        let ind_motive = || arrow_(quot_as(), prop());
        ctx.add_definition(
            "Quotient_ind".to_string(),
            head2_ty(pi_(
                "P",
                ind_motive(),
                arrow_(
                    pi_(
                        "x",
                        v_("a"),
                        Term::App(Box::new(v_("P")), Box::new(mk_at(v_("x")))),
                    ),
                    pi_(
                        "q",
                        quot_as(),
                        Term::App(Box::new(v_("P")), Box::new(v_("q"))),
                    ),
                ),
            )),
            head2(lam_(
                "P",
                ind_motive(),
                lam_(
                    "hcase",
                    pi_(
                        "x",
                        v_("a"),
                        Term::App(Box::new(v_("P")), Box::new(mk_at(v_("x")))),
                    ),
                    lam_(
                        "q",
                        quot_as(),
                        apps_(
                            gl("Quot_ind"),
                            &[v_("a"), rel(), v_("P"), v_("hcase"), v_("q")],
                        ),
                    ),
                ),
            )),
        );

        // Quotient_rec : the DEPENDENT eliminator, `Quot_rec` with the setoid's relation. The
        // respect obligation is stated up to transport along `Quot_sound`, exactly as Lean's is
        // (`Eq.ndrec (f a) (Quotient.sound p) = f b`).
        let dmot = || arrow_(quot_as(), type0());
        let mk_raw = |x: Term| apps_(gl("Quot_mk"), &[v_("a"), rel(), x]);
        let at_m = |x: Term| Term::App(Box::new(v_("motive")), Box::new(x));
        let dep_cases = || pi_("x", v_("a"), at_m(mk_raw(v_("x"))));
        let dep_respect = || {
            pi_(
                "x",
                v_("a"),
                pi_(
                    "y",
                    v_("a"),
                    pi_(
                        "hxy",
                        apps_(rel(), &[v_("x"), v_("y")]),
                        apps_(
                            gl("Eq"),
                            &[
                                at_m(mk_raw(v_("y"))),
                                apps_(
                                    gl("Eq_rec_dep"),
                                    &[
                                        apps_(gl("Quot"), &[v_("a"), rel()]),
                                        mk_raw(v_("x")),
                                        lam_(
                                            "q2",
                                            apps_(gl("Quot"), &[v_("a"), rel()]),
                                            lam_(
                                                "_hq",
                                                apps_(
                                                    gl("Eq"),
                                                    &[
                                                        apps_(gl("Quot"), &[v_("a"), rel()]),
                                                        mk_raw(v_("x")),
                                                        v_("q2"),
                                                    ],
                                                ),
                                                at_m(v_("q2")),
                                            ),
                                        ),
                                        Term::App(Box::new(v_("f")), Box::new(v_("x"))),
                                        mk_raw(v_("y")),
                                        apps_(
                                            gl("Quot_sound"),
                                            &[v_("a"), rel(), v_("x"), v_("y"), v_("hxy")],
                                        ),
                                    ],
                                ),
                                Term::App(Box::new(v_("f")), Box::new(v_("y"))),
                            ],
                        ),
                    ),
                ),
            )
        };
        ctx.add_definition(
            "Quotient_rec".to_string(),
            head2_ty(pi_(
                "motive",
                dmot(),
                pi_(
                    "f",
                    dep_cases(),
                    arrow_(dep_respect(), pi_("q", quot_as(), at_m(v_("q")))),
                ),
            )),
            head2(lam_(
                "motive",
                dmot(),
                lam_(
                    "f",
                    dep_cases(),
                    lam_(
                        "hresp",
                        dep_respect(),
                        lam_(
                            "q",
                            quot_as(),
                            apps_(
                                gl("Quot_rec"),
                                &[v_("a"), rel(), v_("motive"), v_("f"), v_("hresp"), v_("q")],
                            ),
                        ),
                    ),
                ),
            )),
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
            Term::Pi {
                param: p.to_string(),
                param_type: Box::new(t),
                body_type: Box::new(b),
            }
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
            xs.iter()
                .fold(f, |a, x| Term::App(Box::new(a), Box::new(x.clone())))
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
                        pi(
                            "y",
                            v("A"),
                            arrow(apps(v("R"), &[v("y"), v("x")]), acc(v("y"))),
                        ),
                        acc(v("x")),
                    ),
                ),
            ),
        );
        ctx.add_constructor("Acc_intro", "Acc", intro_ty);

        // Acc_rec — the AUTO-DERIVED dependent eliminator carrying the functional
        // induction hypothesis (two-kernel verified in tests/well_founded.rs). A
        // kernel-checked definition, not an axiom.
        // Ordinary definition publication keeps Global callers and lets the
        // existing application rules infer the motive's universe.
        let (_levels, rec_ty, rec_body) =
            crate::recursor::derive_recursor_universe_poly(ctx, "Acc")
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

        // WellFounded_fix — Lean's `WellFounded.fix`, the combinator EVERY well-founded
        // definition goes through. `Acc_rec` alone leaves each caller to rebuild it, and a
        // rebuilt-by-hand fixpoint is a different definition from the one the Lean source names.
        //
        //   WellFounded_fix.{u} : Π(A)(R)(C : A → Sort u). WellFounded A R →
        //                     (Π(x:A). (Π(y:A). R y x → C y) → C x) → Π(x:A). C x
        //
        // A DEFINITION derived from `Acc_rec`, never an axiom: `Acc_rec`'s minor premise already
        // hands the step both the accessibility field and the functional induction hypothesis, so
        // the step `F` needs only the latter. The accessibility proof is consumed at `wf x`, which
        // is what makes the recursion ι-COMPUTE rather than sit stuck.
        let motive_ty = pi("_", v("A"), Term::Sort(Universe::Var("wf_u".to_string())));
        // ih : Π(y:A). R y x → C y — the recursive calls available at `x`.
        let ih_at = |x: &str| {
            pi(
                "y",
                v("A"),
                arrow(
                    apps(v("R"), &[v("y"), v(x)]),
                    Term::App(Box::new(v("C")), Box::new(v("y"))),
                ),
            )
        };
        let c_at = |x: &str| Term::App(Box::new(v("C")), Box::new(v(x)));
        let step_ty = pi("x", v("A"), arrow(ih_at("x"), c_at("x")));
        let fix_ty = pi(
            "A",
            type0(),
            pi(
                "R",
                rel(),
                pi(
                    "C",
                    motive_ty.clone(),
                    arrow(
                        apps(g("WellFounded"), &[v("A"), v("R")]),
                        arrow(step_ty.clone(), pi("x", v("A"), c_at("x"))),
                    ),
                ),
            ),
        );
        // λA R C wf F x. Acc_rec A R (λz. λ_. C z) (λz. λh. λih. F z ih) x (wf x)
        let rec_motive = lam_(
            "z",
            v("A"),
            lam_(
                "_",
                apps(g("Acc"), &[v("A"), v("R"), v("z")]),
                Term::App(Box::new(v("C")), Box::new(v("z"))),
            ),
        );
        // h : Π(y:A). R y z → Acc A R y — the constructor's own functional field.
        let acc_field_ty = pi(
            "y",
            v("A"),
            arrow(
                apps(v("R"), &[v("y"), v("z")]),
                apps(g("Acc"), &[v("A"), v("R"), v("y")]),
            ),
        );
        let rec_step = lam_(
            "z",
            v("A"),
            lam_(
                "h",
                acc_field_ty,
                lam_("ih", ih_at("z"), apps(v("F"), &[v("z"), v("ih")])),
            ),
        );
        let fix_body = lam_(
            "A",
            type0(),
            lam_(
                "R",
                rel(),
                lam_(
                    "C",
                    motive_ty,
                    lam_(
                        "wf",
                        apps(g("WellFounded"), &[v("A"), v("R")]),
                        lam_(
                            "F",
                            step_ty,
                            lam_(
                                "x",
                                v("A"),
                                apps(
                                    g("Acc_rec"),
                                    &[
                                        v("A"),
                                        v("R"),
                                        rec_motive,
                                        rec_step,
                                        v("x"),
                                        Term::App(Box::new(v("wf")), Box::new(v("x"))),
                                    ],
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        );
        ctx.add_definition("WellFounded_fix".to_string(), fix_ty, fix_body);
    }

    /// `Eq.{u} : Π(A : Sort u). A → A → Prop` and `refl.{u} : Π(A : Sort u). Π(x:A). Eq A x x`.
    ///
    /// Equality over EVERY carrier Lean allows. Lean's is `{α : Sort u}`, and the difference is
    /// not cosmetic: a carrier pinned to `Type 0` cannot state `α = β` between two TYPES, which is
    /// what `eq_of_heq` casts along, what `∃ α : Type, α = Nat` needs, and what every `Set`- or
    /// category-level equation in Mathlib is.
    ///
    /// `Prop` stays the CODOMAIN, literally: an equation is a proposition whatever its carrier
    /// lives in, and computing `max` there instead would promote equations out of `Prop` and
    /// silently disable proof irrelevance.
    ///
    /// No new primitive is needed. `type_checker::implicit_levels_for_head` turns a bare
    /// `Term::Global` head's free level variables into metavariables at the application, and
    /// `solve_domain_levels` reads the solution off the argument's ACTUAL type — so `Eq Nat 2 2`
    /// solves `u := 0` and `Eq Type0 α Nat` solves `u := 1`. Choosing an instantiation is not
    /// accepting one: `check_type` still runs against the substituted domain afterwards, and the
    /// de Bruijn re-checker solves the same way from the same specification at its own
    /// application rule.
    fn register_eq(ctx: &mut Context) {
        // Eq.{u} : Π(A : Sort u). A → A → Prop
        let eq_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Var("u".to_string()))),
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

        // refl.{u} : Π(A : Sort u). Π(x : A). Eq A x x
        let refl_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Var("u".to_string()))),
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
    /// kernel-checked definition with a polymorphic motive sort. Ordinary registration
    /// preserves bare-Global application and its implicit universe instantiation.
    /// `Eq` and `refl` must already be registered.
    fn register_eq_recursor(ctx: &mut Context) {
        let (_levels, ty, body) = crate::recursor::derive_recursor_universe_poly(ctx, "Eq")
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
                ap(
                    ap(ap(ap(gl("Eq_rec_dep"), vr("A")), vr("x")), dep_motive),
                    vr("base"),
                ),
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
            lm(
                "_",
                eq3(vr("A"), vr("x"), vr("y'")),
                eq3(vr("A"), vr("y'"), vr("x")),
            ),
        );
        let refl_a_x = ap(ap(gl("refl"), vr("A")), vr("x"));
        let call = ap(
            ap(
                ap(
                    ap(ap(ap(gl("Eq_rec_dep"), vr("A")), vr("x")), motive),
                    refl_a_x,
                ),
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
            lm(
                "_",
                eq3(vr("A"), vr("y"), vr("z'")),
                eq3(vr("A"), vr("x"), vr("z'")),
            ),
        );
        let call = ap(
            ap(
                ap(
                    ap(ap(ap(gl("Eq_rec_dep"), vr("A")), vr("y")), motive),
                    vr("hxy"),
                ),
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
    /// Heterogeneous equality — `HEq A B a b`, the equation Lean's equation compiler, `Eq.rec`
    /// on an indexed family, and `cases`/`injection` on a dependent hypothesis all GENERATE and
    /// the source never writes.
    ///
    /// `Eq` takes ONE carrier, so a heterogeneous equation cannot be spelled as an `Eq` at either
    /// side: without this family there is a whole class of goals a translator cannot even STATE,
    /// which is worse than one it cannot prove.
    ///
    /// It costs the trusted base NOTHING. `HEq` is an inductive FAMILY with a checked
    /// constructor and a derived eliminator — declaring it ad hoc through the unchecked paths
    /// registers a name with no eliminator, i.e. a new axiom wearing an inductive's clothes.
    ///
    /// The shape is forced by the arity `Π(A B:Type). A → B → Prop`: LOGOS parameters are an
    /// arity PREFIX, and the constructor `HEq_refl : Π(A)(a:A). HEq A A a a` binds `A` and `a`
    /// but instantiates the second arity slot with `A` rather than binding it. So `A` is the one
    /// uniform parameter and `B`, `x`, `y` are indices. (Lean orders the arity
    /// `{α} → α → {β} → β → Prop`, which makes `α` and `a` BOTH parameters; ours is the more
    /// general elimination — the motive abstracts `x` as well.)
    ///
    /// Large elimination holds for the same reason `Eq`'s does: `HEq` is a single-constructor
    /// `Prop` whose one non-propositional field `a` reappears verbatim among the result's
    /// indices, so [`crate::type_checker::is_subsingleton_prop`] admits it and `HEq_rec`'s motive
    /// lands in `Type 0`.
    ///
    /// `heq_of_eq` — the direction that IS derivable at a `Type 0`-pinned `Eq` — is PROVED by J.
    /// Its converse `eq_of_heq` is not derivable here and is deliberately NOT assumed: Lean
    /// proves it by `cast`ing along an equation `α = β` BETWEEN CARRIERS, which needs `Eq` at a
    /// `Type 1` carrier (universe-polymorphic `Eq`). Assuming it instead would be Coq's
    /// `JMeq_eq` — an axiom — and would hide the real dependency.
    fn register_heq(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let prop = || Term::Sort(Universe::Prop);
        let v = |s: &str| Term::Var(s.to_string());
        let heq = |a: Term, b: Term, x: Term, y: Term| apps_(gl("HEq"), &[a, b, x, y]);

        // HEq : Π(A B : Type 0). A → B → Prop — one uniform parameter, three indices.
        ctx.add_indexed_inductive(
            "HEq",
            pi_(
                "A",
                type0(),
                pi_("B", type0(), arrow_(v("A"), arrow_(v("B"), prop()))),
            ),
            1,
        );

        // HEq_refl : Π(A:Type). Π(a:A). HEq A A a a
        Self::checked_ctor(
            ctx,
            "HEq_refl",
            "HEq",
            pi_(
                "A",
                type0(),
                pi_("a", v("A"), heq(v("A"), v("A"), v("a"), v("a"))),
            ),
        );

        // HEq_rec — the dependent eliminator, a kernel-checked definition.
        //
        // Its TYPE comes from `derive_recursor`, so the shape is the canonical one the kernel
        // itself computes (and calling it also holds `HEq` to the kernel's own eliminability
        // rules — a family that would not derive must not get a hand-written eliminator).
        //
        // Its BODY is a plain `Match`, not the derived `Fix`. `HEq` is NON-RECURSIVE: `HEq_refl`
        // has no recursive occurrence, so the fixpoint is dead weight — and worse than dead.
        // `derive_recursor` wraps every eliminator in a `Fix` whose recursion argument is the
        // FIRST INDEX, and the kernel unfolds a `Fix` only when that argument is in constructor
        // form. `HEq`'s first index is `B : Type 0`, which is never a constructor, so the derived
        // body is stuck forever: `HEq_rec … (HEq_refl A a)` type-checks and does not ι-reduce.
        // A `Match` on a constructor-form discriminant reduces immediately, so the eliminator
        // COMPUTES — which is the difference between an eliminator and an axiom with a body.
        let (rec_ty, _derived) = crate::recursor::derive_recursor(ctx, "HEq")
            .expect("HEq's dependent eliminator must derive");
        let motive_ty = pi_(
            "B",
            type0(),
            pi_(
                "x",
                v("A"),
                pi_(
                    "y",
                    v("B"),
                    arrow_(heq(v("A"), v("B"), v("x"), v("y")), type0()),
                ),
            ),
        );
        let base_ty = pi_(
            "a",
            v("A"),
            apps_(
                v("P"),
                &[
                    v("A"),
                    v("a"),
                    v("a"),
                    apps_(gl("HEq_refl"), &[v("A"), v("a")]),
                ],
            ),
        );
        let match_motive = lam_(
            "i0",
            type0(),
            lam_(
                "i1",
                v("A"),
                lam_(
                    "i2",
                    v("i0"),
                    lam_(
                        "ih",
                        heq(v("A"), v("i0"), v("i1"), v("i2")),
                        apps_(v("P"), &[v("i0"), v("i1"), v("i2"), v("ih")]),
                    ),
                ),
            ),
        );
        let rec_body = lam_(
            "A",
            type0(),
            lam_(
                "P",
                motive_ty,
                lam_(
                    "f0",
                    base_ty,
                    lam_(
                        "B",
                        type0(),
                        lam_(
                            "x",
                            v("A"),
                            lam_(
                                "y",
                                v("B"),
                                lam_(
                                    "h",
                                    heq(v("A"), v("B"), v("x"), v("y")),
                                    Term::Match {
                                        discriminant: Box::new(v("h")),
                                        motive: Box::new(match_motive),
                                        cases: vec![v("f0")],
                                    },
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        );
        ctx.add_definition("HEq_rec".to_string(), rec_ty, rec_body);

        // heq_of_eq : Π(A:Type). Π(a b:A). Eq A a b → HEq A A a b
        //   := λA a b h. Eq_rec_dep A a (λy. λ_. HEq A A a y) (HEq_refl A a) b h
        let heq_of_eq_ty = pi_(
            "A",
            type0(),
            pi_(
                "a",
                v("A"),
                pi_(
                    "b",
                    v("A"),
                    arrow_(
                        apps_(gl("Eq"), &[v("A"), v("a"), v("b")]),
                        heq(v("A"), v("A"), v("a"), v("b")),
                    ),
                ),
            ),
        );
        let motive = lam_(
            "y",
            v("A"),
            lam_(
                "_",
                apps_(gl("Eq"), &[v("A"), v("a"), v("y")]),
                heq(v("A"), v("A"), v("a"), v("y")),
            ),
        );
        let heq_of_eq_body = lam_(
            "A",
            type0(),
            lam_(
                "a",
                v("A"),
                lam_(
                    "b",
                    v("A"),
                    lam_(
                        "h",
                        apps_(gl("Eq"), &[v("A"), v("a"), v("b")]),
                        apps_(
                            gl("Eq_rec_dep"),
                            &[
                                v("A"),
                                v("a"),
                                motive,
                                apps_(gl("HEq_refl"), &[v("A"), v("a")]),
                                v("b"),
                                v("h"),
                            ],
                        ),
                    ),
                ),
            ),
        );
        ctx.add_definition("heq_of_eq".to_string(), heq_of_eq_ty, heq_of_eq_body);

        // ---- cast, and `eq_of_heq` — the bridge BACK, PROVED (Init/Prelude.lean:464) -------
        //
        // Lean writes
        //
        //   theorem eq_of_heq {α : Sort u} {a a' : α} (h : HEq a a') : Eq a a' :=
        //     have : (α β : Sort u) → (a : α) → (b : β) → HEq a b → (h : Eq α β) → Eq (cast h a) b :=
        //       fun _ _ _ _ h₁ => h₁.rec (fun _ => rfl)
        //     this α α a a' h rfl
        //
        // and the `rfl` in the `HEq.refl` case is where the whole argument lives: it asks the
        // kernel to accept `cast h a ≡ a` for an ARBITRARY `h : α = α`. Lean's kernel does that by
        // K-like ι-reduction; LOGOS's fires the `Eq` ι-rule on a syntactic `refl` only, and an
        // earlier wave concluded from that the theorem was unreachable and left the gap open.
        //
        // It is reachable, by the OTHER door the same fact comes through. LOGOS's conversion is
        // PROOF-IRRELEVANT, so `hc : Eq (Type 0) A A` and `refl (Type 0) A` — two proofs of one
        // proposition — are already definitionally equal, and therefore
        //
        //     refl (Eq (Type 0) A A) hc  :  Eq (Eq (Type 0) A A) hc (refl (Type 0) A)
        //
        // TYPE-CHECKS. That is UIP at the carrier equation, as a theorem rather than as a
        // reduction rule, and once it is a propositional equation it can be SUBSTITUTED ALONG:
        // rewrite `hc` to `refl` inside `Eq A (cast A A hc a) a`, and the goal becomes
        // `Eq A (cast A A (refl (Type 0) A) a) a`, whose cast ι-reduces because the proof standing
        // there is now syntactically `refl`. `refl A a` closes it.
        //
        // So `eq_of_heq` costs the trusted base NOTHING and is not Coq's `JMeq_eq`: no axiom is
        // added, the K-like reduction rule is not added either, and the step that was missing is
        // supplied by proof irrelevance — which the kernel already has and which is exactly the
        // rule Lean's K-like ι-reduction is itself justified by.
        //
        // `cast` is Lean's own (`Init/Prelude.lean`, `cast : α = β → α → β`), registered here
        // because `eq_of_heq`'s statement is unwriteable without it and because every `HEq`
        // elimination in translated Lean source mentions it.
        let cast_ty = pi_(
            "A",
            type0(),
            pi_(
                "B",
                type0(),
                arrow_(
                    apps_(gl("Eq"), &[type0(), v("A"), v("B")]),
                    arrow_(v("A"), v("B")),
                ),
            ),
        );
        let cast_at = |a: Term, b: Term, h: Term, x: Term| {
            apps_(
                gl("Eq_rec_dep"),
                &[
                    type0(),
                    a.clone(),
                    lam_(
                        "y",
                        type0(),
                        lam_(
                            "_hy",
                            apps_(gl("Eq"), &[type0(), a.clone(), v("y")]),
                            v("y"),
                        ),
                    ),
                    x,
                    b,
                    h,
                ],
            )
        };
        ctx.add_definition(
            "cast".to_string(),
            cast_ty,
            lam_(
                "A",
                type0(),
                lam_(
                    "B",
                    type0(),
                    lam_(
                        "hc",
                        apps_(gl("Eq"), &[type0(), v("A"), v("B")]),
                        lam_("x", v("A"), cast_at(v("A"), v("B"), v("hc"), v("x"))),
                    ),
                ),
            ),
        );

        // cast_refl : Π(A:Type). Π(hc : Eq (Type 0) A A). Π(a:A). Eq A (cast A A hc a) a
        //
        // UIP at the carrier, then substitution. The `refl (Eq …) hc` below is the load-bearing
        // step and it type-checks ONLY because conversion is proof-irrelevant — it is the theorem
        // "every proof of `A = A` is `rfl`", which is Streicher's K, held as a PROPOSITION here
        // rather than as a reduction rule.
        let carrier_eq = || apps_(gl("Eq"), &[type0(), v("A"), v("A")]);
        let refl_a = || apps_(gl("refl"), &[type0(), v("A")]);
        let cast_refl_ty = pi_(
            "A",
            type0(),
            pi_(
                "hc",
                carrier_eq(),
                pi_(
                    "a",
                    v("A"),
                    apps_(
                        gl("Eq"),
                        &[
                            v("A"),
                            apps_(gl("cast"), &[v("A"), v("A"), v("hc"), v("a")]),
                            v("a"),
                        ],
                    ),
                ),
            ),
        );
        let cast_refl_body = lam_(
            "A",
            type0(),
            lam_(
                "hc",
                carrier_eq(),
                lam_(
                    "a",
                    v("A"),
                    apps_(
                        gl("Eq_rec_dep"),
                        &[
                            carrier_eq(),
                            refl_a(),
                            lam_(
                                "e",
                                carrier_eq(),
                                lam_(
                                    "_he",
                                    apps_(gl("Eq"), &[carrier_eq(), refl_a(), v("e")]),
                                    apps_(
                                        gl("Eq"),
                                        &[
                                            v("A"),
                                            apps_(gl("cast"), &[v("A"), v("A"), v("e"), v("a")]),
                                            v("a"),
                                        ],
                                    ),
                                ),
                            ),
                            apps_(gl("refl"), &[v("A"), v("a")]),
                            v("hc"),
                            apps_(
                                gl("Eq_sym"),
                                &[
                                    carrier_eq(),
                                    v("hc"),
                                    refl_a(),
                                    apps_(gl("refl"), &[carrier_eq(), v("hc")]),
                                ],
                            ),
                        ],
                    ),
                ),
            ),
        );
        ctx.add_definition("cast_refl".to_string(), cast_refl_ty, cast_refl_body);

        // eq_of_heq : Π(A:Type). Π(a b:A). HEq A A a b → Eq A a b
        //
        //   := λA a b h. HEq_rec A
        //        (λB x y _. Π(hc : Eq (Type 0) A B). Eq B (cast A B hc x) y)
        //        (λz hc. cast_refl A hc z)
        //        A a b h (refl (Type 0) A)
        //
        // Lean's motive exactly (`Init/Prelude.lean:465`), discharged by `cast_refl` instead of by
        // the kernel's K-like ι-rule.
        let heq_mot = lam_(
            "B",
            type0(),
            lam_(
                "x",
                v("A"),
                lam_(
                    "y",
                    v("B"),
                    lam_(
                        "_hxy",
                        heq(v("A"), v("B"), v("x"), v("y")),
                        pi_(
                            "hc",
                            apps_(gl("Eq"), &[type0(), v("A"), v("B")]),
                            apps_(
                                gl("Eq"),
                                &[
                                    v("B"),
                                    apps_(gl("cast"), &[v("A"), v("B"), v("hc"), v("x")]),
                                    v("y"),
                                ],
                            ),
                        ),
                    ),
                ),
            ),
        );
        let eq_of_heq_ty = pi_(
            "A",
            type0(),
            pi_(
                "a",
                v("A"),
                pi_(
                    "b",
                    v("A"),
                    arrow_(
                        heq(v("A"), v("A"), v("a"), v("b")),
                        apps_(gl("Eq"), &[v("A"), v("a"), v("b")]),
                    ),
                ),
            ),
        );
        let eq_of_heq_body = lam_(
            "A",
            type0(),
            lam_(
                "a",
                v("A"),
                lam_(
                    "b",
                    v("A"),
                    lam_(
                        "h",
                        heq(v("A"), v("A"), v("a"), v("b")),
                        Term::App(
                            Box::new(apps_(
                                gl("HEq_rec"),
                                &[
                                    v("A"),
                                    heq_mot,
                                    lam_(
                                        "z",
                                        v("A"),
                                        lam_(
                                            "hz",
                                            carrier_eq(),
                                            apps_(gl("cast_refl"), &[v("A"), v("hz"), v("z")]),
                                        ),
                                    ),
                                    v("A"),
                                    v("a"),
                                    v("b"),
                                    v("h"),
                                ],
                            )),
                            Box::new(refl_a()),
                        ),
                    ),
                ),
            ),
        );
        ctx.add_definition("eq_of_heq".to_string(), eq_of_heq_ty, eq_of_heq_body);
    }

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

    /// `Ex.{u} : Π(A : Sort u). (A → Prop) → Prop`
    /// `witness.{u} : Π(A : Sort u). Π(P : A → Prop). Π(x : A). P x → Ex A P`
    ///
    /// The existential, over EVERY domain Lean allows. `∃ α : Type, α = Nat` is ordinary Lean —
    /// `Exists` there is `{α : Sort u}` — and a domain binder pinned to `Sort (Type 0)` refuses
    /// it outright: `Sort (Type 0) : Type 1`, and the application rule's `check_type` reports a
    /// `Type1`-against-`Type0` mismatch that reads like a universe bug rather than the
    /// expressivity gap it is.
    ///
    /// The `Prop` CODOMAIN is literal and stays that way: `Ex` landing in `Prop` over an
    /// arbitrary domain is exactly the impredicativity `imax` already gives, and it is what makes
    /// `∃ α : Type, …` a proposition rather than a `Type 1` datum.
    ///
    /// **No new primitive is needed for this.** `type_checker::implicit_levels_for_head` turns the
    /// free level variables of a bare `Term::Global` head's type into fresh metavariables at the
    /// application — the one place there is an argument to solve them from — and
    /// `solve_domain_levels` reads the solution off the argument's actual type. So `Ex Nat P`
    /// solves `u := 0` and `Ex Type0 P` solves `u := 1`, and a WRONG choice still fails, because
    /// `check_type` runs against the substituted domain afterwards. The de Bruijn re-checker does
    /// the same at its own application rule, from the same specification.
    fn register_ex(ctx: &mut Context) {
        // Ex.{u} : Π(A : Sort u). (A → Prop) → Prop
        let ex_type = Term::Pi {
            param: "A".to_string(),
            param_type: Box::new(Term::Sort(Universe::Var("u".to_string()))),
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
            param_type: Box::new(Term::Sort(Universe::Var("u".to_string()))),
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

    /// The deep embedding: `Univ` and `Syntax`, the operations `reflection` defines over them
    /// (checked definitions the native rules accelerate), and the ones that remain assumptions with
    /// computation rules (`syn_step`, `syn_eval`, `concludes`, the tactics and their combinators).
    fn register_reflection(ctx: &mut Context) {
        Self::register_univ(ctx);
        Self::register_syntax(ctx);
        crate::reflection::register(ctx);
        Self::register_syn_step(ctx);
        Self::register_syn_eval(ctx);
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
        Self::register_try_decide(ctx);
        Self::register_try_intro_tactics(ctx);
        Self::register_tactic_sequencing(ctx);
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

    // -------------------------------------------------------------------------
    // De Bruijn Operations (Substitution)
    // -------------------------------------------------------------------------

    // -------------------------------------------------------------------------
    // Beta Reduction (Computation)
    // -------------------------------------------------------------------------

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

    // -------------------------------------------------------------------------
    // Diagonalization (Self-Reference)
    // -------------------------------------------------------------------------

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

    /// try_decide : Syntax -> Derivation
    ///
    /// The `decide` tactic — decides a decidable proposition by reflection + computation. Reuses the
    /// `DAutoSolve` proof constructor (registered by [`register_try_auto`]), so only the tactic
    /// symbol needs declaring here.
    fn register_try_decide(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());
        ctx.add_declaration(
            "try_decide",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax),
                body_type: Box::new(derivation),
            },
        );
    }

    /// `try_split`/`try_left`/`try_right` : Syntax -> Derivation — the ∧- and ∨-introduction
    /// tactics (`split`, `left`, `right`). All reuse the `DAutoSolve` proof constructor.
    fn register_try_intro_tactics(ctx: &mut Context) {
        for name in ["try_split", "try_left", "try_right", "try_assumption"] {
            let syntax = Term::Global("Syntax".to_string());
            let derivation = Term::Global("Derivation".to_string());
            ctx.add_declaration(
                name,
                Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax),
                    body_type: Box::new(derivation),
                },
            );
        }
    }

    /// Tactic sequencing: `try_script : Syntax -> Syntax -> Derivation`. A `## Theorem … Proof:
    /// intro. <closer>.` parses to `try_script <name-list> <goal>`, where the name-list is a
    /// reflected, inert list of tactic names; its conclusion is decided by evaluating the script
    /// against the threaded goal + hypotheses (see `try_concludes_reduce` / `eval_script_names`).
    fn register_tactic_sequencing(ctx: &mut Context) {
        let syntax = Term::Global("Syntax".to_string());
        let derivation = Term::Global("Derivation".to_string());
        ctx.add_declaration(
            "try_script",
            Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(syntax.clone()),
                body_type: Box::new(Term::Pi {
                    param: "_".to_string(),
                    param_type: Box::new(syntax),
                    body_type: Box::new(derivation),
                }),
            },
        );
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

        // BVec : Nat -> Type 0 — the `Nat` is an INDEX, not a uniform parameter.
        //
        // `BVNil : BVec Zero` and `BVCons : Bit -> Π(n:Nat). BVec n -> BVec (Succ n)` target
        // DIFFERENT lengths, which is the definition of an index. Registering it through the
        // unchecked `add_inductive` recorded no `inductive_params` entry, so
        // `Context::inductive_num_params` fell back to the arity and answered 1 — the whole arity
        // treated as a parameter. The consequence was that a `Match` whose motive abstracts the
        // LENGTH — the only kind of match a length-indexed bitvector exists for — was untypeable,
        // while a match that ignored the length was accepted with branches whose result lengths
        // did not agree with their patterns.
        let bvec_type = Term::Pi {
            param: "n".to_string(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(type0),
        };
        ctx.add_indexed_inductive("BVec", bvec_type, 0);

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
                        Term::Var("b".to_string()),     // B1 case: return b
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
                        Term::Var("b".to_string()),     // B0 case: return b
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

        let bit = Term::Global("Bit".to_string());

        for (name, rec, op) in [
            ("bv_and", "bv_and_rec", "bit_and"),
            ("bv_or", "bv_or_rec", "bit_or"),
            ("bv_xor", "bv_xor_rec", "bit_xor"),
        ] {
            let body = Self::make_bvec_binop_fix(rec, op, &nat, &bvec_n, &bit);
            ctx.add_definition(name.to_string(), bv_binop_type.clone(), body);
        }

        let bv_not_body = Self::make_bvec_unop_fix("bv_not_rec", "bit_not", &nat, &bvec_n, &bit);
        ctx.add_definition("bv_not".to_string(), bv_unop_type, bv_not_body);
    }

    /// `λ(k:Nat). λ(_:BVec k). <result k>` — the motive shape an INDEXED `BVec` match takes:
    /// it abstracts the length as well as the vector, so each branch is checked at its own
    /// length. This is the whole difference the index makes, and it is why every branch below
    /// returns a vector whose length matches its pattern rather than whatever the scrutinee had.
    fn bvec_motive(result: Term) -> Term {
        lam_(
            "bvLen",
            gl("Nat"),
            lam_(
                "_",
                Term::App(Box::new(gl("BVec")), Box::new(v_("bvLen"))),
                result,
            ),
        )
    }

    /// `match k with Zero => Zero | Succ j => j` — `Nat.pred`, inline.
    ///
    /// Written in place rather than registered as a prelude constant: it exists only to spell
    /// the RESULT LENGTH of a tail projection (`BVec k → BVec (pred k)`), and a new global would
    /// be a name in the standard library that nothing else needs.
    fn nat_pred_of(k: Term) -> Term {
        Term::Match {
            discriminant: Box::new(k),
            motive: Box::new(lam_("_", gl("Nat"), gl("Nat"))),
            cases: vec![gl("Zero"), lam_("j", gl("Nat"), v_("j"))],
        }
    }

    /// `head w` — the leading bit of `w : BVec k`, `B0` at the empty vector.
    ///
    /// Total by construction, which is what makes it usable inside a length-indexed recursion:
    /// the empty case is REACHABLE at the type level even where the caller knows it cannot occur,
    /// and a partial projection would need an unreachability proof the matcher cannot synthesise.
    fn bvec_head_of(w: Term) -> Term {
        Term::Match {
            discriminant: Box::new(w),
            motive: Box::new(Self::bvec_motive(gl("Bit"))),
            cases: vec![
                gl("B0"),
                lam_(
                    "b",
                    gl("Bit"),
                    lam_(
                        "m",
                        gl("Nat"),
                        lam_(
                            "t",
                            Term::App(Box::new(gl("BVec")), Box::new(v_("m"))),
                            v_("b"),
                        ),
                    ),
                ),
            ],
        }
    }

    /// `tail w : BVec (pred k)` for `w : BVec k`. At `k = Succ m` the result type ι-reduces to
    /// `BVec m`, which is exactly the length the recursive call needs.
    fn bvec_tail_of(w: Term) -> Term {
        Term::Match {
            discriminant: Box::new(w),
            motive: Box::new(Self::bvec_motive(Term::App(
                Box::new(gl("BVec")),
                Box::new(Self::nat_pred_of(v_("bvLen"))),
            ))),
            cases: vec![
                gl("BVNil"),
                lam_(
                    "b",
                    gl("Bit"),
                    lam_(
                        "m",
                        gl("Nat"),
                        lam_(
                            "t",
                            Term::App(Box::new(gl("BVec")), Box::new(v_("m"))),
                            v_("t"),
                        ),
                    ),
                ),
            ],
        }
    }

    /// `bv_and`/`bv_or`/`bv_xor` — a pointwise fold over two vectors of the SAME length,
    /// written so that every branch's result length is the one its pattern forces.
    ///
    ///   fix rec. λn. λv1. λv2.
    ///     (match v1 return λ(k:Nat). λ(_:BVec k). BVec k → BVec k with
    ///      | BVNil            => λ_. BVNil
    ///      | BVCons b1 m t1   => λw. BVCons (op b1 (head w)) m (rec m t1 (tail w))) v2
    ///
    /// The shape is forced by the index, and the forcing is the point. Matching `v1` and then
    /// `v2` independently — the pre-index spelling — cannot type-check once the length is an
    /// index: after `v1` matches `BVCons b1 m t1` the result must be a `BVec (Succ m)`, and a
    /// second match on `v2` would have to produce that from a `BVNil` branch whose pattern says
    /// `BVec Zero`. Only Lean-style INDEX REFINEMENT (learning `n ≡ Succ m` inside the branch)
    /// would let the two matches be siblings, and `infer_indexed_match` does not refine.
    ///
    /// So the second vector is consumed by the total projections `head`/`tail` instead, at the
    /// length the FIRST vector fixed. Every branch is well-typed at its own index, the recursion
    /// still ι-computes on constructor-form vectors, and the denotation is unchanged.
    fn make_bvec_binop_fix(
        rec_name: &str,
        bit_op: &str,
        nat: &Term,
        bvec_n: &Term,
        bit: &Term,
    ) -> Term {
        let bvec_at = |k: Term| Term::App(Box::new(gl("BVec")), Box::new(k));
        // motive: λ(k:Nat). λ(_:BVec k). BVec k → BVec k
        let motive = Self::bvec_motive(arrow_(bvec_at(v_("bvLen")), bvec_at(v_("bvLen"))));

        // BVCons (bit_op b1 (head w)) m (rec m t1 (tail w))
        let combined = apps_(gl(bit_op), &[v_("b1"), Self::bvec_head_of(v_("w"))]);
        let recursed = apps_(
            v_(rec_name),
            &[v_("m"), v_("t1"), Self::bvec_tail_of(v_("w"))],
        );
        let cons_case = lam_(
            "b1",
            bit.clone(),
            lam_(
                "m",
                nat.clone(),
                lam_(
                    "t1",
                    bvec_at(v_("m")),
                    lam_(
                        "w",
                        bvec_at(Term::App(Box::new(gl("Succ")), Box::new(v_("m")))),
                        apps_(gl("BVCons"), &[combined, v_("m"), recursed]),
                    ),
                ),
            ),
        );
        let nil_case = lam_("_", bvec_at(gl("Zero")), gl("BVNil"));

        let dispatch = Term::Match {
            discriminant: Box::new(v_("v1")),
            motive: Box::new(motive),
            cases: vec![nil_case, cons_case],
        };

        Term::Fix {
            name: rec_name.to_string(),
            body: Box::new(lam_(
                "n",
                nat.clone(),
                lam_(
                    "v1",
                    bvec_n.clone(),
                    lam_(
                        "v2",
                        bvec_n.clone(),
                        Term::App(Box::new(dispatch), Box::new(v_("v2"))),
                    ),
                ),
            )),
        }
    }

    /// `bv_not` — the unary fold. A single vector needs no index refinement: each branch returns
    /// a vector at exactly its pattern's length.
    ///
    ///   fix rec. λn. λv.
    ///     match v return λ(k:Nat). λ(_:BVec k). BVec k with
    ///     | BVNil          => BVNil
    ///     | BVCons b m t   => BVCons (op b) m (rec m t)
    fn make_bvec_unop_fix(
        rec_name: &str,
        bit_op: &str,
        nat: &Term,
        bvec_n: &Term,
        bit: &Term,
    ) -> Term {
        let bvec_at = |k: Term| Term::App(Box::new(gl("BVec")), Box::new(k));
        let motive = Self::bvec_motive(bvec_at(v_("bvLen")));
        let cons_case = lam_(
            "b",
            bit.clone(),
            lam_(
                "m",
                nat.clone(),
                lam_(
                    "t",
                    bvec_at(v_("m")),
                    apps_(
                        gl("BVCons"),
                        &[
                            Term::App(Box::new(gl(bit_op)), Box::new(v_("b"))),
                            v_("m"),
                            apps_(v_(rec_name), &[v_("m"), v_("t")]),
                        ],
                    ),
                ),
            ),
        );
        Term::Fix {
            name: rec_name.to_string(),
            body: Box::new(lam_(
                "n",
                nat.clone(),
                lam_(
                    "v",
                    bvec_n.clone(),
                    Term::Match {
                        discriminant: Box::new(v_("v")),
                        motive: Box::new(motive),
                        cases: vec![gl("BVNil"), cons_case],
                    },
                ),
            )),
        }
    }

    // =========================================================================
    // THE STRUCTURAL TYPES LEAN CORE SHIPS
    // =========================================================================

    /// `Option`, `Prod`, `Sum`, `Sigma`, `Subtype`, `Fin`, `List`, `Array`, the machine widths,
    /// `Char`/`String`, `Iff`, `Subsingleton`/`Squash` and `Inhabited`/`Nonempty` — the layer a
    /// mathematician needs before a statement can even be WRITTEN.
    ///
    /// Three decisions run through all of it.
    ///
    /// **Everything is defined, nothing is assumed.** Not one of these is an
    /// `add_declaration`: they are inductives, structures and definitions with bodies, so every
    /// projection and every eliminator ι-reduces and the trusted base is exactly the size it was.
    /// An axiomatised `Array_get` would type-check and never compute, which turns a capability
    /// gap into a `rfl` that silently stops holding far from the declaration.
    ///
    /// **`Type 0` only, and said out loud.** Lean's `Prod` is
    /// `Type u → Type v → Type (max u v)`; `add_inductive` takes ONE fixed sort and there is no
    /// level-parameter list on an inductive, so every type here is its `Type 0` instance. That is
    /// a genuine restriction, not a monomorphisation trick: the names mean the `Type 0` family
    /// and nothing above it.
    ///
    /// **The bound is computed, never assumed.** `Fin` needs a `Nat` order, and every order
    /// primitive in the prelude is `Int → Int → Bool` with sixteen `Int` axioms behind it.
    /// Rather than a second axiom set for `Nat`, the bound uses the prelude's own shallow idiom:
    /// [`Self::register_nat_order`] defines `natLe`/`natLt` by structural recursion, and
    /// `Nat_lt i n` is `Eq Bool (natLt i n) true` — inhabited by `refl` exactly when the bound
    /// holds, and UNINHABITED when it does not, decided by ι-reduction.
    fn register_core_types(ctx: &mut Context) {
        Self::register_iff(ctx);
        Self::register_nat_order(ctx);
        Self::register_option(ctx);
        Self::register_prod(ctx);
        Self::register_sum(ctx);
        Self::register_sigma(ctx);
        Self::register_subtype(ctx);
        Self::register_fin(ctx);
        Self::register_list(ctx);
        Self::register_array(ctx);
        Self::register_machine_words(ctx);
        Self::register_char_and_string(ctx);
        Self::register_truncation(ctx);
        Self::register_inhabited(ctx);
        Self::register_ofnat(ctx);
    }

    /// Numeral dictionaries, generalized to Sort u; Lean's Type u instances use the
    /// corresponding successor level. The Nat dictionary is a checked definition.
    fn register_ofnat(ctx: &mut Context) {
        let v = |name: &str| Term::Var(name.to_string());
        let level = Universe::Var("u".to_string());
        ctx.add_structure_poly_at(
            "OfNat",
            vec!["u".to_string()],
            level.clone(),
            &[("A", Term::Sort(level)), ("n", gl("Nat"))],
            &[("ofNat", v("A"))],
        )
        .expect("the OfNat dictionary is well formed");
        let at_nat = |name: &str| Term::Const {
            name: name.to_string(),
            levels: vec![Universe::Type(0)],
        };
        let ty = pi_("n", gl("Nat"), apps_(at_nat("OfNat"), &[gl("Nat"), v("n")]));
        let body = lam_(
            "n",
            gl("Nat"),
            apps_(at_nat("OfNat_mk"), &[gl("Nat"), v("n"), v("n")]),
        );
        ctx.add_definition("instOfNatNat".to_string(), ty.clone(), body);
        ctx.add_fallback_instance(ty, gl("instOfNatNat"));
        ctx.set_binder_kinds(
            "OfNat_ofNat",
            vec![
                crate::elaborate::ParamKind::Implicit,
                crate::elaborate::ParamKind::Explicit,
                crate::elaborate::ParamKind::Instance,
            ],
        );
    }

    /// `Iff : Prop → Prop → Prop := λp q. And (p → q) (q → p)`.
    ///
    /// A DEFINITION, so `↔` is δ-transparent to the conjunction and `.mp`/`.mpr` are the `And`
    /// projections. A fresh inductive would be a different proposition that nothing — `propext`
    /// included — can bridge back to `And`, and an axiom would have no elimination at all.
    fn register_iff(ctx: &mut Context) {
        let prop = || Term::Sort(Universe::Prop);
        let v = |s: &str| Term::Var(s.to_string());
        ctx.add_definition(
            "Iff".to_string(),
            pi_("p", prop(), pi_("q", prop(), prop())),
            lam_(
                "p",
                prop(),
                lam_(
                    "q",
                    prop(),
                    apps_(gl("And"), &[arrow_(v("p"), v("q")), arrow_(v("q"), v("p"))]),
                ),
            ),
        );
    }

    /// The `Nat` order, as a COMPUTATION rather than an axiom set.
    ///
    /// ```text
    /// natLe : Nat → Nat → Bool := fix f. λm n. match m { 0 => true
    ///                                                  | m+1 => match n { 0 => false
    ///                                                                   | n+1 => f m n } }
    /// natLt m n := natLe (Succ m) n
    /// Nat_le m n : Prop := Eq Bool (natLe m n) true
    /// Nat_lt m n : Prop := Eq Bool (natLt m n) true
    /// ```
    ///
    /// Two properties earn this shape. The recursion stops as soon as EITHER operand reaches
    /// `Zero`, so a decision costs `min(m, n)` steps — a small index into a `2^64` range is
    /// cheap, which is what makes `UInt64` usable. And `natLt i Zero` reduces to `false` for an
    /// OPEN `i`, because the match is on `Succ i`, which is already a constructor application —
    /// so `Fin 0` is uninhabited by computation, with no lemma to prove.
    ///
    /// The literal bridge does the rest: `Literal::Nat(n)` peels one `Succ` per step, so the
    /// order works on compact literals of any size without materialising a Peano spine.
    fn register_nat_order(ctx: &mut Context) {
        let nat = || gl("Nat");
        let bool_ty = || gl("Bool");
        let v = |s: &str| Term::Var(s.to_string());
        let succ = |x: Term| Term::App(Box::new(gl("Succ")), Box::new(x));

        // match n { Zero => false, Succ n' => f m' n' }
        let inner = Term::Match {
            discriminant: Box::new(v("n")),
            motive: Box::new(lam_("_", nat(), bool_ty())),
            cases: vec![
                gl("false"),
                lam_("n✧", nat(), apps_(v("natLe✧"), &[v("m✧"), v("n✧")])),
            ],
        };
        // match m { Zero => true, Succ m' => inner }
        let outer = Term::Match {
            discriminant: Box::new(v("m")),
            motive: Box::new(lam_("_", nat(), bool_ty())),
            cases: vec![gl("true"), lam_("m✧", nat(), inner)],
        };
        let nat_le_body = Term::Fix {
            name: "natLe✧".to_string(),
            body: Box::new(lam_("m", nat(), lam_("n", nat(), outer))),
        };
        let binop = || pi_("_", nat(), pi_("_", nat(), bool_ty()));
        ctx.add_definition("natLe".to_string(), binop(), nat_le_body);

        // natLt m n := natLe (Succ m) n
        ctx.add_definition(
            "natLt".to_string(),
            binop(),
            lam_(
                "m",
                nat(),
                lam_("n", nat(), apps_(gl("natLe"), &[succ(v("m")), v("n")])),
            ),
        );

        // The Prop layer a bound proof inhabits: `Eq Bool (natLe m n) true`, the same shallow
        // idiom the Int order axioms are stated over.
        let prop_of = |op: &str| {
            lam_(
                "m",
                nat(),
                lam_(
                    "n",
                    nat(),
                    apps_(
                        gl("Eq"),
                        &[bool_ty(), apps_(gl(op), &[v("m"), v("n")]), gl("true")],
                    ),
                ),
            )
        };
        let rel = || pi_("_", nat(), pi_("_", nat(), Term::Sort(Universe::Prop)));
        ctx.add_definition("Nat_le".to_string(), rel(), prop_of("natLe"));
        ctx.add_definition("Nat_lt".to_string(), rel(), prop_of("natLt"));

        // THE DECISION PROCEDURES. `if n < 10 then … else …` is most of what an `if` in
        // mathematics branches on, and until now the prelude shipped decidable EQUALITY and
        // nothing else — the ordering could be stated and never decided.
        //
        //   decLeNat : Π(a b:Nat). Decidable (Nat_le a b)
        //   decLtNat : Π(a b:Nat). Decidable (Nat_lt a b)
        //
        // Both are DEFINITIONS, not axioms, and they cost the trusted base nothing: `Nat_le a b`
        // IS `Eq Bool (natLe a b) true`, so the decision is `decEqBool` at the computed Boolean.
        // `decEqBool` already carries both proofs — `refl` on the true side and the derived
        // `true ≠ false` refutation on the false side — so the `isFalse` branch receives a REAL
        // `Not (Nat_lt a b)` rather than an assumption, and the whole thing ι-reduces: `natLe`
        // computes the Boolean, `decEqBool` matches on it, and the branch is selected.
        //
        // Deliberately NOT a redefinition of the `Int` builtin `lt`. That builtin is an opaque
        // `Int → Int → Bool` and giving it a Nat ordering would make the gap look like a naming
        // problem while changing what every existing `Int` comparison means.
        let dec_of = |op: &str, rel_name: &str| {
            (
                pi_(
                    "a",
                    nat(),
                    pi_(
                        "b",
                        nat(),
                        Term::App(
                            Box::new(gl("Decidable")),
                            Box::new(apps_(gl(rel_name), &[v("a"), v("b")])),
                        ),
                    ),
                ),
                lam_(
                    "a",
                    nat(),
                    lam_(
                        "b",
                        nat(),
                        apps_(
                            gl("decEqBool"),
                            &[apps_(gl(op), &[v("a"), v("b")]), gl("true")],
                        ),
                    ),
                ),
            )
        };
        let (le_ty, le_body) = dec_of("natLe", "Nat_le");
        ctx.add_definition("decLeNat".to_string(), le_ty, le_body);
        let (lt_ty, lt_body) = dec_of("natLt", "Nat_lt");
        ctx.add_definition("decLtNat".to_string(), lt_ty, lt_body);
    }

    /// `Option A` — `none` registered BEFORE `some`, as in Lean, so a translator that preserves
    /// source order lands on the right branch.
    ///
    /// `Option` is Lean's core container. Registering it does not register a monad: `Bind`,
    /// `Pure` and the rest of the do-notation surface stay absent, because an axiomatised `bind`
    /// would never reduce.
    fn register_option(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let v = |s: &str| Term::Var(s.to_string());
        let opt_a = || Term::App(Box::new(gl("Option")), Box::new(v("A")));

        ctx.add_indexed_inductive("Option", pi_("A", type0(), type0()), 1);
        Self::checked_ctor(ctx, "none", "Option", pi_("A", type0(), opt_a()));
        Self::checked_ctor(
            ctx,
            "some",
            "Option",
            pi_("A", type0(), arrow_(v("A"), opt_a())),
        );
    }

    pub(crate) fn division_carrier_reference() -> &'static Context {
        static REFERENCE: std::sync::OnceLock<Context> = std::sync::OnceLock::new();
        REFERENCE.get_or_init(|| {
            let mut ctx = Context::new();
            ctx.with_equations_suspended(|ctx| {
                Self::register_bool(ctx);
                Self::register_prod(ctx);
            });
            ctx
        })
    }

    /// `Prod A B` with `fst`/`snd`, through `add_structure` — the only path that registers the
    /// [`crate::context::StructInfo`] gating definitional eta, so `⟨p.1, p.2⟩ ≡ p`.
    fn register_prod(ctx: &mut Context) {
        let type0 = Term::Sort(Universe::Type(0));
        let v = |s: &str| Term::Var(s.to_string());
        ctx.add_structure(
            "Prod",
            &[("A", type0.clone()), ("B", type0)],
            &[("fst", v("A")), ("snd", v("B"))],
        );
    }

    /// `Sum A B` with `Sum_inl`/`Sum_inr`.
    ///
    /// The injections are NOT spelled `inl`/`inr`: the prelude's disjunction `Or` already owns
    /// `left`/`right`, and `Or` is the Prop-valued one whose eliminator's motive is pinned to
    /// `Prop`. Keeping the two apart is what stops a `Type`-valued case analysis from being
    /// written against a proof.
    fn register_sum(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let v = |s: &str| Term::Var(s.to_string());
        let sum_ab = || apps_(gl("Sum"), &[v("A"), v("B")]);

        ctx.add_indexed_inductive("Sum", pi_("A", type0(), pi_("B", type0(), type0())), 2);
        for (name, field) in [("Sum_inl", "A"), ("Sum_inr", "B")] {
            Self::checked_ctor(
                ctx,
                name,
                "Sum",
                pi_("A", type0(), pi_("B", type0(), arrow_(v(field), sum_ab()))),
            );
        }
    }

    /// `Sigma A B` — the genuinely DEPENDENT pair: `snd`'s TYPE mentions `fst`'s VALUE, so
    /// `s.2 : B s.1`. Erasing that dependency gives a constant motive and the wrong projection
    /// type, which type-checks and means something else.
    fn register_sigma(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let v = |s: &str| Term::Var(s.to_string());
        ctx.add_structure(
            "Sigma",
            &[("A", type0()), ("B", arrow_(v("A"), type0()))],
            &[
                ("fst", v("A")),
                ("snd", Term::App(Box::new(v("B")), Box::new(v("fst")))),
            ],
        );
    }

    /// `PSigma A B` — Lean's `PSigma.{u, v}`, the dependent pair at ANY sorts: `A : Sort u` and
    /// `B : A → Sort v`, in `Sort (max 1 u v)`, the least sort that stores both, as
    /// `add_structure_poly` computes it. `Sigma` above is the `Type 0` pair a type-level
    /// development writes; this is the one `Quot_rec` is built over, because a quotient's carrier
    /// is `Sort u` and so is the first component of the pair `Quot_lift` targets.
    fn register_psigma(ctx: &mut Context) {
        let v = |s: &str| Term::Var(s.to_string());
        let sort = |level: &str| Term::Sort(Universe::Var(level.to_string()));
        ctx.add_structure_poly(
            "PSigma",
            vec!["u".to_string(), "v".to_string()],
            &[("A", sort("u")), ("B", arrow_(v("A"), sort("v")))],
            &[
                ("fst", v("A")),
                ("snd", Term::App(Box::new(v("B")), Box::new(v("fst")))),
            ],
        )
        .expect("`PSigma` is Lean's dependent pair at any sorts");
    }

    /// `Subtype A p` = `{x : A // p x}` — the value and a PROOF about that value, checked.
    /// `Subtype_property s : p (Subtype_val s)`, not `p` at some other element.
    fn register_subtype(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let prop = || Term::Sort(Universe::Prop);
        let v = |s: &str| Term::Var(s.to_string());
        ctx.add_structure(
            "Subtype",
            &[("A", type0()), ("p", arrow_(v("A"), prop()))],
            &[
                ("val", v("A")),
                ("property", Term::App(Box::new(v("p")), Box::new(v("val")))),
            ],
        );
    }

    /// `Fin n : Type 0 := Subtype Nat (λi. Nat_lt i n)` — Lean's own encoding, a `Nat` carrying
    /// a proof that it is below the bound.
    ///
    /// `Fin 0` is EMPTY without an appeal to a lemma: `Nat_lt i Zero` unfolds to
    /// `Eq Bool (natLe (Succ i) Zero) true`, and `natLe (Succ i) Zero` ι-reduces to `false` for
    /// every `i`, open or closed.
    fn register_fin(ctx: &mut Context) {
        let nat = || gl("Nat");
        let v = |s: &str| Term::Var(s.to_string());
        ctx.add_definition(
            "Fin".to_string(),
            pi_("_", nat(), Term::Sort(Universe::Type(0))),
            lam_(
                "n",
                nat(),
                apps_(
                    gl("Subtype"),
                    &[
                        nat(),
                        lam_("i", nat(), apps_(gl("Nat_lt"), &[v("i"), v("n")])),
                    ],
                ),
            ),
        );
    }

    /// Lean's `List A` — `nil` before `cons`, with the recursor and the three operations `Array`
    /// is built from.
    ///
    /// Deliberately NOT `TList`/`EList`. `EList` is monomorphic in `Entity` (the concrete target
    /// the induction tactic's generic eliminator needs) and `TList`'s constructors are spelled
    /// `TNil`/`TCons`; both stay exactly as they were. `List` is the name a Lean source writes,
    /// and a REPL program that declares its own `List` still overwrites this one.
    ///
    /// `List_rec`'s motive is `List A → Type 0`, not `List A → Sort u` — the same `Type 0`
    /// restriction as everything else in this section. A `Prop`-motive induction over a list goes
    /// through `Match`/`Fix` directly.
    fn register_list(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let nat = || gl("Nat");
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let list_a = || ap(gl("List"), v("A"));
        let succ = |x: Term| ap(gl("Succ"), x);
        let nil_a = || ap(gl("nil"), v("A"));
        let cons_a = |h: Term, t: Term| apps_(gl("cons"), &[v("A"), h, t]);

        ctx.add_indexed_inductive("List", pi_("A", type0(), type0()), 1);
        Self::checked_ctor(ctx, "nil", "List", pi_("A", type0(), list_a()));
        Self::checked_ctor(
            ctx,
            "cons",
            "List",
            pi_("A", type0(), arrow_(v("A"), arrow_(list_a(), list_a()))),
        );

        // List_rec : Π(A). Π(P : List A → Type 0). P nil →
        //            (Π(h:A). Π(t:List A). P t → P (cons h t)) → Π(l : List A). P l
        let motive_ty = || arrow_(list_a(), type0());
        let p_at = |x: Term| ap(v("P"), x);
        let cons_step = || {
            pi_(
                "h",
                v("A"),
                pi_(
                    "t",
                    list_a(),
                    arrow_(p_at(v("t")), p_at(cons_a(v("h"), v("t")))),
                ),
            )
        };
        let rec_ty = pi_(
            "A",
            type0(),
            pi_(
                "P",
                motive_ty(),
                arrow_(
                    p_at(nil_a()),
                    arrow_(cons_step(), pi_("l", list_a(), p_at(v("l")))),
                ),
            ),
        );
        let rec_match = Term::Match {
            discriminant: Box::new(v("l")),
            motive: Box::new(lam_("l✧", list_a(), p_at(v("l✧")))),
            cases: vec![
                v("pnil"),
                lam_(
                    "h",
                    v("A"),
                    lam_(
                        "t",
                        list_a(),
                        apps_(v("pcons"), &[v("h"), v("t"), ap(v("List_rec✧"), v("t"))]),
                    ),
                ),
            ],
        };
        ctx.add_definition(
            "List_rec".to_string(),
            rec_ty,
            lam_(
                "A",
                type0(),
                lam_(
                    "P",
                    motive_ty(),
                    lam_(
                        "pnil",
                        p_at(nil_a()),
                        lam_(
                            "pcons",
                            cons_step(),
                            fix_on_list("List_rec✧", list_a(), rec_match),
                        ),
                    ),
                ),
            ),
        );

        // List_length : Π(A). List A → Nat
        let len_match = Term::Match {
            discriminant: Box::new(v("l")),
            motive: Box::new(lam_("_", list_a(), nat())),
            cases: vec![
                gl("Zero"),
                lam_(
                    "_h",
                    v("A"),
                    lam_("t", list_a(), succ(ap(v("List_length✧"), v("t")))),
                ),
            ],
        };
        ctx.add_definition(
            "List_length".to_string(),
            pi_("A", type0(), arrow_(list_a(), nat())),
            lam_(
                "A",
                type0(),
                fix_on_list("List_length✧", list_a(), len_match),
            ),
        );

        // List_append : Π(A). List A → List A → List A
        let app_match = Term::Match {
            discriminant: Box::new(v("l")),
            motive: Box::new(lam_("_", list_a(), list_a())),
            cases: vec![
                v("r"),
                lam_(
                    "h",
                    v("A"),
                    lam_(
                        "t",
                        list_a(),
                        cons_a(v("h"), apps_(v("List_append✧"), &[v("t"), v("r")])),
                    ),
                ),
            ],
        };
        ctx.add_definition(
            "List_append".to_string(),
            pi_("A", type0(), arrow_(list_a(), arrow_(list_a(), list_a()))),
            lam_(
                "A",
                type0(),
                fix_on_list("List_append✧", list_a(), lam_("r", list_a(), app_match)),
            ),
        );

        Self::register_list_get(ctx);
    }

    /// `List_get : Π(A). Π(l : List A). Π(i : Nat). Nat_lt i (List_length A l) → A` — Lean's
    /// bound-proof indexing, TOTAL with no default and no out-of-range behaviour to choose.
    ///
    /// The two obligations the recursion has to discharge are both free, which is the whole point
    /// of stating the bound as `Eq Bool (natLt i len) true`:
    ///
    /// - In the `nil` branch the bound is `Nat_lt i Zero`, which ι-reduces to
    ///   `Eq Bool false true`. `Eq_false_true_absurd` turns it into `False` and `False_rec_type`
    ///   eliminates into `A` — the branch is unreachable, and it is unreachable by COMPUTATION.
    /// - In the `cons`/`Succ k` branch the recursive call needs `Nat_lt k len` where the
    ///   hypothesis is `Nat_lt (Succ k) (Succ len)`. Those unfold to
    ///   `natLe (Succ k) len` and `natLe (Succ (Succ k)) (Succ len)`, and the second ι-reduces to
    ///   the first — so the SAME proof is passed down with no transport and no lemma.
    fn register_list_get(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let nat = || gl("Nat");
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let list_a = || ap(gl("List"), v("A"));
        let len_of = |l: Term| apps_(gl("List_length"), &[v("A"), l]);
        let bound = |i: Term, l: Term| apps_(gl("Nat_lt"), &[i, len_of(l)]);
        let cons_list = || apps_(gl("cons"), &[v("A"), v("hd"), v("tl")]);

        Self::register_absurd_helpers(ctx);

        let get_ty = pi_(
            "A",
            type0(),
            pi_(
                "l",
                list_a(),
                pi_("i", nat(), arrow_(bound(v("i"), v("l")), v("A"))),
            ),
        );

        // nil branch: λi. λh. False_rec_type A (Eq_false_true_absurd h)
        let nil_case = lam_(
            "i",
            nat(),
            lam_(
                "h",
                bound(v("i"), ap(gl("nil"), v("A"))),
                apps_(
                    gl("False_rec_type"),
                    &[v("A"), ap(gl("Eq_false_true_absurd"), v("h"))],
                ),
            ),
        );

        // cons branch: λ(hd:A). λ(tl:List A). λ(i:Nat). λ(h : Nat_lt i (length (hd::tl))).
        //   (match i return (λi✧. Nat_lt i✧ (length (hd::tl)) → A)
        //      { Zero => λ_. hd, Succ k => λh✧. rec tl k h✧ }) h
        //
        // The match ABSTRACTS over the bound proof and is then applied to it, because the
        // proof's type mentions `i` and has to be refined per branch.
        let index_match = Term::Match {
            discriminant: Box::new(v("i")),
            motive: Box::new(lam_(
                "i✧",
                nat(),
                arrow_(bound(v("i✧"), cons_list()), v("A")),
            )),
            cases: vec![
                lam_("_h", bound(gl("Zero"), cons_list()), v("hd")),
                lam_(
                    "k",
                    nat(),
                    lam_(
                        "h✧",
                        bound(ap(gl("Succ"), v("k")), cons_list()),
                        apps_(v("List_get✧"), &[v("tl"), v("k"), v("h✧")]),
                    ),
                ),
            ],
        };
        let cons_case = lam_(
            "hd",
            v("A"),
            lam_(
                "tl",
                list_a(),
                lam_(
                    "i",
                    nat(),
                    lam_(
                        "h",
                        bound(v("i"), cons_list()),
                        Term::App(Box::new(index_match), Box::new(v("h"))),
                    ),
                ),
            ),
        );

        let list_match = Term::Match {
            discriminant: Box::new(v("l")),
            motive: Box::new(lam_(
                "l✧",
                list_a(),
                pi_("i", nat(), arrow_(bound(v("i"), v("l✧")), v("A"))),
            )),
            cases: vec![nil_case, cons_case],
        };
        ctx.add_definition(
            "List_get".to_string(),
            get_ty,
            lam_("A", type0(), fix_on_list("List_get✧", list_a(), list_match)),
        );
    }

    /// The two lemmas a bound-checked eliminator needs, both DERIVED.
    ///
    /// `False_rec_type : Π(A:Type 0). False → A` is the large elimination a zero-constructor
    /// `Prop` is entitled to, and `Eq_false_true_absurd : Eq Bool false true → False` is Bool
    /// no-confusion: transport the motive `λb. match b { true => False, false => True }` along
    /// the equation and feed it `I`.
    fn register_absurd_helpers(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let v = |s: &str| Term::Var(s.to_string());

        // False_rec_type : Π(A:Type 0). False → A
        ctx.add_definition(
            "False_rec_type".to_string(),
            pi_("A", type0(), arrow_(gl("False"), v("A"))),
            lam_(
                "A",
                type0(),
                lam_(
                    "h",
                    gl("False"),
                    Term::Match {
                        discriminant: Box::new(v("h")),
                        motive: Box::new(lam_("_", gl("False"), v("A"))),
                        cases: vec![],
                    },
                ),
            ),
        );

        // notTrue : Bool → Prop := λb. match b { true => False, false => True }
        // (Bool's constructors are registered `true` then `false`.)
        ctx.add_definition(
            "Bool_isFalse".to_string(),
            pi_("_", gl("Bool"), Term::Sort(Universe::Prop)),
            lam_(
                "b",
                gl("Bool"),
                Term::Match {
                    discriminant: Box::new(v("b")),
                    motive: Box::new(lam_("_", gl("Bool"), Term::Sort(Universe::Prop))),
                    cases: vec![gl("False"), gl("True")],
                },
            ),
        );

        // Eq_false_true_absurd : Eq Bool false true → False
        //   := λh. Eq_rec Bool false Bool_isFalse I true h
        let eq_ft = apps_(gl("Eq"), &[gl("Bool"), gl("false"), gl("true")]);
        ctx.add_definition(
            "Eq_false_true_absurd".to_string(),
            arrow_(eq_ft.clone(), gl("False")),
            lam_(
                "h",
                eq_ft,
                apps_(
                    gl("Eq_rec"),
                    &[
                        gl("Bool"),
                        gl("false"),
                        gl("Bool_isFalse"),
                        gl("I"),
                        gl("true"),
                        v("h"),
                    ],
                ),
            ),
        );
    }

    /// `Array A` — a structure over `List A`, exactly as Lean's is over `List α`, with `size`,
    /// `push` and a `get` that TAKES THE BOUND PROOF.
    ///
    /// The dangerous substitution T10-011 names is `Array ↦ TList`: the `foldl`/`map`/`size`
    /// interface lines up, so most sources translate without a type error while `Array.get` goes
    /// from indexing-with-a-proof to a recursion that must decide what to do out of range.
    /// `Array_get` has no out-of-range behaviour to decide, because there is no way to call it
    /// out of range.
    fn register_array(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let nat = || gl("Nat");
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let list_a = || ap(gl("List"), v("A"));
        let array_a = || ap(gl("Array"), v("A"));
        let data_of = |a: Term| apps_(gl("Array_data"), &[v("A"), a]);

        ctx.add_structure("Array", &[("A", type0())], &[("data", list_a())]);

        // Array_size : Π(A). Array A → Nat := λA a. List_length A a.data
        ctx.add_definition(
            "Array_size".to_string(),
            pi_("A", type0(), arrow_(array_a(), nat())),
            lam_(
                "A",
                type0(),
                lam_(
                    "a",
                    array_a(),
                    apps_(gl("List_length"), &[v("A"), data_of(v("a"))]),
                ),
            ),
        );

        // Array_push : Π(A). Array A → A → Array A
        //   := λA a x. Array_mk A (List_append A a.data (cons A x (nil A)))
        ctx.add_definition(
            "Array_push".to_string(),
            pi_("A", type0(), arrow_(array_a(), arrow_(v("A"), array_a()))),
            lam_(
                "A",
                type0(),
                lam_(
                    "a",
                    array_a(),
                    lam_(
                        "x",
                        v("A"),
                        apps_(
                            gl("Array_mk"),
                            &[
                                v("A"),
                                apps_(
                                    gl("List_append"),
                                    &[
                                        v("A"),
                                        data_of(v("a")),
                                        apps_(gl("cons"), &[v("A"), v("x"), ap(gl("nil"), v("A"))]),
                                    ],
                                ),
                            ],
                        ),
                    ),
                ),
            ),
        );

        // Array_get : Π(A). Π(a : Array A). Π(i : Nat). Nat_lt i (Array_size A a) → A
        //   := λA a i h. List_get A a.data i h
        //
        // `Array_size A a` δβ-unfolds to `List_length A a.data`, so the array's bound proof IS
        // the list's — no coercion, no restatement.
        let bound =
            |i: Term, a: Term| apps_(gl("Nat_lt"), &[i, apps_(gl("Array_size"), &[v("A"), a])]);
        ctx.add_definition(
            "Array_get".to_string(),
            pi_(
                "A",
                type0(),
                pi_(
                    "a",
                    array_a(),
                    pi_("i", nat(), arrow_(bound(v("i"), v("a")), v("A"))),
                ),
            ),
            lam_(
                "A",
                type0(),
                lam_(
                    "a",
                    array_a(),
                    lam_(
                        "i",
                        nat(),
                        lam_(
                            "h",
                            bound(v("i"), v("a")),
                            apps_(gl("List_get"), &[v("A"), data_of(v("a")), v("i"), v("h")]),
                        ),
                    ),
                ),
            ),
        );
    }

    /// `UInt8`/`UInt16`/`UInt32`/`UInt64`/`USize` as `Fin 2^N` — Lean's own definition.
    ///
    /// These are the bounded CARRIERS. Their wrapping arithmetic (`(255 : UInt8) + 1 = 0`) is a
    /// separate capability and is NOT provided: LOGOS's arithmetic is exact and unbounded by
    /// construction, so a modular operation has to be defined rather than inherited, and defining
    /// it needs `Nat` `+`/`%` plus `Nat.mod_lt`. What IS provided is the invariant — a value of
    /// one of these types carries a proof that it is below the width, so the carriers cannot be
    /// confused with `Nat` and cannot hold an out-of-range value.
    fn register_machine_words(ctx: &mut Context) {
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let lit = |s: &str| {
            Term::Lit(Literal::Nat(
                logicaffeine_base::BigInt::parse_decimal(s).expect("a decimal width"),
            ))
        };
        for (name, size) in [
            ("UInt8", "256"),
            ("UInt16", "65536"),
            ("UInt32", "4294967296"),
            ("UInt64", "18446744073709551616"),
            ("USize", "18446744073709551616"),
        ] {
            ctx.add_definition(
                name.to_string(),
                Term::Sort(Universe::Type(0)),
                ap(gl("Fin"), lit(size)),
            );
        }

        // UInt32_toNat : UInt32 → Nat — `Subtype.val` at the width, the projection `Char`'s
        // validity predicate reads its argument through.
        let v = |s: &str| Term::Var(s.to_string());
        let nat = || gl("Nat");
        ctx.add_definition(
            "UInt32_toNat".to_string(),
            pi_("_", gl("UInt32"), nat()),
            lam_(
                "c",
                gl("UInt32"),
                apps_(
                    gl("Subtype_val"),
                    &[
                        nat(),
                        lam_(
                            "i",
                            nat(),
                            apps_(gl("Nat_lt"), &[v("i"), lit("4294967296")]),
                        ),
                        v("c"),
                    ],
                ),
            ),
        );
    }

    /// `Char = { val : UInt32 // val.isValidChar }` and `String` as a structure over
    /// `List Char` — Lean's definitions, both of them.
    ///
    /// `Char.isValidChar n` is `n < 0xD800 ∨ (0xDFFF < n ∧ n < 0x110000)`: the surrogate range is
    /// excluded, so a `Char` really is a Unicode scalar value and not a `UInt32` wearing a label.
    /// The two re-encodings the corpus names are both dead ends — a `Nat` `Char` makes
    /// `Nat → Char` and `Nat → Nat` the same type, and a one-character `Text` has nothing to
    /// project, `Text` being opaque.
    ///
    /// `Text` STAYS OPAQUE. It is the carrier `Literal::Text` lands in, and giving it
    /// constructors would give the same string two incompatible normal forms — the literal and
    /// its cons-list spelling. `String` and `Text` are therefore different types here, and
    /// bridging them needs a primitive reduction rule (`"ab" ++ "cd" ⤳ "abcd"`) that lives in
    /// the reducer, not in the prelude.
    fn register_char_and_string(ctx: &mut Context) {
        let prop = || Term::Sort(Universe::Prop);
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
        let lit = |s: &str| {
            Term::Lit(Literal::Nat(
                logicaffeine_base::BigInt::parse_decimal(s).expect("a decimal code point"),
            ))
        };
        let n_of = || ap(gl("UInt32_toNat"), v("c"));
        let lt = |a: Term, b: Term| apps_(gl("Nat_lt"), &[a, b]);

        // Char_isValid : UInt32 → Prop
        ctx.add_definition(
            "Char_isValid".to_string(),
            pi_("_", gl("UInt32"), prop()),
            lam_(
                "c",
                gl("UInt32"),
                apps_(
                    gl("Or"),
                    &[
                        lt(n_of(), lit("55296")),
                        apps_(
                            gl("And"),
                            &[lt(lit("57343"), n_of()), lt(n_of(), lit("1114112"))],
                        ),
                    ],
                ),
            ),
        );

        // Char : Type 0 := Subtype UInt32 Char_isValid
        ctx.add_definition(
            "Char".to_string(),
            Term::Sort(Universe::Type(0)),
            apps_(gl("Subtype"), &[gl("UInt32"), gl("Char_isValid")]),
        );

        // structure String where data : List Char
        ctx.add_structure("String", &[], &[("data", ap(gl("List"), gl("Char")))]);
    }

    /// `Subsingleton` as a `Prop`-valued CLASS, and `Squash` as truncation that stays in `Type`.
    ///
    /// `Subsingleton` goes through [`Context::add_structure_at`] at `Prop`, not `add_structure`:
    /// the inferred sort floors at `Type 0`, and a `Type 0` `Subsingleton α` inside a
    /// proposition's statement promotes that statement out of `Prop` through `imax`. The declared
    /// sort is CHECKED by `check_constructor_universes`, so this is a claim the kernel verifies
    /// rather than a sort taken on faith.
    ///
    /// The kernel's own subsingleton notion — read off an inductive's constructor list to decide
    /// large elimination — is a property of a DECLARATION and is not addressable from a term.
    /// The class is, which is why Mathlib derives instances of it and the kernel rule cannot
    /// stand in: `Unit` is a `Type`, so proof irrelevance does not apply and `Subsingleton Unit`
    /// must be PROVED.
    ///
    /// `Squash A := Quot A (λ_ _. True)` — every two elements are related, so the quotient has at
    /// most one class, and it is a `Type`. The `Prop`-valued truncation is `Nonempty`, and
    /// eliminating THAT into `Type` is choice.
    fn register_truncation(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let v = |s: &str| Term::Var(s.to_string());

        let elim_field = pi_(
            "x",
            v("a"),
            pi_("y", v("a"), apps_(gl("Eq"), &[v("a"), v("x"), v("y")])),
        );
        ctx.add_structure_at(
            "Subsingleton",
            Universe::Prop,
            &[("a", type0())],
            &[("elim", elim_field)],
        )
        .expect("`Subsingleton` is a Prop-valued class: its only field is an equation");

        ctx.add_definition(
            "Squash".to_string(),
            pi_("_", type0(), type0()),
            lam_(
                "A",
                type0(),
                apps_(
                    gl("Quot"),
                    &[v("A"), lam_("_", v("A"), lam_("_", v("A"), gl("True")))],
                ),
            ),
        );
    }

    /// `Inhabited` (DATA — a chosen element) and `Nonempty` (a PROPOSITION — that one exists).
    ///
    /// The difference is the whole point, and it is a sort difference. `Inhabited` goes through
    /// `add_structure`, which is what registers the `StructInfo` gating eta, and lands in
    /// `Type 0` because it stores an `α`. `Nonempty` must land in `Prop`, so it cannot go through
    /// `add_structure` at all: it is an indexed inductive at `Prop` with a checked constructor.
    ///
    /// A `Type 0` `Nonempty` would be eligible for large elimination — and a `Nonempty α` that
    /// yields its witness IS `Classical.choice`, derived rather than declared, which the axiom
    /// census would never see. The kernel refuses it because `Nonempty`'s single constructor
    /// takes DATA, so it is not a subsingleton.
    fn register_inhabited(ctx: &mut Context) {
        let type0 = || Term::Sort(Universe::Type(0));
        let prop = || Term::Sort(Universe::Prop);
        let v = |s: &str| Term::Var(s.to_string());
        let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));

        ctx.add_structure("Inhabited", &[("a", type0())], &[("default", v("a"))]);

        // `instance : Inhabited Nat`, in the FALLBACK tier.
        //
        // The prelude refused this for a wave, and the refusal was right at the time:
        // `Context::instances()` is a push-only Vec resolved FIRST-MATCH, so a prelude entry
        // would have been entry zero and would have permanently outranked every user instance
        // for the same goal — and it would have moved `instances().len()`, which is a count
        // taken against a baseline of nothing.
        //
        // [`Context::add_fallback_instance`] is the tier that unblocks it, and it is strictly
        // better than Lean's answer to the same problem: Lean expresses "library instance, low
        // priority" with a NUMBER a user instance can accidentally undercut, while a separate
        // tier consulted only after the main scan has failed cannot be undercut at all. The
        // entry stays out of `instances()`, which remains the faithful record of what was
        // explicitly declared.
        ctx.add_fallback_instance(
            ap(gl("Inhabited"), gl("Nat")),
            apps_(gl("Inhabited_mk"), &[gl("Nat"), gl("Zero")]),
        );
        ctx.add_fallback_instance(
            ap(gl("Inhabited"), gl("Bool")),
            apps_(gl("Inhabited_mk"), &[gl("Bool"), gl("true")]),
        );

        ctx.add_indexed_inductive("Nonempty", pi_("a", type0(), prop()), 1);
        Self::checked_ctor(
            ctx,
            "Nonempty_intro",
            "Nonempty",
            pi_("a", type0(), arrow_(v("a"), ap(gl("Nonempty"), v("a")))),
        );
    }

    /// Register a prelude constructor through the CHECKED path — positivity and the
    /// constructor-universe rule — so a prelude inductive is held to the same standard a user
    /// declaration is. A failure here is a bug in this file, not in the caller's input.
    fn checked_ctor(ctx: &mut Context, name: &str, inductive: &str, ty: Term) {
        ctx.add_constructor_checked(name, inductive, ty)
            .unwrap_or_else(|e| {
                panic!(
                    "prelude constructor `{name} : {inductive}` must pass the checked path: {e:?}"
                )
            });
    }
}

// =========================================================================
// THE AXIOM AUDIT TRAIL — our `#print axioms`, and the taint that cannot be lost.
// =========================================================================

use std::collections::BTreeSet;

/// Names that mark a proof as **ADMITTED** rather than proved.
///
/// `sorryAx` is Lean's; the rest are the other placeholders a translated or hand-edited
/// environment can acquire. LOGOS deliberately registers NONE of them, so seeing one means a
/// caller reached past `StandardLibrary::register` — which is the single most dangerous thing that
/// can happen to a proof library, because the result type-checks, BOTH kernels agree, and
/// `verified == true` really does hold.
pub const ADMITTED_AXIOMS: &[&str] =
    &["sorryAx", "sorry", "admit", "lcProof", "Lean.trustCompiler"];

/// Whether a name marks an admitted (not proved) step.
pub fn is_admitted_axiom(name: &str) -> bool {
    ADMITTED_AXIOMS.contains(&name)
}

/// Axioms and constants LOGOS deliberately does NOT have, each with the sentence a diagnostic
/// should say instead of "unbound variable".
///
/// This exists because the information that distinguishes `sorryAx` from a typo is the NAME and
/// nothing else: inside `infer_type` an unregistered `Term::Global` carries no provenance, so a
/// single verdict for the whole arm cannot both say "your proof rests on an admission" for
/// `sorryAx` and "this local is out of scope" for `id`. A name-keyed registry separates them, and
/// putting it HERE — beside the trusted base it is the complement of — is what keeps the two in
/// step: an axiom that later gets registered must be deleted from this table, and
/// `the_foreign_axiom_registry_is_disjoint_from_the_trusted_base` in the lane's own test file
/// fails if it is not.
pub const FOREIGN_AXIOMS: &[(&str, &str)] = &[
    (
        "sorryAx",
        "an ADMITTED proof: `sorryAx` proves anything, so LOGOS never registers it. A proof term \
         that cites it is INCOMPLETE, not misspelled — adding the name to the prelude would make \
         every certificate ever minted claim an axiom set it was not checked against",
    ),
    (
        "sorry",
        "an ADMITTED proof: LOGOS has no `sorry`, deliberately. The proof is incomplete",
    ),
    (
        "admit",
        "an ADMITTED proof: LOGOS has no `admit`, deliberately. The proof is incomplete",
    ),
    (
        "lcProof",
        "Lean's compiler-internal proof placeholder. It is not part of any trusted base and has \
         no LOGOS counterpart",
    ),
    (
        "Lean.trustCompiler",
        "Lean's assumption that its own compiler is correct. LOGOS's analogue is the narrower \
         `reduceBool`/`ofReduceBool` pair, which is what `native_decide` rests on",
    ),
    (
        "Classical.choice",
        "Lean's dotted spelling. LOGOS registers this axiom as `Classical_choice`",
    ),
    (
        "Classical.em",
        "Lean's dotted spelling. LOGOS derives this from `dne` and registers it as `Classical_em` \
         — a definition, not an axiom",
    ),
    (
        "Quot.sound",
        "Lean's dotted spelling. LOGOS registers this axiom as `Quot_sound`",
    ),
    (
        "Lean.ofReduceBool",
        "Lean's dotted spelling. LOGOS registers this axiom as `ofReduceBool`",
    ),
    (
        "Lean.reduceBool",
        "Lean's dotted spelling. LOGOS registers this axiom as `reduceBool`",
    ),
];

/// The diagnostic sentence for a constant the trusted base deliberately lacks, or `None` when the
/// name is simply not registered (a typo, a missing import, an out-of-scope local).
///
/// A kernel that answers `UnboundVariable("sorryAx")` invites exactly one repair — add the name to
/// the prelude — and that repair is catastrophic. A kernel that answers `UnboundVariable("id")`
/// invites the right one. Same arm, same shape, different name: this function is the only thing
/// that can tell them apart.
pub fn foreign_axiom_note(name: &str) -> Option<&'static str> {
    FOREIGN_AXIOMS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, why)| *why)
}

/// What a proof rests on: the axioms it cites transitively, and the names that resolve to nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AxiomUsage {
    /// Axioms cited, TRANSITIVELY through definitions. Sorted, so a report is stable and diffable.
    pub axioms: BTreeSet<String>,
    /// Names that resolve to NOTHING in the context checked against. Non-empty means the proof was
    /// minted against a larger base than the one re-checking it — trusted-base drift, named.
    pub unresolved: BTreeSet<String>,
}

impl AxiomUsage {
    /// Fold another report into this one — the union, on both halves.
    ///
    /// A closure is almost never over one term: a declaration is its TYPE and its BODY, and a
    /// certificate is its proof term and the proposition it CLAIMS. Every caller that walks two
    /// terms goes through here, so no caller can merge the axioms and forget the unresolved
    /// names — the half that reports trusted-base DRIFT, and the half a summary is most tempted
    /// to drop.
    pub fn absorb(&mut self, other: AxiomUsage) {
        self.axioms.extend(other.axioms);
        self.unresolved.extend(other.unresolved);
    }

    /// Every name this proof rests on, declared or not — the ONE view the three query methods
    /// below share.
    ///
    /// `unresolved` is not a lesser half. A name the checking context does not have is
    /// trusted-base DRIFT: the proof was minted against a larger base than the one re-checking
    /// it, and `sorryAx` re-checked against a clean prelude lands there rather than in `axioms`,
    /// because the clean prelude never declared it. A query that read `axioms` alone therefore
    /// answered "rests on nothing outside this list" for exactly the proof the audit trail exists
    /// to catch.
    fn cited(&self) -> impl Iterator<Item = &String> {
        self.axioms.iter().chain(self.unresolved.iter())
    }

    /// Does this proof rest on anything outside `permitted`?
    ///
    /// A trust POLICY is written from this, so it counts an unresolved name too — see
    /// `AxiomUsage::cited`. Permitting a name by spelling silences it whichever half it is in,
    /// so one list governs both.
    pub fn beyond(&self, permitted: &[&str]) -> BTreeSet<String> {
        self.cited()
            .filter(|a| !permitted.contains(&a.as_str()))
            .cloned()
            .collect()
    }

    /// Every ADMITTED name this proof rests on, transitively — the taint.
    ///
    /// Better than Lean here, and deliberately: Lean warns on the declaration that CONTAINS the
    /// `sorry` and says nothing about its dependants, so a theorem three lemmas downstream of an
    /// admission looks finished and `#print axioms` is a command you have to remember to run. This
    /// reads the closure, so the taint is a property of the proof rather than of a warning you
    /// might have missed, and it cannot be lost by renaming, re-exporting or citing at a distance.
    pub fn admitted(&self) -> BTreeSet<String> {
        self.cited()
            .filter(|a| is_admitted_axiom(a))
            .cloned()
            .collect()
    }

    /// Is this proof tainted by an admission? Sugar for `!admitted().is_empty()`.
    pub fn is_admitted(&self) -> bool {
        !self.admitted().is_empty()
    }

    /// A proof that cites any name a caller considers admitted is not a proof, and neither is
    /// anything that transitively cites it.
    pub fn is_tainted_by(&self, admitted: &[&str]) -> bool {
        self.cited().any(|a| admitted.contains(&a.as_str()))
    }

    /// The `#print axioms` report, rendered. One line, stable, diffable — and it names the
    /// admission FIRST when there is one, because that is the part a reader must not scroll past.
    pub fn render(&self, subject: &str) -> String {
        let mut out = String::new();
        let tainted = self.admitted();
        if !tainted.is_empty() {
            out.push_str(&format!(
                "'{subject}' is ADMITTED, not proved: it rests on {}\n",
                tainted.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if self.axioms.is_empty() && self.unresolved.is_empty() {
            out.push_str(&format!("'{subject}' depends on no axioms"));
            return out;
        }
        if !self.axioms.is_empty() {
            out.push_str(&format!(
                "'{subject}' depends on axioms: [{}]",
                self.axioms.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        if !self.unresolved.is_empty() {
            if !self.axioms.is_empty() {
                out.push('\n');
            }
            out.push_str(&format!(
                "'{subject}' cites names this trusted base does not have: [{}]",
                self.unresolved
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        out
    }
}

/// Internally derived evidence for a checked opaque declaration. Its direct
/// edges point into Context's shared certification graph, never a copied closure.
#[derive(Debug, Clone)]
pub(crate) struct CheckedOpaqueDependencies {
    pub(crate) usage: AxiomUsage,
    pub(crate) dependencies: BTreeSet<String>,
}

pub(crate) struct CheckedOpaqueTrace {
    pub(crate) checked: CheckedOpaqueDependencies,
    pub(crate) graph: std::collections::BTreeMap<String, BTreeSet<String>>,
}

/// Every AXIOM a term actually depends on — Lean's `#print axioms`, as a function.
///
/// The closure is TRANSITIVE through definitions, following both a definition's BODY and its TYPE
/// (a type can cite an axiom the body never mentions). A one-hop scan of a proof term sees only the
/// lemma it names, which is precisely how a `sorry` three lemmas upstream stays invisible.
///
/// A name that resolves to nothing is reported too, in `unresolved`: a proof naming a constant the
/// trusted base does not have IS trusted-base drift, and it must be visible as that rather than as
/// a name-resolution symptom.
///
/// Lives in `prelude` rather than in `certificate` on purpose. `certificate` is behind the `serde`
/// feature; the audit trail is not optional, and the REPL's `Print Axioms`, the kernel's own
/// diagnostics and `logicaffeine_proof`'s verdicts all need it in the default build.
pub fn axiom_dependencies(ctx: &Context, term: &Term) -> AxiomUsage {
    dependency_trace(ctx, &[term], false, None).checked.usage
}

/// The axiom closure of `term`, as [`axiom_dependencies`], where `known` answers the axioms of a constant whose
/// closure was already taken: the walk takes that answer at the constant instead of walking below it. Reachability is
/// transitive, so the answer is what the walk would have found there, and a constant whose closure was taken had
/// every name below it resolved. An importer that audits each declaration in order passes the sets it recorded, so
/// a declaration's walk covers its own terms and what is not yet audited rather than the whole closure again.
pub fn axiom_dependencies_given(
    ctx: &Context,
    term: &Term,
    known: &dyn Fn(&str) -> Option<Vec<String>>,
) -> AxiomUsage {
    dependency_trace(ctx, &[term], false, Some(known)).checked.usage
}

pub(crate) fn checked_opaque_dependencies(
    ctx: &Context,
    ty: &Term,
    body: &Term,
) -> CheckedOpaqueTrace {
    dependency_trace(ctx, &[ty, body], true, None)
}

/// Usage preserves the existing assumption-leaf policy. Certification edges
/// additionally cover assumption TYPES, constructor owners/inventories and
/// previously checked opaque references. Context shares those direct edges.
/// `known` (usage only, never a footprint) stops the walk at a constant whose axioms it answers.
fn dependency_trace(
    ctx: &Context,
    roots: &[&Term],
    footprint: bool,
    known: Option<&dyn Fn(&str) -> Option<Vec<String>>>,
) -> CheckedOpaqueTrace {
    debug_assert!(!(footprint && known.is_some()), "a footprint walks every edge");
    let mut result = CheckedOpaqueTrace {
        checked: CheckedOpaqueDependencies {
            usage: AxiomUsage::default(),
            dependencies: BTreeSet::new(),
        },
        graph: std::collections::BTreeMap::new(),
    };
    let mut seen = BTreeSet::<(String, bool)>::new();
    let mut queue = Vec::<(String, bool)>::new();
    fn enqueue(
        term: &Term,
        used: bool,
        queue: &mut Vec<(String, bool)>,
        mut edges: Option<&mut BTreeSet<String>>,
    ) {
        let mut names = Vec::new();
        collect_globals(term, &mut names);
        for name in names {
            if let Some(edges) = edges.as_deref_mut() {
                edges.insert(name.clone());
            }
            queue.push((name, used));
        }
    }
    for root in roots {
        enqueue(
            root,
            true,
            &mut queue,
            footprint.then_some(&mut result.checked.dependencies),
        );
    }
    while let Some((name, used)) = queue.pop() {
        if !seen.insert((name.clone(), used)) {
            continue;
        }
        if let Some(axioms) = known.and_then(|known| known(&name)) {
            if used {
                result.checked.usage.axioms.extend(axioms);
            }
            continue;
        }
        if let Some(checked) = ctx.checked_opaque_dependencies(&name) {
            if used {
                result.checked.usage.absorb(checked.usage.clone());
            }
            // This node's existing graph already records its complete proof
            // footprint. Keeping the named edge shares it without flattening.
            continue;
        }
        let mut direct = BTreeSet::new();
        if let Some((_, ty, body)) = ctx.get_universe_poly(&name) {
            enqueue(ty, used, &mut queue, footprint.then_some(&mut direct));
            enqueue(body, used, &mut queue, footprint.then_some(&mut direct));
        } else if let Some(body) = ctx.get_definition_body(&name) {
            enqueue(body, used, &mut queue, footprint.then_some(&mut direct));
            if let Some(ty) = ctx.get_definition_type(&name) {
                enqueue(ty, used, &mut queue, footprint.then_some(&mut direct));
            }
        } else if ctx.is_inductive(&name) || ctx.is_constructor(&name) {
            if let Some(ty) = ctx.get_global(&name) {
                enqueue(ty, used, &mut queue, footprint.then_some(&mut direct));
            }
            if footprint {
                if let Some(owner) = ctx.constructor_inductive(&name) {
                    direct.insert(owner.to_string());
                    queue.push((owner.to_string(), false));
                }
            }
            for (constructor, ty) in ctx.get_constructors(&name) {
                if footprint {
                    direct.insert(constructor.to_string());
                    queue.push((constructor.to_string(), false));
                }
                enqueue(ty, used, &mut queue, footprint.then_some(&mut direct));
            }
        } else if crate::quotient::prelude_kind(ctx, &name).is_some() {
            if let Some(ty) = ctx.get_global(&name) {
                enqueue(ty, used, &mut queue, footprint.then_some(&mut direct));
            }
            if footprint {
                if let Some(certificate) = &ctx.prelude_quotient {
                    for dependency in &certificate.dependencies {
                        direct.insert(dependency.clone());
                        queue.push((dependency.clone(), false));
                    }
                }
            }
        } else if crate::observational::kind(ctx, &name).is_some() {
            if let Some((_, ty)) = ctx.poly_declaration(&name) {
                enqueue(ty, used, &mut queue, footprint.then_some(&mut direct));
            }
            if footprint {
                if let Some(certificate) = &ctx.observational {
                    for dependency in &certificate.dependencies {
                        direct.insert(dependency.clone());
                        queue.push((dependency.clone(), false));
                    }
                }
            }
        } else if let Some((_, ty)) = ctx
            .poly_declaration(&name)
            .filter(|_| ctx.quotient_primitives.contains_key(&name))
        {
            enqueue(ty, used, &mut queue, footprint.then_some(&mut direct));
            if footprint {
                if let Some(certificate) = ctx.quotient_primitives.get(&name) {
                    for dependency in &certificate.dependencies {
                        direct.insert(dependency.clone());
                        queue.push((dependency.clone(), false));
                    }
                }
            }
        } else if let Some(ty) = ctx
            .get_global(&name)
            .or_else(|| ctx.poly_declaration(&name).map(|(_, ty)| ty))
        {
            if used {
                result.checked.usage.axioms.insert(name.clone());
            }
            // An assumption is a LEAF of the usage report, as in Lean's `CollectAxioms`, whose
            // `axiomInfo` arm records the axiom and never visits its type: `#print axioms element`
            // for `axiom element : Carrier` is `[element]`. A declaration whose own statement
            // mentions `Carrier` is charged for it through that statement. The type still enters
            // the certification footprint, so changing it invalidates what was checked against it.
            enqueue(ty, false, &mut queue, footprint.then_some(&mut direct));
        } else {
            // A missing certification dependency must prevent publication too.
            result.checked.usage.unresolved.insert(name.clone());
        }
        if footprint {
            result.graph.entry(name).or_default().extend(direct);
        }
    }
    result
}

/// The axiom closure of a NAMED constant in a context — what `Print Axioms <name>.` reports.
///
/// A definition reports what its statement and body rest on; an axiom reports itself alone,
/// as Lean's `#print axioms` does. `None` when the name is not registered.
pub fn axiom_dependencies_of(ctx: &Context, name: &str) -> Option<AxiomUsage> {
    if let Some(checked) = ctx.checked_opaque_dependencies(name) {
        return Some(checked.usage.clone());
    }
    if let Some((_, ty, body)) = ctx.get_universe_poly(name) {
        let mut usage = axiom_dependencies(ctx, body);
        usage.absorb(axiom_dependencies(ctx, ty));
        return Some(usage);
    }
    if let Some(body) = ctx.get_definition_body(name) {
        let mut usage = axiom_dependencies(ctx, &body.clone());
        if let Some(ty) = ctx.get_definition_type(name) {
            usage.absorb(axiom_dependencies(ctx, &ty.clone()));
        }
        return Some(usage);
    }
    ctx.get_global(name)
        .or_else(|| ctx.poly_declaration(name).map(|(_, ty)| ty))?;
    // Inductive types, constructors, certified quotient primitives, the certified observational
    // constants and assumptions all read the one closure. An axiom rests on ITSELF — a report that omitted the name would say a
    // `sorry`-shaped declaration depends on nothing — and, as in Lean's `#print axioms`, on
    // nothing its type mentions: the closure reads an assumption as a leaf.
    Some(axiom_dependencies(ctx, &Term::Global(name.to_string())))
}

/// Collect every `Global`/`Const` name appearing anywhere in a term.
fn collect_globals(t: &Term, out: &mut Vec<String>) {
    match t {
        Term::Global(n) => out.push(n.clone()),
        Term::Const { name, .. } => out.push(name.clone()),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => {
            collect_globals(param_type, out);
            collect_globals(body_type, out);
        }
        Term::Lambda {
            param_type, body, ..
        } => {
            collect_globals(param_type, out);
            collect_globals(body, out);
        }
        Term::App(f, a) => {
            collect_globals(f, out);
            collect_globals(a, out);
        }
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            collect_globals(discriminant, out);
            collect_globals(motive, out);
            for c in cases {
                collect_globals(c, out);
            }
        }
        Term::Fix { body, .. } => collect_globals(body, out),
        Term::MutualFix { defs, .. } => {
            for (_, d) in defs {
                collect_globals(d, out);
            }
        }
        Term::Let {
            ty, value, body, ..
        } => {
            collect_globals(ty, out);
            collect_globals(value, out);
            collect_globals(body, out);
        }
        Term::Annotated { term, .. } => collect_globals(term, out),
        Term::Sort(_) | Term::Var(_) | Term::Lit(_) | Term::Hole => {}
    }
}

// -------------------------------------------------------------------------
// Term-building shorthands for the structural-type section.
// -------------------------------------------------------------------------

fn gl(name: &str) -> Term {
    Term::Global(name.to_string())
}

fn v_(name: &str) -> Term {
    Term::Var(name.to_string())
}

fn apps_(head: Term, args: &[Term]) -> Term {
    args.iter()
        .fold(head, |acc, a| Term::App(Box::new(acc), Box::new(a.clone())))
}

fn pi_(param: &str, param_type: Term, body_type: Term) -> Term {
    Term::Pi {
        param: param.to_string(),
        param_type: Box::new(param_type),
        body_type: Box::new(body_type),
    }
}

fn arrow_(from: Term, to: Term) -> Term {
    pi_("_", from, to)
}

fn lam_(param: &str, param_type: Term, body: Term) -> Term {
    Term::Lambda {
        param: param.to_string(),
        param_type: Box::new(param_type),
        body: Box::new(body),
    }
}

/// `fix name. λ(l : list_ty). body` — a recursion whose FIRST argument is the one it recurses on.
///
/// The kernel's `Fix` rule unfolds `(fix f. b) arg` only when `arg` is constructor-form, and it
/// looks at the argument the fixpoint is IMMEDIATELY applied to. A `fix` that binds its type
/// parameter first is therefore stuck forever: `List_length Nat [1,2,3]` hands the fixpoint
/// `Nat`, which is not a constructor and does not reduce. Parameters are uniform across the
/// recursion and belong OUTSIDE it — which is also what Lean does, and what makes the recursive
/// call `rec t` rather than `rec A t`.
fn fix_on_list(name: &str, list_ty: Term, body: Term) -> Term {
    Term::Fix {
        name: name.to_string(),
        body: Box::new(lam_("l", list_ty, body)),
    }
}
