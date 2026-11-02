//! No product source reads the private workspace.
//!
//! The private workspace (`work/`, and any other root entry git ignores that is not a local build or tool
//! location) is its own repository and is absent from every public checkout. A product test or script that reads
//! a path under it passes on a machine that has the workspace and fails on a public clone; an `include_str!` of
//! one stops the whole test build there. This lock finds every such read statically:
//! - a compile-time include (`include_str!`, `include_bytes!`, `#[path = "…"]`) whose target, resolved from the
//!   including file (or from `CARGO_MANIFEST_DIR`), lands in a private root;
//! - a Rust path literal handed to a join or a read (`root().join("work/…")`, `Path::new("work/…")`,
//!   `format!("work/…")`) that starts at a private root;
//! - a script path into a private root from the repository root (`$ROOT/work/…`, `ROOT / "work/…"`,
//!   `os.path.join(root, "work", …)`).
//! A read the product keeps on purpose is listed in `GUARDED`, with the guard that skips it on a public checkout
//! quoted from the file; an entry whose guard is gone, or whose read is gone, fails too.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test workspace_reads_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/public_manifest.rs"]
mod public_manifest;

use public_manifest::{checkout, git_ignored, repo_root, Classifier};

/// `(file, the read, the guard that skips it where the workspace is absent)`.
const GUARDED: &[(&str, &str, &str)] = &[(
    "scripts/bump-version.sh",
    "HOOK=\"$REPO_ROOT/work/release-guards/bump/post-bump.sh\"",
    "if [ -x \"$HOOK\" ]; then",
)];

/// `(file, what it does)`: developer tools whose OWN output lives in the workspace. They create what they read,
/// so a public checkout runs them unchanged; they are not reads of private material.
const OWN_OUTPUT: &[(&str, &str)] = &[
    (
        "scripts/dev/next-wiki.sh",
        "saves a fetched article under work/wikis/ and triages it there",
    ),
    (
        "scripts/dev/trace-wiki.sh",
        "writes its traces under work/wikis/traces/ by default",
    ),
    (
        "scripts/dev/triage-wiki.sh",
        "writes its verdicts under work/wikis/triage/ by default",
    ),
];

fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

fn dirname(p: &str) -> &str {
    p.rsplit_once('/').map_or("", |(d, _)| d)
}

/// The crate directory a file belongs to: the nearest ancestor holding a `Cargo.toml`.
fn crate_dir(rel: &str, has_manifest: &dyn Fn(&str) -> bool) -> String {
    let mut d = dirname(rel);
    while !d.is_empty() {
        if has_manifest(d) {
            return d.to_string();
        }
        d = dirname(d);
    }
    String::new()
}

fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//")
        || t.starts_with('#') && !t.starts_with("#[") && !t.starts_with("#!")
        || t.starts_with('*')
}

/// The string literal that opens at byte `at` (just past its quote), up to the closing quote.
fn literal_at(text: &str, at: usize, quote: char) -> &str {
    let rest = &text[at..];
    &rest[..rest.find(quote).unwrap_or(rest.len())]
}

/// A script line that reaches `root` from the repository root: a path component `root` anchored at a variable or a
/// command substitution (`$ROOT/work/…`, `${repo_root}/work`, `$(dirname "$0")/../work`), or a string literal that
/// starts at it and is joined, divided onto a root or assigned (`ROOT / "work/…"`, `os.path.join(root, "work", …)`,
/// `out="work/…"`). An absolute `/work/…` is some other machine's path (a container mount), not this checkout's.
fn script_reads(line: &str, root: &str) -> bool {
    let b = line.as_bytes();
    let mut from = 0;
    while let Some(at) = line[from..].find(root) {
        let i = from + at;
        let j = i + root.len();
        from = j;
        if !matches!(b.get(j), Some(b'/') | Some(b'"') | Some(b'\'') | None) || i == 0 {
            continue;
        }
        let head = &line[..i - 1];
        match b[i - 1] {
            b'/' => {
                let ident = head
                    .bytes()
                    .rev()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
                    .count();
                let variable = ident > 0 && head[..head.len() - ident].ends_with('$');
                if variable || head.ends_with('}') || head.ends_with(')') || head.ends_with("..") {
                    return true;
                }
            }
            b'"' | b'\'' => {
                let head = head.trim_end();
                let joined = (head.ends_with(',') || head.ends_with('(')) && head.contains("join(");
                let path = b.get(j) == Some(&b'/');
                if head.ends_with('/')
                    || joined
                    || path && (head.ends_with('=') || head.ends_with('('))
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Every read of a private root in one file: `(line, what)`.
fn reads(
    rel: &str,
    text: &str,
    private: &[String],
    crate_of: &dyn Fn(&str) -> String,
) -> Vec<(usize, String)> {
    let lands = |target: &str| -> bool {
        normalize(target).is_some_and(|t| {
            private
                .iter()
                .any(|p| t == p.trim_end_matches('/') || t.starts_with(p.as_str()))
        })
    };
    let starts_private = |lit: &str| -> bool {
        let lit = lit.trim_start_matches("./");
        let mut rest = lit;
        while let Some(r) = rest.strip_prefix("../") {
            rest = r;
        }
        private
            .iter()
            .any(|p| rest == p.trim_end_matches('/') || rest.starts_with(p.as_str()))
    };
    let mut out = Vec::new();
    let rust = rel.ends_with(".rs");
    let python = rel.ends_with(".py");
    let shell = rel.ends_with(".sh") || rel.ends_with(".bash");
    for (n, line) in text.lines().enumerate() {
        if is_comment(line) {
            continue;
        }
        let n = n + 1;
        if rust {
            for opener in ["include_str!(\"", "include_bytes!(\"", "#[path = \""] {
                let mut from = 0;
                while let Some(at) = line[from..].find(opener) {
                    let start = from + at + opener.len();
                    let target = literal_at(line, start, '"');
                    if lands(&format!("{}/{}", dirname(rel), target)) {
                        out.push((n, format!("compile-time include of {target}")));
                    }
                    from = start;
                }
            }
            if let Some(at) = line.find("env!(\"CARGO_MANIFEST_DIR\"), \"") {
                let start = at + "env!(\"CARGO_MANIFEST_DIR\"), \"".len();
                let target = literal_at(line, start, '"');
                if lands(&format!(
                    "{}/{}",
                    crate_of(rel),
                    target.trim_start_matches('/')
                )) {
                    out.push((
                        n,
                        format!("compile-time include of $CARGO_MANIFEST_DIR{target}"),
                    ));
                }
            }
            for call in [
                "join(\"",
                "join(&\"",
                "Path::new(\"",
                "PathBuf::from(\"",
                "read_to_string(\"",
                "read_dir(\"",
                "File::open(\"",
                "fs::read(\"",
                "format!(\"",
            ] {
                let mut from = 0;
                while let Some(at) = line[from..].find(call) {
                    let start = from + at + call.len();
                    let lit = literal_at(line, start, '"');
                    if starts_private(lit) {
                        out.push((n, format!("path literal `{lit}`")));
                    }
                    from = start;
                }
            }
        }
        if (python || shell)
            && private
                .iter()
                .any(|p| script_reads(line, p.trim_end_matches('/')))
        {
            out.push((n, line.trim().chars().take(120).collect()));
        }
    }
    out
}

/// The withheld root entries of this checkout, and `work/` wherever git ignores it: a public clone has no
/// workspace on disk, and a read of it is exactly what fails there.
fn private_roots() -> Vec<String> {
    let mut roots = Classifier::load().withheld_dirs();
    if !roots.iter().any(|r| r == "work/") && git_ignored(&["work".to_string()])[0] {
        roots.push("work/".to_string());
    }
    roots
}

#[test]
fn no_product_source_reads_the_private_workspace() {
    let root = repo_root();
    let private = private_roots();
    assert!(
        private.iter().any(|p| p == "work/"),
        "the private workspace `work/` is a withheld root: {private:?}"
    );
    let has_manifest = |d: &str| root.join(d).join("Cargo.toml").is_file();
    let crate_of = |rel: &str| crate_dir(rel, &has_manifest);
    let exempt: Vec<&str> = OWN_OUTPUT.iter().map(|(f, _)| *f).collect();
    let mut bad = Vec::new();
    for rel in checkout() {
        if !(rel.ends_with(".rs")
            || rel.ends_with(".py")
            || rel.ends_with(".sh")
            || rel.ends_with(".bash"))
        {
            continue;
        }
        if exempt.contains(&rel.as_str()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        for (n, what) in reads(&rel, &text, &private, &crate_of) {
            let line = text.lines().nth(n - 1).unwrap_or_default();
            if GUARDED
                .iter()
                .any(|(f, read, _)| *f == rel && line.contains(read))
            {
                continue;
            }
            bad.push(format!("{rel}:{n}: {what}"));
        }
    }
    assert!(
        bad.is_empty(),
        "a product source reads the private workspace, so a public clone fails where this checkout passes. \
         Read what the product holds, or guard the read and list it in GUARDED:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn every_guarded_read_is_still_guarded() {
    let root = repo_root();
    let private = private_roots();
    let has_manifest = |d: &str| root.join(d).join("Cargo.toml").is_file();
    let crate_of = |rel: &str| crate_dir(rel, &has_manifest);
    let mut bad = Vec::new();
    for (file, read, guard) in GUARDED {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        let found = reads(file, &text, &private, &crate_of);
        if !found
            .iter()
            .any(|(n, _)| text.lines().nth(n - 1).is_some_and(|l| l.contains(read)))
        {
            bad.push(format!(
                "{file}: the guarded read `{read}` is gone — delete its entry"
            ));
        }
        if !text.contains(guard) {
            bad.push(format!("{file}: `{read}` is no longer behind `{guard}`"));
        }
    }
    for (file, _) in OWN_OUTPUT {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
        if reads(file, &text, &private, &crate_of).is_empty() {
            bad.push(format!(
                "{file} no longer names the workspace — delete its OWN_OUTPUT entry"
            ));
        }
    }
    assert!(bad.is_empty(), "\n  {}", bad.join("\n  "));
}

#[test]
fn the_detector_finds_every_form_of_read() {
    let private = vec!["work/".to_string()];
    let crate_of = |_: &str| "crates/logicaffeine_tests".to_string();
    let hits = |rel: &str, text: &str| reads(rel, text, &private, &crate_of).len();
    let t = "crates/logicaffeine_tests/tests/x.rs";
    assert_eq!(
        hits(
            t,
            "const M: &str = include_str!(\"../../../work/corpus/manifest.json\");\n"
        ),
        1
    );
    assert_eq!(
        hits(
            t,
            "const M: &[u8] = include_bytes!(\"../../../work/a.bin\");\n"
        ),
        1
    );
    assert_eq!(
        hits(t, "#[path = \"../../../work/locks/tests/x.rs\"]\nmod x;\n"),
        1
    );
    assert_eq!(
        hits(
            t,
            "include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../../work/a.json\"))\n"
        ),
        1
    );
    assert_eq!(
        hits(t, "let d = root().join(\"work/corpus/verdicts\");\n"),
        1
    );
    assert_eq!(hits(t, "let d = root().join(\"work\");\n"), 1);
    assert_eq!(hits(t, "let p = Path::new(\"../work/x\");\n"), 1);
    assert_eq!(
        hits(t, "let r = read(&format!(\"work/corpus/{doc}\"));\n"),
        1
    );
    assert_eq!(
        hits("scripts/x.sh", "OUT=\"$REPO_ROOT/work/corpus/verdicts\"\n"),
        1
    );
    assert_eq!(
        hits(
            "scripts/x.py",
            "d = os.path.join(root, \"work\", \"corpus\", \"verdicts\")\n"
        ),
        1
    );
    assert_eq!(
        hits("scripts/x.py", "p = ROOT / \"work/corpus/manifest.json\"\n"),
        1
    );
    assert_eq!(
        hits("scripts/x.py", "p = root/\"work/corpus\"/case[\"path\"]\n"),
        1
    );
    assert_eq!(
        hits(
            t,
            "const M: &str = include_str!(\"fixtures/lean_conformance/manifest.json\");\n"
        ),
        0
    );
    assert_eq!(hits(t, "// see `work/corpus/verdicts/README.md`\n"), 0);
    assert_eq!(hits(t, "let w = r[\"work\"][f].as_u64();\n"), 0);
    assert_eq!(
        hits(t, "let text = \"See `work/plan.md`, the work/ folder\";\n"),
        0
    );
    assert_eq!(hits(t, ".filter(|p| !p.starts_with(\"work/\"))\n"), 0);
    assert_eq!(
        hits(
            "scripts/x.py",
            "out[\"work\"] = dict(out[\"work\"] or {})\n"
        ),
        0
    );
    assert_eq!(hits("scripts/x.sh", "# writes under work/wikis\n"), 0);
    assert_eq!(
        hits(
            "scripts/x.sh",
            "outdir=\"${2:-${repo_root}/work/wikis/traces/${stem}}\"\n"
        ),
        1
    );
    assert_eq!(
        hits("scripts/x.sh", "cd \"$(dirname \"$0\")/../work/x\"\n"),
        1
    );
    assert_eq!(
        hits("scripts/x.sh", "article=\"work/wikis/${slug}.txt\"\n"),
        1
    );
    assert_eq!(hits("scripts/x.sh", "cd /work/spatial\n"), 0);
    assert_eq!(
        hits(
            "scripts/x.sh",
            "docker run -v $ROOT:/work -v /tmp:/out img klayout -r /work/$deck\n"
        ),
        0
    );
    assert_eq!(
        hits("scripts/x.sh", "bash run.sh --spatial-home=/work/\n"),
        0
    );
    assert_eq!(
        hits(
            "scripts/x.sh",
            "echo \"  Updated work/research/nano (Cargo.toml)\"\n"
        ),
        0
    );
    assert_eq!(
        hits(
            "scripts/x.sh",
            "cp \"$ROOT/framework/a\" \"$ROOT/workspace/b\"\n"
        ),
        0
    );
    assert_eq!(
        hits("scripts/x.py", "if p.get(\"work\") != row.get(\"work\"):\n"),
        0
    );
    assert_eq!(
        hits("scripts/x.py", "d = os.path.join(\"work\", \"lean\")\n"),
        1
    );
    assert_eq!(
        hits(
            "crates/logicaffeine_tests/tests/y.rs",
            "const M: &str = include_str!(\"../work.json\");\n"
        ),
        0
    );
}

/// `(file, the private path it may name, why)`: a product script is public, and a path below the workspace in it —
/// in code, a comment or a message — tells every reader what the workspace holds. Each name a script keeps is here.
const NAMED: &[(&str, &str, &str)] = &[
    (
        "scripts/bump-version.sh",
        "work/release-guards/",
        "the release machinery's root: the post-bump hook moves the workspace's own version stamps",
    ),
    (
        "scripts/dev/next-wiki.sh",
        "work/wikis/",
        "its own output (OWN_OUTPUT)",
    ),
    (
        "scripts/dev/trace-wiki.sh",
        "work/wikis/",
        "its own output (OWN_OUTPUT)",
    ),
    (
        "scripts/dev/triage-wiki.sh",
        "work/wikis/",
        "its own output (OWN_OUTPUT)",
    ),
];

/// Whether `path` is the directory `dir` (declared with its trailing `/`) or lies below it: `work/wikis` and
/// `work/wikis/a.txt` are under `work/wikis/`, `work/wikisx` is not.
fn under(path: &str, dir: &str) -> bool {
    let dir = dir.trim_end_matches('/');
    path == dir
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// A product script: shell, Python, JavaScript or PowerShell, a CI workflow, or a git hook.
fn is_script(rel: &str) -> bool {
    !rel.contains("/node_modules/")
        && ([".sh", ".bash", ".py", ".js", ".mjs", ".ps1"]
            .iter()
            .any(|s| rel.ends_with(s))
            || rel.starts_with(".github/workflows/")
            || rel.starts_with(".githooks/"))
}

/// Every private path `line` names: `<root><name>` for a private root, repo-relative (bare, `./` or `../`, or behind
/// a variable, `}` or `)`), never part of a longer word (`network/`), never a directory of the same name below
/// another (`benchmarks/work/`), never an absolute `/work/…` (another machine's mount).
fn names_in_line(line: &str, root: &str) -> Vec<String> {
    let b = line.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = line[from..].find(root) {
        let i = from + at;
        let j = i + root.len();
        from = j;
        if !b.get(j).is_some_and(|&c| word(c) || c == b'.' || c == b'-') {
            continue;
        }
        let relative = match i.checked_sub(1).map(|k| b[k]) {
            None => true,
            Some(c) if word(c) || c == b'-' || c == b'.' => false,
            Some(b'/') => match i.checked_sub(2).map(|k| b[k]) {
                Some(b'}') | Some(b')') => true,
                Some(b'.') => true,
                Some(c) if word(c) => {
                    let ident = line[..i - 1].bytes().rev().take_while(|&c| word(c)).count();
                    line[..i - 1 - ident].ends_with('$')
                }
                _ => false,
            },
            Some(_) => true,
        };
        if relative {
            let len = line[i..]
                .bytes()
                .take_while(|&c| word(c) || c == b'.' || c == b'-' || c == b'/')
                .count();
            out.push(line[i..i + len].to_string());
        }
    }
    out
}

/// Every private path a script names, with its line.
fn names(text: &str, private: &[String]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        for root in private {
            out.extend(names_in_line(line, root).into_iter().map(|p| (n + 1, p)));
        }
    }
    out
}

#[test]
fn no_product_script_names_a_private_path() {
    let root = repo_root();
    let private = private_roots();
    let mut bad = Vec::new();
    let mut scanned = 0;
    for rel in checkout().into_iter().filter(|r| is_script(r)) {
        let Ok(text) = std::fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        scanned += 1;
        for (n, path) in names(&text, &private) {
            if !NAMED
                .iter()
                .any(|(f, dir, _)| *f == rel && under(&path, dir))
            {
                bad.push(format!("{rel}:{n}: `{path}`"));
            }
        }
    }
    assert!(
        scanned > 500,
        "only {scanned} scripts scanned — the scan is broken"
    );
    assert!(
        bad.is_empty(),
        "a product script names the private workspace's contents. Move the private work behind the hook the \
         workspace provides, or name the path in NAMED with the reason it is public:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn every_named_path_is_still_named() {
    let root = repo_root();
    let private = private_roots();
    let stale: Vec<String> = NAMED
        .iter()
        .filter(|(file, prefix, _)| {
            let text = std::fs::read_to_string(root.join(file)).unwrap_or_default();
            !names(&text, &private).iter().any(|(_, p)| under(p, prefix))
        })
        .map(|(file, prefix, _)| {
            format!("{file} no longer names {prefix} — delete its NAMED entry")
        })
        .collect();
    assert!(stale.is_empty(), "\n  {}", stale.join("\n  "));
}

#[test]
fn the_name_detector_sees_a_repo_relative_private_path_and_nothing_else() {
    let w = |line: &str| names_in_line(line, "work/");
    assert_eq!(
        w("NANO_DIR=\"$REPO_ROOT/work/research/nano\""),
        ["work/research/nano"]
    );
    assert_eq!(
        w("  echo \"  Updated work/research/nano (Cargo.toml)\""),
        ["work/research/nano"]
    );
    assert_eq!(
        w("# the marketing tree lives in the private workspace (work/marketing)"),
        ["work/marketing"]
    );
    assert_eq!(w("cd \"$(dirname \"$0\")/../work/x\""), ["work/x"]);
    assert_eq!(
        w("outdir=\"${2:-${repo_root}/work/wikis/traces/${stem}}\""),
        ["work/wikis/traces/"]
    );
    assert_eq!(
        w("p = ROOT / \"work/corpus/manifest.json\""),
        ["work/corpus/manifest.json"]
    );
    assert_eq!(
        w("HOOK=\"$REPO_ROOT/work/release-guards/bump/post-bump.sh\""),
        ["work/release-guards/bump/post-bump.sh"]
    );
    assert!(w("docker run -v $ROOT:/work -v /tmp:/out img klayout -r /work/$deck").is_empty());
    assert!(w("cd /work/spatial").is_empty());
    assert!(w("cp \"$ROOT/framework/a\" \"$ROOT/workspace/b\"").is_empty());
    assert!(w("url=\"https://example.com/network/x\"").is_empty());
    assert!(w("ls benchmarks/work/x").is_empty());
    assert!(w("the work/ folder").is_empty());
    assert!(under("work/wikis", "work/wikis/") && under("work/wikis/a/b.txt", "work/wikis/"));
    assert!(!under("work/wikisx", "work/wikis/") && !under("work", "work/wikis/"));
}
