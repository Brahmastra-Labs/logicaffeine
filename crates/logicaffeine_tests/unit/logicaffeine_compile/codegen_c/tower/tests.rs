use super::*;

/// Every function of the C ABI as its Rust source declares it, with its parameter count.
fn abi_entries() -> std::collections::BTreeMap<String, usize> {
    let source = include_str!("../../../../../logicaffeine_data/src/abi.rs");
    let mut entries = std::collections::BTreeMap::new();
    let mut rest = source;
    while let Some(at) = rest.find("extern \"C\" fn logos_rt_value_") {
        let from = &rest[at + "extern \"C\" fn ".len()..];
        let name_end = from.find('(').expect("a parameter list");
        let name = from[..name_end].to_string();
        let params_end = from.find(')').expect("a closed parameter list");
        let params = from[name_end + 1..params_end].trim();
        let count = if params.is_empty() { 0 } else { params.split(',').filter(|p| !p.trim().is_empty()).count() };
        entries.insert(name, count);
        rest = &from[params_end..];
    }
    // The operations the ABI writes through its tables: `name => method;` under `binary!`, `relation!` (two
    // values) and `unary!` (one).
    for (table, count) in [("binary! {", 2), ("relation! {", 2), ("unary! {", 1)] {
        let at = source.find(table).unwrap_or_else(|| panic!("the ABI has no `{table}` table"));
        let body = &source[at + table.len()..];
        let body = &body[..body.find('}').expect("a closed table")];
        for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (name, _) = line.split_once("=>").expect("`name => method;`");
            entries.insert(name.trim().to_string(), count);
        }
    }
    entries
}

/// Every function of the C ABI as the C runtime declares it, with its parameter count.
fn c_declarations() -> std::collections::BTreeMap<String, usize> {
    let mut entries = std::collections::BTreeMap::new();
    for line in RUNTIME.lines().filter(|l| l.starts_with("extern ")) {
        let at = line.find("logos_rt_value_").expect("a declaration of the value runtime");
        let from = &line[at..];
        let name_end = from.find('(').expect("a parameter list");
        let params_end = from.rfind(')').expect("a closed parameter list");
        let params = from[name_end + 1..params_end].trim();
        let count = if params == "void" { 0 } else { params.split(',').count() };
        entries.insert(from[..name_end].to_string(), count);
    }
    entries
}

#[test]
fn the_c_runtime_declares_exactly_the_abi_the_rust_side_defines() {
    let abi = abi_entries();
    let c = c_declarations();
    assert!(abi.len() >= 50, "the ABI census found only {} entries", abi.len());
    assert_eq!(c, abi, "the C declarations and the Rust ABI differ in a name or a parameter count");
}

#[test]
fn the_c_runtime_announces_that_it_links_the_value_runtime() {
    assert!(super::super::value_runtime::links(RUNTIME), "tower.c does not declare the value runtime it needs");
}

#[test]
fn every_exact_type_name_is_one_the_reach_analysis_seeds() {
    for name in TYPE_NAMES {
        let source = format!("## Main\nLet x: {name} be 1.\nShow x.\n");
        let found = crate::ui_bridge::with_parsed_program_checked(&source, |parsed, interner| {
            let (stmts, _, _) = parsed.expect("the program parses");
            Ok(TowerTypes::of_program(stmts, interner).is_some())
        });
        assert_eq!(found.ok(), Some(true), "a binding declared {name} is not reached by the extension");
    }
}
