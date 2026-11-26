//! ★ LOCK — the English prover certifies the rules a technical standard is written in. ★
//!
//! A kit standard names its parts by codes (`PC1`, `F6`, `T20`), says what every part of one kind does to every part
//! of another ("every controller protects every string"), and restricts a rule to the parts that have something ("every
//! kit that has a battery has a fuse"). Each of these is an ordinary first-order argument, and each must be certified:
//!
//! - a name with digits in it is a constant like any other;
//! - a transitive verb between two universals instantiates both;
//! - a relative clause with `has` restricts its universal, and a part without the thing is not swept in.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test technical_rules_prove_lock`

use logicaffeine_compile::compile_theorem_for_ui;

fn theorem(name: &str, givens: &[&str], goal: &str) -> logicaffeine_compile::TheoremCompileResult {
    let mut source = format!("## Theorem: {name}\n");
    for given in givens {
        source.push_str(&format!("Given: {given}\n"));
    }
    source.push_str(&format!("Prove: {goal}\nProof: Auto.\n"));
    compile_theorem_for_ui(&source)
}

fn certifies(name: &str, givens: &[&str], goal: &str) {
    let result = theorem(name, givens, goal);
    assert!(result.error.is_none(), "{name} did not compile: {:?}", result.error);
    assert!(
        result.verified,
        "{name}: from {givens:?} the prover does not certify {goal:?} ({:?}); premises {:?}",
        result.verification_error, result.premises
    );
}

fn refuses(name: &str, givens: &[&str], goal: &str) {
    let result = theorem(name, givens, goal);
    assert!(result.error.is_none(), "{name} did not compile: {:?}", result.error);
    assert!(!result.verified, "{name}: from {givens:?} the prover certified {goal:?}; premises {:?}", result.premises);
}

#[test]
fn a_name_with_digits_is_a_constant() {
    for id in ["B1", "PC1", "F6", "T20", "V150"] {
        certifies(&format!("Mortal_{id}"), &["Every man is mortal.", &format!("{id} is a man.")], &format!("{id} is mortal."));
    }
    refuses("Not_B2", &["Every man is mortal.", "B1 is a man."], "B2 is mortal.");
}

#[test]
fn a_name_with_digits_is_a_constant_in_a_chain() {
    certifies(
        "Kit_Chain",
        &["Every kit is a product.", "Every product is labeled.", "PC1 is a kit."],
        "PC1 is labeled.",
    );
}

#[test]
fn a_transitive_verb_between_two_universals_instantiates_both() {
    certifies(
        "Carl_Protects_Sam",
        &["Every controller protects every string.", "Carl is a controller.", "Sam is a string."],
        "Carl protects Sam.",
    );
    certifies(
        "C1_Protects_S2",
        &["Every controller protects every string.", "C1 is a controller.", "S2 is a string."],
        "C1 protects S2.",
    );
    refuses(
        "Not_A_String",
        &["Every controller protects every string.", "Carl is a controller.", "Sam is a panel."],
        "Carl protects Sam.",
    );
}

#[test]
fn a_relative_clause_with_has_restricts_its_universal() {
    certifies(
        "Kitty_Has_A_Fuse",
        &["Every kit that has a battery has a fuse.", "Kitty is a kit.", "Kitty has a battery."],
        "Kitty has a fuse.",
    );
    refuses(
        "No_Battery_No_Fuse",
        &["Every kit that has a battery has a fuse.", "Kitty is a kit."],
        "Kitty has a fuse.",
    );
}
