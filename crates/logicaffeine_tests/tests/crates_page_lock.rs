//! The site's `/crates` page describes each crate in its manifest's own words.
//!
//! crates.io, docs.rs and `cargo search` show a crate's `Cargo.toml` `description`; the page at
//! `logicaffeine.com/crates` (`apps/logicaffeine_web/src/ui/pages/crates.rs`) shows its own table. The table
//! lists exactly the published crates plus the web app, and each entry's `description` is its manifest's
//! byte for byte, so the one sentence per crate is written once, in the manifest, and read everywhere. The
//! `/crates` section of `apps/logicaffeine_web/public/llms.txt`, the page's summary for language models, lists the
//! same published crates as `- <package>: <manifest description>` lines.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test crates_page_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::path::PathBuf;

const PAGE: &str = "apps/logicaffeine_web/src/ui/pages/crates.rs";
const LLMS: &str = "apps/logicaffeine_web/public/llms.txt";
const TABLES: &[&str] = &["CORE_CRATES", "VERIFY_CRATES", "APP_CRATES"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// The string literal a struct-expression field holds.
fn field(
    fields: &syn::punctuated::Punctuated<syn::FieldValue, syn::Token![,]>,
    name: &str,
) -> Option<String> {
    fields.iter().find_map(|f| match (&f.member, &f.expr) {
        (
            syn::Member::Named(id),
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }),
        ) if id == name => Some(s.value()),
        _ => None,
    })
}

/// (directory name, description) for every entry of the page's crate tables.
fn page_entries() -> BTreeMap<String, String> {
    let file =
        syn::parse_file(&read(PAGE)).unwrap_or_else(|e| panic!("{PAGE} does not parse: {e}"));
    let mut out = BTreeMap::new();
    let mut seen = 0;
    for item in &file.items {
        let syn::Item::Const(c) = item else { continue };
        if !TABLES.iter().any(|t| c.ident == t) {
            continue;
        }
        seen += 1;
        let syn::Expr::Reference(r) = &*c.expr else {
            panic!("`{}` is not a slice literal", c.ident)
        };
        let syn::Expr::Array(array) = &*r.expr else {
            panic!("`{}` is not a slice literal", c.ident)
        };
        for elem in &array.elems {
            let syn::Expr::Struct(s) = elem else {
                panic!("`{}` holds a non-struct entry", c.ident)
            };
            let name = field(&s.fields, "name")
                .unwrap_or_else(|| panic!("an entry of `{}` has no literal name", c.ident));
            let description = field(&s.fields, "description")
                .unwrap_or_else(|| panic!("`{name}` has no literal description"));
            assert!(
                out.insert(name.clone(), description).is_none(),
                "`{name}` is listed twice on the page"
            );
        }
    }
    assert_eq!(seen, TABLES.len(), "the page's crate tables are {TABLES:?}");
    out
}

struct Manifest {
    package: String,
    description: String,
    publish: bool,
}

/// Every crate and app manifest, by directory name.
fn manifests() -> BTreeMap<String, Manifest> {
    let mut out = BTreeMap::new();
    for parent in ["crates", "apps"] {
        for entry in std::fs::read_dir(repo_root().join(parent))
            .unwrap()
            .flatten()
        {
            let path = entry.path().join("Cargo.toml");
            if !path.is_file() {
                continue;
            }
            let manifest: toml::Value = std::fs::read_to_string(&path).unwrap().parse().unwrap();
            let Some(package) = manifest.get("package") else {
                continue;
            };
            out.insert(
                entry.file_name().to_string_lossy().to_string(),
                Manifest {
                    package: package
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    description: package
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    publish: package
                        .get("publish")
                        .and_then(|p| p.as_bool())
                        .unwrap_or(true),
                },
            );
        }
    }
    out
}

#[test]
fn the_page_lists_the_published_crates_and_the_web_app() {
    let page: Vec<String> = page_entries().into_keys().collect();
    let mut want: Vec<String> = manifests()
        .into_iter()
        .filter(|(_, m)| m.publish)
        .map(|(dir, _)| dir)
        .collect();
    want.push("logicaffeine_web".to_string());
    want.sort();
    assert_eq!(
        page, want,
        "`{PAGE}` lists {page:?}; the published crates plus the web app are {want:?}"
    );
}

#[test]
fn every_page_description_is_the_manifest_description() {
    let manifests = manifests();
    let drift: Vec<String> = page_entries()
        .into_iter()
        .filter_map(|(dir, text)| {
            let manifest = manifests.get(&dir)?;
            (manifest.description != text).then(|| {
                format!(
                    "{dir}:\n  page:     {text}\n  manifest: {}",
                    manifest.description
                )
            })
        })
        .collect();
    assert!(
        drift.is_empty(),
        "`{PAGE}` descriptions differ from the manifests':\n{}",
        drift.join("\n")
    );
}

#[test]
fn llms_txt_lists_the_published_crates_in_their_manifests_words() {
    let text = read(LLMS);
    let start = text
        .find("\n### /crates\n")
        .unwrap_or_else(|| panic!("`{LLMS}` has no `### /crates` section"));
    let body = &text[start + "\n### /crates\n".len()..];
    let body = &body[..body.find("\n### ").unwrap_or(body.len())];
    let listed: Vec<String> = body
        .lines()
        .filter(|l| l.starts_with("- "))
        .map(str::to_string)
        .collect();
    let mut want: Vec<String> = manifests()
        .into_values()
        .filter(|m| m.publish)
        .map(|m| format!("- {}: {}", m.package, m.description))
        .collect();
    want.sort();
    let mut got = listed.clone();
    got.sort();
    assert_eq!(got, want, "`{LLMS}`'s /crates section must list every published crate as `- <package>: <manifest description>`");
}
