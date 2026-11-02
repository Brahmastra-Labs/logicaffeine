//! The Studio's resizable layout: the desktop columns — file sidebar, then up to three
//! panels — and the height of the bottom-docked debugger drawer.
//!
//! Widths are CSS pixels. The sidebar, the left panel and the right panel each hold their
//! width; the center panel is the flexible one and takes whatever the others leave. A
//! resizer moves exactly one boundary, so a drag changes only the two columns beside it —
//! every other column keeps its width to the pixel. Every column has a minimum, and the
//! minimums fit side by side at the narrowest desktop width, so the row never overflows.
//!
//! The layout the user shapes by dragging is a preference; what the Studio draws is that
//! preference [`Columns::fit`] to the current width. Narrowing the window squeezes the
//! drawn layout, and widening it again brings the preference back unchanged.
//!
//! A phone shows one pane at a time ([`MobileTab`], opened on [`first_pane`]), and keeps the
//! caret's line above its keyboard ([`caret_in_view`]).

use crate::ui::state::StudioMode;

/// The narrowest window that gets the side-by-side layout; below it the Studio stacks.
pub const DESKTOP_MIN_WIDTH: f64 = (crate::ui::responsive::breakpoints::MOBILE_MAX + 1) as f64;

pub const SIDEBAR_MIN: f64 = 140.0;
pub const LEFT_MIN: f64 = 200.0;
pub const CENTER_MIN: f64 = 200.0;
pub const RIGHT_MIN: f64 = 200.0;

/// Width of a resizer bar.
pub const RESIZER: f64 = 6.0;
/// The hairline between neighbours in the panel row.
pub const GAP: f64 = 1.0;
/// Differences smaller than this are floating-point noise, not layout.
const SUBPIXEL: f64 = 1e-6;

/// The pane a phone shows, one at a time: the editor, the output, or the third panel (Console,
/// Context or the picture). Every Run and Execute shows the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MobileTab {
    #[default]
    Panel1,
    Panel2,
    Panel3,
}

/// The pane a phone opens a file of `mode` on: the output for a file that runs or previews as it
/// opens, the editor for Math, which waits for Execute. Decided by the mode alone, so the server's
/// render is the phone's first frame.
pub fn first_pane(mode: StudioMode) -> MobileTab {
    match mode {
        StudioMode::Math => MobileTab::Panel1,
        StudioMode::Logic | StudioMode::Code | StudioMode::Hardware | StudioMode::Web => MobileTab::Panel2,
    }
}

/// A stretch of the page from top to bottom, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub top: f64,
    pub bottom: f64,
}

/// How far the editor scrolls so the caret's line shows in `view`, the part of its pane the reader
/// can see (above an on-screen keyboard): nothing when it shows already, up to its top when it is
/// above, down to its bottom when it is below. A line taller than the view shows from its top.
pub fn caret_in_view(caret: Extent, view: Extent) -> f64 {
    if caret.top < view.top || caret.bottom - caret.top > view.bottom - view.top {
        caret.top - view.top
    } else if caret.bottom > view.bottom {
        caret.bottom - view.bottom
    } else {
        0.0
    }
}

/// Which optional columns are on screen. The left and center panels always are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    pub sidebar: bool,
    pub right: bool,
}

/// A resizer, named by the boundary it moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resizer {
    SidebarLeft,
    LeftCenter,
    CenterRight,
}

/// The fixed-width columns. The center is derived: see [`Columns::center`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Columns {
    pub sidebar: f64,
    pub left: f64,
    pub right: f64,
}

impl Columns {
    /// Pixels taken by the resizer bars and hairlines for the columns shown.
    pub fn chrome(shown: Shown) -> f64 {
        let sidebar_bar = if shown.sidebar { RESIZER } else { 0.0 };
        // left | bar | center, and with a right panel: | bar | right — a hairline between each pair.
        let panel_row = if shown.right { 2.0 * RESIZER + 4.0 * GAP } else { RESIZER + 2.0 * GAP };
        sidebar_bar + panel_row
    }

    /// The center panel's width in a row `width` pixels wide.
    pub fn center(&self, width: f64, shown: Shown) -> f64 {
        width
            - Self::chrome(shown)
            - if shown.sidebar { self.sidebar } else { 0.0 }
            - self.left
            - if shown.right { self.right } else { 0.0 }
    }

    /// The layout a first visit starts from: a 300px sidebar beside panels split 35% / 40% /
    /// 25%, the proportions the Studio has always opened with.
    pub fn initial(width: f64, shown: Shown) -> Self {
        // 300px: the longest shipped example name, three folders deep, reads whole (DAT-11).
        let sidebar = 300.0;
        let panels = width - Self::chrome(Shown { sidebar: true, right: true }) - sidebar;
        Columns { sidebar, left: 0.35 * panels, right: 0.25 * panels }.fit(width, shown)
    }

    /// The layout after dragging `resizer` by `dx` pixels, where `self` is the layout on
    /// screen when the drag began. Only the two columns beside the resizer change, and
    /// neither passes its minimum.
    pub fn dragged(self, resizer: Resizer, dx: f64, width: f64, shown: Shown) -> Self {
        let center = self.center(width, shown);
        let mut next = self;
        match resizer {
            Resizer::SidebarLeft if shown.sidebar => {
                let dx = bounded(dx, SIDEBAR_MIN - self.sidebar, self.left - LEFT_MIN);
                next.sidebar += dx;
                next.left -= dx;
            }
            Resizer::LeftCenter => {
                next.left += bounded(dx, LEFT_MIN - self.left, center - CENTER_MIN);
            }
            Resizer::CenterRight if shown.right => {
                next.right -= bounded(dx, CENTER_MIN - center, self.right - RIGHT_MIN);
            }
            Resizer::SidebarLeft | Resizer::CenterRight => {}
        }
        next
    }

    /// This layout drawn in a row `width` pixels wide. Each column is held to its bounds;
    /// any change in width lands on the center first, and when the center would drop below
    /// its minimum the right panel gives way, then the left, then the sidebar, each down to
    /// its own minimum. A column that is not shown keeps its width for when it returns.
    pub fn fit(self, width: f64, shown: Shown) -> Self {
        let mut next = Columns {
            sidebar: self.sidebar.max(SIDEBAR_MIN),
            left: self.left.max(LEFT_MIN),
            right: self.right.max(RIGHT_MIN),
        };
        let mut short = CENTER_MIN - next.center(width, shown);
        if short < SUBPIXEL {
            return next;
        }
        let mut give = |column: &mut f64, min: f64| {
            let taken = (*column - min).clamp(0.0, short.max(0.0));
            *column -= taken;
            short -= taken;
        };
        if shown.right {
            give(&mut next.right, RIGHT_MIN);
        }
        give(&mut next.left, LEFT_MIN);
        if shown.sidebar {
            give(&mut next.sidebar, SIDEBAR_MIN);
        }
        next
    }
}

/// `dx` limited to `[lo, hi]`. A range left empty by a row too narrow for every minimum
/// allows no movement past its tighter end.
fn bounded(dx: f64, lo: f64, hi: f64) -> f64 {
    dx.max(lo).min(hi)
}

/// The drawer can shrink to its control bar and no further.
pub const DRAWER_MIN: f64 = 44.0;
/// However tall the drawer grows, the panel row above it keeps at least this much.
pub const PANELS_MIN_HEIGHT: f64 = 100.0;
/// The drawer's height when it first opens: room for its controls and its tabs.
pub const DRAWER_INITIAL: f64 = 340.0;

/// The drawer's height after dragging its grip by `dy` pixels (down is positive), where
/// `start` is its height when the drag began and `room` is the height the panel row and the
/// drawer share.
pub fn drawer_dragged(start: f64, dy: f64, room: f64) -> f64 {
    drawer_fit(start - dy, room)
}

/// A drawer height drawn in `room` pixels shared with the panel row: never below the
/// control bar, and never so tall the panels lose their minimum.
pub fn drawer_fit(height: f64, room: f64) -> f64 {
    docked_fit(height, room, DRAWER_MIN, PANELS_MIN_HEIGHT)
}

/// A pane docked below another in `room` pixels: never below its own `min`, never so tall
/// the pane above loses `above`. When the room cannot hold both, the docked pane keeps `min`.
fn docked_fit(height: f64, room: f64, min: f64, above: f64) -> f64 {
    height.min(room - above).max(min)
}

/// Logic's proof drawer shrinks to its bar and no further.
pub const PROOF_MIN: f64 = 44.0;
/// The proof drawer's height when it first opens: room for its bar and the three steps.
pub const PROOF_DRAWER_INITIAL: f64 = 200.0;

/// The proof drawer's height after dragging its grip by `dy` pixels (down is positive), where
/// `start` is its height when the drag began and `room` is the height the panel row and the
/// drawer share: never below its bar, and never so tall the panels lose their minimum.
pub fn proof_dragged(start: f64, dy: f64, room: f64) -> f64 {
    docked_fit(start - dy, room, PROOF_MIN, PANELS_MIN_HEIGHT)
}

impl Resizer {
    /// The `data-resizer` attribute that names this resizer's bar in the page.
    pub fn attr(self) -> &'static str {
        match self {
            Resizer::SidebarLeft => "sidebar-left",
            Resizer::LeftCenter => "left-center",
            Resizer::CenterRight => "center-right",
        }
    }
}

/// On a phone the panels stack; the upper one's share of the stack, in percent, stays
/// between these bounds so neither panel can be dragged out of sight.
pub const SPLIT_MIN: f64 = 15.0;
pub const SPLIT_MAX: f64 = 85.0;

/// The upper panel's share of a stack `height` pixels tall whose top edge is at `top`, when
/// the split bar is at `y`.
pub fn split_percent(y: f64, top: f64, height: f64) -> f64 {
    if height <= 0.0 {
        return 50.0;
    }
    ((y - top) / height * 100.0).clamp(SPLIT_MIN, SPLIT_MAX)
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio_layout/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/studio_layout/bands_tests.rs"]
mod bands_tests;
