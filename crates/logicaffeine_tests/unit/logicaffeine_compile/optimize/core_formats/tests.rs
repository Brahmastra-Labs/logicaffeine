use super::{precision_source, specializer_precision_source, BEGIN, END, PRECISIONS, SPECIALIZER_BEGIN, SPECIALIZER_END};

/// The generated region of `text`, its opening and closing lines included.
fn region(text: &str) -> Option<&str> {
    let start = text.find(BEGIN)?;
    let end = text[start..].find(END)? + start + END.len() + 1;
    text.get(start..end)
}

#[test]
fn the_self_interpreter_holds_the_generated_precisions() {
    let held = region(crate::compile::core_interp_source()).expect("core_interp.logos holds the generated region");
    assert!(
        held == precision_source(),
        "core_interp.logos's precisions are not the generator's: run `regenerate_the_self_interpreters_precisions`\n--- held ---\n{held}\n--- generated ---\n{}",
        precision_source()
    );
}

#[test]
fn every_precision_formats_through_the_languages_own_hole() {
    let generated = precision_source();
    for binder in ["fpInt", "fpReal"] {
        for digits in 0..=PRECISIONS {
            let branch = format!(
                "            If fpDigits equals {digits}:\n                Return a new VText with value \"{{{binder}:.{digits}}}\".\n"
            );
            assert!(generated.contains(&branch), "`{binder}` to {digits} digits formats as `{{{binder}:.{digits}}}`\n{generated}");
        }
    }
}

#[test]
fn a_precision_past_the_branches_stops_with_what_the_self_interpreter_awaits() {
    let generated = precision_source();
    assert!(
        generated.ends_with(&format!(
            "    Return a new VError with msg \"the self-interpreter applies no `.{{fpDigits}}`: it awaits a precision of at most {PRECISIONS} digits\".\n{END}\n"
        )),
        "a precision no branch formats stops, never with other digits\n{generated}"
    );
    assert!(
        generated.contains("        Otherwise:\n            Return a new VText with value valToText(fpValue).\n"),
        "a value that is no number shows as itself, as the language's `.N` on a non-number does"
    );
}

/// The specializer's generated region of `text`, its opening and closing lines included.
fn specializer_region(text: &str) -> Option<&str> {
    let start = text.find(SPECIALIZER_BEGIN)?;
    let end = text[start..].find(SPECIALIZER_END)? + start + SPECIALIZER_END.len() + 1;
    text.get(start..end)
}

#[test]
fn the_specializer_holds_the_generated_precisions() {
    let held = specializer_region(crate::compile::pe_source_text()).expect("pe_source.logos holds the generated region");
    assert!(
        held == specializer_precision_source(),
        "pe_source.logos's precisions are not the generator's: run `regenerate_the_self_interpreters_precisions`\n--- held ---\n{held}\n--- generated ---\n{}",
        specializer_precision_source()
    );
}

#[test]
fn the_specializer_folds_each_precision_the_self_interpreter_formats() {
    let generated = specializer_precision_source();
    for binder in ["ppInt", "ppReal"] {
        for digits in 0..=PRECISIONS {
            let branch = format!(
                "            If ppDigits equals {digits}:\n                Return a new CText with value \"{{{binder}:.{digits}}}\".\n"
            );
            assert!(generated.contains(&branch), "a known `{binder}` to {digits} digits folds to `{{{binder}:.{digits}}}`\n{generated}");
        }
    }
    assert!(
        generated.ends_with(&format!("    Return a new CVar with name \"__pe_no_unfold\".\n{SPECIALIZER_END}\n")),
        "anything else is left to the residual\n{generated}"
    );
}

/// Rewrite `core_interp.logos`'s and `pe_source.logos`'s generated regions.
#[test]
#[ignore = "rewrites core_interp.logos and pe_source.logos: run after the generator changes"]
fn regenerate_the_self_interpreters_precisions() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/optimize/core_interp.logos");
    let text = std::fs::read_to_string(&path).expect("read core_interp.logos");
    let held = region(&text).expect("core_interp.logos holds the generated region").to_string();
    std::fs::write(&path, text.replacen(&held, &precision_source(), 1)).expect("write core_interp.logos");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/optimize/pe_source.logos");
    let text = std::fs::read_to_string(&path).expect("read pe_source.logos");
    let held = specializer_region(&text).expect("pe_source.logos holds the generated region").to_string();
    std::fs::write(&path, text.replacen(&held, &specializer_precision_source(), 1)).expect("write pe_source.logos");
}
