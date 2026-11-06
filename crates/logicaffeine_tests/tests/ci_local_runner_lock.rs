//! ★ LOCK — `scripts/run-ci-locally.py`, the shipped CI workflow run on this machine before it runs on GitHub. ★
//!
//! The runner executes `.github/workflows/test.yml` itself: every `run:` step verbatim, every `${{ }}` with GitHub's
//! semantics, every `uses:` action through its local equivalent. Its value is that a local green means a GitHub green,
//! so what it must never do is guess. Its `plan` command parses the workflow, evaluates what it can, and refuses (exit
//! 2, naming the culprit) any action, runner label, key, trigger, context or function it has no faithful meaning for.
//! This lock holds both halves:
//!
//! - the shipped workflow plans on every trigger it declares, every job listed, and the job that publishes (a `write`
//!   permission) never runs locally;
//! - each kind of thing the runner cannot reproduce is refused by name, so a new action or a new runner label in the
//!   workflow fails here, not as a silent difference between the local run and CI;
//! - static `if:` conditions follow GitHub's expression rules (case-insensitive string equality, `&&` over `||`, `!`,
//!   the implicit `success()`, `contains`), a value of several expressions is each in its place, and every value `plan`
//!   can evaluate before a run it evaluates there;
//! - the runner's container installs nothing the workflow installs for itself, so each job's own install step runs
//!   locally as it runs on the hosted image.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test ci_local_runner_lock`

#![cfg(all(unix, not(target_arch = "wasm32")))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn plan(workflow: &Path, events: &[&str]) -> Output {
    let args: Vec<String> = events.iter().flat_map(|e| ["--event".to_string(), e.to_string()]).collect();
    plan_with(workflow, &args.iter().map(String::as_str).collect::<Vec<_>>())
}

fn plan_with(workflow: &Path, args: &[&str]) -> Output {
    Command::new("python3")
        .arg(repo_root().join("scripts/run-ci-locally.py"))
        .arg("plan")
        .arg("--workflow")
        .arg(workflow)
        .args(args)
        .output()
        .expect("python3 runs the local CI runner")
}

/// A workflow written to a temp directory whole: `on` and `jobs` as the caller gives them.
fn workflow(yaml: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("wf.yml");
    std::fs::write(&path, yaml).unwrap();
    (dir, path)
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// A workflow written to a temp directory: `jobs` is the YAML under `jobs:`, indented two spaces.
fn fixture(jobs: &str) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("wf.yml");
    std::fs::write(&path, format!("name: Fixture\non:\n  push:\n    branches: [main]\n  schedule:\n    - cron: '0 7 * * *'\njobs:\n{jobs}")).unwrap();
    (dir, path)
}

const PLAIN_STEPS: &str = "    steps:\n      - uses: actions/checkout@v4\n      - run: echo ok\n";

fn job(name: &str, extra: &str) -> String {
    format!("  {name}:\n    runs-on: ubuntu-24.04\n{extra}{PLAIN_STEPS}")
}

fn refused(jobs: &str, culprit: &str) {
    let (_dir, wf) = fixture(jobs);
    let out = plan(&wf, &[]);
    let err = text(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "the runner must refuse, naming `{culprit}`: {err}{}", text(&out.stdout));
    assert!(err.contains(culprit), "the refusal names `{culprit}`:\n{err}");
}

/// The verdict `plan` gives job `key` under `event`.
fn verdict(stdout: &str, event: &str, key: &str) -> Option<String> {
    let mut current = None;
    for line in stdout.lines() {
        if let Some(ev) = line.strip_suffix(':').filter(|l| !l.starts_with(' ')) {
            current = Some(ev.to_string());
            continue;
        }
        if current.as_deref() == Some(event) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let width = key.split_whitespace().count();
            if cols.len() > width && cols[..width].join(" ") == key {
                return Some(cols[width + 1..].join(" "));
            }
        }
    }
    None
}

/// The job names under the shipped workflow's `jobs:` (two-space keys) and its triggers (two-space keys under `on:`).
fn shipped_jobs_and_triggers() -> (Vec<String>, Vec<String>) {
    let src = std::fs::read_to_string(repo_root().join(".github/workflows/test.yml")).unwrap();
    let mut section = "";
    let (mut jobs, mut triggers) = (Vec::new(), Vec::new());
    for line in src.lines() {
        if !line.starts_with(' ') && line.ends_with(':') {
            section = line.trim_end_matches(':');
            continue;
        }
        if !line.starts_with(' ') && !line.trim().is_empty() && !line.starts_with('#') {
            section = "";
        }
        let Some(rest) = line.strip_prefix("  ") else { continue };
        if rest.starts_with(' ') || rest.starts_with('#') {
            continue;
        }
        let Some((key, _)) = rest.split_once(':') else { continue };
        match section {
            "jobs" => jobs.push(key.to_string()),
            "on" => triggers.push(key.to_string()),
            _ => {}
        }
    }
    (jobs, triggers)
}

#[test]
fn the_shipped_workflow_plans_on_every_trigger_it_declares() {
    let (jobs, triggers) = shipped_jobs_and_triggers();
    assert!(jobs.len() >= 5 && !triggers.is_empty(), "read jobs {jobs:?} and triggers {triggers:?} from test.yml");
    let out = plan(&repo_root().join(".github/workflows/test.yml"), &[]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "the runner refuses the shipped workflow:\n{}", text(&out.stderr));
    for event in &triggers {
        for name in &jobs {
            let found = stdout.lines().any(|l| {
                l.split_whitespace().next().is_some_and(|k| k == name)
            });
            assert!(found, "{event}: job `{name}` is missing from the plan:\n{stdout}");
        }
        assert!(stdout.contains(&format!("{event}:")), "the plan covers `{event}`:\n{stdout}");
    }
}

#[test]
fn the_job_that_publishes_never_runs_locally() {
    let out = plan(&repo_root().join(".github/workflows/test.yml"), &["push"]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        verdict(&stdout, "push", "badge").as_deref(),
        Some("not run locally: holds a write permission"),
        "the badge job force-pushes with a write token; locally it must not run:\n{stdout}"
    );
    let (_dir, wf) = fixture(&job("release", "    permissions:\n      contents: write\n"));
    let stdout = text(&plan(&wf, &["push"]).stdout);
    assert_eq!(verdict(&stdout, "push", "release").as_deref(), Some("not run locally: holds a write permission"));
}

#[test]
fn an_action_without_a_local_equivalent_is_refused() {
    refused(
        "  build:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: docker/build-push-action@v6\n        with:\n          push: false\n",
        "docker/build-push-action@v6",
    );
}

#[test]
fn a_runner_label_without_a_profile_is_refused() {
    refused(&job("gpu", "").replace("ubuntu-24.04", "self-hosted"), "self-hosted");
}

/// A hosted label this machine cannot be (another operating system, another architecture) is named in the plan, and
/// what needs it does not run either: a leg left out is reported, never counted as passed.
#[test]
fn a_runner_this_machine_cannot_host_is_named_and_never_run() {
    let jobs = [
        job("mac", "").replace("ubuntu-24.04", "macos-latest"),
        job("win", "").replace("ubuntu-24.04", "windows-latest"),
        job("arm", "").replace("ubuntu-24.04", "ubuntu-22.04-arm"),
        job("after_mac", "    needs: mac\n"),
        job("jammy", "").replace("ubuntu-24.04", "ubuntu-22.04"),
    ]
    .concat();
    let (_dir, wf) = fixture(&jobs);
    let out = plan(&wf, &["push"]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    for (key, want) in [
        ("mac", "not run locally: needs a macOS host"),
        ("win", "not run locally: needs a Windows host"),
        ("arm", "not run locally: needs an arm64 host"),
        ("after_mac", "not run locally: needs mac, which does not run locally"),
        ("jammy", "runs"),
    ] {
        assert_eq!(verdict(&stdout, "push", key).as_deref(), Some(want), "{key}:\n{stdout}");
    }
}

/// A step in a shell the local image lacks is refused only where it would run: one its own `if:` keeps off this
/// machine's runner (a PowerShell packaging step for Windows legs) plans, an unconditional one does not.
#[test]
fn a_shell_it_lacks_is_refused_only_where_a_step_would_run_it() {
    let (_dir, wf) = fixture(
        "  pkg:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: echo zip\n        if: runner.os == 'Windows'\n        \
         shell: pwsh\n      - run: echo tar\n        if: runner.os != 'Windows'\n        shell: bash\n",
    );
    let out = plan(&wf, &["push"]);
    assert!(out.status.success(), "a pwsh step kept off Linux by its if: plans:\n{}", text(&out.stderr));
    refused(
        "  s:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: echo hi\n        if: runner.os == 'Linux'\n        shell: pwsh\n",
        "pwsh",
    );
}

/// `actions/cache` restores nothing locally (each job starts cold, as a cache miss does on GitHub) and saves nothing;
/// an input it has no meaning for is refused.
#[test]
fn a_cache_step_is_a_cold_start() {
    let (_dir, wf) = fixture(
        "  c:\n    runs-on: ubuntu-24.04\n    steps:\n      - uses: actions/checkout@v4\n      - uses: actions/cache@v4\n        \
         with:\n          path: |\n            ~/.cargo/registry/index/\n            target/\n          \
         key: ${{ runner.os }}-${{ hashFiles('**/Cargo.lock') }}\n",
    );
    let out = plan(&wf, &["push"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
}

#[test]
fn a_key_the_runner_has_no_meaning_for_is_refused() {
    refused(&job("db", "    services:\n      pg:\n        image: postgres\n"), "services");
    refused(&job("ctr", "    container: ubuntu:22.04\n"), "container");
    refused(
        "  s:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: echo hi\n        shell: pwsh\n",
        "pwsh",
    );
}

#[test]
fn a_trigger_the_runner_cannot_replay_is_refused() {
    let dir = TempDir::new().unwrap();
    let wf = dir.path().join("wf.yml");
    std::fs::write(&wf, format!("name: F\non:\n  workflow_run:\n    workflows: [Tests]\njobs:\n{}", job("a", ""))).unwrap();
    let out = plan(&wf, &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("workflow_run"), "{}", text(&out.stderr));
}

#[test]
fn a_context_or_function_it_cannot_evaluate_is_refused() {
    refused(&job("s", "    env:\n      TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}\n"), "secrets");
    refused(&job("v", "    if: vars.ENABLED == 'true'\n"), "vars");
}

/// `hashFiles` digests the workspace files its patterns match as the runner does: each file's SHA-256, in path order,
/// fed to one SHA-256; the empty string when nothing matches.
#[test]
fn hashfiles_digests_the_matched_files() {
    let ws = TempDir::new().unwrap();
    std::fs::create_dir_all(ws.path().join("a/b")).unwrap();
    std::fs::write(ws.path().join("Cargo.lock"), "root\n").unwrap();
    std::fs::write(ws.path().join("a/b/Cargo.lock"), "nested\n").unwrap();
    std::fs::write(ws.path().join("a/Cargo.toml"), "unmatched\n").unwrap();
    let eval = |expr: &str| {
        let out = Command::new("python3")
            .arg(repo_root().join("scripts/run-ci-locally.py"))
            .args(["eval", "--workspace"])
            .arg(ws.path())
            .arg(expr)
            .output()
            .unwrap();
        assert!(out.status.success(), "{expr}: {}", text(&out.stderr));
        text(&out.stdout).trim().to_string()
    };
    let digest = |bytes: &[u8]| {
        let mut child = Command::new("sha256sum")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        let out = child.wait_with_output().unwrap();
        text(&out.stdout).split_whitespace().next().unwrap().to_string()
    };
    let raw = |hex: &str| (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect::<Vec<u8>>();
    let mut both = raw(&digest(b"root\n"));
    both.extend(raw(&digest(b"nested\n")));
    let mut ordered = raw(&digest(b"nested\n"));
    ordered.extend(raw(&digest(b"root\n")));
    // Path order: `Cargo.lock` sorts before `a/b/Cargo.lock`.
    assert_eq!(eval("hashFiles('**/Cargo.lock')"), digest(&both));
    assert_ne!(digest(&both), digest(&ordered), "the order is part of the digest");
    assert_eq!(eval("hashFiles('a/**/Cargo.lock')"), digest(&raw(&digest(b"nested\n"))));
    assert_eq!(eval("hashFiles('**/Cargo.lock', 'a/Cargo.toml')").len(), 64);
    assert_eq!(eval("hashFiles('**/missing')"), "");
}

#[test]
fn a_matrix_shape_it_cannot_expand_is_refused() {
    refused(
        &job("m", "    strategy:\n      matrix: ${{ fromJSON(needs.setup.outputs.matrix) }}\n"),
        "matrix given by an expression",
    );
}

/// `include` and `exclude` expand as GitHub documents: an exclusion removes every combination it matches; an
/// inclusion extends each combination whose original values it does not overwrite, and becomes a combination of its
/// own when it extends none; a matrix of inclusions alone is one leg per inclusion.
#[test]
fn include_and_exclude_expand_as_github_does() {
    let jobs = [
        job(
            "grid",
            "    strategy:\n      matrix:\n        os: [a, b]\n        v: [1, 2]\n        exclude:\n          - os: b\n            v: 2\n        \
             include:\n          - os: a\n            extra: x\n          - v: 2\n            flag: y\n          - os: c\n            v: 9\n",
        ),
        job(
            "list",
            "    strategy:\n      matrix:\n        include:\n          - { target: t1, os: ubuntu-24.04 }\n          - { target: t2, os: ubuntu-22.04 }\n",
        ),
    ]
    .concat();
    let (_dir, wf) = fixture(&jobs);
    let out = plan(&wf, &["push"]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    for leg in ["grid (a, 1, x)", "grid (a, 2, x, y)", "grid (b, 1)", "grid (c, 9)", "list (t1, ubuntu-24.04)", "list (t2, ubuntu-22.04)"] {
        assert_eq!(verdict(&stdout, "push", leg).as_deref(), Some("runs"), "{leg}:\n{stdout}");
    }
    assert_eq!(verdict(&stdout, "push", "grid (b, 2)"), None, "excluded:\n{stdout}");
    assert_eq!(stdout.lines().filter(|l| l.trim_start().starts_with("grid (")).count(), 4, "{stdout}");
}

/// A dispatch reads the inputs it declares, their defaults when not given; an input it does not declare is refused,
/// and a tag ref is a tag.
#[test]
fn a_dispatch_reads_its_declared_inputs_and_a_tag_is_a_tag() {
    let (_dir, wf) = workflow(
        "name: D\non:\n  push:\n    tags: ['v*']\n  workflow_dispatch:\n    inputs:\n      tag:\n        required: false\n        default: ''\n\
         env:\n  RELEASE_TAG: ${{ inputs.tag || (github.ref_type == 'tag' && github.ref_name || '') }}\njobs:\n\
         \x20 given:\n    runs-on: ubuntu-24.04\n    if: startsWith(inputs.tag, 'v')\n    steps:\n      - run: echo $RELEASE_TAG\n\
         \x20 tagged:\n    runs-on: ubuntu-24.04\n    if: github.ref_type == 'tag'\n    steps:\n      - run: echo tag\n",
    );
    let given = text(&plan_with(&wf, &["--event", "workflow_dispatch", "--input", "tag=v0.11.0"]).stdout);
    assert_eq!(verdict(&given, "workflow_dispatch", "given").as_deref(), Some("runs"), "{given}");
    assert_eq!(verdict(&given, "workflow_dispatch", "tagged").as_deref(), Some("skipped by its if:"), "{given}");
    let bare = text(&plan_with(&wf, &["--event", "workflow_dispatch"]).stdout);
    assert_eq!(verdict(&bare, "workflow_dispatch", "given").as_deref(), Some("skipped by its if:"), "{bare}");
    let pushed = text(&plan_with(&wf, &["--event", "push", "--ref-name", "refs/tags/v0.11.0"]).stdout);
    assert_eq!(verdict(&pushed, "push", "tagged").as_deref(), Some("runs"), "{pushed}");
    let out = plan_with(&wf, &["--event", "workflow_dispatch", "--input", "tga=v1"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out.stdout));
    assert!(text(&out.stderr).contains("tga"), "{}", text(&out.stderr));
}

/// The shipped release workflow plans as the dispatch that releases a pushed tag: the Linux legs run here, every
/// macOS, Windows and arm64 leg is named as not run, the job that publishes never runs, nor what needs it.
#[test]
fn the_release_workflow_plans_as_a_dispatch_of_its_tag() {
    let out = plan_with(
        &repo_root().join(".github/workflows/release.yml"),
        &["--event", "workflow_dispatch", "--input", "tag=v0.11.0"],
    );
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "the runner refuses release.yml:\n{}", text(&out.stderr));
    let ev = "workflow_dispatch";
    let leg = |prefix: &str| -> Vec<(String, String)> {
        stdout
            .lines()
            .map(str::trim_start)
            .filter(|l| l.starts_with(prefix))
            .map(|l| {
                let (key, rest) = l.split_once(')').unwrap();
                (format!("{key})"), rest.split_whitespace().skip(1).collect::<Vec<_>>().join(" "))
            })
            .collect()
    };
    let cli = leg("build-cli (");
    assert_eq!(cli.len(), 10, "{stdout}");
    for (key, v) in &cli {
        let want = if key.contains("ubuntu-22.04-arm") {
            "not run locally: needs an arm64 host"
        } else if key.contains("macos") {
            "not run locally: needs a macOS host"
        } else if key.contains("windows") {
            "not run locally: needs a Windows host"
        } else {
            "runs"
        };
        assert_eq!(v, want, "{key}:\n{stdout}");
    }
    assert_eq!(cli.iter().filter(|(_, v)| v == "runs").count(), 2, "linux-x64 lean and full run here:\n{stdout}");
    let vsix = leg("package-vscode (");
    assert_eq!(vsix.len(), 6, "{stdout}");
    for (key, v) in &vsix {
        if key.contains("darwin") || key.contains("win32") {
            assert!(
                v.starts_with("not run locally: needs the artifact logicaffeine-lsp-"),
                "{key} packages a server built on a host this machine is not: {v}\n{stdout}"
            );
        } else {
            assert_eq!(v, "runs", "{key}:\n{stdout}");
        }
    }
    assert_eq!(verdict(&stdout, ev, "github-release").as_deref(), Some("not run locally: holds a write permission"));
    let installs = leg("e2e-install (");
    assert_eq!(installs.len(), 5, "{stdout}");
    for (key, v) in installs {
        assert!(v.starts_with("not run locally"), "{key} installs the published release: {v}\n{stdout}");
    }
    assert!(
        verdict(&stdout, ev, "e2e-install-windows").is_some_and(|v| v.starts_with("not run locally")),
        "{stdout}"
    );
}

#[test]
fn static_conditions_follow_githubs_expression_rules() {
    let jobs = [
        job("case_folded", "    if: github.event_name == 'PUSH'\n"),
        job("negated", "    if: github.event_name != 'schedule' && !false\n"),
        job("precedence", "    if: false && false || true\n"),
        job("not_schedule", "    if: ${{ github.event_name != 'schedule' }}\n"),
        job("contains", "    if: contains('push pull_request', github.event_name)\n"),
        job("on_main", "    if: github.ref == 'refs/heads/main'\n"),
        job("never", "    if: github.event_name == 'release'\n"),
    ]
    .concat();
    let (_dir, wf) = fixture(&jobs);
    let out = plan(&wf, &["push", "schedule"]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let expect = [
        ("push", "case_folded", "runs"),
        ("schedule", "case_folded", "skipped by its if:"),
        ("push", "negated", "runs"),
        ("schedule", "negated", "skipped by its if:"),
        ("push", "precedence", "runs"),
        ("schedule", "not_schedule", "skipped by its if:"),
        ("push", "contains", "runs"),
        ("schedule", "contains", "skipped by its if:"),
        ("push", "on_main", "runs"),
        ("push", "never", "skipped by its if:"),
    ];
    for (event, key, want) in expect {
        assert_eq!(verdict(&stdout, event, key).as_deref(), Some(want), "{event} {key}:\n{stdout}");
    }
}

/// A value made of several expressions is each of them in its place, as GitHub reads `${{ runner.temp }}/${{ env.X }}`;
/// `plan` evaluates every value it can before a run, so one the runner cannot read fails there, not an hour in.
#[test]
fn a_value_of_several_expressions_evaluates_each() {
    let (_dir, wf) = fixture(
        "  a:\n    runs-on: ubuntu-24.04\n    env:\n      ARCHIVE: a.tar.zst\n    steps:\n      - uses: actions/checkout@v4\n      \
         - uses: actions/upload-artifact@v4\n        with:\n          name: arc\n          \
         path: ${{ runner.temp }}/${{ env.ARCHIVE }}\n      - run: echo ${{ github.event_name }}-${{ github.ref_name }}\n",
    );
    let out = plan(&wf, &["push"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    refused(
        "  b:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: echo ${{ fromJSON('not json') }}\n",
        "does not evaluate",
    );
}

#[test]
fn every_matrix_leg_is_its_own_job() {
    let (_dir, wf) = fixture(&job("test", "    strategy:\n      fail-fast: false\n      matrix:\n        partition: [1, 2, 3]\n"));
    let stdout = text(&plan(&wf, &["push"]).stdout);
    for leg in ["test (1)", "test (2)", "test (3)"] {
        assert_eq!(verdict(&stdout, "push", leg).as_deref(), Some("runs"), "{leg}:\n{stdout}");
    }
}

/// The packages a workflow step installs with `apt-get install`, for every step of a shipped workflow.
fn workflow_apt_packages(file: &str) -> BTreeSet<String> {
    let src = std::fs::read_to_string(repo_root().join(".github/workflows").join(file)).unwrap();
    let mut out = BTreeSet::new();
    for line in src.lines() {
        if let Some((_, rest)) = line.split_once("apt-get install") {
            for word in rest.split_whitespace() {
                if !word.starts_with('-') && word != "&&" {
                    out.insert(word.to_string());
                }
            }
        }
    }
    out
}

/// The packages one of the runner's container images installs.
fn image_apt_packages(image: &str) -> BTreeSet<String> {
    let src = std::fs::read_to_string(repo_root().join("scripts/test/ci-runner").join(format!("{image}.Dockerfile"))).unwrap();
    let block: String = src
        .split("apt-get install -y --no-install-recommends")
        .nth(1)
        .and_then(|b| b.split("&& rm -rf").next())
        .expect("the Dockerfile has one apt-get install block")
        .to_string();
    block
        .split_whitespace()
        .filter(|w| *w != "\\")
        .map(str::to_string)
        .collect()
}

/// Each image with the workflow whose jobs run on it, and the packages a step installs that the hosted image already
/// carries (actions/runner-images, images/ubuntu/Ubuntu2404-Readme.md and Ubuntu2204-Readme.md, "Installed apt
/// packages" and the tools list), so the local image carries them too.
const IMAGES: &[(&str, &str, &[&str])] = &[("ubuntu-24.04", "test.yml", &["jq"]), ("ubuntu-22.04", "release.yml", &["cmake"])];

#[test]
fn the_image_installs_nothing_the_workflow_installs_for_itself() {
    for (image_name, file, on_the_hosted_image) in IMAGES {
        let (workflow, image) = (workflow_apt_packages(file), image_apt_packages(image_name));
        assert!(workflow.len() >= 4, "read {file}'s installs: {workflow:?}");
        assert!(image.contains("python3-yaml") && image.contains("sudo"), "read {image_name}'s installs: {image:?}");
        let both: Vec<&String> =
            workflow.intersection(&image).filter(|p| !on_the_hosted_image.contains(&p.as_str())).collect();
        assert!(
            both.is_empty(),
            "the {image_name} container preinstalls what a {file} step installs, so that step can no longer fail \
             locally the way it fails on the hosted image: {both:?}"
        );
    }
}

#[test]
fn the_image_carries_no_tool_the_hosted_runner_lacks() {
    for (image_name, _, _) in IMAGES {
        let image = image_apt_packages(image_name);
        for absent in ["yosys", "verilator", "nvidia-cuda-toolkit", "strace", "iverilog", "z3", "qemu-user", "gcc-13"] {
            assert!(
                !image.contains(absent),
                "`{absent}` is not on the hosted {image_name} image; a test that finds it locally runs a path CI skips"
            );
        }
    }
}
