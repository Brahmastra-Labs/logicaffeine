//! # Context-mixing compression of byte streams
//!
//! The general-purpose compressor: a byte stream is coded one bit at a time, most significant bit first, every
//! bit predicted by many models at once and coded by the arithmetic coder in [`crate::entropy`].
//!
//! **The representation.** Before anything is modeled the encoder looks for structure a byte model cannot
//! see, and changes the representation so it can — each change exactly reversible and named in the header:
//! - DEFLATE streams (ZIP entries, gzip members, zlib streams) that [`deflate`] re-creates bit for bit are
//!   replaced by their inflated bytes ([`streams`]);
//! - a 16-bit image (a byte parity under which neighbouring samples are close, and a row width under which the
//!   sample above is close) is replaced by its median-predictor residuals, high byte first;
//! - otherwise, in 64 KiB blocks that look like x86 code, call and jump targets are made absolute, so calls
//!   to one function become one repeated byte string.
//!
//! **The model.** Each context — orders 1 to 16, sparse lags, words, word pairs and triples, the byte one line
//! and one record up, indirect and text-shape contexts, and, inside an image or code, contexts from the image's
//! neighbours or the instruction decoder in [`x86`] — hashes to a slot holding a bit history and the last bytes
//! that followed it. Each gives the mixer four predictions (its bit history through an adaptive map, the same
//! only while the history is one-sided, its last byte's run, and its last three bytes at this bit position). A
//! match model follows the longest earlier occurrence of the recent bytes, surviving single mismatches. A
//! two-layer gated mixer, its weight sets chosen by the partial byte, the match state, the previous bytes, how
//! many orders are known and the model in use, combines them; five adaptive probability maps refine the result
//! and a last mixer weighs the refinements.
//!
//! Everything is integer-only and every table is sized from the input length, so the decoder rebuilds the
//! encoder's state exactly from the header alone.

use crate::describe::{read_uvarint, write_uvarint};
use crate::entropy::{hash2, hash3, prefetch, stretch, AdaptiveProb, Apm, BitIo, Calibration, Decoder, Encoder, Mixer, Mixer2};

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/cm/tests.rs"]
mod tests;

mod alpha;
mod dmc;
mod lstm;
mod markup;
mod ols;
mod residual;
pub mod deflate;
pub mod lzw;
pub mod streams;
pub mod x86;
use x86::{Field, X86};

/// Hashed context orders (bytes of history), plus the word model.
const ORDERS: [usize; 9] = [1, 2, 3, 4, 5, 6, 8, 12, 16];
/// Sparse contexts — bytes at fixed lags with the nearer bytes skipped, which is where records, tables
/// and binary structures line up: two back, three back, and two-and-four back.
const SPARSE: [&[usize]; 3] = [&[2], &[3], &[2, 4]];
const WORD: usize = ORDERS.len();
/// The word-bigram context: the current word together with the one before it.
const BIGRAM: usize = WORD + 1 + SPARSE.len();
/// The match context: the byte the match model predicts, with its length band — so the bit histories
/// learn what a predicted byte is actually followed by, not only how often the match is right.
const MATCHED: usize = BIGRAM + 1;
const RECORD: usize = MATCHED + 1;
/// Record contexts: the byte one record up with its column, and the bytes one and two records up.
const RECORDS: usize = 2;
/// Line contexts: the byte one line up with the one right of it and the previous byte, the bytes one and two
/// lines up at this column, the column with the previous two bytes, and the bytes above-left and above with the
/// previous two.
const COLUMN: usize = RECORD + RECORDS;
const COLUMNS: usize = 4;
/// Indirect contexts: the bytes that followed a context the previous times it occurred — what this context led to
/// before, as a context for what it leads to now — for the last byte (up to four bytes back), the last two bytes,
/// the top five bits of the last three and the top four bits of the last four, alone and with the context.
const INDIRECT: usize = COLUMN + COLUMNS;
/// Text contexts: the word trigram; the current word with the one two back; the character-class shape of the
/// last eight bytes; the last six letters across word breaks; the current number with the word before; the three
/// words before the current one; the open brackets and quotes; the layout (column, distance to the last capital
/// and the last sentence end); the previous word with how long ago it last occurred; the sentence's first word;
/// the current word's case; the words that followed the previous word, and the previous two, last time; and the
/// markup around the text ([`markup`]): the tag's name with the innermost open element, that element with the one
/// last closed, the tag with its attribute, the two innermost elements with the last byte, and the depth with the
/// bytes since the last bracket.
const INDIRECTS: usize = 10;
const TEXT: usize = INDIRECT + INDIRECTS;
const TEXTS: usize = 35;
/// Word-successor tables: the word that followed a word (and a word pair) the last time.
const NEXT_WORDS: usize = 1 << 16;
/// Word-recurrence table size: where each word (by hash) last ended.
const WORD_POSITIONS: usize = 1 << 16;
/// Binary contexts, live only while the text group is off: single bytes and pairs at the lags where binary
/// records, tables and word-aligned fields line up, and two record contexts — the byte one record up with the last
/// byte, and the byte the field one record up predicts ([`field_carry`]), each with the column; and the place in an
/// aligned 8-byte word with the last two bytes, and with the last byte and the byte a word back.
const BINARY: usize = TEXT + TEXTS;
const BINARIES: usize = 54;
/// The binary contexts' lags: each context hashes the bytes this many back.
const BINARY_LAGS: [&[usize]; 22] = [
    &[1, 3],
    &[1, 4],
    &[4],
    &[5],
    &[6],
    &[7],
    &[8],
    &[1, 5],
    &[4, 8],
    &[3, 2],
    &[2, 4, 6],
    &[6, 8],
    &[4, 3],
    &[5, 4],
    &[6, 5],
    &[7, 6],
    &[3, 5],
    &[5, 7],
    &[1, 6],
    &[3, 6],
    &[8, 7],
    &[1, 3, 5],
];
const CONTEXTS: usize = BINARY + BINARIES;
/// Image contexts, live only when the encoder found a 16-bit image (whose bytes are then residuals of the median
/// predictor): the local activity, the texture of the neighbours against the prediction, the neighbours'
/// residuals, the intensity, and the other predictors (mean, gradient, west, north, north-east edge) relative to
/// the median one; the low byte's contexts also hold the high byte.
const IMAGE: usize = CONTEXTS;
const IMAGES: usize = 12;
/// 8-bit image contexts, live inside the image regions of opened GIF and PNG data (rows of known length): each
/// with the pixel's channel, the neighbours west and north, the north-west and north-east corners, two back in
/// the row and column, the gradient and mean predictions, the position in the row, the byte before, and the exact
/// neighbour combinations a palette image's shapes sit in (west, north and north-east; the four around; the row
/// above to two east; west, two west, north and north-west).
const IMAGE8: usize = CONTEXTS + CODES;
const IMAGE8S: usize = 37;
/// PDF text contexts, live inside the string literals of a PDF text object (`BT` … `ET`): from the text those
/// strings spell, joined across the kerning between fragments — the word so far, it with the word before, the last
/// three and five letters, and the word with the class of the last text byte.
const PDF: usize = IMAGE8 + IMAGE8S;
const PDFS: usize = 5;
const SLOTS: usize = CONTEXTS + if IMAGES > CODES + IMAGE8S + PDFS { IMAGES } else { CODES + IMAGE8S + PDFS };
/// The LSTM at `Max` ([`lstm`]): cells a layer, layers, and the bytes between its training steps.
const LSTM_CELLS: usize = 96;
const LSTM_LAYERS: usize = 2;
const LSTM_HORIZON: usize = 20;
/// The LSTM's stretched probability in bands of 128, keying the first APM with the partial byte.
const LSTM_BANDS: usize = 32;
/// An APM knot's update count: where it starts (the identity curve is worth that many bits) and where it stops (the
/// step settles at `1/(APM_LIMIT + 1.5)`, slow enough to average out noise, quick enough to follow a drifting file).
const APM_START: u16 = 4;
const APM_LIMIT: u16 = 255;

/// A probability map of the model's: `contexts` curves whose points count their updates from [`APM_START`] to
/// [`APM_LIMIT`].
fn counting_apm(contexts: usize) -> Apm {
    Apm::new(contexts).with_counts(APM_START, APM_LIMIT)
}

/// The places a record's column is told apart in by the record selector, and by the selector of the partial byte's
/// agreement with the byte one record up: a record longer than that is scaled down to it.
const RECORD_PLACES: usize = 128;
const AGREE_PLACES: usize = 32;
/// How the first mixer's weight sets settle: a set chosen `n` times steps at `(FLOOR + (16 − FLOOR)·HALF / (HALF + n))`
/// sixteenths of the learning rate — the full rate at first, half way to the floor after `HALF` choices.
const MIX_SETTLE_HALF: u32 = 4096;
const MIX_SETTLE_FLOOR: u32 = 6;
/// The residual histograms' predictors and activity buckets.
const PREDICTORS: usize = 8;
const ACTIVITY: usize = 12;

/// How far back the similarity period is looked for.
const SIMILARITY_LAGS: usize = 1024;

/// The live-slot mask's 64-bit words.
const MASK_WORDS: usize = SLOTS.div_ceil(64);
const _: () = assert!(SLOTS <= 256, "a slot's index is a byte");
/// x86 contexts, in the same slots as the image ones (an input is an image or code, never both), live in the
/// blocks the branch filter switched on: the field and position the decoder puts the next byte at, with the
/// opcode, ModRM byte, prefixes, the instructions before, the operand bytes so far and the byte this opcode
/// last had at this position.
const EXE: usize = CONTEXTS;
const EXES: usize = 16;

/// The DEC Alpha contexts, in the slots the x86 ones use: a stream's code is one or the other.
const ALPHA: usize = CONTEXTS;
const ALPHAS: usize = 18;
/// The code contexts' slots: as many as the larger of the two kinds needs.
const CODES: usize = if EXES > ALPHAS { EXES } else { ALPHAS };
/// Mixer weight sets of the special models: x86 field × position (64) plus one for bytes outside code, and the
/// image's activity × byte phase.
const EXE_SELECT: usize = 65;
const IMAGE_SELECT: usize = 64;
/// An image region's weight sets follow the x86 ones: channel (8, a PNG filter byte the ninth) × activity (8).
const REGION_SELECT: usize = 9 * 8;
const SPECIAL_SELECT: usize = if EXE_SELECT + REGION_SELECT > IMAGE_SELECT { EXE_SELECT + REGION_SELECT } else { IMAGE_SELECT };
/// A byte value recurring at the same distance this many times running sets the record length.
const RECORD_CONFIRM: u32 = 4;
const RECORD_MAX: usize = 4096;
/// The contexts [`Level::Fast`] keeps: orders 1–6, the word, and the match model's predicted byte.
const FAST_CONTEXTS: [usize; 8] = [0, 1, 2, 3, 4, 5, WORD, MATCHED];
/// Bytes a match must share before the match model trusts its pointer, and the longer key of the second
/// candidate table: the most recent place the last 24 bytes occurred is a better pointer than the most recent
/// place the last 6 did, whenever both exist.
const MATCH_MIN: usize = 6;
const MATCH_LONG: usize = 24;
/// The longest match the model tracks, and how far back a new candidate is verified.
const MATCH_CAP: usize = 65535;
const MATCH_VERIFY: usize = 400;
/// A match at least this long survives a mispredicted byte: its pointer steps past the miss, so a repeat with
/// one changed byte keeps predicting.
const MATCH_RECOVER: usize = 16;
/// Length buckets of the match confidence: exact below 16, then by powers of two.
const MATCH_BUCKETS: usize = 28;
const LIMIT: u32 = 1023;
/// Order 2's direct probabilities: `2^DIRECT_BITS` hashed cells.
const DIRECT_BITS: u32 = 22;

/// Whether a context belongs to the text group: live only while the recent bytes are text.
fn is_text_context(id: usize) -> bool {
    id == WORD || id == BIGRAM || (TEXT..TEXT + TEXTS).contains(&id)
}

/// The byte a record's field predicts for the next position of `history`: the 4-byte little-endian field it falls in
/// (aligned in the stream), one record of `r` bytes up, moved by the delta the field's bytes so far show against it —
/// for the field's first byte, the delta between the two records before — so offsets and addresses that grow from
/// record to record carry exactly. `None` for records under four bytes or before two records of history.
fn field_carry(history: &[u8], r: usize) -> Option<u8> {
    let n = history.len();
    let seen = n & 3;
    let start = n - seen;
    if r < 4 || start < 2 * r {
        return None;
    }
    let field = |at: usize| u32::from_le_bytes([history[at], history[at + 1], history[at + 2], history[at + 3]]);
    let up = field(start - r);
    let predicted = if seen == 0 {
        up.wrapping_add(up.wrapping_sub(field(start - 2 * r)))
    } else {
        let low = history[start..].iter().rev().fold(0u32, |v, &b| v << 8 | u32::from(b));
        let bits = 8 * seen as u32;
        let delta = (low.wrapping_sub(up) << (32 - bits)) as i32 >> (32 - bits);
        up.wrapping_add(delta as u32)
    };
    Some((predicted >> (8 * seen)) as u8)
}

/// Whether a context belongs to the binary group: live only while the text group is off.
fn is_binary_context(id: usize) -> bool {
    (BINARY..BINARY + BINARIES).contains(&id)
}

/// The text group switches on once this many of the last 64 bytes are printable ASCII or whitespace, and off
/// again below `TEXT_OFF` (the gap keeps it from flickering at a boundary).
const TEXT_ON: u32 = 56;
const TEXT_OFF: u32 = 48;

#[inline]
fn is_text_byte(b: u8) -> bool {
    matches!(b, 9 | 10 | 13 | 32..=126)
}

/// How hard the model works, chosen by the encoder and named in the header.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Orders 1–6, the word and the match model, each context giving its bit history and its run, two weight
    /// sets and two refinements: the codec for messages, several times faster than `Max`.
    Fast,
    /// Every context and model, each context giving four predictions, five weight sets, five refinements.
    Max,
}

impl Level {
    fn byte(self) -> u8 {
        match self {
            Level::Fast => 1,
            Level::Max => 2,
        }
    }

    fn from_byte(b: u8) -> Option<Self> {
        match b {
            1 => Some(Level::Fast),
            2 => Some(Level::Max),
            _ => None,
        }
    }
}

/// A bit history in one byte: the zeros and ones it has counted (each 0..=15; seeing a bit raises its count and,
/// once the other count is past two, roughly halves the other — so a context that changed its mind recently is
/// not outvoted by its distant past) and the order of the last [`RECENT`] bits it saw, so two histories with the
/// same counts but a different recent order predict apart. Each reachable history is numbered breadth-first from
/// the empty one, state 0.
struct BitHistories {
    next: [[u8; 2]; 256],
    counts: [[u8; 2]; 256],
}

/// How many of its latest bits, in order, a bit history keeps.
const RECENT: u32 = 3;

static HISTORIES: BitHistories = bit_histories();

const fn history_step(key: [u8; 4], bit: bool) -> [u8; 4] {
    let [mut n0, mut n1, recent, len] = key;
    if bit {
        n1 = if n1 < 15 { n1 + 1 } else { 15 };
        if n0 > 2 {
            n0 = (n0 + 2) / 2;
        }
    } else {
        n0 = if n0 < 15 { n0 + 1 } else { 15 };
        if n1 > 2 {
            n1 = (n1 + 2) / 2;
        }
    }
    let mask = ((1u32 << RECENT) - 1) as u8;
    let recent = ((recent << 1) | bit as u8) & mask;
    let len = if (len as u32) < RECENT { len + 1 } else { len };
    [n0, n1, recent, len]
}

const fn bit_histories() -> BitHistories {
    let mut keys = [[0u8; 4]; 256];
    let mut known = 1;
    let mut next = [[0u8; 2]; 256];
    let mut i = 0;
    while i < known {
        let mut b = 0;
        while b < 2 {
            let t = history_step(keys[i], b == 1);
            let mut j = 0;
            while j < known && !(keys[j][0] == t[0] && keys[j][1] == t[1] && keys[j][2] == t[2] && keys[j][3] == t[3]) {
                j += 1;
            }
            if j == known {
                assert!(known < 256, "more bit histories than a byte holds");
                keys[known] = t;
                known += 1;
            }
            next[i][b] = j as u8;
            b += 1;
        }
        i += 1;
    }
    let mut counts = [[0u8; 2]; 256];
    let mut k = 0;
    while k < 256 {
        counts[k] = [keys[k][0], keys[k][1]];
        k += 1;
    }
    BitHistories { next, counts }
}

/// The zeros and ones a bit history has counted.
#[inline]
fn counts(state: u8) -> (u8, u8) {
    let [n0, n1] = HISTORIES.counts[usize::from(state)];
    (n0, n1)
}

/// Whether a bit history has seen both bit values.
#[inline]
fn is_mixed(state: u8) -> bool {
    let (n0, n1) = counts(state);
    n0 != 0 && n1 != 0
}

#[inline]
fn next_state(state: u8, bit: bool) -> u8 {
    HISTORIES.next[usize::from(state)][usize::from(bit)]
}

/// The adaptive map row every (context, bit position) starts from: each history seeded with its counts'
/// estimate, so the first visits already lean right.
fn seeded_maps() -> &'static [AdaptiveProb; 256] {
    static ROW: std::sync::OnceLock<[AdaptiveProb; 256]> = std::sync::OnceLock::new();
    ROW.get_or_init(|| {
        std::array::from_fn(|s| {
            let (n0, n1) = counts(s as u8);
            let mut cell = AdaptiveProb::new();
            for _ in 0..n1.min(4) {
                cell.update_with_limit(true, 2);
            }
            for _ in 0..n0.min(4) {
                cell.update_with_limit(false, 2);
            }
            cell
        })
    })
}

/// The contexts' memory: two hashed tables of 64-byte lines that every context shares, so a context costs one
/// cache line per nibble and one that meets many distinct histories (a long order on binary data) takes the room
/// one that meets few (order 1, or a group that is off) leaves.
///
/// The first nibble's table holds three 21-byte buckets a line: a 16-bit check, the 15 bit histories of the
/// nibble's bit tree, and the context's byte history (the last three bytes that followed it, how many of those
/// exist, and how many times running the latest did). The second nibble's holds four 16-byte buckets a line: a
/// check byte and the 15 histories. A line is the hash's place in the table by multiply-shift (its high bits), the
/// checks come from its low bits. A bucket whose check belongs to another context is cleared before use, so a
/// colliding context starts fresh instead of inheriting a stranger's history; on a miss the bucket whose most-used
/// history (the nibble's first decision) has seen the fewest bits is the one taken.
struct ContextTable {
    first: Vec<[u8; 64]>,
    second: Vec<[u8; 16]>,
}

const FIRST_WAYS: usize = 3;
const FIRST_BUCKET: usize = 21;
/// Where a first-nibble bucket's histories and byte history start.
const FIRST_STATES: usize = 1;
const FIRST_HISTORY: usize = 17;

/// A bucket's place: its line and the byte its states are counted from (a state for nibble prefix `nib` sits
/// at `at + nib`).
#[derive(Clone, Copy, Default)]
struct BucketAt {
    line: u32,
    at: u8,
}

/// A bit history's weight for eviction: how many bits it has seen, roughly.
#[inline]
fn weight(state: u8) -> u8 {
    let (n0, n1) = counts(state);
    n0 + n1
}

impl ContextTable {
    /// A table of `lines` 64-byte lines for each nibble.
    fn new(lines: usize) -> Self {
        let lines = lines.max(1);
        Self { first: vec![[0; 64]; lines], second: vec![[0; 16]; 4 * lines] }
    }

    /// The line of `lines` that `h` falls in: its high bits, scaled.
    #[inline]
    fn line(h: u64, lines: usize) -> usize {
        ((u128::from(h) * lines as u128) >> 64) as usize
    }

    #[inline]
    fn first_line(&self, h: u64) -> usize {
        Self::line(h, self.first.len())
    }

    /// The first of the four 16-byte buckets that make the second-nibble line `h` falls in.
    #[inline]
    fn second_line(&self, h: u64) -> usize {
        4 * Self::line(h, self.second.len() / 4)
    }

    /// Start fetching the line the first or second nibble's lookup for `h` will search.
    #[inline]
    fn prefetch_first(&self, h: u64) {
        prefetch(&self.first[self.first_line(h)]);
    }

    #[inline]
    fn prefetch_second(&self, h: u64) {
        prefetch(&self.second[self.second_line(h)]);
    }

    fn first_bucket(&mut self, h: u64) -> BucketAt {
        let line = self.first_line(h);
        let check = h as u16 | 1;
        let l = &mut self.first[line];
        let mut victim = 0;
        let mut least = u8::MAX;
        for way in 0..FIRST_WAYS {
            let o = way * FIRST_BUCKET;
            if u16::from_le_bytes([l[o], l[o + 1]]) == check {
                return BucketAt { line: line as u32, at: (o + FIRST_STATES) as u8 };
            }
            let w = weight(l[o + FIRST_STATES + 1]);
            if w < least {
                least = w;
                victim = o;
            }
        }
        l[victim..victim + FIRST_BUCKET].fill(0);
        l[victim..victim + 2].copy_from_slice(&check.to_le_bytes());
        BucketAt { line: line as u32, at: (victim + FIRST_STATES) as u8 }
    }

    fn second_bucket(&mut self, h: u64) -> BucketAt {
        let line = self.second_line(h);
        let check = (h >> 16) as u8 | 1;
        let mut victim = line;
        let mut least = u8::MAX;
        for i in line..line + 4 {
            if self.second[i][0] == check {
                return BucketAt { line: i as u32, at: 0 };
            }
            let w = weight(self.second[i][1]);
            if w < least {
                least = w;
                victim = i;
            }
        }
        self.second[victim] = [0; 16];
        self.second[victim][0] = check;
        BucketAt { line: victim as u32, at: 0 }
    }

    #[inline]
    fn state(&self, first: bool, b: BucketAt, nib: usize) -> u8 {
        if first { self.first[b.line as usize][usize::from(b.at) + nib] } else { self.second[b.line as usize][nib] }
    }

    #[inline]
    fn set_state(&mut self, first: bool, b: BucketAt, nib: usize, state: u8) {
        if first {
            self.first[b.line as usize][usize::from(b.at) + nib] = state;
        } else {
            self.second[b.line as usize][nib] = state;
        }
    }

    /// The byte history of the first-nibble bucket at `b`.
    fn history(&self, b: BucketAt) -> ByteHistory {
        let o = usize::from(b.at) - FIRST_STATES + FIRST_HISTORY;
        let h = &self.first[b.line as usize][o..o + 4];
        ByteHistory { bytes: [h[0], h[1], h[2]], run: h[3] & 63, seen: h[3] >> 6 }
    }

    fn set_history(&mut self, b: BucketAt, h: ByteHistory) {
        let o = usize::from(b.at) - FIRST_STATES + FIRST_HISTORY;
        self.first[b.line as usize][o..o + 4].copy_from_slice(&[h.bytes[0], h.bytes[1], h.bytes[2], h.seen << 6 | h.run.min(63)]);
    }
}

/// A context's byte history: the last three bytes that followed it, how many times running the latest did,
/// and how many of the three exist.
#[derive(Clone, Copy, Default)]
struct ByteHistory {
    bytes: [u8; 3],
    run: u8,
    seen: u8,
}

impl ByteHistory {
    fn push(mut self, byte: u8) -> Self {
        self.run = if self.seen > 0 && self.bytes[0] == byte { (self.run + 1).min(63) } else { 1 };
        self.bytes = [byte, self.bytes[0], self.bytes[1]];
        self.seen = (self.seen + 1).min(3);
        self
    }
}

/// Run-model cells per context: run length (16) × bit-position group (4) × whether the byte before differed
/// (2) × the predicted bit (2), then the second-latest byte's cells: differed (2) × bit position (8) × bit (2).
const RUN_CELLS: usize = 16 * 4 * 2 * 2 + 2 * 8 * 2;
/// Byte-history cells per context: whether the bit history is mixed (2) × which of the last three bytes exist
/// and their bits at this position (16) × bit position (8).
const BH_CELLS: usize = 2 * 16 * 8;

/// A stretch of the modeled bytes that is an image: rows of `stride` bytes, the first `lead` of each before the
/// pixels, `bpp` bytes a pixel; `palette` when its values are palette indices (a GIF's), which name colours, so that
/// only whether two are equal means anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Region {
    start: usize,
    end: usize,
    stride: usize,
    bpp: usize,
    lead: usize,
    palette: bool,
}

/// Two 32-bit probabilities of a 1 made one: when both lean the same way, the geometric mean of their probabilities
/// of the other bit; otherwise their mean.
fn agree(a: u32, b: u32) -> u64 {
    let (a, b) = (u64::from(a), u64::from(b));
    const HALF: u64 = 1 << 31;
    const ONE: u64 = 1 << 32;
    if a >= HALF && b >= HALF {
        ONE - ((ONE - a) * (ONE - b)).isqrt()
    } else if a < HALF && b < HALF {
        (a * b).isqrt()
    } else {
        (a + b) / 2
    }
}

/// The image regions every level's streams leave in the modeled bytes, in order: each level's moved past the streams
/// the levels after it opened before them.
fn all_regions(first: &[streams::Stream], deeper: &[Vec<streams::Stream>]) -> Vec<Region> {
    let mut out = regions(first);
    for level in deeper {
        for g in &mut out {
            let shift: isize = level.iter().filter(|s| s.offset + s.compressed <= g.start).map(|s| s.raw as isize - s.compressed as isize).sum();
            g.start = (g.start as isize + shift) as usize;
            g.end = (g.end as isize + shift) as usize;
        }
        out.extend(regions(level));
    }
    out.sort_by_key(|g| g.start);
    out
}

/// The image regions one level's opened streams leave in its opened bytes, in order.
fn regions(found: &[streams::Stream]) -> Vec<Region> {
    let mut delta: isize = 0;
    let mut out = Vec::new();
    for s in found {
        let start = (s.offset as isize + delta) as usize;
        if let Some((stride, bpp, lead)) = s.codec.geometry() {
            if bpp < stride {
                out.push(Region { start, end: start + s.raw, stride, bpp, lead, palette: s.codec.indexed() });
            }
        }
        delta += s.raw as isize - s.compressed as isize;
    }
    out
}

/// A 16-bit image the encoder found: samples start at byte `parity` (0 or 1), rows are `width` samples, and the
/// stream holds each sample high byte first (the encoder swapped the little-endian pairs).
#[derive(Clone, Copy)]
struct Image {
    parity: usize,
    width: usize,
}

struct Model {
    table: ContextTable,
    level: Level,
    /// The context each slot holds: the level's generic contexts, then the image's or the x86 model's.
    ids: [usize; SLOTS],
    /// The slots in three groups: always live, live after text (`always..generic`), and the image's or the x86
    /// model's (`generic..` — live in an image, or inside a code block).
    always: usize,
    /// Where the text group ends and the binary group (`text_end..generic`) begins.
    text_end: usize,
    generic: usize,
    allocated: usize,
    /// The slots live for the next byte, and the same as a bit set.
    live: [u8; SLOTS],
    live_n: usize,
    live_mask: [u64; MASK_WORDS],
    /// How many of the last 64 bytes were text, and whether the text group is on.
    texty: u32,
    text_on: bool,
    special_on: bool,
    /// Where the x86 (or 16-bit image) slots end and the image-region slots (`exe_end..allocated`) begin.
    exe_end: usize,
    regions: Vec<Region>,
    /// The region the next byte is in or before, and whether the next byte is inside it.
    region: usize,
    region_on: bool,
    /// Where the PDF slots start (they run to `allocated`), and the PDF text state: inside a text object, the
    /// string nesting depth there, an escape pending, the kerning number since the last string, and the text the
    /// strings spell (the current word, the one before, the last letters, the class of the last text byte).
    pdf_start: usize,
    pdf_on: bool,
    pdf_object: bool,
    pdf_depth: u32,
    pdf_escape: bool,
    pdf_kern: u64,
    pdf_word: u64,
    pdf_prev_word: u64,
    pdf_letters: u64,
    pdf_class: u64,
    /// Mixer inputs per slot (two at `Fast`, four at `Max`), the stride between their groups, and where the
    /// match model's and the bias inputs sit.
    groups: usize,
    stride: usize,
    match_input: usize,
    /// The DMC forest (at `Max`) and where its inputs sit, after the bias.
    dmc: Option<dmc::Forest>,
    dmc_input: usize,
    /// The residual histograms (at `Max`), where their inputs sit, and whether they are live (outside text).
    histograms: Option<residual::Residuals>,
    residual_input: usize,
    /// The LSTM (at `Max`) and where its input sits.
    lstm: Option<lstm::Lstm>,
    lstm_input: usize,
    x86: X86,
    /// The branch filter's blocks: the x86 model is live inside the switched-on ones.
    exe: Vec<bool>,
    /// The DEC Alpha code spans, the first one not yet behind the coder, and the words around the next byte in one.
    alpha: Vec<alpha::Span>,
    alpha_at: usize,
    words: alpha::Words,
    /// Per Alpha base register, the displacement its last memory instruction used; per opcode, its last word.
    alpha_displacement: [u16; 32],
    alpha_by_opcode: [u32; 64],
    /// Per register (the floating ones from 32), the last word that wrote it.
    alpha_writer: [u32; 64],
    /// The registers Alpha instructions last read or wrote, the latest first, and the last six opcodes.
    alpha_recent: [u8; 4],
    alpha_ops: u64,
    /// The byte each opcode last had at each displacement / immediate position.
    last_operand: Vec<u8>,
    image: Option<Image>,
    /// The image's samples rebuilt so far, and their prediction residuals.
    samples: Vec<u16>,
    /// The image's predictor, in step with the one that made the residuals.
    ols: ols::Ols,
    residuals: Vec<i16>,
    /// The image's mixer weight set: activity bucket and which byte of the sample is next.
    image_select: usize,
    /// Inside x86 code, the last instruction's kind and where in an instruction the next byte falls: the key the
    /// lexical weight-set selector uses there.
    code_lexical: usize,
    /// Inside x86 code, the opcode, the decoder's field and position and the previous instruction's opcode: the key
    /// the hashed-pair weight-set selector uses there.
    code_select: u64,

    /// Each context's run slot for the current byte, and the run it read (count 0 = none).
    /// Each slot's first-nibble bucket for the current byte, where its byte history lives.
    first_at: [BucketAt; SLOTS],
    run_hist: [ByteHistory; SLOTS],
    run_maps: Vec<AdaptiveProb>,
    run_cell: [usize; SLOTS],
    bh_maps: Vec<AdaptiveProb>,
    bh_cell: [usize; SLOTS],
    maps: Vec<AdaptiveProb>,
    order0: Vec<AdaptiveProb>,
    /// Order 1's probabilities by (previous byte, partial byte) at a slow and a fast rate, then order 2's, hashed.
    direct: Vec<AdaptiveProb>,
    direct_at: [usize; 3],
    direct_input: usize,
    match_cells: Vec<AdaptiveProb>,
    mixer: Mixer2,
    apm1: Apm,
    apm2: Apm,
    apm3: Apm,
    apm4: Apm,
    apm5: Apm,
    apm6: Apm,
    /// Which of the latest bits the final prediction put on the wrong side of 1/2, the latest lowest.
    misses: u32,
    apm_mask: usize,
    /// Learns how far to trust each refinement, per partial byte and (at `Max`) per band of the run of correctly
    /// predicted bits: none, 1, 2–3, 4–7, 8–15, 16–31, all of the last 32.
    final_mixer: Mixer,
    /// What the final probability turned out to be worth, per bit position: the coder's probability.
    calibration: Calibration,
    /// The same of the refinements' average: the coder's probability is the two calibrations' mean.
    calibration2: Calibration,
    /// Each context's hash for the current byte (computed at the byte boundary).
    base: [u64; SLOTS],
    /// The bucket each context uses for the current nibble, and the history it read for the current bit.
    bucket: [BucketAt; SLOTS],
    used: [u8; SLOTS],
    /// The current nibble's bits so far behind a leading one (1..=15): the slot inside each bucket.
    nib: usize,
    match_table: Vec<u32>,
    match_long: Vec<u32>,
    match_mask: usize,
    match_ptr: usize,
    match_len: usize,
    /// Which of the last eight predicted bytes the match got wrong (bit 0 = the latest).
    match_miss: u8,
    word: u64,
    prev_word: u64,
    word2: u64,
    word3: u64,
    /// Tokens — runs of letters, digits and a decimal point after a digit, so numbers count as words — the four
    /// latest, and the gaps (the bytes between tokens, a newline read as a space) before the current and the last.
    tokens: [u64; 4],
    in_token: bool,
    /// The binary chart: slot v holds the last three bytes as they were when the byte before last had top bits v.
    chart: [u32; 24],
    /// Per lag d (1..=1024), a decaying average of |x − x[−d]| (×32): the lag with the smallest is the period.
    similarity: Vec<u16>,
    gaps: [u64; 2],
    /// The current line so far and the whole previous line, hashed.
    line_hash: u64,
    prev_line_hash: u64,
    next_after: Vec<u64>,
    next_after2: Vec<u64>,
    /// Open brackets and quotes, innermost lowest, and how deep.
    nest: u64,
    /// Bytes since the last capital letter and the last sentence punctuation.
    since_upper: u32,
    since_punct: u32,
    word_positions: Vec<u32>,
    /// Where the text sits in its markup.
    markup: markup::Markup,
    /// How long ago (log2) the previous word occurred before its latest occurrence.
    word_gap: u64,
    sentence_first: u64,
    in_sentence_words: u32,
    /// The current word's case: first letter upper, any later letter upper.
    case: u64,
    word_len: u32,
    /// Three bits of character class per byte, the latest lowest.
    shape: u64,
    letters: u64,
    number: u64,
    /// Record-length detection: each byte value's last position, the distance it last recurred at, and
    /// how many times running it recurred at that same distance.
    last_seen: Vec<(u32, u32, u32)>,
    record_len: usize,
    /// Where the current line and the one before it start.
    line_start: usize,
    prev_line_start: usize,
    prev2_line_start: usize,
    followed1: Vec<u32>,
    followed2: Vec<u16>,
    /// What followed the top five bits of three bytes, the top four of four, and two letters (case-folded).
    followed3: Vec<u16>,
    followed4: Vec<u16>,
    followed_lower: Vec<u32>,
    history: Vec<u8>,
    c0: u32,
}

impl Model {
    /// A model whose code blocks are x86 (`exe`) or DEC Alpha (`alpha`), at most one of the two.
    fn coded(len: usize, level: Level, image: Option<Image>, exe: Vec<bool>, alpha: Vec<alpha::Span>, regions: Vec<Region>) -> Self {
        let max = level == Level::Max;
        let image = image.filter(|_| max);
        let exe = if max && exe.iter().any(|&b| b) { exe } else { Vec::new() };
        let alpha = if max && image.is_none() && exe.is_empty() { alpha } else { Vec::new() };
        let regions = if max && image.is_none() { regions } else { Vec::new() };
        let special = image.is_some() || !exe.is_empty() || !alpha.is_empty() || !regions.is_empty();
        let mut ids = [0; SLOTS];
        let chosen: Vec<usize> = if max { (0..CONTEXTS).collect() } else { FAST_CONTEXTS.to_vec() };
        let mut ordered: Vec<usize> = chosen.iter().copied().filter(|&c| !is_text_context(c) && !is_binary_context(c)).collect();
        let always = ordered.len();
        ordered.extend(chosen.iter().copied().filter(|&c| is_text_context(c)));
        let text_end = ordered.len();
        ordered.extend(chosen.iter().copied().filter(|&c| is_binary_context(c)));
        let generic = ordered.len();
        ids[..generic].copy_from_slice(&ordered);
        let exe_n = if image.is_some() {
            IMAGES
        } else if !exe.is_empty() {
            EXES
        } else if !alpha.is_empty() {
            ALPHAS
        } else {
            0
        };
        let region_n = if regions.is_empty() { 0 } else { IMAGE8S };
        for i in 0..exe_n {
            ids[generic + i] = CONTEXTS + i;
        }
        for i in 0..region_n {
            ids[generic + exe_n + i] = IMAGE8 + i;
        }
        let exe_end = generic + exe_n;
        let pdf_n = if max && image.is_none() { PDFS } else { 0 };
        for i in 0..pdf_n {
            ids[exe_end + region_n + i] = PDF + i;
        }
        let pdf_start = exe_end + region_n;
        let allocated = pdf_start + pdf_n;
        let groups = if max { 4 } else { 2 };
        let inputs = 1 + groups * allocated + 2 + if max { dmc::Forest::INPUTS + PREDICTORS + 4 } else { 0 };
        let lg = usize::BITS - len.max(1).leading_zeros();
        let bits = lg.saturating_sub(5).clamp(10, 22);
        let apm_bits = (lg + 2).clamp(10, 16);
        let match_bits = lg.clamp(10, 24);
        Self {
            level,
            ids,
            always,
            text_end,
            generic,
            allocated,
            live: std::array::from_fn(|k| k as u8),
            live_n: always,
            live_mask: std::array::from_fn(|w| match always.saturating_sub(64 * w) {
                0 => 0,
                bits if bits >= 64 => u64::MAX,
                bits => (1 << bits) - 1,
            }),
            texty: 0,
            text_on: false,
            special_on: image.is_some(),
            exe_end,
            regions,
            region: 0,
            region_on: false,
            pdf_start,
            pdf_on: false,
            pdf_object: false,
            pdf_depth: 0,
            pdf_escape: false,
            pdf_kern: 0,
            pdf_word: 0,
            pdf_prev_word: 0,
            pdf_letters: 0,
            pdf_class: 0,
            groups,
            stride: allocated,
            match_input: 1 + groups * allocated,
            dmc: max.then(|| dmc::Forest::new(len)),
            dmc_input: 1 + groups * allocated + 2,
            histograms: max.then(|| residual::Residuals::new(PREDICTORS, ACTIVITY)),
            residual_input: 1 + groups * allocated + 2 + dmc::Forest::INPUTS,
            // The direct probabilities are the mixer's last three inputs.
            direct_input: inputs - 3,
            lstm: max.then(|| lstm::Lstm::new(LSTM_CELLS, LSTM_LAYERS, LSTM_HORIZON)),
            lstm_input: 1 + groups * allocated + 2 + dmc::Forest::INPUTS + PREDICTORS,
            // Each slot brings `2^(bits - 2)` lines to the shared table, a PDF slot a quarter of that: it is live
            // only inside PDF strings.
            table: ContextTable::new((0..allocated).map(|k| 1usize << (if k >= pdf_start { bits.saturating_sub(2).max(10) } else { bits } - 2)).sum()),
            x86: X86::new(),
            exe,
            alpha,
            words: alpha::Words::new(),
            alpha_displacement: [0; 32],
            alpha_by_opcode: [0; 64],
            alpha_writer: [0; 64],
            alpha_recent: [0; 4],
            alpha_ops: 0,
            alpha_at: 0,
            last_operand: vec![0; 1024 * 16],
            image,
            samples: Vec::new(),
            ols: ols::Ols::new(),
            residuals: Vec::new(),
            first_at: [BucketAt::default(); SLOTS],
            run_hist: [ByteHistory::default(); SLOTS],
            run_maps: vec![AdaptiveProb::new(); allocated * RUN_CELLS],
            run_cell: [usize::MAX; SLOTS],
            bh_maps: vec![AdaptiveProb::new(); if max { allocated * BH_CELLS } else { 0 }],
            bh_cell: [0; SLOTS],
            maps: seeded_maps().iter().copied().cycle().take(allocated * 256 * 8).collect(),
            order0: vec![AdaptiveProb::new(); 256],
            direct: vec![AdaptiveProb::new(); if max { (2 << 16) + (1 << DIRECT_BITS) } else { 0 }],
            direct_at: [0; 3],
            match_cells: vec![AdaptiveProb::new(); MATCH_BUCKETS * 4 * 8 * 2],
            mixer: if max {
                Mixer2::new(inputs, &[256 * 3, 256, if special { SPECIAL_SELECT * 256 } else { 256 }, (ORDERS.len() + 1) * 256, 8 * 8 * 8, 256 * 8, MATCH_BUCKETS * 2 * 8, 4096, 128 * 8, RECORD_PLACES * 8, 4096, 256, 8 * 8 * 8 * 8, 16 * AGREE_PLACES, 4 * 256], 8).with_learning_rates(3, 2).with_settling(MIX_SETTLE_HALF, MIX_SETTLE_FLOOR)
            } else {
                Mixer2::new(inputs, &[256 * 3, 256], 8).with_settling(MIX_SETTLE_HALF, MIX_SETTLE_FLOOR)
            },
            image_select: 0,
            code_lexical: 0,
            code_select: 0,
            apm1: counting_apm(if max { LSTM_BANDS * 256 } else { 256 }),
            apm2: counting_apm(1 << apm_bits),
            apm3: counting_apm(1 << apm_bits),
            apm4: counting_apm(1 << apm_bits),
            apm5: counting_apm(MATCH_BUCKETS * 2 * 256),
            apm6: counting_apm(8 * 256),
            misses: 0,
            apm_mask: (1 << apm_bits) - 1,
            final_mixer: Mixer::new(if max { 7 } else { 3 }, if max { 7 * 256 } else { 256 }).with_learning_rate(2),
            calibration: Calibration::new(8),
            calibration2: Calibration::new(8),
            base: [0; SLOTS],
            bucket: [BucketAt::default(); SLOTS],
            used: [0; SLOTS],
            nib: 1,
            match_table: vec![0; 1 << match_bits],
            match_long: vec![0; 1 << match_bits],
            match_mask: (1 << match_bits) - 1,
            match_ptr: 0,
            match_len: 0,
            match_miss: 0,
            word: 0,
            prev_word: 0,
            word2: 0,
            word3: 0,
            tokens: [0; 4],
            in_token: false,
            chart: [0; 24],
            similarity: vec![u16::MAX / 2; SIMILARITY_LAGS + 1],
            gaps: [0; 2],
            line_hash: 0,
            prev_line_hash: 0,
            next_after: vec![0; NEXT_WORDS],
            next_after2: vec![0; NEXT_WORDS],
            nest: 0,
            since_upper: 0,
            since_punct: 0,
            word_positions: vec![0; WORD_POSITIONS],
            markup: markup::Markup::new(),
            word_gap: 0,
            sentence_first: 0,
            in_sentence_words: 0,
            case: 0,
            word_len: 0,
            shape: 0,
            letters: 0,
            number: 0,
            last_seen: vec![(0, 0, 0); 256],
            record_len: 0,
            line_start: 0,
            prev_line_start: 0,
            prev2_line_start: 0,
            followed1: vec![0; 256],
            followed2: vec![0; 1 << 16],
            followed3: vec![0; 1 << 15],
            followed4: vec![0; 1 << 16],
            followed_lower: vec![0; 1 << 16],
            history: Vec::with_capacity(len),
            c0: 1,
        }
    }

    /// The predicted next bit of the match model's byte, if the match is live and agrees with the bits
    /// of the current byte seen so far.
    #[inline]
    fn expected_bit(&self) -> Option<bool> {
        if self.match_len == 0 {
            return None;
        }
        let predicted = u32::from(self.history[self.match_ptr]) | 256;
        let depth = 32 - self.c0.leading_zeros() - 1;
        (predicted >> (8 - depth) == self.c0).then(|| (predicted >> (7 - depth)) & 1 == 1)
    }

    /// Slot `j`'s context hash, made its context's own in the shared table: two contexts whose hashes agree never
    /// read each other's histories.
    #[inline]
    fn slot_hash(&self, j: usize) -> u64 {
        let id = self.ids[j];
        self.base[id] ^ (id as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    /// The third weight-set selector: where the bytes are a coded representation's or an image region's, its own
    /// context with the partial byte; elsewhere the byte two back.
    #[inline]
    fn third_select(&self, c2: usize) -> usize {
        if self.image.is_some() || !self.exe.is_empty() || !self.alpha.is_empty() || self.region_on {
            self.image_select * 256 + self.c0 as usize
        } else {
            c2
        }
    }

    /// The record selectors at bit `depth` of the next byte after `c1`: the column in the record (in [`RECORD_PLACES`]
    /// places) by the bit position; how the partial byte agrees with the byte one record up (the top nibble of their
    /// difference) by the column in [`AGREE_PLACES`] places; the position in a 4-byte word by the last byte. With no
    /// record found, the column is 0 and the byte one record up is 0.
    fn record_selectors(&self, depth: usize, c1: usize) -> [usize; 3] {
        let n = self.history.len();
        let r = self.record_len;
        let place = |places: usize| if r > 0 { (n % r) * places / r.max(places) } else { 0 };
        let up = if r > 0 { n.checked_sub(r).map_or(0, |i| usize::from(self.history[i])) } else { 0 };
        let partial = (self.c0 as usize) << (8 - depth) & 0xFF;
        [place(RECORD_PLACES) * 8 + depth, ((up ^ partial) >> 4) | place(AGREE_PLACES) << 4, (n & 3) * 256 + c1]
    }

    fn code_bit<I: BitIo>(&mut self, io: &mut I, bit: bool) -> bool {
        let first = self.c0 < 16;
        if self.nib == 1 {
            let c0 = u64::from(self.c0);
            if first {
                for k in 0..self.live_n {
            let j = usize::from(self.live[k]);
                    let b = self.table.first_bucket(hash2(self.slot_hash(j), c0));
                    self.bucket[j] = b;
                    self.first_at[j] = b;
                    self.run_hist[j] = self.table.history(b);
                }
            } else {
                for k in 0..self.live_n {
            let j = usize::from(self.live[k]);
                    self.bucket[j] = self.table.second_bucket(hash2(self.slot_hash(j), c0));
                }
            }
        } else if (4..8).contains(&self.nib) && first {
            // The first nibble's last bit is next: the second nibble's line is one of two, fetch both.
            let c0 = u64::from(self.c0) << 1;
            for k in 0..self.live_n {
            let j = usize::from(self.live[k]);
                let base = self.slot_hash(j);
                self.table.prefetch_second(hash2(base, c0));
                self.table.prefetch_second(hash2(base, c0 | 1));
            }
        }
        for k in 0..self.live_n {
            let j = usize::from(self.live[k]);
            let state = self.table.state(first, self.bucket[j], self.nib);
            self.used[j] = state;
            let p = self.maps[(j * 8 + (31 - self.c0.leading_zeros()) as usize) * 256 + usize::from(state)].p();
            let st = if state == 0 { 0 } else { stretch(p) };
            let mixed = is_mixed(state);
            self.mixer.set(1 + j, st);
            if self.groups == 4 {
                self.mixer.set(1 + 2 * self.stride + j, if mixed { 0 } else { st });
            }
        }
        let depth_bits = 31 - self.c0.leading_zeros();
        let bp = depth_bits as usize;
        let group = [0, 1, 2, 2, 2, 3, 3, 3][bp];
        for k in 0..self.live_n {
            let j = usize::from(self.live[k]);
            let h = self.run_hist[j];
            let first = u32::from(h.bytes[0]) | 256;
            let second = u32::from(h.bytes[1]) | 256;
            self.run_cell[j] = usize::MAX;
            let mut input = 0;
            if h.seen >= 1 && first >> (8 - depth_bits) == self.c0 {
                let e = ((first >> (7 - depth_bits)) & 1) as usize;
                let differed = usize::from(h.seen >= 2 && h.bytes[1] != h.bytes[0]);
                let run = usize::from(h.run).min(15);
                let cell = j * RUN_CELLS + ((run * 4 + group) * 2 + differed) * 2 + e;
                self.run_cell[j] = cell;
                input = stretch(self.run_maps[cell].p());
            } else if h.seen >= 2 && second >> (8 - depth_bits) == self.c0 {
                let e = ((second >> (7 - depth_bits)) & 1) as usize;
                let differed = usize::from(h.seen >= 3 && h.bytes[2] != h.bytes[1]);
                let cell = j * RUN_CELLS + 256 + (differed * 8 + bp) * 2 + e;
                self.run_cell[j] = cell;
                input = stretch(self.run_maps[cell].p());
            }
            self.mixer.set(1 + self.stride + j, input);
            if self.groups < 4 {
                continue;
            }
            let bit_of = |b: u8| usize::from((b >> (7 - depth_bits)) & 1);
            let bh_state = match h.seen {
                0 => 0,
                1 => 2 | bit_of(h.bytes[0]),
                2 => 4 | bit_of(h.bytes[0]) | bit_of(h.bytes[1]) << 1,
                _ => 8 | bit_of(h.bytes[0]) | bit_of(h.bytes[1]) << 1 | bit_of(h.bytes[2]) << 2,
            };
            let state = self.used[j];
            let mixed = usize::from(is_mixed(state));
            let cell = j * BH_CELLS + (mixed * 16 + bh_state) * 8 + bp;
            self.bh_cell[j] = cell;
            self.mixer.set(1 + 3 * self.stride + j, if h.seen == 0 { 0 } else { stretch(self.bh_maps[cell].p()) });
        }
        self.mixer.set(0, stretch(self.order0[self.c0 as usize].p()));
        let expected = self.expected_bit();
        let depth = (31 - self.c0.leading_zeros()) as usize;
        let misses = (self.match_miss.count_ones() as usize).min(3);
        let match_cell = expected.map(|e| ((match_bucket(self.match_len) * 4 + misses) * 8 + depth) * 2 + usize::from(e));
        self.mixer.set(self.match_input, match_cell.map_or(0, |c| stretch(self.match_cells[c].p())));
        self.mixer.set(self.match_input + 1, 256);

        // Weight sets by the partial byte and the match state (none, short, long): how much to trust the
        // match model and the long orders depends on whether a long repeat is in progress.
        let match_band = match self.match_len {
            0 => 0,
            1..=15 => 1,
            _ => 2,
        };
        let c1 = usize::from(self.history.last().copied().unwrap_or(0));
        let n = self.history.len();
        let c2 = n.checked_sub(2).map_or(0, |i| usize::from(self.history[i]));
        let apm_mask = self.apm_mask;
        if self.level == Level::Fast {
            let pm = self.mixer.mix(&[self.c0 as usize + 256 * match_band, c1], depth);
            let p1 = self.apm1.refine(pm, self.c0 as usize);
            let p2 = self.apm2.refine(pm, ((self.c0 as usize) | c1 << 8) & apm_mask);
            for (i, q) in [pm, p1, p2].into_iter().enumerate() {
                self.final_mixer.set(i, stretch(q));
            }
            let p = self.final_mixer.mix(self.c0 as usize);
            let bit = io.code_fine(bit, self.calibration.p(p, depth));
            self.learn(bit, match_cell);
            self.apm1.update(bit);
            self.apm2.update(bit);
            self.final_mixer.update(bit);
            self.calibration.update(bit);
            self.c0 = (self.c0 << 1) | u32::from(bit);
            return bit;
        }
        if let Some(forest) = &self.dmc {
            for i in 0..dmc::Forest::INPUTS {
                self.mixer.set(self.dmc_input + i, forest.input(i));
            }
        }
        if let Some(histograms) = &self.histograms {
            for i in 0..PREDICTORS {
                self.mixer.set(self.residual_input + i, if self.text_on { 0 } else { histograms.input(i, self.c0, depth as u32) });
            }
        }
        let lstm = self.lstm.as_ref().map(|net| net.input(self.c0));
        if let Some(input) = lstm {
            self.mixer.set(self.lstm_input, input);
        }
        let c0 = self.c0 as usize;
        self.direct_at = [c1 << 8 | c0, 1 << 16 | c1 << 8 | c0, (2 << 16) + (hash3(0x4449_5232, (c2 as u64) << 8 | c1 as u64, c0 as u64) as usize & ((1 << DIRECT_BITS) - 1))];
        for (i, &at) in self.direct_at.iter().enumerate() {
            self.mixer.set(self.direct_input + i, stretch(self.direct[at].p()));
        }
        let known = self.used[..ORDERS.len()].iter().filter(|&&s| s != 0).count();
        let third = self.third_select(c2);
        let lexical = if self.special_on && (!self.exe.is_empty() || !self.alpha.is_empty()) {
            self.code_lexical * 8 + depth
        } else {
            (self.word_len.min(7) as usize * 8 + byte_class(c1 as u8) as usize) * 8 + depth
        };
        let matched = (match_bucket(self.match_len) * 2 + expected.map_or(0, usize::from)) * 8 + depth;
        let pair = if self.special_on && (!self.exe.is_empty() || !self.alpha.is_empty()) {
            (hash3(0x4F50_5345, self.code_select, u64::from(self.c0)) >> 52) as usize
        } else {
            (hash3(0x5345_4C32, (c2 as u64) << 8 | c1 as u64, u64::from(self.c0)) >> 52) as usize
        };
        let col = (self.history.len() - self.line_start).min(127) * 8 + depth;
        let [rec, above, lane] = self.record_selectors(depth, c1);
        let c3 = n.checked_sub(3).map_or(0, |i| usize::from(self.history[i]));
        let top = ((depth * 8 + (c1 >> 5)) * 8 + (c2 >> 5)) * 8 + ((self.c0 as usize) << (8 - depth) >> 5 & 7);
        let pm = self.mixer.mix(
            &[self.c0 as usize + 256 * match_band, c1, third, known * 256 + self.c0 as usize, lexical, c1 * 8 + depth, matched, pair, col, rec, (hash2(self.shape & 0xFFF, u64::from(self.c0)) >> 52) as usize, c3, top, above, lane],
            depth,
        );
        let band = lstm.map_or(0, |input| ((input + 2048) >> 7) as usize);
        let p1 = self.apm1.refine(pm, band * 256 + self.c0 as usize);
        let p2 = self.apm2.refine(pm, ((self.c0 as usize) | c1 << 8) & apm_mask);
        let p3 = self.apm3.refine(pm, hash3(0x4150_4D33, (c2 as u64) << 8 | c1 as u64, u64::from(self.c0)) as usize & apm_mask);
        let p4 = self.apm4.refine(pm, hash2(self.base[3], u64::from(self.c0)) as usize & apm_mask);
        let p5 = self.apm5.refine(pm, (match_bucket(self.match_len) * 2 + expected.map_or(0, usize::from)) * 256 + self.c0 as usize);
        let this_byte = self.misses & ((1 << depth) - 1);
        let misses3 = (self.misses & 1) as usize | usize::from(this_byte >> 1 != 0) << 1 | usize::from((self.misses >> depth) & 0xFF != 0) << 2;
        let p6 = self.apm6.refine(pm, misses3 * 256 + self.c0 as usize);
        for (i, q) in [pm, p1, p2, p3, p4, p5, p6].into_iter().enumerate() {
            self.final_mixer.set(i, stretch(q));
        }
        let run = self.misses.trailing_zeros();
        let band = (32 - run.leading_zeros()) as usize;
        let p = self.final_mixer.mix(self.c0 as usize + 256 * band);
        let refined = ((u32::from(p1) + u32::from(p2) + u32::from(p3) + u32::from(p4) + u32::from(p5)) / 5) as u16;
        let fine = agree(self.calibration.p(p, depth), self.calibration2.p(refined, depth));
        let bit = io.code_fine(bit, fine as u32);
        self.learn(bit, match_cell);
        self.apm1.update(bit);
        self.apm2.update(bit);
        self.apm3.update(bit);
        self.apm4.update(bit);
        self.apm5.update(bit);
        self.apm6.update(bit);
        self.misses = self.misses << 1 | u32::from((p >= 2048) != bit);
        self.final_mixer.update(bit);
        self.calibration.update(bit);
        self.calibration2.update(bit);
        self.c0 = (self.c0 << 1) | u32::from(bit);
        bit
    }

    /// Update every context, map and the mixer with the coded bit.
    fn learn(&mut self, bit: bool, match_cell: Option<usize>) {
        let first = self.c0 < 16;
        for k in 0..self.live_n {
            let j = usize::from(self.live[k]);
            let state = self.used[j];
            self.maps[(j * 8 + (31 - self.c0.leading_zeros()) as usize) * 256 + usize::from(state)].update_with_limit(bit, LIMIT);
            self.table.set_state(first, self.bucket[j], self.nib, next_state(state, bit));
        }
        self.nib = if self.nib >= 8 { 1 } else { self.nib * 2 + usize::from(bit) };
        self.order0[self.c0 as usize].update_with_limit(bit, LIMIT);
        for k in 0..self.live_n {
            let j = usize::from(self.live[k]);
            if self.run_cell[j] != usize::MAX {
                self.run_maps[self.run_cell[j]].update_with_limit(bit, LIMIT);
            }
            if self.groups == 4 && self.run_hist[j].seen > 0 {
                self.bh_maps[self.bh_cell[j]].update_with_limit(bit, LIMIT);
            }
        }
        if let Some(c) = match_cell {
            self.match_cells[c].update_with_limit(bit, LIMIT);
        }
        if let Some(forest) = &mut self.dmc {
            forest.update(bit);
        }
        if self.level == Level::Max {
            let [a, b, c] = self.direct_at;
            self.direct[a].update_with_limit(bit, LIMIT);
            self.direct[b].update_with_limit(bit, 30);
            self.direct[c].update_with_limit(bit, LIMIT);
        }
        self.mixer.update(bit);
    }

    fn code_byte<I: BitIo>(&mut self, io: &mut I, byte: u8) -> u8 {
        for i in (0..8).rev() {
            self.code_bit(io, (byte >> i) & 1 == 1);
        }
        let byte = (self.c0 & 0xFF) as u8;
        self.c0 = 1;
        self.end_of_byte(byte);
        byte
    }

    /// Called at every byte boundary in an image: once a sample's two bytes are in, rebuild the sample from its
    /// residual, then set the image contexts for the next byte from the rebuilt samples and their residuals.
    fn image_contexts(&mut self, image: Image) {
        let n = self.history.len();
        let w = image.width;
        let Some(rel) = n.checked_sub(image.parity) else {
            return;
        };
        let k = rel / 2;
        if rel % 2 == 0 && k > 0 {
            let at = n - 2;
            let code = u16::from(self.history[at]) << 8 | u16::from(self.history[at + 1]);
            let residual = unfold_residual(code);
            let f = ols::Ols::features(&self.samples, k - 1, w);
            let sample = self.ols.predict(&f).wrapping_add(residual as u16);
            self.samples.push(sample);
            self.ols.update(&f, sample);
            self.residuals.push(residual);
        }
        let s = |back: usize| if k < back { 0 } else { i32::from(self.samples[k - back]) };
        let r = |back: usize| if k < back { 0 } else { i32::from(self.residuals[k - back]) };
        let (west, north, nw, ne, ww, nn, nne) = (s(1), s(w), s(w + 1), s(w - 1), s(2), s(2 * w), s(2 * w - 1));
        let predicted = i32::from(self.ols.predict(&ols::Ols::features(&self.samples, k, w)));
        let activity = (west - nw).abs() + (north - nw).abs() + (north - ne).abs() + 2 * (r(1).abs() + r(w).abs());
        let qa = u64::from(32 - (activity as u32).leading_zeros());
        let texture = [west, north, nw, ne, ww, nn, 2 * north - nn, 2 * west - ww]
            .iter()
            .enumerate()
            .fold(0u64, |t, (i, &v)| t | u64::from(v > predicted) << i);
        let q = |v: i32| -> u64 {
            let m = (32 - (v.unsigned_abs()).leading_zeros()).min(7) as i64;
            (if v < 0 { -m } else { m } + 8) as u64
        };
        let phase = if rel % 2 == 0 { 0u64 } else { 256 | u64::from(self.history[n - 1]) };
        let ctx = [
            qa,
            qa << 8 | texture,
            q(r(1)) << 4 | q(r(w)),
            qa << 8 | q(r(1)) << 4 | q(r(w)),
            texture << 8 | q(r(1)) << 4 | q(r(w)),
            ((predicted >> 9) as u64) << 8 | qa,
            qa << 8 | q(r(w - 1)) << 4 | q(r(w + 1)),
            qa << 8 | q(r(2)) << 4 | q(r(1)),
            ((west + north + 1) / 2 - predicted).clamp(-255, 255) as u64,
            ((west - predicted).clamp(-127, 127) as u64 & 0xFF) << 8 | ((north - predicted).clamp(-127, 127) as u64 & 0xFF),
            ((west + north - nw - predicted).clamp(-255, 255) as u64 & 0x3FF) << 4 | qa,
            ((north + ne - nne - predicted).clamp(-255, 255) as u64 & 0x3FF) << 4 | qa,
        ];
        for (i, &c) in ctx.iter().enumerate() {
            self.base[IMAGE + i] = hash3(0x494D_4730 + i as u64, phase, c);
        }
        self.image_select = (qa as usize).min(31) * 2 + rel % 2;
    }

    /// Whether the byte at `at` (never behind the last one asked about) falls in an Alpha span, and whether it is the
    /// span's first byte.
    fn alpha_word(&mut self, at: usize) -> (bool, bool) {
        while self.alpha.get(self.alpha_at).is_some_and(|s| s.end() <= at) {
            self.alpha_at += 1;
        }
        match self.alpha.get(self.alpha_at) {
            Some(s) if s.start <= at => (true, at == s.start),
            _ => (false, false),
        }
    }

    /// Follow DEC Alpha code over `byte`, then switch the Alpha contexts on for the next byte when it falls in an Alpha
    /// span (or off, clearing their mixer inputs) and set them from the words around it and the instructions that last
    /// wrote its registers, and the code selectors from its place in the word and its opcode's class.
    fn alpha_contexts(&mut self, byte: u8) {
        let n = self.history.len();
        if self.alpha_word(n - 1).0 {
            self.words.push(byte);
            if self.words.position() == 0 {
                let w = alpha::natural(self.words.previous(0));
                let op = (w >> 26) as usize;
                if matches!(op, 0x08..=0x0F | 0x20..=0x2F) {
                    self.alpha_displacement[(w >> 16 & 31) as usize] = w as u16;
                }
                self.alpha_by_opcode[op] = w;
                if let Some(r) = alpha::destination(w) {
                    self.alpha_writer[r] = w;
                }
                for r in alpha::reads(w).into_iter().flatten().chain(alpha::destination(w)) {
                    let r = r as u8;
                    let at = self.alpha_recent.iter().position(|&x| x == r).unwrap_or(3);
                    self.alpha_recent.copy_within(0..at, 1);
                    self.alpha_recent[0] = r;
                }
                self.alpha_ops = (self.alpha_ops << 6 | u64::from(w >> 26)) & ((1 << 36) - 1);
            }
        }
        let (inside, first) = self.alpha_word(n);
        if first {
            self.words.restart();
        }
        self.special_on = inside;
        if !inside {
            self.image_select = EXE_SELECT - 1;
            return;
        }
        let w = &self.words;
        let at = w.position() as u64;
        let now = at << 32 | u64::from(w.current());
        let (w1, w2, w3) = (u64::from(alpha::natural(w.previous(0))), u64::from(alpha::natural(w.previous(1))), u64::from(alpha::natural(w.previous(2))));
        let lead = if at == 0 { 0 } else { u64::from(w.current() >> (8 * (at - 1))) | 256 };
        let ctx = [
            hash2(0x414C_5030, now),
            hash3(0x414C_5031, now, w1 >> 26),
            hash3(0x414C_5032, now, w1 << 2 | (n as u64 >> 2) & 3),
            hash3(0x414C_5033, now, (w1 >> 26) << 12 | (w2 >> 26) << 6 | w3 >> 26),
            hash3(0x414C_5034, now, w2),
            hash3(0x414C_5035, now, w1 >> 16 & 0x3FF),
            hash3(0x414C_5036, now, w1 & 0xFFFF),
            hash3(0x414C_5037, at << 9 | lead, (w1 >> 26) << 6 | w2 >> 26),
            if at >= 2 {
                hash3(0x414C_5038, now, u64::from(self.alpha_displacement[(w.current() >> (8 * (at - 2)) & 31) as usize]))
            } else {
                hash2(0x414C_5038, now)
            },
            if at >= 1 { hash3(0x414C_5039, now, u64::from(self.alpha_by_opcode[(lead >> 2 & 63) as usize])) } else { hash3(0x414C_5039, now, w1) },
            hash3(0x414C_503A, now, (w1 >> 26) << 5 | w1 & 31),
            hash3(0x414C_503B, now, (w1 >> 21 & 31) << 5 | w2 >> 21 & 31),
            hash3(0x414C_503C, now, (w1 >> 26) << 7 | w1 >> 5 & 0x7F),
            hash3(0x414C_503D, now, w3),
            hash3(0x414C_503E, now, alpha::source(w.current(), at as usize).map_or(0, |r| u64::from(self.alpha_writer[r]) | 1 << 32)),
            hash3(0x414C_503F, now, alpha::destination(w1 as u32).map_or(64, |r| r as u64) << 7 | alpha::destination(w2 as u32).map_or(64, |r| r as u64)),
            hash3(0x414C_5040, now, u64::from(u32::from_le_bytes(self.alpha_recent))),
            hash3(0x414C_5041, now, self.alpha_ops),
        ];
        self.base[ALPHA..ALPHA + ALPHAS].copy_from_slice(&ctx);
        self.image_select = at as usize * 16 + if at == 0 { (w1 >> 28) as usize } else { (lead >> 4 & 15) as usize };
        let op = if at == 0 { w1 >> 26 } else { u64::from(w.current()) >> (8 * at - 6) };
        self.code_lexical = at as usize * 16 + alpha::class(op as u32);
        self.code_select = if at == 0 { 1 << 40 | (w1 >> 26) << 6 | w2 >> 26 } else { (at << 32 | u64::from(w.current())) << 6 | w1 >> 26 };
    }

    /// Follow the instruction stream over `byte`, then switch the x86 contexts on for the next byte inside a code
    /// block (or off outside one, clearing their mixer inputs) and set them from the decoder's state.
    fn exe_contexts(&mut self, byte: u8) {
        let field = self.x86.field();
        if matches!(field, Field::Displacement | Field::Immediate) {
            let at = usize::from(self.x86.opcode() & 0x3FF) * 16 + usize::from(field == Field::Immediate) * 8 + usize::from(self.x86.index()).min(7);
            self.last_operand[at] = byte;
        }
        self.x86.push(byte);
        let n = self.history.len();
        let inside = self.exe.get(n >> BRANCH_BLOCK_LOG).copied().unwrap_or(false);
        self.special_on = inside;
        if !inside {
            self.image_select = EXE_SELECT - 1;
            return;
        }
        let x = &self.x86;
        let field = x.field();
        let index = u64::from(x.index());
        let at = u64::from(field as u8) << 8 | index;
        let opcode = u64::from(x.opcode());
        let modrm = u64::from(x.modrm());
        let [p1, p2] = x.previous();
        let (p1, p2) = (u64::from(p1), u64::from(p2));
        let c1 = u64::from(byte);
        let last = if matches!(field, Field::Displacement | Field::Immediate) {
            let i = usize::from(x.opcode() & 0x3FF) * 16 + usize::from(field == Field::Immediate) * 8 + usize::from(x.index()).min(7);
            u64::from(self.last_operand[i]) | 256
        } else {
            0
        };
        let starting = matches!(field, Field::Start);
        let r = x.recent();
        let kinds = x.kinds();
        let place = match field {
            Field::Start | Field::AfterPrefix | Field::Escape | Field::Escape3 => 0,
            Field::ModRm | Field::Sib => 1,
            Field::Displacement => 2,
            Field::Immediate => 3,
        };
        self.code_lexical = (kinds & 15) as usize * 4 + place;
        let ctx = [
            hash3(0x5838_3630, at, opcode << 4 | u64::from(x.prefixes())),
            hash3(0x5838_3631, at, opcode << 16 | modrm),
            if starting { hash3(0x5838_3632, at, p1 << 16 | p2) } else { hash3(0x5838_3632, at, opcode << 16 | p1) },
            hash3(0x5838_3633, at, opcode << 8 | c1),
            hash3(0x5838_3634, at << 32 | opcode << 16 | modrm, u64::from(x.operand())),
            hash3(0x5838_3635, at, opcode << 16 | last),
            hash3(0x5838_3636, at, p1 << 32 | p2 << 16 | opcode),
            hash3(0x5838_3637, at, opcode << 24 | modrm << 8 | u64::from(x.operand() & 0xFF)),
            hash3(0x5838_3638, at, hash3(u64::from(r[0]), u64::from(r[1]), u64::from(r[2]))),
            hash3(0x5838_3639, at << 16 | opcode, hash3(u64::from(r[0]), u64::from(r[1]) << 32 | u64::from(r[2]), u64::from(r[3]) << 32 | u64::from(r[4]))),
            hash3(0x5838_363A, at << 16 | opcode, (u64::from(r[0] >> 16) << 48) | (u64::from(r[1] >> 16) << 32) | (u64::from(r[2] >> 16) << 16) | u64::from(r[3] >> 16)),
            hash3(0x5838_363B, at << 16 | opcode, u64::from(x.operand()) << 32 | u64::from(r[0])),
            hash3(0x5838_363C, at, u64::from(kinds & 0xFFFF)),
            hash3(0x5838_363D, at << 16 | opcode, u64::from(kinds)),
            hash3(0x5838_363E, at, u64::from(kinds & 0xFF) << 16 | u64::from(r[0] >> 4 & 0x1FF)),
            hash3(0x5838_3641, at, (n as u64 & 15) << 8 | c1),
        ];
        self.base[EXE..EXE + EXES].copy_from_slice(&ctx);
        self.image_select = (field as usize) * 8 + usize::from(x.index()).min(7);
        self.code_select = opcode << 16 | at << 32 | p1;
    }

    /// Follow PDF text objects and their string literals over `byte`; inside a string, add the byte to the text
    /// the strings spell. Then switch the PDF contexts on for the next byte inside a string (off elsewhere).
    fn pdf_contexts(&mut self, byte: u8) {
        let n = self.history.len();
        let at = |back: usize| n.checked_sub(back).map_or(b' ', |i| self.history[i]);
        let operator = |a: u8, b: u8| at(3).is_ascii_whitespace() && at(2) == a && at(1) == b;
        if self.pdf_depth == 0 {
            if operator(b'B', b'T') {
                self.pdf_object = true;
            } else if operator(b'E', b'T') {
                self.pdf_object = false;
            }
        }
        if self.pdf_object {
            let in_string = self.pdf_depth > 0;
            if in_string && self.pdf_escape {
                self.pdf_escape = false;
            } else if in_string && byte == b'\\' {
                self.pdf_escape = true;
            } else if byte == b'(' {
                if self.pdf_depth == 0 && self.pdf_kern >= 1000 {
                    self.pdf_prev_word = self.pdf_word;
                    self.pdf_word = 0;
                }
                self.pdf_depth += u32::from(self.pdf_depth < 16);
                self.pdf_kern = 0;
            } else if byte == b')' && in_string {
                self.pdf_depth -= 1;
            }
            if in_string && self.pdf_depth > 0 && byte != b'(' {
                let lower = byte.to_ascii_lowercase();
                if lower.is_ascii_alphabetic() || lower >= 0x80 {
                    self.pdf_word = hash2(self.pdf_word, u64::from(lower));
                    self.pdf_letters = self.pdf_letters << 8 | u64::from(lower);
                } else if self.pdf_word != 0 {
                    self.pdf_prev_word = self.pdf_word;
                    self.pdf_word = 0;
                }
                self.pdf_class = byte_class(byte);
            } else if self.pdf_depth == 0 {
                // A negative kerning of three digits or more between two strings is a word space.
                self.pdf_kern = match byte {
                    b'-' => 1,
                    b'0'..=b'9' if self.pdf_kern > 0 => (self.pdf_kern * 10).min(1000),
                    _ => 0,
                };
            }
        }
        let on = self.pdf_object && self.pdf_depth > 0;
        if on {
            let ctx = [
                self.pdf_word,
                hash2(self.pdf_word, self.pdf_prev_word),
                self.pdf_letters & 0xFF_FFFF,
                self.pdf_letters & 0xFF_FFFF_FFFF,
                hash2(self.pdf_word, self.pdf_class),
            ];
            for (i, &c) in ctx.iter().enumerate() {
                self.base[PDF + i] = hash2(0x5044_4630 + i as u64, c);
            }
        }
        self.pdf_on = on;
    }

    /// Switch the image-region contexts on for the next byte inside a region (off outside one) and set them from
    /// its neighbours in the rows above and before.
    fn region_contexts(&mut self) {
        let n = self.history.len();
        while self.region < self.regions.len() && self.regions[self.region].end <= n {
            self.region += 1;
        }
        let Some(&r) = self.regions.get(self.region).filter(|r| r.start <= n) else {
            self.region_on = false;
            if self.exe.is_empty() {
                self.image_select = EXE_SELECT - 1;
            }
            return;
        };
        self.region_on = true;
        let at = n - r.start;
        let x = at % r.stride;
        let channel = if x < r.lead { 8 } else { ((x - r.lead) % r.bpp).min(7) as u64 };
        let history = &self.history;
        // The pixel `dx` across and `dy` rows up; 256 off the image or not yet coded.
        let px = |dx: isize, dy: usize| -> u64 {
            let across = x as isize + dx * r.bpp as isize;
            let k = (dy * r.stride) as isize - dx * r.bpp as isize;
            if across < 0 || across >= r.stride as isize || k < 1 || k as usize > at { 256 } else { u64::from(history[n - k as usize]) }
        };
        let (w, ww, www) = (px(-1, 0), px(-2, 0), px(-3, 0));
        let (north, nw, ne, nww, nee, neee) = (px(0, 1), px(-1, 1), px(1, 1), px(-2, 1), px(2, 1), px(3, 1));
        let (nn, nnw, nne, nnww, nnee, nnn) = (px(0, 2), px(-1, 2), px(1, 2), px(-2, 2), px(2, 2), px(0, 3));
        let col = x.min(4095) as u64;
        let pair = |a: u64, b: u64| a << 9 | b;
        // Neighbours alone, with the column, in pairs reaching three rows up and three pixels across, and in triples.
        let mut ctx = [
            north, nw, ne, nww, nee, ww, nn,
            pair(w, col) | 1 << 40, pair(north, col) | 2 << 40, pair(nw, col) | 3 << 40, pair(ne, col) | 4 << 40,
            pair(w, north), pair(w, nw), pair(w, ne), pair(w, nee), pair(w, nww), pair(north, nw), pair(north, ne),
            pair(nw, ne), pair(north, nn), pair(nw, nnww), pair(ne, nnee), pair(nw, nww), pair(nw, nnw), pair(ne, nee),
            pair(ne, nne), pair(north, nnw), pair(north, nne), pair(north, nnn), pair(w, www), pair(ww, nee), pair(ww, nn),
            pair(w, neee), pair(pair(w, north), nw), pair(pair(north, nn), nnn), pair(pair(w, ne), nee),
            pair(pair(w, north), ne),
        ];
        let v = |p: u64| if p > 255 { 0 } else { p as i32 };
        let (iw, inorth, inw, ine) = (v(w), v(north), v(nw), v(ne));
        if !r.palette {
            // A grey or colour image's values measure light: its gradient and the neighbours' averages predict.
            ctx[30] = (iw + inorth - inw).clamp(0, 255) as u64 | 1 << 40;
            ctx[31] = ((iw + inorth) / 2) as u64 | (((inorth + ine) / 2) as u64) << 8 | 2 << 40;
        }
        // Each image its own statistics: another image's palette may give the same index another colour.
        for (i, &c) in ctx.iter().enumerate() {
            self.base[IMAGE8 + i] = hash3(0x494D_3830 + i as u64, channel | (self.region as u64) << 8, c);
        }
        // The weight sets: for palette indices by which neighbours are equal, otherwise by the activity.
        let class = if r.palette {
            usize::from(w == north) | usize::from(north == nw) << 1 | usize::from(w == nw) << 2
        } else {
            let activity = ((iw - inorth).abs() + (inorth - inw).abs()) as u32;
            ((32 - activity.leading_zeros()) as usize).min(7)
        };
        self.image_select = EXE_SELECT + channel as usize * 8 + class;
    }

    /// Rebuild the live slot list from the groups' switches, clearing the mixer inputs of slots that leave it.
    fn relive(&mut self) {
        let mut n = 0;
        let mut mask = [0u64; MASK_WORDS];
        let mut add = |range: std::ops::Range<usize>, live: &mut [u8; SLOTS]| {
            for k in range {
                live[n] = k as u8;
                mask[k / 64] |= 1 << (k % 64);
                n += 1;
            }
        };
        add(0..self.always, &mut self.live);
        if self.text_on {
            add(self.always..self.text_end, &mut self.live);
        } else if self.image.is_none() {
            add(self.text_end..self.generic, &mut self.live);
        }
        if self.special_on {
            add(self.generic..self.exe_end, &mut self.live);
        }
        if self.region_on {
            add(self.exe_end..self.pdf_start, &mut self.live);
        }
        if self.pdf_on {
            add(self.pdf_start..self.allocated, &mut self.live);
        }
        for (w, (&was, &now)) in self.live_mask.iter().zip(&mask).enumerate() {
            let mut left = was & !now;
            while left != 0 {
                let j = 64 * w + left.trailing_zeros() as usize;
                left &= left - 1;
                for group in 0..self.groups {
                    self.mixer.set(1 + group * self.stride + j, 0);
                }
            }
        }
        self.live_n = n;
        self.live_mask = mask;
    }

    fn end_of_byte(&mut self, byte: u8) {
        if let Some(forest) = &mut self.dmc {
            forest.end_of_byte(byte);
        }
        if let Some(net) = &mut self.lstm {
            net.update(byte);
        }
        for k in 0..self.live_n {
            let j = usize::from(self.live[k]);
            let h = self.run_hist[j].push(byte);
            self.table.set_history(self.first_at[j], h);
        }
        if self.match_len > 0 {
            let hit = self.history[self.match_ptr] == byte;
            self.match_miss = self.match_miss << 1 | u8::from(!hit);
            if hit {
                self.match_len = (self.match_len + 1).min(MATCH_CAP);
                self.match_ptr += 1;
            } else if self.match_len >= MATCH_RECOVER && self.match_miss.count_ones() <= 3 {
                self.match_len /= 2;
                self.match_ptr += 1;
            } else {
                self.match_len = 0;
                self.match_miss = 0;
            }
        }
        self.history.push(byte);
        let n = self.history.len();
        if byte.is_ascii_alphabetic() {
            if self.word == 0 {
                self.case = u64::from(byte.is_ascii_uppercase());
                self.word_len = 0;
            } else if byte.is_ascii_uppercase() {
                self.case |= 2;
            }
            self.word = hash2(self.word, u64::from(byte.to_ascii_lowercase()));
            self.word_len += 1;
        } else if self.word != 0 {
            let slot = &mut self.word_positions[self.word as usize & (WORD_POSITIONS - 1)];
            let gap = if *slot == 0 { 0 } else { n as u32 - *slot };
            *slot = n as u32;
            self.word_gap = u64::from(32 - gap.leading_zeros());
            if self.in_sentence_words == 0 {
                self.sentence_first = self.word;
            }
            self.in_sentence_words += 1;
            self.next_after[self.prev_word as usize & (NEXT_WORDS - 1)] = self.word;
            self.next_after2[hash2(self.word2, self.prev_word) as usize & (NEXT_WORDS - 1)] = self.word;
            self.word3 = self.word2;
            self.word2 = self.prev_word;
            self.prev_word = self.word;
            self.word = 0;
            self.word_len = 0;
        }
        match byte {
            b'(' | b'[' | b'{' | b'<' => self.nest = self.nest << 8 | u64::from(byte),
            b')' | b']' | b'}' | b'>' => self.nest >>= 8,
            b'"' => {
                if self.nest & 0xFF == u64::from(b'"') {
                    self.nest >>= 8;
                } else {
                    self.nest = self.nest << 8 | u64::from(byte);
                }
            }
            b'.' | b'!' | b'?' => {
                self.since_punct = 0;
                self.in_sentence_words = 0;
            }
            _ => {}
        }
        self.since_punct = self.since_punct.saturating_add(1);
        self.since_upper = if byte.is_ascii_uppercase() { 0 } else { self.since_upper.saturating_add(1) };
        self.shape = (self.shape << 3 | byte_class(byte)) & 0xFF_FFFF;
        if byte.is_ascii_alphabetic() {
            self.letters = (self.letters << 8 | u64::from(byte.to_ascii_lowercase())) & 0xFFFF_FFFF_FFFF;
        }
        self.number = if byte.is_ascii_digit() { hash2(self.number, u64::from(byte)) } else { 0 };
        let prev = n.checked_sub(2).map_or(0, |i| self.history[i]);
        let token_byte = byte.is_ascii_alphanumeric() || (byte == b'.' && prev.is_ascii_digit());
        if token_byte {
            if !self.in_token {
                self.tokens = [0, self.tokens[0], self.tokens[1], self.tokens[2]];
                self.gaps = [0, self.gaps[0]];
                self.in_token = true;
            }
            self.tokens[0] = hash2(self.tokens[0], u64::from(byte.to_ascii_lowercase()));
        } else {
            self.in_token = false;
            self.gaps[0] = hash2(self.gaps[0], u64::from(if byte == b'\n' { b' ' } else { byte }));
        }
        if byte == b'\n' {
            self.prev_line_hash = self.line_hash;
            self.line_hash = 0;
        } else {
            self.line_hash = hash2(self.line_hash, u64::from(byte));
        }
        for (j, &order) in ORDERS.iter().enumerate() {
            let start = n.saturating_sub(order);
            let ctx = self.history[start..].iter().fold(order as u64, |h, &b| h.wrapping_mul(0x100_0000_01B3) ^ u64::from(b));
            self.base[j] = hash2(ctx, j as u64);
        }
        self.base[WORD] = hash3(0x57_4F52_44, self.word, u64::from(byte.is_ascii_alphabetic()));
        for (j, lags) in SPARSE.iter().enumerate() {
            let ctx = lags.iter().fold(0x5350_4152u64 + j as u64, |h, &lag| {
                hash2(h, u64::from(n.checked_sub(lag).map_or(0, |i| self.history[i])))
            });
            self.base[WORD + 1 + j] = ctx;
        }
        self.base[BIGRAM] = hash3(0x4249_4752, self.word, self.prev_word);
        let (last, gap, run) = self.last_seen[usize::from(byte)];
        let d = n as u32 - last;
        let run = if d == gap { run + 1 } else { 0 };
        self.last_seen[usize::from(byte)] = (n as u32, d, run);
        if run >= RECORD_CONFIRM && (2..=RECORD_MAX as u32).contains(&d) {
            self.record_len = d as usize;
        }
        let r = self.record_len;
        let at = |back: usize| u64::from(n.checked_sub(back).filter(|_| back > 0).map_or(0, |i| self.history[i]));
        let column = if r > 0 { (n % r) as u64 } else { 0 };
        self.base[RECORD] = hash3(0x5245_4331, at(r), column << 16 | r as u64);
        self.base[RECORD + 1] = hash3(0x5245_4332, at(r) << 8 | at(2 * r), r as u64);
        if byte == b'\n' {
            self.prev2_line_start = self.prev_line_start;
            self.prev_line_start = self.line_start;
            self.line_start = n;
        }
        let col = n - self.line_start;
        let above = (self.prev_line_start + col < self.line_start)
            .then(|| u64::from(self.history[self.prev_line_start + col]) | 256)
            .unwrap_or(0);
        let line_byte = |start: usize, end: usize, at: usize| -> u64 {
            (start + at < end).then(|| u64::from(self.history[start + at]) | 256).unwrap_or(0)
        };
        let above_right = line_byte(self.prev_line_start, self.line_start, col + 1);
        let above_left = if col > 0 { line_byte(self.prev_line_start, self.line_start, col - 1) } else { 0 };
        let above2 = line_byte(self.prev2_line_start, self.prev_line_start, col);
        let c2 = n.checked_sub(2).map_or(0, |i| u64::from(self.history[i]));
        self.base[COLUMN] = hash3(0x434F_4C31, above << 9 | above_right, u64::from(byte));
        self.base[COLUMN + 1] = hash3(0x434F_4C32, above << 9 | above2, col.min(255) as u64);
        self.base[COLUMN + 2] = hash3(0x434F_4C33, col.min(1023) as u64, c2 << 8 | u64::from(byte));
        self.base[COLUMN + 3] = hash3(0x434F_4C34, above_left << 9 | above, c2 << 8 | u64::from(byte));
        self.base[TEXT] = hash3(0x5452_4931, hash2(self.word, self.prev_word), self.word2);
        self.base[TEXT + 1] = hash3(0x534B_4950, self.word, self.word2);
        self.base[TEXT + 2] = hash2(0x5348_4150, self.shape);
        self.base[TEXT + 3] = hash2(0x4C45_5454, self.letters);
        self.base[TEXT + 4] = hash3(0x4E55_4D42, self.number, self.prev_word);
        self.base[TEXT + 5] = hash3(0x5452_4932, hash2(self.prev_word, self.word2), self.word3 ^ self.word);
        self.base[TEXT + 6] = hash3(0x4E45_5354, self.nest & 0xFF_FFFF, u64::from(byte));
        self.base[TEXT + 7] = hash3(
            0x4C41_594F,
            (col.min(255) as u64) << 8 | u64::from(self.since_upper.min(15)) << 4 | u64::from(32 - self.since_punct.leading_zeros()),
            byte_class(byte),
        );
        self.base[TEXT + 8] = hash3(0x5245_4355, self.prev_word, self.word_gap << 8 | u64::from(byte));
        self.base[TEXT + 9] = hash3(0x5345_4E54, self.sentence_first, self.word);
        self.base[TEXT + 10] = hash3(0x4341_5345, self.word, self.case << 8 | u64::from(byte));
        if let Some(histograms) = &mut self.histograms {
            histograms.update(byte);
        }
        let lags = SIMILARITY_LAGS.min(n.saturating_sub(1));
        let recent = &self.history[n - 1 - lags..n - 1];
        for (score, &old) in self.similarity[1..=lags].iter_mut().zip(recent.iter().rev()) {
            *score = *score - (*score >> 5) + u16::from(byte.abs_diff(old));
        }
        let period = (1..=lags).min_by_key(|&d| self.similarity[d]).unwrap_or(1);
        let back = |k: usize| n.checked_sub(k * period).map_or(0u64, |i| u64::from(self.history[i]));
        let (p1, p2) = (back(1), back(2));
        let ahead = (2 * p1).wrapping_sub(p2) & 0xFF;
        let d = period as u64;
        if let Some(histograms) = &mut self.histograms {
            let at = |back: usize| n.checked_sub(back).map_or(0u8, |i| self.history[i]);
            let (c1, c2, c3, c4) = (at(1), at(2), at(3), at(4));
            let (n1, n2, nw) = (at(period), at(2 * period), at(period + 1));
            let line = |a: u8, b: u8| a.wrapping_mul(2).wrapping_sub(b);
            let activity = u32::from(c1.abs_diff(c2)) + u32::from(c2.abs_diff(c3));
            let bucket = (32 - activity.leading_zeros()) as usize;
            for (i, value) in [c1, line(c1, c2), c2, line(c2, c4), n1, line(n1, n2), n1.wrapping_add(c1).wrapping_sub(nw), c4].into_iter().enumerate() {
                histograms.set(i, value, bucket);
            }
        }
        self.base[BINARY + 46] = hash3(0x5349_4D31, d, p1);
        self.base[BINARY + 47] = hash3(0x5349_4D32, d, p1 << 8 | p2);
        self.base[BINARY + 48] = hash3(0x5349_4D33, d, ahead);
        self.base[BINARY + 49] = hash3(0x5349_4D34, d, p1 << 8 | u64::from(byte));
        let last3 = (0..3).fold(0u32, |w, k| w << 8 | n.checked_sub(3 - k).map_or(0, |i| u32::from(self.history[i])));
        self.chart[(last3 >> 13 & 7) as usize] = last3;
        self.chart[8 + (last3 >> 10 & 7) as usize] = last3;
        self.chart[16 + (last3 >> 8 & 7) as usize] = last3;
        let tops = u64::from(last3 >> 5 & 0x07_0707);
        let fields = [tops, u64::from(last3 >> 2 & 0x07_0707), u64::from(last3 & 0x07_0707)];
        for (v, &k) in self.chart[8..].iter().enumerate() {
            self.base[BINARY + 30 + v] = hash3(0x4348_5255 + v as u64, fields[1 + v / 8], u64::from(k));
        }
        for (v, &k) in self.chart[..8].iter().enumerate() {
            self.base[BINARY + 22 + v] = hash3(0x4348_5254 + v as u64, tops, u64::from(k));
        }
        for (i, lags) in BINARY_LAGS.iter().enumerate() {
            let key = lags.iter().fold(0u64, |k, &lag| k << 8 | u64::from(n.checked_sub(lag).map_or(0, |at| self.history[at])));
            self.base[BINARY + i] = hash2(0x4249_4E30 + i as u64, key);
        }
        self.base[BINARY + 50] = hash3(0x5245_4333, at(r) << 8 | u64::from(byte), column << 16 | r as u64);
        let carried = field_carry(&self.history, r).map_or(0, |b| u64::from(b) | 256);
        self.base[BINARY + 51] = hash3(0x5245_4334, carried << 2 | (n & 3) as u64, column << 16 | r as u64);
        self.base[BINARY + 52] = hash3(0x5157_4431, (n & 7) as u64, at(1) << 8 | at(2));
        self.base[BINARY + 53] = hash3(0x5157_4432, (n & 7) as u64, at(1) << 8 | at(8));
        // What followed the previous word (and the previous pair) last time, with the word so far: whether history
        // repeats, and what to expect when it does not.
        let expected1 = self.next_after[self.prev_word as usize & (NEXT_WORDS - 1)];
        let expected2 = self.next_after2[hash2(self.word2, self.prev_word) as usize & (NEXT_WORDS - 1)];
        self.base[TEXT + 11] = hash3(0x4E58_5431, expected1, self.word);
        self.base[TEXT + 12] = hash3(0x4E58_5432, expected2, self.word);
        let [t0, t1, t2, _] = self.tokens;
        let [g0, g1] = self.gaps;
        let c1 = u64::from(byte);
        self.base[TEXT + 13] = hash3(0x544B_4E30, t0, g0);
        self.base[TEXT + 14] = hash3(0x544B_4E31, t0 << 8 | c1, g1);
        self.base[TEXT + 15] = hash3(0x544B_4E32, g0 << 8 | c1, t1);
        self.base[TEXT + 16] = hash3(0x544B_4E33, t0, t1);
        self.base[TEXT + 17] = hash3(0x544B_4E34, hash2(t0, t1), t2);
        self.base[TEXT + 18] = hash3(0x544B_4E35, hash2(g0, t1), hash2(g1, t2));
        self.base[TEXT + 19] = hash3(0x544B_4E36, hash2(t0, t1), hash2(g1, t2));
        self.base[TEXT + 20] = hash3(0x544B_4E37, hash2(t0, t1), g1);
        self.base[TEXT + 21] = hash3(0x4C49_4E31, self.prev_line_hash, (col.min(1023) as u64) << 8 | c1);
        self.base[TEXT + 22] = hash3(0x4C49_4E32, self.prev_line_hash, self.line_hash);
        self.base[TEXT + 26] = hash3(0x534B_3032, self.word << 8 | c1, self.word2);
        self.base[TEXT + 27] = hash3(0x534B_3033, self.word << 8 | c1, self.word3);
        self.base[TEXT + 28] = hash3(0x534B_3133, self.word << 8 | c1, hash2(self.prev_word, self.word3));
        self.base[TEXT + 29] = hash3(0x534B_3233, self.word << 8 | c1, hash2(self.word2, self.word3));
        self.base[TEXT + 25] = hash3(0x4C4F_4341, (n >> 10) as u64, self.tokens[0] << 8 | c1);
        self.markup.update(byte);
        for (i, &key) in self.markup.contexts().iter().enumerate() {
            self.base[TEXT + 30 + i] = hash3(0x4D41_524B + i as u64, key, if i == 3 { c1 } else { 0 });
        }
        let c1 = usize::from(byte);
        let at = |back: usize| n.checked_sub(back).map_or(0usize, |i| usize::from(self.history[i]));
        let top3 = |b: usize| at(b) >> 3 | (at(b + 1) >> 3) << 5 | (at(b + 2) >> 3) << 10;
        let top4 = |b: usize| at(b) >> 4 | (at(b + 1) >> 4) << 4 | (at(b + 2) >> 4) << 8 | (at(b + 3) >> 4) << 12;
        let lower2 = |b: usize| (at(b + 1) as u8).to_ascii_lowercase() as usize | ((at(b) as u8).to_ascii_lowercase() as usize) << 8;
        let (p3, p4, pl) = (top3(2), top4(2), lower2(2));
        self.followed3[p3] = self.followed3[p3] << 8 | u16::from(byte);
        self.followed4[p4] = self.followed4[p4] << 8 | u16::from(byte);
        self.followed_lower[pl] = self.followed_lower[pl] << 8 | u32::from(byte.to_ascii_lowercase());
        if n >= 2 {
            let p1 = usize::from(self.history[n - 2]);
            self.followed1[p1] = self.followed1[p1] << 8 | u32::from(byte);
            if n >= 3 {
                let p2 = usize::from(self.history[n - 3]) << 8 | p1;
                self.followed2[p2] = self.followed2[p2] << 8 | u16::from(byte);
            }
        }
        let c2 = n.checked_sub(2).map_or(0, |i| usize::from(self.history[i])) << 8 | c1;
        let (h1, h2) = (u64::from(self.followed1[c1]), u64::from(self.followed2[c2]));
        let (d3, d4) = (top3(1), top4(1));
        let (h3, h4) = (u64::from(self.followed3[d3]), u64::from(self.followed4[d4]));
        self.base[INDIRECT] = hash3(0x494E_4431, c1 as u64, h1 & 0xFFFF);
        self.base[INDIRECT + 1] = hash3(0x494E_4432, c2 as u64, h2);
        self.base[INDIRECT + 2] = hash2(0x494E_4433, h1 & 0xFF_FFFF);
        self.base[INDIRECT + 3] = hash2(0x494E_4434, h1);
        self.base[INDIRECT + 4] = hash3(0x494E_4435, d4 as u64, h4);
        self.base[INDIRECT + 5] = hash3(0x494E_4436, c1 as u64, h1 & 0xFF);
        self.base[INDIRECT + 6] = hash2(0x494E_4437, h2 & 0xFF);
        self.base[INDIRECT + 7] = hash3(0x494E_4438, c2 as u64, h2 & 0xFF);
        self.base[INDIRECT + 8] = hash3(0x494E_4439, d3 as u64, h3);
        self.base[INDIRECT + 9] = hash2(0x494E_443A, h4 & 0xFF);
        let hl = u64::from(self.followed_lower[lower2(1)]);
        self.base[TEXT + 23] = hash3(0x494E_444C, hl & 0xFF, u64::from(byte.to_ascii_lowercase()));
        self.base[TEXT + 24] = hash2(0x494E_444D, hl & 0xFF_FFFF);
        if n >= MATCH_MIN {
            let key = hash1_bytes(&self.history[n - MATCH_MIN..]) as usize & self.match_mask;
            let long_key = (n >= MATCH_LONG).then(|| hash1_bytes(&self.history[n - MATCH_LONG..]) as usize & self.match_mask);
            if self.match_len < MATCH_VERIFY || self.match_miss != 0 {
                let candidates = [Some(self.match_table[key]), long_key.map(|k| self.match_long[k])];
                for candidate in candidates.into_iter().flatten() {
                    let candidate = candidate as usize;
                    if candidate == 0 || candidate >= n || candidate == self.match_ptr {
                        continue;
                    }
                    let limit = MATCH_VERIFY.min(candidate);
                    let mut len = 0;
                    while len < limit && self.history[candidate - 1 - len] == self.history[n - 1 - len] {
                        len += 1;
                    }
                    if len >= MATCH_MIN && len > self.match_len {
                        self.match_len = len;
                        self.match_ptr = candidate;
                        self.match_miss = 0;
                    }
                }
            }
            self.match_table[key] = n as u32;
            if let Some(k) = long_key {
                self.match_long[k] = n as u32;
            }
        }
        let predicted = if self.match_len > 0 { u64::from(self.history[self.match_ptr]) | 256 } else { 0 };
        let band = match self.match_len {
            0 => 0,
            1..=15 => 1,
            16..=31 => 2,
            _ => 3,
        };
        self.base[MATCHED] = hash3(0x4D41_5443, predicted, band);
        if let Some(image) = self.image {
            self.image_contexts(image);
        } else if !self.exe.is_empty() {
            self.exe_contexts(byte);
        } else if !self.alpha.is_empty() {
            self.alpha_contexts(byte);
        }
        if !self.regions.is_empty() {
            self.region_contexts();
        }
        if self.pdf_start < self.allocated {
            self.pdf_contexts(byte);
        }
        self.texty += u32::from(is_text_byte(byte));
        if n > 64 {
            self.texty -= u32::from(is_text_byte(self.history[n - 65]));
        }
        self.text_on = if self.text_on { self.texty >= TEXT_OFF } else { self.texty >= TEXT_ON };
        self.relive();
        // Every context's run slot and first-nibble line are known now and read at the next bit: start all the
        // fetches at once, so the misses overlap instead of queueing one after another.
        for k in 0..self.live_n {
            let j = usize::from(self.live[k]);
            self.table.prefetch_first(hash2(self.slot_hash(j), 1));
        }
    }
}

/// A byte's character class: control and space, lower, upper, digit, sentence punctuation, brackets and quotes,
/// other ASCII, and non-ASCII.
#[inline]
fn byte_class(b: u8) -> u64 {
    match b {
        b'a'..=b'z' => 1,
        b'A'..=b'Z' => 2,
        b'0'..=b'9' => 3,
        b'.' | b',' | b';' | b':' | b'!' | b'?' => 4,
        b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'<' | b'>' | b'"' | b'\'' => 5,
        0..=32 => 0,
        128..=255 => 7,
        _ => 6,
    }
}

/// The confidence bucket of a match length: exact below 16, then one bucket per power of two.
#[inline]
fn match_bucket(len: usize) -> usize {
    if len < 16 {
        len
    } else {
        (12 + (usize::BITS - len.leading_zeros()) as usize).min(MATCH_BUCKETS - 1)
    }
}

fn hash1_bytes(b: &[u8]) -> u64 {
    b.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &x| (h ^ u64::from(x)).wrapping_mul(0x100_0000_01B3))
}

/// The x86 branch filter works in blocks of this many bytes, each switched on or off by the encoder.
const BRANCH_BLOCK_LOG: u32 = 16;
/// A block's calls and jumps must mostly carry near targets (top operand byte 0x00 or 0xFF), and at least this
/// many of them, before the filter turns their relative targets into absolute ones.
const BRANCH_MIN: usize = 16;
const FILTER_BRANCH: u8 = 1;
const FILTER_IMAGE16: u8 = 2;
const FILTER_STREAMS: u8 = 4;
const FILTER_ALPHA: u8 = 8;
const FILTER_NESTED: u8 = 16;

/// How many levels of streams are opened in all: a stream's, then the streams its decoded bytes hold (a ZIP entry's
/// GIF image), and one level more.
const NESTED_LEVELS: usize = 3;
/// The image detector looks at this many samples from the middle of the input, and tries row widths up to
/// `IMAGE_MAX_WIDTH` samples on a subsample of `IMAGE_PROBES` positions.
const IMAGE_SPAN: usize = 1 << 17;
const IMAGE_PROBES: usize = 1 << 14;
const IMAGE_MAX_WIDTH: usize = 4096;

/// A 16-bit little-endian image: one byte parity makes neighbouring samples far closer than the other does,
/// and one row width puts a sample's northern neighbour far closer than a typical lag does (a quarter of the
/// median lag's distance; text, code and records sit above four fifths of it).
fn detect_image16(data: &[u8]) -> Option<Image> {
    let total = data.len().saturating_sub(1) / 2;
    if total < 4 * IMAGE_MAX_WIDTH + IMAGE_PROBES {
        return None;
    }
    let span = total.min(IMAGE_SPAN);
    let first = (total - span) / 2;
    let sample = |parity: usize, k: usize| i64::from(u16::from_le_bytes([data[parity + 2 * k], data[parity + 2 * k + 1]]));
    let smooth = |parity: usize| (first + 1..first + span).map(|k| (sample(parity, k) - sample(parity, k - 1)).abs()).sum::<i64>();
    let (even, odd) = (smooth(0), smooth(1));
    let parity = usize::from(odd < even);
    if even.min(odd) * 4 >= even.max(odd) {
        return None;
    }
    let step = (span / IMAGE_PROBES).max(1);
    let probes: Vec<usize> = (0..IMAGE_PROBES).map(|i| first + 2 * IMAGE_MAX_WIDTH + i * step).filter(|&k| k < total).collect();
    let distance = |back: usize| probes.iter().map(|&k| (sample(parity, k) - sample(parity, k - back)).abs()).sum::<i64>();
    let mut lags: Vec<(i64, usize)> = (8..=IMAGE_MAX_WIDTH).map(|w| (distance(w), w)).collect();
    let (north, width) = *lags.iter().min()?;
    let mid = lags.len() / 2;
    let (median, _) = *lags.select_nth_unstable(mid).1;
    (north * 4 < median).then_some(Image { parity, width })
}

/// A 16-bit residual as sign and magnitude, sign in the top bit — so the high byte of a small residual is 0x00
/// or 0x80 and the magnitude's bits follow the sign.
#[inline]
fn fold_residual(r: i16) -> u16 {
    if r >= 0 { r as u16 } else { 0x8000 | (-(i32::from(r) + 1)) as u16 }
}

#[inline]
fn unfold_residual(code: u16) -> i16 {
    if code & 0x8000 == 0 { code as i16 } else { (-i32::from(code & 0x7FFF) - 1) as i16 }
}

/// Replace each little-endian sample from `parity` on by its MED residual, sign-magnitude, high byte first
/// (`forward`), or rebuild the samples from those residuals. A trailing odd byte is left as it is.
fn image_residuals(data: &mut [u8], image: Image, forward: bool) {
    let start = image.parity.min(data.len());
    let count = (data.len() - start) / 2;
    let mut samples: Vec<u16> = Vec::with_capacity(count);
    let mut ols = ols::Ols::new();
    for k in 0..count {
        let at = start + 2 * k;
        let f = ols::Ols::features(&samples, k, image.width);
        let predicted = ols.predict(&f);
        let sample = if forward {
            let sample = u16::from_le_bytes([data[at], data[at + 1]]);
            let code = fold_residual(sample.wrapping_sub(predicted) as i16);
            data[at..at + 2].copy_from_slice(&code.to_be_bytes());
            sample
        } else {
            let code = u16::from_be_bytes([data[at], data[at + 1]]);
            let sample = predicted.wrapping_add(unfold_residual(code) as u16);
            data[at..at + 2].copy_from_slice(&sample.to_le_bytes());
            sample
        };
        samples.push(sample);
        ols.update(&f, sample);
    }
}

/// The blocks whose `E8`/`E9` (call/jump rel32) opcodes look like x86 code: most operands are near targets.
fn branch_blocks(data: &[u8]) -> Vec<bool> {
    let blocks = data.len().div_ceil(1 << BRANCH_BLOCK_LOG);
    let mut near = vec![0usize; blocks];
    let mut all = vec![0usize; blocks];
    let mut i = 0;
    while i + 5 <= data.len() {
        if data[i] == 0xE8 || data[i] == 0xE9 {
            let b = i >> BRANCH_BLOCK_LOG;
            all[b] += 1;
            near[b] += usize::from(data[i + 4] == 0 || data[i + 4] == 0xFF);
            i += 5;
        } else {
            i += 1;
        }
    }
    (0..blocks).map(|b| near[b] >= BRANCH_MIN && near[b] * 2 >= all[b]).collect()
}

/// Rewrite the near targets of `E8`/`E9` (call, jump) and `0F 80..8F` (conditional jump) in the switched-on blocks
/// from relative to absolute (`forward`) or back. A target is near when its top byte is `00` or `FF`; the absolute
/// one, taken modulo 2²⁵ and sign-extended, keeps that property, so the inverse finds exactly the operands the
/// forward pass rewrote. The rewritten operand holds the target's low three bytes most significant first, then the top
/// byte. Opcode bytes are never changed and an operand is never scanned as an opcode, so both directions visit the
/// same instructions: the inverse undoes the forward pass exactly, whatever the bytes are.
fn branch_filter(data: &mut [u8], on: &[bool], forward: bool) {
    let near = |top: u8| top == 0 || top == 0xFF;
    let wide = |v: u32| ((v << 7) as i32 >> 7) as u32;
    let mut i = 0;
    while i + 5 <= data.len() {
        let width = match data[i] {
            0xE8 | 0xE9 => 1,
            0x0F if i + 6 <= data.len() && data[i + 1] & 0xF0 == 0x80 => 2,
            _ => 0,
        };
        if width == 0 || !on[i >> BRANCH_BLOCK_LOG] {
            i += 1;
            continue;
        }
        let o = i + width;
        let at = (o + 4) as u32;
        if near(data[o + 3]) {
            if forward {
                let target = wide(u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]).wrapping_add(at));
                data[o..o + 4].copy_from_slice(&[(target >> 16) as u8, (target >> 8) as u8, target as u8, (target >> 24) as u8]);
            } else {
                let target = u32::from(data[o]) << 16 | u32::from(data[o + 1]) << 8 | u32::from(data[o + 2]) | u32::from(data[o + 3]) << 24;
                data[o..o + 4].copy_from_slice(&wide(target.wrapping_sub(at)).to_le_bytes());
            }
        }
        i = o + 4;
    }
}

/// Code `body` at `level` with a model for an image or code blocks (x86 or DEC Alpha) as given: the arithmetic-coded bits
/// alone.
fn encode_coded(body: &[u8], level: Level, image: Option<Image>, exe: Vec<bool>, alpha: Vec<alpha::Span>, regions: Vec<Region>) -> Vec<u8> {
    let mut model = Model::coded(body.len(), level, image, exe, alpha, regions);
    let mut enc = Encoder::new();
    for &b in body {
        model.code_byte(&mut enc, b);
    }
    enc.finish()
}

/// The streams' headers, every level's, as one block: their length, the coded length, then the bytes under the fast
/// model — a run of small streams repeats its settings field for field, which raw would cost an archive of them a few
/// percent.
fn write_packed(head: &[u8], out: &mut Vec<u8>) {
    let coded = encode_coded(head, Level::Fast, None, Vec::new(), Vec::new(), Vec::new());
    write_uvarint(head.len() as u64, out);
    write_uvarint(coded.len() as u64, out);
    out.extend_from_slice(&coded);
}

/// Read a [`write_packed`] block for an original of `len` bytes: `None` when it is cut short or claims more header
/// than so many bytes could need.
fn read_packed(buf: &[u8], pos: &mut usize, len: usize) -> Option<Vec<u8>> {
    let n = usize::try_from(read_uvarint(buf, pos)?).ok()?;
    let m = usize::try_from(read_uvarint(buf, pos)?).ok()?;
    if n > len.saturating_mul(HEADER_PER_BYTE).saturating_add(1 << 12) {
        return None;
    }
    let coded = buf.get(*pos..pos.checked_add(m)?)?;
    *pos += m;
    let mut model = Model::coded(n, Level::Fast, None, Vec::new(), Vec::new(), Vec::new());
    let mut dec = Decoder::new(coded);
    Some((0..n).map(|_| model.code_byte(&mut dec, 0)).collect())
}

/// The streams a header names for an original of `len` bytes under `filters` — the first level's, the deeper levels',
/// and the bytes the model then codes. `None` when the block is malformed, a level is empty or opens past what the
/// original may, or bytes follow the last level: refused here, before any modeling.
fn read_streams(buf: &[u8], pos: &mut usize, len: usize, filters: u8) -> Option<(Vec<streams::Stream>, Vec<Vec<streams::Stream>>, usize)> {
    if filters & FILTER_STREAMS == 0 {
        return Some((Vec::new(), Vec::new(), len));
    }
    let head = read_packed(buf, pos, len)?;
    let mut at = 0;
    let (found, mut body_len) = streams::read_header(&head, &mut at, len)?;
    let mut deeper = Vec::new();
    if filters & FILTER_NESTED != 0 {
        let count = usize::try_from(read_uvarint(&head, &mut at)?).ok()?;
        if !(1..NESTED_LEVELS).contains(&count) {
            return None;
        }
        for _ in 0..count {
            let (level, opened) = streams::read_header(&head, &mut at, body_len)?;
            if level.is_empty() || opened > streams::expansion_bound(len) {
                return None;
            }
            deeper.push(level);
            body_len = opened;
        }
    }
    (at == head.len()).then_some((found, deeper, body_len))
}

/// The most header bytes a byte of the original can need, every level's streams and their cuts together.
const HEADER_PER_BYTE: usize = 64;

/// A [`BitIo`] that codes nothing and adds up what each bit would cost, in 2⁻²⁴-bit units: fine enough that a
/// near-certain bit is charged what the coder pays for it.
struct CostMeter(u64);

impl BitIo for CostMeter {
    #[inline]
    fn code_fine(&mut self, bit: bool, p32: u32) -> bool {
        let p = f64::from(p32.max(1)) / 4_294_967_296.0;
        self.0 += (-(if bit { p } else { 1.0 - p }).log2() * f64::from(1 << 24)).round() as u64;
        bit
    }
}

/// What each byte the model codes costs at `level`, in 1/256 bits: the bytes after the representation changes
/// [`compress`] would make, and each one's cost — an entropy profile, showing where the information is. Each
/// byte's cost is its share of the running total rounded to 1/256 bit, so the costs add up to the total.
pub fn cost_profile(data: &[u8], level: Level) -> (Vec<u8>, Vec<u32>) {
    let r = represent(data, usize::MAX);
    let body = r.body;
    let mut model = Model::coded(body.len(), level, r.image, r.exe, r.alpha, r.regions);
    let mut meter = CostMeter(0);
    let mut charged = 0u64;
    let costs = body
        .iter()
        .map(|&b| {
            model.code_byte(&mut meter, b);
            let total = (meter.0 + (1 << 15)) >> 16;
            let cost = total - charged;
            charged = total;
            cost as u32
        })
        .collect();
    (body, costs)
}

/// Compress `data` at [`Level::Max`]: its length; the level; the filters applied (a flags byte: reproducible
/// DEFLATE streams opened up, then either a 16-bit image's residuals or the x86 branch filter) and each one's
/// header; then the arithmetic-coded bits of the filtered bytes.
pub fn compress(data: &[u8]) -> Vec<u8> {
    compress_with(data, Level::Max, usize::MAX)
}

/// [`compress`] at `level`, opening no DEFLATE stream that would take the bytes the model codes past
/// `max_modeled` — the cap a receiver passes to [`decompress_within`].
pub fn compress_with(data: &[u8], level: Level, max_modeled: usize) -> Vec<u8> {
    let r = represent(data, max_modeled);
    let mut out = Vec::new();
    write_uvarint(data.len() as u64, &mut out);
    out.push(level.byte());
    let filters = u8::from(!r.streams.is_empty()) * FILTER_STREAMS
        | u8::from(r.image.is_some()) * FILTER_IMAGE16
        | u8::from(!r.exe.is_empty()) * FILTER_BRANCH
        | u8::from(!r.alpha.is_empty()) * FILTER_ALPHA
        | u8::from(!r.deeper.is_empty()) * FILTER_NESTED;
    out.push(filters);
    if !r.streams.is_empty() {
        let mut head = Vec::new();
        streams::write_header(&r.streams, &mut head);
        if !r.deeper.is_empty() {
            write_uvarint(r.deeper.len() as u64, &mut head);
            for level in &r.deeper {
                streams::write_header(level, &mut head);
            }
        }
        write_packed(&head, &mut out);
    }
    if let Some(img) = r.image {
        write_uvarint(img.parity as u64, &mut out);
        write_uvarint(img.width as u64, &mut out);
    } else if !r.exe.is_empty() {
        let mut bitmap = vec![0u8; r.exe.len().div_ceil(8)];
        for (b, _) in r.exe.iter().enumerate().filter(|(_, &x)| x) {
            bitmap[b / 8] |= 1 << (b % 8);
        }
        out.extend_from_slice(&bitmap);
    } else if !r.alpha.is_empty() {
        alpha::write(&r.alpha, &mut out);
    }
    out.extend_from_slice(&encode_coded(&r.body, level, r.image, r.exe, r.alpha, r.regions));
    out
}

/// What the model is shown: the reproducible DEFLATE streams opened up (within `max_modeled` bytes), then a
/// 16-bit image's residuals or, in the blocks that look like x86 code, absolute branch targets.
struct Representation {
    streams: Vec<streams::Stream>,
    /// The streams found inside opened ones, level by level.
    deeper: Vec<Vec<streams::Stream>>,
    image: Option<Image>,
    /// The branch filter's blocks, empty when no block is switched on.
    exe: Vec<bool>,
    /// The DEC Alpha code spans ([`alpha::spans`]), empty when there are none.
    alpha: Vec<alpha::Span>,
    /// The image regions of the opened streams.
    regions: Vec<Region>,
    body: Vec<u8>,
}

fn represent(data: &[u8], max_modeled: usize) -> Representation {
    let mut found = streams::find(data);
    let mut modeled = data.len();
    found.retain(|s| {
        let grown = modeled - s.compressed + s.raw;
        let keep = grown <= max_modeled.max(data.len());
        if keep {
            modeled = grown;
        }
        keep
    });
    let mut body = if found.is_empty() { data.to_vec() } else { streams::open(data, &found) };
    let mut deeper: Vec<Vec<streams::Stream>> = Vec::new();
    let budget = max_modeled.max(data.len()).min(streams::expansion_bound(data.len()));
    while !found.is_empty() && deeper.len() + 1 < NESTED_LEVELS {
        let images = all_regions(&found, &deeper);
        let mut level = streams::find(&body);
        let mut size = body.len();
        level.retain(|s| {
            let in_image = images.iter().any(|g| s.offset < g.end && g.start < s.offset + s.compressed);
            let grown = size - s.compressed + s.raw;
            let keep = !in_image && grown <= budget;
            if keep {
                size = grown;
            }
            keep
        });
        if level.is_empty() {
            break;
        }
        body = streams::open(&body, &level);
        deeper.push(level);
    }
    let image = detect_image16(&body);
    let mut exe = if image.is_some() { Vec::new() } else { branch_blocks(&body) };
    // A stream's code is modeled as x86 or as DEC Alpha, whichever covers more of it.
    let mut alpha = if image.is_some() { Vec::new() } else { alpha::spans(&body) };
    let x86_bytes = exe.iter().filter(|&&b| b).count() << BRANCH_BLOCK_LOG;
    let alpha_bytes: usize = alpha.iter().map(|s| 4 * s.words).sum();
    if alpha_bytes > x86_bytes {
        exe.clear();
    } else {
        alpha.clear();
        if x86_bytes == 0 {
            exe.clear();
        }
    }
    if let Some(img) = image {
        image_residuals(&mut body, img, true);
    } else if !exe.is_empty() {
        branch_filter(&mut body, &exe, true);
    } else if !alpha.is_empty() {
        alpha::filter(&mut body, &alpha, true);
    }
    let regions = all_regions(&found, &deeper);
    Representation { streams: found, deeper, image, exe, alpha, regions, body }
}

/// Decompress a [`compress`] stream. `None` when its declared length exceeds `max_len` (checked before any model
/// is built), its opened-up streams would pass [`streams::expansion_bound`], or the stream is malformed.
pub fn decompress(buf: &[u8], max_len: usize) -> Option<Vec<u8>> {
    decompress_within(buf, max_len, usize::MAX)
}

/// [`decompress`], also refusing a stream whose model would code more than `max_modeled` bytes — the decoder's
/// work is the bytes it models, which opened DEFLATE streams make larger than the output.
pub fn decompress_within(buf: &[u8], max_len: usize, max_modeled: usize) -> Option<Vec<u8>> {
    let mut pos = 0;
    let len = usize::try_from(read_uvarint(buf, &mut pos)?).ok()?;
    if len > max_len {
        return None;
    }
    let level = Level::from_byte(*buf.get(pos)?)?;
    let filters = *buf.get(pos + 1)?;
    pos += 2;
    if filters & !(FILTER_BRANCH | FILTER_IMAGE16 | FILTER_STREAMS | FILTER_ALPHA | FILTER_NESTED) != 0
        || (filters & (FILTER_BRANCH | FILTER_IMAGE16 | FILTER_ALPHA)).count_ones() > 1
        || (filters & FILTER_NESTED != 0 && filters & FILTER_STREAMS == 0)
    {
        return None;
    }
    let (found, deeper, body_len) = read_streams(buf, &mut pos, len, filters)?;
    if body_len > max_modeled.max(len) {
        return None;
    }
    let image = if filters & FILTER_IMAGE16 != 0 {
        let parity = usize::try_from(read_uvarint(buf, &mut pos)?).ok()?;
        let width = usize::try_from(read_uvarint(buf, &mut pos)?).ok()?;
        if parity > 1 || !(8..=IMAGE_MAX_WIDTH).contains(&width) {
            return None;
        }
        Some(Image { parity, width })
    } else {
        None
    };
    let blocks = body_len.div_ceil(1 << BRANCH_BLOCK_LOG);
    let on: Vec<bool> = if filters & FILTER_BRANCH != 0 {
        let bitmap = buf.get(pos..pos + blocks.div_ceil(8))?;
        pos += bitmap.len();
        (0..blocks).map(|b| bitmap[b / 8] >> (b % 8) & 1 == 1).collect()
    } else {
        Vec::new()
    };
    let spans = if filters & FILTER_ALPHA != 0 { alpha::read(buf, &mut pos, body_len)? } else { Vec::new() };
    let mut model = Model::coded(body_len, level, image, on.clone(), spans.clone(), all_regions(&found, &deeper));
    let mut dec = Decoder::new(buf.get(pos..)?);
    let mut out = Vec::with_capacity(body_len.min(1 << 20));
    for _ in 0..body_len {
        out.push(model.code_byte(&mut dec, 0));
    }
    if !on.is_empty() {
        branch_filter(&mut out, &on, false);
    }
    if !spans.is_empty() {
        alpha::filter(&mut out, &spans, false);
    }
    if let Some(img) = image {
        image_residuals(&mut out, img, false);
    }
    for level in deeper.iter().rev() {
        out = streams::close(&out, level)?;
    }
    if found.is_empty() {
        return Some(out);
    }
    streams::close(&out, &found).filter(|original| original.len() == len)
}
