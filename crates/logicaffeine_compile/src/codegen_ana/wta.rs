//! Winner-take-all — the O7 stretch: the MARGIN CERTIFICATE arrives before the tile does.
//!
//! A WTA circuit picks the largest of n currents; near ties it picks WRONG, because each
//! competitor carries an input-referred offset. The honest theorem: with per-cell offsets
//! bounded by the k·σ envelope, the physical winner equals the ideal winner whenever the
//! certified top-two margin EXCEEDS the resolution band `2·k·σ_offset/s` (both competitors
//! can shift toward each other by k·σ at most). [`wta_winner`] states exactly that — a
//! provable flag, never an unconditional claim — and the lock's fixed-seed Monte-Carlo shows
//! both sides: provable margins never flip, and a near-tie really does flip on real dies.
//! `ArgMax` STAYS in `KNOWN_PLANNED`: the tile lowers when the margin replay reaches the
//! independent checker.

use super::nonideal::{NonIdealityModel, KSIGMA};

/// One judged winner-take-all readout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WtaOutcome {
    /// The ideal argmax.
    pub winner: usize,
    /// The certified top-two margin (math domain).
    pub margin: f64,
    /// The k·σ resolution band the margin must clear (math domain).
    pub band: f64,
    /// TRUE iff the winner is provable on every die inside the modeled envelope.
    pub provable: bool,
}

/// Judge an ideal value vector at block scale `s`: argmax, top-two margin, and whether the
/// margin clears the offset resolution band. `None` for fewer than two competitors.
pub fn wta_winner(values: &[f64], model: &NonIdealityModel, s: f64) -> Option<WtaOutcome> {
    if values.len() < 2 || values.iter().any(|v| !v.is_finite()) || !(s > 0.0) {
        return None;
    }
    let mut idx: Vec<usize> = (0..values.len()).collect();
    idx.sort_by(|a, b| values[*b].partial_cmp(&values[*a]).unwrap());
    let (w, second) = (idx[0], idx[1]);
    let margin = values[w] - values[second];
    let band = (2.0 * KSIGMA * model.offset_sigma_v.0 / s).next_up();
    Some(WtaOutcome {
        winner: w,
        margin,
        band,
        provable: margin > band,
    })
}
