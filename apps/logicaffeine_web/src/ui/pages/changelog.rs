//! The root `CHANGELOG.md`, read the way the roadmap shows it.
//!
//! The roadmap's release history titles each release with a headline, and its Next release block
//! lists what `[Unreleased]` has added so far. Both read the CHANGELOG through this module. It is
//! std-only: the web crate's `build.rs` includes this file with `#[path]` and bakes `[Unreleased]`
//! into the page at build time, and the crate's tests run the same code.
//!
//! The headline rule is the one `scripts/generate-roadmap.sh` applies when it writes
//! `roadmap_history.json` (its `clean_title`), and `tests/roadmap_page.rs` holds the two readings
//! to the same answer on every release the CHANGELOG records.

/// What `[Unreleased]` holds: every addition's headline, in order, and how many other entries
/// there are.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Unreleased {
    /// The headline of each entry under `### Added`.
    pub added: Vec<String>,
    /// Entries under `### Changed`, `### Deprecated` and `### Removed`.
    pub changed: usize,
    /// Entries under `### Fixed` and `### Security`.
    pub fixed: usize,
}

/// A release's section of the CHANGELOG: the text between its `## [version]` heading line and the
/// next `## ` heading. `None` when the CHANGELOG has no such release.
pub fn section<'a>(changelog: &'a str, version: &str) -> Option<&'a str> {
    let heading = format!("## [{version}]");
    let dated = format!("{heading} ");
    let mut offset = 0;
    let mut start = None;
    for line in changelog.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if text.starts_with("## ") {
            if let Some(start) = start {
                return Some(&changelog[start..offset]);
            }
            if text == heading || text.starts_with(&dated) {
                start = Some(offset + line.len());
            }
        }
        offset += line.len();
    }
    start.map(|start| &changelog[start..])
}

/// A release's title, as the release history shows it: the headline of the section's first line
/// of content, which is its first entry or a prose preamble such as "First public release.".
pub fn release_title(section: &str) -> String {
    section
        .lines()
        .find(|line| !line.trim().is_empty() && !line.starts_with("### "))
        .map(|line| headline(line.strip_prefix("- ").unwrap_or(line)))
        .unwrap_or_default()
}

/// One line's headline. An entry's is its first bold lead; a line with no bold text keeps its
/// words, less one pair of wrapping parentheses. Backticks are dropped, runs of whitespace become
/// one space, a trailing period goes, and a headline over 72 characters is cut back to its last
/// whole word and ends in an ellipsis.
pub fn headline(line: &str) -> String {
    let line = line.strip_prefix("- ").unwrap_or(line);
    let raw = match line.find("**") {
        Some(open) => {
            let lead = &line[open + 2..];
            &lead[..lead.find('*').unwrap_or(lead.len())]
        }
        None => {
            let line = line.strip_prefix('(').unwrap_or(line);
            line.strip_suffix(')').unwrap_or(line)
        }
    };
    let unticked = raw.replace('`', "");
    let joined = unticked.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = joined.strip_suffix('.').unwrap_or(&joined);
    if text.chars().count() <= 72 {
        return text.to_string();
    }
    let cut: String = text.chars().take(72).collect();
    let whole = match cut.rfind(' ') {
        Some(space) => &cut[..space],
        None => &cut[..],
    };
    format!("{whole}…")
}

/// The `[Unreleased]` section, read. A CHANGELOG with no `[Unreleased]` section has nothing in
/// progress.
pub fn unreleased(changelog: &str) -> Unreleased {
    let Some(body) = section(changelog, "Unreleased") else { return Unreleased::default() };
    let mut next = Unreleased::default();
    let mut kind = "";
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            kind = heading.trim();
        } else if line.starts_with("- ") {
            match kind {
                "Added" => next.added.push(headline(line)),
                "Changed" | "Deprecated" | "Removed" => next.changed += 1,
                "Fixed" | "Security" => next.fixed += 1,
                _ => {}
            }
        }
    }
    next
}

#[cfg(test)]
#[path = "../../../../../crates/logicaffeine_tests/unit/logicaffeine_web/ui/pages/changelog/tests.rs"]
mod tests;
