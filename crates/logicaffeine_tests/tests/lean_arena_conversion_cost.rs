use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn checks_proj_of_stuck_prop() {
    let e = adapter::check_str(include_str!(
        "fixtures/lean_arena/proj-of-stuck-prop.ndjson"
    ))
    .unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}

#[test]
fn checks_proj_of_subst_prop() {
    let e = adapter::check_str(include_str!(
        "fixtures/lean_arena/proj-of-subst-prop.ndjson"
    ))
    .unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}

#[test]
fn checks_rec_missing_ih() {
    let e =
        adapter::check_str(include_str!("fixtures/lean_arena/rec-missing-ih.ndjson")).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}

#[test]
fn checks_rec_of_subst_prop() {
    let e = adapter::check_str(include_str!("fixtures/lean_arena/rec-of-subst-prop.ndjson"))
        .unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
}
