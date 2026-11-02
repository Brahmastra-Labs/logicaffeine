use super::*;

#[test]
fn test_mvregister_new() {
    let reg: MVRegister<String> = MVRegister::new(1);
    assert!(reg.values().is_empty());
}

#[test]
fn test_mvregister_set_get() {
    let mut reg: MVRegister<String> = MVRegister::new(1);
    reg.set("hello".to_string());
    assert_eq!(reg.values().len(), 1);
    assert_eq!(reg.values()[0], &"hello".to_string());
}

#[test]
fn test_mvregister_concurrent() {
    let mut a: MVRegister<String> = MVRegister::new(1);
    let mut b: MVRegister<String> = MVRegister::new(2);

    a.set("from-a".to_string());
    b.set("from-b".to_string());
    a.merge(&b);

    assert_eq!(a.values().len(), 2);
}

#[test]
fn test_mvregister_resolve() {
    let mut a: MVRegister<String> = MVRegister::new(1);
    let mut b: MVRegister<String> = MVRegister::new(2);

    a.set("from-a".to_string());
    b.set("from-b".to_string());
    a.merge(&b);
    a.resolve("resolved".to_string());

    assert_eq!(a.values().len(), 1);
    assert_eq!(a.values()[0], &"resolved".to_string());
}
