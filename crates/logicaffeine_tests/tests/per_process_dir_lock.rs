//! A suite's per-process scratch directory (`common/scratch.rs`) leaves nothing behind. The process that made it
//! removes it at exit, whether its tests passed or failed. A process killed before its exit leaves its directory to
//! the next process that uses the prefix, which removes it. A directory whose process still runs, and a name or a
//! file that only shares the prefix, are never touched. The suites that make a per-process directory under the
//! temporary directory take it from the helper: a census that the hand-made ones may only leave.

#[path = "common/scratch.rs"]
mod scratch;

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// The variable that makes this test binary a scratch child, set to `<prefix> <ending>`.
const CHILD: &str = "LOGOS_SCRATCH_CHILD";

/// Re-executed with `LOGOS_SCRATCH_CHILD` = `<prefix> <ending>`, this test makes the prefix's scratch directory, writes
/// a file into it, prints `SCRATCH <directory>` and ends as asked: `pass` returns, `fail` panics, `hang` sleeps until
/// it is killed. Run any other way, it returns at once.
#[test]
fn the_scratch_child_process() {
    let Ok(how) = std::env::var(CHILD) else {
        return;
    };
    let (prefix, ending) = how.split_once(' ').expect("LOGOS_SCRATCH_CHILD is `<prefix> <ending>`");
    let d = scratch::dir(prefix);
    std::fs::write(d.join("program.cu"), "__global__ void k(){}\n").unwrap();
    println!("SCRATCH {}", d.display());
    match ending {
        "pass" => {}
        "fail" => panic!("the scratch child fails as it was asked to"),
        "hang" => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
        other => panic!("no scratch child ending `{other}`"),
    }
}

/// This binary, re-executed as a scratch child of `prefix` that ends as `ending`, its output piped.
fn child(prefix: &str, ending: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "the_scratch_child_process", "--nocapture", "--quiet"])
        .env(CHILD, format!("{prefix} {ending}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

/// The directory a scratch child printed.
fn printed(out: &str) -> PathBuf {
    out.lines()
        .find_map(|l| l.strip_prefix("SCRATCH "))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("the scratch child printed no directory:\n{out}"))
}

/// The directory `prefix`'s scratch helper names for process `pid`.
fn named(prefix: &str, pid: u32) -> PathBuf {
    std::env::temp_dir().join(format!("{prefix}_{pid}"))
}

/// A pid no process can hold: the kernel hands out pids below `pid_max`.
fn never_a_process() -> u32 {
    let max: u32 = std::fs::read_to_string("/proc/sys/kernel/pid_max").unwrap().trim().parse().unwrap();
    assert!(!scratch::running(max as i32), "pid {max}, at pid_max, runs");
    max
}

#[test]
fn a_passing_process_removes_its_scratch_directory_when_it_exits() {
    let prefix = "logos_scratch_lock_pass";
    let c = child(prefix, "pass");
    let pid = c.id();
    let out = c.wait_with_output().unwrap();
    assert!(out.status.success(), "the passing scratch child failed: {:?}", out.status);
    let d = printed(&String::from_utf8_lossy(&out.stdout));
    assert_eq!(d, named(prefix, pid), "the scratch directory is named by its prefix and its process");
    assert!(!d.exists(), "{} outlived the process that made it", d.display());
}

#[test]
fn a_failing_process_removes_its_scratch_directory_when_it_exits() {
    let prefix = "logos_scratch_lock_fail";
    let c = child(prefix, "fail");
    let pid = c.id();
    let out = c.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(101), "the failing scratch child exits as a failed test run: {:?}", out.status);
    let d = printed(&String::from_utf8_lossy(&out.stdout));
    assert_eq!(d, named(prefix, pid));
    assert!(!d.exists(), "{} outlived the failed process that made it", d.display());
}

#[test]
fn a_killed_process_leaves_its_directory_to_the_next_use_of_the_prefix() {
    let prefix = "logos_scratch_lock_killed";
    let mut c = child(prefix, "hang");
    let mut lines = BufReader::new(c.stdout.take().unwrap()).lines();
    let d = loop {
        let line = lines.next().expect("the hanging scratch child closed its output before naming its directory").unwrap();
        if let Some(path) = line.strip_prefix("SCRATCH ") {
            break PathBuf::from(path);
        }
    };
    assert_eq!(d, named(prefix, c.id()));
    assert!(d.join("program.cu").is_file(), "the scratch child wrote nothing into {}", d.display());
    c.kill().unwrap();
    c.wait().unwrap();
    assert!(d.exists(), "a killed process runs no exit handler, yet {} is gone", d.display());

    let mine = scratch::dir(prefix);
    assert!(!d.exists(), "the next use of `{prefix}` left the killed process's {}", d.display());
    assert!(mine.is_dir() && std::fs::read_dir(&mine).unwrap().next().is_none(), "{} is not made empty", mine.display());
}

#[test]
fn the_first_use_in_a_process_makes_its_directory_empty() {
    let prefix = "logos_scratch_lock_reused";
    let d = named(prefix, std::process::id());
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("sub/program.cu"), "left by a dead process that held this pid").unwrap();
    assert_eq!(scratch::dir(prefix), d);
    assert!(std::fs::read_dir(&d).unwrap().next().is_none(), "{} kept what a dead process of the same pid left", d.display());
    std::fs::write(d.join("program.cu"), "").unwrap();
    assert_eq!(scratch::dir(prefix), d);
    assert!(d.join("program.cu").is_file(), "a later use in the same process emptied {}", d.display());
}

#[test]
fn a_running_process_s_directory_and_names_that_only_share_the_prefix_stay() {
    let prefix = "logos_scratch_lock_kept";
    let root = std::env::temp_dir();
    let dead = never_a_process();
    let mut sleeper = Command::new("sleep").arg("600").spawn().unwrap();
    let live = sleeper.id();

    let orphan = named(prefix, dead);
    let living = named(prefix, live);
    let kept_dirs = [living.clone(), root.join(format!("{prefix}_x_{dead}")), root.join(format!("{prefix}_+{dead}")), root.join(format!("{prefix}{dead}"))];
    let kept_files = [root.join(format!("{prefix}_{}", dead - 1)), root.join(format!("{prefix}_{dead}.txt"))];
    for d in std::iter::once(&orphan).chain(&kept_dirs) {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join("program.cu"), "").unwrap();
    }
    for f in &kept_files {
        std::fs::write(f, "").unwrap();
    }

    let found: BTreeSet<PathBuf> = scratch::orphaned(&root, prefix).into_iter().collect();
    let mine = scratch::dir(prefix);
    let survived = |p: &Path| p.exists();
    let report = (survived(&orphan), kept_dirs.iter().map(|d| survived(d)).collect::<Vec<_>>(), kept_files.iter().map(|f| survived(f)).collect::<Vec<_>>());

    sleeper.kill().unwrap();
    sleeper.wait().unwrap();
    for d in &kept_dirs {
        let _ = std::fs::remove_dir_all(d);
    }
    for f in &kept_files {
        let _ = std::fs::remove_file(f);
    }

    assert_eq!(found, BTreeSet::from([orphan.clone()]), "the orphans of `{prefix}`");
    assert!(mine.is_dir());
    assert!(!report.0, "{} (no process) outlived the sweep", orphan.display());
    assert_eq!(report.1, vec![true; kept_dirs.len()], "a directory of a running process or a look-alike name was swept: {kept_dirs:?}");
    assert_eq!(report.2, vec![true; kept_files.len()], "a file was swept: {kept_files:?}");
}

#[test]
fn a_process_s_own_and_another_user_s_processes_count_as_running() {
    assert!(scratch::running(std::process::id() as i32));
    assert!(scratch::running(1), "init runs: as another user's process, kill refuses it with EPERM, which is not absence");
    assert!(scratch::running(0) && scratch::running(-1), "no pid at or below zero is ever swept");
    assert!(!scratch::running(never_a_process() as i32));
}

/// The suites that make a per-process directory under the temporary directory by hand and never remove a directory,
/// from before the helper. They may only leave this list, by taking their directory from `common/scratch.rs`.
const HAND_MADE: &[&str] = &[
    "c_binding_scope_lock",
    "c_bytecode_perf_lock",
    "c_family_scheduler_conformance_lock",
    "c_header_names_lock",
    "c_scheduler_port_lock",
    "cuda_forward_rope_lock",
    "cuda_knob_consumers_lock",
    "cuda_spatial_persistent_lock",
    "cuda_spatial_ring_lock",
    "optimizer_name_capture_lock",
    "original_harvest_controls",
    "qemu_wrapper_lock",
    "random_int_lock",
    "runtime_js_lock",
    "shared_map_all_tiers",
    "show_buffering_lock",
    "spatial_fabric_lowering_lock",
    "spatial_one_ir_lock",
    "ssr_requests_do_not_share_instances",
    "stepped_tier_lock",
    "wasm_float_display_lock",
    "wasm_temporal_host_lock",
];

/// Whether a suite's source makes a per-process directory under the temporary directory by hand and never removes a
/// directory: a statement that starts from `temp_dir()` and names `process::id()`, a `create_dir_all`, and no
/// `remove_dir_all` anywhere.
fn makes_its_own(src: &str) -> bool {
    let pid_named = src.match_indices("temp_dir()").any(|(i, _)| {
        let end = src[i..].find(';').map_or(src.len(), |j| i + j);
        src[i..end].contains("process::id()")
    });
    pid_named && src.contains("create_dir_all") && !src.contains("remove_dir_all")
}

#[test]
fn no_new_suite_makes_its_per_process_directory_by_hand() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut found = BTreeSet::new();
    for e in std::fs::read_dir(&tests).unwrap().flatten() {
        let path = e.path();
        if path.extension().is_some_and(|x| x == "rs") && makes_its_own(&std::fs::read_to_string(&path).unwrap()) {
            found.insert(path.file_stem().unwrap().to_string_lossy().into_owned());
        }
    }
    let listed: BTreeSet<String> = HAND_MADE.iter().map(|s| s.to_string()).collect();
    let new: Vec<_> = found.difference(&listed).collect();
    let left: Vec<_> = listed.difference(&found).collect();
    assert!(new.is_empty(), "these suites make a per-process temporary directory by hand and never remove it; take it from common/scratch.rs: {new:?}");
    assert!(left.is_empty(), "these suites no longer make their directory by hand; drop them from HAND_MADE: {left:?}");
}

#[test]
fn the_census_sees_a_hand_made_directory_and_nothing_else() {
    let hand = "let d = std::env::temp_dir()\n    .join(format!(\"logos_x_{}\", std::process::id()));\nstd::fs::create_dir_all(&d).unwrap();";
    assert!(makes_its_own(hand), "a statement split over lines");
    assert!(!makes_its_own(&format!("{hand}\nlet _ = std::fs::remove_dir_all(&d);")), "a suite that removes its directory");
    assert!(!makes_its_own("let d = scratch::dir(\"logos_x\");\nstd::fs::create_dir_all(d.join(\"sub\")).unwrap();"), "the helper's directory");
    assert!(!makes_its_own("let d = std::env::temp_dir().join(\"logos_x\");\nlet p = std::process::id();\nstd::fs::create_dir_all(&d).unwrap();"), "a pid named outside the path's statement");
    assert!(!makes_its_own("let f = std::env::temp_dir().join(format!(\"logos_x_{}.txt\", std::process::id()));\nstd::fs::write(&f, \"\").unwrap();"), "a file, no directory made");
}

#[test]
fn the_lane_suites_take_their_directories_from_the_helper() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for (suite, prefix) in [("cuda_lane_lock", "logos_cuda_lane"), ("lane_compile_lock", "logos_lane_compile"), ("model_gguf_lock", "logos_gguf_lock")] {
        let src = std::fs::read_to_string(tests.join(format!("{suite}.rs"))).unwrap();
        assert!(src.contains("#[path = \"common/scratch.rs\"]\nmod scratch;"), "{suite} does not include the scratch helper");
        assert!(src.contains(&format!("scratch::dir(\"{prefix}\")")), "{suite} does not take `{prefix}` from the helper");
        assert!(!src.contains("process::id()"), "{suite} names its own process id");
    }
}
