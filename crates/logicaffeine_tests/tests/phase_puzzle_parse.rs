use logicaffeine_language::compile;

// ── helpers ──────────────────────────────────────────────────────────────────

fn ok(clue: &str) -> String {
    compile(clue).unwrap_or_else(|e| panic!("expected OK for {:?}, got {:?}", clue, e))
}

fn contains_all(s: &str, needles: &[&str]) -> bool {
    needles.iter().all(|n| s.contains(n))
}

// ── Lexer: -ly suffix falsely classified as Adverb ───────────────────────────

#[test]
fn family_is_noun_not_adverb() {
    // "family" ends in "-ly" but is a common noun — must not be Adverb
    let out = ok("The Woodard family owns the house.");
    assert!(
        out.contains("Woodard") || out.contains("woodard"),
        "expected Woodard in output; got: {}", out
    );
    assert!(
        out.contains("family") || out.contains("Family"),
        "expected family in output; got: {}", out
    );
}

#[test]
fn family_possessive_sentence() {
    // "X is the Woodard family's house." — possessive with "family" must not break
    let out = ok("Leiman Manor is the Woodard family's house.");
    assert!(out.contains("Woodard") || out.contains("woodard"), "got: {}", out);
    assert!(out.contains("Manor") || out.contains("manor") || out.contains("Leiman"), "got: {}", out);
}

// ── Lexer: "from" in NL mode must be Preposition, not Token::From ─────────────

#[test]
fn is_from_location() {
    // "X is from Y" — origin predicate; "from" must parse as a preposition PP
    let out = ok("The tawny cobra is from New Guinea.");
    assert!(out.contains("cobra") || out.contains("Cobra"), "got: {}", out);
    assert!(out.contains("Guinea") || out.contains("guinea"), "got: {}", out);
}

#[test]
fn subject_from_pp() {
    let out = ok("The species from Australia won.");
    assert!(out.contains("Australia") || out.contains("australia"), "got: {}", out);
}

// ── Multi-word ProperName compounds ──────────────────────────────────────────

// ── Reduced relatives in nominal coordination (list / either-or / of-pair) ───
// A past/-ing participle on a coordinated NP member ("the peak CLIMBED in 1845")
// is a reduced relative. parse_noun_phrase's conservative handler leaves it
// stranded mid-coordination; the shared try_consume_reduced_relative helper
// attaches it at every coordination site so the member's restrictor is not lost.

#[test]
fn enumerated_identity_list_member_reduced_relative() {
    // Each descriptive member keeps its reduced relative as a distinct restrictor.
    let s = ok("The peak first summited by Irving Igor, the peak first climbed in 1899, and the peak first climbed in 1845 are three different mountains.");
    // Member 1: First ∧ Summit ∧ By(Igor)
    assert!(contains_all(&s, &["Summit", "Igor"]),
        "member 1 reduced relative lost; got: {}", s);
    // Members 2 & 3: First ∧ Climb ∧ In(year) — both years must survive distinctly.
    assert!(contains_all(&s, &["Climb", "First", "1899", "1845"]),
        "member reduced-relative years lost; got: {}", s);
    // Three distinct entities.
    assert!(s.matches("¬").count() >= 3, "pairwise distinctness lost; got: {}", s);
}

#[test]
fn list_subject_first_member_reduced_relative() {
    // The FIRST member carries the reduced relative (the comma is hidden behind the
    // participle) — the list gate must still route here.
    let s = ok("The peak climbed in 1845, the dog, and Mt. Quinn are three different mountains.");
    assert!(contains_all(&s, &["Peak", "Climb", "1845", "Dog", "Mt_Quinn"]),
        "first-member reduced relative or later members lost; got: {}", s);
}

#[test]
fn either_or_disjunct_reduced_relative() {
    // "X is either the A first VERBed … or the B first VERBed …" — both disjuncts'
    // reduced relatives survive, predicated of the subject.
    let s = ok("Fushil was either the island first seen by Captain Guizburuaga or the island first seen by Captain Norris.");
    assert!(contains_all(&s, &["See", "Captain_Guizburuaga", "Captain_Norris"]),
        "either-or disjunct reduced relative lost; got: {}", s);
    assert!(s.contains("∨"), "must be a disjunction; got: {}", s);
}

#[test]
fn of_pair_member_reduced_relative() {
    // An of-pair member's reduced relative survives as its restrictor.
    let s = ok("Of the island first seen by Captain Norris and Bob, one is big and the other is small.");
    assert!(contains_all(&s, &["Island", "See", "Captain_Norris"]),
        "of-pair member reduced relative lost; got: {}", s);
    assert!(s.contains("∨"), "of-pair is an exclusive disjunction; got: {}", s);
}

#[test]
fn reduced_relative_main_clause_not_misread_as_list() {
    // Regression: the widened list gate must restore cleanly for ordinary clauses —
    // a reduced-relative subject and a plain main clause stay correct.
    let red = ok("The peak climbed in 1845 is a mountain.");
    assert!(contains_all(&red, &["Peak", "Climb", "1845", "Mountain"]), "got: {}", red);
    // A plain intransitive-with-PP main clause keeps its event + PP.
    let main = ok("The team arrived in 1989.");
    assert!(contains_all(&main, &["Arrive", "1989"]), "got: {}", main);
    // Clause coordination (no trailing "are different") must not become a list.
    let coord = ok("The peak rose, and the valley fell.");
    assert!(contains_all(&coord, &["Rise", "Fall"]) || contains_all(&coord, &["rose", "fell"]),
        "clause coordination broke; got: {}", coord);
}

#[test]
fn two_word_proper_name_as_subject() {
    // "Captain Quinn" is a two-word proper name; both words must land in output
    let out = ok("Captain Quinn led the vessel.");
    assert!(
        out.contains("Captain_Quinn") || (out.contains("Captain") && out.contains("Quinn")),
        "both parts of Captain Quinn must appear; got: {}", out
    );
}

#[test]
fn two_word_place_name_in_pp() {
    // "Yellow Bend" is a two-word place name
    let out = ok("The vessel went to Yellow Bend.");
    assert!(
        out.contains("Yellow") && out.contains("Bend"),
        "both parts of Yellow Bend must appear; got: {}", out
    );
}

#[test]
fn woodlawn_on_york_court() {
    // "York Court" two-word place; PP predicate after copula
    let out = ok("Woodlawn is on York Court.");
    assert!(out.contains("Woodlawn") || out.contains("woodlawn"), "got: {}", out);
    assert!(out.contains("York") && out.contains("Court"), "York Court must appear; got: {}", out);
}

// ── Passive voice ─────────────────────────────────────────────────────────────

#[test]
fn passive_owned_by() {
    // "X is owned by Y" — passive with by-agent
    let out = ok("Leiman Manor is owned by the Woodard family.");
    assert!(out.contains("Manor") || out.contains("manor") || out.contains("Leiman"), "got: {}", out);
    assert!(out.contains("Woodard") || out.contains("woodard"), "got: {}", out);
    // Must express the possession/ownership relation — not just parse without meaning
    assert!(
        out.contains("Own") || out.contains("own") || out.contains("Poss") || out.contains("By"),
        "ownership relation must appear; got: {}", out
    );
}

#[test]
fn passive_was_led_by() {
    let out = ok("The Samantha was led by Captain Quinn.");
    assert!(out.contains("Samantha") || out.contains("samantha"), "got: {}", out);
    assert!(out.contains("Quinn") || out.contains("quinn"), "got: {}", out);
    assert!(
        out.contains("Lead") || out.contains("lead") || out.contains("Led") || out.contains("led"),
        "leading relation must appear; got: {}", out
    );
}

// ── OfPairXor: "Of A and B, one VP₁, the other VP₂" ─────────────────────────

#[test]
fn of_pair_xor_simple_copula() {
    // Both branches and both entities must appear; output must be a disjunction
    let out = ok("Of Tara and Bessie, one is tall and the other is short.");
    assert!(out.contains("Tara"), "Tara must appear; got: {}", out);
    assert!(out.contains("Bessie"), "Bessie must appear; got: {}", out);
    assert!(out.contains("Tall") || out.contains("tall"), "tall must appear; got: {}", out);
    assert!(out.contains("Short") || out.contains("short"), "short must appear; got: {}", out);
    assert!(out.contains("∨") || out.contains(" ∨ ") || out.contains("or"), "must be disjunction; got: {}", out);
}

#[test]
fn of_pair_xor_transitive_verb() {
    let out = ok("Of Tara and Bessie, one danced the hustle and the other danced the lindy.");
    assert!(out.contains("Tara"), "Tara must appear; got: {}", out);
    assert!(out.contains("Bessie"), "Bessie must appear; got: {}", out);
    assert!(out.contains("hustle") || out.contains("Hustle"), "hustle must appear; got: {}", out);
    assert!(out.contains("lindy") || out.contains("Lindy"), "lindy must appear; got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "must be disjunction; got: {}", out);
}

#[test]
fn of_pair_xor_with_relative_clause() {
    // "Of the person who danced sixth and Patti, one scored 190 points and the other performed the lindy."
    let out = ok("Of the person who danced sixth and Patti, one scored 190 points and the other performed the lindy.");
    assert!(out.contains("Patti"), "Patti must appear; got: {}", out);
    assert!(out.contains("190") || out.contains("points") || out.contains("Points"), "score must appear; got: {}", out);
    assert!(out.contains("lindy") || out.contains("Lindy"), "lindy must appear; got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "must be disjunction; got: {}", out);
}

#[test]
fn of_pair_xor_location_predicate() {
    let out = ok("Of the 1848 home and the Evans home, one is Woodlawn and the other is on Rosewood Street.");
    assert!(out.contains("Woodlawn") || out.contains("woodlawn"), "Woodlawn must appear; got: {}", out);
    assert!(out.contains("Rosewood") || out.contains("rosewood"), "Rosewood must appear; got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "must be disjunction; got: {}", out);
}

// ── EitherOr: mid-sentence disjunctive predication ──────────────────────────

#[test]
fn either_or_copula_proper_names() {
    // "X is either A or B" — disjunctive identity claim
    let out = ok("The dancer is either Tara or Bessie.");
    assert!(out.contains("Tara"), "Tara must appear; got: {}", out);
    assert!(out.contains("Bessie"), "Bessie must appear; got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "must be disjunction; got: {}", out);
}

#[test]
fn either_or_verb_object() {
    // "X danced either A or B" — disjunctive theme
    let out = ok("Tara danced either the hustle or the lindy.");
    assert!(out.contains("Tara"), "got: {}", out);
    assert!(out.contains("hustle") || out.contains("Hustle"), "got: {}", out);
    assert!(out.contains("lindy") || out.contains("Lindy"), "got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "must be disjunction; got: {}", out);
}

#[test]
fn either_or_building_predicate() {
    // Real puzzle clue
    let out = ok("The building is either the 1855 building or the Evans house.");
    assert!(out.contains("1855") || out.contains("Evans"), "got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "must be disjunction; got: {}", out);
}

#[test]
fn either_or_was_past_tense() {
    let out = ok("Frank was either the player who wore number 29 or the person who played second base.");
    assert!(out.contains("Frank"), "got: {}", out);
    assert!(out.contains("29") || out.contains("second"), "got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "must be disjunction; got: {}", out);
}

// ── Passive + bare "sometime before/after" descriptive standard ──────────────

#[test]
fn passive_bare_temporal_of_phrase_standard() {
    let out = ok("The photo was taken sometime before the photo of the red panda.");
    assert!(out.contains("Take(Photo)") && out.contains("Before(Photo,"),
        "passive + bare-before relates the theme to the standard; got: {}", out);
    assert!(out.contains("Photo(") && out.contains("Panda"),
        "the descriptive standard survives; got: {}", out);
}

#[test]
fn passive_bare_temporal_possessive_standard() {
    let out = ok("The bird was bought sometime before Faye's pet.");
    assert!(out.contains("Before(Bird,") && out.contains("Pet(") && out.contains("Possesses(Faye"),
        "possessive standard preserved; got: {}", out);
}

#[test]
fn passive_count_offset_unchanged() {
    // Regression: the count/unit offset path still works.
    let out = ok("The photo was taken 1 month after the tree.");
    assert!(out.contains("Month(Photo) = Add(Month("), "got: {}", out);
}

// ── Distinct-participle reduced relatives (ergative verbs) ───────────────────

#[test]
fn distinct_participle_reduced_relative_grown() {
    // "grown" (participle ≠ "grew" past) is unambiguously non-finite → a passive
    // reduced relative even though "grow" is ergative (not transitive-marked).
    let out = ok("The flower grown in Hardy is red.");
    assert!(out.contains("Grow(") && out.contains("In(") && out.contains("Hardy") && out.contains("Red("),
        "grown-in reduced relative attaches; got: {}", out);
}

#[test]
fn ergative_past_tense_stays_main_clause() {
    // Regression: "grew" (the PAST tense, ≠ participle) is a main clause, NOT a
    // reduced relative — must keep its event.
    let out = ok("The flower grew in Hardy.");
    assert!(out.contains("Grow(e)") && out.contains("∃e"),
        "past-tense ergative stays a main-clause event; got: {}", out);
}

// ── List-subject AllDifferent over DISTINCT entities ─────────────────────────

#[test]
fn list_alldifferent_distinct_entities_not_vacuous() {
    // The core logic-grid constraint: members must be DISTINCT entities so ¬x=y is
    // MEANINGFUL — previously all reduced to the same `Box` constant → vacuous
    // ¬Box=Box (a constraint that does nothing, useless to the prover).
    let out = ok("The red box, the blue box, and the green box are three different boxes.");
    assert!(out.contains("Red(") && out.contains("Blue(") && out.contains("Green("),
        "each member's distinguishing adjective kept; got: {}", out);
    assert!(out.contains("∃x") && out.contains("∃y") && out.contains("∃z"),
        "three distinct existential entities; got: {}", out);
    assert!(!out.contains("¬Box = Box"), "distinctness must NOT be vacuous; got: {}", out);
}

#[test]
fn list_alldifferent_proper_names_distributive() {
    // Regression: bare proper-name members stay distributive constants.
    let out = ok("Tara, Bessie, and Frank are reptiles.");
    assert!(out.contains("Reptiles(Tara)") && out.contains("Reptiles(Bessie)")
        && out.contains("Reptiles(Frank)"), "got: {}", out);
}

// ── Copula comparative with a descriptive standard ───────────────────────────
// "X is [N unit] ADJ-er than <standard>" where the standard carries a PP or a
// relative clause — the standard becomes a distinct ∃-entity with its restrictor.

#[test]
fn copula_comparative_pp_standard() {
    let out = ok("The gnome is shorter than the figure with the yellow hat.");
    assert!(out.contains("Figure(") && out.contains("With(") && out.contains("Shorter(Gnome,"),
        "the standard's PP must survive as a distinct entity; got: {}", out);
}

#[test]
fn copula_comparative_reduced_relative_standard() {
    let out = ok("The tank is 5 gallons smaller than the tank going to Philo.");
    assert!(out.contains("Go(") && out.contains("To(") && out.contains("Philo"),
        "the standard's reduced relative must survive; got: {}", out);
    assert!(out.contains("Smaller(Tank,") && out.contains("5 gallons"), "measure kept; got: {}", out);
}

#[test]
fn copula_comparative_relative_clause_standard() {
    let out = ok("Bob is shorter than the person who paid.");
    assert!(out.contains("Person(") && out.contains("Pay") && out.contains("Shorter(Bob,"),
        "the standard's relative clause must survive; got: {}", out);
}

#[test]
fn copula_comparative_bare_standard_unchanged() {
    // Regression: a bare standard stays a constant.
    let out = ok("The tank is shorter than the box.");
    assert!(out.contains("Shorter(Tank, Box)"), "got: {}", out);
}

// ── Impersonal "the one" & possessive "had" ──────────────────────────────────

#[test]
fn impersonal_the_one_with_pp() {
    // "the one with 804 followers" — "one" is the impersonal pronoun, not the
    // numeral 1.
    let out = ok("The one with 804 followers is happy.");
    assert!(out.contains("One(") && out.contains("804"), "got: {}", out);
}

#[test]
fn possessive_had_main_clause() {
    // "Bob had the port" — possessive HAVE in the past, not a perfect auxiliary.
    let out = ok("Bob had the port.");
    assert!(out.contains("Have(e)") && out.contains("Theme(e, Port)") && out.contains("Past"),
        "possessive had → past Have event; got: {}", out);
}

#[test]
fn possessive_had_in_relative_clause_standard() {
    // "than the one who had the bordeaux" — impersonal one + possessive-had rel
    // clause, as an arithmetic-comparative standard.
    let out = ok("Bob paid 1 dollar more than the one who had the bordeaux.");
    assert!(out.contains("One(") && out.contains("Have(") && out.contains("Bordeaux"),
        "the one who had X preserved as the standard; got: {}", out);
    assert!(out.contains("Add(Dollar("), "solver-ready offset; got: {}", out);
}

#[test]
fn past_perfect_had_run_unchanged() {
    // Regression: "had + participle" stays the past perfect.
    let out = ok("John had run.");
    assert!(out.contains("Perf") && out.contains("Run"), "past perfect preserved; got: {}", out);
}

// ── Redundant-unit comparative & control-verb eventive use ───────────────────

#[test]
fn redundant_unit_comparative_votes_more_votes() {
    // "received 7 votes more votes than Ken" — the unit before AND after the
    // comparative; the redundant one is consumed so the offset still parses.
    let out = ok("The dentist received 7 votes more votes than Ken.");
    assert!(out.contains("Vote(Dentist) = Add(Vote(Ken), 7)"),
        "solver-ready vote offset; got: {}", out);
}

#[test]
fn control_verb_eventive_with_pp() {
    // "start" is a subject-control verb but here is plain eventive — its PP adjunct
    // must attach (no infinitival complement follows).
    let out = ok("The outing started at Greektown.");
    assert!(out.contains("Start(e)") && out.contains("At(e, Greektown)"),
        "eventive control verb keeps its PP adjunct; got: {}", out);
}

#[test]
fn control_verb_with_infinitive_unchanged() {
    // Regression guard: "wants to play" must still be a control structure.
    let out = ok("The child wants to play.");
    assert!(out.contains("Play"), "control-with-infinitive preserved; got: {}", out);
}

// ── Of-pair XOR with verb-object / coordinate VPs ────────────────────────────
// "Of A and B, one VP₁ and the other VP₂" — VP₁ must be bounded at the "and the
// other" marker so a verb-object VP ("one teaches yoga and …") does not swallow
// "and the other …" as coordinated objects.

#[test]
fn of_pair_verb_object_vp() {
    let out = ok("Of the box and Frank, one teaches yoga and the other is blue.");
    assert!(out.contains("Teach") && out.contains("Yoga"), "VP₁ verb+object kept; got: {}", out);
    assert!(out.contains("Blue"), "VP₂ kept; got: {}", out);
    assert!(out.contains("∨"), "XOR disjunction; got: {}", out);
}

#[test]
fn of_pair_two_verb_object_vps() {
    let out = ok("Of the box and Frank, one wanted raspberries and the other asked for kale.");
    assert!(out.contains("Want") && out.contains("Raspberries"), "VP₁ kept; got: {}", out);
    assert!(out.contains("Ask") && out.contains("Kale"), "VP₂ kept; got: {}", out);
}

#[test]
fn of_pair_proper_names_unchanged() {
    // Regression guard: the bounded-VP rewrite must keep proper-name of-pairs intact.
    let out = ok("Of Tara and Bessie, one danced the hustle and the other danced the lindy.");
    assert!(out.contains("Hustle") && out.contains("Lindy"), "got: {}", out);
    assert!(out.contains("Tara") && out.contains("Bessie"), "got: {}", out);
}

// ── Clock-time literals (orderable minutes-from-midnight) ────────────────────
// "9:30am" / "8:15 pm" (space variant) are time VALUES the prover can order,
// represented as minutes-from-midnight. As a PP object ("is at 9:30am") or a
// pre-nominal modifier ("the 8:15 pm event").

#[test]
fn time_literal_pp_object_no_space() {
    let out = ok("Frank is at 9:30am.");
    assert!(out.contains("At(Frank, 570)"), "9:30am = 570 min from midnight; got: {}", out);
}

#[test]
fn time_literal_pp_object_with_space() {
    // "8:15 pm" — space before pm must still lex as one time literal.
    let out = ok("The meeting is at 8:15 pm.");
    assert!(out.contains("At(") && out.contains("1215"), "8:15pm = 1215 min; got: {}", out);
}

#[test]
fn time_literal_prenominal_modifier() {
    let out = ok("The 8:15 pm event is important.");
    assert!(out.contains("1215_event"), "time names the head noun; got: {}", out);
    assert!(out.contains("Important("), "matrix kept; got: {}", out);
}

// ── Copula degree-adverb comparatives ("is somewhat shorter than") ───────────

#[test]
fn copula_somewhat_comparative() {
    // "somewhat" is a degree adverb (lexicon degree_adverbs) — skipped before the
    // comparative so the comparison parses (strict, no measurable offset).
    let out = ok("The gnome is somewhat shorter than the figure.");
    assert!(out.contains("Shorter(Gnome, Figure)"), "got: {}", out);
}

#[test]
fn copula_much_comparative() {
    let out = ok("The gnome is much taller than the figure.");
    assert!(out.contains("Taller(Gnome, Figure)"), "got: {}", out);
}

#[test]
fn copula_plain_adjective_not_eaten_by_degree_skip() {
    // Regression guard: a bare adjective complement is NOT a degree adverb.
    let out = ok("The gnome is happy.");
    assert!(out.contains("Happy("), "plain adjective complement preserved; got: {}", out);
}

// ── Either/or copula complement keeps reduced-relative PPs ───────────────────

#[test]
fn either_or_complement_keeps_reduced_relative() {
    // "is either the medicine sourced from a fig or the beetle" — the reduced
    // relative must survive in the disjunct (was dropped to Medicine(x) ∨ Beetle(x)).
    let out = ok("The drug is either the medicine sourced from a fig or the beetle.");
    assert!(out.contains("Source(") && out.contains("From(") && out.contains("Fig"),
        "the disjunct's reduced relative must be preserved; got: {}", out);
    assert!(out.contains("∨"), "still a disjunction; got: {}", out);
}

#[test]
fn either_or_complement_keeps_made_of() {
    let out = ok("Bonita's piece is either the $250 piece or the piece made of teak.");
    assert!(out.contains("Make(") && out.contains("Of(") && out.contains("Teak"),
        "made-of reduced relative kept in the disjunct; got: {}", out);
}

// ── Past-participle passive reduced relatives (transitive verbs) ─────────────
// "the medicine sourced from a fig", "the chair made of teak" — a past participle
// of a TRANSITIVE verb + PP (no object) is an unambiguous passive reduced relative
// restricting the NP. Lexical transitivity (lexicon Feature::Transitive) is the
// discriminator from an intransitive main clause ("the box arrived in April").

#[test]
fn transitive_reduced_relative_made_of() {
    let out = ok("The chair made of teak is heavy.");
    assert!(out.contains("Make(") && out.contains("Of(") && out.contains("Teak"),
        "made-of reduced relative restricts the chair; got: {}", out);
    assert!(out.contains("Heavy("), "matrix predicate kept; got: {}", out);
}

#[test]
fn transitive_reduced_relative_in_of_pair_member() {
    let out = ok("Of the drug sourced from a fig and the beetle, one is red and the other is blue.");
    assert!(out.contains("Source(") && out.contains("From(") && out.contains("Fig"),
        "sourced-from reduced relative preserved in an of-pair member; got: {}", out);
}

#[test]
fn intransitive_plus_pp_stays_main_clause() {
    // Regression guard: an INTRANSITIVE verb + PP is a main clause, NOT a reduced
    // relative — must keep its event and not collapse to a bare NP.
    let out = ok("The box arrived in April.");
    assert!(out.contains("Arrive(") && out.contains("∃e"),
        "intransitive + PP stays a main-clause event; got: {}", out);
}

#[test]
fn ergative_sold_for_price_stays_main_clause() {
    // "sell" is excluded from transitive-reduced-relative: "X sold for $N" is a
    // main clause (price), not "X [that was] sold for $N".
    let out = ok("The stamp sold for $105.");
    assert!(out.contains("Sell(") && out.contains("For(") && out.contains("105"),
        "sold-for stays a price main clause; got: {}", out);
}

// ── Rate comparatives ("less per gallon than") ───────────────────────────────
// The per-unit RATE basis must be preserved (folded into the measure name) so the
// prover compares per-unit prices, not raw amounts. Dropping "per gallon" would be
// a meaning-loss parse.

#[test]
fn rate_comparative_vague() {
    let out = ok("The store charges less per gallon than the business.");
    assert!(out.contains("Charge_per_Gallon"), "rate basis folded into measure; got: {}", out);
    assert!(out.contains("Less("), "vague rate comparison is strict; got: {}", out);
}

#[test]
fn rate_comparative_with_offset() {
    let out = ok("The store charges 5 dollars less per gallon than the business.");
    assert!(out.contains("Charge_per_Gallon(Store) = Sub(Charge_per_Gallon("),
        "solver-ready rate offset; got: {}", out);
    assert!(out.contains("5 dollars"), "the offset's currency is preserved; got: {}", out);
}

#[test]
fn rate_comparative_for_price() {
    let out = ok("The carrots sell for 10 cents less per pound than the beets.");
    assert!(out.contains("Sell_per_Pound") && out.contains("Sub(Sell_per_Pound("),
        "for-price + per-pound rate, solver-ready; got: {}", out);
}

// ── Proper-name passive subjects (constant reading + PP/offset) ──────────────
// A proper-name passive subject is a specific referent (a constant), not an
// existential type — and carries trailing PP adjuncts and calendar-unit offsets.

#[test]
fn proper_name_passive_is_constant() {
    let out = ok("Pluniden was approved.");
    assert!(out.contains("Approve(Pluniden)"), "proper name → constant theme; got: {}", out);
    assert!(!out.contains("Pluniden(x)"), "must NOT be an existential type reading; got: {}", out);
}

#[test]
fn proper_name_passive_with_temporal_offset() {
    let out = ok("Pluniden was approved 2 months after Influenza.");
    assert!(out.contains("Months(Pluniden) = Add(Months(Influenza), 2)"),
        "solver-ready offset on a proper-name passive; got: {}", out);
}

#[test]
fn proper_name_passive_offset_relative_clause_standard() {
    let out = ok("Pluniden was approved 2 months after the medicine that treats influenza.");
    assert!(out.contains("Medicine(") && out.contains("Treat"),
        "the rel-clause standard must be preserved as a distinct entity; got: {}", out);
    assert!(out.contains("Add(Months("), "solver-ready offset; got: {}", out);
}

#[test]
fn bare_plural_passive_stays_existential() {
    // Regression guard: a bare plural is a TYPE (existential), not a constant.
    let out = ok("Drugs were approved.");
    assert!(out.contains("∃") && out.contains("Drugs("),
        "bare plural keeps its existential-over-type reading; got: {}", out);
}

// ── Embedded passive/coordinate VPs (of-pair members, delegated clauses) ─────
// The embedded VP parser must handle passive participles with trailing PP
// adjuncts ("was found in Spain"), coordinate objects ("was at 88.2 W"), and a
// calendar-unit offset ("was taken 1 month after Y") — as capable as the main
// passive path — so of-pair "one VP₁ and the other VP₂" doesn't drop them.

#[test]
fn of_pair_passive_member_with_pp() {
    let out = ok("Of the box and Frank, one was found in Spain and the other is blue.");
    assert!(out.contains("Find") && out.contains("In(") && out.contains("Spain"),
        "passive member's verb + PP must survive; got: {}", out);
    assert!(out.contains("∨"), "XOR disjunction; got: {}", out);
}

#[test]
fn of_pair_coordinate_member_vp() {
    let out = ok("Of the box and Frank, one is red and the other was at 88.2 W.");
    assert!(out.contains("At(") && out.contains("88.2 W"),
        "coordinate VP object must survive; got: {}", out);
}

#[test]
fn embedded_passive_temporal_offset() {
    let out = ok("Of the box and Frank, one was taken 1 month after Ken and the other is blue.");
    assert!(out.contains("Month(") && out.contains("Add(Month(Ken), 1)"),
        "passive member's calendar offset must be solver-ready; got: {}", out);
}

#[test]
fn next_cycle_not_eaten_as_pp() {
    // Regression guard: the embedded PP-adjunct loop must NOT swallow an SVA
    // cycle-temporal delay (kept for the temporal wrappers).
    let out = ok("The box was found in Spain.");
    assert!(out.contains("In(Box, Spain)"), "plain passive+PP still works; got: {}", out);
}

// ── Ordinal dates ─────────────────────────────────────────────────────────────
// "Month Nth" (ordinal day) names the same date as "Month N"; an attributive
// date absorbs the head noun it modifies ("the April 15th birthday").

#[test]
fn ordinal_date_attributive_compound() {
    let out = ok("The child has the April 15th birthday.");
    assert!(out.contains("April_15_birthday"),
        "ordinal date + head noun must compound; got: {}", out);
}

#[test]
fn ordinal_date_with_age_measure() {
    let out = ok("The child with the April 15th birthday is 12 years old.");
    assert!(out.contains("April_15_birthday"), "date kept; got: {}", out);
    assert!(out.contains("Old(x, 12 years)") || out.contains("12 years"),
        "age measure kept; got: {}", out);
}

// ── Passive + calendar-unit offset ───────────────────────────────────────────
// "X was V-ed N <unit> (after|before) Y" → the passive event AND a solver-ready
// temporal offset on the theme: Unit(X) = add|sub(Unit(Y), N).

#[test]
fn passive_with_temporal_offset_after() {
    let out = ok("The photo was taken 1 month after the tree.");
    assert!(out.contains("Take"), "passive event kept; got: {}", out);
    assert!(out.contains("Month(Photo) = Add(Month("), "solver-ready offset (after→Add); got: {}", out);
}

#[test]
fn passive_with_temporal_offset_before_is_sub() {
    let out = ok("The shot was taken 1 month before Ken.");
    assert!(out.contains("Sub(Month(Ken), 1)"), "before→Sub over the bare-name standard; got: {}", out);
}

#[test]
fn active_temporal_offset_unchanged_after_refactor() {
    // Regression guard for the shared-constraint refactor: the active VP path
    // must stay byte-identical.
    let out = ok("Tara performed 2 weeks after Bessie.");
    assert!(out.contains("Weeks(Tara) = Add(Weeks(Bessie), 2)"), "got: {}", out);
}

// ── NeitherNor: correlative negation with descriptive disjuncts ──────────────
// "Neither A nor B VP" → ¬VP(A) ∧ ¬VP(B). Each disjunct may be a multi-word
// proper name, a possessive, or a description with PPs / relative clauses — all
// preserved with ZERO meaning loss (a description binds a distinct ∃-variable
// carrying its restrictor; a bare name/definite stays a constant).

#[test]
fn neither_nor_proper_names_preserved() {
    // Regression guard: bare proper names stay constants, byte-identical to before.
    let out = ok("Neither John nor Mary came.");
    assert!(out.matches('¬').count() >= 2, "both disjuncts negated; got: {}", out);
    assert!(out.contains("John") && out.contains("Mary"), "got: {}", out);
}

#[test]
fn neither_nor_multiword_name_and_pp_description() {
    let out = ok("Neither Belle Grove nor the home on Wright Street is the 1855 building.");
    assert!(out.contains("Belle_Grove"), "multi-word proper name kept as constant; got: {}", out);
    assert!(out.contains("On(") && out.contains("Wright_Street"),
        "the descriptive disjunct's PP must be preserved; got: {}", out);
    assert!(out.matches('¬').count() >= 2, "both disjuncts negated; got: {}", out);
}

#[test]
fn neither_nor_possessive_and_relative_disjuncts() {
    let out = ok("Neither Pam's client nor the person who paid $150 was the client.");
    assert!(out.contains("Possesses(Pam"), "possessive disjunct must keep the possessor; got: {}", out);
    assert!(out.contains("Pay") || out.contains("150"),
        "relative-clause disjunct must keep 'who paid $150'; got: {}", out);
}

#[test]
fn neither_nor_bare_definites_stay_constants() {
    // No restrictions to lose → bare definites remain simple constants.
    let out = ok("Neither the bookshelf nor the lamp is red.");
    assert!(out.contains("¬Red(Bookshelf)") || out.contains("Red(Bookshelf)"), "got: {}", out);
    assert!(out.contains("¬Red(Lamp)") || out.contains("Red(Lamp)"), "got: {}", out);
}

// ── Possessives ───────────────────────────────────────────────────────────────

#[test]
fn simple_possessive_straight_apostrophe() {
    let out = ok("Jon's episode aired first.");
    assert!(out.contains("Jon") || out.contains("jon"), "got: {}", out);
    assert!(out.contains("episode") || out.contains("Episode"), "got: {}", out);
}

#[test]
fn multi_word_possessor() {
    // "Alvarado family's house" — multi-word possessor
    let out = ok("The Alvarado family's house is on Cora Street.");
    assert!(out.contains("Alvarado"), "Alvarado must appear; got: {}", out);
    assert!(out.contains("house") || out.contains("House"), "house must appear; got: {}", out);
    assert!(out.contains("Cora") || out.contains("Street"), "location must appear; got: {}", out);
}

#[test]
fn possessor_modifier_survives_bound_to_possessor() {
    // ZERO MEANING LOSS: "the red team's trophy" — the possessor "the red team"
    // carries an adjective. Reducing the possessor to the bare constant `team`
    // silently drops "red": a broken parse. The possessor must become a restricted
    // entity ∃p(Team(p) ∧ Red(p) ∧ Possesses(p, subject)).
    let out = ok("The prize is the red team's trophy.");
    assert!(out.contains("Trophy") || out.contains("trophy"), "head 'trophy' must appear; got: {}", out);
    assert!(out.contains("Possesses"), "possession relation must appear; got: {}", out);
    assert!(out.contains("Team") || out.contains("team"), "possessor noun must appear; got: {}", out);
    // The load-bearing assertion: the possessor's adjective must NOT be dropped.
    assert!(out.contains("Red") || out.contains("red"), "possessor's adjective 'red' must survive; got: {}", out);
    // …and the possessor must be a quantified entity carrying that restrictor,
    // not a bare constant (which is what strands the adjective).
    assert!(out.contains('∃'), "possessor must be an existential entity; got: {}", out);
}

#[test]
fn possessor_negation() {
    // "neither A nor B is X's Y"
    let out = ok("Neither the 1834 home nor the house on Cora Street is the Alvarado family's house.");
    assert!(out.contains("Alvarado"), "Alvarado must appear; got: {}", out);
    assert!(
        out.contains("¬") || out.contains("Neither") || out.contains("not") || out.contains("Not"),
        "negation must appear; got: {}", out
    );
}

// ── Enumerated identity with a leading proper-name member ────────────────────

#[test]
fn enumerated_identity_leading_proper_name_member() {
    // "The four people are Anthony, the waiter, the musician and the dentist." — a
    // proper-name FIRST member must not divert to the simple "X is Y" identity and
    // strand the rest; the whole list becomes the group identity (mixed proper /
    // definite members), the AllDifferent domain the solver needs.
    let out = ok("The four people are Anthony, the waiter, the musician and the dentist.");
    assert!(out.contains("Anthony"), "first (proper-name) member kept; got: {}", out);
    assert!(out.contains("Waiter") || out.contains("waiter"), "definite member kept; got: {}", out);
    assert!(out.contains("Musician") || out.contains("musician"), "member kept; got: {}", out);
    assert!(out.contains("Dentist") || out.contains("dentist"), "last member kept; got: {}", out);
    assert!(out.contains('⊕') || out.contains("="), "the group identity must be built; got: {}", out);
}

#[test]
fn enumerated_identity_described_member_keeps_restrictor() {
    // "The four winners are the person from Spain, Bob and Carol." — a DESCRIBED
    // member ("the person from Spain") must become a distinct ∃ entity carrying its
    // PP restrictor, NOT collapse to its bare head. The pre-fix output dropped "from
    // Spain" entirely (4_winners = Person ⊕ Bob ⊕ Carol) — a silent meaning loss.
    let out = ok("The four winners are the person from Spain, Bob and Carol.");
    assert!(out.contains("Spain"), "the member's PP restrictor must survive; got: {}", out);
    assert!(out.contains("From") || out.contains("from"), "the PP relation must survive; got: {}", out);
    assert!(out.contains("Bob"), "proper member kept; got: {}", out);
    assert!(out.contains("Carol"), "proper member kept; got: {}", out);
    assert!(out.contains('⊕'), "the group identity must be built; got: {}", out);
}

#[test]
fn enumerated_identity_who_relative_first_member() {
    // "The four winners are the winner who won the prize, the person from Australia,
    // Deb Daniels and Jemma Jenks." — a who-relative on the FIRST member hides the
    // comma that opens the list. The pre-fix parser entered the single "X is the Y
    // who Z" copula path and stranded the rest (TrailingTokens). Now all four members
    // survive: the relative-clause entity, the PP entity, and two proper names.
    let out = ok(
        "The four winners are the winner who won the prize, the person from Australia, Deb Daniels and Jemma Jenks.",
    );
    assert!(out.contains("Win"), "the first member's relative clause must survive; got: {}", out);
    assert!(out.contains("Prize") || out.contains("prize"), "the relative's object must survive; got: {}", out);
    assert!(out.contains("Australia"), "the PP member must survive; got: {}", out);
    assert!(out.contains("Deb_Daniels") || out.contains("Daniels"), "proper member kept; got: {}", out);
    assert!(out.contains("Jemma_Jenks") || out.contains("Jenks"), "proper member kept; got: {}", out);
    assert!(out.contains('⊕'), "the group identity must be built; got: {}", out);
}

#[test]
fn enumerated_identity_bare_definites_stay_constants() {
    // Regression guard: bare NAMED definites ("the hustle") must stay referring
    // constants in the group identity (distinct by unique name), NOT become ∃
    // entities — only DESCRIBED members open variables.
    let out = ok("The four dances are the hustle, the lindy, the twist, and the waltz.");
    assert!(out.contains("Hustle"), "named member kept as constant; got: {}", out);
    assert!(out.contains("Lindy"), "named member kept; got: {}", out);
    assert!(out.contains("Twist"), "named member kept; got: {}", out);
    assert!(out.contains("Waltz"), "named member kept; got: {}", out);
    assert!(out.contains('⊕'), "the group identity must be built; got: {}", out);
}

#[test]
fn copula_relative_without_list_is_single_identity() {
    // Guards the speculative restore: "X is the Y who Z" with NO following comma must
    // run the existing single copula-relative path unchanged — a single identity, NOT
    // a one-member group.
    let out = ok("The champion is the person who finished first.");
    assert!(out.contains("Champion") || out.contains("champion"), "subject kept; got: {}", out);
    assert!(out.contains("Person") || out.contains("person"), "predicate noun kept; got: {}", out);
    assert!(out.contains("Finish") || out.contains("finish"), "the relative clause must survive; got: {}", out);
    assert!(!out.contains('⊕'), "a non-list copula-relative must not build a group; got: {}", out);
}

#[test]
fn either_or_disjunct_keeps_pp_restrictor() {
    // "Edmund is either the child from Cornville or the child from El Monte." — the
    // disjuncts share a head ("child") and are told apart ONLY by their PP. The
    // copula either-or used bare nominal_predication (no PPs), collapsing both to
    // Child(Edmund) ∨ Child(Edmund) — indistinguishable, unsolvable. Now each PP
    // survives so the two disjuncts are genuinely different entities.
    let out = ok("Edmund is either the child from Cornville or the child from El Monte.");
    assert!(out.contains("Cornville"), "first disjunct's PP must survive; got: {}", out);
    assert!(out.contains("El_Monte") || out.contains("El Monte"), "second disjunct's PP must survive; got: {}", out);
    assert!(out.contains('∨'), "the disjunction must be built; got: {}", out);
}

#[test]
fn either_or_reduced_relative_disjunct() {
    // "Darwin is either the gator caught in Lynn or the animal that is 12.0 feet
    // long." — the first disjunct is a PASSIVE reduced relative ("caught in Lynn").
    // "catch" has past == participle and is not in the near-dead transitive table,
    // so it failed (TrailingTokens). In a copula-complement (nominal) position a
    // past-participle + PP is unambiguously a reduced relative for any non-pure-
    // intransitive verb. Both disjuncts now survive.
    let out = ok("Darwin is either the gator caught in Lynn or the animal that is 12.0 feet long.");
    assert!(out.contains("Catch") || out.contains("catch"), "the reduced relative's verb must survive; got: {}", out);
    assert!(out.contains("Lynn"), "the reduced relative's PP must survive; got: {}", out);
    assert!(out.contains("Long") || out.contains("long"), "the second disjunct must survive; got: {}", out);
    assert!(out.contains('∨'), "the disjunction must be built; got: {}", out);
}

#[test]
fn subject_main_clause_not_a_passive_reduced_relative() {
    // Regression guard for the nominal-context relaxation: in SUBJECT position
    // (nominal_np_context = false) a transitive-capable past verb + PP is the MAIN
    // clause, never a passive reduced relative — "The chef cooked in Paris" must read
    // ∃e(Cook(e) ∧ Agent(e, chef) …), not "the chef [that was] cooked in Paris". The
    // relaxation fires ONLY in a nominal complement position.
    let out = ok("The chef cooked in Paris.");
    assert!(out.contains("Cook") || out.contains("cook"), "the main verb must survive; got: {}", out);
    assert!(out.contains("Agent"), "the chef must be the event's agent, not a passive theme; got: {}", out);
    assert!(out.contains("Paris"), "the PP must survive; got: {}", out);
}

#[test]
fn who_s_contraction_is_copula_not_possessive() {
    // "who's going" = "who IS going" — the 's after a WH-relativizer / subject pronoun
    // is the copula clitic, not a possessive (those words never take possessive 's;
    // their genitive is the dedicated form whose/its/his). Previously the lexer emitted
    // a Possessive token → ExpectedContentWord. Now the relative clause parses.
    let out = ok("Felicia is either the person who's going to Stanford or the person who's going to Harvard.");
    assert!(out.contains("Stanford"), "first disjunct's relative clause must survive; got: {}", out);
    assert!(out.contains("Harvard"), "second disjunct's relative clause must survive; got: {}", out);
    assert!(out.contains('∨'), "the disjunction must be built; got: {}", out);
    assert!(out.contains("Go") || out.contains("go"), "the contracted copula's progressive verb must survive; got: {}", out);
}

#[test]
fn possessive_clitic_after_noun_is_not_a_copula_contraction() {
    // Regression guard for the 's-contraction rule: 's after a NOUN / proper name / an
    // indefinite pronoun ("someone") is the genitive, NOT "is". The contraction only
    // fires for subject pronouns and wh-relativizers.
    let house = ok("The Woodard family's house is large.");
    assert!(house.contains("Woodard") && (house.contains("House") || house.contains("house")),
        "the possessor and possessed must survive; got: {}", house);
    let someone = ok("Someone's car is red.");
    assert!(someone.contains("Car") || someone.contains("car"), "possessive head kept; got: {}", someone);
    assert!(someone.contains("Have") || someone.contains("Possess"),
        "'someone's car' must stay a possessive, not 'someone is car'; got: {}", someone);
}

#[test]
fn native_is_an_english_word_not_an_ffi_keyword() {
    // "native" is the Logos FFI-modifier keyword in code, but in a natural-language
    // clue it is an ordinary noun ("the Oregon native" = a native person) or adjective
    // ("the native culture"). The lexer kept emitting TokenType::Native in Declarative
    // (clue) mode — now gated to Imperative, and "native" is a Human-sort noun.
    let person = ok("The New Mexico native is the architect.");
    assert!(person.contains("New_Mexico_native") || person.contains("Native"),
        "'native' must head the NP as a noun; got: {}", person);
    assert!(person.contains("Architect") || person.contains("architect"),
        "the predicate must survive; got: {}", person);
    let adj = ok("The native culture was Lakota.");
    assert!(adj.contains("Native") && adj.contains("Lakota"),
        "'native culture' and its identity must survive; got: {}", adj);
}

#[test]
fn item_is_an_english_noun_not_a_collection_keyword() {
    // "item"/"items" are code collection-op keywords that were missing the Imperative
    // gate their sibling ops (push/pop/length/size) have, so they pre-empted the
    // ordinary English noun and "the blue ITEM" stranded ("item" trailed after the
    // adjective). Now "item" is a plain noun in clues and composes with modifiers.
    let blue = ok("The blue item is red.");
    assert!(blue.contains("Item") && blue.contains("Blue"),
        "'item' must head the NP and keep its adjective; got: {}", blue);
    // it composes with everything, e.g. a verbal comparative standard
    let cmp = ok("The phone case took 10 more minutes to print than the blue item.");
    assert!(cmp.contains("Item") && cmp.contains("Add"),
        "'the blue item' as a comparative standard must survive; got: {}", cmp);
}

#[test]
fn reduced_relative_subject_with_do_support_or_perfect_vp() {
    // "The book priced at $55 doesn't have a cover." — a subject with a reduced
    // relative ("priced at $55") followed by a do-support / perfect matrix VP. The
    // subject-reduced-relative detector only accepted a copula or finite verb as the
    // matrix, so do-support ("doesn't") / "had" made it bail and the reduced-relative
    // verb became the main clause, stranding the real predicate. Now both survive.
    let neg = ok("The book priced at $55 doesn't have a cover.");
    assert!(neg.contains("Price") && neg.contains("At"),
        "the subject's reduced relative must survive; got: {}", neg);
    assert!(neg.contains("Cover") && neg.contains("Have"),
        "the do-support matrix VP must survive; got: {}", neg);
    assert!(neg.contains('¬'), "the negation must survive; got: {}", neg);
    let perf = ok("The well cut through gravel had a leak.");
    assert!(perf.contains("Cut") && perf.contains("Gravel"),
        "the reduced relative survives with a perfect matrix; got: {}", perf);
    assert!(perf.contains("Leak"), "the matrix VP survives; got: {}", perf);
}

#[test]
fn subject_reduced_relative_with_leading_ordinal_adverb() {
    // "The peak FIRST climbed in 1845 is tall." — a leading ordinal/temporal adverb on
    // a subject reduced relative hid the participle from the detector, stranding the
    // clue (TrailingTokens at the adverb). The adverb is now consumed as part of the
    // relative and surfaces as a modifier over the subject, so nothing is lost.
    let s = ok("The peak first climbed in 1845 is tall.");
    assert!(s.contains("Climb"), "the reduced-relative verb must survive; got: {}", s);
    assert!(s.contains("In(") && s.contains("1845"),
        "the date PP must survive; got: {}", s);
    assert!(s.contains("First"), "the leading adverb must survive; got: {}", s);
    assert!(s.contains("Tall"), "the matrix predicate must survive; got: {}", s);
}

#[test]
fn perfect_aspect_relative_clause() {
    // "the skydiver who HAS DONE 49 jumps" — a relative clause headed by the perfect
    // auxiliary + participle. `parse_relative_clause` greedily read "has" as possessive
    // HAVE and stranded the participle (TrailingTokens at "done"/"won"). Now it routes
    // to the aspect chain like a main-clause perfect, so the whole event survives.
    let pred = ok("Jorge is the skydiver who has done 49 jumps.");
    assert!(pred.contains("Skydiver(Jorge)"),
        "the predicate-nominal head must survive; got: {}", pred);
    assert!(pred.contains("Do") && pred.contains("49") && pred.contains("Jump"),
        "the perfect-aspect event and its object must survive; got: {}", pred);
    assert!(pred.contains("Perf"),
        "the perfect aspect must be marked; got: {}", pred);
    // Also as a subject relative: "The skydiver who has done 49 jumps is happy."
    let subj = ok("The skydiver who has done 49 jumps is happy.");
    assert!(subj.contains("Skydiver") && subj.contains("Do") && subj.contains("Jump"),
        "the subject relative's perfect event must survive; got: {}", subj);
    assert!(subj.contains("Happy"), "the matrix must survive; got: {}", subj);
}

#[test]
fn where_locative_relative_clause() {
    // "the episode WHERE the survivor brought the knife" — a locative/circumstance
    // relative with its OWN subject; the head is the LOCATION of the clause's event.
    // It stranded at "where" (TrailingTokens). Now the embedded subject + VP parse and
    // the event is located at the head via In(e, head).
    let s = ok("The episode where the survivor brought the knife is good.");
    assert!(s.contains("Bring") && s.contains("Survivor") && s.contains("Knife"),
        "the embedded clause must survive in full; got: {}", s);
    assert!(s.contains("In(e,") || s.contains("In("),
        "the head must be the locative of the event; got: {}", s);
    assert!(s.contains("Good"), "the matrix must survive; got: {}", s);
    // Pronoun subject in the where-clause.
    let pron = ok("The trip where they saw 16 stars was fun.");
    assert!(pron.contains("See") && pron.contains("They"),
        "a pronoun embedded subject must survive; got: {}", pron);
    assert!(pron.contains("In("), "the locative must survive; got: {}", pron);
}

#[test]
fn exactly_before_a_count_is_a_redundant_exactness_marker() {
    // "doesn't serve exactly 2 people", "has exactly 804 followers" — "exactly N" is a
    // redundant exactness marker before a count/measure (the value is already exact in
    // the FOL), and it stranded the count (TrailingTokens{Number}). It is dropped, so
    // the count parses identically to the bare form, with zero meaning loss.
    let serve = ok("The ham serves exactly 2 people.");
    assert!(serve.contains("Serve") && serve.contains("2 people"),
        "the count must survive after exactly; got: {}", serve);
    let neg = ok("Tina doesn't have exactly 804 Twitter followers.");
    assert!(neg.contains("804") && neg.contains('¬'),
        "the negated exact count must survive; got: {}", neg);
    // "exactly" NOT before a number is untouched (stays an adverb).
    assert_eq!(
        ok("The ham serves 2 people."),
        serve,
        "exactly N must compile identically to the bare count"
    );
}

#[test]
fn noun_incorporation_gerund_compound_object() {
    // "started WEIGHT LIFTING", "enjoys BIRD WATCHING" — a clause-final gerund after a
    // noun head is a noun-incorporation compound object (the noun is the gerund's
    // incorporated object). The gerund stranded; now it folds to Weight_lifting.
    let s = ok("Jackie started weight lifting.");
    assert!(s.contains("Weight_lifting") && s.contains("Start"),
        "the compound gerund object must survive; got: {}", s);
    // A reduced relative with its own object is untouched ("the man lifting WEIGHTS").
    let rel = ok("The man lifting weights is strong.");
    assert!(rel.contains("Lift") && rel.contains("Weights") && !rel.contains("Man_lifting"),
        "a reduced relative must not be mis-folded; got: {}", rel);
}

#[test]
fn measure_premodified_compound_head_subject() {
    // "the 60 gallon FISH TANK" — a numeric-measure premodifier on a MULTI-WORD
    // compound head. The compound loop broke at the ambiguous "fish" in a subject and
    // stranded the rest; now the measured compound folds into one head.
    let s = ok("The 60 gallon fish tank is red.");
    assert!(s.contains("60_gallon_fish_tank"),
        "the whole measured compound must fold into one head; got: {}", s);
    assert!(s.contains("Red"), "the matrix predicate must survive; got: {}", s);
    // Single-word measured head and the noun/verb ambiguity guard are unaffected.
    assert!(ok("The 60 gallon tank is red.").contains("60_gallon_tank"));
    assert!(ok("Time flies.").contains("Fly"), "ambiguity guard must hold");
}

#[test]
fn postposed_worth_measure_on_subject() {
    // "the magnate WORTH $27 billion" — a postposed measure-adjective on the subject
    // (the same Worth(x, measure) the copula complement builds). It stranded
    // (TrailingTokens{Adjective}); now it surfaces as a restrictor with zero loss.
    let s = ok("The magnate worth $27 billion is rich.");
    assert!(s.contains("Worth") && s.contains("27 billion"),
        "the worth-measure restrictor must survive; got: {}", s);
    assert!(s.contains("Rich"), "the matrix predicate must survive; got: {}", s);
}

#[test]
fn passive_made_of_material_complement() {
    // "X was made OF rosewood" — a passive participle's "of"-complement is the MATERIAL,
    // not a possessive; the passive PP loop excluded "of" and stranded it. Now it lowers
    // to Of(x, material), like the "made of teak" reduced relative.
    let s = ok("The $275 item was made of rosewood.");
    assert!(s.contains("Make") && s.contains("Of(") && s.contains("Rosewood"),
        "the passive material complement must survive; got: {}", s);
    // by/in passives are unaffected.
    let by = ok("The bird was trained by Bob.");
    assert!(by.contains("Train") && by.contains("Bob"),
        "a by-agent passive still works; got: {}", by);
}

#[test]
fn multi_word_proper_name_absorbs_all_caps_words() {
    // "Delta Gamma Pi", "Beta Pi Omega" — a proper name can be 3+ words; the head
    // absorbed only two ("Delta_Gamma") and stranded the third ("Pi"). Now every
    // consecutive capitalized label folds into one name.
    let s = ok("Delta Gamma Pi was founded in 1948.");
    assert!(s.contains("Delta_Gamma_Pi"),
        "all three name words must fold into one entity; got: {}", s);
    assert!(s.contains("Found") && s.contains("1948"),
        "the predicate must survive; got: {}", s);
    // Two-word names still fold (regression guard).
    let two = ok("Ray Ricardo won the prize.");
    assert!(two.contains("Ray_Ricardo"), "two-word names still fold; got: {}", two);
}

#[test]
fn capitalized_modal_or_aux_is_a_proper_name() {
    // "started by Will Waters" — a capitalized modal/auxiliary MID-sentence is a proper
    // name, not the function word ("Will" was lexed as the future auxiliary). Lowercase
    // modals/auxiliaries are unaffected.
    let name = ok("The startup was started by Will Waters.");
    assert!(name.contains("Will_Waters"),
        "the capitalized auxiliary must lex as a proper name; got: {}", name);
    let modal = ok("Alice will win.");
    assert!(modal.contains("Future") && modal.contains("Win"),
        "a lowercase modal must still be the auxiliary; got: {}", modal);
}

#[test]
fn do_support_with_performative_verb() {
    // "Chip didn't ORDER the root beer" — a bare verb the lexicon also lists as a
    // performative ("order", "call") is the clause's main verb after do-support, not a
    // speech act. It stranded as TrailingTokens{Performative}; now it is the verb.
    let s = ok("Chip didn't order the root beer.");
    assert!(s.contains("Order") && s.contains("Root_beer"),
        "the do-support verb + object must survive; got: {}", s);
    assert!(s.contains('¬'), "the negation must survive; got: {}", s);
}

#[test]
fn whose_possessive_relative_clause() {
    // "the town WHOSE mayor is Kevin King" — a possessive relative: the head's
    // possessed entity satisfies the clause. It stranded at "whose". Now whose lexes as
    // a relativizer and lowers to ∃m(Mayor(m) ∧ Possesses(head, m) ∧ <VP over m>).
    let s = ok("The town is the town whose mayor is Kevin King.");
    assert!(s.contains("Mayor") && s.contains("Possesses"),
        "the possessed entity + possession must survive; got: {}", s);
    assert!(s.contains("Kevin_King") || s.contains("Kevin"),
        "the clause's VP value must survive; got: {}", s);
}

#[test]
fn relativizers_attach_uniformly_in_members() {
    // who/that/where/whose attach UNIFORMLY at member sites via try_attach_relative —
    // either-or members, neither/nor members, predicate nominals. These corpus clues
    // each stranded at where/whose before the lift.
    let eo_whose =
        ok("Charles City is either the town with a population of 37,000 or the town whose mayor is Kevin King.");
    assert!(eo_whose.contains("Mayor") && eo_whose.contains('∨'),
        "either-or with a whose member must build the disjunction; got: {}", eo_whose);
    let eo_where =
        ok("Neil's episode is either the show that aired on April 20th or the show where the survivor brought the rope.");
    assert!(eo_where.contains("Bring") && eo_where.contains("In("),
        "either-or with a where member must keep the locative event; got: {}", eo_where);
    let nn_whose =
        ok("Neither the town whose mayor is Ida Ingram nor the town whose mayor is Gil Gonzales is Vernon.");
    assert!(nn_whose.contains("Ida_Ingram") || nn_whose.contains("Ida"),
        "neither/nor with whose members must keep both; got: {}", nn_whose);
    // Comma-list members also route through the shared dispatcher: a where-relative
    // member ("the town where they met") used to strand at "where".
    let list_where =
        ok("Alice, the town where they met, and Carol are three different places.");
    assert!(list_where.contains("In(") && list_where.contains("Meet"),
        "a list where-member must keep its locative event; got: {}", list_where);
}

#[test]
fn postnominal_comparative_dimension_is_the_noun() {
    // "has a wingspan [somewhat] LONGER than the bird" — a comparative AFTER the object
    // noun. The dimension is the NOUN (Wingspan), like the prenominal "a longer wingspan
    // than". Previously the noun was eaten as a degree modifier and the dimension
    // defaulted to the verb (Greater(Have(x), Have(y)) — lossy), and an intervening
    // degree adverb ("somewhat") made it bail entirely (TrailingTokens at the comparative).
    let bare = ok("Buddy has a wingspan longer than the bird.");
    assert!(bare.contains("Greater(Wingspan(Buddy)") || bare.contains("Wingspan(Buddy)"),
        "the dimension must be the noun (Wingspan), not the verb; got: {}", bare);
    assert!(!bare.contains("Have(Buddy), Have"),
        "the dimension must NOT default to the verb; got: {}", bare);
    let vague = ok("The instrument has a face somewhat wider than the other instrument.");
    assert!(vague.contains("Face(Instrument)"),
        "the degree adverb must not fold into the dimension; got: {}", vague);
    assert!(!vague.contains("Face_somewhat"),
        "the degree adverb must be discarded, not joined to the dimension; got: {}", vague);
}

#[test]
fn leading_adverb_in_relative_clause() {
    // "who FIRST started in 1983" — a clause-initial adverb in a relative clause hid the
    // verb from dispatch, stranding the clue (TrailingTokens) or dropping the clause to
    // `?`. It is now consumed and conjoined over the gap, so nothing is lost.
    let pred = ok("Jorge is the skydiver who first started in 1983.");
    assert!(pred.contains("Skydiver(Jorge)"),
        "the predicate-nominal head must survive; got: {}", pred);
    assert!(pred.contains("Start") && pred.contains("1983"),
        "the relative-clause event must survive; got: {}", pred);
    assert!(pred.contains("First"),
        "the clause-initial adverb must survive; got: {}", pred);
    let subj = ok("The skydiver who first started in 1983 is happy.");
    assert!(subj.contains("Start") && subj.contains("First") && subj.contains("Happy"),
        "the subject relative keeps event + adverb + matrix; got: {}", subj);
    assert!(!subj.contains('?'),
        "the relative clause must not collapse to an unknown; got: {}", subj);
}

#[test]
fn copula_complement_led_by_temporal_adverb() {
    // "was FIRST", "is NOW the leader" — a copula complement led by a temporal/ordinal
    // adverb failed with ExpectedContentWord at the adverb. A shared helper now reads it
    // in BOTH copula paths (parse_atom + parse_predicate): the adverb is conjoined with
    // any following predicate, or stands alone when it is the whole complement.
    let bare = ok("Gloria's sighting was first.");
    assert!(bare.contains("First"),
        "the bare adverbial complement must survive; got: {}", bare);
    let withnp = ok("The car is now the leader.");
    assert!(withnp.contains("Leader") && withnp.contains("Now"),
        "the predicate nominal AND the adverb must both survive; got: {}", withnp);
}

#[test]
fn temporal_ordering_with_descriptive_standard() {
    // "won her prize before the winner who won the prize in chemistry" — a BARE
    // before/after ordering after a definite/possessive object, with a relative-clause
    // standard. The definite-object path didn't chain the bare-temporal handler (Gap A)
    // and the bare path dropped a who/that relative on the standard (Gap B). Both fixed.
    let s = ok("Glenda won her prize before the winner who won the prize in chemistry.");
    assert!(s.contains("Before"), "the ordering must survive; got: {}", s);
    assert!(s.contains("Chemistry") || s.contains("chemistry"),
        "the standard's relative clause must survive; got: {}", s);
    let sail = ok("Eugene set sail before the person who took the cruise.");
    assert!(sail.contains("Before") && sail.contains("Cruise"),
        "ordering + relative-clause standard must survive; got: {}", sail);
}

#[test]
fn temporal_ordering_against_a_year() {
    // "won a prize before 1989", "happened after 2010" — a numeric year is a temporal
    // reference the prover can order; before only NP standards were accepted.
    let before = ok("Tara won a prize before 1989.");
    assert!(before.contains("Before") && before.contains("1989"),
        "the year ordering must survive; got: {}", before);
    let after = ok("The event happened after 2010.");
    assert!(after.contains("After") && after.contains("2010"),
        "the year ordering must survive; got: {}", after);
}

#[test]
fn relative_clause_copula_with_temporal_adverb() {
    // "the player who's now with the Cubs" — a temporal adverb after the relative
    // clause's copula framed the predication but stranded the clue (ExpectedContentWord
    // at the adverb). It is now conjoined over the gap.
    let s = ok("The person who's now with the Tigers graduated 1 year before Orlando.");
    assert!(s.contains("With") && s.contains("Tigers"),
        "the relative copula complement must survive; got: {}", s);
    assert!(s.contains("Now"), "the temporal adverb must survive; got: {}", s);
    assert!(s.contains("Graduate"), "the matrix VP must survive; got: {}", s);
}

#[test]
fn abbreviation_period_is_not_a_sentence_terminator() {
    // "Mr.", "Dr.", "153 ft.", "Mt." — the dot after a known abbreviation is part of
    // the abbreviation, not a clause terminator. The lexer was emitting a Period there,
    // stranding the rest of the clue (TrailingTokens / ExpectedContentWord). Now the
    // dot is suppressed mid-clue (lexicon `abbreviations`), so the whole clue parses.
    let title = ok("The patient who will be seeing Dr. Zamora is 2 years older than Noel.");
    assert!(title.contains("Zamora"), "the name after the title abbreviation must survive; got: {}", title);
    assert!(title.contains("Older") || title.contains("older"), "the comparative must survive; got: {}", title);
    let unit = ok("Of The Senator and Zeke's Spruce, one is 153 ft. tall and the other is 80 years old.");
    assert!(unit.contains("Senator") && unit.contains("80 years"),
        "both of-pair members and the unit-abbreviation measure must survive; got: {}", unit);
    assert!(unit.contains('∨'), "the of-pair XOR must be built; got: {}", unit);
}

#[test]
fn verbal_comparative_with_unit_dimension() {
    // "[verb] N more UNIT than X" — a comparative whose dimension is a measure UNIT
    // after the comparative ("took 10 more MINUTES than", "finished with 3 ounces more
    // gold than"). The dimension scan only recognised plain noun units, so a
    // calendar/clock unit ("minutes") made it bail (TrailingTokens at the comparative);
    // and the "with" measure-prefix wasn't dropped. Both now yield solver-ready arithmetic.
    let mins = ok("The design took 10 more minutes than the item.");
    assert!(mins.contains("Minute") && mins.contains("Add"),
        "the unit-dimension arithmetic comparative must survive; got: {}", mins);
    let gold = ok("Gil finished with 3 ounces more gold than Harry.");
    assert!(gold.contains("Add") && gold.contains("Harry"),
        "the 'finished with N more …' comparative must survive; got: {}", gold);
    // regression: "finished with a medal" stays a plain PP, not a comparative
    let medal = ok("Gil finished with a medal.");
    assert!(medal.contains("Medal") && (medal.contains("With") || medal.contains("with")),
        "a non-comparative 'with' PP must still parse; got: {}", medal);
}

#[test]
fn reduced_relative_with_stranded_preposition() {
    // "the animal Eva works WITH" — an object-gap reduced relative where the head is
    // the object of a STRANDED preposition ("Eva works with [the animal]"), not the
    // verb's direct object. The clause parser bound the gap as the Theme and left the
    // preposition dangling (TrailingTokens). Now the gap binds to the preposition:
    // ∃e(Work(e) ∧ Agent(e, Eva) ∧ With(e, x)).
    let out = ok("The animal Eva works with is either Nikatrice or Ramoran.");
    assert!(out.contains("Work") && out.contains("With"),
        "the stranded-preposition relative must survive; got: {}", out);
    assert!(out.contains("Eva"), "the relative-clause subject must survive; got: {}", out);
    assert!(out.contains('∨') && out.contains("Nikatrice"),
        "the either-or matrix must survive; got: {}", out);
    // regression: a direct-object gap ("the prize Tara won") is unaffected
    let direct = ok("The prize Tara won is red.");
    assert!(direct.contains("Win") && direct.contains("Theme"),
        "direct-object gap relative still binds the Theme; got: {}", direct);
}

#[test]
fn bare_temporal_ordering_after_indefinite_object() {
    // "X has an appointment AFTER Y" / "before Z" — a bare temporal ordering (no
    // count) trailing an INDEFINITE/quantified object. The object quantifier closed
    // and the ordering stranded (only the COUNTED offset "4 days before" was checked).
    // Now the bare ordering relates the subject to the standard, outside the object ∃.
    let after = ok("Patsy has an appointment after Inez.");
    assert!(after.contains("After") && after.contains("Inez"),
        "the bare 'after X' ordering must survive; got: {}", after);
    assert!(after.contains("Appointment") && after.contains("Have"),
        "the possession event must survive; got: {}", after);
    let before = ok("Patsy has an appointment before the meeting.");
    assert!(before.contains("Before") && (before.contains("Meet") || before.contains("meet")),
        "the bare 'before X' ordering with a descriptive standard must survive; got: {}", before);
}

// ── Plural + PP subject (verb-only-noun over-consumption) ────────────────────

#[test]
fn plural_pp_subject_keeps_verbal_predicate() {
    // "the goods from Spain sell quickly" — a definite-PLURAL subject with a PP must
    // not swallow the base-form matrix verb ("sell", a verb-only-noun) into the NP as
    // a deverbal compound. The VP, its adverb, and the subject PP all survive.
    let out = ok("The goods from Spain sell quickly.");
    assert!(out.contains("Sell"), "the matrix verb must survive; got: {}", out);
    assert!(out.contains("Quickly") || out.contains("quickly"), "the adverb must survive; got: {}", out);
    assert!(out.contains("Spain"), "the subject PP must survive; got: {}", out);
}

#[test]
fn deverbal_noun_compound_at_np_tail_still_folds() {
    // Regression guard for the plural-PP fix: a base-form verb-word at the NP TAIL in
    // a PP object is still a deverbal noun-noun compound ("an amber base").
    let out = ok("The perfume with an amber base is expensive.");
    assert!(out.contains("Amber"), "the deverbal compound must survive; got: {}", out);
    assert!(out.contains("Perfume") || out.contains("perfume"), "the head must survive; got: {}", out);
}

// ── Comparative multi-word dimension ─────────────────────────────────────────

#[test]
fn comparative_multiword_dimension_survives() {
    // "somewhat less baking time than" — a MULTI-WORD dimension noun phrase after the
    // comparative must be consumed (not strand at "less"); both sides compare the same
    // measure so the comparison is solver-ready.
    let out = ok("The ham requires somewhat less baking time than the recipe.");
    assert!(
        out.contains("Less") || out.contains("less") || out.contains("<") || out.contains(">"),
        "the comparison must survive; got: {}", out
    );
    assert!(out.contains("time") || out.contains("Time"), "the dimension must survive; got: {}", out);
    assert!(out.contains("Recipe") || out.contains("recipe"), "the standard must survive; got: {}", out);
}

// ── Comparative subject-side restrictor ──────────────────────────────────────

#[test]
fn comparative_subject_restrictor_survives() {
    // The SUBJECT of a copula comparative, like the standard, must keep its
    // restrictor: "the fall Derrick photographed in 1987" becomes a distinct ∃
    // entity carrying its reduced relative, not the bare head constant — and stays
    // distinct from the standard.
    let out = ok("The fall Derrick photographed in 1987 is shorter than the blue fall.");
    assert!(out.contains("Photograph"), "subject's relative verb must survive; got: {}", out);
    assert!(out.contains("Derrick"), "subject's embedded subject must survive; got: {}", out);
    assert!(out.contains("1987"), "subject's PP complement must survive; got: {}", out);
    assert!(out.contains("Blue") || out.contains("blue"), "standard's adjective kept; got: {}", out);
    assert!(
        out.contains("Shorter") || out.contains("shorter") || out.contains("<") || out.contains(">"),
        "the comparison must survive; got: {}", out
    );
}

// ── Reduced object-gap relative clauses ──────────────────────────────────────

#[test]
fn reduced_object_relative_in_subject() {
    // "the prize Tara won" = the prize [that] Tara won — an object-gap reduced
    // relative (the relativizer is dropped). The clause MUST survive: Tara is the
    // agent, the head is the theme/gap. Dropping it loses who won what.
    let out = ok("The prize Tara won is gold.");
    assert!(out.contains("Prize") || out.contains("prize"), "head 'prize' must appear; got: {}", out);
    assert!(out.contains("Win"), "the relative verb 'won' must survive; got: {}", out);
    assert!(out.contains("Tara"), "the embedded subject 'Tara' must survive; got: {}", out);
    assert!(out.contains("Agent"), "Tara must be the agent of the relative event; got: {}", out);
    assert!(out.contains("Theme"), "the head must fill the object gap (Theme); got: {}", out);
}

#[test]
fn reduced_object_relative_as_comparative_standard() {
    // The same reduced relative as a comparative STANDARD must not drop the clause:
    // "shorter than the prize Tara won" keeps the Win event over the standard.
    let out = ok("Rhoqua is shorter than the prize Tara won.");
    assert!(out.contains("Win"), "the standard's relative verb must survive; got: {}", out);
    assert!(out.contains("Tara"), "the standard's embedded subject must survive; got: {}", out);
    assert!(
        out.contains("Shorter") || out.contains("shorter") || out.contains("<") || out.contains(">"),
        "the comparison must survive; got: {}", out
    );
}

#[test]
fn reduced_object_relative_keeps_pp_complement() {
    // "the waterfall Derrick photographed in 1989" — the embedded clause's PP
    // complement ("in 1989") must attach to the photograph EVENT, not strand. Also
    // exercises the transitive-capable default ("photograph" is not lexicon-marked).
    let out = ok("Rhoqua is 5 ft shorter than the waterfall Derrick photographed in 1989.");
    assert!(out.contains("Photograph"), "the relative verb must survive; got: {}", out);
    assert!(out.contains("Derrick"), "the embedded subject must survive; got: {}", out);
    assert!(out.contains("1989"), "the PP complement (year) must survive; got: {}", out);
    assert!(out.contains("In("), "the PP must attach to the event (In); got: {}", out);
}

#[test]
fn apposition_with_overt_object_is_not_a_reduced_relative() {
    // "The winner Tara beat the champion." — Tara beat the champion (apposition),
    // NOT "the winner [that] Tara beat" (which would drop "the champion"). The
    // EMPTY-object slot is what makes a reduced object relative; an OVERT object
    // means apposition, and the object must survive as the theme.
    let out = ok("The winner Tara beat the champion.");
    assert!(out.contains("Champion") || out.contains("champion"), "overt object must survive; got: {}", out);
    assert!(out.contains("Beat"), "the verb must appear; got: {}", out);
    assert!(out.contains("Tara"), "the subject Tara must appear; got: {}", out);
}

#[test]
fn two_word_proper_name_subject_not_a_reduced_relative() {
    // Regression: a bare two-word proper name "Ray Ricardo won …" must stay one
    // name (no determiner) — NOT be split into "Ray [that] Ricardo won".
    let out = ok("Ray Ricardo won the gold medal.");
    assert!(out.contains("Ray_Ricardo") || out.contains("Ray Ricardo") || out.contains("RayRicardo"),
        "two-word proper name kept whole; got: {}", out);
}

// ── Attributive measure-adjectives ───────────────────────────────────────────

#[test]
fn attributive_measure_adjective_survives() {
    // "the 80 year old piece" — an attributive measure phrase (Number + unit)
    // modifying a gradable adjective is a DEGREE property of the head. It must
    // survive as Old(entity, 80 …), mirroring the predicative "is 80 years old" —
    // not be dropped (stranding the unit) nor fused into an opaque blob.
    let out = ok("Of the celluloid doll and the 80 year old piece, one is from Spain and the other is from France.");
    assert!(out.contains("Old"), "the 'old' degree must survive; got: {}", out);
    assert!(out.contains("80"), "the measure value 80 must survive; got: {}", out);
    assert!(out.contains("Piece") || out.contains("piece"), "head 'piece' must survive; got: {}", out);
    assert!(out.contains("Celluloid") || out.contains("celluloid"), "the other member is kept; got: {}", out);
}

// ── Arithmetic comparatives ──────────────────────────────────────────────────

#[test]
fn arithmetic_comparative_lower() {
    // "X scored N points lower than Y" → Score(X) = Score(Y) - N
    let out = ok("Tara scored 3 points lower than Bessie.");
    assert!(out.contains("Tara"), "got: {}", out);
    assert!(out.contains("Bessie") || out.contains("bessie"), "got: {}", out);
    assert!(out.contains("3"), "the offset 3 must appear; got: {}", out);
    // Must express a comparison, not drop the arithmetic
    assert!(
        out.contains("Score") || out.contains("score") || out.contains("<") || out.contains(">") || out.contains("="),
        "comparison must appear; got: {}", out
    );
}

#[test]
fn arithmetic_comparative_higher() {
    let out = ok("The person who danced fifth scored 6 points higher than the person who danced the hustle.");
    assert!(out.contains("6"), "offset 6 must appear; got: {}", out);
    assert!(out.contains("∨") || out.contains("∀") || out.contains("∃"), "quantifier must appear; got: {}", out);
}

#[test]
fn vague_comparative_somewhat_higher() {
    // "somewhat higher" = strictly greater, no specific offset
    let out = ok("Whoever danced the hustle scored somewhat higher than Shirley.");
    assert!(out.contains("Shirley"), "got: {}", out);
    assert!(out.contains("hustle") || out.contains("Hustle"), "got: {}", out);
    assert!(
        out.contains(">") || out.contains("Greater") || out.contains("greater") || out.contains("∀"),
        "strict inequality must appear; got: {}", out
    );
}

// ── Possessed-quality comparative direction (scale polarity) ─────────────────
// "X has a [comparative] [dimension] than Y" → a solver-ready measure comparison
// Dimension(X) ⋛ Dimension(Y). The relation MUST follow the adjective's scale
// polarity: negative-pole adjectives (narrow, thin, short) mean LESS; positive-pole
// (wide, long, tall) mean GREATER. A backwards relation is a meaning-WRONG parse.

#[test]
fn possessed_comparative_narrower_is_less() {
    // "narrower" is negative-pole: Wingspan(falcon) < Wingspan(bird)
    let out = ok("The falcon has a narrower wingspan than the bird.");
    assert!(out.contains("Less(Wingspan"), "narrower must be Less; got: {}", out);
    assert!(!out.contains("Greater(Wingspan"), "narrower must NOT be Greater; got: {}", out);
}

#[test]
fn possessed_comparative_thinner_is_less() {
    let out = ok("The falcon has a thinner beak than the bird.");
    assert!(out.contains("Less(Beak"), "thinner must be Less; got: {}", out);
    assert!(!out.contains("Greater(Beak"), "thinner must NOT be Greater; got: {}", out);
}

#[test]
fn possessed_comparative_wider_is_greater() {
    // "wider" is positive-pole (and exercises silent-e comparative tokenization)
    let out = ok("The falcon has a wider wingspan than the bird.");
    assert!(out.contains("Greater(Wingspan"), "wider must be Greater; got: {}", out);
    assert!(!out.contains("Less(Wingspan"), "wider must NOT be Less; got: {}", out);
}

#[test]
fn possessed_comparative_longer_is_greater() {
    let out = ok("The falcon has a longer wingspan than the bird.");
    assert!(out.contains("Greater(Wingspan"), "longer must be Greater; got: {}", out);
}

#[test]
fn possessed_comparative_somewhat_narrower() {
    // A leading indefinite article + degree modifier must not block the parse:
    // "has a somewhat narrower wingspan than Y" → strict Less(Wingspan(...), ...)
    let out = ok("The amur falcon has a somewhat narrower wingspan than the bird.");
    assert!(out.contains("Less(Wingspan"), "somewhat narrower must be Less; got: {}", out);
    assert!(!out.contains("Greater(Wingspan"), "must NOT be Greater; got: {}", out);
}

#[test]
fn silent_e_comparative_larger() {
    // "larger" = "large" + silent-e drop — must tokenize as a comparative
    let out = ok("The falcon has a larger wingspan than the bird.");
    assert!(out.contains("Greater(Wingspan"), "larger must be Greater; got: {}", out);
}

// ── Unknown domain vocabulary ─────────────────────────────────────────────────

#[test]
fn unknown_noun_after_article() {
    // "hustle" is not in the lexicon — must parse as a Noun constant
    let out = ok("Tara danced the hustle.");
    assert!(out.contains("Tara"), "got: {}", out);
    assert!(out.contains("hustle") || out.contains("Hustle"), "hustle must appear; got: {}", out);
}

#[test]
fn hyphenated_noun_compound() {
    // "boogie-woogie" is a hyphenated domain noun
    let out = ok("Bessie performed the boogie-woogie.");
    assert!(out.contains("Bessie"), "got: {}", out);
    assert!(
        out.contains("boogie") || out.contains("Boogie"),
        "boogie-woogie must appear; got: {}", out
    );
}

#[test]
fn unknown_nouns_in_list() {
    let out = ok("The four dances are the hustle, the lindy, the twist, and the waltz.");
    assert!(out.contains("hustle") || out.contains("Hustle"), "got: {}", out);
    assert!(out.contains("lindy") || out.contains("Lindy"), "got: {}", out);
}

// ── Temporal offsets ──────────────────────────────────────────────────────────

#[test]
fn temporal_offset_weeks_after() {
    // "X performed 2 weeks after Y" → Ord(X_time) = Ord(Y_time) + 2
    let out = ok("Tara performed 2 weeks after Bessie.");
    assert!(out.contains("Tara"), "got: {}", out);
    assert!(out.contains("Bessie") || out.contains("bessie"), "got: {}", out);
    assert!(out.contains("2"), "offset 2 must appear; got: {}", out);
    assert!(
        out.contains("week") || out.contains("Week") || out.contains("After") || out.contains("after"),
        "temporal relation must appear; got: {}", out
    );
}

#[test]
fn temporal_offset_months_before() {
    let out = ok("Becky will launch 2 months before the graduate who will be studying radiation.");
    assert!(out.contains("Becky") || out.contains("becky"), "got: {}", out);
    assert!(out.contains("2"), "offset must appear; got: {}", out);
    assert!(out.contains("month") || out.contains("Month") || out.contains("before"), "got: {}", out);
}

// ── Whoever: universal free relative ─────────────────────────────────────────

#[test]
fn whoever_universal() {
    // "Whoever VP₁ VP₂" → ∀x(VP₁(x) → VP₂(x))
    let out = ok("Whoever danced the hustle scored somewhat higher than Shirley.");
    assert!(
        out.contains("∀") || out.contains("ForAll") || out.contains("All"),
        "must be universally quantified; got: {}", out
    );
    assert!(out.contains("Shirley"), "got: {}", out);
    assert!(out.contains("hustle") || out.contains("Hustle"), "got: {}", out);
}

// ── Regression: previously working clues must still work ─────────────────────

#[test]
fn regression_negation_simple() {
    let out = ok("Tara didn't dance sixth.");
    assert!(out.contains("Tara"), "got: {}", out);
    assert!(out.contains("¬") || out.contains("Not") || out.contains("not"), "negation must appear; got: {}", out);
}

#[test]
fn regression_neither_nor() {
    let out = ok("Neither the 1834 home nor the house on Cora Street is the mansion.");
    assert!(
        out.contains("¬") || out.contains("Not") || out.contains("Neither"),
        "negation must appear; got: {}", out
    );
}

#[test]
fn regression_simple_copula() {
    let out = ok("Woodlawn is the home.");
    assert!(out.contains("Woodlawn"), "got: {}", out);
}

#[test]
fn regression_whoever_simple() {
    let out = ok("Whoever won the prize received a medal.");
    assert!(out.contains("prize") || out.contains("Prize"), "got: {}", out);
}

// ── Solver-ready arithmetic: comparatives & offsets compile to linear
//    arithmetic the LIA oracle consumes (also the basis for math word problems).
//    "sub"/"add" are the function names engine.rs::try_arithmetic recognises;
//    they render capitalised as "Sub"/"Add".

#[test]
fn arithmetic_exact_offset_is_equality_with_subtraction() {
    // "N points lower than Y" → Point(X) = Sub(Point(Y), N): a solvable equality.
    let out = ok("Tara scored 3 points lower than Bessie.");
    assert!(out.contains("= "), "must be an equality; got: {}", out);
    assert!(out.contains("Sub("), "exact offset must subtract; got: {}", out);
    assert!(out.contains("3"), "offset magnitude kept; got: {}", out);
    assert!(out.contains("Tara") && out.contains("Bessie"), "both entities; got: {}", out);
}

#[test]
fn arithmetic_more_noun_than_is_equality_with_addition() {
    // "N more games than Y" → Game(X) = Add(Game(Y), N).
    let out = ok("Tara played 1 more game than Ira.");
    assert!(out.contains("= "), "must be an equality; got: {}", out);
    assert!(out.contains("Add("), "more → addition; got: {}", out);
    assert!(out.contains("1"), "offset magnitude kept; got: {}", out);
    assert!(out.contains("Ira"), "standard entity kept; got: {}", out);
}

#[test]
fn vague_comparative_is_strict_inequality_no_offset() {
    // "somewhat higher than Y" → Greater(Score(X), Score(Y)): strict, no offset.
    let out = ok("Whoever danced the hustle scored somewhat higher than Shirley.");
    assert!(out.contains("Greater("), "strict inequality predicate; got: {}", out);
    assert!(out.contains("Shirley"), "standard entity kept; got: {}", out);
}

#[test]
fn temporal_offset_is_equality_with_arithmetic() {
    // "N weeks after Y" → Weeks(X) = Add(Weeks(Y), N): orderable by the oracle.
    let out = ok("Tara performed 2 weeks after Bessie.");
    assert!(out.contains("= "), "must be an equality; got: {}", out);
    assert!(out.contains("Add("), "after → addition; got: {}", out);
    assert!(out.contains("2"), "offset magnitude kept; got: {}", out);
}

#[test]
fn currency_and_thousands_separator_tokenize_as_number() {
    // "$25,000" — currency marker stripped, thousands separator dropped, the
    // magnitude (25000) survives as a number into the arithmetic constraint.
    let out = ok("Tara scored $25,000 less than Bessie.");
    assert!(out.contains("25000"), "currency magnitude kept; got: {}", out);
    assert!(out.contains("Sub("), "less → subtraction; got: {}", out);
}

#[test]
fn that_after_noun_heads_relative_clause_even_with_verbal_noun() {
    // "the vessel that saw 6 manatees" — "saw" (also a noun) must read as the
    // relative-clause verb, not turn "that" into a demonstrative.
    let out = ok("The vessel that saw 6 manatees went to Yellow Bend.");
    assert!(out.contains("See") || out.contains("see") || out.contains("Saw"), "relative verb kept; got: {}", out);
    assert!(out.contains("Bend"), "main clause kept; got: {}", out);
}

// ── List subjects & the AllDifferent constraint (core of logic-grid solving) ──

#[test]
fn list_subject_distributive_predication() {
    // "A, B and C are reptiles" → each is a reptile (predicate distributes).
    let out = ok("The asp, the cobra and the python are reptiles.");
    assert!(out.contains("Asp") || out.contains("asp"), "asp kept; got: {}", out);
    assert!(out.contains("Cobra") || out.contains("cobra"), "cobra kept; got: {}", out);
    assert!(out.contains("Python") || out.contains("python"), "python kept; got: {}", out);
    assert!(out.contains("Reptile") || out.contains("reptile"), "type predicate kept; got: {}", out);
}

#[test]
fn list_subject_all_different_is_pairwise_distinct() {
    // "A, B and C are all different" → pairwise distinctness, the AllDifferent
    // constraint the puzzle solver needs. Must appear as inequalities, not be
    // collapsed to a single opaque predicate.
    let out = ok("The asp, the cobra and the python are all different animals.");
    assert!(out.contains("asp") || out.contains("Asp"), "asp kept; got: {}", out);
    assert!(out.contains("python") || out.contains("Python"), "python kept; got: {}", out);
    assert!(
        out.contains("≠") || out.contains("¬") || out.contains("Different") || out.contains("≠"),
        "distinctness must appear; got: {}", out
    );
}

#[test]
fn verb_with_for_money_pp() {
    // "X sold for $105" — the money is the object of the "for" PP, kept in FOL.
    let out = ok("The stamp sold for $105.");
    assert!(out.contains("105"), "price kept; got: {}", out);
    assert!(out.contains("Sell") || out.contains("sold") || out.contains("Sold"), "verb kept; got: {}", out);
}

#[test]
fn possessive_subject_with_for_money_pp() {
    // Possessive subject + "sold for $105" together.
    let out = ok("Elodie's perfume sold for $105.");
    assert!(out.contains("Elodie"), "possessor kept; got: {}", out);
    assert!(out.contains("105"), "price kept; got: {}", out);
}

#[test]
fn price_comparative_for_money_less_than() {
    // "X sold for $25,000 less than Y" → price arithmetic (solver-ready), while
    // plain "sold for $105" stays a price PP.
    let out = ok("The stamp sold for $25,000 less than the Yellownose.");
    assert!(out.contains("25000"), "offset kept; got: {}", out);
    assert!(out.contains("Sub("), "less → subtraction; got: {}", out);
    assert!(out.contains("Yellownose"), "standard kept; got: {}", out);
}

#[test]
fn price_for_money_plain_stays_pp() {
    let out = ok("The stamp sold for $105.");
    assert!(out.contains("105"), "price kept; got: {}", out);
    assert!(!out.contains("Sub("), "no spurious arithmetic; got: {}", out);
}

#[test]
fn quoted_proper_name_is_an_entity() {
    // A quoted name in prose ("the \"Yellownose\"") parses as a named entity.
    let out = ok("The stamp was the \"Yellownose\".");
    assert!(out.contains("Yellownose"), "quoted name kept; got: {}", out);
}

#[test]
fn either_or_with_quoted_names() {
    let out = ok("The stamp was either the \"Yellownose\" or the \"Bull's Dove\".");
    assert!(out.contains("Yellownose"), "got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "disjunction; got: {}", out);
}

#[test]
fn rate_per_unit_parses() {
    // "$2.50 per pound" — the rate denominator is kept, not stranded.
    let out = ok("The vegetables sell for $2.50 per pound.");
    assert!(out.contains("2.5"), "amount kept; got: {}", out);
    assert!(out.contains("Per") || out.contains("Pound") || out.contains("pound"), "rate kept; got: {}", out);
}

#[test]
fn ly_proper_name_possessor() {
    // "Billy" ends in -ly but is a proper name, not an adverb.
    let out = ok("Billy's goods are fresh.");
    assert!(out.contains("Billy"), "possessor name kept; got: {}", out);
}

#[test]
fn relative_clause_subject_either_or_copula() {
    // "The X that VP was either A or B" — relative-clause subject + disjunctive
    // copula complement (was previously stranded at "either").
    let out = ok("The fragrance that sold for $105 was either Camille's perfume or the purple perfume.");
    assert!(out.contains("Camille"), "first disjunct kept; got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "disjunction; got: {}", out);
}

#[test]
fn relative_clause_subject_np_predication_copula() {
    let out = ok("The stamp that sold for $105 was the \"Yellownose\".");
    assert!(out.contains("Yellownose"), "predicate name kept; got: {}", out);
}

#[test]
fn label_noun_cardinal_compound() {
    // "wore number 7" — the cardinal labels the noun, not stranded.
    let out = ok("Frank wore number 29.");
    assert!(out.contains("29"), "label value kept; got: {}", out);
    assert!(out.contains("Frank"), "got: {}", out);
}

#[test]
fn passive_relative_clause_with_pp() {
    // "that was issued in 1868" — passive relative + trailing PP; the year PP
    // must survive (zero meaning loss), not be stranded.
    let out = ok("The stamp that was issued in 1868 is rare.");
    assert!(out.contains("1868"), "year kept; got: {}", out);
    assert!(out.contains("In(") || out.contains("in("), "trailing PP kept; got: {}", out);
}

#[test]
fn of_pair_xor_preserves_possessor_in_np1() {
    // Of-pair must NOT drop the possessor of a multi-word possessive NP₁
    // ("Ray Ricardo's stamp") — zero meaning loss.
    let out = ok("Of Ray Ricardo's stamp and the home, one is red and the other is blue.");
    assert!(out.contains("Ricardo"), "possessor of NP1 must survive; got: {}", out);
    assert!(out.contains("Possess") || out.contains("Have"), "possession relation kept; got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "disjunction; got: {}", out);
}

#[test]
fn of_pair_xor_preserves_relative_clause_in_np1() {
    // "the person who danced sixth" must keep the relative clause, not reduce
    // to the ordinal "sixth".
    let out = ok("Of the person who danced sixth and Patti, one scored 190 points and the other performed the lindy.");
    assert!(out.contains("Person") || out.contains("person"), "head noun kept; got: {}", out);
    assert!(out.contains("Dance") || out.contains("dance"), "relative clause kept; got: {}", out);
}

#[test]
fn of_pair_xor_descriptive_nps_do_not_collapse() {
    // Two NPs sharing a head ("the red stamp" / "the blue stamp") must NOT both
    // reduce to the same bare constant "Stamp" — that yields two IDENTICAL XOR
    // branches (vacuous + self-contradictory). Each must be a distinct ∃ variable
    // carrying its adjective restrictor, with an explicit inequality.
    let out = ok("Of the red stamp and the blue stamp, one was the Jenny Penny and the other was the Yellownose.");
    assert!(out.contains("Red") || out.contains("red"), "first adjective kept; got: {}", out);
    assert!(out.contains("Blue") || out.contains("blue"), "second adjective kept; got: {}", out);
    assert!(out.contains('∃'), "descriptive entities are existential variables; got: {}", out);
    assert!(out.contains('¬') || out.contains("≠"), "the two entities must be asserted distinct; got: {}", out);
    assert!(out.contains("Jenny_Penny") || out.contains("Jenny"), "first predicate kept; got: {}", out);
    assert!(out.contains("Yellownose"), "second predicate kept; got: {}", out);
}

#[test]
fn of_pair_xor_shared_head_entities_stay_distinct() {
    // "the 1848 home" and "the Evans home" share the head "home" but are distinct
    // individuals — the labels must survive and an inequality must separate them.
    let out = ok("Of the 1848 home and the Evans home, one is Woodlawn and the other is on Rosewood Street.");
    assert!(out.contains("1848"), "first label kept; got: {}", out);
    assert!(out.contains("Evans"), "second label kept; got: {}", out);
    assert!(out.contains('¬') || out.contains("≠"), "entities asserted distinct; got: {}", out);
}

#[test]
fn of_pair_xor_numeric_label_head_kept() {
    // "the $125,000 stamp" — a numeric label heads the NP and "stamp" is a
    // verb-only noun. The full parse must keep "125000_stamp", not fall back to a
    // bare "Stamp" constant that silently drops the price label that identifies
    // which stamp it is.
    let out = ok("Of Ray Ricardo's stamp and the $125,000 stamp, one was the Jenny Penny and the other was the Yellownose.");
    assert!(out.contains("125000"), "the $125,000 label must survive; got: {}", out);
    assert!(out.contains("Ricardo"), "possessor of the other NP kept; got: {}", out);
    assert!(out.contains('¬') || out.contains("≠"), "entities asserted distinct; got: {}", out);
}

#[test]
fn either_or_disjuncts_preserve_possessors() {
    // "either Elodie's perfume or Camille's perfume" must keep BOTH possessors —
    // otherwise the disjunction collapses to Perfume ∨ Perfume (vacuous).
    let out = ok("The scent was either Elodie's perfume or Camille's perfume.");
    assert!(out.contains("Elodie"), "first possessor kept; got: {}", out);
    assert!(out.contains("Camille"), "second possessor kept; got: {}", out);
    assert!(out.contains("Possess"), "possession relation kept; got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "disjunction; got: {}", out);
}

#[test]
fn either_or_verb_object_preserves_pp() {
    // "danced either the hustle from Spain or the lindy" must keep "from Spain".
    let out = ok("Tara danced either the hustle from Spain or the lindy.");
    assert!(out.contains("Spain"), "PP on disjunct object must survive; got: {}", out);
    assert!(out.contains("From") || out.contains("from"), "PP relation kept; got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "disjunction; got: {}", out);
}

#[test]
fn comparative_standard_preserves_possessor() {
    // "lower than Quinn Quade's stamp" must keep the possessor in the standard.
    let out = ok("Tara scored 3 points lower than Quinn Quade's stamp.");
    assert!(out.contains("Quade") || out.contains("Quinn"), "standard possessor kept; got: {}", out);
    assert!(out.contains("Possess"), "possession relation kept; got: {}", out);
}

#[test]
fn comparative_standard_distinct_from_subject() {
    // When the subject and the standard share a head noun ("stamp" … than … "stamp"),
    // the standard must be a DISTINCT entity — otherwise the measure compares the
    // subject to ITSELF (Less(Sell(Stamp), Sell(Stamp)) — vacuous) and the
    // possessor misattaches to the subject. The standard gets its own existential
    // variable carrying its possessor.
    let out = ok("The stamp sold for somewhat less than Quinn Quade's stamp.");
    assert!(out.contains("Quade") || out.contains("Quinn"), "standard possessor kept; got: {}", out);
    assert!(out.contains("Possess"), "possession relation kept; got: {}", out);
    // The collapse signature: the comparison's two measure arguments are the SAME
    // term — the subject compared to itself. The fix gives the standard its own
    // variable so the two arguments differ.
    assert!(
        !out.contains("Sell(Stamp), Sell(Stamp)"),
        "standard collapsed into the subject (self-comparison); got: {}", out
    );
    assert!(out.contains("Less") || out.contains("less") || out.contains("Sub"), "comparison kept; got: {}", out);
}

#[test]
fn comparative_standard_with_pp_parses() {
    // "less than the perfume from Spain" — PP standard parses and keeps the PP.
    let out = ok("The stamp sold for somewhat less than the perfume from Spain.");
    assert!(out.contains("Spain"), "PP on standard kept; got: {}", out);
    assert!(out.contains("Less") || out.contains("less"), "comparison kept; got: {}", out);
}

#[test]
fn temporal_offset_standard_preserves_possessor_and_pp() {
    // "after Quinn Quade's debut" keeps the possessor; "after the show from
    // Spain" keeps the PP (and parses).
    let out = ok("Tara performed 2 weeks after Quinn Quade's debut.");
    assert!(out.contains("Quade") || out.contains("Quinn"), "standard possessor kept; got: {}", out);
    let out2 = ok("Tara performed 2 weeks after the show from Spain.");
    assert!(out2.contains("Spain"), "standard PP kept; got: {}", out2);
}

#[test]
fn clock_time_unit_offset_and_ordinal_disambiguation() {
    // Clock units (hour/minute/second) work in temporal offsets ("1 hour before"),
    // but the SINGULAR "second"/"minute" stay an ordinal/adjective when NOT after a
    // count ("played second base", "the detail is minute") — context-sensitive.
    let off = ok("The class begins 1 hour before the yoga session.");
    assert!(off.contains("Begin"), "verb event kept; got: {}", off);
    assert!(off.contains("Hour") && off.contains("Sub"), "solver-ready hour offset; got: {}", off);
    let ord = ok("Frank was either the player who wore number 29 or the person who played second base.");
    assert!(ord.contains("Second") || ord.contains("second"), "second base ordinal kept; got: {}", ord);
    assert!(ord.contains("∨") || ord.contains("or"), "disjunction; got: {}", ord);
}

#[test]
fn multiword_possessed_compound_noun() {
    // "Bernard's fountain pen" — the possessed noun is a multi-word compound and
    // must join into one symbol, keeping the possessor; it must not strand "pen".
    let out = ok("Bernard's fountain pen is red.");
    assert!(out.contains("Fountain_pen") || out.contains("fountain_pen"), "compound possessed noun; got: {}", out);
    assert!(out.contains("Bernard"), "possessor kept; got: {}", out);
    // And as a comparative standard.
    let std = ok("The pen sold for 50 dollars more than Bernard's fountain pen.");
    assert!(std.contains("Fountain_pen") || std.contains("fountain_pen"), "compound standard; got: {}", std);
}

#[test]
fn copula_coordinate_pp_object() {
    // "The cache is at 40.5912 N." — a numeric PP object (geocache coordinate)
    // in a copula PP must keep the amount and its direction, not strand "N".
    let out = ok("The cache is at 40.5912 N.");
    assert!(out.contains("40.5912"), "coordinate amount kept; got: {}", out);
    assert!(out.contains("At") || out.contains("at"), "PP relation kept; got: {}", out);
}

#[test]
fn definite_passive_keeps_subject_restrictions() {
    // A definite passive subject WITH restrictions must keep them: "The case at
    // 40.5912 N is hidden." keeps the coordinate; a BARE definite subject stays
    // simple ("The butler was caught." → Past(catch(butler))).
    let out = ok("The case at 40.5912 N is hidden.");
    assert!(out.contains("40.5912"), "passive subject PP kept; got: {}", out);
    let bare = ok("The butler was caught.");
    assert!(bare.contains("Catch") || bare.contains("catch"), "bare passive intact; got: {}", bare);
    assert!(!bare.contains('∃'), "bare definite passive stays simple (no ∃); got: {}", bare);
}

#[test]
fn passive_with_trailing_pp_adjunct() {
    // A passive matrix with a locative/temporal PP adjunct must attach the PP
    // ("was taken on May 9", "was found in Spain"), not strand it after the verb.
    let a = ok("The shot was taken on May 9.");
    assert!(a.contains("On") && a.contains("May_9"), "passive PP adjunct kept; got: {}", a);
    let b = ok("The box was found in Spain.");
    assert!(b.contains("In") && b.contains("Spain"), "passive PP adjunct kept; got: {}", b);
    // The by-agent passive must be unaffected.
    let c = ok("The cake was eaten by John.");
    assert!(c.contains("John") && c.contains("Eat"), "by-agent passive intact; got: {}", c);
}

#[test]
fn month_day_date_np() {
    // "Month Day" date NPs ("June 11", "May 3", "December 25") form a single date
    // symbol; "May" the month must not be left as the modal token. Month names
    // come from the lexicon (`is_month`), not a hardcoded parser list.
    let a = ok("The meeting was on June 11.");
    assert!(a.contains("June_11"), "June 11 → June_11; got: {}", a);
    let b = ok("The meeting was on May 3.");
    assert!(b.contains("May_3"), "May 3 → May_3 (May as month); got: {}", b);
    let c = ok("The party is on December 25.");
    assert!(c.contains("December_25"), "December 25 → December_25; got: {}", c);
}

#[test]
fn past_participle_reduced_relative_before_main_verb() {
    // "The box found in Spain held the ring." — a past-participle reduced relative
    // ("found in Spain") restricts the subject before a finite MAIN VERB ("held").
    // The two-verb signature disambiguates it from a lone main verb; a perfect
    // auxiliary ("has been eaten") must NOT be mistaken for the participle.
    let out = ok("The box found in Spain held the ring.");
    assert!(out.contains("Find"), "reduced-relative participle kept; got: {}", out);
    assert!(out.contains("Spain"), "reduced-relative PP kept; got: {}", out);
    assert!(out.contains("Have") || out.contains("Hold") || out.contains("held"), "main verb kept; got: {}", out);
    // The perfect passive must be untouched by the reduced-relative path.
    let pp = ok("The apple has been eaten.");
    assert!(pp.contains("Perf") && pp.contains("Pass"), "perfect passive intact; got: {}", pp);
    // A verb whose object starts with an -ing GERUND ("used bowling pins") must
    // NOT be read as a reduced relative — the -ing is a gerund modifier of the
    // object, not a finite matrix verb. The verb must keep its event + object.
    let ger = ok("The performer used bowling pins.");
    assert!(ger.contains("Use") && (ger.contains("Theme") || ger.contains("Bowl") || ger.contains("Pin")),
        "verb + gerund-compound object kept (not a reduced relative); got: {}", ger);
}

#[test]
fn calendar_unit_measure_adjective() {
    // "Edmund is 12 years old." — a calendar unit ("years") is a measure unit in a
    // dimensional-adjective predication, like "5 inches long". The temporal OFFSET
    // ("2 weeks after X") must remain unaffected.
    let out = ok("Edmund is 12 years old.");
    assert!(out.contains("12") && (out.contains("year") || out.contains("Year")), "age measure kept; got: {}", out);
    assert!(out.contains("Old") || out.contains("old"), "dimensional adjective kept; got: {}", out);
    let off = ok("Tara performed 2 weeks after Bessie.");
    assert!(off.contains("Perform"), "temporal offset still parses; got: {}", off);
}

#[test]
fn passive_by_agent_reduced_relative() {
    // A past participle + "by"-agent is an unambiguous passive reduced relative:
    // "The photo published by Wildzone …" → Photo(x) ∧ Publish(x) ∧ By(x, Wildzone).
    let out = ok("The photo published by Wildzone is old.");
    assert!(out.contains("Publish"), "passive participle kept; got: {}", out);
    assert!(out.contains("Wildzone"), "by-agent kept; got: {}", out);
    // A finite transitive main verb with an object (no "by") must NOT be eaten.
    let out2 = ok("The committee approved the plan.");
    assert!(out2.contains("Approve") && out2.contains("Plan"), "main verb + object intact; got: {}", out2);
}

#[test]
fn ing_reduced_relative_in_nonsubject_positions() {
    // An "-ing" reduced relative must restrict its NP wherever the NP appears,
    // not only as a subject: here on an of-pair member ("the assignment beginning
    // in June") and a temporal standard ("the person arriving at Paradise").
    let out = ok("Of the shark project and the assignment beginning in June, one is red and the other is blue.");
    assert!(out.contains("Begin"), "of-pair member reduced relative kept; got: {}", out);
    assert!(out.contains("June"), "reduced-relative PP kept; got: {}", out);
    let out2 = ok("Oscar will leave 1 day after the person arriving at Paradise.");
    assert!(out2.contains("Arriv"), "standard reduced relative kept; got: {}", out2);
    assert!(out2.contains("Paradise"), "standard reduced-relative PP kept; got: {}", out2);
}

#[test]
fn bare_temporal_before_and_after_with_distinct_standard() {
    // "before X" (the bare-before gap) and "after the X" (definite standard) must
    // parse to a directed temporal relation over a DISTINCT standard entity;
    // a vague adverb ("sometime") is skipped, the standard's PP is kept.
    let out = ok("Tara performed before Bessie.");
    assert!(out.contains("Before"), "Before relation kept; got: {}", out);
    assert!(out.contains("Bessie"), "standard kept; got: {}", out);
    let out2 = ok("The assignment starts sometime after the project on the Orion.");
    assert!(out2.contains("After"), "After relation kept; got: {}", out2);
    assert!(out2.contains("Orion"), "standard PP kept; got: {}", out2);
}

#[test]
fn reduced_relative_past_participle_restricts_subject() {
    // "The drug sourced from a beetle is red." — the past-participle reduced
    // relative "sourced from a beetle" restricts the subject; it must NOT strand
    // before the matrix copula, and must keep both the participle and its PP.
    let out = ok("The drug sourced from a beetle is red.");
    assert!(out.contains("Sourc"), "reduced-relative participle kept; got: {}", out);
    assert!(out.contains("Beetle") || out.contains("beetle"), "participle PP object kept; got: {}", out);
    assert!(out.contains("Red") || out.contains("red"), "matrix predicate kept; got: {}", out);
}

#[test]
fn reduced_relative_with_either_or_keeps_both() {
    // Reduced relative + either/or copula complement: both the restriction
    // ("approved in March") and the disjunction must survive.
    let out = ok("The pharmaceutical approved in March is either the Boston drug or the Athens drug.");
    assert!(out.contains("Approve"), "reduced relative kept; got: {}", out);
    assert!(out.contains("March"), "reduced-relative PP kept; got: {}", out);
    assert!(out.contains('∨') || out.contains("or"), "disjunction kept; got: {}", out);
}

#[test]
fn reduced_relative_does_not_eat_finite_main_verb() {
    // "The drug treats meningitis." — "treats" is the finite main verb, NOT a
    // reduced relative (no copula follows); the event predication must stand.
    let out = ok("The drug treats meningitis.");
    assert!(out.contains("Treat"), "main verb kept as predicate; got: {}", out);
    assert!(out.contains("Meningitis") || out.contains("meningitis"), "object kept; got: {}", out);
}

#[test]
fn gerund_compound_object_in_relative_clause() {
    // "The performer who used bowling pins finished." — the relative clause's
    // object "bowling pins" is a gerund-noun compound (-ing "bowling" + noun);
    // it must be the object, not stop the relative clause and strand the matrix.
    let out = ok("The performer who used bowling pins finished.");
    assert!(out.contains("Bowl_pins") || out.contains("Bowl") && out.contains("Pin"),
        "gerund compound object kept; got: {}", out);
    assert!(out.contains("Finish") || out.contains("finish"), "matrix verb kept; got: {}", out);
}

#[test]
fn ordinal_position_offset() {
    // "N places/spots ahead of/behind X" → a solver-ready ordinal offset over a
    // DISTINCT standard, with the right direction (ahead/before → sub, behind/after
    // → add). Previously this dropped "of Bob" (meaning loss) or failed.
    let ahead = ok("Tara finished 2 places ahead of Bob.");
    assert!(ahead.contains("Place") && ahead.contains("Sub") && ahead.contains("Bob"),
        "ahead → Place=Sub(Place(std),N); got: {}", ahead);
    let behind = ok("Tara finished 1 place behind Bob.");
    assert!(behind.contains("Place") && behind.contains("Add"),
        "behind → Place=Add(...); got: {}", behind);
    // "1 spot before X" previously failed outright.
    let spot = ok("Violet performed 1 spot before Frank.");
    assert!(spot.contains("Place") && spot.contains("Frank"), "spot-before parses; got: {}", spot);
}

#[test]
fn list_subject_members_keep_restrictions() {
    // A list-subject member with a PP (coordinate) or possessor must keep it —
    // dropping it loses which one the member is (meaning loss).
    let coord = ok("The cache at 40.6656 N, the box, and the case are three different containers.");
    assert!(coord.contains("40.6656"), "member coordinate kept; got: {}", coord);
    assert!(coord.contains('¬') || coord.contains("≠"), "AllDifferent kept; got: {}", coord);
    let poss = ok("Mildred's vegetables, the onions, and Billy's produce are three different vegetables.");
    assert!(poss.contains("Mildred") && poss.contains("Billy"), "member possessors kept; got: {}", poss);
    assert!(poss.contains("Possess"), "possession relation kept; got: {}", poss);
}

#[test]
fn list_subject_n_different_alldifferent() {
    // "A, B, and C are three different X." — the "three" restates the member count
    // before "different"; the list must still yield the AllDifferent constraint
    // (pairwise distinctness) plus the type predicate on each member.
    let out = ok("The produce, the onions, and the carrots are three different vegetables.");
    assert!(out.contains("Produce") && out.contains("Onions") && out.contains("Carrots"),
        "all three members kept; got: {}", out);
    assert!(out.contains("Vegetable") || out.contains("vegetable"), "type predicate kept; got: {}", out);
    assert!(out.contains('¬') || out.contains("≠"), "pairwise distinctness (AllDifferent); got: {}", out);
}

#[test]
fn bare_verbal_comparative_with_entity_standard() {
    // "The apples cost less than the potatoes." — a bare comparative (no measure,
    // no degree) with an ENTITY standard must parse, comparing the two via the
    // verb's measure with a DISTINCT standard entity (not the subject itself).
    let out = ok("The apples cost less than the potatoes.");
    assert!(out.contains("Less") || out.contains("less"), "comparison kept; got: {}", out);
    assert!(out.contains("Potato") || out.contains("potato"), "standard entity kept; got: {}", out);
    assert!(!out.contains("Cost(Apple), Cost(Apple)") && !out.contains("Cost(Apples), Cost(Apples)"),
        "standard must be distinct from subject; got: {}", out);
}

#[test]
fn copula_possessive_predicate_elided_noun() {
    // "The scent is Camille's." — a possessive predicate with an elided noun
    // means the subject BELONGS to Camille: Possesses(Camille, scent). It must
    // not strand the "'s" (and must not read it as an identity "scent = Camille").
    let out = ok("The scent is Camille's.");
    assert!(out.contains("Camille"), "possessor kept; got: {}", out);
    assert!(out.contains("Possess"), "possession relation, not identity; got: {}", out);
}

#[test]
fn np_with_numeric_pp_object() {
    // "The class with 15 people …" — a numeric PP object on a noun phrase must
    // parse (and keep the count), not strand "15 people" as trailing tokens.
    let out = ok("The class with 15 people is large.");
    assert!(out.contains("15"), "the count must survive; got: {}", out);
    assert!(out.contains("With") || out.contains("with"), "the PP relation kept; got: {}", out);
    assert!(out.contains("Large") || out.contains("large"), "matrix predicate kept; got: {}", out);
}

#[test]
fn either_or_copula_keeps_subject_pp() {
    // "The class with 15 people is either A or B." — the either/or copula path
    // must keep the SUBJECT's PP ("with 15 people"); dropping it loses which
    // class the disjunction is about.
    let out = ok("The class with 15 people is either the yoga session or the dance session.");
    assert!(out.contains("15"), "subject PP count kept; got: {}", out);
    assert!(out.contains("With") || out.contains("with"), "subject PP relation kept; got: {}", out);
    assert!(out.contains("∨") || out.contains("or"), "disjunction kept; got: {}", out);
}

#[test]
fn relative_clause_pp_number_does_not_eat_matrix_verb() {
    // "The stamp that was issued in 1850 sold for $105." — the relative clause's
    // numeric PP object "in 1850" must STOP at the year and NOT swallow the matrix
    // past-tense verb "sold" as a measure unit ("1850 Sell"), which would erase the
    // sentence's main predication.
    let out = ok("The stamp that was issued in 1850 sold for $105.");
    assert!(!out.contains("1850 Sell") && !out.contains("1850_Sell"),
        "the matrix verb 'sold' must not be eaten into the PP object; got: {}", out);
    assert!(out.contains("1850"), "the year is kept; got: {}", out);
    assert!(out.contains("Sell") || out.contains("sell"), "the matrix verb survives; got: {}", out);
}

#[test]
fn adjective_before_verb_only_noun() {
    // "the rare stamp" — "stamp" is verb-lexicon-only; after an adjective it must
    // still be the head noun, not misread as a verb.
    let out = ok("The rare stamp is red.");
    assert!(out.contains("Stamp"), "verb-only noun head kept; got: {}", out);
    assert!(out.contains("Rare") || out.contains("rare"), "adjective kept; got: {}", out);
}

#[test]
fn adverb_before_verb_only_noun_not_consumed() {
    // Regression guard: "studies hard pass" — "hard" is an adverb modifying
    // "studies", and "pass" is the main verb, NOT the head of a "hard pass" NP.
    // The verb-only-noun-after-adjective promotion is article-gated so this stays
    // a clause with "pass" as its verb.
    let out = ok("Can every student who studies hard pass the exam?");
    assert!(out.contains("Pass") || out.contains("pass"), "main verb kept; got: {}", out);
    assert!(out.contains("Student") || out.contains("student"), "subject kept; got: {}", out);
}

#[test]
fn numeric_label_verb_only_noun_compound() {
    // "the 125000 stamp" / "$125,000 stamp" — numeric label + verb-only noun.
    let out = ok("The $125,000 stamp was the Yellownose.");
    assert!(out.contains("125000") && out.contains("Stamp"), "label+noun compound; got: {}", out);
    assert!(out.contains("Yellownose"), "predicate kept; got: {}", out);
}

#[test]
fn copula_identity_subject_possessor_kept() {
    // "Ray Ricardo's stamp is the Yellownose." — the SUBJECT's possessor must
    // survive the "X is the Y" predicate-nominal path. Dropping it loses whose
    // stamp X is (→ a vacuous Yellownose(Stamp)) — a meaning-loss parse.
    let out = ok("Ray Ricardo's stamp is the Yellownose.");
    assert!(out.contains("Ricardo") || out.contains("Ray"), "subject possessor kept; got: {}", out);
    assert!(out.contains("Yellownose"), "predicate kept; got: {}", out);
}

#[test]
fn copula_identity_subject_adjective_kept() {
    // "The red stamp is the Yellownose." — the SUBJECT's adjective must survive
    // the definite restriction; dropping "red" silently weakens the description.
    let out = ok("The red stamp is the Yellownose.");
    assert!(out.contains("Red") || out.contains("red"), "subject adjective kept; got: {}", out);
    assert!(out.contains("Yellownose"), "predicate kept; got: {}", out);
}

#[test]
fn copula_possessive_np_complement() {
    // "The box is Kerry's project." — the predicate nominal is a POSSESSIVE NP
    // with an explicit head noun. Both the kind (Project) and the ownership
    // (Possesses(Kerry, box)) must survive. The "Clark is Superman" identity path
    // must NOT swallow "Kerry" and strand "'s project" — that is a meaning-loss
    // parse (it would drop the whole predicate).
    let out = ok("The box is Kerry's project.");
    assert!(out.contains("Project"), "predicate kind kept; got: {}", out);
    assert!(out.contains("Possesses") && out.contains("Kerry"), "ownership kept; got: {}", out);
}

#[test]
fn copula_possessive_np_complement_in_of_pair() {
    // The same possessive-NP complement inside an of-pair XOR: both branches must
    // carry the full Project ∧ Possesses(Kerry, ·) predicate.
    let out = ok("Of the box and Frank, one is Kerry's project and the other is on the Odyssey.");
    assert!(out.contains("Project") && out.contains("Possesses"), "possessive kept in both branches; got: {}", out);
    assert!(out.contains("∨"), "XOR disjunction present; got: {}", out);
}

#[test]
fn object_compound_head_ambiguous_noun_verb() {
    // "The doll has a glass head." — the head noun "head" lexes Ambiguous
    // (verb/noun). Inside an OBJECT NP at a clause boundary the verb reading is
    // structurally impossible (the clause already has "has"), so "glass head"
    // compounds. Dropping/stranding "head" is a parse failure (meaning loss of
    // the whole object).
    let out = ok("The doll has a glass head.");
    assert!(out.contains("Glass_head"), "compound object head kept; got: {}", out);
    assert!(out.contains("Have") && out.contains("Doll"), "predication kept; got: {}", out);
}

#[test]
fn object_compound_head_does_not_eat_subject_verb() {
    // Regression guard: the object-compound rule is gated to NON-subject
    // (greedy=false) NPs, so an intransitive subject + ambiguous main verb is
    // NOT swallowed into a compound noun. "The man runs." keeps "runs" as the
    // verb, not "The_man_runs".
    let out = ok("The man runs.");
    assert!(out.contains("Run"), "subject's main verb kept; got: {}", out);
}

#[test]
fn relative_clause_subject_copula_keeps_restriction() {
    // "The scent that sold for $75 is the white fragrance." — the SUBJECT's
    // relative clause ("that sold for $75") must survive when the predicate is a
    // copula complement. Previously it was silently dropped (a meaning-loss
    // parse that still returned Ok). Both the relative-clause event and the
    // predicate-nominal must appear, bound to the same entity.
    let out = ok("The scent that sold for $75 is the white fragrance.");
    assert!(out.contains("Sell"), "relative clause kept; got: {}", out);
    assert!(out.contains("Fragrance") && out.contains("White"), "predicate nominal kept; got: {}", out);
}

#[test]
fn relative_clause_subject_copula_no_leak() {
    // Regression guard: the stashed restriction is keyed to the subject noun and
    // cleared per clause, so a following clause without a relative does not
    // inherit it. "The man who runs is happy." then a plain identity must stay
    // clean.
    let out = ok("The man who runs is happy.");
    assert!(out.contains("Run") && out.contains("Happy"), "rel + predicate kept; got: {}", out);
    let out2 = ok("Clark is Superman.");
    assert!(out2.contains("Clark") && out2.contains("Superman") && !out2.contains("Run"),
        "no leak into next clause; got: {}", out2);
}

#[test]
fn comparative_fewer_bare_count() {
    // "Silk Mist received fewer nominations than Blue Moon." — "fewer" is the
    // count counterpart of "less" (decreasing). It must grade like "less":
    // Less(Nominations(X), Nominations(Y)). Previously "fewer" was not lexed as a
    // Comparative, so an object-NP path swallowed "fewer nominations" and "than"
    // stranded (a hard parse failure).
    let out = ok("Silk Mist received fewer nominations than Blue Moon.");
    assert!(out.contains("Less("), "decreasing comparison kept; got: {}", out);
    assert!(out.contains("Nominations"), "measure noun kept; got: {}", out);
    assert!(out.contains("Silk_Mist") && out.contains("Blue_Moon"), "both standards kept; got: {}", out);
}

#[test]
fn comparative_fewer_with_offset() {
    // "Bob received 3 fewer votes than Tom." — the numeric offset makes this exact
    // arithmetic: Vote(Bob) = Sub(Vote(Tom), 3). Solver-ready lowercase op.
    let out = ok("Bob received 3 fewer votes than Tom.");
    assert!(out.contains("Sub(") && out.contains("3"), "arithmetic offset kept; got: {}", out);
    assert!(out.contains("Vote"), "measure kept; got: {}", out);
}

#[test]
fn bare_few_quantifier_unaffected() {
    // Regression guard: making "fewer" a comparative must NOT turn bare "few"
    // into an adjective — it stays a quantifier. "Few students passed." → FEW.
    let out = ok("Few students passed.");
    assert!(out.contains("FEW") || out.contains("Few"), "few quantifier kept; got: {}", out);
    assert!(out.contains("Pass"), "verb kept; got: {}", out);
}

#[test]
fn comparative_descriptive_standard_with_measure_pp() {
    // "The tea that steeps for 2.5 minutes costs 1 dollar more than the tea that
    // steeps for 4 minutes." — BOTH the subject and the comparative standard are
    // definite descriptions with a relative clause carrying a measure PP. All
    // four constraints (both steep-times + the arithmetic offset) must survive.
    let out = ok("The tea that steeps for 2.5 minutes costs 1 dollar more than the tea that steeps for 4 minutes.");
    assert!(out.contains("2.5 minutes"), "subject relative measure kept; got: {}", out);
    assert!(out.contains("4 minutes"), "standard relative measure kept; got: {}", out);
    assert!(out.contains("Add(") && out.contains("Dollar"), "arithmetic offset kept; got: {}", out);
    assert!(out.matches("Steep").count() >= 2, "both steep events kept; got: {}", out);
}

#[test]
fn of_measure_pp_on_head_noun() {
    // "of <number> <unit>" specifies a head noun's measured value — a restrictor,
    // not a partitive. "The book of 500 pages is heavy." → Of(x, 500 pages).
    let out = ok("The book of 500 pages is heavy.");
    assert!(out.contains("Of(") && out.contains("500 pages"), "of-measure restrictor kept; got: {}", out);
}

#[test]
fn nested_of_measure_in_with_pp_kept() {
    // "with a maximum range of 650 ft" — the of-measure is nested inside the
    // with-PP object. The measure must NOT be dropped when the object collapses
    // to a constant: With(x, Range) ∧ Of(Range, 650 ft). Dropping "650 ft" would
    // be a fake-Ok meaning-loss parse.
    let out = ok("The machine with a maximum range of 650 ft is fast.");
    assert!(out.contains("With") && out.contains("Range"), "with-PP kept; got: {}", out);
    assert!(out.contains("Of(") && out.contains("650 ft"), "nested measure kept; got: {}", out);
}

#[test]
fn partitive_of_the_still_works() {
    // Regression guard: the of-measure rule only fires on bare "of <number>";
    // a real partitive "of the <number> N" is unaffected.
    let out = ok("Two of the boys ran.");
    assert!(out.contains("=2") || out.contains("∃=2") || out.contains("2."), "partitive count kept; got: {}", out);
}

#[test]
fn modal_headed_relative_clause_with_measure_pp() {
    // "the device that can fly for 40 minutes" — a modal-headed relative clause
    // ("can fly") with a trailing measure PP ("for 40 minutes"). Both the modal
    // capability and the duration must survive. parse_relative_clause now handles
    // modals (◇ over the gap), and the modal aspect chain attaches measure PPs.
    let out = ok("The device that can fly for 40 minutes is red.");
    assert!(out.contains("Fly"), "modal verb event kept; got: {}", out);
    assert!(out.contains("40 minutes"), "measure PP kept; got: {}", out);
    assert!(out.contains("◇") || out.contains("Modal") || out.contains("Poss"), "modality kept; got: {}", out);
}

#[test]
fn comparative_with_modal_relative_standard() {
    // The modal relative as a comparative standard: "than the device that can fly
    // for 40 minutes" — the full arithmetic comparison plus the standard's modal
    // capability and duration all survive.
    let out = ok("The device costs 150 dollars more than the device that can fly for 40 minutes.");
    assert!(out.contains("Add(") && out.contains("150"), "arithmetic kept; got: {}", out);
    assert!(out.contains("Fly") && out.contains("40 minutes"), "standard's modal+measure kept; got: {}", out);
}

#[test]
fn list_subject_with_relative_clause_first_member() {
    // "The scent that sold for $75, the white fragrance, and the purple perfume
    // are three different perfumes." — the FIRST list member carries a relative
    // clause. The list parser must consume it before the comma gate. All three
    // members become distinct entities; the relative clause survives.
    let out = ok("The scent that sold for $75, the white fragrance, and the purple perfume are three different perfumes.");
    assert!(out.contains("Sell"), "first member's relative clause kept; got: {}", out);
    assert!(out.contains("Fragrance") && out.contains("Perfume"), "other members kept; got: {}", out);
    assert!(out.matches("¬").count() >= 3, "pairwise distinctness over 3 members; got: {}", out);
}

#[test]
fn relative_clause_subject_not_misparsed_as_list() {
    // Regression guard: an ordinary relative-clause subject (no list) must still
    // parse normally — the list parser restores cleanly when no comma-list follows.
    let out = ok("The scent that sold for $75 is red.");
    assert!(out.contains("Sell") && out.contains("Red"), "rel-clause subject intact; got: {}", out);
    assert!(!out.contains("¬"), "no spurious distinctness; got: {}", out);
}

#[test]
fn item_items_are_nouns_in_declarative() {
    // "item"/"items" are LOGOS collection keywords only in imperative code; in a
    // declarative clue they are ordinary nouns. "...are three different items."
    // must predicate Items of each member, not strand a reserved token.
    let out = ok("The red car, the blue car and Bladescape are three different items.");
    assert!(out.matches("Items").count() >= 3, "items predicated of each member; got: {}", out);
    let out2 = ok("The item is red.");
    assert!(out2.contains("Item") && out2.contains("Red"), "item as head noun; got: {}", out2);
}

#[test]
fn positional_adverb_in_relative_clause() {
    // "the person who went first" — a positional/temporal adverb ("first") inside
    // a relative clause anchors the event's ordinal position (the same
    // TemporalAnchor the main clause builds). It must not strand.
    let out = ok("The person who went first won.");
    assert!(out.contains("First"), "positional anchor kept; got: {}", out);
    assert!(out.contains("Go") && out.contains("Win"), "both events kept; got: {}", out);
}

#[test]
fn multiword_proper_name_identity() {
    // "The home is Porcher Place." — a multi-word proper name (place/title) is a
    // single entity. The identity must absorb both words ("Porcher_Place"), not
    // strand the second. Place names ("Highland Drive", "Holly Street") pervade
    // the corpus.
    let out = ok("The home is Porcher Place.");
    assert!(out.contains("Porcher_Place"), "multi-word proper name compounded; got: {}", out);
    let out2 = ok("Clark is Superman.");
    assert!(out2.contains("Clark = Superman"), "single-word identity intact; got: {}", out2);
}

#[test]
fn of_pair_xor_with_proper_name_identity_branch() {
    // "Of … one is Porcher Place and the other is on Highland Drive." — one XOR
    // branch is an identity to a multi-word proper name, the other a PP. Both
    // branches, both place names, must survive.
    let out = ok("Of the 1855 home and the Evans family's building, one is Porcher Place and the other is on Highland Drive.");
    assert!(out.contains("Porcher_Place") && out.contains("Highland_Drive"), "both place names kept; got: {}", out);
    assert!(out.contains("∨"), "XOR disjunction present; got: {}", out);
}

#[test]
fn base_is_a_noun_compound_in_relative_clause() {
    // "the person who played second base" — "base" must be available as a noun so
    // "second base" compounds (Second_base), even inside a relative clause where
    // the matrix verb follows the object. Previously "base" lexed verb-only,
    // producing a spurious Base(e) event.
    let out = ok("The person who played second base played 1 more game than Ira.");
    assert!(out.contains("Second_base"), "second base compounded; got: {}", out);
    assert!(out.contains("Add(") && out.contains("Game"), "matrix comparative kept; got: {}", out);
    assert!(!out.contains("Base(e"), "no spurious base event; got: {}", out);
}

#[test]
fn indefinite_object_adjective_kept() {
    // "Bob has a red book." — an indefinite object's adjective is part of its
    // description. Dropping "red" (→ a bare Book(x)) is a silent meaning-loss
    // parse that still returned Ok. Both the kind and the adjective must survive.
    let out = ok("Bob has a red book.");
    assert!(out.contains("Book") && (out.contains("Red")), "object adjective kept; got: {}", out);
}

#[test]
fn indefinite_object_of_measure_kept() {
    // "The Zarobit-C has a maximum range of 250 ft." — the object carries an
    // adjective ("maximum") AND an of-measure restrictor ("of 250 ft"). Both must
    // survive on the object entity, not be dropped or stranded.
    let out = ok("The Zarobit-C has a maximum range of 250 ft.");
    assert!(out.contains("Range") && out.contains("Maximum"), "object adjective kept; got: {}", out);
    assert!(out.contains("Of(") && out.contains("250 ft"), "of-measure restrictor kept; got: {}", out);
}

#[test]
fn quantified_subject_object_adjective_kept() {
    // "Every man has a red book." — under a UNIVERSAL subject the object goes
    // through a distinct (restriction-VP) path that also dropped the object's
    // adjective. All four object-handling paths must preserve the description.
    let out = ok("Every man has a red book.");
    assert!(out.contains("Book") && out.contains("Red"), "object adjective kept under ∀; got: {}", out);
    let out2 = ok("No man owns a red book.");
    assert!(out2.contains("Red"), "object adjective kept under negative quantifier; got: {}", out2);
}

#[test]
fn copula_definite_predicate_nominal_keeps_pp() {
    // "Neither Beta nor Delta is the frat on Holly Street." — the predicate
    // nominal "the frat on Holly Street" carries a PP. Predicating it of each
    // subject must keep On(·, Holly Street); dropping it (→ bare ¬Frat) is a
    // meaning-loss parse. The PP must appear under each negated conjunct.
    let out = ok("Neither Beta nor Delta is the frat on Holly Street.");
    assert!(out.contains("On") && out.contains("Holly_Street"), "predicate-nominal PP kept; got: {}", out);
    assert!(out.contains("Frat"), "predicate noun kept; got: {}", out);
    assert!(out.matches("Holly_Street").count() >= 2, "PP applied to both subjects; got: {}", out);
}

#[test]
fn relative_clause_subject_copula_pp_predicate() {
    // "The animal that is red is from Australia." — a relative-clause subject
    // followed by a matrix copula whose complement is a PP predicate ("is from
    // Australia"). The rel-clause-subject copula path previously only handled
    // NP / adjective / identity complements, so the PP stranded. Both the
    // relative restriction and the PP predicate must survive.
    let out = ok("The animal that is red is from Australia.");
    assert!(out.contains("Red"), "relative clause kept; got: {}", out);
    assert!(out.contains("From") && out.contains("Australia"), "PP predicate kept; got: {}", out);
}

#[test]
fn suit_is_a_noun() {
    // "Bob wears the blue suit." — "suit" was verb-only in the lexicon, stranding
    // it as an NP head after an adjective. It is a common clothing noun.
    let out = ok("Bob wears the blue suit.");
    assert!(out.contains("suit") || out.contains("Suit"), "suit kept as noun; got: {}", out);
    assert!(out.contains("Wear"), "verb kept; got: {}", out);
}

#[test]
fn definite_object_adjective_kept() {
    // "Bob ate the red apple." — a DEFINITE object's adjective is part of its
    // description; dropping "red" (→ bare Theme(e, Apple)) is a silent
    // meaning-loss parse that still returned Ok. Must keep Red(Apple).
    let out = ok("Bob ate the red apple.");
    assert!(out.contains("Apple") && out.contains("Red"), "definite object adjective kept; got: {}", out);
}

#[test]
fn definite_object_adjective_not_confused_with_secondary_predication() {
    // Regression guard: a POST-object adjective ("painted the door red") is a
    // resultative/depictive, NOT an object-NP adjective — must stay Result/Depictive,
    // not be double-counted or mis-placed.
    let out = ok("Bob painted the door red.");
    assert!(out.contains("Result") && out.contains("Red"), "resultative kept; got: {}", out);
}

#[test]
fn compound_color_modifier() {
    // "the lime green shirt" — "lime" (a noun) immediately before the color
    // adjective "green" is a compound-color pre-modifier ("Lime_green"), not the
    // head. Previously "lime" became the head and "green shirt" stranded.
    let out = ok("Bob wears the lime green shirt.");
    assert!(out.contains("Lime_green"), "compound color kept; got: {}", out);
    assert!(out.contains("Shirt"), "head noun kept; got: {}", out);
    // regression: a head noun + adjective predicate via copula is untouched
    let out2 = ok("The red car is fast.");
    assert!(out2.contains("Red") && out2.contains("Fast"), "no spurious compounding; got: {}", out2);
}

#[test]
fn temporal_offset_after_aspectual_verb() {
    // "Bob started skydiving 2 years after Leslie." — a temporal offset trailing
    // an aspectual/presup-trigger verb + gerund ("started skydiving") attaches to
    // the asserted clause; previously it stranded.
    let out = ok("Bob started skydiving 2 years after Leslie.");
    assert!(out.contains("Skydiv"), "complement kept; got: {}", out);
    assert!(out.contains("Add(") && out.contains("Leslie"), "temporal offset kept; got: {}", out);
}

#[test]
fn relative_clause_subject_composes_with_presup_trigger() {
    // LIFT AND SHIFT: "The person who won started skydiving 2 years after Leslie."
    // — an aspectual/presup-trigger matrix ("started skydiving") composes over a
    // RELATIVE-CLAUSE subject, via the shared term-parametric parse_presupposition.
    // Previously the rel-clause matrix dispatch only handled plain verbs, so a
    // presup-trigger matrix stranded.
    let out = ok("The person who won started skydiving 2 years after Leslie.");
    assert!(out.contains("Win") && out.contains("Skydiv"), "rel clause + complement kept; got: {}", out);
    assert!(out.contains("Add(") && out.contains("Leslie"), "temporal offset composed; got: {}", out);
    assert!(out.contains("Presup"), "presupposition projected; got: {}", out);
}

#[test]
fn did_as_main_verb_in_relative_clause() {
    // "The person who did the dishes left." / "who did 49 jumps" — "did" before a
    // non-verb is the MAIN verb "do" (performed), not do-support; previously it
    // was parsed as an auxiliary and the object stranded.
    let out = ok("The person who did the dishes left.");
    assert!(out.contains("Do") && out.contains("Dishes"), "did as main verb 'do'; got: {}", out);
    assert!(out.contains("Leave") || out.contains("Left"), "matrix verb kept; got: {}", out);
}

#[test]
fn digit_counting_np_object_matches_word_form() {
    // "Bob saw 6 brown manatees." — a digit-led counting NP object with an
    // adjective is a COUNT (∃=6 over a manatee entity carrying Brown), NOT a
    // measure. Previously the adjective was mis-read as a measure unit, yielding
    // a bogus `Recipient(e, 6 brown) ∧ Theme(e, Manatees)` — an invented role and
    // a dropped count binding (meaning loss). The digit form must now match the
    // word form ("six brown manatees") exactly.
    let digit = ok("Bob saw 6 brown manatees.");
    let word = ok("Bob saw six brown manatees.");
    assert_eq!(digit, word, "digit and word counting NPs must agree;\n digit: {}\n word:  {}", digit, word);
    assert!(digit.contains("∃=6"), "exact count quantifier kept; got: {}", digit);
    assert!(digit.contains("Brown("), "object adjective predicated of the entity; got: {}", digit);
    assert!(digit.contains("Theme(e, x)"), "the counted entity is the Theme; got: {}", digit);
    assert!(!digit.contains("Recipient"), "no invented Recipient role; got: {}", digit);
}

#[test]
fn digit_counting_np_definite_subject() {
    // The same counting-NP object under a definite subject with its uniqueness
    // wrapper: "The vessel saw 6 brown manatees." → ∃=6 nested inside the vessel's
    // ∃/uniqueness, Brown predicated, Theme over the counted entity.
    let out = ok("The vessel saw 6 brown manatees.");
    assert!(out.contains("∃=6"), "exact count kept; got: {}", out);
    assert!(out.contains("Brown("), "object adjective kept; got: {}", out);
    assert!(out.contains("Vessel("), "definite subject kept; got: {}", out);
    assert!(!out.contains("Recipient"), "no invented Recipient role; got: {}", out);
}

#[test]
fn digit_number_noun_without_adjective_stays_measure() {
    // Regression guard: a bare `Number Noun` with NO adjective is a measure, not a
    // count — the adjective is the discriminator. "Tara scored 190 points." must
    // keep the measure reading (Theme over the 190-point value), unchanged.
    let out = ok("Tara scored 190 points.");
    assert!(out.contains("190"), "measure magnitude kept; got: {}", out);
    assert!(!out.contains("∃=190"), "bare Number Noun stays a measure, not a count; got: {}", out);
    assert!(!out.contains("Recipient"), "no invented Recipient role; got: {}", out);
}

#[test]
fn deverbal_noun_head_counting_np() {
    // "Bob saw 6 previous jumps." — "jumps" is a VERB-only lexicon entry, but a
    // number + adjective forces a nominal, so it is a DEVERBAL NOUN head. The
    // object is a counting NP ∃=6 over a jump entity carrying Previous — NOT a
    // perception small clause (which previously gave Theme(e, [Jumps(Previous)]),
    // silently dropping the count) and NOT a measure. Digit must match word form.
    let digit = ok("Bob saw 6 previous jumps.");
    let word = ok("Bob saw six previous jumps.");
    assert_eq!(digit, word, "digit and word deverbal counting NPs must agree;\n digit: {}\n word: {}", digit, word);
    assert!(digit.contains("∃=6"), "exact count kept; got: {}", digit);
    assert!(digit.contains("Jump(x)"), "deverbal noun head predicated of the entity; got: {}", digit);
    assert!(digit.contains("Previous("), "adjective predicated of the entity; got: {}", digit);
    assert!(digit.contains("Theme(e, x)"), "counted entity is the Theme; got: {}", digit);
    assert!(!digit.contains("Recipient"), "no invented Recipient role; got: {}", digit);
    assert!(!digit.contains('['), "no small-clause Proposition term; got: {}", digit);
}

#[test]
fn deverbal_counting_np_under_definite_subject_and_ditransitive() {
    // The headline corpus pattern: a definite subject + ditransitive "made" + a
    // deverbal counting-NP object. "The diver made 49 previous jumps." →
    // ∃=49 x(Jump(x) ∧ Previous(x) ∧ … Theme(e, x)) nested under the diver's
    // uniqueness; "made" must NOT split it into Recipient + Theme.
    let out = ok("The diver made 49 previous jumps.");
    assert!(out.contains("∃=49"), "exact count kept; got: {}", out);
    assert!(out.contains("Jump(x)") && out.contains("Previous("), "deverbal noun + adjective kept; got: {}", out);
    assert!(out.contains("Diver("), "definite subject kept; got: {}", out);
    assert!(!out.contains("Recipient"), "no invented Recipient role; got: {}", out);
}

#[test]
fn genuine_perception_small_clause_preserved() {
    // Regression guard for the perception bail: a REAL perceived-event small
    // clause (a noun subject before the inner verb) must still parse as a
    // proposition, not be mistaken for a counting-NP object.
    let out = ok("Mary heard the bell ring.");
    assert!(out.contains("Hear"), "perception verb kept; got: {}", out);
    assert!(out.contains("Ring") && out.contains("Bell"), "perceived event kept; got: {}", out);
}

#[test]
fn deverbal_counting_np_in_relative_clause_restriction() {
    // The relative-clause restriction-VP path (quantifier.rs): "The skydiver who
    // completed 49 previous jumps wears the white suit." — the counting NP binds
    // a ∃=49 entity INSIDE the relative clause, scoped under the head's own
    // binding, with the adjective preserved. Previously this stranded ("jumps"
    // TrailingTokens). The matrix clause ("wears the white suit") is kept.
    let out = ok("The skydiver who completed 49 previous jumps wears the white suit.");
    assert!(out.contains("∃=49"), "exact count in relative clause; got: {}", out);
    assert!(out.contains("Jump(") && out.contains("Previous("), "deverbal noun + adjective kept; got: {}", out);
    assert!(out.contains("Skydiver("), "head noun kept; got: {}", out);
    assert!(out.contains("Wear") && out.contains("Suit"), "matrix clause kept; got: {}", out);
    assert!(!out.contains("Recipient"), "no invented Recipient role; got: {}", out);
}

#[test]
fn deverbal_counting_np_restriction_noun_head_now_parses() {
    // Regression+fix: "The vessel that saw 6 brown manatees went to Yellow Bend."
    // previously failed (TrailingTokens) — the restriction-VP had no counting-NP
    // branch. Now it binds ∃=6 and the matrix "went to Yellow Bend" survives.
    let out = ok("The vessel that saw 6 brown manatees went to Yellow Bend.");
    assert!(out.contains("∃=6"), "count kept in relative clause; got: {}", out);
    assert!(out.contains("Brown("), "object adjective kept; got: {}", out);
    assert!(out.contains("Yellow_Bend") || out.contains("Bend"), "matrix clause kept; got: {}", out);
}

#[test]
fn deverbal_counting_np_under_perfect_aspect() {
    // The perfect/modal object path (modal.rs): "Philip has completed 49 previous
    // jumps." — a counting NP under perfect aspect binds ∃=49 keeping the
    // adjective; previously the digit object stranded ("49" TrailingTokens).
    let out = ok("Philip has completed 49 previous jumps.");
    assert!(out.contains("∃=49"), "count under perfect aspect; got: {}", out);
    assert!(out.contains("Jump(") && out.contains("Previous("), "deverbal noun + adjective kept; got: {}", out);
    assert!(out.contains("Perf"), "perfect aspect kept; got: {}", out);
}

#[test]
fn did_as_main_verb_in_main_clause() {
    // "Philip did the dishes." — "did" (Auxiliary(Past)) followed by an object NP
    // is the MAIN verb "Do", not do-support. parse_predicate_impl handled this;
    // the simple-clause path (parse_atom) did not, so proper-name-subject "did X"
    // stranded (TrailingTokens). Now it builds a Do event.
    let out = ok("Philip did the dishes.");
    assert!(out.contains("Do(e)") && out.contains("Agent(e, Philip)"), "did → main verb Do; got: {}", out);
    assert!(out.contains("Dishes"), "object kept; got: {}", out);
    // True auxiliary usages must be UNCHANGED.
    let neg = ok("John did not run.");
    assert!(neg.contains("¬") && neg.contains("Run"), "did not → negated do-support; got: {}", neg);
    let emph = ok("John did run.");
    assert!(emph.contains("Run(e)") && !emph.contains("Do(e)"), "did run → emphatic do-support; got: {}", emph);
}

#[test]
fn did_as_main_verb_with_counting_np_object() {
    // "Philip did 49 previous jumps." — main-verb "did" + a deverbal counting NP
    // object → ∃=49 over the jump entity carrying Previous. (The headline
    // skydiving construction, modulo the perfect "has done" / of-pair layers.)
    let out = ok("Philip did 49 previous jumps.");
    assert!(out.contains("∃=49"), "count kept; got: {}", out);
    assert!(out.contains("Do(e)"), "main verb Do; got: {}", out);
    assert!(out.contains("Jump(") && out.contains("Previous("), "deverbal noun + adjective kept; got: {}", out);
    assert!(!out.contains("Recipient"), "no invented Recipient role; got: {}", out);
}

#[test]
fn label_noun_with_digit_value() {
    // "number 7" / "page 204" with a DIGIT (not just the word "seven") joins the
    // label noun into one entity symbol. Previously the digit split into a bogus
    // Recipient(Number) ∧ Theme(7) — a label-losing parse. Digit must match word.
    let digit = ok("Tara wore number 7.");
    assert!(digit.contains("Number_7"), "digit label joined; got: {}", digit);
    assert!(!digit.contains("Recipient"), "no split Recipient/Theme; got: {}", digit);
    let word = ok("Tara wore number seven.");
    assert_eq!(digit, word, "digit and word labels must agree;\n digit: {}\n word: {}", digit, word);
    let page = ok("The book is on page 204.");
    assert!(page.contains("Page_204"), "digit page label; got: {}", page);
    // Regression: a plain number object on a NON-label noun stays a measure.
    let plain = ok("Bessie played 9 games.");
    assert!(plain.contains("9 games") && !plain.contains("Games_9"), "plain count not a label; got: {}", plain);
}

#[test]
fn copula_complement_relative_clause() {
    // "X is the Y who/that Z" — a relative clause on the predicate nominal is
    // predicated of the subject (being that player entails X played). Previously
    // the "who"/"that" stranded (TrailingTokens), failing this pervasive pattern.
    let out = ok("Zachary was the player who played 9 games.");
    assert!(out.contains("Player(Zachary)"), "predicate nominal kept; got: {}", out);
    assert!(out.contains("Play(e)") && out.contains("9 games"), "relative clause + object kept; got: {}", out);
    // composes under a definite (uniqueness) subject + the relative clause
    let rel = ok("The winner who got the medal was the player who played 9 games.");
    assert!(rel.contains("Winner") && rel.contains("Player(") && rel.contains("Play(e)"), "stacked rel clauses; got: {}", rel);
}

#[test]
fn copula_complement_relative_clause_under_neither_and_either() {
    // The shared conjoin_trailing_relative helper composes across the neither/nor
    // and either/or copula-complement paths (verb.rs + rel-clause-subject in mod.rs).
    let neither = ok("Neither Zachary nor Pam was the player who played.");
    assert!(neither.contains("Player(Zachary)") && neither.contains("Play(e)"), "neither + complement rc; got: {}", neither);
    assert!(neither.contains("¬"), "neither negates; got: {}", neither);
    let either = ok("The person who got the prize was either Heather or the one who gave the talk.");
    assert!(either.contains("Heather") && either.contains("Give(e)"), "either + 2nd-disjunct rc; got: {}", either);
    assert!(either.contains("∨"), "either is a disjunction; got: {}", either);
}

#[test]
fn bare_comparative_predicate_no_than() {
    // "X is older" with no "than" — a comparative predicate relative to an implied
    // standard (the other entity in a pair). Previously ExpectedThan. Build the
    // COMPARATIVE Older(X), not the base Old(X) — the degree must survive.
    let out = ok("The gnome is older.");
    assert!(out.contains("Older(Gnome)"), "comparative degree kept (not Old); got: {}", out);
    // composes in an of-pair XOR: complementary comparative predicates pair up
    let pair = ok("Of the falcon that won and the cobra, one is older and the other is faster.");
    assert!(pair.contains("Older(") && pair.contains("Faster("), "of-pair comparative VPs; got: {}", pair);
    assert!(pair.contains("∨"), "of-pair is an XOR disjunction; got: {}", pair);
    // regression: comparative WITH "than" is unchanged (binary)
    let than = ok("The gnome is older than the falcon.");
    assert!(than.contains("Older(Gnome, Falcon)"), "comparative with standard kept; got: {}", than);
    // regression: plain adjective is not a comparative
    let plain = ok("The gnome is old.");
    assert!(plain.contains("Old(") && !plain.contains("Older"), "plain adjective unchanged; got: {}", plain);
}

#[test]
fn copula_measure_complement_in_quantified_and_of_pair_vps() {
    // "is 14 inches tall" — a measure phrase + dimensional adjective → Tall(x, 14
    // inches). parse_atom had this; the verb.rs copula (of-pair / quantified
    // subjects) did not, so of-pair VPs like "the other is 14 inches tall"
    // stranded. Now they compose (with the bare-comparative fix too).
    let pair = ok("Of the snake and the cobra, one is 30 inches long and the other is shorter.");
    assert!(pair.contains("Long(") && pair.contains("30 inches"), "measure complement in of-pair VP; got: {}", pair);
    assert!(pair.contains("Shorter("), "bare comparative composes; got: {}", pair);
    assert!(pair.contains("∨"), "of-pair XOR; got: {}", pair);
    // regression: parse_atom measure complement unchanged
    let std = ok("The gnome is 14 inches tall.");
    assert!(std.contains("Tall(") && std.contains("14 inches"), "standalone measure complement; got: {}", std);
    // a measure-OFFSET comparative must NOT be mis-read as Identity
    let off = ok("The gnome is 2 inches taller than the falcon.");
    assert!(off.contains("Taller") || off.contains("Tall"), "offset comparative kept; got: {}", off);
}

#[test]
fn of_pair_classifier_noun_skipped() {
    // "Of NP1 and NP2, one TYPE … the other TYPE …" — the redundant classifier
    // noun after "one"/"the other" (the of-pair already binds the entity) is
    // skipped so the VP starts at the real predicate. Previously the of-pair scan
    // left "type" and bailed (fell back, failing at "Of").
    let out = ok("Of the goods that sell for $2.50 per pound and the artichokes, one type is sold by Vincent and the other type is from Iowa City.");
    assert!(out.contains("Vincent") && out.contains("Iowa"), "both of-pair VPs kept; got: {}", out);
    assert!(out.contains("∨"), "of-pair XOR; got: {}", out);
    // regression: an aspectual verb after "one" is NOT a classifier (don't skip it)
    let asp = ok("Of Tara and Bessie, one started skydiving and the other quit.");
    assert!(asp.contains("Skydiv") && asp.contains("Quit"), "aspectual verb kept; got: {}", asp);
}

#[test]
fn temporal_offset_after_direct_object() {
    // "X won her prize 4 years after Y" — a temporal offset trailing the DIRECT
    // OBJECT (not immediately after the verb) → solver-ready add/sub relating the
    // two positions. parse_temporal_offset_constraint is now invoked after the
    // object/PP loop; previously the offset stranded (Trailing/Number).
    let out = ok("Tara won her prize 4 years after Glenda.");
    assert!(out.contains("Win(e)"), "main event kept; got: {}", out);
    assert!(out.contains("Years(Tara)") && out.contains("Add(") && out.contains("Glenda"),
        "solver-ready temporal offset (add); got: {}", out);
    // a descriptive standard binds a distinct entity with its restrictor
    let desc = ok("The winner won her prize 4 years after the person from France.");
    assert!(desc.contains("Years") && desc.contains("France"), "descriptive standard kept; got: {}", desc);
}

#[test]
fn temporal_offset_after_indefinite_object() {
    // "X has a birthday 4 days before Y" — the temporal offset trails an
    // INDEFINITE/quantified object (∃ over the object), so the constraint
    // conjoins OUTSIDE the object quantifier (it relates the SUBJECT's position).
    // Previously stranded (Trailing/Number); the obj-quantifier path now invokes
    // the same self-guarding temporal-offset check.
    let out = ok("The 12-year-old has a birthday 8 days after the one from Cornville.");
    assert!(out.contains("Birthday(") , "object quantifier kept; got: {}", out);
    assert!(out.contains("Days") && out.contains("Add(") && out.contains("Cornville"),
        "solver-ready temporal offset with descriptive standard; got: {}", out);
}

#[test]
fn ing_reduced_relative_with_direct_object() {
    // "the origami DEPICTING a dragon" — an active -ing reduced relative with a
    // DIRECT OBJECT → Depict(x, Dragon). Previously only PP complements ("arriving
    // AT Paradise") were consumed, so the object stranded ("a" TrailingTokens).
    let out = ok("The origami depicting a dragon is blue.");
    assert!(out.contains("Origami(") && out.contains("Depict(") && out.contains("Dragon"),
        "reduced relative + object kept; got: {}", out);
    assert!(out.contains("Blue("), "matrix predicate kept; got: {}", out);
    // regressions: PP-complement -ing, passive participle, and finite -ing predicate
    let pp = ok("The person arriving at Paradise won.");
    assert!(pp.contains("Arriv") && pp.contains("Paradise"), "PP-complement -ing unchanged; got: {}", pp);
    let prog = ok("The dog is running.");
    assert!(prog.contains("Prog") || prog.contains("Run"), "finite -ing predicate unchanged; got: {}", prog);
}

#[test]
fn counting_np_with_propername_modifier() {
    // "has 78 LinkedIn connections", "640 Twitter followers" — a counting NP whose
    // modifier is a ProperName (Twitter/LinkedIn/Facebook), not an adjective →
    // ∃=78 over the (compound) noun. Previously split into a bogus measure
    // (Recipient(78 LinkedIn) ∧ Theme(Connections)).
    let out = ok("The person has 78 LinkedIn connections.");
    assert!(out.contains("∃=78"), "exact count kept; got: {}", out);
    assert!(out.contains("LinkedIn") && out.contains("connections"), "modifier + head kept; got: {}", out);
    assert!(!out.contains("Recipient"), "no bogus measure split; got: {}", out);
    // regression: a bare Number+unit (no modifier) stays a measure
    let m = ok("Tara scored 190 points.");
    assert!(m.contains("190") && !m.contains("∃=190"), "measure unchanged; got: {}", m);
}

#[test]
fn rate_denominator_after_measure() {
    // "pays $700 a month", "700 dollars per month", "$2.50 per pound" — a rate
    // denominator after a MEASURE object → Per(event, unit), solver-ready.
    // Previously parse_measure_phrase ate the article "a" as a unit and "month"
    // stranded; the measure branch's content-word check also grabbed "a".
    let a = ok("The family pays $700 a month.");
    assert!(a.contains("Per(") && a.contains("Month"), "a-rate kept; got: {}", a);
    let per = ok("Bob pays 700 dollars per month.");
    assert!(per.contains("700 dollars") && per.contains("Per(") && per.contains("Month"), "per-rate kept; got: {}", per);
    // regression: a real noun-after-measure ("3 children") is NOT a rate
    let kids = ok("Mary has 3 children.");
    assert!(kids.contains("3 children") && !kids.contains("Per("), "noun-after-measure unchanged; got: {}", kids);
}

#[test]
fn progressive_transitive_direct_object() {
    // "is paying the rent", "is reading a book", "is paying $700" — a PROGRESSIVE
    // transitive takes a DIRECT OBJECT → Prog(Verb(subject, object)). The copula
    // passive/progressive path handled to-goals and by-passives but not a direct
    // object, so it stranded. Gated to Progressive (a passive's theme is the subject).
    let np = ok("The family is paying the rent.");
    assert!(np.contains("Pay(Family, Rent)") && np.contains("Prog"), "progressive transitive object; got: {}", np);
    let meas = ok("The family is paying $700.");
    assert!(meas.contains("Pay(Family, 700)"), "progressive measure object; got: {}", meas);
    // regressions: passive (no object), intransitive progressive + PP, by-passive
    let pass = ok("The book was read.");
    assert!(pass.contains("Read(Book)") && !pass.contains("Read(Book,"), "passive has no object; got: {}", pass);
    let intr = ok("The family is moving in June.");
    assert!(intr.contains("Move(Family)") && intr.contains("June"), "intransitive progressive + PP; got: {}", intr);
}

#[test]
fn instance_label_is_general_not_a_word_list() {
    // A head noun + a bare number is an instance LABEL — and this is a GENERAL
    // grammatical rule, NOT a hardcoded word list: it must work for nouns never
    // seen in the curated set ("car 7", "exhibit 5", "lane 3"), so it generalizes
    // to unseen clues. Both digits and word-numbers join into one symbol.
    for (clue, sym) in [
        ("The player in lane 3 won.", "Lane_3"),
        ("The car 7 is fast.", "Car_7"),
        ("The exhibit 5 is famous.", "Exhibit_5"),
        ("Bob wore number 7.", "Number_7"),
    ] {
        let out = ok(clue);
        assert!(out.contains(sym), "instance label {} for {:?}; got: {}", sym, clue, out);
    }
    // Exclusions: a MEASURE (number + unit) is not a label; a counting NP (number
    // BEFORE the noun) is not a label; a MONTH+day is a date, not a label.
    let meas = ok("The gnome is 14 inches tall.");
    assert!(meas.contains("14 inches") && !meas.contains("Gnome_14"), "measure not a label; got: {}", meas);
    let count = ok("The team won 3 games.");
    assert!(count.contains("3 games") && !count.contains("Games_3"), "counting NP not a label; got: {}", count);
    let date = ok("The child has the April 15th birthday.");
    assert!(date.contains("April") && !date.contains("April_15("), "month+day is a date not a label; got: {}", date);
}

#[test]
fn of_pair_elided_possessive_predicate() {
    // "Of A and B, one VP and the other is Ginger's." — a copula predicate with an
    // ELIDED possessed noun ("is Ginger's.") in the of-pair / quantified-subject
    // VP. parse_atom handled it; the verb.rs copula (which of-pair VPs route
    // through) did not — it stranded at the period (ExpectedContentWord).
    let out = ok("Of Tara and Bessie, one is Ginger's and the other is Tasha's.");
    assert!(out.contains("Possesses(Ginger,") && out.contains("Possesses(Tasha,"), "both elided possessives; got: {}", out);
    assert!(out.contains("∨"), "of-pair XOR; got: {}", out);
    // regressions: standalone elided possessive + full possessive NP complement
    let std = ok("Tara is Ginger's.");
    assert!(std.contains("Possesses(Ginger, Tara)"), "standalone elided possessive; got: {}", std);
    let full = ok("Tara is Kerry's project.");
    assert!(full.contains("Project(Tara)") && full.contains("Possesses(Kerry, Tara)"), "full possessive NP unchanged; got: {}", full);
}

#[test]
fn reduced_relative_numeric_pp_object() {
    // "the seashell found IN 1992", "the message that was sent in 1976" — a NUMERIC
    // PP object (a year/amount) in a reduced-relative / participle PP. Previously
    // the PP loop only took noun/article objects, so the year stranded.
    let out = ok("The seashell found in 1992 is rare.");
    assert!(out.contains("Find(") && out.contains("In(") && out.contains("1992"), "year PP kept; got: {}", out);
    // composes in an either/or with year-dated disjuncts
    let either = ok("George's letter is either the message that was sent in 1976 or the bottle found in 2010.");
    assert!(either.contains("1976") && either.contains("2010") && either.contains("∨"), "both year disjuncts; got: {}", either);
}

#[test]
fn verb_tagged_noun_compound_in_pp_object() {
    // "with a cork COVER", "with a faux leather COVER", "with an amber BASE" — a
    // base-form verb-only word after a noun head, INSIDE a PP object (an
    // unambiguously nominal tail), is a deverbal noun-noun compound, not a verb.
    let out = ok("The journal with a cork cover is cheap.");
    assert!(out.contains("Cork_Cover") && !out.contains("Cover(e)"), "cork cover compound; got: {}", out);
    let leather = ok("The book with a faux leather cover won.");
    assert!(leather.contains("leather_Cover") || leather.contains("Leather_Cover"), "leather cover compound; got: {}", leather);
    // regressions: a SUBJECT base-form verb (no nominal context) stays a verb
    let subj = ok("The people vote in November.");
    assert!(subj.contains("Vote(e)") && !subj.contains("People_vote"), "subject verb not folded; got: {}", subj);
    let runs = ok("The man runs fast.");
    assert!(runs.contains("Run(e)") && !runs.contains("Man_runs"), "subject verb not folded; got: {}", runs);
}

#[test]
fn verb_tagged_noun_compound_in_copula_complement() {
    // A copula complement is parsed AFTER the copula is consumed, so a verb-word
    // head in it can never be the matrix verb — it is a deverbal noun-noun
    // compound. "the orange PACK", "the Russell Road PROJECT".
    let pack = ok("The Lugmor pack is the orange pack.");
    assert!(pack.contains("Orange_Pack") && !pack.contains("Pack(e)"), "orange pack compound; got: {}", pack);
    let proj = ok("The job is the Russell Road project.");
    assert!(proj.contains("Russell_Road_project") || proj.contains("Russell Road_project") || proj.to_lowercase().contains("project"), "russell road project; got: {}", proj);
    assert!(!proj.contains("Project(e)"), "project not a verb; got: {}", proj);
}

#[test]
fn verb_tagged_noun_compound_in_either_disjunct() {
    // Disjuncts of "is either A or B" are likewise post-copula nominal positions.
    let out = ok("The Lugmor pack is either Leroy's pack or the silver pack.");
    assert!(out.contains("Silver_Pack") && !out.contains("Pack(e)"), "silver pack disjunct compound; got: {}", out);
    assert!(out.contains("∨"), "must be a disjunction; got: {}", out);
    // possessor of the first disjunct preserved (no meaning loss)
    assert!(out.contains("Leroy") && out.contains("Possesses"), "possessor kept; got: {}", out);
}

#[test]
fn verb_tagged_noun_compound_in_comparative_standard() {
    // A "than" standard is a nominal position — a verb-word head there is a
    // deverbal noun ("larger than the orange PACK", "than the investing SHOW").
    let pack = ok("The Pinkster pack is 10 liters larger than the orange pack.");
    assert!(pack.contains("Orange_Pack") && !pack.contains("Pack(e)"), "orange pack standard; got: {}", pack);
    let show = ok("Al Acosta's show has 2 million more downloads than the investing show.");
    assert!(show.contains("Invest_Show") && !show.contains("Show(e)"), "investing show standard; got: {}", show);
    // bare proper-name standard stays a constant (no regression)
    let bare = ok("Bob is taller than Tom.");
    assert!(bare.contains("Tom"), "bare name standard; got: {}", bare);
}

#[test]
fn verb_tagged_noun_compound_in_temporal_standard() {
    // A "N unit after/before STD" temporal standard is nominal — a verb-word
    // head is a deverbal noun ("after the goblin shark PROJECT", "before the SHOW").
    let proj = ok("The project starts 1 month after the goblin shark project.");
    assert!(proj.contains("Goblin_shark_project") || proj.contains("Goblin_Shark_project") || proj.contains("shark_project"), "goblin shark project standard; got: {}", proj);
    assert!(!proj.to_lowercase().contains("project(e)"), "project not a verb; got: {}", proj);
    // bare-name temporal standard stays a constant (no regression)
    let bare = ok("Tara performed 2 weeks after Bessie.");
    assert!(bare.contains("Bessie"), "bare name temporal standard; got: {}", bare);
}

#[test]
fn predicate_adjective_postposed_measure_complement() {
    // "is worth $N" — a predicate adjective with a POSTPOSED money/measure
    // complement → Worth(subject, $N). Both the simple-subject (parse_atom) and
    // the shared VP parser (of-pair/neither/quantified) must handle it.
    let simple = ok("The mogul is worth $26 billion.");
    assert!(simple.contains("Worth(") && simple.contains("26 billion"), "simple worth; got: {}", simple);
    // of-pair: BOTH branches must keep Worth AND the other predicate (zero loss)
    let pair = ok("Of Nadine Newton and the mogul from Denmark, one works in the software industry and the other is worth $26 billion.");
    assert!(pair.matches("Worth(").count() >= 2, "both XOR branches keep Worth; got: {}", pair);
    assert!(pair.contains("Work(") && pair.contains("∨"), "work predicate + disjunction kept; got: {}", pair);
}

#[test]
fn place_name_with_state_abbreviation() {
    // US "City, ST" — one location entity; the state disambiguator is kept in the
    // symbol (zero meaning loss). Works in copula PP, comparative standard, of-pair.
    let copula = ok("Mary is from Barnstable, ME.");
    assert!(copula.contains("From(Mary, Barnstable_ME)"), "copula City,ST; got: {}", copula);
    let std = ok("The Ninigreth oyster costs somewhat more than the one from Barnstable, ME.");
    assert!(std.contains("Barnstable_ME"), "comparative-standard City,ST; got: {}", std);
    // a Title-case name after a comma is NOT a state (no spurious merge)
    let names = ok("Mary is from France.");
    assert!(names.contains("From(Mary, France)"), "plain place unaffected; got: {}", names);
}

#[test]
fn has_object_as_role_predicative() {
    // "X has Y as [its] Z" — Y fills role Z for X. FOL: Have(X, Y) ∧ Z(Y); the
    // "its" (= X) is redundant with the Have-link. Tested on the corpus form,
    // an of-pair member, which routes through the shared parse_predicate_impl VP
    // parser — BOTH XOR branches must keep the Have-link AND the role predicate
    // (zero meaning loss). (The simple-subject standalone routes through the
    // duplicated parse_atom object path and is an honest-fail pending the
    // parse_atom/verb.rs copula-VP unification — never the old lossy bundle.)
    let pair = ok("Of the town in Plymouth County and the city with a population of 31,000, one is Wapello and the other has Al Acosta as its mayor.");
    assert!(pair.matches("Mayor(Al_Acosta)").count() >= 2, "role kept in both XOR branches; got: {}", pair);
    assert!(pair.matches("Theme(e, Al_Acosta)").count() >= 2, "Have-link kept in both branches; got: {}", pair);
    assert!(pair.contains("∨"), "of-pair disjunction; got: {}", pair);
    assert!(!pair.contains("_as_") && !pair.contains("_As_"), "no lossy 'as'-bundling; got: {}", pair);

    // The standalone simple-subject form must NEVER produce the old lossy
    // "Smith_as_captain" bundle — honest failure is acceptable, meaning-loss is not.
    if let Ok(f) = compile("The team has Smith as captain.") {
        let s = format!("{}", f);
        assert!(!s.contains("_as_") && !s.contains("_As_"), "no lossy 'as'-bundling; got: {}", s);
    }
}

#[test]
fn relative_clause_indefinite_object_folds_compound() {
    // The indefinite-article object of a relative-clause verb must parse the FULL
    // NP (folding a noun-noun compound) and keep its adjectives/PPs — previously a
    // single consume_content_word() grabbed only the first word ("a kayaking
    // REGIMEN" stranded "regimen").
    let yoga = ok("The person who used the vegetarian diet lost 2 fewer pounds than the friend who started a yoga regimen.");
    assert!(yoga.contains("Yoga_regimen") && !yoga.contains("Regimen("), "compound folded; got: {}", yoga);
    // adjective on an indefinite rel-clause object preserved (zero meaning loss)
    let adj = ok("The person who has a red car won.");
    assert!(adj.contains("Red(") , "object adjective kept; got: {}", adj);
    // donkey anaphora over an indefinite rel-clause object still binds
    let donkey = ok("Every farmer who owns a donkey beats it.");
    assert!(donkey.contains("Donkey(") && donkey.contains("Beat("), "donkey binding intact; got: {}", donkey);
}

#[test]
fn indefinite_gerund_premodifier_is_article_not_variable() {
    // "a kayaking regimen", "a running shoe" — a PROGRESSIVE verb (gerund) before
    // a noun head is a pre-nominal modifier, so "a" is an ARTICLE, not a logic
    // variable. The is_variable_a heuristic must not strand the noun head.
    let won = ok("The person who started a kayaking regimen won.");
    assert!(won.contains("Kayak_regimen") && won.contains("Win("), "gerund NP folds, matrix verb attaches; got: {}", won);
    let either = ok("Mandy is either the friend who used the gluten-free diet or the dieter who started a kayaking regimen.");
    assert!(either.contains("Kayak_regimen") && either.contains("∨"), "gerund in either-disjunct; got: {}", either);
    // regression: a logic-variable "a" before a finite verb stays a variable
    let donkey = ok("Every farmer who owns a donkey beats it.");
    assert!(donkey.contains("Donkey(") && donkey.contains("Beat("), "donkey binding intact; got: {}", donkey);
}

#[test]
fn plural_subject_different_predicate_keeps_distinctness() {
    // "A and B are different N" — "different" asserts the members are pairwise
    // DISTINCT; dropping it loses the AllDifferent constraint. The members are
    // also each predicated N (zero meaning loss).
    let out = ok("Tara and Bessie are different people.");
    assert!(out.contains("People(Tara)") && out.contains("People(Bessie)"), "both members predicated; got: {}", out);
    assert!(out.contains("¬Tara = Bessie") || out.contains("¬(Tara = Bessie)") || (out.contains("Tara = Bessie") && out.contains("¬")), "distinctness kept; got: {}", out);
    // regression: plural subject without "different" stays plain distribution
    let men = ok("Socrates and Plato are men.");
    assert!(men.contains("Men(Socrates)") && men.contains("Men(Plato)") && !men.contains("¬"), "no spurious distinctness; got: {}", men);
}

#[test]
fn measure_premodified_noun_folds_across_paths() {
    // "N unit NOUN" ("190 degree water") is ONE premodified-noun entity
    // (190_degree_water), folded in rel-clause objects, PP objects, and modal
    // PPs — not a measure with the head stranded. The count-measure forms ("190
    // points", "6 manatees") stay bare measures (the head is the unit, no extra
    // head noun follows).
    let rel = ok("The Ali Shan is either the tea that steeps for 2.5 minutes or the variety that requires 190 degree water.");
    assert!(rel.contains("190_degree_water") && !rel.contains("Recipient"), "rel-clause fold; got: {}", rel);
    let modal = ok("The tea that steeps for 4 minutes shouldn't brew with 190 degree water.");
    assert!(modal.contains("190_degree_water"), "modal-PP fold; got: {}", modal);
    // regression: a bare measure object keeps its count, matrix verb attaches
    let pts = ok("The team that scored 190 points won.");
    assert!(pts.contains("190 Point") || pts.contains("190 points"), "count measure preserved; got: {}", pts);
    assert!(pts.contains("Win("), "matrix verb attaches; got: {}", pts);
    // regression: measure PP stays a measure
    let ppl = ok("The team with 15 people won.");
    assert!(ppl.contains("15 people"), "measure PP preserved; got: {}", ppl);
}

#[test]
fn decimal_money_premodifier_folds_in_disjunct() {
    // "$5.25 purchase" — a DECIMAL/grouped money amount must count as a numeric
    // head so the deverbal-noun fold fires (5.25_Purchase), not split into a
    // 5.25 entity + a stray Purchase event. head_is_numeric accepts '.'/','.
    let dec = ok("The order is either the order that included cranberries or the $5.25 purchase.");
    assert!(dec.contains("5.25_Purchase") && !dec.contains("Purchase(e)"), "decimal money folds; got: {}", dec);
    // integer + large grouped numbers still fold
    let int = ok("The order is the $5 purchase.");
    assert!(int.contains("5_Purchase"), "integer money folds; got: {}", int);
}

#[test]
fn progressive_verb_in_copular_relative_clause() {
    // "that is printing 100 pages" — a verb after the copula in a copular
    // relative is a PROGRESSIVE verb phrase (∃e(Print(e) ∧ Agent ∧ Theme)), not
    // a predicate adjective; its object must not strand.
    let out = ok("The press that is printing 100 pages won.");
    assert!(out.contains("Print(") && out.contains("100 pages") && out.contains("Win("), "progressive VP + object + matrix verb; got: {}", out);
    // regression: predicate-adjective / PP / measure copular relatives unaffected
    let red = ok("The stamp that is red won.");
    assert!(red.contains("Red(x)") || red.contains("Red("), "adjective copular relative; got: {}", red);
    let pp = ok("The book that is on the table won.");
    assert!(pp.contains("On(") , "PP copular relative; got: {}", pp);
    let meas = ok("The rope that is 30 inches long won.");
    assert!(meas.contains("Long(") && meas.contains("30 inches"), "measure copular relative; got: {}", meas);
}

#[test]
fn alphanumeric_codes_and_grades_lex_as_one_token() {
    // "AV-435"/"FRZ-192" (uppercase code + hyphen + digits) and "B+"/"A+"
    // (grade) are single identifiers, not arithmetic minus/plus.
    let code = ok("The graduate who will be studying solar storms will be on mission AV-435.");
    assert!(code.contains("AV-435"), "code kept whole; got: {}", code);
    let plate = ok("The car has the FRZ-192 plates.");
    assert!(plate.contains("FRZ-192"), "plate code kept; got: {}", plate);
    let grade = ok("The student got an A+.");
    assert!(grade.contains("A+"), "grade kept whole; got: {}", grade);
}

#[test]
fn multiword_proper_name_possessor_complement() {
    // "is Tim Tucker's film" — a MULTI-WORD proper-name possessor in a copula
    // complement: Possesses(Tim_Tucker, x) ∧ Film(x), kept (not stranded at "'s").
    // Routes through the shared VP parser (neither/nor, of-pair, quantified subj).
    let out = ok("Neither the movie with a running time of 65 minutes nor Tearful Night is Tim Tucker's film.");
    assert!(out.contains("Tim_Tucker") && out.contains("Possesses"), "multi-word possessor kept; got: {}", out);
    // regression: single-word possessor + elided still correct
    let kerry = ok("The film is Kerry's film.");
    assert!(kerry.contains("Possesses(Kerry") && kerry.contains("Film("), "single possessor; got: {}", kerry);
    let drive = ok("The place is Highland Drive.");
    assert!(drive.contains("Highland_Drive"), "multi-word identity (no possessive) unaffected; got: {}", drive);
}

#[test]
fn multiword_proper_name_ending_in_verb_word() {
    // "Bald Hill Run" — a place name whose last word lexes as a verb ("Run"); a
    // CAPITALIZED verb-word after a CAPITALIZED proper-name head is a name part.
    let out = ok("The event was at Bald Hill Run.");
    assert!(out.contains("Bald_Hill_Run") && !out.contains("Run(e)"), "place name folds; got: {}", out);
    // regression: lowercase sentence verbs stay verbs
    let runs = ok("The man runs.");
    assert!(runs.contains("Run(e)") && !runs.contains("Man_runs"), "subject verb not folded; got: {}", runs);
    let won = ok("John won.");
    assert!(won.contains("Win(") && !won.contains("John_won"), "matrix verb not folded; got: {}", won);
}

#[test]
fn verb_tagged_noun_in_object_position_folds_at_boundary() {
    // A base-form verb-word in an OBJECT NP at a clause boundary is a deverbal
    // noun ("the onion DIP", "the spicy tataki ROLL") — folded via the Verb arm's
    // object_compound_boundary. Donkey/small-clause cases must stay intact.
    let dip = ok("The friend who brought the onion dip won.");
    assert!(dip.contains("Onion_Dip") && !dip.contains("Dip(e)"), "onion dip folds; got: {}", dip);
    // donkey: the quantifier's nuclear verb (pronoun follows) is NOT folded
    let donkey = ok("Most farmers who own a donkey beat it.");
    assert!(donkey.contains("Beat(") && (donkey.contains("MOST") || donkey.contains("Most")) && !donkey.contains("Donkey_beat"), "donkey nuclear verb safe; got: {}", donkey);
    // perception small clause preserved
    let saw = ok("The teacher saw the man run.");
    assert!(saw.contains("Run(Man)") && !saw.contains("Man_run"), "small clause preserved; got: {}", saw);
}

#[test]
fn coordinated_predicate_adjectives_and_ampersand_color() {
    // "is black & red" / "is brown and red" — coordinated predicate adjectives →
    // Adj1(x) ∧ Adj2(x). "&" between lowercase words lexes to "and" (color); "&"
    // between capitalized names ("Leach & Mccall") stays a firm-name joiner.
    let amp = ok("The pygmy racer is brown & red.");
    assert!(amp.contains("Brown(") && amp.contains("Red("), "ampersand color coordinates; got: {}", amp);
    let and = ok("The animal is black and red.");
    assert!(and.contains("Black(") && and.contains("Red("), "and color coordinates; got: {}", and);
    let firm = ok("Kelvin was hired by Leach & Mccall.");
    assert!(firm.contains("Leach_Mccall"), "firm name kept whole; got: {}", firm);
    // of-pair (shared VP parser) color
    let pair = ok("Of the asp and the cobra, one is from Spain and the other is black & red.");
    assert!(pair.contains("Black(") && pair.contains("Red("), "of-pair color coordinates; got: {}", pair);
}

#[test]
fn ditransitive_definite_indirect_object() {
    // "gave the winner the prize" — a DEFINITE indirect object carries no
    // quantifier, so it took the definite-object branch which (in both the
    // verb.rs and mod.rs VP parsers) never picked up the direct object. The DO
    // now folds and the double-object builder assigns Recipient(IO) ∧ Theme(DO).
    let proper = ok("John gave the winner the prize.");
    assert!(proper.contains("Recipient(") && proper.contains("Winner") && proper.contains("Theme(") && proper.contains("Prize"),
        "proper-subject definite IO ditransitive; got: {}", proper);
    let def = ok("The teacher gave the student a pencil.");
    assert!(def.contains("Recipient(") && def.contains("Student") && def.contains("Pencil"),
        "definite-subject definite IO ditransitive; got: {}", def);
    // possessive-pronoun IO ("its survivor") in an of-pair disjunct (shared VP)
    let pair = ok("Of the red show and the blue show, one featured Neil and the other gave its survivor the water filter.");
    assert!(pair.contains("Recipient(") && pair.contains("Survivor") && pair.contains("Water_Filter"),
        "of-pair ditransitive with possessive-pronoun IO; got: {}", pair);
    // regression: the dative "to" form keeps the recipient as a PP, not a DO
    let proper_io = ok("John gave Mary the book.");
    assert!(proper_io.contains("Recipient(") && proper_io.contains("Mary") && proper_io.contains("Book"),
        "proper-name IO still works; got: {}", proper_io);
}

#[test]
fn do_support_negation_folds_full_complement() {
    // "does/do/did/won't VERB …" routed through the shared finite-verb builder, so
    // the negated VP folds every complement form — bare-noun object, measure/money,
    // and trailing PPs — exactly as the positive path does. Dropping any of these
    // (the old reduced do-support paths did) is a meaning-loss parse.
    let money = ok("The calculus book doesn't cost $29.99.");
    assert!(money.contains("Cost(") && money.contains("29.99") && money.contains('¬'),
        "negated money complement folds; got: {}", money);
    let noun = ok("Kari's candle doesn't contain clove.");
    assert!(noun.contains("Contain(") && noun.contains("Clove") && noun.contains('¬'),
        "negated bare-noun object folds; got: {}", noun);
    let measure = ok("The quilt doesn't measure 60 inches.");
    assert!(measure.contains("Measure(") && measure.contains("60") && measure.contains('¬'),
        "negated measure complement folds; got: {}", measure);
    let pp_did = ok("Stacey's sighting didn't happen in May.");
    assert!(pp_did.contains("Happen(") && pp_did.contains("In(") && pp_did.contains("May") && pp_did.contains('¬'),
        "negated past PP folds; got: {}", pp_did);
    let pp_will = ok("The flier won't go to Kansas.");
    assert!(pp_will.contains("Go(") && pp_will.contains("To(") && pp_will.contains("Kansas") && pp_will.contains('¬'),
        "negated future PP folds; got: {}", pp_will);
    // regression: NPI under do-support negation is still licensed and scoped
    let npi = ok("John did not see anything.");
    assert!(npi.contains("Thing") && npi.contains("See") && npi.contains('¬'),
        "NPI object preserved under negation; got: {}", npi);
    // regression (butler modus-tollens chain): a pronoun object stays LITERAL and
    // the negated definite subject is NOT given a uniqueness clause — otherwise a
    // proof goal ("the butler did not do it") desyncs from the conditional
    // antecedent it must refute ("if the butler did it …" keeps Theme(e, It)).
    let butler = ok("The butler did not do it.");
    assert!(butler.contains("Theme(e, It)") && !butler.contains("= Butler"),
        "pronoun object literal + no uniqueness wrap under do-support negation; got: {}", butler);
}

#[test]
fn phrasal_particle_then_pp_keeps_both() {
    // "came out in 1995" — the particle "out" precedes a PP, so the clause-final
    // phrasal path can't fire (no boundary; phrasal lookup returns None). The verb
    // VP parser folds the bare particle into a predicate AND the PP loop attaches
    // In(e, ...). Both must survive — dropping either loses meaning.
    let pair = ok("Of the red card and the blue card, one came out in 1995 and the other came out in 2001.");
    assert!(pair.contains("Out(") && pair.contains("In(") && pair.contains("1995") && pair.contains("2001"),
        "particle + PP both survive in of-pair; got: {}", pair);
    // regression: clause-final particle (no PP) still folds into the event scope
    let plain = ok("The plant came out.");
    assert!(plain.contains("Come(") && plain.contains("Out("), "clause-final particle still works; got: {}", plain);
    // regression: lexicalized phrasal verb ("give up" → Surrender) unaffected
    let surr = ok("The team gave up.");
    assert!(surr.contains("Surrender("), "lexicalized phrasal verb preserved; got: {}", surr);
}

// ── Disjunctive copula complement under a universal ──────────────────────────

#[test]
fn universal_disjunctive_complement_two_way() {
    // "Every color is red or blue" — the `or` distributes the bound variable
    // INSIDE the consequent: ∀x(Color(x) → (Red(x) ∨ Blue(x))). It must NOT lift
    // to sentence level (which stranded a bare `Blue` atom not applied to x).
    let out = ok("Every color is red or blue.");
    assert_eq!(out, "∀x((Color(x) → (Red(x) ∨ Blue(x))))", "got: {}", out);
}

#[test]
fn universal_disjunctive_complement_three_way() {
    // The comma-coordinated list "red, blue, or green" is a 3-way disjunction of
    // predicates over the same bound variable — previously a TrailingTokens error.
    let out = ok("Every color is red, blue, or green.");
    assert_eq!(
        out,
        "∀x((Color(x) → ((Red(x) ∨ Blue(x)) ∨ Green(x))))",
        "got: {}", out
    );
}

#[test]
fn universal_single_complement_unchanged() {
    // The single-complement copular universal is untouched by the disjunction path.
    let out = ok("Every man is mortal.");
    assert_eq!(out, "∀x((Man(x) → Mortal(x)))", "got: {}", out);
}
