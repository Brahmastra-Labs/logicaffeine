//! Markdown rendering for the shared teaching brain
//! (`logicaffeine_language::teach`) — the LSP's presentation of a lesson.
//!
//! One renderer feeds hover AND completion documentation, so the two can
//! never phrase a lesson differently; the terminal REPL renders the same
//! lessons in ANSI. The quickguide link goes through [`guide_url`] — one
//! function to change if the guide ever moves off the repository.

use lsp_types::{Documentation, MarkupContent, MarkupKind};

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
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/teach_md/tests.rs"]
mod tests;
