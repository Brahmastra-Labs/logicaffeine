//! The GitHub workflows as the locks read them: a workflow's text, its jobs by name, a job's steps, and the
//! variables a workflow's top-level `env:` sets.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows")
}

/// The text of `.github/workflows/<name>`.
pub fn workflow(name: &str) -> String {
    let path = dir().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every workflow file's name, in order.
pub fn workflow_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir())
        .expect("the workflows directory")
        .map(|e| e.expect("a workflow entry").file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".yml") || n.ends_with(".yaml"))
        .collect();
    names.sort();
    names
}

/// Each job's text, keyed by its name: the lines from `  <name>:` to the next job.
pub fn jobs(text: &str) -> BTreeMap<String, String> {
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
pub fn steps(job: &str) -> Vec<String> {
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

/// The variables a workflow's top-level `env:` table sets, each to its value as written.
pub fn env(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(at) = text.find("\nenv:\n") else { return out };
    for line in text[at + "\nenv:\n".len()..].lines() {
        if !line.starts_with("  ") && !line.trim().is_empty() {
            break;
        }
        let entry = line.trim();
        if entry.starts_with('#') || entry.is_empty() || line.starts_with("   ") {
            continue;
        }
        if let Some((key, value)) = entry.split_once(':') {
            out.insert(key.trim().to_string(), value.trim().trim_matches('"').trim_matches('\'').to_string());
        }
    }
    out
}

/// The job's `runs-on:` label, as written.
pub fn runs_on(job: &str) -> Option<String> {
    job.lines()
        .find_map(|l| l.strip_prefix("    runs-on:"))
        .map(|v| v.trim().to_string())
}
