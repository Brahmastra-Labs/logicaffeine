use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn checks_043_eqtype() {
    let input = include_str!("fixtures/lean_arena/043_eqType.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_069_eqrec() {
    let input = include_str!("fixtures/lean_arena/069_eqRec.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_099_rulek() {
    let input = include_str!("fixtures/lean_arena/099_ruleK.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_100_rulekbad() {
    let input = include_str!("fixtures/lean_arena/100_ruleKbad.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}

#[test]
fn checks_101_rulekacc() {
    let input = include_str!("fixtures/lean_arena/101_ruleKAcc.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}

#[test]
fn checks_rec_k_lie() {
    let input = include_str!("fixtures/lean_arena/rec-k-lie.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}

#[test]
fn checks_nat_rec_k_lie() {
    let input = include_str!("fixtures/lean_arena/nat-rec-k-lie.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}

#[test]
fn checks_k_rec_conv() {
    let input = include_str!("fixtures/lean_arena/k-rec-conv.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}
