use super::*;
use std::collections::HashMap;

fn point(x: i64, y: i64) -> RuntimeValue {
    let mut f = HashMap::new();
    f.insert("x".to_string(), RuntimeValue::Int(x));
    f.insert("y".to_string(), RuntimeValue::Int(y));
    RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Point".to_string(),
        fields: f,
    }))
}
fn int_field(v: &RuntimeValue, name: &str) -> i64 {
    match v {
        RuntimeValue::Struct(sv) => match sv.fields.get(name) {
            Some(RuntimeValue::Int(n)) => *n,
            other => panic!("field {name} not an int: {other:?}"),
        },
        other => panic!("not a struct: {other:?}"),
    }
}

#[test]
fn from_values_homogeneous_structs_is_columnar() {
    let r = ListRepr::from_values(vec![point(0, 0), point(1, 2), point(2, 4)]);
    assert!(
        matches!(r, ListRepr::Structs { .. }),
        "a homogeneous struct list de-boxes to columns"
    );
    assert_eq!(r.len(), 3);
    assert!(!r.is_empty());
}

#[test]
fn structs_get_reconstructs_exact() {
    let r = ListRepr::from_values(vec![point(0, 0), point(1, 2), point(2, 4)]);
    let s = r.get(1).unwrap();
    assert_eq!(int_field(&s, "x"), 1);
    assert_eq!(int_field(&s, "y"), 2);
    assert!(r.get(3).is_none(), "out-of-range index is None");
}

#[test]
fn structs_to_values_reconstructs_all_rows() {
    let r = ListRepr::from_values(vec![point(5, 6), point(7, 8)]);
    let vs = r.to_values();
    assert_eq!(vs.len(), 2);
    assert_eq!(int_field(&vs[0], "x"), 5);
    assert_eq!(int_field(&vs[1], "y"), 8);
}

#[test]
fn structs_truncate_is_columnwise() {
    let mut r = ListRepr::from_values(vec![point(0, 0), point(1, 1), point(2, 2)]);
    r.truncate(2);
    assert!(
        matches!(r, ListRepr::Structs { .. }),
        "truncate keeps it columnar"
    );
    assert_eq!(r.len(), 2);
    assert_eq!(int_field(&r.get(1).unwrap(), "x"), 1);
}

#[test]
fn columnar_field_read_is_direct() {
    let r = ListRepr::from_values(vec![point(10, 20), point(30, 40)]);
    // get_field reads one column directly — no StructValue reconstruction.
    assert_eq!(r.get_field(0, "x"), Some(RuntimeValue::Int(10)));
    assert_eq!(r.get_field(1, "y"), Some(RuntimeValue::Int(40)));
    assert_eq!(r.get_field(0, "z"), None, "missing field");
    assert!(r.get_field(5, "x").is_none(), "out of range");
    // the column accessor exposes the raw packed column for array-speed scans.
    match r.column("x") {
        Some(ListRepr::Ints(v)) => assert_eq!(v, &vec![10, 30]),
        other => panic!("expected an Ints column, got {other:?}"),
    }
    // a boxed list has no columns.
    let boxed = ListRepr::Boxed(vec![point(1, 2)]);
    assert!(boxed.get_field(0, "x").is_none());
    assert!(boxed.column("x").is_none());
}

#[test]
fn structs_mutation_decolumnarizes_and_stays_correct() {
    // A set to a non-struct value de-columnarizes (make_boxed) but preserves
    // every prior row exactly — the soundness invariant.
    let mut r = ListRepr::from_values(vec![point(0, 0), point(1, 1), point(2, 2)]);
    r.set(1, RuntimeValue::Int(99));
    assert!(
        matches!(r, ListRepr::Boxed(_)),
        "a mutating set de-columnarizes"
    );
    assert_eq!(int_field(&r.get(0).unwrap(), "x"), 0);
    assert_eq!(r.get(1), Some(RuntimeValue::Int(99)));
    assert_eq!(int_field(&r.get(2).unwrap(), "y"), 2);
}

#[test]
fn structs_push_stays_correct() {
    let mut r = ListRepr::from_values(vec![point(0, 0)]);
    r.push(point(1, 1));
    assert_eq!(r.len(), 2);
    assert_eq!(int_field(&r.get(0).unwrap(), "x"), 0);
    assert_eq!(int_field(&r.get(1).unwrap(), "x"), 1);
}

#[test]
fn heterogeneous_structs_stay_boxed() {
    // Ragged field set ⇒ boxed.
    let mut only_x = HashMap::new();
    only_x.insert("x".to_string(), RuntimeValue::Int(9));
    let odd = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Point".to_string(),
        fields: only_x,
    }));
    let r = ListRepr::from_values(vec![point(0, 0), odd]);
    assert!(
        matches!(r, ListRepr::Boxed(_)),
        "ragged field sets stay boxed"
    );

    // Mixed type names ⇒ boxed.
    let mut cf = HashMap::new();
    cf.insert("x".to_string(), RuntimeValue::Int(1));
    cf.insert("y".to_string(), RuntimeValue::Int(2));
    let q = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Other".to_string(),
        fields: cf,
    }));
    let r2 = ListRepr::from_values(vec![point(0, 0), q]);
    assert!(
        matches!(r2, ListRepr::Boxed(_)),
        "mixed type names stay boxed"
    );
}

/// The fastest of `rounds` alternating timings of `a` and `b`, `iters` passes of each per round, in
/// nanoseconds. A round the scheduler preempts runs slower, never faster, so each side's fastest round
/// is its cost with the least interference; alternating puts both sides under the same load.
fn fastest_alternating(
    rounds: u32,
    iters: u32,
    mut a: impl FnMut(),
    mut b: impl FnMut(),
) -> (u128, u128) {
    let (mut fastest_a, mut fastest_b) = (u128::MAX, u128::MAX);
    for _ in 0..rounds {
        let t = std::time::Instant::now();
        for _ in 0..iters {
            a();
        }
        fastest_a = fastest_a.min(t.elapsed().as_nanos().max(1));
        let t = std::time::Instant::now();
        for _ in 0..iters {
            b();
        }
        fastest_b = fastest_b.min(t.elapsed().as_nanos().max(1));
    }
    (fastest_a, fastest_b)
}

#[test]
fn columnar_field_scan_is_faster_than_boxed_and_iteration_is_not_slower() {
    // The in-memory win, measured on the shared `ListRepr` primitive (both the
    // tree-walker and the VM read lists through it). Reported with --nocapture.
    // Each comparison is the fastest of alternating rounds (`fastest_alternating`),
    // so a loaded machine cannot stand in for either representation.
    const N: usize = 5000;
    const ROUNDS: u32 = 6;
    const ITERS: u32 = 50;

    let rows: Vec<RuntimeValue> = (0..N as i64).map(|i| point(i, i * 2)).collect();
    let columnar = ListRepr::from_values(rows.clone());
    assert!(
        matches!(columnar, ListRepr::Structs { .. }),
        "the columnar baseline must be Structs"
    );
    let boxed = ListRepr::Boxed(rows);

    // (a) FIELD SCAN — sum the "x" field across the list. Columnar reads the raw
    //     Vec<i64> column (array speed); boxed reconstructs/clones each struct and
    //     hashmap-looks-up "x". This is the "arrays not heap" win.
    let scan_columnar = || -> i64 {
        match columnar.column("x") {
            Some(ListRepr::Ints(v)) => v.iter().copied().sum(),
            _ => unreachable!(),
        }
    };
    let scan_boxed = || -> i64 {
        let mut s = 0i64;
        for i in 0..boxed.len() {
            if let Some(RuntimeValue::Struct(sv)) = boxed.get(i) {
                if let Some(RuntimeValue::Int(x)) = sv.fields.get("x") {
                    s += *x;
                }
            }
        }
        s
    };
    assert_eq!(scan_columnar(), scan_boxed(), "the two scans must agree");

    let (col_ns, box_ns) = fastest_alternating(
        ROUNDS,
        ITERS,
        || {
            std::hint::black_box(scan_columnar());
        },
        || {
            std::hint::black_box(scan_boxed());
        },
    );
    println!(
        "\n[E3] field scan (sum x over {N}, fastest of {ROUNDS} rounds ×{ITERS}): columnar {col_ns} ns vs boxed {box_ns} ns  ({:.1}× faster)",
        box_ns as f64 / col_ns as f64
    );
    assert!(
        col_ns * 2 <= box_ns,
        "columnar field scan should be ≥2× faster: columnar {col_ns} vs boxed {box_ns}"
    );

    // (b) FULL-ROW iteration — reconstruct every struct. Columnar rebuilds each
    //     row's StructValue from its columns where boxed clones a stored one, so it
    //     costs more per row; reported for honesty, asserted only "not
    //     catastrophically slower".
    let iter_repr = |r: &ListRepr| {
        let mut acc = 0i64;
        for i in 0..r.len() {
            if let Some(RuntimeValue::Struct(sv)) = r.get(i) {
                if let (Some(RuntimeValue::Int(x)), Some(RuntimeValue::Int(y))) =
                    (sv.fields.get("x"), sv.fields.get("y"))
                {
                    acc += x + y;
                }
            }
        }
        acc
    };
    assert_eq!(
        iter_repr(&columnar),
        iter_repr(&boxed),
        "full-row iteration must agree"
    );
    let (col_it, box_it) = fastest_alternating(
        ROUNDS,
        ITERS,
        || {
            std::hint::black_box(iter_repr(&columnar));
        },
        || {
            std::hint::black_box(iter_repr(&boxed));
        },
    );
    println!(
        "[E3] full-row iter (fastest of {ROUNDS} rounds ×{ITERS}): columnar {col_it} ns vs boxed {box_it} ns  ({:.2}× of boxed)",
        col_it as f64 / box_it as f64
    );
    assert!(
        col_it <= box_it * 3,
        "full-row iteration must not be catastrophically slower: {col_it} vs {box_it}"
    );
}

#[test]
fn zero_field_structs_stay_boxed_and_keep_count() {
    // A columnar store has no column to carry the row count when the struct has
    // no fields, so such a list MUST stay boxed (and report the right length).
    let unit = || {
        RuntimeValue::Struct(Box::new(StructValue {
            type_name: "Unit".to_string(),
            fields: HashMap::new(),
        }))
    };
    let r = ListRepr::from_values(vec![unit(), unit(), unit()]);
    assert!(
        matches!(r, ListRepr::Boxed(_)),
        "zero-field struct list stays boxed"
    );
    assert_eq!(r.len(), 3, "row count is preserved");
}
