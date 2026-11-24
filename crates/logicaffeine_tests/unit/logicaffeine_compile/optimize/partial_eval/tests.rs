use super::*;

/// One literal of every kind the specializer names, with the values at each kind's edges.
fn every_literal(interner: &mut Interner) -> Vec<Literal> {
    vec![
        Literal::Number(0),
        Literal::Number(7),
        Literal::Number(-7),
        Literal::Number(i64::MIN),
        Literal::Number(i64::MAX),
        Literal::Float(0.0),
        Literal::Float(-0.0),
        Literal::Float(15.0),
        Literal::Float(f64::NAN),
        Literal::Text(interner.intern("ul")),
        Literal::Text(interner.intern("")),
        Literal::Boolean(true),
        Literal::Boolean(false),
        Literal::Nothing,
        Literal::Char('a'),
        Literal::Char('\u{10FFFF}'),
        Literal::Duration(-5),
        Literal::Duration(300),
        Literal::Date(-1),
        Literal::Date(19_000),
        Literal::Moment(-3),
        Literal::Moment(1_700_000_000_000_000_000),
        Literal::Span { months: -2, days: 3 },
        Literal::Span { months: 14, days: -30 },
        Literal::Time(0),
        Literal::Time(86_399_999_999_999),
    ]
}

/// Function names a specialization can be made from, among them names that hold `_s` themselves.
const ORIGINS: &[&str] = &["vdOpenKeyed", "f", "__render_App", "count_down", "a_s", "x_s1", "keep_s_"];

#[test]
fn a_specialization_names_the_function_it_was_made_from() {
    let mut interner = Interner::new();
    let literals = every_literal(&mut interner);
    for origin in ORIGINS {
        let func = interner.intern(origin);
        for (i, lit) in literals.iter().enumerate() {
            let other = literals[(i * 7 + 3) % literals.len()].clone();
            let shapes: [Vec<Option<Literal>>; 4] = [
                vec![Some(lit.clone())],
                vec![None, Some(lit.clone())],
                vec![Some(lit.clone()), None, Some(other.clone())],
                vec![None, Some(other), None, Some(lit.clone())],
            ];
            for shape in &shapes {
                let fresh = make_spec_name(&mut interner, func, shape, &|_| false);
                let name = interner.resolve(fresh).to_string();
                assert_eq!(specialization_origin(&name), *origin, "{name} is made from {origin}");
                // A name already taken grows `_`s until it is not; the origin is still the function's.
                let mut taken = vec![fresh];
                for _ in 0..3 {
                    let held = taken.clone();
                    let next = make_spec_name(&mut interner, func, shape, &|s| held.contains(&s));
                    let name = interner.resolve(next).to_string();
                    assert_eq!(specialization_origin(&name), *origin, "{name} is made from {origin}");
                    taken.push(next);
                }
            }
        }
    }
}

#[test]
fn a_specialization_of_a_specialization_names_the_first_function() {
    let mut interner = Interner::new();
    let f = interner.intern("vdAdoptTree");
    let once = make_spec_name(&mut interner, f, &[Some(Literal::Boolean(true))], &|_| false);
    for position in 0..3 {
        let mut shape = vec![None; position];
        shape.push(Some(Literal::Number(-2)));
        let twice = make_spec_name(&mut interner, once, &shape, &|_| false);
        assert_eq!(specialization_origin(interner.resolve(twice)), "vdAdoptTree");
        let thrice = make_spec_name(&mut interner, twice, &[Some(Literal::Nothing)], &|s| s == twice);
        assert_eq!(specialization_origin(interner.resolve(thrice)), "vdAdoptTree");
    }
}

#[test]
fn a_function_that_is_no_specialization_is_its_own_origin() {
    for name in [
        "vdOpenKeyed",
        "fib",
        "x_s",
        "x_sa_1",
        "x_s1",
        "x_s1_",
        "_s0_1",
        "count_s_1",
        "f_s0_",
        "f_s0_q1",
        "f_s0_sp1",
        "f_s0_tz",
        "f_s0_m",
        "f_s0_dux",
        "",
    ] {
        assert_eq!(specialization_origin(name), name, "{name:?}");
    }
}
