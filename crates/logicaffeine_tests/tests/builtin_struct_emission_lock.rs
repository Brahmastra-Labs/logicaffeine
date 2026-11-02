//! A compiler-provided struct is in a program's generated code only when the program uses it.
//!
//! `HttpError` (a typed fetch's `Err`) and `File` (a dropped file) are registered in every program's
//! type registry, so their field layouts are known wherever one appears. Emitting every registered
//! struct put both — and their `Show` impls — into every compiled program, a three-line one included.
//! The AOT-Rust and C emitters now define a builtin struct only when the program names it (a
//! construction, an annotation) or a type it defines holds one; a program that does use one still
//! builds and runs it on every tier.

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

use logicaffeine_compile::compile::{compile_to_c, compile_to_rust};

const BUILTIN_STRUCTS: &[&str] = &["HttpError", "File"];

fn rust_defines(rust: &str, name: &str) -> bool {
    rust.contains(&format!("pub struct {name} "))
}

fn c_defines(c: &str, name: &str) -> bool {
    c.contains(&format!("typedef struct {name} {name};"))
}

fn on_every_tier(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, expected);
}

#[test]
fn a_program_that_uses_no_builtin_struct_defines_none() {
    let programs = [
        "## Main\nLet mutable text be \"hello\".\nSet text to text + \" world\".\nShow text.\n",
        "## A Point has:\n    An x: Int.\n\n## Main\nLet p be a new Point with x 3.\nShow p.\n",
        "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(21).\n",
    ];
    for src in programs {
        let rust =
            compile_to_rust(src).unwrap_or_else(|e| panic!("compiles to Rust: {e:?}\n{src}"));
        let c = compile_to_c(src).unwrap_or_else(|e| panic!("compiles to C: {e:?}\n{src}"));
        for name in BUILTIN_STRUCTS {
            assert!(
                !rust_defines(&rust, name),
                "the Rust of a program that never names `{name}` defines it:\n{src}\n{rust}"
            );
            assert!(
                !c_defines(&c, name),
                "the C of a program that never names `{name}` defines it:\n{src}\n{c}"
            );
        }
    }
}

#[test]
fn a_program_that_builds_an_http_error_defines_it_and_runs() {
    let src = "## Main\nLet e be a new HttpError with status 404 and message \"missing\".\nShow e's status.\nShow e's message.\n";
    let rust = compile_to_rust(src).expect("compiles to Rust");
    assert!(
        rust_defines(&rust, "HttpError"),
        "a program that builds an `HttpError` must define it:\n{rust}"
    );
    assert!(
        !rust_defines(&rust, "File"),
        "…and only the builtin struct it uses:\n{rust}"
    );
    on_every_tier(src, "404\nmissing");
}

#[test]
fn a_program_that_takes_a_file_defines_it_and_runs() {
    let src = "## To describe (f: File) -> Text:\n    Return f's name.\n\n## Main\nLet f be a new File with name \"notes.txt\" and size 12.\nShow describe(f).\nShow f's size.\n";
    let rust = compile_to_rust(src).expect("compiles to Rust");
    assert!(
        rust_defines(&rust, "File"),
        "a program that takes a `File` must define it:\n{rust}"
    );
    on_every_tier(src, "notes.txt\n12");
}

#[test]
fn a_record_that_holds_a_builtin_struct_brings_it_along() {
    let src = "## An Upload has:\n    A file: File.\n    An owner: Text.\n\n## Main\nLet u be a new Upload with file (a new File with name \"a.png\" and size 3) and owner \"ada\".\nShow u's owner.\n";
    let rust = compile_to_rust(src).expect("compiles to Rust");
    assert!(
        rust_defines(&rust, "File"),
        "a record with a `File` field must bring `File` along:\n{rust}"
    );
    let c = compile_to_c(src).expect("compiles to C");
    assert!(
        c_defines(&c, "File"),
        "a record with a `File` field must bring `File` along in C:\n{c}"
    );
    on_every_tier(src, "ada");
}
