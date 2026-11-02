use super::*;

#[test]
fn nil_and_max_render_canonically() {
    assert_eq!(
        Uuid::NIL.to_string(),
        "00000000-0000-0000-0000-000000000000"
    );
    assert_eq!(
        Uuid::MAX.to_string(),
        "ffffffff-ffff-ffff-ffff-ffffffffffff"
    );
    assert!(Uuid::NIL.is_nil());
    assert!(Uuid::MAX.is_max());
    assert_eq!(Uuid::NIL.version(), 0);
    assert_eq!(Uuid::MAX.version(), 0xF);
}

#[test]
fn parse_round_trips_every_accepted_form() {
    let canonical = "550e8400-e29b-41d4-a716-446655440000";
    let u = Uuid::parse(canonical).unwrap();
    assert_eq!(u.to_string(), canonical);
    // Simple, braced, urn, uppercase — all parse to the same value.
    assert_eq!(Uuid::parse("550e8400e29b41d4a716446655440000").unwrap(), u);
    assert_eq!(
        Uuid::parse("{550e8400-e29b-41d4-a716-446655440000}").unwrap(),
        u
    );
    assert_eq!(
        Uuid::parse("urn:uuid:550e8400-e29b-41d4-a716-446655440000").unwrap(),
        u
    );
    assert_eq!(
        Uuid::parse("550E8400-E29B-41D4-A716-446655440000").unwrap(),
        u
    );
    assert_eq!(
        Uuid::parse("  550e8400-e29b-41d4-a716-446655440000  ").unwrap(),
        u
    );
}

#[test]
fn parse_rejects_malformed_input() {
    assert_eq!(Uuid::parse(""), None);
    assert_eq!(Uuid::parse("not-a-uuid"), None);
    assert_eq!(Uuid::parse("550e8400-e29b-41d4-a716-44665544000"), None); // too short
    assert_eq!(Uuid::parse("550e8400-e29b-41d4-a716-4466554400000"), None); // too long
    assert_eq!(Uuid::parse("550e8400xe29b-41d4-a716-446655440000"), None); // hyphen slot wrong
    assert_eq!(Uuid::parse("ZZZe8400-e29b-41d4-a716-446655440000"), None); // non-hex
}

#[test]
fn parse_validates_every_hex_position_and_case() {
    // The SIMD path must accept both cases and the extremes, byte-identical to the scalar table.
    assert_eq!(
        Uuid::parse("ffffffff-ffff-ffff-ffff-ffffffffffff").unwrap(),
        Uuid::MAX
    );
    assert_eq!(
        Uuid::parse("FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF").unwrap(),
        Uuid::MAX
    );
    assert_eq!(
        Uuid::parse("00000000-0000-0000-0000-000000000000").unwrap(),
        Uuid::NIL
    );
    let base = "550e8400-e29b-41d4-a716-446655440000";
    assert_eq!(
        Uuid::parse("550E8400-e29b-41D4-A716-446655440000").unwrap(),
        Uuid::parse(base).unwrap(),
        "mixed-case canonical must equal lowercase",
    );
    // Corrupt EVERY hex slot in turn with each boundary char adjacent to a valid range — the
    // parallel range check must reject all of them at all 32 positions (catches an off-by-one in
    // the SIMD validation, and the maddubs/pshufb lane layout via the value cases below).
    for i in 0..36 {
        if matches!(i, 8 | 13 | 18 | 23) {
            continue; // hyphen slots handled separately
        }
        for c in [b'g', b'G', b':', b'/', b'@', b'`', b' ', 0x00u8, 0xff] {
            let mut bad = base.as_bytes().to_vec();
            bad[i] = c;
            if let Ok(s) = std::str::from_utf8(&bad) {
                assert_eq!(
                    Uuid::parse(s),
                    None,
                    "must reject byte {c:#x} at position {i}"
                );
            }
        }
    }
}

#[test]
fn parse_many_round_trips_a_packed_column() {
    // Build a packed column of ids, format them, and bulk-parse the buffer back — must equal.
    let mut ids = Vec::new();
    let mut st = 0x1234_5678_9abc_def0u64;
    for _ in 0..257 {
        let mut b = [0u8; 16];
        for x in b.iter_mut() {
            st = st.wrapping_mul(6364136223846793005).wrapping_add(1);
            *x = (st >> 56) as u8;
        }
        ids.push(Uuid::from_bytes(b));
    }
    let packed = encode_many(&ids);
    assert_eq!(Uuid::parse_many(&packed).unwrap(), ids);
    // Malformed inputs are all rejected: empty and a non-multiple length.
    assert_eq!(Uuid::parse_many(&[]), None);
    assert_eq!(Uuid::parse_many(&packed[..packed.len() - 1]), None);
    // A single bad byte anywhere in ANY record must sink the whole batch — the batch-validated
    // fast path accumulates three independent validity channels (the SIMD hex-lane mask, the
    // scalar tail digits, and the four hyphens), so corruption is probed in each channel, at the
    // first record, an interior record, and the very last record. Positions 8/13/18/23 are the
    // hyphens; 32–35 are the scalar tail; everything else is a SIMD-lane hex digit.
    let n = ids.len();
    for &rec in &[0usize, 3, n - 1] {
        for &pos in &[0usize, 5, 8, 12, 18, 23, 30, 32, 35] {
            let mut bad = packed.clone();
            let byte = 36 * rec + pos;
            bad[byte] = if pos == 8 || pos == 13 || pos == 18 || pos == 23 {
                b'f' // a hyphen slot filled with a hex digit — dash channel must still reject
            } else {
                b'z' // a hex slot filled with a non-hex byte — hex/tail channel must reject
            };
            assert_eq!(
                Uuid::parse_many(&bad),
                None,
                "record {rec} byte {pos} corruption not rejected"
            );
        }
    }
}

#[test]
fn name_based_versions_match_the_rfc_worked_examples() {
    // RFC-published name-based ids under the DNS namespace.
    let v3 = Uuid::new_v3(Uuid::NAMESPACE_DNS, b"www.example.com");
    assert_eq!(v3.version(), 3);
    assert_eq!(v3.variant(), Variant::Rfc4122);
    assert_eq!(v3.to_string(), "5df41881-3aed-3515-88a7-2f4a814cf09e");

    let v5 = Uuid::new_v5(Uuid::NAMESPACE_DNS, b"www.example.com");
    assert_eq!(v5.version(), 5);
    assert_eq!(v5.variant(), Variant::Rfc4122);
    assert_eq!(v5.to_string(), "2ed6657d-e927-568b-95e1-2665a8aea6a2");
}

#[test]
fn version_and_variant_bits_are_correct_for_every_generated_version() {
    let node = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06];
    let cases: [(Uuid, u8); 7] = [
        (Uuid::new_v1(0x1234_5678_9ABC, 0x0102, node), 1),
        (Uuid::new_v3(Uuid::NAMESPACE_URL, b"a"), 3),
        (Uuid::new_v4([0xAB; 16]), 4),
        (Uuid::new_v5(Uuid::NAMESPACE_URL, b"a"), 5),
        (Uuid::new_v6(0x1234_5678_9ABC, 0x0102, node), 6),
        (Uuid::new_v7(0x0190_0000_0000, [0x11; 10]), 7),
        (Uuid::new_v8([0x77; 16]), 8),
    ];
    for (u, want) in cases {
        assert_eq!(u.version(), want, "version nibble for v{want}");
        assert_eq!(u.variant(), Variant::Rfc4122, "variant for v{want}");
        // Round-trips through canonical text.
        assert_eq!(
            Uuid::parse(&u.to_string()).unwrap(),
            u,
            "round trip v{want}"
        );
    }
}

#[test]
fn v7_is_time_ordered_by_bytes() {
    // Two v7 ids one millisecond apart sort in time order regardless of the random tail.
    let earlier = Uuid::new_v7(1000, [0xFF; 10]);
    let later = Uuid::new_v7(1001, [0x00; 10]);
    assert!(
        earlier < later,
        "v7 must sort by timestamp: {earlier} !< {later}"
    );
}

#[test]
fn differential_against_the_uuid_crate() {
    // Name-based (deterministic) versions, byte-for-byte against the reference crate.
    for name in ["", "a", "www.example.com", "the quick brown fox", "🦀"] {
        let ours = Uuid::new_v3(Uuid::NAMESPACE_DNS, name.as_bytes());
        let theirs = uuid::Uuid::new_v3(&uuid::Uuid::NAMESPACE_DNS, name.as_bytes());
        assert_eq!(
            ours.to_bytes(),
            *theirs.as_bytes(),
            "v3 differs for {name:?}"
        );

        let ours = Uuid::new_v5(Uuid::NAMESPACE_URL, name.as_bytes());
        let theirs = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, name.as_bytes());
        assert_eq!(
            ours.to_bytes(),
            *theirs.as_bytes(),
            "v5 differs for {name:?}"
        );
    }

    // v4 / v7 from identical entropy/time must equal the crate's builder output.
    let rand16 = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ];
    let ours = Uuid::new_v4(rand16);
    let theirs = uuid::Builder::from_random_bytes(rand16).into_uuid();
    assert_eq!(ours.to_bytes(), *theirs.as_bytes(), "v4 differs");

    let rand10 = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xfe, 0xdc];
    let ms: u64 = 0x0190_2233_4455;
    let ours = Uuid::new_v7(ms, rand10);
    let theirs = uuid::Builder::from_unix_timestamp_millis(ms, &rand10).into_uuid();
    assert_eq!(ours.to_bytes(), *theirs.as_bytes(), "v7 differs");

    // Parse + display agree with the crate over random ids.
    let mut state: u64 = 0xDEAD_BEEF_CAFE_F00D;
    for _ in 0..2000 {
        let mut bytes = [0u8; 16];
        for byte in bytes.iter_mut() {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *byte = (state >> 56) as u8;
        }
        let ours = Uuid::from_bytes(bytes);
        let theirs = uuid::Uuid::from_bytes(bytes);
        assert_eq!(ours.to_string(), theirs.to_string(), "display differs");
        assert_eq!(
            Uuid::parse(&theirs.to_string()).unwrap(),
            ours,
            "parse differs"
        );
        assert_eq!(
            ours.version(),
            theirs.get_version_num() as u8,
            "version differs"
        );
    }
}
