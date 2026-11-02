#![cfg(feature = "web-tests")]
//! The Learn roadmap's journey over the real curriculum.
//!
//! `learn_state`'s unit tests pin the rules on toy roadmaps; these walk the curriculum the page
//! ships: every lesson in order, every module's link, and the journey kept in `UserProgress`
//! through a save and a load, as the browser keeps it.

use logicaffeine_web::content::ContentEngine;
use logicaffeine_web::learn_state::Journey;
use logicaffeine_web::progress::UserProgress;

/// Every lesson id, in roadmap order.
fn roadmap() -> Vec<String> {
    ContentEngine::shared()
        .eras()
        .iter()
        .flat_map(|e| e.modules.iter())
        .flat_map(|m| m.sections.iter().map(|s| s.id.clone()))
        .collect()
}

#[test]
fn the_roadmap_has_lessons() {
    assert!(
        roadmap().len() >= 70,
        "only {} lessons on the roadmap",
        roadmap().len()
    );
}

#[test]
fn walking_the_whole_roadmap_finishes_every_lesson_in_order() {
    let order = roadmap();
    let mut journey = Journey::new(order.clone(), Vec::new(), None);
    let mut walked = Vec::new();
    loop {
        walked.push(journey.open().expect("a lesson is open").to_string());
        journey.finish();
        if !journey.advance() {
            break;
        }
    }
    assert_eq!(walked, order);
    assert!(journey.is_finished());
}

#[test]
fn every_module_link_opens_that_module_at_its_first_lesson() {
    let order = roadmap();
    for module in ContentEngine::shared()
        .eras()
        .iter()
        .flat_map(|e| e.modules.iter())
    {
        let ids: Vec<&str> = module.sections.iter().map(|s| s.id.as_str()).collect();
        let mut journey = Journey::new(order.clone(), Vec::new(), None);
        assert!(
            journey.open_first_unfinished(&ids),
            "/learn#{} opens nothing",
            module.meta.id
        );
        assert_eq!(
            journey.open(),
            ids.first().copied(),
            "/learn#{}",
            module.meta.id
        );
    }
}

#[test]
fn the_journey_survives_a_save_and_a_load() {
    let order = roadmap();
    let mut journey = Journey::new(order.clone(), Vec::new(), None);
    for _ in 0..3 {
        journey.finish();
        journey.advance();
    }
    let mut progress = UserProgress::new();
    progress.lessons = journey.done_ids().map(str::to_string).collect();
    progress.open_lesson = journey.open().map(str::to_string);

    let saved = serde_json::to_string(&progress).unwrap();
    let back: UserProgress = serde_json::from_str(&saved).unwrap();
    let again = Journey::new(
        order.clone(),
        back.lessons.iter().cloned(),
        back.open_lesson.as_deref(),
    );
    assert_eq!(
        again.open(),
        Some(order[3].as_str()),
        "the reload opens the lesson left open"
    );
    assert_eq!(again.done_count(), 3);
    for id in &order[..3] {
        assert!(
            again.is_done(id),
            "lesson {id} is no longer done after the reload"
        );
    }
}
