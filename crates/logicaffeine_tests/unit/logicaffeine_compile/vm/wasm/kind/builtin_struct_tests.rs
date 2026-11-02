use super::*;

/// The exact census of structs the compiler hands to author code fully formed (never spliced from
/// an `## An X has:` declaration, so [`CompiledProgram::struct_types`] never carries them). Adding a
/// THIRD one (the next `HttpError`/`File`) without updating this test is exactly the class of miss
/// this table exists to close (the `HttpError` and `File` misses) — this test locks both the
/// membership and the declared shape, reading the table's own source rather than a name.
#[test]
fn builtin_struct_types_are_exact() {
    let defs = builtin_struct_types();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["HttpError", "File"],
        "the exact, ordered census of compiler-materialized structs"
    );

    let http_error = defs.iter().find(|d| d.name == "HttpError").unwrap();
    assert_eq!(
        http_error.fields,
        vec![
            ("status".to_string(), BoundaryType::Int),
            ("message".to_string(), BoundaryType::Text)
        ],
        "HttpError's shape must match assets/std/http.lg's own declaration exactly"
    );

    let file = defs.iter().find(|d| d.name == "File").unwrap();
    assert_eq!(
        file.fields,
        vec![
            ("name".to_string(), BoundaryType::Text),
            ("size".to_string(), BoundaryType::Int),
            ("type".to_string(), BoundaryType::Text),
            ("bytes".to_string(), BoundaryType::Seq(Box::new(BoundaryType::Int))),
        ],
        "File's shape is `{{name, size, type, bytes}}` — `file's name`, `file's size`, `file's type` \
         and `file's bytes`, in that order, the record every tier binds a dropped file to"
    );
}

/// `resolve_named_layout` must resolve EVERY builtin struct even when `struct_types` is empty — the
/// exact condition a program that never demand-imports the struct's real declaration is in. A
/// regression here reopens G10 (`HttpError`) or G20 (`File`): `e's message` / `file's size` refuse
/// as "field access with no static slot" again.
#[test]
fn resolve_named_layout_falls_back_to_every_builtin_struct() {
    for def in builtin_struct_types() {
        let layout = resolve_named_layout(&def.name, &[], &[]).unwrap_or_else(|| {
            panic!(
                "{} must resolve with no spliced struct_types entry",
                def.name
            )
        });
        assert_eq!(
            layout.len(),
            def.fields.len(),
            "{}'s layout must carry every declared field",
            def.name
        );
        for ((_, kind, _), (fname, ftype)) in layout.iter().zip(def.fields.iter()) {
            assert_eq!(
                Some(*kind),
                boundary_to_kind(ftype),
                "{}'s field `{fname}` must resolve to its declared type's Kind",
                def.name
            );
        }
    }
}

/// A program that DOES declare its own real `struct_types` entry for a builtin struct's name (e.g.
/// demand-importing `http.lg` for real, or a future page defining its own `File`) must resolve to
/// THAT entry, never the built-in default — the fallback only fills a gap, it never shadows a real
/// declaration.
#[test]
fn resolve_named_layout_prefers_a_real_struct_types_entry_over_the_builtin_fallback() {
    let real = StructTypeDef {
        name: "HttpError".to_string(),
        fields: vec![("status".to_string(), BoundaryType::Int)],
    };
    let layout = resolve_named_layout("HttpError", std::slice::from_ref(&real), &[])
        .expect("a real struct_types entry must resolve");
    assert_eq!(
        layout.len(),
        1,
        "the REAL (one-field) declaration must win over the built-in's two-field default"
    );
}
