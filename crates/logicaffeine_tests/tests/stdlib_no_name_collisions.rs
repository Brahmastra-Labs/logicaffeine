//! Pit-of-success lock for the stdlib `.lg` prelude modules.
//!
//! `loader::apply_prelude` splices EVERY module a program references (not transitively) into a
//! single compilation. If two modules define a `## To <name>` with the same name, a program that
//! pulls in both gets a DUPLICATE definition — a latent landmine that only fires for the specific
//! program that happens to reference a function from each colliding module. (This bit us once:
//! `bytes.lg` and `encoding.lg` both defined `hexDigit`/`hexVal`.)
//!
//! This lock makes that class of bug impossible: every `## To` function name must be unique across
//! the whole canonical prelude tree, so any two modules are always safe to co-import.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Extract the function name from a `## To [native] <name> (…)` header line, if this is one.
fn function_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("## To ")?;
    let rest = rest.strip_prefix("native ").unwrap_or(rest);
    // The name is the identifier token up to the first space or `(`.
    let end = rest
        .find(|c: char| c == ' ' || c == '(')
        .unwrap_or(rest.len());
    let name = &rest[..end];
    if !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        Some(name)
    } else {
        None
    }
}

#[test]
fn no_stdlib_function_name_is_defined_in_two_modules() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/assets/std");

    // name -> module that first defined it
    let mut owner: HashMap<String, String> = HashMap::new();
    let mut collisions: Vec<String> = Vec::new();
    let mut modules = 0;
    let mut names = 0;

    let mut entries: Vec<_> = fs::read_dir(&dir)
        .expect("canonical assets/std exists")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "lg"))
        .collect();
    entries.sort();

    for path in &entries {
        modules += 1;
        let module = path.file_name().unwrap().to_string_lossy().to_string();
        let text = fs::read_to_string(path).unwrap();
        // A module may legitimately define a name once; guard against declaring the SAME name
        // twice within one module skewing the cross-module check by tracking (name, module).
        for line in text.lines() {
            if let Some(name) = function_name(line) {
                names += 1;
                match owner.get(name) {
                    Some(other) if *other != module => collisions.push(format!(
                        "`{name}` is defined in BOTH `{other}` and `{module}` — rename one \
                         (apply_prelude splices both when a program references each, causing a \
                         duplicate definition)"
                    )),
                    _ => {
                        owner.entry(name.to_string()).or_insert(module.clone());
                    }
                }
            }
        }
    }

    assert!(
        modules >= 6,
        "the prelude tree lost modules (found {modules})"
    );
    assert!(
        names >= 200,
        "suspiciously few `## To` functions parsed ({names}) — parser drift?"
    );
    assert!(
        collisions.is_empty(),
        "stdlib prelude function-name collisions ({}):\n{}",
        collisions.len(),
        collisions.join("\n")
    );
}

#[test]
fn function_name_parser_handles_the_header_forms() {
    assert_eq!(
        function_name("## To bytesToHex (xs: Seq of Int) -> Text:"),
        Some("bytesToHex")
    );
    assert_eq!(
        function_name("## To perm (n: Int) and (k: Int) -> Int:"),
        Some("perm")
    );
    assert_eq!(
        function_name("## To native read (path: Text) -> Result:"),
        Some("read")
    );
    assert_eq!(
        function_name("## To newBuilder () -> StringBuilder:"),
        Some("newBuilder")
    );
    assert_eq!(function_name("    Let x be 1."), None);
    assert_eq!(function_name("## A StringBuilder has:"), None);
}
