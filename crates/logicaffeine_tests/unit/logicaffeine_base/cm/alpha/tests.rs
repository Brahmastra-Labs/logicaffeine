use super::*;

/// SplitMix64.
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

/// Plausible Alpha code: loads, stores, address arithmetic, operates, branches and calls, as little-endian words.
fn alpha_like(words: usize, seed: u64) -> Vec<u8> {
    let mut rng = Mix(seed);
    let mut out = Vec::with_capacity(words * 4);
    for _ in 0..words {
        let r = rng.next();
        let reg = |shift: u32| ((r >> shift) & 31) as u32;
        let w = match r % 10 {
            0 | 1 => 0x29 << 26 | reg(8) << 21 | 30 << 16 | ((r >> 20) as u32 & 0x3F8),
            2 => 0x2D << 26 | reg(8) << 21 | 30 << 16 | ((r >> 20) as u32 & 0x3F8),
            3 => 0x08 << 26 | reg(8) << 21 | reg(13) << 16 | ((r >> 20) as u32 & 0xFFFF),
            4 => 0x09 << 26 | 29 << 21 | 27 << 16 | ((r >> 20) as u32 & 0xFF),
            5 | 6 => 0x11 << 26 | reg(8) << 21 | reg(13) << 16 | 0x20 << 5 | reg(18),
            7 => 0x10 << 26 | reg(8) << 21 | reg(13) << 16 | 0x20 << 5 | reg(18),
            8 => 0x34 << 26 | 26 << 21 | ((r >> 24) as u32 & 0x1F_FFFF),
            _ => 0x39 << 26 | reg(8) << 21 | ((r >> 24) as u32 & 0xFF),
        };
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

#[test]
fn alpha_code_is_found_at_its_word_phase_and_text_and_noise_are_not() {
    let code = alpha_like(40_000, 1);
    assert_eq!(blocks(&code), vec![1; 3], "aligned code: every block on at phase 0");
    assert_eq!(spans(&code), vec![Span { start: 0, words: 40_000 }], "consecutive blocks of one phase are one span");
    for lead in 1..4 {
        let mut shifted = vec![0x55; lead];
        shifted.extend_from_slice(&code);
        assert_eq!(blocks(&shifted)[1], lead as u8 + 1, "{lead} leading bytes: the words start {lead} bytes into a block");
        assert_eq!(spans(&shifted), vec![Span { start: lead, words: 40_000 }]);
    }
    let text = b"the codec ships a column of values and every decoder reads it back exactly ".repeat(2000);
    assert!(blocks(&text).iter().all(|&p| p == 0) && spans(&text).is_empty(), "text is not Alpha code");
    let mut rng = Mix(2);
    let noise: Vec<u8> = (0..200_000).map(|_| rng.next() as u8).collect();
    assert!(blocks(&noise).iter().all(|&p| p == 0) && spans(&noise).is_empty(), "noise is not Alpha code");
}

/// A minimal Alpha ECOFF object: the file header, an 80-byte optional header and `sections` (name, file offset, size),
/// then `body` laid at the end.
fn ecoff(sections: &[(&[u8], usize, usize)], body: &[(usize, &[u8])], len: usize) -> Vec<u8> {
    let mut o = vec![0u8; len];
    o[..2].copy_from_slice(&ECOFF_MAGIC);
    o[2..4].copy_from_slice(&(sections.len() as u16).to_le_bytes());
    o[20..22].copy_from_slice(&80u16.to_le_bytes());
    for (k, &(name, offset, size)) in sections.iter().enumerate() {
        let h = ECOFF_HEADER + 80 + 64 * k;
        o[h..h + name.len()].copy_from_slice(name);
        o[h + 24..h + 32].copy_from_slice(&(size as u64).to_le_bytes());
        o[h + 32..h + 40].copy_from_slice(&(offset as u64).to_le_bytes());
    }
    for &(at, bytes) in body {
        o[at..at + bytes.len()].copy_from_slice(bytes);
    }
    o
}

#[test]
fn an_ecoff_objects_code_sections_are_taken_exactly_however_short() {
    // .text 6000 bytes at 1024 and .init right after it; .data between real text and the tail is not code.
    let code = alpha_like(1500 + 16, 4);
    let data = vec![0x07u8; 3000];
    let obj = ecoff(
        &[(b".text", 1024, 6000), (b".init", 7024, 64), (b".data", 7088, 3000)],
        &[(1024, &code), (7088, &data)],
        10_088,
    );
    assert_eq!(spans(&obj), vec![Span { start: 1024, words: 1516 }], ".text and .init meet: one span, exactly");
    assert!(blocks(&obj).iter().all(|&p| p == 0), "too short for the block test, found by the section table");
    // Inside a larger stream (a tar member at offset 512), with Alpha code elsewhere that only the block test finds.
    let mut stream = vec![0u8; 512];
    stream.extend_from_slice(&obj);
    stream.resize(1 << 17, 0);
    stream.extend_from_slice(&alpha_like(20_000, 5));
    let found = spans(&stream);
    assert_eq!(found[0], Span { start: 512 + 1024, words: 1516 });
    assert_eq!(found[1..], [Span { start: 1 << 17, words: 20_000 }], "the headerless code after it, by blocks");
}

#[test]
fn a_section_table_that_points_past_the_data_or_at_noise_is_ignored() {
    let code = alpha_like(1000, 6);
    let past = ecoff(&[(b".text", 1024, 1 << 20)], &[(1024, &code)], 5024);
    assert!(spans(&past).is_empty(), "a section past the end");
    let mut rng = Mix(7);
    let noise: Vec<u8> = (0..4000).map(|_| rng.next() as u8).collect();
    let fake = ecoff(&[(b".text", 1024, 4000)], &[(1024, &noise)], 5024);
    assert!(spans(&fake).is_empty(), "a code section that is not Alpha code");
    let odd = ecoff(&[(b".text", 1026, 4000)], &[(1024, &code)], 5024);
    assert!(spans(&odd).is_empty(), "a section off word alignment");
    let many = ecoff(&[(b".text", 1024, 4000)], &[(1024, &code)], 5024);
    let mut broken = many.clone();
    broken[2..4].copy_from_slice(&900u16.to_le_bytes());
    assert!(spans(&broken).is_empty(), "an implausible section count");
}

#[test]
fn the_filter_makes_calls_absolute_and_words_big_endian_and_undoes_itself_on_any_bytes() {
    // bsr r26 at word 5 (byte 20) with displacement 0x10: its target word is 5 + 0x10.
    let mut code = vec![0u8; 64];
    code[20..24].copy_from_slice(&(0x34u32 << 26 | 26 << 21 | 0x10).to_le_bytes());
    code[24..28].copy_from_slice(&0x2D5E_0010u32.to_le_bytes());
    let spans = [Span { start: 0, words: 16 }];
    let mut body = code.clone();
    filter(&mut body, &spans, true);
    assert_eq!(&body[20..24], &(0x34u32 << 26 | 26 << 21 | (0x10 + 5)).to_be_bytes(), "the call's target is absolute");
    assert_eq!(&body[24..28], &0x2D5E_0010u32.to_be_bytes(), "every word is stored most significant byte first");
    filter(&mut body, &spans, false);
    assert_eq!(body, code);
    let mut rng = Mix(3);
    for len in [0usize, 1, 3, 4, 7, 65_536, 65_539, 200_001] {
        let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let mut spans = Vec::new();
        let mut at = 0;
        while len > 8 && at + 8 < len {
            let start = at + (rng.next() % 7) as usize;
            let words = (1 + rng.next() % 9000) as usize;
            let words = words.min((len - start) / 4);
            if words == 0 {
                break;
            }
            spans.push(Span { start, words });
            at = start + 4 * words;
        }
        let mut body = data.clone();
        filter(&mut body, &spans, true);
        filter(&mut body, &spans, false);
        assert_eq!(body, data, "{len} bytes, {} spans", spans.len());
    }
}

#[test]
fn spans_round_trip_through_the_header_and_bad_ones_are_refused() {
    let spans = vec![Span { start: 333, words: 1000 }, Span { start: 9000, words: 5 }];
    let mut out = Vec::new();
    write(&spans, &mut out);
    let mut pos = 0;
    assert_eq!(read(&out, &mut pos, 10_000), Some(spans.clone()));
    assert_eq!(pos, out.len());
    let mut pos = 0;
    assert_eq!(read(&out, &mut pos, 9_010), None, "a span past the body");
    for bad in [vec![Span { start: 0, words: 0 }], Vec::new()] {
        let mut out = Vec::new();
        write(&bad, &mut out);
        let mut pos = 0;
        assert_eq!(read(&out, &mut pos, 10_000), None, "{bad:?}");
    }
}

#[test]
fn the_word_tracker_follows_positions_and_words() {
    let code = alpha_like(64, 4);
    let mut body = code.clone();
    filter(&mut body, &[Span { start: 0, words: 64 }], true);
    let mut a = Words::new();
    for (i, &b) in body.iter().enumerate() {
        assert_eq!(a.position(), i % 4, "byte {i}");
        a.push(b);
        if i % 4 == 3 {
            assert_eq!(a.previous(0), u32::from_be_bytes(body[i - 3..=i].try_into().unwrap()));
            if i >= 7 {
                assert_eq!(a.previous(1), u32::from_be_bytes(body[i - 7..=i - 4].try_into().unwrap()));
            }
        }
    }
    a.restart();
    assert_eq!((a.position(), a.current()), (0, 0));
}

#[test]
fn arranging_a_word_moves_its_fields_keeps_its_opcode_and_natural_undoes_it() {
    let mut rng = Mix(11);
    for op in 0..64u32 {
        for _ in 0..20_000 {
            let w = op << 26 | (rng.next() as u32 & 0x03FF_FFFF);
            let c = arrange(w);
            assert_eq!(c >> 26, op, "{w:#010x}: the opcode never moves");
            assert_eq!(natural(c), w, "{w:#010x} -> {c:#010x}");
        }
    }
    // addq r1, #7, r3: Ra 1, literal 7, flag, function 0x20, Rc 3 -> flag+function, Rc, Ra, literal.
    let addq = 0x10u32 << 26 | 1 << 21 | 7 << 13 | 1 << 12 | 0x20 << 5 | 3;
    assert_eq!(arrange(addq), 0x10 << 26 | (0x80 | 0x20) << 18 | 3 << 13 | 1 << 8 | 7);
    // A register-form operate keeps its zero bits in the 8-bit field after Ra.
    let bis = 0x11u32 << 26 | 31 << 21 | 4 << 16 | 0x20 << 5 | 9;
    assert_eq!(arrange(bis), 0x11 << 26 | 0x20 << 18 | 9 << 13 | 31 << 8 | 4 << 3);
    // cpys f1, f2, f3: the 11-bit function first.
    let cpys = 0x17u32 << 26 | 1 << 21 | 2 << 16 | 0x020 << 5 | 3;
    assert_eq!(arrange(cpys), 0x17 << 26 | 0x020 << 15 | (1 << 5 | 2) << 5 | 3);
    // rpcc r0: the 16-bit function first.
    let rpcc = 0x18u32 << 26 | 0 << 21 | 31 << 16 | 0xC000;
    assert_eq!(arrange(rpcc), 0x18 << 26 | 0xC000 << 10 | 31);
    // jsr r26, (r27), hint 0x123: the kind first.
    let jsr = 0x1Au32 << 26 | 26 << 21 | 27 << 16 | 1 << 14 | 0x123;
    assert_eq!(arrange(jsr), 0x1A << 26 | 1 << 24 | (26 << 5 | 27) << 14 | 0x123);
    for w in [0x29u32 << 26 | 1 << 21 | 30 << 16 | 0x18, 0x34 << 26 | 26 << 21 | 0x1234, 0x0B << 26 | 31 << 21 | 30 << 16] {
        assert_eq!(arrange(w), w, "{w:#010x}: memory and branch words are not rearranged");
    }
}

fn word(op: u32, ra: u32, rb: u32, low: u32) -> u32 {
    op << 26 | ra << 21 | rb << 16 | low
}

/// The words of `body` (a filtered span from byte 0), back to their natural field order.
fn naturals(body: &[u8]) -> Vec<u32> {
    body.chunks_exact(4).map(|b| natural(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))).collect()
}

fn le(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

#[test]
fn a_global_pointer_pair_becomes_the_absolute_pointer_however_far_apart_its_halves_are_scheduled() {
    // gp = 0x4060_8000 computed from the address of each `ldah gp, hi(pv|ra)`; its `lda gp, lo(gp)` 1 to 3 words later.
    let gp: u32 = 0x4060_8000;
    let mut code = vec![word(0x11, 31, 31, 0x20 << 5 | 31); 64];
    let mut pairs = Vec::new();
    for (k, gap, base) in [(3usize, 1usize, 27u32), (20, 2, 26), (40, 3, 26)] {
        let x = gp.wrapping_sub(4 * k as u32);
        let lo = x & 0xFFFF;
        let hi = x.wrapping_sub(lo as u16 as i16 as i32 as u32) >> 16;
        code[k] = word(0x09, 29, base, hi);
        code[k + gap] = word(0x08, 29, 29, lo);
        pairs.push((k, k + gap));
    }
    let data = le(&code);
    let spans = [Span { start: 0, words: 64 }];
    let mut body = data.clone();
    filter(&mut body, &spans, true);
    let w = naturals(&body);
    for &(k, j) in &pairs {
        assert_eq!((w[k] & 0xFFFF, w[j] & 0xFFFF), (gp >> 16, gp & 0xFFFF), "pair at {k}, {j}: the pointer itself");
        assert_eq!((w[k] >> 16, w[j] >> 16), (code[k] >> 16, code[j] >> 16), "opcode and registers kept");
    }
    filter(&mut body, &spans, false);
    assert_eq!(body, data);
}

#[test]
fn a_global_pointer_half_without_its_partner_in_reach_or_in_its_span_is_left_alone() {
    let filler = word(0x11, 31, 31, 0x20 << 5 | 31);
    let mut code = vec![filler; 64];
    code[2] = word(0x09, 29, 27, 0x1234);
    code[2 + 9] = word(0x08, 29, 29, 0x5678); // past the reach
    code[20] = word(0x09, 29, 26, 0x1111);
    code[21] = word(0x09, 29, 26, 0x2222); // another `ldah gp` first: the first has no partner
    code[22] = word(0x08, 29, 29, 0x3333);
    code[40] = word(0x09, 29, 5, 0x4444); // based on neither pv nor ra
    code[41] = word(0x08, 29, 29, 0x5555);
    code[62] = word(0x09, 29, 27, 0x6666); // its partner is outside the span
    code[63] = word(0x08, 29, 29, 0x7777);
    let data = le(&code);
    let spans = [Span { start: 0, words: 63 }];
    let mut body = data.clone();
    filter(&mut body, &spans, true);
    let w = naturals(&body[..4 * 63]);
    for k in [2, 11, 20, 40, 41, 62] {
        assert_eq!(w[k], code[k], "word {k} unchanged but for its byte order");
    }
    assert_ne!((w[21], w[22]), (code[21], code[22]), "the second `ldah gp` pairs with the `lda` after it");
    assert_eq!(&body[4 * 63..], &data[4 * 63..], "outside the span nothing is touched");
    filter(&mut body, &spans, false);
    assert_eq!(body, data);
}

#[test]
fn a_calls_hint_becomes_its_target_and_other_jumps_keep_theirs() {
    let mut code = vec![word(0x11, 31, 31, 0x20 << 5 | 31); 32];
    code[7] = word(0x1A, 26, 27, 1 << 14 | 0x3FFE); // jsr: hint 7 + 0x3FFE wraps in 14 bits
    code[9] = word(0x1A, 31, 3, 0x0010); // jmp: a switch through a table
    code[12] = word(0x1A, 31, 26, 2 << 14 | 1); // ret
    code[14] = word(0x1A, 26, 3, 3 << 14 | 0x0020); // jsr_coroutine
    let data = le(&code);
    let spans = [Span { start: 0, words: 32 }];
    let mut body = data.clone();
    filter(&mut body, &spans, true);
    let w = naturals(&body);
    assert_eq!(w[7], word(0x1A, 26, 27, 1 << 14 | (0x3FFE + 7) & 0x3FFF));
    for k in [9, 12, 14] {
        assert_eq!(w[k], code[k], "word {k}: only a call's hint is made absolute");
    }
    filter(&mut body, &spans, false);
    assert_eq!(body, data);
}

#[test]
fn rewritten_alpha_code_round_trips_in_spans_at_any_phase() {
    let mut rng = Mix(13);
    let mut code = alpha_like(30_000, 12);
    // Sprinkle global pointer pairs, jumps and operates of every kind through it.
    for _ in 0..3000 {
        let k = (rng.next() % 29_990) as usize;
        let r = rng.next() as u32;
        let ws: &[u32] = match r % 4 {
            0 => &[word(0x09, 29, 26 + (r >> 8 & 1), r >> 9 & 0xFFFF), word(0x08, 29, 29, r >> 7 & 0xFFFF)],
            1 => &[word(0x1A, r >> 3 & 31, r >> 8 & 31, r >> 13 & 0xFFFF)],
            2 => &[(0x10 + (r & 3)) << 26 | (r >> 6 & 0x03FF_FFFF)],
            _ => &[(0x14 + (r & 3)) << 26 | (r >> 6 & 0x03FF_FFFF), 0x18 << 26 | (r >> 5 & 0x03FF_FFFF)],
        };
        for (i, w) in ws.iter().enumerate() {
            code[4 * (k + i)..4 * (k + i) + 4].copy_from_slice(&w.to_le_bytes());
        }
    }
    for lead in 0..4 {
        let mut data = vec![0x5Au8; lead];
        data.extend_from_slice(&code);
        let spans = [Span { start: lead, words: 10_000 }, Span { start: lead + 40_004, words: 19_999 }];
        let mut body = data.clone();
        filter(&mut body, &spans, true);
        assert_ne!(body, data);
        filter(&mut body, &spans, false);
        assert_eq!(body, data, "{lead} lead bytes");
    }
}

#[test]
fn every_opcode_has_a_class_and_the_classes_split_by_format() {
    assert!((0..64).all(|op| class(op) < 16));
    let classes: std::collections::BTreeSet<usize> = (0..64).map(class).collect();
    assert_eq!(classes.len(), 16, "all sixteen classes are used");
    for (ops, c) in [(&[0x08][..], 1), (&[0x09], 2), (&[0x10], 4), (&[0x11], 5), (&[0x12], 6), (&[0x13, 0x1C], 7), (&[0x1A], 10)] {
        assert!(ops.iter().all(|&op| class(op) == c), "{ops:x?}");
    }
    assert!((0x14..=0x17).all(|op| class(op) == 8) && (0x20..=0x27).all(|op| class(op) == 11));
    assert!((0x28..=0x2B).all(|op| class(op) == 12) && (0x2C..=0x2F).all(|op| class(op) == 13));
    assert!((0x30..=0x37).all(|op| class(op) == 14) && (0x38..=0x3F).all(|op| class(op) == 15));
}

#[test]
fn a_words_destination_is_the_register_it_writes_and_none_for_stores_branches_and_r31() {
    assert_eq!(destination(word(0x29, 1, 30, 8)), Some(1), "ldq r1, 8(sp)");
    assert_eq!(destination(word(0x08, 16, 30, 0x10)), Some(16), "lda a0, 16(sp)");
    assert_eq!(destination(word(0x09, 29, 27, 1)), Some(29), "ldah gp, 1(pv)");
    assert_eq!(destination(word(0x0B, 31, 30, 0)), None, "unop: a load into r31");
    assert_eq!(destination(word(0x23, 2, 30, 0)), Some(34), "ldt f2: the floating registers from 32");
    assert_eq!(destination(0x10 << 26 | 1 << 21 | 2 << 16 | 0x20 << 5 | 3), Some(3), "addq: Rc");
    assert_eq!(destination(0x16 << 26 | 1 << 21 | 2 << 16 | 0x0A0 << 5 | 4), Some(36), "addt f1, f2, f4");
    assert_eq!(destination(word(0x1A, 26, 27, 1 << 14)), Some(26), "jsr: its return address");
    assert_eq!(destination(0x34 << 26 | 26 << 21 | 5), Some(26), "bsr");
    assert_eq!(destination(word(0x2D, 9, 30, 8)), None, "stq");
    assert_eq!(destination(0x39 << 26 | 1 << 21 | 5), None, "beq");
    assert_eq!(destination(0x11 << 26 | 31 << 21 | 31 << 16 | 0x20 << 5 | 31), None, "nop: bis into r31");
}

#[test]
fn a_partial_words_source_is_known_once_its_bytes_hold_it() {
    // The word's first `at` bytes, as the model sees them (arranged, most significant first).
    let partial = |w: u32, at: usize| arrange(w) >> (32 - 8 * at);
    let ldq = word(0x29, 1, 9, 8);
    assert_eq!((source(partial(ldq, 1), 1), source(partial(ldq, 2), 2), source(partial(ldq, 3), 3)), (None, Some(9), Some(9)), "a load's base");
    let addq = 0x10 << 26 | 5 << 21 | 6 << 16 | 0x20 << 5 | 7;
    assert_eq!((source(partial(addq, 2), 2), source(partial(addq, 3), 3)), (None, Some(5)), "an operate's Ra, after its Rc");
    let bne = 0x3D << 26 | 4 << 21 | 0x40;
    assert_eq!(source(partial(bne, 2), 2), Some(4), "a branch's tested register");
    let fbeq = 0x31 << 26 | 3 << 21 | 0x40;
    assert_eq!(source(partial(fbeq, 2), 2), Some(35), "a floating branch's");
    let jsr = word(0x1A, 26, 27, 1 << 14 | 0x55);
    assert_eq!((source(partial(jsr, 2), 2), source(partial(jsr, 3), 3)), (None, Some(27)), "a jump's target register");
    let addt = 0x16 << 26 | 1 << 21 | 2 << 16 | 0x0A0 << 5 | 4;
    assert_eq!(source(partial(addt, 3), 3), Some(33), "a floating operate's Fa");
    assert_eq!(source(partial(0x34 << 26 | 26 << 21 | 5, 3), 3), None, "bsr reads no register");
    assert_eq!(source(0, 0), None);
}

#[test]
fn a_words_reads_are_the_registers_it_takes_and_never_r31() {
    assert_eq!(reads(word(0x29, 1, 30, 8)), [None, Some(30)], "ldq r1, 8(sp): the base");
    assert_eq!(reads(word(0x2D, 9, 30, 8)), [Some(9), Some(30)], "stq r9, 8(sp): the value and the base");
    assert_eq!(reads(word(0x27, 2, 30, 0)), [Some(34), Some(30)], "stt f2, 0(sp): a floating value");
    assert_eq!(reads(word(0x08, 16, 30, 0x10)), [None, Some(30)], "lda a0, 16(sp)");
    assert_eq!(reads(0x10 << 26 | 1 << 21 | 2 << 16 | 0x20 << 5 | 3), [Some(1), Some(2)], "addq r1, r2, r3");
    assert_eq!(reads(0x10 << 26 | 1 << 21 | 7 << 13 | 1 << 12 | 0x20 << 5 | 3), [Some(1), None], "addq r1, #7, r3: a literal is no register");
    assert_eq!(reads(0x16 << 26 | 1 << 21 | 2 << 16 | 0x0A0 << 5 | 4), [Some(33), Some(34)], "addt f1, f2, f4");
    assert_eq!(reads(word(0x1A, 26, 27, 1 << 14)), [None, Some(27)], "jsr: its target");
    assert_eq!(reads(0x3D << 26 | 4 << 21 | 5), [Some(4), None], "bne r4");
    assert_eq!(reads(0x31 << 26 | 3 << 21 | 5), [Some(35), None], "fbeq f3");
    assert_eq!(reads(0x11 << 26 | 31 << 21 | 31 << 16 | 0x20 << 5 | 31), [None, None], "nop: bis r31, r31");
    assert_eq!(reads(0x34 << 26 | 26 << 21 | 5), [None, None], "bsr reads nothing");
    assert_eq!(reads(0x18 << 26 | 0xC000), [None, None], "rpcc: the miscellaneous group reads none here");
}

#[test]
fn data_that_only_looks_like_opcodes_is_not_alpha_code() {
    // Image rows (runs of one palette index, decoded from a GIF) and a text of capitals and spaces: nearly every word's
    // top byte is a common opcode, but no register-form operate keeps its zero bits, and no load or store is based on
    // the stack or global pointer.
    let pixels: Vec<u8> = (0..1 << 16).map(|i: usize| if i % 97 < 7 { 0xB4 } else { 0xA3 }).collect();
    let capitals: Vec<u8> = b"GGGGOOOOIIII!!!!GGGGGGGGOOOOOOOO".iter().copied().cycle().take(1 << 16).collect();
    for (name, data) in [("pixels", &pixels), ("capitals", &capitals)] {
        let common = data.chunks_exact(4).filter(|w| COMMON >> (w[3] >> 2) & 1 == 1).count();
        assert!(common * 256 >= data.len() / 4 * COMMON_SHARE, "{name}: the opcode share alone would take it for code");
        assert_eq!(blocks(data), vec![0], "{name}: not Alpha code");
        assert!(spans(data).is_empty(), "{name}");
    }
    // Each half of the structure alone: operates that set their zero bits beside stack-based stores, and valid operates
    // beside loads based on neither pointer.
    let words = |a: u32, b: u32| -> Vec<u8> { (0..1 << 14).flat_map(|i| if i % 2 == 0 { a } else { b }.to_le_bytes()).collect() };
    let stray = words(0x2D << 26 | 1 << 21 | 30 << 16 | 8, 0x11 << 26 | 1 << 21 | 2 << 16 | 7 << 13 | 0x20 << 5 | 3);
    let unframed = words(0x29 << 26 | 1 << 21 | 3 << 16 | 8, 0x11 << 26 | 1 << 21 | 2 << 16 | 0x20 << 5 | 3);
    assert_eq!(blocks(&stray), vec![0], "operates breaking their zero bits");
    assert_eq!(blocks(&unframed), vec![0], "memory never based on the stack or global pointer");
    let framed = words(0x29 << 26 | 1 << 21 | 30 << 16 | 8, 0x11 << 26 | 1 << 21 | 2 << 16 | 0x20 << 5 | 3);
    assert_eq!(blocks(&framed), vec![1], "the same words with a stack base and clean operates are code");
    let code = alpha_like(20_000, 21);
    assert_eq!(blocks(&code), vec![1, 1], "real code keeps its structure");
}
