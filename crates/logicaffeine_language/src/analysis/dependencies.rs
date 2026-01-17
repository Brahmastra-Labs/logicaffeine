//! Dependency Scanner for the Hyperlink Module System.
//!
//! Scans the "Abstract" (first paragraph) of a LOGOS document for Markdown links,
//! which are interpreted as module dependencies.
//!
//! Syntax: `[Alias](URI)` where:
//! - Alias: The name to reference the module by (e.g., "Geometry")
//! - URI: The location of the module source (e.g., "file:./geo.md", "logos:std")

/// A dependency declaration found in the document's abstract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    /// The alias to use when referencing this module (e.g., "Geometry")
    pub alias: String,
    /// The URI pointing to the module source (e.g., "file:./geo.md")
    pub uri: String,
    /// Start position in the source for error reporting
    pub start: usize,
    /// End position in the source for error reporting
    pub end: usize,
}

/// Scans the first paragraph (Abstract) of a LOGOS file for `[Alias](URI)` links.
///
/// The Abstract is defined as the first non-empty block of text following the
/// module header (# Name). Links inside this paragraph are treated as imports.
/// Scanning stops at the first empty line after the abstract or when a code
/// block header (`##`) is encountered.
///
/// # Example
///
/// ```text
/// # My Game
///
/// This module uses [Geometry](file:./geo.md) for math.
///
/// ## Main
/// Let x be 1.
/// ```
///
/// Returns: `[Dependency { alias: "Geometry", uri: "file:./geo.md", ... }]`
pub fn scan_dependencies(source: &str) -> Vec<Dependency> {
    let mut dependencies = Vec::new();
    let mut in_abstract = false;
    let mut abstract_started = false;
    let mut current_pos = 0;

    for line in source.lines() {
        let line_start = current_pos;
        let trimmed = line.trim();

        // Track position for the next line
        current_pos += line.len() + 1; // +1 for newline

        // Skip completely empty lines before the abstract starts
        if trimmed.is_empty() {
            if abstract_started && in_abstract {
                // Empty line after abstract content - we're done
                break;
            }
            continue;
        }

        // Skip the header line (# Title)
        if trimmed.starts_with("# ") && !trimmed.starts_with("## ") {
            continue;
        }

        // Stop at code block headers (## Main, ## Definition, etc.)
        if trimmed.starts_with("## ") {
            break;
        }

        // We found non-empty, non-header content - this is the abstract
        in_abstract = true;
        abstract_started = true;

        // Scan this line for Markdown links [Alias](URI)
        scan_line_for_links(line, line_start, &mut dependencies);
    }

    dependencies
}

/// Scans a single line for Markdown link patterns `[Alias](URI)`.
fn scan_line_for_links(line: &str, line_start: usize, deps: &mut Vec<Dependency>) {
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Look for opening bracket
        if bytes[i] == b'[' {
            let link_start = line_start + i;
            i += 1;

            // Read the alias (text between [ and ])
            let alias_start = i;
            while i < bytes.len() && bytes[i] != b']' {
                i += 1;
            }

            if i >= bytes.len() {
                // No closing bracket found
                break;
            }

            let alias = &line[alias_start..i];
            i += 1; // Skip ]

            // Expect immediate opening parenthesis
            if i >= bytes.len() || bytes[i] != b'(' {
                continue;
            }
            i += 1; // Skip (

            // Read the URI (text between ( and ))
            let uri_start = i;
            let mut paren_depth = 1;
            while i < bytes.len() && paren_depth > 0 {
                if bytes[i] == b'(' {
                    paren_depth += 1;
                } else if bytes[i] == b')' {
                    paren_depth -= 1;
                }
                if paren_depth > 0 {
                    i += 1;
                }
            }

            if paren_depth != 0 {
                // No closing parenthesis found
                break;
            }

            let uri = &line[uri_start..i];
            let link_end = line_start + i + 1;
            i += 1; // Skip )

            // Skip empty aliases or URIs
            if alias.is_empty() || uri.is_empty() {
                continue;
            }

            deps.push(Dependency {
                alias: alias.trim().to_string(),
                uri: uri.trim().to_string(),
                start: link_start,
                end: link_end,
            });
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_dependency_scanning() {
        let source = r#"
# My Game

This uses [Geometry](file:./geo.md) and [Physics](logos:std).

## Main
Let x be 1.
"#;
        let deps = scan_dependencies(source);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].alias, "Geometry");
        assert_eq!(deps[0].uri, "file:./geo.md");
        assert_eq!(deps[1].alias, "Physics");
        assert_eq!(deps[1].uri, "logos:std");
    }

    #[test]
    fn ignores_links_after_abstract() {
        let source = r#"
# Header

This is the abstract with [Dep1](file:a.md).

This second paragraph has [Dep2](file:b.md).

## Main
Let x be 1.
"#;
        let deps = scan_dependencies(source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].alias, "Dep1");
    }

    #[test]
    fn no_dependencies_without_abstract() {
        let source = r#"
# Module

## Main
Let x be 1.
"#;
        let deps = scan_dependencies(source);
        assert_eq!(deps.len(), 0);
    }

    #[test]
    fn multiline_abstract() {
        let source = r#"
# My Project

This project uses [Math](file:./math.md) for calculations
and [IO](file:./io.md) for input/output operations.

## Main
Let x be 1.
"#;
        let deps = scan_dependencies(source);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].alias, "Math");
        assert_eq!(deps[1].alias, "IO");
    }

    #[test]
    fn handles_spaces_in_alias() {
        let source = r#"
# App

Uses the [Standard Library](logos:std).

## Main
"#;
        let deps = scan_dependencies(source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].alias, "Standard Library");
        assert_eq!(deps[0].uri, "logos:std");
    }

    #[test]
    fn handles_https_urls() {
        let source = r#"
# App

Uses [Physics](https://logicaffeine.dev/pkg/physics).

## Main
"#;
        let deps = scan_dependencies(source);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].alias, "Physics");
        assert_eq!(deps[0].uri, "https://logicaffeine.dev/pkg/physics");
    }

    #[test]
    fn handles_multiple_links_on_one_line() {
        let source = r#"
# App

Uses [A](file:a.md), [B](file:b.md), and [C](file:c.md).

## Main
"#;
        let deps = scan_dependencies(source);
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].alias, "A");
        assert_eq!(deps[1].alias, "B");
        assert_eq!(deps[2].alias, "C");
    }
}
