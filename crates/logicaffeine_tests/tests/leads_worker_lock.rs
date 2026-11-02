//! The pricing form's lead worker (`services/leads`) and the joints it shares with the rest of the tree.
//!
//! The worker's vitest suite and the page's Rust validator walk one vector file, and the worker's deploy
//! workflow reruns that suite whenever the file or the worker changes, then migrates D1 before it deploys.
//! Those joints live in three places (the workflow's path filter, the suite's imports, the migrate and
//! deploy commands), and nothing but this lock notices one drifting from the others: a moved vector file
//! would leave the workflow blind to it, and a reordered deploy would ship code ahead of its schema.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test leads_worker_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

const VECTORS: &str = "crates/logicaffeine_tests/tests/fixtures/lead_vectors.json";
const WORKFLOW: &str = ".github/workflows/deploy-leads.yml";
const WORKER: &str = "services/leads";
const SUITES: &[&str] = &["services/leads/src/lead.test.js", "services/leads/src/index.test.js"];
const PLACEHOLDER_ID: &str = "00000000-0000-0000-0000-000000000000";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

/// `a/./b/../c` → `a/c`.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().unwrap_or_else(|| panic!("{path} climbs out of the repository"));
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

/// The lines of `jobs.<job>` in the workflow: from `  <job>:` to the next line indented two spaces or less.
fn job<'a>(workflow: &'a str, name: &str) -> Vec<&'a str> {
    let header = format!("  {name}:");
    let mut lines = workflow.lines().skip_while(|l| *l != header);
    assert!(lines.next().is_some(), "{WORKFLOW} has no `{name}` job");
    lines.take_while(|l| l.is_empty() || l.starts_with("   ") || l.trim_start().starts_with('#')).collect()
}

/// The index of the first line of `lines` that contains `needle`.
fn line_of(lines: &[&str], needle: &str, what: &str) -> usize {
    lines.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{what}: no line contains `{needle}`"))
}

fn wrangler() -> toml::Table {
    read(&format!("{WORKER}/wrangler.toml")).parse().expect("services/leads/wrangler.toml is TOML")
}

fn database_name() -> String {
    let config = wrangler();
    let dbs = config["d1_databases"].as_array().expect("wrangler.toml declares [[d1_databases]]");
    assert_eq!(dbs.len(), 1, "the lead worker binds exactly one D1 database");
    dbs[0]["database_name"].as_str().expect("d1 database_name").to_string()
}

#[test]
fn the_vector_file_is_the_one_both_worker_suites_import() {
    for suite in SUITES {
        let text = read(suite);
        let import = text
            .lines()
            .find_map(|l| l.strip_prefix("import vectors from '").and_then(|rest| rest.strip_suffix("';")))
            .unwrap_or_else(|| panic!("{suite} does not `import vectors from '…lead_vectors.json'`"));
        let dir = suite.rsplit_once('/').map(|(d, _)| d).unwrap();
        assert_eq!(
            normalize(&format!("{dir}/{import}")),
            VECTORS,
            "{suite} imports its vectors from {import}, which is not {VECTORS}"
        );
    }
}

#[test]
fn the_vector_file_holds_both_fields_with_forty_cases_each() {
    let vectors: serde_json::Value = serde_json::from_str(&read(VECTORS)).expect("the vector file is JSON");
    assert_eq!(vectors["version"], 1, "{VECTORS} changed version; both validators must be told");
    for field in ["email", "phone"] {
        let cases = vectors[field].as_array().unwrap_or_else(|| panic!("{VECTORS} has no `{field}` array"));
        assert!(cases.len() >= 40, "{VECTORS} holds {} {field} cases; the floor is 40", cases.len());
        assert!(cases.iter().any(|c| c["valid"] == true), "no valid {field} case");
        assert!(cases.iter().any(|c| c["valid"] == false), "no invalid {field} case");
    }
    assert!(vectors["trim_set"].as_array().is_some_and(|t| !t.is_empty()), "{VECTORS} has no trim_set");
}

#[test]
fn the_deploy_workflow_reruns_on_the_worker_and_on_the_vectors() {
    let workflow = read(WORKFLOW);
    for (trigger, next) in [("  push:", "  pull_request:"), ("  pull_request:", "  workflow_dispatch:")] {
        let start = workflow.find(trigger).unwrap_or_else(|| panic!("{WORKFLOW} has no `{}` trigger", trigger.trim()));
        let end = workflow[start..].find(next).map(|i| start + i).unwrap_or(workflow.len());
        let block = &workflow[start..end];
        for path in [format!("{WORKER}/**"), VECTORS.to_string(), WORKFLOW.to_string()] {
            assert!(
                block.contains(&format!("- '{path}'")),
                "{WORKFLOW}'s `{}` trigger does not watch {path}",
                trigger.trim()
            );
        }
    }
}

#[test]
fn the_deploy_waits_for_the_tests_and_migrates_before_it_deploys() {
    let workflow = read(WORKFLOW);

    let test = job(&workflow, "test");
    let install = line_of(&test, "run: npm ci", "the test job");
    let vitest = line_of(&test, "run: npx vitest run", "the test job");
    assert!(install < vitest, "the test job must install from the lockfile before it runs vitest");

    let deploy = job(&workflow, "deploy");
    assert!(deploy.iter().any(|l| l.trim() == "needs: test"), "the deploy job must need the test job");
    assert!(
        deploy.iter().any(|l| l.contains("github.ref == 'refs/heads/main'")),
        "the deploy job must deploy only from main"
    );
    let guard = line_of(&deploy, PLACEHOLDER_ID, "the deploy job's placeholder guard");
    let migrate = line_of(&deploy, &format!("run: npx wrangler d1 migrations apply {} --remote", database_name()), "the deploy job");
    let ship = deploy
        .iter()
        .position(|l| l.trim() == "run: npx wrangler deploy")
        .expect("the deploy job has no `npx wrangler deploy` step");
    assert!(guard < migrate, "the deploy job must refuse the placeholder database id before it migrates");
    assert!(migrate < ship, "the deploy job must migrate D1 before it deploys the worker");
    assert!(
        !workflow.contains("npm install") && !workflow.contains("|| true"),
        "{WORKFLOW} must install from the lockfile and let no step fail silently"
    );
}

/// `npm ci` trusts the lockfile only as far as the npm that reads it, so both jobs install on the one
/// Node release (and its bundled npm) the lockfile was verified with, never on a floating major.
#[test]
fn both_jobs_install_on_the_exact_node_release_the_lockfile_was_verified_with() {
    let workflow = read(WORKFLOW);
    let versions: Vec<&str> = workflow
        .lines()
        .filter_map(|l| l.trim().strip_prefix("node-version:"))
        .map(|v| v.trim().trim_matches('\'').trim_matches('"'))
        .collect();
    assert_eq!(versions.len(), 2, "{WORKFLOW} must set node-version once in each job, found {versions:?}");
    for v in &versions {
        let exact = v.split('.').count() == 3 && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        assert!(exact, "node-version {v:?} floats; pin the exact release the lockfile was verified with");
    }
    assert_eq!(versions[0], versions[1], "the test and deploy jobs install on different Node releases");
}

#[test]
fn the_placeholder_guard_names_the_placeholder_wrangler_toml_carries() {
    let config = wrangler();
    let id = config["d1_databases"][0]["database_id"].as_str().expect("d1 database_id");
    assert!(
        id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "services/leads/wrangler.toml database_id {id:?} is not a UUID"
    );
    let workflow = read(WORKFLOW);
    let guard = format!("^database_id = \"{PLACEHOLDER_ID}\"$");
    assert!(workflow.contains(&guard), "{WORKFLOW} must refuse the placeholder with the exact line `{guard}`");
    if id == PLACEHOLDER_ID {
        let line = format!("database_id = \"{PLACEHOLDER_ID}\"");
        assert!(
            read(&format!("{WORKER}/wrangler.toml")).lines().any(|l| l == line),
            "the placeholder in wrangler.toml must match the guard's line exactly, or the guard would miss it"
        );
    }
}

#[test]
fn the_package_scripts_name_the_database_the_worker_binds() {
    let package: serde_json::Value =
        serde_json::from_str(&read(&format!("{WORKER}/package.json"))).expect("services/leads/package.json is JSON");
    let name = database_name();
    for script in ["db:create", "db:migrate", "db:migrate:local"] {
        let command = package["scripts"][script].as_str().unwrap_or_else(|| panic!("package.json has no `{script}` script"));
        assert!(command.contains(&format!(" {name}")), "package.json `{script}` does not name the database {name}: {command}");
    }
    assert!(root().join(WORKER).join("package-lock.json").is_file(), "services/leads has no package-lock.json for `npm ci`");
}
