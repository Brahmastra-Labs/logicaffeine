//! The exact joins of a scope's bindings, on parsed programs, and the conversions an integer takes where a rational
//! is held.
use super::*;

fn with_program<R>(src: &str, f: impl FnOnce(&[Stmt], &TypeRegistry, &Interner) -> R) -> R {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, registry, _) = parsed.expect("the program parses");
        f(stmts, registry, interner)
    })
}

/// The joins of Main in `body` (a program's Main body lines after a rational `r`), as `name: Rust type` sorted.
fn main_joins(body: &str) -> Vec<String> {
    let src = format!(
        "## To native parseInt (s: Text) -> Int\n\n## Main\nLet r: Rational be 1 / parseInt(\"3\").\n{body}"
    );
    with_program(&src, |stmts, registry, interner| {
        let main: Vec<&Stmt> = stmts.iter().filter(|s| !matches!(s, Stmt::FunctionDef { .. })).collect();
        let main: Vec<Stmt> = main.into_iter().cloned().collect();
        let types: HashMap<Symbol, String> =
            HashMap::from([(interner.lookup("r").expect("r is interned"), RATIONAL.to_string())]);
        let t = Typing { interner, variable_types: &types, registry };
        let mut out: Vec<String> = joins(&main, &HashSet::new(), t)
            .into_iter()
            .map(|(var, t)| format!("{}: {t}", interner.resolve(var)))
            .collect();
        out.sort();
        out
    })
}

#[test]
fn an_integer_counter_that_gains_a_fraction_is_held_as_a_rational() {
    assert_eq!(
        main_joins("Let mutable t be parseInt(\"0\").\nSet t to t + r.\nShow t.\n"),
        ["t: LogosRational"]
    );
}

#[test]
fn a_binding_copied_from_a_joined_one_is_joined_too() {
    assert_eq!(
        main_joins("Let mutable t be 0.\nLet mutable u be 0.\nSet u to t.\nSet t to t + r.\nShow u.\n"),
        ["t: LogosRational", "u: LogosRational"],
        "the round after `t` widens sees `u` take it"
    );
}

#[test]
fn a_list_of_integers_a_rational_is_pushed_onto_or_stored_into_holds_rationals() {
    assert_eq!(
        main_joins("Let mutable xs be [1, 2].\nPush r to xs.\nShow xs.\n"),
        ["xs: LogosSeq<LogosRational>"]
    );
    assert_eq!(
        main_joins("Let mutable xs be [1, 2].\nSet item 1 of xs to r.\nShow xs.\n"),
        ["xs: LogosSeq<LogosRational>"]
    );
}

#[test]
fn a_counter_read_through_a_loop_over_rationals_is_joined() {
    assert_eq!(
        main_joins("Let ys be [r, r * 2].\nLet mutable total be 0.\nRepeat for y in ys:\n    Set total to total + y.\nShow total.\n"),
        ["total: LogosRational"]
    );
}

#[test]
fn bindings_whose_values_agree_or_meet_a_real_are_left_alone() {
    assert!(main_joins("Let mutable t be r.\nSet t to t * 2.\nShow t.\n").is_empty(), "already a rational");
    assert!(main_joins("Let ys be [1, r].\nShow ys.\n").is_empty(), "the literal holds rationals itself");
    assert!(
        main_joins("Let mutable t be 0.\nSet t to t + 1.5.\nShow t.\n").is_empty(),
        "an integer and a real are no exact join"
    );
}

#[test]
fn a_name_the_scope_binds_twice_declares_or_loops_over_is_not_joined() {
    assert!(main_joins("If 1 > 0:\n    Let t be 1.\n    Show t.\nIf 2 > 0:\n    Let t be r.\n    Show t.\n").is_empty());
    assert!(main_joins("Let mutable t: Rational be 0.\nSet t to t + r.\nShow t.\n").is_empty());
    assert!(main_joins("Repeat for t in [1, 2]:\n    Show t + r.\n").is_empty());
}

#[test]
fn a_function_defined_in_the_scope_is_its_own_scope() {
    let src = "## To native parseInt (s: Text) -> Int\n\n## To f (q: Rational) -> Rational:\n    Let mutable t be 0.\n    Set t to t + q.\n    Return t.\n\n## Main\nShow f(1 / parseInt(\"3\")).\n";
    with_program(src, |stmts, registry, interner| {
        let none = HashMap::new();
        let t = Typing { interner, variable_types: &none, registry };
        assert!(joins(stmts, &HashSet::new(), t).is_empty(), "Main sees no `t`");
        let Some(Stmt::FunctionDef { params, body, .. }) =
            stmts.iter().find(|s| matches!(s, Stmt::FunctionDef { name, .. } if interner.resolve(*name) == "f"))
        else {
            panic!("f is defined")
        };
        let q = params[0].0;
        let given = HashSet::from([q]);
        let types = HashMap::from([(q, RATIONAL.to_string())]);
        let joined = joins(body, &given, Typing { interner, variable_types: &types, registry });
        assert_eq!(joined.len(), 1);
        assert_eq!(joined.values().next().map(String::as_str), Some(RATIONAL));
    });
}

#[test]
fn an_integer_held_where_rationals_are_is_the_rational_it_equals() {
    with_program("## Main\nLet n be 5.\nShow n.\n", |_, _, interner| {
        let n = Expr::Identifier(interner.lookup("n").expect("n is interned"));
        let none = HashMap::new();
        assert_eq!(
            converted(&n, "n", &LogosType::Int, &LogosType::Rational, &none).as_deref(),
            Some("LogosRational::from_i64((n) as i64)")
        );
        let big = HashMap::from([(interner.lookup("n").expect("n"), "i64|__bigint".to_string())]);
        assert_eq!(
            converted(&n, "n", &LogosType::Int, &LogosType::Rational, &big).as_deref(),
            Some("LogosRational::from(n)"),
            "an integer past a machine word converts whole"
        );
        let ints = LogosType::Seq(Box::new(LogosType::Int));
        let rationals = LogosType::Seq(Box::new(LogosType::Rational));
        assert_eq!(
            converted(&n, "n", &ints, &rationals, &none).as_deref(),
            Some("ToRationals::to_rationals(&n)")
        );
        assert_eq!(converted(&n, "n", &LogosType::Rational, &LogosType::Rational, &none), None);
        assert_eq!(converted(&n, "n", &LogosType::Int, &LogosType::Float, &none), None);
        assert_eq!(converted(&n, "n", &LogosType::String, &LogosType::Rational, &none), None);
    });
}

#[test]
fn a_rational_at_an_integer_place_is_refused_in_the_interpreters_words() {
    let code = integer_at("q", IntegerPlace::ListRead);
    let (past_word, fraction) = integer_place_refusals(IntegerPlace::ListRead);
    assert_eq!(code, format!("q.integer({past_word:?}, {fraction:?})"));
    assert!(!fraction.is_empty() && !past_word.is_empty());
}

#[test]
fn a_read_of_a_records_field_is_typed_by_the_fields_declaration() {
    let src = "## To native parseInt (s: Text) -> Int\n\n## A Point has:\n    an x, which is Rational.\n    a y, which is Seq of Rational.\n    a n, which is Int.\n\n## Main\nLet p be a new Point with x 1 and y [] and n 2.\nShow p's x / 2.\nShow item 1 of p's y + 1.\nShow p's n.\n";
    with_program(src, |stmts, registry, interner| {
        let p = interner.lookup("p").expect("p is interned");
        let types = HashMap::from([(p, "Point".to_string())]);
        let t = Typing { interner, variable_types: &types, registry };
        let mut shown: Vec<LogosType> = Vec::new();
        for s in stmts {
            if let Stmt::Show { object, .. } = s {
                shown.push(type_of(object, t));
            }
        }
        assert_eq!(shown, [LogosType::Rational, LogosType::Rational, LogosType::Int]);
    });
}

/// An expression is typed from the bindings and functions it names wherever they sit in it: a call inside a sum and a
/// binding inside an index keep their types though the environment holds nothing else.
#[test]
fn an_expression_is_typed_from_the_names_it_holds_wherever_they_sit() {
    let src = "## To native parseInt (s: Text) -> Int\n\n## To halve (x: Rational) -> Rational:\n    Return x / 2.\n\n## Main\nLet r: Rational be 1 / parseInt(\"3\").\nLet ys be [r, r, r].\nShow halve(item 2 of ys) + r.\n";
    with_program(src, |stmts, registry, interner| {
        let sym = |name: &str| interner.lookup(name).unwrap_or_else(|| panic!("`{name}` is interned"));
        let shown = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::Show { object, .. } => Some(*object),
                _ => None,
            })
            .expect("Main shows the sum");
        let types: HashMap<Symbol, String> = HashMap::from([
            (sym("r"), RATIONAL.to_string()),
            (sym("ys"), format!("LogosSeq<{RATIONAL}>")),
        ]);
        super::super::expr::set_fn_return_types(&HashMap::from([(sym("halve"), RATIONAL.to_string())]));
        let t = Typing { interner, variable_types: &types, registry };
        let sum = type_of(shown, t);
        let Expr::BinaryOp { left: Expr::Call { args, .. }, .. } = shown else { panic!("the sum is a call plus a binding") };
        let item = type_of(args[0], t);
        super::super::expr::set_fn_return_types(&HashMap::new());
        assert_eq!(sum, LogosType::Rational, "the call's declared result and the binding type the sum");
        assert_eq!(item, LogosType::Rational, "the binding inside the index types the item");
    });
}
