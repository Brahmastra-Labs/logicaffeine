use super::*;

#[test]
fn zerocopy_reads_are_in_place_and_correct() {
    // The zero-copy row is only honest if it actually reads IN PLACE (the aligned-slice fast
    // path), not the decode fallback. Assert the fast path fires and the borrowed slice is
    // bit-exact with the source — this is what makes the fair capnp/arrow comparison real.
    let ints: Vec<i64> = (0..1000).map(|i| i * 7 - 3).collect();
    let rv = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Ints(ints.clone()))));
    let bytes = encode_zerocopy(&rv);
    let view = view_message(&bytes).expect("native view opens");
    let slice = view
        .as_i64_slice()
        .expect("int zero-copy MUST read the aligned column IN PLACE");
    assert_eq!(
        slice,
        ints.as_slice(),
        "the borrowed i64 slice equals the source (no copy, no loss)"
    );
    let want = ints.iter().fold(0i64, |a, &x| a.wrapping_add(x));
    assert_eq!(
        zerocopy_sum_i64(&bytes),
        want,
        "zero-copy int sum == materialized sum"
    );

    let floats: Vec<f64> = (0..1000).map(|i| i as f64 * 0.5 - 1.0).collect();
    let rvf = RuntimeValue::List(Rc::new(RefCell::new(ListRepr::Floats(floats.clone()))));
    let fb = encode_zerocopy(&rvf);
    let fview = view_message(&fb).expect("native view opens");
    let fslice = fview
        .as_f64_slice()
        .expect("float zero-copy MUST read the aligned column IN PLACE");
    assert_eq!(
        fslice,
        floats.as_slice(),
        "the borrowed f64 slice equals the source"
    );
    let fwant: f64 = floats.iter().sum();
    assert_eq!(
        zerocopy_sum_f64(&fb),
        fwant,
        "zero-copy float sum == materialized sum"
    );
}

#[test]
fn zerocopy_points_read_matches_materialized() {
    // The columnar points zero-copy path reads each field's CONTIGUOUS blob in place; it must
    // (a) actually find both fixed columns (the fast path fires) and (b) sum EXACTLY what a full
    // decode would. Fields are canonically sorted → column 0 = x, column 1 = y.
    let pts: Vec<(i64, i64)> = (0..500).map(|i| (i * 3 - 7, -i * 5 + 2)).collect();
    let rv = rv_list(pts.iter().map(|&(x, y)| point_struct(x, y)).collect());
    let bytes = encode_columnar(&rv);
    let view = view_message(&bytes).expect("native view opens");
    let xcol = view
        .structs_fixed_i64_col(0)
        .expect("x column MUST read as a contiguous blob");
    let ycol = view
        .structs_fixed_i64_col(1)
        .expect("y column MUST read as a contiguous blob");
    assert_eq!(xcol.len(), pts.len() * 8, "x column is n contiguous i64s");
    assert_eq!(ycol.len(), pts.len() * 8, "y column is n contiguous i64s");
    let want = pts
        .iter()
        .fold(0i64, |a, &(x, y)| a.wrapping_add(x).wrapping_add(y));
    assert_eq!(
        zerocopy_sum_points(&bytes),
        want,
        "zero-copy columnar points sum == materialized sum"
    );
}

#[test]
fn fair_scenarios_carry_a_zero_copy_row() {
    let report = build_report(20);
    for id in ["ints", "floats", "points"] {
        let s = report
            .scenarios
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no '{id}' scenario"));
        assert!(
            s.rows.iter().any(|r| r.codec == "logos (zero-copy)"),
            "'{id}' must carry a 'logos (zero-copy)' row (the fair zero-copy read path)"
        );
    }
}
