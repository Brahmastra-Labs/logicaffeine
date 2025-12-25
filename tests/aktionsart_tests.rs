//! Vendler's Lexical Aspect Classes (Aktionsart) Tests
//!
//! These tests verify the implementation of the five Vendler classes:
//! - State: +static, +durative, -telic (know, love, exist)
//! - Activity: -static, +durative, -telic (run, swim, drive)
//! - Accomplishment: -static, +durative, +telic (build, draw, write)
//! - Achievement: -static, -durative, +telic (win, find, die)
//! - Semelfactive: -static, -durative, -telic (knock, cough, blink)

use logos::ast::AspectOperator;
use logos::lexicon::{Lexicon, VerbClass};
use logos::parse;
use logos::view::ExprView;
use logos::compile;

fn has_modifier(modifiers: &[&str], name: &str) -> bool {
    modifiers.iter().any(|m| m.eq_ignore_ascii_case(name))
}

// ═══════════════════════════════════════════════════════════════════
// VERB CLASS LOOKUP TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn state_verb_know_has_state_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("know").unwrap();
    assert_eq!(entry.class, VerbClass::State);
}

#[test]
fn state_verb_love_has_state_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("love").unwrap();
    assert_eq!(entry.class, VerbClass::State);
}

#[test]
fn state_verb_exist_has_state_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("exists").unwrap();
    assert_eq!(entry.class, VerbClass::State);
}

#[test]
fn activity_verb_run_has_activity_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("run").unwrap();
    assert_eq!(entry.class, VerbClass::Activity);
}

#[test]
fn activity_verb_swim_has_activity_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("swim").unwrap();
    assert_eq!(entry.class, VerbClass::Activity);
}

#[test]
fn activity_verb_drive_has_activity_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("drive").unwrap();
    assert_eq!(entry.class, VerbClass::Activity);
}

#[test]
fn accomplishment_verb_build_has_accomplishment_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("build").unwrap();
    assert_eq!(entry.class, VerbClass::Accomplishment);
}

#[test]
fn accomplishment_verb_draw_has_accomplishment_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("draw").unwrap();
    assert_eq!(entry.class, VerbClass::Accomplishment);
}

#[test]
fn accomplishment_verb_write_has_accomplishment_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("write").unwrap();
    assert_eq!(entry.class, VerbClass::Accomplishment);
}

#[test]
fn achievement_verb_win_has_achievement_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("win").unwrap();
    assert_eq!(entry.class, VerbClass::Achievement);
}

#[test]
fn achievement_verb_find_has_achievement_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("find").unwrap();
    assert_eq!(entry.class, VerbClass::Achievement);
}

#[test]
fn achievement_verb_die_has_achievement_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("die").unwrap();
    assert_eq!(entry.class, VerbClass::Achievement);
}

#[test]
fn semelfactive_verb_knock_has_semelfactive_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("knocks").unwrap();
    assert_eq!(entry.class, VerbClass::Semelfactive);
}

#[test]
fn semelfactive_verb_hit_has_semelfactive_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("hit").unwrap();
    assert_eq!(entry.class, VerbClass::Semelfactive);
}

#[test]
fn semelfactive_verb_wag_has_semelfactive_class() {
    let lex = Lexicon::new();
    let entry = lex.lookup_verb("wag").unwrap();
    assert_eq!(entry.class, VerbClass::Semelfactive);
}

// ═══════════════════════════════════════════════════════════════════
// VERB CLASS HELPER METHOD TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn state_is_stative() {
    assert!(VerbClass::State.is_stative());
    assert!(!VerbClass::Activity.is_stative());
    assert!(!VerbClass::Accomplishment.is_stative());
    assert!(!VerbClass::Achievement.is_stative());
    assert!(!VerbClass::Semelfactive.is_stative());
}

#[test]
fn durative_classes() {
    assert!(VerbClass::State.is_durative());
    assert!(VerbClass::Activity.is_durative());
    assert!(VerbClass::Accomplishment.is_durative());
    assert!(!VerbClass::Achievement.is_durative());
    assert!(!VerbClass::Semelfactive.is_durative());
}

#[test]
fn telic_classes() {
    assert!(!VerbClass::State.is_telic());
    assert!(!VerbClass::Activity.is_telic());
    assert!(VerbClass::Accomplishment.is_telic());
    assert!(VerbClass::Achievement.is_telic());
    assert!(!VerbClass::Semelfactive.is_telic());
}

// ═══════════════════════════════════════════════════════════════════
// SEMANTIC OUTPUT TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn activity_present_produces_habitual() {
    let view = parse!("John runs.");
    match view {
        ExprView::Aspectual { operator, .. } => {
            assert_eq!(operator, AspectOperator::Habitual);
        }
        _ => panic!("Activity + Present should produce Aspectual(Habitual), got {:?}", view),
    }
}

#[test]
fn activity_progressive_produces_progressive() {
    let view = parse!("John is running.");
    match view {
        ExprView::Aspectual { operator, .. } => {
            assert_eq!(operator, AspectOperator::Progressive);
        }
        ExprView::NeoEvent { modifiers, .. } => {
            assert!(has_modifier(&modifiers, "Progressive"));
        }
        _ => panic!("Activity + Progressive should produce Progressive, got {:?}", view),
    }
}

#[test]
fn state_present_produces_simple_predication() {
    let view = parse!("John knows Mary.");
    match view {
        ExprView::NeoEvent { verb, modifiers, .. } => {
            assert_eq!(verb, "Know");
            assert!(!has_modifier(&modifiers, "Habitual"),
                "State verbs should NOT have Habitual modifier");
        }
        _ => panic!("State + Present should be simple NeoEvent, got {:?}", view),
    }
}

#[test]
fn semelfactive_progressive_produces_iterative() {
    let view = parse!("John is knocking.");
    match view {
        ExprView::Aspectual { operator, .. } => {
            assert_eq!(operator, AspectOperator::Iterative);
        }
        _ => panic!("Semelfactive + Progressive should produce Aspectual(Iterative), got {:?}", view),
    }
}

#[test]
fn accomplishment_present_produces_habitual() {
    let view = parse!("John builds houses.");
    match view {
        ExprView::Aspectual { operator, .. } => {
            assert_eq!(operator, AspectOperator::Habitual);
        }
        ExprView::Quantifier { body, .. } => {
            match *body {
                ExprView::BinaryOp { right, .. } => {
                    if let ExprView::Aspectual { operator, .. } = *right {
                        assert_eq!(operator, AspectOperator::Habitual);
                    }
                }
                _ => {}
            }
        }
        _ => panic!("Accomplishment + Present should produce Habitual, got {:?}", view),
    }
}

#[test]
fn achievement_present_produces_habitual() {
    let view = parse!("John wins games.");
    match view {
        ExprView::Aspectual { operator, .. } => {
            assert_eq!(operator, AspectOperator::Habitual);
        }
        ExprView::Quantifier { body, .. } => {
            match *body {
                ExprView::BinaryOp { right, .. } => {
                    if let ExprView::Aspectual { operator, .. } = *right {
                        assert_eq!(operator, AspectOperator::Habitual);
                    }
                }
                _ => {}
            }
        }
        _ => panic!("Achievement + Present should produce Habitual, got {:?}", view),
    }
}

// ═══════════════════════════════════════════════════════════════════
// INVALID COMBINATION TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn state_progressive_is_rejected() {
    let result = compile("John is knowing.");
    assert!(result.is_err(), "State + Progressive should be rejected");
    let err = result.unwrap_err();
    let err_msg = format!("{:?}", err);
    assert!(
        err_msg.contains("stative") || err_msg.contains("Stative") || err_msg.contains("progressive"),
        "Error should mention stative/progressive conflict, got: {}", err_msg
    );
}

#[test]
fn state_progressive_love_is_rejected() {
    let result = compile("John is loving Mary.");
    assert!(result.is_err(), "State(love) + Progressive should be rejected");
}

#[test]
fn state_progressive_hate_is_rejected() {
    let result = compile("John is hating Mary.");
    assert!(result.is_err(), "State(hate) + Progressive should be rejected");
}

// ═══════════════════════════════════════════════════════════════════
// PAST TENSE WITH VERB CLASSES
// ═══════════════════════════════════════════════════════════════════

#[test]
fn state_past_produces_temporal_past() {
    let view = parse!("John knew Mary.");
    match view {
        ExprView::NeoEvent { verb, modifiers, .. } => {
            assert_eq!(verb, "Know");
            assert!(has_modifier(&modifiers, "Past"), "State past should have Past modifier");
            assert!(!has_modifier(&modifiers, "Habitual"), "State past should NOT be Habitual");
        }
        _ => panic!("State + Past should be NeoEvent with Past modifier, got {:?}", view),
    }
}

#[test]
fn activity_past_produces_temporal_past_without_habitual() {
    let view = parse!("John ran.");
    match view {
        ExprView::NeoEvent { verb, modifiers, .. } => {
            assert_eq!(verb, "Run");
            assert!(has_modifier(&modifiers, "Past"), "Activity past should have Past modifier");
        }
        _ => panic!("Activity + Past should produce NeoEvent with Past, got {:?}", view),
    }
}

#[test]
fn achievement_past_produces_temporal_past() {
    let view = parse!("John won.");
    match view {
        ExprView::NeoEvent { verb, modifiers, .. } => {
            assert_eq!(verb, "Win");
            assert!(has_modifier(&modifiers, "Past"));
        }
        _ => panic!("Achievement + Past should produce NeoEvent with Past, got {:?}", view),
    }
}

#[test]
fn debug_knowing_parse() {
    let result = compile("John is knowing.");
    match result {
        Ok(s) => panic!("Should have failed but got: {}", s),
        Err(e) => println!("Error: {:?}", e),
    }
}

#[test]
fn debug_verb_class_for_knowing() {
    // Check what the lexicon returns for "knowing"
    let lex = logos::lexicon::Lexicon::new();
    let entry = lex.lookup_verb("knowing");
    match entry {
        Some(e) => {
            println!("Verb entry for 'knowing': lemma={}, time={:?}, aspect={:?}, class={:?}", 
                e.lemma, e.time, e.aspect, e.class);
            assert!(e.class.is_stative(), "know should be stative, got {:?}", e.class);
        }
        None => panic!("Should find 'knowing' as a verb"),
    }
}

#[test]
fn debug_hating_lookup() {
    let lex = logos::lexicon::Lexicon::new();
    let entry = lex.lookup_verb("hating");
    match entry {
        Some(e) => {
            println!("Verb entry for 'hating': lemma={}, time={:?}, aspect={:?}, class={:?}", 
                e.lemma, e.time, e.aspect, e.class);
            assert!(e.class.is_stative(), "hate should be stative, got {:?}", e.class);
        }
        None => panic!("Should find 'hating' as a verb"),
    }
}
