use logicaffeine_language::compile_forest;

#[test]
fn lexical_ambiguity_duck() {
    // "I saw her duck."
    // Reading 1: "duck" is Noun (the bird). See(I, Duck)
    // Reading 2: "duck" is Verb (the action). See(I, [Duck]) (perception complement)
    let results = compile_forest("I saw her duck.");

    assert!(
        results.len() >= 2,
        "Should produce multiple readings for 'duck' (Noun/Verb). Got {} readings: {:?}",
        results.len(),
        results
    );
}

#[test]
fn structural_ambiguity_pp_attachment() {
    // "I saw the man with the telescope."
    // 1. Saw using telescope (Instrument)
    // 2. Man having telescope (Modifier)
    let results = compile_forest("I saw the man with the telescope.");
    assert!(
        results.len() >= 2,
        "Should detect PP attachment ambiguity. Got {} readings: {:?}",
        results.len(),
        results
    );
}

#[test]
fn pp_attachment_both_readings_include_pp() {
    // Both readings must include the PP - telescope should appear in BOTH valid readings
    let results = compile_forest("I saw the man with the telescope.");

    // Filter to only complete readings (containing logical structure, not just constants like "Speaker")
    let complete_readings: Vec<_> = results.iter()
        .filter(|r| r.contains("See") || r.contains("With"))
        .collect();

    assert_eq!(complete_readings.len(), 2, "Expected 2 complete readings, got {}: {:?}", complete_readings.len(), complete_readings);

    // Both complete readings should include the telescope
    for (i, reading) in complete_readings.iter().enumerate() {
        assert!(
            reading.contains("Telescope") || reading.contains("telescope"),
            "Reading {} missing PP (no Telescope): {}", i + 1, reading
        );
    }

    // One should be VP-attachment: With(e, ...)
    let has_vp = complete_readings.iter().any(|r| r.contains("With(e"));
    // One should be NP-attachment: With(Man, ...) or With(x, ...)
    let has_np = complete_readings.iter().any(|r|
        r.contains("With(Man") || r.contains("With(x") || r.contains("With(y")
    );

    assert!(has_vp, "Missing VP-attachment reading (With(e, ...)).\nReadings: {:?}", complete_readings);
    assert!(has_np, "Missing NP-attachment reading (With(Man/x, ...)).\nReadings: {:?}", complete_readings);
}

#[test]
fn unambiguous_sentence_single_reading() {
    // "John runs." - unambiguous, should be exactly 1 reading
    let results = compile_forest("John runs.");
    assert_eq!(
        results.len(),
        1,
        "Unambiguous sentence should have exactly 1 parse. Got: {:?}",
        results
    );
}

#[test]
fn ambiguous_bear_noun_verb() {
    // "The bear" - "bear" can be animal (Noun) or carry (Verb)
    // In subject position after "The", should resolve to Noun (single reading)
    let results = compile_forest("The bear sleeps.");
    assert_eq!(
        results.len(),
        1,
        "Bear in subject position should resolve to Noun. Got: {:?}",
        results
    );
}

#[test]
fn ambiguous_love_preserved() {
    // "Love is patient." - Love as Noun (abstract concept)
    // "I love you." - Love as Verb (action)
    // Both should parse correctly
    let noun_results = compile_forest("Love is patient.");
    let verb_results = compile_forest("I love you.");

    assert!(
        !noun_results.is_empty(),
        "Love as noun subject should parse"
    );
    assert!(
        !verb_results.is_empty(),
        "Love as verb should parse"
    );
}

#[test]
fn classic_time_flies() {
    // "Time flies like an arrow."
    // Classic AI ambiguity test
    // 1. Time passes quickly (like an arrow does)
    // 2. Time-flies (insect species) are fond of arrows
    // 3. Measure flies in the manner of an arrow
    let results = compile_forest("Time flies like an arrow.");
    assert!(
        results.len() >= 1,
        "Should produce at least one reading for classic ambiguity. Got: {:?}",
        results
    );
}

#[test]
fn forest_preserves_all_valid_parses() {
    // "Flying planes can be dangerous."
    // 1. Planes that are flying (gerund modifier)
    // 2. The act of flying planes (gerund subject)
    let results = compile_forest("Flying planes can be dangerous.");
    assert!(
        results.len() >= 1,
        "Should produce readings for structural ambiguity. Got: {:?}",
        results
    );
}
