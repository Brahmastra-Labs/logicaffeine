//! The workbench's state, held in a running app: it opens in the opening's mode with the opening's
//! text, and it keeps what the reader wrote in a file for the visit, so opening another file and
//! coming back shows the edit (D3 of the homepage plan, settled 2026-10-16).
use super::*;
use crate::ui::examples::seeded_examples;
use crate::ui::pages::studio_files::examples_tree;
use std::cell::Cell;

thread_local! {
    static STATE: Cell<Option<WorkbenchState>> = const { Cell::new(None) };
}

#[derive(Props, Clone, PartialEq)]
struct HostProps {
    path: &'static str,
}

/// A page that holds a workbench opened on `path`, and leaves its state where the test can reach it.
#[allow(non_snake_case)]
fn Host(props: HostProps) -> Element {
    let opening = use_hook(|| StudioOpening::for_route(Some(props.path)));
    let state = use_workbench(&opening, examples_tree);
    STATE.with(|cell| cell.set(Some(state)));
    rsx! {}
}

/// A running app with a workbench opened on `path`, and the state it holds.
fn opened_on(path: &'static str) -> (VirtualDom, WorkbenchState) {
    let mut vdom = VirtualDom::new_with_props(Host, HostProps { path });
    vdom.rebuild_in_place();
    let state = STATE.with(Cell::take).expect("the host holds no workbench");
    (vdom, state)
}

fn shipped(path: &str) -> &'static str {
    seeded_examples().find(|spec| spec.vfs_path == path).unwrap_or_else(|| panic!("{path} is not a shipped example")).source
}

#[test]
fn the_state_opens_in_the_openings_mode_with_its_text() {
    for (path, mode) in [
        ("/examples/logic/quantifiers.logic", StudioMode::Logic),
        ("/examples/code/basics/control-flow.logos", StudioMode::Code),
        ("/examples/math/auto-tactic.logos", StudioMode::Math),
        ("/examples/hardware/handshake.hw", StudioMode::Hardware),
        ("/examples/code/web/website.logos", StudioMode::Web),
    ] {
        let (vdom, state) = opened_on(path);
        vdom.in_scope(ScopeId::ROOT, || {
            assert_eq!(*state.mode.peek(), mode, "{path} opens in {:?}", *state.mode.peek());
            assert_eq!(*state.editor(mode).peek(), shipped(path), "{path} opens without its text");
            assert_eq!(state.current_file.peek().as_deref(), Some(path), "{path} is not the open file");
        });
    }
}

#[test]
fn an_edit_survives_opening_another_file() {
    let first = "/examples/code/basics/control-flow.logos";
    let second = "/examples/logic/quantifiers.logic";
    let (vdom, state) = opened_on(first);
    vdom.in_scope(ScopeId::ROOT, || {
        let edited = format!("{}\nShow \"edited\".\n", shipped(first));
        state.editor(StudioMode::Code).clone().set(edited.clone());
        state.choose(second.to_string());
        assert_eq!(state.current_file.peek().as_deref(), Some(second), "choosing a file does not open it");
        assert_eq!(state.opening_text(first, shipped(first).to_string()), edited, "coming back to {first} loses the edit");
        assert_eq!(state.opening_text(second, shipped(second).to_string()), shipped(second), "a file never edited opens as shipped");
    });
}

#[test]
fn an_edit_is_kept_by_its_own_file_only() {
    let first = "/examples/code/basics/control-flow.logos";
    let (vdom, state) = opened_on(first);
    vdom.in_scope(ScopeId::ROOT, || {
        state.editor(StudioMode::Code).clone().set("Show 1.".to_string());
        state.choose("/examples/math/auto-tactic.logos".to_string());
        for (path, spec) in seeded_examples().map(|spec| (spec.vfs_path, spec)).filter(|(path, _)| *path != first) {
            assert_eq!(state.opening_text(path, spec.source.to_string()), spec.source, "{path} opens with another file's edit");
        }
    });
}

/// The Studio's preview document is the page's own, first paint and all, with the centering style
/// in its head and nowhere else; a page that does not build gets the overlay, uncentered.
#[test]
fn the_preview_document_centers_the_page_and_keeps_its_first_paint() {
    use super::{preview_document, PREVIEW_CENTERING_STYLE};
    let source = shipped("/examples/code/web/counter.logos");
    let doc = preview_document(source);
    let painted = logicaffeine_compile::web_host::assemble_preview_painted(source).expect("counter builds");
    assert_eq!(doc.matches(PREVIEW_CENTERING_STYLE).count(), 1);
    assert!(doc.find(PREVIEW_CENTERING_STYLE).unwrap() < doc.find("</head>").unwrap());
    assert!(doc.ends_with(&painted[painted.find("<body").unwrap()..]), "the first paint is not kept");
    assert_eq!(doc.replace(PREVIEW_CENTERING_STYLE, ""), painted, "the Studio changes more than the one style");
    assert!(PREVIEW_CENTERING_STYLE.contains("safe center"), "a page taller than the frame must start at the top");
    let broken = preview_document("## Page /\nThis is not a page.");
    assert!(!broken.contains(PREVIEW_CENTERING_STYLE), "the error overlay is not a page to center");
}

// ---------------------------------------------------------------------------------------------
// Opening a file shows what it does: every output column goes back to its Output view, so a
// reader who last looked at a program as Rust sees the next example's output, not its Rust. The
// last run's time belonged to the last file, so it goes too until the new one has run.
// ---------------------------------------------------------------------------------------------

#[test]
fn choosing_another_example_selects_output_in_the_code_column() {
    let (vdom, state) = opened_on("/examples/code/basics/control-flow.logos");
    vdom.in_scope(ScopeId::ROOT, || {
        for shown in [CodeOutputMode::Rust, CodeOutputMode::C, CodeOutputMode::RiscV, CodeOutputMode::Wasm] {
            state.code_output_mode.clone().set(shown);
            let other = seeded_examples()
                .map(|spec| spec.vfs_path)
                .find(|path| path.starts_with("/examples/code/") && *path != "/examples/code/basics/control-flow.logos")
                .expect("a second Code example ships");
            state.choose(other.to_string());
            assert_eq!(*state.code_output_mode.peek(), CodeOutputMode::Interpret, "opening {other} after {shown:?} does not show its output");
        }
    });
}

#[test]
fn choosing_a_file_of_any_mode_selects_output_in_every_column() {
    let (vdom, state) = opened_on("/examples/code/basics/control-flow.logos");
    vdom.in_scope(ScopeId::ROOT, || {
        for path in ["/examples/math/auto-tactic.logos", "/examples/hardware/handshake.hw", "/examples/logic/quantifiers.logic", "/examples/code/basics/control-flow.logos"] {
            state.code_output_mode.clone().set(CodeOutputMode::Rust);
            state.math_output_mode.clone().set(CodeOutputMode::Rust);
            state.hw_output_mode.clone().set(CodeOutputMode::Rust);
            state.logic_output_mode.clone().set(LogicView::Rust);
            state.choose(path.to_string());
            assert_eq!(*state.code_output_mode.peek(), CodeOutputMode::Interpret, "{path}: the Code column");
            assert_eq!(*state.math_output_mode.peek(), CodeOutputMode::Interpret, "{path}: the Math column");
            assert_eq!(*state.hw_output_mode.peek(), CodeOutputMode::Interpret, "{path}: the Hardware column");
            assert_eq!(*state.logic_output_mode.peek(), LogicView::Logic, "{path}: the Logic column");
        }
    });
}

#[test]
fn choosing_a_file_clears_the_last_run_s_time() {
    let (vdom, state) = opened_on("/examples/code/basics/control-flow.logos");
    vdom.in_scope(ScopeId::ROOT, || {
        state.run_time.clone().set(Some(RunTime { millis: 3.0, finished: true }));
        state.choose("/examples/logic/quantifiers.logic".to_string());
        assert_eq!(*state.run_time.peek(), None, "the last file's run time is still shown");
    });
}

#[test]
fn a_fresh_workbench_has_no_run_time() {
    let (vdom, state) = opened_on("/examples/logic/quantifiers.logic");
    vdom.in_scope(ScopeId::ROOT, || assert_eq!(*state.run_time.peek(), None));
}

// ---------------------------------------------------------------------------------------------
// The Web preview's frame is only ever created with its document in place, and another document
// gets another frame. A sandboxed frame that goes in empty and is filled in the same task stays
// blank in Chromium (a shipped page opened from the tree, with its prebuilt document already
// fetched, showed nothing until something set the document again).
// ---------------------------------------------------------------------------------------------

fn preview_html(doc: &str) -> String {
    #[derive(Props, Clone, PartialEq)]
    struct Doc {
        doc: String,
    }
    #[allow(non_snake_case)]
    fn Frame(props: Doc) -> Element {
        rsx! { PreviewFrame { doc: props.doc } }
    }
    let mut vdom = VirtualDom::new_with_props(Frame, Doc { doc: doc.to_string() });
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

#[test]
fn there_is_no_preview_frame_before_there_is_a_document() {
    let html = preview_html("");
    assert!(!html.contains("<iframe"), "an empty preview frame is created, to be filled later: {html}");
    assert!(html.contains("preview-waiting"), "the panel shows nothing at all while it waits: {html}");
}

#[test]
fn a_preview_frame_is_created_with_its_document() {
    let doc = preview_document(shipped("/examples/code/web/counter.logos"));
    let html = preview_html(&doc);
    assert_eq!(html.matches("<iframe").count(), 1, "{html}");
    assert!(!html.contains("srcdoc=\"\""), "the preview frame is created empty");
    assert!(html.contains(&format!("title=\"{PREVIEW_TITLE}\"")), "the preview has no name for a screen reader");
    assert!(html.contains("sandbox=\"allow-scripts\""), "the preview is not sandboxed");
}

#[test]
fn each_preview_document_has_its_own_key() {
    let counter = preview_document(shipped("/examples/code/web/counter.logos"));
    let form = preview_document(shipped("/examples/code/web/form.logos"));
    assert_eq!(preview_key(&counter), preview_key(&counter.clone()), "one document, two keys: its frame would be rebuilt for nothing");
    assert_ne!(preview_key(&counter), preview_key(&form), "two documents, one key: the second would be set on the first's frame");
    assert_ne!(preview_key(&counter), preview_key(""));
}

// ---------------------------------------------------------------------------------------------
// A restore shows itself: over the editor, the reader's text pixelates away one character at a
// time as the file's text forms in the mirror of that order, then the layer fades and leaves the
// editor holding the file.
// ---------------------------------------------------------------------------------------------

fn swap_html(before: &str, after: &str) -> String {
    #[derive(Props, Clone, PartialEq)]
    struct Texts {
        before: String,
        after: String,
    }
    #[allow(non_snake_case)]
    fn Swap(props: Texts) -> Element {
        rsx! { HealSwap { before: props.before, after: props.after } }
    }
    let mut vdom = VirtualDom::new_with_props(Swap, Texts { before: before.to_string(), after: after.to_string() });
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

/// The text the renderer escaped back, for the characters these tests use.
fn unescape(text: &str) -> String {
    text.replace("&#34;", "\"").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn layer_text(html: &str, class: &str) -> String {
    html.split(&format!("class=\"{class}\""))
        .skip(1)
        .map(|rest| rest[rest.find('>').unwrap() + 1..rest.find("</span>").unwrap()].to_string())
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn a_restore_pixelates_every_character_of_both_texts() {
    let mine = "Show \"mine\".\nShow 2.";
    let shipped = "## Main\nShow \"Hello\".";
    let html = swap_html(mine, shipped);
    assert!(html.contains("class=\"heal-swap\""), "no restore layer: {html}");
    assert!(html.contains("aria-hidden=\"true\""), "a screen reader would read both texts: {html}");
    assert_eq!(html.matches("class=\"heal-px out\"").count(), mine.chars().count(), "a character of the reader's text does not dissolve");
    assert_eq!(html.matches("class=\"heal-px in\"").count(), shipped.chars().count(), "a character of the file does not form");
    assert_eq!(unescape(&layer_text(&html, "heal-px out")), mine, "the dissolving layer is not the reader's text");
    assert_eq!(unescape(&layer_text(&html, "heal-px in")), shipped, "the forming layer is not the file");
}

#[test]
fn each_character_starts_at_its_turn() {
    let html = swap_html("ab", "xyz");
    for delay in heal::pixel_turns(3, 11, true).iter().map(|&turn| heal::pixel_delay_ms(turn, 3)) {
        assert!(html.contains(&format!("animation-delay: {delay}ms;")), "no character forms at {delay} ms: {html}");
    }
}

#[test]
fn a_restore_too_long_to_pixelate_simply_shows_the_file() {
    let long = "x".repeat(heal::PIXEL_LIMIT + 1);
    assert!(!swap_html(&long, "short").contains("heal-swap"));
    assert!(!swap_html("short", &long).contains("heal-swap"));
    assert!(swap_html(&"x".repeat(heal::PIXEL_LIMIT), "short").contains("heal-swap"), "a text at the limit still pixelates");
}

#[test]
fn the_heal_card_for_a_restore_carries_the_pixel_layer() {
    #[allow(non_snake_case)]
    fn Restored() -> Element {
        let flight = use_signal(|| heal::Flight::Restored { previous: "Show \"mine\".".to_string() });
        let editor = use_signal(|| "## Main\nShow \"Hello\".".to_string());
        rsx! { HealOverlay { flight, editor } }
    }
    let mut vdom = VirtualDom::new(Restored);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    assert!(html.contains("class=\"heal-swap\""), "a restore does not pixelate: {html}");
    assert!(html.contains(heal::RESTORED), "the restore's card is gone: {html}");
}
