//! `there exists a d: T such that P` binds `d` AT `T`.
//!
//! The literate existential used to hand the kernel `Ex T (λ d : _. P)` — the binder's type
//! thrown away and replaced by the global `_`, the kernel's match-case placeholder, which only
//! `check_type` knows to read off an expected `Π`. Two things carried it: the elaborator's λ did not
//! put its binder in scope, and the kernel accepted a `Hole` for `Eq`'s carrier. Both were closed
//! (the λ now scopes its binder at its written domain; a hole is not a term — SPEC B), and with them
//! `d` was scoped at the type `_` and `concludes d` refused: "expected Derivation, found _". The
//! type was written; the binder carries it.

use logicaffeine_kernel::interface::Repl;
use logicaffeine_kernel::Term;

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}

/// No hole and no placeholder survives into a registered body.
fn has_placeholder(t: &Term) -> bool {
    match t {
        Term::Hole => true,
        Term::Global(n) => n == "_",
        Term::App(f, a) => has_placeholder(f) || has_placeholder(a),
        Term::Lambda {
            param_type, body, ..
        } => has_placeholder(param_type) || has_placeholder(body),
        Term::Pi {
            param_type,
            body_type,
            ..
        } => has_placeholder(param_type) || has_placeholder(body_type),
        Term::Let {
            ty, value, body, ..
        } => has_placeholder(ty) || has_placeholder(value) || has_placeholder(body),
        Term::Fix { body, .. } => has_placeholder(body),
        Term::MutualFix { defs, .. } => defs.iter().any(|(_, b)| has_placeholder(b)),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            has_placeholder(discriminant)
                || has_placeholder(motive)
                || cases.iter().any(has_placeholder)
        }
        Term::Annotated { term, .. } => has_placeholder(term),
        Term::Var(_) | Term::Sort(_) | Term::Lit(_) | Term::Const { .. } => false,
    }
}

const PROVABLE: &str =
    "## To be Provable (s: Syntax) -> Prop:\n    Yield there exists a d: Derivation such that (concludes(d) equals s).";

#[test]
fn an_existential_binds_its_variable_at_the_written_type() {
    let mut repl = Repl::new();
    repl.execute(PROVABLE)
        .expect("the Gödel literate provability predicate");
    assert_eq!(
        repl.execute("Check Provable.").unwrap(),
        "Provable : Syntax -> Prop"
    );
    let body = repl
        .context()
        .get_definition_body("Provable")
        .expect("a definition")
        .clone();
    assert!(
        !has_placeholder(&body),
        "a hole or `_` reached the registered body: {body:?}"
    );
    let expected = lam(
        "s",
        g("Syntax"),
        app(
            app(g("Ex"), g("Derivation")),
            lam(
                "d",
                g("Derivation"),
                app(
                    app(app(g("Eq"), g("Syntax")), app(g("concludes"), v("d"))),
                    v("s"),
                ),
            ),
        ),
    );
    assert_eq!(
        body, expected,
        "`Eq`'s carrier is solved to `Syntax` from `concludes d`"
    );

    // The vernacular spelling of the same predicate registers the same body.
    repl.execute(
        "Definition Provable2 : Syntax -> Prop := fun s : Syntax => Ex Derivation (fun d : Derivation => Eq _ (concludes d) s).",
    )
    .expect("the vernacular spelling");
    assert_eq!(
        repl.context().get_definition_body("Provable2"),
        Some(&expected)
    );
}

#[test]
fn a_parameterised_type_is_bound_as_written() {
    let mut repl = Repl::new();
    repl.execute("## To be Inhabited_list -> Prop:\n    Yield there exists a xs: List of Nat such that (xs equals xs).")
        .expect("an existential over `List Nat`");
    let body = repl
        .context()
        .get_definition_body("Inhabited_list")
        .expect("a definition")
        .clone();
    assert!(!has_placeholder(&body), "{body:?}");
    let list_nat = app(g("List"), g("Nat"));
    assert_eq!(
        body,
        app(
            app(g("Ex"), list_nat.clone()),
            lam(
                "xs",
                list_nat.clone(),
                app(app(app(g("Eq"), list_nat), v("xs")), v("xs"))
            )
        ),
    );
}

/// The route that used to carry the literate existential — a hole for `Eq`'s carrier, accepted
/// against any sort — is also the route to an equation between two types. It stays closed.
#[test]
fn an_existential_cannot_equate_values_of_two_types() {
    let mut repl = Repl::new();
    let refused = repl.execute(
        "## To be Odd (n: Nat) -> Prop:\n    Yield there exists a b: Bool such that (b equals n).",
    );
    assert!(
        refused.is_err(),
        "`b : Bool` equals `n : Nat` is not a proposition: {refused:?}"
    );
    assert!(
        repl.context().get_definition_body("Odd").is_none(),
        "nothing was registered"
    );
}
