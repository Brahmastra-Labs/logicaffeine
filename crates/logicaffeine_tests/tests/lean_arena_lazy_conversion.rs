use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn checks_irrelevance_before_evaluation() {
    let input = include_str!("fixtures/lean_arena/irrelevance-before-evaluation.ndjson");
    adapter::check_str(input).unwrap();
}

#[test]
fn checks_refute_cheap_first() {
    let input = include_str!("fixtures/lean_arena/refute-cheap-first.ndjson");
    let e = adapter::check_str(input).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}

#[test]
fn checks_refute_cheap_last() {
    let input = include_str!("fixtures/lean_arena/refute-cheap-last.ndjson");
    let e = adapter::check_str(input).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}

#[test]
fn checks_discarded_argument() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/discarded-argument.ndjson"
    ))
    .unwrap();
}
#[test]
fn checks_discarded_argument_match() {
    adapter::check_str(include_str!(
        "fixtures/lean_arena/discarded-argument-match.ndjson"
    ))
    .unwrap();
}
