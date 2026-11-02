use super::*;

#[test]
fn bytes_takes_every_byte_and_names_the_first_item_that_is_not_one() {
    assert_eq!(
        bytes("message", &[0, 1, 127, 255]),
        Ok(vec![0, 1, 127, 255])
    );
    assert_eq!(bytes("message", &[]), Ok(vec![]));
    for (s, item, x) in [
        (vec![256i64], 1, 256i64),
        (vec![0, -1], 2, -1),
        (vec![1, 2, 3, 1000, -5], 4, 1000),
        (vec![i64::MIN], 1, i64::MIN),
        (vec![i64::MAX], 1, i64::MAX),
    ] {
        assert_eq!(
            bytes("message", &s),
            Err(format!(
                "the message must be bytes (0 to 255); item {item} is {x}"
            )),
            "{s:?}"
        );
    }
}

#[test]
fn a_byte_array_takes_exactly_its_length() {
    let seed: Vec<i64> = (0..32).collect();
    assert_eq!(byte_array::<32>("seed", &seed).unwrap()[31], 31);
    assert_eq!(
        byte_array::<32>("seed", &seed[..31]),
        Err("the seed is 31 bytes; it must be 32".to_string())
    );
    let mut long = seed.clone();
    long.push(0);
    assert_eq!(
        byte_array::<32>("seed", &long),
        Err("the seed is 33 bytes; it must be 32".to_string())
    );
    assert_eq!(
        byte_array::<32>("seed", &[300; 32]),
        Err("the seed must be bytes (0 to 255); item 1 is 300".to_string()),
        "a non-byte is named before the length"
    );
    assert_eq!(
        exact_bytes("ciphertext", &[1, 2], 3),
        Err("the ciphertext is 2 bytes; it must be 3".to_string())
    );
}

#[test]
fn integers_are_held_to_their_range() {
    assert_eq!(int_in("width d", 1, 1, 11), Ok(1));
    assert_eq!(int_in("width d", 11, 1, 11), Ok(11));
    assert_eq!(
        int_in("width d", 12, 1, 11),
        Err("the width d must be from 1 to 11; it is 12".to_string())
    );
    assert_eq!(
        int_in("width d", 0, 1, 11),
        Err("the width d must be from 1 to 11; it is 0".to_string())
    );
    assert_eq!(non_negative("count", 0), Ok(0));
    assert_eq!(
        non_negative("count", -3),
        Err("the count must not be negative; it is -3".to_string())
    );
}

#[test]
fn coefficient_counts_and_ranges_name_what_broke() {
    assert_eq!(count("polynomial", 256, 256), Ok(()));
    assert_eq!(
        count("polynomial", 3, 256),
        Err("the polynomial has 3 coefficients; it must have 256".to_string())
    );
    assert_eq!(
        same_count("operands", 256, 255),
        Err("the operands have 256 and 255 coefficients; they must match".to_string())
    );
    assert_eq!(ints_below("polynomial", &[0, 3328], 3329), Ok(()));
    assert_eq!(
        ints_below("polynomial", &[0, 3329, -1], 3329),
        Err("the polynomial must hold values from 0 to 3328; item 2 is 3329".to_string())
    );
    assert_eq!(
        ints_below("polynomial", &[-1], 3329),
        Err("the polynomial must hold values from 0 to 3328; item 1 is -1".to_string())
    );
    assert_eq!(words_below("polynomial", &[Word16(3328)], 3329), Ok(()));
    assert_eq!(
        words_below("polynomial", &[Word16(1), Word16(65535)], 3329),
        Err("the polynomial must hold values from 0 to 3328; item 2 is 65535".to_string())
    );
}

#[test]
fn bytes_come_back_as_the_ints_a_program_reads() {
    assert_eq!(seq_of_bytes(&[0, 255, 7]).to_vec(), vec![0i64, 255, 7]);
}
