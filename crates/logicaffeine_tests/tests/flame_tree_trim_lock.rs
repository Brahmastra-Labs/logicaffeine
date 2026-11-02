//! The flamegraph the /benchmarks page draws is `scripts/dev/flamefold.py --tree-json`: a `perf script` profile folded
//! into a tree small enough to ship to a browser. Trimming must never lie about where time went.
//!
//! Driven here on a synthetic `perf script` with known weights: totals are conserved (every node's weight is at least
//! the sum of its children's, and the root is the whole profile), a stack is re-rooted at the program's entry and one
//! that never reaches it is kept under `(outside the program)`, runs of `[unknown]` merge, three hundred tiny leaves fold
//! into one `(other)`, a hundred-deep recursion stops at the depth cap, and the node cap holds.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test flame_tree_trim_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::fmt::Write as _;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// One `perf script` sample: a header carrying the period, then the frames leaf-first.
fn sample(out: &mut String, period: u64, leaf_first: &[&str]) {
    let _ = writeln!(out, "bench 4242 100.000001:     {period} cycles:u: ");
    for (i, frame) in leaf_first.iter().enumerate() {
        let _ = writeln!(
            out,
            "\t    {:x} {frame}+0x10 (/tmp/bench)",
            0x5555_0000_u64 + i as u64
        );
    }
    out.push('\n');
}

type Node = (usize, u64, Vec<serde_json::Value>);

fn node(v: &serde_json::Value) -> Node {
    let a = v.as_array().expect("a node is [index, weight, children]");
    (
        a[0].as_u64().unwrap() as usize,
        a[1].as_u64().unwrap(),
        a[2].as_array().unwrap().clone(),
    )
}

#[test]
fn trimming_keeps_every_weight_and_every_bound() {
    let mut profile = String::new();
    sample(
        &mut profile,
        600_000,
        &["leaf_a", "work", "main", "__libc_start_main"],
    );
    sample(
        &mut profile,
        300_000,
        &[
            "leaf_b",
            "[unknown]",
            "[unknown]",
            "[unknown]",
            "work",
            "main",
            "__libc_start_main",
        ],
    );
    let tiny: Vec<String> = (0..300).map(|i| format!("tiny_{i:03}")).collect();
    for name in &tiny {
        sample(&mut profile, 300, &[name, "main", "__libc_start_main"]);
    }
    let deep: Vec<String> = (0..100).map(|i| format!("recur_{i:03}")).collect();
    let mut deep_stack: Vec<&str> = deep.iter().map(String::as_str).collect();
    deep_stack.extend(["main", "__libc_start_main"]);
    sample(&mut profile, 1_000, &deep_stack);
    sample(&mut profile, 9_000, &["_dl_start", "_start"]);
    let expected_total: u64 = 600_000 + 300_000 + 300 * 300 + 1_000 + 9_000;

    let dir = tempfile::TempDir::new().unwrap();
    let path = |f: &str| dir.path().join(f);
    let mut child = std::process::Command::new("python3")
        .arg(repo_root().join("scripts/dev/flamefold.py"))
        .args([
            "--folded",
            path("p.folded").to_str().unwrap(),
            "--summary",
            path("p.json").to_str().unwrap(),
        ])
        .args([
            "--tree-json",
            path("p.tree.json").to_str().unwrap(),
            "--root",
            "main",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("python3 runs flamefold.py");
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(profile.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success(), "flamefold.py failed");

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path("p.tree.json")).unwrap()).unwrap();
    let frames: Vec<String> = doc["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        doc["total"].as_u64(),
        Some(expected_total),
        "the tree's total is the whole profile"
    );
    let root = node(&doc["tree"]);
    assert_eq!((frames[root.0].as_str(), root.1), ("all", expected_total));

    let (mut nodes, mut max_depth) = (0usize, 0usize);
    let mut stack = vec![(root.clone(), 0usize, String::new())];
    while let Some(((idx, weight, kids), depth, parent)) = stack.pop() {
        nodes += 1;
        max_depth = max_depth.max(depth);
        let name = &frames[idx];
        let children: Vec<Node> = kids.iter().map(node).collect();
        let sum: u64 = children.iter().map(|c| c.1).sum();
        assert!(
            sum <= weight,
            "{name}: children weigh {sum}, more than the node's {weight}"
        );
        assert!(
            !(name == "[unknown]" && parent == "[unknown]"),
            "a run of [unknown] frames must merge into one"
        );
        for c in children {
            stack.push((c, depth + 1, name.clone()));
        }
    }
    assert!(nodes <= 400, "the node cap holds: {nodes}");
    assert!(max_depth <= 49, "the depth cap holds: {max_depth}");

    let top: Vec<(String, u64)> = root
        .2
        .iter()
        .map(node)
        .map(|(i, w, _)| (frames[i].clone(), w))
        .collect();
    assert!(
        top.iter()
            .any(|(n, w)| n == "main" && *w == expected_total - 9_000),
        "re-rooted at main: {top:?}"
    );
    assert!(
        top.iter()
            .any(|(n, w)| n == "(outside the program)" && *w == 9_000),
        "startup outside main is kept: {top:?}"
    );
    assert!(
        !frames.iter().any(|f| f == "__libc_start_main"),
        "frames above the root are not drawn"
    );
    assert!(
        frames.iter().any(|f| f == "(other)"),
        "the tiny leaves fold into (other)"
    );
    assert!(
        !frames.iter().any(|f| f.starts_with("tiny_")),
        "no tiny leaf is drawn on its own"
    );
    let summary: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path("p.json")).unwrap()).unwrap();
    assert_eq!(
        doc["buckets"]["compute_pct"], summary["compute_pct"],
        "the tree carries the same buckets as the summary"
    );
}

#[test]
fn the_tree_names_the_frames_with_the_most_self_time() {
    let mut profile = String::new();
    sample(&mut profile, 700, &["hot_leaf", "work", "main"]);
    sample(&mut profile, 200, &["warm_leaf", "work", "main"]);
    sample(&mut profile, 100, &["hot_leaf", "other_caller", "main"]);
    sample(&mut profile, 50, &["work", "main"]);
    let dir = tempfile::TempDir::new().unwrap();
    let path = |f: &str| dir.path().join(f);
    let mut child = std::process::Command::new("python3")
        .arg(repo_root().join("scripts/dev/flamefold.py"))
        .args([
            "--folded",
            path("p.folded").to_str().unwrap(),
            "--summary",
            path("p.json").to_str().unwrap(),
        ])
        .args([
            "--tree-json",
            path("p.tree.json").to_str().unwrap(),
            "--root",
            "main",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(profile.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path("p.tree.json")).unwrap()).unwrap();
    let top: Vec<(String, f64)> = doc["top_self"]
        .as_array()
        .expect("top_self is a list")
        .iter()
        .map(|e| {
            (
                e["frame"].as_str().unwrap().to_string(),
                e["self_pct"].as_f64().unwrap(),
            )
        })
        .collect();
    let pct = |w: f64| (w / 1050.0 * 100.0 * 1000.0).round() / 1000.0;
    assert_eq!(
        top,
        [
            ("hot_leaf".to_string(), pct(800.0)),
            ("warm_leaf".to_string(), pct(200.0)),
            ("work".to_string(), pct(50.0))
        ],
        "self time summed per frame over every stack it is the leaf of, largest first",
    );
}

#[test]
fn every_frame_carries_the_kind_the_page_colours_it_by() {
    let mut profile = String::new();
    sample(&mut profile, 400, &["work", "main"]);
    sample(&mut profile, 300, &["malloc", "work", "main"]);
    sample(&mut profile, 200, &["__printf_chk", "main"]);
    sample(&mut profile, 100, &["core::fmt::write", "main"]);
    sample(&mut profile, 100, &["logos::qs [jit]", "[unknown]"]);
    sample(&mut profile, 50, &["_dl_start", "_start"]);
    let dir = tempfile::TempDir::new().unwrap();
    let path = |f: &str| dir.path().join(f);
    let mut child = std::process::Command::new("python3")
        .arg(repo_root().join("scripts/dev/flamefold.py"))
        .args([
            "--folded",
            path("p.folded").to_str().unwrap(),
            "--summary",
            path("p.json").to_str().unwrap(),
        ])
        .args([
            "--tree-json",
            path("p.tree.json").to_str().unwrap(),
            "--root",
            "main",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(profile.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path("p.tree.json")).unwrap()).unwrap();
    let frames = doc["frames"].as_array().unwrap();
    let kinds = doc["kinds"].as_array().expect("a kind per frame");
    assert_eq!(frames.len(), kinds.len(), "kinds run parallel to frames");
    let kind_of = |name: &str| {
        let i = frames
            .iter()
            .position(|f| f == name)
            .unwrap_or_else(|| panic!("{name} is a frame: {frames:?}"));
        kinds[i].as_str().unwrap().to_string()
    };
    assert_eq!(kind_of("all"), "synthetic");
    assert_eq!(kind_of("main"), "compute");
    assert_eq!(kind_of("work"), "compute");
    assert_eq!(kind_of("malloc"), "alloc");
    assert_eq!(kind_of("__printf_chk"), "output");
    assert_eq!(kind_of("core::fmt::write"), "fmt");
    assert_eq!(kind_of("logos::qs [jit]"), "jit");
    assert_eq!(kind_of("(JIT code, callers not unwound)"), "synthetic");
    assert_eq!(kind_of("(outside the program)"), "synthetic");
}

#[test]
fn rust_frames_are_demangled_and_unhashed() {
    let mut profile = String::new();
    sample(
        &mut profile,
        1_000,
        &["_ZN4core3ptr13drop_in_place17h0123456789abcdefE", "main"],
    );
    let dir = tempfile::TempDir::new().unwrap();
    let path = |f: &str| dir.path().join(f);
    let mut child = std::process::Command::new("python3")
        .arg(repo_root().join("scripts/dev/flamefold.py"))
        .args([
            "--folded",
            path("p.folded").to_str().unwrap(),
            "--summary",
            path("p.json").to_str().unwrap(),
        ])
        .args([
            "--tree-json",
            path("p.tree.json").to_str().unwrap(),
            "--root",
            "main",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(profile.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path("p.tree.json")).unwrap()).unwrap();
    let frames: Vec<&str> = doc["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(
        frames.contains(&"core::ptr::drop_in_place"),
        "demangled with the ::h hash stripped: {frames:?}"
    );
}
