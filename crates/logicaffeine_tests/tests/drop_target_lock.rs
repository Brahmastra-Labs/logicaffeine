//! LOCK — an element a `When a file is dropped on "<sel>":` sentence names is a real drop target:
//! its registration cancels the browser's drag defaults, on every lowered tier.
//!
//! A browser delivers `drop` only to an element whose `dragover` was cancelled, and navigates the
//! tab to the file when `drop` itself is not. The cancelled events are one catalogue,
//! `lgx_lower::DROP_TARGET_CANCELS`; the rows below read it, and the list written here from the
//! HTML drag-and-drop processing model pins the catalogue itself. A cancel is a property of the
//! registration — a `Listen` whose flags carry `.prevent` — so it is read where the page listens.

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::semantics::dom::{self, DomOp, LISTEN_PASSIVE, LISTEN_PREVENT};
use logicaffeine_language::parser::lgx_lower::DROP_TARGET_CANCELS;

#[path = "compile_common/mod.rs"]
mod common;

/// The events HTML's drag-and-drop processing model requires a page to cancel for an element to
/// accept a drop and keep the tab where it is — written independently of the catalogue.
const HTML_DROP_TARGET_MODEL: &[&str] = &["dragenter", "dragover", "drop"];

/// Each shape a drop target is lowered through: the parameter's declared type decides what binds.
fn drop_target_pages() -> Vec<(&'static str, String)> {
    let root = |param: &str, read: &str| {
        format!(
            "## Component App\nState:\n    Let got be \"\".\nActions:\n    \
             When a file is dropped on \"#zone\" ({param}):\n        Set got to {read}.\n\
             View:\n    <div id=\"zone\">drop here</div>\n    <p id=\"got\">{{got}}</p>\n\n\
             ## Main\nMount App.\n"
        )
    };
    vec![
        ("a File parameter", root("file: File", "file's name")),
        ("a Text parameter", root("name: Text", "name")),
    ]
}

/// Whether a registration with `flags` cancels its event's default: `.prevent`, and not `.passive`,
/// which a browser holds above it.
fn cancels(flags: i64) -> bool {
    flags & i64::from(LISTEN_PREVENT) != 0 && flags & i64::from(LISTEN_PASSIVE) == 0
}

/// Every `(node, event)` the lowered VM registered with its default cancelled, in order.
fn cancelling_registrations(src: &str) -> Vec<(i64, String)> {
    dom::reset();
    let out = vm_outcome(src);
    assert_eq!(
        out.error, None,
        "the drop-target page runs on the lowered VM:\n{src}"
    );
    dom::op_log()
        .into_iter()
        .filter_map(|op| match op {
            DomOp::Listen { node, event, flags } if cancels(i64::from(flags)) => {
                Some((i64::from(node), event))
            }
            _ => None,
        })
        .collect()
}

/// The events the lowered VM registered with their default cancelled on the node `#zone` resolves
/// to.
fn cancelled_on_the_zone(src: &str) -> Vec<String> {
    let registered = cancelling_registrations(src);
    let zone = dom::query_selector("#zone");
    assert!(zone >= 0, "the page drew `#zone`:\n{src}");
    registered
        .into_iter()
        .filter(|(node, _)| *node == zone)
        .map(|(_, event)| event)
        .collect()
}

#[test]
fn the_catalogue_is_the_html_drop_target_model() {
    let mut catalogued: Vec<&str> = DROP_TARGET_CANCELS.to_vec();
    let mut model: Vec<&str> = HTML_DROP_TARGET_MODEL.to_vec();
    catalogued.sort_unstable();
    model.sort_unstable();
    assert_eq!(
        catalogued, model,
        "a drop target cancels exactly the drag events the model requires"
    );
}

#[test]
fn every_drop_target_cancels_every_catalogued_drag_default_on_the_lowered_vm() {
    for (shape, src) in drop_target_pages() {
        let cancelled = cancelled_on_the_zone(&src);
        for event in DROP_TARGET_CANCELS {
            let times = cancelled.iter().filter(|e| e == event).count();
            assert_eq!(
                times, 1,
                "{shape}: `{event}` is cancelled on the drop zone exactly once per paint, got \
                 {cancelled:?}:\n{src}"
            );
        }
    }
}

#[test]
fn a_drop_target_cancels_nothing_on_an_element_it_does_not_name() {
    for (shape, src) in drop_target_pages() {
        let registered = cancelling_registrations(&src);
        let zone = dom::query_selector("#zone");
        let elsewhere: Vec<&(i64, String)> = registered
            .iter()
            .filter(|(node, _)| *node != zone)
            .collect();
        assert!(
            elsewhere.is_empty(),
            "{shape}: only the drop zone cancels its drag defaults: {elsewhere:?}"
        );
        assert!(
            !registered.is_empty(),
            "{shape}: the zone's own cancels are there to tell apart:\n{src}"
        );
    }
}

/// The browser module, run on a host that keeps where the page listens: one element carries every
/// catalogued drag event with its default cancelled, each exactly once, and it is the element the
/// sentence names — a file dropped there reaches the handler.
#[cfg(feature = "wasm-jit")]
#[test]
fn every_drop_target_cancels_every_catalogued_drag_default_in_the_browser_module() {
    for (shape, src) in drop_target_pages() {
        let mut page = common::wasm_host::mount_interactive(&src);
        let registered = page.registrations();
        let cancelling: Vec<&(i64, String, i64)> = registered
            .iter()
            .filter(|(_, _, flags)| cancels(*flags))
            .collect();
        let mut zones: Vec<i64> = cancelling.iter().map(|(node, _, _)| *node).collect();
        zones.sort_unstable();
        zones.dedup();
        assert_eq!(
            zones.len(),
            1,
            "{shape}: one element cancels the drag defaults: {registered:?}"
        );
        for event in DROP_TARGET_CANCELS {
            let times = cancelling.iter().filter(|(_, e, _)| e == event).count();
            assert_eq!(
                times, 1,
                "{shape}: `{event}` is cancelled on the drop zone exactly once: {registered:?}"
            );
        }
        let answered = page
            .dispatch(zones[0], "filedropped", "photo.png")
            .unwrap_or_else(|e| panic!("{shape}: {e}"));
        assert!(answered, "{shape}: the element that cancels is the one the sentence names — a drop there is heard");
    }
}

/// A component the page PLACES hears its own sentences like one it mounts: its drop zone is drawn
/// into its parent before either reaches the document, and the sentence still finds the element
/// among what the component drew, so its drag defaults are cancelled there.
#[test]
fn a_placed_components_drop_zone_is_a_drop_target_on_the_lowered_vm() {
    let src = "## Component Zone\nState:\n    Let got be \"\".\nActions:\n    \
               When a file is dropped on \"#zone\" (file: File):\n        Set got to file's name.\n\
               View:\n    <div id=\"zone\">{got}</div>\n\n\
               ## Component App\nView:\n    <main><Zone/></main>\n\n## Main\nMount App.\n";
    let cancelled = cancelled_on_the_zone(src);
    for event in DROP_TARGET_CANCELS {
        assert!(
            cancelled.iter().any(|e| e == event),
            "a placed component's drop zone cancels `{event}` on its own element, got {cancelled:?}"
        );
    }
}
