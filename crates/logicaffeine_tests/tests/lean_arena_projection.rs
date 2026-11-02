use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn checks_089_projprop1() {
    let input = include_str!("fixtures/lean_arena/089_projProp1.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_098_projred() {
    let input = include_str!("fixtures/lean_arena/098_projRed.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_087_projoutofrange() {
    let input = include_str!("fixtures/lean_arena/087_projOutOfRange.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}

#[test]
fn checks_088_projnotstruct() {
    let input = include_str!("fixtures/lean_arena/088_projNotStruct.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}

#[test]
fn checks_090_projprop2() {
    let input = include_str!("fixtures/lean_arena/090_projProp2.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}

#[test]
fn checks_proj_of_prop() {
    let input = include_str!("fixtures/lean_arena/proj-of-prop.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}

#[test]
fn checks_proj_non_structure() {
    let input = include_str!("fixtures/lean_arena/proj-non-structure.ndjson");
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 1, "{error:?}");
}

#[test]
fn checks_091_projprop3() {
    let input = include_str!("fixtures/lean_arena/091_projProp3.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_092_projprop4() {
    let input = include_str!("fixtures/lean_arena/092_projProp4.ndjson");
    let e = adapter::check_str(input).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}

#[test]
fn checks_093_projprop5() {
    let input = include_str!("fixtures/lean_arena/093_projProp5.ndjson");
    let e = adapter::check_str(input).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}

#[test]
fn checks_094_projprop6() {
    let input = include_str!("fixtures/lean_arena/094_projProp6.ndjson");
    let e = adapter::check_str(input).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}
