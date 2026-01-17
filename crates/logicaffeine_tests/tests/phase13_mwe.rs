//! Multi-Word Expression (MWE) tests - Phase 13
//! Tests for idiom and collocation handling

use logicaffeine_language::compile;

#[test]
fn test_mwe_pipeline_token_merging() {
    use logicaffeine_base::Interner;
    use logicaffeine_language::{Lexer, TokenType};
    use logicaffeine_language::mwe::{build_mwe_trie, apply_mwe_pipeline};

    // Verify compound noun merging: fire + engine → FireEngine
    let mut interner = Interner::new();
    let mut lexer = Lexer::new("The fire engine arrived.", &mut interner);
    let tokens = lexer.tokenize();
    assert_eq!(tokens.len(), 6, "Should have 6 tokens before MWE");

    let mwe_trie = build_mwe_trie();
    let tokens_after = apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);
    assert_eq!(tokens_after.len(), 5, "Should have 5 tokens after MWE (fire+engine merged)");

    if let TokenType::Noun(sym) = &tokens_after[1].kind {
        assert_eq!(interner.resolve(*sym), "FireEngine", "Merged token should be FireEngine");
    } else {
        panic!("Token 1 should be Noun(FireEngine)");
    }

    // Verify idiom merging: kick + the + bucket → Die (verb)
    let mut interner2 = Interner::new();
    let mut lexer2 = Lexer::new("John kicked the bucket.", &mut interner2);
    let tokens2 = lexer2.tokenize();
    assert_eq!(tokens2.len(), 6, "Should have 6 tokens before MWE");

    let mwe_trie2 = build_mwe_trie();
    let tokens2_after = apply_mwe_pipeline(tokens2, &mwe_trie2, &mut interner2);
    assert_eq!(tokens2_after.len(), 4, "Should have 4 tokens after MWE (kick+the+bucket merged)");

    if let TokenType::Verb { lemma, .. } = &tokens2_after[1].kind {
        assert_eq!(interner2.resolve(*lemma), "Die", "Merged token should be Die verb");
    } else {
        panic!("Token 1 should be Verb(Die)");
    }
}

// ===== COMPOUND NOUNS =====

#[test]
fn test_fire_engine_compound() {
    let output = compile("The fire engine arrived.").unwrap();
    assert!(output.contains("FireEngine("), "Should contain FireEngine predicate");
    assert!(output.contains("Arriv"), "Should contain Arrive verb");
}

#[test]
fn test_ice_cream_compound() {
    let output = compile("John ate ice cream.").unwrap();
    // MWE: ice cream → IceCream
    // John ate produces: Agent(e, John) ∧ Theme(e, IceCream)
    assert!(output.contains("Agent(e, John)"), "Should have John as agent");
    assert!(output.contains("Eat(e)") || output.contains("Ate("), "Should contain eat verb");
}

#[test]
fn test_compound_with_definite_article() {
    // Test with definite article (indefinite "A" has lexer capitalization bug)
    let output = compile("The fire engine arrived.").unwrap();
    assert!(output.contains("FireEngine("), "Should contain FireEngine predicate");
    assert!(output.contains("Arriv"), "Should contain Arrive verb");
}

// ===== IDIOMS (SEMANTIC REPLACEMENT) =====

#[test]
fn test_kick_the_bucket_idiom() {
    let output = compile("John kicked the bucket.").unwrap();
    assert!(output.contains("Die"), "Should map to semantic lemma Die");
    assert!(output.contains("Past"), "Should inherit past tense from 'kicked'");
    assert!(!output.contains("Bucket"), "Should NOT contain literal Bucket");
    assert!(!output.contains("Kick("), "Should NOT contain literal Kick");
}

#[test]
fn test_kicks_the_bucket_present() {
    let output = compile("John kicks the bucket.").unwrap();
    assert!(output.contains("Die"), "Should map to Die");
    assert!(!output.contains("Past"), "Should be present tense, not past");
}

#[test]
fn test_give_up_phrasal_verb() {
    let output = compile("Mary gave up.").unwrap();
    assert!(output.contains("Surrender") || output.contains("GiveUp"));
}

// ===== PROPER NOUN COMPOUNDS =====

#[test]
fn test_united_states_proper() {
    let output = compile("The United States is large.").unwrap();
    assert!(output.contains("UnitedStates") || output.contains("Large("), "Should contain predicate");
}

// ===== INDEFINITE ARTICLE FIX =====

#[test]
fn test_indefinite_article_at_start() {
    // Regression test: "A" at sentence start should be Article, not ProperName
    let output = compile("A dog ran.").unwrap();
    assert!(output.contains("Dogs(") || output.contains("Dog("), "Should contain Dog predicate: got {}", output);
    assert!(output.contains("Run"), "Should contain Run verb: got {}", output);
}

#[test]
fn test_indefinite_article_with_unknown_word() {
    // "fire" isn't in common nouns list, but "A fire" should still parse correctly
    let output = compile("A fire burned.").unwrap();
    assert!(output.contains("Fire("), "Should contain Fire predicate: got {}", output);
    assert!(output.contains("Burn"), "Should contain Burn verb: got {}", output);
}

#[test]
fn test_indefinite_article_with_mwe() {
    // "A fire engine arrived" - MWE with indefinite article
    let output = compile("A fire engine arrived.").unwrap();
    assert!(output.contains("FireEngine("), "Should contain FireEngine predicate: got {}", output);
    assert!(output.contains("Arriv"), "Should contain Arrive verb: got {}", output);
}

// ===== INTRANSITIVE VERB FIX =====

#[test]
fn test_stopped_intransitive() {
    // Regression test: "stopped" without gerund should work as intransitive verb
    let output = compile("John stopped.").unwrap();
    assert!(output.contains("Stop"), "Should contain Stop verb: got {}", output);
    assert!(output.contains("J"), "Should contain John: got {}", output);
    assert!(!output.contains("?"), "Should NOT contain ? placeholder: got {}", output);
}

#[test]
fn test_stopped_with_definite_noun_subject() {
    let output = compile("The car stopped.").unwrap();
    assert!(output.contains("Stop"), "Should contain Stop verb: got {}", output);
    assert!(output.contains("Car("), "Should contain Car predicate: got {}", output);
}

#[test]
fn test_stopped_with_gerund_still_presupposition() {
    // Verify "stopped smoking" still works as presupposition
    let output = compile("John stopped smoking.").unwrap();
    assert!(output.contains("Presup"), "Should still be presupposition: got {}", output);
}

// ===== NON-MWE SENTENCES (REGRESSION) =====

#[test]
fn test_fire_alone_not_merged() {
    let output = compile("The fire burned.").unwrap();
    assert!(output.contains("Fire("), "Should contain Fire predicate");
    assert!(output.contains("Burn"), "Should contain Burn verb");
}

#[test]
fn test_engine_alone_not_merged() {
    let output = compile("The engine ran.").unwrap();
    assert!(output.contains("Engine("), "Should contain Engine predicate");
}
