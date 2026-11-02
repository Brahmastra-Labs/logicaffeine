use super::*;
use logicaffeine_base::Interner;

#[test]
fn referent_source_universal_force() {
    assert!(ReferentSource::ConditionalAntecedent.gets_universal_force());
    assert!(ReferentSource::UniversalRestrictor.gets_universal_force());
    assert!(!ReferentSource::MainClause.gets_universal_force());
    assert!(!ReferentSource::ProperName.gets_universal_force());
}

#[test]
fn drs_new_has_main_box() {
    let drs = Drs::new();
    assert_eq!(drs.boxes.len(), 1);
    assert_eq!(drs.current_box, 0);
    assert_eq!(drs.boxes[0].box_type, Some(BoxType::Main));
}

#[test]
fn drs_enter_exit_box() {
    let mut drs = Drs::new();
    assert_eq!(drs.current_box, 0);

    let ant_idx = drs.enter_box(BoxType::ConditionalAntecedent);
    assert_eq!(ant_idx, 1);
    assert_eq!(drs.current_box, 1);
    assert_eq!(drs.boxes[1].parent, Some(0));

    drs.exit_box();
    assert_eq!(drs.current_box, 0);
}

#[test]
fn drs_introduce_referent_tracks_source() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();

    let x = interner.intern("x");
    let farmer = interner.intern("Farmer");

    // In main box - should be MainClause
    drs.introduce_referent(x, farmer, Gender::Male, Number::Singular);
    assert_eq!(drs.boxes[0].universe[0].source, ReferentSource::MainClause);

    // Enter conditional antecedent
    drs.enter_box(BoxType::ConditionalAntecedent);
    let y = interner.intern("y");
    let donkey = interner.intern("Donkey");
    drs.introduce_referent(y, donkey, Gender::Neuter, Number::Singular);
    assert_eq!(
        drs.boxes[1].universe[0].source,
        ReferentSource::ConditionalAntecedent
    );
}

#[test]
fn drs_conditional_antecedent_accessible_from_consequent() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();

    // Enter conditional antecedent
    let ant_idx = drs.enter_box(BoxType::ConditionalAntecedent);
    let y = interner.intern("y");
    let donkey = interner.intern("Donkey");
    drs.introduce_referent(y, donkey, Gender::Neuter, Number::Singular);
    drs.exit_box();

    // Enter conditional consequent
    let cons_idx = drs.enter_box(BoxType::ConditionalConsequent);

    // Consequent should be able to access antecedent
    assert!(drs.is_accessible(ant_idx, cons_idx));
}

#[test]
fn drs_negation_blocks_accessibility() {
    let mut drs = Drs::new();

    // Enter negation scope
    let neg_idx = drs.enter_box(BoxType::NegationScope);
    drs.exit_box();

    // Main box should NOT be able to access negation scope
    assert!(!drs.is_accessible(neg_idx, 0));
}

#[test]
fn drs_get_universal_referents() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();

    let x = interner.intern("x");
    let farmer = interner.intern("Farmer");
    drs.introduce_referent(x, farmer, Gender::Male, Number::Singular);

    drs.enter_box(BoxType::ConditionalAntecedent);
    let y = interner.intern("y");
    let donkey = interner.intern("Donkey");
    drs.introduce_referent(y, donkey, Gender::Neuter, Number::Singular);

    let universals = drs.get_universal_referents();
    assert_eq!(universals.len(), 1);
    assert_eq!(universals[0], y);
}

#[test]
fn drs_pronoun_resolution_marks_used() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();

    drs.enter_box(BoxType::UniversalRestrictor);
    let y = interner.intern("y");
    let donkey = interner.intern("Donkey");
    drs.introduce_referent(y, donkey, Gender::Neuter, Number::Singular);

    // Resolve "it" - should find donkey
    let resolved = drs.resolve_pronoun(drs.current_box, Gender::Neuter, Number::Singular);
    assert_eq!(resolved, Ok(y));

    // Should be marked as used
    assert!(drs.boxes[1].universe[0].used_by_pronoun);
}

#[test]
fn a_pronoun_reaches_for_an_individual_before_the_event_after_it() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();
    drs.enter_box(BoxType::ConditionalAntecedent);
    let dog = interner.intern("dog");
    drs.introduce_referent(dog, interner.intern("Dog"), Gender::Unknown, Number::Singular);
    let barking = interner.intern("e");
    drs.introduce_event_referent(barking, interner.intern("Event"));
    assert_eq!(drs.resolve_pronoun(drs.current_box, Gender::Neuter, Number::Singular), Ok(dog));
}

#[test]
fn a_pronoun_prefers_the_referent_whose_gender_it_names() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();
    drs.enter_box(BoxType::ConditionalAntecedent);
    let john = interner.intern("John");
    drs.introduce_proper_name(john, john, Gender::Male);
    let dog = interner.intern("x");
    drs.introduce_referent(dog, interner.intern("Dog"), Gender::Unknown, Number::Singular);
    assert_eq!(
        drs.resolve_pronoun(drs.current_box, Gender::Male, Number::Singular),
        Ok(john),
        "he names John's gender; the dog's is only accommodated",
    );
    let mary = interner.intern("Mary");
    drs.introduce_proper_name(mary, mary, Gender::Female);
    let cat = interner.intern("y");
    drs.introduce_referent(cat, interner.intern("Cat"), Gender::Unknown, Number::Singular);
    assert_eq!(drs.resolve_pronoun(drs.current_box, Gender::Female, Number::Singular), Ok(mary));
}

#[test]
fn a_pronoun_accommodates_an_unknown_gender_when_nothing_agrees_exactly() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();
    drs.enter_box(BoxType::ConditionalAntecedent);
    let farmer = interner.intern("x");
    drs.introduce_referent(farmer, interner.intern("Farmer"), Gender::Unknown, Number::Singular);
    drs.introduce_event_referent(interner.intern("e"), interner.intern("Event"));
    assert_eq!(drs.resolve_pronoun(drs.current_box, Gender::Male, Number::Singular), Ok(farmer));
}

#[test]
fn among_referents_that_agree_alike_the_most_recent_wins() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();
    drs.enter_box(BoxType::ConditionalAntecedent);
    let john = interner.intern("John");
    drs.introduce_proper_name(john, john, Gender::Male);
    let bill = interner.intern("Bill");
    drs.introduce_proper_name(bill, bill, Gender::Male);
    assert_eq!(drs.resolve_pronoun(drs.current_box, Gender::Male, Number::Singular), Ok(bill));
    let dog = interner.intern("x");
    drs.introduce_referent(dog, interner.intern("Dog"), Gender::Unknown, Number::Singular);
    let cat = interner.intern("y");
    drs.introduce_referent(cat, interner.intern("Cat"), Gender::Unknown, Number::Singular);
    assert_eq!(drs.resolve_pronoun(drs.current_box, Gender::Unknown, Number::Singular), Ok(cat));
}

#[test]
fn a_conditional_quantifies_over_its_individuals_not_its_events() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();
    drs.enter_box(BoxType::ConditionalAntecedent);
    let dog = interner.intern("x");
    drs.introduce_referent(dog, interner.intern("Dog"), Gender::Unknown, Number::Singular);
    drs.introduce_event_referent(interner.intern("e"), interner.intern("Event"));
    assert_eq!(drs.get_universal_referents(), vec![dog]);
}

#[test]
fn a_rewind_forgets_what_an_abandoned_parse_introduced() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();
    drs.enter_box(BoxType::ConditionalAntecedent);
    let dog = interner.intern("x");
    drs.introduce_referent(dog, interner.intern("Dog"), Gender::Unknown, Number::Singular);
    let mark = drs.mark();
    let antecedent = drs.current_box;

    drs.introduce_event_referent(interner.intern("e"), interner.intern("Event"));
    drs.enter_box(BoxType::NegationScope);
    drs.introduce_referent(interner.intern("y"), interner.intern("Cat"), Gender::Unknown, Number::Singular);
    drs.rewind(&mark);

    assert_eq!(drs.current_box, antecedent, "the abandoned parse's box is still open");
    assert_eq!(drs.get_universal_referents(), vec![dog], "an abandoned referent is still quantified");
    assert_eq!(drs.boxes.len(), 2, "the abandoned parse's box is still there");
}

#[test]
fn a_pronoun_finds_the_event_when_no_individual_agrees() {
    let mut interner = Interner::new();
    let mut drs = Drs::new();
    drs.enter_box(BoxType::ConditionalAntecedent);
    drs.introduce_referent(interner.intern("x"), interner.intern("Woman"), Gender::Female, Number::Singular);
    let singing = interner.intern("e");
    drs.introduce_event_referent(singing, interner.intern("Event"));
    assert_eq!(drs.resolve_pronoun(drs.current_box, Gender::Neuter, Number::Singular), Ok(singing));
    assert!(drs.boxes[drs.current_box].universe[1].is_event);
}
