//! What a word prefix went on with last time, for the completion contexts: for each prefix (its hash, case folded) where
//! the byte that followed it last sits, and how many times in a row that byte came again. A prefix only ever spelled one
//! way is predicted as that word, and a name a template repeats as the name it used last.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/completion/tests.rs"]
mod tests;

use crate::entropy::hash3;

/// The contexts a tracker gives.
pub const CONTEXTS: usize = 2;
/// Prefixes are kept in this many slots (a power of two), by their hash.
const SLOTS: usize = 1 << 22;
/// The longest run counted.
const LONGEST_RUN: u8 = 15;

pub struct Completion {
    /// Where in the history the byte that last followed each slot's prefix sits (0: none yet).
    at: Vec<u32>,
    /// How many times in a row that byte came again.
    run: Vec<u8>,
}

impl Default for Completion {
    fn default() -> Self {
        Self::new()
    }
}

impl Completion {
    pub fn new() -> Self {
        Self { at: vec![0; SLOTS], run: vec![0; SLOTS] }
    }

    fn slot(prefix: u64) -> usize {
        (prefix >> 10) as usize & (SLOTS - 1)
    }

    /// Note that the history's last byte followed `prefix`, the word as it was before that byte (0 between words:
    /// nothing is noted).
    pub fn update(&mut self, prefix: u64, history: &[u8]) {
        if prefix == 0 {
            return;
        }
        let k = Self::slot(prefix);
        let now = history.len() - 1;
        let at = self.at[k] as usize;
        let again = at > 0 && at < now && history[at] == history[now];
        self.run[k] = if again { (self.run[k] + 1).min(LONGEST_RUN) } else { 0 };
        self.at[k] = now as u32;
    }

    /// The contexts for the word so far (`prefix`, `len` letters; 0 between words): the byte that followed it last time
    /// with its run and the word's length; and that byte with the run (up to 3) and the prefix itself.
    pub fn contexts(&self, prefix: u64, len: u32, history: &[u8]) -> [u64; CONTEXTS] {
        let k = Self::slot(prefix);
        let at = self.at[k] as usize;
        let (predicted, run) = if prefix != 0 && at > 0 { (u64::from(history[at]) | 256, u64::from(self.run[k])) } else { (0, 0) };
        let len = u64::from(len.min(15));
        [hash3(0x5743_4D31, predicted, run << 4 | len), hash3(0x5743_4D32, predicted << 4 | run.min(3), prefix)]
    }
}
