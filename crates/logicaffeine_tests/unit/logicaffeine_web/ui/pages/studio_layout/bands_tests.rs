//! The layout's rules for phones and tablets: which pane a phone opens on, how far the editor scrolls
//! to keep the caret's line above the keyboard, and the columns a tablet on its side fits.
use super::*;
use crate::ui::state::StudioMode;

/// A phone shows the pane that shows the file: the output for a file that runs or previews as it
/// opens, the editor for Math, which waits for Execute.
#[test]
fn a_phone_opens_on_the_pane_that_shows_the_file() {
    for (mode, pane) in [
        (StudioMode::Logic, MobileTab::Panel2),
        (StudioMode::Code, MobileTab::Panel2),
        (StudioMode::Hardware, MobileTab::Panel2),
        (StudioMode::Web, MobileTab::Panel2),
        (StudioMode::Math, MobileTab::Panel1),
    ] {
        assert!(first_pane(mode) == pane, "{mode:?} opens on the wrong pane");
    }
}

fn extent(top: f64, bottom: f64) -> Extent {
    Extent { top, bottom }
}

#[test]
fn the_caret_line_stays_in_view() {
    let view = extent(100.0, 400.0);
    assert_eq!(caret_in_view(extent(150.0, 170.0), view), 0.0, "a line inside the view needs no scroll");
    assert_eq!(caret_in_view(extent(100.0, 120.0), view), 0.0, "a line on the view's top edge needs no scroll");
    assert_eq!(caret_in_view(extent(380.0, 400.0), view), 0.0, "a line on the view's bottom edge needs no scroll");
    assert_eq!(caret_in_view(extent(60.0, 80.0), view), -40.0, "a line above the view scrolls up to its top");
    assert_eq!(caret_in_view(extent(450.0, 470.0), view), 70.0, "a line below the view scrolls down to its bottom");
    assert_eq!(caret_in_view(extent(390.0, 410.0), view), 10.0, "a line cut by the view's bottom scrolls it whole into view");
    assert_eq!(caret_in_view(extent(50.0, 500.0), view), -50.0, "a line taller than the view shows from its top");
    assert_eq!(caret_in_view(extent(150.0, 170.0), extent(300.0, 300.0)), -150.0, "a view with no height shows the line's top");
}

/// A tablet on its side keeps the desktop's columns, the Files column shown: each keeps its minimum.
#[test]
fn a_tablet_in_landscape_fits_the_columns() {
    for width in [1024.0, 1180.0, 1366.0] {
        for right in [false, true] {
            let shown = Shown { sidebar: true, right };
            let columns = Columns::initial(width, shown);
            assert!(columns.sidebar >= SIDEBAR_MIN, "{width}px: the Files column is {}", columns.sidebar);
            assert!(columns.left >= LEFT_MIN, "{width}px: the editor column is {}", columns.left);
            assert!(columns.center(width, shown) >= CENTER_MIN, "{width}px: the center column is {}", columns.center(width, shown));
            if right {
                assert!(columns.right >= RIGHT_MIN, "{width}px: the third column is {}", columns.right);
            }
        }
    }
}
