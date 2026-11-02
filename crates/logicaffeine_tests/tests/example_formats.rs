//! Every Logic, Math and Hardware example is written in its kind's clean format — the text a
//! reader meets first in the Studio. Common to all: no tabs or trailing space, one final newline,
//! no leading or doubled blank lines, lines within 100 columns, and a comment header that names
//! the example. Each kind adds its own layout rules (see `docs/plans/studio-editor-formats.md`).
//! Whether an example still does what it promises is `example_health`'s and the contract's to say;
//! this holds its shape.

#![cfg(feature = "web-tests")]

use logicaffeine_web::ui::examples::{ExampleSpec, ALL_HARDWARE_SPECS, ALL_LOGIC_EXAMPLES, ALL_MATH_EXAMPLES};

const WIDTH: usize = 100;

/// Math examples whose banner rules and wide lines another suite pins byte for byte, so this lock
/// leaves those two rules to it and holds the rest: `studio_math_document_door` reads the Collatz
/// example as written, from its opening `-- ====` banner on.
const LAYOUT_PINNED_ELSEWHERE: &[&str] = &["/examples/math/collatz.logos"];

/// A fault the pinned layout owns: a banner rule, or a line past [`WIDTH`].
fn pinned_layout_fault(fault: &str) -> bool {
    fault.ends_with("a section is named by a plain `-- Title`") || fault.ends_with(" columns wide")
}

/// What is wrong with `example`'s layout in the ways every kind shares, given its comment marker.
fn common_faults(example: &ExampleSpec, marker: &str) -> Vec<String> {
    let text = example.source;
    let mut faults = Vec::new();
    if !text.ends_with('\n') || text.ends_with("\n\n") {
        faults.push("does not end with exactly one newline".into());
    }
    if text.starts_with('\n') {
        faults.push("starts with a blank line".into());
    }
    if text.contains("\n\n\n") {
        faults.push("has two blank lines in a row".into());
    }
    for (n, line) in text.lines().enumerate() {
        let at = n + 1;
        if line.contains('\t') {
            faults.push(format!("line {at} has a tab"));
        }
        if line != line.trim_end() {
            faults.push(format!("line {at} has trailing space"));
        }
        if line.chars().count() > WIDTH {
            faults.push(format!("line {at} is {} columns wide", line.chars().count()));
        }
    }
    match text.lines().next() {
        Some(first) if first.starts_with(&format!("{marker} ")) && first.len() > marker.len() + 1 => {
            if first.contains('&') {
                faults.push(format!("the title {first:?} uses '&' for 'and'"));
            }
        }
        first => faults.push(format!("does not open with a `{marker} Title` line: {first:?}")),
    }
    faults
}

fn report(kind: &str, faults: Vec<(&str, Vec<String>)>) {
    let faults: Vec<String> = faults
        .into_iter()
        .filter(|(_, f)| !f.is_empty())
        .map(|(path, f)| format!("{path}:\n  {}", f.join("\n  ")))
        .collect();
    assert!(faults.is_empty(), "{kind} examples out of format:\n{}", faults.join("\n"));
}

/// Leading spaces of each non-blank line, as (1-based line, count).
fn indents(text: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(n, l)| (n + 1, l.len() - l.trim_start().len(), l))
}

#[test]
fn logic_examples_are_in_the_logic_format() {
    let mut all = Vec::new();
    for example in ALL_LOGIC_EXAMPLES {
        let mut faults = common_faults(example, "#");
        for (at, indent, _) in indents(example.source) {
            if indent > 0 {
                faults.push(format!("line {at} is indented; Logic is written at the margin"));
            }
        }
        if example.source.contains("## Theorem:") {
            let body: Vec<&str> = example.source.lines().filter(|l| !l.trim().is_empty() && !l.starts_with("# ")).collect();
            let place = |prefix: &str| body.iter().position(|l| l.starts_with(prefix));
            match (place("## Theorem:"), place("Prove:"), body.last()) {
                (Some(header), Some(prove), Some(last)) => {
                    if !last.starts_with("Proof:") {
                        faults.push(format!("the proof is not the theorem's last line: {last:?}"));
                    }
                    if body[header + 1..prove].iter().any(|l| !l.starts_with("Given")) {
                        faults.push("something other than a Given sits between the header and Prove".into());
                    }
                }
                _ => faults.push("a theorem without its header, Prove or Proof".into()),
            }
        }
        all.push((example.vfs_path, faults));
    }
    report("Logic", all);
}

#[test]
fn math_examples_are_in_the_math_format() {
    let mut all = Vec::new();
    for example in ALL_MATH_EXAMPLES {
        let mut faults = common_faults(example, "--");
        // A development (`## Theory …`) is read by the LOGOS parser, which has no `--` comments:
        // its heading names it.
        if example.source.starts_with("## Theory ") {
            faults.retain(|f| !f.starts_with("does not open with"));
        }
        for (at, indent, line) in indents(example.source) {
            if indent % 4 != 0 {
                faults.push(format!("line {at} is indented {indent} spaces, not a multiple of 4"));
            }
            let rest = line.trim_start().strip_prefix("--").map(str::trim);
            if rest.is_some_and(|r| r.len() >= 3 && r.chars().all(|c| matches!(c, '=' | '-' | '*' | '#'))) {
                faults.push(format!("line {at} is a banner rule; a section is named by a plain `-- Title`"));
            }
        }
        let mut prose = example.source.lines().filter(|l| l.trim_start().starts_with("--"));
        if prose.any(|l| l.contains("GÖDEL") || l.contains("Godel")) {
            faults.push("spells Gödel another way in its prose".into());
        }
        if LAYOUT_PINNED_ELSEWHERE.contains(&example.vfs_path) {
            faults.retain(|f| !pinned_layout_fault(f));
        }
        all.push((example.vfs_path, faults));
    }
    report("Math", all);
}

#[test]
fn hardware_specs_are_in_the_spec_format() {
    let mut all = Vec::new();
    for example in ALL_HARDWARE_SPECS.iter().filter(|s| s.vfs_path.ends_with(".hw")) {
        let mut faults = common_faults(example, "#");
        for (at, indent, _) in indents(example.source) {
            if indent > 0 {
                faults.push(format!("line {at} is indented; a spec is written at the margin"));
            }
        }
        all.push((example.vfs_path, faults));
    }
    report("Hardware spec", all);
}

#[test]
fn verilog_examples_are_in_the_verilog_format() {
    let mut all = Vec::new();
    for example in ALL_HARDWARE_SPECS.iter().filter(|s| s.vfs_path.ends_with(".v")) {
        let mut faults = common_faults(example, "//");
        for (at, indent, line) in indents(example.source) {
            if indent % 2 != 0 {
                faults.push(format!("line {at} is indented {indent} spaces, not a multiple of 2"));
            }
            if !line.is_ascii() {
                faults.push(format!("line {at} has characters outside ASCII"));
            }
            let code = line.split("//").next().unwrap_or_default();
            let statements = code.matches(';').count();
            if statements > 1 && !code.trim_start().starts_with("for ") {
                faults.push(format!("line {at} holds {statements} statements"));
            }
            if let Some(op) = tight_assignment(code) {
                faults.push(format!("line {at} writes `{op}` without a space on each side"));
            }
        }
        all.push((example.vfs_path, faults));
    }
    report("Verilog", all);
}

/// An assignment (`=` or `<=`) in `code` without a space on each side, if there is one.
fn tight_assignment(code: &str) -> Option<&'static str> {
    let bytes = code.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'=' {
            continue;
        }
        let before = i.checked_sub(1).map(|j| bytes[j]);
        let after = bytes.get(i + 1).copied();
        // `==`, `!=`, `>=`, `===`: comparisons, not assignments.
        if matches!(before, Some(b'=' | b'!' | b'>')) || after == Some(b'=') {
            continue;
        }
        let (op, start) = if before == Some(b'<') { ("<=", i - 1) } else { ("=", i) };
        let spaced_before = start.checked_sub(1).is_some_and(|j| bytes[j] == b' ');
        let spaced_after = after == Some(b' ');
        if !(spaced_before && spaced_after) {
            return Some(op);
        }
    }
    None
}

/// The Verilog designs live in `examples.rs` as multi-line raw strings a reader can follow, not
/// one-line literals of `\n` escapes.
#[test]
fn verilog_sources_are_written_as_they_read() {
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../apps/logicaffeine_web/src/ui/examples.rs")).expect("examples.rs");
    let start = source.find("pub const RTL_EXAMPLES").expect("the Verilog registry");
    let end = start + source[start..].find("\n];").expect("its end");
    let escaped: Vec<&str> = source[start..end].lines().filter(|l| l.contains("\\n")).map(str::trim).collect();
    assert!(escaped.is_empty(), "Verilog written as escaped one-line strings:\n{}", escaped.iter().map(|l| &l[..l.len().min(80)]).collect::<Vec<_>>().join("\n"));
}

#[test]
fn every_math_example_left_to_another_lock_is_one_the_studio_ships() {
    for path in LAYOUT_PINNED_ELSEWHERE {
        assert!(
            ALL_MATH_EXAMPLES.iter().any(|e| e.vfs_path == *path),
            "{path} is exempt from the Math format but is no Math example"
        );
    }
    assert!(pinned_layout_fault("line 1 is a banner rule; a section is named by a plain `-- Title`"));
    assert!(pinned_layout_fault("line 181 is 101 columns wide"));
    assert!(!pinned_layout_fault("line 4 has a tab"), "only the pinned layout is left to the other lock");
}
