use super::*;

/// Whether a range oracle is installed now.
fn range_oracle_installed() -> bool {
    RANGE_ORACLE.with(|c| c.borrow().is_some())
}

#[test]
fn test_literal_number() {
    let interner = Interner::new();
    let synced_vars = HashSet::new();
    let expr = Expr::Literal(Literal::Number(42));
    assert_eq!(codegen_expr(&expr, &interner, &synced_vars), "42");
}

#[test]
fn test_literal_boolean() {
    let interner = Interner::new();
    let synced_vars = HashSet::new();
    assert_eq!(
        codegen_expr(
            &Expr::Literal(Literal::Boolean(true)),
            &interner,
            &synced_vars
        ),
        "true"
    );
    assert_eq!(
        codegen_expr(
            &Expr::Literal(Literal::Boolean(false)),
            &interner,
            &synced_vars
        ),
        "false"
    );
}

#[test]
fn test_literal_nothing() {
    let interner = Interner::new();
    let synced_vars = HashSet::new();
    assert_eq!(
        codegen_expr(&Expr::Literal(Literal::Nothing), &interner, &synced_vars),
        "()"
    );
}

/// The range oracle holds facts keyed by the addresses of one program's nodes, including what that program's
/// own guards established while it was emitted. It ends with its compile: after each of two programs compiled
/// back to back no oracle is installed, and the second compiles to exactly what it compiles to alone.
#[test]
fn the_range_oracle_ends_with_its_compile() {
    let tiled = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"40\").\nLet mutable a be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n * n:\n    Push i % 7 to a.\n    Push 0 to c.\n    Set i to i + 1.\nSet i to 0.\nWhile i is less than n:\n    Let mutable k be 0.\n    While k is less than n:\n        Let mutable j be 0.\n        While j is less than n:\n            Let idx be i * n + j + 1.\n            Set item idx of c to (item idx of c) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of a).\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nShow item 1 of c.\n";
    let plain = "## Main\nLet xs be [10, 20, 30].\nLet mutable i be 1.\nWhile i is at most 4:\n    Show item i of xs.\n    Set i to i + 1.\n";
    let alone = crate::compile::compile_to_rust(plain).expect("compiles");
    assert!(
        !range_oracle_installed(),
        "the range oracle outlived its compile"
    );
    let first = crate::compile::compile_to_rust(tiled).expect("compiles");
    assert!(
        first.contains("step_by"),
        "the first program tiles, establishing facts as it is emitted:\n{first}"
    );
    assert!(
        !range_oracle_installed(),
        "the range oracle outlived its compile"
    );
    let after = crate::compile::compile_to_rust(plain).expect("compiles");
    assert!(
        !range_oracle_installed(),
        "the range oracle outlived its compile"
    );
    assert_eq!(
        after, alone,
        "a program compiles the same after another as it does alone"
    );
}

/// A machine-scalar binding is formatted from a copy; anything else is formatted as it is written.
#[test]
fn a_shown_machine_scalar_binding_is_a_copy() {
    let mut interner = Interner::new();
    let (x, s, t) = (interner.intern("x"), interner.intern("s"), interner.intern("type"));
    let types: HashMap<Symbol, String> = [
        (x, "i64".to_string()),
        (s, "String".to_string()),
        (t, "f64".to_string()),
    ]
    .into_iter()
    .collect();
    let id = Expr::Identifier(x);
    assert_eq!(shown(&id, "x", &types), "{ x }");
    assert_eq!(shown(&Expr::Identifier(t), "r#type", &types), "{ r#type }", "a raw identifier is a name");
    assert_eq!(shown(&Expr::Identifier(s), "s", &types), "s", "a `String` would move");
    assert_eq!(shown(&id, "x.get()", &types), "x.get()", "only a name read as itself is copied");
    assert_eq!(shown(&id, "*x", &types), "*x");
    assert_eq!(shown(&Expr::Literal(Literal::Number(4)), "4", &types), "4", "a literal is a value already");
    for ty in ["bool", "u8", "char", "Word32", "i64|__hl:xs"] {
        let typed: HashMap<Symbol, String> = [(x, ty.to_string())].into_iter().collect();
        assert_eq!(shown(&id, "x", &typed), "{ x }", "{ty}");
    }
    assert_eq!(shown(&id, "x", &HashMap::new()), "x", "an unknown type is not copied");
}

/// A concatenation's literals are written into its format string, braces doubled and Rust escapes kept; the text is
/// the literal itself when no hole is left.
#[test]
fn a_concatenation_writes_its_literals_into_the_format() {
    let text = Formatted { fmt: "total: {}, half: {}!".to_string(), args: vec!["{ total }".to_string(), "h".to_string()] };
    assert_eq!(text.format(), "format!(\"total: {}, half: {}!\", { total }, h)");
    assert_eq!(text.format_args(), "format_args!(\"total: {}, half: {}!\", { total }, h)");
    let literal = Formatted { fmt: "a {{b}} \\\"c\\\"".to_string(), args: vec![] };
    assert_eq!(literal.format(), "::std::string::String::from(\"a {b} \\\"c\\\"\")");
    assert_eq!(literal.format_args(), "format_args!(\"a {{b}} \\\"c\\\"\")");
}

/// The type of what the emitter writes, read from the form it writes: a construction names its type, a list or set
/// literal its items' shared one, `sqrt` the root it takes; anything a binding's tracked type would have to vouch for
/// has none.
#[test]
fn a_written_form_fixes_its_type() {
    use crate::analysis::registry::{TypeDef, VariantDef};
    use crate::ast::stmt::TypeExpr;
    let mut interner = Interner::new();
    let (point, shape, pair, circle) =
        (interner.intern("Point"), interner.intern("Shape"), interner.intern("Pair"), interner.intern("Circle"));
    let (seq, map, int, text, set_of, sqrt, x, a, b, c, z) = (
        interner.intern("Seq"),
        interner.intern("Map"),
        interner.intern("Int"),
        interner.intern("Text"),
        interner.intern("setOf"),
        interner.intern("sqrt"),
        interner.intern("x"),
        interner.intern("a"),
        interner.intern("b"),
        interner.intern("c"),
        interner.intern("z"),
    );
    let t = interner.intern("T");
    let mut registry = TypeRegistry::new();
    let record = |generics: Vec<Symbol>| TypeDef::Struct { fields: vec![], generics, is_portable: false, is_shared: false };
    registry.register(point, record(vec![]));
    registry.register(pair, record(vec![t]));
    registry.register(
        shape,
        TypeDef::Enum {
            variants: vec![VariantDef { name: circle, fields: vec![] }],
            generics: vec![],
            is_portable: false,
            is_shared: false,
        },
    );
    let types: HashMap<Symbol, String> = [
        (a, "Set<i64>".to_string()),
        (b, "Set<i64>".to_string()),
        (c, "Set<String>".to_string()),
        (x, "f64".to_string()),
        (z, "LogosComplexF".to_string()),
    ]
    .into_iter()
    .collect();
    let of = |e: &Expr| written_type(e, &types, &registry, &interner);

    let (one, two, half, word) = (
        Expr::Literal(Literal::Number(1)),
        Expr::Literal(Literal::Number(2)),
        Expr::Literal(Literal::Float(0.5)),
        Expr::Literal(Literal::Text(text)),
    );
    assert_eq!(of(&one).as_deref(), Some("i64"));
    assert_eq!(of(&word).as_deref(), Some("String"));

    let fresh_point = Expr::New { type_name: point, type_args: vec![], init_fields: vec![] };
    let built_point = Expr::New { type_name: point, type_args: vec![], init_fields: vec![(x, &one)] };
    let fresh_pair = Expr::New { type_name: pair, type_args: vec![], init_fields: vec![] };
    let built_pair = Expr::New { type_name: pair, type_args: vec![TypeExpr::Primitive(int)], init_fields: vec![(x, &one)] };
    let ints = Expr::New { type_name: seq, type_args: vec![TypeExpr::Primitive(int)], init_fields: vec![] };
    let counts = Expr::New {
        type_name: map,
        type_args: vec![TypeExpr::Primitive(text), TypeExpr::Primitive(int)],
        init_fields: vec![],
    };
    let bare_seq = Expr::New { type_name: seq, type_args: vec![], init_fields: vec![] };
    assert_eq!(of(&fresh_point).as_deref(), Some("Point"));
    assert_eq!(of(&built_point).as_deref(), Some("Point"));
    assert_eq!(of(&fresh_pair), None, "a generic record built without its arguments names no whole type");
    assert_eq!(of(&built_pair), None, "a generic record's fields are written without its arguments");
    assert_eq!(of(&ints).as_deref(), Some("Seq<i64>"));
    assert_eq!(of(&counts).as_deref(), Some("Map<String, i64>"));
    assert_eq!(of(&bare_seq), None, "a library container built without arguments names no whole type");

    let variant = Expr::NewVariant { enum_name: shape, variant: circle, fields: vec![] };
    assert_eq!(of(&variant).as_deref(), Some("Shape"));
    assert_eq!(of(&Expr::List(vec![&variant, &variant])).as_deref(), Some("LogosSeq<Shape>"));
    assert_eq!(of(&Expr::List(vec![&one, &two])).as_deref(), Some("LogosSeq<i64>"));
    assert_eq!(of(&Expr::List(vec![&one, &half])), None, "items of two types share none");
    assert_eq!(of(&Expr::List(vec![])), None, "an empty list has no item to read");
    assert_eq!(of(&Expr::List(vec![&one, &Expr::Identifier(x)])), None, "a name's type is tracked, not written");

    assert_eq!(of(&Expr::Call { function: set_of, args: vec![&one, &two] }).as_deref(), Some("Set<i64>"));
    assert_eq!(of(&Expr::Call { function: set_of, args: vec![&one, &word] }), None, "items of two types share none");

    let (real, complex) = (Expr::Identifier(x), Expr::Identifier(z));
    assert_eq!(of(&Expr::Call { function: sqrt, args: vec![&real] }).as_deref(), Some("f64"));
    assert_eq!(
        of(&Expr::Call { function: sqrt, args: vec![&one] }).as_deref(),
        Some("f64"),
        "an integer operand is read as an `f64`"
    );
    assert_eq!(of(&Expr::Call { function: sqrt, args: vec![&complex] }).as_deref(), Some("LogosComplexF"));

    let (ea, eb, ec) = (Expr::Identifier(a), Expr::Identifier(b), Expr::Identifier(c));
    assert_eq!(of(&Expr::Union { left: &ea, right: &eb }).as_deref(), Some("Set<i64>"));
    assert_eq!(of(&Expr::Intersection { left: &ea, right: &eb }).as_deref(), Some("Set<i64>"));
    assert_eq!(of(&Expr::Union { left: &ea, right: &ec }), None, "sets tracked as two types vouch for neither");
    assert_eq!(of(&Expr::Identifier(x)), None, "a name's type is tracked, not written");
}

/// A builtin's word is the one the builtins' shared record names: a fixed width's word for a builtin that builds or
/// answers one, the first argument's word for one that passes its argument's through, and none for a builtin that
/// returns no word or a program's own function of the name (declarer wins).
#[test]
fn a_builtins_word_is_the_one_its_record_names() {
    let mut interner = Interner::new();
    for (name, word) in [
        ("word8", "Word8"),
        ("word16", "Word16"),
        ("word32", "Word32"),
        ("word64", "Word64"),
        ("word64Shl", "Word64"),
        ("word64Shr", "Word64"),
        ("word64And", "Word64"),
        ("word32Shr", "Word32"),
    ] {
        let f = interner.intern(name);
        assert_eq!(builtin_word(f, None, &interner), Some(word), "{name}");
        assert_eq!(builtin_word(f, Some("Word8"), &interner), Some(word), "{name} answers its own width");
    }
    for name in ["word_and", "word_or", "word_not", "rotl", "rotr"] {
        let f = interner.intern(name);
        assert_eq!(builtin_word(f, Some("Word32"), &interner), Some("Word32"), "{name}");
        assert_eq!(builtin_word(f, Some("Lanes4Word32"), &interner), Some("Lanes4Word32"), "{name} over lanes");
        assert_eq!(builtin_word(f, None, &interner), None, "{name} of an operand of no known word");
    }
    for (name, lanes) in [("splat16Word16", "Lanes16Word16"), ("lanes4Of", "Lanes4Word32"), ("packus16", "Lanes16Word8")] {
        assert_eq!(builtin_word(interner.intern(name), None, &interner), Some(lanes), "{name} returns its vector");
    }
    for name in ["nttBcastLo", "nttBcastHi", "nttBlend"] {
        let f = interner.intern(name);
        assert_eq!(builtin_word(f, Some("Lanes16Word16"), &interner), Some("Lanes16Word16"), "{name} keeps its lanes");
    }
    for name in ["intOfWord32", "seqOfLanes16", "hsumLanes4", "sqrt", "word128", "length"] {
        assert_eq!(builtin_word(interner.intern(name), Some("Word32"), &interner), None, "{name} returns no word");
    }

    let (one, two) = (Expr::Literal(Literal::Number(1)), Expr::Literal(Literal::Number(2)));
    let (word32, word64, word_and, rotl, int_of, splat, unpack) = (
        interner.intern("word32"),
        interner.intern("word64"),
        interner.intern("word_and"),
        interner.intern("rotl"),
        interner.intern("intOfWord32"),
        interner.intern("splat16Word16"),
        interner.intern("seqOfLanes16"),
    );
    let of = |e: &Expr| written_type(e, &HashMap::new(), &TypeRegistry::new(), &interner);
    let (a, b) = (Expr::Call { function: word32, args: vec![&one] }, Expr::Call { function: word32, args: vec![&two] });
    let wide = Expr::Call { function: word64, args: vec![&one] };
    assert_eq!(of(&a).as_deref(), Some("Word32"));
    assert_eq!(of(&Expr::Call { function: word_and, args: vec![&a, &b] }).as_deref(), Some("Word32"));
    assert_eq!(of(&Expr::Call { function: rotl, args: vec![&wide, &two] }).as_deref(), Some("Word64"));
    assert_eq!(of(&Expr::Call { function: word_and, args: vec![&one, &two] }), None, "an Int operand is no word");
    assert_eq!(of(&Expr::Call { function: int_of, args: vec![&a] }).as_deref(), Some("i64"));
    let lanes = Expr::Call { function: splat, args: vec![&one] };
    assert_eq!(of(&lanes).as_deref(), Some("Lanes16Word16"));
    assert_eq!(of(&Expr::Call { function: unpack, args: vec![&lanes] }).as_deref(), Some("LogosSeq<i64>"));
    assert_eq!(of(&Expr::Call { function: unpack, args: vec![&lanes, &one] }), None, "a call of another arity");

    set_fn_return_types(&[(word32, "i64".to_string()), (int_of, "Text".to_string())].into_iter().collect());
    let declared = (builtin_word(word32, None, &interner), of(&a), of(&Expr::Call { function: int_of, args: vec![&a] }));
    set_fn_return_types(&HashMap::new());
    assert_eq!(declared, (None, None, None), "a program's own `word32` and `intOfWord32` are its functions");
}

#[test]
fn a_promoted_binding_its_arithmetic_and_a_promoting_call_are_logos_ints_already() {
    let mut interner = Interner::new();
    let (count, n, f, g) = (interner.intern("count"), interner.intern("n"), interner.intern("f"), interner.intern("g"));
    let types: HashMap<Symbol, String> =
        [(count, "i64|__bigint".to_string()), (n, "i64".to_string())].into_iter().collect();
    let (big, word, one) = (Expr::Identifier(count), Expr::Identifier(n), Expr::Literal(Literal::Number(1)));
    assert!(tolerant_logos_int(&big, &interner, &types), "a promoted binding");
    assert!(!tolerant_logos_int(&word, &interner, &types), "a machine word");
    assert!(!tolerant_logos_int(&one, &interner, &types), "a literal");
    for op in [BinaryOpKind::Add, BinaryOpKind::Subtract, BinaryOpKind::Multiply, BinaryOpKind::Divide, BinaryOpKind::Modulo] {
        let with_big = Expr::BinaryOp { op, left: &big, right: &one };
        assert!(tolerant_logos_int(&with_big, &interner, &types), "{op:?} with a promoted operand");
        let words = Expr::BinaryOp { op, left: &word, right: &one };
        assert!(!tolerant_logos_int(&words, &interner, &types), "{op:?} of machine words");
    }
    for op in [BinaryOpKind::Lt, BinaryOpKind::Eq, BinaryOpKind::BitAnd, BinaryOpKind::Shl] {
        let compared = Expr::BinaryOp { op, left: &big, right: &one };
        assert!(!tolerant_logos_int(&compared, &interner, &types), "{op:?} answers no LogosInt");
    }
    let (huge, two, hundred) = (Expr::Literal(Literal::Number(i64::MAX)), Expr::Literal(Literal::Number(2)), Expr::Literal(Literal::Number(100)));
    let past = Expr::BinaryOp { op: BinaryOpKind::Multiply, left: &huge, right: &two };
    assert!(tolerant_logos_int(&past, &interner, &types), "two literals whose product passes the word fold to a LogosInt");
    let within = Expr::BinaryOp { op: BinaryOpKind::Multiply, left: &two, right: &hundred };
    assert!(!tolerant_logos_int(&within, &interner, &types), "two literals whose product fits fold to a word");
    let power = Expr::BinaryOp { op: BinaryOpKind::Pow, left: &two, right: &hundred };
    assert!(tolerant_logos_int(&power, &interner, &types), "`2 ^ 100` is `logos_pow_exact`");
    let small_power = Expr::BinaryOp { op: BinaryOpKind::Pow, left: &two, right: &two };
    assert!(!tolerant_logos_int(&small_power, &interner, &types), "`2 ^ 2` folds to the word 4");
    let word_power = Expr::BinaryOp { op: BinaryOpKind::Pow, left: &word, right: &two };
    assert!(tolerant_logos_int(&word_power, &interner, &types), "an Int power the emission does not fold");
    set_bigint_returning_fns(&[f].into_iter().collect());
    let promoting = tolerant_logos_int(&Expr::Call { function: f, args: vec![&word] }, &interner, &types);
    let plain = tolerant_logos_int(&Expr::Call { function: g, args: vec![&word] }, &interner, &types);
    set_bigint_returning_fns(&HashSet::new());
    assert!(promoting, "a call to a function that returns a LogosInt");
    assert!(!plain, "a call to one that returns a word");

    assert_eq!(as_logos_int(&big, &interner, &types, || "count".to_string()), "count");
    assert_eq!(as_logos_int(&big, &interner, &types, || "(count.add(&count))".to_string()), "count.add(&count)");
    assert_eq!(as_logos_int(&word, &interner, &types, || "n".to_string()), "logicaffeine_data::LogosInt::from(n)");
}

#[test]
fn only_a_ring_or_bitwise_operation_keeps_its_operands_low_bits() {
    use BinaryOpKind::*;
    for op in [Add, Subtract, Multiply, BitAnd, BitOr, BitXor] {
        assert!(keeps_low_bits(op), "{op:?} keeps its operands' low bits");
    }
    for op in [Divide, Modulo, Shl, Shr, Lt, Gt, LtEq, GtEq, Eq, NotEq, And, Or] {
        assert!(!keeps_low_bits(op), "{op:?} reads an operand whole");
    }
}

#[test]
fn literals_and_their_min_and_max_have_no_machine_type_of_their_own() {
    let mut interner = Interner::new();
    let (min, max, abs, x) = (interner.intern("min"), interner.intern("max"), interner.intern("abs"), interner.intern("x"));
    let (a, b) = (Expr::Literal(Literal::Number(-42)), Expr::Literal(Literal::Number(7)));
    let name = Expr::Identifier(x);
    assert!(!type_fixed(&a, &interner));
    let sum = Expr::BinaryOp { op: BinaryOpKind::Add, left: &a, right: &b };
    assert!(!type_fixed(&sum, &interner), "literals joined by an operator");
    let least = Expr::Call { function: min, args: vec![&a, &b] };
    assert!(!type_fixed(&least, &interner), "`(-42).min(7)` is of no type of its own");
    let most = Expr::Call { function: max, args: vec![&least, &b] };
    assert!(!type_fixed(&most, &interner));
    let with_name = Expr::Call { function: min, args: vec![&a, &name] };
    assert!(type_fixed(&with_name, &interner), "a binding fixes `min`'s type");
    let scaled = Expr::BinaryOp { op: BinaryOpKind::Multiply, left: &name, right: &b };
    assert!(type_fixed(&scaled, &interner));
    let absolute = Expr::Call { function: abs, args: vec![&a] };
    assert!(type_fixed(&absolute, &interner), "`abs` writes its receiver typed");
}
