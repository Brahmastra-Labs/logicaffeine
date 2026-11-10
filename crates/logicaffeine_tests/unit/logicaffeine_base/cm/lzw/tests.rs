use super::*;

/// SplitMix64, the generator `lzw_digests.py` mirrors.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Pixel indices below `colors`, in runs of 1 to 9 — an indexed image's texture.
fn pixels(seed: u64, n: usize, colors: u64) -> Vec<u8> {
    let mut r = Mix(seed);
    let mut out = Vec::with_capacity(n + 9);
    let mut v = 0u8;
    while out.len() < n {
        let x = r.next();
        if x % 3 == 0 {
            v = ((x >> 8) % colors) as u8;
        }
        out.extend(std::iter::repeat(v).take(1 + ((x >> 16) % 9) as usize));
    }
    out.truncate(n);
    out
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01B3))
}

/// (seed, pixels, colors, minimum code size, clear-at, length, FNV-1a 64) of the reference encoder's output — the
/// encoder that reproduced 85 of the 106 GIF images in Silesia's samba exactly.
const REFERENCE: &[(u64, usize, u64, u8, u16, usize, u64)] = &[
    (1, 50000, 4, 2, 4096, 3361, 0xF56D7B05373EAE75),
    (1, 50000, 4, 2, 4095, 3361, 0xF56D7B05373EAE75),
    (1, 50000, 4, 2, 1000, 3429, 0xC6DD587F1E74ECF0),
    (2, 200000, 16, 4, 4096, 20368, 0x127F2E71F29BE0CE),
    (2, 200000, 16, 4, 4095, 20376, 0x4FDEA36ADA9F1C32),
    (2, 200000, 16, 4, 1000, 22999, 0xA7427155DD30A2E1),
    (3, 300000, 200, 8, 4096, 69965, 0xDCCB1237C2D15E0A),
    (3, 300000, 200, 8, 4095, 69989, 0xED263D51DCA23BF7),
    (3, 300000, 200, 8, 1000, 102668, 0xAA46949DC97D235B),
    (4, 100000, 256, 8, 4096, 26481, 0xA078A9B1DE5CA031),
    (4, 100000, 256, 8, 4095, 26478, 0xA621935F8F87F1EA),
    (4, 100000, 256, 8, 1000, 35753, 0xF47057DE8FD99417),
    (5, 0, 2, 2, 4096, 1, 0xAF63A14C8601884B),
    (5, 0, 2, 2, 4095, 1, 0xAF63A14C8601884B),
    (5, 0, 2, 2, 1000, 1, 0xAF63A14C8601884B),
    (6, 1, 2, 2, 4096, 2, 0x08FE6F07B598ABD6),
    (6, 1, 2, 2, 4095, 2, 0x08FE6F07B598ABD6),
    (6, 1, 2, 2, 1000, 2, 0x08FE6F07B598ABD6),
    (7, 120000, 64, 6, 4096, 17764, 0x2EBDB760F7EBE3A1),
    (7, 120000, 64, 6, 4095, 17762, 0xF806F296C20E0929),
    (7, 120000, 64, 6, 1000, 25942, 0xC74E4A964D4ED11D),
];

#[test]
fn encode_writes_what_the_reference_encoder_writes() {
    for &(seed, n, colors, min_code, clear_at, len, digest) in REFERENCE {
        let z = encode(&pixels(seed, n, colors), min_code, clear_at);
        assert_eq!((z.len(), fnv(&z)), (len, digest), "seed {seed}, {n} pixels, code size {min_code}, clear at {clear_at}");
    }
}

#[test]
fn decode_reverses_encode_at_every_code_size_and_clear_policy() {
    for min_code in 2..=8u8 {
        let colors = 1u64 << min_code;
        for clear_at in [4096u16, 4095, 4000, 1000, (1u16 << min_code) + 3] {
            for (seed, n) in [(11u64, 0usize), (12, 1), (13, 2), (14, 70_000)] {
                let p = pixels(seed + u64::from(min_code), n, colors);
                let z = encode(&p, min_code, clear_at);
                assert_eq!(decode(&z, min_code, n).as_deref(), Some(&p[..]), "code size {min_code}, clear at {clear_at}, {n} pixels");
            }
        }
    }
}

#[test]
fn decode_refuses_what_is_not_a_code_stream() {
    let p = pixels(21, 30_000, 16);
    let z = encode(&p, 4, 4096);
    assert!(decode(&z, 1, p.len()).is_none(), "code size 1 does not exist");
    assert!(decode(&z, 9, p.len()).is_none(), "nor does 9");
    assert!(decode(&z, 4, p.len() - 1).is_none(), "one pixel past the budget");
    for cut in [0, 1, z.len() / 2, z.len() - 1] {
        assert!(decode(&z[..cut], 4, p.len()).is_none(), "cut at {cut}");
    }
    // A clear code, then a code past the table: 5-bit codes 16 (clear) then 30 (unassigned).
    assert!(decode(&[0b1100_0000 | 16, 0b0000_0011], 4, 100).is_none());
}

/// An LZW encoder whose dictionary forgets: it looks a string up in a 5003-slot hash, searching at most `probes`
/// slots, and a string with no free slot among them takes its home slot from the one there. A string it lost so is
/// written as a shorter match and added again under a new code, which later matches find.
fn lossy_lzw(pixels: &[u8], min_code: u8, probes: usize) -> Vec<u8> {
    const SLOTS: usize = 5003;
    let clear = 1usize << min_code;
    let end = clear + 1;
    let mut out = Vec::new();
    let (mut buf, mut have) = (0u64, 0u32);
    let mut emit = |code: usize, bits: u32, out: &mut Vec<u8>| {
        buf |= (code as u64) << have;
        have += bits;
        while have >= 8 {
            out.push(buf as u8);
            buf >>= 8;
            have -= 8;
        }
    };
    let mut table: Vec<Option<(u32, u16)>> = vec![None; SLOTS];
    let home = |key: u32| (key as usize).wrapping_mul(2_654_435_761) % SLOTS;
    let mut bits = u32::from(min_code) + 1;
    let mut next = clear + 2;
    emit(clear, bits, &mut out);
    let mut cur = usize::from(pixels[0]);
    for &b in &pixels[1..] {
        let key = (cur as u32) << 8 | u32::from(b);
        let h = home(key);
        let window = (0..probes).map(|i| (h + i) % SLOTS);
        if let Some(code) = window.clone().find_map(|i| table[i].filter(|&(k, _)| k == key).map(|(_, c)| c)) {
            cur = usize::from(code);
            continue;
        }
        emit(cur, bits, &mut out);
        if next < 4096 {
            let i = window.clone().find(|&i| table[i].is_none()).unwrap_or(h);
            table[i] = Some((key, next as u16));
            next += 1;
            if next > 1 << bits && bits < 12 {
                bits += 1;
            }
        } else {
            emit(clear, bits, &mut out);
            table.fill(None);
            next = clear + 2;
            bits = u32::from(min_code) + 1;
        }
        cur = usize::from(b);
    }
    emit(cur, bits, &mut out);
    emit(end, bits, &mut out);
    if have > 0 {
        out.push(buf as u8);
    }
    out
}

#[test]
fn an_encoder_that_loses_strings_is_rebuilt_from_the_pixels_it_cut_at() {
    for (seed, colors, probes) in [(31u64, 16u64, 1usize), (32, 64, 2), (33, 200, 8)] {
        let p = pixels(seed, 60_000, colors);
        let z = lossy_lzw(&p, 8, probes);
        assert_eq!(decode(&z, 8, p.len()).as_deref(), Some(&p[..]), "a lossy dictionary still writes a valid stream");
        assert_ne!(encode(&p, 8, 4096), z, "greedy matching alone does not write it ({probes} probes)");
        let (back, cut) = cuts(&z, 8, 4096, p.len()).expect("every match it wrote fits the table");
        assert_eq!(back, p);
        assert!(!cut.is_empty() && cut.windows(2).all(|w| w[0] < w[1]) && cut.iter().all(|&c| (c as usize) < p.len()));
        assert_eq!(encode_cut(&p, 8, 4096, &cut), z, "{probes} probes: rebuilt from {} cuts", cut.len());
    }
    // No cuts is the greedy encoder, and a stream it wrote has none.
    let p = pixels(34, 20_000, 32);
    assert_eq!(encode_cut(&p, 6, 4096, &[]), encode(&p, 6, 4096));
    assert_eq!(cuts(&encode(&p, 6, 4096), 6, 4096, p.len()).map(|(_, c)| c), Some(Vec::new()));
}

#[test]
fn a_stream_whose_matches_outrun_the_table_has_no_cuts() {
    // Cleared at 4096 entries but read as clearing at 300: past the 300th entry its matches run where the reader's
    // table cannot carry them.
    let p = pixels(35, 60_000, 64);
    let z = encode(&p, 6, 4096);
    assert!(cuts(&z, 6, 300, p.len()).is_none());
    assert!(cuts(&z[..z.len() / 2], 6, 4096, p.len()).is_none(), "nor has a stream that does not decode");
}
