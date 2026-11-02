//! No C compiler's predefined macro takes the name of a temporary the C tiers number.
//!
//! The C emitter names its temporaries by a prefix and a counter (`__r12`, `__lo3`, `__logos_k8`). Such a name is an
//! identifier only while no macro of that name exists, and C compilers predefine a few lowercase names of exactly that
//! shape: GCC and clang define `__k8` for the default x86-64 target, and `__i386` and `__i686` for 32-bit x86. A
//! temporary spelled `__k{n}` became the literal `1` wherever its counter reached 8, and the program no longer
//! compiled. Every prefix the emitter numbers is read here from the emitter's own source (both C tiers live in
//! `codegen_c`), and every C compiler on the machine is asked for its predefined macros under the flags the C builds
//! use: none may be one of those prefixes followed by a counter.

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

/// The source of the C emitter: the AST tier, the bytecode tier and their shared types.
fn emitter_sources() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src/codegen_c");
    let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .map(|p| {
            let text = std::fs::read_to_string(&p).unwrap();
            (p.file_name().unwrap().to_string_lossy().into_owned(), text)
        })
        .collect();
    files.sort();
    files
}

/// The prefixes the emitter numbers a name with: a `__name` directly followed by a format placeholder (`__r{n}`,
/// `__selslot_{}`), or passed to the temporary allocator (`temp("__at", …)`, which appends its counter).
fn numbered_prefixes(source: &str) -> BTreeSet<String> {
    let bytes = source.as_bytes();
    let mut found = BTreeSet::new();
    let mut at = 0;
    while let Some(off) = source[at..].find("__") {
        let start = at + off;
        let mut end = start + 2;
        if end < bytes.len() && bytes[end].is_ascii_lowercase() {
            while end < bytes.len() && (bytes[end].is_ascii_lowercase() || bytes[end] == b'_') {
                end += 1;
            }
            let preceded = start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
            let placeholder = bytes.get(end) == Some(&b'{') && bytes.get(end + 1) != Some(&b'{');
            let allocated = source[..start].ends_with("temp(\"") && bytes.get(end) == Some(&b'"');
            if !preceded && (placeholder || allocated) {
                found.insert(source[start..end].to_string());
            }
        }
        at = end.max(start + 2);
    }
    found
}

/// The C compilers on this machine: the one the C builds run (`$CC`, else `cc`), and `gcc` and `clang` when present.
fn compilers() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let configured = std::env::var("CC")
        .ok()
        .filter(|cc| !cc.trim().is_empty())
        .unwrap_or_else(|| "cc".to_string());
    for cc in [configured.as_str(), "gcc", "clang"] {
        let runs = Command::new(cc).arg("--version").output().is_ok_and(|o| o.status.success());
        if runs && !names.iter().any(|n| n == cc) {
            names.push(cc.to_string());
        }
    }
    names
}

/// The macros `cc` predefines under `flags`, or `None` when it does not accept them (a compiler built without a
/// 32-bit target refuses `-m32`).
fn predefined_macros(cc: &str, flags: &[&str]) -> Option<BTreeSet<String>> {
    let out = Command::new(cc)
        .args(flags)
        .args(["-dM", "-E", "-x", "c", "/dev/null"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix("#define "))
            .filter_map(|rest| rest.split([' ', '(']).next())
            .map(str::to_string)
            .collect(),
    )
}

/// Whether `name` is `prefix` followed by a counter the emitter could write: decimal digits without a leading zero.
fn numbers(prefix: &str, name: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|counter| {
        !counter.is_empty()
            && counter.bytes().all(|b| b.is_ascii_digit())
            && (counter == "0" || !counter.starts_with('0'))
    })
}

/// The flag sets the C builds compile under: a debug build's default target, a release build's host CPU, and 32-bit x86.
const FLAG_SETS: &[&[&str]] = &[&["-O0", "-g"], &["-O3", "-march=native"], &["-m32"]];

#[test]
fn no_predefined_macro_is_a_numbered_temporary() {
    let mut prefixes = BTreeSet::new();
    for (_, text) in emitter_sources() {
        prefixes.extend(numbered_prefixes(&text));
    }
    let compilers = compilers();
    assert!(!compilers.is_empty(), "no C compiler runs on this machine (tried $CC, cc, gcc, clang)");
    let mut collisions = Vec::new();
    let mut asked = 0;
    for cc in &compilers {
        for flags in FLAG_SETS {
            let Some(macros) = predefined_macros(cc, flags) else {
                continue;
            };
            asked += 1;
            for name in &macros {
                for prefix in prefixes.iter().filter(|p| numbers(p, name)) {
                    collisions.push(format!("{cc} {}: `{name}` is the temporary `{prefix}{{n}}`", flags.join(" ")));
                }
            }
        }
    }
    assert!(asked > 0, "no compiler in {compilers:?} listed its predefined macros");
    assert!(
        collisions.is_empty(),
        "a predefined macro replaces a temporary the C emitter numbers:\n  {}\n\nprefixes read from the emitter: {prefixes:?}",
        collisions.join("\n  ")
    );
}

#[test]
fn the_census_reads_every_way_the_emitter_numbers_a_name() {
    let source = r#"
        format!("int64_t __lo{n} = {}; Seq_i64 __r{n} = x;", a);
        let t = ctx.temp("__at", CType::Int64);
        let s = format!("__selslot_{}", k);
        let c = format!("__c{}", n);
        let not_numbered = "__builtin_expect(x, 1) {{ __attribute__ }}";
        let escaped = format!("{{ __x{{ }}");
        let inner = "a__b{n}";
    "#;
    let found: Vec<String> = numbered_prefixes(source).into_iter().collect();
    assert_eq!(found, ["__at", "__c", "__lo", "__r", "__selslot_"]);
}

#[test]
fn a_counter_is_digits_without_a_leading_zero() {
    assert!(numbers("__k", "__k8"));
    assert!(numbers("__i", "__i386"));
    assert!(numbers("__r", "__r0"));
    assert!(!numbers("__k", "__k"));
    assert!(!numbers("__k", "__k08"));
    assert!(!numbers("__x", "__x86_64"));
    assert!(!numbers("__lo", "__logos_k8"));
}

#[test]
fn the_listing_read_is_the_compilers_own() {
    if !cfg!(target_arch = "x86_64") {
        return;
    }
    let cc = compilers().into_iter().next().expect("a C compiler");
    let macros = predefined_macros(&cc, &["-O0"]).expect("the compiler lists its macros");
    assert!(
        macros.contains("__k8") && macros.contains("__amd64"),
        "{cc} names `__k8` and `__amd64` for the default x86-64 target; the listing read has neither"
    );
}
