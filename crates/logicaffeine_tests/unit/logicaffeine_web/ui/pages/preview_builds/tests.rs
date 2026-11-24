use super::*;

#[test]
fn a_free_worker_gets_the_source_at_once() {
    let mut queue = BuildQueue::default();
    assert_eq!(queue.ask("a".to_string(), 1), Some("a".to_string()));
}

#[test]
fn a_busy_worker_is_sent_only_the_latest_source_when_it_finishes() {
    let mut queue = BuildQueue::default();
    queue.ask("a".to_string(), 1);
    assert_eq!(queue.ask("b".to_string(), 2), None);
    assert_eq!(queue.ask("c".to_string(), 3), None);
    assert_eq!(queue.finish(), (Some(1), Some("c".to_string())), "b was typed over before its turn");
    assert_eq!(queue.finish(), (Some(3), None));
    assert_eq!(queue.finish(), (None, None), "nothing is left to finish");
}

#[test]
fn a_source_already_asked_for_is_not_built_twice() {
    let mut queue = BuildQueue::default();
    queue.ask("a".to_string(), 1);
    assert_eq!(queue.ask("a".to_string(), 2), None);
    queue.ask("b".to_string(), 3);
    assert_eq!(queue.ask("b".to_string(), 4), None);
    assert_eq!(queue.finish(), (Some(1), Some("b".to_string())));
    assert_eq!(queue.finish(), (Some(3), None));
}

#[test]
fn a_lost_worker_hands_back_every_asker() {
    let mut queue = BuildQueue::default();
    queue.ask("a".to_string(), 1);
    queue.ask("b".to_string(), 2);
    assert_eq!(queue.abandon(), vec![1, 2]);
    assert_eq!(queue.ask("c".to_string(), 3), Some("c".to_string()), "the queue is empty again");
}

#[test]
fn the_kept_documents_never_grow_past_their_bound() {
    let mut built = HashMap::new();
    for i in 0..(KEPT_DOCUMENTS * 4) {
        remember(&mut built, format!("source {i}"), format!("doc {i}"));
        assert!(built.len() <= KEPT_DOCUMENTS, "{} documents kept", built.len());
    }
    assert_eq!(built.get(&format!("source {}", KEPT_DOCUMENTS * 4 - 1)), Some(&format!("doc {}", KEPT_DOCUMENTS * 4 - 1)), "the newest build is kept");
}

#[test]
fn building_the_same_source_again_keeps_every_other_document() {
    let mut built = HashMap::new();
    for i in 0..KEPT_DOCUMENTS {
        remember(&mut built, format!("source {i}"), "doc".to_string());
    }
    remember(&mut built, "source 0".to_string(), "new".to_string());
    assert_eq!(built.len(), KEPT_DOCUMENTS);
    assert_eq!(built.get("source 0").map(String::as_str), Some("new"));
}
