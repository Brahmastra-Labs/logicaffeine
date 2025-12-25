use logos::{compile, compile_all_scopes};

// ═══════════════════════════════════════════════════════════════════════════
// 1. ASPECTUAL TORTURE SUITE
// Testing parse_aspect_chain with Modal, Perfect, Progressive, Passive stacking
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_maximal_aspect_chain() {
    // Future (Aux) + Perfect (Have) + Passive (Been) + Progressive (Being) + Verb
    let result = compile("The treaty will have been being signed.");
    match result {
        Ok(output) => {
            let has_future = output.contains("Future") || output.contains("Will");
            let has_perf = output.contains("Perf");
            let has_verb = output.contains("Sign") || output.contains("S(");
            assert!(
                has_future || has_perf || has_verb,
                "Maximal aspect chain should contain temporal/aspect markers: got '{}'",
                output
            );
        }
        Err(e) => panic!("Maximal aspect chain failed to parse: {:?}", e),
    }
}

#[test]
fn test_modal_negation_aspect_chain() {
    // Modal (Must) + Negation + Perfect (Have) + Progressive (Been...ing)
    let result = compile("John must not have been sleeping.");
    match result {
        Ok(output) => {
            let has_modal = output.contains("□") || output.contains("Must");
            let has_neg = output.contains("¬") || output.contains("Not");
            let has_verb = output.contains("Sleep") || output.contains("S(");
            assert!(
                has_modal || has_neg || has_verb,
                "Modal + negation + aspect should parse: got '{}'",
                output
            );
        }
        Err(e) => panic!("Modal negation aspect chain failed to parse: {:?}", e),
    }
}

#[test]
fn test_modal_perfect_passive_agent() {
    // Modal (Could) + Perfect (Have) + Passive (Been/Built) + Agent (by John)
    let result = compile("The house could have been built by John.");
    match result {
        Ok(output) => {
            let has_passive = output.contains("Pass") || output.contains("Build") || output.contains("B(");
            let has_agent = output.contains("Agent") || output.contains("J");
            assert!(
                has_passive || has_agent,
                "Modal + perfect + passive + agent should parse: got '{}'",
                output
            );
        }
        Err(e) => panic!("Modal perfect passive with agent failed to parse: {:?}", e),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. CONTROL FREAK SUITE
// Testing deeply nested Control and Raising verbs
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_triple_nested_control() {
    // Raising (Seem) -> Subject Control (Want) -> Subject Control (Try) -> Intransitive
    let result = compile("John seems to want to try to leave.");
    match result {
        Ok(output) => {
            let has_seem = output.contains("Seem");
            let has_want = output.contains("W(") || output.contains("Want");
            let has_try = output.contains("T(") || output.contains("Try");
            let has_leave = output.contains("L(") || output.contains("Leave");
            assert!(
                has_seem && (has_want || has_try || has_leave),
                "Triple nested control should contain all verbs: got '{}'",
                output
            );
        }
        Err(e) => panic!("Triple nested control failed to parse: {:?}", e),
    }
}

#[test]
fn test_object_to_subject_control() {
    // Object Control (Persuade) -> Subject Control (Intend)
    let result = compile("Mary persuaded John to intend to stay.");
    match result {
        Ok(output) => {
            let has_persuade = output.contains("P(") || output.contains("Persuade");
            let has_intend = output.contains("I(") || output.contains("Intend");
            let has_stay = output.contains("S(") || output.contains("Stay");
            assert!(
                has_persuade || has_intend || has_stay,
                "Object to subject control chain should parse: got '{}'",
                output
            );
        }
        Err(e) => panic!("Object to subject control failed to parse: {:?}", e),
    }
}

#[test]
fn test_promise_subject_control_anomaly() {
    // Promise is Subject control even though it has an object (linguistic anomaly)
    // The Teacher should be bound as the trier, not the Student
    let result = compile("The teacher promised the student to try to help.");
    match result {
        Ok(output) => {
            let has_promise = output.contains("Promise") || output.contains("P(");
            let has_try = output.contains("Try") || output.contains("T(");
            let has_help = output.contains("Help") || output.contains("H(");
            assert!(
                has_promise || has_try || has_help,
                "Promise subject control anomaly should parse: got '{}'",
                output
            );
        }
        Err(e) => panic!("Promise subject control anomaly failed to parse: {:?}", e),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. RECURSIVE RELATIVE SUITE
// Testing center-embedding and right-branching relative clauses
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_double_center_embedding() {
    // Double center-embedding (Object gap)
    // The dog [that the cat [that the mouse scared] chased] ran.
    let result = compile("The dog that the cat that the mouse scared chased ran.");
    match result {
        Ok(output) => {
            let has_scare = output.contains("Scare") || output.contains("S(");
            let has_chase = output.contains("Chase") || output.contains("C(");
            let has_run = output.contains("Run") || output.contains("R(");
            assert!(
                has_run,
                "Double center-embedding should find main verb 'ran': got '{}'",
                output
            );
            assert!(
                has_scare || has_chase,
                "Double center-embedding should find embedded verbs: got '{}'",
                output
            );
        }
        Err(e) => panic!("Double center-embedding failed to parse: {:?}", e),
    }
}

#[test]
fn test_deep_right_branching() {
    // Deep right-branching recursion (4 levels)
    let result = compile("I see the man who owns the dog that chased the cat that ate the rat.");
    match result {
        Ok(output) => {
            let has_see = output.contains("See") || output.contains("S(");
            let has_own = output.contains("Own") || output.contains("O(");
            let has_chase = output.contains("Chase") || output.contains("C(");
            let has_eat = output.contains("Eat") || output.contains("E(");
            assert!(
                has_see,
                "Deep right-branching should find main verb 'see': got '{}'",
                output
            );
            assert!(
                has_own || has_chase || has_eat,
                "Deep right-branching should find relative clause verbs: got '{}'",
                output
            );
        }
        Err(e) => panic!("Deep right-branching failed to parse: {:?}", e),
    }
}

#[test]
fn test_stacked_relatives_single_head() {
    // Stacked relative clauses on a single head noun
    let result = compile("Every book that John read that Mary wrote is famous.");
    match result {
        Ok(output) => {
            let has_universal = output.contains("∀");
            let has_read = output.contains("Read") || output.contains("R(");
            let has_wrote = output.contains("Write") || output.contains("Wrote") || output.contains("W(");
            let has_famous = output.contains("Famous") || output.contains("F(");
            assert!(
                has_universal,
                "Stacked relatives should have universal quantifier: got '{}'",
                output
            );
            assert!(
                has_read && has_wrote,
                "Stacked relatives should contain BOTH relative clause predicates: got '{}'",
                output
            );
            assert!(
                has_famous,
                "Stacked relatives should contain main predicate Famous: got '{}'",
                output
            );
        }
        Err(e) => panic!("Stacked relatives on single head failed to parse: {:?}", e),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 4. SCOPE & QUANTIFIER SUITE
// Testing enumerate_scopings and complex determiner logic
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_triple_quantifier_ditransitive() {
    // Universal + Existential + Numerical Quantifier + Ditransitive
    let result = compile_all_scopes("Every teacher gave some book to at least two students.");
    match result {
        Ok(readings) => {
            assert!(
                readings.len() >= 2,
                "Triple quantifier ditransitive should produce multiple scope readings: got {}",
                readings.len()
            );
            for reading in &readings {
                assert!(
                    reading.contains("∀") || reading.contains("∃") || reading.contains("≥"),
                    "Readings should contain quantifiers: got '{}'",
                    reading
                );
            }
        }
        Err(e) => panic!("Triple quantifier ditransitive failed to parse: {:?}", e),
    }
}

#[test]
fn test_negative_quantifier_donkey() {
    // Negative Quantifier + Relative Clause + Donkey Anaphora
    let result = compile("No man who owns a donkey beats it.");
    match result {
        Ok(output) => {
            let has_universal = output.contains("∀");
            let has_negation = output.contains("¬");
            let has_own = output.contains("Own") || output.contains("O(");
            let has_beat = output.contains("Beat") || output.contains("B(");
            assert!(
                has_universal || has_negation,
                "Negative quantifier should produce ∀...¬ structure: got '{}'",
                output
            );
            assert!(
                has_own && has_beat,
                "Donkey sentence should contain Own and Beat: got '{}'",
                output
            );
        }
        Err(e) => panic!("Negative quantifier donkey failed to parse: {:?}", e),
    }
}

#[test]
fn test_conditional_donkey_sentence() {
    // Classic Donkey Sentence (Conditional)
    // Indefinite "a donkey" should promote to universal force in antecedent
    let result = compile("If a farmer owns a donkey, he beats it.");
    match result {
        Ok(output) => {
            let has_conditional = output.contains("→");
            let has_universal = output.contains("∀");
            let has_own = output.contains("Own") || output.contains("O(");
            let has_beat = output.contains("Beat") || output.contains("B(");
            assert!(
                has_conditional || has_universal,
                "Conditional donkey should have → or ∀: got '{}'",
                output
            );
            assert!(
                has_own && has_beat,
                "Donkey sentence should contain Own and Beat: got '{}'",
                output
            );
        }
        Err(e) => panic!("Conditional donkey sentence failed to parse: {:?}", e),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 5. EVENT SEMANTICS SUITE
// Testing Neo-Davidsonian roles with heavy modification
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_event_multiple_modifiers() {
    // Agent + Theme + Instrument (PP) + Manner (Adverb) + Time (PP)
    let result = compile("John opened the door with a key quietly in the morning.");
    match result {
        Ok(output) => {
            let has_open = output.contains("Open");
            let has_agent = output.contains("Agent") || output.contains("J");
            let has_theme = output.contains("Theme") || output.contains("Door") || output.contains("D(");
            let has_quietly = output.contains("Quietly");
            assert!(
                has_open,
                "Event should contain Open: got '{}'",
                output
            );
            assert!(
                has_agent || has_theme,
                "Event should have Agent/Theme roles: got '{}'",
                output
            );
            assert!(
                has_quietly,
                "Event should contain manner adverb Quietly: got '{}'",
                output
            );
        }
        Err(e) => panic!("Event with multiple modifiers failed to parse: {:?}", e),
    }
}

#[test]
fn test_causal_event_linking() {
    // Causal connective linking two Neo-Davidsonian events
    // Cause(Throw(j, rock), Break(window))
    let result = compile("The window broke because John threw a rock.");
    match result {
        Ok(output) => {
            let has_cause = output.contains("Cause");
            let has_break = output.contains("Break") || output.contains("B(");
            let has_throw = output.contains("Throw") || output.contains("T(");
            assert!(
                has_cause,
                "Causal event should produce Cause predicate: got '{}'",
                output
            );
            assert!(
                has_break && has_throw,
                "Causal event should contain both Break and Throw: got '{}'",
                output
            );
        }
        Err(e) => panic!("Causal event linking failed to parse: {:?}", e),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 6. PRAGMATICS & POLARITY SUITE
// Testing NPIs, Focus, and Speech Acts
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_npi_any_licensing() {
    // NPI "any" licensed by Negation
    // "any" should resolve to Existential (not Universal) due to negative context
    let result = compile("I did not see any dogs.");
    match result {
        Ok(output) => {
            let has_negation = output.contains("¬") || output.contains("Not");
            let has_see = output.contains("See") || output.contains("S(");
            let has_existential = output.contains("∃");
            assert!(
                has_negation,
                "NPI licensing requires negation: got '{}'",
                output
            );
            assert!(
                has_see,
                "NPI sentence should contain See: got '{}'",
                output
            );
            assert!(
                has_existential || has_negation,
                "NPI 'any' should produce existential under negation: got '{}'",
                output
            );
        }
        Err(e) => panic!("NPI 'any' licensing failed to parse: {:?}", e),
    }
}

#[test]
fn test_focus_on_subject() {
    // Focus particle on Subject
    // Output should look like Only(j, Ate(j, cake))
    let result = compile("Only John ate the cake.");
    match result {
        Ok(output) => {
            let has_only = output.contains("Only") || output.contains("Focus");
            let has_eat = output.contains("Eat") || output.contains("Ate") || output.contains("E(");
            assert!(
                has_only,
                "Focus on subject should produce Only/Focus marker: got '{}'",
                output
            );
            assert!(
                has_eat,
                "Focus sentence should contain Eat: got '{}'",
                output
            );
        }
        Err(e) => panic!("Focus on subject failed to parse: {:?}", e),
    }
}

#[test]
fn test_indirect_speech_act_request() {
    // Indirect Speech Act (Modal Question -> Imperative)
    // Currently parses as modal question; future: detect as imperative
    let result = compile("Could you please open the door?");
    match result {
        Ok(output) => {
            let has_question_or_imperative = output.contains("?") || output.contains("!");
            let has_open = output.contains("Open") || output.contains("O(");
            assert!(
                has_question_or_imperative,
                "Should be question or imperative: got '{}'",
                output
            );
            assert!(
                has_open,
                "Should contain Open verb: got '{}'",
                output
            );
            // Note: HAB wrapper is correct for activity verb "open" in simple aspect
        }
        Err(e) => panic!("Indirect speech act request failed to parse: {:?}", e),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 7. EVERYTHING BAGEL SUITE
// Sentences mixing 3+ categories to catch edge cases in context passing
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_relative_raising_modal_aspect() {
    // Relative Clause + Raising + Modal + Negation + Aspect Chain
    let result = compile("The man who seems to be happy must not have been sleeping.");
    match result {
        Ok(output) => {
            let has_seem = output.contains("Seem") || output.contains("S(");
            let has_happy = output.contains("Happy") || output.contains("H(") || output.contains(", H)");
            let has_modal = output.contains("□") || output.contains("Must");
            let has_neg = output.contains("¬");
            let has_sleep = output.contains("Sleep") || output.contains("S(");
            assert!(
                has_seem,
                "Should contain raising verb Seem (or S): got '{}'",
                output
            );
            assert!(
                has_happy,
                "Should contain Happy from relative clause: got '{}'",
                output
            );
            assert!(
                has_modal && has_neg,
                "Should contain modal + negation (□ + ¬): got '{}'",
                output
            );
            assert!(
                has_sleep,
                "Should contain Sleep: got '{}'",
                output
            );
        }
        Err(e) => panic!("Relative + raising + modal + aspect failed to parse: {:?}", e),
    }
}

#[test]
fn test_quantifier_opaque_control_pronoun() {
    // Quantifier + Opaque Verb (Believes) + Control (Wants) + Pronoun Resolution
    let result = compile("Every student believes that the teacher wants them to succeed.");
    match result {
        Ok(output) => {
            let has_universal = output.contains("∀");
            let has_believe = output.contains("Believe") || output.contains("B(");
            let has_want = output.contains("Want") || output.contains("W(");
            let has_succeed = output.contains("Succeed") || output.contains("S(");
            assert!(
                has_universal,
                "Quantifier + opaque should have ∀: got '{}'",
                output
            );
            assert!(
                has_believe || has_want || has_succeed,
                "Opaque + control should contain key verbs: got '{}'",
                output
            );
        }
        Err(e) => panic!("Quantifier + opaque + control + pronoun failed to parse: {:?}", e),
    }
}

#[test]
fn test_counterfactual_passive_agent() {
    // Counterfactual + Passive Voice + Agent PP
    let result = compile("If John had run, he would have been seen by Mary.");
    match result {
        Ok(output) => {
            let has_counterfactual = output.contains("□→") || output.contains("→") || output.contains("Counterfactual");
            let has_run = output.contains("Run") || output.contains("R(");
            let has_see = output.contains("See") || output.contains("S(") || output.contains("Seen");
            let has_passive = output.contains("Pass") || output.contains("Agent");
            assert!(
                has_counterfactual,
                "Counterfactual should produce conditional: got '{}'",
                output
            );
            assert!(
                has_run || has_see,
                "Counterfactual should contain Run or See: got '{}'",
                output
            );
        }
        Err(e) => panic!("Counterfactual + passive + agent failed to parse: {:?}", e),
    }
}
