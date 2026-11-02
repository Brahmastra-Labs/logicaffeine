//! The documented `largo` surface is the one the binary parses.
//!
//! Subcommands come from the clap `Commands` enum in `apps/logicaffeine_cli/src/cli.rs` and emit targets from
//! the `EmitTarget` value enum in `apps/logicaffeine_cli/src/commands/emit.rs`. The CLI guide (`docs/cli.md`)
//! gives every subcommand a row in its command table and a `###` section, and names no subcommand the enum
//! lacks; the crate README's overview table has a row per subcommand; every enumeration of emit targets in
//! public prose is the whole set; and no public prose states how many subcommands or targets there are, since
//! a count is a claim that goes stale the day a variant is added.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test cli_docs_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::PathBuf;

use quote::ToTokens;

const CLI_RS: &str = "apps/logicaffeine_cli/src/cli.rs";
const EMIT_RS: &str = "apps/logicaffeine_cli/src/commands/emit.rs";
const GUIDE: &str = "docs/cli.md";
const CRATE_README: &str = "apps/logicaffeine_cli/README.md";

/// Public prose that describes the CLI surface.
const PROSE: &[&str] = &["README.md", GUIDE, CRATE_README];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// clap's default name for a variant: kebab-case of the identifier.
fn kebab(ident: &str) -> String {
    let mut out = String::new();
    for (i, c) in ident.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The value a `name = "…"` argument inside a `#[command(…)]`/`#[value(…)]` attribute sets, if any.
fn renamed(attr: &syn::Attribute) -> Option<String> {
    if !(attr.path().is_ident("command") || attr.path().is_ident("value")) {
        return None;
    }
    let tokens = attr.meta.to_token_stream().to_string();
    let rest = tokens.split_once("name = \"")?.1;
    Some(rest.split('"').next().unwrap_or_default().to_string())
}

fn hidden(attr: &syn::Attribute) -> bool {
    (attr.path().is_ident("command") || attr.path().is_ident("value"))
        && attr
            .meta
            .to_token_stream()
            .to_string()
            .replace(' ', "")
            .contains("hide=true")
}

/// The command-line names of `enum <name>`'s variants in `src`, as clap derives them.
fn enum_names(src: &str, name: &str) -> BTreeSet<String> {
    let file = syn::parse_file(src).unwrap_or_else(|e| panic!("does not parse: {e}"));
    let item = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(e) if e.ident == name => Some(e),
            _ => None,
        })
        .unwrap_or_else(|| panic!("`enum {name}` not found"));
    let names: BTreeSet<String> = item
        .variants
        .iter()
        .filter(|v| !v.attrs.iter().any(hidden))
        .map(|v| {
            v.attrs
                .iter()
                .find_map(renamed)
                .unwrap_or_else(|| kebab(&v.ident.to_string()))
        })
        .collect();
    assert!(!names.is_empty(), "no variants in `enum {name}`");
    names
}

fn subcommands() -> BTreeSet<String> {
    enum_names(&read(CLI_RS), "Commands")
}

fn emit_targets() -> BTreeSet<String> {
    enum_names(&read(EMIT_RS), "EmitTarget")
}

/// Every backticked span in `text`.
fn ticks(text: &str) -> impl Iterator<Item = &str> {
    text.split('`').skip(1).step_by(2)
}

/// The subcommand a table row documents: its first cell's first backticked span, minus a leading `largo `.
fn row_commands(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|l| l.starts_with('|') && !l.starts_with("|--") && !l.starts_with("| Command"))
        .filter_map(|l| {
            let first_cell = l
                .trim_start_matches('|')
                .split(" | ")
                .next()
                .unwrap_or_default();
            let words: Vec<&str> = ticks(first_cell)
                .map(|span| span.trim_start_matches("largo "))
                .filter_map(|span| span.split_whitespace().next())
                .collect();
            (!words.is_empty()).then_some(words)
        })
        .flatten()
        .map(str::to_string)
        .collect()
}

/// The subcommands the guide's `###` headings name.
fn section_commands(guide: &str) -> BTreeSet<String> {
    guide
        .lines()
        .filter(|l| l.starts_with("### "))
        .flat_map(ticks)
        .filter_map(|span| span.split_whitespace().next())
        .map(str::to_string)
        .collect()
}

fn missing(want: &BTreeSet<String>, have: &BTreeSet<String>) -> Vec<String> {
    want.difference(have).cloned().collect()
}

#[test]
fn the_parsers_see_the_real_enums() {
    let cmds = subcommands();
    for known in ["new", "build", "run", "emit", "test", "lean"] {
        assert!(
            cmds.contains(known),
            "`Commands` parse lost `{known}`: {cmds:?}"
        );
    }
    let targets = emit_targets();
    for known in ["rust", "c", "wasm", "wasm-linked", "wasm-dom"] {
        assert!(
            targets.contains(known),
            "`EmitTarget` parse lost `{known}`: {targets:?}"
        );
    }
    assert_eq!(kebab("WasmLinked"), "wasm-linked");
}

#[test]
fn every_subcommand_has_a_row_in_the_guide_table() {
    let gap = missing(&subcommands(), &row_commands(&read(GUIDE)));
    assert!(
        gap.is_empty(),
        "`{GUIDE}`'s command table has no row for {gap:?} (from the `Commands` enum in `{CLI_RS}`)"
    );
}

#[test]
fn every_subcommand_has_a_section_in_the_guide() {
    let gap = missing(&subcommands(), &section_commands(&read(GUIDE)));
    assert!(
        gap.is_empty(),
        "`{GUIDE}` has no `### \\`<name>\\`` section for {gap:?}"
    );
}

#[test]
fn the_guide_documents_no_subcommand_the_binary_lacks() {
    let cmds = subcommands();
    let extra: Vec<String> = section_commands(&read(GUIDE))
        .into_iter()
        .filter(|c| !cmds.contains(c))
        .collect();
    assert!(
        extra.is_empty(),
        "`{GUIDE}` has sections for {extra:?}, which the `Commands` enum does not declare"
    );
}

#[test]
fn every_subcommand_has_a_row_in_the_crate_readme() {
    let gap = missing(&subcommands(), &row_commands(&read(CRATE_README)));
    assert!(
        gap.is_empty(),
        "`{CRATE_README}`'s command table has no row for {gap:?}"
    );
}

#[test]
fn every_emit_target_has_a_row_in_the_guide() {
    let guide = read(GUIDE);
    let section = guide
        .split_once("### `emit`")
        .unwrap_or_else(|| panic!("`{GUIDE}` has no emit section"))
        .1;
    let section = section.split("\n### ").next().unwrap_or_default();
    let rows: BTreeSet<String> = section
        .lines()
        .filter(|l| l.starts_with("| `"))
        .filter_map(|l| ticks(l).next())
        .map(str::to_string)
        .collect();
    let gap = missing(&emit_targets(), &rows);
    assert!(
        gap.is_empty(),
        "`{GUIDE}`'s emit table has no row for {gap:?} (from `EmitTarget` in `{EMIT_RS}`)"
    );
}

#[test]
fn an_enumeration_of_emit_targets_is_the_whole_set() {
    let targets = emit_targets();
    let mut partial = Vec::new();
    for rel in PROSE {
        for span in ticks(&read(rel)) {
            let Some(rest) = span.strip_prefix("emit ") else {
                continue;
            };
            let list = rest
                .trim_start_matches('<')
                .split(['>', ' '])
                .next()
                .unwrap_or_default();
            if !list.contains('|') {
                continue;
            }
            let named: BTreeSet<String> = list
                .split('|')
                .map(|t| t.trim_matches('\\').to_string())
                .collect();
            let gap = missing(&targets, &named);
            if !gap.is_empty() {
                partial.push(format!("`{rel}`: `{span}` omits {gap:?}"));
            }
        }
    }
    assert!(
        partial.is_empty(),
        "partial emit-target enumerations (write `emit <target>`): {partial:#?}"
    );
}

const NUMBER_WORDS: &[&str] = &[
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
    "thirty",
    "forty",
    "dozen",
];

/// A count followed by `subcommands`, `commands`, `targets` or `emit targets`.
fn counted_claims(text: &str) -> Vec<String> {
    let words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || "(),;:—".contains(c))
        .filter(|w| !w.is_empty())
        .collect();
    let mut out = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let lw = w.to_ascii_lowercase();
        let is_count =
            lw.chars().all(|c| c.is_ascii_digit()) || NUMBER_WORDS.contains(&lw.as_str());
        if !is_count {
            continue;
        }
        let next = words
            .get(i + 1)
            .map(|n| n.to_ascii_lowercase())
            .unwrap_or_default();
        let after = words
            .get(i + 2)
            .map(|n| n.to_ascii_lowercase())
            .unwrap_or_default();
        let noun = next.trim_end_matches('.');
        if noun == "subcommands"
            || noun == "targets"
            || (noun == "emit" && after.starts_with("targets"))
        {
            out.push(format!("{w} {next}"));
        }
    }
    out
}

#[test]
fn a_partial_enumeration_is_seen_in_either_spelling() {
    let targets = emit_targets();
    for span in [
        "emit <rust\\|c\\|wasm>",
        "emit rust|c|wasm",
        "emit rust|c|wasm -o out.rs",
    ] {
        let rest = span.strip_prefix("emit ").unwrap();
        let list = rest
            .trim_start_matches('<')
            .split(['>', ' '])
            .next()
            .unwrap_or_default();
        let named: BTreeSet<String> = list
            .split('|')
            .map(|t| t.trim_matches('\\').to_string())
            .collect();
        assert!(
            !missing(&targets, &named).is_empty(),
            "`{span}` must read as a partial list"
        );
    }
}

#[test]
fn no_public_prose_counts_the_subcommands_or_targets() {
    assert_eq!(
        counted_claims("the ten targets and 23 subcommands"),
        ["ten targets", "23 subcommands"]
    );
    assert!(counted_claims("every subcommand, 3 flags").is_empty());
    let claims: Vec<String> = PROSE
        .iter()
        .flat_map(|rel| {
            counted_claims(&read(rel))
                .into_iter()
                .map(move |c| format!("`{rel}`: {c}"))
        })
        .collect();
    assert!(
        claims.is_empty(),
        "counts of the CLI surface go stale; name the set or link the guide: {claims:#?}"
    );
}
