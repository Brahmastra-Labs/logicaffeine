use super::*;

#[test]
fn every_resizer_names_its_bar_uniquely() {
    let names = [Resizer::SidebarLeft, Resizer::LeftCenter, Resizer::CenterRight].map(Resizer::attr);
    let unique: std::collections::HashSet<_> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "{names:?}");
}

#[test]
fn the_split_follows_the_bar() {
    assert_close(split_percent(500.0, 100.0, 800.0), 50.0, "middle");
    assert_close(split_percent(300.0, 100.0, 800.0), 25.0, "upper quarter");
    assert_close(split_percent(700.0, 100.0, 800.0), 75.0, "lower quarter");
}

#[test]
fn the_split_keeps_both_panels_in_sight() {
    assert_close(split_percent(0.0, 100.0, 800.0), SPLIT_MIN, "above the stack");
    assert_close(split_percent(5000.0, 100.0, 800.0), SPLIT_MAX, "below the stack");
}

#[test]
fn an_empty_stack_splits_evenly() {
    assert_close(split_percent(300.0, 100.0, 0.0), 50.0, "zero height");
}

const ALL: Shown = Shown { sidebar: true, right: true };
const NO_RIGHT: Shown = Shown { sidebar: true, right: false };
const NO_SIDEBAR: Shown = Shown { sidebar: false, right: true };
const BARE: Shown = Shown { sidebar: false, right: false };
const EVERY_SHOWN: [Shown; 4] = [ALL, NO_RIGHT, NO_SIDEBAR, BARE];
const WIDE: f64 = 1920.0;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn assert_close(a: f64, b: f64, what: &str) {
    assert!(close(a, b), "{what}: {a} != {b}");
}

fn layout() -> Columns {
    Columns::initial(WIDE, ALL)
}

/// The resizers a layout with these columns shows.
fn resizers(shown: Shown) -> Vec<Resizer> {
    let mut r = vec![Resizer::LeftCenter];
    if shown.sidebar {
        r.push(Resizer::SidebarLeft);
    }
    if shown.right {
        r.push(Resizer::CenterRight);
    }
    r
}

/// Every shown column is within its bounds and the row fits `width`.
fn assert_valid(c: Columns, width: f64, shown: Shown, context: &str) {
    if shown.sidebar {
        assert!(c.sidebar >= SIDEBAR_MIN - 1e-9, "{context}: sidebar {} below its minimum", c.sidebar);
    }
    assert!(c.left >= LEFT_MIN - 1e-9, "{context}: left {} below its minimum", c.left);
    if shown.right {
        assert!(c.right >= RIGHT_MIN - 1e-9, "{context}: right {} below its minimum", c.right);
    }
    assert!(c.center(width, shown) >= CENTER_MIN - 1e-9, "{context}: center {} below its minimum", c.center(width, shown));
}

#[test]
fn the_minimums_fit_side_by_side_at_the_narrowest_desktop_width() {
    let all_minimums = SIDEBAR_MIN + LEFT_MIN + CENTER_MIN + RIGHT_MIN + Columns::chrome(ALL);
    assert!(all_minimums <= DESKTOP_MIN_WIDTH, "{all_minimums}px of minimums cannot fit {DESKTOP_MIN_WIDTH}px");
}

#[test]
fn chrome_counts_the_bars_and_hairlines_that_are_drawn() {
    assert_close(Columns::chrome(ALL), 3.0 * RESIZER + 4.0 * GAP, "all columns");
    assert_close(Columns::chrome(NO_RIGHT), 2.0 * RESIZER + 2.0 * GAP, "no right panel");
    assert_close(Columns::chrome(NO_SIDEBAR), 2.0 * RESIZER + 4.0 * GAP, "no sidebar");
    assert_close(Columns::chrome(BARE), RESIZER + 2.0 * GAP, "panels only");
}

#[test]
fn the_first_layout_keeps_todays_proportions() {
    // Before this model the panels were 35% / 40% / 25% of the row beside a 240px sidebar; the
    // sidebar is 300px now, so the longest example name reads whole (DAT-11).
    let c = layout();
    let panels = WIDE - Columns::chrome(ALL) - 300.0;
    assert_close(c.sidebar, 300.0, "sidebar");
    assert_close(c.left, 0.35 * panels, "left");
    assert_close(c.right, 0.25 * panels, "right");
    assert_close(c.center(WIDE, ALL), 0.40 * panels, "center");
}

#[test]
fn the_first_layout_is_valid_at_every_desktop_width() {
    for shown in EVERY_SHOWN {
        let mut width = DESKTOP_MIN_WIDTH;
        while width <= 3840.0 {
            assert_valid(Columns::initial(width, shown), width, shown, &format!("initial at {width}px, {shown:?}"));
            width += 7.0;
        }
    }
}

#[test]
fn dragging_left_center_moves_only_that_boundary() {
    let c = layout();
    for dx in [-60.0, 60.0] {
        let d = c.dragged(Resizer::LeftCenter, dx, WIDE, ALL);
        assert_eq!(d.sidebar, c.sidebar, "sidebar moved");
        assert_eq!(d.right, c.right, "right moved");
        assert_close(d.left, c.left + dx, "left");
        assert_close(d.center(WIDE, ALL), c.center(WIDE, ALL) - dx, "center");
    }
}

#[test]
fn dragging_center_right_moves_only_that_boundary() {
    let c = layout();
    for dx in [-60.0, 60.0] {
        let d = c.dragged(Resizer::CenterRight, dx, WIDE, ALL);
        assert_eq!(d.sidebar, c.sidebar, "sidebar moved");
        assert_eq!(d.left, c.left, "left moved");
        assert_close(d.right, c.right - dx, "right");
        assert_close(d.center(WIDE, ALL), c.center(WIDE, ALL) + dx, "center");
    }
}

#[test]
fn dragging_sidebar_left_moves_only_that_boundary() {
    let c = layout();
    for dx in [-60.0, 60.0] {
        let d = c.dragged(Resizer::SidebarLeft, dx, WIDE, ALL);
        assert_eq!(d.right, c.right, "right moved");
        assert_close(d.center(WIDE, ALL), c.center(WIDE, ALL), "center moved");
        assert_close(d.sidebar, c.sidebar + dx, "sidebar");
        assert_close(d.left, c.left - dx, "left");
    }
}

#[test]
fn a_drag_stops_at_each_neighbours_minimum() {
    let c = layout();
    let center = c.center(WIDE, ALL);

    let d = c.dragged(Resizer::LeftCenter, -5000.0, WIDE, ALL);
    assert_close(d.left, LEFT_MIN, "left pushed to its minimum");
    let d = c.dragged(Resizer::LeftCenter, 5000.0, WIDE, ALL);
    assert_close(d.center(WIDE, ALL), CENTER_MIN, "center pushed to its minimum");
    assert_close(d.left, c.left + (center - CENTER_MIN), "left took exactly what the center gave");

    let d = c.dragged(Resizer::CenterRight, -5000.0, WIDE, ALL);
    assert_close(d.center(WIDE, ALL), CENTER_MIN, "center pushed to its minimum");
    let d = c.dragged(Resizer::CenterRight, 5000.0, WIDE, ALL);
    assert_close(d.right, RIGHT_MIN, "right pushed to its minimum");

    let d = c.dragged(Resizer::SidebarLeft, -5000.0, WIDE, ALL);
    assert_close(d.sidebar, SIDEBAR_MIN, "sidebar pushed to its minimum");
    assert_close(d.left, c.left + (c.sidebar - SIDEBAR_MIN), "left took exactly what the sidebar gave");
}

#[test]
fn the_file_browser_has_no_fixed_maximum() {
    // It grows until the left panel beside it reaches its minimum, and no further.
    let c = layout();
    let d = c.dragged(Resizer::SidebarLeft, 5000.0, WIDE, ALL);
    assert_close(d.left, LEFT_MIN, "left panel");
    assert_close(d.sidebar, c.sidebar + (c.left - LEFT_MIN), "file browser");
    assert!(d.sidebar > 600.0, "a 1920px window lets the file browser pass 600px: {}", d.sidebar);
}

#[test]
fn a_wide_file_browser_survives_a_refit() {
    let wide = Columns { sidebar: 700.0, left: 400.0, right: 300.0 };
    assert_eq!(wide.fit(WIDE, ALL), wide, "a layout that fits keeps its wide file browser");
}

#[test]
fn the_bars_are_six_pixels_wide() {
    assert_close(RESIZER, 6.0, "resizer bar");
}

#[test]
fn a_sidebar_drag_stops_where_the_left_panel_hits_its_minimum() {
    let c = Columns { sidebar: 200.0, left: 260.0, right: 400.0 };
    let d = c.dragged(Resizer::SidebarLeft, 5000.0, WIDE, ALL);
    assert_close(d.left, LEFT_MIN, "left");
    assert_close(d.sidebar, 260.0, "sidebar grew by exactly what the left panel gave");
}

#[test]
fn a_resizer_that_is_not_shown_moves_nothing() {
    let c = layout();
    assert_eq!(c.dragged(Resizer::CenterRight, 80.0, WIDE, NO_RIGHT), c);
    assert_eq!(c.dragged(Resizer::SidebarLeft, 80.0, WIDE, NO_SIDEBAR), c);
}

#[test]
fn a_zero_drag_changes_nothing() {
    let c = layout();
    for r in resizers(ALL) {
        assert_eq!(c.dragged(r, 0.0, WIDE, ALL), c, "{r:?}");
    }
}

#[test]
fn every_drag_leaves_the_other_columns_exactly_alone() {
    for shown in EVERY_SHOWN {
        let c = Columns::initial(WIDE, shown);
        let mut dx = -3000.0;
        while dx <= 3000.0 {
            for r in resizers(shown) {
                let d = c.dragged(r, dx, WIDE, shown);
                match r {
                    Resizer::SidebarLeft => {
                        assert_eq!(d.right, c.right, "{r:?} {dx} moved the right panel");
                        assert_close(d.center(WIDE, shown), c.center(WIDE, shown), &format!("{r:?} {dx} moved the center"));
                    }
                    Resizer::LeftCenter => {
                        assert_eq!(d.sidebar, c.sidebar, "{r:?} {dx} moved the sidebar");
                        assert_eq!(d.right, c.right, "{r:?} {dx} moved the right panel");
                    }
                    Resizer::CenterRight => {
                        assert_eq!(d.sidebar, c.sidebar, "{r:?} {dx} moved the sidebar");
                        assert_eq!(d.left, c.left, "{r:?} {dx} moved the left panel");
                    }
                }
                assert_valid(d, WIDE, shown, &format!("{r:?} by {dx} with {shown:?}"));
            }
            dx += 37.0;
        }
    }
}

#[test]
fn a_layout_that_fits_is_drawn_as_it_is() {
    let c = layout();
    assert_eq!(c.fit(WIDE, ALL), c);
    assert_eq!(c.fit(2560.0, ALL), c, "a wider window hands the extra to the center");
}

#[test]
fn a_narrower_window_takes_from_the_center_first() {
    let c = layout();
    let narrower = WIDE - (c.center(WIDE, ALL) - CENTER_MIN);
    let f = c.fit(narrower, ALL);
    assert_eq!(f, c, "the fixed columns kept their width");
    assert_close(f.center(narrower, ALL), CENTER_MIN, "center");
}

#[test]
fn past_the_center_minimum_the_right_then_left_then_sidebar_give_way() {
    let c = Columns { sidebar: 300.0, left: 500.0, right: 400.0 };
    let base = |c: Columns| c.sidebar + c.left + c.right + CENTER_MIN + Columns::chrome(ALL);

    let f = c.fit(base(c) - 150.0, ALL);
    assert_close(f.right, 250.0, "right gives first");
    assert_eq!((f.sidebar, f.left), (c.sidebar, c.left));

    let f = c.fit(base(c) - 300.0, ALL);
    assert_close(f.right, RIGHT_MIN, "right at its minimum");
    assert_close(f.left, 400.0, "then the left gives");
    assert_eq!(f.sidebar, c.sidebar);

    let f = c.fit(base(c) - 600.0, ALL);
    assert_close(f.left, LEFT_MIN, "left at its minimum");
    assert_close(f.sidebar, 200.0, "then the sidebar gives");

    let f = c.fit(DESKTOP_MIN_WIDTH, ALL);
    assert_valid(f, DESKTOP_MIN_WIDTH, ALL, "narrowest desktop");
}

#[test]
fn widening_the_window_again_restores_the_layout() {
    let c = layout();
    let squeezed = c.fit(1000.0, ALL);
    assert_ne!(squeezed, c, "1000px should squeeze this layout");
    assert_eq!(c.fit(WIDE, ALL), c, "the preference itself is untouched");
}

#[test]
fn a_hidden_right_panel_hands_its_room_to_the_center_and_keeps_its_width() {
    let c = layout();
    let f = c.fit(WIDE, NO_RIGHT);
    assert_eq!(f.right, c.right, "the right panel's width is kept for when it returns");
    assert_close(f.center(WIDE, NO_RIGHT), c.center(WIDE, ALL) + c.right + RESIZER + 2.0 * GAP, "center");
}

#[test]
fn a_squeezed_layout_never_leaves_its_bounds() {
    let prefs = [
        layout(),
        Columns { sidebar: 400.0, left: 1200.0, right: 900.0 },
        Columns { sidebar: 150.0, left: 200.0, right: 200.0 },
        Columns { sidebar: 90.0, left: 40.0, right: 3000.0 },
    ];
    for pref in prefs {
        for shown in EVERY_SHOWN {
            let mut width = DESKTOP_MIN_WIDTH;
            while width <= 3840.0 {
                assert_valid(pref.fit(width, shown), width, shown, &format!("{pref:?} at {width}px, {shown:?}"));
                width += 11.0;
            }
        }
    }
}

const ROOM: f64 = 900.0;

#[test]
fn the_proof_follows_the_divider() {
    assert_eq!(proof_dragged(200.0, -50.0, 600.0), 250.0);
    assert_eq!(proof_dragged(200.0, 30.0, 600.0), 170.0);
}

#[test]
fn the_proof_shrinks_to_its_status_line_and_no_further() {
    assert_eq!(proof_dragged(200.0, 500.0, 600.0), PROOF_MIN);
}

#[test]
fn the_proof_grows_until_the_panels_keep_their_minimum() {
    assert_eq!(proof_dragged(200.0, -1000.0, 600.0), 600.0 - PANELS_MIN_HEIGHT);
}

#[test]
fn a_room_too_short_for_both_keeps_the_proof_bar() {
    assert_eq!(proof_dragged(100.0, 0.0, 120.0), PROOF_MIN);
}

#[test]
fn dragging_the_drawer_grip_up_grows_it_by_exactly_that_much() {
    assert_close(drawer_dragged(DRAWER_INITIAL, -100.0, ROOM), DRAWER_INITIAL + 100.0, "up");
    assert_close(drawer_dragged(DRAWER_INITIAL, 100.0, ROOM), DRAWER_INITIAL - 100.0, "down");
    assert_close(drawer_dragged(DRAWER_INITIAL, 0.0, ROOM), DRAWER_INITIAL, "still");
}

#[test]
fn the_drawer_drags_all_the_way_down_to_its_control_bar() {
    assert_close(drawer_dragged(DRAWER_INITIAL, 5000.0, ROOM), DRAWER_MIN, "floor");
}

#[test]
fn the_drawer_drags_up_until_the_panels_keep_their_minimum() {
    assert_close(drawer_dragged(DRAWER_INITIAL, -5000.0, ROOM), ROOM - PANELS_MIN_HEIGHT, "ceiling");
}

#[test]
fn the_drawer_can_travel_most_of_the_screen() {
    // "Free to drag up and down": at 1080p the drawer spans nearly the whole Studio body.
    let room = 1080.0 - 97.0 - 49.0;
    let travel = drawer_dragged(DRAWER_INITIAL, -5000.0, room) - drawer_dragged(DRAWER_INITIAL, 5000.0, room);
    assert!(travel >= room - PANELS_MIN_HEIGHT - DRAWER_MIN - 1e-9, "only {travel}px of travel in {room}px");
}

#[test]
fn a_short_window_squeezes_the_drawer_and_a_tall_one_restores_it() {
    assert_close(drawer_fit(600.0, 500.0), 500.0 - PANELS_MIN_HEIGHT, "squeezed");
    assert_close(drawer_fit(600.0, 1200.0), 600.0, "restored");
    assert_close(drawer_fit(10.0, 1200.0), DRAWER_MIN, "never below the control bar");
}

#[test]
fn a_room_too_short_for_both_minimums_keeps_the_control_bar() {
    assert_close(drawer_fit(300.0, 120.0), DRAWER_MIN, "fit");
    assert_close(drawer_dragged(300.0, -50.0, 120.0), DRAWER_MIN, "drag");
}

#[test]
fn every_drawer_drag_stays_within_its_bounds() {
    let mut room = 200.0;
    while room <= 2200.0 {
        let mut dy = -3000.0;
        while dy <= 3000.0 {
            let h = drawer_dragged(DRAWER_INITIAL, dy, room);
            assert!(h >= DRAWER_MIN - 1e-9, "{h} below the control bar (room {room}, dy {dy})");
            assert!(h <= (room - PANELS_MIN_HEIGHT).max(DRAWER_MIN) + 1e-9, "{h} crowds the panels (room {room}, dy {dy})");
            dy += 53.0;
        }
        room += 29.0;
    }
}

/// A small deterministic generator, so the long random walk below replays exactly.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[test]
fn random_drags_and_window_changes_keep_every_invariant() {
    let mut rng = Lcg(0x5eed_c01d);
    for walk in 0..200 {
        let mut width = DESKTOP_MIN_WIDTH + rng.next() * 3000.0;
        let mut shown = EVERY_SHOWN[(rng.next() * 4.0) as usize % 4];
        let mut pref = Columns::initial(width, shown);
        for step in 0..60 {
            let context = format!("walk {walk} step {step}");
            if rng.next() < 0.2 {
                width = DESKTOP_MIN_WIDTH + rng.next() * 3000.0;
                shown = EVERY_SHOWN[(rng.next() * 4.0) as usize % 4];
            }
            let on_screen = pref.fit(width, shown);
            assert_valid(on_screen, width, shown, &context);
            let rs = resizers(shown);
            let r = rs[(rng.next() * rs.len() as f64) as usize % rs.len()];
            let dx = (rng.next() - 0.5) * 2400.0;
            let d = on_screen.dragged(r, dx, width, shown);
            assert_valid(d, width, shown, &format!("{context}: {r:?} by {dx}"));
            let untouched = match r {
                Resizer::SidebarLeft => d.right == on_screen.right && close(d.center(width, shown), on_screen.center(width, shown)),
                Resizer::LeftCenter => d.sidebar == on_screen.sidebar && d.right == on_screen.right,
                Resizer::CenterRight => d.sidebar == on_screen.sidebar && d.left == on_screen.left,
            };
            assert!(untouched, "{context}: {r:?} by {dx} moved a column it does not border");
            pref = d;
        }
    }
}
