//! `test_home` — moves one crate's tests into `crates/logicaffeine_tests`.
//!
//! ```text
//! cargo run -p logicaffeine-tests --example test_home -- plan  <crate dir>   # print every move, touch nothing
//! cargo run -p logicaffeine-tests --example test_home -- apply <crate dir>   # do it, write the ledger
//! ```
//!
//! **Unit tests.** Every outermost test-only `mod` in the crate's module graph whose code sits under
//! `src/` moves to `crates/logicaffeine_tests/unit/<crate dir>/<module path>.rs` (a `src/main.rs` root
//! adds `main/`, a `src/bin/<name>.rs` root adds `bin/<name>/`). The declaration stays behind as a
//! stub carrying the original outer attributes plus `#[path = "…"]`, so the module keeps its place in
//! the tree: its test names and its access to private items do not change. An inline body is written
//! byte-for-byte, dedented one level only when every line outside a multi-line literal carries that
//! indent; outer doc comments become inner ones.
//!
//! **Integration tests.** Everything under `<crate>/tests/` moves to `crates/logicaffeine_tests/tests/`
//! with the same relative layout. A binary or helper whose name is already taken there gets the crate's
//! short name as a prefix; `fixtures/` and `goldens/` merge file by file (a file collision is fatal).
//!
//! **Paths.** A path literal is re-derived, never pattern-edited: it is resolved against what it meant
//! at the old location, mapped through the move ledger, and re-relativized from the new one. That runs
//! where the context is certain: `include_str!`/`include_bytes!`/`include!`, `#[path]` and implicit
//! `mod x;` resolution, and literals in a statement rooted in `CARGO_MANIFEST_DIR` (a unit test still
//! compiles in its own crate, an integration test's manifest dir becomes the test crate's). Any other
//! literal whose meaning the move would change is REPORTED, not touched. Last, every repo-relative
//! mention of a moved path in any text file of the repository is rewritten from the ledger.
//!
//! `apply` writes `logs/test-home/<crate dir>-ledger.tsv` (kind, old, new) and `…-report.txt`.

#[path = "../tests/common/module_graph.rs"]
mod module_graph;
#[path = "../tests/common/public_manifest.rs"]
mod public_manifest;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use proc_macro2::{Delimiter, LineColumn, TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::Visit;

use module_graph::normalize;

// ─── paths ─────────────────────────────────────────────────────────────────────────────────────

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}

fn test_crate() -> PathBuf {
    repo_root().join("crates/logicaffeine_tests")
}

fn rel(p: &Path) -> String {
    p.strip_prefix(repo_root())
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// `to` relative to directory `from` (both absolute and normalized).
fn relpath(to: &Path, from: &Path) -> String {
    let to: Vec<Component> = to.components().collect();
    let from: Vec<Component> = from.components().collect();
    let common = to
        .iter()
        .zip(from.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<String> = Vec::new();
    for _ in common..from.len() {
        parts.push("..".into());
    }
    for c in &to[common..] {
        parts.push(c.as_os_str().to_string_lossy().to_string());
    }
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

// ─── the ledger ────────────────────────────────────────────────────────────────────────────────

/// Old absolute path → new absolute path, for files and directories. `map` resolves any path through
/// its longest moved ancestor.
#[derive(Default)]
struct Ledger {
    moves: BTreeMap<PathBuf, (PathBuf, &'static str)>,
}

impl Ledger {
    fn add(&mut self, kind: &'static str, old: PathBuf, new: PathBuf) {
        if let Some((prev, _)) = self.moves.get(&old) {
            assert_eq!(prev, &new, "{} is planned to move to two places", rel(&old));
        }
        self.moves.insert(old, (new, kind));
    }

    fn map(&self, p: &Path) -> PathBuf {
        let mut probe = p.to_path_buf();
        let mut tail: Vec<std::ffi::OsString> = Vec::new();
        loop {
            if let Some((new, _)) = self.moves.get(&probe) {
                let mut out = new.clone();
                for t in tail.iter().rev() {
                    out.push(t);
                }
                return out;
            }
            match (
                probe.file_name().map(|s| s.to_os_string()),
                probe.parent().map(Path::to_path_buf),
            ) {
                (Some(name), Some(parent)) => {
                    tail.push(name);
                    probe = parent;
                }
                _ => return p.to_path_buf(),
            }
        }
    }
}

// ─── source positions ──────────────────────────────────────────────────────────────────────────

/// Byte offsets for proc-macro2 line/column positions (columns count chars).
struct Positions<'a> {
    src: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> Positions<'a> {
    fn new(src: &'a str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Positions { src, line_starts }
    }

    fn byte(&self, lc: LineColumn) -> usize {
        let start = self.line_starts[lc.line - 1];
        let line = &self.src[start..];
        line.char_indices()
            .nth(lc.column)
            .map(|(i, _)| start + i)
            .unwrap_or(start + line.len())
    }

    fn line_of_byte(&self, b: usize) -> usize {
        match self.line_starts.binary_search(&b) {
            Ok(i) => i + 1,
            Err(i) => i,
        }
    }
}

/// Every string-like literal token with its span, and the absolute line numbers strictly inside a
/// multi-line literal (their leading whitespace is content, never indentation).
fn literal_lines(src: &str) -> (Vec<(LineColumn, LineColumn, String)>, BTreeSet<usize>) {
    let mut lits = Vec::new();
    let mut protected = BTreeSet::new();
    fn walk(
        ts: TokenStream,
        lits: &mut Vec<(LineColumn, LineColumn, String)>,
        protected: &mut BTreeSet<usize>,
    ) {
        for tt in ts {
            match tt {
                TokenTree::Group(g) => walk(g.stream(), lits, protected),
                TokenTree::Literal(l) => {
                    let (s, e) = (l.span().start(), l.span().end());
                    for line in s.line + 1..=e.line {
                        protected.insert(line);
                    }
                    lits.push((s, e, l.to_string()));
                }
                _ => {}
            }
        }
    }
    if let Ok(ts) = TokenStream::from_str(src) {
        walk(ts, &mut lits, &mut protected);
    }
    (lits, protected)
}

// ─── path literals ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ctx {
    /// `include_str!`/`include_bytes!`/`include!`: relative to the containing file's directory.
    Include,
    /// `#[path = "…"]`.
    PathAttr,
    /// A statement rooted in `CARGO_MANIFEST_DIR`: relative to the package directory.
    Manifest,
    /// Anything else.
    Other,
}

/// One plain string literal in a file, with its context.
#[derive(Clone, Debug)]
struct Lit {
    start: usize,
    end: usize,
    value: String,
    ctx: Ctx,
    line: usize,
}

fn contains_manifest(tokens: &[TokenTree]) -> bool {
    tokens.iter().any(|t| match t {
        TokenTree::Ident(i) => i == "CARGO_MANIFEST_DIR",
        TokenTree::Literal(l) => l.to_string().contains("CARGO_MANIFEST_DIR"),
        TokenTree::Group(g) if g.delimiter() != Delimiter::Brace => {
            contains_manifest(&g.stream().into_iter().collect::<Vec<_>>())
        }
        _ => false,
    })
}

fn plain_str(lit: &str) -> Option<String> {
    if !lit.starts_with('"') {
        return None;
    }
    syn::parse_str::<syn::LitStr>(lit).ok().map(|s| s.value())
}

/// Every plain string literal in `src`, classified by the context that decides what it is relative to.
fn classify_literals(src: &str) -> Vec<Lit> {
    let pos = Positions::new(src);
    let mut out = Vec::new();
    let Ok(ts) = TokenStream::from_str(src) else {
        return out;
    };

    fn visit(tokens: Vec<TokenTree>, inherited: Ctx, pos: &Positions, out: &mut Vec<Lit>) {
        // Split into statements at `;` so a manifest root in one statement governs its own literals.
        let mut stmts: Vec<Vec<TokenTree>> = vec![Vec::new()];
        for t in tokens {
            let end = matches!(&t, TokenTree::Punct(p) if p.as_char() == ';');
            stmts.last_mut().unwrap().push(t);
            if end {
                stmts.push(Vec::new());
            }
        }
        for stmt in stmts {
            let base = if inherited == Ctx::Other && contains_manifest(&stmt) {
                Ctx::Manifest
            } else {
                inherited
            };
            for i in 0..stmt.len() {
                match &stmt[i] {
                    TokenTree::Group(g) => {
                        let prev_ident = |k: usize| match i.checked_sub(k).map(|j| &stmt[j]) {
                            Some(TokenTree::Ident(id)) => Some(id.to_string()),
                            _ => None,
                        };
                        let bang = matches!(i.checked_sub(1).map(|j| &stmt[j]), Some(TokenTree::Punct(p)) if p.as_char() == '!');
                        let ctx = if bang
                            && matches!(
                                prev_ident(2).as_deref(),
                                Some("include_str" | "include_bytes" | "include")
                            ) {
                            if contains_manifest(&g.stream().into_iter().collect::<Vec<_>>()) {
                                Ctx::Manifest
                            } else {
                                Ctx::Include
                            }
                        } else if g.delimiter() == Delimiter::Bracket
                            && g.stream()
                                .into_iter()
                                .next()
                                .is_some_and(|t| matches!(&t, TokenTree::Ident(id) if id == "path"))
                        {
                            Ctx::PathAttr
                        } else if g.delimiter() == Delimiter::Brace {
                            Ctx::Other
                        } else {
                            base
                        };
                        let child_ctx = if ctx == Ctx::Other && g.delimiter() == Delimiter::Brace {
                            Ctx::Other
                        } else {
                            ctx
                        };
                        visit(g.stream().into_iter().collect(), child_ctx, pos, out);
                    }
                    TokenTree::Literal(l) => {
                        if let Some(value) = plain_str(&l.to_string()) {
                            let start = pos.byte(l.span().start());
                            let end = pos.byte(l.span().end());
                            out.push(Lit {
                                start,
                                end,
                                value,
                                ctx: base,
                                line: l.span().start().line,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    visit(ts.into_iter().collect(), Ctx::Other, &pos, &mut out);
    out
}

/// The path a literal names when read against `base`, when its first component exists there (or it
/// climbs with `..`): `Some(absolute normalized path)`. A `{}`/`{name}` prefix (a `format!` root) is
/// stripped first. Returns the prefix that must be kept when rewriting.
fn resolve_literal(value: &str, base: &Path) -> Option<(String, PathBuf, String)> {
    let (prefix, path_part) = match value.find("}/") {
        Some(i) if value.starts_with('{') => (value[..i + 1].to_string(), &value[i + 1..]),
        _ => (String::new(), value),
    };
    let lead = if path_part.starts_with('/') && !prefix.is_empty() {
        "/"
    } else if path_part.starts_with('/') {
        "/"
    } else {
        ""
    };
    let trimmed = path_part.trim_start_matches('/');
    if trimmed.is_empty()
        || trimmed.contains("://")
        || trimmed.contains('{')
        || trimmed.contains('\n')
        || trimmed.contains(' ')
    {
        return None;
    }
    if Path::new(trimmed).is_absolute() {
        return None;
    }
    let first = trimmed.split('/').next().unwrap_or("");
    if first != ".." && !base.join(first).exists() {
        return None;
    }
    Some((
        format!("{prefix}{lead}"),
        normalize(&base.join(trimmed)),
        trimmed.to_string(),
    ))
}

/// The literal's text rewritten so that, read from `new_base`, it names where `ledger` put the path
/// it named from `old_base`. `None` when the text already names the right place.
fn rederive(value: &str, old_base: &Path, new_base: &Path, ledger: &Ledger) -> Option<String> {
    let (keep, old_target, trimmed) = resolve_literal(value, old_base)?;
    // A pure climb (`..`, `../..`) is one step of a `join` chain whose other steps this literal
    // cannot see; every package sits at the same depth, so the chain keeps its meaning unrewritten.
    if trimmed.trim_end_matches('/').split('/').all(|c| c == "..") {
        return None;
    }
    let new_target = ledger.map(&old_target);
    if normalize(&new_base.join(&trimmed)) == new_target {
        return None;
    }
    let mut text = relpath(&new_target, new_base);
    if trimmed.ends_with('/') && !text.ends_with('/') {
        text.push('/');
    }
    Some(format!("{keep}{text}"))
}

fn quote(value: &str) -> String {
    format!("{value:?}")
}

// ─── the plan ──────────────────────────────────────────────────────────────────────────────────

/// A text edit on one file: replace bytes `start..end` with `text`.
#[derive(Clone, Debug)]
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

fn apply_edits(src: &str, mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
    let mut out = src.to_string();
    let mut last_start = usize::MAX;
    for e in edits {
        assert!(e.end <= last_start, "overlapping edits at byte {}", e.start);
        out.replace_range(e.start..e.end, &e.text);
        last_start = e.start;
    }
    out
}

#[derive(Default)]
struct Plan {
    /// Every crate in this batch.
    crates: Vec<PathBuf>,
    /// The crate whose steps are running.
    crate_dir: PathBuf,
    short: String,
    /// How many crates of the batch bring each top-level `tests/` name (a name two of them bring is
    /// prefixed in both, like a name the test crate already has).
    incoming: BTreeMap<String, usize>,
    /// Every destination claimed so far in the batch.
    claimed: BTreeSet<PathBuf>,
    ledger: Ledger,
    /// New files to create: absolute path → content (unit bodies).
    create: BTreeMap<PathBuf, String>,
    /// Files to move whole: old → new (their content may also be rewritten, keyed by the NEW path).
    moves: BTreeMap<PathBuf, PathBuf>,
    /// Edits to existing (or moved, keyed by new path) files.
    edits: BTreeMap<PathBuf, Vec<Edit>>,
    report: Vec<String>,
    notes: Vec<String>,
}

impl Plan {
    fn edit(&mut self, file: &Path, e: Edit) {
        self.edits.entry(file.to_path_buf()).or_default().push(e);
    }
}

fn short_name(crate_dir: &Path) -> String {
    let name = crate_dir.file_name().unwrap().to_string_lossy().to_string();
    name.strip_prefix("logicaffeine_")
        .unwrap_or(&name)
        .to_string()
}

/// Where a unit test module lives in the test crate.
fn unit_target(crate_dir: &Path, root: &Path, module: &[String]) -> PathBuf {
    let mut p = test_crate()
        .join("unit")
        .join(crate_dir.file_name().unwrap());
    let root_rel = root
        .strip_prefix(crate_dir)
        .unwrap_or(root)
        .to_string_lossy()
        .replace('\\', "/");
    if root_rel == "src/main.rs" {
        p.push("main");
    } else if let Some(bin) = root_rel.strip_prefix("src/bin/") {
        p.push("bin");
        p.push(bin.trim_end_matches("/main.rs").trim_end_matches(".rs"));
    } else {
        assert!(
            root_rel == "src/lib.rs" || root_rel == "build.rs",
            "unexpected crate root {root_rel}"
        );
    }
    let (last, dirs) = module.split_last().expect("a test module has a name");
    for d in dirs {
        p.push(d);
    }
    p.push(format!("{last}.rs"));
    p
}

// ─── unit mode ─────────────────────────────────────────────────────────────────────────────────

/// Finds the `ItemMod` a `TestMod` recorded, by name and first line.
struct FindMod<'a> {
    name: &'a str,
    line: usize,
    found: Option<syn::ItemMod>,
}

impl<'a, 'ast> Visit<'ast> for FindMod<'a> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        if self.found.is_none() && m.ident == self.name && m.span().start().line == self.line {
            self.found = Some(m.clone());
            return;
        }
        syn::visit::visit_item_mod(self, m);
    }
}

fn leading_ws(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

fn plan_unit(plan: &mut Plan) {
    let crate_dir = plan.crate_dir.clone();
    let src_dir = crate_dir.join("src");
    let unit_root = test_crate().join("unit");
    let graph = module_graph::walk(&crate_dir);

    // Pass 1: the ledger entries, so every literal rewrite below sees every move. A file-backed test
    // module moves with its whole subtree, every file to its mirrored module path.
    let mut work: Vec<(module_graph::TestMod, PathBuf)> = Vec::new();
    let mut subtrees: BTreeMap<PathBuf, Vec<(PathBuf, Vec<String>, PathBuf)>> = BTreeMap::new();
    for tm in &graph.test_mods {
        let already_home = tm
            .target
            .as_ref()
            .is_some_and(|t| t.starts_with(&unit_root));
        if already_home || !tm.file.starts_with(&src_dir) {
            continue;
        }
        let target = unit_target(&crate_dir, &tm.root, &tm.module);
        if let Some(old) = &tm.target {
            let mut subtree = vec![(old.clone(), tm.module.clone(), target.clone())];
            for f in &graph.files {
                if f.module.len() > tm.module.len()
                    && f.module.starts_with(&tm.module)
                    && f.root == tm.root
                {
                    subtree.push((
                        f.path.clone(),
                        f.module.clone(),
                        unit_target(&crate_dir, &tm.root, &f.module),
                    ));
                }
            }
            for (o, _, n) in &subtree {
                plan.ledger.add("unit-file", o.clone(), n.clone());
            }
            subtrees.insert(old.clone(), subtree);
        }
        work.push((tm.clone(), target));
    }
    let owns_dir: BTreeMap<PathBuf, bool> = graph
        .files
        .iter()
        .map(|f| (f.path.clone(), f.owns_dir))
        .collect();

    // Pass 2: stubs, bodies, moved files.
    for (tm, target) in work {
        let src = std::fs::read_to_string(&tm.file).unwrap();
        let pos = Positions::new(&src);
        let ast = syn::parse_file(&src).unwrap();
        let mut finder = FindMod {
            name: &tm.name,
            line: tm.line,
            found: None,
        };
        finder.visit_file(&ast);
        let m = finder.found.unwrap_or_else(|| {
            panic!(
                "{}:{}: cannot re-find mod {}",
                rel(&tm.file),
                tm.line,
                tm.name
            )
        });

        let item_start = pos.byte(m.span().start());
        let item_end = pos.byte(m.span().end());
        let insert_at = match &m.vis {
            syn::Visibility::Inherited => pos.byte(m.mod_token.span.start()),
            v => pos.byte(v.span().start()),
        };
        let mod_line_start = src[..insert_at].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let indent = leading_ws(&src[mod_line_start..]).to_string();
        let stub_literal = relpath(&target, &tm.path_base);

        // Outer doc comments move into the unit file as inner docs; every other attribute stays.
        let mut doc_ranges: Vec<(usize, usize)> = Vec::new();
        let mut inner_docs: Vec<String> = Vec::new();
        let mut path_attr_range: Option<(usize, usize)> = None;
        // syn keeps an inline body's inner attributes in `attrs` too; they stay with the body.
        for a in m
            .attrs
            .iter()
            .filter(|a| matches!(a.style, syn::AttrStyle::Outer))
        {
            let (s, e) = (pos.byte(a.span().start()), pos.byte(a.span().end()));
            if a.path().is_ident("doc") {
                let text = &src[s..e];
                inner_docs.push(if let Some(rest) = text.strip_prefix("///") {
                    format!("//!{rest}")
                } else if let Some(rest) = text.strip_prefix("/**") {
                    format!("/*!{rest}")
                } else {
                    text.replacen("#[", "#![", 1)
                });
                // Drop the line the doc sat on when it sat alone.
                let line_start = src[..s].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let after = src[e..].find('\n').map(|i| e + i + 1).unwrap_or(e);
                if src[line_start..s].trim().is_empty() && src[e..after].trim().is_empty() {
                    let next_indent = leading_ws(&src[after..]).len();
                    doc_ranges.push((s, after + next_indent));
                } else {
                    doc_ranges.push((s, e));
                }
            } else if a.path().is_ident("path") {
                path_attr_range = Some((s, e));
            }
        }

        let path_attr = format!("#[path = {}]", quote(&stub_literal));
        let mut prefix = String::new();
        let mut cursor = item_start;
        let mut cut: Vec<(usize, usize)> = doc_ranges.clone();
        if let Some(r) = path_attr_range {
            cut.push(r);
        }
        cut.sort();
        for (s, e) in &cut {
            if *s >= cursor {
                prefix.push_str(&src[cursor..*s]);
                if Some((*s, *e)) == path_attr_range {
                    prefix.push_str(&path_attr);
                }
                cursor = *e;
            }
        }
        prefix.push_str(&src[cursor..insert_at]);
        let vis_and_mod = &src[insert_at..pos.byte(m.ident.span().end())];
        let stub = if path_attr_range.is_some() {
            format!("{prefix}{vis_and_mod};")
        } else {
            format!("{prefix}{path_attr}\n{indent}{vis_and_mod};")
        };

        match &m.content {
            Some((brace, _)) => {
                let open = pos.byte(brace.span.open().start());
                let close = pos.byte(brace.span.close().start());
                // Path literals inside the body, re-derived for the new file, applied by offset.
                let old_dir = tm.file.parent().unwrap().to_path_buf();
                let new_dir = target.parent().unwrap().to_path_buf();
                let mut body_edits: Vec<Edit> = Vec::new();
                for l in classify_literals(&src)
                    .into_iter()
                    .filter(|l| l.start > open && l.end <= close)
                {
                    let rewritten = match l.ctx {
                        Ctx::Include => rederive(&l.value, &old_dir, &new_dir, &plan.ledger),
                        Ctx::PathAttr => {
                            plan.report.push(format!(
                                "{}:{}: nested #[path] inside moved test module — check by hand",
                                rel(&tm.file),
                                l.line
                            ));
                            None
                        }
                        Ctx::Manifest => rederive(&l.value, &crate_dir, &crate_dir, &plan.ledger),
                        Ctx::Other => {
                            if let Some(new) =
                                rederive(&l.value, &crate_dir, &crate_dir, &plan.ledger)
                            {
                                plan.report.push(format!("{}:{}: literal {:?} would name {} after the move — not rewritten (unknown context)", rel(&tm.file), l.line, l.value, new));
                            }
                            None
                        }
                    };
                    if let Some(new) = rewritten {
                        plan.notes.push(format!(
                            "{}:{}: {:?} → {:?}",
                            rel(&target),
                            l.line,
                            l.value,
                            new
                        ));
                        body_edits.push(Edit {
                            start: l.start - (open + 1),
                            end: l.end - (open + 1),
                            text: quote(&new),
                        });
                    }
                }
                let body_owned = apply_edits(&src[open + 1..close], body_edits);
                let body = body_owned.as_str();
                let (_, protected) = literal_lines(&src);
                let body_first_line = pos.line_of_byte(open);
                let level = format!("{indent}    ");
                let lines: Vec<&str> = body.split('\n').collect();
                let dedentable = lines.iter().enumerate().all(|(i, l)| {
                    let abs = body_first_line + i;
                    i == 0
                        || l.trim().is_empty()
                        || protected.contains(&abs)
                        || l.starts_with(&level)
                });
                let mut text = String::new();
                for d in &inner_docs {
                    text.push_str(d);
                    text.push('\n');
                }
                let mut out_lines: Vec<String> = Vec::new();
                for (i, l) in lines.iter().enumerate() {
                    let abs = body_first_line + i;
                    if i == 0 {
                        if !l.trim().is_empty() {
                            out_lines.push(l.trim_start().to_string());
                        }
                        continue;
                    }
                    if dedentable && !protected.contains(&abs) && l.starts_with(&level) {
                        out_lines.push(l[level.len()..].to_string());
                    } else if dedentable && !protected.contains(&abs) && l.trim().is_empty() {
                        out_lines.push(String::new());
                    } else {
                        out_lines.push(l.to_string());
                    }
                }
                while out_lines.first().is_some_and(|l| l.trim().is_empty()) {
                    out_lines.remove(0);
                }
                while out_lines.last().is_some_and(|l| l.trim().is_empty()) {
                    out_lines.pop();
                }
                if !dedentable {
                    plan.report.push(format!(
                        "{}:{}: body of {} kept its indentation (a line lacks `{level}`)",
                        rel(&tm.file),
                        tm.line,
                        tm.module.join("::")
                    ));
                }
                text.push_str(&out_lines.join("\n"));
                text.push('\n');
                plan.create.insert(target.clone(), text);
                plan.edit(
                    &tm.file,
                    Edit {
                        start: item_start,
                        end: item_end,
                        text: stub,
                    },
                );
                plan.notes.push(format!(
                    "unit  {}::{}  {} → {}",
                    rel(&tm.file),
                    tm.module.join("::"),
                    rel(&tm.file),
                    rel(&target)
                ));
            }
            None => {
                let old = tm.target.clone().expect("a declaration resolves to a file");
                plan.edit(
                    &tm.file,
                    Edit {
                        start: item_start,
                        end: item_end,
                        text: stub,
                    },
                );
                for (f_old, _f_module, f_new) in subtrees.get(&old).cloned().unwrap_or_default() {
                    plan.moves.insert(f_old.clone(), f_new.clone());
                    plan.notes
                        .push(format!("unit  {} → {}", rel(&f_old), rel(&f_new)));
                    let fsrc = std::fs::read_to_string(&f_old).unwrap();
                    let old_dir = f_old.parent().unwrap().to_path_buf();
                    let new_dir = f_new.parent().unwrap().to_path_buf();
                    for l in classify_literals(&fsrc) {
                        let rewritten = match l.ctx {
                            Ctx::Include => rederive(&l.value, &old_dir, &new_dir, &plan.ledger),
                            // Module declarations are re-derived below, from the module tree.
                            Ctx::PathAttr => None,
                            Ctx::Manifest => {
                                rederive(&l.value, &crate_dir, &crate_dir, &plan.ledger)
                            }
                            Ctx::Other => {
                                if let Some(new) =
                                    rederive(&l.value, &crate_dir, &crate_dir, &plan.ledger)
                                {
                                    plan.report.push(format!("{}:{}: literal {:?} would name {} after the move — not rewritten (unknown context)", rel(&f_old), l.line, l.value, new));
                                }
                                None
                            }
                        };
                        if let Some(new) = rewritten {
                            plan.notes.push(format!(
                                "{}:{}: {:?} → {:?}",
                                rel(&f_new),
                                l.line,
                                l.value,
                                new
                            ));
                            plan.edit(
                                &f_new,
                                Edit {
                                    start: l.start,
                                    end: l.end,
                                    text: quote(&new),
                                },
                            );
                        }
                    }
                    // Every `mod x;` in a moved file points at its child's mirrored home. A moved file
                    // is `#[path]`-loaded, so it owns its directory.
                    let owns = owns_dir.get(&f_old).copied().unwrap_or(false);
                    for e in redirect_child_mods(&fsrc, &f_old, owns, &f_new, &plan.ledger) {
                        plan.edit(&f_new, e);
                    }
                }
            }
        }
    }
}

/// Collects every non-inline `mod x;` in a file with the inline-module path it sits under.
struct ModDecls {
    stack: Vec<String>,
    out: Vec<(Vec<String>, syn::ItemMod)>,
}

impl<'ast> Visit<'ast> for ModDecls {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        match &m.content {
            None => self.out.push((self.stack.clone(), m.clone())),
            Some(_) => {
                self.stack.push(m.ident.to_string());
                syn::visit::visit_item_mod(self, m);
                self.stack.pop();
            }
        }
    }
}

/// Edits that point every `mod x;` of a moved file at wherever the ledger moved the file it declared.
/// The declaration is resolved from the file's OLD place by the module-path rules; the new file is
/// `#[path]`-loaded, so it owns its directory.
fn redirect_child_mods(
    src: &str,
    old_file: &Path,
    old_owns_dir: bool,
    new_file: &Path,
    ledger: &Ledger,
) -> Vec<Edit> {
    let Ok(ast) = syn::parse_file(src) else {
        return Vec::new();
    };
    let pos = Positions::new(src);
    let mut v = ModDecls {
        stack: Vec::new(),
        out: Vec::new(),
    };
    v.visit_file(&ast);
    let old_dir = old_file.parent().unwrap().to_path_buf();
    let mut edits = Vec::new();
    for (stack, m) in v.out {
        let path_attr = m
            .attrs
            .iter()
            .find(|a| matches!(a.style, syn::AttrStyle::Outer) && a.path().is_ident("path"));
        let mut owned = if old_owns_dir {
            old_dir.clone()
        } else {
            old_dir.join(old_file.file_stem().unwrap())
        };
        for s in &stack {
            owned.push(s);
        }
        let old_target = match module_graph::path_attr(&m.attrs) {
            Some(p) if stack.is_empty() => normalize(&old_dir.join(p)),
            Some(p) => normalize(&owned.join(p)),
            None => {
                let flat = owned.join(format!("{}.rs", m.ident));
                if flat.exists() {
                    flat
                } else {
                    owned.join(m.ident.to_string()).join("mod.rs")
                }
            }
        };
        assert!(
            old_target.exists(),
            "{}: `mod {};` resolves to {}, which does not exist",
            rel(old_file),
            m.ident,
            rel(&old_target)
        );
        let new_target = ledger.map(&old_target);
        let mut base = new_file.parent().unwrap().to_path_buf();
        for s in &stack {
            base.push(s);
        }
        let literal = quote(&relpath(&new_target, &base));
        match path_attr {
            Some(a) => {
                let (s, e) = (pos.byte(a.span().start()), pos.byte(a.span().end()));
                edits.push(Edit {
                    start: s,
                    end: e,
                    text: format!("#[path = {literal}]"),
                });
            }
            None => {
                let at = match &m.vis {
                    syn::Visibility::Inherited => pos.byte(m.mod_token.span.start()),
                    vis => pos.byte(vis.span().start()),
                };
                let line_start = src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
                let indent = leading_ws(&src[line_start..]).to_string();
                edits.push(Edit {
                    start: at,
                    end: at,
                    text: format!("#[path = {literal}]\n{indent}"),
                });
            }
        }
    }
    edits
}

// ─── integration mode ──────────────────────────────────────────────────────────────────────────

fn plan_integration_ledger(plan: &mut Plan) {
    let tests = plan.crate_dir.join("tests");
    let Ok(entries) = std::fs::read_dir(&tests) else {
        return;
    };
    let home = test_crate().join("tests");
    // The directory as a whole maps onto the test crate's; entries below refine it (renames).
    plan.ledger.add("tests-dir", tests.clone(), home.clone());
    let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    names.sort();
    for old in names {
        let name = old.file_name().unwrap().to_string_lossy().to_string();
        let merge = old.is_dir() && matches!(name.as_str(), "fixtures" | "goldens");
        if merge {
            for f in walk_files(&old) {
                let new = home.join(&name).join(f.strip_prefix(&old).unwrap());
                assert!(
                    !new.exists() && !plan.claimed.contains(&new),
                    "{} collides with {}",
                    rel(&f),
                    rel(&new)
                );
                plan.claimed.insert(new.clone());
                plan.ledger.add("fixture", f.clone(), new);
            }
            plan.ledger
                .add("fixture-dir", old.clone(), home.join(&name));
            continue;
        }
        let mut new = home.join(&name);
        if new.exists() || plan.incoming.get(&name).copied().unwrap_or(0) > 1 {
            new = home.join(format!("{}_{name}", plan.short));
            assert!(
                !new.exists() && !plan.claimed.contains(&new),
                "{} collides even with the prefixed name {}",
                rel(&old),
                rel(&new)
            );
            plan.notes
                .push(format!("RENAME {} → {}", rel(&old), rel(&new)));
        }
        plan.claimed.insert(new.clone());
        let kind = if old.is_file() && name.ends_with(".rs") {
            "binary"
        } else if old.is_dir() && old.join("main.rs").is_file() {
            "binary"
        } else {
            "helper"
        };
        plan.ledger.add(kind, old, new);
    }
}

fn walk_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn plan_integration(plan: &mut Plan) {
    let tests = plan.crate_dir.join("tests");
    if !tests.is_dir() {
        return;
    }
    let crate_dir = plan.crate_dir.clone();
    let home_pkg = test_crate();
    let moved: Vec<(PathBuf, PathBuf)> = plan
        .ledger
        .moves
        .iter()
        .filter(|(old, (_, kind))| {
            old.starts_with(&tests) && matches!(*kind, "binary" | "helper" | "fixture")
        })
        .map(|(o, (n, _))| (o.clone(), n.clone()))
        .collect();
    for (old, new) in &moved {
        plan.moves.insert(old.clone(), new.clone());
        plan.notes
            .push(format!("move  {} → {}", rel(old), rel(new)));
    }
    // Rust sources among the moved files get their paths re-derived.
    let mut rust_files: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (old, new) in &moved {
        if old.is_dir() {
            for f in walk_files(old) {
                if f.extension().is_some_and(|x| x == "rs") {
                    rust_files.push((f.clone(), new.join(f.strip_prefix(old).unwrap())));
                }
            }
        } else if old.extension().is_some_and(|x| x == "rs") {
            rust_files.push((old.clone(), new.clone()));
        }
    }
    for (old, new) in rust_files {
        let src = std::fs::read_to_string(&old).unwrap();
        let old_dir = old.parent().unwrap().to_path_buf();
        let new_dir = new.parent().unwrap().to_path_buf();
        for l in classify_literals(&src) {
            let climb =
                !l.value.is_empty() && l.value.trim_end_matches('/').split('/').all(|c| c == "..");
            if climb
                && l.value.trim_end_matches('/') == ".."
                && crate_dir.parent() != home_pkg.parent()
                && l.ctx != Ctx::Include
            {
                plan.report.push(format!("{}:{}: a single `..` climbs to {} at home but to {} from the new home — check the chain by hand", rel(&old), l.line, rel(crate_dir.parent().unwrap()), rel(home_pkg.parent().unwrap())));
            }
            let rewritten = match l.ctx {
                Ctx::Include | Ctx::PathAttr => {
                    rederive(&l.value, &old_dir, &new_dir, &plan.ledger)
                }
                Ctx::Manifest => rederive(&l.value, &crate_dir, &home_pkg, &plan.ledger),
                Ctx::Other => {
                    if let Some(n) = rederive(&l.value, &crate_dir, &home_pkg, &plan.ledger) {
                        plan.report.push(format!("{}:{}: literal {:?} would need {:?} from the new home — not rewritten (unknown context)", rel(&old), l.line, l.value, n));
                    }
                    None
                }
            };
            if let Some(n) = rewritten {
                plan.notes
                    .push(format!("{}:{}: {:?} → {:?}", rel(&new), l.line, l.value, n));
                plan.edit(
                    &new,
                    Edit {
                        start: l.start,
                        end: l.end,
                        text: quote(&n),
                    },
                );
            }
        }
        // Implicit `mod x;` whose file moved somewhere the implicit rule no longer finds.
        if let Ok(ast) = syn::parse_file(&src) {
            let pos = Positions::new(&src);
            let owns_dir = old.parent() == Some(tests.as_path())
                || matches!(
                    old.file_name().and_then(|n| n.to_str()),
                    Some("mod.rs" | "main.rs")
                );
            for item in &ast.items {
                let syn::Item::Mod(m) = item else { continue };
                if m.content.is_some() || module_graph::path_attr(&m.attrs).is_some() {
                    continue;
                }
                let base_old = if owns_dir {
                    old_dir.clone()
                } else {
                    old_dir.join(old.file_stem().unwrap())
                };
                let flat = base_old.join(format!("{}.rs", m.ident));
                let target_old = if flat.exists() {
                    flat
                } else {
                    base_old.join(m.ident.to_string()).join("mod.rs")
                };
                let target_new = plan.ledger.map(&target_old);
                let base_new = if owns_dir {
                    new_dir.clone()
                } else {
                    new_dir.join(new.file_stem().unwrap())
                };
                let implicit_new = [
                    base_new.join(format!("{}.rs", m.ident)),
                    base_new.join(m.ident.to_string()).join("mod.rs"),
                ];
                if !implicit_new.contains(&target_new) {
                    let at = match &m.vis {
                        syn::Visibility::Inherited => pos.byte(m.mod_token.span.start()),
                        v => pos.byte(v.span().start()),
                    };
                    let line_start = src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let indent = leading_ws(&src[line_start..]).to_string();
                    let text = format!(
                        "#[path = {}]\n{indent}",
                        quote(&relpath(&target_new, &new_dir))
                    );
                    plan.notes.push(format!(
                        "{}: `mod {};` → #[path = {:?}]",
                        rel(&new),
                        m.ident,
                        relpath(&target_new, &new_dir)
                    ));
                    plan.edit(
                        &new,
                        Edit {
                            start: at,
                            end: at,
                            text,
                        },
                    );
                }
            }
        }
    }
}

// ─── repository-wide references ────────────────────────────────────────────────────────────────

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".claude",
    "target",
    "node_modules",
    ".codex-worktrees",
    "logs",
    "mutants.out",
    "mutants.out.old",
    "lost+found",
];

/// Vendored third-party trees and run output at the repository root: nothing in them names our paths.
const SKIP_TOP: &[&str] = &["external", "models", "reference-kernels", "tools"];

fn text_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                if SKIP_DIRS.contains(&name.as_str())
                    || name.starts_with("target")
                    || p.join("CACHEDIR.TAG").exists()
                {
                    continue;
                }
                if d == root && SKIP_TOP.contains(&name.as_str()) {
                    continue;
                }
                if rel(&p) == "benchmarks/results/history" || rel(&p) == "work/research" {
                    continue;
                }
                stack.push(p);
            } else if ft.is_file() {
                let Ok(md) = e.metadata() else { continue };
                if md.len() <= 4 * 1024 * 1024 {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

fn boundary(c: Option<char>) -> bool {
    !matches!(c, Some(ch) if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
}

/// The golden `test_home_lock` freezes: moved binaries leave it by deletion, never by path rewrite.
const TEST_HOME_GOLDEN: &str =
    "crates/logicaffeine_tests/tests/goldens/crate_local_test_binaries.txt";

/// Every mention of a moved path, as a repo-relative string, rewritten to its new location, over the
/// files as they are on disk now.
fn reference_edits(ledger: &Ledger) -> BTreeMap<PathBuf, Vec<Edit>> {
    let root = repo_root();
    let mut pairs: Vec<(String, String)> = ledger
        .moves
        .iter()
        .map(|(o, (n, _))| (rel(o), rel(n)))
        .filter(|(o, n)| o != n)
        .collect();
    pairs.sort_by_key(|(o, _)| std::cmp::Reverse(o.len()));
    let mut out = BTreeMap::new();
    for f in text_files(&root) {
        if rel(&f) == TEST_HOME_GOLDEN {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        if !pairs.iter().any(|(o, _)| text.contains(o.as_str())) {
            continue;
        }
        let mut edits: Vec<Edit> = Vec::new();
        let mut claimed: Vec<(usize, usize)> = Vec::new();
        for (old, new) in &pairs {
            let mut from = 0;
            while let Some(i) = text[from..].find(old.as_str()) {
                let s = from + i;
                let e = s + old.len();
                from = e;
                let before = text[..s].chars().next_back();
                let after = text[e..].chars().next();
                let ok_before = boundary(before) || before == Some('/');
                let ok_after = boundary(after) || after == Some('/');
                if !(ok_before && ok_after) || claimed.iter().any(|(cs, ce)| s < *ce && e > *cs) {
                    continue;
                }
                claimed.push((s, e));
                edits.push(Edit {
                    start: s,
                    end: e,
                    text: new.clone(),
                });
            }
        }
        if !edits.is_empty() {
            out.insert(f, edits);
        }
    }
    out
}

// ─── driver ────────────────────────────────────────────────────────────────────────────────────

fn build_plan(crate_rels: &[String]) -> Plan {
    let mut plan = Plan::default();
    for c in crate_rels {
        let crate_dir = normalize(&repo_root().join(c.trim_end_matches('/')));
        assert!(
            crate_dir.join("Cargo.toml").is_file(),
            "{c} is not a package directory"
        );
        assert!(
            crate_dir != test_crate(),
            "the test crate is the destination, not a source"
        );
        if let Ok(rd) = std::fs::read_dir(crate_dir.join("tests")) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !matches!(name.as_str(), "fixtures" | "goldens") {
                    *plan.incoming.entry(name).or_default() += 1;
                }
            }
        }
        plan.crates.push(crate_dir);
    }
    // Integration ledgers first: unit literals that reach into any `tests/` must see where it goes.
    for c in plan.crates.clone() {
        plan.short = short_name(&c);
        plan.crate_dir = c;
        plan_integration_ledger(&mut plan);
    }
    for c in plan.crates.clone() {
        plan.short = short_name(&c);
        plan.crate_dir = c;
        plan_unit(&mut plan);
        plan_integration(&mut plan);
    }
    plan
}

fn print_plan(plan: &Plan) {
    for n in &plan.notes {
        println!("{n}");
    }
    let refs = reference_edits(&plan.ledger);
    for (f, e) in &refs {
        println!("refs  {} ({} mention(s))", rel(f), e.len());
    }
    println!(
        "\n{} new unit file(s), {} moved path(s), {} edited file(s)",
        plan.create.len(),
        plan.moves.len(),
        plan.edits.len()
    );
    // A renamed binary is also named by bare stem in runners, filters and docs.
    let renamed: Vec<(String, String)> = plan
        .ledger
        .moves
        .iter()
        .filter(|(o, (n, k))| *k == "binary" && o.file_name() != n.file_name())
        .map(|(o, (n, _))| {
            (
                o.file_stem().unwrap().to_string_lossy().to_string(),
                n.file_stem().unwrap().to_string_lossy().to_string(),
            )
        })
        .collect();
    if !renamed.is_empty() {
        println!("\nRENAMED binaries — bare-stem mentions to review:");
        for f in text_files(&repo_root()) {
            let Ok(text) = std::fs::read_to_string(&f) else {
                continue;
            };
            for (old, new) in &renamed {
                for (i, line) in text.lines().enumerate() {
                    let hit = line.match_indices(old.as_str()).any(|(s, _)| {
                        boundary(line[..s].chars().next_back())
                            && boundary(line[s + old.len()..].chars().next())
                    });
                    if hit {
                        println!(
                            "  {}:{}: {old} (→ {new}): {}",
                            rel(&f),
                            i + 1,
                            line.trim().chars().take(140).collect::<String>()
                        );
                    }
                }
            }
        }
    }
    if !plan.report.is_empty() {
        println!("\nREVIEW ({}):", plan.report.len());
        for r in &plan.report {
            println!("  {r}");
        }
    }
}

fn apply(plan: &Plan) {
    let root = repo_root();
    // 1. Edits to files that stay put.
    for (file, edits) in &plan.edits {
        if plan.moves.values().any(|n| n == file)
            || plan
                .moves
                .iter()
                .any(|(o, n)| file.starts_with(n) && !o.is_file() && file != n)
        {
            continue;
        }
        if plan.create.contains_key(file) {
            continue;
        }
        let src =
            std::fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", rel(file)));
        std::fs::write(file, apply_edits(&src, edits.clone())).unwrap();
    }
    // 2. Moves (files and directories), then edits keyed by their new paths.
    for (old, new) in &plan.moves {
        if !old.exists() {
            continue;
        }
        std::fs::create_dir_all(new.parent().unwrap()).unwrap();
        if old.is_dir() && new.is_dir() {
            for f in walk_files(old) {
                let dest = new.join(f.strip_prefix(old).unwrap());
                assert!(!dest.exists(), "{} would overwrite {}", rel(&f), rel(&dest));
                std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
                std::fs::rename(&f, &dest).unwrap();
            }
        } else {
            assert!(!new.exists(), "{} would overwrite {}", rel(old), rel(new));
            std::fs::rename(old, new)
                .unwrap_or_else(|e| panic!("move {} → {}: {e}", rel(old), rel(new)));
        }
    }
    for (file, edits) in &plan.edits {
        let moved_here = plan.moves.values().any(|n| n == file)
            || plan
                .moves
                .iter()
                .any(|(_, n)| file.starts_with(n) && file != n);
        if !moved_here || plan.create.contains_key(file) {
            continue;
        }
        let src = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("read moved {}: {e}", rel(file)));
        std::fs::write(file, apply_edits(&src, edits.clone())).unwrap();
    }
    // 3. New unit files (with any repo-relative mentions inside them already re-derived).
    for (file, text) in &plan.create {
        assert!(!file.exists(), "{} already exists", rel(file));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let text = match plan.edits.get(file) {
            Some(e) => apply_edits(text, e.clone()),
            None => text.clone(),
        };
        std::fs::write(file, text).unwrap();
    }
    // 4. Repo-relative mentions of every moved path, over the files where they now are.
    let mut ref_log = Vec::new();
    for (file, edits) in reference_edits(&plan.ledger) {
        let src = std::fs::read_to_string(&file).unwrap();
        ref_log.push(format!("refs  {} ({} mention(s))", rel(&file), edits.len()));
        std::fs::write(&file, apply_edits(&src, edits)).unwrap();
    }
    // 5. The frozen crate-local list loses the binaries that moved home.
    let golden = root.join(TEST_HOME_GOLDEN);
    if let Ok(text) = std::fs::read_to_string(&golden) {
        let gone: Vec<String> = plan
            .crates
            .iter()
            .map(|c| format!("{}/tests/", rel(c)))
            .collect();
        let kept: Vec<&str> = text
            .lines()
            .filter(|l| !gone.iter().any(|g| l.starts_with(g.as_str())))
            .collect();
        std::fs::write(&golden, kept.join("\n") + "\n").unwrap();
    }
    // 6. Emptied source directories go.
    for c in &plan.crates {
        remove_empty_dirs(&c.join("tests"));
    }
    for (old, _) in &plan.moves {
        if let Some(parent) = old.parent() {
            remove_empty_dirs(parent);
        }
    }
    // 7. Ledger and report.
    let logs = root.join("logs/test-home");
    std::fs::create_dir_all(&logs).unwrap();
    for c in &plan.crates {
        let name = c.file_name().unwrap().to_string_lossy().to_string();
        let mut ledger = String::from("kind\told\tnew\n");
        for (o, (n, k)) in &plan.ledger.moves {
            if o.starts_with(c) {
                ledger.push_str(&format!("{k}\t{}\t{}\n", rel(o), rel(n)));
            }
        }
        std::fs::write(logs.join(format!("{name}-ledger.tsv")), ledger).unwrap();
    }
    let name = format!(
        "batch-{}",
        plan.crates
            .iter()
            .map(|c| short_name(c))
            .collect::<Vec<_>>()
            .join("+")
    );
    let mut report = plan.notes.join("\n");
    report.push('\n');
    report.push_str(&ref_log.join("\n"));
    report.push_str("\n\nREVIEW:\n");
    report.push_str(&plan.report.join("\n"));
    report.push('\n');
    std::fs::write(logs.join(format!("{name}-report.txt")), report).unwrap();
}

fn remove_empty_dirs(dir: &Path) {
    if !dir.is_dir() {
        return;
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                remove_empty_dirs(&e.path());
            }
        }
    }
    if std::fs::read_dir(dir)
        .map(|mut r| r.next().is_none())
        .unwrap_or(false)
    {
        let _ = std::fs::remove_dir(dir);
    }
}

// ─── census: every test in the workspace, statically ───────────────────────────────────────────

/// One `#[test]` function: where it runs and under which conditions.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TestId {
    /// `unit\t<crate dir>\t<root>` or `integ\t<binary>`.
    home: String,
    /// Module path inside the target, ending in the function name.
    path: String,
    ignored: bool,
    /// Every cfg predicate on the way to it except `test` (sorted, so file splits cannot reorder it).
    cfgs: Vec<String>,
}

struct CensusWalk<'a> {
    home: String,
    out: &'a mut Vec<TestId>,
}

fn cfg_preds(attrs: &[syn::Attribute]) -> Vec<String> {
    attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg"))
        .map(|a| module_graph::compact(&a.meta))
        .filter(|c| c != "cfg(test)")
        .collect()
}

impl<'a> CensusWalk<'a> {
    /// `file` as module `module`, reached under `chain`; `owns_dir` per the module-path rules.
    fn file(&mut self, file: &Path, module: Vec<String>, chain: Vec<String>, owns_dir: bool) {
        let src = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("census: read {}: {e}", rel(file)));
        let ast = syn::parse_file(&src)
            .unwrap_or_else(|e| panic!("census: {} does not parse: {e}", rel(file)));
        let mut chain = chain;
        chain.extend(cfg_preds(&ast.attrs));
        let dir = file.parent().unwrap().to_path_buf();
        let base = if owns_dir {
            dir.clone()
        } else {
            dir.join(file.file_stem().unwrap())
        };
        self.items(&ast.items, file, &dir, &base, &module, &chain, true);
    }

    #[allow(clippy::too_many_arguments)]
    fn items(
        &mut self,
        items: &[syn::Item],
        file: &Path,
        file_dir: &Path,
        base: &Path,
        module: &[String],
        chain: &[String],
        top: bool,
    ) {
        for item in items {
            match item {
                syn::Item::Fn(f) if f.attrs.iter().any(module_graph::is_test_attr) => {
                    let mut c = chain.to_vec();
                    c.extend(cfg_preds(&f.attrs));
                    c.sort();
                    let mut path = module.to_vec();
                    path.push(f.sig.ident.to_string());
                    let ignored = f.attrs.iter().any(|a| a.path().is_ident("ignore"));
                    self.out.push(TestId {
                        home: self.home.clone(),
                        path: path.join("::"),
                        ignored,
                        cfgs: c,
                    });
                }
                syn::Item::Mod(m) => {
                    let mut c = chain.to_vec();
                    c.extend(cfg_preds(&m.attrs));
                    let mut sub = module.to_vec();
                    sub.push(m.ident.to_string());
                    match &m.content {
                        Some((_, inner)) => {
                            self.items(
                                inner,
                                file,
                                file_dir,
                                &base.join(m.ident.to_string()),
                                &sub,
                                &c,
                                false,
                            );
                        }
                        None => {
                            let (target, via_path) = match module_graph::path_attr(&m.attrs) {
                                Some(p) if top => (normalize(&file_dir.join(p)), true),
                                Some(p) => (normalize(&base.join(p)), true),
                                None => {
                                    let flat = base.join(format!("{}.rs", m.ident));
                                    (
                                        if flat.exists() {
                                            flat
                                        } else {
                                            base.join(m.ident.to_string()).join("mod.rs")
                                        },
                                        false,
                                    )
                                }
                            };
                            let owns = via_path
                                || matches!(
                                    target.file_name().and_then(|n| n.to_str()),
                                    Some("mod.rs")
                                );
                            self.file(&target, sub, c, owns);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn workspace_packages(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for parent in ["crates", "apps", "scripts"] {
        let Ok(rd) = std::fs::read_dir(root.join(parent)) else {
            continue;
        };
        for e in rd.flatten() {
            if e.path().join("Cargo.toml").is_file() && e.path().join("src").is_dir() {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

/// Every test function in every package: unit tests from each crate root, integration tests from each
/// `tests/` binary root.
fn census(root: &Path) -> Vec<TestId> {
    let mut out = Vec::new();
    for pkg in workspace_packages(root) {
        let name = pkg.file_name().unwrap().to_string_lossy().to_string();
        for root in module_graph::roots(&pkg) {
            if root.file_name().is_some_and(|n| n == "build.rs") {
                continue;
            }
            let root_rel = root
                .strip_prefix(&pkg)
                .unwrap()
                .to_string_lossy()
                .to_string();
            let mut w = CensusWalk {
                home: format!("unit\t{name}\t{root_rel}"),
                out: &mut out,
            };
            w.file(&root, Vec::new(), Vec::new(), true);
        }
        if let Ok(rd) = std::fs::read_dir(pkg.join("tests")) {
            let mut bins: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            bins.sort();
            for p in bins {
                let (root, bin) = if p.is_file() && p.extension().is_some_and(|x| x == "rs") {
                    (
                        p.clone(),
                        p.file_stem().unwrap().to_string_lossy().to_string(),
                    )
                } else if p.join("main.rs").is_file() {
                    (
                        p.join("main.rs"),
                        p.file_name().unwrap().to_string_lossy().to_string(),
                    )
                } else {
                    continue;
                };
                let mut w = CensusWalk {
                    home: format!("integ\t{name}\t{bin}"),
                    out: &mut out,
                };
                w.file(&root, Vec::new(), Vec::new(), true);
            }
        }
    }
    out.sort();
    out
}

fn write_census(path: &Path, root: &Path) {
    let ids = census(root);
    let mut text = String::new();
    for t in &ids {
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            t.home,
            t.path,
            if t.ignored { "ignored" } else { "-" },
            t.cfgs.join(" & ")
        ));
    }
    std::fs::write(path, text).unwrap();
    println!("census: {} test functions → {}", ids.len(), rel(path));
}

/// Before/after census comparison. An integration binary is keyed by (package dir, binary); every
/// ledger maps a moved one to (logicaffeine_tests, its new name).
fn census_diff(before: &Path, after: &Path) -> bool {
    let mut renames: BTreeMap<(String, String), String> = BTreeMap::new();
    if let Ok(rd) = std::fs::read_dir(repo_root().join("logs/test-home")) {
        for e in rd.flatten() {
            if !e.file_name().to_string_lossy().ends_with("-ledger.tsv") {
                continue;
            }
            for line in std::fs::read_to_string(e.path()).unwrap().lines().skip(1) {
                let cols: Vec<&str> = line.split('\t').collect();
                if cols.len() == 3 && cols[0] == "binary" {
                    let name = |p: &str| {
                        let p = Path::new(p);
                        if p.extension().is_some_and(|x| x == "rs") {
                            p.file_stem()
                        } else {
                            p.file_name()
                        }
                        .unwrap()
                        .to_string_lossy()
                        .to_string()
                    };
                    // `<pkg dir>/tests/<binary>`: the package dir is the component before `tests`.
                    let parts: Vec<&str> = cols[1].split('/').collect();
                    let pkg = parts
                        .iter()
                        .position(|c| *c == "tests")
                        .map(|i| parts[i - 1].to_string())
                        .unwrap_or_default();
                    renames.insert((pkg, name(cols[1])), name(cols[2]));
                }
            }
        }
    }
    let load = |p: &Path, map: bool| -> BTreeMap<String, usize> {
        let mut m = BTreeMap::new();
        for line in std::fs::read_to_string(p).unwrap().lines() {
            let mut cols: Vec<String> = line.split('\t').map(String::from).collect();
            if map && cols[0] == "integ" {
                if let Some(n) = renames.get(&(cols[1].clone(), cols[2].clone())) {
                    cols[1] = "logicaffeine_tests".into();
                    cols[2] = n.clone();
                }
            }
            *m.entry(cols.join("\t")).or_default() += 1;
        }
        m
    };
    let b = load(before, true);
    let a = load(after, false);
    let mut ok = true;
    for (k, n) in &b {
        let m = a.get(k).copied().unwrap_or(0);
        if m != *n {
            ok = false;
            println!("LOST    {k} (before {n}, after {m})");
        }
    }
    for (k, n) in &a {
        if !b.contains_key(k) {
            ok = false;
            println!("NEW     {k} ({n})");
        }
    }
    let total_b: usize = b.values().sum();
    let total_a: usize = a.values().sum();
    println!(
        "census: before {total_b}, after {total_a} — {}",
        if ok { "PARITY OK" } else { "PARITY BROKEN" }
    );
    ok
}

// ─── research split: which tests of a file reach a research module ────────────────────────────

const RESEARCH: &[&str] = &[
    "mathir",
    "knot",
    "attack",
    "collapse",
    "doc",
    "census",
    "cofactor",
    "polycalc_gfp",
    "polycalc_zm",
    "orbit_stability",
    "minrank",
    "res_width",
    "category_collapse",
];

fn flat_idents(ts: TokenStream, out: &mut Vec<String>) {
    for t in ts {
        match t {
            TokenTree::Group(g) => {
                out.push("(".into());
                flat_idents(g.stream(), out);
                out.push(")".into());
            }
            TokenTree::Ident(i) => out.push(i.to_string()),
            TokenTree::Punct(p) => out.push(p.as_char().to_string()),
            TokenTree::Literal(_) => out.push("lit".into()),
        }
    }
}

/// Idents a `use` tree brings into scope from research modules of `logicaffeine_proof`.
fn research_names_of_use(tree: &syn::UseTree, path: &mut Vec<String>, out: &mut BTreeSet<String>) {
    match tree {
        syn::UseTree::Path(p) => {
            path.push(p.ident.to_string());
            research_names_of_use(&p.tree, path, out);
            path.pop();
        }
        syn::UseTree::Name(n) => {
            // `use a::m::{self, …}` brings `m` itself into scope.
            let (full, name) = if n.ident == "self" {
                (path.clone(), path.last().cloned().unwrap_or_default())
            } else {
                let mut full = path.clone();
                full.push(n.ident.to_string());
                (full, n.ident.to_string())
            };
            if is_research_path(&full) {
                out.insert(name);
            }
        }
        syn::UseTree::Rename(r) => {
            let mut full = path.clone();
            full.push(r.ident.to_string());
            if is_research_path(&full) {
                out.insert(r.rename.to_string());
            }
        }
        syn::UseTree::Glob(_) => {
            if is_research_path(path) {
                out.insert("*".into());
            }
        }
        syn::UseTree::Group(g) => {
            for t in &g.items {
                research_names_of_use(t, path, out);
            }
        }
    }
}

fn is_research_path(full: &[String]) -> bool {
    full.len() >= 2 && full[0] == "logicaffeine_proof" && RESEARCH.contains(&full[1].as_str())
}

/// Whether a token run names a research module directly: `logicaffeine_proof :: <research>`.
fn names_research(toks: &[String]) -> bool {
    toks.windows(4).any(|w| {
        w[0] == "logicaffeine_proof"
            && w[1] == ":"
            && w[2] == ":"
            && RESEARCH.contains(&w[3].as_str())
    })
}

fn item_name(item: &syn::Item) -> Option<String> {
    Some(match item {
        syn::Item::Fn(f) => f.sig.ident.to_string(),
        syn::Item::Struct(s) => s.ident.to_string(),
        syn::Item::Enum(e) => e.ident.to_string(),
        syn::Item::Const(c) => c.ident.to_string(),
        syn::Item::Static(c) => c.ident.to_string(),
        syn::Item::Type(t) => t.ident.to_string(),
        syn::Item::Mod(m) => m.ident.to_string(),
        syn::Item::Trait(t) => t.ident.to_string(),
        syn::Item::Macro(m) => m.ident.as_ref()?.to_string(),
        _ => return None,
    })
}

/// Per top-level item: does it (transitively, through this file's own items) reach research?
fn research_reach(file: &syn::File) -> Vec<bool> {
    use quote::ToTokens;
    let mut research_names: BTreeSet<String> = BTreeSet::new();
    for item in &file.items {
        if let syn::Item::Use(u) = item {
            research_names_of_use(&u.tree, &mut Vec::new(), &mut research_names);
        }
    }
    let toks: Vec<Vec<String>> = file
        .items
        .iter()
        .map(|i| {
            let mut v = Vec::new();
            flat_idents(i.to_token_stream(), &mut v);
            v
        })
        .collect();
    let names: Vec<Option<String>> = file.items.iter().map(item_name).collect();
    let mut reach: Vec<bool> = file
        .items
        .iter()
        .zip(&toks)
        .map(|(item, t)| {
            !matches!(item, syn::Item::Use(_))
                && (names_research(t) || t.iter().any(|x| research_names.contains(x)))
        })
        .collect();
    loop {
        let reached: BTreeSet<String> = names
            .iter()
            .zip(&reach)
            .filter(|(_, r)| **r)
            .filter_map(|(n, _)| n.clone())
            .collect();
        let mut changed = false;
        for i in 0..reach.len() {
            if !reach[i]
                && !matches!(file.items[i], syn::Item::Use(_))
                && toks[i].iter().any(|x| reached.contains(x))
            {
                reach[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    reach
}

fn is_test_fn(item: &syn::Item) -> bool {
    matches!(item, syn::Item::Fn(f) if f.attrs.iter().any(module_graph::is_test_attr))
}

/// Splits one test file at item boundaries, byte-for-byte: the research half (every item but the
/// tests that never reach research) goes to `out`, the product half (every item that does not reach
/// research, minus `use`s that import only from research modules) stays at `file`.
fn split_research(file: &str, out: &str) {
    let src = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{file}: {e}"));
    let ast = syn::parse_file(&src).unwrap_or_else(|e| panic!("{file}: {e}"));
    let pos = Positions::new(&src);
    let reach = research_reach(&ast);
    let starts: Vec<usize> = ast
        .items
        .iter()
        .map(|i| pos.byte(i.span().start()))
        .collect();
    let ends: Vec<usize> = ast.items.iter().map(|i| pos.byte(i.span().end())).collect();
    let header = &src[..starts.first().copied().unwrap_or(src.len())];
    let trailer = &src[ends.last().copied().unwrap_or(0)..];
    let mut research = header.to_string();
    let mut product = header.to_string();
    for (k, item) in ast.items.iter().enumerate() {
        let seg_start = if k == 0 { starts[0] } else { ends[k - 1] };
        let seg = &src[seg_start..ends[k]];
        let is_test = is_test_fn(item)
            || matches!(item, syn::Item::Mod(m) if m.content.as_ref().is_some_and(|(_, inner)| inner.iter().any(is_test_fn)));
        let research_use = match item {
            syn::Item::Use(u) => {
                let mut names = BTreeSet::new();
                research_names_of_use(&u.tree, &mut Vec::new(), &mut names);
                let mut all = BTreeSet::new();
                collect_use_leaves(&u.tree, &mut all);
                !names.is_empty() && names.len() == all.len()
            }
            _ => false,
        };
        if let syn::Item::Use(u) = item {
            let mut names = BTreeSet::new();
            research_names_of_use(&u.tree, &mut Vec::new(), &mut names);
            let mut all = BTreeSet::new();
            collect_use_leaves(&u.tree, &mut all);
            assert!(
                names.is_empty() || names.len() == all.len(),
                "{file}: a `use` mixes research and product names — split it by hand first:\n{seg}"
            );
        }
        if !(is_test && !reach[k]) {
            research.push_str(seg);
        }
        if !reach[k] && !research_use {
            product.push_str(seg);
        }
    }
    research.push_str(trailer);
    product.push_str(trailer);
    std::fs::create_dir_all(Path::new(out).parent().unwrap()).unwrap();
    std::fs::write(out, research).unwrap();
    std::fs::write(file, product).unwrap();
}

fn collect_use_leaves(tree: &syn::UseTree, out: &mut BTreeSet<String>) {
    match tree {
        syn::UseTree::Path(p) => collect_use_leaves(&p.tree, out),
        syn::UseTree::Name(n) => {
            out.insert(if n.ident == "self" {
                "self".to_string()
            } else {
                n.ident.to_string()
            });
        }
        syn::UseTree::Rename(r) => {
            out.insert(r.rename.to_string());
        }
        syn::UseTree::Glob(_) => {
            out.insert("*".into());
        }
        syn::UseTree::Group(g) => g.items.iter().for_each(|t| collect_use_leaves(t, out)),
    }
}

/// `<file>\t<research tests>\t<product tests>` for each file.
fn classify_research(files: &[String]) {
    for f in files {
        let src = std::fs::read_to_string(f).unwrap_or_else(|e| panic!("{f}: {e}"));
        let Ok(ast) = syn::parse_file(&src) else {
            println!("{f}\tPARSE-ERROR");
            continue;
        };
        let reach = research_reach(&ast);
        let (mut r, mut p) = (0, 0);
        for (item, hit) in ast.items.iter().zip(&reach) {
            if is_test_fn(item) {
                if *hit {
                    r += 1
                } else {
                    p += 1
                }
            }
        }
        // Tests nested in modules count by the module's reach.
        for (item, hit) in ast.items.iter().zip(&reach) {
            if let syn::Item::Mod(m) = item {
                if let Some((_, inner)) = &m.content {
                    let n = inner.iter().filter(|i| is_test_fn(i)).count();
                    if *hit {
                        r += n
                    } else {
                        p += n
                    }
                }
            }
        }
        println!("{f}\t{r}\t{p}");
    }
}

/// `goldens/src_test_items.txt` as `unit_home_lock` computes it: per governed file, the test-only
/// attributes left in `src/` outside every test module.
fn write_items(out: &Path) {
    let held = public_manifest::held_back(&public_manifest::Classifier::load());
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for dir in public_manifest::packages().into_keys() {
        if dir == "crates/logicaffeine_tests" || held.contains_key(&dir) {
            continue;
        }
        let path = repo_root().join(&dir);
        if !path.join("src").is_dir() {
            continue;
        }
        for item in module_graph::walk(&path).test_items {
            if item.file.starts_with(path.join("src")) {
                *counts.entry(rel(&item.file)).or_default() += 1;
            }
        }
    }
    let mut text = String::from(
        "# Test-only attributes left in a crate's src/ outside every test module, per file (unit_home_lock).\n\
         # Instrumentation production code calls stays; a helper only tests use belongs in the unit file\n\
         # beside them. A count may only fall and a file may not appear: lower a count when code leaves.\n",
    );
    for (f, n) in &counts {
        text.push_str(&format!("{f}\t{n}\n"));
    }
    std::fs::write(out, text).unwrap();
    println!(
        "items: {} file(s), {} attribute(s) → {}",
        counts.len(),
        counts.values().sum::<usize>(),
        rel(out)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("census") if args.len() == 2 || args.len() == 3 => {
            let root = args.get(2).map(PathBuf::from).unwrap_or_else(repo_root);
            write_census(&repo_root().join(&args[1]), &root)
        }
        Some("census-diff") if args.len() == 3 => {
            if !census_diff(&repo_root().join(&args[1]), &repo_root().join(&args[2])) {
                std::process::exit(1);
            }
        }
        Some("items") if args.len() == 2 => write_items(&repo_root().join(&args[1])),
        Some("classify-research") if args.len() >= 2 => classify_research(&args[1..]),
        Some("split-research") if args.len() == 3 => split_research(&args[1], &args[2]),
        Some(mode @ ("plan" | "apply")) if args.len() >= 2 => {
            let plan = build_plan(&args[1..]);
            print_plan(&plan);
            if mode == "apply" {
                apply(&plan);
                println!("\napplied; ledger + report in logs/test-home/");
            }
        }
        _ => {
            eprintln!("usage: test_home <plan|apply> <crate dir>… | census <out.tsv> [root] | census-diff <before.tsv> <after.tsv>");
            std::process::exit(2);
        }
    }
}
