//! The standard library's text, read as the prelude reads it: pure functions over a module's source, with no
//! dependency beyond `std`.
//!
//! The crate's build script compiles this file too (`#[path]`) and runs [`index`] once over the embedded modules,
//! writing what every process would otherwise derive at startup — each module's names and natives, which modules
//! its code pulls in, the bytes its splice keeps and where each library function's section lies — as statics the
//! loader reads ([`crate::loader`]). A process that splices nothing reads none of the library's text.

/// Every stdlib module that auto-imports, by file stem under `assets/std/`, in splice order. The build script
/// embeds each as `STD_<STEM>` and lists them, in this order, as the loader's `PRELUDE_MODULES`. The trigger
/// identifiers and collision keys are not hand-maintained — they are derived from each module's own definitions
/// ([`defined_names`]), so adding a module here makes its whole vocabulary live with nothing else to update.
/// `std.lg` is absent: its `Unit`, `Pair` and `Triple` stay an explicit import (`logos:std`).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const PRELUDE_FILES: [&str; 46] = [
    // Core algebraic types (Result/Option) come first: later modules'
    // signatures (file.lg's `-> Result of Text and Text`) refer to them.
    "core",
    "concurrency",
    "net",
    "io",
    "crdt",
    "env",
    "file",
    "tcp",
    "web",
    "session",
    "postgres",
    "random",
    "time",
    "http",
    "static",
    "crypto",
    "uuid",
    "media",
    "math",
    "strings",
    // After `strings`: `forms.lg` validates through its `trim`.
    "forms",
    "ui",
    "iter",
    "convert",
    "error",
    "rng",
    "regex",
    "json",
    "encoding",
    "compress",
    "bits",
    "unicode",
    "operator",
    "bytes",
    "chars",
    "statistics",
    "builder",
    "collections",
    "heap",
    "sort",
    "calendar",
    "blas",
    "lapack",
    "clapack",
    "vision",
    "vdom",
];

/// What the prelude needs of its modules, derived from their text alone ([`index`]).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct StdlibIndex {
    /// Per module, the names it defines ([`module_names`]).
    pub names: Vec<Vec<String>>,
    /// Per module, the names its spliced code declares `## To native` ([`module_native_names`]).
    pub natives: Vec<Vec<String>>,
    /// Per module, every other module with a name that is not builtin-backed and that its spliced code uses
    /// ([`references`]): splicing the module splices these too.
    pub pulls: Vec<Vec<usize>>,
    /// Per module, whether its spliced code mentions `text_from_bytes`.
    pub mentions_text_from_bytes: Vec<bool>,
    /// Per module, the byte ranges of its source whose concatenation is its spliced code ([`spliced_code`]).
    pub splice: Vec<Vec<(usize, usize)>>,
    /// Every name a module's code opens a `## To` header line with, at any indentation, sorted: the sections a
    /// pruner may drop.
    pub functions: Vec<String>,
    /// Every `## To` section of the spliced modules as `(name, module, start, end)`, the section being
    /// `module[start..end]` of the module's source; sorted by name, and a name two modules define keeps the later
    /// module's section.
    pub library: Vec<(String, usize, usize, usize)>,
}

/// A module's code as the prelude splices it: its `## Note` prose and its builtin-backed declarations stripped.
pub(crate) fn spliced_code(source: &str) -> String {
    strip_builtin_backed_decls(&strip_note_blocks(module_code(source)))
}

/// Derive the [`StdlibIndex`] of `modules`, given in splice order. Panics when a module's spliced code is not a
/// sequence of its source's lines, or a library section is not one run of its source: either would leave the
/// index unable to name the bytes, and the build must stop rather than embed an index that disagrees with the text.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn index(modules: &[&str]) -> StdlibIndex {
    let spliced: Vec<String> = modules.iter().map(|m| spliced_code(m)).collect();
    let names: Vec<Vec<String>> = modules.iter().map(|m| module_names(m)).collect();
    let natives = spliced
        .iter()
        .map(|code| module_native_names(code))
        .collect();
    let pulls = spliced
        .iter()
        .enumerate()
        .map(|(j, code)| {
            let words = whole_words(code);
            (0..modules.len())
                .filter(|&k| {
                    k != j
                        && names[k].iter().any(|n| {
                            !builtin_backed(n) && may_name(&words, n) && references(code, n)
                        })
                })
                .collect()
        })
        .collect();
    let mentions_text_from_bytes = spliced
        .iter()
        .map(|code| code.contains("text_from_bytes"))
        .collect();
    let splice: Vec<Vec<(usize, usize)>> = modules
        .iter()
        .zip(&spliced)
        .enumerate()
        .map(|(m, (source, code))| {
            let runs = line_runs(source, code).unwrap_or_else(|| {
                panic!("module {m}: its spliced code is not a sequence of its source's lines")
            });
            assert_eq!(
                runs.iter().map(|&(s, e)| &source[s..e]).collect::<String>(),
                *code,
                "module {m}: the splice's runs do not rebuild its spliced code"
            );
            runs
        })
        .collect();
    let mut by_name: std::collections::BTreeMap<String, (usize, usize, usize)> =
        std::collections::BTreeMap::new();
    for (m, code) in spliced.iter().enumerate() {
        for (name, range) in function_section_ranges(code) {
            let (start, end) = source_range(&splice[m], range.clone()).unwrap_or_else(|| {
                panic!("module {m}: `## To {name}` spans bytes its splice strips, so no one run of the source holds it")
            });
            assert_eq!(
                &modules[m][start..end],
                &code[range],
                "module {m}: `## To {name}` maps to other bytes"
            );
            by_name.insert(name, (m, start, end));
        }
    }
    let library = by_name
        .into_iter()
        .map(|(name, (m, s, e))| (name, m, s, e))
        .collect();
    let functions = modules
        .iter()
        .flat_map(|source| {
            module_code(source)
                .lines()
                .filter_map(|l| fn_section_name(&format!("{l}\n")))
        })
        .collect::<std::collections::BTreeSet<String>>()
        .into_iter()
        .collect();
    StdlibIndex {
        names,
        natives,
        pulls,
        mentions_text_from_bytes,
        splice,
        functions,
        library,
    }
}

/// The byte runs of `source` whose concatenation is `code`, when `code` is a sequence of `source`'s lines (the last
/// one possibly without its newline), taken earliest first and adjacent lines merged into one run.
fn line_runs(source: &str, code: &str) -> Option<Vec<(usize, usize)>> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut wanted = code.split_inclusive('\n').peekable();
    let mut at = 0;
    for line in source.split_inclusive('\n') {
        let start = at;
        at += line.len();
        let Some(&want) = wanted.peek() else { break };
        let taken = if line == want {
            line.len()
        } else if wanted.clone().nth(1).is_none() && line.strip_suffix('\n') == Some(want) {
            want.len()
        } else {
            continue;
        };
        wanted.next();
        match runs.last_mut() {
            Some(run) if run.1 == start => run.1 = start + taken,
            _ => runs.push((start, start + taken)),
        }
    }
    wanted.peek().is_none().then_some(runs)
}

/// Where `range` of the code `runs` rebuild lies in the source, when it falls inside one run.
fn source_range(runs: &[(usize, usize)], range: std::ops::Range<usize>) -> Option<(usize, usize)> {
    let mut code_at = 0;
    for &(s, e) in runs {
        let len = e - s;
        if range.start >= code_at && range.end <= code_at + len {
            return Some((s + range.start - code_at, s + range.end - code_at));
        }
        code_at += len;
    }
    None
}

/// Every maximal run of identifier bytes in `text`: the only places [`references`] can find a name made of
/// identifier bytes.
pub(crate) fn whole_words(text: &str) -> std::collections::HashSet<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .collect()
}

/// Whether a whole-word scan of a text whose [`whole_words`] are `words` can find `name`. A name of identifier
/// bytes occurs as a whole word only as one of the text's words; any other name is left to the scan.
pub(crate) fn may_name(words: &std::collections::HashSet<&str>, name: &str) -> bool {
    !name.bytes().all(is_ident_byte) || words.contains(name)
}

/// The CODE of a module — from its first `##` section onward, dropping the
/// markdown title + leading prose. Literate Logos only skips prose *before* the
/// first section, so when modules are concatenated only the leading prose is a
/// hazard; stripping it makes the join parse cleanly. Documentation prose stays
/// in the source files.
pub(crate) fn module_code(md: &str) -> &str {
    if let Some(i) = md.find("\n## ") {
        &md[i + 1..]
    } else if md.starts_with("## ") {
        md
    } else {
        ""
    }
}

/// A module's code with `## Note` documentation blocks removed — what the
/// prelude actually prepends. Notes are the IDE's per-definition doc carrier
/// (see `prelude_module_sources`); the runtime prelude stays lean, note-free,
/// and byte-identical to the pre-documentation join.
///
/// A note is EVERYTHING from its `## Note` line to the next `##` header or the next
/// bare type declaration (`A Request has:` inside a `## Definition` body, core.lg's
/// `A Result of [T] and [E] is either:`) — several paragraphs when the author wrote a
/// one-line summary followed by details (the IDE leads hover with the summary). A
/// blank line never ends a note: it separates paragraphs OF the note. Locked by
/// `note_stripping_removes_every_paragraph_of_a_note` — a stripper that stopped at
/// the first blank line leaked every later paragraph into the prelude as prose.
pub(crate) fn strip_note_blocks(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut in_note = false;
    for line in code.split_inclusive('\n') {
        let trimmed = line.trim();
        if in_note {
            let header = trimmed.starts_with("## ") && trimmed != "## Note";
            if header || is_bare_type_declaration(line) {
                in_note = false;
                out.push_str(line);
            }
            continue;
        }
        if trimmed == "## Note" {
            in_note = true;
            continue;
        }
        out.push_str(line);
    }
    out
}

/// A top-level bare type declaration — the one non-`##` line kind that can follow a note:
/// `A Name has:`, `A Name of [T] and [E] is either:`, `A Name is Shared and has:`,
/// `A Name is a record.`, `A Name is a generic collection.` (column 0, `A`/`An`, a
/// Capitalized name, optional `of [..]`/`and [..]` binders, then the declaring verb).
/// Prose that merely opens with an article and a capitalized word (`A StringBuilder
/// accumulates …`, `An HTTP response …: …`) is not one.
fn is_bare_type_declaration(line: &str) -> bool {
    if line.starts_with(' ') || line.starts_with('\t') {
        return false;
    }
    let line = line.trim_end();
    let Some(rest) = line.strip_prefix("An ").or_else(|| line.strip_prefix("A ")) else {
        return false;
    };
    let mut words = rest.split_whitespace();
    let Some(name) = words.next() else {
        return false;
    };
    if !name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return false;
    }
    let tail: Vec<&str> = words.collect();
    // Skip the generic binders: `of [T]`, `and [E]`, `of [K] and [V]`.
    let mut i = 0;
    while i + 1 < tail.len() && matches!(tail[i], "of" | "and") && tail[i + 1].starts_with('[') {
        i += 2;
    }
    let verb = tail[i..].join(" ");
    verb == "has:"
        || verb == "is either:"
        || verb == "is Shared and has:"
        || (verb.starts_with("is a ") && verb.ends_with('.'))
}

/// The article-led declaration a trimmed line opens — `A Name …` / `An Name …`, in a block or on its
/// own section header (`## A Response has:`) — as the text after the article and whether the line was
/// a `##` header. A section header only ever declares a type, never a variant.
fn article_declaration(t: &str) -> Option<(&str, bool)> {
    let (declaration, on_header) = match t.strip_prefix("## ") {
        Some(rest) => (rest, true),
        None => (t, false),
    };
    let rest = declaration
        .strip_prefix("A ")
        .or_else(|| declaration.strip_prefix("An "))?;
    Some((rest, on_header))
}

/// Builtin-backed names can never DEMAND an import — the engine resolves
/// `List`/`Map` without the prelude, and `Option` is constructed by the
/// `some x`/`none` LITERALS (codegen lowers them to `std::option::Option`;
/// a prepended enum declaration would emit a clashing `user_types::Option`).
/// Letting any of them trigger would also change collection-using programs'
/// expanded bytes for no semantic gain. (They still count on the collision
/// side: a user-declared `Map` type suppresses the module, declarer-wins as
/// everywhere else.)
pub(crate) fn builtin_backed(name: &str) -> bool {
    matches!(name, "List" | "Map" | "Option" | "Some" | "None")
}

/// Drop a module's declaration lines for builtin-backed types before
/// prepending. `A Map of [K] and [V] is a generic collection.` is
/// documentation of a builtin — REGISTERING it would shadow the builtin's own
/// `Map of K to V` syntax with the user-generic `of … and …` form and break
/// every `Map of Text to Int` annotation downstream. The engine resolves these
/// types natively; the prelude must stay silent about them.
pub(crate) fn strip_builtin_backed_decls(code: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut in_stripped_enum = false;
    for line in code.lines() {
        let t = line.trim();
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if in_stripped_enum {
            // A stripped enum header owns its indented variant block.
            if indented || t.is_empty() {
                continue;
            }
            in_stripped_enum = false;
        }
        if let Some((rest, _)) = article_declaration(t) {
            if let Some(word) = rest
                .split(|c: char| c == '(' || c == '.' || c.is_whitespace())
                .next()
            {
                if !indented && builtin_backed(word) {
                    in_stripped_enum = t.ends_with("is either:");
                    continue;
                }
            }
        }
        out.push(line);
    }
    out.join("\n")
}

/// The names a module's CODE defines, used both as auto-import triggers and collision keys
/// ("declarer wins"): helper / native procedures (`## To [native] <name>`) and the names of
/// *type definitions* (`A <Name> has …` / `A <Name> is …` / `A <Name> of [T] …`).
///
/// Enum variant constructors (`A Debug.`, `A Some (value: T).`, `A Failure with msg Text.`)
/// are names too: constructing `a new Ok with value …` or matching `When Ok (v):` must pull in
/// the module that declares the enum, and a program's own variant of the same name must be
/// recognized as the program's. A bare mention in prose never triggers — [`references`] only
/// counts use positions. Field lines (`a sender, which is Int.`) are lowercase and skip.
pub(crate) fn defined_names(code: &str) -> Vec<String> {
    defined_names_with_visibility(code)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

/// Like `defined_names`, but also reports whether each definition is `private`
/// (declared `## To private …` / `## A private …`). Public-by-default: a name with
/// no `private` marker is `false`. Powers module encapsulation — a private name is
/// usable by its owning module but not importable by any other file. Public so the
/// language server can filter private names out of its cross-module surfaces
/// (completion, auto-import, workspace symbols) from the one source of truth.
pub fn defined_names_with_visibility(code: &str) -> Vec<(String, bool)> {
    let mut names = Vec::new();
    // Inside an `is either:` block every indented line is a variant, and a variant's article is
    // optional (`Failure (message: Text).` declares `Failure` exactly as `A Failure (…).` does).
    let mut in_either_block = false;
    for line in code.lines() {
        let t = line.trim();
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !indented && !t.is_empty() {
            in_either_block = false;
        }
        if in_either_block && indented && !t.starts_with("A ") && !t.starts_with("An ") {
            if let Some(word) = t
                .split(|c: char| c == '(' || c == '.' || c.is_whitespace())
                .next()
            {
                let tail = t[word.len()..].trim_start();
                let is_variant = tail.starts_with('(') || tail == "." || tail.starts_with("with");
                if is_variant && word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    names.push((word.to_string(), false));
                }
            }
            continue;
        }
        if !indented && t.ends_with("is either:") {
            in_either_block = true;
        }
        if t.starts_with("## To ") {
            if let Some(header) = function_header(t) {
                names.push((header.name.to_string(), header.private));
            }
            continue;
        }
        if let Some((rest, on_header)) = article_declaration(t) {
            let (rest, private) = match rest.strip_prefix("private ") {
                Some(r) => (r, true),
                None => (rest, false),
            };
            if let Some(word) = rest
                .split(|c: char| c == '(' || c == '.' || c.is_whitespace())
                .next()
            {
                if word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                    // A type *header* continues with `has` / `is` / `of`. A bare
                    // `A Name.` / `A Name (fields).` is a variant constructor —
                    // ALSO a name: constructing `a new Ok with value …` or
                    // matching `When Ok (v):` must pull in the module that
                    // declares the enum (core.lg's Result/Option), and the same
                    // rule on the user side keeps "declarer wins" symmetric — a
                    // user enum with an `Ok` variant suppresses the prelude's.
                    let tail = rest[word.len()..].trim_start();
                    let is_type_header =
                        tail.starts_with("has") || tail.starts_with("is") || tail.starts_with("of");
                    // Variants in BOTH spellings — concise `A Ok (value: T).` /
                    // bare `A None.` AND natural `A Failure with msg Text.` —
                    // otherwise declarer-wins is asymmetric: a user's natural-
                    // form `Failure` variant would fail to suppress io.lg's.
                    let is_variant =
                        tail.starts_with('(') || tail == "." || tail.starts_with("with");
                    if is_type_header || (is_variant && !on_header) {
                        names.push((word.to_string(), private));
                    }
                }
            }
        }
    }
    names
}

/// The names a prelude module owns (derived from its CODE, notes stripped —
/// documentation prose must never mint a trigger name).
pub(crate) fn module_names(src: &str) -> Vec<String> {
    defined_names(&strip_note_blocks(module_code(src)))
}

/// The names a module declares `## To native …` — bound to a runtime implementation BY NAME on
/// every tier, so hygiene can never rename one (see [`crate::loader::apply_prelude_mapped`]).
pub(crate) fn module_native_names(code: &str) -> Vec<String> {
    code.lines()
        .filter_map(function_header)
        .filter(|header| header.native)
        .map(|header| header.name.to_string())
        .collect()
}

pub(crate) fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Does `source` USE `name` — call it, launch it, or name it as a type? We require `name`
/// to appear as a whole word in a *use position* (immediately called `name(`, or preceded
/// by an invocation/type keyword like `a`/`an`/`the`/`new`/`of`/`to`/`Call` or a `:`), so a
/// bare mention in prose, a string literal, or a larger identifier never drags the module
/// in. This is what lets the auto-import stay invisible without false positives.
pub(crate) fn references(source: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let sb = source.as_bytes();
    let mut search_from = 0;
    while let Some(off) = source[search_from..].find(name) {
        let start = search_from + off;
        let end = start + name.len();
        search_from = start + 1;
        // Whole word: the name must not be part of a larger identifier.
        if start > 0 && is_ident_byte(sb[start - 1]) {
            continue;
        }
        if end < sb.len() && is_ident_byte(sb[end]) {
            continue;
        }
        // Call form: `name(`.
        if end < sb.len() && sb[end] == b'(' {
            return true;
        }
        // Use position: preceded ON THE SAME LINE by an invocation/type keyword, a `:`
        // (param/field type), or a `->` (return type — `-> Result of Int and Text` is as
        // much a use of `Result` as a parameter annotation is). `When` covers match arms
        // on a module's enum variants (`When Ok (v):`). The licensing token must share
        // the name's line: a `has:` header colon or a trailing `to` that merely ENDS the
        // previous line says nothing about the first word of the next one — otherwise a
        // struct FIELD named like a stdlib function (`rank: Divergent Int` under
        // `A Page is Shared and has:`) would splice that module in (see the unit locks).
        let prefix = source[..start].trim_end();
        let gap_spans_a_line = source[prefix.len()..start].contains('\n');
        if gap_spans_a_line {
            continue;
        }
        // A template FILTER is a call written left to right: `{price | money | pad 8}` uses
        // `money` and `pad` exactly as `money(price)` would, so the demand scan must see it.
        if prefix.ends_with(':') || prefix.ends_with("->") || prefix.ends_with('|') {
            return true;
        }
        let last_word = prefix
            .rsplit(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("");
        if matches!(
            last_word,
            "a" | "an" | "the" | "new" | "of" | "to" | "Call" | "When"
        ) {
            return true;
        }
    }
    false
}

/// A `## To …` function header: the name it defines and its modifiers.
pub(crate) struct FunctionHeader<'a> {
    pub name: &'a str,
    pub native: bool,
    pub private: bool,
}

/// Read `line` as a `## To …` function header. The modifiers `native` and `private` come in any
/// order; the name ends at the first `(` (the parameters), `:` (a header with none, `## To worker:`),
/// or whitespace (`## To size of [T] …`). The one header reader every name-level pass shares, so a
/// parameterless function is `worker` everywhere, never `worker:` in one pass and `worker` in another.
pub(crate) fn function_header(line: &str) -> Option<FunctionHeader<'_>> {
    let mut rest = line.trim().strip_prefix("## To ")?;
    let (mut native, mut private) = (false, false);
    loop {
        if let Some(r) = rest.strip_prefix("native ") {
            native = true;
            rest = r;
        } else if let Some(r) = rest.strip_prefix("private ") {
            private = true;
            rest = r;
        } else {
            break;
        }
    }
    let name = rest
        .split(|c: char| c == '(' || c == ':' || c.is_whitespace())
        .next()?;
    (!name.is_empty()).then_some(FunctionHeader {
        name,
        native,
        private,
    })
}

/// The name a `## To <name> …` section defines, `None` for any other section
/// (`## A` struct, `## Main`, prose).
pub(crate) fn fn_section_name(section: &str) -> Option<String> {
    function_header(section.lines().next()?).map(|header| header.name.to_string())
}

/// Split code into its `## To` function sections: `(name, range)`, the section running from its header to the next
/// `## ` header, trimmed, as a byte range of `code`.
pub(crate) fn function_section_ranges(code: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let trimmed = |start: usize, end: usize| {
        let text = &code[start..end];
        let lead = text.len() - text.trim_start().len();
        start + lead..start + lead + text.trim().len()
    };
    let mut out = Vec::new();
    let mut current: Option<(String, usize)> = None;
    let mut at = 0;
    for line in code.split_inclusive('\n') {
        if line.starts_with("## ") {
            if let Some((name, start)) = current.take() {
                out.push((name, trimmed(start, at)));
            }
            if let Some(name) = fn_section_name(line) {
                current = Some((name, at));
            }
        }
        at += line.len();
    }
    if let Some((name, start)) = current {
        out.push((name, trimmed(start, code.len())));
    }
    out
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_compile/stdlib_text/tests.rs"]
mod tests;
