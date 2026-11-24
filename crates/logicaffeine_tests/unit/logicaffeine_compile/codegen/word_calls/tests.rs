use super::*;
use crate::semantics::builtins::BUILTIN_NAMES;

/// The runtime's word functions, as written.
fn word_rt() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_system/src/word_rt.rs");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The parameter types and the return type of the `word_rt` function `name`, read off its signature.
fn signature(source: &str, name: &str) -> Option<(Vec<String>, String)> {
    let head = format!("pub fn {name}(");
    let rest = &source[source.find(&head)? + head.len()..];
    let close = rest.find(')')?;
    let params = rest[..close]
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| p.split_once(':').map_or(p, |(_, ty)| ty).trim().to_string())
        .collect();
    let returned = rest[close + 1..].trim_start().strip_prefix("->")?;
    Some((params, returned[..returned.find('{')?].trim().to_string()))
}

/// Every `pub fn` the runtime's word module declares, by name.
fn declared(source: &str) -> Vec<&str> {
    source
        .match_indices("pub fn ")
        .filter_map(|(at, head)| {
            let name = &source[at + head.len()..];
            name.find(|c: char| c == '(' || c == '<').map(|end| &name[..end])
        })
        .collect()
}

/// Each row names a function the runtime declares, taking a list by reference and everything else by value, and
/// returning the type the row states.
#[test]
fn every_row_is_the_runtimes_signature() {
    let source = word_rt();
    for call in WORD_CALLS {
        let (params, returned) =
            signature(&source, call.rust).unwrap_or_else(|| panic!("word_rt declares no `{}`", call.rust));
        assert_eq!(params.len(), call.takes.len(), "{}: {params:?}", call.rust);
        for (param, takes) in params.iter().zip(call.takes) {
            assert_eq!(param.starts_with('&'), *takes == Takes::List, "{}: {param}", call.rust);
        }
        assert_eq!(returned, call.result, "{} returns", call.rust);
    }
}

/// Every word function the runtime declares is a row, or one the emitter writes elsewhere: `word32`/`word64`, whose
/// literal arguments fold, and the functions generic over the word.
#[test]
fn every_runtime_word_function_has_a_row() {
    const WRITTEN_ELSEWHERE: &[&str] = &["word32", "word64", "rotl", "rotr", "word_and", "word_or", "word_not"];
    let source = word_rt();
    let functions = declared(&source);
    assert!(functions.len() > 40, "the runtime's word functions were read: {functions:?}");
    for f in functions {
        assert!(
            WORD_CALLS.iter().any(|call| call.rust == f) || WRITTEN_ELSEWHERE.contains(&f),
            "`word_rt::{f}` has no row: a builtin that compiles to it is written by no rule of the table"
        );
    }
    for f in WRITTEN_ELSEWHERE {
        assert!(WORD_CALLS.iter().all(|call| call.rust != *f), "`{f}` is written elsewhere");
    }
}

/// Each row is found by the name a program calls its builtin and by its arity, and by nothing else.
#[test]
fn a_row_is_found_by_its_builtins_name_and_arity() {
    for call in WORD_CALLS {
        let name = BUILTIN_NAMES
            .iter()
            .find(|(_, id)| *id == call.builtin)
            .map(|(name, _)| *name)
            .unwrap_or_else(|| panic!("{:?} has a name", call.builtin));
        assert!(std::ptr::eq(by_name(name, call.takes.len()).expect(name), call), "{name}");
        assert!(by_name(name, call.takes.len() + 1).is_none(), "{name} with one argument too many");
        assert!(std::ptr::eq(of_builtin(call.builtin).expect(name), call));
    }
    for (at, call) in WORD_CALLS.iter().enumerate() {
        assert!(WORD_CALLS[at + 1..].iter().all(|other| other.builtin != call.builtin), "one row per {:?}", call.builtin);
    }
    assert!(by_name("word32", 1).is_none(), "`word32` is written elsewhere");
    assert!(by_name("sqrt", 1).is_none());
}

/// A call lends the list a lane vector is packed from and passes every other argument.
#[test]
fn a_call_lends_the_list_it_packs() {
    assert_eq!(by_name("lanes16Word16", 1).unwrap().lower(&["xs".to_string()]), "lanes16_word16(&(xs))");
    assert_eq!(by_name("mulhi16", 2).unwrap().lower(&["a".to_string(), "b".to_string()]), "mulhi16(a, b)");
    assert_eq!(
        by_name("lanes4Of", 4).unwrap().lower(&["a", "b", "c", "d"].map(String::from)),
        "lanes4_of(a, b, c, d)"
    );
    assert_eq!(by_name("intOfWord32", 1).unwrap().result, "i64");
}
