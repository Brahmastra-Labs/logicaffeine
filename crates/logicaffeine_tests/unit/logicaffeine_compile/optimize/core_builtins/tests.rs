use super::{dispatch_source, folded, specializer_source, BEGIN, END, SPECIALIZER_BEGIN, SPECIALIZER_END};
use crate::semantics::builtin_overloads::SelfInterpreted;
use crate::semantics::builtins::{BuiltinId, BUILTIN_NAMES, SYSTEM_NATIVES};

const CORE_INTERP: &str = "src/optimize/core_interp.logos";

/// The generated region of `text`, its opening and closing lines included.
fn region(text: &str) -> Option<&str> {
    let start = text.find(BEGIN)?;
    let end = text[start..].find(END)? + start + END.len() + 1;
    text.get(start..end)
}

#[test]
fn the_self_interpreter_holds_the_generated_dispatch() {
    let held = region(crate::compile::core_interp_source()).expect("core_interp.logos holds the generated region");
    assert!(
        held == dispatch_source(),
        "core_interp.logos's builtins are not the table's: run `regenerate_the_self_interpreters_builtins`\n--- held ---\n{held}\n--- generated ---\n{}",
        dispatch_source()
    );
}

#[test]
fn every_defined_builtin_has_its_definition() {
    let interp = crate::compile::core_interp_source();
    for id in BuiltinId::ALL {
        if id.self_interpreted() == SelfInterpreted::Defined {
            let header = format!("## To coreDefined{id:?} (callArgs: Seq of CVal) -> CVal:");
            assert!(interp.contains(&header), "{id:?} is defined by the self-interpreter, which holds no `{header}`");
        }
    }
    assert!(
        interp.contains("If name equals \"args\":") && interp.contains("Return item \"args()\" of env."),
        "the self-interpreter answers `args()` from the arguments its run was given"
    );
}

#[test]
fn every_typed_builtin_has_its_function_in_the_dispatch() {
    let generated = dispatch_source();
    for id in BuiltinId::ALL {
        if let SelfInterpreted::Overloads { .. } = id.self_interpreted() {
            let header = format!("## To coreBuiltin{id:?} (args: Seq of CVal) -> CVal:");
            assert!(generated.contains(&header), "{id:?} has no function in the dispatch");
        }
    }
}

/// The text of the generated function whose header starts `header`, up to the next function's.
fn function<'g>(generated: &'g str, header: &str) -> &'g str {
    let start = generated.find(header).unwrap_or_else(|| panic!("the dispatch holds `{header}`"));
    let end = generated[start + 1..].find("\n## To ").map_or(generated.len(), |at| start + 1 + at);
    &generated[start..end]
}

#[test]
fn a_call_to_a_builtin_the_self_interpreter_awaits_stops_with_what_it_awaits() {
    let generated = dispatch_source();
    let awaited_of = function(&generated, "## To coreBuiltinAwaited (name: Text) -> Text:");
    let lines: Vec<&str> = awaited_of.lines().collect();
    let mut awaited = 0;
    let natives = SYSTEM_NATIVES.iter().map(|(name, _, id)| (name, id));
    for (name, id) in BUILTIN_NAMES.iter().map(|(name, id)| (name, id)).chain(natives) {
        let SelfInterpreted::Awaits(why) = id.self_interpreted() else {
            continue;
        };
        awaited += 1;
        let test = format!("(name equals \"{name}\")");
        let at = lines
            .iter()
            .position(|l| l.trim_start().starts_with("If ") && l.ends_with(':') && l.contains(&test))
            .unwrap_or_else(|| panic!("coreBuiltinAwaited does not answer `{name}`\n{awaited_of}"));
        assert_eq!(lines.get(at + 1).map(|l| l.trim()), Some(format!("Return \"{why}\".").as_str()), "`{name}` awaits {why}");
        let length = lines[..at]
            .iter()
            .rev()
            .find_map(|l| l.trim().strip_prefix("If nameLength equals ").and_then(|n| n.strip_suffix(':')))
            .unwrap_or_else(|| panic!("`{name}` is decided under no length"));
        assert_eq!(length, name.chars().count().to_string(), "`{name}` is decided among the names of its length");
    }
    assert!(awaited > 0, "the table awaits builtins, and each stops");
    assert!(awaited_of.ends_with("    Return \"\".\n"), "any other name awaits nothing\n{awaited_of}");
    let dispatch = function(&generated, "## To coreBuiltin (name: Text)");
    assert!(
        dispatch.contains(
            "    Let awaited be coreBuiltinAwaited(name).\n    If awaited is not \"\":\n        Return a new VError with msg \"the self-interpreter applies no `{name}`: it awaits {awaited}\".\n"
        ),
        "a call to a builtin the self-interpreter awaits stops with what it awaits\n{dispatch}"
    );
}

#[test]
fn a_call_to_a_system_native_reaches_its_builtin() {
    let generated = dispatch_source();
    let known = function(&generated, "## To coreBuiltinKnown (name: Text) -> Bool:");
    let awaited_of = function(&generated, "## To coreBuiltinAwaited (name: Text) -> Text:");
    for (name, _, id) in SYSTEM_NATIVES {
        let test = format!("(name equals \"{name}\")");
        match id.self_interpreted() {
            SelfInterpreted::Awaits(_) => assert!(awaited_of.contains(&test), "the native `{name}` awaits what {id:?} awaits"),
            _ => assert!(known.contains(&test), "the native `{name}` runs {id:?}"),
        }
    }
}

#[test]
fn a_call_to_no_function_stops_as_the_tree_walker_stops() {
    let generated = dispatch_source();
    let dispatch = function(&generated, "## To coreBuiltin (name: Text)");
    assert!(
        dispatch.ends_with("    Return a new VError with msg \"Unknown function: {name}\".\n"),
        "a call to no function and no builtin stops with the tree-walker's report\n{dispatch}"
    );
    assert!(!dispatch.contains("VNothing"), "a call the self-interpreter cannot apply never answers nothing\n{dispatch}");
}

/// Rewrite `core_interp.logos`'s generated region from the table.
#[test]
#[ignore = "rewrites core_interp.logos: run after the table changes"]
fn regenerate_the_self_interpreters_builtins() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CORE_INTERP);
    let text = std::fs::read_to_string(&path).expect("read core_interp.logos");
    let held = region(&text).expect("core_interp.logos holds the generated region").to_string();
    std::fs::write(&path, text.replacen(&held, &dispatch_source(), 1)).expect("write core_interp.logos");
}

/// The specializer's generated builtins in `text`, their opening and closing lines included.
fn specializer_region(text: &str) -> Option<&str> {
    let start = text.find(SPECIALIZER_BEGIN)?;
    let end = text[start..].find(SPECIALIZER_END)? + start + SPECIALIZER_END.len() + 1;
    text.get(start..end)
}

#[test]
fn the_specializer_holds_the_generated_builtins() {
    let held = specializer_region(crate::compile::pe_source_text()).expect("pe_source.logos holds the generated region");
    assert!(
        held == specializer_source(),
        "pe_source.logos's builtins are not the table's: run `regenerate_the_specializers_builtins`\n--- held ---\n{held}\n--- generated ---\n{}",
        specializer_source()
    );
    assert!(
        crate::compile::pe_source_text().contains("Let pfFolded be peFoldBuiltin(fn1, peArgs)."),
        "a call to a builtin on known arguments reaches the generated folds"
    );
}

#[test]
fn the_specializer_folds_only_pure_builtins_on_the_constants_it_holds() {
    use crate::semantics::builtins::BuiltinEffect;
    let builtins = folded();
    assert!(builtins.len() >= 22, "the arithmetic, text and calendar builtins fold: {}", builtins.len());
    for (name, id, overloads) in builtins {
        assert_eq!(id.effect(), BuiltinEffect::Pure, "`{name}` folds, so it is pure");
        for o in overloads {
            for ty in o.params.iter().chain(std::iter::once(&o.answer)) {
                assert!(super::constant_of(ty).is_some(), "`{name}` folds a `{ty}`, which the specializer holds no constant of");
            }
        }
    }
    for id in [
        BuiltinId::ParseInt,
        BuiltinId::ParseFloat,
        BuiltinId::TextFromBytes,
        BuiltinId::ParseTimestamp,
        BuiltinId::MakeDate,
        BuiltinId::InZone,
        BuiltinId::TimeNow,
    ] {
        assert!(folded().iter().all(|(_, f, _)| *f != id), "{id:?} can stop the program or answers by the clock: it never folds");
    }
    let held = specializer_source();
    assert!(held.contains("Return a new CFloat with value sqrt((fbInt1 + 0.0))."), "an Int widens to a Real parameter, as the self-interpreter widens it");
    assert!(held.contains("Return a new CInt with value year_of(fbMoment1)."), "`year_of` of a known instant folds to its year");
    assert!(
        held.contains("Let fbAnswered be text_bytes(fbText1).") && held.contains("Return a new CList with items fbAnsweredItems."),
        "the bytes of a known text fold to a known list of their values"
    );
}

/// Rewrite `pe_source.logos`'s generated builtins from the table.
#[test]
#[ignore = "rewrites pe_source.logos: run after the table changes"]
fn regenerate_the_specializers_builtins() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/optimize/pe_source.logos");
    let text = std::fs::read_to_string(&path).expect("read pe_source.logos");
    let held = specializer_region(&text).expect("pe_source.logos holds the region").to_string();
    std::fs::write(&path, text.replacen(&held, &specializer_source(), 1)).expect("write pe_source.logos");
}
