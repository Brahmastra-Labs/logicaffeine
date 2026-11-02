//! ★ LOCK — a compiler-synthesized struct is the SAME shape everywhere it is described. ★
//!
//! `HttpError` and `File` are never written by an `## An X has:` declaration an author's program
//! or an imported module owns; the compiler hands them to every program fully formed
//! (`crates/logicaffeine_language/src/analysis/registry.rs::with_primitives`, `BUILTIN_STRUCT_NAMES`).
//! The wasm AOT backend cannot read that registry at codegen time, so it carries its OWN copy of
//! the same two shapes (`crates/logicaffeine_compile/src/vm/wasm/kind.rs::builtin_struct_types`)
//! for its boundary-type resolution — and `assets/std/http.lg` documents `HttpError`'s shape in
//! prose for authors to read. Three descriptions of one thing drift apart silently unless
//! something reads all three and compares them: this file is that something.
//!
//! A name in one list and not the others, or the same name with fields in a different order or of
//! a different type, means `x's <field>` resolves to a different slot on different tiers — the
//! exact defect `HttpError` and `File` had.

use std::path::PathBuf;

fn read(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// One struct's field shape, in declaration order: `(field name, type name)`.
type Shape = Vec<(String, String)>;

/// Every `intern("…")` literal's contents, in the order they appear.
fn all_interned(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(at) = rest.find("intern(\"") {
        rest = &rest[at + "intern(\"".len()..];
        let end = rest
            .find('"')
            .unwrap_or_else(|| panic!("unterminated intern(\" in {rest}"));
        out.push(rest[..end].to_string());
        rest = &rest[end..];
    }
    out
}

/// `registry.rs`'s `BUILTIN_STRUCT_NAMES` list, read from its own source.
fn declared_names() -> Vec<String> {
    let src = read("../logicaffeine_language/src/analysis/registry.rs");
    let marker = "BUILTIN_STRUCT_NAMES: &[&str] = &[";
    let start = src
        .find(marker)
        .expect("registry.rs declares BUILTIN_STRUCT_NAMES")
        + marker.len();
    let end = start
        + src[start..]
            .find(']')
            .expect("BUILTIN_STRUCT_NAMES closes with `]`");
    src[start..end]
        .split(',')
        .filter_map(|s| {
            let s = s.trim().trim_matches('"');
            (!s.is_empty()).then(|| s.to_string())
        })
        .collect()
}

/// The field shape `registry.rs::with_primitives` registers `name` with: every `FieldDef`'s own
/// `name: interner.intern("…")` and `ty: FieldType::…`, taken in the order they appear — one
/// `TypeDef::Struct`'s `fields: vec![…]` region for `name`. A bare `Primitive`/`Named(interner.
/// intern("…"))` reads as its own interned type name; a `Generic { base: interner.intern("B"),
/// params: vec![FieldType::Primitive(interner.intern("P"))] }` — the only generic shape a
/// built-in struct field needs, one level and one parameter deep, such as `File`'s own `bytes:
/// Seq of Int` — reads as `"B of P"`, matching [`wasm_shape`]'s own composite string for the
/// identical shape on the wasm side.
fn registry_shape(name: &str) -> Shape {
    let src = read("../logicaffeine_language/src/analysis/registry.rs");
    let head = format!("intern(\"{name}\"),\n            TypeDef::Struct {{");
    let at = src
        .find(&head)
        .unwrap_or_else(|| panic!("registry.rs must register `{name}` as a `TypeDef::Struct`"));
    let fields_marker = "fields: vec![";
    let fields_start = at
        + src[at..]
            .find(fields_marker)
            .unwrap_or_else(|| panic!("`{name}`'s TypeDef::Struct must declare `fields: vec![…]`"))
        + fields_marker.len();
    let fields_end = fields_start
        + src[fields_start..]
            .find("\n                ],")
            .unwrap_or_else(|| panic!("`{name}`'s fields vec must close with `],`"));
    let region = &src[fields_start..fields_end];
    let mut out = Vec::new();
    let mut rest = region;
    while let Some(fd_at) = rest.find("FieldDef {") {
        let after = &rest[fd_at + "FieldDef {".len()..];
        let mut depth = 1i32;
        let mut end = 0;
        for (i, c) in after.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i;
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(
            end > 0,
            "`{name}`'s FieldDef never closes its own `{{}}` in {after:?}"
        );
        out.push(parse_field_def_block(&after[..end], name));
        rest = &after[end..];
    }
    assert!(
        !out.is_empty(),
        "`{name}`'s fields region has no `FieldDef {{…}}` entries"
    );
    out
}

/// One `FieldDef { name: interner.intern("…"), ty: FieldType::…, is_public: … }` block's own
/// `(field name, type string)`, per [`registry_shape`]'s own doc comment.
fn parse_field_def_block(block: &str, struct_name: &str) -> (String, String) {
    let field_name = all_interned(block)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("`{struct_name}`'s FieldDef has no interned name: {block}"));
    let ty_marker = "ty: FieldType::";
    let ty_at = block.find(ty_marker).unwrap_or_else(|| {
        panic!("`{struct_name}`'s FieldDef `{field_name}` has no `ty: FieldType::…`: {block}")
    }) + ty_marker.len();
    let ty_rest = &block[ty_at..];
    if ty_rest.trim_start().starts_with("Generic") {
        let lits = all_interned(ty_rest);
        assert_eq!(
            lits.len(),
            2,
            "`{struct_name}`'s FieldDef `{field_name}` is a `FieldType::Generic` this extractor \
             only reads one level/one parameter deep (`base`, one `params` entry): {ty_rest}"
        );
        (field_name, format!("{} of {}", lits[0], lits[1]))
    } else {
        let type_name = all_interned(ty_rest).into_iter().next().unwrap_or_else(|| {
            panic!("`{struct_name}`'s FieldDef `{field_name}` has no interned type name: {ty_rest}")
        });
        (field_name, type_name)
    }
}

/// The field shape `vm/wasm/kind.rs::builtin_struct_types` gives `name`: each
/// `("field".to_string(), BoundaryType::…)` entry of its `StructTypeDef`, in order. A bare
/// `BoundaryType::Y` reads as `"Y"`; `BoundaryType::Seq(Box::new(BoundaryType::P))` — the only
/// nested shape a built-in struct field needs, matching [`registry_shape`]'s own
/// `FieldType::Generic` one level/one parameter deep — reads as `"Seq of P"`.
fn wasm_shape(name: &str) -> Shape {
    let src = read("../logicaffeine_compile/src/vm/wasm/kind.rs");
    let head = format!("name: \"{name}\".to_string(),");
    let at = src.find(&head).unwrap_or_else(|| {
        panic!(
            "vm/wasm/kind.rs's builtin_struct_types() must declare a StructTypeDef named `{name}`"
        )
    });
    let fields_marker = "fields: vec![";
    let fields_start = at
        + src[at..]
            .find(fields_marker)
            .unwrap_or_else(|| panic!("`{name}`'s StructTypeDef must declare `fields: vec![…]`"))
        + fields_marker.len();
    let fields_end = fields_start
        + src[fields_start..]
            .find("],")
            .unwrap_or_else(|| panic!("`{name}`'s fields vec must close with `],`"));
    let region = &src[fields_start..fields_end];
    let mut out = Vec::new();
    for entry in split_top_level_parens(region) {
        out.push(parse_wasm_field_entry(entry, name));
    }
    assert!(
        !out.is_empty(),
        "`{name}`'s fields region has no `(\"…\".to_string(), BoundaryType::…)` entries"
    );
    out
}

/// Splits `region` on its TOP-LEVEL commas — the ones not nested inside `(...)` — so a
/// `("field".to_string(), BoundaryType::Seq(Box::new(BoundaryType::Int)))` entry's own internal
/// commas are never mistaken for the separator between two entries.
fn split_top_level_parens(region: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in region.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                let piece = region[start..i].trim();
                if !piece.is_empty() {
                    out.push(piece);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = region[start..].trim();
    if !last.is_empty() {
        out.push(last);
    }
    out
}

/// One `("field".to_string(), BoundaryType::…)` entry's own `(field name, type string)`, per
/// [`wasm_shape`]'s own doc comment.
fn parse_wasm_field_entry(entry: &str, struct_name: &str) -> (String, String) {
    let q1 = entry
        .find('"')
        .unwrap_or_else(|| panic!("field entry has no name in {entry:?} for `{struct_name}`"));
    let after_q1 = &entry[q1 + 1..];
    let q2 = after_q1
        .find('"')
        .unwrap_or_else(|| panic!("unterminated field name in {entry:?} for `{struct_name}`"));
    let field = after_q1[..q2].to_string();
    let rest = &after_q1[q2 + 1..];
    let ty_marker = "BoundaryType::";
    let ty_at = rest.find(ty_marker).unwrap_or_else(|| {
        panic!("field `{field}` of `{struct_name}` has no BoundaryType in {entry:?}")
    }) + ty_marker.len();
    let ty_rest = &rest[ty_at..];
    let base_end = ty_rest
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(ty_rest.len());
    let base = &ty_rest[..base_end];
    let ty_string = if base == "Seq" && ty_rest[base_end..].trim_start().starts_with('(') {
        let inner_marker = "BoundaryType::";
        let inner_at = ty_rest[base_end..]
            .find(inner_marker)
            .unwrap_or_else(|| panic!("`Seq(…)` field `{field}` of `{struct_name}` has no inner BoundaryType in {entry:?}"))
            + base_end
            + inner_marker.len();
        let inner_rest = &ty_rest[inner_at..];
        let inner_end = inner_rest
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(inner_rest.len());
        format!("Seq of {}", &inner_rest[..inner_end])
    } else {
        base.to_string()
    };
    (field, ty_string)
}

/// A standard-library `An <name> has:\n    a <field>, which is <Type>.\n    …` prose declaration
/// for `name`, if one exists anywhere under `assets/std/*.lg` — `None` when no module documents
/// this builtin's shape at all (true today of `File`, which no `.lg` file mentions by shape).
fn doc_shape(name: &str) -> Option<(String, Shape)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/assets/std");
    let marker = format!("An {name} has:\n");
    for entry in std::fs::read_dir(&dir)
        .expect("assets/std exists")
        .flatten()
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lg") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap_or_default();
        let Some(at) = src.find(&marker) else {
            continue;
        };
        let mut out = Vec::new();
        for line in src[at + marker.len()..].lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("a ").or_else(|| line.strip_prefix("an ")) else {
                break;
            };
            let Some((field, rest)) = rest.split_once(", which is ") else {
                break;
            };
            out.push((field.to_string(), rest.trim_end_matches('.').to_string()));
        }
        if !out.is_empty() {
            return Some((path.file_name().unwrap().to_string_lossy().to_string(), out));
        }
    }
    None
}

/// Anti-vacuity: the extractor must actually find the two builtins this file exists to guard, not
/// silently return nothing because a refactor moved them.
#[test]
fn the_extractor_finds_httperror_and_file() {
    let declared = declared_names();
    assert!(
        declared.len() >= 2,
        "expected registry.rs's BUILTIN_STRUCT_NAMES to list at least HttpError and File, found \
         {declared:?} — the extractor has stopped matching, not that the list shrank"
    );
    for name in ["HttpError", "File"] {
        assert!(
            declared.contains(&name.to_string()),
            "BUILTIN_STRUCT_NAMES must list `{name}`"
        );
        assert!(
            !registry_shape(name).is_empty(),
            "`{name}`'s registry.rs shape must not be empty"
        );
        assert!(
            !wasm_shape(name).is_empty(),
            "`{name}`'s vm/wasm/kind.rs shape must not be empty"
        );
    }
}

#[test]
fn every_builtin_struct_name_is_in_both_tables() {
    let declared = declared_names();
    let wasm_src = read("../logicaffeine_compile/src/vm/wasm/kind.rs");
    for name in &declared {
        assert!(
            wasm_src.contains(&format!("name: \"{name}\".to_string(),")),
            "`{name}` is in registry.rs's BUILTIN_STRUCT_NAMES but vm/wasm/kind.rs's \
             builtin_struct_types() does not declare it — the wasm AOT backend would refuse or \
             mis-resolve a field of this type"
        );
    }
    for name in ["HttpError", "File"] {
        assert!(
            declared.contains(&name.to_string()),
            "`{name}` names a StructTypeDef in vm/wasm/kind.rs's builtin_struct_types() but \
             registry.rs's BUILTIN_STRUCT_NAMES does not list it — every OTHER tier would resolve \
             its fields against a type the registry never registered"
        );
    }
}

#[test]
fn every_builtin_struct_field_shape_agrees_between_registry_and_wasm() {
    for name in declared_names() {
        let reg = registry_shape(&name);
        let wasm = wasm_shape(&name);
        assert_eq!(
            reg, wasm,
            "`{name}`'s field shape disagrees between registry.rs (the type checker every other \
             tier reads) and vm/wasm/kind.rs's builtin_struct_types() (the wasm AOT backend's own \
             boundary-type table) — `x's <field>` would resolve to a different slot on the wasm \
             tier than on every other one"
        );
    }
}

#[test]
fn a_documented_builtin_struct_matches_its_own_doc() {
    let mut checked = Vec::new();
    for name in declared_names() {
        let Some((file, doc)) = doc_shape(&name) else {
            continue;
        };
        checked.push(name.clone());
        assert_eq!(
            doc,
            registry_shape(&name),
            "`assets/std/{file}` documents `An {name} has:` with a shape that disagrees with \
             registry.rs's own `{name}` — a doc that lies about a builtin's fields is worse than \
             no doc"
        );
    }
    assert!(
        checked.contains(&"HttpError".to_string()),
        "expected `assets/std/http.lg` to document `An HttpError has:` — the extractor found no \
         documented shape to check at all, which means the anti-vacuity guard itself is blind"
    );
}

/// Proves [`parse_field_def_block`] reads a `FieldType::Generic` field (`File`'s planned
/// `bytes: Seq of Int`, S17) as `"Seq of Int"` — not just on the two SIMPLE fields every builtin
/// struct has had until now. Without this, `registry_shape`/`wasm_shape`'s Generic branches could
/// go untested until the day `registry.rs` and `vm/wasm/kind.rs` actually gain one, which is
/// exactly the day a parsing mistake here would be hardest to tell apart from a real shape
/// disagreement.
#[test]
fn a_generic_field_def_reads_as_base_of_param() {
    let block = "\n                name: interner.intern(\"bytes\"),\n                ty: FieldType::Generic {\n                    base: interner.intern(\"Seq\"),\n                    params: vec![FieldType::Primitive(interner.intern(\"Int\"))],\n                },\n                is_public: true,\n            ";
    assert_eq!(
        parse_field_def_block(block, "File"),
        ("bytes".to_string(), "Seq of Int".to_string())
    );
}

/// [`parse_field_def_block`]'s converse on the simple (non-Generic) shape every existing builtin
/// struct field already uses.
#[test]
fn a_primitive_field_def_reads_as_its_bare_type_name() {
    let block = "\n                name: interner.intern(\"size\"),\n                ty: FieldType::Primitive(interner.intern(\"Int\")),\n                is_public: true,\n            ";
    assert_eq!(
        parse_field_def_block(block, "File"),
        ("size".to_string(), "Int".to_string())
    );
}

/// [`parse_wasm_field_entry`]'s Generic side: `BoundaryType::Seq(Box::new(BoundaryType::Int))`
/// reads as `"Seq of Int"`, matching [`a_generic_field_def_reads_as_base_of_param`]'s registry-side
/// answer for the identical shape — the actual invariant `every_builtin_struct_field_shape_agrees_
/// between_registry_and_wasm` depends on once a Generic field exists on either side.
#[test]
fn a_seq_wasm_field_entry_reads_as_seq_of_its_element() {
    let entry = "(\"bytes\".to_string(), BoundaryType::Seq(Box::new(BoundaryType::Int)))";
    assert_eq!(
        parse_wasm_field_entry(entry, "File"),
        ("bytes".to_string(), "Seq of Int".to_string())
    );
}

/// [`parse_wasm_field_entry`]'s converse on the simple (bare) shape every existing builtin struct
/// field already uses.
#[test]
fn a_bare_wasm_field_entry_reads_as_its_type_name() {
    let entry = "(\"size\".to_string(), BoundaryType::Int)";
    assert_eq!(
        parse_wasm_field_entry(entry, "File"),
        ("size".to_string(), "Int".to_string())
    );
}

/// [`split_top_level_parens`] must not split a `Seq(Box::new(BoundaryType::Int))` entry's own
/// internal commas from a genuinely different entry's — the multi-field regression
/// `every_builtin_struct_field_shape_agrees_between_registry_and_wasm` would otherwise silently
/// mis-pair fields the moment a struct has more than two of them (which `File` is about to, S17).
#[test]
fn split_top_level_parens_keeps_a_nested_seq_entry_whole() {
    let region = "(\"name\".to_string(), BoundaryType::Text), (\"bytes\".to_string(), BoundaryType::Seq(Box::new(BoundaryType::Int))), (\"size\".to_string(), BoundaryType::Int)";
    let parts = split_top_level_parens(region);
    assert_eq!(
        parts,
        vec![
            "(\"name\".to_string(), BoundaryType::Text)",
            "(\"bytes\".to_string(), BoundaryType::Seq(Box::new(BoundaryType::Int)))",
            "(\"size\".to_string(), BoundaryType::Int)",
        ]
    );
}
