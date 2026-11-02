//! Benchmark fairness ratchet — every implementation in `benchmarks/programs/` runs the algorithm `main.c` runs, and
//! every script builds and times it the way the publishable run does, so no comparison is quietly tilted.
//!
//! - Zig uses truncated `@rem`, never floored `@mod`. An expert reviewer (Ray Myers, "Fibs and Fibonacci") found the
//!   Zig ports using `@mod` where every other language uses truncated `%`: every operand here is non-negative, so the
//!   two agree, but `@mod` pays for a sign-correction branch `@rem` does not — a Zig-only handicap. A benchmark that
//!   genuinely needs floored modulo on signed data goes in `MOD_ALLOW` with a justification; today that set is empty.
//! - collect and two_sum hand-roll their table in every language, as `main.c` does: no standard-library map or set,
//!   which would compare one language's library against another's hand-written table (seven languages once did).
//! - Every benchmark script builds an implementation through `suite_build`, with the command `lib/languages.tsv`
//!   publishes: no script carries a compiler command line of its own. The development scripts once timed `gcc -O2` and
//!   `rustc -O` binaries against a full-release LOGOS, and reused a `-O2` C binary as the `-O3` baseline.
//! - `suite_build` reuses an artifact only when the same command and toolchain built it and it is newer than its
//!   source (a LOGOS artifact also newer than largo); a failed build leaves neither artifact nor stamp behind.
//! - The README's build table and the compile-time labels state exactly the flags the build passes.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test benchmark_fairness_lock`

#![cfg(all(unix, not(target_arch = "wasm32")))]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// The rows of a `benchmarks/lib/` registry file, by column name.
fn registry(name: &str) -> Vec<BTreeMap<String, String>> {
    let text = read(&repo_root().join("benchmarks/lib").join(name));
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().expect("a header row").split('\t').collect();
    lines
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            header
                .iter()
                .map(|h| h.to_string())
                .zip(l.split('\t').map(str::to_string))
                .collect()
        })
        .collect()
}

/// Every benchmark directory under `benchmarks/programs/`.
fn program_dirs() -> Vec<(String, PathBuf)> {
    let programs = repo_root().join("benchmarks/programs");
    let mut out: Vec<(String, PathBuf)> = fs::read_dir(&programs)
        .unwrap_or_else(|e| panic!("read {programs:?}: {e}"))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| (p.file_name().unwrap().to_string_lossy().into_owned(), p))
        .collect();
    out.sort();
    out
}

/// (benchmark dir, reason) — Zig sources permitted to use floored `@mod` because they operate on genuinely-signed
/// data. Goal: empty. Every current site is on non-negative operands and must use `@rem`.
const MOD_ALLOW: &[(&str, &str)] = &[];

#[test]
fn zig_benchmarks_use_truncated_rem_not_floored_mod() {
    let programs = program_dirs();
    let missing: Vec<&String> = programs
        .iter()
        .filter(|(_, dir)| !dir.join("main.zig").is_file())
        .map(|(name, _)| name)
        .collect();
    assert!(
        missing.is_empty(),
        "every benchmark has a Zig implementation: {missing:?}"
    );

    let mut offenders = Vec::new();
    for (name, dir) in &programs {
        if MOD_ALLOW.iter().any(|(b, _)| b == name) {
            continue;
        }
        let src = read(&dir.join("main.zig"));
        for (i, line) in src.lines().enumerate() {
            if line.contains("@mod") {
                offenders.push(format!("  {name}/main.zig:{}: {}", i + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "Zig benchmark(s) use floored `@mod` where truncated `@rem` is correct and \
         faster (every operand here is non-negative, and `@rem` matches the `%` every \
         other language uses). Replace `@mod` with `@rem`:\n{}",
        offenders.join("\n")
    );
}

/// The benchmarks whose subject is a hash table `main.c` hand-rolls.
const HASH_TABLE_BENCHMARKS: &[&str] = &["collect", "two_sum"];

/// Per implementation file, what reaching for the language's own map or set looks like.
const STDLIB_TABLES: &[(&str, &[&str])] = &[
    (
        "main.cpp",
        &[
            "unordered_map",
            "unordered_set",
            "std::map",
            "std::set",
            "<map>",
            "<set>",
        ],
    ),
    (
        "main.rs",
        &[
            "HashMap",
            "HashSet",
            "BTreeMap",
            "BTreeSet",
            "std::collections",
        ],
    ),
    (
        "main.zig",
        &["HashMap", "AutoHashMap", "ArrayHashMap", "BufSet"],
    ),
    ("main.go", &["map[", "make(map"]),
    (
        "Main.java",
        &[
            "HashMap",
            "HashSet",
            "TreeMap",
            "TreeSet",
            "Hashtable",
            "java.util.Map",
            "java.util.Set",
        ],
    ),
    ("main.js", &["new Map", "new Set", "Object.create(null)"]),
    (
        "main.nim",
        &[
            "import tables",
            "import sets",
            "std/tables",
            "std/sets",
            "Table[",
            "HashSet[",
            "initTable",
            "toHashSet",
        ],
    ),
    (
        "main.py",
        &["dict(", "set(", "= {}", "defaultdict", "Counter("],
    ),
    (
        "main.rb",
        &["Hash.new", "= {}", "Set.new", "require 'set'", "to_set"],
    ),
];

/// Whether `needle` occurs in `src` where it is not the tail of a longer identifier (`KeySet.new` is no `Set.new`).
fn contains_word(src: &str, needle: &str) -> bool {
    src.match_indices(needle).any(|(i, _)| {
        !src[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// collect and two_sum measure the table `main.c` hand-rolls: flat slots, Fibonacci hashing (the golden-ratio
/// multiplier, 2^64/phi — JavaScript, without a 64-bit multiply, takes 2^32/phi), linear probing. Every other
/// implementation hand-rolls the same table. LOGOS's `Map of Int to Int` is that table in its runtime, which is why
/// `main.lg` keeps its natural map.
#[test]
fn hash_table_benchmarks_hand_roll_their_table_in_every_language() {
    let mut offenders = Vec::new();
    for bench in HASH_TABLE_BENCHMARKS {
        let dir = repo_root().join("benchmarks/programs").join(bench);
        let c = read(&dir.join("main.c"));
        assert!(
            c.contains("0x9E3779B97F4A7C15") && c.contains("& mask"),
            "{bench}/main.c is the reference table: Fibonacci hashing and linear probing"
        );
        for (file, library) in STDLIB_TABLES {
            let src = read(&dir.join(file));
            for needle in *library {
                if contains_word(&src, needle) {
                    offenders.push(format!("{bench}/{file}: `{needle}`"));
                }
            }
            let multiplier = if *file == "main.js" {
                "0x9E3779B9"
            } else {
                "0x9E3779B97F4A7C15"
            };
            let hashes = src.match_indices(multiplier).any(|(at, _)| {
                src[at + multiplier.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .all(|c| "uUlL".contains(c))
            });
            if !hashes {
                offenders.push(format!(
                    "{bench}/{file}: does not hash by the golden-ratio multiplier {multiplier} as main.c does"
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a hash-table benchmark departs from the table main.c hand-rolls:\n  {}",
        offenders.join("\n  ")
    );
    assert!(contains_word("let s = Set.new", "Set.new") && contains_word("Set.new", "Set.new"));
    assert!(
        !contains_word("seen = KeySet.new(n)", "Set.new") && !contains_word("bitmap[i]", "map[")
    );
}

/// The tools that compile a benchmark implementation into something a script then runs.
const COMPILERS: &[&str] = &[
    "gcc", "g++", "cc", "c++", "clang", "clang++", "rustc", "zig", "go", "javac", "nim",
];

/// Commands that run the command after them (with their own options first), and the shell words a command follows.
const WRAPPERS: &[&str] = &[
    "timeout",
    "gtimeout",
    "run_timeout",
    "taskset",
    "nice",
    "env",
    "exec",
    "time",
    "nohup",
    "sudo",
    "then",
    "do",
    "if",
    "elif",
    "else",
    "!",
];

/// A line of shell with its comment dropped and every quoted string blanked: a word inside quotes (a message, an awk
/// program, a version query's `$(…)`) is no command.
fn unquoted_code(line: &str) -> String {
    if line.trim_start().starts_with('#') {
        return String::new();
    }
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    for c in line.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                out.push(' ');
            }
            Some(_) => out.push(' '),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                out.push(' ');
            }
            None if c == '#' && prev.is_whitespace() => break,
            None => out.push(c),
        }
        prev = c;
    }
    out
}

/// The compiler a line of shell runs as a command, when it compiles: not asked for its version, and not emitting
/// assembly or IR for reading (`-S`). A command starts a line or follows `&&`, `||`, `;`, `|`, a subshell's `(`, `{` or
/// a backtick, after any environment assignments and wrappers (`timeout 60`, `taskset -c 0-15`, `then`, …); a `case`
/// label's alternatives (`cargo|rustc)`) and an array literal's items (`+=(gcc)`) are not commands.
fn compiler_invocation(line: &str) -> Option<String> {
    let mut code = unquoted_code(line).replace("=(", "=[");
    let label_end = code.trim_start().find(')').filter(|&end| {
        let label = &code.trim_start()[..end];
        !label.is_empty()
            && label
                .chars()
                .all(|c| c.is_alphanumeric() || "_|*.+-".contains(c))
    });
    if let Some(end) = label_end {
        code = code.trim_start()[end + 1..].to_string();
    }
    let mut segments = vec![code];
    for sep in ["&&", "||", ";", "|", "(", ")", "`", "{", "}"] {
        segments = segments
            .iter()
            .flat_map(|s| s.split(sep).map(str::to_string).collect::<Vec<_>>())
            .collect();
    }
    for segment in segments {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        let mut i = 0;
        while i < tokens.len() {
            let t = tokens[i];
            let assignment = t.split_once('=').is_some_and(|(name, _)| {
                !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            });
            let after_wrapper = i > 0
                && WRAPPERS.contains(&tokens[i - 1])
                && (t.starts_with('-')
                    || t.starts_with('"')
                    || t.chars().all(|c| c.is_ascii_digit() || c == '.'));
            let cpu_list = i > 1 && tokens[i - 1] == "-c" && tokens[i - 2] == "taskset";
            if assignment || WRAPPERS.contains(&t) || after_wrapper || cpu_list {
                i += 1;
                continue;
            }
            break;
        }
        let Some(first) = tokens.get(i) else {
            continue;
        };
        if !COMPILERS.contains(first) {
            continue;
        }
        let rest = &tokens[i + 1..];
        let compiles = match *first {
            "zig" => rest
                .first()
                .is_some_and(|s| s.starts_with("build") || *s == "cc" || *s == "c++"),
            "go" => rest.first().is_some_and(|s| *s == "build" || *s == "run"),
            "nim" => rest
                .first()
                .is_some_and(|s| ["c", "cpp", "compile", "js"].contains(s)),
            _ => true,
        };
        let query = rest.iter().any(|s| {
            [
                "--version",
                "-version",
                "version",
                "-v",
                "-S",
                "-E",
                "-dumpversion",
            ]
            .contains(s)
        });
        if compiles && !query {
            return Some(first.to_string());
        }
    }
    None
}

/// The scripts that build or time benchmark implementations.
fn benchmark_scripts() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in ["benchmarks", "benchmarks/lib", "scripts/bench"] {
        for e in fs::read_dir(repo_root().join(dir)).expect(dir).flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "sh") {
                files.push(p);
            }
        }
    }
    files.sort();
    files
}

/// Every benchmark script builds an implementation through `suite_build`, so every number any script reports is of the
/// binary the command `lib/languages.tsv` publishes produces. A compiler command line in a script is a second build
/// command: the drift that timed `gcc -O2`, `rustc -O` and an untuned Zig, Go and Nim against a full-release LOGOS.
#[test]
fn every_benchmark_script_builds_through_suite_build() {
    let mut offenders = Vec::new();
    for path in benchmark_scripts() {
        let rel = path
            .strip_prefix(repo_root())
            .unwrap()
            .display()
            .to_string();
        for (i, line) in read(&path).lines().enumerate() {
            if let Some(tool) = compiler_invocation(line) {
                offenders.push(format!(
                    "{rel}:{}: runs {tool} itself: {}",
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "build an implementation with `suite_build <bench> <id>` (benchmarks/lib/suite.sh), never a compiler line of \
         the script's own:\n  {}",
        offenders.join("\n  ")
    );

    for shape in [
        "            gcc -O2 -o \"$BIN_DIR/${bench}_c\" \"$PROGRAMS_DIR/$bench/main.c\" -lm 2>/dev/null ;;",
        "    if gcc -O2 -o \"$BIN_DIR/${bench}_c\" \"$PROGRAMS_DIR/$bench/main.c\" -lm 2>/dev/null; then",
        "            rustc --edition 2021 -O -o \"$BIN_DIR/${bench}_rs\" \"$PROGRAMS_DIR/$bench/main.rs\" 2>/dev/null ;;",
        "            run_timeout \"$BUILD_TIMEOUT\" zig build-exe -O ReleaseFast --name \"${bench}_zig\" main.zig && \\",
        "    ( cd \"$BENCH\" && go build -o \"$BIN/go\" main.go 2>/dev/null ) && CMD[go]=\"$BIN/go $N\"",
        "GOAMD64=v3 go build -o bench main.go",
        "            nim c -d:release --hints:off -o:\"$BIN_DIR/${bench}_nim\" main.nim 2>/dev/null ;;",
        "            javac -d \"$BIN_DIR/java/$bench\" \"$PROGRAMS_DIR/$bench/Main.java\" 2>/dev/null ;;",
        "    [ -f main.c ] && clang++ -O3 -o x main.cpp",
        "timeout 60 taskset -c 0-15 gcc -O3 -o x main.c",
    ] {
        assert!(
            compiler_invocation(shape).is_some(),
            "the rule must see a build in: {shape}"
        );
    }
    for fine in [
        "gcc --version",
        "    --arg gcc \"$(gcc --version 2>/dev/null | head -1)\" \\",
        "    clang -S $flags -fno-asynchronous-unwind-tables \\",
        "    command -v clang &>/dev/null && dump_c \"$bench\"",
        "have gcc     || APT_PACKAGES+=(gcc)",
        "if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; then",
        "        c) echo gcc ;; cpp) echo g++ ;; rust) echo rustc ;; zig) echo zig ;;",
        "zig version",
        "go version",
        "# gcc -O2 builds this",
        "report \"C\"          gcc      \"gcc --version\"",
        "            cargo|rustc|cargo-nextest|cc1|cc1plus|ld|ld.lld|dx|zig|javac|go|nim)",
        "awk '{s+=$1;n++; if(exp($1)<1)c++} END{printf \"%.3fx\\n\", exp(s/n)}' \"$LOG\"",
        "    info \"Installing gcc for the C rows...\"",
    ] {
        assert!(
            compiler_invocation(fine).is_none(),
            "the rule must accept: {fine}"
        );
    }
}

/// A fixture tree holding the real `benchmarks/lib/`, one benchmark (`fib`: C, Java and LOGOS sources) and stub tools
/// that log every invocation: `gcc` and `javac` build a program unless told to fail or to write nothing, and answer
/// `--version` with what the `version` file holds; `largo` builds a LOGOS project the way `logos_build` reads it.
struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let bench = root.join("benchmarks");
        fs::create_dir_all(bench.join("lib")).unwrap();
        for e in fs::read_dir(repo_root().join("benchmarks/lib"))
            .unwrap()
            .flatten()
        {
            if e.path().is_file() {
                fs::copy(e.path(), bench.join("lib").join(e.file_name())).unwrap();
            }
        }
        let fib = bench.join("programs/fib");
        fs::create_dir_all(&fib).unwrap();
        fs::write(fib.join("main.c"), "int main(void) { return 0; }\n").unwrap();
        fs::write(fib.join("Main.java"), "class Main {}\n").unwrap();
        fs::write(fib.join("main.lg"), "## Main\nShow 1.\n").unwrap();
        let stubs = root.join("stubs");
        fs::create_dir_all(&stubs).unwrap();
        let stub = |name: &str, body: &str| {
            let path = stubs.join(name);
            fs::write(
                &path,
                format!("#!/bin/sh\necho \"{name} $*\" >> \"$STUB_LOG\"\n{body}\n"),
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        };
        let versioned = "case \"$*\" in *--version*) cat \"$STUB_VERSION\"; exit 0 ;; esac\n\
                         [ -f \"$STUB_FAIL\" ] && { echo 'main: error: the fixture fails this build' >&2; exit 1; }\n\
                         [ -f \"$STUB_SILENT\" ] && exit 0";
        stub(
            "gcc",
            &format!(
                "{versioned}\nout=\"\"; while [ $# -gt 0 ]; do [ \"$1\" = -o ] && out=\"$2\"; shift; done\n\
                 printf '#!/bin/sh\\necho 1\\n' > \"$out\" && chmod +x \"$out\""
            ),
        );
        stub(
            "javac",
            &format!(
                "{versioned}\nout=\"\"; while [ $# -gt 0 ]; do [ \"$1\" = -d ] && out=\"$2\"; shift; done\n\
                 : > \"$out/Main.class\""
            ),
        );
        stub(
            "largo",
            "case \"$*\" in *--version*) cat \"$STUB_VERSION\"; exit 0 ;; esac\n\
             [ -f \"$STUB_FAIL\" ] && exit 1\n\
             mkdir -p target/release/build/src && printf '#!/bin/sh\\necho 1\\n' > target/release/bench && \
             chmod +x target/release/bench && echo 'fn main() {}' > target/release/build/src/main.rs",
        );
        fs::write(root.join("version"), "toolchain 1.0\n").unwrap();
        fs::write(root.join("log"), "").unwrap();
        Fixture { _dir: dir, root }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Run `suite_build fib <id>` in a fresh shell: whether it succeeded, whether it reused the artifact, how many
    /// builds (stub invocations other than a version query) it ran, and its output.
    fn build(&self, id: &str, env: &[(&str, &str)]) -> (bool, bool, usize, String) {
        let builds = |log: &str| log.lines().filter(|l| !l.contains("--version")).count();
        let before = builds(&read(&self.path("log")));
        let mut cmd = Command::new("bash");
        cmd.arg("-c")
            .arg(format!(
                ". '{root}/benchmarks/lib/suite.sh' && LARGO='{root}/stubs/largo' && suite_build fib {id}; rc=$?; \
                 echo \"reused=$SUITE_BUILD_REUSED\"; exit $rc",
                root = self.root.display()
            ))
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.path("stubs").display(),
                    std::env::var("PATH").unwrap_or_default()
                ),
            )
            .env("STUB_LOG", self.path("log"))
            .env("STUB_VERSION", self.path("version"))
            .env("STUB_FAIL", self.path("fail"))
            .env("STUB_SILENT", self.path("silent"))
            .env_remove("BENCH_WORK_DIR")
            .env_remove("FORCE_BUILD")
            .env_remove("BENCH_ONLY")
            .env_remove("JIT_IMAGE_BUILD")
            .env_remove("JIT_IMAGE_RUN");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let after = builds(&read(&self.path("log")));
        (
            out.status.success(),
            text.contains("reused=1"),
            after - before,
            text,
        )
    }

    /// Set a file's modification time.
    fn touch(&self, rel: &str, when: &str) {
        let status = Command::new("touch")
            .args(["-d", when])
            .arg(self.path(rel))
            .status()
            .unwrap();
        assert!(status.success(), "touch {rel}");
    }
}

/// `suite_build` builds with the registry's command and stamps the artifact with it; reuses the artifact only while the
/// stamp still names this command and toolchain and the artifact is newer than its source; rebuilds an artifact no
/// stamp vouches for (another script's build, under another command line) and every artifact under FORCE_BUILD=1; and
/// leaves nothing behind when a build fails or produces nothing.
#[test]
fn suite_build_reuses_only_what_the_registry_command_built() {
    let f = Fixture::new();
    let bin = "benchmarks/bin/fib_c";
    let stamp = format!("{bin}.build");

    let (ok, reused, builds, log) = f.build("c", &[]);
    assert!(
        ok && !reused && builds == 1,
        "the first call builds once:\n{log}"
    );
    assert!(
        f.path(bin).is_file(),
        "the artifact is the registry's {{bin}}:\n{log}"
    );
    let registry_build = registry("languages.tsv")
        .into_iter()
        .find(|r| r["id"] == "c")
        .unwrap()["build"]
        .clone();
    let want = registry_build
        .replace("{bin}", &f.path(bin).display().to_string())
        .replace(
            "{src}",
            &f.path("benchmarks/programs/fib/main.c")
                .display()
                .to_string(),
        );
    assert_eq!(
        read(&f.path(&stamp)),
        format!("{want}\ntoolchain 1.0"),
        "the stamp records the filled build command and the toolchain"
    );
    let invoked = read(&f.path("log"));
    assert!(
        invoked.lines().any(|l| l == want),
        "the build ran the registry's command line exactly ({want}):\n{invoked}"
    );

    let (ok, reused, builds, log) = f.build("c", &[]);
    assert!(
        ok && reused && builds == 0,
        "a stamped, fresh artifact is reused:\n{log}"
    );

    f.touch("benchmarks/programs/fib/main.c", "2099-01-01");
    let (ok, reused, builds, log) = f.build("c", &[]);
    assert!(
        ok && !reused && builds == 1,
        "a source newer than the artifact rebuilds it:\n{log}"
    );
    f.touch("benchmarks/programs/fib/main.c", "2001-01-01");

    fs::write(f.path(&stamp), "gcc -O2 -o bench main.c -lm\ntoolchain 1.0").unwrap();
    let (ok, reused, builds, log) = f.build("c", &[]);
    assert!(
        ok && !reused && builds == 1,
        "an artifact another command built is rebuilt:\n{log}"
    );

    fs::remove_file(f.path(&stamp)).unwrap();
    let (ok, reused, builds, log) = f.build("c", &[]);
    assert!(
        ok && !reused && builds == 1,
        "an artifact no stamp vouches for is rebuilt:\n{log}"
    );

    fs::write(f.path("version"), "toolchain 2.0\n").unwrap();
    let (ok, reused, builds, log) = f.build("c", &[]);
    assert!(
        ok && !reused && builds == 1,
        "a new toolchain rebuilds:\n{log}"
    );

    let (ok, reused, builds, log) = f.build("c", &[("FORCE_BUILD", "1")]);
    assert!(
        ok && !reused && builds == 1,
        "FORCE_BUILD=1 rebuilds a reusable artifact:\n{log}"
    );

    fs::write(f.path("fail"), "").unwrap();
    let (ok, _, _, log) = f.build("c", &[("FORCE_BUILD", "1")]);
    assert!(
        !ok && log.contains("the fixture fails this build"),
        "a failed build fails, with the compiler's error:\n{log}"
    );
    assert!(
        !f.path(bin).exists() && !f.path(&stamp).exists(),
        "a failed build leaves neither artifact nor stamp, so nothing stale is timed"
    );
    fs::remove_file(f.path("fail")).unwrap();

    fs::write(f.path("silent"), "").unwrap();
    let (ok, _, _, log) = f.build("c", &[]);
    assert!(
        !ok && log.contains("produced no"),
        "a build that succeeds without producing the artifact fails by name:\n{log}"
    );
    assert!(!f.path(&stamp).exists(), "and stamps nothing");
}

/// A directory artifact (Java's classes) is stamped beside its directory and reused like a file.
#[test]
fn suite_build_stamps_a_directory_artifact_beside_it() {
    let f = Fixture::new();
    let (ok, reused, builds, log) = f.build("java", &[]);
    assert!(
        ok && !reused && builds == 1,
        "javac builds the classes:\n{log}"
    );
    assert!(
        f.path("benchmarks/bin/java/fib/Main.class").is_file(),
        "{log}"
    );
    assert!(f.path("benchmarks/bin/java/fib.build").is_file(), "{log}");
    let (ok, reused, builds, log) = f.build("java", &[]);
    assert!(
        ok && reused && builds == 0,
        "the stamped classes are reused:\n{log}"
    );
}

/// A LOGOS artifact is rebuilt when largo is newer than it: iterating on the compiler rebuilds exactly the LOGOS
/// binaries.
#[test]
fn suite_build_rebuilds_a_logos_artifact_older_than_largo() {
    let f = Fixture::new();
    f.touch("benchmarks/programs/fib/main.lg", "2001-01-01");
    f.touch("stubs/largo", "2001-01-01");
    let (ok, reused, builds, log) = f.build("logos_release", &[]);
    assert!(
        ok && !reused && builds == 1,
        "largo builds the program:\n{log}"
    );
    assert!(
        f.path("benchmarks/bin/fib_logos_release").is_file(),
        "{log}"
    );
    let (ok, reused, builds, log) = f.build("logos_release", &[]);
    assert!(
        ok && reused && builds == 0,
        "a LOGOS artifact newer than largo is reused:\n{log}"
    );
    f.touch("benchmarks/bin/fib_logos_release", "2002-01-01");
    f.touch("stubs/largo", "2003-01-01");
    let (ok, reused, builds, log) = f.build("logos_release", &[]);
    assert!(
        ok && !reused && builds == 1,
        "a largo newer than the artifact rebuilds it:\n{log}"
    );
}

/// The README's build table states, row by row, flags the registry's commands pass: each backticked fragment of a row
/// appears in the command that builds a language that row names (its run command when it has no build; for a LOGOS
/// tier, whose build is a suite function, the `largo` command it runs), and every non-LOGOS row of `languages.tsv` is
/// named by some row.
#[test]
fn the_readme_build_table_states_the_registry_flags() {
    const ROWS: &[(&str, &[&str])] = &[
        ("C, C++ (gcc, clang)", &["c", "c_clang", "cpp", "cpp_clang"]),
        ("Rust", &["rust"]),
        ("Zig", &["zig"]),
        ("Go", &["go"]),
        ("Nim", &["nim"]),
        ("Java, JavaScript", &["java", "js"]),
        ("LOGOS", &["logos_release", "logos_c"]),
    ];
    let langs = registry("languages.tsv");
    let commands = |id: &str| -> String {
        let row = langs
            .iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("a {id} row"));
        let command = if row["tier"] == "logos" {
            &row["shown"]
        } else if row["build"] == "-" {
            &row["run"]
        } else {
            &row["build"]
        };
        command.replace('"', "")
    };
    let readme = read(&repo_root().join("benchmarks/README.md"));
    let mut named = Vec::new();
    for line in readme.lines().filter(|l| l.starts_with("| ")) {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        let Some((label, ids)) = ROWS.iter().find(|(label, _)| cells.get(1) == Some(label)) else {
            continue;
        };
        named.extend(ids.iter().copied());
        let fragments: Vec<&str> = cells
            .get(2)
            .copied()
            .unwrap_or_default()
            .split('`')
            .skip(1)
            .step_by(2)
            .collect();
        assert!(
            !fragments.is_empty(),
            "the README's {label} row names its flags: {line}"
        );
        for fragment in fragments {
            assert!(
                ids.iter().any(|id| commands(id).contains(fragment)),
                "the README says the {label} build passes `{fragment}`; lib/languages.tsv says otherwise: {:?}",
                ids.iter().map(|id| commands(id)).collect::<Vec<_>>()
            );
        }
    }
    for row in langs.iter().filter(|r| r["tier"] != "logos") {
        assert!(
            named.contains(&row["id"].as_str()),
            "the README's build table names no row for {} ({})",
            row["id"],
            row["label"]
        );
    }
}

/// A compile-time row that reuses a language's build (`@build <id>`) is labelled with that build's command: its
/// compiler and every flag, less the operands (`-o {bin}`, `{src}`, `-d {dir}`), the setup (`cd {tmp} && `), the
/// libraries and the quiet switches, optionally followed by a parenthetical note. A label that drops or invents a flag
/// misstates what the compile time measured.
#[test]
fn every_compile_time_label_is_its_build_command() {
    let langs = registry("languages.tsv");
    for row in registry("compilers.tsv") {
        let Some(id) = row["command"].strip_prefix("@build ") else {
            continue;
        };
        let build = &langs
            .iter()
            .find(|r| r["id"] == id)
            .unwrap_or_else(|| panic!("compilers.tsv: @build {id} names no language"))["build"];
        let mut command = build.clone();
        for setup in ["cd {tmp} && ", "mkdir -p {dir} && "] {
            command = command.replace(setup, "");
        }
        let tokens: Vec<&str> = command.split_whitespace().collect();
        let mut kept = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            let t = tokens[i];
            if ["-o", "-d"].contains(&t) && tokens.get(i + 1).is_some_and(|n| n.contains('{')) {
                i += 2;
                continue;
            }
            if !t.contains('{') && !["-lm", "--hints:off"].contains(&t) {
                kept.push(t);
            }
            i += 1;
        }
        let label = row["label"].split(" (").next().unwrap().trim();
        assert_eq!(
            label,
            kept.join(" "),
            "compilers.tsv labels the {id} compile time {:?}; its build runs {build:?}",
            row["label"]
        );
    }
}
