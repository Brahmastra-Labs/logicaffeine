//! Heal: the fairy that puts a file back as it was before the reader changed it (DAT-424).
//!
//! The Heal button in the Studio and on the home page restores the shipped example (or, for a
//! reader's own file, the text it opened with): Collin, "the heal button in the home page and
//! studio just brings the example program to its normal state before the user changed it". It is
//! instant, nothing leaves the page, and Undo brings the reader's version back. [`restore`] decides
//! it; this module is the pure core (the flight's states, the card's words, the pixelation
//! schedule), and the workbench draws it.

/// The fairy, served from the site (the L of Z, her quest form).
pub const FAIRY: &str = "/assets/heal-fairy.png";

/// What the card says once Heal has put the file back.
pub const RESTORED: &str = "Back to how the file was before your changes. Undo brings them back.";

/// What the card says when the file is already as it was.
pub const ALREADY_ORIGINAL: &str = "Nothing to put back. The file is as it was before any changes.";

/// What the card says when there is no earlier version to go back to.
pub const NO_ORIGINAL: &str = "There is no earlier version of this file to go back to.";

/// Heal pressed over `current`, with `original` the shipped example or the text the file opened
/// with: the file goes back to it (the flight remembers the reader's version for Undo), or the card
/// says it already is, or that there is nothing to go back to. The caller writes `original` into
/// the editor when this answers [`Flight::Restored`].
pub fn restore(current: &str, original: Option<&str>) -> Flight {
    match original {
        None => Flight::Failed(NO_ORIGINAL.to_string()),
        Some(original) if same_text(current, original) => Flight::Failed(ALREADY_ORIGINAL.to_string()),
        Some(_) => Flight::Restored { previous: current.to_string() },
    }
}

/// Whether two texts say the same thing line for line, spaces at a line's end and blank lines at
/// the end aside: a file that differs only in whitespace has nothing to put back.
pub fn same_text(before: &str, after: &str) -> bool {
    let norm = |t: &str| t.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_end().to_string();
    norm(before) == norm(after)
}

/// Where the fairy is and what the card shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Flight {
    /// On her button.
    Idle,
    /// Nothing put back: the reason, in the card's place.
    Failed(String),
    /// The file is back as it was. `previous` is the reader's version, for Undo.
    Restored { previous: String },
}

impl Flight {
    /// Done, or closing the card: the editor stays as it is, and the fairy goes home.
    pub fn undo(self) -> Flight {
        Flight::Idle
    }

    /// Undo after a restore: the reader's version to write back into the editor, and the fairy goes
    /// home. Nothing to write back from any other state.
    pub fn take_back(self) -> (Flight, Option<String>) {
        match self {
            Flight::Restored { previous } => (Flight::Idle, Some(previous)),
            _ => (Flight::Idle, None),
        }
    }
}

/// What the dock bar says while the card is up; nothing when the fairy is home.
pub fn narration(flight: &Flight) -> Option<String> {
    match flight {
        Flight::Idle => None,
        Flight::Failed(reason) => Some(reason.clone()),
        Flight::Restored { .. } => Some(RESTORED.to_string()),
    }
}

/// The most characters a restore animates one by one; past that the editor simply shows the file.
pub const PIXEL_LIMIT: usize = 2000;

/// How long, in milliseconds, the characters of a restore take their turns over.
pub const PIXEL_SPAN_MS: f64 = 700.0;

/// Each of `len` characters' turn in the pixelation schedule: the reader's text dissolves in
/// [`pixel_steps`] order, and the file's text, `forming`, comes in the mirror of it, so the last
/// character to go is the first to come.
pub fn pixel_turns(len: usize, seed: u64, forming: bool) -> Vec<usize> {
    let mut turn = vec![0usize; len];
    for (step, &index) in pixel_steps(len, seed).iter().enumerate() {
        turn[index] = if forming { len - 1 - step } else { step };
    }
    turn
}

/// When a character with `turn` among `len` starts to change, in whole milliseconds: the turns
/// spread evenly over [`PIXEL_SPAN_MS`], rounded down so the last one still starts inside it.
pub fn pixel_delay_ms(turn: usize, len: usize) -> u32 {
    if len == 0 {
        return 0;
    }
    (turn as f64 * PIXEL_SPAN_MS / len as f64).floor() as u32
}

/// The order in which `len` characters dissolve, deterministic from `seed`, so the two halves of
/// the animation mirror: the new text forms in the reverse of this order.
pub fn pixel_steps(len: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..len).collect();
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).max(1);
    for i in (1..len).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state % (i as u64 + 1)) as usize;
        order.swap(i, j);
    }
    order
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/heal/tests.rs"]
mod tests;
