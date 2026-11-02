//! The Studio's heal worker (`services/heal`) and the joints it shares with the rest of the tree
//! (work/LLM_HEAL.md, DAT-108). The app's endpoint, the worker's route, the origins it answers, the
//! model and its limits, the secret that never enters the tree, and the deploy that reruns the
//! worker's tests before it ships: each is set in one place and read in another, and nothing but
//! this lock notices one drifting from the others.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test heal_worker_lock`

use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn wrangler() -> toml::Table {
    read("services/heal/wrangler.toml").parse().expect("services/heal/wrangler.toml parses")
}

fn var(name: &str) -> String {
    wrangler()["vars"][name].as_str().unwrap_or_else(|| panic!("[vars] {name} is a string")).to_string()
}

/// The app sends heals to the route the worker answers on, and the production address is what
/// the app falls back to when no build-time address is given.
#[test]
fn the_app_sends_heals_where_the_worker_listens() {
    let routes = wrangler()["routes"].as_array().expect("[[routes]]").clone();
    let patterns: Vec<String> = routes.iter().map(|r| r["pattern"].as_str().unwrap().to_string()).collect();
    assert_eq!(patterns, ["api.logicaffeine.com/heal"], "the worker's route");
    let app = read("apps/logicaffeine_web/src/ui/pages/heal.rs");
    assert!(app.contains("None => \"https://api.logicaffeine.com/heal\""), "the app's production endpoint is not the worker's route");
    assert!(app.contains("option_env!(\"LOGOS_HEAL_URL\")"), "the app does not read LOGOS_HEAL_URL at build time");
    let index = read("services/heal/src/index.js");
    assert!(index.contains("'/heal'"), "the worker does not answer /heal only");
}

/// The worker answers the site's origins and nothing else, over JSON, within its limits.
#[test]
fn the_worker_answers_the_site_within_its_limits() {
    let origins = var("ALLOWED_ORIGINS");
    for origin in ["https://logicaffeine.com", "https://www.logicaffeine.com", "https://logicaffeine.pages.dev", "https://*.logicaffeine.pages.dev"] {
        assert!(origins.split(',').any(|o| o == origin), "ALLOWED_ORIGINS lacks {origin}: {origins}");
    }
    assert!(!origins.contains("http://"), "a plain http origin is allowed in production: {origins}");
    assert!(!origins.contains('*') || origins.contains("https://*.logicaffeine.pages.dev"), "a wildcard origin beyond the preview deploys: {origins}");
    assert_eq!(var("UPSTREAM_URL"), "https://integrate.api.nvidia.com/v1/chat/completions");
    let model = var("MODEL");
    assert!(model.contains('/') && !model.contains(' '), "MODEL is not a catalogue id: {model}");
    let timeout: u64 = var("UPSTREAM_TIMEOUT_MS").parse().expect("UPSTREAM_TIMEOUT_MS is a number");
    assert!((10_000..=90_000).contains(&timeout), "the upstream timeout is {timeout} ms");
    let bytes: usize = var("MAX_TEXT_BYTES").parse().expect("MAX_TEXT_BYTES is a number");
    assert_eq!(bytes, 8192, "the most text one heal takes");
    let heal = read("services/heal/src/heal.js");
    assert!(heal.contains("export const MAX_TEXT_BYTES = 8192;"), "the worker's own cap is not 8 KB");
    let limits = wrangler()["ratelimits"].as_array().expect("[[ratelimits]]").clone();
    assert_eq!(limits.len(), 1, "one rate limit");
    assert_eq!(limits[0]["name"].as_str(), Some("HEAL_IP_LIMIT"));
    assert_eq!(limits[0]["simple"]["limit"].as_integer(), Some(10), "ten heals a minute an address");
    assert_eq!(limits[0]["simple"]["period"].as_integer(), Some(60));
}

/// The key reaches the worker as a secret and a local `.dev.vars` only; neither is in the tree.
#[test]
fn the_key_never_enters_the_tree() {
    let ignore = read(".gitignore");
    assert!(ignore.lines().any(|l| l.trim() == ".dev.vars") && ignore.lines().any(|l| l.trim() == "**/.dev.vars"), ".gitignore does not ignore .dev.vars everywhere");
    for file in ["services/heal/wrangler.toml", "services/heal/src/index.js", "services/heal/src/heal.js", "services/heal/README.md", "apps/logicaffeine_web/src/ui/pages/heal.rs"] {
        let text = read(file);
        assert!(!text.contains("nvapi-"), "{file} carries what looks like a key");
    }
    let index = read("services/heal/src/index.js");
    assert!(index.contains("env.NVIDIA_API_KEY"), "the worker does not read the key from its environment");
    assert!(!index.contains("NVIDIA_API_KEY ="), "the worker assigns the key");
    let tracked = std::process::Command::new("git").args(["-C", root().to_str().unwrap(), "ls-files", "--", "services/heal"]).output().expect("git");
    let listed = String::from_utf8_lossy(&tracked.stdout);
    assert!(!listed.contains(".dev.vars"), "a .dev.vars is tracked: {listed}");
}

/// The deploy reruns the worker's tests on every change to the worker and ships only after them,
/// and says how the secret is set.
#[test]
fn the_deploy_tests_first_and_names_the_secret() {
    let workflow = read(".github/workflows/deploy-heal.yml");
    assert!(workflow.contains("services/heal/**"), "the deploy does not watch the worker's files");
    assert!(workflow.contains("npx vitest run"), "the deploy does not run the worker's tests");
    assert!(workflow.contains("needs: test"), "the deploy does not wait for the tests");
    assert!(workflow.contains("wrangler secret put NVIDIA_API_KEY"), "the deploy does not say how the secret is set");
    assert!(workflow.contains("npx wrangler deploy"), "nothing deploys");
    let readme = read("services/heal/README.md");
    assert!(readme.contains("wrangler secret put NVIDIA_API_KEY") && readme.contains(".dev.vars"), "the README does not say how the key reaches the worker");
}

/// The worker's prompt mends a fragment as lines only, which is what the Studio sends: the lines a
/// reader wrote, never the file around them.
#[test]
fn a_fragment_is_what_the_studio_sends_and_what_the_worker_mends() {
    let app = read("apps/logicaffeine_web/src/ui/pages/heal.rs");
    assert!(app.contains("pub fragment: bool"), "the app's request has no fragment flag");
    let workbench = read("apps/logicaffeine_web/src/ui/pages/studio_workbench.rs");
    assert!(workbench.contains("heal::edited_regions(") && workbench.contains("heal::splice(") && workbench.contains(", true).await"), "the Studio does not send the edited regions as fragments");
    let heal = read("services/heal/src/heal.js");
    assert!(heal.contains("fragment: body.fragment === true"), "the worker does not read the fragment flag");
    assert!(heal.contains("mended lines only"), "the worker's prompt does not mend a fragment as lines only");
    let index = read("services/heal/src/index.js");
    assert!(index.contains("promptFor(asked.mode, asked.text, asked.fragment)"), "the worker does not pass the flag to the prompt");
}
