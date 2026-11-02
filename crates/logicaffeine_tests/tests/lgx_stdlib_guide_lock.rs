//! ★ LOCK — `docs/lgx-stdlib.md` documents `forms.lg`, `ui.lg` and `session.lg` from their own
//! source, with an example project that really uses each. ★
//!
//! Three things hold here:
//!
//! - each module's definition table in the guide is GENERATED from the module's `## Note`s (the
//!   first paragraph of each, the summary hover shows) through `teach::extract_literate_docs`, and
//!   the text between the guide's `<!-- generated from <m>.lg … -->` markers is that table byte for
//!   byte;
//! - each module's introduction and its section of the guide name the same example project, the
//!   project exists, and the loader splices the module into at least one of its pages — the
//!   loader's own demand, so a `## Session` block that makes the compiler call `signSession`
//!   counts as using `session.lg` exactly as a written `errorFor(…)` counts as using `forms.lg`;
//! - every `examples/<name>` the guide or the three modules name exists.
//!
//! The page's LOGOS blocks are run by `canonical_teaching_lock` (every `logos-dom` page) and
//! `doc_examples` (every `logos` program and its `output`), which read every file in `docs/`.
//!
//! Regenerate the tables: `./scripts/release/lgx/gen-reference.sh`.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::analysis::assemble::assemble_project;
use logicaffeine_compile::loader::apply_prelude;
use logicaffeine_language::teach::{extract_literate_docs, module_doc};
use std::path::{Path, PathBuf};

const GUIDE: &str = "docs/lgx-stdlib.md";
const REGENERATE: &str = "./scripts/release/lgx/gen-reference.sh";

/// `(module, the guide's section heading, the example project that uses it)`.
const WEB_MODULES: &[(&str, &str, &str)] = &[
    ("forms", "## Forms — `forms.lg`", "signup_form"),
    ("ui", "## UI — `ui.lg`", "data_table"),
    ("session", "## Sessions — `session.lg`", "login"),
];

const END_MARKER: &str = "<!-- end generated -->";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

fn module_source(module: &str) -> String {
    read(&format!(
        "crates/logicaffeine_compile/assets/std/{module}.lg"
    ))
}

fn start_marker(module: &str) -> String {
    format!("<!-- generated from {module}.lg by lgx_stdlib_guide_lock: {REGENERATE} -->")
}

/// A `##` definition header as a reader writes a call: `## To f (a: T) and (b: U) -> R:` is
/// `f(a: T, b: U) -> R`; a type header keeps its sentence without the `## `.
fn definition_form(signature: &str) -> String {
    let Some(rest) = signature.strip_prefix("## To ") else {
        return signature
            .trim_start_matches("## ")
            .trim_end_matches(':')
            .to_string();
    };
    let (head, ret) = match rest.split_once("->") {
        Some((h, r)) => (h, Some(r.trim().trim_end_matches(':').trim())),
        None => (rest.trim_end_matches(':'), None),
    };
    let name = head
        .split(|c: char| c == '(' || c.is_whitespace())
        .next()
        .unwrap_or("");
    let mut params = Vec::new();
    let mut scan = head;
    while let Some(open) = scan.find('(') {
        let Some(close) = scan[open..].find(')') else {
            break;
        };
        let group = scan[open + 1..open + close].trim();
        if !group.is_empty() {
            params.push(group.to_string());
        }
        scan = &scan[open + close + 1..];
    }
    match ret {
        Some(r) => format!("{name}({}) -> {r}", params.join(", ")),
        None => format!("{name}({})", params.join(", ")),
    }
}

/// A table cell: a `|` would end it.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// The generated table of one module: every public definition, with its note's first paragraph.
fn table(module: &str) -> String {
    let src = module_source(module);
    let mut out = String::from("| Definition | What it is |\n|---|---|\n");
    let mut rows = 0;
    for doc in extract_literate_docs(&src) {
        if doc.signature.starts_with("## To private ") || doc.signature.starts_with("## A private ")
        {
            continue;
        }
        let note = doc
            .doc
            .as_deref()
            .unwrap_or_else(|| panic!("{module}.lg: `{}` carries no `## Note`", doc.signature));
        let summary = note
            .split("\n\n")
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!(
            "| {} | {} |\n",
            cell(&format!("`{}`", definition_form(&doc.signature))),
            cell(&summary)
        ));
        rows += 1;
    }
    assert!(rows > 0, "{module}.lg declares no public definition");
    out
}

/// The whole generated region of one module, markers included.
fn region(module: &str) -> String {
    format!(
        "{}\n\n{}\n{END_MARKER}",
        start_marker(module),
        table(module)
    )
}

/// Where the region of `module` sits in `guide`: `(start, end)` byte offsets, end exclusive.
fn region_span(guide: &str, module: &str) -> Option<(usize, usize)> {
    let prefix = format!("<!-- generated from {module}.lg ");
    let start = guide.find(&prefix)?;
    let end = start + guide[start..].find(END_MARKER)? + END_MARKER.len();
    Some((start, end))
}

/// The guide with every module's region replaced by its generated text.
fn regenerated(guide: &str) -> String {
    let mut out = guide.to_string();
    for (module, ..) in WEB_MODULES {
        let (start, end) = region_span(&out, module).unwrap_or_else(|| {
            panic!("{GUIDE} has no `<!-- generated from {module}.lg … -->` … `{END_MARKER}` region")
        });
        out.replace_range(start..end, &region(module));
    }
    out
}

/// The text of the guide's section under `heading`, up to the next `## ` heading outside a fence
/// (a LOGOS block's own `## Page` lines are not headings of the guide).
fn section<'g>(guide: &'g str, heading: &str) -> &'g str {
    let at = guide
        .find(&format!("\n{heading}\n"))
        .unwrap_or_else(|| panic!("{GUIDE} has no `{heading}` section"));
    let body = &guide[at + heading.len() + 2..];
    let mut in_fence = false;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        } else if !in_fence && line.starts_with("## ") {
            return &body[..offset];
        }
        offset += line.len();
    }
    body
}

/// Every `examples/<name>` a text mentions.
fn example_mentions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("examples/") {
        let tail = &rest[at + "examples/".len()..];
        let name: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
        rest = tail;
    }
    out
}

/// The `.lg` sources a project ships, sorted: `pages/*.lg` and `src/*.lg`.
fn project_sources(project: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["pages", "src"] {
        let Ok(entries) = std::fs::read_dir(project.join(dir)) else {
            continue;
        };
        out.extend(
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("lg")),
        );
    }
    out.sort();
    out
}

/// Whether the loader splices `module` into `source`: its prelude expansion carries a header the
/// module declares.
fn splices(source: &str, module: &str) -> bool {
    let expanded = apply_prelude(source);
    let module_src = module_source(module);
    extract_literate_docs(&module_src)
        .iter()
        .filter(|d| d.signature.starts_with("## To "))
        .any(|d| expanded.lines().any(|l| l.trim_end() == d.signature))
}

/// With `LOGOS_REGEN=1` (`./scripts/release/lgx/gen-reference.sh`) it rewrites the guide's generated regions first.
#[test]
fn each_generated_table_is_the_modules_own_notes() {
    if std::env::var_os("LOGOS_REGEN").is_some() {
        let guide = read(GUIDE);
        std::fs::write(repo_root().join(GUIDE), regenerated(&guide)).expect("write the guide");
    }
    let committed = read(GUIDE);
    let generated = regenerated(&committed);
    if committed != generated {
        let at = committed
            .lines()
            .zip(generated.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| committed.lines().count().min(generated.lines().count()));
        panic!(
            "{GUIDE}'s definition tables are not the modules' own notes — a `## Note` or a definition \
             changed, or a table was edited by hand. Regenerate:\n\n    {REGENERATE}\n\nFirst difference at \
             line {}:\n  committed: {:?}\n  generated: {:?}",
            at + 1,
            committed.lines().nth(at).unwrap_or("<end of file>"),
            generated.lines().nth(at).unwrap_or("<end of file>"),
        );
    }
}

#[test]
fn every_generated_region_belongs_to_a_documented_module() {
    let guide = read(GUIDE);
    let regions: Vec<String> = guide
        .match_indices("<!-- generated from ")
        .map(|(at, _)| {
            guide[at + "<!-- generated from ".len()..]
                .split(".lg ")
                .next()
                .unwrap_or("")
                .to_string()
        })
        .collect();
    let known: Vec<&str> = WEB_MODULES.iter().map(|(m, ..)| *m).collect();
    assert_eq!(
        regions, known,
        "{GUIDE} carries one generated region per web module, in order"
    );
    for (module, heading, _) in WEB_MODULES {
        assert!(
            section(&guide, heading).contains(&start_marker(module)),
            "{module}.lg's table sits in its own section, `{heading}`"
        );
    }
}

#[test]
fn each_module_section_names_a_landed_example_that_uses_it() {
    let guide = read(GUIDE);
    let mut wrong = Vec::new();
    for (module, heading, example) in WEB_MODULES {
        let intro = module_doc(&module_source(module)).unwrap_or_default();
        if !example_mentions(&intro).contains(&example.to_string()) {
            wrong.push(format!(
                "{module}.lg's introduction does not name examples/{example}"
            ));
        }
        if !example_mentions(section(&guide, heading)).contains(&example.to_string()) {
            wrong.push(format!(
                "{GUIDE}'s `{heading}` section does not name examples/{example}"
            ));
        }
        let project = repo_root().join("examples").join(example);
        if !project.join("Largo.toml").is_file() {
            wrong.push(format!(
                "examples/{example} is not a project (no Largo.toml)"
            ));
            continue;
        }
        let sources = project_sources(&project);
        let uses = sources.iter().any(|page| {
            let assembled = assemble_project(page)
                .unwrap_or_else(|e| panic!("{} does not assemble: {e}", page.display()));
            splices(&assembled.source, module)
        });
        if !uses {
            wrong.push(format!(
                "examples/{example} is named as {module}.lg's example, and the loader splices {module}.lg into none of \
                 its sources {sources:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the web modules' examples do not hold:\n  {}",
        wrong.join("\n  ")
    );
}

#[test]
fn every_example_the_guide_or_the_modules_name_exists() {
    let mut texts = vec![(GUIDE.to_string(), read(GUIDE))];
    for (module, ..) in WEB_MODULES {
        texts.push((format!("{module}.lg"), module_source(module)));
    }
    let mut missing = Vec::new();
    for (name, text) in &texts {
        for example in example_mentions(text) {
            if !repo_root()
                .join("examples")
                .join(&example)
                .join("Largo.toml")
                .is_file()
            {
                missing.push(format!(
                    "{name} names examples/{example}, which is not a project"
                ));
            }
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

/// The splice check discriminates: a program that uses no web module splices none, and a program
/// calling one function of a module splices that module and not the others.
#[test]
fn the_splice_check_tells_use_from_absence() {
    let plain = "## Main\nShow 1 + 1.\n";
    for (module, ..) in WEB_MODULES {
        assert!(
            !splices(plain, module),
            "a program using nothing of {module}.lg splices it"
        );
    }
    let one = "## Main\nShow pageCount(10, 3).\n";
    assert!(splices(one, "ui"), "a call to pageCount splices ui.lg");
    assert!(
        !splices(one, "forms") && !splices(one, "session"),
        "a ui.lg call splices only ui.lg"
    );
    assert_eq!(
        definition_form("## To pageEnd (page: Int) and (perPage: Int) and (total: Int) -> Int:"),
        "pageEnd(page: Int, perPage: Int, total: Int) -> Int"
    );
    assert_eq!(
        definition_form("## To noErrors () -> Seq of FieldError:"),
        "noErrors() -> Seq of FieldError"
    );
    assert_eq!(definition_form("## A FieldError has:"), "A FieldError has");
    assert_eq!(
        example_mentions("see [x](../examples/login) and `examples/data_table`."),
        ["login", "data_table"]
    );
    let guide = "# G\n\n## A\n\none\n\n```logos\n## Main\nShow 1.\n```\n\ntwo\n\n## B\n\nthree\n";
    assert_eq!(
        section(guide, "## A"),
        "\none\n\n```logos\n## Main\nShow 1.\n```\n\ntwo\n\n"
    );
    assert_eq!(section(guide, "## B"), "\nthree\n");
}
