//! Lib-unit coverage for the `Context` accessors/mutators the integration suite does not
//! exercise directly (struct/definition/declaration queries, mutual-block recording, and the
//! auto-tactic hint registry). Kills the `-> None`/`-> true`/`-> empty`/`> vs >=` survivors.
use super::*;

fn sort0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn nullary(name: &str) -> MutualInductive {
    MutualInductive {
        name: name.to_string(),
        sort: sort0(),
        num_params: 0,
        constructors: vec![(format!("mk{name}"), g(name))],
    }
}

#[test]
fn struct_info_distinguishes_structures_from_plain_inductives() {
    let mut c = Context::new();
    c.add_structure("S", &[("A", sort0())], &[("f", Term::Var("A".to_string()))]);
    c.add_inductive("Nat", sort0());
    assert!(c.struct_info("S").is_some(), "S is a registered structure");
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    c.add_structure("Bad", &[("A", sort0())], &[("f", g("A"))]);
    assert!(
        c.struct_info("Bad").is_none(),
        "a field naming an unbound constant is refused"
    );
    assert!(c.refused_declarations().iter().any(|(n, _)| n == "Bad"));
    assert!(
        c.struct_info("Nat").is_none(),
        "a plain inductive has no struct info"
    );
    assert!(c.struct_info("Nonexistent").is_none());
}

#[test]
fn is_definition_is_true_only_for_definitions() {
    let mut c = Context::new();
    c.add_definition("foo".to_string(), sort0(), g("Nat"));
    c.add_inductive("Bar", sort0());
    c.add_declaration("ax", sort0());
    assert!(c.is_definition("foo"), "foo has a body");
    assert!(!c.is_definition("Bar"), "an inductive is not a definition");
    assert!(!c.is_definition("ax"), "an axiom is not a definition");
    assert!(!c.is_definition("missing"));
}

#[test]
fn iter_declarations_and_definitions_yield_registered_entries() {
    let mut c = Context::new();
    c.add_declaration("ax", sort0());
    c.add_definition("d".to_string(), sort0(), g("Nat"));
    let decls: Vec<&str> = c.iter_declarations().map(|(n, _)| n).collect();
    assert!(
        decls.contains(&"ax"),
        "iter_declarations must yield the axiom, got {decls:?}"
    );
    let defs: Vec<&str> = c.iter_definitions().map(|(n, _, _)| n).collect();
    assert!(
        defs.contains(&"d"),
        "iter_definitions must yield the definition, got {defs:?}"
    );
}

#[test]
fn a_single_member_is_not_recorded_as_a_mutual_block() {
    let mut c = Context::new();
    c.add_mutual_inductives(std::slice::from_ref(&nullary("Foo")))
        .unwrap();
    assert!(
        c.mutual_block_of("Foo").is_none(),
        "a lone inductive is NOT a mutual block"
    );
    // Positive control: a genuine ≥2-member block IS recorded.
    c.add_mutual_inductives(&[nullary("A"), nullary("B")])
        .unwrap();
    assert_eq!(
        c.mutual_block_of("A"),
        Some(["A".to_string(), "B".to_string()].as_slice()),
        "a real 2-member block records its members",
    );
}

#[test]
fn hints_register_deduplicated_and_are_queryable() {
    let mut c = Context::new();
    c.add_hint("thm1");
    c.add_hint("thm1"); // duplicate — must NOT double-register
    c.add_hint("thm2");
    assert!(c.is_hint("thm1"), "thm1 is a registered hint");
    assert!(c.is_hint("thm2"));
    assert!(!c.is_hint("other"), "an unregistered name is not a hint");
    let hints = c.get_hints();
    assert_eq!(
        hints.iter().filter(|h| *h == "thm1").count(),
        1,
        "thm1 registered exactly once"
    );
    assert!(hints.contains(&"thm2".to_string()));
}
