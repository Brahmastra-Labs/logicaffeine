//! eda_bench — the EDA arenas: our engines against ABC's on the public corpora, every answer checked, a JSON of
//! results per arena.
//!
//! - `mc`: model checking on HWMCC'24's bit-level track. Ours is `mc_check --engine loc`, each cone reduced by its
//!   signal correspondence, then PDR on localization abstractions of a cone of over a thousand latches and on the whole
//!   of a smaller one, one thread like ABC's (an invariant accepted only once refuted three ways, a run only once
//!   replayed); ABC's is `pdr` after [`ABC_READ`], which
//!   gives ABC the circuit AIGER 1.9 describes. Each verdict is scored against the competition's consensus: a verdict
//!   that contradicts it is wrong. The competition's own solvers are counted from its results table, each correct
//!   verdict it reached within the same limit, once in real seconds and once in CPU seconds over all its threads, the
//!   clock our engines run under (its runs, its hardware).
//! - `bmc`: bounded model checking on the same track's designs the competition did not find safe, the only ones a run
//!   can answer. Ours is `mc_check --engine bmc`, a clock at a time and a window of clocks at a time by turns, a run
//!   accepted only once replayed; ABC's is `bmc3` after [`ABC_READ`]. Scored as `mc` is.
//! - `rewrite`: logic rewriting on the EPFL combinational suite. Ours is `rewrite_check`, each result proven equal to
//!   its input by `fraig`; ABC's are `rewrite` run four times and its ten-command `resyn2` script. Each of our results
//!   is also checked by ABC's `cec` within the same CPU limit: equal, or past the limit, and an unequal one fails the
//!   run.
//! - `count`: exact model counting on the 2024 model counting competition's first track, ours alone (`count_check`).
//!
//! Every engine run is a subprocess limited and timed in CPU seconds (`prlimit --cpu`, the child's user and system
//! time), so a loaded box ranks the engines alike; runs go `--jobs` at a time. With `--cores`, one worker runs on each
//! CPU listed, every run it makes pinned there (`taskset`): on a machine of performance and efficiency cores a run's
//! CPU seconds then never depend on which kind it landed on. The arena's JSON names the CPUs.
//!
//! Usage:  eda_bench <mc|bmc|rewrite|count> [--jobs <n> | --cores <cpu,cpu,…>] [--timeout <seconds>]
//!         [--limit <instances>]
//! Env:    OURS_MC_BIN OURS_REWRITE_BIN OURS_COUNT_BIN (default: the examples beside this one), ABC_BIN (default
//!         `yosys-abc`), HWMCC_DIR HWMCC_CSV EPFL_DIR MCC_DIR (default: under `external/`), EDA_OUT (default
//!         `benchmarks/results/eda/<arena>.json`).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// What a CPU-limited run of one engine gave: its stdout and stderr, and its seconds; `None` past the limit.
struct Run {
    output: Option<String>,
    seconds: f64,
}

/// The CPUs `--cores` lists, one worker on each.
static CORES: OnceLock<Vec<usize>> = OnceLock::new();

thread_local! {
    /// The CPU this worker's runs are pinned to, under `--cores`.
    static PINNED: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Run `program` with `args` within `timeout` CPU seconds, its rusage written to a file of its own, on this worker's CPU
/// when it has one.
fn run(program: &str, args: &[String], timeout: f64) -> Run {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let rusage = std::env::temp_dir().join(format!(
        "eda-rusage-{}-{}.txt",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut cmd = Command::new("/usr/bin/time");
    cmd.arg("-f").arg("%U %S").arg("-o").arg(&rusage);
    if let Some(cpu) = PINNED.get() {
        cmd.arg("taskset").arg("-c").arg(cpu.to_string());
    }
    cmd.arg("timeout").arg("-k").arg("2").arg(format!("{}", timeout * 4.0 + 10.0));
    cmd.arg("prlimit").arg(format!("--cpu={}", timeout.ceil() as u64));
    cmd.arg(program).args(args);
    cmd.stdin(Stdio::null());
    let out = cmd.output();
    let seconds = std::fs::read_to_string(&rusage)
        .ok()
        .and_then(|text| {
            let mut fields = text.lines().last()?.split_whitespace().map(|f| f.parse::<f64>().ok());
            Some(fields.next()?? + fields.next()??)
        })
        .unwrap_or(timeout);
    let _ = std::fs::remove_file(&rusage);
    match out {
        Ok(o) if !matches!(o.status.code(), Some(124) | Some(137) | Some(152)) && seconds < timeout => Run {
            output: Some(format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
            seconds,
        },
        _ => Run { output: None, seconds: timeout },
    }
}

/// Run `work` over `items`, `jobs` at a time, each result in its item's place; under `--cores`, worker `w` on the `w`-th
/// CPU listed.
fn in_parallel<T: Sync, R: Send>(items: &[T], jobs: usize, work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|scope| {
        let (next, results, work) = (&next, &results, &work);
        for w in 0..jobs.max(1) {
            scope.spawn(move || {
                PINNED.set(CORES.get().and_then(|cores| cores.get(w)).copied());
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= items.len() {
                        break;
                    }
                    let r = work(&items[i]);
                    results.lock().expect("no worker panicked")[i] = Some(r);
                }
            });
        }
    });
    results.into_inner().expect("no worker panicked").into_iter().map(|r| r.expect("every item ran")).collect()
}

/// The number after `key=` in `text`.
fn field(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("{key}="))? + key.len() + 1;
    Some(text[at..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '.').collect())
}

/// The And-node count ABC's `print_stats` gives, the last one in `text`.
fn ands(text: &str) -> Option<u64> {
    let at = text.rfind("and =")? + 5;
    text[at..].split_whitespace().next()?.parse().ok()
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn env_or(key: &str, default: impl Into<String>) -> String {
    std::env::var(key).unwrap_or_else(|_| default.into())
}

/// A binary built beside this one, `name`, unless `key` names another.
fn sibling(key: &str, name: &str) -> String {
    let beside = std::env::current_exe().ok().and_then(|p| Some(p.parent()?.join(name)));
    env_or(key, beside.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| name.to_string()))
}

/// The files under `dir` with extension `ext`, sorted, at most `limit`.
fn files(dir: &Path, ext: &str, limit: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.to_string_lossy().ends_with(ext) {
                found.push(p);
            }
        }
    }
    found.sort();
    found.truncate(limit);
    found
}

/// The run's provenance: the date, the CPU, the clock and the CPU limit, and ABC's version when it races.
fn provenance(arena: &str, corpus: &str, timeout: f64, abc: Option<&str>) -> String {
    let date = Command::new("date").args(["-u", "+%Y-%m-%d"]).output().ok();
    let date = date.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|t| t.lines().find(|l| l.starts_with("model name"))?.split(':').nth(1).map(|s| s.trim().to_string()))
        .unwrap_or_default();
    let mut s = format!(
        "  \"arena\": \"{arena}\",\n  \"corpus\": \"{}\",\n  \"date\": \"{date}\",\n  \"cpu\": \"{}\",\n  \"clock\": \"cpu\",\n  \"timeout_s\": {timeout},\n",
        json_escape(corpus),
        json_escape(&cpu)
    );
    if let Some(cores) = CORES.get() {
        s.push_str(&format!("  \"cores\": [{}],\n", cores.iter().map(usize::to_string).collect::<Vec<_>>().join(", ")));
    }
    if let Some(bin) = abc {
        let version = Command::new("yosys").arg("-V").output().ok();
        let version = version.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        s.push_str(&format!("  \"abc\": \"{}\",\n  \"abc_version\": \"{}\",\n", json_escape(bin), json_escape(&version)));
    }
    s
}

fn write(arena: &str, json: String) {
    let out = env_or("EDA_OUT", format!("benchmarks/results/eda/{arena}.json"));
    if let Some(dir) = Path::new(&out).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&out, json).unwrap_or_else(|e| panic!("{out}: {e}"));
    println!("wrote {out}");
}

/// A verdict of model checking.
fn verdict(ours: &str) -> Option<&'static str> {
    if ours.contains(" unsafe") {
        Some("unsafe")
    } else if ours.contains(" safe") {
        Some("safe")
    } else {
        None
    }
}

/// How ABC reads a design of the model-checking arenas before its engine runs: `undc` takes the first value of each
/// latch free to start anywhere from an input of its own, one latch added to mark the first clock; `zero` makes every
/// latch start at zero; `fold` folds the invariant constraints into the property. ABC's engines read a latch free to
/// start anywhere as starting at zero, and so does `fold` before `undc`: without `undc`, a proof of ABC's would cover
/// only the runs from those latches at zero, which is no proof of the circuit.
const ABC_READ: &str = "logic; undc; strash; zero; fold";

/// Model checking (`mc`, by PDR) or bounded model checking (`bmc`) on HWMCC'24, ours against ABC's.
fn model_checking(arena: &str, jobs: usize, timeout: f64, limit: usize) {
    let bounded = arena == "bmc";
    let ours_bin = sibling("OURS_MC_BIN", "mc_check");
    let abc_bin = env_or("ABC_BIN", "yosys-abc");
    let dir = PathBuf::from(env_or("HWMCC_DIR", "external/hwmcc/hwmcc24/aiger"));
    let csv = env_or("HWMCC_CSV", "external/hwmcc/hwmcc24/hwmcc24_results_aiger.csv");
    let mut votes: BTreeMap<String, (u32, u32)> = BTreeMap::new();
    let mut entries: Vec<(String, String, String, f64, f64)> = Vec::new();
    let table = std::fs::read_to_string(&csv).unwrap_or_else(|e| panic!("{csv}: {e}"));
    let mut lines = table.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split(',').collect();
    let column = |name: &str| header.iter().position(|h| *h == name).unwrap_or_else(|| panic!("a {name} column"));
    let (bench, solver, result) = (column("benchmark"), column("solver"), column("result"));
    let (real, cpu) = (column("time_real"), column("time_cpu"));
    for line in lines {
        let cells: Vec<&str> = line.split(',').collect();
        let (Some(b), Some(r)) = (cells.get(bench), cells.get(result)) else { continue };
        let seconds = |at: usize| cells.get(at).and_then(|t| t.parse::<f64>().ok());
        if let (Some(who), Some(real), Some(cpu)) = (cells.get(solver), seconds(real), seconds(cpu)) {
            entries.push((b.to_string(), who.to_string(), r.to_string(), real, cpu));
        }
        let tally = votes.entry(b.to_string()).or_default();
        match *r {
            "sat" => tally.0 += 1,
            "unsat" => tally.1 += 1,
            _ => {}
        }
    }
    let consensus = |name: &str| -> &'static str {
        match votes.get(name) {
            Some(&(sat, unsat)) if sat + unsat > 0 => {
                if sat >= unsat {
                    "unsafe"
                } else {
                    "safe"
                }
            }
            _ => "unknown",
        }
    };
    let mut designs = files(&dir, ".aig", usize::MAX);
    if bounded {
        designs.retain(|f| consensus(&f.strip_prefix(&dir).unwrap_or(f).to_string_lossy()) != "safe");
    }
    designs.truncate(limit);
    let rows = in_parallel(&designs, jobs, |f| {
        let name = f.strip_prefix(&dir).unwrap_or(f).to_string_lossy().into_owned();
        let path = f.to_string_lossy().into_owned();
        let args: Vec<String> = if bounded {
            vec!["--engine".into(), "bmc".into(), "--max-depth".into(), "1000000".into(), path]
        } else {
            vec!["--engine".into(), "loc".into(), path]
        };
        let ours = run(&ours_bin, &args, timeout);
        let script = format!("read_aiger {}; {ABC_READ}; {}", f.display(), if bounded { "bmc3" } else { "pdr" });
        let abc = run(&abc_bin, &["-c".into(), script], timeout);
        let ours_says = ours.output.as_deref().and_then(verdict);
        let abc_says = abc.output.as_deref().and_then(|o| {
            if o.contains("Property proved") {
                Some("safe")
            } else if o.contains("was asserted") {
                Some("unsafe")
            } else {
                None
            }
        });
        eprintln!("{name}: ours {ours_says:?} {:.2}s, abc {abc_says:?} {:.2}s", ours.seconds, abc.seconds);
        (name, (ours_says, ours.seconds), (abc_says, abc.seconds))
    });
    let mut json = String::from("{\n");
    let corpus = if bounded {
        "HWMCC'24 bit-level track (AIGER), the designs the competition did not find safe"
    } else {
        "HWMCC'24 bit-level track (AIGER)"
    };
    json.push_str(&provenance(arena, corpus, timeout, Some(&abc_bin)));
    json.push_str(&format!("  \"abc_read\": \"{ABC_READ}\",\n"));
    let mut field: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (b, who, r, real, cpu) in &entries {
        let said = match r.as_str() {
            "sat" => "unsafe",
            "unsat" => "safe",
            _ => continue,
        };
        if designs.iter().any(|f| f.strip_prefix(&dir).unwrap_or(f).to_string_lossy() == *b) && consensus(b) == said {
            let tally = field.entry(who.clone()).or_default();
            tally.0 += usize::from(*real <= timeout);
            tally.1 += usize::from(*cpu <= timeout);
        }
    }
    let mut field: Vec<(String, (usize, usize))> = field.into_iter().filter(|(_, (real, cpu))| real + cpu > 0).collect();
    field.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    json.push_str("  \"field_clock\": \"real\",\n  \"field\": [");
    let solver = |(who, (real, cpu)): &(String, (usize, usize))| {
        format!("{{\"solver\": \"{}\", \"solved\": {real}, \"solved_cpu\": {cpu}}}", json_escape(who))
    };
    json.push_str(&field.iter().map(solver).collect::<Vec<_>>().join(", "));
    json.push_str("],\n");
    json.push_str("  \"engines\": [\"ours\", \"abc\"],\n  \"summary\": {\n");
    let mut summaries = Vec::new();
    for (k, engine) in ["ours", "abc"].iter().enumerate() {
        let (mut safe, mut unsafe_, mut wrong, mut par2) = (0, 0, 0, 0.0);
        for (name, o, a) in &rows {
            let (says, seconds) = if k == 0 { o } else { a };
            match says {
                Some(v) => {
                    if *v == "safe" { safe += 1 } else { unsafe_ += 1 }
                    let c = consensus(name);
                    if c != "unknown" && c != *v {
                        wrong += 1;
                    }
                    par2 += seconds;
                }
                None => par2 += 2.0 * timeout,
            }
        }
        summaries.push(format!(
            "    \"{engine}\": {{\"solved\": {}, \"safe\": {safe}, \"unsafe\": {unsafe_}, \"wrong\": {wrong}, \"par2\": {:.1}}}",
            safe + unsafe_,
            par2
        ));
        if k == 0 {
            assert_eq!(wrong, 0, "our verdicts contradict the competition's consensus on {wrong} designs");
        }
    }
    json.push_str(&summaries.join(",\n"));
    json.push_str("\n  },\n  \"designs\": [\n");
    let cell = |says: Option<&str>, seconds: f64| match says {
        Some(v) => format!("{{\"verdict\": \"{v}\", \"seconds\": {seconds:.2}}}"),
        None => format!("{{\"verdict\": null, \"seconds\": {seconds:.2}}}"),
    };
    let body: Vec<String> = rows
        .iter()
        .map(|(name, o, a)| {
            format!(
                "    {{\"name\": \"{}\", \"consensus\": \"{}\", \"ours\": {}, \"abc\": {}}}",
                json_escape(name),
                consensus(name),
                cell(o.0, o.1),
                cell(a.0, a.1)
            )
        })
        .collect();
    json.push_str(&body.join(",\n"));
    json.push_str("\n  ]\n}\n");
    write(arena, json);
}

/// ABC's `resyn2` script, spelled out (yosys-abc has no `abc.rc` aliases).
const RESYN2: &str = "balance; rewrite; refactor; balance; rewrite; rewrite -z; balance; refactor -z; rewrite -z; balance";

fn rewrite(jobs: usize, timeout: f64, limit: usize) {
    let ours_bin = sibling("OURS_REWRITE_BIN", "rewrite_check");
    let abc_bin = env_or("ABC_BIN", "yosys-abc");
    let dir = PathBuf::from(env_or("EPFL_DIR", "external/epfl/benchmarks-master"));
    let out = std::env::temp_dir().join(format!("eda-rewrite-{}", std::process::id()));
    std::fs::create_dir_all(&out).expect("the output directory is made");
    let designs: Vec<PathBuf> = ["arithmetic", "random_control"]
        .iter()
        .flat_map(|sub| files(&dir.join(sub), ".aig", usize::MAX))
        .take(limit)
        .collect();
    let rows = in_parallel(&designs, jobs, |f| {
        let name = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let path = f.to_string_lossy().into_owned();
        let stats = |script: String| run(&abc_bin, &["-c".into(), script], timeout);
        let input = stats(format!("read_aiger {path}; strash; print_stats")).output.as_deref().and_then(ands);
        let ours = run(&ours_bin, &["--out".into(), out.to_string_lossy().into_owned(), path.clone()], timeout);
        let result = out.join(format!("{name}.aig"));
        let proven = ours.output.as_deref().is_some_and(|o| o.contains("after=") && !o.contains("not proven"));
        let ours_ands = ours.output.as_deref().and_then(|o| field(o, "after")).and_then(|a| a.parse::<u64>().ok());
        let cec = stats(format!("cec {path} {}", result.display()));
        let equal = match cec.output.as_deref() {
            Some(o) if o.contains("Networks are equivalent") => "equal",
            Some(o) if o.contains("NOT EQUIVALENT") || o.contains("are not equivalent") => "unequal",
            _ => "timeout",
        };
        assert!(!proven || equal != "unequal", "{name}: ABC's cec finds our rewritten circuit unequal to its input");
        let four = stats(format!("read_aiger {path}; strash; rewrite; rewrite; rewrite; rewrite; print_stats"));
        let resyn2 = stats(format!("read_aiger {path}; strash; {RESYN2}; print_stats"));
        eprintln!("{name}: input {input:?}, ours {ours_ands:?} (proven {proven}, ABC's cec {equal})");
        (
            name,
            input,
            (proven.then_some(ours_ands).flatten(), ours.seconds, equal),
            (four.output.as_deref().and_then(ands), four.seconds),
            (resyn2.output.as_deref().and_then(ands), resyn2.seconds),
        )
    });
    let _ = std::fs::remove_dir_all(&out);
    let opt = |x: Option<u64>| x.map(|v| v.to_string()).unwrap_or_else(|| "null".into());
    let mut json = String::from("{\n");
    json.push_str(&provenance("rewrite", "EPFL combinational suite (arithmetic, random_control)", timeout, Some(&abc_bin)));
    json.push_str("  \"engines\": [\"ours\", \"abc_rewrite_x4\", \"abc_resyn2\"],\n  \"summary\": {\n");
    let total = |pick: &dyn Fn(&(String, Option<u64>, (Option<u64>, f64, &str), (Option<u64>, f64), (Option<u64>, f64))) -> (Option<u64>, f64)| {
        let (mut ands, mut seconds, mut done) = (0u64, 0.0, 0usize);
        for row in &rows {
            let (a, s) = pick(row);
            if let Some(a) = a {
                ands += a;
                done += 1;
            }
            seconds += s;
        }
        format!("{{\"designs\": {done}, \"ands\": {ands}, \"seconds\": {seconds:.2}}}")
    };
    json.push_str(&format!(
        "    \"input\": {{\"designs\": {}, \"ands\": {}}},\n    \"ours\": {},\n    \"abc_rewrite_x4\": {},\n    \"abc_resyn2\": {}\n",
        rows.iter().filter(|r| r.1.is_some()).count(),
        rows.iter().filter_map(|r| r.1).sum::<u64>(),
        total(&|r| (r.2 .0, r.2 .1)),
        total(&|r| r.3),
        total(&|r| r.4)
    ));
    json.push_str("  },\n  \"designs\": [\n");
    let body: Vec<String> = rows
        .iter()
        .map(|(name, input, ours, four, resyn2)| {
            format!(
                "    {{\"name\": \"{}\", \"input\": {}, \"ours\": {{\"ands\": {}, \"seconds\": {:.2}, \"cec\": \"{}\"}}, \"abc_rewrite_x4\": {{\"ands\": {}, \"seconds\": {:.2}}}, \"abc_resyn2\": {{\"ands\": {}, \"seconds\": {:.2}}}}}",
                json_escape(name),
                opt(*input),
                opt(ours.0),
                ours.1,
                ours.2,
                opt(four.0),
                four.1,
                opt(resyn2.0),
                resyn2.1
            )
        })
        .collect();
    json.push_str(&body.join(",\n"));
    json.push_str("\n  ]\n}\n");
    write("rewrite", json);
}

fn count(jobs: usize, timeout: f64, limit: usize) {
    let ours_bin = sibling("OURS_COUNT_BIN", "count_check");
    let dir = PathBuf::from(env_or("MCC_DIR", "external/mcc2024/mc2024-track1-mc_competition"));
    let scratch = std::env::temp_dir().join(format!("eda-count-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).expect("the scratch directory is made");
    let formulas = files(&dir, ".cnf.xz", limit);
    let rows = in_parallel(&formulas, jobs, |f| {
        let name = f.file_name().map(|s| s.to_string_lossy().trim_end_matches(".cnf.xz").to_string()).unwrap_or_default();
        let cnf = scratch.join(format!("{name}.cnf"));
        let unpacked = Command::new("xz").arg("-dc").arg(f).output().expect("xz runs");
        std::fs::write(&cnf, &unpacked.stdout).expect("the formula is unpacked");
        let ours = run(&ours_bin, &[cnf.to_string_lossy().into_owned()], timeout);
        let _ = std::fs::remove_file(&cnf);
        let models = ours.output.as_deref().and_then(|o| field(o, "models"));
        eprintln!("{name}: {} {:.2}s", models.as_deref().map_or("—", |m| if m.len() > 20 { "…" } else { m }), ours.seconds);
        (name, models, ours.seconds)
    });
    let _ = std::fs::remove_dir_all(&scratch);
    let solved = rows.iter().filter(|r| r.1.is_some()).count();
    let par2: f64 = rows.iter().map(|r| if r.1.is_some() { r.2 } else { 2.0 * timeout }).sum();
    let mut json = String::from("{\n");
    json.push_str(&provenance("count", "the 2024 model counting competition, track 1", timeout, None));
    json.push_str(&format!(
        "  \"engines\": [\"ours\"],\n  \"summary\": {{\n    \"ours\": {{\"solved\": {solved}, \"par2\": {par2:.1}}}\n  }},\n  \"formulas\": [\n"
    ));
    let body: Vec<String> = rows
        .iter()
        .map(|(name, models, seconds)| {
            format!(
                "    {{\"name\": \"{}\", \"ours\": {{\"models\": {}, \"seconds\": {seconds:.2}}}}}",
                json_escape(name),
                models.as_ref().map(|m| format!("\"{m}\"")).unwrap_or_else(|| "null".into())
            )
        })
        .collect();
    json.push_str(&body.join(",\n"));
    json.push_str("\n  ]\n}\n");
    write("count", json);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || -> ! {
        eprintln!(
            "usage: eda_bench <mc|bmc|rewrite|count> [--jobs <n> | --cores <cpu,cpu,…>] [--timeout <seconds>] \
             [--limit <instances>]"
        );
        std::process::exit(2)
    };
    let Some(arena) = args.first() else { usage() };
    let (mut jobs, mut timeout, mut limit) = (8usize, 60.0f64, usize::MAX);
    let mut rest = args[1..].iter();
    while let Some(flag) = rest.next() {
        let value = rest.next().unwrap_or_else(|| usage());
        match flag.as_str() {
            "--jobs" => jobs = value.parse().unwrap_or_else(|_| usage()),
            "--timeout" => timeout = value.parse().unwrap_or_else(|_| usage()),
            "--limit" => limit = value.parse().unwrap_or_else(|_| usage()),
            "--cores" => {
                let cores: Vec<usize> = value.split(',').map(|c| c.parse().unwrap_or_else(|_| usage())).collect();
                jobs = cores.len();
                CORES.set(cores).unwrap_or_else(|_| usage());
            }
            _ => usage(),
        }
    }
    match arena.as_str() {
        "mc" | "bmc" => model_checking(arena, jobs, timeout, limit),
        "rewrite" => rewrite(jobs, timeout, limit),
        "count" => count(jobs, timeout, limit),
        _ => usage(),
    }
}
