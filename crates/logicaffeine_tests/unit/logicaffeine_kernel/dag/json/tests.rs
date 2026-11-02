use super::*;

#[test]
fn nested_objects_arrays_and_escapes() {
    let line: &[u8] = b"{\"app\":{\"arg\":6,\"fn\":5},\"ie\":7,\"s\":\"a\\\"b\\u00e9\\ud83d\\ude00\",\"xs\":[1,[2,3],{}],\"t\":true}";
    let tape = Tape::parse(line, Vec::new()).map_err(|_| ()).unwrap();
    let app = tape.get(0, "app").unwrap();
    assert_eq!(tape.u64(tape.get(app, "fn").unwrap()), Some(5));
    assert_eq!(tape.u64(tape.get(0, "ie").unwrap()), Some(7));
    assert_eq!(tape.str(tape.get(0, "s").unwrap()).unwrap(), "a\"bé😀");
    assert_eq!(tape.elems(tape.get(0, "xs").unwrap()).count(), 3);
    assert_eq!(tape.bool(tape.get(0, "t").unwrap()), Some(true));
}

#[test]
fn integers_are_exact_or_absent() {
    let line: &[u8] = b"[0,18446744073709551615,18446744073709551616,-1,1.5,1e3,007]";
    let tape = Tape::parse(line, Vec::new()).map_err(|_| ()).unwrap();
    let got: Vec<Option<u64>> = tape.elems(0).map(|i| tape.u64(i)).collect();
    assert_eq!(
        got,
        [Some(0), Some(u64::MAX), None, None, None, None, Some(7)]
    );
}

#[test]
fn malformed_records_are_errors() {
    for bad in [
        &b"{"[..],
        b"{\"a\":}",
        b"{\"a\":1,}",
        b"[1 2]",
        b"{\"a\":1} x",
        b"\"\x01\"",
    ] {
        assert!(
            Tape::parse(bad, Vec::new()).is_err(),
            "{:?}",
            std::str::from_utf8(bad)
        );
    }
}
