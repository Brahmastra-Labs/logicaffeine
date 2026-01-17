//! Phase CRDT Variants: Tests for Set Bias, YATA, and ORMap
//!
//! Wave 5 completion: Full language support for all CRDT variants.

mod common;

use logicaffeine_compile::compile::compile_to_rust;
use common::{assert_output, assert_runs};

// =============================================================================
// Set Bias Tests
// =============================================================================

#[test]
fn test_shared_set_default_addwins_codegen() {
    let source = r#"## Definition
A Tags is Shared and has:
    a labels, which is a SharedSet of Text.

## Main
Let t be a new Tags.
Show "done"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logicaffeine_data::crdt::ORSet<String>"),
        "Default SharedSet should emit ORSet. Got:\n{}",
        rust
    );
}

#[test]
fn test_shared_set_explicit_addwins_codegen() {
    let source = r#"## Definition
A Tags is Shared and has:
    a labels, which is a SharedSet (AddWins) of Text.

## Main
Let t be a new Tags.
Show "done"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logicaffeine_data::crdt::ORSet<String, logicaffeine_data::crdt::AddWins>"),
        "Explicit AddWins should emit ORSet with AddWins. Got:\n{}",
        rust
    );
}

#[test]
fn test_shared_set_removewins_codegen() {
    let source = r#"## Definition
A Blacklist is Shared and has:
    a blocked, which is a SharedSet (RemoveWins) of Text.

## Main
Let bl be a new Blacklist.
Show "done"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logicaffeine_data::crdt::ORSet<String, logicaffeine_data::crdt::RemoveWins>"),
        "RemoveWins should emit ORSet with RemoveWins. Got:\n{}",
        rust
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_shared_set_addwins_e2e() {
    assert_output(
        r#"## Definition
A Party is Shared and has:
    a guests, which is a SharedSet (AddWins) of Text.

## Main
Let mutable p be a new Party.
Add "Alice" to p's guests.
Add "Bob" to p's guests.
Show length of p's guests."#,
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_shared_set_removewins_e2e() {
    assert_runs(
        r#"## Definition
A Blocklist is Shared and has:
    a blocked, which is a SharedSet (RemoveWins) of Text.

## Main
Let mutable bl be a new Blocklist.
Add "spam@test.com" to bl's blocked.
Remove "spam@test.com" from bl's blocked.
Show "done"."#,
    );
}

// =============================================================================
// YATA/CollaborativeSequence Tests
// =============================================================================

#[test]
fn test_collaborative_sequence_codegen() {
    let source = r#"## Definition
A Document is Shared and has:
    a text, which is a CollaborativeSequence of Text.

## Main
Let d be a new Document.
Show "done"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logicaffeine_data::crdt::YATA<String>"),
        "CollaborativeSequence should emit YATA. Got:\n{}",
        rust
    );
}

#[test]
fn test_shared_sequence_yata_modifier_codegen() {
    let source = r#"## Definition
A Editor is Shared and has:
    a lines, which is a SharedSequence (YATA) of Text.

## Main
Let e be a new Editor.
Show "done"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logicaffeine_data::crdt::YATA<String>"),
        "SharedSequence (YATA) should emit YATA. Got:\n{}",
        rust
    );
}

#[test]
fn test_shared_sequence_default_rga_codegen() {
    let source = r#"## Definition
A Log is Shared and has:
    an entries, which is a SharedSequence of Text.

## Main
Let l be a new Log.
Show "done"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logicaffeine_data::crdt::RGA<String>"),
        "Default SharedSequence should emit RGA. Got:\n{}",
        rust
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_collaborative_sequence_e2e() {
    assert_output(
        r#"## Definition
A Editor is Shared and has:
    a chars, which is a CollaborativeSequence of Text.

## Main
Let mutable e be a new Editor.
Append "H" to e's chars.
Append "i" to e's chars.
Show length of e's chars."#,
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_shared_sequence_yata_e2e() {
    assert_output(
        r#"## Definition
A Doc is Shared and has:
    a lines, which is a SharedSequence (YATA) of Text.

## Main
Let mutable d be a new Doc.
Append "Line 1" to d's lines.
Append "Line 2" to d's lines.
Show length of d's lines."#,
        "2",
    );
}

// =============================================================================
// SharedMap/ORMap Tests
// =============================================================================

#[test]
fn test_shared_map_codegen() {
    let source = r#"## Definition
A Cache is Shared and has:
    an entries, which is a SharedMap from Text to Int.

## Main
Let c be a new Cache.
Show "done"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logicaffeine_data::crdt::ORMap<String, i64>"),
        "SharedMap should emit ORMap. Got:\n{}",
        rust
    );
}

#[test]
fn test_ormap_alias_codegen() {
    let source = r#"## Definition
A Store is Shared and has:
    a data, which is a ORMap from Text to Bool.

## Main
Let s be a new Store.
Show "done"."#;

    let rust = compile_to_rust(source).expect("Should compile");
    assert!(
        rust.contains("logicaffeine_data::crdt::ORMap<String, bool>"),
        "ORMap should emit ORMap. Got:\n{}",
        rust
    );
}

// =============================================================================
// Mixed CRDT Struct Tests
// =============================================================================

#[test]
fn test_all_crdt_types_in_struct_codegen() {
    let source = r#"## Definition
A GameState is Shared and has:
    a score, which is a Tally.
    a players, which is a SharedSet (AddWins) of Text.
    a blockedPlayers, which is a SharedSet (RemoveWins) of Text.
    a chatHistory, which is a SharedSequence of Text.
    a document, which is a CollaborativeSequence of Text.
    a settings, which is a SharedMap from Text to Int.
    a currentMode, which is a Divergent Text.

## Main
Let g be a new GameState.
Show "done"."#;

    let rust = compile_to_rust(source).expect("Should compile");

    // Verify all types are generated correctly
    assert!(rust.contains("logicaffeine_data::crdt::PNCounter"), "Should have PNCounter for Tally");
    assert!(rust.contains("logicaffeine_data::crdt::ORSet<String, logicaffeine_data::crdt::AddWins>"), "Should have AddWins ORSet");
    assert!(rust.contains("logicaffeine_data::crdt::ORSet<String, logicaffeine_data::crdt::RemoveWins>"), "Should have RemoveWins ORSet");
    assert!(rust.contains("logicaffeine_data::crdt::RGA<String>"), "Should have RGA");
    assert!(rust.contains("logicaffeine_data::crdt::YATA<String>"), "Should have YATA");
    assert!(rust.contains("logicaffeine_data::crdt::ORMap<String, i64>"), "Should have ORMap");
    assert!(rust.contains("logicaffeine_data::crdt::MVRegister<String>"), "Should have MVRegister");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn test_mixed_crdt_operations_e2e() {
    assert_output(
        r#"## Definition
A App is Shared and has:
    a counter, which is a Tally.
    a tags, which is a SharedSet (AddWins) of Text.
    a history, which is a CollaborativeSequence of Text.

## Main
Let mutable app be a new App.
Increase app's counter by 100.
Decrease app's counter by 30.
Add "active" to app's tags.
Append "Started" to app's history.
Show app's counter."#,
        "70",
    );
}
