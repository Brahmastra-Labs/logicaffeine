use super::*;
use super::super::deflate::{deflate, gzip, Params};
use super::super::streams::{close, find, open};

const LEVEL6: Params = Params { level: 6, mem_level: 8, window_bits: 15 };

fn text(n: usize, seed: u64) -> Vec<u8> {
    const WORDS: [&str; 12] = ["tar", "holds", "the", "files", "each", "header", "sums", "its", "own", "bytes", "zip", "entry"];
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

/// How a tar writer spells a header's checksum.
#[derive(Clone, Copy)]
enum Shape {
    /// Six digits padded with zeros, a NUL and a space (GNU tar, Python, Go).
    Zeros,
    /// Six places padded with spaces, a NUL and a space (old GNU tar).
    Spaces,
    /// Seven digits and a NUL.
    Seven,
}

fn spell(sum: u32, shape: Shape) -> [u8; 8] {
    let s = match shape {
        Shape::Zeros => format!("{sum:06o}\0 "),
        Shape::Spaces => format!("{sum:6o}\0 "),
        Shape::Seven => format!("{sum:07o}\0"),
    };
    s.as_bytes().try_into().unwrap()
}

/// A ustar header for `name`, `size` bytes, its checksum spelled `shape`.
fn tar_header(name: &[u8], size: usize, shape: Shape) -> Vec<u8> {
    let mut h = vec![0u8; 512];
    h[..name.len()].copy_from_slice(name);
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0001750\0");
    h[116..124].copy_from_slice(b"0001750\0");
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[136..148].copy_from_slice(b"07345216734\0");
    h[156] = b'0';
    h[257..265].copy_from_slice(b"ustar  \0");
    h[265..273].copy_from_slice(b"tristen\0");
    h[148..156].fill(b' ');
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(&spell(sum, shape));
    h
}

fn tar(files: &[(&[u8], Vec<u8>)], shape: Shape) -> (Vec<u8>, Vec<usize>) {
    let mut out = Vec::new();
    let mut heads = Vec::new();
    for (name, data) in files {
        heads.push(out.len());
        out.extend(tar_header(name, data.len(), shape));
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(512) * 512, 0);
    }
    out.resize(out.len() + 1024, 0);
    (out, heads)
}

fn three_files() -> Vec<(&'static [u8], Vec<u8>)> {
    vec![(&b"docs/README"[..], text(700, 1)), (&b"docs/COPYING"[..], text(2000, 2)), (&b"src/main.c"[..], text(10, 3))]
}

#[test]
fn a_tar_header_s_checksum_becomes_zeros_in_its_own_shape_and_comes_back() {
    for (shape, blank) in [(Shape::Zeros, &b"000000\0 "[..]), (Shape::Spaces, b" 00000\0 "), (Shape::Seven, b"0000000\0")] {
        let (original, heads) = tar(&three_files(), shape);
        let mut data = original.clone();
        let marks = blank_tar(&mut data);
        assert_eq!(marks, Marks { found: 3, kept: vec![] });
        for &h in &heads {
            assert_eq!(&data[h + 148..h + 156], blank, "the digits are zeros, the spaces and NULs stay");
            data[h + 148..h + 156].copy_from_slice(&original[h + 148..h + 156]);
        }
        assert_eq!(data, original, "nothing but the checksums changed");
        for &h in &heads {
            data[h + 148..h + 156].copy_from_slice(blank);
        }
        assert_eq!(restore_tar(&mut data, &marks), Some(()));
        assert_eq!(data, original);
    }
}

#[test]
fn a_checksum_that_is_wrong_missing_or_not_octal_is_left_as_it_is() {
    let (original, heads) = tar(&three_files(), Shape::Zeros);
    let mut odd = original.clone();
    // One more than the sum; a field of NULs; a digit 8; a field already all zeros, which the decoder would take
    // for a blank.
    let field = |d: &mut Vec<u8>, h: usize, f: &[u8]| d[h + 148..h + 156].copy_from_slice(f);
    let sum = |d: &[u8], h: usize| -> u32 { d[h..h + 512].iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { 32 } else { u32::from(b) }).sum() };
    let s0 = sum(&odd, heads[0]);
    field(&mut odd, heads[0], &spell(s0 + 1, Shape::Zeros));
    let (mut wide, _) = tar(&vec![(&b"x"[..], text(1, 9)); 4], Shape::Zeros);
    let w = wide.len();
    wide.truncate(w - 1024);
    let at = odd.len() - 1024;
    odd.splice(at..at, wide);
    let more: Vec<usize> = (0..4).map(|i| at + i * 1024).collect();
    field(&mut odd, more[0], b"\0\0\0\0\0\0\0\0");
    field(&mut odd, more[1], b"001238\0 ");
    field(&mut odd, more[2], b"000000\0 ");
    let mut data = odd.clone();
    let marks = blank_tar(&mut data);
    assert_eq!(marks.found, 7);
    assert_eq!(marks.kept, vec![0, 3, 4, 5], "the wrong, missing, non-octal and zero checksums");
    assert_eq!(&data[heads[0] + 148..heads[0] + 156], &odd[heads[0] + 148..heads[0] + 156]);
    assert_eq!(&data[more[2] + 148..more[2] + 156], b"000000\0 ", "an all-zero field is kept, not restored");
    assert_eq!(&data[heads[1] + 148..heads[1] + 156], b"000000\0 ");
    assert_eq!(&data[more[3] + 148..more[3] + 156], b"000000\0 ");
    assert_eq!(restore_tar(&mut data, &marks), Some(()));
    assert_eq!(data, odd);
}

#[test]
fn ustar_in_a_file_s_bytes_is_no_header() {
    let mut body = text(3000, 4);
    body[900..905].copy_from_slice(b"ustar");
    let (original, _) = tar(&[(&b"notes"[..], body)], Shape::Zeros);
    let mut data = original.clone();
    let marks = blank_tar(&mut data);
    assert_eq!(marks, Marks { found: 2, kept: vec![1] });
    assert_eq!(restore_tar(&mut data, &marks), Some(()));
    assert_eq!(data, original);
}

#[test]
fn restoring_refuses_a_record_that_does_not_match_what_it_finds() {
    let (original, _) = tar(&three_files(), Shape::Zeros);
    let mut data = original.clone();
    let marks = blank_tar(&mut data);
    for wrong in [Marks { found: 2, kept: vec![] }, Marks { found: 4, kept: vec![3] }] {
        assert_eq!(restore_tar(&mut data.clone(), &wrong), None, "{wrong:?}");
    }
    assert_eq!(restore_tar(&mut original.clone(), &Marks::none(3)), Some(()), "nothing to restore is no work");
    let mut cut = data.clone();
    cut.truncate(512 * 4 + 100);
    assert_eq!(restore_tar(&mut cut, &marks), None, "a header gone");
}

/// How a ZIP entry is held.
#[derive(Clone, Copy, PartialEq)]
enum Held {
    Deflated,
    Stored,
    /// Deflated, its CRC and sizes in a data descriptor after it (with the descriptor's signature or without).
    Described { signature: bool },
}

fn le16(out: &mut Vec<u8>, v: usize) {
    out.extend_from_slice(&(v as u16).to_le_bytes());
}

fn le32(out: &mut Vec<u8>, v: usize) {
    out.extend_from_slice(&(v as u32).to_le_bytes());
}

/// A ZIP archive of `entries`, every field as a writer puts it: CRC-32s, sizes, offsets, a central directory and an end
/// record.
fn zip(entries: &[(&[u8], Vec<u8>, Held)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, raw, held) in entries {
        let body = if *held == Held::Stored { raw.clone() } else { deflate(raw, LEVEL6) };
        let crc = crc32(raw) as usize;
        let described = matches!(held, Held::Described { .. });
        let offset = out.len();
        let fields = |out: &mut Vec<u8>, zero: bool| {
            le32(out, if zero { 0 } else { crc });
            le32(out, if zero { 0 } else { body.len() });
            le32(out, if zero { 0 } else { raw.len() });
        };
        let (flags, method) = (if described { 8 } else { 0 }, if *held == Held::Stored { 0 } else { 8 });
        out.extend_from_slice(b"PK\x03\x04\x14\x00");
        le16(&mut out, flags);
        le16(&mut out, method);
        out.extend_from_slice(&[0x21, 0x6E, 0xBF, 0x2C]);
        fields(&mut out, described);
        le16(&mut out, name.len());
        le16(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(&body);
        if let Held::Described { signature } = held {
            if *signature {
                out.extend_from_slice(b"PK\x07\x08");
            }
            fields(&mut out, false);
        }
        central.extend_from_slice(b"PK\x01\x02\x14\x03\x14\x00");
        le16(&mut central, flags);
        le16(&mut central, method);
        central.extend_from_slice(&[0x21, 0x6E, 0xBF, 0x2C]);
        fields(&mut central, false);
        le16(&mut central, name.len());
        central.extend_from_slice(&[0; 8]);
        le32(&mut central, 0x81A4_0000);
        le32(&mut central, offset);
        central.extend_from_slice(name);
    }
    let cd = out.len();
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    le16(&mut out, entries.len());
    le16(&mut out, entries.len());
    le32(&mut out, central.len());
    le32(&mut out, cd);
    le16(&mut out, 0);
    out
}

fn four_entries() -> Vec<(&'static [u8], Vec<u8>, Held)> {
    vec![
        (&b"content/browser.js"[..], text(9000, 11), Held::Deflated),
        (&b"skin/icon.gif"[..], text(300, 12), Held::Stored),
        (&b"skin/"[..], Vec::new(), Held::Stored),
        (&b"locale/en-US/browser.dtd"[..], text(4000, 13), Held::Deflated),
    ]
}

/// Where each local header and each central entry of a [`zip`] archive starts.
fn layout(z: &[u8]) -> (Vec<usize>, Vec<usize>) {
    let at = |sig: &[u8]| z.windows(4).enumerate().filter(|(_, w)| *w == sig).map(|(i, _)| i).collect::<Vec<_>>();
    (at(b"PK\x03\x04"), at(b"PK\x01\x02"))
}

#[test]
fn a_zip_archive_s_crcs_sizes_and_offsets_become_zeros_and_come_back() {
    let entries = four_entries();
    let original = zip(&entries);
    let (local, central) = layout(&original);
    let mut data = original.clone();
    let marks = blank_archives(&mut data);
    assert_eq!(marks, Marks { found: 1, kept: vec![] });
    let mut expect = original.clone();
    for (i, (_, _, held)) in entries.iter().enumerate() {
        let n = if *held == Held::Stored { 4 } else { 12 };
        expect[local[i] + 14..local[i] + 14 + n].fill(0);
        expect[central[i] + 16..central[i] + 28].fill(0);
        expect[central[i] + 42..central[i] + 46].fill(0);
    }
    assert_eq!(data, expect, "a deflated entry's CRC and sizes, a stored one's CRC (its sizes say where it ends), and the directory's copies with each offset");
    assert_eq!(restore_archives(&mut data, &marks), Some(()));
    assert_eq!(data, original);
}

#[test]
fn a_data_descriptor_s_fields_are_blanked_with_or_without_its_signature() {
    for signature in [true, false] {
        let entries = vec![(&b"a.txt"[..], text(5000, 21), Held::Described { signature }), (&b"b.txt"[..], text(3000, 22), Held::Deflated)];
        let original = zip(&entries);
        let (local, central) = layout(&original);
        let mut data = original.clone();
        let marks = blank_archives(&mut data);
        assert_eq!(marks, Marks { found: 1, kept: vec![] }, "signature {signature}");
        let descriptor = local[1] - 12;
        assert!(data[descriptor..local[1]].iter().all(|&b| b == 0), "the descriptor's CRC and sizes");
        if signature {
            assert_eq!(&data[descriptor - 4..descriptor], b"PK\x07\x08", "its signature stays");
        }
        assert_eq!(&data[local[0] + 14..local[0] + 26], &original[local[0] + 14..local[0] + 26], "the local header's zeros stay as they were");
        assert!(data[central[0] + 16..central[0] + 28].iter().all(|&b| b == 0));
        assert_eq!(restore_archives(&mut data, &marks), Some(()));
        assert_eq!(data, original);
    }
}

#[test]
fn an_archive_whose_directory_disagrees_with_its_entries_keeps_every_field() {
    let entries = four_entries();
    let (_, central) = layout(&zip(&entries));
    let mut wrong_crc = zip(&entries);
    wrong_crc[central[3] + 16] ^= 1;
    let mut wrong_offset = zip(&entries);
    wrong_offset[central[1] + 42] ^= 1;
    let mut encrypted = zip(&entries);
    let (local, _) = layout(&encrypted);
    encrypted[local[0] + 6] |= 1;
    let mut bzip2 = zip(&entries);
    bzip2[local[1] + 8] = 12;
    let mut short_directory = zip(&entries);
    let e = short_directory.len() - 22;
    short_directory[e + 12] -= 1;
    for (what, original) in [("a CRC", wrong_crc), ("an offset", wrong_offset), ("an encrypted entry", encrypted), ("another method", bzip2), ("a directory size", short_directory)] {
        let mut data = original.clone();
        let marks = blank_archives(&mut data);
        assert_eq!(marks, Marks { found: 1, kept: vec![0] }, "{what}");
        assert_eq!(data, original, "{what}");
    }
}

#[test]
fn a_signature_blanking_would_erase_keeps_the_archive_as_it_is() {
    // A stored entry of 1541 bytes (its size 05 06 00 00) whose CRC-32 ends in 50 4B: an end record's signature
    // straddles the two fields, in the local header and in the directory, and blanking the CRC would erase it — the
    // decoder would not find what the encoder found.
    let base = text(1537, 81);
    let raw = (0u32..).map(|k| [&base[..], &k.to_le_bytes()].concat()).find(|r| crc32(r) >> 16 == 0x4B50).unwrap();
    let original = zip(&[(&b"forged"[..], raw, Held::Stored)]);
    let mut data = original.clone();
    let marks = blank_archives(&mut data);
    assert_eq!(marks, Marks { found: 3, kept: vec![0, 1, 2] });
    assert_eq!(data, original);
}

#[test]
fn an_archive_stored_in_another_is_blanked_after_it_and_restored_before_it() {
    // The outer entry's CRC covers the inner archive's own fields: computed from them as they are on both sides.
    let inner = zip(&four_entries());
    let outer_entries = vec![(&b"readme"[..], text(2000, 31), Held::Deflated), (&b"inner.jar"[..], inner.clone(), Held::Stored)];
    let original = zip(&outer_entries);
    let mut data = original.clone();
    let marks = blank_archives(&mut data);
    assert_eq!(marks, Marks { found: 2, kept: vec![] });
    let at = original.windows(inner.len()).position(|w| w == &inner[..]).unwrap();
    let (local, _) = layout(&inner);
    assert!(data[at + local[0] + 14..at + local[0] + 26].iter().all(|&b| b == 0), "the inner archive is blanked too");
    assert_eq!(restore_archives(&mut data, &marks), Some(()));
    assert_eq!(data, original);
}

#[test]
fn archives_side_by_side_in_a_tar_are_each_blanked() {
    let jars: Vec<Vec<u8>> = (0..3).map(|i| zip(&[(&b"x.js"[..], text(3000 + 500 * i, 40 + i as u64), Held::Deflated)])).collect();
    let (original, _) = tar(&[(&b"a.jar"[..], jars[0].clone()), (&b"b.jar"[..], jars[1].clone()), (&b"c.jar"[..], jars[2].clone())], Shape::Spaces);
    let mut data = original.clone();
    let marks = blank_archives(&mut data);
    assert_eq!(marks, Marks { found: 3, kept: vec![] });
    let tar_marks = blank_tar(&mut data);
    assert_eq!(tar_marks, Marks { found: 3, kept: vec![] }, "and the tar's headers around them");
    assert_eq!(restore_tar(&mut data, &tar_marks), Some(()));
    assert_eq!(restore_archives(&mut data, &marks), Some(()));
    assert_eq!(data, original);
}

fn chunk(out: &mut Vec<u8>, kind: &[u8], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A greyscale PNG: `IHDR`, a `tEXt`, its image data in two `IDAT` chunks, `IEND`; and where each chunk's CRC sits.
fn png_image(width: usize, height: usize) -> (Vec<u8>, Vec<(usize, bool)>) {
    let mut out = PNG_SIGNATURE.to_vec();
    let mut crcs = Vec::new();
    let mut put = |out: &mut Vec<u8>, kind: &[u8], body: &[u8]| {
        chunk(out, kind, body);
        crcs.push((out.len() - 4, kind == b"IDAT"));
    };
    let mut ihdr = (width as u32).to_be_bytes().to_vec();
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
    put(&mut out, b"IHDR", &ihdr);
    put(&mut out, b"tEXt", b"Software\0logos");
    let rows: Vec<u8> = (0..height).flat_map(|y| std::iter::once(0).chain((0..width).map(move |x| ((x * 3 + y * 5) % 251) as u8))).collect();
    let z = zlib(&rows, None);
    let half = z.len().div_ceil(2);
    put(&mut out, b"IDAT", &z[..half]);
    put(&mut out, b"IDAT", &z[half..]);
    put(&mut out, b"IEND", &[]);
    (out, crcs)
}

#[test]
fn a_png_s_chunk_crcs_become_zeros_and_come_back_but_its_image_data_s_stay() {
    let (original, crcs) = png_image(40, 30);
    let mut data = original.clone();
    let marks = blank_images(&mut data);
    assert_eq!(marks, Marks { found: 1, kept: vec![] });
    for &(at, idat) in &crcs {
        if idat {
            assert_eq!(&data[at..at + 4], &original[at..at + 4], "opening the image data checks these");
        } else {
            assert_eq!(&data[at..at + 4], &[0; 4]);
        }
    }
    assert_eq!(find(&data), find(&original), "the image data opens as before");
    assert_eq!(find(&data).len(), 1);
    assert_eq!(restore_images(&mut data, &marks), Some(()));
    assert_eq!(data, original);
}

#[test]
fn a_png_cut_short_or_with_a_wrong_crc_keeps_every_crc() {
    let (whole, crcs) = png_image(16, 16);
    let mut wrong = whole.clone();
    wrong[crcs[1].0] ^= 0x40;
    let cut = whole[..whole.len() - 6].to_vec();
    for (what, original) in [("a wrong CRC", wrong), ("no IEND", cut)] {
        let mut data = original.clone();
        assert_eq!(blank_images(&mut data), Marks { found: 1, kept: vec![0] }, "{what}");
        assert_eq!(data, original, "{what}");
    }
}

#[test]
fn a_png_stored_in_a_jar_comes_back_before_the_jar_s_crc_is_computed_over_it() {
    let (image, _) = png_image(24, 24);
    let original = zip(&[(&b"skin/logo.png"[..], image, Held::Stored), (&b"skin/a.css"[..], text(3000, 71), Held::Deflated)]);
    let mut data = original.clone();
    let archives = blank_archives(&mut data);
    let images = blank_images(&mut data);
    assert!(archives.any() && images.any());
    let blanked = data.clone();
    assert_eq!(restore_images(&mut data, &images), Some(()));
    assert_eq!(restore_archives(&mut data, &archives), Some(()));
    assert_eq!(data, original);
    let mut backwards = blanked;
    assert_eq!(restore_archives(&mut backwards, &archives), Some(()));
    assert_eq!(restore_images(&mut backwards, &images), Some(()));
    assert_ne!(backwards, original, "the jar's CRC of the image, computed over its blanks, is wrong");
}

/// A zlib stream of `raw` with its Adler-32, or with `trailer` in its place.
fn zlib(raw: &[u8], trailer: Option<[u8; 4]>) -> Vec<u8> {
    let mut z = vec![0x78, 0x9C];
    z.extend(deflate(raw, LEVEL6));
    z.extend_from_slice(&trailer.unwrap_or(adler32(raw).to_be_bytes()));
    z
}

#[test]
fn a_zlib_stream_s_adler_and_a_gzip_member_s_crc_and_length_are_blanked_once_opened() {
    let (a, b, c) = (text(8000, 51), text(6000, 52), text(4000, 53));
    let mut file = b"%PDF-1.4\n1 0 obj\n<< /Length 9 /Filter /FlateDecode >>\nstream\n".to_vec();
    file.extend(zlib(&a, None));
    file.extend_from_slice(b"\nendstream\nendobj\n");
    let gz_at = file.len();
    file.extend_from_slice(&[0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 3]);
    file.extend(gzip(&b, 6));
    file.extend_from_slice(&crc32(&b).to_le_bytes());
    file.extend_from_slice(&(b.len() as u32).to_le_bytes());
    file.extend(zlib(&c, Some([0; 4])));
    file.extend_from_slice(b"\ntrailer");
    let streams = find(&file);
    assert_eq!(streams.len(), 3);
    assert!(streams[1].offset == gz_at + 10);
    let mut opened = open(&file, &streams);
    let before = opened.clone();
    let codes = blank_trailers(&mut opened, &streams);
    assert_eq!(codes, vec![TRAILER_ZLIB, TRAILER_GZIP, TRAILER_NONE], "a zero trailer is no Adler-32 of its stream");
    let changed: Vec<usize> = (0..opened.len()).filter(|&i| opened[i] != before[i]).collect();
    assert!(changed.len() >= 8 && changed.len() <= 12, "four bytes, then eight, became zeros");
    assert!(changed.iter().all(|&i| opened[i] == 0));
    assert_eq!(restore_trailers(&mut opened, &streams, &codes), Some(()));
    assert_eq!(opened, before);
    assert_eq!(close(&opened, &streams).as_deref(), Some(&file[..]));
    assert_eq!(restore_trailers(&mut opened.clone(), &streams, &codes[..2]), None, "a code for every stream");
}

#[test]
fn a_trailer_never_reaches_into_the_next_stream() {
    // Two DEFLATE streams back to back, the second's decoded bytes beginning with the first's Adler-32: those four
    // bytes are the second stream's, not a trailer.
    let a = text(5000, 61);
    let mut b = adler32(&a).to_be_bytes().to_vec();
    b.extend(text(5000, 62));
    let (za, zb) = (deflate(&a, LEVEL6), deflate(&b, LEVEL6));
    let mut file = vec![0x78, 0x9C];
    file.extend_from_slice(&za);
    file.extend_from_slice(&zb);
    let streams = vec![
        Stream { offset: 2, compressed: za.len(), raw: a.len(), codec: Codec::Deflate(LEVEL6) },
        Stream { offset: 2 + za.len(), compressed: zb.len(), raw: b.len(), codec: Codec::Deflate(LEVEL6) },
    ];
    let mut opened = open(&file, &streams);
    assert_eq!(&opened[2 + a.len()..2 + a.len() + 4], &adler32(&a).to_be_bytes());
    let before = opened.clone();
    assert_eq!(blank_trailers(&mut opened, &streams), vec![TRAILER_NONE, TRAILER_NONE]);
    assert_eq!(opened, before);
    assert_eq!(restore_trailers(&mut opened, &streams, &[TRAILER_ZLIB, TRAILER_NONE]), None, "a record that says otherwise is refused");
}

#[test]
fn the_record_round_trips_and_refuses_what_a_decoder_must_not_trust() {
    let r = Record {
        archives: vec![Marks { found: 3, kept: vec![1] }, Marks { found: 0, kept: vec![] }, Marks { found: 5, kept: vec![0, 4] }],
        images: vec![Marks { found: 1, kept: vec![] }, Marks { found: 2, kept: vec![1] }, Marks { found: 0, kept: vec![] }],
        trailers: vec![vec![TRAILER_ZLIB, TRAILER_NONE], vec![TRAILER_GZIP]],
        tar: Marks { found: 1629, kept: vec![7, 8, 1000] },
    };
    let mut out = Vec::new();
    write_record(&r, &mut out);
    let mut pos = 0;
    assert_eq!(read_record(&out, &mut pos, &[2, 1], 1 << 20), Some(r.clone()));
    assert_eq!(pos, out.len());
    for cut in 0..out.len() {
        assert!(read_record(&out[..cut], &mut 0, &[2, 1], 1 << 20).is_none(), "cut at {cut}");
    }
    assert!(read_record(&out, &mut 0, &[2, 1], 1000).is_none(), "more found than the bytes could hold");
    let bad = |m: Marks| {
        let mut o = Vec::new();
        write_record(&Record { archives: vec![m], images: vec![Marks::default()], trailers: vec![], tar: Marks::default() }, &mut o);
        o
    };
    assert!(read_record(&bad(Marks { found: 2, kept: vec![2] }), &mut 0, &[], 100).is_none(), "an index past the found");
    // Archives found 2, kept 3 (indices 0, 1, 2), then the images' and the tar's marks.
    assert!(read_record(&[2, 3, 0, 0, 0, 0, 0, 0, 0], &mut 0, &[], 100).is_none(), "more kept than found");
    let mut code3 = Vec::new();
    write_record(&Record { archives: vec![Marks::default(); 2], images: vec![Marks::default(); 2], trailers: vec![vec![3]], tar: Marks::default() }, &mut code3);
    assert!(read_record(&code3, &mut 0, &[1], 100).is_none(), "trailer code 3");
}

/// A small xorshift for the fuzz below.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn any_bytes_come_back_from_blanking_and_restoring() {
    // Real containers, pieces of them, their signatures planted in noise, overlapping headers, fields flipped: whatever
    // is blanked comes back exactly.
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let (t, _) = tar(&three_files(), Shape::Zeros);
    let z = zip(&four_entries());
    let (g, _) = png_image(20, 20);
    for round in 0..200 {
        let mut data = Vec::new();
        while data.len() < 20_000 {
            match rng.next() % 7 {
                0 => data.extend_from_slice(&t),
                1 => data.extend_from_slice(&z),
                6 => data.extend_from_slice(&g),
                2 => {
                    let piece = [&t[..], &z[..], &g[..]][rng.next() as usize % 3];
                    let a = rng.next() as usize % piece.len();
                    let b = a + rng.next() as usize % (piece.len() - a);
                    data.extend_from_slice(&piece[a..b]);
                }
                3 => data.extend_from_slice([&b"ustar"[..], b"PK\x05\x06", PNG_SIGNATURE][rng.next() as usize % 3]),
                4 => {
                    // A second tar header 100 to 400 bytes into the last one.
                    let h = tar_header(b"overlap", 3, Shape::Zeros);
                    let back = data.len().min(100 + rng.next() as usize % 300);
                    let at = data.len() - back;
                    data.truncate(at);
                    data.extend(h);
                }
                _ => data.extend((0..rng.next() % 300).map(|_| rng.next() as u8)),
            }
        }
        for _ in 0..rng.next() % 4 {
            let i = rng.next() as usize % data.len();
            data[i] ^= 1 << (rng.next() % 8);
        }
        let original = data.clone();
        let archives = blank_archives(&mut data);
        let images = blank_images(&mut data);
        let tars = blank_tar(&mut data);
        assert_eq!(restore_tar(&mut data, &tars), Some(()), "round {round}");
        assert_eq!(restore_images(&mut data, &images), Some(()), "round {round}");
        assert_eq!(restore_archives(&mut data, &archives), Some(()), "round {round}");
        assert!(data == original, "round {round}");
    }
}
