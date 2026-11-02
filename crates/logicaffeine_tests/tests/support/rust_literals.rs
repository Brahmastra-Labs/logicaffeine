//! Reading string literals out of Rust source, as rustc reads them: escapes decoded, `\`
//! line continuations folded, raw strings verbatim, comments and character literals skipped. Each
//! literal knows the function (or `const`) it sits in and whether it is a `format!` template.
//! Shared by the locks that read source; each uses what it needs.
#![allow(dead_code)]

use std::collections::BTreeMap;

/// Poll `read` until two consecutive calls agree, retrying a failed call too (a `None`) rather
/// than propagating it — the general shape [`try_read_stable`] applies to the filesystem. Twenty
/// tries; `None` only if every one of them failed. Its own function so the retry/stability logic
/// is testable against a scripted `read` with no real file or real timing involved.
fn poll_until_stable(tries: u32, mut read: impl FnMut(u32) -> Option<String>) -> Option<String> {
    let mut last: Option<String> = None;
    for attempt in 0..tries {
        if let Some(text) = read(attempt) {
            if last.as_deref() == Some(text.as_str()) {
                return Some(text);
            }
            last = Some(text);
        }
    }
    last
}

/// Read a file another lane may be mid-write to — a census or corpus discovery over the shared
/// tree during a multi-lane campaign will eventually land a read between an editor's write calls.
/// Without an atomic rename, that reads old and new content concatenated: a source census sees a
/// spurious pattern (or misses a real one) and a `.lg` page reads as a page with a doubled block.
/// Read until two consecutive reads agree, retrying a transient error too (the file briefly absent
/// mid-replace, or deleted between a directory listing and this read). Never loops forever —
/// twenty tries, 5ms apart. `None` only for a file that stays unreadable for the whole window
/// (deleted for good, not merely mid-replace).
pub fn try_read_stable(path: &std::path::Path) -> Option<String> {
    poll_until_stable(20, |attempt| {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::read_to_string(path).ok()
    })
}

/// [`try_read_stable`] for a file the caller requires to exist (a listed example page, a lock file
/// this corpus names by path) — panics naming the path rather than the file's own absence.
pub fn read_stable(path: &std::path::Path) -> String {
    try_read_stable(path).unwrap_or_else(|| panic!("could not read {}", path.display()))
}

#[cfg(test)]
mod poll_until_stable_tests {
    //! `poll_until_stable` is tested against a SCRIPTED `read`, not a real file: a real torn read
    //! is a race no sleep duration makes deterministic (a small file's write is one syscall on
    //! most filesystems and essentially never truncates mid-read in a unit test's lifetime — the
    //! property this guards against is real in a multi-lane campaign's editors, on files far larger
    //! and writes far slower than anything a test could force reliably). Scripting `read` instead
    //! makes every property below exact and mutation-provable.
    use super::poll_until_stable;

    /// A `read` that answers a fixed script, one call per element, then repeats its last answer —
    /// so a script shorter than the attempt budget still has something to return.
    fn scripted(script: Vec<Option<&'static str>>) -> impl FnMut(u32) -> Option<String> {
        move |attempt: u32| {
            let i = (attempt as usize).min(script.len() - 1);
            script[i].map(str::to_string)
        }
    }

    #[test]
    fn two_identical_reads_in_a_row_settle_immediately() {
        let mut calls = 0u32;
        let result = poll_until_stable(20, |_| {
            calls += 1;
            Some("stable".to_string())
        });
        assert_eq!(result.as_deref(), Some("stable"));
        assert_eq!(
            calls, 2,
            "must stop the moment two consecutive reads agree, not keep polling"
        );
    }

    #[test]
    fn a_value_seen_only_once_is_not_returned_as_stable() {
        // "torn1" then "torn2" then "final" three times: only the run of three identical "final"
        // reads is stable. A version that settles for the FIRST read alone (the mutation this
        // proves against) would return "torn1", never seen twice.
        let result = poll_until_stable(
            20,
            scripted(vec![
                Some("torn1"),
                Some("torn2"),
                Some("final"),
                Some("final"),
            ]),
        );
        assert_eq!(
            result.as_deref(),
            Some("final"),
            "settled on a value that was never repeated"
        );
    }

    #[test]
    fn a_failed_read_does_not_reset_the_stability_run_once_a_value_repeats() {
        // reads: A, A (stable at attempt 1) — a failure never even enters the picture here; this
        // pins that two-in-a-row is sufficient and a script that stabilizes early stops early.
        let result = poll_until_stable(20, scripted(vec![Some("A"), Some("A"), Some("B")]));
        assert_eq!(result.as_deref(), Some("A"));
    }

    #[test]
    fn a_failure_between_two_agreeing_reads_is_retried_not_fatal() {
        // A, then a failed read (a transient error), then A again: the two A's are not
        // CONSECUTIVE CALLS but ARE the two most recent successful ones — must still settle on A,
        // proving a transient failure does not need to be invisible to pass, only non-fatal.
        let result = poll_until_stable(20, scripted(vec![Some("A"), None, Some("A"), Some("A")]));
        assert_eq!(result.as_deref(), Some("A"));
    }

    #[test]
    fn never_stabilizing_returns_the_last_value_read_not_none() {
        let result = poll_until_stable(5, |attempt| Some(format!("v{attempt}")));
        assert_eq!(
            result.as_deref(),
            Some("v4"),
            "the last of the 5 tries, never a stable one"
        );
    }

    #[test]
    fn every_read_failing_is_none() {
        let result = poll_until_stable(5, |_| None::<String>);
        assert_eq!(result, None);
    }

    #[test]
    fn it_never_reads_more_than_the_given_number_of_tries() {
        let mut calls = 0u32;
        poll_until_stable(7, |_| {
            calls += 1;
            None::<String>
        });
        assert_eq!(calls, 7);
    }
}

#[cfg(test)]
mod read_stable_tests {
    //! `try_read_stable`/`read_stable` against real files: only the parts real timing CAN prove
    //! reliably (a quiet file, a missing one) — the retry/stability logic itself is
    //! `poll_until_stable_tests`, above.
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "lgx_read_stable_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        p
    }

    #[test]
    fn a_quiet_file_reads_correctly() {
        let path = temp_path("quiet");
        std::fs::write(&path, "stable content").unwrap();
        assert_eq!(read_stable(&path), "stable content");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn try_read_stable_is_none_for_a_file_that_never_existed() {
        let path = temp_path("never_existed");
        std::fs::remove_file(&path).ok();
        assert_eq!(try_read_stable(&path), None);
    }

    #[test]
    #[should_panic(expected = "could not read")]
    fn read_stable_panics_naming_the_path_when_the_file_is_gone() {
        let path = temp_path("gone");
        std::fs::remove_file(&path).ok();
        read_stable(&path);
    }
}

/// One string literal: its value, the function (or `const`) it sits in, and whether it is the
/// template of a `format!`.
#[derive(Debug, Clone)]
pub struct Lit {
    pub value: String,
    pub owner: String,
    pub is_format: bool,
    pub const_name: Option<String>,
}

/// Every string literal in `src`, in order, skipping comments and character literals.
pub fn literals(src: &str) -> Vec<Lit> {
    let b: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut owner = String::from("<file>");
    let mut code_before = String::new();
    while i < b.len() {
        let c = b[i];
        // Comments.
        if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // Character literals and lifetimes: `'x'`, `'\n'`, `'\''` are skipped; `'a` is a lifetime.
        if c == '\'' {
            if b.get(i + 1) == Some(&'\\') {
                let mut j = i + 2;
                while j < b.len() && b[j] != '\'' {
                    j += 1;
                }
                i = j + 1;
                continue;
            }
            if b.get(i + 2) == Some(&'\'') {
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }
        // Raw strings: r"…", r#"…"#.
        if c == 'r'
            && (b.get(i + 1) == Some(&'"') || b.get(i + 1) == Some(&'#'))
            && (i == 0 || !(b[i - 1].is_alphanumeric() || b[i - 1] == '_'))
        {
            let mut j = i + 1;
            let mut hashes = 0;
            while b.get(j) == Some(&'#') {
                hashes += 1;
                j += 1;
            }
            if b.get(j) == Some(&'"') {
                j += 1;
                let start = j;
                loop {
                    if j >= b.len() {
                        break;
                    }
                    if b[j] == '"' && (0..hashes).all(|h| b.get(j + 1 + h) == Some(&'#')) {
                        break;
                    }
                    j += 1;
                }
                let value: String = b[start..j.min(b.len())].iter().collect();
                out.push(lit(value, &owner, &code_before));
                code_before.clear();
                i = j + 1 + hashes;
                continue;
            }
        }
        if c == '"' {
            let mut j = i + 1;
            let mut value = String::new();
            while j < b.len() && b[j] != '"' {
                if b[j] == '\\' {
                    j += 1;
                    match b.get(j) {
                        Some('n') => value.push('\n'),
                        Some('t') => value.push('\t'),
                        Some('r') => value.push('\r'),
                        Some('0') => value.push('\0'),
                        Some('\\') => value.push('\\'),
                        Some('"') => value.push('"'),
                        Some('\'') => value.push('\''),
                        Some('x') => {
                            let hex: String = b[j + 1..j + 3].iter().collect();
                            value.push(char::from(u8::from_str_radix(&hex, 16).unwrap()));
                            j += 2;
                        }
                        Some('u') => {
                            let close = (j..b.len()).find(|k| b[*k] == '}').unwrap();
                            let hex: String = b[j + 2..close].iter().collect();
                            value.push(
                                char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap(),
                            );
                            j = close;
                        }
                        Some('\n') => {
                            while matches!(b.get(j + 1), Some(' ' | '\t' | '\n' | '\r')) {
                                j += 1;
                            }
                        }
                        Some(other) => value.push(*other),
                        None => {}
                    }
                    j += 1;
                    continue;
                }
                value.push(b[j]);
                j += 1;
            }
            out.push(lit(value, &owner, &code_before));
            code_before.clear();
            i = j + 1;
            continue;
        }
        // Track the enclosing `fn`.
        if c == 'f'
            && b.get(i + 1) == Some(&'n')
            && b.get(i + 2) == Some(&' ')
            && (i == 0 || !(b[i - 1].is_alphanumeric() || b[i - 1] == '_'))
        {
            let name: String = b[i + 3..]
                .iter()
                .take_while(|ch| ch.is_alphanumeric() || **ch == '_')
                .collect();
            if !name.is_empty() {
                owner = name;
            }
        }
        code_before.push(c);
        if code_before.len() > 400 {
            let keep_from = code_before
                .char_indices()
                .rev()
                .nth(199)
                .map_or(0, |(at, _)| at);
            code_before.drain(..keep_from);
        }
        i += 1;
    }
    out
}

fn lit(value: String, owner: &str, before: &str) -> Lit {
    let trimmed = before.trim_end();
    let is_format = trimmed.ends_with("format!(");
    let const_name = trimmed.strip_suffix('=').and_then(|t| {
        let t = t.trim_end();
        let t = t
            .strip_suffix("&str")
            .map(str::trim_end)?
            .strip_suffix(':')?
            .trim_end();
        let name: String = t
            .chars()
            .rev()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let name: String = name.chars().rev().collect();
        t[..t.len() - name.len()]
            .trim_end()
            .ends_with("const")
            .then_some(name)
    });
    Lit {
        value,
        owner: const_name.clone().unwrap_or_else(|| owner.to_string()),
        is_format,
        const_name,
    }
}

/// A `format!` template resolved against the file's constants, or `None` when it has a hole no
/// constant fills.
pub fn resolve_format(template: &str, consts: &BTreeMap<String, String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if tail.starts_with("{{") {
            out.push('{');
            rest = &tail[2..];
        } else if tail.starts_with("}}") {
            out.push('}');
            rest = &tail[2..];
        } else if tail.starts_with('{') {
            let close = tail.find('}')?;
            out.push_str(consts.get(&tail[1..close])?);
            rest = &tail[close + 1..];
        } else {
            return None;
        }
    }
    out.push_str(rest);
    Some(out)
}
