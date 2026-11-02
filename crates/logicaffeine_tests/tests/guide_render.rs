//! Component-level SSR assertions for the guide's example blocks.
//!
//! `GuideCodeBlock` is the honesty surface of the Syntax Guide: whatever an
//! example's `Runnability` declares, the rendered affordance must say the same
//! thing. These tests render the component natively (the exact SSG code path)
//! and assert the affordance per variant:
//!
//!   * `Playground` — a Run button; when `expected_output` is present, the
//!     output panel is pre-populated as "Expected output" before the first run.
//!   * `CompiledOnly` — NO Run button (it would fail); a "Compiled" pill
//!     carrying the reason instead.
//!   * `ExpectedError` — a Run button (running IS the demo) plus the panel
//!     pre-populated as "Expected error" with the exact locked diagnostic.

#![cfg(feature = "web-tests")]

use dioxus::prelude::*;
use logicaffeine_web::ui::components::exercise_block::{ExerciseBlock, ExerciseBlockProps};
use logicaffeine_web::ui::components::guide_code_block::{GuideCodeBlock, GuideCodeBlockProps};
use logicaffeine_web::ui::components::misconception_box::{
    MisconceptionBox, MisconceptionBoxProps,
};
use logicaffeine_web::ui::components::recall_block::{RecallCard, RecallCardProps};
use logicaffeine_web::ui::pages::guide::content::{ExampleMode, ExerciseKind, Runnability};

fn render_block(runnability: Runnability, expected_output: Option<String>) -> String {
    render_block_predict(runnability, expected_output, false)
}

fn render_block_predict(
    runnability: Runnability,
    expected_output: Option<String>,
    predict: bool,
) -> String {
    let mut vdom = VirtualDom::new_with_props(
        GuideCodeBlock,
        GuideCodeBlockProps {
            id: "test-example".to_string(),
            label: "Test Example".to_string(),
            mode: ExampleMode::Imperative,
            runnability,
            expected_output,
            predict,
            initial_code: "## Main\nShow 6 * 7.".to_string(),
        },
    );
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    // The component inlines its stylesheet, whose selectors mention every class
    // name — assert against the DOM markup only, not the CSS text.
    html.split("</style>")
        .last()
        .expect("split always yields at least one piece")
        .to_string()
}

#[test]
fn playground_block_offers_a_run_button() {
    let html = render_block(Runnability::Playground, None);
    assert!(
        html.contains("guide-code-btn primary"),
        "Playground example must render a Run button:\n{html}"
    );
    assert!(
        !html.contains("guide-code-compiled-pill"),
        "Playground example must not carry the compiled-only pill:\n{html}"
    );
}

#[test]
fn compiled_only_block_renders_pill_instead_of_run_button() {
    let html = render_block(
        Runnability::CompiledOnly {
            reason: "needs the compiled networking runtime",
        },
        None,
    );
    assert!(
        !html.contains("guide-code-btn primary"),
        "CompiledOnly example must NOT render a Run button that would fail:\n{html}"
    );
    assert!(
        html.contains("guide-code-compiled-pill"),
        "CompiledOnly example must render the compiled pill:\n{html}"
    );
    assert!(
        html.contains("needs the compiled networking runtime"),
        "the pill must carry the reason (tooltip):\n{html}"
    );
    assert!(
        html.contains("/guide#appendix-runnability"),
        "the pill must link to the runnability appendix:\n{html}"
    );
}

#[test]
fn playground_expected_output_prepopulates_the_panel() {
    let html = render_block(Runnability::Playground, Some("42".to_string()));
    assert!(
        html.contains("Expected output"),
        "an example with a locked output must show it before the first run:\n{html}"
    );
    assert!(
        html.contains("42"),
        "the expected output text must render:\n{html}"
    );
}

#[test]
fn expected_error_prepopulates_the_error_panel_and_keeps_run() {
    let html = render_block(
        Runnability::ExpectedError {
            message: "this example fails on purpose",
        },
        None,
    );
    assert!(
        html.contains("guide-code-btn primary"),
        "ExpectedError keeps the Run button — running IS the demo:\n{html}"
    );
    assert!(
        html.contains("Expected error"),
        "the locked diagnostic must be shown as 'Expected error':\n{html}"
    );
    assert!(
        html.contains("this example fails on purpose"),
        "the locked diagnostic text must render:\n{html}"
    );
}

#[test]
fn playground_without_expected_output_shows_no_panel_before_run() {
    let html = render_block(Runnability::Playground, None);
    assert!(
        !html.contains("guide-code-output-header"),
        "no output panel should render before the first run when nothing is locked:\n{html}"
    );
}

// ─── Pedagogy campaign: predict-first, recall cards, misconception boxes ───

/// A predict-first example commits the reader BEFORE the reveal (Muller/POE):
/// the locked output must be absent from the pre-run DOM, replaced by the
/// prediction prompt, with the reveal toggle offered.
#[test]
fn predict_first_hides_the_expected_output_by_default() {
    let html = render_block_predict(Runnability::Playground, Some("999888".to_string()), true);
    assert!(
        !html.contains("999888"),
        "predict-first must NOT leak the locked output into the pre-run DOM:\n{html}"
    );
    assert!(
        html.contains("What will this print?"),
        "predict-first prompts for a committed prediction:\n{html}"
    );
    assert!(
        html.contains("guide-predict-toggle"),
        "the reveal toggle must be offered:\n{html}"
    );
}

/// Predicting a Socratic error is prediction gold: the locked diagnostic must
/// also hide until reveal.
#[test]
fn predict_first_hides_the_expected_error_too() {
    let html = render_block_predict(
        Runnability::ExpectedError {
            message: "this locked diagnostic must hide until reveal",
        },
        None,
        true,
    );
    assert!(
        !html.contains("this locked diagnostic must hide until reveal"),
        "predict-first must not leak the locked error text:\n{html}"
    );
    assert!(
        html.contains("What error do you expect?"),
        "predict-first over an ExpectedError prompts for the error:\n{html}"
    );
}

/// The non-predict path is unchanged: locked output pre-populates the panel.
#[test]
fn non_predict_blocks_still_prepopulate() {
    let html = render_block_predict(Runnability::Playground, Some("424142".to_string()), false);
    assert!(
        html.contains("424142"),
        "a non-predict example keeps the pre-populated expected output:\n{html}"
    );
}

fn strip_styles(html: String) -> String {
    html.split("</style>")
        .last()
        .expect("split always yields at least one piece")
        .to_string()
}

/// A recall card is a flashcard: the answer must be ABSENT from the pre-reveal
/// DOM (conditional render, not CSS hiding — a flashcard that leaks its back
/// teaches nothing).
#[test]
fn recall_card_answer_is_absent_before_reveal() {
    let mut vdom = VirtualDom::new_with_props(
        RecallCard,
        RecallCardProps {
            id: "rq-test".to_string(),
            question: "What does `Let` do that `Set` does not?".to_string(),
            answer: "THE-HIDDEN-ANSWER-TEXT".to_string(),
        },
    );
    vdom.rebuild_in_place();
    let html = strip_styles(dioxus_ssr::render(&vdom));
    assert!(
        html.contains("What does `Let` do that `Set` does not?"),
        "the question renders:\n{html}"
    );
    assert!(
        !html.contains("THE-HIDDEN-ANSWER-TEXT"),
        "the answer must not exist in the pre-reveal DOM:\n{html}"
    );
    assert!(
        html.contains("guide-recall-reveal"),
        "the reveal affordance renders:\n{html}"
    );
}

/// The misconception box states the imported belief in the reader's voice and
/// links its runnable evidence.
#[test]
fn misconception_box_links_its_evidence() {
    let mut vdom = VirtualDom::new_with_props(
        MisconceptionBox,
        MisconceptionBoxProps {
            id: "m-test".to_string(),
            expectation: "In Python, lists start at 0.".to_string(),
            reality: "LOGOS counts from 1, and slices include both ends.".to_string(),
            evidence: "reading-one-based".to_string(),
        },
    );
    vdom.rebuild_in_place();
    let html = strip_styles(dioxus_ssr::render(&vdom));
    assert!(
        html.contains("guide-misconception"),
        "the box renders its class:\n{html}"
    );
    assert!(
        html.contains("In Python, lists start at 0."),
        "the imported belief renders:\n{html}"
    );
    assert!(
        html.contains("#reading-one-based"),
        "the box must deep-link its runnable evidence:\n{html}"
    );
}

/// The exercise block wears its rung on its sleeve (Renkl fading made visible)
/// and keeps hints behind a click — progressive disclosure, not a spoiler wall.
#[test]
fn exercise_block_shows_kind_and_hides_hints() {
    let mut vdom = VirtualDom::new_with_props(
        ExerciseBlock,
        ExerciseBlockProps {
            id: "ex-render-test".to_string(),
            prompt: "Fill the hole.".to_string(),
            mode: ExampleMode::Imperative,
            expected: "42".to_string(),
            starter: "## Main\n???".to_string(),
            kind: ExerciseKind::Completion,
            concepts: vec!["Let binding".to_string()],
            hints: vec!["THE-FIRST-HINT-TEXT".to_string()],
        },
    );
    vdom.rebuild_in_place();
    let html = strip_styles(dioxus_ssr::render(&vdom));
    assert!(
        html.contains("guide-exercise-kind"),
        "the kind badge renders:\n{html}"
    );
    assert!(
        html.contains("Fill in the blank"),
        "Completion reads as 'Fill in the blank' to the reader:\n{html}"
    );
    assert!(
        html.contains("guide-exercise-hint"),
        "the hint affordance renders:\n{html}"
    );
    assert!(
        !html.contains("THE-FIRST-HINT-TEXT"),
        "hint text stays out of the DOM until asked for:\n{html}"
    );
}

/// `> **Mental model:** …` renders as a styled callout — the blockquote
/// convention the pedagogy locks depend on (and generic `> ` quotes render as
/// blockquotes at all, which the old renderer dropped).
#[test]
fn mental_model_blockquote_renders_as_callout() {
    use logicaffeine_web::ui::pages::guide::render_markdown_for_test;

    let html = render_markdown_for_test("> **Mental model:** zones are whiteboard rooms.");
    assert!(
        html.contains("guide-mental-model"),
        "the mental-model callout class must render:\n{html}"
    );
    assert!(
        html.contains("zones are whiteboard rooms"),
        "the analogy text renders:\n{html}"
    );

    let plain = render_markdown_for_test("> a plain quoted line.");
    assert!(
        plain.contains("<blockquote"),
        "generic `> ` lines render as blockquotes:\n{plain}"
    );

    let look = render_markdown_for_test("> **Look back:** what generalizes here?");
    assert!(
        look.contains("guide-look-back"),
        "the Look back callout class must render:\n{look}"
    );
}

// ─── The shared editor: every LOGOS example, exercise and snippet ───

/// The markup alone: every inlined `<style>…</style>` element removed, wherever a component
/// nests one (the shared editor carries its own stylesheet inside the block).
fn markup(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(open) = rest.find("<style") {
        out.push_str(&rest[..open]);
        rest = rest[open..]
            .find("</style>")
            .map_or("", |close| &rest[open + close + "</style>".len()..]);
    }
    out.push_str(rest);
    out
}

/// The class list of every span the editor drew around exactly `word`.
fn token_classes(html: &str, word: &str) -> Vec<String> {
    let closing = format!(">{word}</span>");
    html.match_indices(&closing)
        .filter_map(|(at, _)| {
            let open = html[..at].rfind("<span")?;
            let tag = &html[open..at];
            let class = tag.split("class=\"").nth(1)?.split('"').next()?;
            Some(class.to_string())
        })
        .collect()
}

fn read_as(html: &str, word: &str, class: &str) -> bool {
    token_classes(html, word)
        .iter()
        .any(|c| c.split_whitespace().any(|k| k == class))
}

fn render_example(mode: ExampleMode, code: &str) -> String {
    let mut vdom = VirtualDom::new_with_props(
        GuideCodeBlock,
        GuideCodeBlockProps {
            id: "editor-example".to_string(),
            label: "Editor Example".to_string(),
            mode,
            runnability: Runnability::Playground,
            expected_output: None,
            predict: false,
            initial_code: code.to_string(),
        },
    );
    vdom.rebuild_in_place();
    markup(&dioxus_ssr::render(&vdom))
}

fn render_exercise(mode: ExampleMode, starter: &str) -> String {
    let mut vdom = VirtualDom::new_with_props(
        ExerciseBlock,
        ExerciseBlockProps {
            id: "ex-editor".to_string(),
            prompt: "Try it.".to_string(),
            mode,
            expected: "Hello".to_string(),
            starter: starter.to_string(),
            kind: ExerciseKind::Practice,
            concepts: vec![],
            hints: vec![],
        },
    );
    vdom.rebuild_in_place();
    markup(&dioxus_ssr::render(&vdom))
}

#[test]
fn the_markup_helper_drops_every_stylesheet_and_keeps_the_rest() {
    let html = "<style>.a{}</style><div class=\"x\">one<style>.b{}</style><b>two</b></div><style>.c{}</style>";
    assert_eq!(markup(html), "<div class=\"x\">one<b>two</b></div>");
}

/// An example is edited in the Studio's editor: the highlight layer over the text area, with
/// the statement keywords read by the LOGOS lexer — at whatever case they are written.
#[test]
fn an_example_is_edited_in_the_shared_editor() {
    let html = render_example(
        ExampleMode::Imperative,
        "## Main\nLet x be 5.\nShow x.\nif x is 5:\n    Show x.",
    );
    assert!(
        html.contains("code-editor-highlight") && html.contains("code-editor-textarea"),
        "no shared editor:\n{html}"
    );
    for word in ["Let", "Show", "if"] {
        assert!(
            read_as(&html, word, "tok-keyword"),
            "\"{word}\" is not read as a keyword: {:?}\n{html}",
            token_classes(&html, word)
        );
    }
    assert!(
        !read_as(&html, "x", "tok-keyword"),
        "a name is read as a keyword:\n{html}"
    );
}

/// A Logic example is English: it is read the way the Studio's Logic editor reads it —
/// quantifiers marked, not LOGOS statements.
#[test]
fn a_logic_example_is_read_as_logic() {
    let html = render_example(ExampleMode::Logic, "All birds fly.");
    assert!(
        read_as(&html, "All", "tok-builtin"),
        "\"All\" is not marked as a quantifier: {:?}\n{html}",
        token_classes(&html, "All")
    );
}

#[test]
fn a_theorem_example_is_read_as_logic() {
    let html = render_example(
        ExampleMode::Theorem,
        "## Theorem: Socrates_Is_Mortal\nGiven: All men are mortal.\nGiven: Socrates is a man.\nProve: Socrates is mortal.\nProof: By automation.",
    );
    for word in ["Theorem", "Given", "Prove", "Proof"] {
        assert!(
            read_as(&html, word, "tok-keyword"),
            "\"{word}\" is not read as a keyword: {:?}\n{html}",
            token_classes(&html, word)
        );
    }
    assert!(
        read_as(&html, "All", "tok-builtin"),
        "\"All\" is not marked as a quantifier:\n{html}"
    );
}

#[test]
fn an_exercise_is_edited_in_the_shared_editor() {
    let html = render_exercise(ExampleMode::Imperative, "## Main\nShow \"Hello\".");
    assert!(
        html.contains("code-editor-highlight") && html.contains("code-editor-textarea"),
        "no shared editor:\n{html}"
    );
    assert!(
        read_as(&html, "Show", "tok-keyword"),
        "\"Show\" is not read as a keyword:\n{html}"
    );
}

/// A Logic exercise is answered in First-Order Logic, which is not LOGOS: its answer box reads
/// no LOGOS keywords into the reader's formula.
#[test]
fn a_first_order_logic_answer_is_a_plain_box() {
    let html = render_exercise(ExampleMode::Logic, "∀x(");
    assert!(html.contains("<textarea"), "no answer box:\n{html}");
    assert!(
        !html.contains("code-editor"),
        "a First-Order Logic answer is read as LOGOS:\n{html}"
    );
}

/// LOGOS quoted in the prose is the same code, read the same way: the shared editor, read-only,
/// with a Copy button.
#[test]
fn a_snippet_is_quoted_read_only_in_the_shared_editor() {
    use logicaffeine_web::ui::components::code_editor::Language;
    use logicaffeine_web::ui::components::guide_snippet::{GuideSnippet, GuideSnippetProps};

    let mut vdom = VirtualDom::new_with_props(
        GuideSnippet,
        GuideSnippetProps {
            code: "## Main\nLet clicks be 0.\nShow clicks.".to_string(),
            language: Language::Logos,
        },
    );
    vdom.rebuild_in_place();
    let html = markup(&dioxus_ssr::render(&vdom));
    assert!(
        html.contains("guide-snippet"),
        "the snippet does not name itself:\n{html}"
    );
    assert!(
        html.contains("code-editor-highlight"),
        "the snippet is not in the shared editor:\n{html}"
    );
    assert!(
        read_as(&html, "Let", "tok-keyword"),
        "\"Let\" is not read as a keyword:\n{html}"
    );
    let textarea = html
        .split("<textarea")
        .nth(1)
        .and_then(|t| t.split('>').next())
        .unwrap_or("");
    assert!(
        textarea.contains("readonly"),
        "the reader can edit a quoted snippet: <textarea{textarea}>"
    );
    assert!(
        html.contains(">Copy</button>"),
        "the snippet has no Copy button:\n{html}"
    );
}

/// On the page, every LOGOS and Logic fence is quoted in the shared editor, and the shell and
/// config fences stay plain, copyable text.
#[test]
fn the_guide_quotes_its_logos_in_the_shared_editor_and_its_commands_plainly() {
    use dioxus_history::{History, MemoryHistory};
    use std::rc::Rc;

    let mut vdom = VirtualDom::new(logicaffeine_web::App);
    vdom.provide_root_context(
        Rc::new(MemoryHistory::with_initial_path("/guide")) as Rc<dyn History>
    );
    vdom.rebuild_in_place();
    let html = markup(&dioxus_ssr::render(&vdom));
    let snippets: Vec<&str> = html.split("class=\"guide-snippet\"").skip(1).collect();
    assert_eq!(
        snippets.len(),
        2,
        "the guide quotes {} snippets in the shared editor, not its 2 LOGOS and Logic fences",
        snippets.len()
    );
    assert!(
        snippets.iter().all(|s| s
            .split("</textarea>")
            .next()
            .is_some_and(|s| s.contains("code-editor-highlight"))),
        "a quoted snippet is not in the shared editor"
    );
    assert!(
        read_as(&html, "Await", "tok-keyword"),
        "\"Await\" in the concurrency snippet is not read as a keyword"
    );
    assert!(
        html.contains(
            "<pre><code>curl -fsSL https://logicaffeine.com/install.sh | sh</code></pre>"
        ),
        "the install command is no longer plain, copyable text"
    );
}

/// Every `<style>` element in `html`, and where the first piece of markup that is not a style
/// begins.
fn stylesheets_and_markup_start(html: &str) -> (Vec<&str>, Option<usize>) {
    let mut styles = Vec::new();
    let mut at = 0;
    let mut markup_start = None;
    while let Some(open) = html[at..].find('<').map(|i| at + i) {
        if html[open..].starts_with("<style") {
            let close = html[open..]
                .find("</style>")
                .map_or(html.len(), |i| open + i + "</style>".len());
            styles.push(&html[open..close]);
            at = close;
        } else {
            markup_start.get_or_insert(open);
            at = open + 1;
        }
    }
    (styles, markup_start)
}

/// A block's CSS comes before the markup it styles — its own and its editor's, each once. The
/// guide sets hundreds of editors into one page, so the editor's stylesheet travels with the block
/// that hosts it, ahead of that block's markup, never from inside the editor.
#[test]
fn a_block_places_every_stylesheet_ahead_of_its_markup() {
    use logicaffeine_web::ui::components::code_editor::Language;
    use logicaffeine_web::ui::components::guide_snippet::{GuideSnippet, GuideSnippetProps};

    let render = |mut vdom: VirtualDom| {
        vdom.rebuild_in_place();
        dioxus_ssr::render(&vdom)
    };
    let blocks = [
        (
            "an example",
            render(VirtualDom::new_with_props(
                GuideCodeBlock,
                GuideCodeBlockProps {
                    id: "styled".to_string(),
                    label: "Styled".to_string(),
                    mode: ExampleMode::Imperative,
                    runnability: Runnability::Playground,
                    expected_output: Some("42".to_string()),
                    predict: false,
                    initial_code: "## Main\nShow 6 * 7.".to_string(),
                },
            )),
        ),
        (
            "an exercise",
            render(VirtualDom::new_with_props(
                ExerciseBlock,
                ExerciseBlockProps {
                    id: "ex-styled".to_string(),
                    prompt: "Try it.".to_string(),
                    mode: ExampleMode::Imperative,
                    expected: "42".to_string(),
                    starter: "## Main\nShow 42.".to_string(),
                    kind: ExerciseKind::Practice,
                    concepts: vec![],
                    hints: vec![],
                },
            )),
        ),
        (
            "a snippet",
            render(VirtualDom::new_with_props(
                GuideSnippet,
                GuideSnippetProps {
                    code: "## Main\nShow 42.".to_string(),
                    language: Language::Logos,
                },
            )),
        ),
    ];
    for (what, html) in &blocks {
        let (styles, markup_start) = stylesheets_and_markup_start(html);
        let markup_start = markup_start.unwrap_or_else(|| panic!("{what} renders no markup"));
        let last_style = styles
            .last()
            .map_or(0, |s| s.as_ptr() as usize - html.as_ptr() as usize);
        assert!(
            last_style < markup_start,
            "{what} places a stylesheet inside its markup:\n{html}"
        );
        let editor_rules = styles
            .iter()
            .filter(|s| s.contains("\n.code-editor-textarea {"))
            .count();
        assert_eq!(
            editor_rules, 1,
            "{what} carries the editor's stylesheet {editor_rules} times"
        );
        let fit_rules = styles
            .iter()
            .filter(|s| s.contains(".inline-editor .code-editor-highlight"))
            .count();
        assert_eq!(
            fit_rules, 1,
            "{what} carries the editor's fit to its code {fit_rules} times"
        );
    }
}

#[test]
fn the_stylesheet_scan_tells_styles_from_markup() {
    let (styles, start) =
        stylesheets_and_markup_start("<style>.a{}</style><style>.b{}</style><div>x</div>");
    assert_eq!(styles, vec!["<style>.a{}</style>", "<style>.b{}</style>"]);
    assert_eq!(start, Some(38));
    let (styles, start) = stylesheets_and_markup_start("<div><style>.a{}</style></div>");
    assert_eq!(styles.len(), 1);
    assert_eq!(start, Some(0));
}

#[path = "web_common/prose_words.rs"]
mod prose_words;

/// Guide prose is written wrapped at a comfortable source width. A line break in the source
/// is a space on the page: a paragraph broken into one `<p>` per source line breaks mid-
/// sentence wherever the screen is narrower than the source.
#[test]
fn guide_prose_wrapped_across_lines_renders_as_one_paragraph() {
    use logicaffeine_web::ui::pages::guide::render_markdown_for_test;
    assert_eq!(
        render_markdown_for_test("LOGOS reads\nlike English."),
        "<p>LOGOS reads like English.</p>"
    );
    assert_eq!(
        render_markdown_for_test("One.\n\nTwo."),
        "<p>One.</p><p>Two.</p>"
    );
}

#[test]
fn a_wrapped_guide_callout_flows_without_forced_breaks() {
    use logicaffeine_web::ui::pages::guide::render_markdown_for_test;
    let html = render_markdown_for_test("> **Mental model:** zones are\n> whiteboard rooms.");
    assert!(
        html.starts_with("<blockquote class=\"guide-callout guide-mental-model\">"),
        "{html}"
    );
    assert!(html.contains("zones are whiteboard rooms."), "{html}");
    assert!(
        !html.contains("<br"),
        "a wrapped callout line must not become a forced line break:\n{html}"
    );
}

#[test]
fn a_wrapped_guide_list_item_stays_in_its_item() {
    use logicaffeine_web::ui::pages::guide::render_markdown_for_test;
    assert_eq!(
        render_markdown_for_test("- one\n  more\n- two"),
        "<ul><li>one more</li><li>two</li></ul>"
    );
}

#[test]
fn a_numbered_guide_list_is_numbered() {
    use logicaffeine_web::ui::pages::guide::render_markdown_for_test;
    assert_eq!(
        render_markdown_for_test("1. first\n2. second"),
        "<ol><li>first</li><li>second</li></ol>"
    );
}

/// Every line of every section's prose reaches the page, whole and in order of its words.
#[test]
fn every_line_of_guide_prose_reaches_the_page() {
    use logicaffeine_web::ui::pages::guide::content::SECTIONS;
    use logicaffeine_web::ui::pages::guide::render_markdown_for_test;
    let mut lost = Vec::new();
    for section in SECTIONS.iter() {
        let page = prose_words::words(&prose_words::visible_text(&render_markdown_for_test(
            section.content,
        )));
        for line in prose_words::prose_lines(section.content) {
            if !prose_words::contains_run(&page, &prose_words::shown_words(line)) {
                lost.push(format!("[{}] {}", section.id, line.trim()));
            }
        }
    }
    assert!(
        lost.is_empty(),
        "\n{} guide line(s) missing from the page:\n  {}\n",
        lost.len(),
        lost.join("\n  ")
    );
}
