//! Every path a script writes relative to its own location resolves to what it names.
//!
//! A script under `scripts/` finds the repository and its siblings from where it sits: `$(dirname "$0")/..`, a
//! variable holding one (`ROOT="$(cd "$(dirname "$0")/.." && pwd)"`, then `$ROOT/crates/…`),
//! `Path(__file__).resolve().parents[1] / "x"`, `os.path.dirname(os.path.dirname(os.path.abspath(__file__)))`,
//! `path.join(__dirname, '../x')`, `#[path = "../x"]`. Moving a script to another folder changes what each of
//! them resolves to, and nothing fails until the script runs. This lock resolves every one statically and
//! requires:
//! - a root variable (`ROOT`, `REPO`, `REPO_ROOT`, `repo_root`, …) names the workspace root;
//! - a path that only climbs names the root, or a directory below a script's own folder — never `scripts/` or one
//!   of its folders, where a climb lands when its script moved and the climb did not;
//! - nothing leaves the repository (a sibling checkout is an argument or an environment variable, never assumed);
//! - a path into `scripts/` names a file or folder the checkout holds (nothing is generated there);
//! - any other path with a literal tail names something whose directory the checkout holds (an output a script
//!   writes need not exist yet), a location git ignores (build output, logs, fetched tools), or the repository's
//!   own `.git/`.
//!
//! Every anchor a script writes is one of the recognized forms, so the check sees them all.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test scripts_root_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/public_manifest.rs"]
mod public_manifest;

use public_manifest::{checkout, git_ignored, repo_root};
use std::collections::{BTreeMap, BTreeSet};

/// A path the script names relative to its own location.
#[derive(Debug, Clone, PartialEq)]
struct Anchored {
    /// Byte span in the script's text.
    span: (usize, usize),
    /// Repo-relative, normalized; a leading `..` means it left the repository.
    target: String,
    /// It only climbs: no literal tail below the directory it lands on.
    climb: bool,
    /// The variable it is assigned to, when it is.
    var: Option<String>,
}

fn dirname(p: &str) -> &str {
    p.rsplit_once('/').map_or("", |(d, _)| d)
}

fn normalize(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in p.split('/') {
        match part {
            "" | "." => {}
            ".." if parts.last().is_some_and(|l| *l != "..") => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn join(base: &str, tail: &str) -> String {
    normalize(&format!("{base}/{tail}"))
}

fn is_lit(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_.+@%-/".contains(c)
}

/// The literal path characters of `text` from `at`: `/../benchmarks/silicon`.
fn literal_tail(text: &str, at: usize) -> &str {
    let rest = &text[at..];
    if !rest.starts_with('/') {
        return "";
    }
    let end = rest.find(|c: char| !is_lit(c)).unwrap_or(rest.len());
    &rest[..end]
}

/// `/../..`: it climbs at least once and names nothing below where it lands.
fn only_climbs(tail: &str) -> bool {
    tail.split('/').any(|p| p == "..") && tail.split('/').all(|p| p.is_empty() || p == "..")
}

/// `a` is `b` or a directory above it.
fn is_ancestor(a: &str, b: &str) -> bool {
    a.is_empty() || a == b || b.starts_with(&format!("{a}/"))
}

/// Where the block holding a Python assignment at `line_start` (indented `indent`) ends: the next code line
/// indented less. Module level: the end of the file.
fn py_scope_end(text: &str, line_start: usize, indent: usize) -> usize {
    if indent == 0 {
        return text.len();
    }
    let mut pos = text[line_start..].find('\n').map(|i| line_start + i);
    while let Some(nl) = pos {
        let next = nl + 1;
        let eol = text[next..].find('\n').map_or(text.len(), |i| next + i);
        let line = &text[next..eol];
        let code = line.trim_start();
        if !code.is_empty() && !code.starts_with('#') && line.len() - code.len() < indent {
            return next;
        }
        pos = (eol < text.len()).then_some(eol);
    }
    text.len()
}

/// Every occurrence of `needle` in `text`.
fn find_all<'a>(text: &'a str, needle: &'a str) -> impl Iterator<Item = usize> + 'a {
    text.match_indices(needle).map(|(i, _)| i)
}

const SH_ATOMS: &[&str] = &[
    "$(dirname \"$0\")",
    "$(dirname $0)",
    "$(dirname \"${BASH_SOURCE[0]}\")",
    "$(dirname \"${BASH_SOURCE}\")",
    "$(dirname \"$BASH_SOURCE\")",
    "$(dirname ${BASH_SOURCE[0]})",
];

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The variable a value at `at` is assigned to: `NAME=`, `NAME="$(cd "`, `NAME = ` at the start of its line.
fn assigned_var(text: &str, at: usize, python: bool) -> Option<String> {
    let line_start = text[..at].rfind(['\n', ';']).map_or(0, |i| i + 1);
    let mut head = text[line_start..at].trim_start();
    for kw in ["local ", "export ", "readonly "] {
        head = head.strip_prefix(kw).unwrap_or(head);
    }
    let head = if python {
        head.trim_end().strip_suffix('=')?.trim_end()
    } else {
        let h = head
            .strip_suffix("$(cd \"")
            .or_else(|| head.strip_suffix("\"$(cd \""))
            .unwrap_or(head);
        let h = h.strip_suffix('"').unwrap_or(h);
        h.strip_suffix('=')?
    };
    (!head.is_empty()
        && head.chars().all(is_ident)
        && !head.starts_with(|c: char| c.is_ascii_digit()))
    .then(|| head.to_string())
}

fn sh_anchors(path: &str, text: &str) -> Vec<Anchored> {
    let d = dirname(path);
    let mut out = Vec::new();
    let mut vars: Vec<(String, String)> = Vec::new();
    for atom in SH_ATOMS {
        for at in find_all(text, atom) {
            let tail = literal_tail(text, at + atom.len());
            let target = join(d, tail);
            let var = assigned_var(text, at, false);
            if let Some(v) = &var {
                vars.push((v.clone(), target.clone()));
            }
            out.push(Anchored {
                span: (at, at + atom.len() + tail.len()),
                target,
                climb: only_climbs(tail),
                var,
            });
        }
    }
    for (var, value) in vars {
        for head in [
            format!("${{{var}}}"),
            format!("${var}"),
            format!("\"${var}\""),
            format!("\"${{{var}}}\""),
        ] {
            for at in find_all(text, &head) {
                let after = at + head.len();
                if head.starts_with('$') && text[after..].starts_with(is_ident) {
                    continue;
                }
                let tail = literal_tail(text, after);
                if tail.trim_matches('/').is_empty() {
                    continue;
                }
                out.push(Anchored {
                    span: (at, after + tail.len()),
                    target: join(&value, tail),
                    climb: only_climbs(tail),
                    var: None,
                });
            }
        }
    }
    out
}

/// A Python string literal at `at` (`"…"` or `'…'`), as (contents, end).
fn py_string(text: &str, at: usize) -> Option<(String, usize)> {
    let q = text[at..]
        .chars()
        .next()
        .filter(|c| *c == '"' || *c == '\'')?;
    let close = text[at + 1..].find([q, '\n'])? + at + 1;
    (text[close..].starts_with(q)).then(|| (text[at + 1..close].to_string(), close + 1))
}

fn skip_ws(text: &str, at: usize) -> usize {
    at + text[at..].len() - text[at..].trim_start_matches([' ', '\t']).len()
}

/// ` / "a" / 'b/c'` from `at`: the parts and the end.
fn py_divs(text: &str, mut at: usize) -> (Vec<String>, usize) {
    let mut parts = Vec::new();
    loop {
        let s = skip_ws(text, at);
        if !text[s..].starts_with('/') {
            return (parts, at);
        }
        let s = skip_ws(text, s + 1);
        match py_string(text, s) {
            Some((lit, end)) => {
                parts.push(lit);
                at = end;
            }
            None => return (parts, at),
        }
    }
}

/// `.parent`, `.parents[k]`, `.with_name("x")` steps from `at` applied to `cur`: the new path, the end, and
/// whether any step ran.
fn py_steps(text: &str, mut at: usize, mut cur: String) -> (String, usize, bool) {
    let mut stepped = false;
    loop {
        let rest = &text[at..];
        if let Some(r) = rest.strip_prefix(".parents[") {
            let Some(close) = r.find(']') else { break };
            let Ok(k) = r[..close].parse::<usize>() else {
                break;
            };
            for _ in 0..=k {
                cur = dirname(&cur).to_string();
            }
            at += ".parents[".len() + close + 1;
        } else if rest.starts_with(".parent") && !rest[".parent".len()..].starts_with(is_ident) {
            cur = dirname(&cur).to_string();
            at += ".parent".len();
        } else if let Some(r) = rest.strip_prefix(".with_name(") {
            let s = skip_ws(r, 0);
            let Some((name, end)) = py_string(r, s) else {
                break;
            };
            let close = skip_ws(r, end);
            if !r[close..].starts_with(')') {
                break;
            }
            cur = join(dirname(&cur), &name);
            at += ".with_name(".len() + close + 1;
        } else {
            break;
        }
        stepped = true;
    }
    (cur, at, stepped)
}

const PY_FILE_HEADS: &[&str] = &[
    "pathlib.Path(__file__).resolve()",
    "pathlib.Path(__file__).absolute()",
    "pathlib.Path(__file__)",
    "Path(__file__).resolve()",
    "Path(__file__).absolute()",
    "Path(__file__)",
];

/// How many times `name` is bound in a Python file: assigned (`name =`, `name :=`), a loop or `with` target, or a
/// parameter. A name bound more than once may hold something else where it is read.
fn py_bindings(text: &str, name: &str) -> usize {
    let mut n = 0;
    for at in find_all(text, name) {
        if at > 0 && text[..at].ends_with(|c: char| is_ident(c) || c == '.') {
            continue;
        }
        let after = &text[at + name.len()..];
        if after.starts_with(is_ident) {
            continue;
        }
        let rest = after.trim_start_matches([' ', '\t']);
        let assigned = (rest.starts_with('=') && !rest.starts_with("==")) || rest.starts_with(":=");
        let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let before = text[line_start..at].trim_start();
        let looped = before.starts_with("for ") && rest.starts_with("in ");
        let with_as = before.ends_with(" as ") || before.ends_with(" as(");
        let param = before.starts_with("def ") && before.contains('(') && !before.contains(')');
        if assigned || looped || with_as || param {
            n += 1;
        }
    }
    n
}

fn py_anchors(path: &str, text: &str) -> Vec<Anchored> {
    let d = dirname(path);
    let mut out: Vec<Anchored> = Vec::new();
    let mut taken: BTreeSet<usize> = BTreeSet::new();
    for head in PY_FILE_HEADS {
        for at in find_all(text, head) {
            if !taken.insert(at)
                || (at > 0 && text[..at].ends_with("pathlib.")) && !head.starts_with("pathlib.")
            {
                continue;
            }
            let (dir, end, stepped) = py_steps(text, at + head.len(), path.to_string());
            let (parts, end2) = py_divs(text, end);
            let target = parts.iter().fold(dir.clone(), |acc, p| join(&acc, p));
            let climb = parts.is_empty() && stepped && dir != d && is_ancestor(&dir, d);
            let var = assigned_var(text, at, true);
            out.push(Anchored {
                span: (at, end2),
                target,
                climb,
                var,
            });
        }
    }
    // os.path.dirname(...(os.path.abspath(__file__))), optionally inside os.path.join(<that>, "..", "a")
    for at in find_all(text, "path.dirname(") {
        let start = if text[..at].ends_with("_os.") {
            at - 4
        } else if text[..at].ends_with("os.") {
            at - 3
        } else {
            continue;
        };
        if out.iter().any(|a| a.span.0 <= start && start < a.span.1) {
            continue;
        }
        let mut n = 0;
        let mut i = start;
        loop {
            let rest = &text[i..];
            let step = ["_os.path.dirname(", "os.path.dirname("]
                .iter()
                .find(|p| rest.starts_with(**p));
            match step {
                Some(p) => {
                    n += 1;
                    i = skip_ws(text, i + p.len());
                }
                None => break,
            }
        }
        let rest = &text[i..];
        let inner = [
            "os.path.abspath(__file__)",
            "_os.path.abspath(__file__)",
            "os.path.realpath(__file__)",
            "_os.path.realpath(__file__)",
            "__file__",
        ]
        .iter()
        .find(|p| rest.starts_with(**p));
        let Some(inner) = inner else { continue };
        let mut end = i + inner.len();
        let mut closed = 0;
        while closed < n {
            end = skip_ws(text, end);
            if !text[end..].starts_with(')') {
                break;
            }
            end += 1;
            closed += 1;
        }
        if closed < n {
            continue;
        }
        let mut dir = path.to_string();
        for _ in 0..n {
            dir = dirname(&dir).to_string();
        }
        // a join around it
        let before = &text[..start];
        let trimmed = before.trim_end_matches([' ', '\t', '\n']);
        let join_head = ["_os.path.join(", "os.path.join("]
            .iter()
            .find(|p| trimmed.ends_with(**p));
        if let Some(jh) = join_head {
            let jstart = trimmed.len() - jh.len();
            let mut parts = Vec::new();
            let mut j = end;
            loop {
                let s = skip_ws(text, j);
                let s = if text[s..].starts_with('\n') {
                    skip_ws(text, s + 1)
                } else {
                    s
                };
                if !text[s..].starts_with(',') {
                    j = s;
                    break;
                }
                let s = skip_ws(text, s + 1);
                let s = if text[s..].starts_with('\n') {
                    skip_ws(text, s + 1)
                } else {
                    s
                };
                if let Some((lit, e)) = py_string(text, s) {
                    parts.push(lit);
                    j = e;
                } else if let Some(p) = ["_os.pardir", "os.pardir"]
                    .iter()
                    .find(|p| text[s..].starts_with(**p))
                {
                    parts.push("..".to_string());
                    j = s + p.len();
                } else {
                    j = s;
                    parts.clear();
                    break;
                }
            }
            if text[j..].starts_with(')') && !parts.is_empty() {
                let target = parts.iter().fold(dir.clone(), |acc, p| join(&acc, p));
                let climb = parts.iter().all(|p| p == "..");
                let var = assigned_var(text, jstart, true).or_else(|| {
                    let b = text[..jstart].trim_end_matches([' ', '\t']);
                    b.strip_suffix("os.path.normpath(")
                        .and_then(|_| assigned_var(text, b.len() - "os.path.normpath(".len(), true))
                });
                out.push(Anchored {
                    span: (jstart, j + 1),
                    target,
                    climb,
                    var,
                });
                continue;
            }
        }
        let var = assigned_var(text, start, true);
        out.push(Anchored {
            span: (start, end),
            target: dir.clone(),
            climb: n > 1,
            var,
        });
    }
    // variables, within the block their assignment sits in: `NAME / "x"`, `NAME.parent / "x"`,
    // `os.path.join(NAME, "a", "b")`
    let vars: Vec<(String, String, usize, usize)> = out
        .iter()
        .filter_map(|a| {
            let v = a.var.clone()?;
            if py_bindings(text, &v) != 1 {
                return None;
            }
            let line_start = text[..a.span.0].rfind('\n').map_or(0, |i| i + 1);
            let indent =
                text[line_start..].len() - text[line_start..].trim_start_matches([' ', '\t']).len();
            Some((
                v,
                a.target.clone(),
                a.span.1,
                py_scope_end(text, line_start, indent),
            ))
        })
        .collect();
    let mut uses = Vec::new();
    for (var, value, from, to) in &vars {
        let in_scope = |at: usize| *from <= at && at < *to;
        for at in find_all(text, var).filter(|at| in_scope(*at)) {
            if at > 0 && text[..at].ends_with(|c: char| is_ident(c) || c == '.') {
                continue;
            }
            let after = at + var.len();
            if text[after..].starts_with(is_ident) {
                continue;
            }
            let (dir, end, _) = py_steps(text, after, value.clone());
            let (parts, end2) = py_divs(text, end);
            if parts.is_empty() {
                continue;
            }
            let target = parts.iter().fold(dir, |acc, p| join(&acc, p));
            uses.push(Anchored {
                span: (at, end2),
                target,
                climb: false,
                var: None,
            });
        }
        for jh in ["os.path.join(", "_os.path.join("] {
            for at in find_all(text, &format!("{jh}{var}")).filter(|at| in_scope(*at)) {
                if at > 0 && text[..at].ends_with('_') && !jh.starts_with('_') {
                    continue;
                }
                let mut j = at + jh.len() + var.len();
                let mut parts = Vec::new();
                loop {
                    let s = skip_ws(text, j);
                    if !text[s..].starts_with(',') {
                        j = s;
                        break;
                    }
                    let s = skip_ws(text, s + 1);
                    match py_string(text, s) {
                        Some((lit, e)) => {
                            parts.push(lit);
                            j = e;
                        }
                        None => {
                            parts.clear();
                            break;
                        }
                    }
                }
                if !parts.is_empty() && text[j..].starts_with(')') {
                    let target = parts.iter().fold(value.clone(), |acc, p| join(&acc, p));
                    uses.push(Anchored {
                        span: (at, j + 1),
                        target,
                        climb: false,
                        var: None,
                    });
                }
            }
        }
    }
    out.extend(uses);
    out
}

fn js_anchors(path: &str, text: &str) -> Vec<Anchored> {
    let d = dirname(path);
    let mut out = Vec::new();
    for base in ["__dirname", "import.meta.dirname"] {
        for f in ["path.join(", "path.resolve("] {
            for at in find_all(text, &format!("{f}{base}")) {
                let s = skip_ws(text, at + f.len() + base.len());
                if !text[s..].starts_with(',') {
                    continue;
                }
                let s = skip_ws(text, s + 1);
                let q = text[s..].chars().next().unwrap_or(' ');
                if q != '\'' && q != '"' {
                    continue;
                }
                let Some(close) = text[s + 1..].find(q) else {
                    continue;
                };
                let lit = &text[s + 1..s + 1 + close];
                out.push(Anchored {
                    span: (at, s + close + 2),
                    target: join(d, lit),
                    climb: only_climbs(lit),
                    var: None,
                });
            }
        }
    }
    out
}

fn rs_anchors(path: &str, text: &str) -> Vec<Anchored> {
    let d = dirname(path);
    let mut out = Vec::new();
    for head in ["#[path = \"", "include_str!(\"", "include_bytes!(\""] {
        for at in find_all(text, head) {
            let s = at + head.len();
            let Some(close) = text[s..].find('"') else {
                continue;
            };
            let lit = &text[s..s + close];
            out.push(Anchored {
                span: (at, s + close + 1),
                target: join(d, lit),
                climb: false,
                var: None,
            });
        }
    }
    out
}

fn language(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.').map(|(_, e)| e) {
        Some("sh") | Some("bash") | None => "sh",
        Some("py") => "py",
        Some("js") | Some("mjs") | Some("cjs") => "js",
        Some("rs") => "rs",
        _ => "",
    }
}

fn anchors(path: &str, text: &str) -> Vec<Anchored> {
    match language(path) {
        "sh" => sh_anchors(path, text),
        "py" => py_anchors(path, text),
        "js" => js_anchors(path, text),
        "rs" => rs_anchors(path, text),
        _ => Vec::new(),
    }
}

/// The spellings that derive a path from a script's own location. Each occurrence must sit inside an anchor the
/// recognizer resolved.
fn anchor_tokens(path: &str) -> &'static [&'static str] {
    match language(path) {
        "sh" => &[
            "dirname \"$0\"",
            "dirname $0",
            "dirname \"${BASH_SOURCE",
            "dirname \"$BASH_SOURCE",
            "dirname ${BASH_SOURCE",
            "BASH_SOURCE%/*",
        ],
        "py" => &[
            "Path(__file__)",
            "path.dirname(os.path.abspath(__file__",
            "path.dirname(os.path.realpath(__file__",
            "path.dirname(__file__",
        ],
        "js" => &["__dirname", "import.meta.dirname"],
        "rs" => &["#[path = \"", "include_str!(\"", "include_bytes!(\""],
        _ => &[],
    }
}

const ROOT_VARS: &[&str] = &[
    "root",
    "repo",
    "repo_root",
    "reporoot",
    "workspace",
    "workspace_root",
    "root_dir",
    "repo_dir",
];

struct Tree<'a> {
    files: &'a BTreeSet<String>,
    dirs: &'a BTreeSet<String>,
}

impl Tree<'_> {
    fn holds_dir(&self, d: &str) -> bool {
        d.is_empty() || self.dirs.contains(d)
    }
    fn holds(&self, p: &str) -> bool {
        self.files.contains(p) || self.holds_dir(p)
    }
}

/// What is wrong with the anchored paths of the script at `path`; paths git may ignore are returned for the
/// caller to ask about (their directory need not exist).
fn check(
    path: &str,
    text: &str,
    tree: &Tree,
    script_folders: &BTreeSet<String>,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut wrong = Vec::new();
    let mut ask = Vec::new();
    let found = anchors(path, text);
    let line = |at: usize| text[..at].matches('\n').count() + 1;
    for a in &found {
        let expr = &text[a.span.0..a.span.1];
        let at = format!(
            "{path}:{}: `{}`",
            line(a.span.0),
            expr.lines().next().unwrap_or(expr)
        );
        if a.target.starts_with("..") {
            wrong.push(format!("{at} leaves the repository ({}); pass the location as an argument or environment variable", a.target));
            continue;
        }
        if let Some(v) = &a.var {
            if ROOT_VARS.contains(&v.to_ascii_lowercase().as_str()) && !a.target.is_empty() {
                wrong.push(format!(
                    "{at} assigns `{v}` = `{}`, not the workspace root",
                    a.target
                ));
                continue;
            }
        }
        if a.climb && (a.target == "scripts" || script_folders.contains(&a.target)) {
            wrong.push(format!(
                "{at} climbs to `{}/` — its script moved and the climb did not",
                a.target
            ));
            continue;
        }
        if tree.holds(&a.target) || a.target == ".git" || a.target.starts_with(".git/") {
            continue;
        }
        if a.target.starts_with("scripts/") {
            ask.push((
                a.target.clone(),
                format!("{at} names `{}`, which scripts/ does not hold", a.target),
            ));
            continue;
        }
        if tree.holds_dir(dirname(&a.target)) {
            continue;
        }
        ask.push((
            a.target.clone(),
            format!(
                "{at} names `{}`, whose directory the checkout does not hold",
                a.target
            ),
        ));
    }
    for token in anchor_tokens(path) {
        for at in find_all(text, token) {
            if !found.iter().any(|a| a.span.0 <= at && at < a.span.1) {
                let l = text[..at].rfind('\n').map_or(0, |i| i + 1);
                let r = text[at..].find('\n').map_or(text.len(), |i| at + i);
                let src = text[l..r].trim();
                let comment = src.starts_with('#') || src.starts_with("//") || src.starts_with('*');
                if !comment {
                    wrong.push(format!("{path}:{}: `{src}` derives a path from the script's location in a form this lock does not resolve; write it in one of the forms the module docs list", line(at)));
                }
            }
        }
    }
    (wrong, ask)
}

fn scripts_in(files: &BTreeSet<String>) -> Vec<String> {
    files
        .iter()
        .filter(|p| {
            p.starts_with("scripts/") && !p.contains("/node_modules/") && !language(p).is_empty()
        })
        .filter(|p| {
            let name = p.rsplit('/').next().unwrap_or("");
            name.contains('.')
                || std::fs::read(repo_root().join(p))
                    .map(|b| b.starts_with(b"#!"))
                    .unwrap_or(false)
        })
        .cloned()
        .collect()
}

#[test]
fn every_anchored_path_in_scripts_resolves() {
    let files: BTreeSet<String> = checkout().into_iter().collect();
    let dirs: BTreeSet<String> = files
        .iter()
        .flat_map(|f| {
            let mut out = Vec::new();
            let mut d = dirname(f);
            while !d.is_empty() {
                out.push(d.to_string());
                d = dirname(d);
            }
            out
        })
        .collect();
    let script_folders: BTreeSet<String> = dirs
        .iter()
        .filter(|d| d.starts_with("scripts/") && !d["scripts/".len()..].contains('/'))
        .cloned()
        .collect();
    let tree = Tree {
        files: &files,
        dirs: &dirs,
    };
    let root = repo_root();
    let mut wrong = Vec::new();
    let mut ask: Vec<(String, String)> = Vec::new();
    let scripts = scripts_in(&files);
    assert!(
        scripts.len() > 200,
        "only {} scripts found under scripts/ — the scan is broken",
        scripts.len()
    );
    let mut seen = 0;
    for p in &scripts {
        let text = std::fs::read_to_string(root.join(p)).unwrap_or_default();
        seen += anchors(p, &text).len();
        let (w, a) = check(p, &text, &tree, &script_folders);
        wrong.extend(w);
        ask.extend(a);
    }
    assert!(
        seen > 400,
        "only {seen} anchored paths found — the recognizer stopped recognizing"
    );
    let ignored = git_ignored(&ask.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>());
    wrong.extend(
        ask.into_iter()
            .zip(ignored)
            .filter(|(_, ig)| !ig)
            .map(|((_, msg), _)| msg),
    );
    wrong.sort();
    assert!(
        wrong.is_empty(),
        "\n{} anchored path(s) in scripts/ do not resolve to what they name:\n  {}\n",
        wrong.len(),
        wrong.join("\n  ")
    );
}

/// The checker, over a tree of its own: a script at `path` with `text`, the tree holding `files`.
fn check_fixture(path: &str, text: &str, files: &[&str]) -> Vec<String> {
    let files: BTreeSet<String> = files.iter().map(|s| s.to_string()).collect();
    let dirs: BTreeSet<String> = files
        .iter()
        .flat_map(|f| {
            let mut out = Vec::new();
            let mut d = dirname(f);
            while !d.is_empty() {
                out.push(d.to_string());
                d = dirname(d);
            }
            out
        })
        .collect();
    let folders: BTreeSet<String> = dirs
        .iter()
        .filter(|d| d.starts_with("scripts/") && !d["scripts/".len()..].contains('/'))
        .cloned()
        .collect();
    let (mut wrong, ask) = check(
        path,
        text,
        &Tree {
            files: &files,
            dirs: &dirs,
        },
        &folders,
    );
    wrong.extend(ask.into_iter().map(|(_, m)| m));
    wrong
}

const TREE: &[&str] = &[
    "Cargo.toml",
    "crates/logicaffeine_kernel/src/lib.rs",
    "benchmarks/silicon/sky130_signoff.json",
    "scripts/run-all-tests-fast.sh",
    "scripts/lean/conformance-ratchet.py",
    "scripts/lean/test-arena-only.py",
    "scripts/lean/arena-crown/snapshot.sh",
    "scripts/hw/verify-fpu.sh",
    "scripts/accel/runpod-lib.sh",
    "scripts/accel/runpod-up.sh",
    "scripts/web/test-ssg-render.mjs",
    "scripts/lean/lean-harvest-measure.rs",
];

#[test]
fn the_recognizer_resolves_each_form() {
    let targets = |path: &str, text: &str| {
        anchors(path, text)
            .into_iter()
            .map(|a| a.target)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        targets(
            "scripts/hw/x.sh",
            "cd \"$(dirname \"$0\")/../../benchmarks/silicon\""
        ),
        vec!["benchmarks/silicon"]
    );
    assert_eq!(
        targets("scripts/lean/x.sh", "ROOT=\"$(cd \"$(dirname \"${BASH_SOURCE[0]}\")/../..\" && pwd)\"\nbash \"$ROOT/scripts/lean/y.sh\""),
        vec!["", "scripts/lean/y.sh"]
    );
    assert_eq!(
        targets(
            "scripts/accel/x.sh",
            "source \"$(dirname \"$0\")/runpod-lib.sh\""
        ),
        vec!["scripts/accel/runpod-lib.sh"]
    );
    assert_eq!(
        targets(
            "scripts/lean/x.py",
            "ROOT = Path(__file__).resolve().parents[2]\nS = ROOT / \"scripts/lean/y.py\""
        ),
        vec!["", "scripts/lean/y.py"]
    );
    assert_eq!(
        targets(
            "scripts/lean/x.py",
            "spec = f(Path(__file__).with_name(\"conformance-ratchet.py\"))"
        ),
        vec!["scripts/lean/conformance-ratchet.py"]
    );
    assert_eq!(
        targets(
            "scripts/lean/tests/t.py",
            "S = Path(__file__).resolve().parents[1] / \"lean-research-corpus.py\""
        ),
        vec!["scripts/lean/lean-research-corpus.py"]
    );
    assert_eq!(
        targets(
            "scripts/lean/arena-current/t.py",
            "HERE = Path(__file__).resolve().parent\nR = HERE.parent / 'conformance-ratchet.py'"
        ),
        vec![
            "scripts/lean/arena-current",
            "scripts/lean/conformance-ratchet.py"
        ]
    );
    assert_eq!(
        targets(
            "scripts/bench/x.py",
            "ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))"
        ),
        vec![""]
    );
    assert_eq!(
        targets("scripts/hw/x.py", "ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), \"..\", \"..\"))"),
        vec![""]
    );
    assert_eq!(
        targets("scripts/hw/x.py", "src = _os.path.join(_os.path.dirname(__file__), _os.pardir, _os.pardir, \"benchmarks\", \"silicon\", \"a.json\")"),
        vec!["benchmarks/silicon/a.json"]
    );
    assert_eq!(targets("scripts/bench/x.py", "here = os.path.dirname(os.path.abspath(__file__))\np = os.path.join(here, \"..\", \"..\", \"Cargo.toml\")"), vec!["scripts/bench", "Cargo.toml"]);
    assert_eq!(
        targets(
            "scripts/web/x.mjs",
            "path.join(import.meta.dirname, '../../apps/w/public/sitemap.xml')"
        ),
        vec!["apps/w/public/sitemap.xml"]
    );
    assert_eq!(
        targets(
            "scripts/lean/x.rs",
            "#[path = \"../../crates/t/tests/c.rs\"]\nmod c;"
        ),
        vec!["crates/t/tests/c.rs"]
    );
}

#[test]
fn correct_anchors_pass() {
    let ok = [
        ("scripts/hw/verify-fpu.sh", "cd \"$(dirname \"$0\")/../../benchmarks/silicon\"\n"),
        ("scripts/lean/arena-crown/snapshot.sh", "ROOT=\"$(cd \"$(dirname \"$0\")/../../..\" && pwd)\"\ncat \"$ROOT/Cargo.toml\"\n"),
        ("scripts/accel/runpod-up.sh", "source \"$(dirname \"$0\")/runpod-lib.sh\"\n"),
        ("scripts/lean/test-arena-only.py", "spec = f(Path(__file__).with_name(\"conformance-ratchet.py\"))\nROOT = Path(__file__).resolve().parents[2]\n"),
        ("scripts/web/test-ssg-render.mjs", "const s = path.join(import.meta.dirname, '../../crates/logicaffeine_kernel/src/lib.rs');\n"),
        ("scripts/run-all-tests-fast.sh", "SCRIPT_DIR=\"$(cd \"$(dirname \"${BASH_SOURCE[0]}\")\" && pwd)\"\n\"$SCRIPT_DIR/lean/conformance-ratchet.py\"\n"),
    ];
    for (path, text) in ok {
        let found = check_fixture(path, text, TREE);
        assert!(found.is_empty(), "{path}: {found:?}");
    }
}

#[test]
fn a_script_moved_without_its_paths_is_caught() {
    // each of these was right one folder up
    let caught = [
        ("scripts/hw/verify-fpu.sh", "cd \"$(dirname \"$0\")/../benchmarks/silicon\"\n", "which scripts/ does not hold"),
        ("scripts/lean/x.sh", "ROOT=\"$(cd \"$(dirname \"${BASH_SOURCE[0]}\")/..\" && pwd)\"\n", "not the workspace root"),
        ("scripts/lean/x.sh", "cd \"$(dirname \"$0\")/..\"\n", "climbs to `scripts/`"),
        ("scripts/lean/arena-crown/snapshot.sh", "cd \"$(dirname \"$0\")/../..\"\n", "climbs to `scripts/`"),
        ("scripts/lean/x.py", "ROOT = Path(__file__).resolve().parents[1]\n", "not the workspace root"),
        ("scripts/lean/x.py", "ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))\n", "not the workspace root"),
        ("scripts/accel/x.sh", "source \"$(dirname \"$0\")/conformance-ratchet.py\"\n", "which scripts/ does not hold"),
        ("scripts/web/x.mjs", "path.join(import.meta.dirname, '../apps/x/public/sitemap.xml')", "which scripts/ does not hold"),
        ("scripts/lean/x.rs", "#[path = \"../crates/logicaffeine_kernel/src/lib.rs\"]\n", "which scripts/ does not hold"),
        ("scripts/lean/arena-current/t.py", "HERE = Path(__file__).resolve().parent\nR = HERE.parent / 'scripts/lean/conformance-ratchet.py'\n", "which scripts/ does not hold"),
    ];
    for (path, text, why) in caught {
        let found = check_fixture(path, text, TREE);
        assert!(
            found.iter().any(|m| m.contains(why)),
            "{path} `{}` should fail ({why}): {found:?}",
            text.trim()
        );
    }
}

#[test]
fn a_path_out_of_the_repository_or_an_unknown_form_is_caught() {
    let out = check_fixture(
        "scripts/web/m.js",
        "const f = path.join(__dirname, '../../../elsewhere/sets/a.ts');",
        TREE,
    );
    assert!(
        out.iter().any(|m| m.contains("leaves the repository")),
        "{out:?}"
    );
    let odd = check_fixture(
        "scripts/web/m.js",
        "const f = path.join(__dirname, `../x/${name}.ts`);",
        TREE,
    );
    assert!(
        odd.iter().any(|m| m.contains("does not resolve")),
        "{odd:?}"
    );
    let odd = check_fixture("scripts/dev/x.sh", "D=\"${BASH_SOURCE%/*}/..\"\n", TREE);
    assert!(
        odd.iter().any(|m| m.contains("does not resolve")),
        "{odd:?}"
    );
}

#[test]
fn the_anchor_census_covers_each_language() {
    let files: BTreeSet<String> = checkout().into_iter().collect();
    let mut by_language: BTreeMap<&str, usize> = BTreeMap::new();
    for p in scripts_in(&files) {
        let text = std::fs::read_to_string(repo_root().join(&p)).unwrap_or_default();
        *by_language.entry(language(&p)).or_default() += anchors(&p, &text).len();
    }
    for lang in ["sh", "py", "js", "rs"] {
        assert!(
            by_language.get(lang).copied().unwrap_or(0) > 0,
            "no {lang} anchors recognized in scripts/: {by_language:?}"
        );
    }
}

/// The ignore oracle keeps git's own answer for every path the checkout really holds: a tracked file under
/// real directories is not ignored, and a path under an ignored directory is. Only a path beneath a symlinked
/// ancestor is answered without asking git.
#[test]
fn the_ignore_oracle_keeps_gits_answer_under_real_directories() {
    let asked = vec![
        "crates/logicaffeine_tests/Cargo.toml".to_string(),
        "crates/logicaffeine_tests/tests/scripts_root_lock.rs".to_string(),
        "target/debug/anything".to_string(),
    ];
    assert_eq!(git_ignored(&asked), vec![false, false, true], "{asked:?}");
}

/// A rule decides a path's answer only when it ignores: a path a negated rule (`!…`) keeps is public, as is a path no
/// rule matches. `git check-ignore --verbose` prints the deciding rule either way, a negation with its `!`.
#[test]
fn a_path_a_negated_rule_keeps_is_not_ignored() {
    let printed = b".gitignore\x00121\x00!benchmarks/silicon/kept.json\x00benchmarks/silicon/kept.json\x00\
                    .gitignore\x0099\x00benchmarks/silicon/*.json\x00benchmarks/silicon/dropped.json\x00\
                    \x00\x00\x00README.md\x00";
    assert_eq!(public_manifest::ignored_by(printed, 3), vec![false, true, false]);
    let gitignore = std::fs::read_to_string(repo_root().join(".gitignore")).unwrap();
    let kept: Vec<String> = gitignore
        .lines()
        .filter_map(|l| l.strip_prefix('!'))
        .filter(|p| !p.contains(['*', '?', '[']) && !p.ends_with('/'))
        .map(|p| p.trim_start_matches('/').to_string())
        .collect();
    assert!(!kept.is_empty(), ".gitignore keeps no file by name");
    let ignored = git_ignored(&kept);
    let dropped: Vec<&String> = kept.iter().zip(&ignored).filter(|(_, i)| **i).map(|(p, _)| p).collect();
    assert!(dropped.is_empty(), "the oracle calls files .gitignore keeps by name ignored: {dropped:?}");
}
