//! The documented deploy build is the one CI ships.
//!
//! `.github/workflows/deploy-frontend.yml` is the only recipe that reaches logicaffeine.com.
//! The web README and the scripts' usage notes restate it, and when CI turned wasm-split off
//! (the fork's splitter miscompiles the wasm) those copies kept `--wasm-split --features split`
//! and never said which `dx` to build with. An artifact built from the README with a stock
//! crates.io `dx` crashed on boot (`reading 'listening'`) and stripped every URL's query and
//! fragment — failures production never had, diagnosed as production bugs. These tests hold
//! every restatement to the workflow, so reproducing the deploy locally reproduces the deploy.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

const WORKFLOW: &str = ".github/workflows/deploy-frontend.yml";
const README: &str = "apps/logicaffeine_web/README.md";

/// Shell lines that do something: comment lines dropped, trailing comments cut.
fn command_lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(|line| line.split(" #").next().unwrap().trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// Every release SSG `dx build` written in `text`, in prose or in a command, as its flags:
/// each flag with its value, sorted, the `-p` package left out (every one names this app). A
/// build ends where its code span, comment, or shell command ends — at a `;`, `)`, `"`, `&&` or
/// `|` — so the words after it are never read as its flags.
fn deploy_builds(text: &str) -> Vec<Vec<String>> {
    let mut builds = Vec::new();
    for line in text.lines() {
        let mut rest = line;
        while let Some(at) = rest.find("dx build") {
            let command = &rest[at..];
            let end = command.find(['`', '#', ';', ')', '"', '&', '|']).unwrap_or(command.len());
            let flags = build_flags(&command[..end]);
            if flags.iter().any(|f| f == "--release") && flags.iter().any(|f| f == "--ssg") {
                builds.push(flags);
            }
            rest = &command[end..];
        }
    }
    builds
}

fn build_flags(command: &str) -> Vec<String> {
    let mut flags: Vec<String> = Vec::new();
    let mut words = command.split_whitespace().skip(2).filter(|w| *w != "\\").peekable();
    while let Some(word) = words.next() {
        let value = match words.peek() {
            Some(next) if !next.starts_with('-') => words.next(),
            _ => None,
        };
        if word == "-p" || word == "--package" {
            continue;
        }
        flags.push(match value {
            Some(value) => format!("{word} {value}"),
            None => word.to_string(),
        });
    }
    flags.sort();
    flags
}

fn ci_build() -> Vec<String> {
    let builds = deploy_builds(&command_lines(&read(WORKFLOW)).join("\n"));
    assert_eq!(builds.len(), 1, "{WORKFLOW} should run exactly one release SSG `dx build`; found {builds:?}");
    builds.into_iter().next().unwrap()
}

/// The `./scripts/*.sh` a block of shell runs after its deploy `dx build`, in order.
fn gates_after_build(lines: &[&str]) -> Vec<String> {
    let start = lines
        .iter()
        .position(|line| !deploy_builds(line).is_empty())
        .expect("no release SSG `dx build` in the block");
    lines[start + 1..]
        .iter()
        .flat_map(|line| line.split_whitespace())
        .filter_map(|word| word.strip_prefix("./scripts/"))
        .filter(|script| script.ends_with(".sh"))
        .map(str::to_string)
        .collect()
}

/// The README's fenced shell block that holds the deploy build, as its command lines.
fn readme_deploy_block() -> Vec<String> {
    let readme = read(README);
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in readme.lines() {
        if line.trim_start().starts_with("```") {
            match current.take() {
                Some(block) => blocks.push(block),
                None => current = Some(Vec::new()),
            }
        } else if let Some(block) = current.as_mut() {
            block.push(line.to_string());
        }
    }
    let mut deploy: Vec<Vec<String>> =
        blocks.into_iter().filter(|block| !deploy_builds(&block.join("\n")).is_empty()).collect();
    assert_eq!(deploy.len(), 1, "{README} should hold exactly one code block with the deploy build");
    let block = deploy.pop().unwrap();
    command_lines(&block.join("\n")).into_iter().map(str::to_string).collect()
}

#[test]
fn the_readme_deploy_build_is_the_one_ci_ships() {
    let ci = ci_build();
    let block = readme_deploy_block();
    let readme = deploy_builds(&block.join("\n"));
    assert_eq!(readme, vec![ci.clone()], "{README} documents a deploy build CI does not run; CI builds with {ci:?}");
}

/// Every shell, node and python script under `scripts/`, its category folders included.
fn scripts() -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut dirs = vec![repo_root().join("scripts")];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if matches!(path.extension().and_then(|e| e.to_str()), Some("sh" | "mjs" | "py")) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn every_script_note_names_the_build_ci_ships() {
    let ci = ci_build();
    let scripts = scripts();
    assert!(
        scripts.iter().any(|path| path.ends_with("scripts/web/merge-ssg.sh")),
        "the walk under scripts/ never reached the web scripts"
    );
    let root = repo_root();
    let mut wrong = Vec::new();
    for path in scripts {
        let text = std::fs::read_to_string(&path).unwrap();
        for build in deploy_builds(&text) {
            if build != ci {
                wrong.push(format!("{}: {build:?}", path.strip_prefix(&root).unwrap().display()));
            }
        }
    }
    assert!(wrong.is_empty(), "scripts name a deploy build CI does not run (CI: {ci:?}):\n{}", wrong.join("\n"));
}

#[test]
fn the_readme_runs_the_gates_ci_runs_in_ci_order() {
    let workflow = read(WORKFLOW);
    let ci = gates_after_build(&command_lines(&workflow));
    let block = readme_deploy_block();
    let readme = gates_after_build(&block.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(ci.contains(&"web/merge-ssg.sh".to_string()), "the workflow's gates were not found: {ci:?}");
    assert_eq!(readme, ci, "{README} runs a different sequence after the deploy build than {WORKFLOW}");
}

/// The workflow's `cargo install … dioxus-cli` command, without its `|| true`.
fn ci_dx_install() -> String {
    let workflow = read(WORKFLOW);
    let installs: Vec<String> = command_lines(&workflow)
        .into_iter()
        .filter_map(|line| line.find("cargo install").map(|at| &line[at..]))
        .filter(|command| command.contains("dioxus-cli"))
        .map(|command| command.split("||").next().unwrap().trim().to_string())
        .collect();
    assert_eq!(installs.len(), 1, "{WORKFLOW} should install dx exactly once; found {installs:?}");
    installs.into_iter().next().unwrap()
}

#[test]
fn the_readme_installs_the_dx_ci_builds_with() {
    let install = ci_dx_install();
    let block = readme_deploy_block();
    assert!(
        block.iter().any(|line| line == &install),
        "{README} must install the dx the deploy is built with — a stock crates.io dx is a different \
         CLI than the one that builds logicaffeine.com:\n  {install}"
    );
}

#[test]
fn ci_builds_with_the_dx_of_the_fork_the_app_is_pinned_to() {
    let install = ci_dx_install();
    let ci_rev = install
        .split_whitespace()
        .skip_while(|w| *w != "--rev")
        .nth(1)
        .unwrap_or_else(|| panic!("{WORKFLOW} installs dx without a pinned --rev: {install}"));
    let manifest = read("Cargo.toml");
    let pin = manifest
        .lines()
        .find(|line| line.starts_with("dioxus = {"))
        .expect("the root Cargo.toml pins `dioxus`");
    let app_rev = pin.split("rev = \"").nth(1).and_then(|rest| rest.split('"').next()).expect("dioxus is pinned by rev");
    assert_eq!(ci_rev, app_rev, "CI builds with dx from one fork rev while the app links dioxus from another");
    assert!(
        install.contains("github.com/Brahmastra-Labs/dioxus") && pin.contains("github.com/Brahmastra-Labs/dioxus"),
        "dx and the app must both come from the Brahmastra-Labs dioxus fork"
    );
}

#[test]
fn the_flag_reader_ignores_order_and_the_package() {
    assert_eq!(
        build_flags("dx build -p logicaffeine-web --release --ssg --fullstack --wasm-split --features split"),
        build_flags("dx build --features split --release --fullstack --ssg --wasm-split"),
    );
    assert_ne!(
        build_flags("dx build -p logicaffeine-web --release --ssg --fullstack"),
        build_flags("dx build -p logicaffeine-web --release --ssg --fullstack --wasm-split --features split"),
    );
    assert_eq!(deploy_builds("# Run it AFTER `dx build --release --ssg --fullstack`.").len(), 1);
    assert!(deploy_builds("`dx build --ssg` prerenders through the server binary").is_empty(), "a bare --ssg mention is prose, not the deploy build");
    assert!(deploy_builds("dx build -p logicaffeine-web --release").is_empty(), "a plain release build is not the deploy build");
}

#[test]
fn the_flag_reader_stops_where_the_shell_command_stops() {
    let build = deploy_builds("dx build -p logicaffeine-web --release --ssg --fullstack");
    assert_eq!(build.len(), 1);
    for line in [
        "elif ! dx build -p logicaffeine-web --release --ssg --fullstack; then",
        r#"echo "── as CI builds it (dx build -p logicaffeine-web --release --ssg --fullstack) ──""#,
        r#"run "dx build -p logicaffeine-web --release --ssg --fullstack" now"#,
        "dx build -p logicaffeine-web --release --ssg --fullstack && ./scripts/web/merge-ssg.sh",
        "dx build -p logicaffeine-web --release --ssg --fullstack || exit 1",
        "dx build -p logicaffeine-web --release --ssg --fullstack | tee build.log",
    ] {
        assert_eq!(deploy_builds(line), build, "read past the end of the command in: {line}");
    }
}
