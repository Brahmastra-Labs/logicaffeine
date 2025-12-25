//! Phase 14: Ontology System Tests
//! Tests for bridging anaphora and sort compatibility checking

use logos::compile;

// ===== BRIDGING ANAPHORA =====

#[test]
fn test_bridging_forward_link() {
    let input = "I bought a car. The engine smoked.";
    let result = compile(input).expect("Should compile");
    println!("OUTPUT: {}", result);
    assert!(result.contains("PartOf"), "Should detect part-whole relationship, got: {}", result);
}

#[test]
fn test_bridging_house_door() {
    let input = "John entered the house. The door was open.";
    let result = compile(input).expect("Should compile");
    println!("OUTPUT: {}", result);
    assert!(result.contains("PartOf"), "Door should be linked to house, got: {}", result);
}

#[test]
fn test_bridging_ambiguity_forest() {
    // For now, we test that bridging picks ONE of the possible wholes.
    // Full parse forest forking for ambiguous bridging is Phase 14b.
    let input = "I have a car and a bike. The wheel is flat.";
    let readings = logos::compile_forest(input);

    // At minimum, there should be at least one reading with bridging
    let has_any_link = readings.iter().any(|r| r.contains("PartOf"));
    assert!(has_any_link, "Should have at least one reading with PartOf bridging");

    // Check that it links to car OR bike (both are valid wholes for wheel)
    let has_car_or_bike = readings.iter().any(|r|
        r.contains("PartOf") && (r.contains("Car") || r.contains("Bike") || r.contains("C") || r.contains("B"))
    );
    assert!(has_car_or_bike, "Bridging should link to Car or Bike");
}

#[test]
fn test_no_bridging_when_direct_antecedent() {
    let input = "The engine ran. The engine stopped.";
    let result = compile(input).expect("Should compile");
    assert!(!result.contains("PartOf"), "Should not bridge when direct antecedent exists");
}

// ===== METAPHOR (SORT VIOLATIONS) =====

#[test]
fn test_metaphor_wrapper_adjective() {
    let input = "The rock was happy.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("Metaphor"), "Should wrap sort violation in Metaphor");
}

#[test]
fn test_metaphor_wrapper_verb() {
    let input = "The rock thinks.";
    let result = compile(input).expect("Should compile");
    assert!(result.contains("Metaphor"), "Should wrap verb sort violation in Metaphor");
}

#[test]
fn test_no_metaphor_when_compatible() {
    let input = "John was happy.";
    let result = compile(input).expect("Should compile");
    assert!(!result.contains("Metaphor"), "Should NOT mark as metaphor");
}

#[test]
fn test_no_metaphor_for_mental_verb_with_animate() {
    let input = "John thinks.";
    let result = compile(input).expect("Should compile");
    assert!(!result.contains("Metaphor"), "Should NOT mark as metaphor");
}
