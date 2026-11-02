//! Every crate tier the docs state is the tier the manifests imply.
//!
//! A crate's tier is the length of its longest path of internal dependencies (normal, build and
//! target-specific edges; dev-dependencies are test wiring, not layering), so a dependency always points to a
//! strictly lower tier and publishing in tier order puts every dependency first. Three documents state tiers:
//! the root README's workspace map, `docs/architecture.md`'s tier graph (table and diagram), and the crate
//! hierarchy in `docs/project/VERSIONING.md`. Each must list every crate it covers at its computed tier, and the
//! prose naming the crates outside `default-members` names exactly those.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tier_map_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const README: &str = "README.md";
const ARCHITECTURE: &str = "docs/architecture.md";
const VERSIONING: &str = "docs/project/VERSIONING.md";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

fn toml_at(rel: &str) -> toml::Value {
    read(rel)
        .parse()
        .unwrap_or_else(|e| panic!("{rel} is not TOML: {e}"))
}

struct Member {
    publish: bool,
    deps: BTreeSet<String>,
}

/// Every workspace member under `crates/` and `apps/`, by short name (`logicaffeine-kernel` → `kernel`).
fn members() -> BTreeMap<String, Member> {
    let root = repo_root();
    let mut dirs = Vec::new();
    for parent in ["crates", "apps"] {
        for entry in std::fs::read_dir(root.join(parent)).unwrap().flatten() {
            if entry.path().join("Cargo.toml").is_file() {
                dirs.push(format!("{parent}/{}", entry.file_name().to_string_lossy()));
            }
        }
    }
    let workspace = toml_at("Cargo.toml");
    let listed: Vec<String> = workspace["workspace"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap().to_string())
        .collect();
    let is_member = |dir: &str| {
        listed
            .iter()
            .any(|m| m == dir || (m.ends_with("/*") && dir.starts_with(m.trim_end_matches('*'))))
    };
    let mut out = BTreeMap::new();
    for dir in dirs.into_iter().filter(|d| is_member(d)) {
        let manifest = toml_at(&format!("{dir}/Cargo.toml"));
        let package = &manifest["package"];
        let name = package["name"].as_str().unwrap();
        let Some(short) = name.strip_prefix("logicaffeine-") else {
            continue;
        };
        let mut tables: Vec<&toml::Value> = ["dependencies", "build-dependencies"]
            .iter()
            .filter_map(|s| manifest.get(*s))
            .collect();
        if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
            for target in targets.values() {
                tables.extend(
                    ["dependencies", "build-dependencies"]
                        .iter()
                        .filter_map(|s| target.get(*s)),
                );
            }
        }
        let deps = tables
            .iter()
            .filter_map(|t| t.as_table())
            .flat_map(|t| t.keys())
            .filter_map(|k| k.strip_prefix("logicaffeine-"))
            .map(str::to_string)
            .collect();
        let publish = package
            .get("publish")
            .and_then(|p| p.as_bool())
            .unwrap_or(true);
        out.insert(short.to_string(), Member { publish, deps });
    }
    out
}

fn tiers() -> BTreeMap<String, u32> {
    fn tier(
        name: &str,
        all: &BTreeMap<String, Member>,
        memo: &mut BTreeMap<String, u32>,
        path: &mut Vec<String>,
    ) -> u32 {
        if let Some(t) = memo.get(name) {
            return *t;
        }
        assert!(
            !path.iter().any(|p| p == name),
            "dependency cycle through {path:?} → {name}"
        );
        path.push(name.to_string());
        let t = all[name]
            .deps
            .iter()
            .filter(|d| all.contains_key(*d))
            .map(|d| 1 + tier(d, all, memo, path))
            .max()
            .unwrap_or(0);
        path.pop();
        memo.insert(name.to_string(), t);
        t
    }
    let all = members();
    let mut memo = BTreeMap::new();
    for name in all.keys() {
        tier(name, &all, &mut memo, &mut Vec::new());
    }
    memo
}

/// The short crate name a table cell or diagram token names: link text or backticked, with or without the prefix.
fn crate_name(cell: &str) -> Option<String> {
    let text = cell.trim();
    let text = text
        .strip_prefix('[')
        .and_then(|t| t.split(']').next())
        .unwrap_or(text);
    let text = text.trim_matches('`').trim();
    let short = text
        .strip_prefix("logicaffeine_")
        .or_else(|| text.strip_prefix("logicaffeine-"))
        .unwrap_or(text);
    (!short.is_empty() && short.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
        .then(|| short.replace('_', "-"))
}

/// (crate, stated tier cell) for every row of every markdown table whose header has `Tier` and a crate column.
fn table_rows(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut columns: Option<(usize, usize)> = None;
    for line in text.lines() {
        if !line.starts_with('|') {
            columns = None;
            continue;
        }
        let cells: Vec<&str> = line
            .trim()
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        if line.starts_with("|--") || line.starts_with("|-") {
            continue;
        }
        match columns {
            None => {
                let tier = cells.iter().position(|c| *c == "Tier");
                let krate = cells.iter().position(|c| *c == "Crate");
                columns = tier
                    .zip(krate)
                    .map(|(t, c)| (t, c))
                    .or(Some((usize::MAX, usize::MAX)));
            }
            Some((t, c)) if t != usize::MAX => {
                if let Some(name) = cells.get(c).and_then(|cell| crate_name(cell)) {
                    out.push((name, cells.get(t).unwrap_or(&"").to_string()));
                }
            }
            Some(_) => {}
        }
    }
    out
}

/// (crate, tier) for every `Tier N …` line of a diagram, and for every crate indented under a `Tier N (…):` heading.
fn tier_lines(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("Tier ") {
            let number: String = rest
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '(' && *c != ':')
                .collect();
            let after = rest[number.len()..].trim_start();
            if after.starts_with('(') || after.starts_with(':') {
                current = Some(number);
                continue;
            }
            current = None;
            let names = after.split("  ").next().unwrap_or_default();
            for token in names.split('·') {
                if let Some(name) = crate_name(token) {
                    out.push((name, number.clone()));
                }
            }
            continue;
        }
        if line.starts_with("  ") && !t.is_empty() {
            if let (Some(n), Some(name)) = (&current, crate_name(t)) {
                out.push((name, n.clone()));
                continue;
            }
        }
        if t.is_empty() || !line.starts_with(' ') {
            current = None;
        }
    }
    out
}

fn wrong(rows: &[(String, String)], tiers: &BTreeMap<String, u32>) -> Vec<String> {
    rows.iter()
        .filter_map(|(name, stated)| match tiers.get(name) {
            None => Some(format!("{name}: not a workspace crate")),
            Some(t) if stated.trim() != t.to_string() => {
                Some(format!("{name}: stated `{stated}`, computed {t}"))
            }
            _ => None,
        })
        .collect()
}

fn uncovered(rows: &[(String, String)], want: impl Iterator<Item = String>) -> Vec<String> {
    let named: BTreeSet<&str> = rows.iter().map(|(n, _)| n.as_str()).collect();
    want.filter(|w| !named.contains(w.as_str())).collect()
}

#[test]
fn the_computed_tiers_point_strictly_down() {
    let all = members();
    let tiers = tiers();
    for (name, member) in &all {
        for dep in member.deps.iter().filter(|d| all.contains_key(*d)) {
            assert!(
                tiers[dep] < tiers[name],
                "{name} (tier {}) depends on {dep} (tier {})",
                tiers[name],
                tiers[dep]
            );
        }
    }
    assert_eq!(tiers["base"], 0);
    assert!(
        tiers["compile"] > tiers["language"],
        "compile depends on language"
    );
}

#[test]
fn the_parsers_read_each_shape() {
    let table = "| Tier | Crate | Role |\n|------|-------|------|\n| 0 | [logicaffeine_base](x) | y |\n| — | [logicaffeine_tv](x) | z |\n";
    assert_eq!(
        table_rows(table),
        [
            ("base".to_string(), "0".to_string()),
            ("tv".to_string(), "—".to_string())
        ]
    );
    let flipped = "| Crate | Tier | Role |\n|---|---|---|\n| [kernel](x) | 1 | y |\n";
    assert_eq!(
        table_rows(flipped),
        [("kernel".to_string(), "1".to_string())]
    );
    let diagram = "Tier 0   base · runtime · forge                  (no internal deps)\n";
    assert_eq!(tier_lines(diagram).len(), 3);
    let blocks = "Tier 2 (depends on Tier 0-1):\n  logicaffeine-system\n  logicaffeine-proof\n\nTier 3 (x):\n  logicaffeine-language\n";
    assert_eq!(
        tier_lines(blocks),
        [
            ("system".to_string(), "2".to_string()),
            ("proof".to_string(), "2".to_string()),
            ("language".to_string(), "3".to_string())
        ]
    );
}

#[test]
fn the_readme_workspace_map_states_the_computed_tiers() {
    let tiers = tiers();
    let rows = table_rows(&read(README));
    let bad = wrong(&rows, &tiers);
    assert!(
        bad.is_empty(),
        "`{README}` workspace map disagrees with the manifests: {bad:?}"
    );
    let crates_dir = members().into_keys().filter(|n| {
        repo_root()
            .join(format!("crates/logicaffeine_{}", n.replace('-', "_")))
            .is_dir()
    });
    let gap = uncovered(&rows, crates_dir);
    assert!(gap.is_empty(), "`{README}` workspace map omits {gap:?}");
}

#[test]
fn the_architecture_guide_states_the_computed_tiers() {
    let tiers = tiers();
    let text = read(ARCHITECTURE);
    let mut rows = table_rows(&text);
    rows.extend(tier_lines(&text));
    let bad = wrong(&rows, &tiers);
    assert!(
        bad.is_empty(),
        "`{ARCHITECTURE}` tier graph disagrees with the manifests: {bad:?}"
    );
    let published = members()
        .into_iter()
        .filter(|(_, m)| m.publish)
        .map(|(n, _)| n);
    let gap = uncovered(&rows, published);
    assert!(
        gap.is_empty(),
        "`{ARCHITECTURE}` tier graph omits published {gap:?}"
    );
}

#[test]
fn the_publish_hierarchy_states_the_computed_tiers() {
    let tiers = tiers();
    let rows = tier_lines(&read(VERSIONING));
    let bad = wrong(&rows, &tiers);
    assert!(
        bad.is_empty(),
        "`{VERSIONING}` crate hierarchy disagrees with the manifests: {bad:?}"
    );
    let published = members()
        .into_iter()
        .filter(|(_, m)| m.publish)
        .map(|(n, _)| n);
    let gap = uncovered(&rows, published);
    assert!(
        gap.is_empty(),
        "`{VERSIONING}` crate hierarchy omits published {gap:?}"
    );
}

/// The crate short names backticked in the paragraph that mentions `default-members`.
fn named_outside_default_members(
    text: &str,
    crates: &BTreeSet<String>,
) -> Option<BTreeSet<String>> {
    let para = text.split("\n\n").find(|p| {
        p.contains("default-members") && (p.contains("outside") || p.contains("excluded"))
    })?;
    Some(
        para.split('`')
            .skip(1)
            .step_by(2)
            .filter_map(crate_name)
            .filter(|n| crates.contains(n))
            .collect(),
    )
}

#[test]
fn the_crates_named_outside_default_members_are_exactly_those() {
    let workspace = toml_at("Cargo.toml");
    let defaults: BTreeSet<String> = workspace["workspace"]["default-members"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| {
            m.as_str()?
                .rsplit('/')
                .next()?
                .strip_prefix("logicaffeine_")
                .map(|s| s.replace('_', "-"))
        })
        .collect();
    let crates: BTreeSet<String> = members().into_keys().collect();
    let outside: BTreeSet<String> = crates
        .iter()
        .filter(|n| !defaults.contains(*n))
        .cloned()
        .collect();
    for rel in [README, ARCHITECTURE] {
        if let Some(named) = named_outside_default_members(&read(rel), &crates) {
            assert_eq!(named, outside, "`{rel}` names {named:?} as outside `default-members`; the root Cargo.toml says {outside:?}");
        }
    }
}
