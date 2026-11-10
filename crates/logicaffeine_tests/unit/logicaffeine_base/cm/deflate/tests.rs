use super::*;

/// SplitMix64, the generator `zlib_digests.py` mirrors.
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

const WORDS: [&str; 24] = [
    "the", "codec", "ships", "a", "column", "of", "values", "and", "every", "decoder", "reads", "it", "back", "bit",
    "for", "exactly", "as", "written", "when", "model", "predicts", "next", "symbol", "well",
];

fn wordy_text(n: usize, seed: u64) -> Vec<u8> {
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

/// The inputs the digests were taken of: empty, one byte, a long zero run (window slides, maximal matches),
/// random bytes (stored blocks), text (dynamic trees, symbol-buffer flushes), periodic runs, skewed binary,
/// and repeats farther than zlib's 3-byte match horizon.
fn inputs() -> Vec<Vec<u8>> {
    let mut r = Mix(3);
    let random: Vec<u8> = (0..100_000).map(|_| r.next() as u8).collect();
    let mut s = Mix(6);
    let skewed: Vec<u8> = (0..120_000)
        .map(|_| {
            let v = s.next();
            if v % 4 == 0 { (v >> 8) as u8 } else { (v >> 8) as u8 & 0x0F }
        })
        .collect();
    let mut f = Mix(7);
    let base: Vec<u8> = (0..5000).map(|_| f.next() as u8).collect();
    let far: Vec<u8> = (0..60_000).map(|i| if (i / 3000) % 2 == 0 { base[i % 5000] } else { f.next() as u8 & 0x3F }).collect();
    let mut runs = b"abc".repeat(5000);
    runs.extend_from_slice(&[b'x'; 300]);
    for _ in 0..40 {
        runs.extend(0..=255u8);
    }
    vec![Vec::new(), b"A".to_vec(), vec![0; 70_000], random, wordy_text(200_000, 4), runs, skewed, far]
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01B3))
}

/// (input, level, memory level, window bits, zlib's length, FNV-1a 64 of zlib's bytes), from zlib 1.3.
const ZLIB: &[(usize, u8, u8, u8, usize, u64)] = &[
    (0, 1, 8, 15, 2, 0x0835EE07B4EE5316),
    (0, 2, 8, 15, 2, 0x0835EE07B4EE5316),
    (0, 3, 8, 15, 2, 0x0835EE07B4EE5316),
    (0, 4, 8, 15, 2, 0x0835EE07B4EE5316),
    (0, 5, 8, 15, 2, 0x0835EE07B4EE5316),
    (0, 6, 8, 15, 2, 0x0835EE07B4EE5316),
    (0, 7, 8, 15, 2, 0x0835EE07B4EE5316),
    (0, 8, 8, 15, 2, 0x0835EE07B4EE5316),
    (0, 9, 8, 15, 2, 0x0835EE07B4EE5316),
    (0, 1, 9, 15, 2, 0x0835EE07B4EE5316),
    (0, 2, 9, 15, 2, 0x0835EE07B4EE5316),
    (0, 3, 9, 15, 2, 0x0835EE07B4EE5316),
    (0, 4, 9, 15, 2, 0x0835EE07B4EE5316),
    (0, 5, 9, 15, 2, 0x0835EE07B4EE5316),
    (0, 6, 9, 15, 2, 0x0835EE07B4EE5316),
    (0, 7, 9, 15, 2, 0x0835EE07B4EE5316),
    (0, 8, 9, 15, 2, 0x0835EE07B4EE5316),
    (0, 9, 9, 15, 2, 0x0835EE07B4EE5316),
    (0, 1, 1, 9, 2, 0x0835EE07B4EE5316),
    (0, 1, 1, 12, 2, 0x0835EE07B4EE5316),
    (0, 6, 1, 9, 2, 0x0835EE07B4EE5316),
    (0, 6, 1, 12, 2, 0x0835EE07B4EE5316),
    (0, 9, 1, 9, 2, 0x0835EE07B4EE5316),
    (0, 9, 1, 12, 2, 0x0835EE07B4EE5316),
    (1, 1, 8, 15, 3, 0x81CB35195C827B16),
    (1, 2, 8, 15, 3, 0x81CB35195C827B16),
    (1, 3, 8, 15, 3, 0x81CB35195C827B16),
    (1, 4, 8, 15, 3, 0x81CB35195C827B16),
    (1, 5, 8, 15, 3, 0x81CB35195C827B16),
    (1, 6, 8, 15, 3, 0x81CB35195C827B16),
    (1, 7, 8, 15, 3, 0x81CB35195C827B16),
    (1, 8, 8, 15, 3, 0x81CB35195C827B16),
    (1, 9, 8, 15, 3, 0x81CB35195C827B16),
    (1, 1, 9, 15, 3, 0x81CB35195C827B16),
    (1, 2, 9, 15, 3, 0x81CB35195C827B16),
    (1, 3, 9, 15, 3, 0x81CB35195C827B16),
    (1, 4, 9, 15, 3, 0x81CB35195C827B16),
    (1, 5, 9, 15, 3, 0x81CB35195C827B16),
    (1, 6, 9, 15, 3, 0x81CB35195C827B16),
    (1, 7, 9, 15, 3, 0x81CB35195C827B16),
    (1, 8, 9, 15, 3, 0x81CB35195C827B16),
    (1, 9, 9, 15, 3, 0x81CB35195C827B16),
    (1, 1, 1, 9, 3, 0x81CB35195C827B16),
    (1, 1, 1, 12, 3, 0x81CB35195C827B16),
    (1, 6, 1, 9, 3, 0x81CB35195C827B16),
    (1, 6, 1, 12, 3, 0x81CB35195C827B16),
    (1, 9, 1, 9, 3, 0x81CB35195C827B16),
    (1, 9, 1, 12, 3, 0x81CB35195C827B16),
    (2, 1, 8, 15, 323, 0x2C254788825296A7),
    (2, 2, 8, 15, 323, 0x2C254788825296A7),
    (2, 3, 8, 15, 323, 0x2C254788825296A7),
    (2, 4, 8, 15, 85, 0x3F5C9736FBC00848),
    (2, 5, 8, 15, 85, 0x3F5C9736FBC00848),
    (2, 6, 8, 15, 85, 0x3F5C9736FBC00848),
    (2, 7, 8, 15, 85, 0x3F5C9736FBC00848),
    (2, 8, 8, 15, 85, 0x3F5C9736FBC00848),
    (2, 9, 8, 15, 85, 0x3F5C9736FBC00848),
    (2, 1, 9, 15, 323, 0x2C254788825296A7),
    (2, 2, 9, 15, 323, 0x2C254788825296A7),
    (2, 3, 9, 15, 323, 0x2C254788825296A7),
    (2, 4, 9, 15, 85, 0x3F5C9736FBC00848),
    (2, 5, 9, 15, 85, 0x3F5C9736FBC00848),
    (2, 6, 9, 15, 85, 0x3F5C9736FBC00848),
    (2, 7, 9, 15, 85, 0x3F5C9736FBC00848),
    (2, 8, 9, 15, 85, 0x3F5C9736FBC00848),
    (2, 9, 9, 15, 85, 0x3F5C9736FBC00848),
    (2, 1, 1, 9, 205, 0xAAD9287482340818),
    (2, 1, 1, 12, 350, 0x2105B85F729F5E5F),
    (2, 6, 1, 9, 110, 0xC4B73B2155FBD30A),
    (2, 6, 1, 12, 110, 0xC4B73B2155FBD30A),
    (2, 9, 1, 9, 110, 0xC4B73B2155FBD30A),
    (2, 9, 1, 12, 110, 0xC4B73B2155FBD30A),
    (3, 1, 8, 15, 100035, 0x5011336B7BE6C6CF),
    (3, 2, 8, 15, 100035, 0xA55111B819526D0B),
    (3, 3, 8, 15, 100035, 0xA55111B819526D0B),
    (3, 4, 8, 15, 100035, 0xEA0352781EC1A359),
    (3, 5, 8, 15, 100035, 0xEA0352781EC1A359),
    (3, 6, 8, 15, 100035, 0xEA0352781EC1A359),
    (3, 7, 8, 15, 100035, 0xEA0352781EC1A359),
    (3, 8, 8, 15, 100035, 0xEA0352781EC1A359),
    (3, 9, 8, 15, 100035, 0xEA0352781EC1A359),
    (3, 1, 9, 15, 100020, 0x8CA42BD799F66259),
    (3, 2, 9, 15, 100020, 0x8CA42BD799F66259),
    (3, 3, 9, 15, 100020, 0x8CA42BD799F66259),
    (3, 4, 9, 15, 100020, 0x65D726E1FFA15F39),
    (3, 5, 9, 15, 100020, 0x65D726E1FFA15F39),
    (3, 6, 9, 15, 100020, 0x65D726E1FFA15F39),
    (3, 7, 9, 15, 100020, 0x65D726E1FFA15F39),
    (3, 8, 9, 15, 100020, 0x65D726E1FFA15F39),
    (3, 9, 9, 15, 100020, 0x65D726E1FFA15F39),
    (3, 1, 1, 9, 103938, 0x8B88AE212D7E6419),
    (3, 1, 1, 12, 103938, 0x10D6F3459CECE210),
    (3, 6, 1, 9, 103938, 0x8B88AE212D7E6419),
    (3, 6, 1, 12, 103937, 0xCC8BEE5DCB4D85A9),
    (3, 9, 1, 9, 103938, 0x8B88AE212D7E6419),
    (3, 9, 1, 12, 103937, 0xCC8BEE5DCB4D85A9),
    (4, 1, 8, 15, 44782, 0x10C1C2D002A3C8AB),
    (4, 2, 8, 15, 39463, 0xE8D82902C987F4FC),
    (4, 3, 8, 15, 34226, 0x92A16AD7A869F205),
    (4, 4, 8, 15, 33005, 0xF9866DE182DBBD4A),
    (4, 5, 8, 15, 30635, 0x699D07E8FC63048B),
    (4, 6, 8, 15, 27330, 0x5BE016B0D6DCA1D8),
    (4, 7, 8, 15, 26260, 0x63D9F168BE067895),
    (4, 8, 8, 15, 25926, 0x700CDD97D85961D4),
    (4, 9, 8, 15, 25926, 0x700CDD97D85961D4),
    (4, 1, 9, 15, 44896, 0xA011FFD919F28088),
    (4, 2, 9, 15, 39627, 0x5628B35CFD34D353),
    (4, 3, 9, 15, 34145, 0xF524104D33BB4D09),
    (4, 4, 9, 15, 33240, 0xC41BBF8E3C21ADAF),
    (4, 5, 9, 15, 30841, 0x523B205EAC13EA02),
    (4, 6, 9, 15, 27453, 0xE05B3619C6FB92D8),
    (4, 7, 9, 15, 26352, 0x2EF5828811107F0F),
    (4, 8, 9, 15, 25926, 0x700CDD97D85961D4),
    (4, 9, 9, 15, 25926, 0x700CDD97D85961D4),
    (4, 1, 1, 9, 75878, 0x9986E1F2AEDFA12A),
    (4, 1, 1, 12, 50889, 0xFDCB0CCDD1C7374D),
    (4, 6, 1, 9, 64367, 0xBC38964399798A96),
    (4, 6, 1, 12, 33225, 0x16BB632509CB7B26),
    (4, 9, 1, 9, 64366, 0xA7F59FED832C3A0D),
    (4, 9, 1, 12, 33132, 0xB7AFEE1570FFFF81),
    (5, 1, 8, 15, 512, 0xD8224C8D06436A22),
    (5, 2, 8, 15, 512, 0xD8224C8D06436A22),
    (5, 3, 8, 15, 512, 0xD8224C8D06436A22),
    (5, 4, 8, 15, 418, 0x7C0FBF8691F2999A),
    (5, 5, 8, 15, 418, 0x7C0FBF8691F2999A),
    (5, 6, 8, 15, 418, 0x7C0FBF8691F2999A),
    (5, 7, 8, 15, 418, 0x7C0FBF8691F2999A),
    (5, 8, 8, 15, 418, 0x7C0FBF8691F2999A),
    (5, 9, 8, 15, 418, 0x7C0FBF8691F2999A),
    (5, 1, 9, 15, 512, 0xD8224C8D06436A22),
    (5, 2, 9, 15, 512, 0xD8224C8D06436A22),
    (5, 3, 9, 15, 512, 0xD8224C8D06436A22),
    (5, 4, 9, 15, 418, 0x7C0FBF8691F2999A),
    (5, 5, 9, 15, 418, 0x7C0FBF8691F2999A),
    (5, 6, 9, 15, 418, 0x7C0FBF8691F2999A),
    (5, 7, 9, 15, 418, 0x7C0FBF8691F2999A),
    (5, 8, 9, 15, 418, 0x7C0FBF8691F2999A),
    (5, 9, 9, 15, 418, 0x7C0FBF8691F2999A),
    (5, 1, 1, 9, 10702, 0x6D247F60C681D94E),
    (5, 1, 1, 12, 938, 0x40A34BF7406195AD),
    (5, 6, 1, 9, 10610, 0xE5CD8313D88C2784),
    (5, 6, 1, 12, 364, 0x78A8AE88F68E73F6),
    (5, 9, 1, 9, 10610, 0xE5CD8313D88C2784),
    (5, 9, 1, 12, 364, 0x78A8AE88F68E73F6),
    (6, 1, 8, 15, 94840, 0xCFFC24EB33341EA0),
    (6, 2, 8, 15, 94760, 0x7B93746B2F6AF6C3),
    (6, 3, 8, 15, 94755, 0xE57599B3B079ABA3),
    (6, 4, 8, 15, 91745, 0x36CE8F7F047A9C59),
    (6, 5, 8, 15, 91742, 0x9423BE7A2D4AC89B),
    (6, 6, 8, 15, 91742, 0x9423BE7A2D4AC89B),
    (6, 7, 8, 15, 91742, 0x9423BE7A2D4AC89B),
    (6, 8, 8, 15, 91742, 0x9423BE7A2D4AC89B),
    (6, 9, 8, 15, 91742, 0x9423BE7A2D4AC89B),
    (6, 1, 9, 15, 94953, 0x8393876DB1013AE7),
    (6, 2, 9, 15, 94902, 0xB5679AA6A0E5649C),
    (6, 3, 9, 15, 94901, 0x317C30B1BCF9AF84),
    (6, 4, 9, 15, 91681, 0x9FBDA4567D99703F),
    (6, 5, 9, 15, 91677, 0xA989AB78EE8BE80D),
    (6, 6, 9, 15, 91677, 0xA989AB78EE8BE80D),
    (6, 7, 9, 15, 91677, 0xA989AB78EE8BE80D),
    (6, 8, 9, 15, 91677, 0xA989AB78EE8BE80D),
    (6, 9, 9, 15, 91677, 0xA989AB78EE8BE80D),
    (6, 1, 1, 9, 117056, 0xC91C5B8DEC9206E7),
    (6, 1, 1, 12, 117531, 0x077EB3961B582045),
    (6, 6, 1, 9, 117056, 0x40140450E833AA6B),
    (6, 6, 1, 12, 116076, 0xCD9AA57D06FABBF2),
    (6, 9, 1, 9, 117056, 0x40140450E833AA6B),
    (6, 9, 1, 12, 116076, 0xCD9AA57D06FABBF2),
    (7, 1, 8, 15, 30036, 0xB08CD92E324D1859),
    (7, 2, 8, 15, 30036, 0xB08CD92E324D1859),
    (7, 3, 8, 15, 30036, 0xB08CD92E324D1859),
    (7, 4, 8, 15, 29260, 0xEDE7DBDCBEDAD8AD),
    (7, 5, 8, 15, 29260, 0xEDE7DBDCBEDAD8AD),
    (7, 6, 8, 15, 29260, 0xEDE7DBDCBEDAD8AD),
    (7, 7, 8, 15, 29260, 0xEDE7DBDCBEDAD8AD),
    (7, 8, 8, 15, 29255, 0x54AA657A099A9892),
    (7, 9, 8, 15, 29255, 0x54AA657A099A9892),
    (7, 1, 9, 15, 30442, 0x925DC81DD52A34E6),
    (7, 2, 9, 15, 30442, 0x925DC81DD52A34E6),
    (7, 3, 9, 15, 30442, 0x925DC81DD52A34E6),
    (7, 4, 9, 15, 29785, 0x1A894D1AD5CFF488),
    (7, 5, 9, 15, 29785, 0x1A894D1AD5CFF488),
    (7, 6, 9, 15, 29785, 0x1A894D1AD5CFF488),
    (7, 7, 9, 15, 29785, 0x1A894D1AD5CFF488),
    (7, 8, 9, 15, 29785, 0xC1704D396B288078),
    (7, 9, 9, 15, 29785, 0xC1704D396B288078),
    (7, 1, 1, 9, 59755, 0x637498E4BB69B4C1),
    (7, 1, 1, 12, 59821, 0x959074C12907DE2B),
    (7, 6, 1, 9, 59755, 0x637498E4BB69B4C1),
    (7, 6, 1, 12, 59837, 0x750F53A03AF66479),
    (7, 9, 1, 9, 59755, 0x637498E4BB69B4C1),
    (7, 9, 1, 12, 59837, 0x750F53A03AF66479),
];

#[test]
fn deflate_matches_zlib_byte_for_byte() {
    let inputs = inputs();
    for &(i, level, mem_level, window_bits, len, digest) in ZLIB {
        let ours = deflate(&inputs[i], Params { level, mem_level, window_bits });
        assert_eq!(
            (ours.len(), fnv(&ours)),
            (len, digest),
            "input {i} at level {level}, memory level {mem_level}, window {window_bits}"
        );
    }
}

#[test]
fn the_digest_table_tells_levels_and_memory_levels_apart() {
    let text = |level: u8, mem: u8| ZLIB.iter().find(|r| r.0 == 4 && r.1 == level && r.2 == mem && r.3 == 15).map(|r| r.5);
    assert_ne!(text(1, 8), text(9, 8));
    assert_ne!(text(6, 8), text(6, 9));
    assert_eq!(ZLIB.len(), inputs().len() * 24, "every input under every parameter set");
}

#[test]
fn inflate_reverses_deflate_for_every_input_and_parameter_set() {
    for (i, data) in inputs().iter().enumerate() {
        for &(j, level, mem_level, window_bits, _, _) in ZLIB.iter().filter(|r| r.0 == i) {
            let z = deflate(data, Params { level, mem_level, window_bits });
            let back = inflate(&z, data.len()).unwrap_or_else(|| panic!("input {j} level {level} inflates"));
            assert_eq!(back.data, *data);
            assert_eq!(back.consumed, z.len());
        }
    }
}

#[test]
fn inflate_reports_where_the_stream_ends_when_bytes_follow_it() {
    let data = wordy_text(5000, 9);
    let mut z = deflate(&data, Params { level: 6, mem_level: 8, window_bits: 15 });
    let end = z.len();
    z.extend_from_slice(b"PK trailing bytes that are not part of the stream");
    let back = inflate(&z, data.len()).expect("a stream with bytes after it");
    assert_eq!((back.data, back.consumed), (data, end));
}

#[test]
fn inflate_refuses_truncated_illegal_and_oversized_streams() {
    let data = wordy_text(20_000, 11);
    let z = deflate(&data, Params { level: 9, mem_level: 8, window_bits: 15 });
    for cut in [1, 2, z.len() / 2, z.len() - 1] {
        assert!(inflate(&z[..cut], data.len()).is_none(), "cut at {cut}");
    }
    assert!(inflate(&[0b111], 100).is_none(), "block type 3 does not exist");
    // A fixed block whose first symbol is a match: distance 1 with nothing decoded yet.
    assert!(inflate(&[0x03, 0x02, 0x00], 100).is_none(), "a match reaching before the stream");
    assert!(inflate(&z, data.len() - 1).is_none(), "one byte over the output budget");
    // A stored block whose length check disagrees.
    assert!(inflate(&[0x01, 0x05, 0x00, 0x00, 0x00, 1, 2, 3, 4, 5], 100).is_none());
    assert_eq!(inflate(&[0x01, 0x05, 0x00, 0xFA, 0xFF, 1, 2, 3, 4, 5], 100).map(|i| i.data), Some(vec![1, 2, 3, 4, 5]));
}

#[test]
fn every_level_compresses_text_and_none_expands_random_bytes_past_stored_framing() {
    let text = wordy_text(100_000, 13);
    let mut r = Mix(17);
    let random: Vec<u8> = (0..50_000).map(|_| r.next() as u8).collect();
    for level in 1..=9 {
        let p = Params { level, mem_level: 8, window_bits: 15 };
        assert!(deflate(&text, p).len() * 3 < text.len(), "level {level} on text");
        // Stored blocks cost 5 bytes per block of at most 64 KiB, and a block flushes every 16 383 symbols.
        assert!(deflate(&random, p).len() <= random.len() + 5 * (random.len() / 16_383 + 1), "level {level} on random bytes");
    }
}

/// (input, level, gzip's length, FNV-1a 64 of gzip's DEFLATE bytes), from gzip 1.12 (`gzip -n -<level>`).
const GZIP: &[(usize, u8, usize, u64)] = &[
    (0, 1, 2, 0x0835EE07B4EE5316),
    (0, 2, 2, 0x0835EE07B4EE5316),
    (0, 3, 2, 0x0835EE07B4EE5316),
    (0, 4, 2, 0x0835EE07B4EE5316),
    (0, 5, 2, 0x0835EE07B4EE5316),
    (0, 6, 2, 0x0835EE07B4EE5316),
    (0, 7, 2, 0x0835EE07B4EE5316),
    (0, 8, 2, 0x0835EE07B4EE5316),
    (0, 9, 2, 0x0835EE07B4EE5316),
    (1, 1, 3, 0x81CB35195C827B16),
    (1, 2, 3, 0x81CB35195C827B16),
    (1, 3, 3, 0x81CB35195C827B16),
    (1, 4, 3, 0x81CB35195C827B16),
    (1, 5, 3, 0x81CB35195C827B16),
    (1, 6, 3, 0x81CB35195C827B16),
    (1, 7, 3, 0x81CB35195C827B16),
    (1, 8, 3, 0x81CB35195C827B16),
    (1, 9, 3, 0x81CB35195C827B16),
    (2, 1, 323, 0x2C254788825296A7),
    (2, 2, 323, 0x2C254788825296A7),
    (2, 3, 323, 0x2C254788825296A7),
    (2, 4, 85, 0x3F5C9736FBC00848),
    (2, 5, 85, 0x3F5C9736FBC00848),
    (2, 6, 85, 0x3F5C9736FBC00848),
    (2, 7, 85, 0x3F5C9736FBC00848),
    (2, 8, 85, 0x3F5C9736FBC00848),
    (2, 9, 85, 0x3F5C9736FBC00848),
    (3, 1, 100020, 0xCBBB44890CA66D83),
    (3, 2, 100020, 0x8CA42BD799F66259),
    (3, 3, 100020, 0x8CA42BD799F66259),
    (3, 4, 100020, 0x65D726E1FFA15F39),
    (3, 5, 100020, 0x65D726E1FFA15F39),
    (3, 6, 100020, 0x65D726E1FFA15F39),
    (3, 7, 100020, 0x65D726E1FFA15F39),
    (3, 8, 100020, 0x65D726E1FFA15F39),
    (3, 9, 100020, 0x65D726E1FFA15F39),
    (4, 1, 44761, 0x5FBE3761C2A184C0),
    (4, 2, 39440, 0x02A09F7A315105DE),
    (4, 3, 34226, 0x92A16AD7A869F205),
    (4, 4, 33005, 0xF9866DE182DBBD4A),
    (4, 5, 30635, 0x699D07E8FC63048B),
    (4, 6, 27330, 0x5BE016B0D6DCA1D8),
    (4, 7, 26260, 0x63D9F168BE067895),
    (4, 8, 25926, 0x700CDD97D85961D4),
    (4, 9, 25926, 0x700CDD97D85961D4),
    (5, 1, 512, 0xD8224C8D06436A22),
    (5, 2, 512, 0xD8224C8D06436A22),
    (5, 3, 512, 0xD8224C8D06436A22),
    (5, 4, 418, 0x7C0FBF8691F2999A),
    (5, 5, 418, 0x7C0FBF8691F2999A),
    (5, 6, 418, 0x7C0FBF8691F2999A),
    (5, 7, 418, 0x7C0FBF8691F2999A),
    (5, 8, 418, 0x7C0FBF8691F2999A),
    (5, 9, 418, 0x7C0FBF8691F2999A),
    (6, 1, 94955, 0x12B20F6C5222ACCA),
    (6, 2, 94905, 0x514F645D1296AE08),
    (6, 3, 94901, 0x317C30B1BCF9AF84),
    (6, 4, 91681, 0xC19ED8F8EA7C508E),
    (6, 5, 91677, 0xA989AB78EE8BE80D),
    (6, 6, 91677, 0xA989AB78EE8BE80D),
    (6, 7, 91677, 0xA989AB78EE8BE80D),
    (6, 8, 91677, 0xA989AB78EE8BE80D),
    (6, 9, 91677, 0xA989AB78EE8BE80D),
    (7, 1, 30442, 0x064838B82E7B3EA8),
    (7, 2, 30442, 0x064838B82E7B3EA8),
    (7, 3, 30442, 0x064838B82E7B3EA8),
    (7, 4, 29785, 0x1A894D1AD5CFF488),
    (7, 5, 29785, 0x1A894D1AD5CFF488),
    (7, 6, 29785, 0x1A894D1AD5CFF488),
    (7, 7, 29785, 0x1A894D1AD5CFF488),
    (7, 8, 29785, 0xC1704D396B288078),
    (7, 9, 29785, 0xC1704D396B288078),
];

#[test]
fn gzip_rules_match_gzip_byte_for_byte() {
    let inputs = inputs();
    for &(i, level, len, digest) in GZIP {
        let ours = gzip(&inputs[i], level);
        assert_eq!((ours.len(), fnv(&ours)), (len, digest), "input {i} at gzip level {level}");
        assert_eq!(inflate(&ours, inputs[i].len()).map(|b| b.data), Some(inputs[i].clone()), "input {i} level {level} inflates");
    }
    assert_eq!(GZIP.len(), inputs.len() * 9, "every input at every level");
    let differing = GZIP
        .iter()
        .filter(|&&(i, level, len, digest)| ZLIB.iter().any(|z| (z.0, z.1, z.2, z.3) == (i, level, 8, 15) && (z.4, z.5) != (len, digest)))
        .count();
    assert!(differing >= 20, "{differing} of the vectors differ from zlib's at the same level: the rules are exercised");
}

/// gzip 1.12's DEFLATE bytes of the few-but-long input at levels 3 to 9 (`gzip -n -<level>`): length, FNV-1a 64.
const LONG_FEW: &[(u8, usize, u64)] = &[
    (3, 57078, 0xAE79_681F_FC14_9B18),
    (4, 50959, 0x5F3D_408B_6F4D_5AAC),
    (5, 49223, 0xF8F6_CBD9_5B5F_BFDE),
    (6, 48029, 0xE1BF_DEB9_D051_9BD7),
    (7, 47949, 0x3C82_88A9_E949_1867),
    (8, 47937, 0x6CF8_E011_E19F_B002),
    (9, 47937, 0x6CF8_E011_E19F_B002),
];

#[test]
fn gzip_ends_a_block_early_where_its_matches_are_few_but_long() {
    // Five random bytes, then twenty copied from earlier: fewer matches than literals, yet a stretch of them codes in
    // under half the bytes it covers, so above level 2 gzip ends its blocks early there.
    let mut r = Mix(8);
    let mut data: Vec<u8> = Vec::new();
    while data.len() < 120_000 {
        for _ in 0..5 {
            data.push(r.next() as u8);
        }
        if data.len() > 40 {
            let from = (r.next() % (data.len() as u64 - 30)) as usize;
            data.extend_from_within(from..from + 20);
        }
    }
    data.truncate(120_000);
    for &(level, len, digest) in LONG_FEW {
        let ours = gzip(&data, level);
        assert_eq!((ours.len(), fnv(&ours)), (len, digest), "level {level}");
        assert_eq!(inflate(&ours, data.len()).map(|b| b.data), Some(data.clone()), "level {level} inflates");
    }
}
