use super::*;

/// The program bindings each function of `source` names itself, by function name, sorted.
fn reached(source: &str) -> Vec<(String, Vec<String>)> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("the program parses");
        let functions: HashSet<Symbol> = stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::FunctionDef { name, .. } => Some(*name),
                _ => None,
            })
            .collect();
        let mut resolver = Resolver::new(&functions);
        for (at, s) in stmts.iter().enumerate() {
            if !matches!(s, Stmt::FunctionDef { .. }) {
                resolver.root_statement(at, s);
            }
        }
        for s in stmts {
            if let Stmt::FunctionDef {
                name, params, body, ..
            } = s
            {
                resolver.function(*name, params.iter().map(|(p, _)| *p), body);
            }
        }
        let mut out: Vec<(String, Vec<String>)> = resolver
            .reached
            .iter()
            .filter(|(_, names)| !names.is_empty())
            .map(|(f, names)| {
                let mut names: Vec<String> = names
                    .iter()
                    .map(|n| interner.resolve(*n).to_string())
                    .collect();
                names.sort();
                (interner.resolve(*f).to_string(), names)
            })
            .collect();
        out.sort();
        out
    })
}

fn row(f: &str, names: &[&str]) -> (String, Vec<String>) {
    (f.to_string(), names.iter().map(|n| n.to_string()).collect())
}

#[test]
fn a_function_reaches_the_root_binding_it_reads_and_the_one_it_sets() {
    let src = "## To bump:\n    Set counter to counter + step.\n\n## Main\nLet step be 2.\nLet mutable counter be 0.\nbump().\n";
    assert_eq!(reached(src), vec![row("bump", &["counter", "step"])]);
}

#[test]
fn a_parameter_of_the_same_name_is_the_functions_own() {
    let src = "## To shadow (counter: Int) -> Int:\n    Return counter + 1.\n\n## Main\nLet counter be 10.\nShow shadow(1).\n";
    assert_eq!(reached(src), vec![]);
}

#[test]
fn a_functions_own_let_is_its_own_from_where_it_stands() {
    // The first read is the program's `x`; after the function's own `Let x`, `x` is the local.
    let src = "## To peek -> Int:\n    Show x.\n    Let x be 5.\n    Return x.\n\n## Main\nLet x be 1.\nShow peek().\n";
    assert_eq!(reached(src), vec![row("peek", &["x"])]);
}

#[test]
fn a_block_binding_of_main_is_no_program_binding() {
    let src = "## To peek -> Int:\n    Return inner.\n\n## Main\nIf true:\n    Let inner be 5.\n    Show peek().\n";
    assert_eq!(reached(src), vec![]);
}

#[test]
fn a_loop_variable_and_an_arm_binding_are_the_blocks_own() {
    let src = "## To walk (xs: Seq of Int) -> Int:\n    Let mutable s be 0.\n    Repeat for n in xs:\n        Set s to s + n.\n    Return s.\n\n\
               ## Main\nLet n be 3.\nLet s be 4.\nShow walk([1, 2]).\n";
    assert_eq!(reached(src), vec![]);
}

#[test]
fn a_closure_sees_the_functions_locals_before_the_programs_bindings() {
    let src = "## To make -> Int:\n    Let k be 7.\n    Let f be (y: Int) -> y + k + base.\n    Return f(1).\n\n\
               ## Main\nLet k be 1.\nLet base be 100.\nShow make().\n";
    assert_eq!(reached(src), vec![row("make", &["base"])]);
}

#[test]
fn a_field_type_is_the_checked_type_written_as_a_record_declares_one() {
    let mut interner = Interner::new();
    let text = interner.intern("Text");
    let int = interner.intern("Int");
    let seq = interner.intern("Seq");
    let map = interner.intern("Map");
    let boxed = interner.intern("Box");
    assert_eq!(
        field_type(&InferType::Int, &mut interner),
        Some(FieldType::Primitive(int))
    );
    assert_eq!(
        field_type(&InferType::Seq(Box::new(InferType::String)), &mut interner),
        Some(FieldType::Generic {
            base: seq,
            params: vec![FieldType::Primitive(text)]
        })
    );
    assert_eq!(
        field_type(
            &InferType::Map(Box::new(InferType::String), Box::new(InferType::Int)),
            &mut interner
        ),
        Some(FieldType::Generic {
            base: map,
            params: vec![FieldType::Primitive(text), FieldType::Primitive(int)]
        })
    );
    assert_eq!(
        field_type(
            &InferType::UserDefined(boxed, vec![InferType::Int]),
            &mut interner
        ),
        Some(FieldType::Generic {
            base: boxed,
            params: vec![FieldType::Primitive(int)]
        })
    );
    assert_eq!(
        field_type(&InferType::UserDefined(boxed, vec![]), &mut interner),
        Some(FieldType::Named(boxed))
    );
    assert_eq!(
        field_type(&InferType::Unknown, &mut interner),
        None,
        "an undecided type is no field type"
    );
}
