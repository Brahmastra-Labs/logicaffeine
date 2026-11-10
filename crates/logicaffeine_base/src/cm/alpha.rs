//! DEC Alpha code for the byte model: where a stream holds Alpha instructions (fixed 32-bit little-endian words) —
//! the code sections an ECOFF object's section table names, exactly, and elsewhere the 64 KiB blocks most of whose
//! words carry the opcodes of loads, stores, address arithmetic, integer operates and branches — a reversible rewrite
//! of those words, and a tracker of the words around the next byte. The rewrite makes what the code computes from its
//! own address absolute (a call's target, a call's hint, the global pointer each procedure and call site rebuilds), so
//! the same target or pointer is the same bits wherever it is used; puts each format's fields in the order that decides
//! the rest (an operate's function before its registers); and stores every word most significant byte first, so the
//! opcode is coded first.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/alpha/tests.rs"]
mod tests;

/// Blocks of 64 KiB, as the x86 branch filter's.
pub const BLOCK_LOG: u32 = 16;

/// Below this many words a block is not judged.
const MIN_WORDS: usize = 1024;

/// The share (of 256) of a block's words whose opcode is a common one, above which the block may be Alpha code.
const COMMON_SHARE: usize = 218;

/// Below this many register-form operates, or memory instructions, a block's structure test of them is not judged.
const MIN_OPERATES: usize = 16;
const MIN_MEMORY: usize = 64;

/// The opcodes that make up most of any Alpha program: `LDA`/`LDAH`, the integer operates, the jumps, the loads and
/// stores, `BR`/`BSR` and the conditional branches.
const COMMON: u64 = {
    let ops: [u32; 30] = [
        0x08, 0x09, 0x10, 0x11, 0x12, 0x13, 0x16, 0x1A, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2C,
        0x2D, 0x30, 0x34, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D, 0x3E, 0x3F,
    ];
    let mut set = 0u64;
    let mut i = 0;
    while i < ops.len() {
        set |= 1 << ops[i];
        i += 1;
    }
    set
};

/// A call: `BSR` (opcode 0x34) saving its return address in `r26`, the top eleven bits of its word.
const CALL: u32 = 0x34 << 5 | 26;

/// The low 21 bits of a branch: its displacement, in words.
const DISPLACEMENT: u32 = (1 << 21) - 1;

/// The low 14 bits of a jump: for a call (`JSR`), the low bits of its target's word address relative to the next
/// word, a hint for the branch predictor.
const HINT: u32 = (1 << 14) - 1;

/// A jump's kind (bits 15..14) that is a call.
const JSR: u32 = 1;

/// The global pointer `r29`, which code rebuilds as `LDAH gp, hi(r)` then `LDA gp, lo(gp)` from a register holding the
/// address of the `LDAH` — the procedure value `r27` at a procedure's entry, the return address `r26` after a call.
const GP: u32 = 29;

/// How many words after its `LDAH gp` the `LDA gp` that completes the pointer may be scheduled.
const GP_REACH: usize = 8;

/// A run of Alpha instruction words: `words` whole words from byte `start`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub start: usize,
    pub words: usize,
}

impl Span {
    pub fn end(&self) -> usize {
        self.start + 4 * self.words
    }
}

/// An ECOFF object's magic for the Alpha, and the size of its file header.
const ECOFF_MAGIC: [u8; 2] = [0x83, 0x01];
const ECOFF_HEADER: usize = 24;

/// The share (of 256) of a span's words whose opcode is a common one, below which a section table's code section is
/// not believed.
const SECTION_SHARE: usize = 128;

fn common_share(words: &[u8]) -> usize {
    let n = words.len() / 4;
    if n == 0 {
        return 0;
    }
    words.chunks_exact(4).filter(|w| COMMON >> (w[3] >> 2) & 1 == 1).count() * 256 / n
}

/// The code sections (`.text`, `.init`, `.fini`) of the ECOFF objects in `data`, from their section tables: each
/// object's header gives its section count and optional-header size, each section header its name, size and file
/// offset. A section that runs past the data, is not whole words, or does not look like Alpha code is left out;
/// sections that meet are joined.
fn ecoff_code(data: &[u8]) -> Vec<Span> {
    let u16_at = |i: usize| u16::from_le_bytes([data[i], data[i + 1]]) as usize;
    let u64_at = |i: usize| u64::from_le_bytes(data[i..i + 8].try_into().unwrap());
    let mut found: Vec<Span> = Vec::new();
    let mut i = 0;
    while i + ECOFF_HEADER <= data.len() {
        if data[i..i + 2] != ECOFF_MAGIC {
            i += 1;
            continue;
        }
        let (sections, optional) = (u16_at(i + 2), u16_at(i + 20));
        let table = i + ECOFF_HEADER + optional;
        if !(1..=64).contains(&sections) || !matches!(optional, 0 | 80) || table + 64 * sections > data.len() {
            i += 1;
            continue;
        }
        let mut code: Vec<Span> = (0..sections)
            .filter_map(|k| {
                let h = table + 64 * k;
                let name = &data[h..h + 8];
                if !matches!(name, b".text\0\0\0" | b".init\0\0\0" | b".fini\0\0\0") {
                    return None;
                }
                let (size, offset) = (u64_at(h + 24), u64_at(h + 32));
                let start = (i as u64).checked_add(offset)?;
                let end = start.checked_add(size)?;
                (size > 0 && size % 4 == 0 && offset % 4 == 0 && end <= data.len() as u64)
                    .then(|| Span { start: start as usize, words: (size / 4) as usize })
            })
            .collect();
        code.sort_by_key(|s| s.start);
        for s in code {
            if common_share(&data[s.start..s.end()]) < SECTION_SHARE {
                continue;
            }
            match found.last_mut() {
                Some(last) if last.end() == s.start => last.words += s.words,
                Some(last) if last.end() > s.start => {}
                _ => found.push(s),
            }
        }
        i += ECOFF_HEADER;
    }
    found
}

/// Where `data` holds Alpha code, in order, none overlapping: the ECOFF code sections, and outside them the blocks
/// [`blocks`] finds, a run of consecutive blocks with their words in step one span from its first word to the run's end.
pub fn spans(data: &[u8]) -> Vec<Span> {
    let exact = ecoff_code(data);
    let mut spans = exact.clone();
    let mut run: Option<(usize, usize)> = None;
    let close = |run: &mut Option<(usize, usize)>, spans: &mut Vec<Span>| {
        if let Some((start, end)) = run.take() {
            spans.push(Span { start, words: (end - start) / 4 });
        }
    };
    for (b, &phase) in blocks(data).iter().enumerate() {
        let (block_start, block_end) = (b << BLOCK_LOG, ((b + 1) << BLOCK_LOG).min(data.len()));
        let start = block_start + usize::from(phase.max(1) - 1);
        let clear = phase != 0 && !exact.iter().any(|s| s.start < block_end && block_start < s.end());
        if !clear {
            close(&mut run, &mut spans);
            continue;
        }
        match run {
            Some((run_start, run_end)) if run_end == block_start && (start - run_start) % 4 == 0 => run = Some((run_start, block_end)),
            _ => {
                close(&mut run, &mut spans);
                run = Some((start, block_end));
            }
        }
    }
    close(&mut run, &mut spans);
    spans.sort_by_key(|s| s.start);
    spans
}

/// Per block: 0 when it is not Alpha code (too few of its words carry common opcodes at its best word phase, or they
/// lack code's structure), else one more than the offset (0..=3) of its words from the block's start.
pub fn blocks(data: &[u8]) -> Vec<u8> {
    data.chunks(1 << BLOCK_LOG)
        .map(|block| {
            if block.len() < MIN_WORDS * 4 {
                return 0;
            }
            let (phase, common) = (0..4)
                .map(|phase| {
                    let common = block[phase..]
                        .chunks_exact(4)
                        .filter(|w| COMMON >> (w[3] >> 2) & 1 == 1)
                        .count();
                    (phase, common)
                })
                .max_by_key(|&(phase, common)| (common, std::cmp::Reverse(phase)))
                .unwrap_or((0, 0));
            let words = (block.len() - phase) / 4;
            if common * 256 >= words * COMMON_SHARE && structured(&block[phase..]) { phase as u8 + 1 } else { 0 }
        })
        .collect()
}

/// Whether words have the structure only code has, beyond common opcodes: at most one in sixteen register-form
/// operates sets the bits the architecture requires to be zero (15..13), and at least one in eight loads and stores is
/// based on the stack pointer (`r30`) or the global pointer (`r29`).
fn structured(words: &[u8]) -> bool {
    let (mut operates, mut stray, mut memory, mut framed) = (0usize, 0usize, 0usize, 0usize);
    for w in words.chunks_exact(4) {
        let w = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
        match w >> 26 {
            0x10..=0x13 if w >> 12 & 1 == 0 => {
                operates += 1;
                stray += usize::from(w >> 13 & 7 != 0);
            }
            0x08..=0x0F | 0x20..=0x2F => {
                memory += 1;
                framed += usize::from(matches!(w >> 16 & 31, 29 | 30));
            }
            _ => {}
        }
    }
    (operates < MIN_OPERATES || stray * 16 <= operates) && (memory < MIN_MEMORY || framed * 8 >= memory)
}

/// Rewrite the words of `spans` for the model (`forward`) or back: their address-relative fields made absolute
/// ([`absolute`]), their fields arranged ([`arrange`]), each word stored most significant byte first. Only the spans'
/// whole words are touched, so any bytes round-trip.
pub fn filter(data: &mut [u8], spans: &[Span], forward: bool) {
    for span in spans {
        let end = span.end().min(data.len());
        let words = end.saturating_sub(span.start) / 4;
        let bytes = &mut data[span.start..span.start + 4 * words];
        let mut w: Vec<u32> = bytes
            .chunks_exact(4)
            .map(|b| if forward { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) } else { natural(u32::from_be_bytes([b[0], b[1], b[2], b[3]])) })
            .collect();
        absolute(&mut w, (span.start >> 2) as u32, forward);
        for (b, &x) in bytes.chunks_exact_mut(4).zip(&w) {
            b.copy_from_slice(&if forward { arrange(x).to_be_bytes() } else { x.to_le_bytes() });
        }
    }
}

/// Make the address-relative fields of a span's words (in natural field order; the first is word `first` of the
/// stream) absolute (`forward`) or relative again: a `BSR`'s displacement becomes its target's word address and a
/// `JSR`'s hint its target's low bits, modulo the field's width; and a global pointer pair's two halves become the
/// pointer itself — the `LDAH`'s address plus the two displacements, its high half in the `LDAH` and its low half in
/// the `LDA`. Which words change, and how they pair, is decided by opcode and register fields alone, which neither
/// direction changes, so both find the same words.
fn absolute(w: &mut [u32], first: u32, forward: bool) {
    let sign = |half: u32| half as u16 as i16 as i32 as u32;
    for k in 0..w.len() {
        let at = first.wrapping_add(k as u32);
        let x = w[k];
        let shift = |field: u32, mask: u32| if forward { field.wrapping_add(at) & mask } else { field.wrapping_sub(at) & mask };
        if x >> 21 == CALL {
            w[k] = x & !DISPLACEMENT | shift(x, DISPLACEMENT);
        } else if x >> 26 == 0x1A && x >> 14 & 3 == JSR {
            w[k] = x & !HINT | shift(x, HINT);
        } else if x >> 26 == 0x09 && x >> 21 & 31 == GP && matches!(x >> 16 & 31, 26 | 27) {
            let Some(j) = gp_partner(w, k) else { continue };
            let address = at.wrapping_mul(4);
            let (hi, lo) = if forward {
                let pointer = (x << 16).wrapping_add(sign(w[j] & 0xFFFF)).wrapping_add(address);
                (pointer >> 16, pointer & 0xFFFF)
            } else {
                let offset = (x << 16 | w[j] & 0xFFFF).wrapping_sub(address);
                (offset.wrapping_sub(sign(offset & 0xFFFF)) >> 16, offset & 0xFFFF)
            };
            w[k] = x & !0xFFFF | hi;
            w[j] = w[j] & !0xFFFF | lo;
        }
    }
}

/// The `LDA gp, lo(gp)` completing the `LDAH gp` at word `k`: the first word within reach that sets the global pointer
/// with `LDA` or `LDAH`, if it is that `LDA`.
fn gp_partner(w: &[u32], k: usize) -> Option<usize> {
    let j = (k + 1..w.len().min(k + 1 + GP_REACH)).find(|&j| matches!(w[j] >> 26, 0x08 | 0x09) && w[j] >> 21 & 31 == GP)?;
    (w[j] >> 26 == 0x08 && w[j] >> 16 & 31 == GP).then_some(j)
}

/// A word's fields in the order the model codes them, after the opcode, which never moves (so [`natural`] knows
/// which rewrite to undo): an integer operate's literal flag and function, then its destination Rc, then Ra, then Rb or
/// the literal; a floating operate's function, then its registers; a miscellaneous instruction's function, then its
/// registers; a jump's kind, then its registers and hint. Loads, stores and branches already lead with what decides
/// the rest.
pub fn arrange(w: u32) -> u32 {
    let op = w & 0xFC00_0000;
    match w >> 26 {
        0x10..=0x13 | 0x1C => op | (w >> 5 & 0xFF) << 18 | (w & 31) << 13 | (w >> 21 & 31) << 8 | w >> 13 & 0xFF,
        0x14..=0x17 => op | (w >> 5 & 0x7FF) << 15 | (w >> 16 & 0x3FF) << 5 | w & 31,
        0x18 => op | (w & 0xFFFF) << 10 | w >> 16 & 0x3FF,
        0x1A => op | (w >> 14 & 3) << 24 | (w >> 16 & 0x3FF) << 14 | w & HINT,
        _ => w,
    }
}

/// An opcode's class (0..16), by format and by what the instruction does: PALcode and reserved, `LDA`, `LDAH`, the
/// byte and word loads and stores, the four integer operate groups (the multiply group with the extension operates),
/// the floating operates, the miscellaneous group with the remaining reserved opcodes, the jumps, the floating loads
/// and stores, the integer loads, the integer stores, the unconditional and floating branches, and the integer
/// conditional branches.
pub fn class(op: u32) -> usize {
    match op & 63 {
        0x00..=0x07 => 0,
        0x08 => 1,
        0x09 => 2,
        0x0A..=0x0F => 3,
        0x10 => 4,
        0x11 => 5,
        0x12 => 6,
        0x13 | 0x1C => 7,
        0x14..=0x17 => 8,
        0x18 | 0x19 | 0x1B | 0x1D..=0x1F => 9,
        0x1A => 10,
        0x20..=0x27 => 11,
        0x28..=0x2B => 12,
        0x2C..=0x2F => 13,
        0x30..=0x37 => 14,
        _ => 15,
    }
}

/// The register (the floating ones from 32) a word in natural field order writes, if any: a load's or `LDA`'s Ra, an
/// operate's Rc, a jump's or `BR`/`BSR`'s return register. Writes to `r31`/`f31`, which read as zero, are none.
pub fn destination(w: u32) -> Option<usize> {
    let (ra, rc) = ((w >> 21 & 31) as usize, (w & 31) as usize);
    let r = match w >> 26 {
        0x08..=0x0C | 0x28..=0x2B | 0x1A | 0x30 | 0x34 => ra,
        0x10..=0x13 | 0x1C => rc,
        0x14..=0x17 => 32 + rc,
        0x20..=0x23 => 32 + ra,
        _ => return None,
    };
    (r & 31 != 31).then_some(r)
}

/// The registers (the floating ones from 32) a word in natural field order reads: a memory instruction's base (and a
/// store's Ra), an operate's Ra and Rb (not a literal), a floating operate's Fa and Fb, a jump's target, a branch's
/// tested register. `r31`/`f31`, which read as zero, are none.
pub fn reads(w: u32) -> [Option<usize>; 2] {
    let (ra, rb) = ((w >> 21 & 31) as usize, (w >> 16 & 31) as usize);
    let (a, b) = match w >> 26 {
        0x08..=0x0C | 0x20..=0x23 | 0x28..=0x2B | 0x1A => (None, Some(rb)),
        0x0D..=0x0F | 0x2C..=0x2F => (Some(ra), Some(rb)),
        0x24..=0x27 => (Some(32 + ra), Some(rb)),
        0x10..=0x13 | 0x1C => (Some(ra), (w >> 12 & 1 == 0).then_some(rb)),
        0x14..=0x17 => (Some(32 + ra), Some(32 + rb)),
        0x38..=0x3F => (Some(ra), None),
        0x31..=0x33 | 0x35..=0x37 => (Some(32 + ra), None),
        _ => (None, None),
    };
    [a.filter(|r| r & 31 != 31), b.filter(|r| r & 31 != 31)]
}

/// The register whose writer best predicts the rest of a word, once the word's first `at` bytes (`current`, in
/// [`arrange`]d field order) hold it: a memory instruction's base, a branch's tested register, a jump's target
/// register, an operate's first source.
pub fn source(current: u32, at: usize) -> Option<usize> {
    if at < 2 {
        return None;
    }
    let w = current << (32 - 8 * at);
    let (r, known) = match w >> 26 {
        0x08..=0x0F | 0x20..=0x2F => ((w >> 16 & 31) as usize, 2),
        0x38..=0x3F => ((w >> 21 & 31) as usize, 2),
        0x31..=0x33 | 0x35..=0x37 => (32 + (w >> 21 & 31) as usize, 2),
        0x1A => ((w >> 14 & 31) as usize, 3),
        0x10..=0x13 | 0x1C => ((w >> 8 & 31) as usize, 3),
        0x14..=0x17 => (32 + (w >> 10 & 31) as usize, 3),
        _ => return None,
    };
    (at >= known).then_some(r)
}

/// A word [`arrange`]d, back in its natural field order.
pub fn natural(c: u32) -> u32 {
    let op = c & 0xFC00_0000;
    match c >> 26 {
        0x10..=0x13 | 0x1C => op | (c >> 8 & 31) << 21 | (c & 0xFF) << 13 | (c >> 18 & 0xFF) << 5 | c >> 13 & 31,
        0x14..=0x17 => op | (c >> 5 & 0x3FF) << 16 | (c >> 15 & 0x7FF) << 5 | c & 31,
        0x18 => op | (c & 0x3FF) << 16 | c >> 10 & 0xFFFF,
        0x1A => op | (c >> 14 & 0x3FF) << 16 | (c >> 24 & 3) << 14 | c & HINT,
        _ => c,
    }
}

/// Write `spans` (in order, none overlapping) after their count: each one's distance from the last one's end, and its
/// words.
pub fn write(spans: &[Span], out: &mut Vec<u8>) {
    crate::describe::write_uvarint(spans.len() as u64, out);
    let mut end = 0;
    for s in spans {
        crate::describe::write_uvarint((s.start - end) as u64, out);
        crate::describe::write_uvarint(s.words as u64, out);
        end = s.end();
    }
}

/// Read spans [`write`] wrote for a body of `len` bytes: `None` unless there is at least one, each holds a word, and
/// they stay in order inside the body.
pub fn read(buf: &[u8], pos: &mut usize, len: usize) -> Option<Vec<Span>> {
    let count = usize::try_from(crate::describe::read_uvarint(buf, pos)?).ok()?;
    if count == 0 || count > len / 4 {
        return None;
    }
    let mut spans = Vec::with_capacity(count);
    let mut end = 0usize;
    for _ in 0..count {
        let gap = usize::try_from(crate::describe::read_uvarint(buf, pos)?).ok()?;
        let words = usize::try_from(crate::describe::read_uvarint(buf, pos)?).ok()?;
        let start = end.checked_add(gap)?;
        let span = Span { start, words };
        if words == 0 || words > len / 4 || span.end() > len {
            return None;
        }
        end = span.end();
        spans.push(span);
    }
    Some(spans)
}

/// The words around the next byte of rewritten Alpha code: where in its word the byte falls, the word's bytes so
/// far, and the words before it.
#[derive(Clone, Default)]
pub struct Words {
    position: usize,
    current: u32,
    previous: [u32; 4],
}

impl Words {
    pub fn new() -> Self {
        Self::default()
    }

    /// Back to a word's first byte, the words before forgotten (a block's start).
    pub fn restart(&mut self) {
        *self = Self::default();
    }

    pub fn push(&mut self, byte: u8) {
        self.current = self.current << 8 | u32::from(byte);
        self.position += 1;
        if self.position == 4 {
            self.previous = [self.current, self.previous[0], self.previous[1], self.previous[2]];
            self.current = 0;
            self.position = 0;
        }
    }

    /// Where the next byte falls in its word (0 for the most significant).
    #[inline]
    pub fn position(&self) -> usize {
        self.position
    }

    /// The current word's bytes so far, the latest lowest.
    #[inline]
    pub fn current(&self) -> u32 {
        self.current
    }

    /// The word `k` (0..=3) words back.
    #[inline]
    pub fn previous(&self, k: usize) -> u32 {
        self.previous[k]
    }
}
