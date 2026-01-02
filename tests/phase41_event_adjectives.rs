// Phase 41: Event Modification Adjectives
//
// "Olga is a beautiful dancer" can mean:
// 1. Intersective: Olga is beautiful AND Olga is a dancer
// 2. Event-modifying: Olga dances beautifully (the dancing is beautiful)
//
// The Council ruled: Generate BOTH readings in parse forest.

use logos::compile_forest;

#[test]
fn beautiful_dancer_two_readings() {
    // "Olga is a beautiful dancer" should produce two readings:
    // Reading 1 (Intersective): Beautiful(Olga) ∧ Dancer(Olga)
    // Reading 2 (Event): ∃e((Dance(e) ∧ Agent(e, Olga)) ∧ Beautiful(e))
    let results = compile_forest("Olga is a beautiful dancer.");

    eprintln!("DEBUG beautiful_dancer readings: {:?}", results);

    assert!(results.len() >= 2,
        "Expected at least 2 readings for 'beautiful dancer', got {}: {:?}",
        results.len(), results);

    // Check that one reading has intersective Beautiful(Olga) ∧ Dancer(Olga)
    let has_intersective = results.iter().any(|r|
        r.contains("Beautiful(Olga)") && r.contains("Dancer(Olga)"));
    assert!(has_intersective, "Missing intersective reading: {:?}", results);

    // Check that one reading has event-modifying Dance(e) and Beautiful(e)
    let has_event_mod = results.iter().any(|r|
        r.contains("Dance(e)") && r.contains("Beautiful(e)") && r.contains("∃e"));
    assert!(has_event_mod, "Missing event-modifying reading: {:?}", results);
}

#[test]
fn tall_dancer_only_intersective() {
    // "tall" is a physical/dimensional adjective - cannot modify events
    // Should produce only ONE reading: Tall(O) ∧ Dancer(O)
    let results = compile_forest("Olga is a tall dancer.");

    eprintln!("DEBUG tall_dancer readings: {:?}", results);

    assert_eq!(results.len(), 1,
        "Expected exactly 1 reading for 'tall dancer' (physical adj), got {}: {:?}",
        results.len(), results);
}

#[test]
fn graceful_singer_two_readings() {
    // "graceful" can modify events, "singer" derives from "Sing"
    let results = compile_forest("Maria is a graceful singer.");

    eprintln!("DEBUG graceful_singer readings: {:?}", results);

    assert!(results.len() >= 2,
        "Expected at least 2 readings for 'graceful singer', got {}",
        results.len());
}

#[test]
fn slow_runner_two_readings() {
    // "slow" can modify events (manner), "runner" derives from "Run"
    let results = compile_forest("John is a slow runner.");

    eprintln!("DEBUG slow_runner readings: {:?}", results);

    assert!(results.len() >= 2,
        "Expected at least 2 readings for 'slow runner', got {}",
        results.len());
}

#[test]
fn young_dancer_only_intersective() {
    // "young" is temporal/age-related - cannot modify events
    let results = compile_forest("Olga is a young dancer.");

    eprintln!("DEBUG young_dancer readings: {:?}", results);

    assert_eq!(results.len(), 1,
        "Expected exactly 1 reading for 'young dancer' (age adj), got {}",
        results.len());
}

#[test]
fn skillful_teacher_two_readings() {
    // "skillful" can modify events (manner), "teacher" derives from "Teach"
    let results = compile_forest("Sarah is a skillful teacher.");

    eprintln!("DEBUG skillful_teacher readings: {:?}", results);

    assert!(results.len() >= 2,
        "Expected at least 2 readings for 'skillful teacher', got {}",
        results.len());
}

#[test]
fn regular_adjective_noun_single_reading() {
    // "happy student" - student is not an agentive noun, so only one reading
    let results = compile_forest("John is a happy student.");

    eprintln!("DEBUG happy_student readings: {:?}", results);

    assert_eq!(results.len(), 1,
        "Expected exactly 1 reading for 'happy student' (non-agentive noun), got {}",
        results.len());
}

#[test]
fn event_reading_structure() {
    // Verify the event reading has correct structure:
    // ∃e((Dance(e) ∧ Agent(e, Olga)) ∧ Beautiful(e))
    let results = compile_forest("Olga is a beautiful dancer.");

    // Find the event reading (contains existential and event variable)
    let event_reading = results.iter().find(|r| r.contains("∃e") && r.contains("Dance(e)"));
    assert!(event_reading.is_some(), "Should have event reading with Dance(e): {:?}", results);

    let reading = event_reading.unwrap();
    assert!(reading.contains("Agent(e"), "Event reading should have Agent role: {}", reading);
}
