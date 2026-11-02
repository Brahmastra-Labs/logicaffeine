//! Dependency Scanner for the Hyperlink Module System.
//!
//! Scans the "Abstract" (first paragraph) of a LOGOS document for Markdown links,
//! which are interpreted as module dependencies.
//!
//! Syntax: `[Alias](URI)` where:
//! - Alias: The name to reference the module by (e.g., "Geometry")
//! - URI: The location of the module source (e.g., "file:./geo.md", "logos:std")

/// One name selected by a per-name import (`Import double as dbl from Util.`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedName {
    /// The real name exported by the module (`double`).
    pub name: String,
    /// The local alias the importer uses, if `as <alias>` was given (`dbl`).
    pub alias: Option<String>,
}

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
    /// For a SELECTIVE import (`Import double from Util.`), the specific names
    /// brought into scope (each with an optional per-name alias). `None` means a
    /// whole-module import — every public definition is in scope.
    pub selection: Option<Vec<SelectedName>>,
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
    // `in_abstract` is true until the opening paragraph ends; `abstract_started` marks that some
    // prose has been seen, so leading blank lines do not close an abstract that never opened.
    let mut in_abstract = true;
    let mut abstract_started = false;
    let mut current_pos = 0;

    for line in source.lines() {
        let line_start = current_pos;
        let trimmed = line.trim();

        // Track position for the next line
        current_pos += line.len() + 1; // +1 for newline

        // A blank line ends the ABSTRACT — the opening paragraph, which is the only place a
        // literate Markdown LINK counts as a dependency. Scanning past it for links would make
        // every `[see also](…)` in a module's prose a module the loader must resolve.
        if trimmed.is_empty() {
            if abstract_started {
                in_abstract = false;
            }
            continue;
        }

        // Skip the header line (# Title)
        if trimmed.starts_with("# ") && !trimmed.starts_with("## ") {
            continue;
        }

        // ★ THE PROSE REGION ENDS AT THE FIRST BLOCK HEADER, AND NOWHERE EARLIER.
        //
        // This used to stop at the first BLANK LINE after any prose — the file's opening
        // "abstract" — which meant a `.lg` written the way every literate module in this repo is
        // written (title, an explanation of what the module is for, THEN the imports) had its
        // imports silently invisible. Nothing was scanned, so nothing was resolved, so the module
        // was never spliced and its type declarations never reached the registry; the program then
        // compiled on as though the record had never been declared. See `import_scan_lock`.
        //
        // Past a `## ` header the file is CODE, where `Import` is a statement the parser handles
        // and a line beginning with that word is not necessarily one at all — so the scan stops
        // there rather than mining the program's text for dependencies.
        if trimmed.starts_with("## ") {
            break;
        }

        abstract_started = true;

        // An INDENTED line in the prose region is a Markdown code block — an illustration of what
        // an import looks like, not an import. Reading the trimmed line would turn every example
        // in every guide into a dependency the loader then fails to resolve.
        let indented = line.starts_with("    ") || line.starts_with('\t');

        // ★ AN EXPLICIT `Import …` COUNTS ANYWHERE IN THE PROSE; A LINK ONLY IN THE ABSTRACT.
        //
        // The two are not the same kind of thing, and treating them alike was the bug in both
        // directions. An `Import Shared.` is a DECLARATION — it says the module depends on
        // Shared, and it means that wherever the author put it. A `[see also](other.md)` is
        // PROSE: a module's explanation may link to things it does not depend on at all, which
        // is why the abstract-only rule exists for links and is worth keeping.
        //
        // Reading only the abstract for BOTH meant an `Import` after the opening paragraph — how
        // every literate `.lg` in this repo is written — was silently dropped: not scanned, so
        // not resolved, so not spliced, and the program compiled on as though the module had
        // never been named. See `import_scan_lock`.
        if !indented {
            let imports = parse_import_statement(trimmed, line_start);
            if !imports.is_empty() {
                dependencies.extend(imports);
                continue;
            }
        }

        if in_abstract {
            scan_line_for_links(line, line_start, &mut dependencies);
        }
    }

    dependencies
}

/// A file's first `## ` section header outside a ``` fence, as its 1-based line and its text: the line
/// above which an `Import` must stand to import anything.
pub fn first_section_header(source: &str) -> Option<(usize, &str)> {
    let mut in_fence = false;
    for (i, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
        } else if !in_fence && trimmed.starts_with("## ") {
            return Some((i + 1, trimmed));
        }
    }
    None
}

/// Every `Import` sentence below a file's first `## ` header, each with its byte span. That is where
/// [`scan_dependencies`] does not read, so such a sentence imports nothing: the module is never
/// spliced and the program runs on without it. A caller refuses each at its own line. A line inside a
/// ``` fence or indented as a code block illustrates an import and is never one.
pub fn misplaced_imports(source: &str) -> Vec<Dependency> {
    let mut found = Vec::new();
    let mut past_header = false;
    let mut in_fence = false;
    let mut line_start = 0;
    for line in source.lines() {
        let start = line_start;
        line_start += line.len() + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if trimmed.starts_with("## ") {
            past_header = true;
            continue;
        }
        let indented = line.starts_with("    ") || line.starts_with('\t');
        if past_header && !indented {
            let sentence_start = start + (line.len() - line.trim_start().len());
            found.extend(parse_import_statement(trimmed, sentence_start));
        }
    }
    found
}

/// Parse an explicit import statement into one or more dependencies:
///
/// * `Import Geometry.` — the bare name resolves to a project file
///   (`geometry.lg` / `geometry.md`, tried by the loader), bound under the
///   alias `Geometry`.
/// * `Import Geometry and Colors.` / `Import Lexer, Parser, Ast.` — several
///   modules in one natural sentence.
/// * `Import Geometry from "shapes/geometry.lg".` — an explicit path (for a
///   subdirectory, or when the file name differs from the alias).
/// * `Import Geometry as Geo.` — bind the module under a different alias
///   (`Geo::…`), e.g. to disambiguate two modules that would share a name.
///
/// The trailing period is optional. Returns an empty vec for a line that is not
/// an import statement (ordinary abstract prose flows on to link scanning).
fn parse_import_statement(line: &str, line_start: usize) -> Vec<Dependency> {
    let Some(rest) = line
        .strip_prefix("Import ")
        .or_else(|| line.strip_prefix("import "))
    else {
        return Vec::new();
    };
    let rest = rest.trim();
    let rest = rest.strip_suffix('.').unwrap_or(rest).trim();
    if rest.is_empty() {
        return Vec::new();
    }
    let end = line_start + line.len();
    // A SELECTIVE import — `<names> from <Bareword>` (unquoted source module) —
    // is decided at the STATEMENT level, before clause-splitting, so the `,`/`and`
    // list is read as NAMES, not modules. `from "quoted"` stays a path import.
    if let Some(dep) = parse_selective_import(rest, line_start, end) {
        return vec![dep];
    }
    split_import_clauses(rest)
        .into_iter()
        .filter_map(|clause| parse_one_import(clause, line_start, end))
        .collect()
}

/// Parse `<names> from <Bareword>` into a single selective dependency, or `None`
/// if this isn't the selective shape (no `from`, or a quoted/multi-word source —
/// those are whole-module path imports handled by [`parse_one_import`]).
fn parse_selective_import(rest: &str, start: usize, end: usize) -> Option<Dependency> {
    let idx = rest.rfind(" from ")?;
    let names_part = rest[..idx].trim();
    let source = rest[idx + " from ".len()..].trim();
    // A quoted or multi-word source is a PATH (whole-module import), not selective.
    if source.is_empty() || source.starts_with('"') || source.contains(char::is_whitespace) {
        return None;
    }
    let selection: Vec<SelectedName> = split_import_clauses(names_part)
        .into_iter()
        .filter_map(parse_selected_name)
        .collect();
    if selection.is_empty() {
        return None;
    }
    Some(Dependency {
        alias: source.to_string(),
        uri: format!("file:./{}", source.to_lowercase()),
        start,
        end,
        selection: Some(selection),
    })
}

/// One entry of a selective import's name list: `<name> [as <alias>]`.
fn parse_selected_name(clause: &str) -> Option<SelectedName> {
    let clause = clause.trim();
    let (name, alias) = match clause.rsplit_once(" as ") {
        Some((n, a)) => (n.trim(), Some(a.trim())),
        None => (clause, None),
    };
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let alias = alias
        .filter(|a| !a.is_empty() && !a.contains(char::is_whitespace))
        .map(str::to_string);
    Some(SelectedName {
        name: name.to_string(),
        alias,
    })
}

/// One import clause: `<name> [from "<path>"] [as <alias>]`.
fn parse_one_import(clause: &str, start: usize, end: usize) -> Option<Dependency> {
    // `as <alias>` renames the binding; the module is still resolved from <name>.
    let (head, bind) = match clause.rsplit_once(" as ") {
        Some((h, b)) => (h.trim(), Some(b.trim())),
        None => (clause, None),
    };
    // `from "<path>"` gives an explicit path; otherwise resolve by name.
    let (name, path) = match head.split_once(" from ") {
        Some((n, p)) => (n.trim(), Some(p.trim().trim_matches('"').trim())),
        None => (head, None),
    };
    if name.is_empty() || name.contains(char::is_whitespace) {
        return None;
    }
    let alias = bind
        .filter(|b| !b.is_empty() && !b.contains(char::is_whitespace))
        .unwrap_or(name);
    let uri = match path {
        Some(p) if p.is_empty() => return None,
        Some(p) if p.starts_with("./") || p.starts_with("../") => format!("file:{p}"),
        Some(p) => format!("file:./{p}"),
        None => format!("file:./{}", name.to_lowercase()),
    };
    Some(Dependency {
        alias: alias.to_string(),
        uri,
        start,
        end,
        selection: None,
    })
}

/// Split an import list on `,` and ` and ` (list + English style) into clauses —
/// but only OUTSIDE double quotes, so a `from "a and b.lg"` path with an
/// embedded separator stays one clause. Separators are ASCII (`"`, `,`, ` `), so
/// every slice boundary is a valid char boundary.
fn split_import_clauses(s: &str) -> Vec<&str> {
    const AND: &str = " and ";
    let bytes = s.as_bytes();
    let mut clauses = Vec::new();
    let mut in_quotes = false;
    let mut clause_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                in_quotes = !in_quotes;
                i += 1;
            }
            b',' if !in_quotes => {
                clauses.push(&s[clause_start..i]);
                i += 1;
                clause_start = i;
            }
            b' ' if !in_quotes && s[i..].starts_with(AND) => {
                clauses.push(&s[clause_start..i]);
                i += AND.len();
                clause_start = i;
            }
            _ => i += 1,
        }
    }
    clauses.push(&s[clause_start..]);
    clauses
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect()
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
                selection: None,
            });
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_language/analysis/dependencies/tests.rs"]
mod tests;
