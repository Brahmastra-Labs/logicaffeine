//! A crate's module graph, resolved the way rustc resolves it, for the locks that reason about what a
//! crate compiles and what it only compiles under test.
//!
//! The walk starts at a crate's roots (`src/lib.rs`, `src/main.rs`, `build.rs`, `src/bin/*`) and
//! follows every `mod name;` declaration by the reference's module-path rules:
//!
//! - `lib.rs`, `main.rs`, `mod.rs` and every file loaded through `#[path]` own their directory, so a
//!   child resolves beside them; any other file `foo.rs` owns `foo/`
//!   (`rustc_expand::module::mod_file_path` treats a `#[path]` file as a mod-rs file);
//! - a `#[path]` outside any inline module is relative to the declaring file's directory; inside
//!   inline modules it is relative to the owned directory extended by the inline module names.
//!
//! Along the way it records every file-embedding macro (`include_str!`, `include_bytes!`, `include!`,
//! `include_dir!`, however deeply nested in other macro arguments), every test-only `mod`, every
//! `#[test]` function, and every other test-only attribute, each with the module path it lives at and
//! whether it compiles only under test. A file reached through ANY production path is production; it
//! is test-only only when every path to it is gated.
//!
//! It also records every `cfg` gate: each file's own `#![cfg]`, the gates on each `mod` declaration it
//! makes (with those of the inline modules around it), and every gated item, statement or expression
//! inside it with the gates of the items enclosing it. A file is walked once, whatever path reached it
//! first, so the gates of every path to a file are recovered by following [`SourceFile::mods`] from the
//! roots.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::Visit;

/// Lexically normalize `a/b/../c` to `a/c` without touching the filesystem (the target may be missing,
/// and that is itself a finding).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

// ─── attributes ────────────────────────────────────────────────────────────────────────────────

/// An attribute's meta with all whitespace removed: `cfg(test)`, `cfg(all(test,feature="x"))`, `test`.
pub fn compact(meta: &syn::Meta) -> String {
    let path = |p: &syn::Path| {
        p.segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect::<Vec<_>>()
            .join("::")
    };
    let text = match meta {
        syn::Meta::Path(p) => path(p),
        syn::Meta::List(l) => format!("{}({})", path(&l.path), l.tokens),
        syn::Meta::NameValue(nv) => path(&nv.path),
    };
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `#[test]`, `#[tokio::test]`, or any attribute path ending in `test`: the item is a test function.
pub fn is_test_attr(attr: &syn::Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|s| s.ident == "test")
}

/// A cfg predicate (compacted, e.g. `all(test,not(feature="x"))`) that can only hold under `--test`:
/// `test` itself, an `all(…)` with any such argument, or an `any(…)` whose every argument is such.
pub fn predicate_implies_test(pred: &str) -> bool {
    if pred == "test" {
        return true;
    }
    if let Some(rest) = pred.strip_prefix("all(") {
        return top_level_args(rest)
            .iter()
            .any(|a| predicate_implies_test(a));
    }
    if let Some(rest) = pred.strip_prefix("any(") {
        let args = top_level_args(rest);
        return !args.is_empty() && args.iter().all(|a| predicate_implies_test(a));
    }
    false
}

/// `#[cfg(P)]` where `P` implies `test`.
pub fn is_test_cfg(attr: &syn::Attribute) -> bool {
    let s = compact(&attr.meta);
    s.strip_prefix("cfg(")
        .and_then(|r| r.strip_suffix(')'))
        .is_some_and(predicate_implies_test)
}

/// The predicate of a `#[cfg(P)]` / `#![cfg(P)]` attribute, compacted (`feature="x"`, `all(test,unix)`);
/// `None` for any other attribute.
pub fn cfg_predicate(attr: &syn::Attribute) -> Option<String> {
    let s = compact(&attr.meta);
    s.strip_prefix("cfg(")
        .and_then(|r| r.strip_suffix(')'))
        .map(str::to_string)
}

/// The item never compiles for a consumer: it is a test function or sits under a test-implying cfg.
pub fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| is_test_attr(a) || is_test_cfg(a))
}

/// The comma-separated arguments at depth zero of `a,b(c,d),e)` (the text after an opening paren).
pub fn top_level_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    let mut in_str = false;
    for ch in s.chars() {
        if in_str {
            cur.push(ch);
            if ch == '"' {
                in_str = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_str = true;
                cur.push(ch);
            }
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' if depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                return out;
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn path_attr(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|a| match &a.meta {
        syn::Meta::NameValue(nv) if nv.path.is_ident("path") => match &nv.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) => Some(s.value()),
            _ => None,
        },
        _ => None,
    })
}

// ─── what the walk records ─────────────────────────────────────────────────────────────────────

/// One file-embedding site.
#[derive(Clone, Debug)]
pub struct Embed {
    /// The file that contains the macro or attribute.
    pub from: PathBuf,
    /// The literal as written.
    pub literal: String,
    /// Where it resolves (lexically normalized).
    pub target: PathBuf,
    pub test_only: bool,
}

/// A test-only `mod` item: `#[cfg(test)] mod tests { … }` or `#[cfg(test)] #[path = "…"] mod x;`.
/// Only the outermost one is recorded; everything inside it is its content.
#[derive(Clone, Debug)]
pub struct TestMod {
    /// The file that declares it.
    pub file: PathBuf,
    /// 1-based line of the item's first token (its first attribute).
    pub line: usize,
    pub name: String,
    /// Module path from the crate root, ending in `name`.
    pub module: Vec<String>,
    /// `true` for a body written in place, `false` for a `mod x;` declaration.
    pub inline: bool,
    pub path_literal: Option<String>,
    /// The file a declaration resolves to (lexically normalized); `None` for an inline body.
    pub target: Option<PathBuf>,
    /// The directory a `#[path]` on this declaration is relative to.
    pub path_base: PathBuf,
    /// The crate root (`src/lib.rs`, `src/main.rs`, `src/bin/…`) the declaring file was reached from.
    pub root: PathBuf,
}

/// A located finding: a `#[test]` function, or a test-only attribute outside every test-only module.
#[derive(Clone, Debug)]
pub struct Located {
    pub file: PathBuf,
    pub line: usize,
    pub what: String,
}

/// A `mod name;` declaration: the file it loads and the gates between the declaring file and it.
#[derive(Clone, Debug)]
pub struct ModDecl {
    /// The file it resolves to (lexically normalized).
    pub target: PathBuf,
    /// The predicates of the inline modules enclosing the declaration and of the declaration itself,
    /// outermost first.
    pub cfg: Vec<String>,
}

/// A gated item, statement or expression inside a file.
#[derive(Clone, Debug)]
pub struct CfgSite {
    /// 1-based line of the `cfg` attribute.
    pub line: usize,
    /// Its own predicate, last, after the predicates of every item enclosing it within the file.
    pub cfg: Vec<String>,
}

/// A file the walk reached.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    /// Module path from the crate root (`[]` for a root).
    pub module: Vec<String>,
    /// Reached only through test-only declarations.
    pub test_only: bool,
    /// The crate root it was reached from.
    pub root: PathBuf,
    /// Its `mod x;` children resolve beside it (a root, `mod.rs`, or a `#[path]`-loaded file).
    pub owns_dir: bool,
    /// The predicates of its own `#![cfg]` attributes.
    pub inner_cfg: Vec<String>,
    /// Every `mod x;` declaration it makes.
    pub mods: Vec<ModDecl>,
    /// Every gated item, statement or expression in it.
    pub cfg_sites: Vec<CfgSite>,
}

#[derive(Default, Debug)]
pub struct ModuleGraph {
    pub files: Vec<SourceFile>,
    pub embeds: Vec<Embed>,
    pub test_mods: Vec<TestMod>,
    pub test_fns: Vec<Located>,
    /// Test-only attributes on anything but a `mod` item, outside every test-only module: gated
    /// helper functions, `use`s, statics, and instrumentation statements inside production functions.
    pub test_items: Vec<Located>,
}

// ─── the walk ──────────────────────────────────────────────────────────────────────────────────

/// A `mod name;` declaration to follow.
struct ChildMod {
    file: PathBuf,
    module: Vec<String>,
    test_only: bool,
    via_path: bool,
    cfg: Vec<String>,
}

struct FileVisitor<'a> {
    file: &'a Path,
    root: &'a Path,
    crate_dir: &'a Path,
    module: Vec<String>,
    owns_dir: bool,
    inline_stack: Vec<String>,
    test_depth: usize,
    file_test_only: bool,
    embeds: Vec<Embed>,
    children: Vec<ChildMod>,
    test_mods: Vec<TestMod>,
    test_fns: Vec<Located>,
    test_items: Vec<Located>,
    /// The predicates of the items and inline modules enclosing the current position, outermost first.
    cfg_stack: Vec<String>,
    cfg_sites: Vec<CfgSite>,
    /// Where each recorded `cfg` attribute starts, so an attribute the item visit recorded is not
    /// recorded again when the default visit reaches it.
    cfg_seen: std::collections::BTreeSet<(usize, usize)>,
}

fn line_of(span: proc_macro2::Span) -> usize {
    span.start().line
}

impl<'a> FileVisitor<'a> {
    fn test_only(&self) -> bool {
        self.file_test_only || self.test_depth > 0
    }

    fn file_dir(&self) -> PathBuf {
        self.file
            .parent()
            .expect("file has a directory")
            .to_path_buf()
    }

    /// The directory `mod name;` resolves against, per the reference's module-path rules.
    fn child_base(&self, via_path_attr: bool) -> PathBuf {
        let mut base = self.file_dir();
        if via_path_attr && self.inline_stack.is_empty() {
            return base;
        }
        if !self.owns_dir {
            base.push(self.file.file_stem().expect("file stem"));
        }
        for m in &self.inline_stack {
            base.push(m);
        }
        base
    }

    fn current_module(&self) -> Vec<String> {
        let mut m = self.module.clone();
        m.extend(self.inline_stack.iter().cloned());
        m
    }

    /// Record the `cfg` gates among `attrs` as one site under the enclosing gates, and return their
    /// predicates for the caller to hold over the item's contents.
    fn gate(&mut self, attrs: &[syn::Attribute]) -> Vec<String> {
        let mut preds = Vec::new();
        let mut line = 0;
        for a in attrs {
            if let Some(p) = cfg_predicate(a) {
                let start = a.span().start();
                self.cfg_seen.insert((start.line, start.column));
                if line == 0 {
                    line = start.line;
                }
                preds.push(p);
            }
        }
        if !preds.is_empty() {
            let mut cfg = self.cfg_stack.clone();
            cfg.extend(preds.iter().cloned());
            self.cfg_sites.push(CfgSite { line, cfg });
        }
        preds
    }

    fn record(&mut self, literal: String, target: PathBuf) {
        self.embeds.push(Embed {
            from: self.file.to_path_buf(),
            literal,
            target: normalize(&target),
            test_only: self.test_only(),
        });
    }

    /// Every `include_str!`/`include_bytes!`/`include!`/`include_dir!` in a token stream, however deeply
    /// it is nested inside other macro arguments.
    fn scan_tokens(&mut self, tokens: &TokenStream) {
        let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
        for i in 0..trees.len() {
            if let TokenTree::Group(g) = &trees[i] {
                self.scan_tokens(&g.stream());
            }
            let TokenTree::Ident(ident) = &trees[i] else {
                continue;
            };
            let name = ident.to_string();
            if !matches!(
                name.as_str(),
                "include_str" | "include_bytes" | "include" | "include_dir"
            ) {
                continue;
            }
            let (Some(TokenTree::Punct(bang)), Some(TokenTree::Group(args))) =
                (trees.get(i + 1), trees.get(i + 2))
            else {
                continue;
            };
            if bang.as_char() == '!' {
                self.embed_args(&args.stream());
            }
        }
    }

    /// The argument of one embed macro: a string literal, `$CARGO_MANIFEST_DIR/…` (include_dir), or
    /// `concat!(env!("CARGO_MANIFEST_DIR"), "…")`. `OUT_DIR` embeds are generated, not shipped.
    fn embed_args(&mut self, args: &TokenStream) {
        let trees: Vec<TokenTree> = args.clone().into_iter().collect();
        match trees.first() {
            Some(TokenTree::Literal(lit)) => {
                let Ok(s) = syn::parse_str::<syn::LitStr>(&lit.to_string()) else {
                    return;
                };
                let value = s.value();
                let target = match value.strip_prefix("$CARGO_MANIFEST_DIR") {
                    Some(rest) => self.crate_dir.join(rest.trim_start_matches('/')),
                    None => self.file_dir().join(&value),
                };
                self.record(value, target);
            }
            Some(TokenTree::Ident(id)) if id == "concat" => {
                let Some(TokenTree::Group(inner)) = trees.get(2) else {
                    return;
                };
                let text: String = inner.stream().to_string();
                let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
                if compact.starts_with("env!(\"CARGO_MANIFEST_DIR\"),") {
                    let rest = &compact["env!(\"CARGO_MANIFEST_DIR\"),".len()..];
                    if let Ok(s) = syn::parse_str::<syn::LitStr>(rest) {
                        let value = s.value();
                        let target = self.crate_dir.join(value.trim_start_matches('/'));
                        self.record(
                            format!("concat!(env!(\"CARGO_MANIFEST_DIR\"), \"{value}\")"),
                            target,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    fn note_test_fn(&mut self, attrs: &[syn::Attribute], name: &syn::Ident) {
        if attrs.iter().any(is_test_attr) {
            self.test_fns.push(Located {
                file: self.file.to_path_buf(),
                line: line_of(name.span()),
                what: name.to_string(),
            });
        }
    }

    fn resolve_child(&self, m: &syn::ItemMod) -> (PathBuf, bool) {
        match path_attr(&m.attrs) {
            Some(p) => (normalize(&self.child_base(true).join(p)), true),
            None => {
                let base = self.child_base(false);
                let flat = base.join(format!("{}.rs", m.ident));
                let file = if flat.exists() {
                    flat
                } else {
                    base.join(m.ident.to_string()).join("mod.rs")
                };
                (normalize(&file), false)
            }
        }
    }
}

fn item_attrs(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(i) => &i.attrs,
        syn::Item::Enum(i) => &i.attrs,
        syn::Item::ExternCrate(i) => &i.attrs,
        syn::Item::Fn(i) => &i.attrs,
        syn::Item::ForeignMod(i) => &i.attrs,
        syn::Item::Impl(i) => &i.attrs,
        syn::Item::Macro(i) => &i.attrs,
        syn::Item::Mod(i) => &i.attrs,
        syn::Item::Static(i) => &i.attrs,
        syn::Item::Struct(i) => &i.attrs,
        syn::Item::Trait(i) => &i.attrs,
        syn::Item::TraitAlias(i) => &i.attrs,
        syn::Item::Type(i) => &i.attrs,
        syn::Item::Union(i) => &i.attrs,
        syn::Item::Use(i) => &i.attrs,
        _ => &[],
    }
}

impl<'a, 'ast> Visit<'ast> for FileVisitor<'a> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let attrs = item_attrs(item);
        let gated = is_test_only(attrs);
        let preds = self.gate(attrs);
        let held = preds.len();
        self.cfg_stack.extend(preds);
        self.visit_item_gated(item, attrs, gated);
        self.cfg_stack.truncate(self.cfg_stack.len() - held);
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        let preds = self.gate(&f.attrs);
        let held = preds.len();
        self.cfg_stack.extend(preds);
        self.note_test_fn(&f.attrs, &f.sig.ident);
        let gated = is_test_only(&f.attrs);
        for a in &f.attrs {
            self.visit_attribute(a);
        }
        if gated {
            self.test_depth += 1;
        }
        syn::visit::visit_impl_item_fn(self, f);
        if gated {
            self.test_depth -= 1;
        }
        self.cfg_stack.truncate(self.cfg_stack.len() - held);
    }

    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        if self.test_depth == 0 && !self.file_test_only && is_test_cfg(attr) {
            let already = self
                .test_items
                .iter()
                .any(|l| l.line == line_of(attr.span()) && l.file == self.file);
            if !already {
                self.test_items.push(Located {
                    file: self.file.to_path_buf(),
                    line: line_of(attr.span()),
                    what: compact(&attr.meta),
                });
            }
        }
        if let Some(p) = cfg_predicate(attr) {
            let start = attr.span().start();
            if self.cfg_seen.insert((start.line, start.column))
                && !matches!(attr.style, syn::AttrStyle::Inner(_))
            {
                let mut cfg = self.cfg_stack.clone();
                cfg.push(p);
                self.cfg_sites.push(CfgSite {
                    line: start.line,
                    cfg,
                });
            }
        }
        syn::visit::visit_attribute(self, attr);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let name = mac
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if matches!(
            name.as_str(),
            "include_str" | "include_bytes" | "include" | "include_dir"
        ) {
            self.embed_args(&mac.tokens);
        }
        self.scan_tokens(&mac.tokens);
    }
}

impl<'a> FileVisitor<'a> {
    /// The item visit proper, with the item's own `cfg` gates already held on the stack.
    fn visit_item_gated<'ast>(
        &mut self,
        item: &'ast syn::Item,
        attrs: &'ast [syn::Attribute],
        gated: bool,
    ) {
        if let syn::Item::Mod(m) = item {
            if gated && self.test_depth == 0 && !self.file_test_only {
                let mut module = self.current_module();
                module.push(m.ident.to_string());
                let (target, inline) = match &m.content {
                    Some(_) => (None, true),
                    None => (Some(self.resolve_child(m).0), false),
                };
                self.test_mods.push(TestMod {
                    file: self.file.to_path_buf(),
                    line: line_of(m.span()),
                    name: m.ident.to_string(),
                    module,
                    inline,
                    path_literal: path_attr(&m.attrs),
                    target,
                    path_base: self.child_base(true),
                    root: self.root.to_path_buf(),
                });
            }
            if gated {
                self.test_depth += 1;
            }
            match &m.content {
                None => {
                    let (file, via_path) = self.resolve_child(m);
                    let mut module = self.current_module();
                    module.push(m.ident.to_string());
                    self.children.push(ChildMod {
                        file,
                        module,
                        test_only: self.test_only(),
                        via_path,
                        cfg: self.cfg_stack.clone(),
                    });
                    for a in &m.attrs {
                        self.visit_attribute(a);
                    }
                }
                Some(_) => {
                    self.inline_stack.push(m.ident.to_string());
                    syn::visit::visit_item_mod(self, m);
                    self.inline_stack.pop();
                }
            }
            if gated {
                self.test_depth -= 1;
            }
            return;
        }

        if let syn::Item::Fn(f) = item {
            self.note_test_fn(&f.attrs, &f.sig.ident);
        }
        // The item's own attributes are recorded (at the depth outside it); its body is test scope.
        for a in attrs {
            self.visit_attribute(a);
        }
        if gated {
            self.test_depth += 1;
        }
        syn::visit::visit_item(self, item);
        if gated {
            self.test_depth -= 1;
        }
    }
}

/// A crate's root files: `src/lib.rs`, `src/main.rs`, `build.rs`, and every `src/bin/*` target.
pub fn roots(crate_dir: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for root in ["src/lib.rs", "src/main.rs", "build.rs"] {
        let p = crate_dir.join(root);
        if p.exists() {
            roots.push(p);
        }
    }
    if let Ok(bins) = std::fs::read_dir(crate_dir.join("src/bin")) {
        let mut bins: Vec<PathBuf> = bins.flatten().map(|e| e.path()).collect();
        bins.sort();
        for p in bins {
            if p.extension().is_some_and(|x| x == "rs") {
                roots.push(p);
            } else if p.join("main.rs").exists() {
                roots.push(p.join("main.rs"));
            }
        }
    }
    roots
}

/// Walk a crate from its roots. Panics when a declared module file cannot be read or parsed: a graph
/// with a silent hole would let every lock built on it pass on less than the whole crate.
pub fn walk(crate_dir: &Path) -> ModuleGraph {
    let roots = roots(crate_dir);
    assert!(
        !roots.is_empty(),
        "{} has no src/lib.rs, src/main.rs, build.rs or src/bin target",
        crate_dir.display()
    );
    walk_roots(crate_dir, roots)
}

/// Walk from the given root files (a crate root, or an integration-test binary's root file).
pub fn walk_roots(crate_dir: &Path, roots: Vec<PathBuf>) -> ModuleGraph {
    // (file, module path, test_only, via_path, root). Production edges pop first, so a file reached both
    // ways is recorded as production. A crate root owns its directory, as a `#[path]` file does.
    let mut queue: Vec<(PathBuf, Vec<String>, bool, bool, PathBuf)> = roots
        .into_iter()
        .rev()
        .map(|r| (r.clone(), Vec::new(), false, true, r))
        .collect();
    let mut visited: BTreeMap<PathBuf, bool> = BTreeMap::new();
    let mut per_file: BTreeMap<PathBuf, (SourceFile, FileRecord)> = BTreeMap::new();

    while let Some((file, module, test_only, via_path, root)) = queue.pop() {
        match visited.get(&file) {
            Some(&was_test) if !(was_test && !test_only) => continue,
            _ => {}
        }
        visited.insert(file.clone(), test_only);
        let src = std::fs::read_to_string(&file).unwrap_or_else(|e| {
            panic!(
                "{}: a `mod` declaration resolves to {} which cannot be read: {e}",
                crate_dir.display(),
                file.display()
            )
        });
        let ast = syn::parse_file(&src)
            .unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()));
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let owns_dir =
            via_path || matches!(name.as_str(), "mod.rs" | "lib.rs" | "main.rs" | "build.rs");
        let mut v = FileVisitor {
            file: &file,
            root: &root,
            crate_dir,
            module: module.clone(),
            owns_dir,
            inline_stack: Vec::new(),
            test_depth: 0,
            file_test_only: test_only,
            embeds: Vec::new(),
            children: Vec::new(),
            test_mods: Vec::new(),
            test_fns: Vec::new(),
            test_items: Vec::new(),
            cfg_stack: Vec::new(),
            cfg_sites: Vec::new(),
            cfg_seen: std::collections::BTreeSet::new(),
        };
        v.visit_file(&ast);
        let inner_cfg: Vec<String> = ast
            .attrs
            .iter()
            .filter(|a| matches!(a.style, syn::AttrStyle::Inner(_)))
            .filter_map(cfg_predicate)
            .collect();
        let record = FileRecord {
            embeds: v.embeds,
            test_mods: v.test_mods,
            test_fns: v.test_fns,
            test_items: v.test_items,
        };
        let mods = v
            .children
            .iter()
            .map(|c| ModDecl {
                target: c.file.clone(),
                cfg: c.cfg.clone(),
            })
            .collect();
        for child in v.children {
            if child.test_only {
                queue.insert(
                    0,
                    (child.file, child.module, true, child.via_path, root.clone()),
                );
            } else {
                queue.push((
                    child.file,
                    child.module,
                    false,
                    child.via_path,
                    root.clone(),
                ));
            }
        }
        let cfg_sites = v.cfg_sites;
        per_file.insert(
            file.clone(),
            (
                SourceFile {
                    path: file,
                    module,
                    test_only,
                    root,
                    owns_dir,
                    inner_cfg,
                    mods,
                    cfg_sites,
                },
                record,
            ),
        );
    }

    let mut graph = ModuleGraph::default();
    for (_, (file, record)) in per_file {
        graph.files.push(file);
        graph.embeds.extend(record.embeds);
        graph.test_mods.extend(record.test_mods);
        graph.test_fns.extend(record.test_fns);
        graph.test_items.extend(record.test_items);
    }
    graph
}

struct FileRecord {
    embeds: Vec<Embed>,
    test_mods: Vec<TestMod>,
    test_fns: Vec<Located>,
    test_items: Vec<Located>,
}

/// Every `#[test]` function in one file, found by parsing it alone (for files no walk reaches).
pub fn test_fns_in_file(file: &Path) -> Vec<Located> {
    let Ok(src) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(ast) = syn::parse_file(&src) else {
        return Vec::new();
    };
    struct V<'a> {
        file: &'a Path,
        out: Vec<Located>,
    }
    impl<'a, 'ast> Visit<'ast> for V<'a> {
        fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
            if f.attrs.iter().any(is_test_attr) {
                self.out.push(Located {
                    file: self.file.to_path_buf(),
                    line: line_of(f.sig.ident.span()),
                    what: f.sig.ident.to_string(),
                });
            }
            syn::visit::visit_item_fn(self, f);
        }
        fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
            if f.attrs.iter().any(is_test_attr) {
                self.out.push(Located {
                    file: self.file.to_path_buf(),
                    line: line_of(f.sig.ident.span()),
                    what: f.sig.ident.to_string(),
                });
            }
            syn::visit::visit_impl_item_fn(self, f);
        }
    }
    let mut v = V {
        file,
        out: Vec::new(),
    };
    v.visit_file(&ast);
    v.out
}
