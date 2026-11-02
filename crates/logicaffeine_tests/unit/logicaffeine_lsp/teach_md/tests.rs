use super::*;
use logicaffeine_language::teach::{doc_for, doc_for_block};
use logicaffeine_language::token::{BlockType, TokenType};

#[test]
fn keyword_rendering_carries_every_lesson_part() {
    let lesson = doc_for(&TokenType::Give).unwrap();
    let md = keyword_hover_md(lesson);
    assert!(md.contains("**Give**"), "{md}");
    assert!(md.contains(lesson.what), "{md}");
    assert!(md.contains(lesson.example), "{md}");
    assert!(md.contains(lesson.question_or_tip), "{md}");
    assert!(md.contains("LOGOS_QUICKGUIDE.md#13-output"), "{md}");
}

#[test]
fn block_rendering_keeps_the_banner() {
    let lesson = doc_for_block(&BlockType::Main);
    let md = block_hover_md(lesson);
    assert!(md.starts_with("**Block Header** — "), "{md}");
    assert!(md.contains(lesson.question_or_tip), "{md}");
}

#[test]
fn lessons_without_anchors_render_without_links() {
    let lesson = doc_for(&TokenType::Escape).unwrap();
    let md = keyword_hover_md(lesson);
    assert!(
        !md.contains("Quick Guide"),
        "Escape has no guide section: {md}"
    );
}

#[test]
fn completion_docs_match_hover_exactly() {
    let lesson = doc_for(&TokenType::Let).unwrap();
    let Documentation::MarkupContent(content) = completion_docs(lesson) else {
        panic!("expected markup docs");
    };
    assert_eq!(content.kind, MarkupKind::Markdown);
    assert_eq!(content.value, keyword_hover_md(lesson));
}
