//! The GitHub release is assembled from artifacts, and every one it copies exists on every tag build.
//!
//! `release.yml`'s `github-release` job downloads the artifacts of the jobs it `needs` and copies them into
//! the release with no fallback, so a copy whose artifact was never uploaded fails the job and there is no
//! release at all. An upload behind an `if:` is exactly that artifact on the build where the condition is
//! false — the brochure PDFs were uploaded only where `marketing/` existed, and the public tree has none.
//!
//! Invariants: every artifact directory the release copies from is uploaded by a job `github-release` needs,
//! by a step with no `if:`; and the release and test workflows carry no marketing (Tristen, 2026-09-26: the
//! brochures do not ship on GitHub releases).
//!
//! The release body is the tag's CHANGELOG section, and GitHub refuses a body over 125,000 characters: the
//! 0.11.0 section is four times that, so a release step that published it whole would fail after every build
//! had passed. Invariant: the notes step, run as the runner runs it, writes every released section within the
//! limit; a section that fits ships whole, and a longer one keeps every heading, each heading's first entries
//! verbatim, and a link to the full notes in the tag's CHANGELOG.md in place of the rest.
//!
//! A re-run of a tag's workflow runs the workflow file of that tag, so a release workflow fixed after the tag
//! could never release it. Invariant: a dispatch names the tag to release, every job checks that tag out, and
//! the release names it and publishes it, drafts included.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test release_assets_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/workflows.rs"]
mod workflows;

use std::path::PathBuf;

use workflows::{jobs, runs_on, script, steps, workflow};

/// Every value a matrix key takes in `job` (`target: x` or `{ …, target: x, … }`).
fn matrix_values(job: &str, key: &str) -> Vec<String> {
    let needle = format!("{key}:");
    let mut out = Vec::new();
    for line in job.lines() {
        let mut rest = line;
        while let Some(i) = rest.find(&needle) {
            let before = rest[..i].chars().last();
            rest = &rest[i + needle.len()..];
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                continue;
            }
            let value: String = rest
                .trim_start()
                .chars()
                .take_while(|c| !matches!(c, ',' | '}' | ' ' | '#'))
                .collect();
            if !value.is_empty() {
                out.push(value);
            }
        }
    }
    out
}

/// The artifact names `job` uploads, each with whether its step is unconditional, matrix references expanded.
fn uploads(job: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let is_upload = |s: &&String| {
        s.lines().any(|l| {
            l.trim_start()
                .trim_start_matches("- ")
                .starts_with("uses: actions/upload-artifact")
        })
    };
    for step in steps(job).iter().filter(is_upload) {
        let unconditional = !step.lines().any(|l| l.trim_start().starts_with("if:"));
        let name = step
            .lines()
            .find_map(|l| {
                l.trim_start()
                    .strip_prefix("name:")
                    .map(str::trim)
                    .filter(|_| l.starts_with("          "))
            })
            .unwrap_or_else(|| panic!("an upload step names its artifact:\n{step}"))
            .to_string();
        let mut names = vec![name];
        for key in ["artifact", "target"] {
            let token = format!("${{{{ matrix.{key} }}}}");
            if names.iter().any(|n| n.contains(&token)) {
                let values = matrix_values(job, key);
                let token = token.as_str();
                names = names
                    .iter()
                    .flat_map(|n| values.iter().map(move |v| n.replace(token, v)))
                    .collect();
            }
        }
        out.extend(names.into_iter().map(|n| (n, unconditional)));
    }
    out
}

/// `glob` (with `*`) matches `name`.
fn glob_matches(glob: &str, name: &str) -> bool {
    let parts: Vec<&str> = glob.split('*').collect();
    let mut rest = name;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            match rest.strip_prefix(part) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(j) => rest = &rest[j + part.len()..],
                None => return false,
            }
        }
    }
    rest.is_empty() || parts.len() > 1
}

/// The artifact directories the release copies from: every `cp <dir>/…` and `for dir in <glob>` of the
/// `Prepare release assets` step, run from the downloaded `artifacts/`.
fn copied_dirs(release_job: &str) -> Vec<String> {
    let step = steps(release_job)
        .into_iter()
        .find(|s| s.contains("Prepare release assets"))
        .expect("github-release has a `Prepare release assets` step");
    let mut out = Vec::new();
    for line in step.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("cp ") {
            let src = rest.split_whitespace().next().unwrap_or_default();
            if let Some((dir, _)) = src.split_once('/') {
                out.push(dir.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("for dir in ") {
            out.push(
                rest.split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            );
        }
    }
    out
}

#[test]
fn every_artifact_the_release_copies_is_uploaded_on_every_tag_build() {
    let text = workflow("release.yml");
    let jobs = jobs(&text);
    let release = jobs.get("github-release").expect("a github-release job");
    let needs_line = release
        .lines()
        .find(|l| l.trim_start().starts_with("needs:"))
        .expect("github-release needs its build jobs");
    let needs: Vec<String> = needs_line
        .split_once(':')
        .unwrap()
        .1
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut available: Vec<(String, bool, String)> = Vec::new();
    for need in &needs {
        let job = jobs.get(need).unwrap_or_else(|| {
            panic!("github-release needs `{need}`, which release.yml does not define")
        });
        available.extend(uploads(job).into_iter().map(|(n, u)| (n, u, need.clone())));
    }
    let dirs = copied_dirs(release);
    assert!(
        dirs.len() >= 3,
        "the release copies the LSP, largo and the VSIX: {dirs:?}"
    );
    for dir in &dirs {
        let matching: Vec<&(String, bool, String)> = available
            .iter()
            .filter(|(n, ..)| glob_matches(dir, n))
            .collect();
        assert!(!matching.is_empty(), "the release copies from `{dir}/`, which no job it needs uploads (uploads: {available:?})");
        let guarded: Vec<&&(String, bool, String)> =
            matching.iter().filter(|(_, u, _)| !u).collect();
        assert!(
            guarded.is_empty(),
            "the release copies from `{dir}/` with no fallback, but its upload is behind an `if:` — on a build \
             where the condition is false the copy fails and there is no release: {guarded:?}"
        );
    }
}

#[test]
fn the_release_and_test_workflows_carry_no_marketing() {
    for name in ["release.yml", "test.yml"] {
        let text = workflow(name);
        for needle in ["marketing/", "brochure"] {
            assert!(!text.contains(needle), "{name} names `{needle}` — the brochures do not ship on GitHub releases (2026-09-26)");
        }
    }
}

#[test]
fn the_parsers_read_the_shapes_they_guard() {
    let wf = "on: push\njobs:\n  build:\n    strategy:\n      matrix:\n        include:\n          - { target: a-b, artifact: tool-a }\n          - { target: c-d, artifact: tool-c }\n    steps:\n      - name: Smoke-run\n        if: matrix.target != 'c-d'\n        run: ./tool --version\n      # upload after packaging: actions/upload-artifact@v4 drops the exec bit\n      - name: Upload\n        uses: actions/upload-artifact@v4\n        with:\n          name: ${{ matrix.artifact }}\n  extra:\n    steps:\n      - name: Upload\n        if: hashFiles('x') != ''\n        uses: actions/upload-artifact@v4\n        with:\n          name: extra\n";
    let jobs = jobs(wf);
    assert_eq!(
        uploads(&jobs["build"]),
        vec![("tool-a".to_string(), true), ("tool-c".to_string(), true)]
    );
    assert_eq!(uploads(&jobs["extra"]), vec![("extra".to_string(), false)]);
    assert!(
        glob_matches("tool-*", "tool-a")
            && !glob_matches("vsix-*", "tool-a")
            && glob_matches("extra", "extra")
    );
}

/// GitHub's ceiling on a release body, in characters.
const BODY_LIMIT: usize = 125_000;

const REPO: &str = "Brahmastra-Labs/logicaffeine";

/// A step's `run: |` block as the runner executes it: the lines under `run: |`, the block's indentation removed.
fn run_block(step: &str) -> String {
    let mut lines = step.lines().skip_while(|l| l.trim() != "run: |").skip(1).peekable();
    let indent = lines
        .peek()
        .map(|l| l.len() - l.trim_start().len())
        .expect("the step's run block has a body");
    let mut out = String::new();
    for line in lines {
        if line.trim().is_empty() {
            out.push('\n');
            continue;
        }
        if line.len() - line.trim_start().len() < indent {
            break;
        }
        out.push_str(&line[indent..]);
        out.push('\n');
    }
    out
}

/// What the release-notes step of `release.yml` writes to `release-notes.md` for `tag`, run under the runner's
/// shell in a directory holding `changelog` as `CHANGELOG.md`.
fn release_notes(changelog: &str, tag: &str) -> String {
    let jobs = jobs(&workflow("release.yml"));
    let step = steps(&jobs["github-release"])
        .into_iter()
        .find(|s| s.contains("name: Extract release notes from CHANGELOG"))
        .expect("github-release has an `Extract release notes from CHANGELOG` step");
    let dir = tempfile::tempdir().expect("a scratch directory");
    std::fs::write(dir.path().join("CHANGELOG.md"), changelog).expect("write CHANGELOG.md");
    let out = std::process::Command::new("bash")
        .args(["--noprofile", "--norc", "-eo", "pipefail", "-c", &run_block(&step)])
        .current_dir(dir.path())
        .env_remove("GITHUB_REF_NAME")
        .env("RELEASE_TAG", tag)
        .env("GITHUB_REPOSITORY", REPO)
        .output()
        .expect("run the release-notes step under bash");
    assert!(
        out.status.success(),
        "the release-notes step failed for {tag}:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read_to_string(dir.path().join("release-notes.md")).expect("the step writes release-notes.md")
}

/// The versions `changelog` has released, newest first.
fn released_versions(changelog: &str) -> Vec<String> {
    changelog
        .lines()
        .filter_map(|l| l.strip_prefix("## ["))
        .filter_map(|l| l.split_once(']'))
        .map(|(v, _)| v.to_string())
        .filter(|v| v != "Unreleased")
        .collect()
}

/// The `## [version]` header line and the section under it, its surrounding blank lines removed.
fn section<'a>(changelog: &'a str, version: &str) -> (&'a str, &'a str) {
    let header = format!("## [{version}]");
    let start = changelog
        .find(&format!("\n{header}"))
        .map(|i| i + 1)
        .unwrap_or_else(|| panic!("no {header} section"));
    let line_end = start + changelog[start..].find('\n').unwrap_or(changelog.len() - start);
    let body = &changelog[line_end..];
    let end = body.find("\n## [").map(|i| i + 1).unwrap_or(body.len());
    (&changelog[start..line_end], body[..end].trim_matches('\n'))
}

/// A section's `### ` headings, each with the text before its first entry and its top-level `- ` entries.
fn headings(section: &str) -> Vec<(String, String, Vec<String>)> {
    let mut out: Vec<(String, String, Vec<String>)> = Vec::new();
    for line in section.split_inclusive('\n') {
        if line.starts_with("### ") {
            out.push((line.trim_end().to_string(), String::new(), Vec::new()));
        } else if let Some((_, intro, entries)) = out.last_mut() {
            if line.starts_with("- ") {
                entries.push(line.to_string());
            } else if let Some(entry) = entries.last_mut() {
                entry.push_str(line);
            } else {
                intro.push_str(line);
            }
        }
    }
    out
}

/// GitHub's anchor for a Markdown heading: lowercased, punctuation but `-` and `_` dropped, spaces to hyphens.
fn github_anchor(heading: &str) -> String {
    heading
        .trim_start_matches('#')
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// The line that stands in for a heading's `omitted` entries left out of the notes for `tag`.
fn pointer(omitted: usize, tag: &str, header: &str) -> String {
    format!(
        "- *…and {omitted} more, in the [full notes](https://github.com/{REPO}/blob/{tag}/CHANGELOG.md#{}).*",
        github_anchor(header)
    )
}

/// The notes the release publishes for `version` of `changelog` are a body GitHub accepts and say what the
/// section says: the section itself when it fits, otherwise every heading, each heading's first entries
/// verbatim, and a pointer to the full notes for each heading cut. Returns, per heading, how many entries it kept.
fn assert_notes(changelog: &str, version: &str) -> Vec<(String, usize, usize)> {
    let tag = format!("v{version}");
    let notes = release_notes(changelog, &tag);
    let (header, section) = section(changelog, version);
    let chars = notes.chars().count();
    assert!(!notes.trim().is_empty(), "the {tag} notes are empty");
    assert!(chars <= BODY_LIMIT, "the {tag} notes are {chars} characters; GitHub refuses a release body over {BODY_LIMIT}");
    if section.chars().count() < BODY_LIMIT {
        assert_eq!(notes.trim_matches('\n'), section, "the {tag} section fits a release body, and its notes are not the section");
        return headings(section).into_iter().map(|(h, _, e)| (h, e.len(), e.len())).collect();
    }
    let want = headings(section);
    let got = headings(&notes);
    assert_eq!(
        got.iter().map(|(h, ..)| h).collect::<Vec<_>>(),
        want.iter().map(|(h, ..)| h).collect::<Vec<_>>(),
        "the cut {tag} notes do not keep every heading of the section, in order"
    );
    let mut kept = Vec::new();
    for ((heading, intro, entries), (_, got_intro, got_entries)) in want.iter().zip(&got) {
        assert_eq!(got_intro.trim(), intro.trim(), "{tag} {heading}: the text before its entries changed");
        let cut = got_entries.last().is_some_and(|e| e.starts_with("- *…and "));
        let shown = &got_entries[..got_entries.len() - usize::from(cut)];
        for (i, (g, w)) in shown.iter().zip(entries).enumerate() {
            assert_eq!(g.trim_end(), w.trim_end(), "{tag} {heading}: entry {i} is not the section's entry {i} verbatim");
        }
        assert!(shown.len() <= entries.len(), "{tag} {heading}: the notes show entries the section does not have");
        if shown.len() < entries.len() {
            assert!(cut, "{tag} {heading}: {} of {} entries are left out with no pointer to the rest", entries.len() - shown.len(), entries.len());
            assert_eq!(
                got_entries.last().unwrap().trim_end(),
                pointer(entries.len() - shown.len(), &tag, header),
                "{tag} {heading}: the pointer to the left-out entries"
            );
        } else {
            assert!(!cut, "{tag} {heading}: every entry is shown, and the notes still point to more");
        }
        kept.push((heading.clone(), shown.len(), entries.len()));
    }
    assert!(
        chars >= BODY_LIMIT * 9 / 10,
        "the cut {tag} notes use {chars} of the {BODY_LIMIT} characters a release body holds; the cut leaves out entries that fit"
    );
    kept
}

#[test]
fn the_release_notes_of_every_released_version_are_a_body_github_accepts() {
    let changelog = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../CHANGELOG.md"))
        .expect("read the root CHANGELOG.md");
    let versions = released_versions(&changelog);
    assert!(versions.iter().any(|v| v == "0.11.0") && versions.len() > 10, "released versions: {versions:?}");
    for version in &versions {
        let kept = assert_notes(&changelog, version);
        if version == "0.11.0" {
            for (heading, shown, total) in &kept {
                if ["### Highlights", "### Breaking changes", "### Removed", "### Security"].contains(&heading.as_str()) {
                    assert_eq!(shown, total, "the 0.11.0 release page cuts {heading}: {shown} of {total} entries");
                }
            }
            assert!(kept.iter().any(|(_, s, t)| s < t), "the 0.11.0 section is over the limit, and no heading was cut");
        }
    }
}

/// A synthetic changelog drives the cut through the shapes the real one has not reached yet: a heading whose first
/// entry alone outgrows the body, multi-line entries, text between a heading and its entries, and a neighbouring
/// release the cut must not reach into.
#[test]
fn the_cut_keeps_whole_entries_under_every_heading_and_points_to_the_rest() {
    let entry = |tag: &str, i: usize, len: usize| {
        let mut e = format!("- **{tag} {i}.** ");
        while e.len() < len {
            e.push_str("A sentence the changelog says about it,\n  continued on an indented line. ");
        }
        e.push('\n');
        e
    };
    let mut changelog = String::from("# Changelog\n\n## [Unreleased]\n\n## [9.9.9] - 2030-01-01\n\n### Highlights\n\n");
    changelog.push_str("- one highlight\n- another highlight\n\n### Added\n\nWhat this release adds.\n\n");
    for i in 0..300 {
        changelog.push_str(&entry("added", i, 900));
    }
    changelog.push_str("\n### Changed\n\n");
    changelog.push_str(&entry("changed", 0, 2 * BODY_LIMIT));
    changelog.push_str(&entry("changed", 1, 200));
    changelog.push_str("\n### Fixed\n\n");
    for i in 0..120 {
        changelog.push_str(&entry("fixed", i, 2_400));
    }
    changelog.push_str("\n### Security\n\n- a security fix\n\n## [9.9.8] - 2029-01-01\n\n### Fixed\n\n- an older fix\n");

    let kept = assert_notes(&changelog, "9.9.9");
    let by = |h: &str| kept.iter().find(|(k, ..)| k == h).map(|(_, s, t)| (*s, *t)).unwrap();
    assert_eq!(by("### Highlights"), (2, 2));
    assert_eq!(by("### Security"), (1, 1));
    assert_eq!(by("### Changed").0, 0, "an entry longer than the whole body is never shown cut mid-entry");
    assert!(by("### Added").0 > 0 && by("### Added").0 < 300, "Added: {:?}", by("### Added"));
    assert!(by("### Fixed").0 > 0 && by("### Fixed").0 < 120, "Fixed: {:?}", by("### Fixed"));
    assert!(!release_notes(&changelog, "v9.9.9").contains("an older fix"), "the 9.9.9 notes reach into 9.9.8");
    assert_eq!(assert_notes(&changelog, "9.9.8"), vec![("### Fixed".to_string(), 1, 1)]);
    assert_eq!(
        release_notes(&changelog, "v1.2.3").trim_end(),
        format!("See the [changelog](https://github.com/{REPO}/blob/main/CHANGELOG.md)."),
        "a tag with no section of its own links the changelog"
    );
}

/// Every line of `text` outside comments.
fn code_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|l| !l.trim_start().starts_with('#'))
}

#[test]
fn a_dispatch_releases_an_existing_tag_with_the_dispatched_workflow() {
    let text = workflow("release.yml");
    let head = &text[..text.find("\njobs:\n").expect("a jobs: table")];
    let dispatch = &head[head.find("workflow_dispatch:").expect("release.yml can be dispatched")..];
    assert!(
        dispatch.contains("inputs:") && dispatch.lines().any(|l| l.trim() == "tag:"),
        "a dispatch of release.yml takes the tag to release:\n{dispatch}"
    );
    let definition = code_lines(head)
        .find(|l| l.trim_start().starts_with("RELEASE_TAG:"))
        .expect("release.yml defines RELEASE_TAG for every job");
    assert!(
        definition.contains("inputs.tag") && definition.contains("github.ref_name"),
        "RELEASE_TAG is the dispatched tag, else the pushed one: {definition}"
    );
    for line in code_lines(&text) {
        if line.contains("GITHUB_REF_NAME") || line.contains("github.ref_name") {
            assert_eq!(line, definition, "release.yml reads the ref's own name past RELEASE_TAG: {line}");
        }
    }
    let jobs = jobs(&text);
    let mut checkouts = 0;
    for (name, job) in &jobs {
        for step in steps(job) {
            if code_lines(&step).any(|l| l.trim_start().trim_start_matches("- ").starts_with("uses: actions/checkout")) {
                if code_lines(&step).any(|l| l.trim() == "path: release-policy") {
                    continue;
                }
                checkouts += 1;
                assert!(
                    code_lines(&step).any(|l| l.trim() == "ref: ${{ env.RELEASE_TAG }}"),
                    "{name} checks out the dispatched ref, not the tag it releases:\n{step}"
                );
            }
        }
    }
    assert!(checkouts >= 7, "release.yml has {checkouts} checkouts; the check reads nothing");
    for name in ["github-release", "e2e-install", "e2e-install-windows"] {
        let job = jobs.get(name).unwrap_or_else(|| panic!("release.yml has a {name} job"));
        let gate = code_lines(job)
            .find(|l| l.starts_with("    if:"))
            .unwrap_or_else(|| panic!("{name} runs for tags only"));
        assert!(
            gate.contains("refs/tags/v") && gate.contains("inputs.tag"),
            "{name} does not run for a dispatched tag: {gate}"
        );
    }
    let create = steps(&jobs["github-release"])
        .into_iter()
        .find(|s| s.contains("softprops/action-gh-release"))
        .expect("github-release creates the release");
    for want in ["tag_name: ${{ env.RELEASE_TAG }}", "draft: false"] {
        assert!(
            code_lines(&create).any(|l| l.trim() == want),
            "the release step does not say `{want}`:\n{create}"
        );
    }
}

#[test]
fn the_run_block_reader_reads_the_shape_it_guards() {
    let step = "      - name: Notes\n        run: |\n          first\n            nested\n\n          last\n        env:\n          X: y\n";
    assert_eq!(run_block(step), "first\n  nested\n\nlast\n");
    assert_eq!(github_anchor("## [0.11.0] - 2026-11-02"), "0110---2026-11-02");
    let (header, body) = section("# C\n\n## [1.0.0] - 2030-01-01\n\n### Fixed\n- a\n\n## [0.9.0]\n- b\n", "1.0.0");
    assert_eq!((header, body), ("## [1.0.0] - 2030-01-01", "### Fixed\n- a"));
    assert_eq!(
        headings("### A\nintro\n- x\n  more\n- y\n### B\n- z\n"),
        vec![
            ("### A".to_string(), "intro\n".to_string(), vec!["- x\n  more\n".to_string(), "- y\n".to_string()]),
            ("### B".to_string(), String::new(), vec!["- z\n".to_string()]),
        ]
    );
}

/// `line` is a `run:` that calls the size check.
fn runs_the_size_check(line: &str) -> bool {
    line.trim_start().starts_with("run:") && line.contains("check-binary-sizes.sh")
}

/// The steps of `job` that run the size check, and the release-policy checkouts before each.
fn budget_steps(job: &str) -> Vec<(String, Option<String>)> {
    let mut policy: Option<String> = None;
    let mut out = Vec::new();
    for step in steps(job) {
        if code_lines(&step).any(|l| l.trim() == "path: release-policy") {
            policy = Some(step.clone());
        }
        if code_lines(&step).any(runs_the_size_check) {
            out.push((step.clone(), policy.clone()));
        }
    }
    out
}

/// A release builds the tag's code and holds it to the release policy of the workflow that dispatched it: the size
/// budgets come from the workflow's own commit, so a budget raised (or lowered) after the tag applies to its binaries.
#[test]
fn a_release_holds_the_tags_binaries_to_the_dispatching_workflows_size_budgets() {
    let jobs = jobs(&workflow("release.yml"));
    let mut checked = Vec::new();
    for (name, job) in &jobs {
        for (step, policy) in budget_steps(job) {
            let run = code_lines(&step).find(|l| runs_the_size_check(l)).unwrap().trim().to_string();
            assert!(
                run.starts_with("run: ./release-policy/scripts/check-binary-sizes.sh --bin-dir target/"),
                "{name} checks sizes with the tag's script and budgets, not the workflow's: {run}"
            );
            let policy = policy.unwrap_or_else(|| panic!("{name} checks sizes with no release-policy checkout before it"));
            for want in ["uses: actions/checkout@v4", "ref: ${{ github.sha }}", "path: release-policy"] {
                assert!(
                    code_lines(&policy).any(|l| l.trim().trim_start_matches("- ") == want),
                    "{name}'s release-policy checkout does not say `{want}`:\n{policy}"
                );
            }
            for file in [".config/size-budgets.toml", "scripts/check-binary-sizes.sh"] {
                assert!(
                    code_lines(&policy).any(|l| l.trim() == file),
                    "{name}'s release-policy checkout leaves out {file}:\n{policy}"
                );
            }
            checked.push(run);
        }
    }
    assert_eq!(checked.len(), 2, "release.yml checks the LSP and largo sizes, each once: {checked:?}");
}

/// `check-binary-sizes.sh` reads the budgets of the tree it sits in and the binaries of the directory it is pointed
/// at, a relative `--bin-dir` read from where it is called: a copy of the release policy checks the tag's build.
#[test]
fn the_size_check_reads_its_own_trees_budgets_and_the_callers_binaries() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = tempfile::tempdir().expect("a scratch directory");
    let policy = dir.path().join("policy");
    std::fs::create_dir_all(policy.join("scripts")).unwrap();
    std::fs::create_dir_all(policy.join(".config")).unwrap();
    std::fs::copy(root.join("scripts/check-binary-sizes.sh"), policy.join("scripts/check-binary-sizes.sh")).unwrap();
    std::fs::write(policy.join(".config/size-budgets.toml"), "[binaries]\nlargo = 1000\nlogicaffeine-lsp = 1000\n").unwrap();
    let caller = dir.path().join("caller");
    std::fs::create_dir_all(caller.join("target/dist")).unwrap();
    let check = |bytes: usize| {
        std::fs::write(caller.join("target/dist/largo"), vec![0u8; bytes]).unwrap();
        std::process::Command::new("bash")
            .args(["../policy/scripts/check-binary-sizes.sh", "--bin-dir", "target/dist", "--only", "largo"])
            .current_dir(&caller)
            .output()
            .expect("run check-binary-sizes.sh")
    };
    let at = check(1000);
    assert!(at.status.success(), "a binary at its budget fails the check:\n{}", String::from_utf8_lossy(&at.stdout));
    let over = check(1001);
    assert!(!over.status.success(), "a binary one byte over its budget passes the check:\n{}", String::from_utf8_lossy(&over.stdout));
    assert!(
        String::from_utf8_lossy(&over.stdout).contains("largo is 1001 bytes, over its 1000 budget"),
        "the check does not name the binary over its budget:\n{}",
        String::from_utf8_lossy(&over.stdout)
    );
}

/// The scripts a workflow line runs by its path (`./scripts/x.sh …`), each as a path in this tree: a script run from the
/// release-policy checkout is the same tree's file.
/// Whether `job` can run on a Windows runner: its `runs-on` names one, or its matrix gives `os` one.
fn runs_on_windows(job: &str) -> bool {
    runs_on(job).is_some_and(|label| label.contains("windows"))
        || matrix_values(job, "os").iter().any(|os| os.contains("windows"))
}

/// The release's jobs that check out on a Windows runner without first turning off the checkout's line-ending
/// conversion.
fn crlf_checkouts(text: &str) -> Vec<String> {
    jobs(text)
        .into_iter()
        .filter(|(_, job)| runs_on_windows(job))
        .filter_map(|(name, job)| {
            let steps = steps(&job);
            let checkout = steps.iter().position(|s| s.contains("uses: actions/checkout"))?;
            (!steps[..checkout].iter().any(|s| script(s).iter().any(|l| l == "git config --global core.autocrlf false")))
                .then_some(name)
        })
        .collect()
}

/// A Windows runner's git checks text out with CRLF line endings, and the prelude's build script reads each module's
/// source line for line against its spliced code (`stdlib_text::index`): with CRLF it panics ("its spliced code is
/// not a sequence of its source's lines"), and the v0.11.0 dispatch lost every Windows binary to it. Each job that
/// checks out on Windows turns the conversion off first, so the tag builds from the bytes it holds.
#[test]
fn every_release_job_on_windows_checks_out_the_trees_own_line_endings() {
    let missing = crlf_checkouts(&workflow("release.yml"));
    assert!(missing.is_empty(), "release jobs that check out CRLF on Windows: {missing:?}");
}

#[test]
fn the_line_ending_reader_reads_the_shape_it_guards() {
    let text = "on: push\n\njobs:\n  kept:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        include:\n          - { os: windows-latest, target: x }\n    steps:\n      - name: Keep the tree's line endings\n        run: git config --global core.autocrlf false\n      - uses: actions/checkout@v4\n  converted:\n    runs-on: windows-latest\n    steps:\n      - uses: actions/checkout@v4\n      - run: git config --global core.autocrlf false\n  linux:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n";
    assert_eq!(crlf_checkouts(text), vec!["converted".to_string()]);
}

fn scripts_run_by_path(workflow: &str) -> Vec<String> {
    code_lines(workflow)
        .map(|l| l.trim().trim_start_matches("- ").trim_start_matches("run:").trim_start())
        .filter_map(|l| l.strip_prefix("./"))
        .map(|l| l.split_whitespace().next().unwrap_or_default())
        .map(|p| p.strip_prefix("release-policy/").unwrap_or(p).to_string())
        .collect()
}

/// A runner executes a script named by its path only when the checkout marks it executable, and the checkout takes the
/// mode from the tree: a script committed without its exec bit fails its step with "Permission denied" and nothing
/// before the release ever runs it.
#[test]
fn every_script_a_workflow_runs_by_its_path_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut seen = 0;
    for entry in std::fs::read_dir(root.join(".github/workflows")).expect("read .github/workflows") {
        let path = entry.expect("a workflow").path();
        if path.extension().is_none_or(|e| e != "yml") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        for script in scripts_run_by_path(&std::fs::read_to_string(&path).expect("read a workflow")) {
            let file = root.join(&script);
            let mode = std::fs::metadata(&file)
                .unwrap_or_else(|e| panic!("{name} runs ./{script}, which is not in the tree: {e}"))
                .permissions()
                .mode();
            assert!(mode & 0o100 != 0, "{name} runs ./{script}, which the tree does not mark executable ({mode:o})");
            seen += 1;
        }
    }
    assert!(seen >= 10, "the workflows run {seen} scripts by path; the check reads nothing");
    assert_eq!(
        scripts_run_by_path("    steps:\n      - run: ./release-policy/scripts/a.sh --x\n        run: |\n          ./scripts/b.sh\n          sh c.sh\n      # run: ./d.sh\n"),
        vec!["scripts/a.sh".to_string(), "scripts/b.sh".to_string()]
    );
}
