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
//! Run: `cargo nextest run -p logicaffeine-tests --test release_assets_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::path::PathBuf;

fn workflow(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".github/workflows")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Each job's text, keyed by its name: the lines from `  <name>:` to the next job.
fn jobs(text: &str) -> BTreeMap<String, String> {
    let body = &text[text.find("\njobs:\n").expect("a jobs: table") + "\njobs:\n".len()..];
    let mut out = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for line in body.lines() {
        let is_job = line.starts_with("  ")
            && !line.starts_with("   ")
            && line.trim_end().ends_with(':')
            && !line.trim_start().starts_with('#');
        if is_job {
            if let Some((name, text)) = current.take() {
                out.insert(name, text);
            }
            current = Some((line.trim().trim_end_matches(':').to_string(), String::new()));
        } else if let Some((_, text)) = current.as_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    if let Some((name, text)) = current {
        out.insert(name, text);
    }
    out
}

/// A job's steps, each as its text.
fn steps(job: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in job.lines() {
        if line.starts_with("      - ") {
            out.push(String::new());
        }
        if let Some(step) = out.last_mut() {
            step.push_str(line);
            step.push('\n');
        }
    }
    out
}

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
