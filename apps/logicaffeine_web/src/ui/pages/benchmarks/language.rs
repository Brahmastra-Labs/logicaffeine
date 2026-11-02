//! Labels, formatting and the headline figures of the Programming Language tab.

use super::*;

pub(super) const GITHUB_REPO: &str = "https://github.com/Brahmastra-Labs/logicaffeine";

pub(super) fn format_time(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else if ms >= 1.0 {
        format!("{:.1}ms", ms)
    } else {
        format!("{:.0}\u{00b5}s", ms * 1000.0)
    }
}

pub(super) fn format_timeout(timeout_ms: f64) -> String {
    let mins = timeout_ms / 60_000.0;
    if mins >= 1.0 {
        format!(">{:.0}min", mins)
    } else {
        format!(">{:.0}s", timeout_ms / 1000.0)
    }
}

pub(super) fn tier_label(tier: &str) -> &'static str {
    match tier {
        "logos" => "LOGOS",
        "systems" => "Systems",
        "managed" => "Managed",
        "interpreted" => "Interpreted",
        "transpiled" => "Transpiled",
        _ => "Other",
    }
}

pub(super) fn lang_label(lang_id: &str) -> &'static str {
    match lang_id {
        "c" => "C",
        "cpp" => "C++",
        "rust" => "Rust",
        "zig" => "Zig",
        "logos_release" => "LOGOS",
        "go" => "Go",
        "java" => "Java",
        "js" => "JavaScript",
        "logos_interp" => "LOGOS (eager)",
        "logos_tiered" => "LOGOS (tiered)",
        "logos_aot" => "LOGOS (AOT-native)",
        "python" => "Python",
        "ruby" => "Ruby",
        "nim" => "Nim",
        _ => "Other",
    }
}

pub(super) fn lang_ext(lang_id: &str) -> &'static str {
    match lang_id {
        "c" => "main.c",
        "cpp" => "main.cpp",
        "rust" => "main.rs",
        "zig" => "main.zig",
        "go" => "main.go",
        "java" => "Main.java",
        "js" => "main.js",
        "python" => "main.py",
        "ruby" => "main.rb",
        "nim" => "main.nim",
        _ => "main",
    }
}

pub(super) fn get_source(sources: &BenchSources, lang_id: &str) -> &'static str {
    match lang_id {
        "c" => sources.c,
        "cpp" => sources.cpp,
        "rust" => sources.rust,
        "zig" => sources.zig,
        "go" => sources.go,
        "java" => sources.java,
        "js" => sources.js,
        "python" => sources.python,
        "ruby" => sources.ruby,
        "nim" => sources.nim,
        _ => "",
    }
}

// Timings at a benchmark's effective reference size (reference_size if it has
// data, else the largest benchmarked size).
pub(super) fn effective_ref(b: &Benchmark) -> Option<&HashMap<String, TimingResult>> {
    if let Some(t) = b.scaling.get(b.reference_size.as_str()) {
        return Some(t);
    }
    b.sizes.iter().rev().find_map(|s| b.scaling.get(s))
}

// Node sits near its ~30ms V8 startup floor when its time is dominated by
// process startup rather than compute; such interpreter benchmarks are flagged
// and kept out of the headline so they don't flatter the interpreter.
pub(super) fn node_floored(t: &TimingResult) -> bool {
    t.mean_ms < 60.0
}

pub(super) fn interp_ref<'a>(b: &'a InterpBenchmark) -> Option<&'a HashMap<String, TimingResult>> {
    b.scaling
        .get(b.reference_size.as_str())
        .or_else(|| b.scaling.values().next())
}

/// Whether an interpreter benchmark counts toward the engine comparison: Node ran off its startup floor, and the
/// LOGOS optimizer did not change the algorithm (`collapsed` — the ids whose collapse note says it did), so both
/// engines ran the same program.
pub(super) fn interp_counts(
    b: &InterpBenchmark,
    collapsed: &std::collections::HashSet<&str>,
) -> bool {
    !collapsed.contains(b.id.as_str())
        && interp_ref(b)
            .and_then(|t| Some(!node_floored(t.get("js")?) && t.contains_key("logos_interp")))
            .unwrap_or(false)
}

// LOGOS interpreter speed vs V8 (Node time / interpreter time), geomean over the interpreter benchmarks that count
// (`interp_counts`). Higher = faster.
pub(super) fn interp_speed_vs_v8(
    data: &InterpData,
    collapsed: &std::collections::HashSet<&str>,
) -> f64 {
    let mut log_sum = 0.0_f64;
    let mut n = 0u32;
    for b in &data.benchmarks {
        if !interp_counts(b, collapsed) {
            continue;
        }
        if let Some(t) = interp_ref(b) {
            if let (Some(js), Some(lg)) = (t.get("js"), t.get("logos_interp")) {
                if js.mean_ms > 0.0 && lg.mean_ms > 0.0 && !node_floored(js) {
                    log_sum += (js.mean_ms / lg.mean_ms).ln();
                    n += 1;
                }
            }
        }
    }
    if n > 0 {
        (log_sum / n as f64).exp()
    } else {
        0.0
    }
}

// LOGOS AOT-native (warm) speed vs V8 (Node time / AOT-native time), geomean over
// the benchmarks that carry a `logos_aot` row. Higher = faster. Returns 0 when no
// run has emitted the AOT-native tier yet, so the headline card stays hidden until
// a real AOT-native run regenerates the data.
pub(super) fn aot_speed_vs_v8(data: &InterpData) -> f64 {
    let mut log_sum = 0.0_f64;
    let mut n = 0u32;
    for b in &data.benchmarks {
        if let Some(t) = interp_ref(b) {
            if let (Some(js), Some(lg)) = (t.get("js"), t.get("logos_aot")) {
                if js.mean_ms > 0.0 && lg.mean_ms > 0.0 && !node_floored(js) {
                    log_sum += (js.mean_ms / lg.mean_ms).ln();
                    n += 1;
                }
            }
        }
    }
    if n > 0 {
        (log_sum / n as f64).exp()
    } else {
        0.0
    }
}

pub(super) fn fmt_n(n: f64) -> String {
    if n >= 1.0e9 {
        format!("{:.0}B", n / 1.0e9)
    } else if n >= 1.0e6 {
        format!("{:.0}M", n / 1.0e6)
    } else if n >= 1.0e3 {
        format!("{:.0}k", n / 1.0e3)
    } else {
        format!("{:.0}", n)
    }
}
