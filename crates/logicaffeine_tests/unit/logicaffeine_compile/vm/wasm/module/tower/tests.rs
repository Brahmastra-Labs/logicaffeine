use super::*;

/// The wasm32 value type a Rust parameter or result type of the runtime crosses as; a function pointer, written out
/// or by an alias `pointers` names, crosses as its table index.
fn valtype(rust: &str, pointers: &[String]) -> u8 {
    let rust = rust.trim();
    if rust.starts_with('*')
        || matches!(rust, "usize" | "i32" | "u32" | "isize")
        || rust.contains("extern \"C\" fn")
        || pointers.iter().any(|p| p == rust)
    {
        I32
    } else if matches!(rust, "i64" | "u64") {
        I64
    } else if rust == "f64" {
        F64
    } else {
        panic!("a runtime type with no wasm32 value type: `{rust}`")
    }
}

/// Every `extern "C"` function of `source` named `logos_rt_*`, with its wasm32 parameter and result types.
fn functions(source: &str) -> std::collections::BTreeMap<String, (Vec<u8>, Vec<u8>)> {
    let pointers: Vec<String> = source
        .split("pub type ")
        .skip(1)
        .filter_map(|alias| alias.split_once(';').map(|(declared, _)| declared))
        .filter_map(|declared| declared.split_once('='))
        .filter(|(_, ty)| ty.contains("extern \"C\" fn"))
        .map(|(name, _)| name.trim().to_string())
        .collect();
    let mut out = std::collections::BTreeMap::new();
    let mut rest = source;
    while let Some(at) = rest.find("extern \"C\" fn logos_rt_") {
        let from = &rest[at + "extern \"C\" fn ".len()..];
        let open = from.find('(').expect("a parameter list");
        let name = from[..open].to_string();
        let mut depth = 0usize;
        let mut close = open;
        for (i, c) in from[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let params: Vec<u8> = from[open + 1..close]
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| valtype(p.split_once(':').expect("`name: type`").1, &pointers))
            .collect();
        let after = &from[close + 1..];
        let body = after.find('{').expect("a body");
        let results = match after[..body].trim().strip_prefix("->") {
            Some(ret) => vec![valtype(ret, &pointers)],
            None => Vec::new(),
        };
        out.insert(name, (params, results));
        rest = &after[body..];
    }
    out
}

/// The data crate's C ABI, its macro tables included: two handles in and one out for `binary!` and `relation!`,
/// one in and one out for `unary!`.
fn abi() -> std::collections::BTreeMap<String, (Vec<u8>, Vec<u8>)> {
    let source = include_str!("../../../../../../../logicaffeine_data/src/abi.rs");
    let mut entries = functions(source);
    for (table, arity) in [("binary! {", 2), ("relation! {", 2), ("unary! {", 1)] {
        let at = source.find(table).unwrap_or_else(|| panic!("the ABI has no `{table}` table"));
        let body = &source[at + table.len()..];
        let body = &body[..body.find('}').expect("a closed table")];
        for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (name, _) = line.split_once("=>").expect("`name => method;`");
            entries.insert(name.trim().to_string(), (vec![I32; arity], vec![I32]));
        }
    }
    entries
}

/// The linked runtime's own functions: the source `link.rs` builds the runtime object from.
fn adapters() -> std::collections::BTreeMap<String, (Vec<u8>, Vec<u8>)> {
    let link = include_str!("../../../../../../../logicaffeine_compile/src/vm/wasm/link.rs");
    let start = link.find("const BIGINT_RUNTIME_SRC: &str = r#\"").expect("the runtime source");
    let body = &link[start..];
    let end = body.find("\"#;").expect("the end of the runtime source");
    functions(&body[..end])
}

#[test]
fn every_value_import_is_a_function_the_runtime_defines_with_its_signature() {
    let (abi, adapters) = (abi(), adapters());
    let mut wrong = Vec::new();
    for h in IMPORTS.iter().copied().chain([HostFn::RationalCmp]) {
        if h == HostFn::PrintText {
            continue;
        }
        let name = h.field();
        let defined = abi.get(name).or_else(|| adapters.get(name));
        match defined {
            None => wrong.push(format!("{name}: no runtime function defines it")),
            Some((params, results)) if *params != h.params() || *results != h.results() => wrong.push(format!(
                "{name}: the module imports ({:?}) -> ({:?}), the runtime defines ({params:?}) -> ({results:?})",
                h.params(),
                h.results()
            )),
            Some(_) => {}
        }
    }
    assert!(wrong.is_empty(), "the module and its runtime disagree:\n{}", wrong.join("\n"));
}

#[test]
fn the_census_reads_every_kind_of_runtime_function() {
    let abi = abi();
    assert_eq!(abi.get("logos_rt_value_item_at"), Some(&(vec![I32, I64], vec![I32])));
    assert_eq!(abi.get("logos_rt_value_float"), Some(&(vec![F64], vec![I32])));
    assert_eq!(abi.get("logos_rt_value_lt"), Some(&(vec![I32, I32], vec![I32])));
    assert_eq!(abi.get("logos_rt_value_sqrt"), Some(&(vec![I32], vec![I32])));
    assert_eq!(abi.get("logos_rt_value_drop"), Some(&(vec![I32], vec![])));
    let adapters = adapters();
    assert_eq!(adapters.get("logos_rt_value_length_i64"), Some(&(vec![I32], vec![I64])));
    assert_eq!(adapters.get("logos_rt_bigint_from_i64"), Some(&(vec![I64], vec![I32])));
}

#[test]
fn every_runtime_operation_is_imported_under_its_own_name() {
    let ops = [
        Op::Add { dst: 0, lhs: 1, rhs: 2 },
        Op::Sub { dst: 0, lhs: 1, rhs: 2 },
        Op::Mul { dst: 0, lhs: 1, rhs: 2 },
        Op::Div { dst: 0, lhs: 1, rhs: 2 },
        Op::ExactDiv { dst: 0, lhs: 1, rhs: 2 },
        Op::FloorDiv { dst: 0, lhs: 1, rhs: 2 },
        Op::Mod { dst: 0, lhs: 1, rhs: 2 },
        Op::Pow { dst: 0, lhs: 1, rhs: 2 },
        Op::Lt { dst: 0, lhs: 1, rhs: 2 },
        Op::Gt { dst: 0, lhs: 1, rhs: 2 },
        Op::LtEq { dst: 0, lhs: 1, rhs: 2 },
        Op::GtEq { dst: 0, lhs: 1, rhs: 2 },
        Op::Eq { dst: 0, lhs: 1, rhs: 2 },
        Op::NotEq { dst: 0, lhs: 1, rhs: 2 },
        Op::ApproxEq { dst: 0, lhs: 1, rhs: 2 },
        Op::BitAnd { dst: 0, lhs: 1, rhs: 2 },
        Op::BitOr { dst: 0, lhs: 1, rhs: 2 },
        Op::BitXor { dst: 0, lhs: 1, rhs: 2 },
        Op::Shl { dst: 0, lhs: 1, rhs: 2 },
        Op::Shr { dst: 0, lhs: 1, rhs: 2 },
        Op::SeqConcat { dst: 0, lhs: 1, rhs: 2 },
    ];
    let builtins = [
        BuiltinId::Floor,
        BuiltinId::Ceil,
        BuiltinId::Round,
        BuiltinId::Abs,
        BuiltinId::Sqrt,
        BuiltinId::Min,
        BuiltinId::Max,
        BuiltinId::Pow,
    ];
    let names = ops
        .iter()
        .map(|op| crate::vm::tower::binary(op).expect("a binary op").3)
        .chain(builtins.iter().map(|b| crate::vm::tower::lifts(*b).expect("a lifting builtin")));
    for name in names {
        let h = operation(name).unwrap_or_else(|e| panic!("`{name}` has no import: {e}"));
        assert_eq!(h.field(), format!("logos_rt_value_{name}"), "`{name}` is imported as another operation");
        assert!(IMPORTS.contains(&h), "`{name}`'s import is not among the module's value imports");
    }
}

#[test]
fn a_self_contained_module_names_the_target_that_runs_the_extension() {
    assert!(REFUSAL.contains("does not run the exact extension"));
    assert!(REFUSAL.contains("wasm-linked"));
}
