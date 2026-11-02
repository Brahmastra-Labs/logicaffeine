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
