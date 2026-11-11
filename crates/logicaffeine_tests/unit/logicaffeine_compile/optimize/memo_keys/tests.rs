use super::{dialect_key_signatures, dialect_keys, key_functions, keys_in, source_keys, KEYS_BEGIN, KEYS_END};

/// The dialects that hold the online specializer's keys under a suffix: their file and their suffix.
const DIALECTS: [(&str, &str); 2] = [("src/optimize/pe_mini_source.logos", "M"), ("src/optimize/pe_bti_source.logos", "B")];

fn dialect_source(suffix: &str) -> &'static str {
    match suffix {
        "M" => crate::compile::pe_mini_source_text(),
        "B" => crate::compile::pe_bti_source_text(),
        other => panic!("no dialect has the suffix `{other}`"),
    }
}

/// Whether `text` calls or defines `name` under exactly that name — never as the tail of a longer one.
fn names(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = &text[at + name.len()..];
        !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') && (after.starts_with('(') || after.starts_with(" ("))
    })
}

/// The variants of the sum type `ty` the specializers' Core IR declares.
fn variants_of(ty: &str) -> Vec<String> {
    let catalog = crate::compile::core_types_for_pe_source();
    let header = [format!("## A {ty} is one of:"), format!("## An {ty} is one of:")];
    let start = header
        .iter()
        .find_map(|h| catalog.find(h.as_str()).map(|at| at + h.len()))
        .unwrap_or_else(|| panic!("the Core IR declares `{ty}`"));
    catalog[start..]
        .lines()
        .skip(1)
        .take_while(|line| line.starts_with("    A"))
        .map(|line| line.trim_start().split(' ').nth(1).expect("a variant has a name").trim_end_matches('.').to_string())
        .collect()
}

#[test]
fn the_online_specializer_holds_the_memo_keys() {
    let keys = source_keys();
    assert!(keys.starts_with(KEYS_BEGIN) && keys.ends_with(KEYS_END));
    let functions = key_functions();
    assert_eq!(functions.first(), Some(&"keyJoin"), "the keys open with the join every key is built by");
    for f in ["exprToKeyPart", "stmtToKeyPart", "stringPartToKeyPart", "policyToKeyPart"] {
        assert!(functions.contains(&f), "the keys define `{f}`: {functions:?}");
    }
    assert!(
        crate::compile::pe_source_text().contains("        Let part be exprToKeyPart(a).\n        Set key to keyJoin(key, part).\n"),
        "the online specializer's memo key joins each argument's key by `keyJoin`"
    );
}

#[test]
fn every_dialect_holds_the_online_specializers_memo_keys() {
    for (file, suffix) in DIALECTS {
        let held = keys_in(dialect_source(suffix)).unwrap_or_else(|| panic!("{file} holds the memo keys"));
        let generated = dialect_keys(suffix);
        assert!(
            held == generated,
            "{file}'s memo keys are not pe_source.logos's: run `regenerate_the_dialects_memo_keys`\n--- held ---\n{held}\n--- generated ---\n{generated}"
        );
    }
}

#[test]
fn a_dialect_calls_only_its_own_keys() {
    for (file, suffix) in DIALECTS {
        let keys = dialect_keys(suffix);
        for f in key_functions() {
            assert!(keys.contains(&format!("## To {f}{suffix} (")), "{file} defines `{f}{suffix}`");
            assert!(!names(&keys, f), "{file}'s keys call the online specializer's own `{f}`");
        }
    }
}

#[test]
fn every_dialect_joins_its_memo_key_by_its_keys() {
    for (file, suffix) in DIALECTS {
        let source = dialect_source(suffix);
        assert!(
            source.contains(&format!("        Let part be exprToKeyPart{suffix}(a).\n        Set key to keyJoin{suffix}(key, part).\n")),
            "{file}'s memo key joins each argument's key by `keyJoin{suffix}`"
        );
        assert!(
            source.contains(&format!("            Set key to keyJoin{suffix}(key, \"s{{part}}\").\n        Otherwise:\n            Set key to keyJoin{suffix}(key, \"d\").\n")),
            "{file}'s cycle key joins a known argument's key and an unknown one's mark by `keyJoin{suffix}`"
        );
    }
}

/// A key function for a sum type of the Core IR spells every variant the type has, so no value of it falls through to
/// a key another value shares.
#[test]
fn the_memo_keys_spell_every_variant_of_every_type_they_key() {
    let keys = source_keys();
    let mut keyed = 0;
    for definition in keys.split("\n## To ").skip(1) {
        let (name, rest) = definition.split_once(' ').expect("a definition declares its signature");
        let Some(ty) = rest
            .strip_prefix('(')
            .and_then(|r| r.split_once(": "))
            .and_then(|(_, r)| r.split_once(')'))
            .map(|(ty, _)| ty)
            .filter(|ty| !ty.contains(' ') && ty.starts_with('C'))
        else {
            continue;
        };
        let body = definition.split("\n## To ").next().unwrap_or(definition);
        for variant in variants_of(ty) {
            assert!(
                body.lines().any(|line| {
                    let line = line.trim_start();
                    line == format!("When {variant}:") || line.starts_with(&format!("When {variant} ("))
                }),
                "`{name}` keys every `{ty}`, but has no arm for `{variant}`"
            );
        }
        keyed += 1;
    }
    assert!(keyed >= 7, "the keys spell each sum type they meet: {keyed}");
}

#[test]
fn a_projection_restores_each_dialect_key_functions_signature() {
    let signatures = dialect_key_signatures("M");
    assert_eq!(signatures.len(), key_functions().len());
    assert!(signatures.contains(&("exprToKeyPartM_".to_string(), "(e: CExpr) -> Text:".to_string())));
    assert!(signatures.contains(&("keyJoinM_".to_string(), "(acc: Text) and (part: Text) -> Text:".to_string())));
    assert!(dialect_key_signatures("B").contains(&("stmtToKeyPartB_".to_string(), "(sk: CStmt) -> Text:".to_string())));
}

#[test]
fn a_dialect_renames_a_key_function_only_where_it_is_one() {
    let keys = dialect_keys("M");
    assert!(keys.contains("## To keyJoinM (acc: Text) and (part: Text) -> Text:\n"));
    assert!(keys.contains("Set sk0k to keyJoinM(sk0k, exprToKeyPartM(sk0x1)).\n"), "a nested call is renamed at each level");
    assert!(keys.contains("Let mutable sk0k be \"CLet\".\n"), "a key's own text keeps the constructor's name");
    assert_eq!(dialect_keys("").as_str(), source_keys(), "under no suffix the keys are the online specializer's own");
}

/// Rewrite pe_mini's and pe_bti's memo keys from the online specializer's.
#[test]
#[ignore = "rewrites pe_mini_source.logos and pe_bti_source.logos: run after pe_source.logos's memo keys change"]
fn regenerate_the_dialects_memo_keys() {
    for (file, suffix) in DIALECTS {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {file}: {e}"));
        let held = keys_in(&text).unwrap_or_else(|| panic!("{file} holds the memo keys")).to_string();
        std::fs::write(&path, text.replacen(&held, &dialect_keys(suffix), 1)).unwrap_or_else(|e| panic!("write {file}: {e}"));
    }
}
