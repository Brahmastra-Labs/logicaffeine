use super::*;

/// SplitMix64 — deterministic test data.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn round_trips(data: &[u8]) -> Vec<u8> {
    let z = compress(data);
    assert_eq!(decompress(&z, data.len()).as_deref(), Some(data), "round trip of {} bytes", data.len());
    z
}

/// Text from a small vocabulary under a first-order word chain: real structure at the byte, word and
/// phrase level, the shapes a text model has to learn.
fn wordy_text(n: usize, seed: u64) -> Vec<u8> {
    const WORDS: [&str; 24] = [
        "the", "codec", "ships", "a", "column", "of", "values", "and", "every", "decoder", "reads", "it", "back",
        "bit", "for", "exactly", "as", "written", "when", "model", "predicts", "next", "symbol", "well",
    ];
    let mut rng = Mix(seed);
    let mut out = Vec::with_capacity(n + 16);
    let mut w = 0usize;
    while out.len() < n {
        w = (w * 7 + 1 + (rng.next() % 5) as usize) % WORDS.len();
        out.extend_from_slice(WORDS[w].as_bytes());
        out.push(if rng.next() % 9 == 0 { b'\n' } else { b' ' });
    }
    out.truncate(n);
    out
}

fn entropy_order_k(data: &[u8], k: usize) -> f64 {
    use std::collections::HashMap;
    let mut ctx: HashMap<&[u8], HashMap<u8, u32>> = HashMap::new();
    for i in k..data.len() {
        *ctx.entry(&data[i - k..i]).or_default().entry(data[i]).or_default() += 1;
    }
    let mut bits = 0.0;
    for counts in ctx.values() {
        let total: u32 = counts.values().sum();
        for &c in counts.values() {
            bits -= f64::from(c) * (f64::from(c) / f64::from(total)).log2();
        }
    }
    bits
}

#[test]
fn round_trips_every_shape_of_input() {
    let mut rng = Mix(1);
    round_trips(&[]);
    round_trips(&[0]);
    round_trips(&[255]);
    round_trips(&(0..=255u8).collect::<Vec<_>>());
    round_trips(&vec![7u8; 100_000]);
    round_trips(&rng.bytes(50_000));
    round_trips(&wordy_text(60_000, 2));
    let mut mixed = wordy_text(20_000, 3);
    mixed.extend(rng.bytes(20_000));
    mixed.extend(std::iter::repeat(0u8).take(20_000));
    mixed.extend(wordy_text(20_000, 4));
    round_trips(&mixed);
}

#[test]
fn compression_is_deterministic() {
    let data = wordy_text(30_000, 5);
    assert_eq!(compress(&data), compress(&data));
}

#[test]
fn random_bytes_cost_at_most_a_sliver_over_raw() {
    let data = Mix(6).bytes(200_000);
    let z = round_trips(&data);
    assert!(z.len() <= data.len() + data.len() / 100 + 32, "{} B for {} random bytes", z.len(), data.len());
}

#[test]
fn a_long_repeated_block_costs_almost_nothing_after_its_first_copy() {
    let block = Mix(7).bytes(10_000);
    let data: Vec<u8> = block.iter().copied().cycle().take(100_000).collect();
    let once = compress(&block).len();
    let all = round_trips(&data).len();
    assert!(all * 100 < once * 105, "ten copies {all} B, one copy {once} B");
}

#[test]
fn structured_text_codes_below_its_order_two_entropy() {
    // The order-2 empirical entropy is what a perfect static order-2 model would pay knowing the counts in
    // advance; a context mixer that also sees words, longer orders and repeats must beat it.
    let data = wordy_text(200_000, 8);
    let z = round_trips(&data);
    let h2 = entropy_order_k(&data, 2) / 8.0;
    assert!((z.len() as f64) < h2 * 0.9, "{} B against order-2 entropy {h2:.0} B", z.len());
    let h0 = entropy_order_k(&data, 0) / 8.0;
    assert!((z.len() as f64) < h0 * 0.5, "{} B against order-0 entropy {h0:.0} B", z.len());
}

#[test]
fn the_decoder_never_panics_and_honours_its_length_cap() {
    let data = wordy_text(1_500, 9);
    let z = compress(&data);
    assert!(decompress(&z, data.len() - 1).is_none(), "a stream longer than the cap is refused");
    // The cap bounds what a hostile length prefix can make the decoder build and run; twice the real
    // length leaves room for every mutated prefix this loop produces without decoding megabytes of noise.
    let cap = data.len() * 2;
    let mut rng = Mix(10);
    for cut in 0..z.len().min(400) {
        let _ = decompress(&z[..cut], cap);
    }
    for _ in 0..300 {
        let mut m = z.clone();
        let i = (rng.next() % m.len() as u64) as usize;
        m[i] ^= 1 << (rng.next() % 8);
        let _ = decompress(&m, cap);
    }
    for len in 0..64 {
        let _ = decompress(&rng.bytes(len), 1 << 16);
    }
}

// ---- Filters: the representations the model is shown -------------------------------------------------

/// A plausible x86 instruction stream: a few dozen instruction templates with near call targets, so the
/// branch filter switches on and the decoder finds real structure.
fn x86_like(n: usize, seed: u64) -> Vec<u8> {
    let mut rng = Mix(seed);
    let mut out = Vec::with_capacity(n + 16);
    let functions: Vec<u32> = (0..64).map(|i| 0x1000 + i * 0x340).collect();
    while out.len() < n {
        let r = rng.next();
        match r % 12 {
            0 => out.extend_from_slice(&[0x55, 0x89, 0xE5]),
            1 => out.extend_from_slice(&[0x83, 0xEC, (r >> 8) as u8 & 0x3C]),
            2 => out.extend_from_slice(&[0x8B, 0x45, 0x08 + ((r >> 8) as u8 & 0x0C)]),
            3 | 4 => {
                let target = functions[(r >> 16) as usize % functions.len()];
                let rel = target.wrapping_sub(out.len() as u32 + 5);
                out.push(0xE8);
                out.extend_from_slice(&rel.to_le_bytes());
            }
            5 => out.extend_from_slice(&[0x89, 0x44, 0x24, (r >> 8) as u8 & 0x1C]),
            6 => out.extend_from_slice(&[0x85, 0xC0, 0x74, (r >> 8) as u8 & 0x3F]),
            7 => out.extend_from_slice(&[0xC9, 0xC3]),
            8 => out.extend_from_slice(&[0x8D, 0x4D, 0xF0 | ((r >> 8) as u8 & 0x0C)]),
            9 => out.extend_from_slice(&[0xB8, (r >> 8) as u8 & 0x0F, 0, 0, 0]),
            10 => out.extend_from_slice(&[0x50 + ((r >> 8) as u8 & 7)]),
            _ => out.extend_from_slice(&[0x0F, 0xB6, 0xC0 | ((r >> 8) as u8 & 0x3F)]),
        }
    }
    out.truncate(n);
    out
}

/// A 16-bit little-endian grayscale image after a `header`-byte prefix: a smooth surface with noise and edges.
fn image16(width: usize, height: usize, header: usize, seed: u64) -> Vec<u8> {
    let mut rng = Mix(seed);
    let mut out = rng.bytes(header);
    for y in 0..height {
        for x in 0..width {
            let surface = 1200.0 + 600.0 * ((x as f64) / 37.0).sin() * ((y as f64) / 23.0).cos();
            let edge = if (x + y / 2) % 97 < 40 { 300.0 } else { 0.0 };
            let noise = (rng.next() % 17) as f64 - 8.0;
            out.extend_from_slice(&((surface + edge + noise).clamp(0.0, 4095.0) as u16).to_le_bytes());
        }
    }
    out
}

#[test]
fn every_filter_path_round_trips() {
    round_trips(&x86_like(200_000, 21));
    round_trips(&image16(320, 200, 0, 22));
    round_trips(&image16(320, 200, 129, 23));
    let mut odd = image16(300, 150, 1, 24);
    odd.push(0xAB);
    round_trips(&odd);
    let text = wordy_text(40_000, 25);
    let z = super::deflate::deflate(&text, super::deflate::Params { level: 6, mem_level: 8, window_bits: 15 });
    let mut zipped = vec![0x78, 0x9C];
    zipped.extend_from_slice(&z);
    zipped.extend(x86_like(100_000, 26));
    round_trips(&zipped);
}

#[test]
fn the_filters_switch_on_for_their_data_and_stay_off_elsewhere() {
    assert!(branch_blocks(&x86_like(200_000, 31)).iter().filter(|&&b| b).count() >= 2);
    assert!(branch_blocks(&wordy_text(200_000, 32)).iter().all(|&b| !b));
    assert!(branch_blocks(&Mix(33).bytes(200_000)).iter().all(|&b| !b));
    let img = image16(500, 160, 37, 34);
    assert_eq!(detect_image16(&img).map(|i| (i.parity, i.width)), Some((1, 500)));
    assert_eq!(detect_image16(&image16(640, 120, 0, 35)).map(|i| (i.parity, i.width)), Some((0, 640)));
    assert!(detect_image16(&wordy_text(400_000, 36)).is_none());
    assert!(detect_image16(&x86_like(400_000, 37)).is_none());
    assert!(detect_image16(&Mix(38).bytes(400_000)).is_none());
}

#[test]
fn near_branch_targets_become_absolute_most_significant_first_and_far_ones_stay() {
    let mut code = vec![0x90u8; 0x500];
    let put = |code: &mut Vec<u8>, at: usize, op: &[u8], rel: i32| {
        code[at..at + op.len()].copy_from_slice(op);
        code[at + op.len()..at + op.len() + 4].copy_from_slice(&rel.to_le_bytes());
    };
    put(&mut code, 0x010, &[0xE9], -0x100);
    put(&mut code, 0x100, &[0xE8], 0x10);
    put(&mut code, 0x200, &[0xE9], -0x20);
    put(&mut code, 0x300, &[0x0F, 0x85], 0x30);
    put(&mut code, 0x400, &[0xE8], 0x7000_0000);
    let on = vec![true; code.len().div_ceil(1 << BRANCH_BLOCK_LOG)];
    let mut body = code.clone();
    branch_filter(&mut body, &on, true);
    // Targets: 0x15 − 0x100 = −0xEB; 0x105 + 0x10 = 0x115; 0x205 − 0x20 = 0x1E5; 0x306 + 0x30 = 0x336 — the low three
    // bytes most significant first, then the top byte, which stays 00 or FF.
    assert_eq!(&body[0x011..0x015], &[0xFF, 0xFF, 0x15, 0xFF]);
    assert_eq!(&body[0x101..0x105], &[0x00, 0x01, 0x15, 0x00]);
    assert_eq!(&body[0x201..0x205], &[0x00, 0x01, 0xE5, 0x00]);
    assert_eq!(&body[0x302..0x306], &[0x00, 0x03, 0x36, 0x00], "a conditional jump's target too");
    assert_eq!(&body[0x401..0x405], &code[0x401..0x405], "a far target is left alone");
    let opcodes = [0x010, 0x100, 0x200, 0x300, 0x301, 0x400];
    assert!(opcodes.iter().all(|&i| body[i] == code[i]), "opcode bytes never change");
    branch_filter(&mut body, &on, false);
    assert_eq!(body, code);
}

#[test]
fn the_branch_filter_and_the_image_residuals_are_exact_inverses_on_any_bytes() {
    let mut rng = Mix(41);
    for len in [0usize, 1, 4, 5, 6, 65_535, 65_536, 65_541, 200_003] {
        let data = rng.bytes(len);
        let on: Vec<bool> = (0..len.div_ceil(1 << BRANCH_BLOCK_LOG)).map(|_| rng.next() % 2 == 0).collect();
        let mut x = data.clone();
        branch_filter(&mut x, &on, true);
        branch_filter(&mut x, &on, false);
        assert_eq!(x, data, "branch filter, {len} bytes");
        for parity in [0, 1] {
            for width in [8, 77, 4096] {
                let mut y = data.clone();
                image_residuals(&mut y, Image { parity, width }, true);
                image_residuals(&mut y, Image { parity, width }, false);
                assert_eq!(y, data, "image residuals, {len} bytes, parity {parity}, width {width}");
            }
        }
    }
}

/// Words of plausible DEC Alpha code after a short unaligned lead: loads and stores off the stack, address arithmetic,
/// operates, calls and branches.
fn alpha_like(bytes: usize, seed: u64) -> Vec<u8> {
    let mut rng = Mix(seed);
    let mut out = wordy_text(333, seed);
    while out.len() < bytes {
        let r = rng.next();
        let reg = |shift: u32| ((r >> shift) & 31) as u32;
        let w: u32 = match r % 8 {
            0 | 1 => 0x29 << 26 | reg(8) << 21 | 30 << 16 | ((r >> 20) as u32 & 0x1F8),
            2 => 0x2D << 26 | reg(8) << 21 | 30 << 16 | ((r >> 20) as u32 & 0x1F8),
            3 => 0x08 << 26 | reg(8) << 21 | reg(13) << 16 | ((r >> 20) as u32 & 0xFF),
            4 | 5 => 0x11 << 26 | reg(8) << 21 | reg(13) << 16 | 0x20 << 5 | reg(18),
            6 => 0x34 << 26 | 26 << 21 | ((r >> 24) as u32 & 0x3FFF),
            _ => 0x39 << 26 | reg(8) << 21 | ((r >> 24) as u32 & 0x7F),
        };
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}


#[test]
fn malformed_filter_headers_are_refused() {
    let data = wordy_text(2_000, 61);
    let z = compress(&data);
    let mut pos = 0;
    let _ = crate::describe::read_uvarint(&z, &mut pos);
    pos += 1;
    for bad_flags in [0x08u8, 0x80, FILTER_BRANCH | FILTER_IMAGE16] {
        let mut m = z.clone();
        m[pos] = bad_flags;
        assert!(decompress(&m, data.len()).is_none(), "flags {bad_flags:#04x}");
    }
    let img = image16(256, 140, 0, 62);
    let zi = compress(&img);
    let mut p = 0;
    let _ = crate::describe::read_uvarint(&zi, &mut p);
    p += 1;
    assert_eq!(zi[p], FILTER_IMAGE16);
    let mut wide = zi[..p + 1].to_vec();
    wide.push(0);
    crate::describe::write_uvarint(IMAGE_MAX_WIDTH as u64 + 1, &mut wide);
    assert!(decompress(&wide, img.len()).is_none(), "a width past the detector's range");
    let mut parity = zi[..p + 1].to_vec();
    parity.push(2);
    assert!(decompress(&parity, img.len()).is_none(), "parity 2");
}

#[test]
fn the_modeled_byte_cap_bounds_the_decoder_and_the_encoder_respects_it() {
    let text = wordy_text(60_000, 63);
    let z = super::deflate::deflate(&text, super::deflate::Params { level: 6, mem_level: 8, window_bits: 15 });
    let mut zl = vec![0x78, 0x9C];
    zl.extend_from_slice(&z);
    let opened = compress(&zl);
    assert!(decompress_within(&opened, zl.len(), zl.len()).is_none(), "the opened text is more than the cap allows");
    assert_eq!(decompress_within(&opened, zl.len(), text.len() + 64).as_deref(), Some(&zl[..]));
    let closed = compress_with(&zl, Level::Max, zl.len());
    assert_eq!(decompress_within(&closed, zl.len(), zl.len()).as_deref(), Some(&zl[..]), "the encoder kept the stream shut");
}

#[test]
fn the_fast_level_round_trips_every_shape_and_stays_within_reach_of_max() {
    let mut rng = Mix(71);
    let text = wordy_text(80_000, 72);
    let shapes = [Vec::new(), vec![9], rng.bytes(30_000), text.clone(), x86_like(100_000, 73), image16(256, 140, 1, 74)];
    for data in &shapes {
        let z = compress_with(data, Level::Fast, usize::MAX);
        assert_eq!(decompress(&z, data.len()).as_deref(), Some(&data[..]), "fast, {} bytes", data.len());
    }
    let fast = compress_with(&text, Level::Fast, usize::MAX).len();
    let max = compress(&text).len();
    assert!(max < fast, "max {max} B, fast {fast} B");
    assert!(fast * 100 < max * 125, "fast {fast} B is within a quarter of max {max} B");
}

#[test]
fn the_level_is_named_in_the_header_and_an_unknown_one_is_refused() {
    let data = wordy_text(3_000, 75);
    for (level, byte) in [(Level::Fast, 1u8), (Level::Max, 2)] {
        let z = compress_with(&data, level, usize::MAX);
        let mut pos = 0;
        let _ = crate::describe::read_uvarint(&z, &mut pos);
        assert_eq!(z[pos], byte);
        for bad in [0u8, 3, 255] {
            let mut m = z.clone();
            m[pos] = bad;
            assert!(decompress(&m, data.len()).is_none(), "level byte {bad}");
        }
    }
}

#[test]
fn the_cost_profile_adds_up_to_what_the_coder_writes() {
    for (data, level) in [(wordy_text(60_000, 81), Level::Max), (x86_like(80_000, 82), Level::Max), (wordy_text(40_000, 83), Level::Fast)] {
        let (body, costs) = cost_profile(&data, level);
        assert_eq!(body, represent(&data, usize::MAX).body);
        assert_eq!(costs.len(), body.len());
        let profiled = costs.iter().map(|&c| u64::from(c)).sum::<u64>() / 256 / 8;
        let coded = compress_with(&data, level, usize::MAX).len() as u64;
        assert!(
            coded.abs_diff(profiled) <= profiled / 100 + 64,
            "the profile says {profiled} B, the coder wrote {coded} B (header included)"
        );
    }
}

#[test]
fn the_cost_profile_is_exact_where_the_model_is_almost_certain() {
    // Past its first copies nearly every bit of a repeated line is coded at P = 4095/4096, which costs 0.00035
    // bits; a profile that rounded each bit to 1/256 bit would charge eleven times that.
    let line = b"C1=CC=C(C=C1)C(=O)O  benzoic acid  122.12 g/mol  mp 122 C\n";
    let data: Vec<u8> = line.iter().copied().cycle().take(line.len() * 4000).collect();
    for level in [Level::Max, Level::Fast] {
        let (_, costs) = cost_profile(&data, level);
        let profiled = costs.iter().map(|&c| u64::from(c)).sum::<u64>().div_ceil(256 * 8);
        let coded = compress_with(&data, level, usize::MAX).len() as u64;
        assert!(
            coded.abs_diff(profiled) <= coded / 50 + 16,
            "{level:?}: the profile says {profiled} B, the coder wrote {coded} B (header included)"
        );
    }
}

// ---- Image regions: opened GIF and PNG data modeled as rows -------------------------------------------

/// An indexed image `width` × `height`: flat shapes on a background, one palette index a pixel.
fn shapes(width: usize, height: usize, seed: u64) -> Vec<u8> {
    let mut rng = Mix(seed);
    let boxes: Vec<(usize, usize, usize, usize, u8)> = (0..12)
        .map(|_| {
            let (x, y) = ((rng.next() % width as u64) as usize, (rng.next() % height as u64) as usize);
            (x, y, 8 + (rng.next() % 40) as usize, 6 + (rng.next() % 30) as usize, (1 + rng.next() % 14) as u8)
        })
        .collect();
    let mut out = vec![0u8; width * height];
    for (y, row) in out.chunks_mut(width).enumerate() {
        for (x, p) in row.iter_mut().enumerate() {
            for &(bx, by, bw, bh, c) in &boxes {
                if (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y) {
                    *p = c;
                }
            }
            if rng.next() % 50 == 0 {
                *p ^= 1;
            }
        }
    }
    out
}

/// A GIF89a holding `pixels` as one `width`-wide image, LZW in 254-byte sub-blocks.
fn gif_file(pixels: &[u8], width: usize) -> Vec<u8> {
    let height = pixels.len() / width;
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.extend_from_slice(&[0x83, 0, 0]);
    out.extend((0..48).map(|i| i as u8));
    out.extend_from_slice(&[0x2C, 0, 0, 0, 0]);
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.extend_from_slice(&[0, 4]);
    for piece in super::lzw::encode(pixels, 4, 4096).chunks(254) {
        out.push(piece.len() as u8);
        out.extend_from_slice(piece);
    }
    out.extend_from_slice(&[0, 0x3B]);
    out
}

#[test]
fn an_opened_gif_becomes_an_image_region_of_its_width() {
    let pixels = shapes(200, 150, 101);
    let mut data = wordy_text(3_000, 102);
    let at = data.len();
    data.extend(gif_file(&pixels, 200));
    let r = represent(&data, usize::MAX);
    assert_eq!(r.streams.len(), 1);
    let start = r.streams[0].offset;
    assert!(start > at);
    assert_eq!(r.regions, vec![Region { start, end: start + pixels.len(), stride: 200, bpp: 1, lead: 0, palette: true }]);
    assert_eq!(&r.body[start..start + pixels.len()], &pixels[..]);
    round_trips(&data);
}

#[test]
fn the_image_region_model_pays_for_itself_on_a_gif() {
    let pixels = shapes(320, 240, 103);
    let data = gif_file(&pixels, 320);
    let r = represent(&data, usize::MAX);
    assert_eq!(r.regions.len(), 1);
    let with = encode_body(&r.body, Level::Max, None, Vec::new(), r.regions.clone());
    let without = encode_body(&r.body, Level::Max, None, Vec::new(), Vec::new());
    assert!(with.len() * 100 < without.len() * 90, "image regions {} B, byte model {} B", with.len(), without.len());
}

#[test]
fn the_region_group_is_live_inside_a_region_and_nowhere_else() {
    let region = Region { start: 100, end: 300, stride: 20, bpp: 1, lead: 0, palette: true };
    let mut model = Model::new(1 << 12, Level::Max, None, Vec::new(), vec![region]);
    let mut meter = CostMeter(0);
    for i in 0..400usize {
        let inside = (region.start..region.end).contains(&i);
        assert_eq!(model.region_on, inside && i > 0, "byte {i}");
        model.code_byte(&mut meter, (i % 7) as u8);
    }
}

#[test]
fn a_records_field_carries_its_growth_into_every_byte() {
    // Relocations of 16 bytes whose first field grows by 8, carrying from byte to byte: every byte of it is predicted.
    let mut history = Vec::new();
    let mut misses = 0;
    for k in 0..40u32 {
        let offset = 0xFFD3_12E8u32.wrapping_add(8 * k);
        let record: Vec<u8> = offset.to_le_bytes().into_iter().chain([2, 1, 0, 0]).chain([0; 8]).collect();
        for &b in &record {
            if history.len() % 16 < 4 && k >= 2 {
                misses += usize::from(field_carry(&history, 16) != Some(b));
            }
            history.push(b);
        }
    }
    assert_eq!(misses, 0, "every byte of the growing field is the one predicted");
    // A field that changes by a different amount each time: the low byte is a guess, the bytes above follow it.
    let mut h = vec![0u8; 32];
    h[0..4].copy_from_slice(&0x0000_10F0u32.to_le_bytes());
    h[16..20].copy_from_slice(&0x0000_10F8u32.to_le_bytes());
    h.push(0x05);
    assert_eq!(field_carry(&h, 16), Some(0x11), "0x10F8 + 0x0D carries into the second byte");
    assert_eq!(field_carry(&h[..20], 16), None, "before two records of history");
    assert_eq!(field_carry(&h, 3), None, "records shorter than a field");
}

#[test]
fn alpha_code_and_an_image_region_in_one_stream_each_keep_their_own_context_slots() {
    let mut data = alpha_like(200_000, 81);
    data.extend(gif_file(&shapes(200, 150, 82), 200));
    let r = represent(&data, usize::MAX);
    assert!(!r.alpha.is_empty() && r.regions.len() == 1, "both the Alpha path and the image-region path are on");
    let m = Model::coded(r.body.len(), Level::Max, r.image, r.exe.clone(), r.alpha.clone(), r.regions.clone());
    let ids = &m.ids[..m.allocated];
    let distinct: std::collections::HashSet<usize> = ids.iter().copied().collect();
    assert_eq!(distinct.len(), ids.len(), "no two slots read one context");
    assert!((0..ALPHAS).all(|i| ids.contains(&(ALPHA + i))) && (0..IMAGE8S).all(|i| ids.contains(&(IMAGE8 + i))));
    assert!(ids.iter().all(|&id| id < SLOTS));
    round_trips(&data);
}

#[test]
fn two_confident_calibrations_agree_geometrically_and_disagreeing_ones_average() {
    const ONE: u64 = 1 << 32;
    let sure = |miss: u64| (ONE - miss) as u32;
    // Both sure of a 1: the miss probabilities' geometric mean, 1e-5 and 1e-3 of 2^32 giving 1e-4.
    let (a, b) = (sure(42_950), sure(4_294_967));
    assert_eq!(agree(a, b), ONE - (42_950u64 * 4_294_967).isqrt());
    assert!(ONE - agree(a, b) < (42_950 + 4_294_967) / 2, "surer than the mean");
    // Both sure of a 0, likewise; disagreeing, the mean.
    assert_eq!(agree(42_950, 4_294_967), (42_950u64 * 4_294_967).isqrt());
    assert_eq!(agree(sure(1000), 1000), ((ONE - 1000) + 1000) / 2);
    for (x, y) in [(1u32, 1u32), (u32::MAX, u32::MAX), (1, u32::MAX), (1 << 31, (1 << 31) - 1), (7, 9)] {
        let z = agree(x, y);
        assert!((1..ONE).contains(&z), "{x}, {y} -> {z}");
        assert_eq!(z, agree(y, x));
    }
}

#[test]
fn a_records_falling_field_borrows_into_every_byte() {
    let mut history = Vec::new();
    let mut misses = 0;
    for k in 0..40u32 {
        let offset = 0x0001_0010u32.wrapping_sub(8 * k);
        for &b in offset.to_le_bytes().iter().chain(&[0u8; 12]) {
            if history.len() % 16 < 4 && k >= 2 {
                misses += usize::from(field_carry(&history, 16) != Some(b));
            }
            history.push(b);
        }
    }
    assert_eq!(misses, 0, "every byte of the falling field is the one predicted");
}

// ---- Nested streams: a stream inside an opened stream --------------------------------------------------------------

/// A zlib stream of `inner` at level 6.
fn zlib(inner: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0x9C];
    z.extend_from_slice(&super::deflate::deflate(inner, super::deflate::Params { level: 6, mem_level: 8, window_bits: 15 }));
    z
}

#[test]
fn a_gif_inside_a_deflate_stream_is_opened_to_its_pixels_too() {
    let pixels = shapes(200, 150, 111);
    let mut data = wordy_text(2_000, 112);
    data.extend(zlib(&gif_file(&pixels, 200)));
    data.extend(wordy_text(1_000, 113));
    let r = represent(&data, usize::MAX);
    assert_eq!(r.streams.len(), 1, "the DEFLATE stream, at the first level");
    assert_eq!(r.deeper.iter().map(Vec::len).collect::<Vec<_>>(), vec![1], "the GIF inside it, at the second");
    assert_eq!(r.regions.len(), 1);
    let g = r.regions[0];
    assert_eq!((g.stride, g.bpp, g.lead), (200, 1, 0));
    assert_eq!(&r.body[g.start..g.end], &pixels[..], "the region is where the body holds the pixels");
    round_trips(&data);
}

#[test]
fn regions_of_every_level_land_where_the_body_holds_their_pixels() {
    let images = [(shapes(120, 80, 121), 120), (shapes(160, 90, 122), 160), (shapes(100, 70, 123), 100)];
    let mut data = wordy_text(1_500, 124);
    data.extend(gif_file(&images[0].0, 120));
    data.extend(wordy_text(700, 125));
    data.extend(zlib(&gif_file(&images[1].0, 160)));
    data.extend(wordy_text(700, 126));
    data.extend(gif_file(&images[2].0, 100));
    let r = represent(&data, usize::MAX);
    assert_eq!((r.streams.len(), r.deeper.len()), (3, 1), "two GIFs and a DEFLATE stream, then the GIF inside it");
    assert_eq!(r.regions.len(), 3);
    for (g, (pixels, width)) in r.regions.iter().zip(&images) {
        assert_eq!(g.stride, *width);
        assert_eq!(&r.body[g.start..g.end], &pixels[..], "the {width}-wide image's region");
    }
    assert!(r.regions.windows(2).all(|w| w[0].end <= w[1].start), "in order, apart");
    round_trips(&data);
}

#[test]
fn a_nested_header_the_decoder_must_not_trust_is_refused() {
    // A zlib stream holding a GIF and then text it squeezes: the first level opens to well past the original's length.
    let mut data = wordy_text(2_000, 131);
    let mut inner = gif_file(&shapes(200, 150, 132), 200);
    inner.extend(wordy_text(20_000, 133));
    data.extend(zlib(&inner));
    let z = compress(&data);
    let mut at = 0;
    read_uvarint(&z, &mut at).unwrap();
    let f = at + 1;
    assert_eq!(z[f] & (FILTER_STREAMS | FILTER_NESTED), FILTER_STREAMS | FILTER_NESTED);
    let mut lone = z.clone();
    lone[f] &= !FILTER_STREAMS;
    assert!(decompress(&lone, data.len()).is_none(), "nested levels without a first");
    // The headers decoded, changed and coded again in place.
    let mut pos = f + 1;
    let head = read_packed(&z, &mut pos, data.len()).unwrap();
    let with = |h: &[u8]| {
        let mut out = z[..f + 1].to_vec();
        write_packed(h, &mut out);
        out.extend_from_slice(&z[pos..]);
        out
    };
    assert_eq!(with(&head), z, "the headers code back to the same bytes");
    let mut hp = 0;
    let (_, opened) = super::streams::read_header(&head, &mut hp, data.len()).unwrap();
    for count in [0u8, NESTED_LEVELS as u8] {
        let mut bad = head.clone();
        bad[hp] = count;
        assert!(decompress(&with(&bad), data.len()).is_none(), "{count} nested levels");
        assert!(read_streams(&with(&bad), &mut (f + 1), data.len(), z[f]).is_none(), "{count} nested levels, at the header");
    }
    // The second level's one stream, its decoded length raised so the level opens to one byte past what the original
    // may — still within what the first level's larger body may, so only the bound on the original refuses it.
    let bound = super::streams::expansion_bound(data.len());
    assert!(opened > data.len() && opened < bound, "the first level opens to {opened} of {}", data.len());
    let (_, second) = super::streams::read_header(&head, &mut (hp + 1), opened).unwrap();
    assert!(second <= bound);
    let mut p = hp + 1;
    read_uvarint(&head, &mut p).unwrap();
    for _ in 0..2 {
        read_uvarint(&head, &mut p).unwrap();
    }
    let raw_at = p;
    let raw = usize::try_from(read_uvarint(&head, &mut p).unwrap()).unwrap();
    let mut past = head[..raw_at].to_vec();
    write_uvarint((raw + bound + 1 - second) as u64, &mut past);
    past.extend_from_slice(&head[p..]);
    let (_, raised) = super::streams::read_header(&past, &mut (hp + 1), opened).expect("within the first level's bound");
    assert_eq!(raised, bound + 1);
    assert!(decompress(&with(&past), data.len()).is_none(), "a second level opening past the original's bound");
    assert!(read_streams(&with(&past), &mut (f + 1), data.len(), z[f]).is_none(), "refused at the header, before modeling");
    assert!(read_streams(&z, &mut (f + 1), data.len(), z[f]).is_some(), "the untouched header reads");
    let mut extra = head.clone();
    extra.push(0);
    assert!(decompress(&with(&extra), data.len()).is_none(), "header bytes past the last level");
    assert!(read_streams(&with(&extra), &mut (f + 1), data.len(), z[f]).is_none(), "trailing header bytes, at the header");
}

/// A GIF89a of `pixels` (any byte values) as one `width`-wide image with a 256-colour palette.
fn gif_file8(pixels: &[u8], width: usize) -> Vec<u8> {
    let height = pixels.len() / width;
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.extend_from_slice(&[0x87, 0, 0]);
    out.extend((0..768).map(|i| (i / 3) as u8));
    out.extend_from_slice(&[0x2C, 0, 0, 0, 0]);
    out.extend_from_slice(&(width as u16).to_le_bytes());
    out.extend_from_slice(&(height as u16).to_le_bytes());
    out.extend_from_slice(&[0, 8]);
    for piece in super::lzw::encode(pixels, 8, 4096).chunks(254) {
        out.push(piece.len() as u8);
        out.extend_from_slice(piece);
    }
    out.extend_from_slice(&[0, 0x3B]);
    out
}

#[test]
fn a_stream_inside_an_opened_images_pixels_stays_pixels() {
    // Pixels whose bytes happen to hold a zlib stream that would open: they are an image's, and stay as they are.
    let mut pixels = shapes(256, 120, 141);
    let inner = zlib(&wordy_text(4_000, 142));
    pixels[5_000..5_000 + inner.len()].copy_from_slice(&inner);
    let mut data = wordy_text(1_000, 143);
    data.extend(zlib(&gif_file8(&pixels, 256)));
    let r = represent(&data, usize::MAX);
    assert_eq!(r.deeper.iter().map(Vec::len).collect::<Vec<_>>(), vec![1], "the GIF opens; nothing inside its pixels does");
    assert_eq!(&r.body[r.regions[0].start..r.regions[0].end], &pixels[..]);
    round_trips(&data);
}

#[test]
fn nested_levels_open_only_within_the_modeled_byte_cap() {
    let pixels = shapes(200, 150, 151);
    let gif = gif_file(&pixels, 200);
    let mut data = wordy_text(2_000, 152);
    data.extend(zlib(&gif));
    let first_level = data.len() - zlib(&gif).len() + gif.len();
    let capped = compress_with(&data, Level::Max, first_level);
    assert_eq!(decompress_within(&capped, data.len(), first_level).as_deref(), Some(&data[..]), "the GIF stayed shut");
    let r = represent(&data, first_level);
    assert!(r.deeper.is_empty() && r.streams.len() == 1);
}

#[test]
fn the_streams_headers_are_entropy_coded() {
    // Two hundred small GIFs: their headers repeat field for field, and coded they take a fraction of their bytes.
    let mut data = Vec::new();
    for k in 0..200u64 {
        data.extend(gif_file(&shapes(24, 16, 300 + k), 24));
        data.extend_from_slice(b"\n<img>\n");
    }
    let r = represent(&data, usize::MAX);
    assert!(r.streams.len() >= 190, "{} streams", r.streams.len());
    let mut head = Vec::new();
    super::streams::write_header(&r.streams, &mut head);
    let mut packed = Vec::new();
    write_packed(&head, &mut packed);
    assert!(packed.len() * 2 < head.len(), "{} header bytes coded into {}", head.len(), packed.len());
    let mut pos = 0;
    assert_eq!(read_packed(&packed, &mut pos, data.len()).as_deref(), Some(&head[..]));
    assert_eq!(pos, packed.len());
    assert!(read_packed(&packed[..packed.len() - 1], &mut 0, data.len()).is_none(), "cut short");
    // The same coded bytes claiming one header byte more than the original could need: 64 a byte and 4 KiB.
    let mut at = 0;
    read_uvarint(&packed, &mut at).unwrap();
    let coded_len = read_uvarint(&packed, &mut at).unwrap();
    let mut huge = Vec::new();
    write_uvarint((data.len() * 64 + (1 << 12) + 1) as u64, &mut huge);
    write_uvarint(coded_len, &mut huge);
    huge.extend_from_slice(&packed[at..]);
    assert!(read_packed(&huge, &mut 0, data.len()).is_none(), "more header than the original could need");
    round_trips(&data);
}

/// A screenshot's look: lines of glyphs (24 random 6x9 bitmaps) in one of three ink colours on a light ground with a
/// few panels, every colour an index scattered over 0..=255.
fn glyph_screen(width: usize, height: usize, seed: u64) -> Vec<u8> {
    let mut rng = Mix(seed);
    let colours: Vec<u8> = (0..8).map(|_| (rng.next() % 256) as u8).collect();
    let glyphs: Vec<Vec<bool>> = (0..24).map(|_| (0..54).map(|_| rng.next() % 3 == 0).collect()).collect();
    let mut out = vec![colours[0]; width * height];
    for _ in 0..4 {
        let (x0, y0) = ((rng.next() % width as u64) as usize, (rng.next() % height as u64) as usize);
        let (w, h) = (40 + (rng.next() % 120) as usize, 20 + (rng.next() % 60) as usize);
        let c = colours[1 + (rng.next() % 2) as usize];
        for y in y0..(y0 + h).min(height) {
            for x in x0..(x0 + w).min(width) {
                out[y * width + x] = c;
            }
        }
    }
    let mut y = 2;
    while y + 9 < height {
        let ink = colours[3 + (rng.next() % 3) as usize];
        let mut x = 2 + (rng.next() % 8) as usize;
        while x + 6 < width && rng.next() % 40 != 0 {
            let g = &glyphs[(rng.next() % 24) as usize];
            for gy in 0..9 {
                for gx in 0..6 {
                    if g[gy * 6 + gx] {
                        out[(y + gy) * width + x + gx] = ink;
                    }
                }
            }
            x += 7;
        }
        y += 12;
    }
    out
}

#[test]
fn inside_a_palette_image_the_weight_sets_follow_which_neighbours_match() {
    // One flat colour: every interior pixel's neighbours all match, the eighth weight set of its channel; the same bytes
    // as a grey image fall in the activity bands instead, the first (no activity).
    for (palette, class) in [(true, 7), (false, 0)] {
        let region = Region { start: 0, end: 64 * 40, stride: 64, bpp: 1, lead: 0, palette };
        let mut model = Model::new(64 * 40, Level::Max, None, Vec::new(), vec![region]);
        let mut meter = CostMeter(0);
        for i in 0..64 * 40 - 1 {
            model.code_byte(&mut meter, 77);
            let next = i + 1;
            if next >= 64 * 2 && (2..60).contains(&(next % 64)) {
                assert_eq!(model.image_select, EXE_SELECT + class, "palette {palette}, pixel {next}");
                assert_eq!(model.third_select(0), (EXE_SELECT + class) * 256 + 1, "the region's own weight sets");
            }
        }
    }
}

#[test]
fn a_screenful_of_glyphs_under_a_scattered_palette_codes_under_its_lock() {
    // Lines of glyphs on panels, every colour an index scattered over 0..=255: the exact neighbourhoods near and far
    // carry what a palette image's indices say.
    let data = gif_file8(&glyph_screen(320, 240, 161), 320);
    let r = represent(&data, usize::MAX);
    assert!(r.regions.len() == 1 && r.regions[0].palette, "one palette region");
    let z = compress(&data);
    assert!(z.len() <= 4_115, "{} bytes", z.len());
    assert_eq!(decompress(&z, data.len()).as_deref(), Some(&data[..]));
}

#[test]
fn each_image_region_keeps_its_own_statistics() {
    // Two flat images in one stream: at the same place in each, their neighbourhoods match, their contexts do not.
    let regions = vec![
        Region { start: 0, end: 64 * 20, stride: 64, bpp: 1, lead: 0, palette: true },
        Region { start: 64 * 20 + 100, end: 64 * 40 + 100, stride: 64, bpp: 1, lead: 0, palette: true },
    ];
    let len = 64 * 40 + 100;
    let mut model = Model::new(len, Level::Max, None, Vec::new(), regions.clone());
    let mut meter = CostMeter(0);
    let mut seen = Vec::new();
    for i in 0..len - 1 {
        model.code_byte(&mut meter, 77);
        for r in &regions {
            if i + 1 == r.start + 64 * 5 + 10 {
                assert!(model.region_on);
                seen.push(model.base[IMAGE8]);
            }
        }
    }
    assert_eq!(seen.len(), 2);
    assert_ne!(seen[0], seen[1], "each image's contexts its own");
}

#[test]
fn every_slot_reads_one_table_holding_the_lines_the_slots_bring() {
    // A slot brings 2^(bits − 2) lines, a PDF slot a quarter of that (it is live only inside PDF strings).
    for len in [1usize << 10, 1 << 20, 3 << 20] {
        for (exe, alpha) in [(Vec::new(), Vec::new()), (vec![true; len.div_ceil(1 << BRANCH_BLOCK_LOG)], Vec::new())] {
            let m = Model::coded(len, Level::Max, None, exe, alpha, Vec::new());
            let bits = (usize::BITS - len.leading_zeros()).saturating_sub(5).clamp(10, 22);
            let pdf_bits = bits.saturating_sub(2).max(10);
            let lines = (m.pdf_start << (bits - 2)) + ((m.allocated - m.pdf_start) << (pdf_bits - 2));
            assert_eq!(m.table.first.len(), lines, "{len} bytes, {} slots", m.allocated);
            assert_eq!(m.table.second.len(), 4 * lines);
        }
    }
}

#[test]
fn a_context_that_meets_many_histories_takes_the_room_idle_slots_leave() {
    // 4,000 distinct contexts of one slot, five times what that slot's own share (2^8 lines of three ways) holds: in the
    // shared table, which the other slots leave mostly empty, nearly every one is still there when it comes back.
    let mut m = Model::coded(1 << 10, Level::Max, None, Vec::new(), Vec::new(), Vec::new());
    let hashes: Vec<u64> = (0..4000u64).map(|i| hash2(0x4D41_4E59, i)).collect();
    for &h in &hashes {
        let b = m.table.first_bucket(h);
        m.table.set_state(true, b, 1, 0x31);
    }
    let kept = hashes.iter().filter(|&&h| {
        let b = m.table.first_bucket(h);
        m.table.state(true, b, 1) == 0x31
    }).count();
    assert!(kept >= 3900, "{kept} of 4000 contexts kept their histories");
}

#[test]
fn two_contexts_whose_hashes_agree_keep_their_own_histories() {
    let mut m = Model::coded(1 << 12, Level::Max, None, Vec::new(), Vec::new(), Vec::new());
    let (a, b) = (0, 1);
    let (ia, ib) = (m.ids[a], m.ids[b]);
    m.base[ia] = 0x1234_5678_9ABC_DEF0;
    m.base[ib] = 0x1234_5678_9ABC_DEF0;
    assert_ne!(m.slot_hash(a), m.slot_hash(b));
    let ha = hash2(m.slot_hash(a), 1);
    let ba = m.table.first_bucket(ha);
    m.table.set_state(true, ba, 1, 0x52);
    let hb = hash2(m.slot_hash(b), 1);
    let bb = m.table.first_bucket(hb);
    assert_eq!(m.table.state(true, bb, 1), 0, "slot b starts fresh instead of reading slot a's history");
    let ba = m.table.first_bucket(ha);
    assert_eq!(m.table.state(true, ba, 1), 0x52, "and slot a still has its own");
}

#[test]
fn a_table_of_any_line_count_spreads_hashes_over_every_line_and_tells_a_lines_hashes_apart() {
    let t = ContextTable::new(3);
    let mut hits = [0usize; 3];
    for i in 0..30_000u64 {
        let h = hash2(0x4C49_4E45, i);
        hits[t.first_line(h)] += 1;
        assert!(t.second_line(h) + 4 <= t.second.len());
    }
    assert!(hits.iter().all(|&n| (9_000..11_000).contains(&n)), "{hits:?}");
    // The line is the hash's high bits: two hashes alike there but not below share a line and must not share a bucket.
    let mut t = ContextTable::new(1);
    let (h1, h2) = (0xABCD_EF01_0000_1111u64, 0xABCD_EF01_0000_2222u64);
    let b = t.first_bucket(h1);
    t.set_state(true, b, 1, 0x25);
    let b = t.first_bucket(h2);
    assert_eq!(t.state(true, b, 1), 0, "the second hash gets a bucket of its own");
    let b = t.first_bucket(h1);
    assert_eq!(t.state(true, b, 1), 0x25);
}

#[test]
fn a_second_nibble_check_tells_apart_check_bytes_that_differ_in_their_lowest_bit() {
    // Two second-nibble hashes alike but in bit 16, their check byte's lowest bit (2 against 3): they share a line,
    // and the check must still tell them apart, so the second takes a bucket of its own.
    let mut t = ContextTable::new(1);
    let (h1, h2) = (0xABCD_EF01_0002_1111u64, 0xABCD_EF01_0003_1111u64);
    assert_eq!(t.second_line(h1), t.second_line(h2));
    let b = t.second_bucket(h1);
    t.set_state(false, b, 1, 0x25);
    let b = t.second_bucket(h2);
    assert_eq!(t.state(false, b, 1), 0, "the second hash gets a bucket of its own");
    let b = t.second_bucket(h1);
    assert_eq!(t.state(false, b, 1), 0x25, "and the first keeps its histories");
    // Only zero, the empty bucket's mark, is not a check: every other byte value is one.
    let checks: std::collections::BTreeSet<u8> = (0..256u64)
        .map(|v| {
            let mut t = ContextTable::new(1);
            let b = t.second_bucket(0xABCD_EF01_0000_1111 | v << 16);
            t.second[b.line as usize][0]
        })
        .collect();
    assert_eq!(checks.len(), 255);
    assert!(!checks.contains(&0));
}

#[test]
fn a_bit_history_keeps_its_last_three_bits_in_order() {
    let walk = |bits: &[u8]| bits.iter().fold(0u8, |s, &b| next_state(s, b == 1));
    // The same counts reached in another order are another history.
    let (a, b) = (walk(&[0, 0, 1, 1]), walk(&[1, 1, 0, 0]));
    assert_eq!(counts(a), (2, 2));
    assert_eq!(counts(a), counts(b));
    assert_ne!(a, b, "0011 and 1100 predict apart");
    // Only the last three are kept: histories alike in counts and in their last three bits are one.
    assert_eq!(walk(&[0, 1, 0, 1, 1]), walk(&[1, 0, 0, 1, 1]));
    // The counts still count, and a bit past two of the other kind roughly halves it.
    assert_eq!(counts(walk(&[1; 20])), (0, 15));
    assert_eq!(counts(walk(&[1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0])), (1, 6));
}

#[test]
fn every_reachable_bit_history_has_a_byte_and_none_returns_to_empty() {
    let mut seen = std::collections::HashSet::from([0u8]);
    let mut todo = vec![0u8];
    while let Some(s) = todo.pop() {
        for bit in [false, true] {
            let t = next_state(s, bit);
            assert_ne!(t, 0, "state 0 is the empty history alone");
            if seen.insert(t) {
                todo.push(t);
            }
        }
    }
    assert!(seen.len() > 200 && seen.len() <= 256, "{} histories", seen.len());
    assert_eq!(counts(0), (0, 0));
    assert!(!is_mixed(next_state(0, true)) && is_mixed(next_state(next_state(0, true), false)));
}

/// An order-1 Markov source: every byte has four distinct successors, at 1/2, 1/4, 1/8 and 1/8 — 1.75 bits a byte.
fn markov_source(rng: &mut Mix) -> Vec<u8> {
    let succ: Vec<[u8; 4]> = (0..256)
        .map(|_| {
            let mut row = [0u8; 4];
            let mut k = 0;
            while k < 4 {
                let b = rng.next() as u8;
                if !row[..k].contains(&b) {
                    row[k] = b;
                    k += 1;
                }
            }
            row
        })
        .collect();
    let mut c = 0u8;
    (0..1 << 18)
        .map(|_| {
            c = succ[usize::from(c)][match rng.next() % 8 {
                0..=3 => 0,
                4 | 5 => 1,
                6 => 2,
                _ => 3,
            }];
            c
        })
        .collect()
}

/// The byte three back picks which lag repeats: under 128 the last byte, otherwise the one before it; one byte in
/// eight is fresh.
fn lag_source(rng: &mut Mix) -> Vec<u8> {
    let mut data = vec![1u8, 2, 3];
    while data.len() < 1 << 17 {
        let n = data.len();
        let next = if rng.next() % 8 == 0 {
            rng.next() as u8
        } else if data[n - 3] < 128 {
            data[n - 1]
        } else {
            data[n - 2]
        };
        data.push(next);
    }
    data
}

fn bits_per_byte(data: &[u8]) -> f64 {
    let (body, costs) = cost_profile(data, Level::Max);
    assert_eq!(body, data, "no representation change on these sources");
    costs.iter().map(|&c| u64::from(c)).sum::<u64>() as f64 / 256.0 / data.len() as f64
}

#[test]
fn a_stationary_order_one_source_codes_within_a_sliver_of_its_entropy_rate() {
    // Order 1's own probabilities, learned with no bit history between, are what close the gap on a source that
    // stationary.
    let per_byte = bits_per_byte(&markov_source(&mut Mix(0x4D4B_31)));
    assert!(per_byte < 1.75 + 0.0765, "{per_byte:.5} bits a byte against 1.75");
}

#[test]
fn weight_sets_chosen_by_the_byte_three_back_learn_which_lag_repeats() {
    let per_byte = bits_per_byte(&lag_source(&mut Mix(0x4C41_47)));
    assert!(per_byte < 1.652, "{per_byte:.5} bits a byte");
}

#[test]
fn the_match_contexts_tell_one_prediction_apart_by_the_bytes_before_it() {
    // Two inputs whose match predicts the same byte at the same length, after another last byte and so another
    // order-3 context: the first match context is the same for both, the other two are not.
    let fed = |text: &[u8]| {
        let mut m = Model::coded(text.len(), Level::Max, None, Vec::new(), Vec::new(), Vec::new());
        let mut meter = CostMeter(0);
        for &b in text {
            m.code_byte(&mut meter, b);
        }
        m
    };
    let a = fed(b"0123456789abcdefghIJklmn=0123456789abcdefghI");
    let b = fed(b"0123456789abcdefghXJklmn=0123456789abcdefghX");
    for m in [&a, &b] {
        assert!(m.match_len > 0, "the second copy is matched");
        assert_eq!(m.history[m.match_ptr], b'J', "and the match predicts J");
    }
    assert_eq!(a.match_len, b.match_len);
    assert_eq!(a.base[MATCHED], b.base[MATCHED], "one prediction at one length band");
    assert_ne!(a.base[MATCHED + 1], b.base[MATCHED + 1], "after another byte");
    assert_ne!(a.base[MATCHED + 2], b.base[MATCHED + 2], "in another order-3 context");
}

/// Phrases of a fixed list, each word a function of the phrase's first word and its place in it; phrases joined by a
/// comma or a full stop.
fn phrase_list(rng: &mut Mix) -> Vec<u8> {
    let word = |rng: &mut Mix| {
        let len = 3 + rng.next() % 5;
        (0..len).map(|_| b'a' + (rng.next() % 26) as u8).collect::<Vec<u8>>()
    };
    let vocab: Vec<Vec<u8>> = (0..64).map(|_| word(rng)).collect();
    let table: Vec<Vec<usize>> = (0..64).map(|_| (0..6).map(|_| (rng.next() % 64) as usize).collect()).collect();
    let mut out = Vec::new();
    while out.len() < 1 << 17 {
        let first = (rng.next() % 64) as usize;
        let len = 2 + (rng.next() % 4) as usize;
        let words: Vec<&[u8]> = std::iter::once(first).chain(table[first][..len].iter().copied()).map(|w| vocab[w].as_slice()).collect();
        out.extend(words.join(&b' '));
        out.extend_from_slice(if rng.next() % 3 != 0 { b", " } else { b". " });
    }
    out.truncate(1 << 17);
    out
}

#[test]
fn a_phrase_says_what_follows_its_start() {
    // Each word follows from its phrase's first word and its place in the phrase; what precedes the phrase is noise.
    let per_byte = bits_per_byte(&phrase_list(&mut Mix(0x504852)));
    assert!(per_byte < 0.379, "{per_byte:.5} bits a byte");
}

#[test]
fn the_prose_contexts_are_live_only_where_text_is_prose() {
    // A table of numbers is text, but too few of its bytes are letters for the prose contexts; words turn them on.
    let fed = |text: &[u8]| {
        let mut m = Model::coded(text.len(), Level::Max, None, Vec::new(), Vec::new(), Vec::new());
        let mut meter = CostMeter(0);
        for &b in text {
            m.code_byte(&mut meter, b);
        }
        m
    };
    let prose_live = |m: &Model| (0..m.live_n).any(|k| (TEXT + PROSE..TEXT + TEXTS).contains(&m.ids[usize::from(m.live[k])]));
    let table: Vec<u8> = (0..400).flat_map(|i| format!("{i:5} {:7.3}\n", f64::from(i) * 0.37).into_bytes()).collect();
    let t = fed(&table);
    assert!(t.text_on, "a table of numbers is text");
    assert!(!prose_live(&t), "but not prose");
    let p = fed(&wordy_text(4000, 7));
    assert!(p.text_on && prose_live(&p), "words are");
}

/// Lines of twenty random letters, each either a copy of the line above or new, one in two.
fn copied_lines(rng: &mut Mix) -> Vec<u8> {
    let fresh = |rng: &mut Mix| (0..20).map(|_| b'a' + (rng.next() % 26) as u8).collect::<Vec<u8>>();
    let mut line = fresh(rng);
    let mut out = Vec::new();
    while out.len() < 1 << 17 {
        if rng.next() % 2 != 0 {
            line = fresh(rng);
        }
        out.extend_from_slice(&line);
        out.push(b'\n');
    }
    out.truncate(1 << 17);
    out
}

#[test]
fn a_line_that_has_matched_the_one_above_goes_on_matching_it() {
    // Each line copies the one above or is new; a few letters in, the run matched so far says which.
    let per_byte = bits_per_byte(&copied_lines(&mut Mix(0x434F50)));
    assert!(per_byte < 2.305, "{per_byte:.5} bits a byte");
}

/// A tar of `files` as GNU tar writes it (checksums `%06o\0 `), and where each header starts.
fn tar_of(files: &[(String, Vec<u8>)]) -> (Vec<u8>, Vec<usize>) {
    let mut out = Vec::new();
    let mut heads = Vec::new();
    for (name, data) in files {
        heads.push(out.len());
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(b"0000644\0");
        h[108..124].copy_from_slice(b"0001750\00001750\0");
        h[124..136].copy_from_slice(format!("{:011o}\0", data.len()).as_bytes());
        h[136..148].copy_from_slice(format!("{:011o}\0", 0o7345216734 + heads.len() * 97).as_bytes());
        h[148..156].fill(b' ');
        h[156] = b'0';
        h[257..265].copy_from_slice(b"ustar  \0");
        h[265..270].copy_from_slice(b"jerry");
        h[297..302].copy_from_slice(b"samba");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        out.extend(h);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    (out, heads)
}

/// A ZIP archive of deflated `entries` with every field as a writer puts it, and where its CRC-32s, sizes and offsets
/// sit (each four bytes).
fn zip_of(entries: &[(String, Vec<u8>)]) -> (Vec<u8>, Vec<usize>) {
    let p6 = super::deflate::Params { level: 6, mem_level: 8, window_bits: 15 };
    let (mut out, mut central, mut fields) = (Vec::new(), Vec::new(), Vec::new());
    let mut central_fields = Vec::new();
    for (i, (name, raw)) in entries.iter().enumerate() {
        let body = super::deflate::deflate(raw, p6);
        let values = [streams::crc32(raw), body.len() as u32, raw.len() as u32];
        let offset = out.len() as u32;
        out.extend_from_slice(b"PK\x03\x04\x14\x00\x00\x00\x08\x00");
        out.extend_from_slice(&(0x2CBF_6E21u32 + 2 * i as u32).to_le_bytes());
        fields.extend([out.len(), out.len() + 4, out.len() + 8]);
        values.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&body);
        central.extend_from_slice(b"PK\x01\x02\x14\x03\x14\x00\x00\x00\x08\x00");
        central.extend_from_slice(&(0x2CBF_6E21u32 + 2 * i as u32).to_le_bytes());
        central_fields.extend([central.len(), central.len() + 4, central.len() + 8]);
        values.iter().for_each(|v| central.extend_from_slice(&v.to_le_bytes()));
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&0x81A4_0000u32.to_le_bytes());
        central_fields.push(central.len());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd = out.len();
    fields.extend(central_fields.iter().map(|&f| cd + f));
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&(cd as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    (out, fields)
}

/// A greyscale PNG `width` by `height`: `IHDR`, a `tEXt`, its image data in one `IDAT`, `IEND`.
fn png_file(width: usize, height: usize) -> Vec<u8> {
    let chunk = |out: &mut Vec<u8>, kind: &[u8], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let crc = streams::crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    };
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = (width as u32).to_be_bytes().to_vec();
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"tEXt", b"Comment\0logo");
    let rows: Vec<u8> = (0..height).flat_map(|y| std::iter::once(0).chain((0..width).map(move |x| ((x * 7 + y * 3) % 253) as u8))).collect();
    let mut z = zlib(&rows);
    z.extend_from_slice(&streams::adler32(&rows).to_be_bytes());
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A ZIP archive of one stored entry.
fn stored_jar(name: &str, data: &[u8]) -> Vec<u8> {
    let mut out = b"PK\x03\x04\x0A\x00\x00\x00\x00\x00\x21\x6E\xBF\x2C".to_vec();
    for v in [streams::crc32(data), data.len() as u32, data.len() as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(data);
    let cd = out.len();
    out.extend_from_slice(b"PK\x01\x02\x14\x03\x0A\x00\x00\x00\x00\x00\x21\x6E\xBF\x2C");
    for v in [streams::crc32(data), data.len() as u32, data.len() as u32] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&[0; 12]);
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(name.as_bytes());
    let size = out.len() - cd;
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0\x01\x00\x01\x00");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&(cd as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

/// A gzip member of `raw` with its CRC-32 and length.
fn gzip_member(raw: &[u8]) -> Vec<u8> {
    let mut g = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 3];
    g.extend(super::deflate::gzip(raw, 6));
    g.extend_from_slice(&streams::crc32(raw).to_le_bytes());
    g.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    g
}

fn source_files(n: usize, seed: u64) -> Vec<(String, Vec<u8>)> {
    (0..n).map(|i| (format!("samba-2.2.3a/source/lib/util_{i:02}.c"), wordy_text(300 + 41 * i, seed + i as u64))).collect()
}

/// What the record of blanked fields may cost over the same bytes with the fields zeroed, for the inputs below: it
/// costs 7, 6 and 9 bytes, where the model paying for the fields costs 49, 498 and 190.
const TAR_SLACK: usize = 12;
const JAR_SLACK: usize = 12;
const TRAILER_SLACK: usize = 14;

/// `compress(data)` against `compress(zeroed)`, where `zeroed` holds the blanks in place of the fields: the model codes
/// the two alike, so the fields cost only the record naming them.
fn fields_cost(data: &[u8], zeroed: &[u8]) -> (usize, usize) {
    let z = round_trips(data);
    (z.len(), compress(zeroed).len())
}

#[test]
fn a_tar_s_checksums_cost_the_model_nothing() {
    let (t, heads) = tar_of(&source_files(60, 300));
    let mut zeroed = t.clone();
    for &h in &heads {
        zeroed[h + 148..h + 154].fill(b'0');
    }
    let (with, base) = fields_cost(&t, &zeroed);
    assert!(with <= base + TAR_SLACK, "{with} bytes, {base} with the checksums zeroed");
}

#[test]
fn a_jar_s_crcs_sizes_and_offsets_cost_the_model_nothing() {
    let entries: Vec<(String, Vec<u8>)> = (0..40).map(|i| (format!("content/navigator/pane{i:02}.js"), wordy_text(400 + 91 * i, 400 + i as u64))).collect();
    let (jar, fields) = zip_of(&entries);
    let mut zeroed = jar.clone();
    for &f in &fields {
        zeroed[f..f + 4].fill(0);
    }
    let (with, base) = fields_cost(&jar, &zeroed);
    assert!(with <= base + JAR_SLACK, "{with} bytes, {base} with the CRCs, sizes and offsets zeroed");
}

#[test]
fn zlib_and_gzip_trailers_cost_the_model_nothing() {
    let mut data = b"%PDF-1.4\n".to_vec();
    let mut trailers = Vec::new();
    for i in 0..30 {
        data.extend_from_slice(format!("{} 0 obj\n<< /Filter /FlateDecode >>\nstream\n", i + 1).as_bytes());
        let raw = wordy_text(700 + 53 * i, 500 + i as u64);
        data.extend(zlib(&raw));
        trailers.push((data.len(), 4));
        data.extend_from_slice(&streams::adler32(&raw).to_be_bytes());
        data.extend_from_slice(b"\nendstream\nendobj\n");
    }
    for i in 0..5 {
        let member = gzip_member(&wordy_text(3000 + 400 * i, 600 + i as u64));
        trailers.push((data.len() + member.len() - 8, 8));
        data.extend(member);
    }
    let mut zeroed = data.clone();
    for &(at, n) in &trailers {
        zeroed[at..at + n].fill(0);
    }
    let (with, base) = fields_cost(&data, &zeroed);
    assert!(with <= base + TRAILER_SLACK, "{with} bytes, {base} with the trailers zeroed");
}

#[test]
fn every_level_s_fields_are_blanked_and_restored() {
    // A tar holding a jar, a gzip member of another tar holding a jar and a zlib stream, and a jar holding a PNG as it
    // is: archives at the first and second levels, an image inside an archive, trailers, and both tars' headers.
    let jar = zip_of(&[("a.js".to_string(), wordy_text(5000, 701)), ("b.css".to_string(), wordy_text(3000, 702))]).0;
    let mut pdf = b"stream\n".to_vec();
    let raw = wordy_text(4000, 703);
    pdf.extend(zlib(&raw));
    pdf.extend_from_slice(&streams::adler32(&raw).to_be_bytes());
    let inner = tar_of(&[("inner/b.jar".to_string(), jar.clone()), ("inner/doc.pdf".to_string(), pdf)]).0;
    let skin = stored_jar("skin/logo.png", &png_file(48, 40));
    let outer = tar_of(&[
        ("a.jar".to_string(), jar),
        ("inner.tar.gz".to_string(), gzip_member(&inner)),
        ("skin.jar".to_string(), skin),
        ("notes".to_string(), wordy_text(2000, 704)),
    ])
    .0;
    let z = round_trips(&outer);
    let mut at = 0;
    read_uvarint(&z, &mut at).unwrap();
    assert_eq!(z[at + 1] & (FILTER_FIELDS | FILTER_NESTED), FILTER_FIELDS | FILTER_NESTED);
    let r = represent(&outer, usize::MAX);
    assert!(r.fields.archives[0].any() && r.fields.archives[1].any(), "{:?}", r.fields.archives);
    assert!(r.fields.trailers.iter().flatten().filter(|&&c| c != 0).count() >= 2, "the gzip member's and the zlib stream's");
    assert!(r.fields.images[0].any(), "the PNG stored in a jar, restored before the jar's CRC is computed over it");
    assert_eq!(r.fields.tar.found - r.fields.tar.kept.len(), 6, "both tars' headers");
}

#[test]
fn a_fields_record_the_decoder_must_not_trust_is_refused() {
    let (t, _) = tar_of(&source_files(10, 800));
    let z = compress(&t);
    let mut at = 0;
    read_uvarint(&z, &mut at).unwrap();
    let f = at + 1;
    assert_eq!(z[f], FILTER_FIELDS, "the record alone in its block");
    let mut pos = f + 1;
    let head = read_packed(&z, &mut pos, t.len()).unwrap();
    let with = |h: &[u8]| {
        let mut out = z[..f + 1].to_vec();
        write_packed(h, &mut out);
        out.extend_from_slice(&z[pos..]);
        out
    };
    assert_eq!(with(&head), z);
    assert_eq!(head, vec![0, 0, 0, 0, 10, 0], "no archive, no image; ten tar headers, none kept");
    for (bad, what) in [
        (vec![0, 0, 0, 0, 11, 0], "a header more"),
        (vec![0, 0, 0, 0, 9, 0], "a header fewer"),
        (vec![1, 0, 0, 0, 10, 0], "an archive that is not there"),
        (vec![0, 0, 1, 0, 10, 0], "an image that is not there"),
        (vec![0, 0, 0, 0, 10, 1, 10], "a kept index past the found"),
        (vec![0, 0, 0, 0, 10], "cut short"),
        (vec![0, 0, 0, 0, 10, 0, 0], "a byte past the record"),
    ] {
        assert!(decompress(&with(&bad), t.len()).is_none(), "{what}");
    }
    let mut unflagged = z.clone();
    unflagged[f] = 0;
    assert_ne!(decompress(&unflagged, t.len()).as_deref(), Some(&t[..]), "without its flag the record is not read and the checksums stay blank");
}

/// C-like source: functions of statements (a call checked and returned on, an assignment from a call, a bare call, a
/// comment) over a fixed set of identifiers with underscores and digits, calls nested in arguments, the body indented
/// by tabs — where a statement's `;`, a call's `(` and `)` and an argument's `,` follow from the identifiers and the
/// brackets open.
fn c_source(rng: &mut Mix) -> Vec<u8> {
    const FUNCS: [&str; 12] = [
        "prs_uint32", "smb_io_unistr2", "init_samr_q_query", "talloc_zero", "DEBUG", "memset", "fstrcpy", "prs_align",
        "make_rpc_hdr", "SIVAL", "get_policy_hnd", "string_to_sid",
    ];
    const VARS: [&str; 10] = ["ps", "depth", "q_u", "r_u", "mem_ctx", "info", "num_entries", "sid", "status", "buf2"];
    fn call(rng: &mut Mix, out: &mut Vec<u8>, nest: u32) {
        out.extend_from_slice(FUNCS[(rng.next() % 12) as usize].as_bytes());
        out.push(b'(');
        for i in 0..1 + rng.next() % 3 {
            if i > 0 {
                out.extend_from_slice(b", ");
            }
            match rng.next() % 5 {
                0 if nest < 2 => call(rng, out, nest + 1),
                1 => {
                    out.push(b'&');
                    out.extend_from_slice(VARS[(rng.next() % 10) as usize].as_bytes());
                }
                2 => out.extend_from_slice((rng.next() % 64).to_string().as_bytes()),
                _ => out.extend_from_slice(VARS[(rng.next() % 10) as usize].as_bytes()),
            }
        }
        out.push(b')');
    }
    let mut out = Vec::new();
    while out.len() < 1 << 17 {
        out.extend_from_slice(b"BOOL ");
        call(rng, &mut out, 2);
        out.extend_from_slice(b"\n{\n");
        for _ in 0..3 + rng.next() % 8 {
            out.push(b'\t');
            match rng.next() % 4 {
                0 => {
                    out.extend_from_slice(b"if (!");
                    call(rng, &mut out, 0);
                    out.extend_from_slice(b")\n\t\treturn False;\n");
                }
                1 => {
                    out.extend_from_slice(VARS[(rng.next() % 10) as usize].as_bytes());
                    out.extend_from_slice(b" = ");
                    call(rng, &mut out, 0);
                    out.extend_from_slice(b";\n");
                }
                2 => {
                    call(rng, &mut out, 0);
                    out.extend_from_slice(b";\n");
                }
                _ => {
                    out.extend_from_slice(b"/* ");
                    out.extend_from_slice(VARS[(rng.next() % 10) as usize].as_bytes());
                    out.extend_from_slice(b" (see ");
                    out.extend_from_slice(FUNCS[(rng.next() % 12) as usize].as_bytes());
                    out.extend_from_slice(b") */\n");
                }
            }
        }
        out.extend_from_slice(b"\treturn True;\n}\n\n");
    }
    out.truncate(1 << 17);
    out
}

#[test]
fn a_c_function_s_punctuation_follows_from_its_identifiers_and_brackets() {
    let per_byte = bits_per_byte(&c_source(&mut Mix(0x434F_4445)));
    assert!(per_byte < 0.5804, "{per_byte:.5} bits a byte");
}

/// Chapters of sentences: common words, and six names each chapter brings, every name beginning with one of eight
/// two-letter prefixes all chapters share — so a name's first letters say which word follows only by which of them
/// came last.
fn chapter_names(rng: &mut Mix) -> Vec<u8> {
    const COMMON: [&str; 16] = [
        "the", "and", "said", "to", "of", "was", "her", "his", "at", "with", "then", "came", "saw", "left", "told", "met",
    ];
    const PREFIXES: [&str; 8] = ["ka", "mo", "be", "si", "lu", "ra", "te", "do"];
    let mut out = Vec::new();
    while out.len() < 1 << 17 {
        let names: Vec<String> = (0..6)
            .map(|_| {
                let tail: String = (0..6 + rng.next() % 5).map(|_| (b'a' + (rng.next() % 26) as u8) as char).collect();
                format!("{}{tail}", PREFIXES[(rng.next() % 8) as usize])
            })
            .collect();
        for _ in 0..15 {
            for w in 0..6 + rng.next() % 6 {
                if w > 0 {
                    out.push(b' ');
                }
                if rng.next() % 2 == 0 {
                    out.extend_from_slice(names[(rng.next() % 6) as usize].as_bytes());
                } else {
                    out.extend_from_slice(COMMON[(rng.next() % 16) as usize].as_bytes());
                }
            }
            out.extend_from_slice(b".\n");
        }
    }
    out.truncate(1 << 17);
    out
}

#[test]
fn a_repeated_name_is_predicted_from_its_first_letters() {
    let per_byte = bits_per_byte(&chapter_names(&mut Mix(0x434F_4D50)));
    assert!(per_byte < 0.9720, "{per_byte:.5} bits a byte");
}

#[test]
fn at_max_the_lstms_prediction_is_a_mixer_input() {
    let text = wordy_text(20_000, 33);
    let mut model = Model::coded(text.len(), Level::Max, None, Vec::new(), Vec::new(), Vec::new());
    assert!(model.lstm.is_some(), "Max has the network");
    let mut meter = CostMeter(0);
    for &b in &text {
        model.code_byte(&mut meter, b);
    }
    assert_ne!(model.mixer.input(model.lstm_input), 0, "its prediction reaches the mixer");
    let fast = Model::coded(1000, Level::Fast, None, Vec::new(), Vec::new(), Vec::new());
    assert!(fast.lstm.is_none(), "Fast has none");
}

impl Model {
    /// A model without DEC Alpha blocks.
    fn new(len: usize, level: Level, image: Option<Image>, exe: Vec<bool>, regions: Vec<Region>) -> Self {
        Self::coded(len, level, image, exe, Vec::new(), regions)
    }
}

/// Code `body` at `level` with a model for an image or x86 code blocks as given: the arithmetic-coded bits alone.
fn encode_body(body: &[u8], level: Level, image: Option<Image>, exe: Vec<bool>, regions: Vec<Region>) -> Vec<u8> {
    encode_coded(body, level, image, exe, Vec::new(), regions)
}

/// Lines of one template whose counter field counts up by one a line, one byte in 512 (not the newline) a stray
/// capital — a near-certain source like a chemical table's records.
fn counter_lines(rng: &mut Mix) -> Vec<u8> {
    let mut out = Vec::new();
    let mut k = 100_000u32;
    while out.len() < 1 << 17 {
        let mut line = format!("  {k}  0  0  0  0  0  0  0  0  0 V2000\n").into_bytes();
        let last = line.len() - 1;
        for b in &mut line[..last] {
            if rng.next() % 512 == 0 {
                *b = b'A' + (rng.next() % 26) as u8;
            }
        }
        out.extend_from_slice(&line);
        k += 1;
    }
    out.truncate(1 << 17);
    out
}

/// The bytes a steady source costs through `apm`: the input always says 0.5, the bit is 1 with p = 4055/4096.
fn steady_through(mut apm: Apm, seed: u64) -> usize {
    let mut rng = Mix(seed);
    let mut enc = Encoder::new();
    for _ in 0..200_000 {
        let bit = rng.next() % 4096 < 4055;
        enc.encode(bit, apm.refine(2048, 0));
        apm.update(bit);
    }
    enc.finish().len()
}

#[test]
fn the_model_s_probability_maps_average_where_a_fixed_rate_map_jitters() {
    // The model's own maps count far enough that a steady bit costs less than at a fixed rate: a cap of a handful of
    // updates would keep them jittering more than the fixed rate does.
    let (fixed, counted) = (steady_through(Apm::new(1), 7), steady_through(counting_apm(1), 7));
    assert!(counted * 100 < fixed * 99, "the model's maps {counted} B vs fixed-rate {fixed} B");
}

#[test]
fn near_certain_lines_are_refined_by_apms_that_count_their_updates() {
    // Most bits here are near certain: the final maps' curves sit at the extremes, where a fixed step's jitter costs
    // the most.
    let per_byte = bits_per_byte(&counter_lines(&mut Mix(0x4C49_4E45)));
    assert!(per_byte < 0.0415, "{per_byte:.5} bits a byte");
}

#[test]
fn the_record_selectors_place_the_column_the_byte_above_and_the_word_lane() {
    let mut model = Model::new(1 << 16, Level::Max, None, Vec::new(), Vec::new());
    model.history = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
    // A 160-byte record: byte 1000 sits at column 40, place 32 of 128 and 8 of 32; one record up is byte 840 = 0x6B.
    model.record_len = 160;
    model.c0 = 0b1_0110;
    assert_eq!(model.record_selectors(4, 0x41), [32 * 8 + 4, 8 << 4, 0x41], "0110 agrees with 0x6B's top nibble");
    model.c0 = 0b1_1110;
    assert_eq!(model.record_selectors(4, 0x41), [32 * 8 + 4, 0x8 | 8 << 4, 0x41], "1110 differs from it in the top bit");
    model.c0 = 1;
    assert_eq!(model.record_selectors(0, 0x41), [32 * 8, 0x6 | 8 << 4, 0x41], "before any bit, the byte above's own top nibble");
    // A 48-byte record, shorter than the 128 places: column 40 is place 40, and 26 of 32; one record up is 952 = 0x8A.
    model.record_len = 48;
    model.c0 = 0b1_10;
    assert_eq!(model.record_selectors(2, 0x41), [40 * 8 + 2, 0x0 | 26 << 4, 0x41]);
    // No record: column 0, nothing above; the lane is the position in a 4-byte word.
    model.record_len = 0;
    model.history.push(0);
    assert_eq!(model.record_selectors(2, 0x7F), [2, 0x8, 256 + 0x7F]);
    model.history.push(0);
    assert_eq!(model.record_selectors(2, 0x7F)[2], 2 * 256 + 0x7F, "byte 1002 is the third of its word");
}

/// Records of 160 bytes: below column 80 a byte copies the one a record up, from there on it repeats the byte before;
/// one byte in eight is fresh. Past column 63 the two halves want opposite weights.
fn role_records(rng: &mut Mix) -> Vec<u8> {
    const R: usize = 160;
    let mut data: Vec<u8> = (0..R).map(|_| rng.next() as u8).collect();
    while data.len() < 1 << 17 {
        let k = data.len();
        let next = if rng.next() % 8 == 0 {
            rng.next() as u8
        } else if k % R < R / 2 {
            data[k - R]
        } else {
            data[k - 1]
        };
        data.push(next);
    }
    data
}

#[test]
fn a_long_record_s_columns_choose_their_own_weight_sets() {
    let per_byte = bits_per_byte(&role_records(&mut Mix(0x524F_4C45)));
    assert!(per_byte < 1.6419, "{per_byte:.5} bits a byte");
}

/// An order-2 source near certainty: each pair of bytes is followed by one byte 63 times in 64, else by a fresh one.
fn skewed_pairs(rng: &mut Mix) -> Vec<u8> {
    let table: Vec<u8> = (0..1 << 16).map(|_| rng.next() as u8).collect();
    let mut data = vec![1u8, 2];
    while data.len() < 1 << 17 {
        let k = usize::from(data[data.len() - 2]) << 8 | usize::from(data[data.len() - 1]);
        let next = if rng.next() % 64 != 0 { table[k] } else { rng.next() as u8 };
        data.push(next);
    }
    data
}

#[test]
fn the_first_mixer_s_weight_sets_settle_on_a_steady_source() {
    // The same few weight sets are chosen over and over: once they slow down, their jitter stops costing bits.
    let per_byte = bits_per_byte(&skewed_pairs(&mut Mix(0x534B_4557)));
    assert!(per_byte < 1.1412, "{per_byte:.5} bits a byte");
}

/// Elements of twelve names nested up to six deep, each closed in order, words between them: a closing tag is spelled
/// by the element it closes, however far back that opened.
fn nested_markup(rng: &mut Mix) -> Vec<u8> {
    const TAGS: [&str; 12] = ["div", "span", "p", "table", "tr", "td", "ul", "li", "em", "b", "a", "code"];
    const WORDS: [&str; 16] = [
        "the", "server", "share", "user", "file", "print", "name", "list", "value", "path", "mode", "host", "time", "set",
        "read", "write",
    ];
    let mut out = Vec::new();
    let mut open: Vec<&str> = Vec::new();
    while out.len() < 1 << 17 {
        let r = rng.next() % 8;
        if open.is_empty() || (r < 3 && open.len() < 6) {
            let tag = TAGS[(rng.next() % 12) as usize];
            out.extend_from_slice(format!("<{tag}>").as_bytes());
            open.push(tag);
        } else if r < 5 {
            let tag = open.pop().expect("an open element");
            out.extend_from_slice(format!("</{tag}>").as_bytes());
        } else {
            out.extend_from_slice(WORDS[(rng.next() % 16) as usize].as_bytes());
            out.push(b' ');
        }
    }
    out.truncate(1 << 17);
    out
}

#[test]
fn a_closing_tag_is_spelled_by_the_element_it_closes() {
    let per_byte = bits_per_byte(&nested_markup(&mut Mix(0x4E45_5354)));
    assert!(per_byte < 0.88, "{per_byte:.5} bits a byte");
}

fn lead_of(line: &[u8]) -> Lead {
    let mut lead = Lead::default();
    for &b in line {
        lead.push(b);
    }
    lead
}

#[test]
fn a_line_s_leading_number_is_read_after_its_spaces() {
    assert_eq!(lead_of(b"  155001 C 0").value(), Some(155_001));
    assert_eq!(lead_of(b"155001").value(), Some(155_001));
    assert_eq!(lead_of(b"  M  END").value(), None, "a line that leads with a letter has no leading number");
    assert_eq!(lead_of(b"12 34").value(), Some(12), "only the first digit run");
    assert!(lead_of(b"x").done);
}

#[test]
fn an_expected_leading_number_keys_its_next_digit_and_whether_the_line_agrees() {
    let expected = b"155002";
    let key = |line: &[u8]| lead_of(line).expect(expected);
    assert_eq!(key(b"  "), ((u64::from(b'1') << 1) << 8 | 1) << 8 | 2, "the first digit, nothing read, two spaces");
    assert_eq!(key(b"1550"), ((u64::from(b'0') << 1) << 8 | 4 << 1 | 1) << 8);
    assert_eq!(key(b"1551"), ((u64::from(b'0') << 1) << 8 | 4 << 1) << 8, "the line has left the expectation");
    assert_eq!(key(b"155002"), (1 << 8 | 6 << 1 | 1) << 8, "past the expected digits");
    assert_eq!(key(b"155002 "), 0, "the leading number is over");
    assert_eq!(lead_of(b"  ").expect(b""), 0, "no expectation");
}

/// Records that each open with an ID one above the last (one in sixteen skips one); the rest of every record is the
/// same two lines.
fn numbered_records(rng: &mut Mix) -> Vec<u8> {
    let mut out = Vec::new();
    let mut id = 155_000u64;
    while out.len() < 1 << 17 {
        out.extend_from_slice(format!("{id}\n  M  END\n$$$$\n").as_bytes());
        id += if rng.next() % 16 == 0 { 2 } else { 1 };
    }
    out.truncate(1 << 17);
    out
}

#[test]
fn a_record_s_id_is_expected_one_above_the_last() {
    let per_byte = bits_per_byte(&numbered_records(&mut Mix(0x4944_5245)));
    assert!(per_byte < 0.029, "{per_byte:.5} bits a byte");
}

#[test]
fn a_table_whose_entries_hold_e8_bytes_is_not_x86_code() {
    // Every 16-byte entry carries `E8 FF FF FF 00`: a near-looking call whose operand repeats as written, so its
    // absolute targets would all differ — the data sections of compiled code look like this.
    let mut rng = Mix(41);
    let mut table = Vec::new();
    while table.len() < 200_000 {
        table.extend_from_slice(&(rng.next() as u32 & 0xFFFF).to_le_bytes());
        table.extend_from_slice(&[0xE8, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00]);
        table.extend_from_slice(&(rng.next() as u32).to_le_bytes());
    }
    assert!(branch_blocks(&table).iter().all(|&b| !b), "{:?}", branch_blocks(&table));
    assert!(branch_blocks(&x86_like(200_000, 31)).iter().filter(|&&b| b).count() >= 2, "code calling 64 functions still is");
    round_trips(&table);
}
