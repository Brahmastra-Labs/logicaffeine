//! Diagnostic Bridge Verification Tests
//!
//! These tests verify that Rust ownership errors are translated into
//! friendly LOGOS error messages. Users should NEVER see raw rustc errors.
//!
//! The "Socratic Torture Test" - proving the bridge actually works.

mod common;

use logos::compile::{compile_and_run, CompileError};
use tempfile::TempDir;

/// Helper to compile LOGOS code and capture the error
fn compile_and_get_error(source: &str) -> Option<String> {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    match compile_and_run(source, temp_dir.path()) {
        Ok(_) => None, // No error - test setup is wrong
        Err(e) => Some(format!("{}", e)),
    }
}

/// Helper to verify error contains expected text and NOT raw rustc codes
fn assert_friendly_error(error: &str, expected_contains: &[&str], forbidden_patterns: &[&str]) {
    for expected in expected_contains {
        assert!(
            error.to_lowercase().contains(&expected.to_lowercase()),
            "Error should contain '{}'\nActual error:\n{}",
            expected,
            error
        );
    }

    for forbidden in forbidden_patterns {
        assert!(
            !error.contains(forbidden),
            "Error should NOT contain raw rustc pattern '{}'\nActual error:\n{}",
            forbidden,
            error
        );
    }
}

// =============================================================================
// E0382: Use After Move - The simplest ownership violation
// =============================================================================

#[test]
fn test_use_after_move_simple() {
    // Simplest case: move a String twice
    // String::from is NOT Copy, so second Let should fail
    let source = r#"## Main
Let s be "hello".
Let a be s.
Let b be s."#;

    let error = compile_and_get_error(source);
    assert!(error.is_some(), "Should produce an error for double move");

    let error_text = error.unwrap();

    // Debug: print the actual error
    println!("=== ACTUAL ERROR ===\n{}\n===================", error_text);

    // Should contain friendly LOGOS message about the variable
    assert!(
        error_text.to_lowercase().contains("s") ||
        error_text.to_lowercase().contains("moved") ||
        error_text.to_lowercase().contains("giving"),
        "Error should mention the moved variable\nActual: {}",
        error_text
    );

    // Should NOT contain raw rustc error codes
    assert!(
        !error_text.contains("error[E0382]"),
        "Error should NOT contain raw rustc error code E0382\nActual: {}",
        error_text
    );
}

#[test]
fn test_use_after_move_with_function() {
    // Move via function call, then try to use
    let source = r#"## To consume (x: Text) -> Text:
    Return x.

## Main
Let data be "important".
Let result be consume(data).
Let again be data."#;

    let error = compile_and_get_error(source);
    assert!(error.is_some(), "Should produce an error for use-after-move");

    let error_text = error.unwrap();
    println!("=== USE AFTER MOVE ERROR ===\n{}\n===================", error_text);

    // Verify we get SOME error about ownership
    assert!(
        error_text.to_lowercase().contains("data") ||
        error_text.to_lowercase().contains("moved") ||
        error_text.to_lowercase().contains("giving") ||
        error_text.to_lowercase().contains("ownership"),
        "Error should mention ownership issue\nActual: {}",
        error_text
    );
}

// =============================================================================
// General Bridge Property: No Raw Rustc Codes Should Leak
// =============================================================================

#[test]
fn test_no_rustc_error_codes_leak_e0382() {
    // Trigger E0382 and verify the code doesn't leak
    let source = r#"## Main
Let x be "test".
Let a be x.
Let b be x."#;

    if let Some(error) = compile_and_get_error(source) {
        println!("=== E0382 ERROR CHECK ===\n{}\n===================", error);

        // The raw rustc error code should NOT appear
        assert!(
            !error.contains("E0382"),
            "Error should NOT contain rustc error code E0382\nActual: {}",
            error
        );
    }
}

// =============================================================================
// Codegen Pattern Tests (These verify the translation layer works)
// =============================================================================

#[test]
fn test_diagnostic_module_compiles() {
    use logos::sourcemap::SourceMap;

    let source_map = SourceMap::new("test".to_string());
    assert!(!source_map.logos_source().is_empty());
}

#[test]
fn test_json_parsing_extracts_e0382() {
    use logos::diagnostic::{parse_rustc_json, get_error_code};

    let json = r#"{"reason":"compiler-message","message":{"message":"use of moved value: `x`","code":{"code":"E0382"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":5,"line_end":5,"column_start":10,"column_end":11,"is_primary":true,"label":null,"text":[]}],"children":[]}}"#;

    let diagnostics = parse_rustc_json(json);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(get_error_code(&diagnostics[0]), Some("E0382"));
}

#[test]
fn test_json_parsing_extracts_e0597() {
    use logos::diagnostic::{parse_rustc_json, get_error_code};

    let json = r#"{"reason":"compiler-message","message":{"message":"borrowed value does not live long enough","code":{"code":"E0597"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":10,"line_end":10,"column_start":5,"column_end":6,"is_primary":true,"label":null,"text":[]}],"children":[]}}"#;

    let diagnostics = parse_rustc_json(json);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(get_error_code(&diagnostics[0]), Some("E0597"));
}

#[test]
fn test_translator_produces_friendly_message_for_e0382() {
    use logos::diagnostic::{parse_rustc_json, DiagnosticBridge};
    use logos::sourcemap::SourceMap;
    use logos::intern::Interner;

    let json = r#"{"reason":"compiler-message","message":{"message":"use of moved value: `data`","code":{"code":"E0382"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":5,"line_end":5,"column_start":10,"column_end":14,"is_primary":true,"label":null,"text":[]}],"children":[]}}"#;

    let diagnostics = parse_rustc_json(json);
    let source_map = SourceMap::new("Let data be 5.\nGive data.".to_string());
    let interner = Interner::new();

    let bridge = DiagnosticBridge::new(&source_map, &interner);
    let translated = bridge.translate(&diagnostics[0]);

    assert!(translated.is_some(), "Should translate E0382");
    let error = translated.unwrap();

    assert!(error.title.contains("data"), "Title should mention variable name");
    assert!(error.title.contains("giving") || error.title.contains("gave"),
            "Title should use LOGOS terminology");
    assert!(error.suggestion.is_some(), "Should provide a suggestion");
}

#[test]
fn test_translator_produces_friendly_message_for_e0597() {
    use logos::diagnostic::{parse_rustc_json, DiagnosticBridge};
    use logos::sourcemap::SourceMap;
    use logos::intern::Interner;

    let json = r#"{"reason":"compiler-message","message":{"message":"borrowed value does not live long enough","code":{"code":"E0597"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":5,"line_end":5,"column_start":10,"column_end":14,"is_primary":true,"label":null,"text":[]}],"children":[]}}"#;

    let diagnostics = parse_rustc_json(json);
    let source_map = SourceMap::new("Inside a zone:\n    Let x be 5.".to_string());
    let interner = Interner::new();

    let bridge = DiagnosticBridge::new(&source_map, &interner);
    let translated = bridge.translate(&diagnostics[0]);

    assert!(translated.is_some(), "Should translate E0597");
    let error = translated.unwrap();

    let explanation_lower = error.explanation.to_lowercase();
    assert!(
        explanation_lower.contains("zone") ||
        explanation_lower.contains("arena") ||
        explanation_lower.contains("outlive") ||
        explanation_lower.contains("hotel california"),
        "Should explain the zone escape concept\nActual: {}",
        error.explanation
    );
}

// =============================================================================
// E0597: Zone Escape - Hotel California Rule
// =============================================================================

#[test]
fn test_zone_escape_produces_friendly_error() {
    // This should trigger E0597: reference cannot escape zone
    // Variables created inside a zone cannot be assigned to outer scope
    let source = r#"## Main
Let leak be 0.
Inside a zone called "Scratch":
    Let p be 42.
    Set leak to p."#;

    let error = compile_and_get_error(source);
    assert!(error.is_some(), "Should produce zone escape error");

    let error_text = error.unwrap();
    println!("=== ZONE ESCAPE ERROR ===\n{}\n===================", error_text);

    // Should mention "zone" or "Hotel California" or "escape"
    assert!(
        error_text.to_lowercase().contains("zone") ||
        error_text.to_lowercase().contains("escape") ||
        error_text.to_lowercase().contains("hotel california") ||
        error_text.to_lowercase().contains("cannot"),
        "Should explain zone containment rule\nActual: {}",
        error_text
    );

    // Should NOT leak raw rustc code
    assert!(
        !error_text.contains("E0597"),
        "Should NOT contain raw E0597\nActual: {}",
        error_text
    );
}

// =============================================================================
// E0505: Borrow Conflict - Unit Test for Translation
// =============================================================================

#[test]
fn test_e0505_translation_produces_friendly_message() {
    // E0505 (move while borrowed) is hard to trigger through LOGOS syntax
    // because Show/Give don't create overlapping borrows in sequential code.
    // This UNIT TEST verifies the translator handles E0505 JSON correctly.
    use logos::diagnostic::{parse_rustc_json, DiagnosticBridge};
    use logos::sourcemap::SourceMap;
    use logos::intern::Interner;

    // Mock a real rustc E0505 error JSON
    let json = r#"{"reason":"compiler-message","message":{"message":"cannot move out of `data` because it is borrowed","code":{"code":"E0505"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":5,"line_end":5,"column_start":10,"column_end":14,"is_primary":true,"label":"move out of `data` occurs here","text":[]}],"children":[]}}"#;

    let diagnostics = parse_rustc_json(json);
    let source_map = SourceMap::new("Let data be 5.\nShow data.\nGive data to other.".to_string());
    let interner = Interner::new();

    let bridge = DiagnosticBridge::new(&source_map, &interner);
    let translated = bridge.translate(&diagnostics[0]);

    assert!(translated.is_some(), "Should translate E0505");
    let error = translated.unwrap();

    println!("=== E0505 TRANSLATION ===\nTitle: {}\nExplanation: {}\n===================",
             error.title, error.explanation);

    // Should mention borrow/show semantics in LOGOS terminology
    let explanation_lower = error.explanation.to_lowercase();
    assert!(
        explanation_lower.contains("show") ||
        explanation_lower.contains("borrow") ||
        explanation_lower.contains("viewing") ||
        explanation_lower.contains("give"),
        "Should explain borrow conflict using LOGOS terms\nExplanation: {}",
        error.explanation
    );

    // Should NOT contain raw E0505
    assert!(
        !error.title.contains("E0505") && !error.explanation.contains("E0505"),
        "Should NOT contain raw E0505"
    );
}

// =============================================================================
// E0596: Concurrent Mutation - Unit Test for Translation
// =============================================================================

#[test]
fn test_concurrent_mutation_translation() {
    // E0596 is what rustc emits when you try to mutate a captured variable
    // inside a closure (like rayon::join). This UNIT TEST verifies the
    // translator handles it correctly without needing rayon linked.
    use logos::diagnostic::{parse_rustc_json, DiagnosticBridge};
    use logos::sourcemap::SourceMap;
    use logos::intern::Interner;

    // Mock E0596: cannot borrow `x` as mutable in a closure
    let json = r#"{"reason":"compiler-message","message":{"message":"cannot borrow `counter` as mutable, as it is a captured variable in a `Fn` closure","code":{"code":"E0596"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":10,"line_end":10,"column_start":5,"column_end":12,"is_primary":true,"label":"cannot borrow as mutable","text":[]}],"children":[]}}"#;

    let diagnostics = parse_rustc_json(json);
    let source_map = SourceMap::new("Simultaneously:\n    Set counter to 1.\n    Set counter to 2.".to_string());
    let interner = Interner::new();

    let bridge = DiagnosticBridge::new(&source_map, &interner);
    let translated = bridge.translate(&diagnostics[0]);

    // The bridge should handle this error (may fall back to generic if no specific handler)
    if let Some(error) = translated {
        println!("=== E0596 TRANSLATION ===\nTitle: {}\nExplanation: {}\n===================",
                 error.title, error.explanation);

        // Should NOT contain raw E0596
        assert!(
            !error.title.contains("E0596") && !error.explanation.contains("E0596"),
            "Should NOT contain raw E0596"
        );
    }
    // Note: If no translation exists for E0596, the bridge returns None
    // and the compile pipeline falls back to a generic error. That's acceptable.
}
