//! ★ LOCK — one definition of what a LOGOS source file is called. ★
//!
//! `.logos` is the language's own name for its files and the one the Studio writes, but every tool
//! spelled `.lg` for itself: largo's page discovery, its error messages, the dev server's watcher,
//! the formatter, the packager, the module loader and the language server. A `.logos` file was a
//! page largo did not build, a module an import could not find, an edit the dev server did not see
//! and a file the formatter skipped, each silently.
//!
//! So every tool asks `logicaffeine_compile::source_files`, and production code never compares an
//! extension with a literal or rebuilds a file's name from its stem: a message about a page names
//! the file that is on disk.

use logicaffeine_compile::source_files::{
    is_module_file, is_source_file, MODULE_EXTENSIONS, SOURCE_EXTENSIONS,
};
use std::path::{Path, PathBuf};

#[test]
fn logos_and_lg_are_program_sources() {
    assert!(is_source_file(Path::new("pages/app.logos")));
    assert!(is_source_file(Path::new("src/main.lg")));
}

#[test]
fn markdown_is_a_module_but_not_a_program_source() {
    assert!(is_module_file(Path::new("src/geometry.md")));
    assert!(!is_source_file(Path::new("src/geometry.md")));
}

#[test]
fn every_program_source_is_a_module() {
    for ext in SOURCE_EXTENSIONS {
        assert!(
            is_module_file(Path::new(&format!("lib/shared.{ext}"))),
            ".{ext} is a module"
        );
    }
}

#[test]
fn the_canonical_extension_comes_first_in_both_lists() {
    assert_eq!(SOURCE_EXTENSIONS[0], "logos");
    assert_eq!(
        &MODULE_EXTENSIONS[..SOURCE_EXTENSIONS.len()],
        &SOURCE_EXTENSIONS[..]
    );
    assert_eq!(MODULE_EXTENSIONS.last(), Some(&"md"), "literate Markdown comes last");
}

#[test]
fn nothing_else_is_a_source_or_a_module() {
    for name in [
        "style.css",
        "Largo.toml",
        "notes.txt",
        "app.logosx",
        "app.lgx",
        "app.logos.bak",
        "app.lg.bak-2026-09-12",
        "logos",
        ".logos",
        "pages/",
    ] {
        assert!(!is_source_file(Path::new(name)), "{name} is not a program source");
        assert!(!is_module_file(Path::new(name)), "{name} is not a module");
    }
}

/// Extensions match exactly as written, the way largo has always matched `.lg`.
#[test]
fn extensions_match_as_written() {
    assert!(!is_source_file(Path::new("pages/App.LOGOS")));
    assert!(!is_source_file(Path::new("src/MAIN.LG")));
}

/// The repository root, two levels above this crate.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The crates whose production code reads LOGOS source from disk.
const READERS: [&str; 4] = [
    "crates/logicaffeine_compile/src",
    "crates/logicaffeine_language/src",
    "crates/logicaffeine_lsp/src",
    "apps/logicaffeine_cli/src",
];

/// A comparison against a source-extension literal, or a file name rebuilt from a stem.
fn offence(line: &str) -> Option<&'static str> {
    const LITERALS: [&str; 3] = ["\"logos\"", "\"lg\"", "\"md\""];
    for lit in LITERALS {
        let compared = [
            format!("Some({lit})"),
            format!("== {lit}"),
            format!("{lit} |"),
            format!("| {lit}"),
            format!("[{lit}"),
        ];
        if compared.iter().any(|c| line.contains(c.as_str())) {
            return Some("compares an extension with a literal");
        }
    }
    for ext in ["}.logos", "}.lg"] {
        let mut rest = line;
        while let Some(at) = rest.find(ext) {
            let after = rest[at + ext.len()..].chars().next();
            if !after.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                return Some("rebuilds a file name from its stem");
            }
            rest = &rest[at + ext.len()..];
        }
    }
    None
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The scan recognises each shape it exists to refuse, and passes the ones it must not.
#[test]
fn the_scan_knows_an_offence_when_it_sees_one() {
    for bad in [
        "if path.extension().and_then(|e| e.to_str()) == Some(\"lg\") {",
        "matches!(ext, Some(\"lg\") | Some(\"md\"))",
        ".is_some_and(|e| e == \"lg\" || e == \"md\");",
        "for ext in [\"lg\", \"md\"] {",
        "format!(\"pages/{stem}.lg does not build\")",
        "requested.join(format!(\"{dirname}.logos\"))",
    ] {
        assert!(offence(bad).is_some(), "the scan must refuse: {bad}");
    }
    for fine in [
        "fs::write(dir.join(\"program.lg\"), &bundle.program)?;",
        "let md = entry.with_extension(MODULE_EXTENSIONS[2]);",
        "format!(\"{name}.lgx\")",
        "(\"text/markdown\", \"md\"),",
    ] {
        assert!(offence(fine).is_none(), "the scan must pass: {fine}");
    }
}

/// ★ THE LOCK. Production code asks [`is_source_file`] / [`is_module_file`] and never decides for
/// itself what a LOGOS file is called.
#[test]
fn source_extension_lock() {
    let root = repo_root();
    let here = root
        .join("crates/logicaffeine_compile/src/source_files.rs")
        .canonicalize()
        .expect("the one definition exists");
    let mut found = Vec::new();
    for reader in READERS {
        let mut files = Vec::new();
        rust_files(&root.join(reader), &mut files);
        for file in files {
            if file.canonicalize().ok().as_ref() == Some(&here) {
                continue;
            }
            let text = std::fs::read_to_string(&file).expect("a readable source file");
            for (n, line) in text.lines().enumerate() {
                if line.starts_with("#[cfg(test)]") {
                    break;
                }
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if let Some(why) = offence(line) {
                    let rel = file.strip_prefix(&root).unwrap_or(&file);
                    found.push(format!("  {}:{} {why}: {}", rel.display(), n + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "production code decides what a LOGOS file is called for itself — ask \
         `logicaffeine_compile::source_files` instead, and name files as they are on disk:\n{}",
        found.join("\n")
    );
}

/// The editor opens exactly the files the toolchain reads as programs.
#[test]
fn the_vscode_language_claims_every_source_extension() {
    let manifest = repo_root().join("editors/vscode/logicaffeine/package.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest).expect("package.json"))
            .expect("package.json is JSON");
    let language = json["contributes"]["languages"]
        .as_array()
        .and_then(|langs| langs.iter().find(|l| l["id"] == "logicaffeine"))
        .expect("the logicaffeine language");
    let claimed: Vec<&str> = language["extensions"]
        .as_array()
        .expect("its extensions")
        .iter()
        .filter_map(|e| e.as_str())
        .collect();
    let expected: Vec<String> = SOURCE_EXTENSIONS.iter().map(|e| format!(".{e}")).collect();
    assert_eq!(claimed, expected, "VS Code must open every LOGOS source, canonical first");
}

/// The editor tells the language server about a change to any file a module may be.
#[test]
fn the_vscode_client_watches_every_module_extension() {
    let client = repo_root().join("editors/vscode/logicaffeine/src/extension.ts");
    let text = std::fs::read_to_string(&client).expect("extension.ts");
    let open = "createFileSystemWatcher(\"**/*.{";
    let start = text.find(open).expect("the client watches LOGOS files") + open.len();
    let end = start + text[start..].find('}').expect("the watcher glob closes");
    let mut watched: Vec<&str> = text[start..end].split(',').map(str::trim).collect();
    watched.sort_unstable();
    let mut expected = MODULE_EXTENSIONS.to_vec();
    expected.sort_unstable();
    assert_eq!(watched, expected, "the watcher glob must name every module extension");
}
