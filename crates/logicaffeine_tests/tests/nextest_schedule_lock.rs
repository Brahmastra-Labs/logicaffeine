//! The timing locks run first, alone, and nothing that starts early holds them back.
//!
//! A `cpu-timing` member needs every test thread (`threads-required = "num-test-threads"`), so nextest starts it only
//! once nothing else of the run is in flight, and holds back every test queued behind it while it waits. If a heavy
//! that starts early (a fuzzer, a simulator chain) shares the timing group's priority, it can start first; the next
//! timing lock then waits for it to end, and so does the rest of the run, with the box idle for the heavy's minutes.
//! So in every profile the timing group's members require every thread and outrank every other override that sets a
//! priority.

use std::path::{Path, PathBuf};

fn nextest_config() -> toml::Table {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    std::fs::read_to_string(root.join(".config/nextest.toml"))
        .unwrap()
        .parse()
        .unwrap()
}

/// Every profile's overrides, as (profile, override table).
fn overrides(config: &toml::Table) -> Vec<(String, toml::Table)> {
    let mut out = Vec::new();
    let profiles = config
        .get("profile")
        .and_then(|p| p.as_table())
        .expect("nextest.toml declares profiles");
    for (name, profile) in profiles {
        for o in profile
            .get("overrides")
            .and_then(|o| o.as_array())
            .into_iter()
            .flatten()
        {
            out.push((
                name.clone(),
                o.as_table().expect("an override is a table").clone(),
            ));
        }
    }
    out
}

fn is_timing(o: &toml::Table) -> bool {
    o.get("test-group").and_then(|g| g.as_str()) == Some("cpu-timing")
}

#[test]
fn every_timing_lock_runs_alone_and_ahead_of_every_early_heavy() {
    let config = nextest_config();
    let group = config
        .get("test-groups")
        .and_then(|g| g.get("cpu-timing"))
        .expect("nextest.toml declares the cpu-timing group");
    assert_eq!(
        group.get("max-threads").and_then(|m| m.as_integer()),
        Some(1),
        "the timing locks never race each other"
    );
    let all = overrides(&config);
    let mut problems = Vec::new();
    let mut profiles_with_timing = 0;
    for profile in all
        .iter()
        .map(|(p, _)| p.clone())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let mine: Vec<&toml::Table> = all
            .iter()
            .filter(|(p, _)| *p == profile)
            .map(|(_, o)| o)
            .collect();
        let timing: Vec<&&toml::Table> = mine.iter().filter(|o| is_timing(o)).collect();
        if timing.is_empty() {
            continue;
        }
        profiles_with_timing += 1;
        for o in &timing {
            if o.get("threads-required").and_then(|t| t.as_str()) != Some("num-test-threads") {
                problems.push(format!(
                    "[{profile}] a cpu-timing override does not require every test thread"
                ));
            }
        }
        let floor = timing
            .iter()
            .map(|o| o.get("priority").and_then(|p| p.as_integer()).unwrap_or(0))
            .min()
            .unwrap();
        for o in mine.iter().filter(|o| !is_timing(o)) {
            if let Some(p) = o.get("priority").and_then(|p| p.as_integer()) {
                if p >= floor {
                    problems.push(format!(
                        "[{profile}] `{}` starts at priority {p}, not below the timing group's {floor}",
                        o.get("filter").and_then(|f| f.as_str()).unwrap_or("?")
                    ));
                }
            }
        }
    }
    assert!(
        profiles_with_timing >= 3,
        "the timing group is configured in default, ci and full"
    );
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The three ways a test source probes the GPU device: the shared seam (`common::require_gpu`, and
/// `common::require_tool` asked for `gpu`) and the device node `/dev/nvidia0` as a path literal. Each is spelled in
/// halves, so this file names the probes without carrying one.
const GPU_PROBES: [&str; 3] = [
    concat!("require_", "gpu()"),
    concat!("require_tool(\"", "gpu\")"),
    concat!("\"/dev/", "nvidia0\""),
];

/// Whether a test file's own source probes the GPU device. A file that probes the device runs on it.
fn probes_the_gpu(src: &str) -> bool {
    GPU_PROBES.iter().any(|probe| src.contains(probe))
}

/// Every test binary of `logicaffeine-tests` whose source probes the GPU device.
fn gpu_binaries() -> std::collections::BTreeSet<String> {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    std::fs::read_dir(&tests)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .filter(|p| probes_the_gpu(&std::fs::read_to_string(p).unwrap()))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect()
}

/// What a `cuda-gpu` filter selects, read from its accepted shapes: `package(logicaffeine-tests) & <term>` or
/// `package(logicaffeine-tests) & (<term> | <term> | …)`, each term `binary(/^prefix/)` or `binary(/^(a|b|…)$/)`, as
/// (prefixes, names). Any other term is refused, so the census judges the filter it reads rather than a guess at it.
fn gpu_selection(filter: &str) -> (Vec<String>, std::collections::BTreeSet<String>) {
    let terms = filter
        .strip_prefix("package(logicaffeine-tests) & ")
        .unwrap_or_else(|| panic!("the cuda-gpu filter is not `package(logicaffeine-tests) & …`: {filter}"));
    let body = terms
        .strip_prefix('(')
        .and_then(|t| t.strip_suffix(')'))
        .unwrap_or(terms);
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    let (mut prefixes, mut names) = (Vec::new(), std::collections::BTreeSet::new());
    for term in body.split(" | ") {
        if let Some(list) = term.strip_prefix("binary(/^(").and_then(|t| t.strip_suffix(")$/)")) {
            for name in list.split('|') {
                assert!(word(name), "the cuda-gpu filter names `{name}`, not a binary");
                names.insert(name.to_string());
            }
        } else if let Some(prefix) = term.strip_prefix("binary(/^").and_then(|t| t.strip_suffix("/)")) {
            assert!(word(prefix), "the cuda-gpu filter's prefix `{prefix}` is not a plain name prefix");
            prefixes.push(prefix.to_string());
        } else {
            panic!("the cuda-gpu filter has a term this census cannot read: `{term}`");
        }
    }
    (prefixes, names)
}

/// One GPU, one test at a time, and only if every test that reaches the device takes its turn: a GPU test outside
/// the group runs beside a member, and a timing lock among them measures the contention instead of its kernel. So
/// in every profile the `cuda-gpu` group selects exactly the binaries whose sources probe the device — none
/// missing, and no name that no longer probes it.
#[test]
fn every_test_that_reaches_the_gpu_takes_its_turn_on_the_one_device() {
    let config = nextest_config();
    let group = config
        .get("test-groups")
        .and_then(|g| g.get("cuda-gpu"))
        .expect("nextest.toml declares the cuda-gpu group");
    assert_eq!(
        group.get("max-threads").and_then(|m| m.as_integer()),
        Some(1),
        "the GPU runs one test at a time"
    );
    let gpu = gpu_binaries();
    assert!(
        gpu.contains("sass_optimize_gemm_gpu_lock") && gpu.contains("cuda_wmma_gpu_lock"),
        "the census reads the device probes of the test sources: {gpu:?}"
    );
    let all = overrides(&config);
    let mut problems = Vec::new();
    for profile in ["default", "ci", "full"] {
        let filters: Vec<&str> = all
            .iter()
            .filter(|(p, o)| p == profile && o.get("test-group").and_then(|g| g.as_str()) == Some("cuda-gpu"))
            .map(|(_, o)| o.get("filter").and_then(|f| f.as_str()).expect("an override has a filter"))
            .collect();
        let [filter] = filters[..] else {
            problems.push(format!("[{profile}] has {} cuda-gpu overrides, not one", filters.len()));
            continue;
        };
        let (prefixes, names) = gpu_selection(filter);
        let selected = |b: &str| prefixes.iter().any(|p| b.starts_with(p.as_str())) || names.contains(b);
        for b in gpu.iter().filter(|b| !selected(b)) {
            problems.push(format!("[{profile}] `{b}` probes the GPU but runs outside the cuda-gpu group"));
        }
        for n in names.iter().filter(|n| !gpu.contains(*n)) {
            problems.push(format!("[{profile}] the cuda-gpu group names `{n}`, which no test source probing the GPU is"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
