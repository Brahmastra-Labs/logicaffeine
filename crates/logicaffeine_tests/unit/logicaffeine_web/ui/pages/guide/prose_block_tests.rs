use super::{fence, prose_blocks, Fence, ProseBlock};
use crate::ui::components::code_editor::Language;

fn html(s: &str) -> ProseBlock {
    ProseBlock::Html(s.to_string())
}

fn code(tag: &str, code: &str) -> ProseBlock {
    ProseBlock::Code { tag: tag.to_string(), code: code.to_string() }
}

/// A fence is its own block, between the prose before and after it — its lines exactly as
/// written, blank lines and indentation kept.
#[test]
fn a_fence_is_its_own_block_between_the_prose_around_it() {
    let blocks = prose_blocks("Before.\n\n```logos\n## Main\n    Show 1.\n\n    Show 2.\n```\n\nAfter.");
    assert_eq!(
        blocks,
        vec![html("<p>Before.</p>"), code("logos", "## Main\n    Show 1.\n\n    Show 2."), html("<p>After.</p>")]
    );
}

#[test]
fn an_untagged_fence_names_no_language() {
    assert_eq!(prose_blocks("```\nShow 1.\n```"), vec![code("", "Show 1.")]);
}

#[test]
fn the_tag_is_read_without_surrounding_space() {
    assert_eq!(prose_blocks("```  bash \nls\n```"), vec![code("bash", "ls")]);
}

#[test]
fn back_to_back_fences_are_two_blocks() {
    assert_eq!(
        prose_blocks("```logic\nProve: A.\n```\n```bash\nlargo prove a.lg\n```"),
        vec![code("logic", "Prove: A."), code("bash", "largo prove a.lg")]
    );
}

#[test]
fn an_unclosed_fence_runs_to_the_end() {
    assert_eq!(prose_blocks("Run:\n```bash\nlargo run"), vec![html("<p>Run:</p>"), code("bash", "largo run")]);
}

/// A list, a table or a quote open before a fence ends where the fence begins — none of
/// them may swallow the code or spill past it.
#[test]
fn a_fence_closes_whatever_prose_was_open() {
    assert_eq!(prose_blocks("- one\n```bash\nls\n```"), vec![html("<ul><li>one</li></ul>"), code("bash", "ls")]);
    assert_eq!(
        prose_blocks("| a |\n|---|\n| 1 |\n```bash\nls\n```"),
        vec![html("<div class=\"table-wrapper\"><table><thead><tr><th>a</th></tr></thead><tbody><tr><td>1</td></tr></tbody></table></div>"), code("bash", "ls")]
    );
    assert_eq!(
        prose_blocks("> a quote\n```bash\nls\n```\nAfter."),
        vec![html("<blockquote>a quote</blockquote>"), code("bash", "ls"), html("<p>After.</p>")]
    );
}

#[test]
fn prose_without_a_fence_is_one_block() {
    assert_eq!(prose_blocks("One.\n\nTwo."), vec![html("<p>One.</p><p>Two.</p>")]);
}

#[test]
fn fence_tags_name_how_the_page_shows_them() {
    assert_eq!(fence("logos"), Some(Fence::Logos));
    assert_eq!(fence("logic"), Some(Fence::Logic));
    for plain in ["bash", "powershell", "toml"] {
        assert_eq!(fence(plain), Some(Fence::Plain), "{plain}");
    }
    for unknown in ["", "logso", "LOGOS", "rust"] {
        assert_eq!(fence(unknown), None, "{unknown:?}");
    }
}

#[test]
fn logos_and_logic_fences_are_read_in_their_editor_language() {
    assert!(Fence::Logos.language() == Some(Language::Logos), "a logos fence is not read as LOGOS code");
    assert!(Fence::Logic.language() == Some(Language::Logic), "a logic fence is not read as Logic");
    assert!(Fence::Plain.language().is_none(), "plain text is read in an editor language");
}
