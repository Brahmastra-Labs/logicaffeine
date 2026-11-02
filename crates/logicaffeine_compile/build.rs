//! Embeds the prelude's modules and writes the index `stdlib_text::index` derives from them, so no process reads
//! the library's text to learn what it defines, what it pulls in or where its functions lie.

#[allow(dead_code)]
#[path = "src/stdlib_text.rs"]
mod stdlib_text;

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let std_dir = Path::new(&manifest).join("assets").join("std");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/stdlib_text.rs");
    let paths: Vec<_> = stdlib_text::PRELUDE_FILES
        .iter()
        .map(|stem| std_dir.join(format!("{stem}.lg")))
        .collect();
    let sources: Vec<String> = paths
        .iter()
        .map(|path| {
            println!("cargo:rerun-if-changed={}", path.display());
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect();
    let modules: Vec<&str> = sources.iter().map(String::as_str).collect();
    let index = stdlib_text::index(&modules);
    let count = modules.len();
    assert!(count <= u8::MAX as usize, "a module index must fit a byte");

    let mut out = String::new();
    for (stem, path) in stdlib_text::PRELUDE_FILES.iter().zip(&paths) {
        let path = path.to_str().expect("a UTF-8 path to the stdlib");
        writeln!(out, "/// `assets/std/{stem}.lg`.").unwrap();
        writeln!(
            out,
            "pub(crate) const STD_{}: &str = include_str!({path:?});",
            stem.to_uppercase()
        )
        .unwrap();
    }
    writeln!(out, "/// The prelude's modules in splice order.").unwrap();
    let consts: Vec<String> = stdlib_text::PRELUDE_FILES
        .iter()
        .map(|s| format!("STD_{}", s.to_uppercase()))
        .collect();
    writeln!(
        out,
        "pub(crate) static PRELUDE_MODULES: [&str; {count}] = [{}];",
        consts.join(", ")
    )
    .unwrap();

    let names = |lists: &[Vec<String>]| -> String {
        lists
            .iter()
            .map(|names| {
                format!(
                    "&[{}]",
                    names
                        .iter()
                        .map(|n| format!("{n:?}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join(",\n    ")
    };
    writeln!(out, "/// Per module, the names it defines.").unwrap();
    writeln!(
        out,
        "pub(crate) static PRELUDE_NAMES: [&[&str]; {count}] = [\n    {}\n];",
        names(&index.names)
    )
    .unwrap();
    writeln!(
        out,
        "/// Per module, the names its spliced code declares `## To native`."
    )
    .unwrap();
    writeln!(
        out,
        "pub(crate) static PRELUDE_NATIVES: [&[&str]; {count}] = [\n    {}\n];",
        names(&index.natives)
    )
    .unwrap();
    writeln!(
        out,
        "/// Per module, the other modules whose names its spliced code uses."
    )
    .unwrap();
    let pulls: Vec<String> = index
        .pulls
        .iter()
        .map(|p| {
            format!(
                "&[{}]",
                p.iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect();
    writeln!(
        out,
        "pub(crate) static PRELUDE_PULLS: [&[u8]; {count}] = [\n    {}\n];",
        pulls.join(",\n    ")
    )
    .unwrap();
    writeln!(
        out,
        "/// Per module, whether its spliced code mentions `text_from_bytes`."
    )
    .unwrap();
    let mentions: Vec<String> = index
        .mentions_text_from_bytes
        .iter()
        .map(|b| b.to_string())
        .collect();
    writeln!(
        out,
        "pub(crate) static PRELUDE_MENTIONS_TEXT_FROM_BYTES: [bool; {count}] = [{}];",
        mentions.join(", ")
    )
    .unwrap();
    writeln!(
        out,
        "/// Per module, the byte ranges of its source whose concatenation is its spliced code."
    )
    .unwrap();
    let splice: Vec<String> = index
        .splice
        .iter()
        .map(|runs| {
            format!(
                "&[{}]",
                runs.iter()
                    .map(|(s, e)| format!("({s}, {e})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect();
    writeln!(
        out,
        "pub(crate) static PRELUDE_SPLICE: [&[(u32, u32)]; {count}] = [\n    {}\n];",
        splice.join(",\n    ")
    )
    .unwrap();
    writeln!(
        out,
        "/// Every name a module's code opens a `## To` header line with, sorted."
    )
    .unwrap();
    let functions: Vec<String> = index.functions.iter().map(|n| format!("{n:?}")).collect();
    writeln!(
        out,
        "pub(crate) static PRELUDE_FUNCTIONS: [&str; {}] = [\n    {}\n];",
        functions.len(),
        functions.join(",\n    ")
    )
    .unwrap();
    writeln!(out, "/// Every library `## To` section as (name, module, start, end) into the module's source, sorted by name.")
        .unwrap();
    let library: Vec<String> = index
        .library
        .iter()
        .map(|(n, m, s, e)| format!("({n:?}, {m}, {s}, {e})"))
        .collect();
    writeln!(
        out,
        "pub(crate) static LIBRARY_SECTIONS: [(&str, u8, u32, u32); {}] = [\n    {}\n];",
        library.len(),
        library.join(",\n    ")
    )
    .unwrap();

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    std::fs::write(Path::new(&out_dir).join("stdlib_index.rs"), out)
        .expect("write the stdlib index");
}
