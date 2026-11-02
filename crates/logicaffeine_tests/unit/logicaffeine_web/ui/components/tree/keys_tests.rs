//! A tree answers the keyboard as the WAI-ARIA tree pattern says: the arrows walk the rows a reader
//! sees, Home and End reach the ends, Right opens a node and then enters it, Left closes it and then
//! climbs to its parent, and Enter or Space press the row.
use super::*;

/// The rows a reader sees, each `(depth, expandable, open)`:
///
/// ```text
/// 0 examples/        open
/// 1   code/          open
/// 2     a.logos
/// 3     b.logos
/// 4   logic/         closed
/// 5 notes.md
/// 6 empty/           open, nothing in it
/// ```
const ROWS: &[(usize, bool, bool)] = &[(0, true, true), (1, true, true), (2, false, false), (2, false, false), (1, true, false), (0, false, false), (0, true, true)];

fn rows() -> Vec<RowShape> {
    ROWS.iter().map(|&(depth, expandable, open)| RowShape { depth, expandable, open }).collect()
}

fn from(at: usize, key: TreeKey) -> TreeMove {
    next_row(&rows(), at, key)
}

#[test]
fn the_keys_are_the_patterns_keys() {
    for (key, want) in [
        ("ArrowDown", Some(TreeKey::Down)),
        ("ArrowUp", Some(TreeKey::Up)),
        ("Home", Some(TreeKey::Home)),
        ("End", Some(TreeKey::End)),
        ("ArrowRight", Some(TreeKey::Right)),
        ("ArrowLeft", Some(TreeKey::Left)),
        ("Enter", Some(TreeKey::Press)),
        (" ", Some(TreeKey::Press)),
        ("Tab", None),
        ("a", None),
        ("Escape", None),
    ] {
        assert_eq!(tree_key(key), want, "{key:?}");
    }
}

#[test]
fn down_moves_to_the_next_row_and_stops_at_the_last() {
    for at in 0..ROWS.len() - 1 {
        assert_eq!(from(at, TreeKey::Down), TreeMove::Focus(at + 1), "Down from row {at}");
    }
    assert_eq!(from(ROWS.len() - 1, TreeKey::Down), TreeMove::Stay, "Down from the last row");
}

#[test]
fn up_moves_to_the_row_above_and_stops_at_the_first() {
    for at in 1..ROWS.len() {
        assert_eq!(from(at, TreeKey::Up), TreeMove::Focus(at - 1), "Up from row {at}");
    }
    assert_eq!(from(0, TreeKey::Up), TreeMove::Stay, "Up from the first row");
}

#[test]
fn home_and_end_reach_the_first_and_the_last_row() {
    for at in 1..ROWS.len() {
        assert_eq!(from(at, TreeKey::Home), TreeMove::Focus(0), "Home from row {at}");
    }
    assert_eq!(from(0, TreeKey::Home), TreeMove::Stay, "Home on the first row");
    for at in 0..ROWS.len() - 1 {
        assert_eq!(from(at, TreeKey::End), TreeMove::Focus(ROWS.len() - 1), "End from row {at}");
    }
    assert_eq!(from(ROWS.len() - 1, TreeKey::End), TreeMove::Stay, "End on the last row");
}

#[test]
fn right_opens_a_closed_node_and_enters_an_open_one() {
    assert_eq!(from(4, TreeKey::Right), TreeMove::Press, "Right on a closed folder opens it");
    assert_eq!(from(0, TreeKey::Right), TreeMove::Focus(1), "Right on an open folder enters its first part");
    assert_eq!(from(1, TreeKey::Right), TreeMove::Focus(2), "Right on an open folder enters its first part");
    assert_eq!(from(6, TreeKey::Right), TreeMove::Stay, "Right on an open folder with nothing in it stays");
    assert_eq!(from(2, TreeKey::Right), TreeMove::Stay, "Right on a file stays");
    assert_eq!(from(5, TreeKey::Right), TreeMove::Stay, "Right on a top-level file stays");
}

#[test]
fn left_closes_an_open_node_and_climbs_from_anything_else() {
    assert_eq!(from(0, TreeKey::Left), TreeMove::Press, "Left on an open folder closes it");
    assert_eq!(from(1, TreeKey::Left), TreeMove::Press, "Left on an open folder closes it");
    assert_eq!(from(3, TreeKey::Left), TreeMove::Focus(1), "Left on a file climbs to its folder");
    assert_eq!(from(2, TreeKey::Left), TreeMove::Focus(1), "Left on a folder's first file climbs to its folder");
    assert_eq!(from(4, TreeKey::Left), TreeMove::Focus(0), "Left on a closed folder climbs to its parent");
    assert_eq!(from(5, TreeKey::Left), TreeMove::Stay, "Left on a top-level file stays");
}

#[test]
fn enter_and_space_press_the_row() {
    for at in 0..ROWS.len() {
        assert_eq!(from(at, TreeKey::Press), TreeMove::Press, "Press on row {at}");
    }
}

#[test]
fn a_tree_of_no_rows_or_a_row_past_the_end_stays() {
    assert_eq!(next_row(&[], 0, TreeKey::Down), TreeMove::Stay);
    assert_eq!(next_row(&rows(), ROWS.len(), TreeKey::Up), TreeMove::Stay);
}

/// A tree with no row selected: an open root over a leaf and a closed node, and a second top row.
#[allow(non_snake_case)]
fn Unselected() -> Element {
    rsx! {
        Tree { kind: "demo", label: "Demo tree",
            TreeNode {
                depth: 0,
                last: false,
                expandable: true,
                open: true,
                stop: true,
                row: rsx! { span { "root" } },
                TreeNode { depth: 1, last: false, row: rsx! { span { "leaf" } } }
                TreeNode { depth: 1, last: true, expandable: true, open: false, row: rsx! { span { "folded" } } }
            }
            TreeNode { depth: 0, last: true, row: rsx! { span { "second" } } }
        }
    }
}

/// The same tree with its leaf selected.
#[allow(non_snake_case)]
fn Selected() -> Element {
    rsx! {
        Tree { kind: "demo", label: "Demo tree",
            TreeNode {
                depth: 0,
                last: false,
                expandable: true,
                open: true,
                row: rsx! { span { "root" } },
                TreeNode { depth: 1, last: false, selected: true, row: rsx! { span { "leaf" } } }
                TreeNode { depth: 1, last: true, expandable: true, open: false, row: rsx! { span { "folded" } } }
            }
            TreeNode { depth: 0, last: true, row: rsx! { span { "second" } } }
        }
    }
}

fn render_tree(tree: fn() -> Element) -> String {
    let mut vdom = VirtualDom::new(tree);
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

/// The start tag of the row that reads `label`.
fn row_tag<'a>(html: &'a str, label: &str) -> &'a str {
    let at = html.find(&format!(">{label}<")).unwrap_or_else(|| panic!("no row reads {label:?}:\n{html}"));
    let start = html[..at].rfind(r#"<div class="tree-item"#).unwrap_or_else(|| panic!("{label:?} is in no row:\n{html}"));
    &html[start..start + html[start..].find('>').unwrap() + 1]
}

#[test]
fn a_tree_is_an_aria_tree_of_rows_at_their_levels() {
    let html = render_tree(Unselected);
    assert!(html.contains(r#"class="demo" data-tree="demo" role="tree" aria-label="Demo tree""#), "the tree is not named as a tree:\n{html}");
    let root = row_tag(&html, "root");
    for attr in [r#"role="treeitem""#, r#"aria-level="1""#, r#"aria-expanded="true""#, r#"aria-selected="false""#] {
        assert!(root.contains(attr), "the root row lacks {attr}: {root}");
    }
    let leaf = row_tag(&html, "leaf");
    assert!(leaf.contains(r#"aria-level="2""#) && !leaf.contains("aria-expanded"), "a leaf at depth 1: {leaf}");
    assert!(row_tag(&html, "folded").contains(r#"aria-expanded="false""#), "a closed node: {}", row_tag(&html, "folded"));
    assert!(html.contains(r#"class="tree-children demo-children" role="group""#), "an open node's parts are not a group:\n{html}");
    assert!(root.starts_with(r#"<div class="tree-item demo-item"#), "the row's class is no longer its first attribute: {root}");
}

#[test]
fn a_tree_has_one_tab_stop_on_its_selected_row_or_else_its_first() {
    let stops = |html: &str| ["root", "leaf", "folded", "second"].into_iter().filter(|label| row_tag(html, label).contains(r#"tabindex="0""#)).collect::<Vec<_>>();
    let unselected = render_tree(Unselected);
    assert_eq!(stops(&unselected), ["root"], "with no row selected, the first row is the tab stop");
    let selected = render_tree(Selected);
    assert_eq!(stops(&selected), ["leaf"], "the selected row is the tab stop");
    assert!(row_tag(&selected, "leaf").contains(r#"aria-selected="true""#), "the selected row does not say so");
    for label in ["root", "folded", "second"] {
        assert!(row_tag(&selected, label).contains(r#"tabindex="-1""#), "{label} is not reachable by the arrows: {}", row_tag(&selected, label));
    }
}
