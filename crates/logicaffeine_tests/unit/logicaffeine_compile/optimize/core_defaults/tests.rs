use super::{
    defaults_source, holds_replicas_of, named_defaults, starting_value, BEGIN, CRDT_AWAITS, END,
};
use crate::interpreter::{declared_default, DeclaredDefault};

/// The generated region of `text`, its opening and closing lines included.
fn region(text: &str) -> Option<&str> {
    let start = text.find(BEGIN)?;
    let end = text[start..].find(END)? + start + END.len() + 1;
    text.get(start..end)
}

#[test]
fn the_self_interpreter_holds_the_generated_defaults() {
    let held = region(crate::compile::core_interp_source()).expect("core_interp.logos holds the generated region");
    assert!(
        held == defaults_source(),
        "core_interp.logos's declared defaults are not the table's: run `regenerate_the_self_interpreters_defaults`\n--- held ---\n{held}\n--- generated ---\n{}",
        defaults_source()
    );
}

#[test]
fn every_declared_type_name_starts_at_the_tree_walkers_default() {
    let generated = defaults_source();
    let lines: Vec<&str> = generated.lines().collect();
    let mut answered = 0;
    for (name, default) in named_defaults() {
        assert_eq!(declared_default(name), default, "`{name}`: the tree-walker's default is the table's");
        let test = format!("(typeName equals \"{name}\")");
        let at = lines
            .iter()
            .position(|l| l.trim_start().starts_with("If ") && l.ends_with(':') && l.contains(&test))
            .unwrap_or_else(|| panic!("coreDeclaredDefault does not answer `{name}`\n{generated}"));
        assert_eq!(
            lines.get(at + 1).map(|l| l.trim()),
            Some(format!("Return {}.", starting_value(default)).as_str()),
            "`{name}` starts at its {default:?} default"
        );
        let length = lines[..at]
            .iter()
            .rev()
            .find_map(|l| l.trim().strip_prefix("If nameLength equals ").and_then(|n| n.strip_suffix(':')))
            .unwrap_or_else(|| panic!("`{name}` is decided under no length"));
        assert_eq!(length, name.chars().count().to_string(), "`{name}` is decided among the names of its length");
        answered += 1;
    }
    assert!(answered >= 30, "the scalar, collection and CRDT names all have defaults: {answered}");
}

#[test]
fn each_declared_type_name_has_one_default() {
    let mut seen = std::collections::HashSet::new();
    for (name, _) in named_defaults() {
        assert!(seen.insert(name), "`{name}` is named twice across the defaults and the CRDT kinds");
    }
}

#[test]
fn a_name_with_no_default_of_its_own_starts_at_nothing() {
    let generated = defaults_source();
    assert!(
        generated.ends_with(&format!("    Return a new VNothing.\n{END}\n")),
        "any other name starts at nothing, as the tree-walker's does\n{generated}"
    );
    for name in ["Point", "Option", "Shape", "", "Int of 8", "Seq of Int"] {
        assert_eq!(declared_default(name), DeclaredDefault::Nothing, "`{name}` has no default of its own");
        assert!(
            !generated.contains(&format!("(typeName equals \"{name}\")")),
            "coreDeclaredDefault decides no `{name}`"
        );
    }
}

#[test]
fn a_held_replica_starts_at_a_template_of_its_kind() {
    let mut counters = 0;
    for (name, default) in named_defaults() {
        if let DeclaredDefault::Crdt(kind) = default {
            if holds_replicas_of(kind) {
                assert_eq!(
                    starting_value(default),
                    format!("a new VCrdt with kind \"{}\" and state a new Map of Text to CVal", kind.value_type_name()),
                    "`{name}` starts at a replica the type name the language gives its kind names"
                );
                counters += 1;
            }
        }
    }
    assert_eq!(counters, 8, "the counters' four names and the sequence's four start at replicas");
}

#[test]
fn a_character_slot_starts_at_the_character_the_tree_walker_starts_it_at() {
    assert!(
        matches!(crate::interpreter::default_for_declared_type("Char"), crate::interpreter::RuntimeValue::Char('\0')),
        "the tree-walker starts a Char slot at NUL"
    );
    assert_eq!(starting_value(DeclaredDefault::Char), "a new VChar with value `\\0`", "so does the self-interpreter");
}

#[test]
fn a_default_the_self_interpreter_cannot_hold_stops_with_what_it_awaits() {
    for (name, default) in named_defaults() {
        let value = starting_value(default);
        match default {
            DeclaredDefault::Crdt(kind) if holds_replicas_of(kind) => {}
            DeclaredDefault::Crdt(_) => {
                assert_eq!(
                    value,
                    format!("a new VError with msg \"the self-interpreter holds no `{{typeName}}`: it awaits {CRDT_AWAITS}\""),
                    "`{name}` stops with what the self-interpreter awaits to hold it"
                );
            }
            _ => assert!(!value.contains("VError") && !value.contains("VNothing"), "`{name}` starts at a value: {value}"),
        }
    }
}

/// Rewrite `core_interp.logos`'s generated region from the table.
#[test]
#[ignore = "rewrites core_interp.logos: run after the table changes"]
fn regenerate_the_self_interpreters_defaults() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/optimize/core_interp.logos");
    let text = std::fs::read_to_string(&path).expect("read core_interp.logos");
    let held = region(&text).expect("core_interp.logos holds the generated region").to_string();
    std::fs::write(&path, text.replacen(&held, &defaults_source(), 1)).expect("write core_interp.logos");
}
