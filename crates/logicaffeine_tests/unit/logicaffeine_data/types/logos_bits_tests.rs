use super::LogosBits;

/// A `Vec<bool>` and a `LogosBits` driven through the same pushes and stores hold the same items.
fn agree(bits: &LogosBits, model: &[bool]) {
    assert_eq!(bits.len(), model.len());
    for (k, &want) in model.iter().enumerate() {
        assert_eq!(bits.item(k as i64 + 1), want, "item {}", k + 1);
        assert_eq!(unsafe { bits.get_unchecked(k) }, want, "offset {k}");
    }
    assert_eq!(bits.iter().collect::<Vec<_>>(), model);
}

#[test]
fn pushes_keep_every_item_across_word_boundaries() {
    let mut bits = LogosBits::new();
    let mut model = Vec::new();
    for k in 0..200 {
        let v = k % 3 == 0 || k == 63 || k == 64;
        bits.push(v);
        model.push(v);
    }
    agree(&bits, &model);
}

#[test]
fn filled_holds_its_value_and_no_stray_bits() {
    for len in [0, 1, 63, 64, 65, 127, 128, 129, 1000] {
        for value in [false, true] {
            let bits = LogosBits::filled(len, value);
            agree(&bits, &vec![value; len]);
            let mut grown = bits.clone();
            grown.push(!value);
            let mut model = vec![value; len];
            model.push(!value);
            agree(&grown, &model);
        }
    }
}

#[test]
fn stores_change_only_their_item() {
    let mut bits = LogosBits::filled(300, false);
    let mut model = vec![false; 300];
    for (k, v) in [
        (0, true),
        (63, true),
        (64, true),
        (299, true),
        (64, false),
        (150, true),
        (150, true),
    ] {
        bits.set_item(k as i64 + 1, v);
        model[k] = v;
        agree(&bits, &model);
    }
    unsafe { bits.set_true_unchecked(5) };
    model[5] = true;
    unsafe { bits.set_unchecked(299, false) };
    model[299] = false;
    agree(&bits, &model);
}

#[test]
fn an_index_from_the_back_names_the_same_item() {
    let mut bits = LogosBits::new();
    for v in [true, false, false, true] {
        bits.push(v);
    }
    assert!(bits.item(-1));
    assert!(!bits.item(-2));
    assert!(bits.item(-4));
    bits.set_item(-2, true);
    assert!(bits.item(3));
}

#[test]
fn an_index_that_names_no_item_fails_with_the_shared_sentence() {
    let bits = LogosBits::filled(4, false);
    for index in [0, 5, -5, i64::MAX, i64::MIN] {
        let got = std::panic::catch_unwind(|| bits.item(index)).expect_err("no item");
        let message = got.downcast_ref::<String>().cloned().unwrap_or_default();
        assert_eq!(
            message,
            crate::failure::index_out_of_bounds(index),
            "index {index}"
        );
    }
}

#[test]
fn equality_is_by_items() {
    let mut a = LogosBits::filled(70, true);
    let mut b = LogosBits::new();
    for _ in 0..70 {
        b.push(true);
    }
    assert_eq!(a, b);
    a.set_item(70, false);
    assert_ne!(a, b);
    assert_ne!(LogosBits::filled(3, false), LogosBits::filled(4, false));
}

/// Bits that are neither all set nor all clear, so a fill that touched the wrong item shows.
fn mixed(len: usize) -> Vec<bool> {
    (0..len).map(|k| (k * 7 + k / 5) % 3 == 0).collect()
}

fn from_model(model: &[bool]) -> LogosBits {
    let mut bits = LogosBits::new();
    for &v in model {
        bits.push(v);
    }
    bits
}

/// A strided fill stores its value at `first`, `first + step`, … (`count` items) and nowhere else, at every step
/// shorter than, equal to and longer than a word, from every phase within a word, up to the last item.
#[test]
fn a_strided_fill_stores_exactly_the_items_one_store_at_a_time_would() {
    for len in [1usize, 2, 63, 64, 65, 129, 300, 1000] {
        for step in 1usize..=140 {
            for first in [0usize, 1, 5, 62, 63, 64, 65, 127, 200, 999] {
                if first >= len {
                    continue;
                }
                let most = (len - 1 - first) / step + 1;
                for count in [0, 1, 2, most / 2, most.saturating_sub(1), most] {
                    if count > most {
                        continue;
                    }
                    for value in [false, true] {
                        let mut model = mixed(len);
                        for k in 0..count {
                            model[first + k * step] = value;
                        }
                        let mut bits = from_model(&mixed(len));
                        unsafe { bits.fill_stride_unchecked(first, count, step, value) };
                        assert!(
                            bits == from_model(&model),
                            "len {len} step {step} first {first} count {count} value {value}"
                        );
                    }
                }
            }
        }
    }
}

/// A fill that reaches the last item leaves the bits past it clear, so pushing after it and comparing still agree.
#[test]
fn a_strided_fill_to_the_last_item_leaves_no_stray_bits() {
    for len in [1usize, 63, 64, 65, 127, 128, 129] {
        for step in [1usize, 2, 3, 63, 64, 65] {
            let first = (len - 1) % step;
            let count = (len - 1 - first) / step + 1;
            let mut bits = LogosBits::filled(len, false);
            unsafe { bits.fill_stride_unchecked(first, count, step, true) };
            let mut model = vec![false; len];
            for k in 0..count {
                model[first + k * step] = true;
            }
            bits.push(false);
            model.push(false);
            agree(&bits, &model);
        }
    }
}

/// A fill through an offset stores what a fill of `(through - first) / step + 1` items stores, and says how many steps
/// past `first` it took, whether or not `through` is an offset of the progression.
#[test]
fn a_fill_through_an_offset_stores_up_to_it_and_counts_its_steps() {
    let len = 400;
    for step in 1usize..=130 {
        for first in [0usize, 3, 63, 64, 100] {
            for through in [first, first + 1, first + step - 1, first + step, first + 2 * step + 1, len - 1] {
                if through < first || through >= len {
                    continue;
                }
                for value in [false, true] {
                    let mut model = mixed(len);
                    let mut at = first;
                    while at <= through {
                        model[at] = value;
                        at += step;
                    }
                    let mut bits = from_model(&mixed(len));
                    let more = unsafe { bits.fill_stride_through_unchecked(first, through, step, value) };
                    assert_eq!(more, (through - first) / step, "step {step} first {first} through {through}");
                    assert!(
                        bits == from_model(&model),
                        "step {step} first {first} through {through} value {value}"
                    );
                }
            }
        }
    }
}
