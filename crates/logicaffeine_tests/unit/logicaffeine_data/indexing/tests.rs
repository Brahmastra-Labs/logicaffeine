use super::*;

#[test]
fn vec_1_based_indexing() {
    let v = vec![10, 20, 30];
    assert_eq!(LogosIndex::logos_get(&v, 1i64), 10);
    assert_eq!(LogosIndex::logos_get(&v, 2i64), 20);
    assert_eq!(LogosIndex::logos_get(&v, 3i64), 30);
}

#[test]
#[should_panic(expected = "Index 0 out of bounds")]
fn vec_zero_index_panics() {
    let v = vec![10, 20, 30];
    let _ = LogosIndex::logos_get(&v, 0i64);
}

#[test]
fn vec_set_1_based() {
    let mut v = vec![10, 20, 30];
    LogosIndexMut::logos_set(&mut v, 2i64, 99);
    assert_eq!(v, vec![10, 99, 30]);
}

#[test]
fn hashmap_string_key() {
    let mut m: FxHashMap<String, i64> = FxHashMap::default();
    m.insert("iron".to_string(), 42);
    assert_eq!(LogosIndex::logos_get(&m, "iron".to_string()), 42);
}

#[test]
fn hashmap_str_key() {
    let mut m: FxHashMap<String, i64> = FxHashMap::default();
    m.insert("iron".to_string(), 42);
    assert_eq!(LogosIndex::logos_get(&m, "iron"), 42);
}

#[test]
fn hashmap_set_key() {
    let mut m: FxHashMap<String, i64> = FxHashMap::default();
    LogosIndexMut::logos_set(&mut m, "iron", 42i64);
    assert_eq!(m.get("iron"), Some(&42));
}

/// Every index from well below the front to well past the end, on lengths 0 through 5: the one rule, spelled
/// out independently of `logos_offset`'s arithmetic.
#[test]
fn logos_offset_is_the_one_index_rule() {
    for len in 0usize..=5 {
        for index in -8i64..=8 {
            let want = if index >= 1 && index as usize <= len {
                Some(index as usize - 1)
            } else if index <= -1 && index.unsigned_abs() as usize <= len {
                Some(len - index.unsigned_abs() as usize)
            } else {
                None
            };
            assert_eq!(
                logos_offset(index, len),
                want,
                "index {index} of {len} items"
            );
        }
    }
    for index in [
        i64::MIN,
        i64::MIN + 1,
        i64::MAX,
        i64::MAX - 1,
        1 << 32,
        (1 << 32) + 1,
        -(1 << 32),
    ] {
        assert_eq!(logos_offset(index, 3), None, "index {index} of 3 items");
    }
}

#[test]
#[should_panic(expected = "Index -4 out of bounds")]
fn resolve_logos_index_names_a_negative_index_past_the_front() {
    let _ = resolve_logos_index(-4, 3);
}

#[test]
#[should_panic(expected = "Index 4294967297 out of bounds")]
fn an_index_past_32_bits_names_no_item_on_any_target() {
    let _ = resolve_logos_index((1 << 32) + 1, 3);
}

#[test]
fn logos_item_reads_and_writes_the_item_an_index_names() {
    let mut v = vec![10, 20, 30];
    assert_eq!(*v.logos_item(1), 10);
    assert_eq!(*v.logos_item(-1), 30);
    assert_eq!(*v.logos_item(-3), 10);
    *v.logos_item_mut(-2) = 99;
    assert_eq!(v, vec![10, 99, 30]);
}

#[test]
#[should_panic(expected = "Index 4 out of bounds")]
fn logos_item_past_the_end_fails_in_the_one_sentence() {
    let v = vec![10, 20, 30];
    let _ = v.logos_item(4);
}

/// `items start through end`: the range inside the sequence, or nothing.
#[test]
fn logos_slice_range_holds_nothing_outside_the_sequence() {
    let cases: &[(i64, i64, usize, core::ops::Range<usize>)] = &[
        (2, 3, 4, 1..3),
        (1, 4, 4, 0..4),
        (2, 7, 4, 0..0),
        (0, 2, 4, 0..2),
        (-1, 2, 4, 0..0),
        (3, 2, 4, 0..0),
        (3, 3, 4, 2..3),
        (1, 0, 4, 0..0),
        (1, 1, 0, 0..0),
        ((1 << 32) + 1, (1 << 32) + 2, 4, 0..0),
    ];
    for (start, end, len, want) in cases {
        assert_eq!(
            logos_slice_range(*start, *end, *len),
            *want,
            "items {start} through {end} of {len}"
        );
    }
    assert_eq!([1, 2, 3, 4].logos_slice(2, 3), &[2, 3]);
    assert!([1, 2, 3, 4].logos_slice(2, 7).is_empty());
}

/// What a counted loop reading items `first ..= last` one at a time does: the offsets it reads, or the
/// sentence it stops with at the first index that names no item.
fn loop_reads(first: i64, last: i64, len: usize) -> Result<Vec<usize>, String> {
    let mut read = Vec::new();
    let mut i = first;
    while i <= last {
        match logos_offset(i, len) {
            Some(offset) => read.push(offset),
            None => return Err(crate::failure::index_out_of_bounds(i)),
        }
        i += 1;
    }
    Ok(read)
}

fn panic_text(
    result: std::thread::Result<core::ops::Range<usize>>,
) -> Result<core::ops::Range<usize>, String> {
    result.map_err(|panic| {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    })
}

/// A loop copied in one step reads what the loop reads and stops where the loop stops, for every run on
/// every short sequence.
#[test]
fn logos_run_through_is_the_loop_it_replaces() {
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for len in 0usize..=5 {
        for first in -8i64..=8 {
            for last in -8i64..=8 {
                let run = panic_text(std::panic::catch_unwind(|| {
                    logos_run_through(first, last, len)
                }));
                match loop_reads(first, last, len) {
                    Ok(read) => {
                        let run = run.unwrap_or_else(|why| {
                            panic!("{first} through {last} of {len}: stopped with {why}")
                        });
                        assert_eq!(
                            run.clone().collect::<Vec<_>>(),
                            read,
                            "{first} through {last} of {len}"
                        );
                    }
                    // What a stopped loop read before it stopped is not observable, so only the stop is compared.
                    Err(why) => assert_eq!(run, Err(why), "{first} through {last} of {len}"),
                }
                let before = panic_text(std::panic::catch_unwind(|| {
                    logos_run_before(first, last + 1, len)
                }));
                assert_eq!(
                    before,
                    panic_text(std::panic::catch_unwind(|| logos_run_through(
                        first, last, len
                    ))),
                    "{first} before {} of {len}",
                    last + 1
                );
            }
        }
    }
    assert_eq!(logos_run_before(3, i64::MIN, 5), 0..0);
    std::panic::set_hook(quiet);
}

#[test]
fn a_map_entry_reads_its_key_and_its_value_from_either_end() {
    let entry = ("iron".to_string(), 26i64);
    assert_eq!(LogosIndex::logos_get(&entry, 1i64), crate::types::Value::from("iron".to_string()));
    assert_eq!(LogosIndex::logos_get(&entry, 2i64), crate::types::Value::from(26i64));
    assert_eq!(LogosIndex::logos_get(&entry, -1i64), crate::types::Value::from(26i64));
    assert_eq!(LogosIndex::logos_get(&entry, -2i64), crate::types::Value::from("iron".to_string()));
}

#[test]
#[should_panic(expected = "Index 3 out of bounds")]
fn an_index_past_a_map_entry_fails_as_any_index_past_the_end() {
    let entry = (3i64, 9i64);
    let _ = LogosIndex::logos_get(&entry, 3i64);
}
