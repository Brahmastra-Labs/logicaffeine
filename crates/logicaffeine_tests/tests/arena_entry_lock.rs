//! The Lean Kernel Arena entries (`arena/*.yaml`) are submittable once pinned: each one conforms
//! to the Arena's checker schema (vendored from the Arena revision it was read at) with its
//! `ref`/`rev` placeholders replaced, points at the public repository, builds the very binary its
//! run line executes, trains its profile on that run line, and stops its build at the first
//! failing step.

use logicaffeine_kernel::interface::lean::package::sha256_hex;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The lean-kernel-arena revision the vendored schema, the rehearsal and the README were read at.
const ARENA_REVISION: &str = "87512b60eb2483562967ebc93a5fb42eaa135389";
const SCHEMA_SHA256: &str = "839814a02b2c4a0f217bf52abee7b4ee4b37b0ed961eb7aa34f94291e337378b";
/// Each entry and the example binary it builds.
const ENTRIES: [(&str, &str); 2] = [
    ("logos-dual", "logos_dual"),
    ("logos-kernel", "logos_kernel"),
];
const REF_PLACEHOLDER: &str = "SUBMISSION_REF";
const REV_PLACEHOLDER: &str = "SUBMISSION_REV";
const CORPUS: &str = "crates/logicaffeine_tests/tests/fixtures/arena_corpus/";

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// The kernel crate, which carries the entries (`arena/`) and the checker binaries (`examples/`).
fn kernel_dir() -> PathBuf {
    workspace_root().join("crates/logicaffeine_kernel")
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn schema() -> serde_json::Value {
    serde_json::from_str(&read(kernel_dir().join("arena/checker.schema.json"))).unwrap()
}

struct Entry {
    name: &'static str,
    example: &'static str,
    fields: Vec<(String, String)>,
}

impl Entry {
    fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn req(&self, key: &str) -> &str {
        self.get(key)
            .unwrap_or_else(|| panic!("{}: no `{key}`", self.name))
    }

    /// The run command, which is a single shell line.
    fn run_line(&self) -> &str {
        let run = self.req("run");
        let lines: Vec<&str> = run.lines().collect();
        assert_eq!(lines.len(), 1, "{}: the run command is one line", self.name);
        lines[0]
    }
}

/// An entry is written in a strict subset of YAML: top-level `key: value` lines and `key: |`
/// literal blocks indented by two spaces. Anything else is refused, which keeps the files
/// reviewable at a glance and makes this reading coincide with the Arena's YAML loader.
fn parse_entry(name: &str, text: &str) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line.is_empty() {
            continue;
        }
        assert!(
            !line.starts_with([' ', '#', '-']),
            "{name}: not a top-level `key: value` line: {line:?}"
        );
        let (key, rest) = line
            .split_once(':')
            .unwrap_or_else(|| panic!("{name}: not a `key: value` line: {line:?}"));
        assert!(
            key.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "{name}: key {key:?}"
        );
        assert!(
            fields.iter().all(|(k, _)| k != key),
            "{name}: duplicate key `{key}`"
        );
        let value = if rest == " |" {
            let first = lines.peek().copied().unwrap_or_default();
            assert!(
                first.starts_with("  ") && !first.starts_with("   "),
                "{name}: the `{key}` block is indented by exactly two spaces"
            );
            let mut block = String::new();
            while let Some(next) = lines.peek().copied() {
                if next.is_empty() {
                    block.push('\n');
                } else if let Some(body) = next.strip_prefix("  ") {
                    block.push_str(body);
                    block.push('\n');
                } else {
                    break;
                }
                lines.next();
            }
            format!("{}\n", block.trim_end_matches('\n'))
        } else {
            let plain = rest
                .strip_prefix(' ')
                .unwrap_or_else(|| panic!("{name}: `{key}:` without a value"));
            assert!(
                !plain.is_empty() && plain == plain.trim(),
                "{name}: `{key}` value {plain:?}"
            );
            match plain.strip_prefix('"') {
                Some(quoted) => {
                    let inner = quoted
                        .strip_suffix('"')
                        .unwrap_or_else(|| panic!("{name}: unterminated `{key}`"));
                    assert!(
                        !inner.contains(['"', '\\']),
                        "{name}: `{key}` needs no escapes"
                    );
                    inner.to_string()
                }
                None => {
                    assert!(
                        !plain
                            .starts_with(['|', '>', '[', '{', '&', '*', '!', '\'', '%', '@', '`'])
                            && !plain.contains(": ")
                            && !plain.contains(" #"),
                        "{name}: `{key}` is a plain scalar: {plain:?}"
                    );
                    plain.to_string()
                }
            }
        };
        fields.push((key.to_string(), value));
    }
    fields
}

fn entries() -> Vec<Entry> {
    let dir = kernel_dir().join("arena");
    let mut found: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter_map(|f| f.strip_suffix(".yaml").map(str::to_string))
        .collect();
    found.sort();
    let expected: Vec<&str> = ENTRIES.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        found, expected,
        "every Arena entry in arena/ is known to this lock"
    );
    ENTRIES
        .iter()
        .map(|&(name, example)| Entry {
            name,
            example,
            fields: parse_entry(name, &read(dir.join(format!("{name}.yaml")))),
        })
        .collect()
}

fn workspace_repository() -> String {
    let manifest = read(workspace_root().join("Cargo.toml"));
    let package = manifest
        .split("\n[workspace.package]\n")
        .nth(1)
        .expect("a [workspace.package] table");
    let table = package.split("\n[").next().unwrap();
    let line = table
        .lines()
        .find(|l| l.starts_with("repository = "))
        .expect("[workspace.package] names the repository");
    line.trim_start_matches("repository = ")
        .trim_matches('"')
        .to_string()
}

#[test]
fn the_vendored_schema_is_the_arenas_at_the_pinned_revision() {
    let bytes = std::fs::read(kernel_dir().join("arena/checker.schema.json")).unwrap();
    assert_eq!(
        sha256_hex(&bytes),
        SCHEMA_SHA256,
        "arena/checker.schema.json is schemas/checker.json of lean-kernel-arena@{ARENA_REVISION}; \
         re-vendor it (scripts/lean/arena-crown/rehearse-submission.py refuses a drifted copy) and re-read this lock"
    );
    let s = schema();
    assert_eq!(
        s["additionalProperties"], false,
        "the Arena refuses fields it does not know"
    );
    let fields: BTreeSet<&str> = s["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let known: BTreeSet<&str> = [
        "build",
        "declines",
        "description",
        "dir",
        "disable",
        "ref",
        "rev",
        "run",
        "threads",
        "timeout",
        "url",
        "version",
    ]
    .into_iter()
    .collect();
    assert_eq!(fields, known);
    assert_eq!(s["properties"]["rev"]["pattern"], "^[a-fA-F0-9]{7,40}$");
}

/// The placeholders are the one way an unpinned entry departs from the schema (`SUBMISSION_REV`
/// is no commit hash); pinned, the same entry conforms.
fn pinned(e: &Entry) -> Vec<(String, String)> {
    e.fields
        .iter()
        .map(|(k, v)| match (k.as_str(), v.as_str()) {
            ("ref", REF_PLACEHOLDER) => (k.clone(), "v0.0.0".to_string()),
            ("rev", REV_PLACEHOLDER) => (k.clone(), "0".repeat(40)),
            _ => (k.clone(), v.clone()),
        })
        .collect()
}

#[test]
fn every_entry_conforms_to_the_arena_checker_schema_once_pinned() {
    let s = schema();
    let properties = s["properties"].as_object().unwrap();
    for e in entries() {
        for (key, value) in &pinned(&e) {
            let spec = properties
                .get(key)
                .unwrap_or_else(|| panic!("{}: `{key}` is not an Arena checker field", e.name));
            // A field is typed directly or as `oneOf` alternatives (`declines`: a name or a list);
            // an entry's value is a scalar, so it must match one scalar alternative.
            let types: Vec<&str> = match spec["type"].as_str() {
                Some(t) => vec![t],
                None => spec["oneOf"]
                    .as_array()
                    .unwrap_or_else(|| panic!("{}: `{key}` has no type in the schema", e.name))
                    .iter()
                    .filter_map(|alternative| alternative["type"].as_str())
                    .collect(),
            };
            let fits = types.iter().any(|t| match *t {
                "string" => true,
                "integer" => value.parse::<u32>().is_ok_and(|n| n >= 1),
                "boolean" => value == "true" || value == "false",
                _ => false,
            });
            assert!(
                fits,
                "{}: `{key}` = {value:?} is none of the schema's {types:?}",
                e.name
            );
            if key == "rev" {
                assert!(
                    (7..=40).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_hexdigit()),
                    "{}: `rev` matches the schema's ^[a-fA-F0-9]{{7,40}}$",
                    e.name
                );
            }
        }
        assert!(
            e.get("url").is_some() && e.get("dir").is_none(),
            "{}: the git-repository form",
            e.name
        );
        assert!(
            !e.req("description").trim().is_empty(),
            "{}: a description",
            e.name
        );
        assert!(e.get("disable").is_none(), "{}: enabled", e.name);
    }
}

/// The tests each entry declines without running. The dual entry checks every declaration with
/// both engines, and mathlib does not fit its timeout twice over; the kernel entry runs everything.
const DECLINES: [(&str, Option<&str>); 2] =
    [("logos-dual", Some("mathlib")), ("logos-kernel", None)];

#[test]
fn every_entry_declines_exactly_the_tests_on_record() {
    let manifest: serde_json::Value =
        serde_json::from_str(&read(workspace_root().join(CORPUS).join("manifest.json"))).unwrap();
    for e in entries() {
        let expected = DECLINES.iter().find(|(name, _)| *name == e.name).unwrap().1;
        assert_eq!(
            e.get("declines"),
            expected,
            "{}: the declines on record",
            e.name
        );
        if let Some(test) = expected {
            assert!(
                manifest["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["name"] == test),
                "{}: declines {test}, which is an Arena test",
                e.name
            );
            assert!(
                e.req("description").contains(&format!("declines {test}")),
                "{}: the description says it declines {test}",
                e.name
            );
        }
    }
}

#[test]
fn every_entry_is_pinned_to_a_full_commit_of_the_public_repository() {
    let repository = workspace_repository();
    for e in entries() {
        assert_eq!(
            e.req("url"),
            repository,
            "{}: the public repository",
            e.name
        );
        let (reference, revision) = (e.req("ref"), e.req("rev"));
        match (reference == REF_PLACEHOLDER, revision == REV_PLACEHOLDER) {
            (true, true) => {}
            (false, false) => {
                assert!(
                    revision.len() == 40
                        && revision
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                    "{}: `rev` is a full lowercase commit hash, not {revision:?}",
                    e.name
                );
                assert!(
                    !reference.is_empty() && !reference.contains(char::is_whitespace),
                    "{}: `ref` is a tag or branch name",
                    e.name
                );
            }
            _ => panic!("{}: `ref` and `rev` are pinned together", e.name),
        }
    }
}

#[test]
fn every_build_stops_at_its_first_failing_step() {
    for e in entries() {
        assert_eq!(
            e.req("build").lines().next(),
            Some("set -eu"),
            "{}: the build begins `set -eu`",
            e.name
        );
    }
}

#[test]
fn every_build_produces_the_binary_its_run_line_executes() {
    let workspace_manifest = read(workspace_root().join("Cargo.toml"));
    assert!(
        workspace_manifest.contains("\n[profile.arena]\n"),
        "the entries build with [profile.arena]"
    );
    for e in entries() {
        let build = e.req("build");
        assert!(
            build
                .lines()
                .any(|l| l == "export CARGO_TARGET_DIR=\"$PWD/target\""),
            "{}: the binary lands where the run line looks, whatever the environment says",
            e.name
        );
        let cargo = format!(
            "cargo build --locked --profile arena -p logicaffeine-kernel --example {}",
            e.example
        );
        assert_eq!(
            build.lines().filter(|l| l.trim_start() == cargo).count(),
            2,
            "{}: one instrumented and one profile-guided build of exactly this binary",
            e.name
        );
        let binary = format!("target/arena/examples/{}", e.example);
        let run = e.run_line();
        assert!(
            run.starts_with(&format!("{binary} ")),
            "{}: the run line executes {binary}",
            e.name
        );
        assert!(
            run.ends_with(" \"$IN\""),
            "{}: the run line reads the test from \"$IN\"",
            e.name
        );
        assert!(
            kernel_dir()
                .join("examples")
                .join(format!("{}.rs", e.example))
                .is_file(),
            "{}",
            e.name
        );
    }
}

#[test]
fn every_build_trains_its_profile_on_the_run_line_over_a_vendored_accept_case() {
    let manifest: serde_json::Value =
        serde_json::from_str(&read(workspace_root().join(CORPUS).join("manifest.json"))).unwrap();
    for e in entries() {
        let build = e.req("build");
        let train = build
            .lines()
            .find_map(|l| l.strip_prefix("TRAIN="))
            .unwrap_or_else(|| panic!("{}: the build names its training input as TRAIN=", e.name));
        let relative = train.strip_prefix(CORPUS).unwrap_or_else(|| {
            panic!(
                "{}: the training input is in the vendored corpus: {train}",
                e.name
            )
        });
        let case = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["path"] == relative && c["location"] == "vendored")
            .unwrap_or_else(|| panic!("{}: {relative} is a vendored corpus case", e.name));
        assert_eq!(
            case["outcome"], "accept",
            "{}: under `set -e` the training run must be accepted",
            e.name
        );
        assert!(
            workspace_root().join(train).is_file(),
            "{}: {train} exists",
            e.name
        );
        let training = e.run_line().replace("\"$IN\"", "\"$TRAIN\"");
        assert!(
            build.lines().any(|l| l == training),
            "{}: the profile is trained on the run line itself: {training}",
            e.name
        );
        assert!(
            build
                .lines()
                .any(|l| l == "llvm-profdata merge -o \"$PWD/pgo/merged.profdata\" \"$PWD/pgo\""),
            "{}: the training profile is merged for the profile-guided build",
            e.name
        );
    }
}

/// The `RUSTFLAGS` of each `cargo build` a recipe runs, in order, with its profile directory written `$PGO`.
fn build_flags(recipe: &str, pgo_dir: &str) -> Vec<String> {
    recipe
        .lines()
        .filter_map(|l| {
            let at = l.find("RUSTFLAGS=\"")? + "RUSTFLAGS=\"".len();
            let end = l[at..].find('"')? + at;
            Some(l[at..end].replace(pgo_dir, "$PGO"))
        })
        .collect()
}

/// Every entry's checker is linked as a position-dependent executable: the loader then patches no pointer of the
/// binary's own tables at start-up, which every test of the Arena counts (1,451 relative relocations, about 11,000
/// instructions a run, in a position-independent build of 10-25). The kernel's build script asks for it, for the
/// crate's examples alone (`cargo:rustc-link-arg-examples`). The entries' `RUSTFLAGS` carry no link argument and no
/// relocation model: they reach every crate the build compiles, a procedural macro's shared object among them, which
/// a linker told `-no-pie` breaks; and a `--target` build, which would keep them off it, checks Mathlib in 0.65–1.4%
/// more instructions.
#[test]
fn every_entry_links_its_checker_position_dependent() {
    for e in entries() {
        let build = e.req("build");
        let flags = build_flags(build, "$PWD/pgo");
        assert_eq!(flags.len(), 2, "{}: an instrumented and a profile-guided build", e.name);
        for f in &flags {
            assert!(
                !f.contains("link-arg") && !f.contains("relocation-model"),
                "{}: no link argument or relocation model in RUSTFLAGS, which reach a procedural macro's link: {f:?}",
                e.name
            );
        }
        assert!(!build.contains("--target"), "{}: built for the host, as the profile was measured", e.name);
    }
    let script = read(kernel_dir().join("build.rs"));
    assert!(
        script.contains(r#"println!("cargo:rustc-link-arg-examples=-no-pie")"#),
        "the kernel's build script links its examples position-dependent"
    );
    assert!(
        script.contains(r#"std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")"#),
        "on Linux, whose linker drivers take `-no-pie`"
    );
    assert!(
        !script.contains("cargo:rustc-link-arg=") && !script.contains("cargo:rustc-link-arg-bins"),
        "for the examples alone: the library and the tests keep the toolchain's default"
    );
}

/// The binary every measurement runs (`scripts/lean/arena-crown/snapshot.sh`) is built with the logos-kernel entry's
/// own flags, build for build, so what is measured is what the Arena builds.
#[test]
fn the_measured_snapshot_is_built_as_the_entry_builds() {
    let entries = entries();
    let kernel = entries.iter().find(|e| e.name == "logos-kernel").unwrap();
    let snapshot = read(workspace_root().join("scripts/lean/arena-crown/snapshot.sh"));
    assert_eq!(
        build_flags(&snapshot, "$PGO"),
        build_flags(kernel.req("build"), "$PWD/pgo"),
        "snapshot.sh's builds carry the entry's RUSTFLAGS"
    );
}

#[test]
fn logos_kernel_names_its_checking_engine() {
    let entries = entries();
    let kernel = entries.iter().find(|e| e.name == "logos-kernel").unwrap();
    let run = kernel.run_line();
    assert!(
        run.contains(" --check sem ") || run.contains(" --check classic "),
        "the entry names its engine, so the binary's default can move without changing what the Arena runs: {run}"
    );
}

#[test]
fn single_threaded_entries_reserve_no_extra_cpu_slots() {
    for e in entries() {
        assert!(
            e.get("threads").is_none(),
            "{}: `threads: N` is declared only by a checker that runs N workers at once",
            e.name
        );
    }
}

#[test]
fn the_readme_and_the_rehearsal_read_the_same_arena_revision() {
    let readme = read(kernel_dir().join("arena/README.md"));
    assert!(
        readme.contains(ARENA_REVISION),
        "arena/README.md names lean-kernel-arena@{ARENA_REVISION}"
    );
    let rehearsal = read(workspace_root().join("scripts/lean/arena-crown/rehearse-submission.py"));
    assert!(
        rehearsal.contains(&format!("ARENA_REVISION = \"{ARENA_REVISION}\"")),
        "the rehearsal runs the Arena at the revision this lock was read at"
    );
}

#[test]
fn the_strict_entry_reader_refuses_what_it_does_not_understand() {
    let ok = parse_entry(
        "t",
        "a: b\nrun: |\n  x \"$IN\"\n\n  y\n\nurl: https://h/x\n",
    );
    assert_eq!(
        ok,
        vec![
            ("a".to_string(), "b".to_string()),
            ("run".to_string(), "x \"$IN\"\n\ny\n".to_string()),
            ("url".to_string(), "https://h/x".to_string()),
        ]
    );
    assert_eq!(
        parse_entry("t", "v: \"1.0\"\n"),
        vec![("v".to_string(), "1.0".to_string())]
    );
    for bad in [
        "a: b\na: c\n",
        "# comment\n",
        "  indented: x\n",
        "a:\n",
        "a: [x]\n",
        "a: b # trailing\n",
        "a: >\n  folded\n",
        "run: |\n    four spaces\n",
        "Key: x\n",
        "- item\n",
    ] {
        let refused = std::panic::catch_unwind(|| parse_entry("t", bad)).is_err();
        assert!(refused, "the entry reader refuses {bad:?}");
    }
}
