//! ★ LOCK — the workspace, test suite included, builds on x86_64 and on Apple Silicon (aarch64). ★
//!
//! An architecture-only SIMD feature may be enabled only in a dependency table that compiles for
//! that architecture alone. libcrux's `simd256` is AVX2 (x86_64) and its `simd128` is NEON
//! (aarch64). Enabled in a table every target reads, the crate's intrinsics fail to compile on the
//! other architecture, which is how the whole test crate stopped building on the Mac mini
//! (libcrux-intrinsics, 108 errors, 2026-11-19). Each target now enables its own backend, and
//! libcrux picks the fastest compiled-in one at run time.
//!
//! A test names the native tier's crates (`logicaffeine_jit`, `logicaffeine_forge`) only under a gate
//! that compiles where they exist. Both crates are empty outside x86_64 System V, so an ungated
//! `use logicaffeine_jit::ForgeTier` failed 911 test targets on the Mac mini (2026-11-19), most of
//! them through the shared helpers every suite includes. A file about the JIT alone opens with the
//! crates' own gate. A file that also tests other tiers gates only the items that name the JIT, and
//! hands the VM no native tier elsewhere, so its other tests still run there.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test portable_suite_lock`

#[path = "common/module_graph.rs"]
mod module_graph;

use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

use module_graph::{cfg_predicate, top_level_args};

/// The SIMD features that exist on one architecture only, and the `cfg` table they belong under.
const ARCH_ONLY: &[(&str, &str)] = &[
    ("simd256", r#"cfg(target_arch = "x86_64")"#),
    ("simd128", r#"cfg(target_arch = "aarch64")"#),
];

const DEPENDENCY_TABLES: &[&str] = &["dependencies", "dev-dependencies", "build-dependencies"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("repo root")
}

/// Every Cargo.toml the workspace builds: the root, each crate and each app.
fn manifests() -> Vec<PathBuf> {
    let root = repo_root();
    let mut found = vec![root.join("Cargo.toml")];
    for dir in ["crates", "apps"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else { continue };
        for entry in entries.flatten() {
            let manifest = entry.path().join("Cargo.toml");
            if manifest.is_file() {
                found.push(manifest);
            }
        }
    }
    found.sort();
    found
}

/// Each dependency table of `manifest` with the `cfg` that gates it (`None` for an ungated table).
fn tables(manifest: &toml::Value) -> Vec<(Option<String>, &toml::value::Table)> {
    let mut out = Vec::new();
    for name in DEPENDENCY_TABLES {
        if let Some(table) = manifest.get(*name).and_then(toml::Value::as_table) {
            out.push((None, table));
        }
    }
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (cfg, body) in targets {
            for name in DEPENDENCY_TABLES {
                if let Some(table) = body.get(*name).and_then(toml::Value::as_table) {
                    out.push((Some(cfg.clone()), table));
                }
            }
        }
    }
    out
}

fn features(spec: &toml::Value) -> Vec<String> {
    spec.get("features")
        .and_then(toml::Value::as_array)
        .map(|list| list.iter().filter_map(|f| f.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

#[test]
fn an_architecture_only_simd_feature_is_enabled_only_for_its_architecture() {
    let mut wrong = Vec::new();
    for path in manifests() {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let manifest: toml::Value = toml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        for (cfg, table) in tables(&manifest) {
            for (dep, spec) in table {
                for feature in features(spec) {
                    let Some((_, needed)) = ARCH_ONLY.iter().find(|(f, _)| *f == feature) else { continue };
                    if cfg.as_deref() != Some(*needed) {
                        wrong.push(format!(
                            "{}: `{dep}` enables `{feature}` under {}, but it compiles only under [target.'{needed}']",
                            path.strip_prefix(repo_root()).unwrap_or(&path).display(),
                            cfg.as_deref().map(|c| format!("[target.'{c}']")).unwrap_or_else(|| "an ungated table".to_string()),
                        ));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(), "architecture-only SIMD features outside their architecture:\n{}", wrong.join("\n"));
}

#[test]
fn the_test_crate_checks_its_crypto_bars_on_both_architectures() {
    let text = std::fs::read_to_string(repo_root().join("crates/logicaffeine_tests/Cargo.toml")).expect("read the test crate manifest");
    let manifest: toml::Value = toml::from_str(&text).expect("parse the test crate manifest");
    let targets = manifest.get("target").and_then(toml::Value::as_table).expect("the test crate has target tables");
    for (cfg, feature) in [(r#"cfg(target_arch = "x86_64")"#, "simd256"), (r#"cfg(target_arch = "aarch64")"#, "simd128")] {
        let dev = targets.get(cfg).and_then(|t| t.get("dev-dependencies")).and_then(toml::Value::as_table);
        let dev = dev.unwrap_or_else(|| panic!("no [target.'{cfg}'.dev-dependencies] in the test crate"));
        for crate_name in ["libcrux-ml-kem", "libcrux-ml-dsa"] {
            let spec = dev.get(crate_name).unwrap_or_else(|| panic!("{crate_name} is missing under {cfg}"));
            assert!(features(spec).iter().any(|f| f == feature), "{crate_name} under {cfg} does not enable {feature}");
        }
    }
}

// ─── the native tier ───────────────────────────────────────────────────────────────────────────

/// The crates of the native tier: each is empty wherever its own `#![cfg]` is false.
const NATIVE_TIER_CRATES: &[&str] = &["logicaffeine_jit", "logicaffeine_forge"];

/// The gate both crates compile under, which is the gate a test that names them uses.
const NATIVE_TIER_GATE: &str = r#"cfg(all(target_arch = "x86_64", not(target_os = "windows")))"#;

/// A target the suite builds for, by the cfg atoms a gate can test.
struct Target {
    triple: &'static str,
    arch: &'static str,
    os: &'static str,
    family: &'static str,
}

/// The targets the suite builds for where the native tier's crates are empty.
const WITHOUT_THE_NATIVE_TIER: &[Target] = &[
    Target { triple: "aarch64-apple-darwin", arch: "aarch64", os: "macos", family: "unix" },
    Target { triple: "aarch64-unknown-linux-gnu", arch: "aarch64", os: "linux", family: "unix" },
    Target { triple: "x86_64-pc-windows-msvc", arch: "x86_64", os: "windows", family: "windows" },
    Target { triple: "wasm32-unknown-unknown", arch: "wasm32", os: "unknown", family: "wasm" },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Truth {
    Yes,
    No,
    Unknown,
}

/// A compacted cfg predicate on `target`: the target's own atoms decide, `test` holds, and anything
/// else (a feature, `debug_assertions`) may go either way.
fn eval(pred: &str, target: &Target) -> Truth {
    let of = |rest: &str| top_level_args(rest).iter().map(|a| eval(a, target)).collect::<Vec<_>>();
    if let Some(rest) = pred.strip_prefix("all(") {
        let args = of(rest);
        return if args.contains(&Truth::No) {
            Truth::No
        } else if args.iter().all(|t| *t == Truth::Yes) {
            Truth::Yes
        } else {
            Truth::Unknown
        };
    }
    if let Some(rest) = pred.strip_prefix("any(") {
        let args = of(rest);
        return if args.contains(&Truth::Yes) {
            Truth::Yes
        } else if args.iter().all(|t| *t == Truth::No) {
            Truth::No
        } else {
            Truth::Unknown
        };
    }
    if let Some(rest) = pred.strip_prefix("not(") {
        let args = of(rest);
        assert_eq!(args.len(), 1, "not() takes one predicate: {pred}");
        return match args[0] {
            Truth::Yes => Truth::No,
            Truth::No => Truth::Yes,
            Truth::Unknown => Truth::Unknown,
        };
    }
    let is = |held: bool| if held { Truth::Yes } else { Truth::No };
    let value = |key: &str| pred.strip_prefix(key).and_then(|v| v.strip_prefix('=')).map(|v| v.trim_matches('"'));
    if let Some(arch) = value("target_arch") {
        return is(arch == target.arch);
    }
    if let Some(os) = value("target_os") {
        return is(os == target.os);
    }
    if let Some(family) = value("target_family") {
        return is(family == target.family);
    }
    match pred {
        "unix" => is(target.family == "unix"),
        "windows" => is(target.family == "windows"),
        "test" => Truth::Yes,
        _ => Truth::Unknown,
    }
}

/// `#[cfg(P)]` where `P` is false on every target without the native tier.
fn fences_the_native_tier(attr: &syn::Attribute) -> bool {
    cfg_predicate(attr).is_some_and(|pred| WITHOUT_THE_NATIVE_TIER.iter().all(|t| eval(&pred, t) == Truth::No))
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

fn impl_item_attrs(item: &syn::ImplItem) -> &[syn::Attribute] {
    match item {
        syn::ImplItem::Const(i) => &i.attrs,
        syn::ImplItem::Fn(i) => &i.attrs,
        syn::ImplItem::Type(i) => &i.attrs,
        syn::ImplItem::Macro(i) => &i.attrs,
        _ => &[],
    }
}

fn trait_item_attrs(item: &syn::TraitItem) -> &[syn::Attribute] {
    match item {
        syn::TraitItem::Const(i) => &i.attrs,
        syn::TraitItem::Fn(i) => &i.attrs,
        syn::TraitItem::Type(i) => &i.attrs,
        syn::TraitItem::Macro(i) => &i.attrs,
        _ => &[],
    }
}

fn expr_attrs(expr: &syn::Expr) -> &[syn::Attribute] {
    match expr {
        syn::Expr::Array(e) => &e.attrs,
        syn::Expr::Assign(e) => &e.attrs,
        syn::Expr::Async(e) => &e.attrs,
        syn::Expr::Await(e) => &e.attrs,
        syn::Expr::Binary(e) => &e.attrs,
        syn::Expr::Block(e) => &e.attrs,
        syn::Expr::Call(e) => &e.attrs,
        syn::Expr::Cast(e) => &e.attrs,
        syn::Expr::Closure(e) => &e.attrs,
        syn::Expr::Field(e) => &e.attrs,
        syn::Expr::ForLoop(e) => &e.attrs,
        syn::Expr::If(e) => &e.attrs,
        syn::Expr::Index(e) => &e.attrs,
        syn::Expr::Let(e) => &e.attrs,
        syn::Expr::Lit(e) => &e.attrs,
        syn::Expr::Loop(e) => &e.attrs,
        syn::Expr::Macro(e) => &e.attrs,
        syn::Expr::Match(e) => &e.attrs,
        syn::Expr::MethodCall(e) => &e.attrs,
        syn::Expr::Paren(e) => &e.attrs,
        syn::Expr::Path(e) => &e.attrs,
        syn::Expr::Range(e) => &e.attrs,
        syn::Expr::Reference(e) => &e.attrs,
        syn::Expr::Return(e) => &e.attrs,
        syn::Expr::Struct(e) => &e.attrs,
        syn::Expr::Try(e) => &e.attrs,
        syn::Expr::Tuple(e) => &e.attrs,
        syn::Expr::Unary(e) => &e.attrs,
        syn::Expr::Unsafe(e) => &e.attrs,
        syn::Expr::While(e) => &e.attrs,
        _ => &[],
    }
}

/// Every place a file names a native-tier crate outside a gate that fences it, by line.
#[derive(Default)]
struct Unfenced {
    fences: usize,
    found: Vec<(usize, String)>,
}

impl Unfenced {
    fn within(&mut self, attrs: &[syn::Attribute], walk: impl FnOnce(&mut Self)) {
        let fenced = usize::from(attrs.iter().any(fences_the_native_tier));
        self.fences += fenced;
        walk(self);
        self.fences -= fenced;
    }

    fn named(&mut self, ident: &proc_macro2::Ident) {
        if self.fences == 0 && NATIVE_TIER_CRATES.iter().any(|c| ident == c) {
            self.found.push((ident.span().start().line, ident.to_string()));
        }
    }

    fn tokens(&mut self, tokens: TokenStream) {
        for tree in tokens {
            match tree {
                TokenTree::Ident(ident) => self.named(&ident),
                TokenTree::Group(group) => self.tokens(group.stream()),
                _ => {}
            }
        }
    }

    fn use_roots(&mut self, tree: &syn::UseTree) {
        match tree {
            syn::UseTree::Path(p) => self.named(&p.ident),
            syn::UseTree::Name(n) => self.named(&n.ident),
            syn::UseTree::Rename(r) => self.named(&r.ident),
            syn::UseTree::Group(g) => g.items.iter().for_each(|t| self.use_roots(t)),
            syn::UseTree::Glob(_) => {}
        }
    }
}

impl<'ast> Visit<'ast> for Unfenced {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        self.within(item_attrs(item), |v| syn::visit::visit_item(v, item));
    }
    fn visit_impl_item(&mut self, item: &'ast syn::ImplItem) {
        self.within(impl_item_attrs(item), |v| syn::visit::visit_impl_item(v, item));
    }
    fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
        self.within(trait_item_attrs(item), |v| syn::visit::visit_trait_item(v, item));
    }
    fn visit_expr(&mut self, expr: &'ast syn::Expr) {
        self.within(expr_attrs(expr), |v| syn::visit::visit_expr(v, expr));
    }
    fn visit_local(&mut self, local: &'ast syn::Local) {
        self.within(&local.attrs, |v| syn::visit::visit_local(v, local));
    }
    fn visit_stmt_macro(&mut self, stmt: &'ast syn::StmtMacro) {
        self.within(&stmt.attrs, |v| syn::visit::visit_stmt_macro(v, stmt));
    }
    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        self.within(&arm.attrs, |v| syn::visit::visit_arm(v, arm));
    }
    fn visit_field(&mut self, field: &'ast syn::Field) {
        self.within(&field.attrs, |v| syn::visit::visit_field(v, field));
    }
    fn visit_field_value(&mut self, field: &'ast syn::FieldValue) {
        self.within(&field.attrs, |v| syn::visit::visit_field_value(v, field));
    }
    fn visit_variant(&mut self, variant: &'ast syn::Variant) {
        self.within(&variant.attrs, |v| syn::visit::visit_variant(v, variant));
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.use_roots(&item.tree);
    }
    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        self.named(&item.ident);
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(first) = path.segments.first() {
            self.named(&first.ident);
        }
        syn::visit::visit_path(self, path);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}

/// Where `source` names a native-tier crate outside a gate that fences it, by line.
fn unfenced_native_tier_uses(source: &str) -> Result<Vec<(usize, String)>, String> {
    let file = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut walk = Unfenced::default();
    walk.within(&file.attrs, |v| v.visit_file(&file));
    Ok(walk.found)
}

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// Every test source of the suite: the integration suites with their shared modules, and the unit-test
/// bodies of every crate but the native tier's own, which compile inside those crates' gate.
fn test_sources() -> Vec<PathBuf> {
    let tests_crate = repo_root().join("crates/logicaffeine_tests");
    let mut files = Vec::new();
    rust_files(&tests_crate.join("tests"), &mut files);
    if let Ok(units) = std::fs::read_dir(tests_crate.join("unit")) {
        for unit in units.flatten() {
            if !NATIVE_TIER_CRATES.iter().any(|c| unit.file_name() == *c) {
                rust_files(&unit.path(), &mut files);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn a_test_names_the_native_tier_only_where_it_exists() {
    let root = repo_root();
    let mut wrong = Vec::new();
    for path in test_sources() {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        if !NATIVE_TIER_CRATES.iter().any(|c| text.contains(c)) {
            continue;
        }
        let shown = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
        match unfenced_native_tier_uses(&text) {
            Ok(found) => wrong.extend(found.into_iter().map(|(line, krate)| format!("{shown}:{line}: `{krate}`"))),
            Err(why) => wrong.push(format!("{shown}: does not parse ({why}), so its gates cannot be read")),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} places name a native-tier crate where it is empty ({}):\n  {}\n\n\
         A file about the JIT alone opens with `#![{NATIVE_TIER_GATE}]`. A file that also tests other tiers puts \
         `#[{NATIVE_TIER_GATE}]` on each use, item, statement or match arm that names the JIT, and hands the VM \
         `None` for the native tier under the negated gate, so the rest of it still runs there.",
        wrong.len(),
        WITHOUT_THE_NATIVE_TIER.iter().map(|t| t.triple).collect::<Vec<_>>().join(", "),
        wrong.join("\n  "),
    );
}

/// The lock reads a gate the way rustc would on each target, and finds every kind of use.
#[test]
fn the_native_tier_fence_reads_gates_as_rustc_does() {
    let attr = |text: &str| {
        syn::parse_str::<syn::DeriveInput>(&format!("{text} struct S;")).expect("an attribute").attrs.remove(0)
    };
    for fenced in [
        format!("#[{NATIVE_TIER_GATE}]"),
        r#"#[cfg(all(feature = "codegen", target_arch = "x86_64", unix))]"#.to_string(),
    ] {
        assert!(fences_the_native_tier(&attr(&fenced)), "{fenced} compiles only where the native tier exists");
    }
    for open in [
        r#"#[cfg(not(target_arch = "wasm32"))]"#,
        r#"#[cfg(all(target_arch = "x86_64", not(target_arch = "wasm32")))]"#,
        r#"#[cfg(any(target_arch = "x86_64", feature = "codegen"))]"#,
        r#"#[cfg(feature = "codegen")]"#,
    ] {
        assert!(!fences_the_native_tier(&attr(open)), "{open} compiles on a target without the native tier");
    }

    let ungated = "use logicaffeine_jit::ForgeTier;\n\
                   fn f() { let _ = logicaffeine_forge::jit::reference_eval; }\n\
                   fn g() { assert!(logicaffeine_jit::REGION_RUNS); }\n";
    let lines: Vec<usize> = unfenced_native_tier_uses(ungated).expect("parses").iter().map(|(l, _)| *l).collect();
    assert_eq!(lines, [1, 2, 3], "a use, a path and a macro argument each name the crate");

    let gated = format!(
        "#[{g}]\nuse logicaffeine_jit::ForgeTier;\n\
         fn f() {{\n    #[{g}]\n    let tier = logicaffeine_jit::ForgeTier::new();\n    \
         match 0 {{\n        #[{g}]\n        0 => assert!(logicaffeine_jit::REGION_RUNS),\n        _ => {{}}\n    }}\n}}\n",
        g = NATIVE_TIER_GATE
    );
    assert!(unfenced_native_tier_uses(&gated).expect("parses").is_empty(), "a gated use, statement and arm are fenced");

    let file_gated = format!("#![{NATIVE_TIER_GATE}]\nuse logicaffeine_jit::ForgeTier;\n");
    assert!(unfenced_native_tier_uses(&file_gated).expect("parses").is_empty(), "a file gate fences the whole file");
}
