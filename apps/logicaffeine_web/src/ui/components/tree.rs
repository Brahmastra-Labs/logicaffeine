//! The Studio's tree: the file browser's folders and files and the Logic column's Syntax Tree
//! are both drawn by it, so they read alike.
//!
//! [`Tree`] holds the geometry, the lines' snapping to the device-pixel grid and the chevron's
//! look; [`TreeNode`] draws one node — the line it hangs from, its row, its parts while it is open.
//! Each user walks its own data and gives every node its row's content.
//!
//! A tree answers the keyboard as the WAI-ARIA tree pattern says: it is one tab stop, on the selected
//! row (or the row its user marks when none is), the arrows walk the rows a reader sees, Home and End
//! reach the ends, Right opens a node and then enters it, Left closes it and then climbs to its parent,
//! and Enter or Space press the row. [`next_row`] decides each key from the rows' shapes alone.

use dioxus::prelude::*;
use crate::ui::components::chevron::{Chevron, CHEVRON_STYLE};
use crate::ui::hooks::{use_device_pixel_grid, PixelGrid};

/// The tree's geometry is one step. Its first line stands `--tree-step` from the tree's edge, every
/// level's line one step further in, and every branch runs one step, from its parent's line to its
/// own; a row's first glyph (a node's chevron, else the first thing in its row) sits `--tree-gap`
/// past the end of its branch. Every length snaps to whole device pixels (`--dp`, one device pixel
/// in CSS pixels, is set from the page's device pixel ratio), and the tree's content starts on a
/// device pixel wherever the layout puts the tree (`--dp-nudge-x`/`-y`, under one device pixel, pad
/// it onto the grid), so each line covers whole screen pixels at any zoom and all read at one
/// weight, `--tree-hair`: about 1.5px and never under two device pixels. A tree sets how far its
/// content stands from its top and bottom with `--tree-pad-top` and `--tree-pad-bottom` (the Studio's
/// inset by default), each held to whole device pixels.
pub const TREE_STYLE: &str = r#"
[data-tree] {
    --dp: 1px;
    --dp-nudge-x: 0px;
    --dp-nudge-y: 0px;
    --tree-step: round(nearest, 18px, var(--dp));
    --tree-gap: round(nearest, 4px, var(--dp));
    --tree-row: round(nearest, 22px, var(--dp));
    --tree-hair: max(calc(2 * var(--dp)), round(nearest, 1.5px, var(--dp)));
    --tree-mid: round(down, calc((var(--tree-row) - var(--tree-hair)) / 2), var(--dp));
    --twistie: 12px;
    --twistie-gap: 4px;
    --tree-line: rgba(255, 255, 255, 0.22);
    padding:
        calc(round(nearest, var(--tree-pad-top, var(--studio-inset, 4px)), var(--dp)) + var(--dp-nudge-y))
        0
        round(nearest, var(--tree-pad-bottom, var(--studio-inset, 4px)), var(--dp))
        var(--dp-nudge-x);
}

.tree-node {
    position: relative;
    -webkit-user-select: none;
    user-select: none;
}

/* VS Code's explorer: 22px rows in its workbench font; a row at `--depth` starts its first glyph
   (`--depth` + 1) steps and a gap in. */
.tree-item {
    position: relative;
    display: flex;
    align-items: center;
    height: var(--tree-row);
    padding: 0 var(--studio-inset, 8px) 0 calc(var(--tree-step) * (var(--depth) + 1) + var(--tree-gap));
    font-family: -apple-system, BlinkMacSystemFont, "Segoe WPC", "Segoe UI", "Ubuntu", "Droid Sans", sans-serif;
    font-size: 13px;
    line-height: var(--tree-row);
    color: rgba(255, 255, 255, 0.8);
    transition: background 0.1s ease;
}

.tree-item.pressable {
    cursor: pointer;
}

.tree-item:hover {
    background: rgba(255, 255, 255, 0.04);
}

/* Under a finger every row is a finger tall, in the file tree and the Syntax Tree alike; the lines
   and branches follow the row. */
@media (pointer: coarse) {
    [data-tree] {
        --tree-row: round(nearest, 44px, var(--dp));
    }
}

/* A row the keyboard reached shows a ring inside its edges; a row pressed with a pointer shows none. */
.tree-item:focus {
    outline: none;
}

.tree-item:focus-visible {
    outline: 1px solid rgba(0, 212, 255, 0.6);
    outline-offset: -1px;
}

.tree-item.selected {
    background: rgba(0, 212, 255, 0.12);
    color: #00d4ff;
}

/* A node's chevron cell, the first glyph of its row. The chevron fills it, so an open node's
   chevron points down from the cell's left edge. */
.tree-item .twistie {
    flex: none;
    width: var(--twistie);
    margin-right: var(--twistie-gap);
    height: var(--tree-row);
    display: flex;
    align-items: center;
    justify-content: center;
}

/* Branches. A child at `--depth` hangs from the line `--depth` steps in; its branch leaves that
   line at the row's middle and runs one step, to the line its own children hang from. The first
   child's line starts a row up, at its parent's branch (the corner where the branch turns down),
   and the last child's line stops at its own branch. */
.tree-line,
.tree-branch {
    position: absolute;
    left: calc(var(--tree-step) * var(--depth));
    background: var(--tree-line);
}

.tree-line {
    top: var(--line-top, 0px);
    bottom: 0;
    width: var(--tree-hair);
}

.tree-children > .tree-node:first-child > .tree-line {
    --line-top: calc(var(--tree-mid) - var(--tree-row));
}

.tree-line.last {
    bottom: auto;
    height: calc(var(--tree-mid) + var(--tree-hair) - var(--line-top, 0px));
}

.tree-branch {
    top: var(--tree-mid);
    width: var(--tree-step);
    height: var(--tree-hair);
}
"#;

/// Which tree this is: the name its parts carry beside the shared ones.
#[derive(Clone, Copy, PartialEq)]
struct Kind(&'static str);

/// A tree, its nodes drawn inside it. `kind` names it and its parts: the tree is the element of
/// class `{kind}` (marked `data-tree`), and each node, row and open node's parts carry
/// `{kind}-node`, `{kind}-item` and `{kind}-children` beside the shared classes. The file
/// browser's tree is `file-tree`.
#[component]
pub fn Tree(
    kind: &'static str,
    /// The tree's name, for a screen reader.
    #[props(default)]
    label: &'static str,
    children: Element,
) -> Element {
    use_context_provider(|| Kind(kind));
    let (grid, on_mounted) = use_device_pixel_grid();
    let PixelGrid { device_px, nudge_x, nudge_y } = grid();

    rsx! {
        div {
            class: "{kind}",
            "data-tree": "{kind}",
            role: "tree",
            "aria-label": (!label.is_empty()).then_some(label),
            style: "--dp: {device_px}px; --dp-nudge-x: {nudge_x}px; --dp-nudge-y: {nudge_y}px;",
            onmounted: move |event| on_mounted.call(event),
            style { "{TREE_STYLE}" }
            style { "{CHEVRON_STYLE}" }
            {children}
        }
    }
}

/// One node of a [`Tree`]: the line it hangs from (below the top level), its row — its branch, its
/// chevron if it has parts, then `row` — and its parts, `children`, while it is open.
#[component]
pub fn TreeNode(
    /// How many levels below the top it sits.
    depth: usize,
    /// The last of its siblings: its line stops at its own branch.
    last: bool,
    /// Whether it has parts to open; its row then leads with the chevron.
    #[props(default)]
    expandable: bool,
    /// Whether its parts are shown.
    #[props(default)]
    open: bool,
    #[props(default)]
    selected: bool,
    /// The row the tree's one tab stop falls on when no row is selected: its first.
    #[props(default)]
    stop: bool,
    /// The row's tooltip.
    #[props(default)]
    title: Option<String>,
    /// What pressing the row does; a row with none is not pressable.
    #[props(default)]
    on_press: Option<EventHandler<()>>,
    /// The row's content after its chevron.
    row: Element,
    children: Element,
) -> Element {
    let Kind(kind) = use_context::<Kind>();
    let row_class = format!(
        "tree-item {kind}-item{}{}",
        if selected { " selected" } else { "" },
        if on_press.is_some() { " pressable" } else { "" }
    );

    rsx! {
        div { class: "tree-node {kind}-node", style: "--depth: {depth};",
            if depth > 0 {
                span { class: if last { "tree-line last" } else { "tree-line" } }
            }
            div {
                class: "{row_class}",
                role: "treeitem",
                "aria-level": "{depth + 1}",
                "aria-selected": if selected { "true" } else { "false" },
                "aria-expanded": expandable.then_some(if open { "true" } else { "false" }),
                tabindex: if selected || stop { "0" } else { "-1" },
                title: title,
                onclick: move |_| {
                    if let Some(press) = on_press {
                        press.call(());
                    }
                },
                onkeydown: move |evt: KeyboardEvent| {
                    let Some(key) = tree_key(&evt.key().to_string()) else {
                        return;
                    };
                    evt.prevent_default();
                    if walk(&evt, key) == TreeMove::Press {
                        if let Some(press) = on_press {
                            press.call(());
                        }
                    }
                },
                if depth > 0 {
                    span { class: "tree-branch" }
                }
                if expandable {
                    span { class: "twistie",
                        Chevron { open }
                    }
                }
                {row}
            }
            if expandable && open {
                div { class: "tree-children {kind}-children", role: "group", {children} }
            }
        }
    }
}

/// A key the tree answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeKey {
    Down,
    Up,
    Home,
    End,
    Right,
    Left,
    /// Enter or Space: what pressing the row does.
    Press,
}

/// The key a keyboard event names, if the tree answers it.
pub fn tree_key(key: &str) -> Option<TreeKey> {
    Some(match key {
        "ArrowDown" => TreeKey::Down,
        "ArrowUp" => TreeKey::Up,
        "Home" => TreeKey::Home,
        "End" => TreeKey::End,
        "ArrowRight" => TreeKey::Right,
        "ArrowLeft" => TreeKey::Left,
        "Enter" | " " => TreeKey::Press,
        _ => return None,
    })
}

/// A row as the keys see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowShape {
    pub depth: usize,
    pub expandable: bool,
    pub open: bool,
}

/// What a key does from a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeMove {
    /// Focus the row at this index of the rows a reader sees.
    Focus(usize),
    /// Press the row: open or close a node, open a file.
    Press,
    Stay,
}

/// What `key` does from row `at` of `rows`, the rows a reader sees in order.
pub fn next_row(rows: &[RowShape], at: usize, key: TreeKey) -> TreeMove {
    let Some(row) = rows.get(at) else {
        return TreeMove::Stay;
    };
    let focus = |to: Option<usize>| match to {
        Some(to) if to != at && to < rows.len() => TreeMove::Focus(to),
        _ => TreeMove::Stay,
    };
    match key {
        TreeKey::Down => focus(Some(at + 1)),
        TreeKey::Up => focus(at.checked_sub(1)),
        TreeKey::Home => focus(Some(0)),
        TreeKey::End => focus(rows.len().checked_sub(1)),
        TreeKey::Right if row.expandable && !row.open => TreeMove::Press,
        TreeKey::Right if row.expandable => focus(Some(at + 1).filter(|&next| rows.get(next).is_some_and(|r| r.depth == row.depth + 1))),
        TreeKey::Right => TreeMove::Stay,
        TreeKey::Left if row.expandable && row.open => TreeMove::Press,
        TreeKey::Left => focus(row.depth.checked_sub(1).and_then(|parent| rows[..at].iter().rposition(|r| r.depth == parent))),
        TreeKey::Press => TreeMove::Press,
    }
}

/// Answer `key` on the row the event came from, among the rows of its tree the page draws: move the
/// focus where [`next_row`] says, or say the row is to be pressed.
#[cfg(target_arch = "wasm32")]
fn walk(evt: &KeyboardEvent, key: TreeKey) -> TreeMove {
    use wasm_bindgen::JsCast;
    let row = evt
        .data()
        .downcast::<web_sys::KeyboardEvent>()
        .and_then(|e| e.target())
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .and_then(|element| element.closest(".tree-item").ok().flatten());
    let Some(row) = row else {
        return TreeMove::Stay;
    };
    let Some(list) = row.closest("[data-tree]").ok().flatten().and_then(|tree| tree.query_selector_all(".tree-item").ok()) else {
        return TreeMove::Stay;
    };
    let rows: Vec<web_sys::Element> = (0..list.length()).filter_map(|i| list.item(i)?.dyn_into().ok()).collect();
    let shapes: Vec<RowShape> = rows
        .iter()
        .map(|r| RowShape {
            depth: r.get_attribute("aria-level").and_then(|l| l.parse::<usize>().ok()).unwrap_or(1).saturating_sub(1),
            expandable: r.has_attribute("aria-expanded"),
            open: r.get_attribute("aria-expanded").as_deref() == Some("true"),
        })
        .collect();
    let Some(at) = rows.iter().position(|r| *r == row) else {
        return TreeMove::Stay;
    };
    let step = next_row(&shapes, at, key);
    if let TreeMove::Focus(to) = step {
        if let Some(target) = rows.get(to).and_then(|r| r.dyn_ref::<web_sys::HtmlElement>()) {
            let _ = target.focus();
        }
    }
    step
}

/// Off the browser no key reaches a row.
#[cfg(not(target_arch = "wasm32"))]
fn walk(_evt: &KeyboardEvent, key: TreeKey) -> TreeMove {
    if key == TreeKey::Press {
        TreeMove::Press
    } else {
        TreeMove::Stay
    }
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/tree/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/components/tree/keys_tests.rs"]
mod keys_tests;
