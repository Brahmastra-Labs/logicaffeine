use super::*;

#[test]
fn test_lww_new() {
    let reg = LWWRegister::new("hello".to_string(), 100);
    assert_eq!(reg.get(), "hello");
    assert_eq!(reg.timestamp(), 100);
}

#[test]
fn test_lww_set() {
    let mut reg = LWWRegister::new("hello".to_string(), 100);
    reg.set("world".to_string(), 200);
    assert_eq!(reg.get(), "world");
    assert_eq!(reg.timestamp(), 200);
}

#[test]
fn test_lww_merge_newer_wins() {
    let r1 = LWWRegister::new("old".to_string(), 100);
    let r2 = LWWRegister::new("new".to_string(), 200);

    let mut r1_copy = r1.clone();
    r1_copy.merge(&r2);
    assert_eq!(r1_copy.get(), "new");
}

#[test]
fn test_lww_merge_older_loses() {
    let r1 = LWWRegister::new("old".to_string(), 100);
    let r2 = LWWRegister::new("new".to_string(), 200);

    let mut r2_copy = r2.clone();
    r2_copy.merge(&r1);
    // r2 had higher timestamp, so it keeps its value
    assert_eq!(r2_copy.get(), "new");
}

#[test]
fn test_lww_merge_idempotent() {
    let reg = LWWRegister::new("test".to_string(), 100);
    let mut reg_copy = reg.clone();
    reg_copy.merge(&reg);
    assert_eq!(reg_copy.get(), "test");
}

#[test]
fn test_lww_with_int() {
    let mut reg = LWWRegister::new(42i64, 100);
    assert_eq!(*reg.get(), 42);
    reg.set(100, 200);
    assert_eq!(*reg.get(), 100);
}

#[test]
fn test_lww_with_bool() {
    let mut reg = LWWRegister::new(false, 100);
    assert_eq!(*reg.get(), false);
    reg.set(true, 200);
    assert_eq!(*reg.get(), true);
}
