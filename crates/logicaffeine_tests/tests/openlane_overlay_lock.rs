//! openlane_overlay_lock — the OpenLane every block is hardened with is the fetched OpenLane plus the changes this
//! repo tracks, and nothing else.
//!
//! WHY. OpenLane is fetched into the ignored `tools/`, and the flow runs that copy. This project's changes to it —
//! the SAT antenna jog router and its unit tests, the Magic antenna ECO, the post-diode upsize, the diode fallback,
//! the repaired `lappend` sites in `cts.tcl` and `resizer_routing_design.tcl`, the routing and XOR steps they are
//! wired into — once existed only there, so no clean checkout could rebuild any hardened block, and every lock
//! reading them failed on a fresh clone. They live in `scripts/openlane-overlay/` over the OpenLane commit named in
//! its `BASE`, and `scripts/openlane-overlay/apply.sh` lays them over the fetched OpenLane before every run.
//!
//! What this file locks:
//!   1. the overlay names a commit, and every script that launches the flow's image launches that commit;
//!   2. wherever OpenLane is installed, it is checked out at that commit and holds every overlay file byte for byte;
//!   3. every script that starts an OpenLane flow lays the overlay over it first — but for one run: a control
//!      (`OL_STOCK=1`), which runs only the comparative bench's stock OpenLane, only when that tree is stamped with
//!      the overlay's own base, and is refused before any flow otherwise;
//!   4. the overlay carries OpenLane's Apache-2.0 LICENSE, and its NOTICE declares every overlay file exactly once,
//!      as modified from the base (and marked so at its head, Apache-2.0 §4(b)) or as added by this project.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(openlane_overlay_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn base() -> String {
    std::fs::read_to_string(root().join("scripts/openlane-overlay/BASE"))
        .expect("the overlay's BASE")
        .trim()
        .to_string()
}

fn overlay_files() -> Vec<PathBuf> {
    let dir = root().join("scripts/openlane-overlay");
    let mut out = Vec::new();
    let mut stack = vec![dir.join("scripts")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d)
            .unwrap_or_else(|e| panic!("read {}: {e}", d.display()))
            .flatten()
        {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == "__pycache__") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(&dir).unwrap().to_path_buf());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn the_overlay_is_over_the_commit_the_flow_image_runs() {
    let base = base();
    assert!(
        base.len() == 40 && base.chars().all(|c| c.is_ascii_hexdigit()),
        "BASE is not a commit: {base:?}"
    );
    let files = overlay_files();
    assert!(
        files.len() >= 15,
        "only {} overlay files — the walk is not working",
        files.len()
    );
    for script in [
        "scripts/anchor-ir-drop.sh",
        "scripts/check-wpe-psram-timing.sh",
        "scripts/remeasure-power.sh",
    ] {
        let text = std::fs::read_to_string(root().join(script))
            .unwrap_or_else(|e| panic!("read {script}: {e}"));
        let pins: Vec<&str> = text
            .match_indices("openlane:")
            .map(|(at, _)| {
                text.get(at + "openlane:".len()..at + "openlane:".len() + 40)
                    .unwrap_or("")
            })
            .collect();
        assert!(
            !pins.is_empty(),
            "{script} launches no pinned OpenLane image"
        );
        assert!(
            pins.iter().all(|p| *p == base),
            "{script} launches OpenLane {pins:?}, the overlay is over {base}"
        );
    }
    let submission = std::fs::read_to_string(root().join("scripts/caravel-submission.sh")).unwrap();
    assert!(
        submission.contains(&format!("The-OpenROAD-Project/OpenLane,{base}")),
        "a submission names the OpenLane it was built with"
    );
}

/// Where OpenLane is installed, it is the overlay's base with every overlay file in place. A clone without the
/// machine-local EDA tree records a counted skip.
#[test]
fn the_installed_openlane_is_the_base_plus_the_overlay() {
    if !common::require_tool("eda-tools") {
        return;
    }
    let ol = root().join("tools/OpenLane");
    let Ok(head) = std::fs::read_to_string(ol.join(".git/HEAD")) else {
        return;
    };
    let head = head.trim();
    let commit = match head.strip_prefix("ref: ") {
        Some(r) => std::fs::read_to_string(ol.join(".git").join(r))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| {
                std::fs::read_to_string(ol.join(".git/packed-refs"))
                    .unwrap_or_default()
                    .lines()
                    .find_map(|l| l.strip_suffix(&format!(" {r}")).map(str::to_string))
                    .unwrap_or_default()
            }),
        None => head.to_string(),
    };
    assert_eq!(
        commit,
        base(),
        "tools/OpenLane is checked out at another commit than the overlay is written over"
    );
    let mut drift = Vec::new();
    for rel in overlay_files() {
        let tracked = std::fs::read(root().join("scripts/openlane-overlay").join(&rel)).unwrap();
        if std::fs::read(ol.join(&rel)).ok().as_deref() != Some(&tracked[..]) {
            drift.push(format!("  {}", rel.display()));
        }
    }
    assert!(
        drift.is_empty(),
        "the installed OpenLane differs from the tracked overlay — run scripts/openlane-overlay/apply.sh, or, if the \
         installed copy holds a change worth keeping, move it into scripts/openlane-overlay first:\n{}",
        drift.join("\n")
    );
}

/// The launch lines of a flow script that start `make quick_run` with no overlay application before them.
fn launches_without_the_overlay(text: &str) -> Vec<usize> {
    let mut applied = false;
    let mut bare = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with('#') {
            continue;
        }
        if code.starts_with("bash \"$ROOT/scripts/openlane-overlay/apply.sh\"") {
            applied = true;
        }
        if code.contains("make quick_run") && !applied {
            bare.push(n + 1);
        }
    }
    bare
}

#[test]
fn every_flow_launch_lays_the_overlay_over_openlane_first() {
    let mut scripts = Vec::new();
    let mut stack = vec![root().join("scripts")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.ends_with("openlane-overlay") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "sh") {
                scripts.push(p);
            }
        }
    }
    let mut launchers = Vec::new();
    let mut bare = Vec::new();
    for p in &scripts {
        let Ok(text) = std::fs::read_to_string(p) else {
            continue;
        };
        if !text
            .lines()
            .any(|l| !l.trim_start().starts_with('#') && l.contains("make quick_run"))
        {
            continue;
        }
        let rel = p.strip_prefix(root()).unwrap().display().to_string();
        for n in launches_without_the_overlay(&text) {
            bare.push(format!("  {rel}:{n}"));
        }
        launchers.push(rel);
    }
    launchers.sort();
    for known in [
        "scripts/caravel-harden.sh",
        "scripts/caravel-macro-harden.sh",
        "scripts/openlane-run.sh",
    ] {
        assert!(
            launchers.iter().any(|l| l == known),
            "{known} no longer starts a flow, or the scan no longer finds it: {launchers:?}"
        );
    }
    assert!(
        bare.is_empty(),
        "these flow launches run OpenLane without the repo's overlay — add \
         `bash \"$ROOT/scripts/openlane-overlay/apply.sh\"` before them:\n{}",
        bare.join("\n")
    );
}

#[test]
fn a_launch_before_the_overlay_or_under_a_commented_apply_is_caught() {
    let applied = "set -e\nbash \"$ROOT/scripts/openlane-overlay/apply.sh\"\n( cd \"$OL\" && make quick_run QUICK_RUN_DESIGN=x )\n";
    assert!(launches_without_the_overlay(applied).is_empty());
    let late = "( cd \"$OL\" && make quick_run QUICK_RUN_DESIGN=x )\nbash \"$ROOT/scripts/openlane-overlay/apply.sh\"\n";
    assert_eq!(launches_without_the_overlay(late), vec![1]);
    let commented = "# bash \"$ROOT/scripts/openlane-overlay/apply.sh\"\n( cd \"$OL\" && make quick_run QUICK_RUN_DESIGN=x )\n";
    assert_eq!(launches_without_the_overlay(commented), vec![2]);
    let missing = "( cd \"$OL\" && make quick_run QUICK_RUN_DESIGN=x )\n";
    assert_eq!(launches_without_the_overlay(missing), vec![1]);
}

/// NOTICE's two lists: the files modified from the base, and the files this project added.
fn notice_lists(notice: &str) -> (Vec<String>, Vec<String>) {
    let (mut modified, mut added) = (Vec::new(), Vec::new());
    let mut section: Option<&mut Vec<String>> = None;
    for line in notice.lines() {
        if line.starts_with("Modified from") {
            section = Some(&mut modified);
        } else if line.starts_with("Added by") {
            section = Some(&mut added);
        } else if let (Some(list), Some(entry)) = (section.as_mut(), line.strip_prefix("  ")) {
            list.push(entry.trim().to_string());
        } else if line.trim().is_empty() {
            section = None;
        }
    }
    (modified, added)
}

const MODIFIED_MARK: &str = "Modified by the LOGOS project from OpenLane";

#[test]
fn the_overlay_is_licensed_and_its_notice_declares_every_file() {
    let dir = root().join("scripts/openlane-overlay");
    let license = std::fs::read_to_string(dir.join("LICENSE"))
        .expect("the overlay carries OpenLane's LICENSE");
    assert!(
        license.contains("Apache License") && license.contains("Version 2.0, January 2004"),
        "scripts/openlane-overlay/LICENSE is not the Apache License 2.0 OpenLane is distributed under"
    );
    if common::require_tool("eda-tools") {
        if let Ok(installed) = std::fs::read_to_string(root().join("tools/OpenLane/LICENSE")) {
            assert_eq!(
                license, installed,
                "the overlay's LICENSE differs from the fetched OpenLane's"
            );
        }
    }
    let notice = std::fs::read_to_string(dir.join("NOTICE")).expect("the overlay carries a NOTICE");
    let base = base();
    assert!(
        notice.contains("The-OpenROAD-Project/OpenLane"),
        "NOTICE does not name the upstream"
    );
    assert!(
        notice.contains(&base),
        "NOTICE does not name the commit the overlay is over ({base})"
    );
    let (modified, added) = notice_lists(&notice);
    assert!(
        !modified.is_empty() && !added.is_empty(),
        "NOTICE lists nothing: {modified:?} {added:?}"
    );

    let files: Vec<String> = overlay_files()
        .iter()
        .map(|p| p.display().to_string())
        .collect();
    let mut declared: Vec<&String> = modified.iter().chain(&added).collect();
    declared.sort();
    let twice: Vec<_> = declared
        .windows(2)
        .filter(|w| w[0] == w[1])
        .map(|w| w[0])
        .collect();
    assert!(
        twice.is_empty(),
        "NOTICE declares these more than once: {twice:?}"
    );
    let undeclared: Vec<_> = files.iter().filter(|f| !declared.contains(f)).collect();
    assert!(
        undeclared.is_empty(),
        "overlay files NOTICE does not declare as modified or added: {undeclared:?}"
    );
    let phantom: Vec<_> = declared.iter().filter(|d| !files.contains(d)).collect();
    assert!(
        phantom.is_empty(),
        "NOTICE declares files the overlay does not hold: {phantom:?}"
    );

    for f in &files {
        let text = std::fs::read_to_string(dir.join(f)).unwrap();
        let head: Vec<&str> = text.lines().take(3).collect();
        let marked = head
            .iter()
            .any(|l| l.contains(MODIFIED_MARK) && l.contains(&base));
        if modified.contains(f) {
            assert!(
                marked,
                "{f} is modified from OpenLane but does not say so at its head (Apache-2.0 §4(b))"
            );
        } else {
            assert!(
                !text.contains(MODIFIED_MARK),
                "{f} is declared as this project's own but is marked as modified OpenLane"
            );
        }
    }
}

/// The runner's one flow without the overlay: in a sandbox tree (no OpenLane image, so nothing past the checks can
/// start), a control is refused before any flow when the stock tree is missing, stamped with another commit, or
/// asked for by any value but 1 — and the overlay step is skipped only under `OL_STOCK=1`.
#[test]
fn only_a_control_on_the_stock_tree_at_the_overlays_base_runs_without_the_overlay() {
    let runner = std::fs::read_to_string(root().join("scripts/openlane-run.sh")).unwrap();
    let skip = runner.lines().position(|l| l.trim() == "if [ \"$OL_STOCK\" != 1 ]; then").expect("the overlay step's guard");
    assert!(
        runner.lines().nth(skip + 1).map_or(false, |l| l.trim() == "bash \"$ROOT/scripts/openlane-overlay/apply.sh\" \"$OL\""),
        "the overlay is laid on every run but a control, over the tree the run flows in"
    );
    assert!(runner.contains("  OL=\"$ROOT/tools/qor/openlane-stock\"\n"), "a control runs the bench's stock tree and no other");
    let sb = std::env::temp_dir().join(format!("overlay-control-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sb);
    for d in ["scripts/openlane-overlay", "benchmarks/silicon", "tools/qor/openlane-stock/designs", "tools/OpenLane/designs"] {
        std::fs::create_dir_all(sb.join(d)).unwrap();
    }
    std::fs::write(sb.join("scripts/openlane-run.sh"), &runner).unwrap();
    std::fs::write(sb.join("scripts/openlane-overlay/BASE"), format!("{}\n", base())).unwrap();
    std::fs::write(sb.join("benchmarks/silicon/toy.v"), "module toy(input clk, output reg q);\n  always @(posedge clk) q <= ~q;\nendmodule\n").unwrap();
    let run = |stock: &str| {
        let out = std::process::Command::new("bash")
            .arg(sb.join("scripts/openlane-run.sh"))
            .args(["toy", "clk", "10"])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", sb.as_os_str())
            .env("OL_STOCK", stock)
            .output()
            .unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).to_string())
    };
    let stamp = sb.join("tools/qor/openlane-stock/.logos-base");
    let (rc, err) = run("1");
    assert!(rc == 1 && err.contains("no stock OpenLane at the overlay's base"), "an unstamped tree: {rc} {err}");
    std::fs::write(&stamp, "0123456789abcdef0123456789abcdef01234567\n").unwrap();
    let (rc, err) = run("1");
    assert!(rc == 1 && err.contains("no stock OpenLane at the overlay's base"), "another commit: {rc} {err}");
    let (rc, err) = run("yes");
    assert!(rc == 1 && err.contains("OL_STOCK is 1"), "a value but 1 or 0: {rc} {err}");
    std::fs::write(&stamp, format!("{}\n", base())).unwrap();
    let (_, err) = run("1");
    assert!(!err.contains("no stock OpenLane"), "the stamped tree passes the check: {err}");
    let _ = std::fs::remove_dir_all(&sb);
}

/// `apply.sh` lays the overlay over the tree it is given — `tools/OpenLane` when it is given none — and refuses one at
/// another commit than the overlay's base before it writes anything: a checkout's commit is its `.git/HEAD`, a release
/// unpacked by `scripts/qor/fetch-qor-tools.py` is stamped with its own in `.logos-base`, and a tree with neither has
/// no commit to check.
#[test]
fn the_overlay_is_laid_over_the_tree_it_is_given_at_the_overlays_base() {
    let sb = std::env::temp_dir().join(format!("overlay-tree-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sb);
    let ov = sb.join("scripts/openlane-overlay");
    std::fs::create_dir_all(ov.join("scripts/sub")).unwrap();
    std::fs::copy(root().join("scripts/openlane-overlay/apply.sh"), ov.join("apply.sh")).unwrap();
    std::fs::write(ov.join("BASE"), format!("{}\n", base())).unwrap();
    std::fs::write(ov.join("scripts/sub/x.tcl"), "puts overlaid\n").unwrap();
    let tree = |rel: &str| {
        let t = sb.join(rel);
        std::fs::create_dir_all(t.join("scripts")).unwrap();
        t
    };
    let apply = |to: Option<&std::path::Path>| {
        let mut c = std::process::Command::new("bash");
        c.arg(ov.join("apply.sh"));
        if let Some(t) = to {
            c.arg(t);
        }
        let o = c.output().unwrap();
        (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stderr).into_owned())
    };
    let laid = |t: &std::path::Path| std::fs::read_to_string(t.join("scripts/sub/x.tcl")).ok();
    let other = "0123456789abcdef0123456789abcdef01234567\n";

    let shared = tree("tools/OpenLane");
    std::fs::create_dir_all(shared.join(".git/refs/heads")).unwrap();
    std::fs::write(shared.join(".git/HEAD"), "ref: refs/heads/superstable\n").unwrap();
    std::fs::write(shared.join(".git/refs/heads/superstable"), other).unwrap();
    let (rc, err) = apply(None);
    assert!(rc == 2 && err.contains("is at 0123456789abcdef"), "a checkout at another commit: {rc} {err}");
    assert_eq!(laid(&shared), None, "refused before anything is laid");
    std::fs::write(shared.join(".git/refs/heads/superstable"), format!("{}\n", base())).unwrap();
    let (rc, err) = apply(None);
    assert!(rc == 0 && laid(&shared).as_deref() == Some("puts overlaid\n"), "no tree given is tools/OpenLane: {rc} {err}");

    let release = tree("tools/qor/openlane-bench");
    let (rc, err) = apply(Some(&release));
    assert!(rc == 2 && err.contains("no commit"), "neither a checkout nor a stamped release: {rc} {err}");
    std::fs::write(release.join(".logos-base"), other).unwrap();
    let (rc, err) = apply(Some(&release));
    assert!(rc == 2 && err.contains("is at 0123456789abcdef"), "a release of another commit: {rc} {err}");
    assert_eq!(laid(&release), None, "refused before anything is laid");
    std::fs::write(release.join(".logos-base"), format!("{}\n", base())).unwrap();
    let (rc, err) = apply(Some(&release));
    assert!(rc == 0 && laid(&release).as_deref() == Some("puts overlaid\n"), "the release at the base: {rc} {err}");

    let (rc, err) = apply(Some(&sb.join("absent")));
    assert!(rc == 1 && err.contains("no OpenLane at"), "a tree that is not there: {rc} {err}");
    let _ = std::fs::remove_dir_all(&sb);
}

/// A comparative-bench run (`OL_BENCH=1`) flows in the bench's own tree, `tools/qor/openlane-bench`: OpenLane as
/// released at the overlay's base (`scripts/qor/fetch-qor-tools.py openlane-bench`), the overlay laid over it before
/// the flow — the flow every overlaid run goes through, in a tree whose designs are never `tools/OpenLane`'s, every
/// one of which `rtl_provenance_lock` traces to RTL the repository tracks. In a sandbox tree (no OpenLane image, so
/// nothing past the checks can start), a bench run is refused before it stages anything when its tree is missing or
/// stamped with another commit, `OL_BENCH` is 1 or 0 and never beside `OL_STOCK=1`; a stamped tree takes the staged
/// design, and a run without `OL_BENCH` stages in `tools/OpenLane`.
#[test]
fn a_bench_run_flows_in_the_bench_tree_at_the_overlays_base() {
    let runner = std::fs::read_to_string(root().join("scripts/openlane-run.sh")).unwrap();
    assert!(runner.contains("  OL=\"$ROOT/tools/qor/openlane-bench\"\n"), "a bench run flows in the bench's tree and no other");
    let sb = std::env::temp_dir().join(format!("overlay-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&sb);
    for d in ["scripts/openlane-overlay", "benchmarks/silicon", "tools/qor/openlane-bench/designs", "tools/OpenLane/designs"] {
        std::fs::create_dir_all(sb.join(d)).unwrap();
    }
    std::fs::write(sb.join("scripts/openlane-run.sh"), &runner).unwrap();
    std::fs::write(sb.join("scripts/openlane-overlay/BASE"), format!("{}\n", base())).unwrap();
    std::fs::write(sb.join("benchmarks/silicon/toy.v"), "module toy(input clk, output reg q);\n  always @(posedge clk) q <= ~q;\nendmodule\n").unwrap();
    let run = |env: &[(&str, &str)]| {
        let mut c = std::process::Command::new("bash");
        c.arg(sb.join("scripts/openlane-run.sh"))
            .args(["toy", "clk", "10"])
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", sb.as_os_str());
        for (k, v) in env {
            c.env(k, v);
        }
        let out = c.output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).to_string())
    };
    let design = |tree: &str| sb.join(tree).join("designs/logos_toy");
    let staged = |tree: &str| design(tree).join("src/toy.v").is_file();
    let (bench, shared) = ("tools/qor/openlane-bench", "tools/OpenLane");
    let stamp = sb.join(bench).join(".logos-base");

    let (rc, err) = run(&[("OL_BENCH", "1")]);
    assert!(rc == 1 && err.contains("no bench OpenLane at the overlay's base"), "an unstamped tree: {rc} {err}");
    std::fs::write(&stamp, "0123456789abcdef0123456789abcdef01234567\n").unwrap();
    let (rc, err) = run(&[("OL_BENCH", "1")]);
    assert!(rc == 1 && err.contains("no bench OpenLane at the overlay's base"), "another commit: {rc} {err}");
    assert!(!staged(bench) && !staged(shared), "refused before anything is staged");

    std::fs::write(&stamp, format!("{}\n", base())).unwrap();
    let (_, err) = run(&[("OL_BENCH", "1")]);
    assert!(staged(bench), "the stamped bench tree takes the design: {err}");
    assert!(!design(shared).exists(), "nothing of a bench run in tools/OpenLane");
    std::fs::remove_dir_all(design(bench)).unwrap();
    let (_, err) = run(&[("OL_BENCH", "0")]);
    assert!(staged(shared) && !design(bench).exists(), "any other run stages in tools/OpenLane: {err}");
    std::fs::remove_dir_all(design(shared)).unwrap();

    for (env, why) in [
        (vec![("OL_BENCH", "yes")], "OL_BENCH is 1"),
        (vec![("OL_BENCH", "1"), ("OL_STOCK", "1")], "never both"),
    ] {
        let (rc, err) = run(&env);
        assert!(rc == 1 && err.contains(why), "{env:?}: exit {rc}, want a refusal naming {why:?}\n{err}");
        assert!(!design(bench).exists() && !design(shared).exists(), "{env:?}: refused before anything is staged");
    }
    let _ = std::fs::remove_dir_all(&sb);
}
