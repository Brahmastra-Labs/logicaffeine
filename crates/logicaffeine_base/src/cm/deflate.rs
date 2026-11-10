//! # DEFLATE, both ways, byte-exact with zlib
//!
//! [`inflate`] decodes a raw DEFLATE stream (RFC 1951). [`deflate`] encodes one exactly as zlib's `deflate`
//! does for a given level, memory level and window — the same matches, the same block boundaries, the same
//! Huffman trees, bit for bit. Together they let the byte model see through compressed data: a stream that
//! inflates and re-deflates to the very same bytes can be stored as its inflated content plus the three zlib
//! parameters, and rebuilt exactly on decode.
//!
//! The encoder follows zlib's algorithm step for step: the rolling 3-byte hash and its chains over a sliding
//! 2·W window, `longest_match` with the level's chain length, lazy and nice limits, the fast (levels 1–3) and
//! lazy (4–9) parsers, a block flushed whenever the symbol buffer fills, and per block the cheapest of a stored,
//! a fixed-Huffman or a dynamic-Huffman encoding with zlib's length-limited tree construction.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/deflate/tests.rs"]
mod tests;

/// A decoded stream: the bytes and how many input bytes the stream occupied (its last byte included).
pub struct Inflated {
    pub data: Vec<u8>,
    pub consumed: usize,
}

const LENGTH_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289,
    16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// The order code-length code lengths are sent in.
const BL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

// ---- Inflate ---------------------------------------------------------------------------------

struct BitReader<'a> {
    buf: &'a [u8],
    pos: usize,
    bit: u32,
    bits: u32,
}

impl<'a> BitReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0, bit: 0, bits: 0 }
    }

    #[inline]
    fn need(&mut self, n: u32) -> Option<()> {
        while self.bits < n {
            let b = *self.buf.get(self.pos)?;
            self.pos += 1;
            self.bit |= u32::from(b) << self.bits;
            self.bits += 8;
        }
        Some(())
    }

    #[inline]
    fn take(&mut self, n: u32) -> Option<u32> {
        if n == 0 {
            return Some(0);
        }
        self.need(n)?;
        let v = self.bit & ((1 << n) - 1);
        self.bit >>= n;
        self.bits -= n;
        Some(v)
    }

    fn align(&mut self) {
        self.bit = 0;
        self.bits = 0;
    }
}

/// A canonical Huffman decoder: code counts per length and the symbols in code order. Decoding walks the code
/// one bit at a time, which is all a recompression check needs.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    /// `None` for an over-subscribed code (an incomplete one is allowed, as zlib allows a single distance code).
    fn new(lengths: &[u8]) -> Option<Self> {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[usize::from(l)] += 1;
        }
        counts[0] = 0;
        let mut left: i32 = 1;
        for &c in &counts[1..] {
            left = (left << 1) - i32::from(c);
            if left < 0 {
                return None;
            }
        }
        let mut offsets = [0u16; 16];
        for l in 1..15 {
            offsets[l + 1] = offsets[l] + counts[l];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[usize::from(offsets[usize::from(l)])] = s as u16;
                offsets[usize::from(l)] += 1;
            }
        }
        Some(Self { counts, symbols })
    }

    fn decode(&self, r: &mut BitReader) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= r.take(1)? as i32;
            let count = i32::from(self.counts[len]);
            if code - count < first {
                return self.symbols.get((index + (code - first)) as usize).copied();
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        None
    }
}

fn fixed_tables() -> (Huffman, Huffman) {
    let mut l = [0u8; 288];
    l[..144].fill(8);
    l[144..256].fill(9);
    l[256..280].fill(7);
    l[280..].fill(8);
    (Huffman::new(&l).expect("fixed literal code"), Huffman::new(&[5u8; 30]).expect("fixed distance code"))
}

/// Decode the raw DEFLATE stream at the start of `input`. `None` when it is malformed, refers before its own
/// start, or would decode to more than `max_out` bytes.
pub fn inflate(input: &[u8], max_out: usize) -> Option<Inflated> {
    let mut r = BitReader::new(input);
    let mut out: Vec<u8> = Vec::new();
    loop {
        let last = r.take(1)? == 1;
        match r.take(2)? {
            0 => {
                r.align();
                let header = input.get(r.pos..r.pos + 4)?;
                let len = usize::from(u16::from_le_bytes([header[0], header[1]]));
                let nlen = u16::from_le_bytes([header[2], header[3]]);
                if nlen != !(len as u16) || out.len() + len > max_out {
                    return None;
                }
                r.pos += 4;
                out.extend_from_slice(input.get(r.pos..r.pos + len)?);
                r.pos += len;
            }
            kind @ (1 | 2) => {
                let (lit, dist) = if kind == 1 {
                    fixed_tables()
                } else {
                    let nlen = r.take(5)? as usize + 257;
                    let ndist = r.take(5)? as usize + 1;
                    let ncode = r.take(4)? as usize + 4;
                    if nlen > 286 || ndist > 30 {
                        return None;
                    }
                    let mut cl = [0u8; 19];
                    for &i in &BL_ORDER[..ncode] {
                        cl[i] = r.take(3)? as u8;
                    }
                    let clh = Huffman::new(&cl)?;
                    let mut lengths = vec![0u8; nlen + ndist];
                    let mut i = 0;
                    while i < nlen + ndist {
                        let sym = clh.decode(&mut r)?;
                        let (value, repeat) = match sym {
                            0..=15 => (sym as u8, 1),
                            16 => (*lengths.get(i.checked_sub(1)?)?, 3 + r.take(2)? as usize),
                            17 => (0, 3 + r.take(3)? as usize),
                            18 => (0, 11 + r.take(7)? as usize),
                            _ => return None,
                        };
                        if i + repeat > nlen + ndist {
                            return None;
                        }
                        lengths[i..i + repeat].fill(value);
                        i += repeat;
                    }
                    if lengths[256] == 0 {
                        return None;
                    }
                    (Huffman::new(&lengths[..nlen])?, Huffman::new(&lengths[nlen..])?)
                };
                loop {
                    let sym = lit.decode(&mut r)?;
                    match sym {
                        0..=255 => {
                            if out.len() >= max_out {
                                return None;
                            }
                            out.push(sym as u8);
                        }
                        256 => break,
                        257..=285 => {
                            let k = usize::from(sym - 257);
                            let len = usize::from(LENGTH_BASE[k]) + r.take(u32::from(LENGTH_EXTRA[k]))? as usize;
                            let d = usize::from(dist.decode(&mut r)?);
                            if d >= 30 {
                                return None;
                            }
                            let distance = usize::from(DIST_BASE[d]) + r.take(u32::from(DIST_EXTRA[d]))? as usize;
                            if distance > out.len() || out.len() + len > max_out {
                                return None;
                            }
                            let start = out.len() - distance;
                            for i in 0..len {
                                let b = out[start + i];
                                out.push(b);
                            }
                        }
                        _ => return None,
                    }
                }
            }
            _ => return None,
        }
        if last {
            return Some(Inflated { data: out, consumed: r.pos });
        }
    }
}

// ---- Deflate, as zlib does it --------------------------------------------------------------

const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
const MIN_LOOKAHEAD: usize = MAX_MATCH + MIN_MATCH + 1;
/// A 3-byte match farther than this is not worth its distance code (levels 4–9).
const TOO_FAR: usize = 4096;
const LITERALS: usize = 256;
const END_BLOCK: usize = 256;
const L_CODES: usize = LITERALS + 1 + 29;
const D_CODES: usize = 30;
const BL_CODES: usize = 19;
const HEAP_SIZE: usize = 2 * L_CODES + 1;
const MAX_BITS: usize = 15;
const MAX_BL_BITS: usize = 7;
const REP_3_6: usize = 16;
const REPZ_3_10: usize = 17;
const REPZ_11_138: usize = 18;
const EXTRA_BLBITS: [u8; BL_CODES] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 7];

/// zlib's per-level tuning: good length, max lazy, nice length, max chain, and whether the lazy parser runs.
const CONFIG: [(usize, usize, usize, usize, bool); 10] = [
    (0, 0, 0, 0, false),
    (4, 4, 8, 4, false),
    (4, 5, 16, 8, false),
    (4, 6, 32, 32, false),
    (4, 4, 16, 16, true),
    (8, 16, 32, 32, true),
    (8, 16, 128, 128, true),
    (8, 32, 128, 256, true),
    (32, 128, 258, 1024, true),
    (32, 258, 258, 4096, true),
];

/// The zlib parameters a stream is re-created with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Params {
    /// 1..=9.
    pub level: u8,
    /// 1..=9 (zlib's default is 8).
    pub mem_level: u8,
    /// 9..=15 (raw DEFLATE window bits; zlib turns 8 into 9).
    pub window_bits: u8,
}

/// Whose rules end a block. zlib's: when the symbol buffer its memory level sizes is full. gzip's — gzip itself and
/// InfoZIP's zip, whose encoder zlib's grew from — when its buffer of 32 Ki symbols is full, or, from level 3 up and
/// every 4096 symbols, early: when under half the symbols are matches and the block so far would code in under half
/// the input it covers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Split {
    Zlib,
    Gzip,
}

/// gzip's symbol buffer, and how many symbols apart it weighs ending a block early.
const GZIP_SYMBOLS: usize = 0x8000;
const GZIP_WEIGH: usize = 0x1000;

/// A Huffman tree node in zlib's layout: frequency (later the code) and parent (later the length) share storage.
#[derive(Clone, Copy, Default)]
struct Node {
    fc: u16,
    dl: u16,
}

struct Codes {
    length_code: [u8; 256],
    dist_code: [u8; 512],
    base_length: [u8; 29],
    base_dist: [u16; 30],
    static_ltree: [Node; L_CODES + 2],
    static_dtree: [Node; D_CODES],
}

fn bi_reverse(mut code: u32, len: u32) -> u16 {
    let mut res = 0u32;
    for _ in 0..len {
        res = res << 1 | (code & 1);
        code >>= 1;
    }
    res as u16
}

fn gen_codes(tree: &mut [Node], max_code: usize, bl_count: &[u16; MAX_BITS + 1]) {
    let mut next_code = [0u16; MAX_BITS + 1];
    let mut code = 0u16;
    for bits in 1..=MAX_BITS {
        code = (code + bl_count[bits - 1]) << 1;
        next_code[bits] = code;
    }
    for n in 0..=max_code {
        let len = usize::from(tree[n].dl);
        if len == 0 {
            continue;
        }
        tree[n].fc = bi_reverse(u32::from(next_code[len]), len as u32);
        next_code[len] += 1;
    }
}

fn codes() -> &'static Codes {
    static TABLES: std::sync::OnceLock<Codes> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let mut c = Codes {
            length_code: [0; 256],
            dist_code: [0; 512],
            base_length: [0; 29],
            base_dist: [0; 30],
            static_ltree: [Node::default(); L_CODES + 2],
            static_dtree: [Node::default(); D_CODES],
        };
        let mut length = 0usize;
        for code in 0..28 {
            c.base_length[code] = length as u8;
            for _ in 0..1 << LENGTH_EXTRA[code] {
                c.length_code[length] = code as u8;
                length += 1;
            }
        }
        c.length_code[length - 1] = 28;
        let mut dist = 0usize;
        for code in 0..16 {
            c.base_dist[code] = dist as u16;
            for _ in 0..1 << DIST_EXTRA[code] {
                c.dist_code[dist] = code as u8;
                dist += 1;
            }
        }
        dist >>= 7;
        for code in 16..D_CODES {
            c.base_dist[code] = (dist << 7) as u16;
            for _ in 0..1 << (DIST_EXTRA[code] - 7) {
                c.dist_code[256 + dist] = code as u8;
                dist += 1;
            }
        }
        let mut bl_count = [0u16; MAX_BITS + 1];
        for (n, node) in c.static_ltree.iter_mut().enumerate() {
            node.dl = match n {
                0..=143 => 8,
                144..=255 => 9,
                256..=279 => 7,
                _ => 8,
            };
            bl_count[usize::from(node.dl)] += 1;
        }
        gen_codes(&mut c.static_ltree, L_CODES + 1, &bl_count);
        for (n, node) in c.static_dtree.iter_mut().enumerate() {
            node.dl = 5;
            node.fc = bi_reverse(n as u32, 5);
        }
        c
    })
}

#[inline]
fn d_code(c: &Codes, dist: usize) -> usize {
    usize::from(if dist < 256 { c.dist_code[dist] } else { c.dist_code[256 + (dist >> 7)] })
}

/// Which tree `build_tree` is building: its static counterpart, extra bits, and length limit.
#[derive(Clone, Copy)]
enum Tree {
    Literal,
    Distance,
    BitLength,
}

struct Deflater<'a> {
    input: &'a [u8],
    read: usize,
    good_match: usize,
    max_lazy: usize,
    nice_match: usize,
    max_chain: usize,
    w_size: usize,
    w_mask: usize,
    window: Vec<u8>,
    prev: Vec<u16>,
    head: Vec<u16>,
    ins_h: usize,
    hash_mask: usize,
    hash_shift: usize,
    block_start: isize,
    match_length: usize,
    prev_match: usize,
    match_available: bool,
    strstart: usize,
    match_start: usize,
    lookahead: usize,
    prev_length: usize,
    insert: usize,
    high_water: usize,
    dyn_ltree: [Node; HEAP_SIZE],
    dyn_dtree: [Node; 2 * D_CODES + 1],
    bl_tree: [Node; 2 * BL_CODES + 1],
    l_max_code: usize,
    d_max_code: usize,
    bl_max_code: usize,
    bl_count: [u16; MAX_BITS + 1],
    heap: [usize; HEAP_SIZE],
    heap_len: usize,
    heap_max: usize,
    depth: [u8; HEAP_SIZE],
    sym_buf: Vec<u8>,
    sym_next: usize,
    sym_end: usize,
    split: Split,
    level: u8,
    /// The block's distance symbols.
    matches: usize,
    opt_len: i64,
    static_len: i64,
    out: Vec<u8>,
    bi_buf: u64,
    bi_valid: u32,
}

impl<'a> Deflater<'a> {
    fn new(input: &'a [u8], p: Params, split: Split) -> Self {
        let w_bits = usize::from(p.window_bits.max(9));
        let w_size = 1 << w_bits;
        let hash_bits = usize::from(p.mem_level) + 7;
        let lit_bufsize = 1 << (usize::from(p.mem_level) + 6);
        let (good_match, max_lazy, nice_match, max_chain, _) = CONFIG[usize::from(p.level)];
        let mut d = Self {
            input,
            read: 0,
            good_match,
            max_lazy,
            nice_match,
            max_chain,
            w_size,
            w_mask: w_size - 1,
            window: vec![0; 2 * w_size],
            prev: vec![0; w_size],
            head: vec![0; 1 << hash_bits],
            ins_h: 0,
            hash_mask: (1 << hash_bits) - 1,
            hash_shift: (hash_bits + MIN_MATCH - 1) / MIN_MATCH,
            block_start: 0,
            match_length: MIN_MATCH - 1,
            prev_match: 0,
            match_available: false,
            strstart: 0,
            match_start: 0,
            lookahead: 0,
            prev_length: MIN_MATCH - 1,
            insert: 0,
            high_water: 0,
            dyn_ltree: [Node::default(); HEAP_SIZE],
            dyn_dtree: [Node::default(); 2 * D_CODES + 1],
            bl_tree: [Node::default(); 2 * BL_CODES + 1],
            l_max_code: 0,
            d_max_code: 0,
            bl_max_code: 0,
            bl_count: [0; MAX_BITS + 1],
            heap: [0; HEAP_SIZE],
            heap_len: 0,
            heap_max: 0,
            depth: [0; HEAP_SIZE],
            sym_buf: vec![0; if split == Split::Gzip { GZIP_SYMBOLS * 3 } else { lit_bufsize * 4 }],
            sym_next: 0,
            sym_end: (lit_bufsize - 1) * 3,
            split,
            level: p.level,
            matches: 0,
            opt_len: 0,
            static_len: 0,
            out: Vec::with_capacity(input.len() / 2 + 64),
            bi_buf: 0,
            bi_valid: 0,
        };
        d.init_block();
        d
    }

    fn max_dist(&self) -> usize {
        self.w_size - MIN_LOOKAHEAD
    }

    // ---- bit output

    #[inline]
    fn send_bits(&mut self, value: u32, length: u32) {
        self.bi_buf |= u64::from(value) << self.bi_valid;
        self.bi_valid += length;
        while self.bi_valid >= 16 {
            self.out.push(self.bi_buf as u8);
            self.out.push((self.bi_buf >> 8) as u8);
            self.bi_buf >>= 16;
            self.bi_valid -= 16;
        }
    }

    fn bi_windup(&mut self) {
        if self.bi_valid > 8 {
            self.out.push(self.bi_buf as u8);
            self.out.push((self.bi_buf >> 8) as u8);
        } else if self.bi_valid > 0 {
            self.out.push(self.bi_buf as u8);
        }
        self.bi_buf = 0;
        self.bi_valid = 0;
    }

    // ---- the window

    #[inline]
    fn update_hash(&mut self, c: u8) {
        self.ins_h = ((self.ins_h << self.hash_shift) ^ usize::from(c)) & self.hash_mask;
    }

    /// INSERT_STRING: hash the 3 bytes at `at`, chain `at` in front of the bucket, return the old head.
    #[inline]
    fn insert_string(&mut self, at: usize) -> usize {
        let c = self.window[at + MIN_MATCH - 1];
        self.update_hash(c);
        let head = self.head[self.ins_h];
        self.prev[at & self.w_mask] = head;
        self.head[self.ins_h] = at as u16;
        usize::from(head)
    }

    fn slide_hash(&mut self) {
        let w = self.w_size as u16;
        for h in self.head.iter_mut().chain(self.prev.iter_mut()) {
            *h = h.saturating_sub(w);
        }
    }

    fn fill_window(&mut self) {
        let wsize = self.w_size;
        let window_size = 2 * wsize;
        loop {
            let mut more = window_size - self.lookahead - self.strstart;
            if self.strstart >= wsize + self.max_dist() {
                self.window.copy_within(wsize..wsize + wsize - more, 0);
                self.match_start = self.match_start.wrapping_sub(wsize);
                self.strstart -= wsize;
                self.block_start -= wsize as isize;
                if self.insert > self.strstart {
                    self.insert = self.strstart;
                }
                self.slide_hash();
                more += wsize;
            }
            if self.read >= self.input.len() {
                break;
            }
            let at = self.strstart + self.lookahead;
            let n = more.min(self.input.len() - self.read);
            self.window[at..at + n].copy_from_slice(&self.input[self.read..self.read + n]);
            self.read += n;
            self.lookahead += n;
            if self.lookahead + self.insert >= MIN_MATCH {
                let mut s = self.strstart - self.insert;
                self.ins_h = usize::from(self.window[s]);
                let c = self.window[s + 1];
                self.update_hash(c);
                while self.insert > 0 {
                    let c = self.window[s + MIN_MATCH - 1];
                    self.update_hash(c);
                    self.prev[s & self.w_mask] = self.head[self.ins_h];
                    self.head[self.ins_h] = s as u16;
                    s += 1;
                    self.insert -= 1;
                    if self.lookahead + self.insert < MIN_MATCH {
                        break;
                    }
                }
            }
            if self.lookahead >= MIN_LOOKAHEAD || self.read >= self.input.len() {
                break;
            }
        }
        // zlib zeroes up to MAX_MATCH bytes past the data the first time they come into reach, so a match scan
        // that runs off the end reads zeros (or the earlier window contents a slide left behind).
        let curr = self.strstart + self.lookahead;
        if self.high_water < window_size {
            if self.high_water < curr {
                let init = (window_size - curr).min(MAX_MATCH);
                self.window[curr..curr + init].fill(0);
                self.high_water = curr + init;
            } else if self.high_water < curr + MAX_MATCH {
                let init = (curr + MAX_MATCH - self.high_water).min(window_size - self.high_water);
                let hw = self.high_water;
                self.window[hw..hw + init].fill(0);
                self.high_water += init;
            }
        }
    }

    fn longest_match(&mut self, mut cur_match: usize) -> usize {
        let mut chain_length = self.max_chain;
        let scan = self.strstart;
        let mut best_len = self.prev_length;
        let mut nice_match = self.nice_match;
        let limit = if self.strstart > self.max_dist() { self.strstart - self.max_dist() } else { 0 };
        let w = &self.window;
        let mut scan_end1 = w[scan + best_len - 1];
        let mut scan_end = w[scan + best_len];
        if self.prev_length >= self.good_match {
            chain_length >>= 2;
        }
        if nice_match > self.lookahead {
            nice_match = self.lookahead;
        }
        loop {
            let m = cur_match;
            if w[m + best_len] == scan_end && w[m + best_len - 1] == scan_end1 && w[m] == w[scan] && w[m + 1] == w[scan + 1] {
                let mut len = 3;
                while len < MAX_MATCH && w[scan + len] == w[m + len] {
                    len += 1;
                }
                if len > best_len {
                    self.match_start = cur_match;
                    best_len = len;
                    if len >= nice_match {
                        break;
                    }
                    scan_end1 = w[scan + best_len - 1];
                    scan_end = w[scan + best_len];
                }
            }
            cur_match = usize::from(self.prev[cur_match & self.w_mask]);
            if cur_match <= limit {
                break;
            }
            chain_length -= 1;
            if chain_length == 0 {
                break;
            }
        }
        best_len.min(self.lookahead)
    }

    // ---- symbols and blocks

    fn init_block(&mut self) {
        for n in self.dyn_ltree[..L_CODES].iter_mut() {
            n.fc = 0;
        }
        for n in self.dyn_dtree[..D_CODES].iter_mut() {
            n.fc = 0;
        }
        for n in self.bl_tree[..BL_CODES].iter_mut() {
            n.fc = 0;
        }
        self.dyn_ltree[END_BLOCK].fc = 1;
        self.opt_len = 0;
        self.static_len = 0;
        self.sym_next = 0;
        self.matches = 0;
    }

    #[inline]
    fn tally_lit(&mut self, c: u8) -> bool {
        self.sym_buf[self.sym_next] = 0;
        self.sym_buf[self.sym_next + 1] = 0;
        self.sym_buf[self.sym_next + 2] = c;
        self.sym_next += 3;
        self.dyn_ltree[usize::from(c)].fc += 1;
        self.block_full()
    }

    #[inline]
    fn tally_dist(&mut self, dist: usize, len: usize) -> bool {
        let c = codes();
        self.sym_buf[self.sym_next] = dist as u8;
        self.sym_buf[self.sym_next + 1] = (dist >> 8) as u8;
        self.sym_buf[self.sym_next + 2] = len as u8;
        self.sym_next += 3;
        self.dyn_ltree[usize::from(c.length_code[len]) + LITERALS + 1].fc += 1;
        self.dyn_dtree[d_code(c, dist - 1)].fc += 1;
        self.matches += 1;
        self.block_full()
    }

    /// Whether the block ends after the symbol just tallied ([`Split`]); gzip's early end bounds the block's coded
    /// size by eight bits a symbol plus five and the extra bits a distance.
    #[inline]
    fn block_full(&self) -> bool {
        match self.split {
            Split::Zlib => self.sym_next == self.sym_end,
            Split::Gzip => {
                let symbols = self.sym_next / 3;
                if self.level > 2 && symbols % GZIP_WEIGH == 0 {
                    let distances: u64 = (0..D_CODES).map(|d| u64::from(self.dyn_dtree[d].fc) * (5 + u64::from(DIST_EXTRA[d]))).sum();
                    let coded = (symbols as u64 * 8 + distances) >> 3;
                    let covered = (self.strstart as isize - self.block_start) as u64;
                    if self.matches < symbols / 2 && coded < covered / 2 {
                        return true;
                    }
                }
                symbols == GZIP_SYMBOLS - 1
            }
        }
    }

    fn tree(&mut self, t: Tree) -> &mut [Node] {
        match t {
            Tree::Literal => &mut self.dyn_ltree[..],
            Tree::Distance => &mut self.dyn_dtree[..],
            Tree::BitLength => &mut self.bl_tree[..],
        }
    }

    #[inline]
    fn smaller(tree: &[Node], depth: &[u8], n: usize, m: usize) -> bool {
        tree[n].fc < tree[m].fc || (tree[n].fc == tree[m].fc && depth[n] <= depth[m])
    }

    fn pqdownheap(&mut self, t: Tree, mut k: usize) {
        let v = self.heap[k];
        let mut j = k << 1;
        let heap_len = self.heap_len;
        let tree: &[Node] = match t {
            Tree::Literal => &self.dyn_ltree[..],
            Tree::Distance => &self.dyn_dtree[..],
            Tree::BitLength => &self.bl_tree[..],
        };
        while j <= heap_len {
            if j < heap_len && Self::smaller(tree, &self.depth, self.heap[j + 1], self.heap[j]) {
                j += 1;
            }
            if Self::smaller(tree, &self.depth, v, self.heap[j]) {
                break;
            }
            self.heap[k] = self.heap[j];
            k = j;
            j <<= 1;
        }
        self.heap[k] = v;
    }

    fn gen_bitlen(&mut self, t: Tree, max_code: usize) {
        let c = codes();
        let (stree, extra, base, max_length): (Option<&[Node]>, &[u8], usize, usize) = match t {
            Tree::Literal => (Some(&c.static_ltree[..]), &LENGTH_EXTRA[..], LITERALS + 1, MAX_BITS),
            Tree::Distance => (Some(&c.static_dtree[..]), &DIST_EXTRA[..], 0, MAX_BITS),
            Tree::BitLength => (None, &EXTRA_BLBITS[..], 0, MAX_BL_BITS),
        };
        self.bl_count = [0; MAX_BITS + 1];
        let root = self.heap[self.heap_max];
        self.tree(t)[root].dl = 0;
        let mut overflow = 0i32;
        let mut h = self.heap_max + 1;
        while h < HEAP_SIZE {
            let n = self.heap[h];
            let dad = usize::from(self.tree(t)[n].dl);
            let mut bits = usize::from(self.tree(t)[dad].dl) + 1;
            if bits > max_length {
                bits = max_length;
                overflow += 1;
            }
            self.tree(t)[n].dl = bits as u16;
            h += 1;
            if n > max_code {
                continue;
            }
            self.bl_count[bits] += 1;
            let xbits = if n >= base { usize::from(extra[n - base]) } else { 0 };
            let f = i64::from(self.tree(t)[n].fc);
            self.opt_len += f * (bits + xbits) as i64;
            if let Some(st) = stree {
                self.static_len += f * (usize::from(st[n].dl) + xbits) as i64;
            }
        }
        if overflow == 0 {
            return;
        }
        loop {
            let mut bits = max_length - 1;
            while self.bl_count[bits] == 0 {
                bits -= 1;
            }
            self.bl_count[bits] -= 1;
            self.bl_count[bits + 1] += 2;
            self.bl_count[max_length] -= 1;
            overflow -= 2;
            if overflow <= 0 {
                break;
            }
        }
        let mut h = HEAP_SIZE;
        for bits in (1..=max_length).rev() {
            let mut n = self.bl_count[bits];
            while n != 0 {
                h -= 1;
                let m = self.heap[h];
                if m > max_code {
                    continue;
                }
                let node = &mut self.tree(t)[m];
                if usize::from(node.dl) != bits {
                    let freq = i64::from(node.fc);
                    let old = i64::from(node.dl);
                    node.dl = bits as u16;
                    self.opt_len += (bits as i64 - old) * freq;
                }
                n -= 1;
            }
        }
    }

    /// zlib's `build_tree`: the Huffman tree for `elems` symbols, its lengths limited and its codes assigned.
    /// Returns the largest symbol with a nonzero frequency.
    fn build_tree(&mut self, t: Tree, elems: usize) -> usize {
        let c = codes();
        let stree: Option<&[Node]> = match t {
            Tree::Literal => Some(&c.static_ltree[..]),
            Tree::Distance => Some(&c.static_dtree[..]),
            Tree::BitLength => None,
        };
        self.heap_len = 0;
        self.heap_max = HEAP_SIZE;
        let mut max_code: isize = -1;
        for n in 0..elems {
            if self.tree(t)[n].fc != 0 {
                self.heap_len += 1;
                self.heap[self.heap_len] = n;
                max_code = n as isize;
                self.depth[n] = 0;
            } else {
                self.tree(t)[n].dl = 0;
            }
        }
        while self.heap_len < 2 {
            let node = if max_code < 2 {
                max_code += 1;
                max_code as usize
            } else {
                0
            };
            self.heap_len += 1;
            self.heap[self.heap_len] = node;
            self.tree(t)[node].fc = 1;
            self.depth[node] = 0;
            self.opt_len -= 1;
            if let Some(st) = stree {
                self.static_len -= i64::from(st[node].dl);
            }
        }
        let max_code = max_code as usize;
        for n in (1..=self.heap_len / 2).rev() {
            self.pqdownheap(t, n);
        }
        let mut node = elems;
        loop {
            let n = self.heap[1];
            self.heap[1] = self.heap[self.heap_len];
            self.heap_len -= 1;
            self.pqdownheap(t, 1);
            let m = self.heap[1];
            self.heap_max -= 1;
            self.heap[self.heap_max] = n;
            self.heap_max -= 1;
            self.heap[self.heap_max] = m;
            let (fn_, fm) = (self.tree(t)[n].fc, self.tree(t)[m].fc);
            self.tree(t)[node].fc = fn_.wrapping_add(fm);
            self.depth[node] = self.depth[n].max(self.depth[m]) + 1;
            self.tree(t)[n].dl = node as u16;
            self.tree(t)[m].dl = node as u16;
            self.heap[1] = node;
            node += 1;
            self.pqdownheap(t, 1);
            if self.heap_len < 2 {
                break;
            }
        }
        self.heap_max -= 1;
        self.heap[self.heap_max] = self.heap[1];
        self.gen_bitlen(t, max_code);
        let bl_count = self.bl_count;
        gen_codes(self.tree(t), max_code, &bl_count);
        max_code
    }

    fn scan_tree(&mut self, t: Tree, max_code: usize) {
        let mut prevlen: isize = -1;
        let mut nextlen = usize::from(self.tree(t)[0].dl);
        let mut count = 0;
        let (mut max_count, mut min_count) = if nextlen == 0 { (138, 3) } else { (7, 4) };
        self.tree(t)[max_code + 1].dl = 0xFFFF;
        for n in 0..=max_code {
            let curlen = nextlen;
            nextlen = usize::from(self.tree(t)[n + 1].dl);
            count += 1;
            if count < max_count && curlen == nextlen {
                continue;
            } else if count < min_count {
                self.bl_tree[curlen].fc += count as u16;
            } else if curlen != 0 {
                if curlen as isize != prevlen {
                    self.bl_tree[curlen].fc += 1;
                }
                self.bl_tree[REP_3_6].fc += 1;
            } else if count <= 10 {
                self.bl_tree[REPZ_3_10].fc += 1;
            } else {
                self.bl_tree[REPZ_11_138].fc += 1;
            }
            count = 0;
            prevlen = curlen as isize;
            (max_count, min_count) = if nextlen == 0 {
                (138, 3)
            } else if curlen == nextlen {
                (6, 3)
            } else {
                (7, 4)
            };
        }
    }

    #[inline]
    fn send_code(&mut self, t: Tree, c: usize) {
        let node = self.tree(t)[c];
        self.send_bits(u32::from(node.fc), u32::from(node.dl));
    }

    fn send_tree(&mut self, t: Tree, max_code: usize) {
        let mut prevlen: isize = -1;
        let mut nextlen = usize::from(self.tree(t)[0].dl);
        let mut count = 0;
        let (mut max_count, mut min_count) = if nextlen == 0 { (138, 3) } else { (7, 4) };
        for n in 0..=max_code {
            let curlen = nextlen;
            nextlen = usize::from(self.tree(t)[n + 1].dl);
            count += 1;
            if count < max_count && curlen == nextlen {
                continue;
            } else if count < min_count {
                while count != 0 {
                    self.send_code(Tree::BitLength, curlen);
                    count -= 1;
                }
            } else if curlen != 0 {
                if curlen as isize != prevlen {
                    self.send_code(Tree::BitLength, curlen);
                    count -= 1;
                }
                self.send_code(Tree::BitLength, REP_3_6);
                self.send_bits(count as u32 - 3, 2);
            } else if count <= 10 {
                self.send_code(Tree::BitLength, REPZ_3_10);
                self.send_bits(count as u32 - 3, 3);
            } else {
                self.send_code(Tree::BitLength, REPZ_11_138);
                self.send_bits(count as u32 - 11, 7);
            }
            count = 0;
            prevlen = curlen as isize;
            (max_count, min_count) = if nextlen == 0 {
                (138, 3)
            } else if curlen == nextlen {
                (6, 3)
            } else {
                (7, 4)
            };
        }
    }

    fn build_bl_tree(&mut self) -> usize {
        self.scan_tree(Tree::Literal, self.l_max_code);
        self.scan_tree(Tree::Distance, self.d_max_code);
        self.bl_max_code = self.build_tree(Tree::BitLength, BL_CODES);
        let mut max_blindex = BL_CODES - 1;
        while max_blindex >= 3 {
            if self.bl_tree[BL_ORDER[max_blindex]].dl != 0 {
                break;
            }
            max_blindex -= 1;
        }
        self.opt_len += 3 * (max_blindex as i64 + 1) + 5 + 5 + 4;
        max_blindex
    }

    fn compress_block(&mut self, fixed: bool) {
        let c = codes();
        let mut sx = 0;
        while sx < self.sym_next {
            let dist = usize::from(self.sym_buf[sx]) | usize::from(self.sym_buf[sx + 1]) << 8;
            let lc = usize::from(self.sym_buf[sx + 2]);
            sx += 3;
            if dist == 0 {
                self.send_symbol(fixed, true, lc);
            } else {
                let code = usize::from(c.length_code[lc]);
                self.send_symbol(fixed, true, code + LITERALS + 1);
                let extra = u32::from(LENGTH_EXTRA[code]);
                if extra != 0 {
                    self.send_bits((lc - usize::from(c.base_length[code])) as u32, extra);
                }
                let dist = dist - 1;
                let code = d_code(c, dist);
                self.send_symbol(fixed, false, code);
                let extra = u32::from(DIST_EXTRA[code]);
                if extra != 0 {
                    self.send_bits((dist - usize::from(c.base_dist[code])) as u32, extra);
                }
            }
        }
        self.send_symbol(fixed, true, END_BLOCK);
    }

    #[inline]
    fn send_symbol(&mut self, fixed: bool, literal: bool, sym: usize) {
        let node = match (fixed, literal) {
            (true, true) => codes().static_ltree[sym],
            (true, false) => codes().static_dtree[sym],
            (false, true) => self.dyn_ltree[sym],
            (false, false) => self.dyn_dtree[sym],
        };
        self.send_bits(u32::from(node.fc), u32::from(node.dl));
    }

    fn stored_block(&mut self, start: usize, len: usize, last: bool) {
        self.send_bits(u32::from(last), 3);
        self.bi_windup();
        self.out.extend_from_slice(&(len as u16).to_le_bytes());
        self.out.extend_from_slice(&(!(len as u16)).to_le_bytes());
        let bytes = self.window[start..start + len].to_vec();
        self.out.extend_from_slice(&bytes);
    }

    fn flush_block(&mut self, last: bool) {
        let stored_len = (self.strstart as isize - self.block_start) as usize;
        self.l_max_code = self.build_tree(Tree::Literal, L_CODES);
        self.d_max_code = self.build_tree(Tree::Distance, D_CODES);
        let max_blindex = self.build_bl_tree();
        let mut opt_lenb = ((self.opt_len + 3 + 7) >> 3) as u64;
        let static_lenb = ((self.static_len + 3 + 7) >> 3) as u64;
        if static_lenb <= opt_lenb {
            opt_lenb = static_lenb;
        }
        if stored_len as u64 + 4 <= opt_lenb && self.block_start >= 0 {
            self.stored_block(self.block_start as usize, stored_len, last);
        } else if static_lenb == opt_lenb {
            self.send_bits((1 << 1) + u32::from(last), 3);
            self.compress_block(true);
        } else {
            self.send_bits((2 << 1) + u32::from(last), 3);
            let (lcodes, dcodes, blcodes) = (self.l_max_code + 1, self.d_max_code + 1, max_blindex + 1);
            self.send_bits((lcodes - 257) as u32, 5);
            self.send_bits((dcodes - 1) as u32, 5);
            self.send_bits((blcodes - 4) as u32, 4);
            for &i in &BL_ORDER[..blcodes] {
                let len = self.bl_tree[i].dl;
                self.send_bits(u32::from(len), 3);
            }
            self.send_tree(Tree::Literal, lcodes - 1);
            self.send_tree(Tree::Distance, dcodes - 1);
            self.compress_block(false);
        }
        self.init_block();
        if last {
            self.bi_windup();
        }
        self.block_start = self.strstart as isize;
    }

    fn run_fast(&mut self) {
        loop {
            if self.lookahead < MIN_LOOKAHEAD {
                self.fill_window();
                if self.lookahead == 0 {
                    break;
                }
            }
            let mut hash_head = 0;
            if self.lookahead >= MIN_MATCH {
                hash_head = self.insert_string(self.strstart);
            }
            if hash_head != 0 && self.strstart - hash_head <= self.max_dist() {
                self.match_length = self.longest_match(hash_head);
            }
            let bflush;
            if self.match_length >= MIN_MATCH {
                bflush = self.tally_dist(self.strstart - self.match_start, self.match_length - MIN_MATCH);
                self.lookahead -= self.match_length;
                if self.match_length <= self.max_lazy && self.lookahead >= MIN_MATCH {
                    self.match_length -= 1;
                    loop {
                        self.strstart += 1;
                        self.insert_string(self.strstart);
                        self.match_length -= 1;
                        if self.match_length == 0 {
                            break;
                        }
                    }
                    self.strstart += 1;
                } else {
                    self.strstart += self.match_length;
                    self.match_length = 0;
                    self.ins_h = usize::from(self.window[self.strstart]);
                    let c = self.window[self.strstart + 1];
                    self.update_hash(c);
                }
            } else {
                bflush = self.tally_lit(self.window[self.strstart]);
                self.lookahead -= 1;
                self.strstart += 1;
            }
            if bflush {
                self.flush_block(false);
            }
        }
        self.insert = self.strstart.min(MIN_MATCH - 1);
        self.flush_block(true);
    }

    fn run_slow(&mut self) {
        loop {
            if self.lookahead < MIN_LOOKAHEAD {
                self.fill_window();
                if self.lookahead == 0 {
                    break;
                }
            }
            let mut hash_head = 0;
            if self.lookahead >= MIN_MATCH {
                hash_head = self.insert_string(self.strstart);
            }
            self.prev_length = self.match_length;
            self.prev_match = self.match_start;
            self.match_length = MIN_MATCH - 1;
            if hash_head != 0 && self.prev_length < self.max_lazy && self.strstart - hash_head <= self.max_dist() {
                self.match_length = self.longest_match(hash_head);
                if self.match_length <= 5 && self.match_length == MIN_MATCH && self.strstart - self.match_start > TOO_FAR {
                    self.match_length = MIN_MATCH - 1;
                }
            }
            if self.prev_length >= MIN_MATCH && self.match_length <= self.prev_length {
                let max_insert = self.strstart + self.lookahead - MIN_MATCH;
                let bflush = self.tally_dist(self.strstart - 1 - self.prev_match, self.prev_length - MIN_MATCH);
                self.lookahead -= self.prev_length - 1;
                self.prev_length -= 2;
                loop {
                    self.strstart += 1;
                    if self.strstart <= max_insert {
                        self.insert_string(self.strstart);
                    }
                    self.prev_length -= 1;
                    if self.prev_length == 0 {
                        break;
                    }
                }
                self.match_available = false;
                self.match_length = MIN_MATCH - 1;
                self.strstart += 1;
                if bflush {
                    self.flush_block(false);
                }
            } else if self.match_available {
                let bflush = self.tally_lit(self.window[self.strstart - 1]);
                if bflush {
                    self.flush_block(false);
                }
                self.strstart += 1;
                self.lookahead -= 1;
            } else {
                self.match_available = true;
                self.strstart += 1;
                self.lookahead -= 1;
            }
        }
        if self.match_available {
            self.tally_lit(self.window[self.strstart - 1]);
            self.match_available = false;
        }
        self.insert = self.strstart.min(MIN_MATCH - 1);
        self.flush_block(true);
    }
}

/// Encode `input` as one raw DEFLATE stream exactly as zlib's `deflate` does with these parameters, the default
/// strategy and a single `Z_FINISH`.
pub fn deflate(input: &[u8], p: Params) -> Vec<u8> {
    assert!((1..=9).contains(&p.level) && (1..=9).contains(&p.mem_level) && (8..=15).contains(&p.window_bits));
    run(Deflater::new(input, p, Split::Zlib))
}

/// `input` deflated as gzip and InfoZIP's zip do at `level` (1..=9): zlib's matcher at memory level 8 with a 32 KiB
/// window, its blocks ended by gzip's rules.
pub fn gzip(input: &[u8], level: u8) -> Vec<u8> {
    assert!((1..=9).contains(&level));
    run(Deflater::new(input, Params { level, mem_level: 8, window_bits: 15 }, Split::Gzip))
}

fn run(mut d: Deflater) -> Vec<u8> {
    if CONFIG[usize::from(d.level)].4 {
        d.run_slow();
    } else {
        d.run_fast();
    }
    d.out
}
