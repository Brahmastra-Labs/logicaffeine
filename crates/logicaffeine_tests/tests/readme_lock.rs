//! README ratchet — every crate README is a documented API surface, and this
//! lock keeps it true: links that survive crates.io, feature tables that match
//! `[features]`, every `pub mod` accounted for, fences that are honest about
//! what compiles, and version claims that track the workspace.
//!
//! Every README also has one shape, so a reader who has read one knows where to
//! look in the next: its title is the package name, its second paragraph opens
//! with the crate's place in the workspace (`Part of the [Logicaffeine](…)
//! workspace, tier N.`, N computed from the manifests), and it ends with the
//! navigation footer whose Changelog link is the crate's own. Its Cargo
//! `description` is one plain line: no trailing period, at most
//! `DESCRIPTION_MAX` characters.
//!
//! The battery below runs per crate so a failure names the crate and the
//! violated invariant. Exemptions are declared in the consts at the top with a
//! reason string; the goal is that they stay empty.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(readme_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const REPO_URL: &str = "https://github.com/Brahmastra-Labs/logicaffeine";

/// (crate dir, module, reason) — pub mods a README may omit. Goal: empty.
const PUB_MOD_EXEMPT: &[(&str, &str, &str)] = &[];

/// (crate dir, feature, reason) — features a README table may omit. Goal: empty.
const FEATURE_EXEMPT: &[(&str, &str, &str)] = &[];

/// (version token, reason) — README version strings that are not the workspace
/// version. Every entry must justify itself.
const VERSION_ALLOW: &[(&str, &str)] =
    &[("0.0.0", "out-of-band tool version (wirebench, wiki_trace)")];

/// The longest Cargo `description` a member may carry: one line on crates.io and the docs landing page.
const DESCRIPTION_MAX: usize = 160;

/// Crates on the docs site beyond the publishable set (deliberate inclusions).
const SITE_EXTRA: &[&str] = &["logicaffeine_web"];

/// Fence info-strings a README may use. Bare fences are banned (rustdoc treats
/// them as Rust doctests once the README is `include_str!`ed into lib.rs), and
/// `ignore` is banned because the suite's doctest pass runs `--include-ignored`.
const ALLOWED_FENCES: &[&str] = &["rust", "rust,no_run", "bash", "sh", "text", "logos", "toml"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

mod checks {
    use super::*;

    pub fn read(rel: &str) -> String {
        let path = repo_root().join(rel);
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
    }

    fn exists(rel: &str) -> bool {
        repo_root().join(rel).exists()
    }

    /// The optional, git-ignored member: absent in CI checkouts.
    fn skip_if_absent(dir: &str) -> bool {
        !exists(dir)
    }

    fn cargo_toml(dir: &str) -> toml::Value {
        let text = read(&format!("{dir}/Cargo.toml"));
        toml::from_str(&text).unwrap_or_else(|e| panic!("parse {dir}/Cargo.toml: {e}"))
    }

    pub fn workspace_version() -> String {
        let root: toml::Value = toml::from_str(&read("Cargo.toml")).expect("parse root Cargo.toml");
        root["workspace"]["package"]["version"]
            .as_str()
            .expect("workspace.package.version")
            .to_string()
    }

    /// Every `](target)` markdown link target with its line number.
    fn link_targets(text: &str) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let mut rest = line;
            while let Some(pos) = rest.find("](") {
                rest = &rest[pos + 2..];
                if let Some(end) = rest.find(')') {
                    out.push((n + 1, rest[..end].trim().to_string()));
                    rest = &rest[end + 1..];
                } else {
                    break;
                }
            }
        }
        out
    }

    struct Fence {
        line: usize,
        info: String,
        body: String,
    }

    fn fences(text: &str) -> Vec<Fence> {
        let lines: Vec<&str> = text.lines().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let t = lines[i].trim_start();
            if let Some(info) = t.strip_prefix("```") {
                let info = info.trim().to_string();
                let start = i + 1;
                let mut body = String::new();
                i += 1;
                while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                    body.push_str(lines[i]);
                    body.push('\n');
                    i += 1;
                }
                out.push(Fence {
                    line: start,
                    info,
                    body,
                });
            }
            i += 1;
        }
        out
    }

    /// Markdown with fenced blocks and inline code spans removed — the prose.
    pub fn prose(text: &str) -> String {
        let mut out = String::new();
        let mut in_fence = false;
        for line in text.lines() {
            if line.trim_start().starts_with("```") {
                in_fence = !in_fence;
                continue;
            }
            if in_fence {
                continue;
            }
            for (i, seg) in line.split('`').enumerate() {
                if i % 2 == 0 {
                    out.push_str(seg);
                }
            }
            out.push('\n');
        }
        out
    }

    pub fn no_escaping_links(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let text = read(&format!("{dir}/README.md"));
        let bad: Vec<String> = link_targets(&text)
            .into_iter()
            .filter(|(_, t)| t.starts_with("../"))
            .map(|(n, t)| format!("{dir}/README.md:{n} → {t}"))
            .collect();
        assert!(
            bad.is_empty(),
            "workspace-escaping relative links break on crates.io — use absolute {REPO_URL} URLs:\n{}",
            bad.join("\n")
        );
    }

    pub fn github_links(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let text = read(&format!("{dir}/README.md"));
        let mut bad = Vec::new();
        for (n, target) in link_targets(&text) {
            let Some(rest) = target.strip_prefix(REPO_URL) else {
                continue;
            };
            let Some(rest) = rest.strip_prefix('/') else {
                continue;
            };
            let (kind, rest) = match rest.split_once('/') {
                Some(pair) => pair,
                None => continue,
            };
            if kind != "blob" && kind != "tree" {
                continue;
            }
            let Some((git_ref, path)) = rest.split_once('/') else {
                bad.push(format!("{dir}/README.md:{n} → {target} (no path)"));
                continue;
            };
            if git_ref != "main" {
                bad.push(format!(
                    "{dir}/README.md:{n} → {target} (pin to main, not {git_ref})"
                ));
                continue;
            }
            let path = path.split(['#', '?']).next().unwrap_or(path);
            if !exists(path) {
                bad.push(format!(
                    "{dir}/README.md:{n} → {target} ({path} does not exist)"
                ));
            }
        }
        assert!(
            bad.is_empty(),
            "repo links must resolve in the working tree:\n{}",
            bad.join("\n")
        );
    }

    pub fn feature_parity(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let manifest = cargo_toml(dir);
        let declared: BTreeSet<String> = manifest
            .get("features")
            .and_then(|f| f.as_table())
            .map(|t| t.keys().filter(|k| *k != "default").cloned().collect())
            .unwrap_or_default();
        if declared.is_empty() {
            return;
        }
        let text = read(&format!("{dir}/README.md"));
        let section: Vec<&str> = {
            let mut in_section = false;
            let mut rows = Vec::new();
            for line in text.lines() {
                if line.starts_with("## ") {
                    in_section = line.trim() == "## Feature flags";
                    continue;
                }
                if in_section {
                    rows.push(line);
                }
            }
            rows
        };
        assert!(
            !section.is_empty(),
            "{dir}: has [features] {declared:?} but README has no `## Feature flags` section"
        );
        let mut documented = BTreeSet::new();
        for row in &section {
            let row = row.trim();
            if !row.starts_with('|') {
                continue;
            }
            let first_cell = row.trim_matches('|').split('|').next().unwrap_or("").trim();
            if first_cell.contains("---") || first_cell == "Feature" {
                continue;
            }
            for (i, seg) in first_cell.split('`').enumerate() {
                if i % 2 == 1 && !seg.is_empty() {
                    documented.insert(seg.to_string());
                }
            }
        }
        // A README may document the `default` feature row; that is never drift.
        documented.remove("default");
        let exempt = |name: &str| {
            FEATURE_EXEMPT
                .iter()
                .any(|(d, f, _)| *d == dir && *f == name)
        };
        let missing: Vec<&String> = declared
            .iter()
            .filter(|f| !documented.contains(*f) && !exempt(f))
            .collect();
        let phantom: Vec<&String> = documented
            .iter()
            .filter(|f| !declared.contains(*f))
            .collect();
        assert!(
            missing.is_empty() && phantom.is_empty(),
            "{dir}: feature-table drift — undocumented {missing:?}, phantom {phantom:?} (declared {declared:?})"
        );
    }

    pub fn pub_mods(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let lib = read(&format!("{dir}/src/lib.rs"));
        let readme = read(&format!("{dir}/README.md"));
        let lines: Vec<&str> = lib.lines().collect();
        let mut missing = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let t = line.trim_start();
            let Some(rest) = t.strip_prefix("pub mod ") else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                continue;
            }
            // `#[doc(hidden)]` (within the few attribute lines above) opts a
            // module out of the documented surface.
            let hidden = lines[i.saturating_sub(4)..i]
                .iter()
                .any(|l| l.contains("doc(hidden)"));
            if hidden {
                continue;
            }
            let exempt = PUB_MOD_EXEMPT
                .iter()
                .any(|(d, m, _)| *d == dir && *m == name);
            if exempt {
                continue;
            }
            let mentioned = readme.contains(&format!("`{name}`"))
                || readme.contains(&format!("`{name}::"))
                || readme.contains(&format!("`{name}/"))
                || readme.contains(&format!("`{name}.rs`"));
            if !mentioned {
                missing.push(name);
            }
        }
        assert!(
            missing.is_empty(),
            "{dir}: README does not mention pub mod(s) {missing:?} — document them (backticked) or exempt with a reason"
        );
    }

    pub fn versions(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let ws = workspace_version();
        let text = read(&format!("{dir}/README.md"));
        let mut bad = Vec::new();
        for (n, line) in text.lines().enumerate() {
            for token in line
                .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                .filter(|t| !t.is_empty())
            {
                let token = token.trim_matches('.');
                let parts: Vec<&str> = token.split('.').collect();
                if parts.len() != 3
                    || !parts
                        .iter()
                        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
                {
                    continue;
                }
                if token == ws || VERSION_ALLOW.iter().any(|(v, _)| *v == token) {
                    continue;
                }
                bad.push(format!(
                    "{dir}/README.md:{} → {token} (workspace is {ws})",
                    n + 1
                ));
            }
        }
        assert!(bad.is_empty(), "stale version claims:\n{}", bad.join("\n"));
    }

    pub fn include_str_present(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let lib = read(&format!("{dir}/src/lib.rs"));
        assert!(
            lib.contains(r#"#![doc = include_str!("../README.md")]"#),
            "{dir}/src/lib.rs must include the README as its crate docs \
             (#![doc = include_str!(\"../README.md\")]) so GitHub, crates.io, docs.rs \
             and the docs site render one surface"
        );
    }

    pub fn cargo_metadata(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let manifest = cargo_toml(dir);
        let package = manifest
            .get("package")
            .and_then(|p| p.as_table())
            .unwrap_or_else(|| panic!("{dir}/Cargo.toml has no [package]"));
        assert_eq!(
            package.get("readme").and_then(|r| r.as_str()),
            Some("README.md"),
            "{dir}/Cargo.toml must set readme = \"README.md\""
        );
        let description_ok = match package.get("description") {
            Some(toml::Value::String(s)) => !s.trim().is_empty(),
            Some(toml::Value::Table(t)) => {
                t.get("workspace").and_then(|w| w.as_bool()) == Some(true)
            }
            _ => false,
        };
        assert!(
            description_ok,
            "{dir}/Cargo.toml must set a non-empty description"
        );
    }

    pub fn fence_hygiene(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let rel = format!("{dir}/README.md");
        let text = read(&rel);
        let mut bad = Vec::new();
        for f in fences(&text) {
            let info: String = f.info.split_whitespace().collect();
            if info.is_empty() {
                bad.push(format!(
                    "{rel}:{} → bare ``` fence (rustdoc doctests it; tag it, e.g. ```text)",
                    f.line
                ));
                continue;
            }
            if info.split(',').any(|attr| attr == "ignore") {
                bad.push(format!(
                    "{rel}:{} → ```{info} (the doctest pass runs --include-ignored; use text or no_run)",
                    f.line
                ));
                continue;
            }
            if !ALLOWED_FENCES.contains(&info.as_str()) {
                bad.push(format!(
                    "{rel}:{} → ```{info} not in the allowlist {ALLOWED_FENCES:?}",
                    f.line
                ));
                continue;
            }
            if info.starts_with("rust") {
                for (i, body_line) in f.body.lines().enumerate() {
                    let t = body_line.trim_start();
                    if t == "#" || t.starts_with("# ") {
                        bad.push(format!(
                            "{rel}:{} → hidden doctest line (renders literally on GitHub/crates.io; make the example self-contained)",
                            f.line + i + 1
                        ));
                    }
                    if t.contains("...") || t.contains('…') {
                        bad.push(format!(
                            "{rel}:{} → placeholder in a rust fence; examples are real code (signature listings go in ```text)",
                            f.line + i + 1
                        ));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "fence hygiene:\n{}", bad.join("\n"));
    }

    fn package_name(dir: &str) -> String {
        cargo_toml(dir)["package"]["name"]
            .as_str()
            .unwrap()
            .to_string()
    }

    pub fn title(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let text = read(&format!("{dir}/README.md"));
        let first = text.lines().next().unwrap_or_default();
        let want = format!("# {}", package_name(dir));
        assert_eq!(
            first, want,
            "{dir}/README.md must open with its package name as the title"
        );
    }

    pub fn part_of(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let text = read(&format!("{dir}/README.md"));
        let lead =
            format!("Part of the [Logicaffeine]({REPO_URL}/blob/main/README.md) workspace, tier ");
        let before_sections: Vec<&str> =
            text.lines().take_while(|l| !l.starts_with("## ")).collect();
        let line = before_sections.iter().find(|l| l.starts_with(&lead)).unwrap_or_else(|| {
            panic!("{dir}/README.md must state its place before its first section: a paragraph opening `{lead}N.`")
        });
        let stated: u32 = line[lead.len()..]
            .split('.')
            .next()
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("{dir}/README.md: `{line}` does not name a tier number"));
        let computed = crate::shape::tier(&package_name(dir));
        assert_eq!(
            stated, computed,
            "{dir}/README.md says tier {stated}; its manifest puts it at tier {computed}"
        );
    }

    pub fn footer(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let text = read(&format!("{dir}/README.md"));
        let lines: Vec<&str> = text.trim_end().lines().collect();
        let want = format!(
            "[Docs index]({REPO_URL}/blob/main/docs/README.md) · [Root README]({REPO_URL}/blob/main/README.md) · \
             [Changelog]({REPO_URL}/blob/main/{dir}/CHANGELOG.md)"
        );
        assert!(
            lines.len() >= 2 && lines[lines.len() - 2] == "---" && lines[lines.len() - 1] == want,
            "{dir}/README.md must end with `---` and the footer\n{want}"
        );
        assert!(
            exists(&format!("{dir}/CHANGELOG.md")),
            "{dir} has no CHANGELOG.md for its footer to link"
        );
    }

    pub fn description_style(dir: &str) {
        if skip_if_absent(dir) {
            return;
        }
        let manifest = cargo_toml(dir);
        let Some(d) = manifest["package"]
            .get("description")
            .and_then(|d| d.as_str())
        else {
            return;
        };
        assert!(
            !d.trim_end().ends_with('.'),
            "{dir}/Cargo.toml description ends with a period: {d:?}"
        );
        assert!(
            !d.contains('\n'),
            "{dir}/Cargo.toml description spans lines"
        );
        assert!(
            d.chars().count() <= DESCRIPTION_MAX,
            "{dir}/Cargo.toml description is {} characters (at most {DESCRIPTION_MAX}): {d:?}",
            d.chars().count()
        );
    }
}

mod shape {
    use super::*;
    use std::collections::BTreeMap;

    /// Package name → the internal package names it depends on (normal, build and target-specific
    /// edges), for every workspace member.
    fn graph() -> BTreeMap<String, BTreeSet<String>> {
        let mut out = BTreeMap::new();
        for dir in crate::workspace_members() {
            let manifest: toml::Value =
                toml::from_str(&checks::read(&format!("{dir}/Cargo.toml"))).unwrap();
            let name = manifest["package"]["name"].as_str().unwrap().to_string();
            let mut tables: Vec<&toml::Value> = ["dependencies", "build-dependencies"]
                .iter()
                .filter_map(|t| manifest.get(*t))
                .collect();
            if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
                for target in targets.values() {
                    tables.extend(
                        ["dependencies", "build-dependencies"]
                            .iter()
                            .filter_map(|t| target.get(*t)),
                    );
                }
            }
            let deps = tables
                .iter()
                .filter_map(|t| t.as_table())
                .flat_map(|t| t.keys().cloned())
                .collect();
            out.insert(name, deps);
        }
        out
    }

    /// A member's tier: one more than the highest tier among its internal dependencies, 0 with none.
    pub fn tier(package: &str) -> u32 {
        fn walk(
            p: &str,
            g: &BTreeMap<String, BTreeSet<String>>,
            memo: &mut BTreeMap<String, u32>,
        ) -> u32 {
            if let Some(t) = memo.get(p) {
                return *t;
            }
            let t = g[p]
                .iter()
                .filter(|d| g.contains_key(*d))
                .map(|d| walk(d, g, memo) + 1)
                .max()
                .unwrap_or(0);
            memo.insert(p.to_string(), t);
            t
        }
        let g = graph();
        assert!(
            g.contains_key(package),
            "{package} is not a workspace member"
        );
        walk(package, &g, &mut BTreeMap::new())
    }
}

macro_rules! readme_battery {
    ($($m:ident : $dir:literal),* $(,)?) => {
        pub const ALL_DIRS: &[&str] = &[$($dir),*];
        $(mod $m {
            const DIR: &str = $dir;
            #[test] fn no_escaping_relative_links() { crate::checks::no_escaping_links(DIR) }
            #[test] fn github_links_resolve()       { crate::checks::github_links(DIR) }
            #[test] fn feature_table_parity()       { crate::checks::feature_parity(DIR) }
            #[test] fn pub_mods_documented()        { crate::checks::pub_mods(DIR) }
            #[test] fn version_claims_current()     { crate::checks::versions(DIR) }
            #[test] fn lib_includes_readme()        { crate::checks::include_str_present(DIR) }
            #[test] fn cargo_metadata_present()     { crate::checks::cargo_metadata(DIR) }
            #[test] fn fence_hygiene()              { crate::checks::fence_hygiene(DIR) }
            #[test] fn title_is_the_package_name()  { crate::checks::title(DIR) }
            #[test] fn states_its_place_and_tier()  { crate::checks::part_of(DIR) }
            #[test] fn ends_with_the_footer()       { crate::checks::footer(DIR) }
            #[test] fn describes_itself_plainly()   { crate::checks::description_style(DIR) }
        })*
    };
}

readme_battery! {
    base:       "crates/logicaffeine_base",
    compile:    "crates/logicaffeine_compile",
    data:       "crates/logicaffeine_data",
    forge:      "crates/logicaffeine_forge",
    jit:        "crates/logicaffeine_jit",
    kernel:     "crates/logicaffeine_kernel",
    language:   "crates/logicaffeine_language",
    lexicon:    "crates/logicaffeine_lexicon",
    lsp:        "crates/logicaffeine_lsp",
    proof:      "crates/logicaffeine_proof",
    runtime:    "crates/logicaffeine_runtime",
    synth:      "crates/logicaffeine_synth",
    system:     "crates/logicaffeine_system",
    tests:      "crates/logicaffeine_tests",
    tv:         "crates/logicaffeine_tv",
    verify:     "crates/logicaffeine_verify",
    wirebench:  "crates/logicaffeine_wirebench",
    cli:        "apps/logicaffeine_cli",
    web:        "apps/logicaffeine_web",
    wiki_trace: "scripts/wiki_trace",
    nano:       "apps/logicaffeine_nano",
}

/// The workspace's member directories, expanded as cargo expands `workspace.members`: a `dir/*` entry is
/// every subdirectory of `dir` holding a `Cargo.toml`. Any other glob is refused rather than read literally.
fn workspace_members() -> Vec<String> {
    let root: toml::Value =
        toml::from_str(&checks::read("Cargo.toml")).expect("parse root Cargo.toml");
    let mut out = Vec::new();
    for member in root["workspace"]["members"]
        .as_array()
        .expect("workspace.members")
    {
        let member = member.as_str().unwrap();
        match member.strip_suffix("/*") {
            Some(parent) => {
                assert!(
                    !parent.contains(['*', '?', '[']),
                    "unsupported member glob {member:?}"
                );
                let mut dirs: Vec<String> = std::fs::read_dir(repo_root().join(parent))
                    .unwrap_or_else(|e| panic!("{parent}: {e}"))
                    .flatten()
                    .filter(|e| e.path().join("Cargo.toml").is_file())
                    .map(|e| format!("{parent}/{}", e.file_name().to_string_lossy()))
                    .collect();
                dirs.sort();
                out.extend(dirs);
            }
            None => {
                assert!(
                    !member.contains(['*', '?', '[']),
                    "unsupported member glob {member:?}"
                );
                out.push(member.to_string());
            }
        }
    }
    out
}

/// Every workspace member with a README must be in the battery — a new crate
/// cannot dodge the ratchet. Battery dirs must exist (except the git-ignored
/// nano, which is optional by design).
#[test]
fn registry_covers_all_members() {
    let mut missing = Vec::new();
    for dir in workspace_members() {
        let dir = dir.as_str();
        if repo_root().join(dir).join("README.md").exists() && !ALL_DIRS.contains(&dir) {
            missing.push(dir.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "members with READMEs missing from the battery: {missing:?}"
    );
    for dir in ALL_DIRS {
        if *dir == "apps/logicaffeine_nano" {
            continue;
        }
        assert!(
            repo_root().join(dir).exists(),
            "battery dir {dir} does not exist (typo?)"
        );
    }
}

/// The docs app has no Cargo.toml — it gets the text-level checks only.
#[test]
fn docs_app_readme_hygiene() {
    const DIR: &str = "apps/logicaffeine_docs";
    checks::no_escaping_links(DIR);
    checks::github_links(DIR);
    checks::versions(DIR);
    checks::fence_hygiene(DIR);
}

/// nano is a standalone workspace pinned by hand — it must track the lockstep
/// version exactly (package version and every internal dep version).
#[test]
fn nano_lockstep() {
    let dir = repo_root().join("apps/logicaffeine_nano");
    if !dir.exists() {
        return;
    }
    let ws = checks::workspace_version();
    let manifest: toml::Value = toml::from_str(&checks::read("apps/logicaffeine_nano/Cargo.toml"))
        .expect("parse nano Cargo.toml");
    assert_eq!(
        manifest["package"]["version"].as_str(),
        Some(ws.as_str()),
        "nano package version must track the workspace"
    );
    let deps = manifest["dependencies"]
        .as_table()
        .expect("nano [dependencies]");
    for (name, spec) in deps {
        if !name.starts_with("logicaffeine-") {
            continue;
        }
        let version = spec.get("version").and_then(|v| v.as_str());
        assert_eq!(
            version,
            Some(ws.as_str()),
            "nano dep {name} must track the workspace version {ws}"
        );
    }
}

/// The docs site must document every publishable crate (and only deliberate
/// extras beyond that set).
#[test]
fn docs_site_curates_published_crates() {
    let mut publishable = BTreeSet::new();
    for dir in workspace_members() {
        let manifest: toml::Value =
            toml::from_str(&checks::read(&format!("{dir}/Cargo.toml"))).unwrap();
        let package = &manifest["package"];
        if package.get("publish").and_then(|p| p.as_bool()) == Some(false) {
            continue;
        }
        let lib_name = package["name"].as_str().unwrap().replace('-', "_");
        publishable.insert(lib_name);
    }

    let build_sh = checks::read("apps/logicaffeine_docs/build.sh");
    let mut curated = BTreeSet::new();
    let mut in_array = false;
    for line in build_sh.lines() {
        let t = line.trim();
        if t.starts_with("CRATES=(") {
            in_array = true;
            continue;
        }
        if in_array {
            if t.starts_with(')') {
                break;
            }
            if !t.is_empty() && !t.starts_with('#') {
                curated.insert(t.to_string());
            }
        }
    }
    assert!(
        !curated.is_empty(),
        "could not parse CRATES=( … ) from build.sh"
    );

    let missing: Vec<&String> = publishable.difference(&curated).collect();
    let unexpected: Vec<&String> = curated
        .iter()
        .filter(|c| !publishable.contains(*c) && !SITE_EXTRA.contains(&c.as_str()))
        .collect();
    assert!(
        missing.is_empty() && unexpected.is_empty(),
        "docs-site curation drift — published crates missing from build.sh: {missing:?}; \
         unexpected entries: {unexpected:?}"
    );
}

/// The workspace documentation URL is the self-hosted rustdoc site; the old
/// value pointed at a crates.io package that does not exist.
#[test]
fn workspace_documentation_url() {
    let root: toml::Value =
        toml::from_str(&checks::read("Cargo.toml")).expect("parse root Cargo.toml");
    assert_eq!(
        root["workspace"]["package"]["documentation"].as_str(),
        Some("https://docs.logicaffeine.com"),
        "workspace.package.documentation must point at the self-hosted docs site"
    );
}

/// Branding: the project is Logicaffeine / LOGOS. `LogicAffeine` may appear
/// only inside code spans (e.g. the `LogicAffeineServer` struct name).
#[test]
fn branding_consistent() {
    let mut bad = Vec::new();
    for dir in ALL_DIRS.iter().chain(&["apps/logicaffeine_docs"]) {
        let readme = repo_root().join(dir).join("README.md");
        if !readme.exists() {
            continue;
        }
        let text = fs::read_to_string(&readme).unwrap();
        if checks::prose(&text).contains("LogicAffeine") {
            bad.push(format!("{dir}/README.md"));
        }
        let manifest_path = repo_root().join(dir).join("Cargo.toml");
        if manifest_path.exists() {
            let manifest: toml::Value =
                toml::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
            if let Some(desc) = manifest["package"]
                .get("description")
                .and_then(|d| d.as_str())
            {
                if desc.contains("LogicAffeine") {
                    bad.push(format!("{dir}/Cargo.toml description"));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "`LogicAffeine` in prose/descriptions (the brand is Logicaffeine / LOGOS): {bad:?}"
    );
}

/// Every version exemption is used by some README: a dead entry would quietly admit that version later.
#[test]
fn every_version_exemption_is_in_use() {
    let readmes: Vec<String> = ALL_DIRS
        .iter()
        .map(|d| repo_root().join(d).join("README.md"))
        .filter(|p| p.exists())
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    let dead: Vec<&str> = VERSION_ALLOW
        .iter()
        .map(|(v, _)| *v)
        .filter(|v| !readmes.iter().any(|r| r.contains(v)))
        .collect();
    assert!(
        dead.is_empty(),
        "version exemptions no README uses (remove them): {dead:?}"
    );
}

/// The tier computation reads the manifests the way `tier_map_lock` does: dependency-free crates are
/// tier 0 and the CLI sits above the compiler.
#[test]
fn the_tier_computation_reads_the_manifests() {
    assert_eq!(shape::tier("logicaffeine-base"), 0);
    assert!(shape::tier("logicaffeine-cli") > shape::tier("logicaffeine-compile"));
    assert!(shape::tier("logicaffeine-compile") > shape::tier("logicaffeine-language"));
}
