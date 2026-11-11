use super::{collection_fields, collection_types, lift_types_source, signature_types, COLLECTION_KIND, ELEMENT_TYPE, FIELD_TYPE};
use crate::optimize::mix_bta::StoreEntry;

/// The definitions of the self-interpreter's run: the Core IR's catalog and the self-interpreter.
fn self_interpreter() -> String {
    format!("{}\n{}", crate::compile::core_types_for_pe_source(), crate::compile::core_interp_source())
}

/// The definitions of the specializer's run: the Core IR's catalog, the self-interpreter, the decompiler and `mix`.
fn specializer() -> String {
    format!(
        "{}\n{}\n{}\n{}",
        crate::compile::core_types_for_pe_source(),
        crate::compile::core_interp_source(),
        crate::compile::decompile_source_text(),
        crate::compile::mix_source_text()
    )
}

fn is_collection(ty: &str) -> bool {
    ty.starts_with("Seq of ") || ty.starts_with("Set of ") || ty.starts_with("Map of ")
}

/// Each `(call, shown)`: `program`'s lift types, run on the tree-walker beside its definitions, show `shown` for `call`.
fn answers(program: &str, calls: &[(&str, &str)]) {
    let mut run = format!("{program}\n{}\n## Main\n", lift_types_source(program));
    for (call, _) in calls {
        run.push_str(&format!("Show {call}.\n"));
    }
    let shown = crate::compile::interpret_bootstrap(&run).unwrap_or_else(|e| panic!("the lift types run: {e:?}"));
    let lines: Vec<&str> = shown.split('\n').collect();
    assert_eq!(lines.len(), calls.len(), "one line per call, shown:\n{shown}");
    for ((call, expected), line) in calls.iter().zip(lines) {
        assert_eq!(line, *expected, "{call}");
    }
}

#[test]
fn a_field_has_the_type_its_constructor_gives_it() {
    answers(
        &self_interpreter(),
        &[
            ("liftedFieldType(\"VSeq\", \"items\")", "Seq of CVal"),
            ("liftedFieldType(\"VMap\", \"entries\")", "Map of Text to CVal"),
            ("liftedFieldType(\"CNew\", \"fieldNames\")", "Seq of Text"),
            ("liftedFieldType(\"CNew\", \"fields\")", "Seq of CExpr"),
            ("liftedFieldType(\"CConcurrent\", \"branches\")", "Seq of Seq of CStmt"),
            ("liftedFieldType(\"CoreNext\", \"env\")", "Map of Text to CVal"),
        ],
    );
    answers(&specializer(), &[("liftedFieldType(\"BFuncDef\", \"statics\")", "Seq of Bool")]);
}

#[test]
fn a_field_of_no_collection_type_has_none() {
    answers(
        &self_interpreter(),
        &[
            ("liftedFieldType(\"VSeq\", \"length\")", ""),
            ("liftedFieldType(\"VInt\", \"value\")", ""),
            ("liftedFieldType(\"VOption\", \"inner\")", ""),
            ("liftedFieldType(\"CNew\", \"typeName\")", ""),
            ("liftedFieldType(\"NoSuchConstructor\", \"items\")", ""),
            ("liftedFieldType(\"\", \"\")", ""),
        ],
    );
}

#[test]
fn a_collection_type_has_its_kind_and_element() {
    answers(
        &self_interpreter(),
        &[
            ("liftedCollectionKind(\"Seq of CVal\")", "Seq"),
            ("liftedCollectionKind(\"Map of Text to CVal\")", "Map"),
            ("liftedCollectionKind(\"Seq of Seq of CStmt\")", "Seq"),
            ("liftedCollectionKind(\"CVal\")", ""),
            ("liftedElementType(\"Seq of CVal\")", "CVal"),
            ("liftedElementType(\"Map of Text to CVal\")", "CVal"),
            ("liftedElementType(\"Seq of Seq of CStmt\")", "Seq of CStmt"),
            ("liftedElementType(\"CVal\")", ""),
            ("liftedElementType(\"\")", ""),
        ],
    );
    answers(&specializer(), &[("liftedElementType(\"Map of Text to Text\")", "Text"), ("liftedCollectionKind(\"Map of Text to Text\")", "Map")]);
}

#[test]
fn a_program_holds_its_own_types_alone() {
    let own = lift_types_source(&self_interpreter());
    let specializer_types = lift_types_source(&specializer());
    let two_level = ["BStatic", "BDVar", "BSIf", "BDIf", "BSBlock", "BWhen", "BFuncDef", "BStmt", "BExpr"];
    assert!(two_level.iter().any(|c| specializer_types.contains(&format!("\"{c}\""))), "the specializer's lift types hold its two-level forms");
    for c in two_level {
        assert!(!own.contains(c), "the self-interpreter's lift types name the specializer's `{c}`");
    }
}

#[test]
fn every_collection_field_of_a_program_is_typed() {
    for program in [self_interpreter(), specializer()] {
        let fields = collection_fields(&program);
        let mut seen = 0;
        for line in program.lines() {
            let Some(rest) = line.strip_prefix("    A ").or_else(|| line.strip_prefix("    An ")) else { continue };
            let Some((ctor, written)) = rest.trim_end_matches('.').split_once(" with ") else { continue };
            for part in written.split(" and ") {
                let Some((field, ty)) = part.split_once(' ') else { continue };
                if is_collection(ty) {
                    seen += 1;
                    assert_eq!(
                        fields.get(ctor).and_then(|f| f.get(field)).map(String::as_str),
                        Some(ty),
                        "`{ctor}`'s field `{field}`"
                    );
                }
            }
        }
        assert!(seen > 20, "the program's constructors have collection fields");
    }
}

#[test]
fn every_collection_type_a_signature_names_has_its_kind_and_element() {
    for program in [self_interpreter(), specializer()] {
        let types = collection_types(&program, &collection_fields(&program));
        let mut named = 0;
        for header in program.lines().filter(|l| l.starts_with("## To ")) {
            for ty in signature_types(header) {
                if is_collection(&ty) {
                    named += 1;
                    assert!(types.contains(&ty), "`{header}` names `{ty}`, which the lift types do not hold");
                }
            }
        }
        assert!(named > 0, "the program's signatures name collection types");
        for ty in &types {
            assert!(!ty.contains([',', ':']), "`{ty}` is no type");
            let element = ty
                .strip_prefix("Seq of ")
                .or_else(|| ty.strip_prefix("Set of "))
                .or_else(|| ty.strip_prefix("Map of ").and_then(|e| e.split_once(" to ")).map(|(_, v)| v))
                .expect("a collection type");
            assert!(!is_collection(element) || types.contains(element), "`{ty}`'s element type `{element}` is not held");
        }
    }
}

#[test]
fn a_signature_names_its_parameters_and_answer() {
    assert_eq!(
        signature_types("## To mixLiftAt (v: CVal) and (liftType: Text) and (out: mutable Seq of CStmt) -> CExpr:"),
        ["CVal", "Text", "Seq of CStmt", "CExpr"]
    );
    assert_eq!(
        signature_types("## To f (xs: Seq of (Seq of Int)) -> Map of Text to (Seq of Int) is exported for native:"),
        ["Seq of Seq of Int", "Map of Text to Seq of Int"]
    );
    assert_eq!(signature_types("## To g (c: mutable Map of Text to CVal):"), ["Map of Text to CVal"]);
    assert_eq!(
        signature_types("## To joinWith (parts: Seq of Text, sep: Text) -> Text:"),
        ["Seq of Text", "Text", "Text"]
    );
    assert!(signature_types("## To h:").is_empty());
}

#[test]
fn a_program_with_a_struct_has_no_lift_types() {
    let refused = std::panic::catch_unwind(|| lift_types_source("## A Point has:\n    An x: Int.\n"));
    assert!(refused.is_err(), "a struct's fields are no constructor's: the lift types refuse it");
}

#[test]
fn the_specializer_asks_the_functions_a_program_carries() {
    let mix = crate::compile::mix_source_text();
    for function in [FIELD_TYPE, COLLECTION_KIND, ELEMENT_TYPE] {
        assert!(mix.contains(&format!("(\"{function}\", ")), "mix.logos never asks `{function}`");
    }
}

/// The function names of a projection's two-level program, as `mix` reads them from its `mixFuncs`.
fn function_names(store: Vec<(&'static str, StoreEntry)>) -> Vec<String> {
    store
        .into_iter()
        .find_map(|(name, entry)| match (name, entry) {
            ("mixFuncs", StoreEntry::Map(functions)) => Some(functions.into_iter().map(|(n, _)| n).collect()),
            _ => None,
        })
        .expect("a two-level program's functions")
}

#[test]
fn the_projections_programs_carry_their_lift_types() {
    for (label, program) in [
        ("the self-interpreter", crate::compile::self_interpreter_two_level()),
        ("the specializer", crate::compile::specializer_two_level()),
    ] {
        let names = function_names(program.unwrap_or_else(|e| panic!("{label}'s two-level program: {e}")).store_entries());
        for function in [FIELD_TYPE, COLLECTION_KIND, ELEMENT_TYPE] {
            assert!(names.iter().any(|n| n == function), "{label} carries no `{function}`");
        }
    }
}
