//! A LOGOS name is never spelled, in the emitted C, as a name the C tier's own headers declare.
//!
//! The C tier includes the C library's headers (`stdio.h`, `stdlib.h`, `unistd.h`, `sys/socket.h`, …), and
//! every name they declare — a function, a typedef, an object, a macro — is in scope in the program's file.
//! A LOGOS function, record or program binding spelled as one of them does not build (`## To rename (n:
//! Text)` against stdio's `int rename(const char *, const char *)`), or worse, a macro rewrites it.
//! `codegen_c::types::escape_c_ident` mangles every such name (`logos_rename`) at the one choke point every
//! spelling passes through, reading the declared names from `c_header_names.txt`. This lock reads the headers
//! themselves, through the system compiler, and fails when the list misses a name they declare — a new
//! header, or another C library — naming the fix: `C_HEADER_NAMES_RECORD=1` rewrites the list.

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::codegen_c::cshake::{parse_c_units, UnitKind};
use logicaffeine_compile::compile::compile_to_c;

fn compile_crate() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile")
}

fn list_path() -> PathBuf {
    compile_crate().join("src/codegen_c/c_header_names.txt")
}

/// Every `#include <…>` the C family's runtimes carry, with the platform it is for: an Apple-only header sits
/// inside `#if defined(__APPLE__)`, and the system compiler that reads the headers decides, as it does
/// building a program, whether this platform includes it. Only a platform guard is kept; a header guarded by
/// the runtime's own configuration (`#if LOGOS_SCHED_WORKERS > 1`) is a header some program includes, and
/// its names are read here whatever the configuration.
fn included_headers() -> BTreeSet<(Vec<String>, String)> {
    let mut headers = BTreeSet::new();
    for dir in ["src/codegen_c", "src/codegen_riscv"] {
        for entry in std::fs::read_dir(compile_crate().join(dir)).unwrap() {
            let path = entry.unwrap().path();
            if !matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "c")) {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let mut guards: Vec<Guard> = Vec::new();
            for line in text.lines() {
                let directive = line.trim_start();
                if let Some(cond) = directive.strip_prefix("#ifdef ") {
                    guards.push(Guard::new(format!("defined({})", cond.trim())));
                } else if let Some(cond) = directive.strip_prefix("#ifndef ") {
                    guards.push(Guard::new(format!("!defined({})", cond.trim())));
                } else if let Some(cond) = directive.strip_prefix("#if ") {
                    guards.push(Guard::new(cond.trim().to_string()));
                } else if let Some(cond) = directive.strip_prefix("#elif ") {
                    if let Some(guard) = guards.last_mut() {
                        guard.elif(cond.trim().to_string());
                    }
                } else if directive.starts_with("#else") {
                    if let Some(guard) = guards.last_mut() {
                        guard.otherwise();
                    }
                } else if directive.starts_with("#endif") {
                    guards.pop();
                }
                let mut rest = line;
                while let Some(at) = rest.find("#include <") {
                    let after = &rest[at + "#include <".len()..];
                    let end = after.find('>').unwrap();
                    let platform = guards
                        .iter()
                        .filter(|g| g.is_platform())
                        .map(Guard::condition)
                        .collect();
                    headers.insert((platform, after[..end].to_string()));
                    rest = &after[end..];
                }
            }
        }
    }
    headers
}

/// The macros a C compiler predefines for the platform it builds for.
const PLATFORM_MACROS: &[&str] = &[
    "__APPLE__",
    "__MACH__",
    "__GLIBC__",
    "__linux__",
    "__unix__",
    "__FreeBSD__",
    "_WIN32",
];

/// One open `#if` chain: the condition its current branch is taken under.
struct Guard {
    /// Every earlier branch's condition, none of which holds in a later branch.
    earlier: Vec<String>,
    current: String,
}

impl Guard {
    fn new(condition: String) -> Self {
        Guard {
            earlier: Vec::new(),
            current: condition,
        }
    }

    fn elif(&mut self, condition: String) {
        self.earlier.push(std::mem::replace(&mut self.current, condition));
    }

    fn otherwise(&mut self) {
        self.earlier
            .push(std::mem::replace(&mut self.current, "1".to_string()));
    }

    fn condition(&self) -> String {
        self.earlier
            .iter()
            .map(|c| format!("!({c}) && "))
            .chain(std::iter::once(format!("({})", self.current)))
            .collect()
    }

    /// Whether the chain asks only which platform the compiler builds for: a branch names a platform macro,
    /// and no branch names anything else (an `#else` asks nothing of its own).
    fn is_platform(&self) -> bool {
        let mut names_a_platform = false;
        let only_platforms = self.earlier.iter().chain([&self.current]).all(|c| {
            let mut rest = c.clone();
            for name in PLATFORM_MACROS {
                rest = rest.replace(&format!("defined({name})"), "");
            }
            names_a_platform |= rest != *c;
            rest.chars().all(|ch| "()!|& 1".contains(ch))
        });
        names_a_platform && only_platforms
    }
}

/// A name a LOGOS program could spell: no leading underscore (the implementation's own).
fn spellable(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The names the headers declare at file scope, and the macros they define, read through the system compiler.
fn declared_names(headers: &BTreeSet<(Vec<String>, String)>) -> BTreeSet<String> {
    let source: String = headers
        .iter()
        .map(|(conditions, h)| {
            if conditions.is_empty() {
                format!("#include <{h}>\n")
            } else {
                format!("#if {}\n#include <{h}>\n#endif\n", conditions.join(" && "))
            }
        })
        .collect();
    let dir = std::env::temp_dir().join(format!("logos_c_header_names_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("headers.c");
    std::fs::write(&file, source).unwrap();
    let out = Command::new("gcc")
        .args(["-E", "-dD", "-x", "c"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "gcc could not preprocess the C tier's headers:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).to_string();

    let mut names = BTreeSet::new();
    let mut code = String::new();
    for line in text.lines() {
        match line.trim_start().strip_prefix("#define ") {
            Some(def) => {
                let end = def
                    .find(|c: char| c == '(' || c.is_whitespace())
                    .unwrap_or(def.len());
                names.insert(def[..end].to_string());
            }
            None if line.trim_start().starts_with('#') => {}
            None => {
                code.push_str(line);
                code.push('\n');
            }
        }
    }
    for unit in parse_c_units(&code) {
        if unit.kind != UnitKind::Preproc {
            names.extend(unit.defined.iter().cloned());
        }
        if let Some(body) = unit
            .text
            .find("enum")
            .and_then(|at| unit.text[at..].find('{').map(|b| at + b))
        {
            // An enum's constants: the name opening each item of its body.
            let close = unit.text[body..]
                .find('}')
                .map(|c| body + c)
                .unwrap_or(unit.text.len());
            for item in unit.text[body + 1..close].split(',') {
                let name = item
                    .trim()
                    .split(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
                    .next()
                    .unwrap_or("");
                names.insert(name.to_string());
            }
        }
    }
    names.retain(|n| spellable(n));
    names
}

fn checked_in() -> BTreeSet<String> {
    std::fs::read_to_string(list_path())
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
fn the_list_holds_every_name_the_c_tiers_headers_declare() {
    if !common::require_tool("gcc") {
        return;
    }
    let declared = declared_names(&included_headers());
    let listed = checked_in();
    if std::env::var_os("C_HEADER_NAMES_RECORD").is_some() {
        let all: BTreeSet<String> = listed.union(&declared).cloned().collect();
        let text: String = all.iter().map(|n| format!("{n}\n")).collect();
        std::fs::write(list_path(), text).unwrap();
        return;
    }
    let missing: Vec<&String> = declared.difference(&listed).collect();
    assert!(
        missing.is_empty(),
        "{} names the C tier's headers declare are missing from {}; a LOGOS name spelled as one of them does \
         not build. Rewrite the list with `C_HEADER_NAMES_RECORD=1`:\n{:?}",
        missing.len(),
        list_path().display(),
        missing
    );
}

#[test]
fn the_list_is_sorted_and_every_line_a_name() {
    let text = std::fs::read_to_string(list_path()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(!lines.is_empty(), "the list is empty");
    for line in &lines {
        assert!(
            spellable(line),
            "`{line}` is not a name a LOGOS program could spell"
        );
    }
    let mut sorted = lines.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        lines, sorted,
        "the list is sorted, one name a line, no repeats"
    );
}

#[test]
fn a_program_spelled_like_the_c_library_builds_and_runs() {
    if !common::require_tool("gcc") {
        return;
    }
    // Functions from stdio, sys/socket and string.h, and a program binding a function reaches from time.h —
    // each a name the headers declare.
    let src = "## To rename (n: Text) -> Text:\n    Return n + \"!\".\n\n\
               ## To bind (x: Int) -> Int:\n    Return x + time.\n\n\
               ## To index -> Int:\n    Return 3.\n\n\
               ## To access:\n    Set time to time + 1.\n\n\
               ## Main\nLet mutable time be 10.\naccess().\nShow rename(\"a\").\nShow bind(1).\nShow index().\n";
    let c = compile_to_c(src).expect("the program lowers to C");
    let dir = std::env::temp_dir().join(format!("logos_c_header_spelling_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (file, bin) = (dir.join("prog.c"), dir.join("prog"));
    std::fs::write(&file, &c).unwrap();
    let out = Command::new("gcc")
        .args(["-O1", "-o"])
        .arg(&bin)
        .arg(&file)
        .arg("-lm")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "does not build:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = Command::new(&bin).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "a!\n12\n3\n");
}
