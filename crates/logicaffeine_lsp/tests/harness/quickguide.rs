//! The shared LOGOS_QUICKGUIDE.md table reader for ratchet tests.
//!
//! One parser, two views: `construct_rows` yields every data row with its
//! section, construct cell, and backticked surface forms; the highlighting
//! ratchet flattens that to `quickguide_surface_forms`, and the teach parity
//! ratchet keys on the rows themselves. Same column rules as the VSCode
//! extension's grammar ratchet.
#![allow(dead_code)]

/// One quickguide table row: where it lives, what it names, how it's written.
pub struct GuideRow {
    /// The nearest preceding markdown heading.
    pub section: String,
    /// The row's first cell — the construct/meaning column.
    pub construct: String,
    /// Backticked surface forms from the recognized surface columns.
    pub forms: Vec<String>,
}

/// Extract every data row from the quickguide's tables, keeping the
/// backticked surface forms of the canonical columns (`(proposed)` rows and
/// `…`-elided forms skipped, `·`-separated alternatives split).
pub fn construct_rows(markdown: &str) -> Vec<GuideRow> {
    const SURFACE_COLUMNS: &[&str] =
        &["canonical", "also works", "form", "symbolic", "english", "examples"];
    let lines: Vec<&str> = markdown.lines().collect();
    let mut surface_columns: Vec<usize> = Vec::new();
    let mut section = String::new();
    let mut rows = Vec::new();

    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if let Some(heading) = line.strip_prefix('#') {
            section = heading.trim_start_matches('#').trim().to_string();
        }
        if !line.starts_with('|') {
            surface_columns.clear();
            continue;
        }
        let cells: Vec<&str> = {
            let inner: Vec<&str> = line.split('|').collect();
            inner[1..inner.len().saturating_sub(1)]
                .iter()
                .map(|c| c.trim())
                .collect()
        };
        let next_is_separator = lines
            .get(i + 1)
            .map(|l| {
                let t = l.trim();
                t.starts_with('|') && t.chars().all(|c| "|-: ".contains(c))
            })
            .unwrap_or(false);
        if next_is_separator {
            surface_columns = cells
                .iter()
                .enumerate()
                .filter(|(_, c)| {
                    SURFACE_COLUMNS.contains(&c.replace('*', "").to_lowercase().as_str())
                })
                .map(|(ix, _)| ix)
                .collect();
            continue;
        }
        if line.chars().all(|c| "|-: ".contains(c)) || surface_columns.is_empty() {
            continue;
        }
        let construct = cells
            .first()
            .map(|c| c.replace('*', "").trim_matches('`').trim().to_string())
            .unwrap_or_default();
        let mut forms = Vec::new();
        for &column in &surface_columns {
            let Some(cell) = cells.get(column) else { continue };
            for alternative in cell.split('\u{b7}') {
                if alternative.contains("(proposed)") {
                    continue;
                }
                let mut rest = alternative;
                while let Some(open) = rest.find('`') {
                    let Some(close) = rest[open + 1..].find('`') else { break };
                    let form = rest[open + 1..open + 1 + close].trim();
                    if !form.is_empty() && !form.contains('\u{2026}') {
                        forms.push(form.to_string());
                    }
                    rest = &rest[open + close + 2..];
                }
            }
        }
        rows.push(GuideRow { section: section.clone(), construct, forms });
    }
    rows
}

/// Extract backticked surface forms from the quickguide's canonical
/// columns (same rules as the extension's grammar ratchet).
pub fn quickguide_surface_forms(markdown: &str) -> Vec<String> {
    construct_rows(markdown).into_iter().flat_map(|row| row.forms).collect()
}

/// Every heading of the guide as a GitHub anchor slug — what a
/// `codeDescription`/`guide_anchor` must resolve against.
pub fn heading_slugs(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start_matches('#');
            (trimmed.len() < line.len()).then(|| {
                trimmed
                    .trim()
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-')
                    .collect::<String>()
                    .replace(' ', "-")
            })
        })
        .collect()
}
