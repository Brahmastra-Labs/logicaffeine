//! The standard library's release surface: what an author SEES of the stdlib — module titles and
//! introductions, the prose of every `## Note` — is held to a contract here, module by module.
//!
//! Each guard is a pure function over module text, and each has a self-test that feeds it a
//! known-bad module and requires it to fire, so a guard can never silently stop guarding.

use logicaffeine_compile::loader::prelude_module_sources;

/// The module's name for messages: its `# Title`, or its first line.
fn module_label(src: &str) -> &str {
    src.lines().next().unwrap_or("").trim_start_matches("# ")
}

/// A module must open with `# <Title>` and at least one paragraph of prose before its first `##`
/// section — the text the reference and the editor present as the module's introduction.
fn title_and_intro_violation(src: &str) -> Option<String> {
    let mut lines = src.lines();
    let first = lines.next().unwrap_or("");
    let Some(title) = first.strip_prefix("# ") else {
        return Some(format!(
            "does not open with a `# Title` line (first line: {first:?})"
        ));
    };
    if title.trim().is_empty() {
        return Some("has an empty `# ` title".to_string());
    }
    let intro: String = lines
        .take_while(|l| !l.starts_with("## "))
        .collect::<Vec<_>>()
        .join("\n");
    if intro.trim().is_empty() {
        return Some(format!(
            "`{title}` has no introduction before its first section"
        ));
    }
    None
}

/// The prose blocks of a module: its introduction and every `## Note` body, each named for what
/// it documents (the header that follows it) and split into paragraphs (blank-line separated),
/// fenced code excluded.
fn prose_blocks(src: &str) -> Vec<(String, Vec<String>)> {
    let mut blocks: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<String> = Some(String::new());
    let mut is_intro = true;
    let mut in_fence = false;
    for line in src.lines().skip(1) {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if line.starts_with("## ") {
            if let Some(text) = current.take() {
                let label = if is_intro {
                    "introduction".to_string()
                } else {
                    format!(
                        "note on `{}`",
                        line.trim_start_matches("## ").trim_end_matches(':')
                    )
                };
                blocks.push((label, paragraphs(&text)));
            }
            is_intro = false;
            if line.trim() == "## Note" {
                current = Some(String::new());
            }
            continue;
        }
        if let Some(text) = current.as_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    if let Some(text) = current {
        blocks.push(("trailing note".to_string(), paragraphs(&text)));
    }
    blocks
}

fn paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// A paragraph that begins with a lowercase word continues the sentence before it: a blank line
/// was typed mid-sentence, and every renderer (hover, completion, the reference) shows two broken
/// paragraphs. A paragraph opening with code (`` ` ``), a list marker or a digit is not prose.
fn broken_paragraph_violation(src: &str) -> Option<String> {
    let mut found = Vec::new();
    for (block, paras) in prose_blocks(src) {
        for pair in paras.windows(2) {
            let next = &pair[1];
            if next.starts_with(|c: char| c.is_ascii_lowercase()) {
                let tail: String = pair[0]
                    .chars()
                    .rev()
                    .take(40)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let head: String = next.chars().take(40).collect();
                found.push(format!("  {block}: …{tail:?} / {head:?}…"));
            }
        }
    }
    (!found.is_empty()).then(|| format!("a blank line splits a sentence:\n{}", found.join("\n")))
}

fn sweep(guard: fn(&str) -> Option<String>) -> Vec<String> {
    prelude_module_sources()
        .iter()
        .filter_map(|src| guard(src).map(|why| format!("{}: {why}", module_label(src))))
        .collect()
}

// ─── Guards over the shipped stdlib ──────────────────────────────────────────

#[test]
fn every_module_opens_with_a_title_and_an_introduction() {
    let bad = sweep(title_and_intro_violation);
    assert!(
        bad.is_empty(),
        "stdlib modules without a title and introduction:\n{}",
        bad.join("\n")
    );
}

#[test]
fn no_module_prose_breaks_a_sentence_across_paragraphs() {
    let bad = sweep(broken_paragraph_violation);
    assert!(
        bad.is_empty(),
        "stdlib prose with a sentence split by a blank line:\n{}",
        bad.join("\n")
    );
}

// ─── Every guard fires on the defect it exists for ───────────────────────────

#[test]
fn title_guard_fires_on_a_module_that_opens_with_a_note() {
    let src = "## Note\nA controller.\n\n## To adaptRate (n: Int) -> Int:\n    Return n.\n";
    assert!(title_and_intro_violation(src).is_some());
}

#[test]
fn title_guard_fires_on_a_title_without_an_introduction() {
    let src = "# Forms\n\n## Note\nValidating input.\n\n## To isBlank (t: Text) -> Bool:\n    Return true.\n";
    assert!(title_and_intro_violation(src).is_some());
}

#[test]
fn title_guard_accepts_a_titled_introduced_module() {
    let src =
        "# Forms\n\nValidating input.\n\n## To isBlank (t: Text) -> Bool:\n    Return true.\n";
    assert_eq!(title_and_intro_violation(src), None);
}

#[test]
fn paragraph_guard_fires_on_a_sentence_split_by_a_blank_line() {
    let src = "# JSON\n\nIntro.\n\n## Note\nThe trapping parser: STOPS THE PROGRAM (assertion failure)\n\non an invalid one.\n\n## To jsonParse (t: Text) -> Int:\n    Return 1.\n";
    assert!(broken_paragraph_violation(src).is_some());
    let intro = "# Compress\n\n`compress` — DEFLATE (RFC 1951)\n\nplus the CRC-32 checksum.\n\n## To crc32 (d: Int) -> Int:\n    Return d.\n";
    assert!(broken_paragraph_violation(intro).is_some());
}

#[test]
fn paragraph_guard_accepts_code_and_capitalized_paragraphs() {
    let src = "# Strings\n\nIntro.\n\n## Note\nSplits text.\n\n`split(s, sep)` returns pieces.\n\n```\nlower case inside a fence\n```\n\nSecond sentence.\n\n## To f (n: Int) -> Int:\n    Return n.\n";
    assert_eq!(broken_paragraph_violation(src), None);
}
