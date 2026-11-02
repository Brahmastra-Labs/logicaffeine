use super::*;
use crate::ui::components::chevron::CHEVRON_SVG;
use std::path::Path;

/// A small tree: a root with an open parent of two leaves, and a closed parent below them.
#[allow(non_snake_case)]
fn Demo() -> Element {
    rsx! {
        Tree { kind: "demo",
            TreeNode {
                depth: 0,
                last: true,
                expandable: true,
                open: true,
                title: "the root".to_string(),
                row: rsx! { span { class: "demo-label", "root" } },
                TreeNode { depth: 1, last: false, row: rsx! { span { class: "demo-label", "leaf" } } }
                TreeNode {
                    depth: 1,
                    last: true,
                    expandable: true,
                    open: false,
                    selected: true,
                    row: rsx! { span { class: "demo-label", "folded" } },
                    TreeNode { depth: 2, last: true, row: rsx! { span { class: "demo-label", "inside the fold" } } }
                }
            }
        }
    }
}

fn render() -> String {
    let mut vdom = VirtualDom::new(Demo);
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

/// The markup of the node whose row reads `label`: from its opening tag to its row's end.
fn node<'a>(html: &'a str, label: &str) -> &'a str {
    let at = html.find(&format!(">{label}<")).unwrap_or_else(|| panic!("no row reads {label:?}:\n{html}"));
    let start = html[..at].rfind(r#"<div class="tree-node"#).unwrap_or_else(|| panic!("the row {label:?} is not in a tree node:\n{html}"));
    &html[start..at]
}

#[test]
fn a_tree_names_its_parts_by_its_kind_beside_the_shared_way() {
    let html = render();
    for class in [r#"class="demo" data-tree="demo""#, r#"class="tree-node demo-node""#, r#"class="tree-item demo-item"#, r#"class="tree-children demo-children""#] {
        assert!(html.contains(class), "the tree has no part {class}:\n{html}");
    }
}

#[test]
fn a_node_below_the_top_level_hangs_from_a_line_by_a_branch() {
    let html = render();
    let root = node(&html, "root");
    assert!(!root.contains("tree-line") && !root.contains("tree-branch"), "the top level draws a line or branch: {root}");
    let leaf = node(&html, "leaf");
    assert!(leaf.contains(r#"class="tree-line""#), "a node below the top level has no line: {leaf}");
    assert!(leaf.contains(r#"class="tree-branch""#), "a node below the top level has no branch: {leaf}");
    assert!(leaf.contains("--depth: 1"), "a node does not carry its depth: {leaf}");
}

#[test]
fn the_last_sibling_stops_its_line_at_its_branch() {
    let html = render();
    assert!(node(&html, "folded").contains(r#"class="tree-line last""#), "the last sibling's line runs on: {}", node(&html, "folded"));
    assert!(!node(&html, "leaf").contains("tree-line last"), "a middle sibling's line stops: {}", node(&html, "leaf"));
}

#[test]
fn a_node_with_parts_leads_with_the_drawn_chevron_and_a_leaf_has_no_cell() {
    let html = render();
    let root = node(&html, "root");
    assert!(root.contains(r#"class="twistie""#) && root.contains(CHEVRON_SVG), "a node with parts has no chevron cell: {root}");
    assert!(root.contains(r#"class="chevron open""#), "an open node's chevron does not point down: {root}");
    let folded = node(&html, "folded");
    assert!(folded.contains(r#"class="chevron""#) && !folded.contains("chevron open"), "a closed node's chevron is turned: {folded}");
    assert!(!node(&html, "leaf").contains("twistie"), "a leaf keeps an empty chevron cell: {}", node(&html, "leaf"));
}

#[test]
fn an_open_node_draws_its_parts_and_a_closed_one_keeps_them_out() {
    let html = render();
    assert!(html.contains(">leaf<"), "an open node's parts are missing:\n{html}");
    assert!(!html.contains("inside the fold"), "a closed node draws its parts:\n{html}");
}

#[test]
fn a_row_carries_its_selection_and_its_tooltip() {
    let html = render();
    assert!(node(&html, "folded").contains("demo-item selected"), "a selected row is not marked: {}", node(&html, "folded"));
    assert!(node(&html, "root").contains(r#"title="the root""#), "a row drops its tooltip: {}", node(&html, "root"));
}

/// One step lays the whole tree out: the gutter to the first line, the gap between the lines of
/// two levels, and every branch. The user set it to 18px on 2026-09-27.
#[test]
fn one_eighteen_pixel_step_lays_the_tree_out() {
    assert!(TREE_STYLE.contains("--tree-step: round(nearest, 18px, var(--dp));"), "the tree's step is not 18px, snapped to device pixels");
    // The body of the rule whose selector list is exactly `selector`.
    let rule = |selector: &str| {
        TREE_STYLE
            .split('}')
            .filter_map(|chunk| chunk.split_once('{'))
            .find(|(selectors, _)| selectors.split("*/").last().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ") == selector)
            .map(|(_, body)| body.to_string())
            .unwrap_or_else(|| panic!("no rule for {selector}"))
    };
    assert!(rule(".tree-branch").contains("width: var(--tree-step);"), "a branch is not one step long");
    assert!(rule(".tree-line, .tree-branch").contains("left: calc(var(--tree-step) * var(--depth));"), "a level's line is not one step further in");
    assert!(rule(".tree-item").contains("calc(var(--tree-step) * (var(--depth) + 1) + var(--tree-gap))"), "a row's first glyph is not (depth + 1) steps and a gap in");
}

/// The tree's geometry is written once. A second stylesheet declaring its step would be a copy
/// the file tree and the Syntax Tree could drift apart by.
#[test]
fn the_tree_geometry_is_defined_once() {
    let components = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui");
    let mut found = Vec::new();
    for dir in ["components", "pages"] {
        for entry in std::fs::read_dir(components.join(dir)).expect("read the sources").filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).expect("read a source");
                if text.contains("--tree-step: round(") {
                    found.push(path.file_name().unwrap().to_string_lossy().into_owned());
                }
            }
        }
    }
    assert_eq!(found, vec!["tree.rs".to_string()], "the tree's step is declared in {found:?}");
}
