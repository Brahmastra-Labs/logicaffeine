use super::*;

#[test]
fn test_yata_append() {
    let mut seq: YATA<char> = YATA::new(1);
    seq.append('a');
    seq.append('b');
    assert_eq!(seq.to_vec(), vec!['a', 'b']);
}

#[test]
fn test_yata_concurrent() {
    let mut a: YATA<char> = YATA::new(1);
    let mut b: YATA<char> = YATA::new(2);

    a.append('A');
    b.append('B');

    a.merge(&b);
    b.merge(&a);

    assert_eq!(a.to_vec(), b.to_vec());
}
