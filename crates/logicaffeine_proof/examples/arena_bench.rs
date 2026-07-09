//! arena_bench — competition-style head-to-head SAT runner.
//!
//! Mimics the SAT-Competition main-track protocol: every engine is handed the byte-identical DIMACS
//! file, run under a fixed wall-clock `timeout`, and judged by its `s SATISFIABLE` / `s
//! UNSATISFIABLE` line. We score PAR-2 (penalised average runtime: solved → wall time, unsolved →
//! 2× timeout) and solved-count, exactly as the competition ranks the main track. Our own answers
//! are re-checked — SAT models verified in-process against the formula — and every engine's verdict
//! is cross-checked against the others so any SAT/UNSAT disagreement (a correctness bug) is flagged
//! loudly rather than silently scored.
//!
//! Engines are run in pure-solve mode (no proof logging) so the timing is the honest solve cost;
//! certified-proof timing is a separate concern handled by run-satbench.sh.
//!
//! Usage:  cargo run --release -p logicaffeine-proof --example arena_bench -- [instances_dir]
//! Env:    ARENA_TIMEOUT=<secs>  ARENA_CAP=<per-category cap>
//!         OURS_BIN  KISSAT_BIN  CADICAL_BIN   (paths; absent binaries are skipped)
//! Out:    benchmarks/results/arena/sat.json  + a summary table on stdout.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::dimacs::{self, DimacsCnf};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Sat,
    Unsat,
    Unknown,
    Timeout,
    Error,
}

impl Verdict {
    fn as_str(self) -> &'static str {
        match self {
            Verdict::Sat => "SAT",
            Verdict::Unsat => "UNSAT",
            Verdict::Unknown => "UNKNOWN",
            Verdict::Timeout => "TIMEOUT",
            Verdict::Error => "ERROR",
        }
    }
    fn solved(self) -> bool {
        matches!(self, Verdict::Sat | Verdict::Unsat)
    }
}

struct Solver {
    name: &'static str,
    bin: String,
}

struct Outcome {
    verdict: Verdict,
    seconds: f64,
    verified: Option<bool>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).cloned().unwrap_or_else(|| "benchmarks/arena/instances/sat".to_string());
    let timeout_s: f64 = env_f64("ARENA_TIMEOUT", 10.0);
    let cap: usize = env_f64("ARENA_CAP", 4.0) as usize;

    let candidates = [
        ("ours", env_str("OURS_BIN", "target/release/logos-sat")),
        ("kissat", env_str("KISSAT_BIN", "/tmp/kissat/build/kissat")),
        ("cadical", env_str("CADICAL_BIN", "/tmp/cadical/build/cadical")),
    ];
    let solvers: Vec<Solver> = candidates
        .into_iter()
        .filter(|(_, bin)| Path::new(bin).exists())
        .map(|(name, bin)| Solver { name, bin })
        .collect();
    if solvers.is_empty() {
        eprintln!("no solver binaries found (build logos-sat: cargo build --release -p logicaffeine-proof --bin logos-sat)");
        std::process::exit(1);
    }

    let instances = collect_instances(&dir, cap);
    if instances.is_empty() {
        eprintln!("no instances under {dir} — run: bash benchmarks/arena/fetch-sat.sh");
        std::process::exit(1);
    }

    eprintln!(
        "arena=sat  solvers=[{}]  timeout={timeout_s}s  cap/category={cap}  instances={}",
        solvers.iter().map(|s| s.name).collect::<Vec<_>>().join(", "),
        instances.len()
    );

    // Accumulators.
    let mut summary: BTreeMap<&str, Stat> = solvers.iter().map(|s| (s.name, Stat::default())).collect();
    let mut cat_solved: BTreeMap<(String, &str), usize> = BTreeMap::new();
    let mut cat_count: BTreeMap<String, usize> = BTreeMap::new();
    let mut conflicts: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut rows: Vec<String> = Vec::new();

    for (idx, inst) in instances.iter().enumerate() {
        let text = match std::fs::read_to_string(&inst.path) {
            Ok(t) => t,
            Err(e) => {
                skipped.push(format!("{} (read: {e})", inst.name));
                continue;
            }
        };
        let cnf = match dimacs::parse(&text) {
            Ok(c) => c,
            Err(e) => {
                skipped.push(format!("{} (parse: {e:?})", inst.name));
                continue;
            }
        };
        *cat_count.entry(inst.category.clone()).or_default() += 1;

        let mut definite: Vec<(&str, Verdict)> = Vec::new();
        let mut per_solver: Vec<(&str, Outcome)> = Vec::new();
        for s in &solvers {
            let (verdict, seconds, stdout) = run_one(&s.bin, &inst.path, timeout_s);
            let verified = if s.name == "ours" {
                match verdict {
                    Verdict::Sat => Some(model_satisfies(&stdout, &cnf)),
                    _ => None,
                }
            } else {
                None
            };
            if verdict.solved() {
                definite.push((s.name, verdict));
            }
            let stat = summary.get_mut(s.name).unwrap();
            stat.record(verdict, seconds, timeout_s, verified);
            if verdict.solved() {
                *cat_solved.entry((inst.category.clone(), s.name)).or_default() += 1;
            }
            per_solver.push((s.name, Outcome { verdict, seconds, verified }));
        }

        // Cross-check: any SAT-vs-UNSAT disagreement is a correctness bug somewhere.
        let sat = definite.iter().any(|(_, v)| *v == Verdict::Sat);
        let unsat = definite.iter().any(|(_, v)| *v == Verdict::Unsat);
        if sat && unsat {
            let detail: Vec<String> =
                definite.iter().map(|(n, v)| format!("{n}={}", v.as_str())).collect();
            conflicts.push(format!("{}: {}", inst.name, detail.join(" ")));
        }

        rows.push(instance_json(inst, &cnf, &per_solver));

        eprintln!(
            "[{:>3}/{}] {:<28} v={:<7} {}",
            idx + 1,
            instances.len(),
            truncate(&inst.name, 28),
            cnf.num_vars,
            per_solver
                .iter()
                .map(|(n, o)| format!("{n}:{}/{:.2}s", o.verdict.as_str(), o.seconds))
                .collect::<Vec<_>>()
                .join("  ")
        );
    }

    print_summary(&solvers, &summary, &cat_count, &cat_solved, &conflicts, timeout_s);
    if !skipped.is_empty() {
        println!("\nskipped {} instance(s) (unreadable/unparseable):", skipped.len());
        for s in &skipped {
            println!("  {s}");
        }
    }
    write_json(&solvers, &summary, &cat_count, &cat_solved, &conflicts, &skipped, &rows, timeout_s, cap);
}

#[derive(Default)]
struct Stat {
    solved: usize,
    sat: usize,
    unsat: usize,
    timeout: usize,
    error: usize,
    par2: f64,
    verified: usize,
    verify_fail: usize,
}

impl Stat {
    fn record(&mut self, v: Verdict, secs: f64, timeout_s: f64, verified: Option<bool>) {
        match v {
            Verdict::Sat => {
                self.solved += 1;
                self.sat += 1;
                self.par2 += secs;
            }
            Verdict::Unsat => {
                self.solved += 1;
                self.unsat += 1;
                self.par2 += secs;
            }
            Verdict::Timeout => {
                self.timeout += 1;
                self.par2 += 2.0 * timeout_s;
            }
            Verdict::Unknown | Verdict::Error => {
                self.error += 1;
                self.par2 += 2.0 * timeout_s;
            }
        }
        match verified {
            Some(true) => self.verified += 1,
            Some(false) => self.verify_fail += 1,
            None => {}
        }
    }
}

struct Instance {
    name: String,
    category: String,
    path: PathBuf,
}

/// Recursively collect `*.cnf`, grouped by their immediate parent directory name (the category),
/// sorted, and capped to `cap` per category for a bounded run.
fn collect_instances(root: &str, cap: usize) -> Vec<Instance> {
    let mut by_cat: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut stack = vec![PathBuf::from(root)];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                // Skip `_`-prefixed dirs (e.g. _malformed quarantine from normalize-sat.sh).
                let skip = p.file_name().map(|n| n.to_string_lossy().starts_with('_')).unwrap_or(false);
                if !skip {
                    stack.push(p);
                }
            } else if p.extension().map(|x| x == "cnf").unwrap_or(false) {
                let cat = p
                    .parent()
                    .and_then(|x| x.file_name())
                    .map(|x| x.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "misc".to_string());
                by_cat.entry(cat).or_default().push(p);
            }
        }
    }
    let mut out = Vec::new();
    for (cat, mut paths) in by_cat {
        paths.sort();
        for p in paths.into_iter().take(cap) {
            let name = p.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            out.push(Instance { name, category: cat.clone(), path: p });
        }
    }
    out
}

/// Run `bin` on `cnf` under a wall-clock `timeout`, returning (verdict, seconds, stdout).
fn run_one(bin: &str, cnf: &Path, timeout_s: f64) -> (Verdict, f64, String) {
    let mut cmd = Command::new("timeout");
    cmd.arg("-k").arg("2").arg(format!("{timeout_s}")).arg(bin).arg(cnf);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    let t = Instant::now();
    let out = cmd.output();
    let seconds = t.elapsed().as_secs_f64();
    match out {
        Ok(o) => {
            // `timeout(1)` exits 124 on TERM, 137 (128+SIGKILL) when -k escalates.
            if matches!(o.status.code(), Some(124) | Some(137)) {
                return (Verdict::Timeout, timeout_s, String::new());
            }
            let s = String::from_utf8_lossy(&o.stdout).into_owned();
            (parse_verdict(&s), seconds, s)
        }
        Err(_) => (Verdict::Error, seconds, String::new()),
    }
}

fn parse_verdict(stdout: &str) -> Verdict {
    let mut v = Verdict::Unknown;
    for line in stdout.lines() {
        let l = line.trim();
        if l == "s SATISFIABLE" {
            v = Verdict::Sat;
        } else if l == "s UNSATISFIABLE" {
            v = Verdict::Unsat;
        }
    }
    v
}

/// Verify a reported SAT model (from `v …` lines) against the formula.
fn model_satisfies(stdout: &str, cnf: &DimacsCnf) -> bool {
    let mut assign = vec![false; cnf.num_vars];
    let mut saw_model = false;
    for line in stdout.lines() {
        let Some(rest) = line.strip_prefix("v") else { continue };
        saw_model = true;
        for tok in rest.split_whitespace() {
            let Ok(n) = tok.parse::<i64>() else { return false };
            if n == 0 {
                continue;
            }
            let idx = (n.unsigned_abs() - 1) as usize;
            if idx >= cnf.num_vars {
                return false;
            }
            assign[idx] = n > 0;
        }
    }
    if !saw_model {
        return false;
    }
    cnf.clauses
        .iter()
        .all(|c| c.iter().any(|l: &Lit| assign[l.var() as usize] == l.is_positive()))
}

// --- reporting ---------------------------------------------------------------------------------

fn print_summary(
    solvers: &[Solver],
    summary: &BTreeMap<&str, Stat>,
    cat_count: &BTreeMap<String, usize>,
    cat_solved: &BTreeMap<(String, &str), usize>,
    conflicts: &[String],
    timeout_s: f64,
) {
    let total: usize = cat_count.values().sum();
    println!("\n=== arena: SAT  ({total} instances, timeout {timeout_s}s, PAR-2) ===\n");
    println!("{:<10} {:>7} {:>5} {:>6} {:>8} {:>10} {:>9}", "solver", "solved", "sat", "unsat", "timeout", "PAR-2", "verified");
    for s in solvers {
        let st = &summary[s.name];
        println!(
            "{:<10} {:>6}/{} {:>5} {:>6} {:>8} {:>10.1} {:>9}",
            s.name, st.solved, total, st.sat, st.unsat, st.timeout, st.par2,
            if st.verify_fail > 0 { format!("{}!FAIL", st.verified) } else { st.verified.to_string() }
        );
    }

    println!("\nper category (solved / total):");
    print!("{:<20}", "category");
    for s in solvers {
        print!(" {:>10}", s.name);
    }
    println!();
    for (cat, n) in cat_count {
        print!("{:<20}", truncate(cat, 20));
        for s in solvers {
            let solved = cat_solved.get(&(cat.clone(), s.name)).copied().unwrap_or(0);
            print!(" {:>10}", format!("{solved}/{n}"));
        }
        println!();
    }

    if conflicts.is_empty() {
        println!("\ncross-check: no SAT/UNSAT disagreements ✓");
    } else {
        println!("\ncross-check: {} DISAGREEMENT(S) — correctness bug:", conflicts.len());
        for c in conflicts {
            println!("  {c}");
        }
    }
}

fn write_json(
    solvers: &[Solver],
    summary: &BTreeMap<&str, Stat>,
    cat_count: &BTreeMap<String, usize>,
    cat_solved: &BTreeMap<(String, &str), usize>,
    conflicts: &[String],
    skipped: &[String],
    rows: &[String],
    timeout_s: f64,
    cap: usize,
) {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str("  \"arena\": \"sat\",\n");
    s.push_str(&format!("  \"timeout_s\": {timeout_s},\n"));
    s.push_str(&format!("  \"cap_per_category\": {cap},\n"));
    s.push_str(&format!(
        "  \"solvers\": [{}],\n",
        solvers.iter().map(|x| format!("\"{}\"", x.name)).collect::<Vec<_>>().join(", ")
    ));
    // summary
    s.push_str("  \"summary\": {\n");
    let mut parts = Vec::new();
    for sv in solvers {
        let st = &summary[sv.name];
        parts.push(format!(
            "    \"{}\": {{\"solved\": {}, \"sat\": {}, \"unsat\": {}, \"timeout\": {}, \"error\": {}, \"par2\": {:.3}, \"verified\": {}, \"verify_fail\": {}}}",
            sv.name, st.solved, st.sat, st.unsat, st.timeout, st.error, st.par2, st.verified, st.verify_fail
        ));
    }
    s.push_str(&parts.join(",\n"));
    s.push_str("\n  },\n");
    // categories
    s.push_str("  \"categories\": {\n");
    let mut cparts = Vec::new();
    for (cat, n) in cat_count {
        let solved: Vec<String> = solvers
            .iter()
            .map(|sv| format!("\"{}\": {}", sv.name, cat_solved.get(&(cat.clone(), sv.name)).copied().unwrap_or(0)))
            .collect();
        cparts.push(format!("    \"{cat}\": {{\"count\": {n}, {}}}", solved.join(", ")));
    }
    s.push_str(&cparts.join(",\n"));
    s.push_str("\n  },\n");
    // conflicts + skipped
    s.push_str(&format!(
        "  \"conflicts\": [{}],\n",
        conflicts.iter().map(|c| format!("\"{}\"", json_escape(c))).collect::<Vec<_>>().join(", ")
    ));
    s.push_str(&format!(
        "  \"skipped\": [{}],\n",
        skipped.iter().map(|c| format!("\"{}\"", json_escape(c))).collect::<Vec<_>>().join(", ")
    ));
    // instances
    s.push_str("  \"instances\": [\n");
    s.push_str(&rows.join(",\n"));
    s.push_str("\n  ]\n}\n");

    let _ = std::fs::create_dir_all("benchmarks/results/arena");
    let out = env_str("ARENA_OUT", "benchmarks/results/arena/sat.json");
    if std::fs::write(&out, &s).is_ok() {
        println!("\nwrote {out}");
    }
}

fn instance_json(inst: &Instance, cnf: &DimacsCnf, per: &[(&str, Outcome)]) -> String {
    let results: Vec<String> = per
        .iter()
        .map(|(n, o)| {
            let ver = match o.verified {
                Some(b) => b.to_string(),
                None => "null".to_string(),
            };
            format!(
                "\"{n}\": {{\"verdict\": \"{}\", \"seconds\": {:.4}, \"verified\": {ver}}}",
                o.verdict.as_str(),
                o.seconds
            )
        })
        .collect();
    format!(
        "    {{\"name\": \"{}\", \"category\": \"{}\", \"vars\": {}, \"clauses\": {}, {}}}",
        json_escape(&inst.name),
        json_escape(&inst.category),
        cnf.num_vars,
        cnf.clauses.len(),
        results.join(", ")
    )
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n.saturating_sub(1)])
    }
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn env_str(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
