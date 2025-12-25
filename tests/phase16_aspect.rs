use logos::compile;

#[test]
fn test_perfect_progressive_active() {
    let input = "John has been eating apples.";
    let result = compile(input).expect("Should compile");

    assert!(result.contains("Perf"), "Should be Perfect: got {}", result);
    assert!(result.contains("Prog"), "Should be Progressive: got {}", result);
    assert!(!result.contains("Pass"), "Should NOT be Passive: got {}", result);
}

#[test]
fn test_perfect_passive() {
    let input = "The apple has been eaten.";
    let result = compile(input).expect("Should compile");

    assert!(result.contains("Perf"), "Should be Perfect: got {}", result);
    assert!(result.contains("Pass"), "Should be Passive: got {}", result);
    assert!(!result.contains("Prog"), "Should NOT be Progressive: got {}", result);
    assert!(!result.contains("Agent"), "Passive subject should NOT be Agent: got {}", result);
}

#[test]
fn test_perfect_copular_state() {
    let input = "John has been happy.";
    let result = compile(input).expect("Should compile");

    assert!(result.contains("Perf"), "Should be Perfect: got {}", result);
    assert!(!result.contains("Pass"), "Copular state should NOT be Passive: got {}", result);
    assert!(!result.contains("Prog"), "Stative should NOT be Progressive: got {}", result);
}

#[test]
fn test_modal_perfect_progressive() {
    let input = "John should have been working.";
    let result = compile(input).expect("Should compile");

    assert!(result.contains("Prog"), "Should be Progressive: got {}", result);
    assert!(!result.contains("Pass"), "Should NOT be Passive: got {}", result);
}
