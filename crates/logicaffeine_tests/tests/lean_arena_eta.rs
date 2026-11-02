use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn checks_075_typesingletonrecreduction() {
    let input = include_str!("fixtures/lean_arena/075_typeSingletonRecReduction.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_107_uniteta1() {
    let input = include_str!("fixtures/lean_arena/107_unitEta1.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_108_uniteta2() {
    let input = include_str!("fixtures/lean_arena/108_unitEta2.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_109_uniteta3() {
    let input = include_str!("fixtures/lean_arena/109_unitEta3.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_110_indexeduniteta() {
    let input = include_str!("fixtures/lean_arena/110_indexedUnitEta.ndjson");
    let e = adapter::check_str(input).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}

#[test]
fn checks_111_structeta() {
    let input = include_str!("fixtures/lean_arena/111_structEta.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_112_indexedstructeta() {
    let input = include_str!("fixtures/lean_arena/112_indexedStructEta.ndjson");
    let e = adapter::check_str(input).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}
