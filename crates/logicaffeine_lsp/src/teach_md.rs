//! Markdown rendering for the shared teaching brain
//! (`logicaffeine_language::teach`) — the LSP's presentation of a lesson.
//!
//! One renderer feeds hover AND completion documentation, so the two can
//! never phrase a lesson differently; the terminal REPL renders the same
//! lessons in ANSI. The quickguide link goes through [`guide_url`] — one
//! function to change if the guide ever moves off the repository.

use tower_lsp::lsp_types::{Documentation, MarkupContent, MarkupKind};

use logicaffeine_language::teach::ConstructDoc;

/// The canonical "read more" URL for a quickguide heading slug.
pub fn guide_url(anchor: &str) -> String {
    format!(
        "https://github.com/Brahmastra-Labs/logicaffeine/blob/main/LOGOS_QUICKGUIDE.md#{anchor}"
    )
}

/// The full lesson as hover markdown: name, one plain sentence, the runnable
/// example, the socratic question or tip, and the guide link when one fits.
pub fn keyword_hover_md(doc: &ConstructDoc) -> String {
    let mut md = format!(
        "**{}**\n\n{}\n\n```\n{}\n```\n\n{}",
        doc.name, doc.what, doc.example, doc.question_or_tip
    );
    push_guide_link(&mut md, doc);
    md
}

/// The lesson for a `##` block header, keeping the Block Header banner.
pub fn block_hover_md(doc: &ConstructDoc) -> String {
    let mut md = format!(
        "**Block Header** — {}\n\n```\n{}\n```\n\n{}",
        doc.what, doc.example, doc.question_or_tip
    );
    push_guide_link(&mut md, doc);
    md
}

/// The lesson as completion-item documentation (same rendering as hover —
/// the editor's docs panel and the hover card always agree).
pub fn completion_docs(doc: &ConstructDoc) -> Documentation {
    Documentation::MarkupContent(MarkupContent {
        kind: MarkupKind::Markdown,
        value: keyword_hover_md(doc),
    })
}

fn push_guide_link(md: &mut String, doc: &ConstructDoc) {
    if let Some(anchor) = doc.guide_anchor {
        md.push_str(&format!("\n\n[Quick Guide]({})", guide_url(anchor)));
    }
}

#[cfg(test)]
mod tests {
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
        assert!(!md.contains("Quick Guide"), "Escape has no guide section: {md}");
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
}
