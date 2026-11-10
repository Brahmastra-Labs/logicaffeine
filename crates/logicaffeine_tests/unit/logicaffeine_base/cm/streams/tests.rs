use super::*;

fn text(n: usize, seed: u64) -> Vec<u8> {
    const WORDS: [&str; 12] = ["stream", "opens", "the", "model", "sees", "every", "byte", "of", "a", "jar", "zip", "entry"];
    let mut s = seed;
    let mut out = Vec::new();
    while out.len() < n {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        out.extend_from_slice(WORDS[(s >> 59) as usize % WORDS.len()].as_bytes());
        out.push(if (s >> 40) % 11 == 0 { b'\n' } else { b' ' });
    }
    out.truncate(n);
    out
}

const LEVEL6: Params = Params { level: 6, mem_level: 8, window_bits: 15 };

/// A ZIP local file header for a deflated entry, then the entry's bytes.
fn zip_entry(name: &[u8], raw: &[u8], p: Params) -> (Vec<u8>, usize, usize) {
    let z = deflate(raw, p);
    let mut out = b"PK\x03\x04\x14\x00\x00\x00\x08\x00".to_vec();
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&(z.len() as u32).to_le_bytes());
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name);
    let at = out.len();
    out.extend_from_slice(&z);
    (out, at, z.len())
}

#[test]
fn a_zip_entry_zlib_wrote_is_found_opened_and_closed_exactly() {
    let raw = text(20_000, 1);
    let (entry, at, clen) = zip_entry(b"chrome/content.js", &raw, LEVEL6);
    let mut file = b"some bytes before the archive ".to_vec();
    let shift = file.len();
    file.extend_from_slice(&entry);
    file.extend_from_slice(b"PK\x01\x02 central directory follows");
    let found = find(&file);
    assert_eq!(found, vec![Stream { offset: shift + at, compressed: clen, raw: raw.len(), codec: Codec::Deflate(LEVEL6) }]);
    let opened = open(&file, &found);
    assert_eq!(opened.len(), file.len() - clen + raw.len());
    assert_eq!(&opened[shift + at..shift + at + raw.len()], &raw[..], "the entry's bytes appear in place");
    assert_eq!(close(&opened, &found).as_deref(), Some(&file[..]));
}

#[test]
fn every_level_and_memory_level_a_zip_tool_uses_is_recognized() {
    let raw = text(30_000, 2);
    for level in 1..=9 {
        for mem_level in [8, 9] {
            let p = Params { level, mem_level, window_bits: 15 };
            let (file, _, _) = zip_entry(b"a.txt", &raw, p);
            let found = find(&file);
            assert_eq!(found.len(), 1, "level {level} memory level {mem_level}");
            let Codec::Deflate(recorded) = found[0].codec else { panic!("a ZIP entry is a DEFLATE stream") };
            assert_eq!(deflate(&raw, recorded), deflate(&raw, p), "an equivalent setting is recorded");
            assert_eq!(close(&open(&file, &found), &found).as_deref(), Some(&file[..]));
        }
    }
}

#[test]
fn gzip_members_and_zlib_streams_are_found_too() {
    let raw = text(10_000, 3);
    let z = deflate(&raw, LEVEL6);
    let mut gz = vec![0x1F, 0x8B, 8, 0x08, 0, 0, 0, 0, 0, 3];
    gz.extend_from_slice(b"name.txt\0");
    let gz_at = gz.len();
    gz.extend_from_slice(&z);
    gz.extend_from_slice(&[0; 8]);
    let mut zl = vec![0x78, 0x9C];
    zl.extend_from_slice(&z);
    zl.extend_from_slice(&[0; 4]);
    for (file, at) in [(gz, gz_at), (zl, 2)] {
        let found = find(&file);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].offset, found[0].compressed), (at, z.len()));
        assert_eq!(close(&open(&file, &found), &found).as_deref(), Some(&file[..]));
    }
}

#[test]
fn a_stream_no_zlib_setting_reproduces_is_left_alone() {
    // Two stored blocks: valid DEFLATE, but zlib would have written one compressed block.
    let raw = text(600, 4);
    let mut z = Vec::new();
    for (i, half) in raw.chunks(300).enumerate() {
        z.push(u8::from(i == 1));
        z.extend_from_slice(&(half.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(half.len() as u16)).to_le_bytes());
        z.extend_from_slice(half);
    }
    assert_eq!(inflate(&z, raw.len()).map(|i| i.data), Some(raw.clone()));
    let mut file = vec![0x78, 0x9C];
    file.extend_from_slice(&z);
    assert!(find(&file).is_empty());
}

#[test]
fn the_expansion_bound_keeps_a_zip_bomb_closed() {
    let raw = vec![0u8; 4 << 20];
    let (file, _, clen) = zip_entry(b"zeros", &raw, LEVEL6);
    assert!(file.len() < 8 << 10 && clen < 8 << 10);
    assert!(raw.len() > expansion_bound(file.len()));
    assert!(find(&file).is_empty(), "opening it would pass the bound the decoder enforces");
}

#[test]
fn the_header_round_trips_and_refuses_what_a_decoder_must_not_trust() {
    let raw = text(5000, 5);
    let (one, _, _) = zip_entry(b"x", &raw, LEVEL6);
    let mut file = one.clone();
    file.extend_from_slice(&one);
    let found = find(&file);
    assert_eq!(found.len(), 2);
    let mut header = Vec::new();
    write_header(&found, &mut header);
    let mut pos = 0;
    let (back, opened) = read_header(&header, &mut pos, file.len()).expect("its own header");
    assert_eq!((back, opened, pos), (found.clone(), open(&file, &found).len(), header.len()));
    // Every truncation is refused.
    for cut in 0..header.len() {
        assert!(read_header(&header[..cut], &mut 0, file.len()).is_none(), "cut at {cut}");
    }
    // A stream running past the original, bad parameters, and an expansion past the bound.
    let past = [Stream { offset: 10, compressed: file.len(), raw: 100, codec: Codec::Deflate(LEVEL6) }];
    let bad = [Stream { offset: 0, compressed: 10, raw: 100, codec: Codec::Deflate(Params { level: 0, mem_level: 8, window_bits: 15 }) }];
    let huge = [Stream { offset: 0, compressed: 10, raw: expansion_bound(file.len()) + 1, codec: Codec::Deflate(LEVEL6) }];
    for streams in [&past[..], &bad[..], &huge[..]] {
        let mut h = Vec::new();
        write_header(streams, &mut h);
        assert!(read_header(&h, &mut 0, file.len()).is_none(), "{streams:?}");
    }
    // A corrupted inflated byte changes the re-deflated length or is caught by close's length check.
    let mut opened = open(&file, &found);
    opened.truncate(opened.len() - 1);
    assert!(close(&opened, &found).is_none());
}

/// A PNG: signature, an `IHDR`, the zlib stream of `raw` cut into `IDAT` chunks of `chunk` bytes, and `IEND`.
fn png(raw: &[u8], chunk: usize, p: Params) -> (Vec<u8>, usize, usize) {
    let mut out = PNG_SIGNATURE.to_vec();
    let put = |out: &mut Vec<u8>, kind: &[u8], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    };
    put(&mut out, b"IHDR", &[0, 0, 0, 64, 0, 0, 0, 32, 8, 0, 0, 0, 0]);
    let mut stream = vec![0x78, 0x9C];
    stream.extend_from_slice(&deflate(raw, p));
    stream.extend_from_slice(&adler32(raw).to_be_bytes());
    let first = out.len();
    for piece in stream.chunks(chunk) {
        put(&mut out, b"IDAT", piece);
    }
    let end = out.len();
    put(&mut out, b"IEND", &[]);
    (out, first, end)
}

#[test]
fn the_checksums_are_the_standard_ones() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    assert_eq!(adler32(&[]), 1);
}

#[test]
fn a_png_s_image_data_is_stitched_opened_and_rebuilt_exactly() {
    let raw = text(40_000, 21);
    let (file, first, end) = png(&raw, 1000, LEVEL6);
    let found = find(&file);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].codec, Codec::Png { params: LEVEL6, chunk: 1000, zlib: [0x78, 0x9C], stride: 65, bpp: 1 });
    assert_eq!((found[0].offset, found[0].compressed, found[0].raw), (first, end - first, raw.len()));
    let opened = open(&file, &found);
    assert_eq!(&opened[first..first + raw.len()], &raw[..]);
    assert_eq!(close(&opened, &found).as_deref(), Some(&file[..]));
    let mut header = Vec::new();
    write_header(&found, &mut header);
    let (back, _) = read_header(&header, &mut 0, file.len()).expect("its own header");
    assert_eq!(back, found);
}

#[test]
fn a_png_whose_chunks_do_not_frame_its_stream_exactly_is_left_alone() {
    let raw = text(20_000, 22);
    let (good, first, _) = png(&raw, 800, LEVEL6);
    assert_eq!(find(&good).len(), 1);
    let mut bad_crc = good.clone();
    let crc_at = first + 8 + 800;
    bad_crc[crc_at] ^= 1;
    assert!(find(&bad_crc).is_empty(), "a wrong CRC cannot be rebuilt");
    // The first chunk one byte shorter than the rest: no single chunk length frames the stream.
    let (mut uneven, first, end) = png(&raw, 800, LEVEL6);
    let region = uneven[first..end].to_vec();
    let mut payload = Vec::new();
    let mut p = 0;
    while p < region.len() {
        let len = u32::from_be_bytes(region[p..p + 4].try_into().unwrap()) as usize;
        payload.extend_from_slice(&region[p + 8..p + 8 + len]);
        p += 12 + len;
    }
    let mut rechunked = Vec::new();
    for (i, piece) in [&payload[..799], &payload[799..]].iter().enumerate() {
        for sub in piece.chunks(if i == 0 { 799 } else { 800 }) {
            rechunked.extend_from_slice(&(sub.len() as u32).to_be_bytes());
            let start = rechunked.len();
            rechunked.extend_from_slice(b"IDAT");
            rechunked.extend_from_slice(sub);
            let crc = crc32(&rechunked[start..]);
            rechunked.extend_from_slice(&crc.to_be_bytes());
        }
    }
    uneven.splice(first..end, rechunked);
    assert!(find(&uneven).is_empty(), "uneven chunks are left closed");
}

/// A GIF89a of `pixels` (`min_code` bits each): a global colour table, a graphic-control extension, one image
/// whose LZW stream (cleared at `clear_at`) sits in sub-blocks of `block` bytes, and the trailer.
fn gif(pixels: &[u8], min_code: u8, clear_at: u16, block: usize) -> (Vec<u8>, usize, usize) {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&[64, 0, 32, 0, 0x80 | (min_code - 1), 0, 0]);
    out.extend((0..3 << min_code).map(|i| i as u8));
    out.extend_from_slice(&[0x21, 0xF9, 4, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&[0x2C, 0, 0, 0, 0, 64, 0, 32, 0, 0, min_code]);
    let first = out.len();
    for piece in lzw::encode(pixels, min_code, clear_at).chunks(block) {
        out.push(piece.len() as u8);
        out.extend_from_slice(piece);
    }
    out.push(0);
    let end = out.len();
    out.push(0x3B);
    (out, first, end)
}

fn indexed(n: usize, seed: u64, colors: u8) -> Vec<u8> {
    let mut s = seed;
    let mut out = Vec::with_capacity(n);
    let mut v = 0u8;
    while out.len() < n {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        if (s >> 61) == 0 {
            v = ((s >> 40) % u64::from(colors)) as u8;
        }
        out.push(v);
    }
    out
}

#[test]
fn a_gif_image_is_opened_to_its_pixels_and_rebuilt_exactly() {
    let pixels = indexed(60_000, 31, 16);
    let (file, first, end) = gif(&pixels, 4, 4096, 254);
    let found = find(&file);
    assert_eq!(found, vec![Stream { offset: first, compressed: end - first, raw: pixels.len(), codec: Codec::Gif { min_code: 4, clear_at: 4096, block: 254, width: 64, tail: 0, cuts: Vec::new(), table: 0, colours: 0 } }]);
    let opened = open(&file, &found);
    assert_eq!(&opened[first..first + pixels.len()], &pixels[..]);
    assert_eq!(close(&opened, &found).as_deref(), Some(&file[..]));
    let mut header = Vec::new();
    write_header(&found, &mut header);
    assert_eq!(read_header(&header, &mut 0, file.len()).map(|(s, _)| s), Some(found));
}

#[test]
fn a_gif_no_known_encoder_policy_reproduces_is_left_alone() {
    let pixels = indexed(60_000, 32, 16);
    let (odd_clear, _, _) = gif(&pixels, 4, 1000, 254);
    assert!(find(&odd_clear).is_empty(), "no encoder clears at 1000 entries");
    let (good, first, end) = gif(&pixels, 4, 4096, 254);
    let mut uneven = good[..first].to_vec();
    let mut stitched = Vec::new();
    let mut p = first;
    while good[p] != 0 {
        stitched.extend_from_slice(&good[p + 1..p + 1 + usize::from(good[p])]);
        p += 1 + usize::from(good[p]);
    }
    for (i, piece) in [&stitched[..100], &stitched[100..]].iter().enumerate() {
        for sub in piece.chunks(if i == 0 { 100 } else { 254 }) {
            uneven.push(sub.len() as u8);
            uneven.extend_from_slice(sub);
        }
    }
    uneven.push(0);
    uneven.extend_from_slice(&good[end..]);
    assert!(find(&uneven).is_empty(), "sub-blocks of no single length");
}

#[test]
fn gif_settings_out_of_range_are_refused() {
    for codec in [
        Codec::Gif { min_code: 1, clear_at: 4096, block: 254, width: 64, tail: 0, cuts: Vec::new(), table: 0, colours: 0 },
        Codec::Gif { min_code: 9, clear_at: 4096, block: 254, width: 64, tail: 0, cuts: Vec::new(), table: 0, colours: 0 },
        Codec::Gif { min_code: 4, clear_at: 4097, block: 254, width: 64, tail: 0, cuts: Vec::new(), table: 0, colours: 0 },
        Codec::Gif { min_code: 4, clear_at: 17, block: 254, width: 64, tail: 0, cuts: Vec::new(), table: 0, colours: 0 },
        Codec::Gif { min_code: 4, clear_at: 4096, block: 0, width: 64, tail: 0, cuts: Vec::new(), table: 0, colours: 0 },
        Codec::Gif { min_code: 4, clear_at: 4096, block: 254, width: 64, tail: 2, cuts: Vec::new(), table: 0, colours: 0 },
    ] {
        let mut h = Vec::new();
        write_header(&[Stream { offset: 0, compressed: 10, raw: 100, codec: codec.clone() }], &mut h);
        assert!(read_header(&h, &mut 0, 1000).is_none(), "{codec:?}");
    }
}

#[test]
fn opened_images_carry_their_row_geometry() {
    let raw = text(20_000, 41);
    let (png_file, _, _) = png(&raw, 1000, LEVEL6);
    assert_eq!(find(&png_file)[0].codec.geometry(), Some((65, 1, 1)), "a 64-pixel grey PNG row: its filter byte and 64 bytes");
    let pixels = indexed(30_000, 42, 16);
    let (gif_file, _, _) = gif(&pixels, 4, 4096, 254);
    assert_eq!(find(&gif_file)[0].codec.geometry(), Some((64, 1, 0)));
    assert_eq!(Codec::Deflate(LEVEL6).geometry(), None);
    assert_eq!(Codec::Png { params: LEVEL6, chunk: 100, zlib: [0x78, 0x9C], stride: 0, bpp: 1 }.geometry(), None, "interlaced");
}

/// Records of a few random bytes and a long repeated phrase: few matches among many symbols, yet half the size, so
/// gzip ends a block every 4096 symbols, where no zlib memory level does.
fn gzip_shaped(records: usize, seed: u64) -> Vec<u8> {
    let phrase = b"<RDF:li resource='urn:help:toc:entry' nc:name='chapter'/>";
    let mut s = seed;
    let mut out = Vec::new();
    for _ in 0..records {
        for _ in 0..8 {
            s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            out.push((s >> 56) as u8);
        }
        out.extend_from_slice(phrase);
    }
    out
}

#[test]
fn a_zip_entry_gzip_made_is_found_named_and_restored() {
    // InfoZIP's zip deflates as gzip does: an entry under a real local header (method 8), its bytes no zlib setting makes.
    let raw = gzip_shaped(2000, 41);
    let z = super::super::deflate::gzip(&raw, 6);
    assert!(parameter_order().iter().all(|&p| deflate(&raw, p) != z), "no zlib setting makes these bytes");
    let mut file = b"PK\x03\x04\x14\x00\x00\x00\x08\x00\x00\x00\x00\x00".to_vec();
    file.extend_from_slice(&crc32(&raw).to_le_bytes());
    file.extend_from_slice(&(z.len() as u32).to_le_bytes());
    file.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    file.extend_from_slice(&9u16.to_le_bytes());
    file.extend_from_slice(&0u16.to_le_bytes());
    file.extend_from_slice(b"notes.txt");
    let at = file.len();
    file.extend_from_slice(&z);
    file.extend_from_slice(b"PK\x01\x02 and the central directory after it");
    let found = find(&file);
    assert_eq!(found, vec![Stream { offset: at, compressed: z.len(), raw: raw.len(), codec: Codec::Gzip { level: 6 } }]);
    let opened = open(&file, &found);
    assert_eq!(close(&opened, &found), Some(file.clone()), "the decoder re-makes gzip's bytes");
    let mut header = Vec::new();
    write_header(&found, &mut header);
    let mut pos = 0;
    assert_eq!(read_header(&header, &mut pos, file.len()).map(|(s, _)| s), Some(found.clone()));
    for level in [0u8, 10, 255] {
        let mut bad = header.clone();
        let last = bad.len() - 1;
        bad[last] = level;
        let mut pos = 0;
        assert!(read_header(&bad, &mut pos, file.len()).is_none(), "gzip level {level} is refused");
    }
}

#[test]
fn only_a_gif_holds_palette_indices() {
    assert!(Codec::Gif { min_code: 4, clear_at: 4096, block: 254, width: 64, tail: 0, cuts: Vec::new(), table: 0, colours: 0 }.indexed());
    assert!(!Codec::Deflate(LEVEL6).indexed());
    assert!(!Codec::Gzip { level: 6 }.indexed());
    assert!(!Codec::Png { params: LEVEL6, chunk: 100, zlib: [0x78, 0x9C], stride: 65, bpp: 1 }.indexed());
}

#[test]
fn a_gif_whose_encoder_flushes_one_more_byte_is_opened_and_rebuilt_exactly() {
    // Some encoders flush a zero byte after the end code: its code stream is ours and one more byte.
    let pixels = indexed(20_000, 33, 16);
    let (plain, first, _) = gif(&pixels, 4, 4096, 254);
    let mut code = lzw::encode(&pixels, 4, 4096);
    code.push(0);
    let mut file = plain[..first].to_vec();
    for piece in code.chunks(254) {
        file.push(piece.len() as u8);
        file.extend_from_slice(piece);
    }
    file.push(0);
    let end = file.len();
    file.push(0x3B);
    let found = find(&file);
    assert_eq!(found, vec![Stream { offset: first, compressed: end - first, raw: pixels.len(), codec: Codec::Gif { min_code: 4, clear_at: 4096, block: 254, width: 64, tail: 1, cuts: Vec::new(), table: 0, colours: 0 } }]);
    let opened = open(&file, &found);
    assert_eq!(close(&opened, &found).as_deref(), Some(&file[..]));
    let mut header = Vec::new();
    write_header(&found, &mut header);
    assert_eq!(read_header(&header, &mut 0, file.len()).map(|(s, _)| s), Some(found));
}

/// [`gif`] around a given LZW code stream.
fn gif_with(code: &[u8], min_code: u8, block: usize) -> (Vec<u8>, usize, usize) {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&[64, 0, 32, 0, 0x80 | (min_code - 1), 0, 0]);
    out.extend((0..3 << min_code).map(|i| i as u8));
    out.extend_from_slice(&[0x2C, 0, 0, 0, 0, 64, 0, 32, 0, 0, min_code]);
    let first = out.len();
    for piece in code.chunks(block) {
        out.push(piece.len() as u8);
        out.extend_from_slice(piece);
    }
    out.push(0);
    let end = out.len();
    out.push(0x3B);
    (out, first, end)
}

#[test]
fn a_gif_whose_encoder_cut_matches_short_opens_with_its_cuts_and_closes_exactly() {
    let pixels = indexed(60_000, 33, 16);
    let cut: Vec<u32> = (1..60).map(|k| k * 997).collect();
    let code = lzw::encode_cut(&pixels, 4, 4096, &cut);
    assert_ne!(code, lzw::encode(&pixels, 4, 4096), "the cuts change the stream");
    let (file, first, end) = gif_with(&code, 4, 254);
    let found = find(&file);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].offset, found[0].compressed, found[0].raw), (first, end - first, pixels.len()));
    let Codec::Gif { cuts, .. } = &found[0].codec else { panic!("a GIF stream") };
    assert!(!cuts.is_empty() && cuts.iter().all(|c| cut.contains(c)), "the cuts that ended a match early, and no others");
    let opened = open(&file, &found);
    assert_eq!(&opened[first..first + pixels.len()], &pixels[..]);
    assert_eq!(close(&opened, &found).as_deref(), Some(&file[..]));
    let mut header = Vec::new();
    write_header(&found, &mut header);
    assert_eq!(read_header(&header, &mut 0, file.len()).map(|(s, _)| s), Some(found));
}

#[test]
fn a_gif_cut_more_often_than_opening_pays_for_stays_closed() {
    let pixels = indexed(60_000, 34, 16);
    let cut: Vec<u32> = (1..20_000).map(|k| k * 3).collect();
    let (file, _, _) = gif_with(&lzw::encode_cut(&pixels, 4, 4096, &cut), 4, 254);
    assert!(find(&file).is_empty(), "past one cut per 16 bytes the list costs what opening saves");
}

#[test]
fn gif_cuts_that_do_not_ascend_inside_the_pixels_are_refused() {
    let gif = |cuts: Vec<u32>| Codec::Gif { min_code: 4, clear_at: 4096, block: 254, width: 64, tail: 0, cuts, table: 0, colours: 0 };
    let mut h = Vec::new();
    write_header(&[Stream { offset: 0, compressed: 10, raw: 100, codec: gif(vec![5, 9, 99]) }], &mut h);
    assert_eq!(read_header(&h, &mut 0, 1000).map(|(s, _)| s[0].codec.clone()), Some(gif(vec![5, 9, 99])));
    for cuts in [vec![0], vec![5, 5], vec![100], vec![5, 200]] {
        let mut h = Vec::new();
        write_header(&[Stream { offset: 0, compressed: 10, raw: 100, codec: gif(cuts.clone()) }], &mut h);
        assert!(read_header(&h, &mut 0, 1000).is_none(), "{cuts:?}");
    }
}

/// A GIF89a of `pixels` (`min_code` bits each) whose colour table is given byte for byte: global, or local to the
/// image with a rising global table beside it.
fn gif_coloured(pixels: &[u8], min_code: u8, table: &[u8], local: bool) -> (Vec<u8>, usize, usize, usize) {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&[64, 0, 32, 0, 0x80 | (min_code - 1), 0, 0]);
    let global_at = out.len();
    if local {
        out.extend((0..3 << min_code).map(|i| i as u8));
    } else {
        out.extend_from_slice(table);
    }
    out.extend_from_slice(&[0x2C, 0, 0, 0, 0, 64, 0, 32, 0]);
    out.push(if local { 0x80 | (min_code - 1) } else { 0 });
    let local_at = out.len();
    if local {
        out.extend_from_slice(table);
    }
    out.push(min_code);
    let first = out.len();
    for piece in lzw::encode(pixels, min_code, 4096).chunks(254) {
        out.push(piece.len() as u8);
        out.extend_from_slice(piece);
    }
    out.push(0);
    let end = out.len();
    out.push(0x3B);
    (out, first, end, if local { local_at } else { global_at })
}

#[test]
fn a_gif_whose_palette_is_out_of_luma_order_opens_with_its_indices_ranked_by_luma() {
    // White, black, green, red: by luma black, red, green, white (by red alone green would come second).
    let table = [255, 255, 255, 0, 0, 0, 0, 160, 0, 180, 0, 0];
    let rank = [3u8, 0, 2, 1];
    for local in [false, true] {
        let pixels = indexed(60_000, 51, 4);
        let (file, first, end, table_at) = gif_coloured(&pixels, 2, &table, local);
        let found = find(&file);
        assert_eq!((found.len(), found[0].offset, found[0].compressed), (1, first, end - first), "local {local}");
        let Codec::Gif { table: back, colours, .. } = found[0].codec.clone() else { panic!("a GIF stream") };
        assert_eq!((back as usize, colours), (first - table_at, 4), "local {local}: the image's own table, four entries");
        let opened = open(&file, &found);
        assert!(opened[first..first + pixels.len()].iter().zip(&pixels).all(|(&o, &p)| o == rank[usize::from(p)]), "each index its luma rank");
        assert_eq!(close(&opened, &found).as_deref(), Some(&file[..]), "local {local}: the indices come back");
        let mut header = Vec::new();
        write_header(&found, &mut header);
        assert_eq!(read_header(&header, &mut 0, file.len()).map(|(s, _)| s), Some(found));
    }
}

#[test]
fn a_gif_whose_palette_rises_in_luma_keeps_its_indices() {
    let pixels = indexed(60_000, 52, 16);
    let (file, first, _) = gif(&pixels, 4, 4096, 254);
    let found = find(&file);
    assert!(matches!(found[0].codec, Codec::Gif { table: 0, colours: 0, .. }), "{:?}", found[0].codec);
    assert_eq!(&open(&file, &found)[first..first + pixels.len()], &pixels[..]);
}

#[test]
fn gif_colour_tables_out_of_the_bytes_before_the_image_are_refused() {
    let gif = |table: u32, colours: u16| Codec::Gif { min_code: 4, clear_at: 4096, block: 254, width: 64, tail: 0, cuts: Vec::new(), table, colours };
    let header = |codec: Codec| {
        let mut h = Vec::new();
        write_header(&[Stream { offset: 100, compressed: 10, raw: 100, codec }], &mut h);
        h
    };
    let ok = header(gif(60, 16));
    assert_eq!(read_header(&ok, &mut 0, 1000).map(|(s, _)| s[0].codec.clone()), Some(gif(60, 16)));
    for (table, colours) in [(47u32, 16u16), (101, 16), (767, 256)] {
        assert!(read_header(&header(gif(table, colours)), &mut 0, 1000).is_none(), "table {table} back, {colours} colours");
    }
    // 256 colours 1600 bytes back from offset 2000 read; the same entry claiming 512 does not.
    let mut wide = Vec::new();
    write_header(&[Stream { offset: 2000, compressed: 10, raw: 100, codec: gif(1600, 256) }], &mut wide);
    assert_eq!(read_header(&wide, &mut 0, 3000).map(|(s, _)| s[0].codec.clone()), Some(gif(1600, 256)));
    let last = wide.len() - 1;
    wide[last] = 8;
    assert!(read_header(&wide, &mut 0, 3000).is_none(), "512 colours");
}
