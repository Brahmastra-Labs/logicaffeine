//! A page's first paint, rendered in process: the markup `largo build` writes into the document it
//! ships, and the markup a painted preview carries into its frame, both for the page's own mount to
//! adopt rather than draw again.
use super::*;

const COUNTER: &str = "## Page /\nState:\n\x20   Let count be 0.\nActions:\n\x20   To bump:\n\x20       Set count to count + 1.\nView:\n\x20   <h1 id=\"count\">Count: {count}</h1>\n\x20   <button id=\"bump\" on-click={bump}>+1</button>\n";

#[test]
fn the_first_paint_is_the_markup_the_page_mounts() {
    let markup = first_paint(COUNTER).expect("the counter page paints");
    assert!(markup.contains("<h1") && markup.contains("Count: 0"), "the first paint is not the page's view:\n{markup}");
    assert!(markup.contains(">+1</button>"), "the first paint lost the button:\n{markup}");
    assert!(!markup.starts_with("<body") && !markup.ends_with("</body>"), "the first paint is the body's content, not a body:\n{markup}");
}

#[test]
fn a_program_that_draws_nothing_has_no_first_paint_and_says_so() {
    let error = first_paint("## Main\n\nShow 1.\n").expect_err("a console program paints no page");
    assert!(error.contains("renders nothing"), "the reason is not that it renders nothing: {error}");
}

#[test]
fn a_painted_preview_is_the_preview_with_the_first_paint_in_its_body() {
    let plain = assemble_preview(COUNTER).expect("the counter builds");
    let painted = assemble_preview_painted(COUNTER).expect("the counter builds");
    let markup = first_paint(COUNTER).expect("the counter page paints");
    let mark = logicaffeine_language::parser::lgx_lower::PRERENDERED_ATTR;
    let body = format!("<body {mark}=\"1\">\n{markup}\n</body>");
    assert!(painted.contains(&body), "the painted preview does not carry the first paint, marked for the page to adopt:\n{painted}");
    assert_eq!(painted.replace(&body, "<body></body>"), plain, "the painted preview differs from the plain one beyond its body");
}

#[test]
fn a_program_with_no_first_paint_previews_plain() {
    let source = "## Main\n\nShow 1.\n";
    assert_eq!(assemble_preview_painted(source).ok(), assemble_preview(source).ok(), "a program with nothing to paint is not previewed as it always was");
}

#[test]
fn a_page_that_does_not_build_is_still_its_build_error() {
    let broken = "## Page /\nView:\n\x20   <h1>{oops}</h1>\n";
    assert!(assemble_preview_painted(broken).is_err(), "a build error no longer surfaces as a ParseError");
}

#[test]
fn painting_twice_paints_the_same() {
    assert_eq!(first_paint(COUNTER), first_paint(COUNTER), "a second paint differs from the first");
    assert_eq!(assemble_preview_painted(COUNTER).ok(), assemble_preview_painted(COUNTER).ok());
}
