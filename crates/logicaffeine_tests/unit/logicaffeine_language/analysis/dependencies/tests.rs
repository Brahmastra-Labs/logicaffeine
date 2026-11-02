use super::*;

#[test]
fn basic_dependency_scanning() {
    let source = r#"
# My Game

This uses [Geometry](file:./geo.md) and [Physics](logos:std).

## Main
Let x be 1.
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 2);
    assert_eq!(deps[0].alias, "Geometry");
    assert_eq!(deps[0].uri, "file:./geo.md");
    assert_eq!(deps[1].alias, "Physics");
    assert_eq!(deps[1].uri, "logos:std");
}

#[test]
fn import_statement_forms() {
    // Bare single, `and`-joined, comma list, explicit `from` path, and `as` rename.
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "# M\n\nImport Util.\n\n## Main\nShow 1.\n",
            &[("Util", "file:./util")],
        ),
        (
            "# M\n\nImport Geometry and Colors.\n\n## Main\nShow 1.\n",
            &[("Geometry", "file:./geometry"), ("Colors", "file:./colors")],
        ),
        (
            "# M\n\nImport Lexer, Parser, Ast.\n\n## Main\nShow 1.\n",
            &[
                ("Lexer", "file:./lexer"),
                ("Parser", "file:./parser"),
                ("Ast", "file:./ast"),
            ],
        ),
        (
            "# M\n\nImport MathUtil from \"lib/mathutil.lg\".\n\n## Main\nShow 1.\n",
            &[("MathUtil", "file:./lib/mathutil.lg")],
        ),
        (
            "# M\n\nImport Geometry as Geo.\n\n## Main\nShow 1.\n",
            &[("Geo", "file:./geometry")],
        ),
        (
            "# M\n\nImport Geometry as Geo and Colors.\n\n## Main\nShow 1.\n",
            &[("Geo", "file:./geometry"), ("Colors", "file:./colors")],
        ),
    ];
    for (src, expected) in cases {
        let deps = scan_dependencies(src);
        let got: Vec<(&str, &str)> = deps
            .iter()
            .map(|d| (d.alias.as_str(), d.uri.as_str()))
            .collect();
        assert_eq!(&got, expected, "import parse mismatch for:\n{src}");
    }
}

#[test]
fn import_path_with_embedded_separators_stays_one_clause() {
    // A `from "<path>"` whose path contains ` and ` or `,` must NOT be split
    // on those separators — the whole quoted path is one clause.
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "# M\n\nImport Data from \"shapes and colors.lg\".\n\n## Main\nShow 1.\n",
            &[("Data", "file:./shapes and colors.lg")],
        ),
        (
            "# M\n\nImport Data from \"a,b.lg\".\n\n## Main\nShow 1.\n",
            &[("Data", "file:./a,b.lg")],
        ),
        // A quoted separator does not disturb a following real clause.
        (
            "# M\n\nImport Data from \"a and b.lg\" and Colors.\n\n## Main\nShow 1.\n",
            &[("Data", "file:./a and b.lg"), ("Colors", "file:./colors")],
        ),
    ];
    for (src, expected) in cases {
        let deps = scan_dependencies(src);
        let got: Vec<(&str, &str)> = deps
            .iter()
            .map(|d| (d.alias.as_str(), d.uri.as_str()))
            .collect();
        assert_eq!(&got, expected, "quoted-path import mis-parsed for:\n{src}");
    }
}

#[test]
fn import_prose_is_not_an_import() {
    // A sentence that merely starts with "Import" (multi-word, no clause shape)
    // must not be misread as an import directive.
    let deps = scan_dependencies("# M\n\nImport your data before running.\n\n## Main\nShow 1.\n");
    assert!(
        deps.is_empty(),
        "prose starting with Import must not import: {deps:?}"
    );
}

#[test]
fn ignores_links_after_abstract() {
    let source = r#"
# Header

This is the abstract with [Dep1](file:a.md).

This second paragraph has [Dep2](file:b.md).

## Main
Let x be 1.
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].alias, "Dep1");
}

#[test]
fn no_dependencies_without_abstract() {
    let source = r#"
# Module

## Main
Let x be 1.
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 0);
}

#[test]
fn multiline_abstract() {
    let source = r#"
# My Project

This project uses [Math](file:./math.md) for calculations
and [IO](file:./io.md) for input/output operations.

## Main
Let x be 1.
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 2);
    assert_eq!(deps[0].alias, "Math");
    assert_eq!(deps[1].alias, "IO");
}

#[test]
fn handles_spaces_in_alias() {
    let source = r#"
# App

Uses the [Standard Library](logos:std).

## Main
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].alias, "Standard Library");
    assert_eq!(deps[0].uri, "logos:std");
}

#[test]
fn handles_https_urls() {
    let source = r#"
# App

Uses [Physics](https://logicaffeine.dev/pkg/physics).

## Main
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].alias, "Physics");
    assert_eq!(deps[0].uri, "https://logicaffeine.dev/pkg/physics");
}

#[test]
fn handles_multiple_links_on_one_line() {
    let source = r#"
# App

Uses [A](file:a.md), [B](file:b.md), and [C](file:c.md).

## Main
"#;
    let deps = scan_dependencies(source);
    assert_eq!(deps.len(), 3);
    assert_eq!(deps[0].alias, "A");
    assert_eq!(deps[1].alias, "B");
    assert_eq!(deps[2].alias, "C");
}

/// An `Import` below a file's first `##` header imports nothing (`scan_dependencies` stops there), so
/// every such sentence is reported, with its span, for the caller to refuse at its own line. Code in a
/// fence or an indented block is an illustration, never an import; the prose above the header is
/// `scan_dependencies`'s, never a misplaced import.
#[test]
fn misplaced_imports_are_the_import_sentences_below_the_first_header() {
    let src = "# Feed\n\nImport Shared.\n\n## Note\n\nThe record comes from its own file.\n\nImport Other.\n\n    Import Indented.\n\n```\nImport Fenced.\n```\n\n## Main\nImport Late and Later.\nShow 1.\n";
    let found = misplaced_imports(src);
    let aliases: Vec<&str> = found.iter().map(|d| d.alias.as_str()).collect();
    assert_eq!(aliases, vec!["Other", "Late", "Later"]);
    let other = &found[0];
    assert_eq!(
        &src[other.start..other.end],
        "Import Other.",
        "the span is the sentence's own line"
    );
    assert_eq!(
        scan_dependencies(src)
            .iter()
            .map(|d| d.alias.as_str())
            .collect::<Vec<_>>(),
        vec!["Shared"]
    );
    assert!(misplaced_imports("# Clean\n\nImport Shared.\n\n## Main\nShow 1.\n").is_empty());
    assert!(misplaced_imports("## Main\nShow 1.\n").is_empty());
}
