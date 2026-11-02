//! ★ LOCK — `randomInt(lo, hi)` is a uniform draw from `lo..=hi` on every host, and a sealed world
//! decides it on every in-process tier. ★
//!
//! A test is a pure function of `(program, seed)`: under a sealed world the tree-walker and the
//! bytecode VM draw from the world's seeded stream, so the same seed gives the same draws on both
//! and another seed gives others. A real host draws from `crypto.getRandomValues` — the browser
//! host (`runtime.js`) and the node shim (`web_host::wasm_host_shim`), both run here under real V8
//! against a compiled module — rejecting the words that would make a value likelier than another.
//! Every host refuses a range whose low bound is above its high bound, by name.

mod common;
use common::require_tool;

use std::process::Command;

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome, RunOutcome};
use logicaffeine_compile::web_host::{browser_host_js, wasm_host_shim};
use logicaffeine_system::testworld::{self, Capabilities, TestWorld};

type Runner = fn(&str) -> RunOutcome;
const RUNNERS: [(&str, Runner); 2] = [("tree-walker", tw_outcome), ("vm", vm_outcome)];

const DRAWS: &str = "## Main\nRepeat for i from 1 to 8:\n    Show randomInt(1, 1000000000).\n";

/// `src` on `run` inside a world sealed with `seed`.
fn sealed(run: Runner, src: &str, seed: u64) -> RunOutcome {
    testworld::install(TestWorld::new(Capabilities::default(), seed));
    let out = run(src);
    testworld::uninstall();
    out
}

#[test]
fn a_sealed_world_decides_every_in_process_draw() {
    let by_tier: Vec<(&str, String, String)> = RUNNERS
        .iter()
        .map(|(tier, run)| {
            let a = sealed(*run, DRAWS, 7);
            let b = sealed(*run, DRAWS, 8);
            assert_eq!(a.error, None, "{tier}: the draws run");
            assert_eq!(b.error, None, "{tier}: the draws run");
            (
                *tier,
                a.output.trim_end().to_string(),
                b.output.trim_end().to_string(),
            )
        })
        .collect();
    let (_, seven, eight) = &by_tier[0];
    assert_eq!(seven.lines().count(), 8, "eight draws shown: {seven}");
    assert_ne!(seven, eight, "another seed draws another stream");
    for (tier, s7, s8) in &by_tier[1..] {
        assert_eq!(
            s7, seven,
            "{tier} draws what the tree-walker draws from seed 7"
        );
        assert_eq!(
            s8, eight,
            "{tier} draws what the tree-walker draws from seed 8"
        );
    }
    let again = sealed(tw_outcome, DRAWS, 7);
    assert_eq!(
        again.output.trim_end(),
        seven,
        "the same seed draws the same stream again"
    );
}

#[test]
fn an_empty_range_is_refused_by_name_on_every_in_process_tier() {
    for (tier, run) in RUNNERS {
        let out = run("## Main\nShow randomInt(5, 1).\n");
        let error = out.error.unwrap_or_else(|| {
            panic!(
                "{tier}: randomInt(5, 1) is refused, not drawn: {}",
                out.output
            )
        });
        assert!(
            error.contains("randomInt(5, 1): the low bound is above the high bound"),
            "{tier}: the refusal names the call and why: {error}"
        );
    }
}

/// Three thousand draws from `0..=2`, tallied, plus a one-value range and a range as wide as the
/// non-negative Ints: `zeros ones twos strays single wide-in-range`, one line each.
const TALLY: &str = "\
## Main
Let mutable zeros be 0.
Let mutable ones be 0.
Let mutable twos be 0.
Let mutable strays be 0.
Repeat for i from 1 to 3000:
    Let r be randomInt(0, 2).
    If r is 0:
        Set zeros to zeros + 1.
    If r is 1:
        Set ones to ones + 1.
    If r is 2:
        Set twos to twos + 1.
    If r is less than 0 or r is greater than 2:
        Set strays to strays + 1.
Show zeros.
Show ones.
Show twos.
Show strays.
Show randomInt(42, 42).
Let wide be randomInt(0, 9223372036854775807).
Show wide is at least 0.
";

/// The tally a host printed, checked: every draw in range, each value near a third, the one-value
/// range its value, the wide draw in range.
fn check_tally(host: &str, stdout: &str) {
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 6, "{host}: six lines:\n{stdout}");
    let counts: Vec<i64> = lines[..4]
        .iter()
        .map(|l| {
            l.trim()
                .parse()
                .unwrap_or_else(|_| panic!("{host}: a count: {l}"))
        })
        .collect();
    assert_eq!(counts[3], 0, "{host}: every draw lies in 0..=2:\n{stdout}");
    assert_eq!(
        counts[..3].iter().sum::<i64>(),
        3000,
        "{host}: every draw is one of the three:\n{stdout}"
    );
    for (value, n) in counts[..3].iter().enumerate() {
        assert!(
            (850..=1150).contains(n),
            "{host}: {value} was drawn {n} times in 3000 — not a uniform third:\n{stdout}"
        );
    }
    assert_eq!(
        lines[4].trim(),
        "42",
        "{host}: a one-value range draws its value"
    );
    assert_eq!(
        lines[5].trim(),
        "true",
        "{host}: a draw from a range as wide as the non-negative Ints lies in it"
    );
}

fn node(dir: &std::path::Path, file: &str) -> (String, String, bool) {
    let out = Command::new("node")
        .arg(dir.join(file))
        .output()
        .expect("node preflighted present but failed to spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("logos-random-int-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Globals enough for `runtime.js` to load under `node` for a program that never touches the
/// document.
const MIN_GLOBALS: &str = r#"
globalThis.document = { body: {}, addEventListener() {} };
globalThis.window = { addEventListener() {}, history: { pushState() {}, replaceState() {} }, location: { pathname: '/', search: '' } };
"#;

/// `src` compiled and run by the browser host under node.
fn in_browser_host(src: &str, tag: &str) -> (String, String, bool) {
    let wasm = compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let csv = wasm
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let from = "const bytes = await (await fetch(WASM_URL)).arrayBuffer();";
    let host = browser_host_js("__random_probe__");
    assert!(
        host.contains(from),
        "the wasm loader shape in browser_host_js drifted"
    );
    let dir = scratch(tag);
    std::fs::write(
        dir.join("probe.mjs"),
        format!(
            "{MIN_GLOBALS}\n{}\n",
            host.replace(from, &format!("const bytes = new Uint8Array([{csv}]);"))
        ),
    )
    .expect("write probe");
    node(&dir, "probe.mjs")
}

/// `src` compiled and run by the node shim.
fn in_node_shim(src: &str, tag: &str) -> (String, String, bool) {
    let wasm = compile_to_wasm(src).unwrap_or_else(|e| panic!("must compile: {e:?}\n{src}"));
    let dir = scratch(tag);
    std::fs::write(dir.join("probe.wasm"), &wasm).expect("write wasm");
    std::fs::write(dir.join("probe.mjs"), wasm_host_shim("probe")).expect("write shim");
    node(&dir, "probe.mjs")
}

#[test]
fn the_browser_host_draws_uniformly_within_the_bounds() {
    if !require_tool("node") {
        return;
    }
    let (stdout, stderr, ok) = in_browser_host(TALLY, "browser");
    assert!(
        ok,
        "the browser host runs the draws:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    check_tally("the browser host", &stdout);
}

#[test]
fn the_node_shim_draws_uniformly_within_the_bounds() {
    if !require_tool("node") {
        return;
    }
    let (stdout, stderr, ok) = in_node_shim(TALLY, "shim");
    assert!(
        ok,
        "the node shim runs the draws:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    check_tally("the node shim", &stdout);
}

#[test]
fn a_real_host_refuses_an_empty_range_by_name() {
    if !require_tool("node") {
        return;
    }
    let src = "## Main\nShow randomInt(5, 1).\n";
    for (host, (stdout, stderr, _)) in [
        ("the browser host", in_browser_host(src, "empty-browser")),
        ("the node shim", in_node_shim(src, "empty-shim")),
    ] {
        let all = format!("{stdout}\n{stderr}");
        assert!(
            stdout.trim().is_empty(),
            "{host}: randomInt(5, 1) is refused, not drawn:\n{all}"
        );
        assert!(
            all.contains("randomInt(5, 1): the low bound is above the high bound"),
            "{host}: the refusal names the call and why:\n{all}"
        );
    }
}

/// Six thousand draws from a span of 3·2⁶², counted below its first third. A draw taken modulo the
/// span lands in that third half the time (2⁶⁴ mod 3·2⁶² = 2⁶², so its values come up twice); a
/// uniform draw lands there a third of the time.
const BIAS: &str = "\
## Main
Let mutable low be 0.
Repeat for i from 1 to 6000:
    Let r be randomInt(-6917529027641081856, 6917529027641081855).
    If r is less than -2305843009213693952:
        Set low to low + 1.
Show low.
";

fn check_unbiased(host: &str, stdout: &str) {
    let low: i64 = stdout
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("{host}: a count: {stdout}"));
    assert!((1800..=2200).contains(&low), "{host}: {low} of 6000 draws fell in the first third of the span — a uniform draw puts 2000 there, a modulo-biased one 3000");
}

#[test]
fn a_sealed_draw_carries_no_modulo_bias() {
    for (tier, run) in RUNNERS {
        let out = sealed(run, BIAS, 11);
        assert_eq!(out.error, None, "{tier}: the draws run");
        check_unbiased(tier, &out.output);
    }
}

#[test]
fn a_sealed_draw_spans_every_int() {
    testworld::install(TestWorld::new(Capabilities::default(), 3));
    let draws: Vec<i64> = (0..64)
        .map(|_| logicaffeine_system::random::randomInt(i64::MIN, i64::MAX))
        .collect();
    let near_ends = (
        logicaffeine_system::random::randomInt(i64::MAX - 1, i64::MAX),
        logicaffeine_system::random::randomInt(i64::MIN, i64::MIN + 1),
    );
    testworld::uninstall();
    assert!(
        draws.iter().any(|d| *d < 0) && draws.iter().any(|d| *d > 0),
        "the whole Int range is drawn from, both signs: {draws:?}"
    );
    assert!(
        (i64::MAX - 1..=i64::MAX).contains(&near_ends.0),
        "{near_ends:?}"
    );
    assert!(
        (i64::MIN..=i64::MIN + 1).contains(&near_ends.1),
        "{near_ends:?}"
    );
}

#[test]
fn a_real_host_draw_carries_no_modulo_bias() {
    if !require_tool("node") {
        return;
    }
    for (host, (stdout, stderr, ok)) in [
        ("the browser host", in_browser_host(BIAS, "bias-browser")),
        ("the node shim", in_node_shim(BIAS, "bias-shim")),
    ] {
        assert!(
            ok,
            "{host} runs the draws:\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        check_unbiased(host, &stdout);
    }
}
