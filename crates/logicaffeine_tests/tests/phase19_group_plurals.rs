use logicaffeine_language::compile_forest;

#[test]
fn test_cardinal_mixed_verb_two_readings() {
    let input = "Two boys lifted a rock.";
    let readings = compile_forest(input);

    assert!(
        readings.len() >= 2,
        "Cardinal + mixed verb should produce 2 readings, got: {:?}",
        readings
    );

    // Distributive: standard cardinal quantifier
    let has_distributive = readings.iter().any(|r| r.contains("∃=2"));

    // Collective: group existential with Member/Count
    let has_collective = readings
        .iter()
        .any(|r| r.contains("Group") && r.contains("Count") && r.contains("Member"));

    assert!(has_distributive, "Should have distributive reading");
    assert!(has_collective, "Should have collective group reading");
}

#[test]
fn test_cardinal_collective_verb_group_reading() {
    // "gather" is strictly collective - should get group reading
    let input = "Three students gathered.";
    let readings = compile_forest(input);

    assert!(
        readings.iter().any(|r| r.contains("Group")),
        "Collective verb should produce group reading, got: {:?}",
        readings
    );
}

#[test]
fn test_cardinal_distributive_verb_single_reading() {
    // "sleep" is strictly distributive - no group reading
    let input = "Two cats slept.";
    let readings = compile_forest(input);

    assert_eq!(
        readings.len(),
        1,
        "Distributive verb should have 1 reading, got: {:?}",
        readings
    );
    assert!(
        readings[0].contains("∃=2"),
        "Should use cardinal quantifier, got: {}",
        readings[0]
    );
    assert!(
        !readings[0].contains("Group"),
        "Should NOT have group reading, got: {}",
        readings[0]
    );
}

#[test]
fn test_cardinal_group_structure() {
    // Verify the structure of the group quantifier output
    let input = "Two boys lifted a piano.";
    let readings = compile_forest(input);

    let group_reading = readings
        .iter()
        .find(|r| r.contains("Group"))
        .expect("Should have a group reading");

    // Should have Group(g)
    assert!(
        group_reading.contains("Group("),
        "Should have Group predicate: {}",
        group_reading
    );

    // Should have Count(g, 2)
    assert!(
        group_reading.contains("Count(") && group_reading.contains("2"),
        "Should have Count predicate with count: {}",
        group_reading
    );

    // Should have Member constraint
    assert!(
        group_reading.contains("Member("),
        "Should have Member predicate: {}",
        group_reading
    );
}
