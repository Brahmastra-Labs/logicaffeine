//! Namespaced cross-module type references (`Alias::Type`).
//!
//! `discover_with_imports` + `merge_registry` intern imported struct types under
//! their import alias — `Geometry::Point`, `Colors::Color` — but until now the
//! surface language could not utter them: the lexer had no `::` handling, so
//! `Let p be a new Geometry::Point with x 3 and y 4.` died with a parse error at
//! the `::`. These tests drive the SAME multi-file path largo uses (tempdir +
//! `discover_with_imports` + `compile_project`) and assert the toy program that
//! constructs imported struct types, reads their fields possessively, and Shows
//! the sum prints `263` — through both the interpreter and the AOT codegen.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use logicaffeine_base::{Arena, Interner};
use logicaffeine_compile::analysis::discover_with_imports;
use logicaffeine_compile::compile::compile_project;
use logicaffeine_compile::Loader;
use logicaffeine_language::arena_ctx::AstContext;
use logicaffeine_language::ast::stmt::{Expr, Stmt, TypeExpr};
use logicaffeine_language::drs::WorldState;
use logicaffeine_language::{Lexer, Parser};

const GEOMETRY_MD: &str = r#"# Geometry

## A Point has:
    An x: Int.
    A y: Int.
"#;

const COLORS_MD: &str = r#"# Colors

## A Color has:
    An r: Int.
    A g: Int.
    A b: Int.
"#;

/// The toy from the card. Constructs `Geometry::Point` and `Colors::Color`,
/// reads their fields possessively, and Shows the sum — 3 + 4 + 100 + 150 + 6 = 263.
const MAIN_LG: &str = r#"# App

Uses [Geometry](file:./geometry.md) and [Colors](file:./colors.md).

## Main
Let p be a new Geometry::Point with x 3 and y 4.
Let c be a new Colors::Color with r 100 and g 150 and b 6.
Let total be p's x plus p's y plus c's r plus c's g plus c's b.
Show total.
"#;

/// Write the three-file toy project into a fresh tempdir and return the paths.
fn write_toy_project(dir: &Path) -> std::path::PathBuf {
    fs::write(dir.join("geometry.md"), GEOMETRY_MD).unwrap();
    fs::write(dir.join("colors.md"), COLORS_MD).unwrap();
    let main_path = dir.join("main.lg");
    fs::write(&main_path, MAIN_LG).unwrap();
    main_path
}

/// Drive the tree-walking interpreter over a multi-file program exactly as a
/// largo project would: discover types across imports with `discover_with_imports`,
/// then parse the entry source with that registry and run the interpreter seeded
/// from it (`with_type_registry`). Returns the joined output lines.
fn interpret_project(entry: &Path) -> Result<String, String> {
    let root = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut loader = Loader::new(root);
    let mut interner = Interner::new();

    let source = fs::read_to_string(entry).map_err(|e| e.to_string())?;

    let registry = discover_with_imports(entry, &source, &mut loader, &mut interner)?;

    let mut lexer = Lexer::new(&source, &mut interner);
    let tokens = lexer.tokenize();
    let mwe_trie = logicaffeine_language::mwe::build_mwe_trie();
    let tokens = logicaffeine_language::mwe::apply_mwe_pipeline(tokens, &mwe_trie, &mut interner);

    let expr_arena = Arena::new();
    let term_arena = Arena::new();
    let np_arena = Arena::new();
    let sym_arena = Arena::new();
    let role_arena = Arena::new();
    let pp_arena = Arena::new();
    let stmt_arena: Arena<Stmt> = Arena::new();
    let imperative_expr_arena: Arena<Expr> = Arena::new();
    let type_expr_arena: Arena<TypeExpr> = Arena::new();

    let ctx = AstContext::with_types(
        &expr_arena,
        &term_arena,
        &np_arena,
        &sym_arena,
        &role_arena,
        &pp_arena,
        &stmt_arena,
        &imperative_expr_arena,
        &type_expr_arena,
    );

    let mut world_state = WorldState::new();
    let registry_for_interp = registry.clone();
    let stmts = {
        let mut parser = Parser::new(tokens, &mut world_state, &mut interner, ctx, registry);
        parser
            .parse_program()
            .map_err(|e| format!("{:?}", e.kind))?
    };

    let mut interp = logicaffeine_compile::interpreter::Interpreter::new(&interner)
        .with_type_registry(&registry_for_interp);
    futures::executor::block_on(interp.run(&stmts))?;
    Ok(interp.output.join("\n"))
}

#[test]
fn multimodule_namespaced_types_interpreter_sum_263() {
    let dir = tempdir().unwrap();
    let main_path = write_toy_project(dir.path());

    let out = interpret_project(&main_path)
        .expect("multi-module namespaced-type program should interpret");
    assert_eq!(
        out.trim(),
        "263",
        "sum of Geometry::Point + Colors::Color fields"
    );
}

#[test]
fn multimodule_namespaced_types_registry_has_qualified_names() {
    let dir = tempdir().unwrap();
    let main_path = write_toy_project(dir.path());

    let root = main_path.parent().unwrap().to_path_buf();
    let mut loader = Loader::new(root);
    let mut interner = Interner::new();
    let source = fs::read_to_string(&main_path).unwrap();
    let registry = discover_with_imports(&main_path, &source, &mut loader, &mut interner)
        .expect("discovery should succeed");

    let point = interner.intern("Geometry::Point");
    let color = interner.intern("Colors::Color");
    assert!(
        registry.is_type(point),
        "registry should hold Geometry::Point"
    );
    assert!(
        registry.is_type(color),
        "registry should hold Colors::Color"
    );
}

#[test]
fn multimodule_namespaced_types_aot_compiles_and_sanitizes_idents() {
    let dir = tempdir().unwrap();
    let main_path = write_toy_project(dir.path());

    let output = compile_project(&main_path).expect("compile_project should succeed");
    let rust = &output.rust_code;

    // The generated Rust must sanitize `::` in the struct identifiers — a bare
    // `Geometry::Point` in a struct declaration or construction is invalid Rust.
    assert!(
        rust.contains("Geometry__Point"),
        "AOT should emit the sanitized `Geometry__Point` ident.\n{rust}"
    );
    assert!(
        rust.contains("Colors__Color"),
        "AOT should emit the sanitized `Colors__Color` ident.\n{rust}"
    );
    assert!(
        !rust.contains("Geometry::Point") && !rust.contains("Colors::Color"),
        "AOT must not leak the raw `Alias::Type` form into Rust.\n{rust}"
    );
    // The struct declaration and its construction must both use the sanitized name.
    assert!(
        rust.contains("struct Geometry__Point"),
        "AOT should declare `struct Geometry__Point`.\n{rust}"
    );
    assert!(
        rust.contains("fn main"),
        "AOT should emit a main function.\n{rust}"
    );
}

/// Type annotation on a `Let` using a namespaced type: `Let q: Geometry::Point be …`.
#[test]
fn multimodule_namespaced_type_annotation_interprets() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("geometry.md"), GEOMETRY_MD).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        r#"# App

Uses [Geometry](file:./geometry.md).

## Main
Let q: Geometry::Point be a new Geometry::Point with x 41 and y 1.
Show q's x plus q's y.
"#,
    )
    .unwrap();

    let out = interpret_project(&main_path).expect("namespaced type annotation should interpret");
    assert_eq!(out.trim(), "42");
}

/// The same annotation program must survive AOT with a sanitized annotation.
#[test]
fn multimodule_namespaced_type_annotation_aot_compiles() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("geometry.md"), GEOMETRY_MD).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        r#"# App

Uses [Geometry](file:./geometry.md).

## Main
Let q: Geometry::Point be a new Geometry::Point with x 41 and y 1.
Show q's x plus q's y.
"#,
    )
    .unwrap();

    let output = compile_project(&main_path).expect("annotated program should compile");
    assert!(
        output.rust_code.contains("Geometry__Point"),
        "annotation type should sanitize to Geometry__Point.\n{}",
        output.rust_code
    );
    assert!(
        !output.rust_code.contains("Geometry::Point"),
        "raw namespaced form must not leak into Rust.\n{}",
        output.rust_code
    );
}

/// Generic instantiation with a namespaced element type: `a new Seq of
/// Geometry::Point` — the namespaced Word flows through `parse_generic_type_args`
/// like any type name.
#[test]
fn multimodule_namespaced_generic_element_type() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("geometry.md"), GEOMETRY_MD).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        r#"# App

Uses [Geometry](file:./geometry.md).

## Main
Let points be a new Seq of Geometry::Point.
Let p be a new Geometry::Point with x 7 and y 8.
Push p to points.
Show length of points.
"#,
    )
    .unwrap();

    let out =
        interpret_project(&main_path).expect("Seq of a namespaced element type should interpret");
    assert_eq!(out.trim(), "1");

    let output = compile_project(&main_path).expect("generic-element program should compile");
    assert!(
        !output.rust_code.contains("Geometry::Point"),
        "raw namespaced form must not leak into Rust.\n{}",
        output.rust_code
    );
}

/// A LOCAL struct whose field is typed by an imported namespaced type:
/// `## A Segment has:  A start: Geometry::Point.` — the field type flows through
/// discovery and codegen with the same sanitized identifier.
#[test]
fn multimodule_namespaced_field_type_in_local_struct() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("geometry.md"), GEOMETRY_MD).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        r#"# App

Uses [Geometry](file:./geometry.md).

## A Segment has:
    An origin: Geometry::Point.
    A target: Geometry::Point.

## Main
Let a be a new Geometry::Point with x 1 and y 2.
Let b be a new Geometry::Point with x 10 and y 20.
Let s be a new Segment with origin a and target b.
Show s's origin's x plus s's target's y.
"#,
    )
    .unwrap();

    let out = interpret_project(&main_path)
        .expect("namespaced field types in a local struct should interpret");
    assert_eq!(out.trim(), "21");

    let output = compile_project(&main_path).expect("field-typed program should compile");
    let rust = &output.rust_code;
    assert!(
        rust.contains("struct Segment"),
        "local struct should be declared.\n{rust}"
    );
    assert!(
        !rust.contains("Geometry::Point"),
        "field type must sanitize — no raw namespaced form in Rust.\n{rust}"
    );
    assert!(
        rust.contains("Geometry__Point"),
        "field type should reference the sanitized struct ident.\n{rust}"
    );
}

/// Negative: an unknown alias (`Bogus`) that was never imported must NOT be
/// silently treated as an imported type, and must never panic. `Bogus::Point`
/// is a syntactically valid namespaced reference now (the `::` lexes), but there
/// is no `Bogus` import, so discovery never registers `Bogus::Point`. The clean
/// failure is a DANGLING reference: the AOT names the sanitized `Bogus__Point`
/// yet declares no matching struct (rustc then rejects it) — as opposed to
/// silently emitting a `struct Bogus__Point` as if the alias resolved. This test
/// locks that no-silent-success invariant across parse → discovery → codegen,
/// and that no stage panics.
#[test]
fn multimodule_unknown_alias_is_not_silently_imported() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("geometry.md"), GEOMETRY_MD).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        r#"# App

Uses [Geometry](file:./geometry.md).

## Main
Let p be a new Bogus::Point with x 3 and y 4.
Show p's x.
"#,
    )
    .unwrap();

    // 1. Discovery must NOT invent `Bogus::Point` — only `Geometry::Point` is imported.
    let root = main_path.parent().unwrap().to_path_buf();
    let mut loader = Loader::new(root);
    let mut interner = Interner::new();
    let source = fs::read_to_string(&main_path).unwrap();
    let registry = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        discover_with_imports(&main_path, &source, &mut loader, &mut interner)
    }))
    .expect("discovery must not panic on an unknown alias")
    .expect("discovery of the valid Geometry import should still succeed");
    let bogus = interner.intern("Bogus::Point");
    assert!(
        !registry.is_type(bogus),
        "an unimported alias must never be registered as a type"
    );

    // 2. The build refuses the dangling reference itself, naming it — rustc is never the gate.
    let err = std::panic::catch_unwind(|| compile_project(&main_path))
        .expect("codegen must not panic on an unknown alias")
        .expect_err("an unimported alias names no type and must be refused");
    assert!(
        format!("{err:?}")
            .contains("`Bogus::Point` is not a type this program declares or imports"),
        "the refusal must name `Bogus::Point`:\n{err:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Namespaced ENUMS. `merge_registry` namespaces an imported enum's TYPE key
// (`Colors::Filter`) while its variant names (`Sepia`, `Noir`) keep their bare
// form. Construction (`a new Sepia`) and Inspect resolve the variant back to the
// namespaced enum key, so every enum codegen site — the `pub enum` decl, the
// `Default` impl, `Expr::NewVariant`, and the `Inspect` match arm — must sanitize
// the `::` exactly as the struct path does, or AOT emits `pub enum Colors::Filter`
// (rustc-invalid). And because `codegen_program` emits EVERY registry enum
// unconditionally, merely IMPORTING a module that declares an enum must not break
// AOT. These lock both the string shape and real execution.
// ─────────────────────────────────────────────────────────────────────────────

const COLORS_ENUM_MD: &str = r#"# Colors

## A Filter is one of:
    A Sepia.
    A Noir.
"#;

/// Write a two-file project whose module declares an ENUM (`Colors::Filter`),
/// imported and used by Main; return the entry path.
fn write_enum_project(dir: &Path) -> std::path::PathBuf {
    fs::write(dir.join("colors.md"), COLORS_ENUM_MD).unwrap();
    let main_path = dir.join("main.lg");
    fs::write(
        &main_path,
        r#"# App

Uses [Colors](file:./colors.md).

## Main
Let f be a new Sepia.
Inspect f:
    When Sepia: Show "sepia".
    When Noir: Show "noir".
"#,
    )
    .unwrap();
    main_path
}

/// The interpreter path must run an imported enum (constructed + Inspect-matched)
/// and print the matched arm — the tier-semantics pin that the AOT lock stands on.
#[test]
fn multimodule_namespaced_enum_interprets() {
    let dir = tempdir().unwrap();
    let main_path = write_enum_project(dir.path());

    let out =
        interpret_project(&main_path).expect("imported enum construct + Inspect should interpret");
    assert_eq!(out.trim(), "sepia");
}

/// The AOT emission for an imported enum must SANITIZE the `::` at every enum
/// codegen site: no raw `Colors::Filter` may leak into a type/path position, and
/// the sanitized `Colors__Filter` ident must appear in both the `pub enum`
/// declaration and the constructed variant. This is the case the string-only
/// battery missed — it FAILS against the pre-fix codegen (`pub enum Colors::Filter`).
#[test]
fn multimodule_namespaced_enum_aot_sanitizes_idents() {
    let dir = tempdir().unwrap();
    let main_path = write_enum_project(dir.path());

    let output = compile_project(&main_path).expect("enum program should compile");
    let rust = &output.rust_code;

    assert!(
        !rust.contains("Colors::Filter"),
        "AOT must not leak the raw `Colors::Filter` enum form into Rust.\n{rust}"
    );
    assert!(
        rust.contains("Colors__Filter"),
        "AOT should emit the sanitized `Colors__Filter` enum ident.\n{rust}"
    );
    assert!(
        rust.contains("pub enum Colors__Filter"),
        "the enum declaration itself must use the sanitized ident.\n{rust}"
    );
    // The constructed variant (`a new Sepia`) resolves to the namespaced enum, so
    // its `EnumName::Variant` path must also be sanitized — never a raw `::` in
    // any declaration/path built from an interned type name. The ONLY `::` left in
    // the emitted enum text is the sanitized `Colors__Filter::Sepia` path plus the
    // legitimate Rust `::` of paths like `Default::default` and crate paths.
    assert!(
        rust.contains("Colors__Filter::Sepia"),
        "the constructed variant path must use the sanitized enum ident.\n{rust}"
    );
}

/// MERELY importing a module that declares an enum (never even constructing a
/// variant) must not break AOT — `codegen_program` emits every registry enum
/// unconditionally, so an unsanitized decl would poison the output. Compile a
/// program that imports the enum module but only uses a plain value.
#[test]
fn multimodule_imported_but_unused_enum_still_compiles() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("colors.md"), COLORS_ENUM_MD).unwrap();
    let main_path = dir.path().join("main.lg");
    fs::write(
        &main_path,
        r#"# App

Uses [Colors](file:./colors.md).

## Main
Show 7.
"#,
    )
    .unwrap();

    let output = compile_project(&main_path).expect("importing an enum module should compile");
    assert!(
        !output.rust_code.contains("Colors::Filter"),
        "an imported-but-unused enum's decl must still be sanitized.\n{}",
        output.rust_code
    );

    // And it must actually rustc-compile and run.
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "imported-enum program must compile+run.\nstderr: {}\n\nRust:\n{}",
        result.stderr, result.rust_code
    );
    assert_eq!(result.stdout.trim(), "7");
}

/// The namespaced-ENUM program must not only string-match — it must rustc-compile
/// and EXECUTE, printing the matched arm. This is the real gate the pre-fix
/// codegen fails at rustc time (`pub enum Colors::Filter` never compiles).
#[test]
fn multimodule_namespaced_enum_aot_runs() {
    let dir = tempdir().unwrap();
    let main_path = write_enum_project(dir.path());

    let output = compile_project(&main_path).expect("enum program should compile");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "namespaced-enum program must compile+run.\nstderr: {}\n\nRust:\n{}",
        result.stderr, result.rust_code
    );
    assert_eq!(result.stdout.trim(), "sepia");
}

/// The FLAGSHIP end-to-end lock: the 263 toy from the card, compiled through the
/// real multi-file `compile_project` path and actually RUN via rustc — proving the
/// emitted Rust compiles and produces `263`, not merely that it string-matches.
#[test]
fn multimodule_namespaced_types_aot_compiles_and_runs_263() {
    let dir = tempdir().unwrap();
    let main_path = write_toy_project(dir.path());

    let output = compile_project(&main_path).expect("compile_project should succeed");
    let result = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    assert!(
        result.success,
        "the 263 toy must compile+run under rustc.\nstderr: {}\n\nRust:\n{}",
        result.stderr, result.rust_code
    );
    assert_eq!(
        result.stdout.trim(),
        "263",
        "compiled multi-module program must print 263.\nRust:\n{}",
        result.rust_code
    );
}

/// A single global guard: for EVERY multimodule fixture in this file, the emitted
/// Rust must contain no `Alias::Type` interned-name leak (no `Geometry::`,
/// `Colors::`, `Bogus::`). Cheap, fixture-driven, and catches a regression at any
/// codegen site the per-test asserts don't happen to cover.
#[test]
fn multimodule_no_raw_namespace_colon_in_any_fixture() {
    // (module files to write, entry filename, entry source) for each fixture.
    let struct_main = r#"# App

Uses [Geometry](file:./geometry.md) and [Colors](file:./colors.md).

## Main
Let p be a new Geometry::Point with x 3 and y 4.
Let c be a new Colors::Color with r 100 and g 150 and b 6.
Let total be p's x plus p's y plus c's r plus c's g plus c's b.
Show total.
"#;
    let enum_main = r#"# App

Uses [Colors](file:./colors.md).

## Main
Let f be a new Sepia.
Inspect f:
    When Sepia: Show "sepia".
    When Noir: Show "noir".
"#;

    // Each case: list of (filename, contents) module files + the entry source.
    let cases: &[(&[(&str, &str)], &str)] = &[
        (
            &[("geometry.md", GEOMETRY_MD), ("colors.md", COLORS_MD)],
            struct_main,
        ),
        (&[("colors.md", COLORS_ENUM_MD)], enum_main),
    ];

    for (modules, main_src) in cases {
        let dir = tempdir().unwrap();
        for (name, contents) in *modules {
            fs::write(dir.path().join(name), contents).unwrap();
        }
        let main_path = dir.path().join("main.lg");
        fs::write(&main_path, main_src).unwrap();

        let output = compile_project(&main_path).expect("fixture should compile");
        let rust = &output.rust_code;
        for alias in ["Geometry::", "Colors::", "Bogus::"] {
            assert!(
                !rust.contains(alias),
                "emitted Rust leaked the raw namespaced prefix `{alias}` — an interned \
                 `Alias::Type` reached an identifier position unsanitized.\n{rust}"
            );
        }
    }
}
