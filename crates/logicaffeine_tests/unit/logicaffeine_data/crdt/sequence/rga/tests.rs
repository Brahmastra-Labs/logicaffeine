use super::*;

#[test]
fn test_rga_append() {
    let mut seq: RGA<String> = RGA::new(1);
    seq.append("a".to_string());
    seq.append("b".to_string());
    assert_eq!(seq.to_vec(), vec!["a", "b"]);
}

#[test]
fn test_rga_insert_before() {
    let mut seq: RGA<String> = RGA::new(1);
    seq.append("b".to_string());
    seq.insert_before(0, "a".to_string());
    assert_eq!(seq.to_vec(), vec!["a", "b"]);
}

#[test]
fn test_rga_remove() {
    let mut seq: RGA<String> = RGA::new(1);
    seq.append("a".to_string());
    seq.append("b".to_string());
    seq.remove(0);
    assert_eq!(seq.to_vec(), vec!["b"]);
}

#[test]
fn test_rga_concurrent_append() {
    let mut a: RGA<String> = RGA::new(1);
    let mut b: RGA<String> = RGA::new(2);

    a.append("from-a".to_string());
    b.append("from-b".to_string());

    a.merge(&b);
    b.merge(&a);

    assert_eq!(a.to_vec(), b.to_vec());
}
