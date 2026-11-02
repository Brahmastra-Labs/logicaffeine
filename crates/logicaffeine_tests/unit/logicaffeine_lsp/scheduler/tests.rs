use super::*;

fn uri() -> Url {
    Url::parse("file:///t.lg").unwrap()
}

fn full_change(text: &str) -> TextDocumentContentChangeEvent {
    TextDocumentContentChangeEvent {
        range: None,
        range_length: None,
        text: text.to_string(),
    }
}

#[test]
fn open_starts_at_generation_zero() {
    let scheduler = Scheduler::new();
    assert_eq!(scheduler.open(uri(), "a".into(), 1), 0);
    assert!(scheduler.is_current(&uri(), 0));
}

#[test]
fn every_change_bumps_the_generation() {
    let scheduler = Scheduler::new();
    scheduler.open(uri(), "a".into(), 1);
    assert_eq!(
        scheduler.apply_changes(&uri(), vec![full_change("b")], 2),
        Some(1)
    );
    assert_eq!(
        scheduler.apply_changes(&uri(), vec![full_change("c")], 3),
        Some(2)
    );
    assert!(
        !scheduler.is_current(&uri(), 1),
        "older generations are stale"
    );
    assert!(scheduler.is_current(&uri(), 2));
}

#[test]
fn current_if_returns_text_only_for_the_live_generation() {
    let scheduler = Scheduler::new();
    scheduler.open(uri(), "a".into(), 1);
    let generation = scheduler
        .apply_changes(&uri(), vec![full_change("newest")], 2)
        .unwrap();
    assert_eq!(
        scheduler.current_if(&uri(), generation),
        Some(("newest".into(), 2))
    );
    assert_eq!(scheduler.current_if(&uri(), generation - 1), None);
}

#[test]
fn changes_to_unopened_documents_are_ignored() {
    let scheduler = Scheduler::new();
    assert_eq!(
        scheduler.apply_changes(&uri(), vec![full_change("x")], 1),
        None
    );
}

#[test]
fn multiple_changes_apply_in_order() {
    let scheduler = Scheduler::new();
    scheduler.open(uri(), "Let x be 5.".into(), 1);
    let generation = scheduler
        .apply_changes(
            &uri(),
            vec![full_change("Let y be 6."), full_change("Let z be 7.")],
            2,
        )
        .unwrap();
    assert_eq!(
        scheduler.current_if(&uri(), generation),
        Some(("Let z be 7.".into(), 2))
    );
}

#[test]
fn close_forgets_the_document() {
    let scheduler = Scheduler::new();
    scheduler.open(uri(), "a".into(), 1);
    scheduler.close(&uri());
    assert!(!scheduler.is_current(&uri(), 0));
}
