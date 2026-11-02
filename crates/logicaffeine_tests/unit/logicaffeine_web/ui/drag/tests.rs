use super::*;

#[test]
fn each_axis_has_its_own_root_class_and_cursor_rule() {
    for axis in [Axis::Columns, Axis::Rows] {
        let class = axis.root_class();
        assert!(DRAG_STYLE.contains(&format!("html.{class},")), "no cursor rule for {class}");
    }
    assert_ne!(Axis::Columns.root_class(), Axis::Rows.root_class());
}

#[test]
fn a_live_drag_blocks_selection_and_iframes() {
    assert!(DRAG_STYLE.contains("html.dragging iframe { pointer-events: none"), "the Web preview would swallow the drag");
    assert!(DRAG_STYLE.contains("user-select: none"), "dragging would select text");
}
