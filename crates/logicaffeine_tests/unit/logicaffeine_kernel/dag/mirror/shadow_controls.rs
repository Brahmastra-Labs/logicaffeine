use super::*;
use crate::error::KernelError;
use crate::term::{Term, Universe};
use std::collections::BTreeSet;

fn g(name: &str) -> Term {
    Term::Global(name.into())
}

/// `P : Prop` with its proof `p`, and `Q : Prop` with none.
fn small() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("P", Term::Sort(Universe::Prop));
    c.add_constructor("p", "P", g("P"));
    c.add_inductive("Q", Term::Sort(Universe::Prop));
    c
}

fn panic_text(run: impl FnOnce() + std::panic::UnwindSafe) -> Option<String> {
    let caught = std::panic::catch_unwind(run).err()?;
    Some(
        caught
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| caught.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default(),
    )
}

#[test]
fn a_refusal_the_core_does_not_share_fails_loudly() {
    let c = small();
    let refused = KernelError::TypeMismatch {
        expected: "P".into(),
        found: "Q".into(),
    };
    let entry = shadow_enter(&c);
    let text = panic_text(|| {
        shadow(&c, &entry, Some(&refused), || {
            vec![logos_env::candidate_definition(
                &c,
                "x",
                None,
                &g("P"),
                &g("p"),
            )]
        })
    })
    .expect("`x : P := p` is a proof the core admits");
    assert!(
        text.contains("the core admits it") && text.contains('x'),
        "{text}"
    );
}

/// Every door checks what it registers, so no door hands the core a declaration it refuses: the
/// accept side's decision is checked on the verdicts a sync returns.
#[test]
fn an_admission_the_core_does_not_share_is_a_disagreement() {
    let verdict = |name: &str, verdict: Result<(), String>| Replayed {
        name: name.to_string(),
        kind: "definition",
        verdict,
    };
    let pending: HashSet<&str> = ["owed"].into_iter().collect();
    let found = admission_disagreements(
        &[
            verdict("bad", Err("`p` is no proof of `Q`".to_string())),
            verdict("good", Ok(())),
            verdict("owed", Ok(())),
            verdict("waits", Err("unknown constant `later`".to_string())),
        ],
        &pending,
        &BTreeSet::new(),
    );
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(
        found[0].contains("`bad`") && found[0].contains("the core refuses it"),
        "{found:?}"
    );
    assert!(
        found[1].contains("`owed`") && found[1].contains("holds it pending"),
        "{found:?}"
    );
    assert!(
        found[2].contains("`waits`") && found[2].contains("the core refuses it"),
        "{found:?}"
    );
    assert!(admission_disagreements(
        &[verdict("owed", Err("pending".to_string()))],
        &pending,
        &BTreeSet::new()
    )
    .is_empty());
}

/// A refusal that follows from one the core made before is that one's: only the first is reported.
#[test]
fn a_refusal_downstream_of_an_earlier_one_is_not_reported_again() {
    let verdict = |name: &str, verdict: Result<(), String>| Replayed {
        name: name.to_string(),
        kind: "definition",
        verdict,
    };
    let downstream: BTreeSet<String> = ["uses_root".to_string()].into_iter().collect();
    let found = admission_disagreements(
        &[
            verdict("root", Err("`p` is no proof of `Q`".to_string())),
            verdict("uses_root", Err("unknown constant 'root'".to_string())),
        ],
        &HashSet::new(),
        &downstream,
    );
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("`root`"), "{found:?}");
}

/// No checked door decided a constructor registered unchecked, so the core's refusal of it is no
/// disagreement; a declaration a door checks over it is refused by the core downstream of that
/// refusal, and is none either.
#[test]
fn an_unchecked_registration_the_core_refuses_does_not_fault_a_door_that_checks_over_it() {
    let mut c = small();
    c.add_constructor("raw", "P", g("Q"));
    c.declare("uses_raw", g("Q"), g("raw"))
        .expect("the Term kernel admits it over the registered `raw`");
}

/// A declaration made under local bindings may be open in them; the core admits closed global
/// declarations, so the shadow stands down there.
#[test]
fn a_door_in_a_context_with_local_bindings_is_not_compared() {
    let mut c = small().extend("x", g("P"));
    c.declare("open", g("P"), Term::Var("x".to_string()))
        .expect("the Term kernel admits the open body");
}

/// Only what a door registers is compared. After a unit test lifted the stable-meaning gate, a
/// replay re-decides dependents checked against meanings their names no longer have; those are not
/// the door's, and what the door registers agrees.
#[test]
fn a_door_after_a_lifted_stable_meaning_gate_is_compared_on_its_own_declarations() {
    let mut c = small();
    c.declare("keep", g("P"), g("p"))
        .expect("both kernels admit it");
    c.permit_meaning_changes_for_cache_tests();
    c.add_declaration("P", Term::Sort(Universe::Type(0)));
    c.declare("later", Term::Sort(Universe::Prop), g("Q"))
        .expect("the Term kernel admits it");
}

#[test]
fn verdicts_both_kernels_share_pass_in_silence() {
    let mut c = small();
    c.declare("ok", g("P"), g("p"))
        .expect("both kernels admit it");
    assert!(
        c.declare("no", g("Q"), g("p")).is_err(),
        "both kernels refuse it"
    );
}

#[test]
fn a_door_s_own_policy_is_not_a_verdict_of_the_theory() {
    let c = small();
    let policy = KernelError::CertificationError("`x` already names a declaration".into());
    let entry = shadow_enter(&c);
    shadow(&c, &entry, Some(&policy), || {
        vec![logos_env::candidate_definition(
            &c,
            "x",
            None,
            &g("P"),
            &g("p"),
        )]
    });
}

#[test]
fn the_standard_library_registers_with_the_shadow_standing_down() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    assert!(
        !crate::prelude::registering(),
        "the registration ends with its flag"
    );
    let mut d = c.clone();
    d.declare(
        "two",
        g("Nat"),
        Term::App(
            Box::new(g("Succ")),
            Box::new(Term::App(Box::new(g("Succ")), Box::new(g("Zero")))),
        ),
    )
    .expect("a declaration after it is shadowed and agreed");
}
