use super::PixelGrid;

/// Ratios a screen or a zoom gives: Firefox's 90% (60/67, a whole 67 app units per device
/// pixel), the harness's 90%, and the common densities and zooms around them.
const RATIOS: [f64; 11] = [0.5, 0.8, 60.0 / 67.0, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 3.0];
/// Corners in CSS pixels: on the grid, a fraction past it, and where the Studio's tree starts.
const CORNERS: [f64; 8] = [0.0, 0.3, 52.3, 158.6, 177.31, 198.0, 247.25, 1000.123];

/// How far `css` CSS pixels lies from the nearest device pixel's edge, in device pixels.
fn off_grid(css: f64, ratio: f64) -> f64 {
    let device = css * ratio;
    (device - device.round()).abs()
}

#[test]
fn a_device_pixel_is_the_inverse_of_the_ratio() {
    for ratio in RATIOS {
        let grid = PixelGrid::at(ratio, 0.0, 0.0);
        assert!((grid.device_px * ratio - 1.0).abs() < 1e-12, "ratio {ratio}: {grid:?}");
    }
}

#[test]
fn a_corner_on_the_grid_is_not_nudged() {
    assert_eq!(PixelGrid::at(1.0, 0.0, 198.0), PixelGrid { device_px: 1.0, nudge_x: 0.0, nudge_y: 0.0 });
    // 198 CSS px is 297 device px at 150%, and 20 is 16 at 80%.
    for (ratio, at) in [(1.5, 198.0), (0.8, 20.0), (2.0, 0.5)] {
        let grid = PixelGrid::at(ratio, at, at);
        assert_eq!((grid.nudge_x, grid.nudge_y), (0.0, 0.0), "ratio {ratio}, corner {at}");
    }
}

#[test]
fn a_corner_between_device_pixels_moves_to_the_next_one() {
    // The Studio's tree starts 198 CSS px down: at 125% that is 247.5 device px, half a device
    // pixel (0.4 CSS px) short of 248.
    let grid = PixelGrid::at(1.25, 0.0, 198.0);
    assert!((grid.nudge_y - 0.4).abs() < 1e-12, "{grid:?}");
    assert_eq!(grid.nudge_x, 0.0);
    // Chrome at a real 90% lays it out at 178.265625 device px: 0.734375 short of 179.
    let grid = PixelGrid::at(0.9, 0.0, 178.265625 / 0.9);
    assert!((grid.nudge_y * 0.9 - 0.734375).abs() < 1e-9, "{grid:?}");
    // Firefox at 80%: 158.6 device px, 0.4 device px (0.5 CSS px) short of 159.
    let grid = PixelGrid::at(0.8, 198.25, 198.25);
    assert!((grid.nudge_x - 0.5).abs() < 1e-9 && (grid.nudge_y - 0.5).abs() < 1e-9, "{grid:?}");
}

#[test]
fn every_nudged_corner_lands_on_a_device_pixel() {
    for ratio in RATIOS {
        for left in CORNERS {
            for top in CORNERS {
                let grid = PixelGrid::at(ratio, left, top);
                for (at, nudge) in [(left, grid.nudge_x), (top, grid.nudge_y)] {
                    assert!((0.0..grid.device_px).contains(&nudge), "ratio {ratio}, corner {at}: a nudge of {nudge} is not under one device pixel");
                    let off = off_grid(at + nudge, ratio);
                    assert!(off < PixelGrid::ON_GRID, "ratio {ratio}, corner {at}: nudged by {nudge} it lands {off} device px off the grid");
                }
            }
        }
    }
}

#[test]
fn layout_noise_at_a_grid_line_is_no_nudge() {
    // Chrome reported the tree's top at 296.50000762939453 device px at a real 150%: a corner
    // a hair either side of a grid line stays where it is.
    for noise in [-4e-4, -1e-6, 1e-6, 4e-4] {
        let grid = PixelGrid::at(1.5, 0.0, (297.0 + noise) / 1.5);
        assert_eq!(grid.nudge_y, 0.0, "a corner {noise} device px from the grid");
    }
}

#[test]
fn an_unmeasurable_ratio_or_corner_is_the_css_pixel_grid() {
    for ratio in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert_eq!(PixelGrid::at(ratio, 3.3, 4.4), PixelGrid::UNMEASURED, "ratio {ratio}");
    }
    let grid = PixelGrid::at(1.25, f64::NAN, f64::INFINITY);
    assert_eq!((grid.nudge_x, grid.nudge_y), (0.0, 0.0), "{grid:?}");
}
