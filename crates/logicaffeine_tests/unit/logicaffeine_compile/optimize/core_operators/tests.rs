use super::{
    dispatch_source, function_of, guard_condition, guards_source, literal_types, specializer_source, spelling_source,
    BEGIN, END, GUARDS_BEGIN, GUARDS_END, SPECIALIZER_BEGIN, SPECIALIZER_END, SPELLING_BEGIN, SPELLING_END,
};
use crate::semantics::operator_overloads::{self_applied, surface_text, SelfApplied, ALL_OPERATORS};

const CORE_INTERP: &str = "src/optimize/core_interp.logos";

const DECOMPILER: &str = "src/optimize/decompile_source.logos";

/// The region of `text` from the line `begin` through the line `end`.
fn between<'t>(text: &'t str, begin: &str, end: &str) -> Option<&'t str> {
    let start = text.find(begin)?;
    let stop = text[start..].find(end)? + start + end.len() + 1;
    text.get(start..stop)
}

/// The generated region of `text`, its opening and closing lines included.
fn region(text: &str) -> Option<&str> {
    between(text, BEGIN, END)
}

#[test]
fn the_self_interpreter_holds_the_generated_operators() {
    let held = region(crate::compile::core_interp_source()).expect("core_interp.logos holds the generated region");
    assert!(
        held == dispatch_source(),
        "core_interp.logos's operators are not the table's: run `regenerate_the_self_interpreters_operators`\n--- held ---\n{held}\n--- generated ---\n{}",
        dispatch_source()
    );
}

#[test]
fn every_operator_the_self_interpreter_applies_has_its_function() {
    let interp = crate::compile::core_interp_source();
    let generated = dispatch_source();
    for op in ALL_OPERATORS {
        let header = format!("## To {} (lv: CVal) and (rv: CVal) -> CVal:", function_of(*op));
        match self_applied(*op) {
            SelfApplied::Overloads { .. } => assert!(generated.contains(&header), "{op:?} has no function in the dispatch"),
            SelfApplied::Defined => {
                assert!(interp.contains(&header), "{op:?} is defined by the self-interpreter, which holds no `{header}`")
            }
            SelfApplied::Awaits(_) => {}
        }
    }
    assert!(
        interp.contains("## To coreOpFamily (op: Text) and (lv: CVal) and (rv: CVal) -> CVal:"),
        "the self-interpreter's value families answer what no overload takes"
    );
}

#[test]
fn an_application_no_overload_takes_reaches_the_value_families() {
    let generated = dispatch_source();
    assert!(generated.contains("Return coreOpFamily(\"+\", lv, rv)."), "`+` on sets and counters reaches the families");
    assert!(generated.contains("If rightInt equals 0:"), "an Int divisor of zero stops the division");
}

#[test]
fn the_decompiler_spells_every_operator_by_the_table() {
    let held = between(crate::compile::decompile_source_text(), SPELLING_BEGIN, SPELLING_END)
        .expect("decompile_source.logos holds the generated spellings");
    assert!(
        held == spelling_source(),
        "decompile_source.logos's spellings are not the table's: run `regenerate_the_decompilers_spellings`\n--- held ---\n{held}\n--- generated ---\n{}",
        spelling_source()
    );
    for (text, spelled) in [("^", " xor "), ("combined with", " combined with "), ("<<", " shifted left by "), ("==", " equals ")] {
        assert!(held.contains(&format!("If op equals \"{text}\":\n            Return \"{spelled}\".")), "`{text}` is spelled `{spelled}`");
    }
}

/// Each specializer's source, the file it is read from, and the suffix its own functions carry.
fn specializers() -> [(&'static str, &'static str, &'static str); 3] {
    [
        (crate::compile::pe_source_text(), "src/optimize/pe_source.logos", ""),
        (crate::compile::pe_mini_source_text(), "src/optimize/pe_mini_source.logos", "M"),
        (crate::compile::pe_bti_source_text(), "src/optimize/pe_bti_source.logos", "B"),
    ]
}

#[test]
fn each_specializer_holds_the_generated_operators() {
    for (text, file, suffix) in specializers() {
        let held = between(text, SPECIALIZER_BEGIN, SPECIALIZER_END).unwrap_or_else(|| panic!("{file} holds the generated region"));
        assert!(
            held == specializer_source(suffix),
            "{file}'s operators are not the table's: run `regenerate_the_specializers_operators`\n--- held ---\n{held}\n--- generated ---\n{}",
            specializer_source(suffix)
        );
        assert!(
            text.contains(&format!("    Return peFoldOperator{suffix}(binOp, lv, rv).")),
            "{file} folds two known values by the generated operators and no others"
        );
    }
}

#[test]
fn a_specializer_folds_each_overload_by_the_languages_own_operator() {
    let generated = specializer_source("");
    let types = literal_types();
    let mut folded = 0;
    for op in ALL_OPERATORS {
        if let SelfApplied::Overloads { overloads, .. } = self_applied(*op) {
            for o in overloads.iter().filter(|o| o.params.iter().all(|p| types.contains(p))) {
                let applied = format!("(fbLeft{} {} fbRight{})", o.params[0], surface_text(*op), o.params[1]);
                assert!(generated.contains(&applied), "{op:?} {o:?} folds as `{applied}`");
                folded += 1;
            }
        }
    }
    assert!(folded >= 30, "the table's overloads between literal types fold: {folded}");
    for ty in &types {
        assert!(generated.contains(&format!("(fbLeft{ty} equals fbRight{ty})")), "two `{ty}`s fold `equals`");
    }
    assert!(generated.contains("(fbLeftInt equals fbRightReal)"), "an Int and a Real fold `equals` as the language compares them");
    assert!(
        generated.contains("                        If fbRightInt equals 0:\n                            Return a new VNothing.\n"),
        "a division by a known zero is left to the run, which stops it"
    );
    assert!(generated.contains("(fbLeftInt ** fbRightInt)") && generated.contains("(fbLeftReal is approximately fbRightReal)"));
    assert!(
        generated.contains("(fbLeftMoment + fbRightDuration)") && generated.contains("(fbLeftMoment is less than fbRightTime)"),
        "two known temporal values fold by the language's own operators"
    );
}

#[test]
fn a_guard_that_holds_stops_the_self_interpreter_before_the_operator() {
    let generated = dispatch_source();
    let mut guards = 0;
    for op in ALL_OPERATORS {
        if let SelfApplied::Overloads { overloads, .. } = self_applied(*op) {
            for guard in overloads.iter().flat_map(|o| o.guards.iter()) {
                assert!(
                    generated.contains(&format!("    Fail with \"{}\".\n", guard.report)),
                    "{op:?}'s guard `{}` stops the run with `{}`",
                    guard.when,
                    guard.report
                );
                assert!(
                    !generated.contains(&format!("Return a new VError with msg \"{}\".", guard.report)),
                    "{op:?}'s guard `{}` stops the run, never answers a failure value",
                    guard.when
                );
                guards += 1;
            }
        }
    }
    assert!(guards >= 10, "the table's guards reach the dispatch: {guards}");
    assert!(
        generated.contains("                    If rightInt equals 0:\n                        Fail with \"Division by zero\".\n                    Return a new VInt with value (leftInt / rightInt).\n"),
        "a zero divisor stops the run on the line before the division, at the division's own indentation"
    );
}

#[test]
fn the_specializer_holds_the_generated_guards() {
    let held = between(crate::compile::pe_source_text(), GUARDS_BEGIN, GUARDS_END)
        .expect("pe_source.logos holds the generated guards");
    assert!(
        held == guards_source(),
        "pe_source.logos's guards are not the table's: run `regenerate_the_specializers_guards`\n--- held ---\n{held}\n--- generated ---\n{}",
        guards_source()
    );
    assert!(
        crate::compile::pe_source_text().contains("peGuardCondition(goOp, peKindOf(goL, goKinds), peKindOf(goR, goKinds), goMsg, goL, goR)"),
        "the finishing pass reads a guard by the operation's operator, its operands' kinds and the guard's report"
    );
}

#[test]
fn every_guard_the_dispatch_applies_reaches_the_finishing_pass() {
    let generated = guards_source();
    let types = literal_types();
    let mut cases = 0;
    for (text, op) in super::answered() {
        if let SelfApplied::Overloads { overloads, .. } = self_applied(op) {
            for o in overloads.iter().filter(|o| !o.guards.is_empty() && o.params.iter().all(|p| types.contains(p))) {
                let first = overloads.iter().find(|f| f.params == o.params).expect("the overload itself takes its types");
                if !std::ptr::eq(first, o) {
                    continue;
                }
                let case = format!("        If gcLeft equals \"{}\" and gcRight equals \"{}\":\n", o.params[0], o.params[1]);
                let at = generated
                    .find(&format!("    If gcOp equals \"{text}\":\n"))
                    .unwrap_or_else(|| panic!("`{text}` holds guards the finishing pass reads"));
                let rest = &generated[at..];
                assert!(rest.contains(&case), "`{text}` on {:?} reaches the finishing pass", o.params);
                for guard in o.guards {
                    assert!(rest.contains(&format!("            If gcReport equals \"{}\":\n", guard.report)));
                    cases += 1;
                }
            }
        }
    }
    assert!(cases >= 10, "every guard of the table is read: {cases}");
    assert!(generated.ends_with("    Return a new CVar with name \"__pe_no_unfold\".\n# The specializer's guards end.\n"));
}

#[test]
fn a_guard_reads_as_the_comparison_it_spells() {
    assert_eq!(
        guard_condition("{right} equals 0"),
        "a new CBinOp with op \"==\" and left gcRightOperand and right (a new CInt with value 0)"
    );
    assert_eq!(
        guard_condition("{right} equals 0.0"),
        "a new CBinOp with op \"==\" and left gcRightOperand and right (a new CFloat with value 0.0)"
    );
    assert_eq!(
        guard_condition("{right} is less than 0"),
        "a new CBinOp with op \"<\" and left gcRightOperand and right (a new CInt with value 0)"
    );
    assert_eq!(
        guard_condition("{left} is at least {right}"),
        "a new CBinOp with op \">=\" and left gcLeftOperand and right gcRightOperand"
    );
    assert_eq!(
        guard_condition("-1 is not {left}"),
        "a new CBinOp with op \"!=\" and left (a new CInt with value -1) and right gcLeftOperand"
    );
}

#[test]
#[should_panic(expected = "is no guard the finishing pass reads")]
fn a_guard_the_finishing_pass_cannot_read_stops_the_generator() {
    guard_condition("{right} divides {left}");
}

/// Rewrite `pe_source.logos`'s generated guards from the table.
#[test]
#[ignore = "rewrites pe_source.logos: run after the table changes"]
fn regenerate_the_specializers_guards() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/optimize/pe_source.logos");
    let text = std::fs::read_to_string(&path).expect("read pe_source.logos");
    let held = between(&text, GUARDS_BEGIN, GUARDS_END).expect("pe_source.logos holds the generated guards").to_string();
    std::fs::write(&path, text.replacen(&held, &guards_source(), 1)).expect("write pe_source.logos");
}

/// Rewrite each specializer's generated operators from the table.
#[test]
#[ignore = "rewrites pe_source.logos, pe_mini_source.logos and pe_bti_source.logos: run after the table changes"]
fn regenerate_the_specializers_operators() {
    for (_, file, suffix) in specializers() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {file}: {e}"));
        let held = between(&text, SPECIALIZER_BEGIN, SPECIALIZER_END).unwrap_or_else(|| panic!("{file} holds the region")).to_string();
        std::fs::write(&path, text.replacen(&held, &specializer_source(suffix), 1)).unwrap_or_else(|e| panic!("write {file}: {e}"));
    }
}

/// Rewrite `decompile_source.logos`'s generated spellings from the table.
#[test]
#[ignore = "rewrites decompile_source.logos: run after the table changes"]
fn regenerate_the_decompilers_spellings() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(DECOMPILER);
    let text = std::fs::read_to_string(&path).expect("read decompile_source.logos");
    let held = between(&text, SPELLING_BEGIN, SPELLING_END).expect("the generated spellings").to_string();
    std::fs::write(&path, text.replacen(&held, &spelling_source(), 1)).expect("write decompile_source.logos");
}

/// Rewrite `core_interp.logos`'s generated region from the table.
#[test]
#[ignore = "rewrites core_interp.logos: run after the table changes"]
fn regenerate_the_self_interpreters_operators() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CORE_INTERP);
    let text = std::fs::read_to_string(&path).expect("read core_interp.logos");
    let held = region(&text).expect("core_interp.logos holds the generated region").to_string();
    std::fs::write(&path, text.replacen(&held, &dispatch_source(), 1)).expect("write core_interp.logos");
}
