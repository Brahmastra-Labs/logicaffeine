//! Conformance tier 9 — **metaprogramming and syntax** — translated case by case into LOGOS.
//!
//! The lazy reading of this tier is "Lean has a macro system, we do not, fourteen `OUT`s". That
//! reading is wrong twice over, and every test below is written to make the two corrections
//! checkable rather than assertable.
//!
//! **First correction — we have a reflective embedding Lean's core does not.** `Syntax` is not a
//! runtime datatype hidden inside an elaborator monad here; it is an INDUCTIVE IN THE THEORY
//! (`prelude.rs` registers `SVar`/`SGlobal`/`SSort`/`SApp`/`SLam`/`SPi`/`SLit`/`SName`), with
//! `syn_quote` (reification), `syn_subst` (splicing — the antiquotation), `syn_lift`, `syn_beta`,
//! `syn_step`, `syn_eval` (bounded evaluation) and `syn_diag` (the diagonal) all computing inside
//! kernel reduction. `Derivation` is a second inductive over it, and the literate `## Theorem …
//! Proof:` door gates a tactic's derivation by normalising `concludes body` against the reified
//! goal. That is how `phase87_reflection` … `phase95_incompleteness` build Gödel's First
//! Incompleteness Theorem *inside* the system. Lean's `Lean.Syntax` cannot be reasoned about from
//! within Lean at all. So the quotation case (T09-005) is a `DIVERGE` where we are ahead.
//!
//! **Second correction — the surface-notation cases are honest gaps, not scope decisions.** The
//! literate parser has exactly ONE infix table — `<= ≤ >= ≥ < > + - *` plus the words `equals`
//! and `implies` — with hard-coded precedence and associativity and no extension point, and the
//! `Proof:` surface has a closed set of fourteen tactic keywords. A mathematician coins notation
//! constantly. `notation`, `infixl`/`infixr`/`prefix`/`postfix`, `macro_rules` and their scoping
//! are `MUST`s that fail today.
//!
//! What is genuinely `OUT` is the part that is a PROGRAM: an `elab` body is arbitrary `MetaM` with
//! the environment, unification and `IO` in scope; a `deriving` handler is `CommandElabM`;
//! `initialize` mutates a persistent environment extension at import time; `run_cmd` performs
//! effects during elaboration. The kernel is pure CoC by construction (the Milner invariant), and
//! `recheck` deliberately REBUILDS its trusted context from `StandardLibrary::register` rather
//! than trusting one an artefact carries — so an environment extension has nowhere to live *by
//! design*, and that design is what makes a certificate independently re-checkable. Those cases
//! assert a LOUD refusal plus the thing the corpus actually cares about: that the declarations
//! such a metaprogram PRODUCES are ordinary and still check.
//!
//! Two findings that are not in the corpus's evidence, both recorded as assertions below:
//!
//! 1. **Write-only names.** `Definition <name> : …` takes its name as a raw text slice up to the
//!    first `:`, while a term's identifier lexer demands `is_alphabetic` then `is_alphanumeric`.
//!    So `Definition h' : Nat := Zero.`, `Definition «a name» : Nat := Zero.` and
//!    `Definition Inner.value : Nat := Zero.` all SUCCEED and key the context at `h'`,
//!    `«a name»`, `Inner.value` — names no term can ever spell. Not a translation loss: a global
//!    that can be declared and never referenced. (T09-011.)
//! 2. **A `deriving` clause is silently absorbed into a constructor type.**
//!    `Inductive Colour := red : Colour deriving BEq.` is ACCEPTED, and registers `red` at type
//!    `((Colour deriving) BEq)` — an application of the inductive to two globals that are not in
//!    the context at all. The clause is not refused, it is mis-parsed, and the constructor
//!    registration path does not notice that the resulting type mentions unbound names.
//!    (T09-006.)

use logicaffeine_kernel::interface::{InterfaceError, ParseError, Repl};

// ============================================================================================
// HELPERS — per-file, per house style.
// ============================================================================================

/// Run one command against a fresh context.
fn once(src: &str) -> Result<String, InterfaceError> {
    Repl::new().execute(src)
}

/// A LOUD refusal: the vernacular does not recognise the command at all, so nothing about it can
/// be silently half-interpreted into a declaration.
fn refused_as_unknown_command(src: &str) -> bool {
    matches!(
        once(src),
        Err(InterfaceError::Parse(ParseError::UnknownCommand(_)))
    )
}

/// A LOUD lexical refusal: the character cannot even be tokenised, so it cannot be smuggled in as
/// an elaborator metavariable (`?x`) or as a `Literal::Text` payload.
fn refused_at_the_lexer(src: &str) -> bool {
    matches!(
        once(src),
        Err(InterfaceError::Parse(ParseError::Expected { .. }))
    )
}

/// Every command in turn against one context; the last result is returned.
fn script(cmds: &[&str]) -> (Repl, Result<String, InterfaceError>) {
    let mut r = Repl::new();
    let mut last = Ok(String::new());
    for c in cmds {
        last = r.execute(c);
    }
    (r, last)
}

/// Names a translator might invent for a command that declares nothing. None of them may ever
/// appear in the flat global namespace: LOGOS keys are plain strings with no namespaces, so a
/// phantom can collide with a real declaration.
const PHANTOM_NAMES: &[&str] = &[
    "notation",
    "infixl",
    "infixr",
    "prefix",
    "postfix",
    "macro",
    "macro_rules",
    "syntax",
    "elab",
    "deriving",
    "initialize",
    "attribute",
    "simp",
    "set_option",
    "run_cmd",
    "evalResult",
    "it",
    "⊕",
    "⊗",
    "⋄",
    "▷",
    "⊞",
    "⊠",
    "⊡",
    "∽",
    "‼",
    "≫",
];

fn no_phantom_globals(r: &Repl) -> Vec<&'static str> {
    PHANTOM_NAMES
        .iter()
        .copied()
        .filter(|n| r.context().get_global(n).is_some())
        .collect()
}

// ============================================================================================
// T09-001 · `notation` — MUST / Gap
// ============================================================================================

/// Lean: `notation:65 a " ⊕ " b => myOp a b`, then `x ⊕ y` and `myOp x y` are the same term.
/// LOGOS has one hard-coded infix table in the literate parser and no way to add to it, so the
/// abbreviation a mathematician reaches for first cannot be written. The written-out form is
/// fine — that half is asserted first, so a regression in it is caught even while the notation
/// half is red.
#[test]
fn a_user_declared_notation_abbreviates_an_application_transparently() {
    let mut r = Repl::new();
    r.execute(
        "Definition myOp : Int -> Int -> Int := \
         (fun a : Int => fun b : Int => ((add a) ((mul b) 2))).",
    )
    .expect("the written-out function must define");
    r.execute(
        "Definition myOther : Int -> Int -> Int := (fun a : Int => fun b : Int => ((mul a) b)).",
    )
    .expect("the second operator's function must define");
    r.execute("Definition wrap : Int -> Int := (fun n : Int => ((add n) 1)).")
        .expect("the bracketing function must define");
    assert_eq!(
        r.execute("Eval ((myOp 3) 4).").ok(),
        Some("11".to_string()),
        "the written-out application must evaluate: myOp 3 4 = 3 + 4*2"
    );

    // A parser-extension command declares NOTHING. Whatever the vernacular does with it, it must
    // not put the token — or the command word — into the flat global namespace.
    let attempted = r.execute("Notation \"a \u{2295} b\" := ((myOp a) b).");
    assert!(
        no_phantom_globals(&r).is_empty(),
        "a notation command must add no global; these appeared: {:?}",
        no_phantom_globals(&r)
    );

    assert!(
        attempted.is_ok(),
        "a user must be able to declare notation for an application: {:?}",
        attempted
    );
    r.execute("Notation \"a \u{2297} b\" := ((myOther a) b).")
        .expect("a second notation at a tighter precedence must declare");
    r.execute("Notation \"\u{27e6} n \u{27e7}\" := (wrap n).")
        .expect("bracketing notation must declare");

    assert_eq!(
        r.execute("Eval (3 \u{2295} 4).").ok(),
        r.execute("Eval ((myOp 3) 4).").ok(),
        "the notation and the written-out spelling must be the same term"
    );
    assert_eq!(
        r.execute("Eval (2 \u{2295} 3 \u{2297} 4).").ok(),
        r.execute("Eval ((myOp 2) ((myOther 3) 4)).").ok(),
        "⊗ at 70 binds tighter than ⊕ at 65 — precedence decides the spine"
    );
    assert_eq!(
        r.execute("Eval \u{27e6} 5 \u{27e7}.").ok(),
        Some("6".to_string()),
        "bracketing notation must expand to its function"
    );
    assert_eq!(
        r.execute("Eval ((1 \u{2295} 2) \u{2295} 3).").ok(),
        r.execute("Eval ((myOp ((myOp 1) 2)) 3).").ok(),
        "a notation applied to a compound argument is not a leaf substitution"
    );
}

// ============================================================================================
// T09-002 · `infixl` / `infixr` / `prefix` / `postfix` — MUST / Gap
// ============================================================================================

/// The associativity trap: `a ⋄ b ⋄ c` under `infixl` is `f (f a b) c` and under `infixr` is
/// `g a (g b c)`, and for a non-associative operator those are different VALUES. LOGOS's one
/// infix table does have real fixity — `-` is left-associative and `*` binds tighter than `+`,
/// asserted first through `ring`, which decides the grouping rather than being told it — but the
/// table is closed, so a user operator has no fixity to declare.
///
/// Note where that table lives: in the LITERATE parser, so the `Statement:` grammar reads
/// `1 - 2 - 3` while the Coq-style `Check`/`Eval` door cannot lex `+` at all. A user fixity would
/// have to reach both.
#[test]
fn user_fixity_decides_whether_a_three_operand_spine_nests_left_or_right() {
    let mut r = Repl::new();

    // The fixity we DO have is real, not accidental: `ring` proves the left grouping and REFUSES
    // the right one, so the associativity is decided rather than assumed.
    assert!(
        r.execute(
            "## Theorem: LeftAssoc\n    Statement: 1 - 2 - 3 equals (1 - 2) - 3.\n    Proof: ring."
        )
        .is_ok(),
        "the built-in `-` must be left-associative"
    );
    assert!(
        r.execute("## Theorem: NotRightAssoc\n    Statement: 1 - 2 - 3 equals 1 - (2 - 3).\n    Proof: ring.")
            .is_err(),
        "left and right grouping of `-` must be different values"
    );
    assert!(
        r.execute(
            "## Theorem: MulBindsTighter\n    Statement: 2 + 3 * 4 equals 14.\n    Proof: ring."
        )
        .is_ok(),
        "`*` must bind tighter than `+`"
    );
    assert!(
        r.execute("## Theorem: NotAddFirst\n    Statement: 2 + 3 * 4 equals (2 + 3) * 4.\n    Proof: ring.")
            .is_err(),
        "the additive-first grouping must be refused"
    );

    r.execute("Definition f : Int -> Int -> Int := (fun a : Int => fun b : Int => ((add ((mul a) 10)) b)).")
        .expect("the left-associative operator's function must define");
    r.execute("Definition g : Int -> Int -> Int := (fun a : Int => fun b : Int => ((add ((mul a) 10)) b)).")
        .expect("the right-associative operator's function must define");
    r.execute("Definition neg : Int -> Int := (fun n : Int => ((sub 100) n)).")
        .expect("the prefix operator's function must define");
    r.execute("Definition bang : Int -> Int := (fun n : Int => ((mul n) n)).")
        .expect("the postfix operator's function must define");

    let left = r.execute("Infixl 65 \"\u{22c4}\" := f.");
    assert!(
        no_phantom_globals(&r).is_empty(),
        "a fixity command must add no global; these appeared: {:?}",
        no_phantom_globals(&r)
    );
    assert!(
        left.is_ok(),
        "a user must be able to declare a LEFT-associative operator: {:?}",
        left
    );
    r.execute("Infixr 65 \"\u{25b7}\" := g.")
        .expect("a user must be able to declare a RIGHT-associative operator");
    r.execute("Prefix \"\u{223d}\" := neg.")
        .expect("a user must be able to declare a prefix operator");
    r.execute("Postfix \"\u{203c}\" := bang.")
        .expect("a user must be able to declare a postfix operator");

    assert_eq!(
        r.execute("Eval (1 \u{22c4} 2 \u{22c4} 3).").ok(),
        r.execute("Eval ((f ((f 1) 2)) 3).").ok(),
        "infixl must nest LEFT: a ⋄ b ⋄ c = f (f a b) c"
    );
    assert_eq!(
        r.execute("Eval (1 \u{25b7} 2 \u{25b7} 3).").ok(),
        r.execute("Eval ((g 1) ((g 2) 3)).").ok(),
        "infixr must nest RIGHT: a ▷ b ▷ c = g a (g b c)"
    );
    assert_ne!(
        r.execute("Eval (1 \u{22c4} 2 \u{22c4} 3).").ok(),
        r.execute("Eval (1 \u{25b7} 2 \u{25b7} 3).").ok(),
        "the two groupings of the same three operands must be different values"
    );
    assert_eq!(
        r.execute("Eval (\u{223d} 7).").ok(),
        r.execute("Eval (neg 7).").ok(),
        "prefix notation must expand to its function"
    );
    assert_eq!(
        r.execute("Eval (7 \u{203c}).").ok(),
        r.execute("Eval (bang 7).").ok(),
        "postfix notation must expand to its function"
    );
}

// ============================================================================================
// T09-003 · `macro_rules` — MUST / Gap
// ============================================================================================

/// A pattern-matching, recursive rewriter with several alternatives tried in order, where a LATER
/// block's alternatives are tried FIRST. LOGOS has no term-macro facility at all. Its one
/// macro-like mechanism lives in another crate — `logicaffeine_proof`'s `TacticEnv::define`
/// registers user-defined TACTIC scripts, expanded textually in place and depth-bounded at
/// `MAX_TACTIC_DEPTH`, which answers the `macro … : tactic` half of this case by a different
/// route. The kernel's own `Proof:` surface has no such door: its fourteen tactic keywords are a
/// closed set, and an unknown one is refused (asserted first) rather than silently accepted.
#[test]
fn a_pattern_matching_term_macro_expands_recursively_and_later_blocks_win() {
    let mut r = Repl::new();
    r.execute("Inductive MyList := mynil : MyList | mycons : Int -> MyList -> MyList.")
        .expect("the inductive the macro builds must declare");

    // The kernel's tactic surface is CLOSED, and closed loudly: an undeclared tactic name is
    // refused, never treated as a no-op that would let a false theorem through.
    let unknown_tactic = r.execute(
        "## Theorem: MacroTactic\n    Statement: (implies (lt 0 x) (lt 0 x)).\n    Proof: reflAll.",
    );
    assert!(
        unknown_tactic.is_err(),
        "an undeclared tactic name must be refused, not silently accepted: {:?}",
        unknown_tactic
    );

    let declared = r.execute("Macro \"nested\" := (fun xs : MyList => xs).");
    assert!(
        no_phantom_globals(&r).is_empty(),
        "a macro declaration must add no global; these appeared: {:?}",
        no_phantom_globals(&r)
    );
    assert!(
        declared.is_ok(),
        "a user must be able to declare a term macro: {:?}",
        declared
    );

    r.execute("MacroRules \"nested![]\" := mynil.")
        .expect("the base alternative must declare");
    r.execute("MacroRules \"nested![$x, $xs]\" := ((mycons $x) nested![$xs]).")
        .expect("the recursive alternative must declare");
    assert_eq!(
        r.execute("Eval nested![1, 2, 3].").ok(),
        r.execute("Eval ((mycons 1) ((mycons 2) ((mycons 3) mynil))).")
            .ok(),
        "a recursive macro must expand fully to the cons chain"
    );

    r.execute("MacroRules \"pick![$x]\" := ((add $x) 1).")
        .expect("the first alternative for `pick!` must declare");
    r.execute("MacroRules \"pick![$x]\" := ((add $x) 100).")
        .expect("a second block for the same syntax must declare");
    assert_eq!(
        r.execute("Eval pick![5].").ok(),
        Some("105".to_string()),
        "a later macro_rules block PREPENDS its alternatives, so it is tried first"
    );

    r.execute("Macro \"twiceIt $e\" := (fun x : Int => ((add ((add $e) $e)) x)).")
        .expect("a binder-introducing macro must declare");
    assert_eq!(
        r.execute("Definition hygienic : Int -> Int := (fun x : Int => ((twiceIt x) 0)).")
            .and_then(|_| r.execute("Eval (hygienic 4)."))
            .ok(),
        Some("8".to_string()),
        "the macro's own binder must not capture the call site's `x`"
    );
}

// ============================================================================================
// T09-004 · `syntax` + `elab` — OUT / Refused
// ============================================================================================

/// An `elab` body is an arbitrary `MetaM` program: it reads the environment, the local context,
/// unification and `IO`, so the same syntax elaborates differently in different positions.
/// Replaying it means embedding the Lean runtime, and running an opaque program inside the
/// trusted core would hollow out the double-kernel re-check. We refuse the metaprogram.
///
/// The nuance that makes the refusal usable — and the thing this test exists to pin — is its
/// SCOPE. What the elaborator emits is an ordinary term, and it must still check. A refusal that
/// swallowed the whole file would be useless on real Lean.
#[test]
fn a_custom_term_elaborator_is_refused_while_the_term_it_produced_still_checks() {
    for src in [
        "elab \"theAnswer\" : term => return mkNatLit 42",
        "elab \"closeWithRfl\" : tactic => do evalTactic",
        "elab_rules : term",
        "syntax \"nested![\" term,* \"]\" : term",
    ] {
        assert!(
            refused_as_unknown_command(src),
            "a metaprogram must be refused by name, not partly interpreted: {:?} gave {:?}",
            src,
            once(src)
        );
    }

    // The refusal leaves nothing behind — no phantom keyed on the command or the syntax it named.
    let (r, _) = script(&[
        "elab \"theAnswer\" : term => return mkNatLit 42",
        "elab \"envIsNonEmpty\" : term => do let env <- getEnv",
    ]);
    assert!(
        no_phantom_globals(&r).is_empty(),
        "a refused elaborator must leave no global behind: {:?}",
        no_phantom_globals(&r)
    );
    assert!(
        r.context().get_global("theAnswer").is_none(),
        "the refused elaborator must not declare the name it was going to elaborate"
    );

    // THE OUTPUT IS ORDINARY. `theAnswer` elaborated to a plain literal, `mixed k = k + theAnswer`
    // to a plain application; both must check and compute.
    let (out, last) = script(&[
        "Definition theAnswer : Int := 42.",
        "Definition mixed : Int -> Int := (fun k : Int => ((add k) theAnswer)).",
        "Eval (mixed 8).",
    ]);
    assert_eq!(
        last.ok(),
        Some("50".to_string()),
        "the declarations an elaborator produced are ordinary and must translate"
    );
    assert!(
        out.context().get_global("mixed").is_some(),
        "over-rejecting the file that merely USED a custom elaborator is the failure mode this \
         case exists to catch"
    );
}

// ============================================================================================
// T09-005 · syntax quotation and antiquotation — DIVERGE / Native
// ============================================================================================

/// Lean's `` `(…) `` builds a value of an opaque runtime type `Lean.Syntax`, spliced with `$x` at
/// parse time, usable only inside a metaprogram and invisible to the logic. We do not have that
/// surface, and the test asserts we refuse it at the LEXER — which matters, because the two
/// tempting encodings are both silent corruptions: `$x`/`?h` as a `Term::Var("?x")` would be an
/// elaborator metavariable the unifier then SOLVES to something arbitrary, and a quotation as
/// `Literal::Text` would typecheck as a string nothing can traverse.
///
/// What we have instead is deeper and is the reason this is a divergence rather than a hole:
/// `Syntax` is an INDUCTIVE IN THE THEORY, so quotation is a value you can compute with and prove
/// about. `syn_quote` reifies a syntax tree into the code that builds it; `syn_subst` splices into
/// a de Bruijn hole — that is the antiquotation, at the object level; `syn_eval` runs the result
/// under a fuel bound; `syn_diag` is the diagonal, from which the fixed-point lemma and Gödel's
/// First Incompleteness Theorem are built (`phase93`–`phase95`). Lean's `Syntax` supports none of
/// that from inside Lean.
#[test]
fn quotation_is_an_object_level_syntax_value_that_splices_and_evaluates() {
    // The Lean surface forms are refused where they are written, not reinterpreted.
    for src in [
        "Definition q : Syntax := `(1 + 1).",
        "Definition spliced : Syntax := ($x + $x).",
        "Definition qt : Syntax := `(tactic| rfl).",
        "Check ?x.",
    ] {
        assert!(
            refused_at_the_lexer(src),
            "a quotation or antiquotation must be refused at the lexer, never encoded as a \
             metavariable or a Text literal: {:?} gave {:?}",
            src,
            once(src)
        );
    }
    let (r, _) = script(&["Definition q : Syntax := `(1 + 1)."]);
    assert!(
        r.context().get_global("q").is_none(),
        "a refused quotation must not leave a definition behind, least of all a Text one"
    );

    // Our route: `Syntax` is a type IN the theory, with constructors and a real eliminator.
    let mut k = Repl::new();
    assert_eq!(
        k.execute("Check Syntax.").ok(),
        Some("Syntax : Type0".to_string()),
        "Syntax must be an inductive in the object theory, not a host-language datatype"
    );
    assert_eq!(
        k.execute("Check SApp.").ok(),
        Some("SApp : Syntax -> Syntax -> Syntax".to_string()),
        "application must be a constructor of the reflected syntax"
    );
    assert_eq!(
        k.execute("Check syn_quote.").ok(),
        Some("syn_quote : Syntax -> Syntax".to_string()),
        "quotation must be a function of the theory"
    );

    k.execute("Definition body : Syntax := ((SApp (SVar 0)) (SLit 7)).")
        .expect("a syntax tree must be constructible");
    let quoted = k
        .execute("Eval (syn_quote body).")
        .expect("quote must compute");
    assert!(
        quoted.contains("SName") && quoted != "((SApp (SVar 0)) (SLit 7))",
        "syn_quote must reify the tree into the CODE that builds it: {:?}",
        quoted
    );

    // ANTIQUOTATION: splicing a term into the hole at de Bruijn 0.
    assert_eq!(
        k.execute("Eval (((syn_subst (SLit 99)) 0) body).").ok(),
        Some("((SApp (SLit 99)) (SLit 7))".to_string()),
        "syn_subst must splice into the quoted tree's hole — the object-level antiquotation"
    );

    // And the spliced code RUNS, under a fuel bound, inside kernel reduction.
    assert_eq!(
        k.execute("Eval ((syn_eval 10) ((SApp ((SLam (SName \"Nat\")) (SVar 0))) (SLit 5))).")
            .ok(),
        Some("(SLit 5)".to_string()),
        "syn_eval must beta-reduce the reflected redex"
    );

    // The diagonal law — quote, then splice the quotation back into its own hole. This is the
    // construction Gödel's theorem is built from, and it is an equation the kernel decides.
    assert_eq!(
        k.execute("Eval (syn_diag body).").ok(),
        k.execute("Eval (((syn_subst (syn_quote body)) 0) body).")
            .ok(),
        "syn_diag x must equal syn_subst (syn_quote x) 0 x"
    );

    // Reflection is CERTIFIED, not a side channel: a derivation over reflected syntax only passes
    // the theorem door when it concludes the stated goal.
    assert!(
        k.execute("## Theorem: RingOk\n    Statement: ((add x) 0) equals x.\n    Proof: ring.")
            .is_ok(),
        "a true goal must be provable through the reflected Derivation"
    );
    assert!(
        k.execute("## Theorem: RingBad\n    Statement: ((add x) 1) equals x.\n    Proof: ring.")
            .is_err(),
        "a false goal must be refused — a well-typed error derivation is not a proof"
    );
}

// ============================================================================================
// T09-006 · custom `deriving` handlers — OUT / Gap
// ============================================================================================

/// `registerDerivingHandler` takes an `Array Name → CommandElabM Bool`: it reads the environment,
/// emits several declarations, logs, and may DECLINE so another handler runs. There is no handler
/// registry, no attribute system and no hook that fires when an inductive is declared, and a
/// handler is a program that no `Term` variant can carry. So the handler is `OUT`.
///
/// But a refusal has to be a refusal. Today the clause is not refused, it is MIS-PARSED:
/// `Inductive Colour := red : Colour deriving BEq.` is accepted and registers `red` at type
/// `((Colour deriving) BEq)`, applying the inductive to two globals that are not in the context.
/// That is strictly worse than rejecting the file — it is a constructor whose declared type names
/// things that do not exist. Asserted last, and red.
///
/// The two halves that already work are asserted first: the declarations a handler would GENERATE
/// are ordinary, and LOGOS's own single automatic derivation — the dependent recursor, produced
/// for every `Inductive` — fires.
#[test]
fn a_deriving_clause_is_refused_rather_than_absorbed_into_a_constructor_type() {
    assert!(
        refused_as_unknown_command("initialize registerDerivingHandler ``MyTag myTagHandler"),
        "registering a deriving handler must be refused: {:?}",
        once("initialize registerDerivingHandler ``MyTag myTagHandler")
    );

    // What the handler would have emitted is ordinary: a definition plus a tag function.
    let (r, last) = script(&[
        "Inductive Flag := up : Flag | down : Flag.",
        "Definition Flag_tag : Flag -> Int := (fun f : Flag => 0).",
        "Eval (Flag_tag up).",
    ]);
    assert_eq!(
        last.ok(),
        Some("0".to_string()),
        "the declarations a deriving handler generates are ordinary and must translate"
    );

    // LOGOS's own derivation-on-declaration: the dependent eliminator, for free, for every
    // inductive. It is not user-extensible, and it is the whole extent of automatic generation.
    assert!(
        Repl::new().execute("Check Flag_rec.").is_err(),
        "a fresh context has no Flag_rec until Flag is declared"
    );
    assert!(
        r.context().get_global("Flag_rec").is_some(),
        "declaring an inductive must derive its dependent recursor"
    );
    let (bare, _) = script(&["Inductive Flag := up : Flag | down : Flag."]);
    assert!(
        bare.context().get_global("Flag_tag").is_none(),
        "no user handler may fire on declaration — there is no registry for one"
    );

    // THE REFUSAL ITSELF. A `deriving` clause must be rejected, and in particular must never be
    // absorbed into a constructor's type.
    let clause = once("Inductive Colour := red : Colour deriving BEq.");
    let (absorbed, _) = script(&["Inductive Colour := red : Colour deriving BEq."]);
    let red_ty = absorbed.context().get_global("red").map(|t| format!("{t}"));
    assert!(
        !red_ty
            .as_deref()
            .is_some_and(|t| t.contains("deriving") || t.contains("BEq")),
        "no constructor may be registered at a type mentioning globals that are not in the \
         context: red : {:?}",
        red_ty
    );
    assert!(
        clause.is_err(),
        "a `deriving` clause must be refused, not parsed as part of the constructor's type: {:?}",
        clause
    );
}

// ============================================================================================
// T09-007 · `initialize` and environment extensions — OUT / Refused
// ============================================================================================

/// `initialize` declares nothing. It runs `IO` at IMPORT time and mutates an environment
/// extension or an `IO.Ref` — state attached to the environment, serialised into the `.olean`,
/// restored on import, and read by later elaboration. A `Context` is flat maps of TERMS, built by
/// a linear sequence of `add_*` calls, with no extension slot, no attribute map and no IO.
///
/// That absence is deliberate, and it is what makes an artefact re-checkable: `recheck` rebuilds
/// the trusted context itself from `StandardLibrary::register` rather than trusting one a
/// certificate carries, so there is nowhere for import-time state to hide. The positive statement
/// of it is asserted below — LOGOS's one attribute-like facility, `Attribute: hint.`, is
/// CONTEXT-LOCAL, and a fresh context has never heard of it.
#[test]
fn an_environment_extension_is_refused_and_no_state_survives_a_fresh_context() {
    for src in [
        "initialize counterRef : IO.Ref Nat <- IO.mkRef 0",
        "initialize myExt : SimplePersistentEnvExtension Name (Array Name) <- registerSimplePersistentEnvExtension",
        "initialize registerTraceClass `t09007.trace",
        "@[simp] Definition tagged : Int := 0.",
        "attribute [simp] laterTagged",
    ] {
        assert!(
            refused_as_unknown_command(src),
            "an environment-extension or attribute registration must be refused: {:?} gave {:?}",
            src,
            once(src)
        );
    }

    // The refusal leaves no trace, and the ordinary declarations in the same file still translate.
    let (r, last) = script(&[
        "initialize counterRef : IO.Ref Nat <- IO.mkRef 0",
        "Definition plainDef : Int -> Int := (fun n : Int => ((add n) 1)).",
        "Eval (plainDef 1).",
    ]);
    assert_eq!(
        last.ok(),
        Some("2".to_string()),
        "a declaration beside a refused initialize must still translate"
    );
    assert!(
        no_phantom_globals(&r).is_empty() && r.context().get_global("counterRef").is_none(),
        "a refused initialize must leave no global: {:?}",
        no_phantom_globals(&r)
    );

    // LOGOS's nearest thing to `@[simp]` is context-local by construction: it lives in the
    // `Context` being built, never in a persistent environment a later session inherits.
    let (hinted, _) = script(&[
        "## Theorem: Tagged\n    Statement: ((add x) 0) equals x.\n    Proof: ring.\n    Attribute: hint.",
    ]);
    assert!(
        hinted.context().get_global("Tagged").is_some(),
        "the hinted theorem must exist in the context that declared it"
    );
    assert!(
        Repl::new().context().get_global("Tagged").is_none(),
        "no attribute may persist into a fresh context — every context is rebuilt from the \
         standard library, which is what lets a certificate be re-checked independently"
    );
}

// ============================================================================================
// T09-008 · `set_option` — DIVERGE / Native
// ============================================================================================

/// Lean's options are mostly cosmetic, and a minority are not: `autoImplicit` decides whether an
/// unbound identifier becomes an implicit binder, so it changes a declaration's ARITY, and
/// `structureEta` changes which terms are definitionally equal. A declaration's meaning therefore
/// depends on ambient configuration — which is why Mathlib turns `autoImplicit` off globally.
///
/// LOGOS has no option surface at all, and that is the divergence: every elaboration decision is a
/// function of the source text alone, so a term is the same term on every machine and a
/// certificate re-checks identically everywhere. The failure mode to avoid is not the missing
/// pragma but a pragma that is ACCEPTED AND IGNORED — a file asking for `autoImplicit false` and
/// silently getting implicits anyway. We refuse it by name instead.
///
/// (The one real cost is elsewhere and is not an option: `normalize` carries a hard fuel and
/// returns the partially-reduced term when it runs out. That is a missing ERROR, not a missing
/// setting, and it is booked as T11-005.)
#[test]
fn no_pragma_can_change_an_elaborated_term_because_no_pragma_is_accepted() {
    for src in [
        "set_option pp.explicit true.",
        "set_option maxHeartbeats 400000.",
        "set_option linter.unusedVariables false.",
        "set_option autoImplicit false.",
        "set_option maxRecDepth 2000.",
        "set_option structureEta false.",
    ] {
        assert!(
            refused_as_unknown_command(src),
            "a pragma must be refused by name, never accepted and ignored: {:?} gave {:?}",
            src,
            once(src)
        );
    }

    // Auto-implicit binding is unconditional, so the arity of a declaration is decided by its own
    // text. The explicit spelling and the auto-bound one must be the SAME type — the case's
    // `sameArity`, which in Lean holds only because a pragma happened to be set the right way.
    let mut plain = Repl::new();
    plain
        .execute("Definition idAuto : A -> A := (fun x : A => x).")
        .expect("auto-implicit binding must work");
    let auto_ty = plain.execute("Check idAuto.").expect("Check idAuto");
    plain
        .execute("Definition idExplicit : forall A : Type0, A -> A := (fun A : Type0 => fun x : A => x).")
        .expect("the explicit spelling must work");
    let explicit_ty = plain
        .execute("Check idExplicit.")
        .expect("Check idExplicit");
    // SAME ARITY, and deliberately NOT the same type. In Lean 4 an auto-bound implicit is
    // universe-POLYMORPHIC — `def idAuto : A -> A` elaborates to `{A : Sort u} -> A -> A`, not to
    // `Type 0` — which is precisely what T05-008 requires and what this campaign shipped. Pinning
    // the two spellings as IDENTICAL asserts the silent monomorphisation T05-008 calls a defect,
    // so the arity is what this case can honestly hold the pragma to.
    assert_eq!(
        auto_ty.matches("->").count(),
        explicit_ty.matches("->").count(),
        "the auto-bound and explicitly-bound declarations must have the same ARITY: {auto_ty} vs {explicit_ty}"
    );
    assert!(
        auto_ty.contains("->"),
        "the auto-bound declaration must still be a function type: {auto_ty}"
    );

    // And the refused pragma changes nothing: the same source, after an attempted
    // `set_option autoImplicit false`, elaborates to the same type in the same context.
    let mut after = Repl::new();
    let attempted = after.execute("set_option autoImplicit false.");
    assert!(
        attempted.is_err(),
        "the pragma must not be accepted: {attempted:?}"
    );
    after
        .execute("Definition idAuto : A -> A := (fun x : A => x).")
        .expect("the declaration after a refused pragma must still elaborate");
    assert_eq!(
        after.execute("Check idAuto.").ok(),
        Some(auto_ty),
        "no pragma may change an elaborated term, because none is accepted"
    );
    assert!(
        no_phantom_globals(&after).is_empty(),
        "a refused pragma must leave no global: {:?}",
        no_phantom_globals(&after)
    );
}

// ============================================================================================
// T09-009 · `#check` / `#eval` / `#print` — MUST / Native
// ============================================================================================

/// The phantom-declaration trap: `#check foo`, `#eval e` and `#print bar` look like commands that
/// "do something", and the tempting move is to emit *something* for each — a declaration for the
/// checked type, a definition holding the evaluated value. LOGOS keys are flat strings with no
/// namespaces, so any such phantom can collide with a real declaration. The correct output is
/// nothing.
///
/// `Repl`'s `Check` and `Eval` are exactly this: they answer the query and leave the context
/// alone. `#print`'s information is reachable too — the definition body is in the context — and
/// the Lean spellings are refused by name rather than half-interpreted.
#[test]
fn check_and_eval_answer_a_query_without_declaring_anything() {
    let mut r = Repl::new();
    r.execute("Definition realDef : Int -> Int := (fun n : Int => ((mul n) 2)).")
        .expect("the one real declaration must define");
    r.execute("## Theorem: realTheorem\n    Statement: ((add x) 0) equals x.\n    Proof: ring.")
        .expect("the one real theorem must prove");

    assert_eq!(
        r.execute("Check realDef.").ok(),
        Some("realDef : Int -> Int".to_string()),
        "Check must report the type"
    );
    assert_eq!(
        r.execute("Eval (realDef 21).").ok(),
        Some("42".to_string()),
        "Eval must normalise the term"
    );
    assert_eq!(
        r.execute("Eval ((add 2) 2).").ok(),
        Some("4".to_string()),
        "Eval of a closed arithmetic term must compute"
    );
    assert!(
        r.execute("Check (fun n : Int => ((add n) 1)).").is_ok(),
        "Check must work on an anonymous term, not only on a name"
    );

    // NOTHING was declared by any of that.
    let leaked = no_phantom_globals(&r);
    assert!(
        leaked.is_empty(),
        "a query command must declare nothing; these names appeared: {leaked:?}"
    );
    assert!(
        r.context().get_global("realDef").is_some()
            && r.context().get_global("realTheorem").is_some(),
        "the context must hold exactly what was declared — and it must hold that"
    );

    // `#print`'s information is reachable: the body is in the context, not only the type.
    assert!(
        r.context().get_definition_body("realDef").is_some(),
        "a definition's body must be inspectable without declaring anything"
    );

    // The Lean spellings are refused by NAME, so an unrecognised query can never be silently
    // treated as a declaration.
    for src in [
        "#check realDef",
        "#eval realDef 21",
        "#print realDef",
        "#print axioms realTheorem",
        "#reduce realDef 5",
        "#guard realDef 4 == 8",
    ] {
        assert!(
            refused_as_unknown_command(src),
            "an unrecognised query command must be refused by name: {:?} gave {:?}",
            src,
            once(src)
        );
    }
}

// ============================================================================================
// T09-010 · doc comments and module docs — MUST / Gap
// ============================================================================================

/// Documentation is not semantic — no term's meaning depends on it — so losing it is a
/// degradation, not a failure. The trap is the fix: `Literal::Text` exists, so a docstring CAN be
/// emitted as a term, which typechecks, puts a name the source never declared into the flat
/// namespace where it can collide, and is useless because there are no string operations. That
/// guard is asserted first and holds today.
///
/// What fails is smaller and more embarrassing for a literate language: the vernacular door does
/// not tolerate prose at all. `/-- … -/`, `/-! … -/` and even `-- …` are refused as unknown
/// commands, so a documented declaration cannot be fed through it — the documentation has to be
/// stripped before the kernel sees the file.
#[test]
fn a_documented_declaration_survives_its_documentation() {
    // The docstring must never become data.
    let (r, _) = script(&[
        "/-- A documented definition. -/",
        "Definition documented : Nat := Zero.",
    ]);
    assert!(
        r.context().get_global("documented_doc").is_none()
            && r.context().get_global("doc").is_none(),
        "a docstring must not be smuggled into the namespace as a Text global"
    );

    // Prose beside a declaration must be documentation, not an error.
    for src in [
        "/-- A documented definition. -/",
        "/-! A module-level doc section. -/",
        "-- an ordinary line comment",
    ] {
        assert!(
            once(src).is_ok(),
            "documentation must be accepted and carry no declaration: {:?} gave {:?}",
            src,
            once(src)
        );
    }

    // And the declaration it documents must still be made.
    let (documented, last) = script(&[
        "/-- A documented definition. -/",
        "Definition documented : Int -> Int := (fun n : Int => ((add n) 1)).",
        "/-- A documented theorem. -/",
        "## Theorem: documentedTheorem\n    Statement: ((add x) 0) equals x.\n    Proof: ring.",
        "Eval (documented 1).",
    ]);
    assert_eq!(
        last.ok(),
        Some("2".to_string()),
        "a documented declaration must translate exactly as the undocumented one does"
    );
    assert!(
        documented
            .context()
            .get_global("documentedTheorem")
            .is_some(),
        "the documented theorem must be in the context"
    );
}

// ============================================================================================
// T09-011 · unicode identifiers and «guillemet names» — MUST / Gap
// ============================================================================================

/// The kernel's names are Rust `String`s, so `α`, `Γ` and `ℕ` are perfectly good kernel names —
/// and, better than the corpus's snapshot suggests, the vernacular's identifier lexer is
/// Unicode-aware (`is_alphabetic` / `is_alphanumeric`), so Greek letters, double-struck capitals
/// and subscripts round-trip through `Definition` and `Check` unmangled. That half is asserted
/// first: it is a real win and a regression in it would be invisible otherwise.
///
/// The half that fails is the PRIME — `h`, `h'`, `h''` is overwhelmingly common in Lean — and the
/// «guillemet name». And the way it fails is the finding: `Definition` takes its name as a raw
/// text slice up to the first `:`, so `Definition h' : Nat := Zero.` SUCCEEDS and keys the context
/// at `h'`, while `Check h'.` fails in the lexer. The same for `«a name»` and for the dotted
/// `Inner.value`. Those are globals that can be declared and never referenced — not a lost
/// spelling, a write-only name. The invariant asserted last is the fix's specification: every name
/// the declaration door accepts, a term must be able to spell.
#[test]
fn every_name_the_declaration_door_accepts_a_term_can_spell() {
    // Unicode identifiers work, and are not mangled on the way through.
    for (name, rendered) in [
        ("\u{3b1}", "\u{3b1} : Nat"),
        ("\u{393}", "\u{393} : Nat"),
        ("\u{2115}plus", "\u{2115}plus : Nat"),
        ("x\u{2081}", "x\u{2081} : Nat"),
    ] {
        let (r, _) = script(&[&format!("Definition {name} : Nat := Zero.")]);
        assert!(
            r.context().get_global(name).is_some(),
            "a unicode identifier must key the context under its own spelling: {name}"
        );
        let mut k = Repl::new();
        k.execute(&format!("Definition {name} : Nat := Zero."))
            .unwrap_or_else(|e| panic!("{name} must define: {e:?}"));
        assert_eq!(
            k.execute(&format!("Check {name}.")).ok(),
            Some(rendered.to_string()),
            "a unicode name must be referenceable and printed unmangled: {name}"
        );
    }

    // THE INVARIANT. Anything the declaration door accepts, a term must be able to name.
    let mut unreferenceable = Vec::new();
    for name in [
        "h'",
        "h''",
        "\u{ab}a name with spaces\u{bb}",
        "\u{ab}weird?!name\u{bb}",
        "Inner.value",
    ] {
        let mut k = Repl::new();
        if k.execute(&format!("Definition {name} : Nat := Zero."))
            .is_err()
        {
            continue; // refusing the name outright is a sound answer too
        }
        if k.execute(&format!("Check {name}.")).is_err() {
            unreferenceable.push(name.to_string());
        }
    }
    assert!(
        unreferenceable.is_empty(),
        "every name `Definition` accepts must be a name a term can spell; these declare but can \
         never be referenced: {unreferenceable:?}"
    );

    // And the Lean idiom the prime is used for must work end to end.
    let (_, primed) = script(&[
        "Definition h : Nat := Zero.",
        "Definition h' : Nat := (Succ Zero).",
        "Check h'.",
    ]);
    assert!(
        primed.is_ok(),
        "`h` and `h'` are different names and both must be usable: {primed:?}"
    );
}

// ============================================================================================
// T09-012 · `run_cmd` and `#eval` of an `IO` action — OUT / Refused
// ============================================================================================

/// An effect is not a value. `#eval (IO.println "hi")` elaborates to a term of type `IO Unit` and
/// then RUNS it, and `IO.FS.readFile` makes the elaborated environment depend on the filesystem —
/// the same source produces different declarations on different machines. The kernel has no IO by
/// construction (the Milner invariant), no `Monad`, no `Bind`, and `Term::Let` is a pure
/// zeta-transparent binding rather than a `do`-step, so there is not merely no representation for
/// the effect: there is no place for it to exist.
///
/// Two things the refusal must NOT do, both asserted: swallow the declarations the effect
/// produced, and leave a half-translated monadic chain against globals that do not exist. And
/// `reduceBool` — the one place an evaluator is deliberately wired into kernel reduction — must
/// stay what it is: pure, and gated to `Bool`. It is not an escape hatch.
#[test]
fn an_elaboration_time_effect_is_refused_and_the_kernel_stays_pure() {
    for src in [
        "run_cmd Lean.logInfo \"elaborating\"",
        "run_cmd elabCommand",
        "#eval (IO.println \"side effect during elaboration\" : IO Unit)",
    ] {
        assert!(
            refused_as_unknown_command(src),
            "an elaboration-time effect must be refused: {:?} gave {:?}",
            src,
            once(src)
        );
    }

    // No monadic vocabulary exists to half-translate a `do` block against.
    let mut r = Repl::new();
    for name in ["IO", "bind", "pure", "Monad", "IO.Ref"] {
        assert!(
            r.execute(&format!("Check {name}.")).is_err(),
            "the kernel must not carry a monadic vocabulary: {name} resolved"
        );
    }
    assert!(
        r.execute("Definition ioValue : IO Unit := unit.").is_err(),
        "a declaration whose type mentions IO must not be accepted"
    );

    // The declaration a `run_cmd` produced is ordinary and must translate.
    let (produced, last) = script(&[
        "run_cmd elabCommand",
        "Definition generatedByRunCmd : Int := 5.",
        "Definition pureDecl : Int -> Int := (fun n : Int => ((add n) generatedByRunCmd)).",
        "Eval (pureDecl 1).",
    ]);
    assert_eq!(
        last.ok(),
        Some("6".to_string()),
        "a declaration a run_cmd generated is ordinary and must translate"
    );
    assert!(
        no_phantom_globals(&produced).is_empty(),
        "a refused run_cmd must leave no global: {:?}",
        no_phantom_globals(&produced)
    );

    // `reduceBool` is the closest LOGOS comes to running code while checking. It stays pure and
    // Bool-gated; routing an effect through it would put arbitrary evaluation into the trusted
    // reduction path under a type claiming to be a decision procedure.
    assert_eq!(
        r.execute("Check reduceBool.").ok(),
        Some("reduceBool : Bool -> Bool".to_string()),
        "reduceBool must stay gated to Bool"
    );
    assert_eq!(
        r.execute("Eval (reduceBool true).").ok(),
        Some("true".to_string()),
        "reduceBool must be a pure evaluation hook, with no world to change"
    );
}

// ============================================================================================
// T09-013 · macro expansion ORDERING — MUST / Gap
// ============================================================================================

/// The integration case for the tier: which `macro_rules` alternative wins, whether a `local`
/// notation is still in scope, whether a `scoped` one needs an `open`, what a rebound token means
/// where. Every one of those is a question only a language with a surface-extension stage can
/// ask, and LOGOS has none — which is also why it cannot answer them.
///
/// The green half asserted first is the thing a translator most often gets wrong in the other
/// direction: a refused parser-extension command must emit ZERO declarations. One phantom global
/// per notation in a file, in a flat namespace, is a collision waiting to happen.
#[test]
fn notation_scope_decides_which_global_a_rebound_token_means() {
    let (r, _) = script(&[
        "Definition op : Int -> Int -> Int := (fun a : Int => fun b : Int => ((add a) ((mul b) 2))).",
        "Definition alt : Int -> Int -> Int := (fun a : Int => fun b : Int => ((add ((mul a) 10)) b)).",
        "Notation \"a \u{229e} b\" := ((op a) b).",
        "Notation \"a \u{22a0} b\" := ((alt a) b).",
        "MacroRules \"bump $x\" := ((add $x) 1).",
    ]);
    assert!(
        no_phantom_globals(&r).is_empty(),
        "every parser-extension command must emit ZERO declarations; these appeared: {:?}",
        no_phantom_globals(&r)
    );
    assert!(
        r.context().get_global("op").is_some() && r.context().get_global("alt").is_some(),
        "the ordinary declarations beside them must survive"
    );

    // ONE TOKEN, TWO MEANINGS, decided by the section it is used in.
    let mut k = Repl::new();
    k.execute("Definition op : Int -> Int -> Int := (fun a : Int => fun b : Int => ((add a) ((mul b) 2))).")
        .expect("op must define");
    k.execute("Definition alt : Int -> Int -> Int := (fun a : Int => fun b : Int => ((add ((mul a) 10)) b)).")
        .expect("alt must define");

    let first = k.execute("Section First.");
    assert!(
        first.is_ok(),
        "a section must scope a local notation: {:?}",
        first
    );
    k.execute("Local Notation \"a \u{229e} b\" := ((op a) b).")
        .expect("a local notation must declare");
    assert_eq!(
        k.execute("Eval (3 \u{229e} 4).").ok(),
        k.execute("Eval ((op 3) 4).").ok(),
        "inside the first section the token must mean `op`"
    );
    k.execute("End First.").expect("the section must close");

    k.execute("Section Second.")
        .expect("a second section must open");
    k.execute("Local Notation \"a \u{229e} b\" := ((alt a) b).")
        .expect("the same token must be rebindable in another scope");
    assert_eq!(
        k.execute("Eval (3 \u{229e} 4).").ok(),
        k.execute("Eval ((alt 3) 4).").ok(),
        "in the second section the same token must mean `alt` — scope, not shadowing, decides"
    );
    k.execute("End Second.")
        .expect("the second section must close");

    // A LATER macro_rules block PREPENDS its alternatives, so it is tried FIRST. A source-order
    // expander answers 6 where Lean answers 1005, and the wrong answer is not a type error.
    k.execute("MacroRules \"bump $x\" := ((add $x) 1).")
        .expect("the first bump alternative must declare");
    k.execute("MacroRules \"bump $x\" := ((add $x) 1000).")
        .expect("the second bump alternative must declare");
    assert_eq!(
        k.execute("Eval (bump 5).").ok(),
        Some("1005".to_string()),
        "the later macro_rules block must win"
    );
}

// ============================================================================================
// T09-014 · custom BINDER notation — OUT / Refused
// ============================================================================================

/// `binder_predicate x " ≫ " y:term => `($x > $y)` extends Lean's BINDER parser so that
/// `∀ x ≫ 0, p x` parses. It declares no constant, produces no term, and has no LOGOS counterpart
/// — the only binders are `Pi` and `Lambda`, over one named variable and one type, with three
/// `ParamKind`s none of which encodes a bound. So the notation is `OUT`.
///
/// What must NOT be out is the mathematics it abbreviates, and that is what this test spends its
/// assertions on. `∀ x > 0, p x` desugars to `∀ x, x > 0 → p x`: an ordinary Pi over an ordinary
/// implication. The literate `Statement:` grammar has no `forall` because free variables are
/// implicitly universally closed, so `(implies (lt 0 x) (lt 0 x))` IS that statement — and the
/// proof needs `intro` for the binder and a second step for the bound the Lean source never wrote
/// separately. Both directions are asserted: the true bounded statement proves, the false one is
/// refused, and the telescope form takes one `intro` per desugared binder.
#[test]
fn a_custom_binder_is_refused_while_the_bounded_quantifier_it_abbreviates_is_proved() {
    for src in [
        "binder_predicate x \" \u{226b} \" y:term => `($x > $y)",
        "notation3 \"\u{2200}\u{1d63} \" (...) \", \" r:(scoped p => p) => r",
    ] {
        assert!(
            refused_as_unknown_command(src),
            "a custom binder declaration must be refused: {:?} gave {:?}",
            src,
            once(src)
        );
    }
    let (r, _) = script(&["binder_predicate x \" \u{226b} \" y:term => `($x > $y)"]);
    assert!(
        no_phantom_globals(&r).is_empty(),
        "a refused binder notation must leave no global: {:?}",
        no_phantom_globals(&r)
    );

    // THE DESUGARED FORM IS ORDINARY, AND IT IS PROVED. `∀ x > 0, pos x` with `pos n := 0 < n`.
    let mut k = Repl::new();
    assert!(
        k.execute(
            "## Theorem: boundedForallDesugared\n    Statement: (implies (lt 0 x) (lt 0 x)).\n    Proof: intro. assumption."
        )
        .is_ok(),
        "the desugared bounded quantifier must be statable and provable"
    );
    assert!(
        k.execute(
            "## Theorem: boundedForallFalse\n    Statement: (implies (lt 0 x) (lt x 0)).\n    Proof: intro. assumption."
        )
        .is_err(),
        "a FALSE bounded statement must be refused — the bound is not a licence"
    );

    // A telescope introduced by one binder token becomes one `intro` per desugared binder, and
    // the extra binders are genuinely introduced rather than assumed away: the SECOND bound is the
    // one discharged here, and a conclusion neither bound gives is still refused.
    assert!(
        k.execute(
            "## Theorem: telescope\n    Statement: (implies (lt 0 x) (implies (lt x 9) (lt x 9))).\n    Proof: intro. intro. assumption."
        )
        .is_ok(),
        "each desugared binder must take its own intro"
    );
    assert!(
        k.execute(
            "## Theorem: telescopeUnsound\n    Statement: (implies (lt 0 x) (implies (lt x 9) (lt 9 x))).\n    Proof: intro. intro. assumption."
        )
        .is_err(),
        "introducing both bounds must not license a conclusion neither of them gives"
    );
}
