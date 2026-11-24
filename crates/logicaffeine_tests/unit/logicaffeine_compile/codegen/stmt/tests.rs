use super::*;
use logicaffeine_language::ast_depth::{for_each_node, Visited};

/// The count proof with no call admitted as pure.
fn collect_bounded_count_accumulators(
    stmts: &[Stmt],
    bignum_vars: &std::collections::HashSet<Symbol>,
) -> std::collections::HashSet<Symbol> {
    collect_bounded_count_accumulators_with_purity(stmts, bignum_vars, &Default::default())
}

/// Each access `item i of <arr>` in the first `Repeat` of `source`, as `<arr> raw` when the fixed-array rule
/// marks it in range and `<arr> checked` when it keeps the rule, with each array's Rust type from `types`.
fn marks(source: &str, types: &[(&str, &str)]) -> Vec<String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let (counter, iterable, body) = stmts
            .iter()
            .find_map(|s| match s {
                Stmt::Repeat {
                    pattern: Pattern::Identifier(counter),
                    iterable,
                    body,
                } => Some((*counter, *iterable, &body[..])),
                _ => None,
            })
            .expect("a counted loop");
        let var_types: HashMap<Symbol, String> = types
            .iter()
            .map(|(name, ty)| {
                (
                    interner.lookup(name).expect("the array is named"),
                    ty.to_string(),
                )
            })
            .collect();
        let _oracle = super::super::expr::install_range_oracle(Some(std::rc::Rc::new(
            crate::optimize::OracleFacts::default(),
        )));
        mark_fixed_array_counter_accesses(counter, iterable, body, &var_types);
        let mut out = Vec::new();
        for_each_node(body, |node| {
            let (collection, index) = match node {
                Visited::Expr(Expr::Index { collection, index }) => (*collection, *index),
                Visited::Stmt(Stmt::SetIndex {
                    collection, index, ..
                }) => (*collection, *index),
                _ => return,
            };
            let Expr::Identifier(arr) = collection else {
                return;
            };
            let proven = super::super::expr::range_proves_index(collection, index);
            out.push(format!(
                "{} {}",
                interner.resolve(*arr),
                if proven { "raw" } else { "checked" }
            ));
        });
        out.sort();
        out
    })
}

const FIXED: &[(&str, &str)] = &[("buf", "[i64; 8]")];

fn main_with(repeat: &str) -> String {
    format!("## Main\nLet mutable buf be a new Seq of Int.\nLet mutable total be 0.\n{repeat}Show total.\n")
}

#[test]
fn a_literal_range_inside_a_fixed_array_marks_its_accesses() {
    let source =
        main_with("Repeat for i from 1 to 8:\n    Set item i of buf to (item i of buf) + 1.\n");
    assert_eq!(marks(&source, FIXED), ["buf raw", "buf raw"]);
    let source = main_with("Repeat for i from 3 to 8:\n    Set total to total + item i of buf.\n");
    assert_eq!(marks(&source, FIXED), ["buf raw"]);
}

#[test]
fn a_range_reaching_past_the_array_keeps_the_rule() {
    let source = main_with("Repeat for i from 1 to 9:\n    Set total to total + item i of buf.\n");
    assert_eq!(marks(&source, FIXED), ["buf checked"]);
}

#[test]
fn a_range_starting_below_one_keeps_the_rule() {
    let source = main_with("Repeat for i from 0 to 7:\n    Set total to total + item i of buf.\n");
    assert_eq!(marks(&source, FIXED), ["buf checked"]);
}

#[test]
fn a_sequence_of_unknown_length_keeps_the_rule() {
    let source = main_with("Repeat for i from 1 to 8:\n    Set total to total + item i of buf.\n");
    assert_eq!(marks(&source, &[("buf", "Vec<i64>")]), ["buf checked"]);
    assert_eq!(marks(&source, &[("buf", "LogosSeq<i64>")]), ["buf checked"]);
}

#[test]
fn a_counter_bound_anew_in_the_body_keeps_the_rule() {
    let nested = main_with("Repeat for i from 1 to 8:\n    Repeat for i from 1 to 20:\n        Set total to total + item i of buf.\n");
    assert_eq!(marks(&nested, FIXED), ["buf checked"]);
}

#[test]
fn an_access_inside_a_closure_keeps_the_rule() {
    let source = "## Main\nLet mutable buf be a new Seq of Int.\nLet mutable other be a new Seq of Int.\nLet mutable total be 0.\nRepeat for i from 1 to 8:\n    Let pick be (i: Int) -> item i of other.\n    Set total to total + item i of buf + pick(i + 1).\nShow total.\n";
    assert_eq!(
        marks(source, &[("buf", "[i64; 8]"), ("other", "[i64; 8]")]),
        ["buf raw", "other checked"]
    );
}

#[test]
fn bounded_count_proof_purity_refuses_a_parameter_shadowing_a_pure_function() {
    let source = "## To chosen (n: Int) -> Int:\n    Return n + 1.\n## To apply (chosen: fn(Int) -> Int) and (x: Int) -> Int:\n    Return chosen(x).\n## Main\nShow 0.\n";
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("function-shadow fixture parses");
        let purity = crate::optimize::purity::Purity::of_program(&stmts, interner);
        assert!(purity.is_pure(interner.lookup("chosen").unwrap()));
        assert!(
            !purity.is_pure(interner.lookup("apply").unwrap()),
            "a function parameter can invoke an impure callback despite its global namesake"
        );
    });
}

#[test]
fn bounded_count_proof_purity_refuses_a_parameter_shadowing_a_pure_builtin() {
    let source = "## To apply (abs: fn(Int) -> Int) and (x: Int) -> Int:\n    Return abs(x).\n## Main\nShow 0.\n";
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("builtin-shadow fixture parses");
        let purity = crate::optimize::purity::Purity::of_program(&stmts, interner);
        assert!(
            !purity.is_pure(interner.lookup("apply").unwrap()),
            "a function parameter is not the builtin with the same spelling"
        );
    });
}

fn count_is_certified(source: &str) -> bool {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("count proof fixture parses");
        let count = interner.lookup("count").expect("count binding");
        collect_bounded_count_accumulators(&stmts, &Default::default()).contains(&count)
    })
}

#[test]
fn bounded_count_proof_certifies_one_iteration_domain() {
    assert!(count_is_certified("## Main\nLet count be 0.\nLet i be 0.\nWhile i < 100:\n    Set count to count + 1.\n    Set i to i + 1.\nShow count.\n"));
}

#[test]
fn bounded_count_proof_rejects_nested_iteration_domains() {
    let source = r#"## Main
Let count be 0.
Let i be 0.
While i < 9223372036854775807:
    Let j be 0.
    While j < 2:
        Let k be 0.
        While k < 2:
            Set count to count + 1.
            Set k to k + 1.
        Set j to j + 1.
    Set i to i + 1.
Show count.
"#;
    assert!(
        !count_is_certified(source),
        "a bound per inner loop does not bound the accumulated outer work"
    );
}

#[test]
fn bounded_count_proof_rejects_effectful_expression_calls() {
    let source = r#"## No Inline
## To raise (count: mutable Int) -> Int:
    Set count to 2 ** 100.
    Return 0.
## Main
Let count be 0.
Let i be 0.
While i < 1:
    Show raise(count).
    Set count to count + 1.
    Set i to i + 1.
Show count.
"#;
    assert!(
        !count_is_certified(source),
        "a call may change an accumulator outside its visible increment"
    );
}

#[test]
fn bounded_count_proof_accepts_an_inclusive_word_bound_from_a_positive_start() {
    for start in [1, 2, 99] {
        let source = format!("## Main\nLet limit be 9223372036854775807.\nLet count be 0.\nLet i be {start}.\nWhile i <= limit:\n    Set count to count + 1.\n    Set i to i + 1.\nShow count.\n");
        assert!(
            count_is_certified(&source),
            "positive start {start} bounds the count by i64::MAX"
        );
    }
}

#[test]
fn bounded_count_proof_rejects_an_inclusive_word_bound_from_zero_or_negative() {
    for setup in [
        "Let i be 0.",
        "Let i be 0 - 1.",
        "Let i be 2.\nSet i to 0.",
        "Let i be 2.\nIf limit > 0:\n    Set i to 0.",
    ] {
        let source = format!("## Main\nLet limit be 9223372036854775807.\nLet count be 0.\n{setup}\nWhile i <= limit:\n    Set count to count + 1.\n    Set i to i + 1.\nShow count.\n");
        assert!(
            !count_is_certified(&source),
            "{setup}: the count may exceed i64::MAX"
        );
    }
}

#[test]
fn bounded_count_proof_rejects_an_exact_inclusive_bound() {
    let source = "## Main\nLet limit be 2 ** 100.\nLet count be 0.\nLet i be 2.\nWhile i <= limit:\n    Set count to count + 1.\n    Set i to i + 1.\nShow count.\n";
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("exact bound fixture parses");
        let limit = interner.lookup("limit").unwrap();
        let count = interner.lookup("count").unwrap();
        let exact = [limit].into_iter().collect();
        assert!(!collect_bounded_count_accumulators(&stmts, &exact).contains(&count));
    });
}

/// A value is simple — its Rust type the logical one, so tracked and inferred types may settle it — only where every
/// name it reads, and every list it indexes, has a type the emitter tracks: inference reads a name it does not know as
/// an `Int`, and a word a builtin returned is none.
#[test]
fn a_value_over_an_untracked_name_is_not_simple() {
    let mut interner = Interner::new();
    let (x, low_bit, xs, ys) = (interner.intern("x"), interner.intern("low_bit"), interner.intern("xs"), interner.intern("ys"));
    let types: HashMap<Symbol, String> =
        [(x, "i64".to_string()), (xs, "LogosSeq<i64>".to_string())].into_iter().collect();
    let (one, four) = (Expr::Literal(Literal::Number(1)), Expr::Literal(Literal::Number(4)));
    let (tracked, untracked) = (Expr::Identifier(x), Expr::Identifier(low_bit));
    let (list, unknown_list) = (Expr::Identifier(xs), Expr::Identifier(ys));
    let shifted = |left: &Expr| simple_value(&Expr::BinaryOp { op: BinaryOpKind::Shl, left, right: &four }, &types);

    assert!(simple_value(&one, &types));
    assert!(simple_value(&tracked, &types));
    assert!(!simple_value(&untracked, &types), "an untracked name may hold any representation");
    assert!(shifted(&tracked));
    assert!(!shifted(&untracked), "a shift of an untracked word is not an Int's");
    assert!(simple_value(&Expr::Not { operand: &tracked }, &types));
    assert!(!simple_value(&Expr::Not { operand: &untracked }, &types));

    assert!(simple_value(&Expr::Index { collection: &list, index: &tracked }, &types));
    assert!(!simple_value(&Expr::Index { collection: &list, index: &untracked }, &types), "the index is read too");
    assert!(
        !simple_value(&Expr::Index { collection: &unknown_list, index: &one }, &types),
        "an untracked list's item has no known type"
    );
    assert!(simple_value(&Expr::Length { collection: &unknown_list }, &types), "a length is an Int whatever the list");
}

/// Every type the runtime keeps `Copy` is read without a clone, and nothing that owns a heap allocation is. The
/// `copy::<T>()` calls are the runtime's own word: a type that stopped being `Copy` fails to compile here first.
#[test]
fn every_type_the_runtime_keeps_copy_is_read_without_a_clone() {
    fn copy<T: Copy>() {}
    use logicaffeine_base::word::{
        Lanes16Word16, Lanes16Word8, Lanes4Word32, Lanes4Word64, Lanes8Word32, Word16, Word32, Word64, Word8,
    };
    use logicaffeine_data::temporal::{LogosDate, LogosDuration, LogosMoment, LogosSpan, LogosTime};
    copy::<Word8>();
    copy::<Word16>();
    copy::<Word32>();
    copy::<Word64>();
    copy::<Lanes16Word8>();
    copy::<Lanes16Word16>();
    copy::<Lanes4Word32>();
    copy::<Lanes8Word32>();
    copy::<Lanes4Word64>();
    copy::<LogosDuration>();
    copy::<LogosDate>();
    copy::<LogosMoment>();
    copy::<LogosTime>();
    copy::<LogosSpan>();
    copy::<logicaffeine_data::LogosUuid>();
    let words = [
        "Word8",
        "Word16",
        "Word32",
        "Word64",
        "Lanes16Word8",
        "Lanes16Word16",
        "Lanes4Word32",
        "Lanes8Word32",
        "Lanes4Word64",
    ];
    assert_eq!(WORD_TYPES, words, "the word list is the runtime's words and lane vectors");
    let temporal = ["LogosDuration", "LogosDate", "LogosMoment", "LogosTime", "LogosSpan", "LogosUuid"];
    for ty in words.into_iter().chain(temporal) {
        assert!(is_copy_type(ty), "{ty} is Copy");
        assert!(has_copy_element_type(&format!("LogosSeq<{ty}>")), "an item of a list of {ty} is Copy");
        assert!(has_copy_element_type(&format!("Vec<{ty}>")), "an item of a vector of {ty} is Copy");
    }
    assert!(is_copy_type("Lanes4Word32|__hl:st_len"), "a hoisting tag does not change the type");
    let owning = ["String", "LogosSeq<i64>", "Vec<Word32>", "logicaffeine_data::LogosInt", "LogosDecimal", "LogosMoney", "Point"];
    for ty in owning {
        assert!(!is_copy_type(ty), "{ty} owns what it holds");
    }
    assert!(!has_copy_element_type("LogosSeq<String>"));
}

/// The types whose every value is one Rust type, and only those: an `Int`, a complex, a list and a rational each have
/// more than one representation, so inference alone never states them for a computed value.
#[test]
fn a_type_of_one_representation_is_stated_however_computed() {
    use crate::analysis::types::LogosType;
    for t in [
        LogosType::Duration,
        LogosType::Date,
        LogosType::Moment,
        LogosType::Time,
        LogosType::Span,
        LogosType::Decimal,
        LogosType::Modular,
        LogosType::Quantity,
        LogosType::Money,
        LogosType::Uuid,
    ] {
        assert!(one_representation(&t), "{t:?}");
        assert!(t.to_rust_type().starts_with("Logos"), "{t:?} is written as its runtime type");
    }
    for t in [
        LogosType::Int,
        LogosType::Float,
        LogosType::String,
        LogosType::Complex,
        LogosType::Rational,
        LogosType::Seq(Box::new(LogosType::Int)),
        LogosType::Unknown,
    ] {
        assert!(!one_representation(&t), "{t:?}");
    }
}

/// A value's names are tracked when every binding it reads has a type in the map, the clock aside.
#[test]
fn a_value_reads_only_tracked_names_or_the_clock() {
    let mut interner = Interner::new();
    let (pause, other, now) = (interner.intern("pause"), interner.intern("other"), interner.intern("now"));
    let types: HashMap<Symbol, String> = [(pause, "LogosDuration".to_string())].into_iter().collect();
    let (p, o, clock) = (Expr::Identifier(pause), Expr::Identifier(other), Expr::Identifier(now));
    let sum = |l, r| Expr::BinaryOp { op: BinaryOpKind::Add, left: l, right: r };
    assert!(names_tracked(&sum(&p, &p), &types, &interner));
    assert!(!names_tracked(&sum(&p, &o), &types, &interner), "`other` is not tracked");
    assert!(names_tracked(&sum(&clock, &p), &types, &interner), "`now` reads the clock");
    assert!(names_tracked(&Expr::Literal(Literal::Number(1)), &types, &interner));
}

/// A registered type's marker comment is left out to its own end, the markers it holds included.
#[test]
fn a_marker_holding_another_is_left_out_whole() {
    let nested = "LogosTuple /* logos_tuple: Set<LogosTuple /* logos_tuple: String,String */>,Set<String> */";
    assert_eq!(nameable(nested).as_deref(), Some("LogosTuple"));
    assert_eq!(nameable("LogosTuple /* logos_tuple: i64,i64 */").as_deref(), Some("LogosTuple"));
    assert_eq!(nameable("Vec<i64>").as_deref(), Some("Vec<i64>"));
    assert_eq!(nameable("TextBytes").as_deref(), Some("&[u8]"));
    assert_eq!(nameable("i64|__bigint"), None);
}

#[test]
fn a_tracked_type_writes_a_value_only_of_its_own_type() {
    let mut interner = Interner::new();
    let (x, word) = (interner.intern("x"), interner.intern("seven"));
    let types: HashMap<Symbol, String> = [(x, "f64".to_string())].into_iter().collect();
    let text = Expr::Literal(Literal::Text(word));
    let half = Expr::Literal(Literal::Float(1.5));
    assert!(!writes_value("String", &half, &interner, &types), "a Real is not the Text the name carries");
    assert!(writes_value("f64", &half, &interner, &types));
    assert!(writes_value("f32", &half, &interner, &types), "a float literal is any machine float");
    assert!(!writes_value("i64", &half, &interner, &types));
    assert!(writes_value("String", &text, &interner, &types));
    assert!(!writes_value("i64", &text, &interner, &types));
    let copy = Expr::Identifier(x);
    assert!(writes_value("f64", &copy, &interner, &types), "a copy is of its source's tracked type");
    assert!(!writes_value("String", &copy, &interner, &types));
    let doubled = Expr::BinaryOp { op: BinaryOpKind::Multiply, left: &copy, right: &half };
    assert!(writes_value("f64", &doubled, &interner, &types));
    assert!(!writes_value("String", &doubled, &interner, &types));
}

#[test]
fn an_item_of_a_borrowed_slice_is_of_the_slices_element_type() {
    let mut interner = Interner::new();
    let (xs, i) = (interner.intern("xs"), interner.intern("i"));
    let types: HashMap<Symbol, String> =
        [(xs, "&[f64]".to_string()), (i, "i64".to_string())].into_iter().collect();
    let (list, at) = (Expr::Identifier(xs), Expr::Identifier(i));
    let item = Expr::Index { collection: &list, index: &at };
    assert!(writes_value("f64", &item, &interner, &types), "an item of a `&[f64]` is an f64");
    assert!(!writes_value("i64", &item, &interner, &types));
    assert!(!writes_value("String", &item, &interner, &types));
    let difference = Expr::BinaryOp { op: BinaryOpKind::Subtract, left: &item, right: &item };
    assert!(writes_value("f64", &difference, &interner, &types));
    assert!(!writes_value("i64", &difference, &interner, &types));
}

#[test]
fn arithmetic_is_written_a_float_wherever_one_operand_is() {
    let mut interner = Interner::new();
    let (xs, i, n) = (interner.intern("xs"), interner.intern("i"), interner.intern("n"));
    let types: HashMap<Symbol, String> =
        [(xs, "[f64; 5]".to_string()), (i, "i64".to_string()), (n, "i64".to_string())].into_iter().collect();
    let (list, at, count) = (Expr::Identifier(xs), Expr::Identifier(i), Expr::Identifier(n));
    let item = Expr::Index { collection: &list, index: &at };
    let ints = Expr::BinaryOp { op: BinaryOpKind::Multiply, left: &at, right: &count };
    let mixed = Expr::BinaryOp { op: BinaryOpKind::Add, left: &item, right: &ints };
    assert_eq!(number_written(&item, &interner, &types), Some("f64"), "an item of an array of reals");
    assert_eq!(number_written(&ints, &interner, &types), Some("i64"));
    assert_eq!(number_written(&mixed, &interner, &types), Some("f64"), "a real operand makes the sum a real");
    let less = Expr::BinaryOp { op: BinaryOpKind::Lt, left: &at, right: &count };
    assert_eq!(number_written(&less, &interner, &types), None, "a comparison is no number");
    assert!(writes_value("bool", &less, &interner, &types));
}

#[test]
fn integer_literals_write_any_machine_integer_that_holds_them() {
    let interner = Interner::new();
    let types: HashMap<Symbol, String> = HashMap::new();
    let small = Expr::Literal(Literal::Number(200));
    let negative = Expr::Literal(Literal::Number(-5));
    let big = Expr::Literal(Literal::Number(1 << 40));
    for ty in ["i64", "i32", "i16", "u8", "u16", "u32", "u64", "usize"] {
        assert!(writes_value(ty, &small, &interner, &types), "200 is a {ty}");
    }
    assert!(!writes_value("i8", &small, &interner, &types), "200 is past an i8");
    assert!(!writes_value("u8", &negative, &interner, &types), "-5 is no u8");
    assert!(!writes_value("u64", &negative, &interner, &types), "-5 is no u64");
    assert!(writes_value("i8", &negative, &interner, &types));
    assert!(!writes_value("i32", &big, &interner, &types), "2^40 is past an i32");
    assert!(writes_value("i64", &big, &interner, &types));
    assert!(!writes_value("f64", &small, &interner, &types), "an integer literal is no float");
    assert!(!writes_value("String", &small, &interner, &types));
    let product = Expr::BinaryOp { op: BinaryOpKind::Multiply, left: &small, right: &negative };
    assert!(writes_value("i16", &product, &interner, &types), "-1000 is an i16");
    assert!(!writes_value("u32", &product, &interner, &types), "-1000 is no u32");
}

#[test]
fn float_literals_are_float_literals_joined_by_arithmetic() {
    let (a, b) = (Expr::Literal(Literal::Float(1.5)), Expr::Literal(Literal::Float(2.0)));
    let n = Expr::Literal(Literal::Number(2));
    assert!(float_literals(&a));
    assert!(float_literals(&Expr::BinaryOp { op: BinaryOpKind::Divide, left: &a, right: &b }));
    assert!(!float_literals(&Expr::BinaryOp { op: BinaryOpKind::Multiply, left: &a, right: &n }));
    assert!(!float_literals(&Expr::BinaryOp { op: BinaryOpKind::Lt, left: &a, right: &b }));
    assert!(!float_literals(&n));
}

#[test]
fn an_empty_value_is_element_free_only_where_every_use_takes_any_element() {
    let source = "## Main\nLet xs be [].\nShow length of xs.\nLet n be none.\nShow n.\nIf n is none:\n    Show 1.\nIf n is equal to nothing:\n    Show 5.\nInspect n:\n    When Some (k):\n        Show 2.\n    When None:\n        Show 3.\nLet ys be [].\nPush 1 to ys.\nLet mutable m be none.\nSet m to some(3).\nLet zs be [].\nShow length of zs.\nLet zs be [1].\nLet w be none.\nInspect w:\n    When Some (j):\n        Show j + 1.\n    When None:\n        Show 4.\n";
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let free = element_free_lets(stmts, interner);
        let named: Vec<&str> = stmts
            .iter()
            .filter(|s| free.contains(&(*s as *const Stmt as usize)))
            .map(|s| match s {
                Stmt::Let { var, .. } => interner.resolve(*var),
                _ => "",
            })
            .collect();
        assert_eq!(named, vec!["xs", "n"], "shown, measured, asked `is none` and inspected without reading: element free");
    });
}
