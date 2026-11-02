//! Every program the repository holds, sorted by where it runs.
//!
//! The sources: the statement and expression probes ([`super::kinds`]), the semantic probes
//! ([`super::semantic`]), the cross-op corpus and the compute corpus of the gauntlet, every benchmark
//! (`interp.lg`, and `main.lg` at small sizes through its arguments), the machine-sympathy kernels
//! ([`super::machine_sympathy`]) at each of their sizes, the failure corpus, and every `logos` and `logos-dom`
//! block of the documentation. A program is sorted by what it contains, read from its parse, never by a label:
//! one that serves is a server, one that declares a page or drives one is a page, and every other program is run
//! on every backend. A program with `## Test` blocks is also a test program.

use std::collections::BTreeMap;

use super::kinds::{self, Probe, StmtKind};
use super::{docs, Program};

#[path = "../gauntlet/mod.rs"]
mod gauntlet;

const FAILURE_CORPUS: &[(&str, &str, &str, &str)] = include!("../failure_corpus.in");

/// The sizes every benchmark's `main.lg` runs at: small enough for the tree-walker, large enough to run every
/// loop and recursion the program has.
pub const BENCHMARK_SIZES: &[&str] = &["1", "3", "8"];

/// The statements that make a program a page.
const PAGE_KINDS: &[StmtKind] = &[
    StmtKind::Ui,
    StmtKind::ComponentDef,
    StmtKind::DirectiveDef,
    StmtKind::Fetch,
    StmtKind::GivenFetch,
    StmtKind::Replay,
    StmtKind::ServeFetches,
    StmtKind::Site,
];

/// The whole corpus.
#[derive(Default)]
pub struct Corpus {
    /// Run on every backend.
    pub programs: Vec<Program>,
    /// Their `## Test` blocks run on every backend that runs tests.
    pub tests: Vec<Program>,
    /// Mounted and driven on every backend that runs a page.
    pub pages: Vec<Program>,
    /// Served on every backend, and answered the same requests.
    pub servers: Vec<Program>,
    /// Meaningful on the Rust backends only; every other backend refuses them as the VM does.
    pub rust_only: Vec<Program>,
    /// Constructs no program can reach, or whose existence is the author's ruling: `(construct, why)`.
    pub open: Vec<(String, String)>,
    /// Probes that do not parse, which is a defect of the probe itself: `(id, advice)`. Any other program the
    /// front end rejects is run on every backend like the rest, since every backend must reject it the same way.
    pub unparsed_probes: Vec<(String, String)>,
    /// What each program that parses contains, by id.
    pub reached: BTreeMap<String, kinds::Reached>,
}

impl Corpus {
    fn place(&mut self, program: Program) {
        let reached =
            std::panic::catch_unwind(|| kinds::reached(&program.source)).unwrap_or_else(|panic| {
                Err(format!(
                    "the compiler panicked reading it: {}",
                    super::panic_text(&*panic)
                ))
            });
        match reached {
            Err(_) => self.programs.push(program),
            Ok(r) => {
                self.reached.insert(program.id.clone(), r.clone());
                if r.stmts.contains(&StmtKind::Serve) {
                    self.servers.push(program);
                } else if PAGE_KINDS.iter().any(|k| r.stmts.contains(k)) {
                    self.pages.push(program);
                } else {
                    if r.stmts.contains(&StmtKind::TestDef) {
                        self.tests.push(program.clone());
                    }
                    self.programs.push(program);
                }
            }
        }
    }

    /// A written probe: placed like any program, except that one which does not parse is a defect of the
    /// probe and one meaningful on the Rust backends only runs there.
    fn place_probe(&mut self, id: String, source: &'static str, rust_only: bool) {
        match kinds::reached(source) {
            Err(advice) => self.unparsed_probes.push((id, advice)),
            Ok(r) if rust_only => {
                self.reached.insert(id.clone(), r);
                self.rust_only.push(Program::new(id, source));
            }
            Ok(_) => self.place(Program::new(id, source)),
        }
    }
}

/// Every statement and expression kind with its probe, by construct name.
fn constructs() -> impl Iterator<Item = (String, Probe)> {
    kinds::ALL_STMT_KINDS
        .iter()
        .map(|&k| (format!("stmt/{k:?}"), kinds::stmt_probe(k)))
        .chain(
            kinds::ALL_EXPR_KINDS
                .iter()
                .map(|&k| (format!("expr/{k:?}"), kinds::expr_probe(k))),
        )
}

/// Every probe written to exercise one construct or builtin, by id, with whether it is meaningful on the Rust
/// backends only: the statement probes, the expression probes, then the builtin probes. A construct whose
/// probe is `Probe::Open` has none.
pub fn probes() -> Vec<(String, &'static str, bool)> {
    let mut out = Vec::new();
    for (construct, probe) in constructs() {
        let (programs, rust_only): (&[&'static str], bool) = match probe {
            Probe::Programs(p)
            | Probe::Tests(p)
            | Probe::Pages(p)
            | Probe::Servers(p)
            | Probe::LoweredAway(p) => (p, false),
            Probe::RustOnly(p) => (p, true),
            Probe::Open(_) => continue,
        };
        out.extend(
            programs
                .iter()
                .enumerate()
                .map(|(i, source)| (format!("{construct}/{i}"), *source, rust_only)),
        );
    }
    out.extend(
        kinds::BUILTIN_PROBES
            .iter()
            .map(|(name, source)| (format!("builtin/{name}"), *source, false)),
    );
    out
}

fn benchmarks_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

/// Every program, each placed by what it contains.
pub fn corpus() -> Corpus {
    let mut c = Corpus::default();
    for (construct, probe) in constructs() {
        if let Probe::Open(why) = probe {
            c.open.push((construct, why.to_string()));
        }
    }
    for (id, source, rust_only) in probes() {
        c.place_probe(id, source, rust_only);
    }
    for (name, source) in super::semantic::SEMANTIC_PROBES {
        c.place(Program::new(format!("semantic/{name}"), *source));
    }
    for p in gauntlet::harvested() {
        c.place(Program::new(format!("gauntlet/{}", p.name), p.src));
    }
    for p in gauntlet::compute_corpus() {
        c.place(Program::new(format!("compute/{}", p.name), p.src));
    }
    let mut benches: Vec<_> = std::fs::read_dir(benchmarks_dir())
        .expect("benchmarks/programs")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    benches.sort();
    for dir in benches {
        let name = dir
            .file_name()
            .expect("a benchmark's name")
            .to_string_lossy()
            .to_string();
        if let Ok(src) = std::fs::read_to_string(dir.join("interp.lg")) {
            c.place(Program::new(format!("benchmark/{name}/interp"), src));
        }
        if let Ok(src) = std::fs::read_to_string(dir.join("main.lg")) {
            for size in BENCHMARK_SIZES {
                let mut p = Program::new(format!("benchmark/{name}/main/{size}"), src.clone());
                p.args = vec![size.to_string()];
                c.place(p);
            }
        }
    }
    for p in super::machine_sympathy::programs() {
        c.place(p);
    }
    for (name, src, _, _) in FAILURE_CORPUS {
        c.place(Program::new(format!("failure/{name}"), *src));
    }
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for block in docs::all_blocks() {
        let source = match block.lang.as_str() {
            "logos" => docs::make_program(&block.body),
            "logos-dom" => block.body.clone(),
            _ => continue,
        };
        let base = format!("doc/{}:{}", block.file, block.line);
        let n = seen.entry(base.clone()).or_default();
        *n += 1;
        let id = if *n == 1 { base } else { format!("{base}#{n}") };
        if block.lang == "logos-dom" {
            if let Ok(Ok(r)) = std::panic::catch_unwind(|| kinds::reached(&source)) {
                c.reached.insert(id.clone(), r);
            }
            c.pages.push(Program::new(id, source));
        } else {
            c.place(Program::new(id, source));
        }
    }
    c
}
