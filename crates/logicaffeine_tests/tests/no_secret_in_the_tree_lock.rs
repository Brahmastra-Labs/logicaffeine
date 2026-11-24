//! no_secret_in_the_tree_lock — no file in the checkout holds a secret (DAT-436).
//!
//! Credentials live outside the tree: in owner-only files under `~/.config`, in Wrangler's own store, and in a
//! Worker's secrets. GitHub's secret scanning does not run on this repository, so this lock is the scan.
//!
//! - No file holds a private key block, a GitHub, Slack, AWS or model-provider token, or a quoted or `=`-assigned
//!   value of 20 or more token characters under a secret's name (`client_secret`, `api_key`, `password`,
//!   `CLOUDFLARE_API_TOKEN`, …). [`DETECTORS`] holds each shape, and each one is shown catching a sample of its
//!   shape and passing a near miss.
//! - No file is named like a secret: an `.env`, a `.dev.vars`, a `*.credentials*`, an SSH private key, a PKCS #12
//!   bundle, a `.netrc` or a `.pgpass`.
//! - `.gitignore` keeps those names out of every directory, checked with `git check-ignore`.
//! - [`VECTORS`] names each file that holds a secret's shape on purpose (public known-answer test vectors), and each
//!   of its rows must still be needed.
//!
//! A refusal names a file, a line and a detector, never the text it matched: a red lands in logs.
//!
//! The checkout is what `git ls-files` lists, tracked or untracked but not ignored, with `work/` left out, so a
//! secret is caught before it is committed as well as after.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test no_secret_in_the_tree_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

/// A secret's shape: a name for the refusal and the test that finds it in one line.
struct Detector {
    name: &'static str,
    finds: fn(&str) -> bool,
}

const DETECTORS: &[Detector] = &[
    Detector { name: "private key block", finds: holds_private_key_block },
    Detector { name: "GitHub token", finds: holds_github_token },
    Detector { name: "GitHub fine-grained token", finds: holds_github_fine_grained_token },
    Detector { name: "Slack token", finds: holds_slack_token },
    Detector { name: "Slack webhook", finds: holds_slack_webhook },
    Detector { name: "AWS access key id", finds: holds_aws_access_key_id },
    Detector { name: "Anthropic key", finds: holds_anthropic_key },
    Detector { name: "OpenAI key", finds: holds_openai_key },
    Detector { name: "NVIDIA key", finds: holds_nvidia_key },
    Detector { name: "Hugging Face token", finds: holds_hugging_face_token },
    Detector { name: "Groq key", finds: holds_groq_key },
    Detector { name: "Google API key", finds: holds_google_api_key },
    Detector { name: "xAI key", finds: holds_xai_key },
    Detector { name: "secret assignment", finds: holds_secret_assignment },
];

/// A file that holds a secret's shape on purpose: the file, the detector that finds it, and why it is no secret.
const VECTORS: &[(&str, &str, &str)] = &[
    (
        "crates/logicaffeine_tests/tests/data/wycheproof/pbkdf2_hmac_sha256.json",
        "secret assignment",
        "Project Wycheproof's published PBKDF2 known-answer vectors: each `password` is a public test input",
    ),
    (
        "crates/logicaffeine_tests/tests/s3_sigv4_lock.rs",
        "AWS access key id",
        "the worked GET Object example of Amazon's Signature Version 4 documentation: its access key id is Amazon's \
         published example key",
    ),
];

/// The names a variable holding a secret goes by, matched without regard to case at the end of a longer name too
/// (`NVIDIA_API_KEY` ends in `api_key`).
const SECRET_NAMES: &[&str] = &[
    "client_secret",
    "api_secret",
    "secret_key",
    "secret_access_key",
    "access_token",
    "refresh_token",
    "auth_token",
    "api_token",
    "api_key",
    "apikey",
    "private_key",
    "password",
    "passwd",
];

/// The shortest value under a secret's name that counts as one: shorter values are placeholders and test fixtures.
const SHORTEST_ASSIGNED_SECRET: usize = 20;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The checkout: every file `git ls-files` lists, tracked or untracked but not ignored, with `work/` left out.
fn checkout() -> Vec<String> {
    let root = repo_root();
    let out = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(&root)
        .output()
        .expect("git ls-files");
    assert!(out.status.success(), "git ls-files failed: {}", String::from_utf8_lossy(&out.stderr));
    let mut paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|path| !path.is_empty() && !path.starts_with("work/"))
        .filter(|path| root.join(path).is_file())
        .map(str::to_string)
        .collect();
    paths.sort();
    paths.dedup();
    assert!(paths.len() > 5000, "the checkout lists only {} files; the walk broke, not the tree", paths.len());
    paths
}

/// A file's text, or `None` for a binary file (a NUL byte in its first 8 KiB).
fn text_of(path: &str) -> Option<String> {
    let bytes = std::fs::read(repo_root().join(path)).unwrap_or_else(|error| panic!("read {path}: {error}"));
    if bytes.iter().take(8192).any(|&byte| byte == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Every (line number, detector name) a file's text holds.
fn findings_in(text: &str) -> Vec<(usize, &'static str)> {
    let mut findings = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for detector in DETECTORS {
            if (detector.finds)(line) {
                findings.push((index + 1, detector.name));
            }
        }
    }
    findings
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn is_alphanumeric_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn is_uppercase_or_digit_byte(byte: u8) -> bool {
    byte.is_ascii_uppercase() || byte.is_ascii_digit()
}

/// How many bytes from `start` on satisfy `allowed`.
fn run_length(bytes: &[u8], start: usize, allowed: fn(u8) -> bool) -> usize {
    bytes[start.min(bytes.len())..].iter().take_while(|&&byte| allowed(byte)).count()
}

/// Whether `line` holds `prefix`, not preceded by a byte `joined` accepts, followed by at least `shortest` bytes
/// that `allowed` accepts.
fn holds_prefixed_run(line: &str, prefix: &str, joined: fn(u8) -> bool, allowed: fn(u8) -> bool, shortest: usize) -> bool {
    let bytes = line.as_bytes();
    line.match_indices(prefix).any(|(at, _)| {
        let starts_a_word = at == 0 || !joined(bytes[at - 1]);
        starts_a_word && run_length(bytes, at + prefix.len(), allowed) >= shortest
    })
}

fn holds_private_key_block(line: &str) -> bool {
    let opening = "-----BEGIN ";
    line.match_indices(opening).any(|(at, _)| {
        let rest = &line[at + opening.len()..];
        let Some(end) = rest.find("-----") else { return false };
        let label = &rest[..end];
        label.bytes().all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b' ')
            && (label.ends_with("PRIVATE KEY") || label.ends_with("PRIVATE KEY BLOCK"))
    })
}

fn holds_github_token(line: &str) -> bool {
    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| holds_prefixed_run(line, prefix, is_word_byte, is_alphanumeric_byte, 36))
}

fn holds_github_fine_grained_token(line: &str) -> bool {
    holds_prefixed_run(line, "github_pat_", is_word_byte, is_word_byte, 60)
}

fn holds_slack_token(line: &str) -> bool {
    let is_slack_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-';
    ["xoxa-", "xoxb-", "xoxo-", "xoxp-", "xoxr-", "xoxs-"]
        .iter()
        .any(|prefix| holds_prefixed_run(line, prefix, is_word_byte, is_slack_byte, 20))
}

fn holds_slack_webhook(line: &str) -> bool {
    let is_webhook_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'/';
    holds_prefixed_run(line, "hooks.slack.com/services/T", |_| false, is_webhook_byte, 20)
}

fn holds_aws_access_key_id(line: &str) -> bool {
    let bytes = line.as_bytes();
    ["AKIA", "ASIA"].iter().any(|prefix| {
        line.match_indices(prefix).any(|(at, _)| {
            let starts_a_word = at == 0 || !is_alphanumeric_byte(bytes[at - 1]);
            starts_a_word && run_length(bytes, at + prefix.len(), is_alphanumeric_byte) == 16
                && run_length(bytes, at + prefix.len(), is_uppercase_or_digit_byte) == 16
        })
    })
}

fn holds_anthropic_key(line: &str) -> bool {
    holds_prefixed_run(line, "sk-ant-", is_word_byte, is_token_byte, 20)
}

fn holds_openai_key(line: &str) -> bool {
    let bytes = line.as_bytes();
    line.match_indices("sk-").any(|(at, _)| {
        let starts_a_word = at == 0 || !is_token_byte(bytes[at - 1]);
        starts_a_word && !line[at..].starts_with("sk-ant-") && run_length(bytes, at + 3, is_token_byte) >= 40
    })
}

fn holds_nvidia_key(line: &str) -> bool {
    holds_prefixed_run(line, "nvapi-", is_word_byte, is_token_byte, 20)
}

fn holds_hugging_face_token(line: &str) -> bool {
    holds_prefixed_run(line, "hf_", is_word_byte, is_alphanumeric_byte, 30)
}

fn holds_groq_key(line: &str) -> bool {
    holds_prefixed_run(line, "gsk_", is_word_byte, is_alphanumeric_byte, 40)
}

fn holds_google_api_key(line: &str) -> bool {
    holds_prefixed_run(line, "AIza", is_word_byte, is_token_byte, 35)
}

fn holds_xai_key(line: &str) -> bool {
    holds_prefixed_run(line, "xai-", is_word_byte, is_alphanumeric_byte, 40)
}

/// Whether a value looks like a credential rather than a word, a path or a placeholder: a letter and a digit, at
/// least [`SHORTEST_ASSIGNED_SECRET`] long.
fn looks_like_a_credential(value: &str) -> bool {
    value.len() >= SHORTEST_ASSIGNED_SECRET
        && value.bytes().any(|byte| byte.is_ascii_alphabetic())
        && value.bytes().any(|byte| byte.is_ascii_digit())
}

fn is_assigned_value_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'+' | b'/' | b'=' | b'.')
}

/// Whether `line` gives a secret's name a credential: `name = "value"`, `"name": "value"`, `name: 'value'` or the
/// unquoted `NAME=value` of an environment file or a shell line.
fn holds_secret_assignment(line: &str) -> bool {
    let lowered = line.to_ascii_lowercase();
    let bytes = line.as_bytes();
    SECRET_NAMES.iter().any(|name| {
        lowered.match_indices(name).any(|(at, _)| {
            let mut cursor = at + name.len();
            if cursor < bytes.len() && is_word_byte(bytes[cursor]) {
                return false;
            }
            if cursor < bytes.len() && matches!(bytes[cursor], b'"' | b'\'') {
                cursor += 1;
            }
            let before_separator = cursor;
            cursor += run_length(bytes, cursor, |byte| byte == b' ' || byte == b'\t');
            let Some(&separator) = bytes.get(cursor) else { return false };
            if separator != b'=' && separator != b':' {
                return false;
            }
            let unquoted_allowed = separator == b'=' && cursor == before_separator;
            cursor += 1;
            cursor += run_length(bytes, cursor, |byte| byte == b' ' || byte == b'\t');
            match bytes.get(cursor) {
                Some(&quote) if matches!(quote, b'"' | b'\'' | b'`') => {
                    let length = run_length(bytes, cursor + 1, is_assigned_value_byte);
                    bytes.get(cursor + 1 + length) == Some(&quote)
                        && looks_like_a_credential(&line[cursor + 1..cursor + 1 + length])
                }
                Some(_) if unquoted_allowed => {
                    let length = run_length(bytes, cursor, is_assigned_value_byte);
                    let ends_the_value = bytes.get(cursor + length).map_or(true, |byte| byte.is_ascii_whitespace());
                    ends_the_value && looks_like_a_credential(&line[cursor..cursor + length])
                }
                _ => false,
            }
        })
    })
}

/// Whether a file's name is one a secret is kept under.
fn is_named_like_a_secret(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    let ssh_private_key = ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"]
        .iter()
        .any(|key| file == *key || (file.starts_with(&format!("{key}_")) && !file.ends_with(".pub")));
    file == ".env"
        || file.starts_with(".env.")
        || file == ".dev.vars"
        || file.contains(".credentials")
        || ssh_private_key
        || file.ends_with(".p12")
        || file.ends_with(".pfx")
        || file == ".netrc"
        || file == ".pgpass"
}

/// A sample of `length` characters cycling through letters and digits, built at run time so this source holds none.
fn token_sample(length: usize) -> String {
    "Ab3Cd5Ef7Gh9Jk2Lm4Np6Qr8St0Uv1Wx3Yz5".chars().cycle().take(length).collect()
}

fn uppercase_sample(length: usize) -> String {
    "AB3CD5EF7GH9JK2LM4NP6QR8ST0UV1WX3YZ5".chars().cycle().take(length).collect()
}

fn joined(parts: &[&str]) -> String {
    parts.concat()
}

/// Each detector catches a line of its shape, and passes the nearest line that is not one.
#[test]
fn every_detector_catches_its_shape_and_passes_a_near_miss() {
    let dashes = "-----";
    let cases: Vec<(&str, String, String)> = vec![
        (
            "private key block",
            joined(&[dashes, "BEGIN OPENSSH ", "PRIVATE KEY", dashes]),
            joined(&[dashes, "BEGIN ", "CERTIFICATE", dashes]),
        ),
        ("GitHub token", joined(&["token = gh", "p_", &token_sample(36)]), joined(&["gh", "p_", &token_sample(20)])),
        (
            "GitHub fine-grained token",
            joined(&["github", "_pat_", &token_sample(82)]),
            joined(&["github", "_pat_", &token_sample(20)]),
        ),
        ("Slack token", joined(&["xox", "b-", &token_sample(30)]), joined(&["xox", "b-", "short"])),
        (
            "Slack webhook",
            joined(&["https://hooks.slack", ".com/services/T", &uppercase_sample(8), "/B", &uppercase_sample(8), "/", &token_sample(24)]),
            joined(&["https://hooks.slack", ".com/services/"]),
        ),
        ("AWS access key id", joined(&["AK", "IA", &uppercase_sample(16)]), joined(&["AK", "IA", &uppercase_sample(15), "z"])),
        ("Anthropic key", joined(&["sk-", "ant-api03-", &token_sample(40)]), joined(&["sk-", "ant-", "short"])),
        ("OpenAI key", joined(&["sk-", "proj-", &token_sample(48)]), joined(&["task-", &token_sample(48)])),
        ("NVIDIA key", joined(&["nv", "api-", &token_sample(64)]), joined(&["nv", "api-"])),
        ("Hugging Face token", joined(&["h", "f_", &token_sample(34)]), joined(&["leh", "f_", &token_sample(34)])),
        ("Groq key", joined(&["gs", "k_", &token_sample(52)]), joined(&["gs", "k_", &token_sample(12)])),
        ("Google API key", joined(&["AI", "za", &token_sample(35)]), joined(&["AI", "za", &token_sample(10)])),
        ("xAI key", joined(&["xa", "i-", &token_sample(80)]), joined(&["xa", "i-", &token_sample(10)])),
        (
            "secret assignment",
            joined(&["\"client", "_secret\": \"", &token_sample(32), "\""]),
            joined(&["\"client", "_secret\": \"", &token_sample(12), "\""]),
        ),
        (
            "secret assignment",
            joined(&["export CLOUDFLARE_API", "_TOKEN=", &token_sample(40)]),
            joined(&["export CLOUDFLARE_API", "_TOKEN=\"$CLOUDFLARE_API", "_TOKEN\""]),
        ),
        (
            "secret assignment",
            joined(&["let nvidia_api", "_key = '", &token_sample(24), "';"]),
            joined(&["let nvidia_api", "_key = std::env::var(\"NVIDIA_API_KEY\")?;"]),
        ),
        (
            "secret assignment",
            joined(&["pass", "word: \"", &token_sample(20), "\""]),
            joined(&["pass", "word: \"correct horse battery staple\""]),
        ),
    ];
    let names: BTreeSet<&str> = cases.iter().map(|(name, _, _)| *name).collect();
    let declared: BTreeSet<&str> = DETECTORS.iter().map(|detector| detector.name).collect();
    assert_eq!(names, declared, "every detector has a case, and every case a detector");
    for (name, catch, pass) in &cases {
        let detector = DETECTORS.iter().find(|detector| detector.name == *name).unwrap();
        assert!((detector.finds)(catch), "{name} misses a line of its shape (sample length {})", catch.len());
        assert!(!(detector.finds)(pass), "{name} refuses a near miss (sample length {})", pass.len());
    }
}

/// No file in the checkout holds a secret, apart from the shapes [`VECTORS`] names.
#[test]
fn no_file_in_the_checkout_holds_a_secret() {
    let mut refusals = Vec::new();
    for path in checkout() {
        let Some(text) = text_of(&path) else { continue };
        for (line, detector) in findings_in(&text) {
            let vector = VECTORS.iter().any(|(file, name, _)| *file == path && *name == detector);
            if !vector {
                refusals.push(format!("{path}:{line}: {detector}"));
            }
        }
    }
    assert!(
        refusals.is_empty(),
        "{} line(s) of the checkout hold what looks like a secret. Move each to its owner-only file under \
         ~/.config or to the service's secret store, rotate it if it was ever pushed, and name a public test vector \
         in VECTORS with the reason:\n{}",
        refusals.len(),
        refusals.join("\n")
    );
}

/// No file in the checkout is named the way a secret is kept.
#[test]
fn no_file_in_the_checkout_is_named_like_a_secret() {
    let named: Vec<String> = checkout().into_iter().filter(|path| is_named_like_a_secret(path)).collect();
    assert!(named.is_empty(), "the checkout holds files named like secrets: {named:?}");
}

/// The naming test knows each name a secret is kept under, and leaves the source files beside them alone.
#[test]
fn the_secret_names_are_the_ones_secrets_are_kept_under() {
    for path in [
        ".env",
        "services/heal/.env.production",
        "services/heal/.dev.vars",
        "deploy.credentials.json",
        "keys/id_ed25519",
        "keys/id_rsa_deploy",
        "signing/release.p12",
        "signing/release.pfx",
        "home/.netrc",
        "home/.pgpass",
    ] {
        assert!(is_named_like_a_secret(path), "{path} is not taken for a secret's file");
    }
    for path in ["apps/logicaffeine_cli/src/project/credentials.rs", "keys/id_ed25519.pub", "keys/id_rsa_deploy.pub", "docs/env.md", "src/environment.rs"] {
        assert!(!is_named_like_a_secret(path), "{path} is taken for a secret's file");
    }
}

/// `.gitignore` keeps a secret's file out of every directory, as git itself reads it.
#[test]
fn the_ignore_file_keeps_every_secrets_file_out() {
    let ignore = std::fs::read_to_string(repo_root().join(".gitignore")).expect("read .gitignore");
    for rule in [".env", ".env.*", ".dev.vars", "**/.dev.vars", "*.credentials*"] {
        assert!(ignore.lines().any(|line| line.trim() == rule), ".gitignore lacks the rule {rule}");
    }
    for path in [
        ".env",
        ".env.local",
        "services/heal/.env",
        "services/heal/.env.production",
        "services/heal/.dev.vars",
        "apps/logicaffeine_web/deploy.credentials.json",
        "scripts/.credentials",
    ] {
        let status = Command::new("git")
            .args(["check-ignore", "--no-index", "-q", path])
            .current_dir(repo_root())
            .status()
            .expect("git check-ignore");
        assert!(status.success(), "git does not ignore {path}");
    }
}

/// Each row of [`VECTORS`] names a file in the checkout that its detector still finds, and says why it is no secret.
#[test]
fn every_vector_is_still_needed() {
    let files: BTreeSet<String> = checkout().into_iter().collect();
    for (file, name, reason) in VECTORS {
        assert!(DETECTORS.iter().any(|detector| detector.name == *name), "VECTORS names the unknown detector {name}");
        assert!(!reason.trim().is_empty(), "VECTORS gives no reason for {file}");
        assert!(files.contains(*file), "VECTORS names {file}, which is not in the checkout");
        let text = text_of(file).unwrap_or_else(|| panic!("{file} is binary"));
        assert!(
            findings_in(&text).iter().any(|(_, detector)| detector == name),
            "VECTORS names {file} for {name}, which no longer finds anything there: drop the row"
        );
    }
}
