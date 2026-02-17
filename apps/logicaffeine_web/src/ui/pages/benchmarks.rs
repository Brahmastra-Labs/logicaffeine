use std::collections::HashMap;
use std::sync::LazyLock;
use dioxus::prelude::*;
use serde::Deserialize;
use crate::ui::components::main_nav::{MainNav, ActivePage};
use crate::ui::components::footer::Footer;
use crate::ui::seo::{JsonLdMultiple, PageHead, organization_schema, breadcrumb_schema, webpage_schema, BreadcrumbItem, pages as seo_pages};

#[derive(Deserialize)]
struct BenchmarkData {
    metadata: Metadata,
    languages: Vec<Language>,
    benchmarks: Vec<Benchmark>,
    summary: SummaryData,
}

#[derive(Deserialize)]
struct Metadata {
    date: String,
    commit: String,
    logos_version: String,
    cpu: String,
    os: String,
    versions: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Language {
    id: String,
    label: String,
    color: String,
    tier: String,
}

#[derive(Deserialize)]
struct Benchmark {
    id: String,
    name: String,
    description: String,
    reference_size: String,
    sizes: Vec<String>,
    logos_source: String,
    generated_rust: String,
    scaling: HashMap<String, HashMap<String, TimingResult>>,
    compilation: HashMap<String, CompilationResult>,
}

#[derive(Deserialize, Clone)]
struct TimingResult {
    mean_ms: f64,
    median_ms: f64,
    stddev_ms: f64,
    min_ms: f64,
    max_ms: f64,
    cv: f64,
    runs: u32,
}

#[derive(Deserialize, Clone)]
struct CompilationResult {
    mean_ms: f64,
    stddev_ms: f64,
}

#[derive(Deserialize)]
struct SummaryData {
    geometric_mean_speedup_vs_c: HashMap<String, f64>,
}

static BENCH_DATA: LazyLock<BenchmarkData> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../../benchmarks/results/latest.json")).unwrap()
});

struct BenchSources {
    c: &'static str,
    cpp: &'static str,
    rust: &'static str,
    zig: &'static str,
    go: &'static str,
    java: &'static str,
    js: &'static str,
    python: &'static str,
    ruby: &'static str,
    nim: &'static str,
    logos: &'static str,
}

macro_rules! bench_sources {
    ($name:literal) => {
        BenchSources {
            c: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.c")),
            cpp: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.cpp")),
            rust: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.rs")),
            zig: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.zig")),
            go: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.go")),
            java: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/Main.java")),
            js: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.js")),
            python: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.py")),
            ruby: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.rb")),
            nim: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.nim")),
            logos: include_str!(concat!("../../../../../benchmarks/programs/", $name, "/main.lg")),
        }
    };
}

static SOURCES: LazyLock<[BenchSources; 32]> = LazyLock::new(|| [
    // Recursion & Function Calls
    bench_sources!("fib"),
    bench_sources!("ackermann"),
    bench_sources!("nqueens"),
    // Sorting
    bench_sources!("bubble_sort"),
    bench_sources!("mergesort"),
    bench_sources!("quicksort"),
    bench_sources!("counting_sort"),
    bench_sources!("heap_sort"),
    // Floating Point
    bench_sources!("nbody"),
    bench_sources!("mandelbrot"),
    bench_sources!("spectral_norm"),
    bench_sources!("pi_leibniz"),
    // Integer Mathematics
    bench_sources!("gcd"),
    bench_sources!("collatz"),
    bench_sources!("primes"),
    // Array Patterns
    bench_sources!("sieve"),
    bench_sources!("matrix_mult"),
    bench_sources!("prefix_sum"),
    bench_sources!("array_reverse"),
    bench_sources!("array_fill"),
    // Hash Maps & Lookup
    bench_sources!("collect"),
    bench_sources!("two_sum"),
    bench_sources!("histogram"),
    // Dynamic Programming
    bench_sources!("knapsack"),
    bench_sources!("coins"),
    // Combinatorial
    bench_sources!("fannkuch"),
    // Memory & Allocation
    bench_sources!("strings"),
    bench_sources!("binary_trees"),
    // Loop Overhead & Control Flow
    bench_sources!("loop_sum"),
    bench_sources!("fib_iterative"),
    bench_sources!("graph_bfs"),
    bench_sources!("string_search"),
]);

const GITHUB_REPO: &str = "https://github.com/Brahmastra-Labs/logicaffeine";

fn format_time(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else if ms >= 1.0 {
        format!("{:.1}ms", ms)
    } else {
        format!("{:.0}\u{00b5}s", ms * 1000.0)
    }
}

fn format_n(n: &str) -> String {
    let bytes: Vec<u8> = n.bytes().collect();
    let len = bytes.len();
    if len <= 3 { return n.to_string(); }
    let mut result = String::with_capacity(len + len / 3);
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(b as char);
    }
    result
}

fn tier_label(tier: &str) -> &'static str {
    match tier {
        "systems" => "Systems",
        "managed" => "Managed",
        "interpreted" => "Interpreted",
        "transpiled" => "Transpiled",
        "logos" => "LOGOS",
        _ => "Other",
    }
}

fn lang_color(lang_id: &str) -> &'static str {
    match lang_id {
        "c" => "#555555",
        "cpp" => "#f34b7d",
        "rust" => "#dea584",
        "zig" => "#f7a41d",
        "go" => "#00ADD8",
        "java" => "#b07219",
        "js" => "#f7df1e",
        "python" => "#3776ab",
        "ruby" => "#cc342d",
        "nim" => "#ffe953",
        _ => "#94a3b8",
    }
}

fn lang_label(lang_id: &str) -> &'static str {
    match lang_id {
        "c" => "C",
        "cpp" => "C++",
        "rust" => "Rust",
        "zig" => "Zig",
        "go" => "Go",
        "java" => "Java",
        "js" => "JavaScript",
        "python" => "Python",
        "ruby" => "Ruby",
        "nim" => "Nim",
        _ => "Other",
    }
}

fn lang_ext(lang_id: &str) -> &'static str {
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

fn compiler_label(key: &str) -> &'static str {
    match key {
        "gcc_-o2" => "gcc -O2",
        "g++_-o2" => "g++ -O2",
        "rustc_-o" => "rustc -O",
        "go_build" => "go build",
        "javac" => "javac",
        "nim_c" => "nim c -d:release",
        "zig_build-exe" => "zig build-exe -O ReleaseFast",
        "largo_build" => "largo build",
        "largo_build_--release" => "largo build --release",
        _ => "unknown",
    }
}

fn get_source(sources: &BenchSources, lang_id: &str) -> &'static str {
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

const BENCHMARKS_STYLE: &str = r#"
.bench-container {
    min-height: 100vh;
    background: linear-gradient(135deg, #070a12 0%, #0b1022 50%, #070a12 100%);
    color: #e5e7eb;
    font-family: ui-sans-serif, system-ui, -apple-system, Segoe UI, Roboto, sans-serif;
}

.bench-hero {
    text-align: center;
    padding: 60px 20px 20px;
    max-width: 800px;
    margin: 0 auto;
}

.bench-hero h1 {
    font-size: 42px;
    font-weight: 800;
    letter-spacing: -1px;
    background: linear-gradient(180deg, #ffffff 0%, rgba(229,231,235,0.78) 100%);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin-bottom: 12px;
}

.bench-hero p {
    color: rgba(229,231,235,0.72);
    font-size: 18px;
    line-height: 1.6;
    margin-bottom: 20px;
}

.bench-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    justify-content: center;
    margin-bottom: 16px;
}

.bench-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
    padding: 5px 12px;
    border-radius: 16px;
    background: rgba(255,255,255,0.05);
    border: 1px solid rgba(255,255,255,0.10);
    color: rgba(229,231,235,0.72);
    text-decoration: none;
    transition: all 0.2s ease;
}

.bench-pill:hover {
    background: rgba(255,255,255,0.08);
    border-color: rgba(255,255,255,0.15);
}

.bench-pill strong {
    color: #e5e7eb;
}

.bench-pill.link {
    cursor: pointer;
    color: #00d4ff;
}

.bench-section-nav {
    display: flex;
    justify-content: center;
    gap: 6px;
    padding: 12px 20px;
    margin-bottom: 24px;
    flex-wrap: wrap;
}

.bench-section-nav a {
    font-size: 12px;
    font-weight: 500;
    padding: 5px 14px;
    border-radius: 16px;
    background: rgba(255,255,255,0.04);
    border: 1px solid rgba(255,255,255,0.08);
    color: rgba(229,231,235,0.6);
    text-decoration: none;
    transition: all 0.2s ease;
    backdrop-filter: blur(12px);
}

.bench-section-nav a:hover {
    background: rgba(255,255,255,0.08);
    color: #e5e7eb;
    border-color: rgba(255,255,255,0.15);
}

.bench-content {
    max-width: 1000px;
    margin: 0 auto;
    padding: 0 20px 80px;
}

.bench-summary {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 16px;
    margin-bottom: 40px;
}

.bench-summary-card {
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 16px;
    padding: 24px;
    text-align: center;
    transition: all 0.2s ease;
}

.bench-summary-card:hover {
    background: rgba(255,255,255,0.05);
    border-color: rgba(255,255,255,0.12);
}

.bench-summary-value {
    font-size: 36px;
    font-weight: 800;
    letter-spacing: -1px;
    margin-bottom: 4px;
}

.bench-summary-value.cyan { color: #00d4ff; }
.bench-summary-value.green { color: #22c55e; }
.bench-summary-value.purple { color: #a78bfa; }

.bench-summary-label {
    font-size: 13px;
    color: rgba(229,231,235,0.56);
}

.bench-tabs {
    display: flex;
    gap: 6px;
    margin-bottom: 24px;
    flex-wrap: wrap;
}

.bench-tab {
    padding: 8px 16px;
    border-radius: 8px;
    border: 1px solid rgba(255,255,255,0.1);
    background: rgba(255,255,255,0.03);
    color: #94a3b8;
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    transition: all 0.2s ease;
}

.bench-tab:hover {
    background: rgba(255,255,255,0.08);
    color: #e8e8e8;
}

.bench-tab.active {
    background: linear-gradient(135deg, rgba(0,212,255,0.25), rgba(129,140,248,0.25));
    color: #00d4ff;
    border-color: rgba(0,212,255,0.4);
}

.bench-section {
    background: rgba(255,255,255,0.03);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 16px;
    padding: 24px;
    margin-bottom: 24px;
}

.bench-section-title {
    font-size: 18px;
    font-weight: 700;
    color: #fff;
    margin-bottom: 6px;
}

.bench-section-desc {
    font-size: 13px;
    color: rgba(229,231,235,0.56);
    margin-bottom: 20px;
}

.bench-chart {
    display: flex;
    flex-direction: column;
    gap: 6px;
}

.bench-tier-label {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.4);
    margin-top: 10px;
    margin-bottom: 2px;
}

.bench-bar-row {
    display: flex;
    align-items: center;
    gap: 12px;
}

.bench-bar-label {
    width: 130px;
    text-align: right;
    font-size: 13px;
    font-weight: 500;
    color: rgba(229,231,235,0.8);
    flex-shrink: 0;
}

.bench-bar-track {
    flex: 1;
    height: 28px;
    background: rgba(255,255,255,0.02);
    border-radius: 6px;
    overflow: hidden;
    position: relative;
}

.bench-bar-fill {
    height: 100%;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    padding-right: 8px;
    min-width: 60px;
    transition: width 0.4s ease;
}

.bench-bar-fill.logos-highlight {
    box-shadow: 0 0 16px rgba(0,212,255,0.35);
}

.bench-bar-time {
    font-size: 11px;
    font-weight: 600;
    color: rgba(0,0,0,0.8);
    white-space: nowrap;
}

.bench-bar-time-outside {
    font-size: 11px;
    font-weight: 600;
    color: rgba(229,231,235,0.6);
    white-space: nowrap;
    margin-left: 8px;
    flex-shrink: 0;
}

.bench-source {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
}

.bench-source-panel {
    display: flex;
    flex-direction: column;
}

.bench-source-header {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    padding: 8px 12px;
    border-radius: 8px 8px 0 0;
    border: 1px solid rgba(255,255,255,0.08);
    border-bottom: none;
}

.bench-source-header.logos {
    background: rgba(0,212,255,0.1);
    color: #00d4ff;
}

.bench-source-header.rust {
    background: rgba(222,165,132,0.1);
    color: #dea584;
}

.bench-source-code {
    background: rgba(0,0,0,0.3);
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 0 0 8px 8px;
    padding: 16px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 12px;
    line-height: 1.5;
    color: #e8e8e8;
    white-space: pre-wrap;
    overflow-x: auto;
    flex: 1;
}

/* Collapsible pattern */
.bench-collapsible-btn {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 10px 0;
    background: none;
    border: none;
    border-top: 1px solid rgba(255,255,255,0.06);
    color: rgba(229,231,235,0.6);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: color 0.2s ease;
    margin-top: 16px;
    text-align: left;
}

.bench-collapsible-btn:hover {
    color: #e5e7eb;
}

.bench-collapsible-chevron {
    display: inline-block;
    font-size: 10px;
    transition: transform 0.2s ease;
}

.bench-collapsible-chevron.open {
    transform: rotate(90deg);
}

.bench-collapsible-body {
    overflow: hidden;
    max-height: 0;
    transition: max-height 0.3s ease;
}

.bench-collapsible-body.open {
    max-height: 8000px;
}

/* Stats table */
.bench-stats-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
    margin-top: 12px;
}

.bench-stats-table th {
    text-align: left;
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.5);
    padding: 6px 8px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.bench-stats-table td {
    padding: 6px 8px;
    color: rgba(229,231,235,0.7);
    border-bottom: 1px solid rgba(255,255,255,0.04);
    font-family: 'SF Mono', 'Fira Code', monospace;
    font-size: 11px;
}

.bench-stats-table tr.highlight td {
    color: #00d4ff;
    background: rgba(0,212,255,0.05);
}

/* Language source collapsible */
.bench-lang-collapsible {
    border: 1px solid rgba(255,255,255,0.06);
    border-radius: 8px;
    margin-bottom: 8px;
    overflow: hidden;
}

.bench-lang-header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    background: rgba(255,255,255,0.02);
    cursor: pointer;
    transition: background 0.2s ease;
}

.bench-lang-header:hover {
    background: rgba(255,255,255,0.05);
}

.bench-lang-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
}

.bench-lang-name {
    font-size: 13px;
    font-weight: 500;
    color: #e5e7eb;
    flex: 1;
}

.bench-lang-version {
    font-size: 11px;
    color: rgba(229,231,235,0.4);
}

.bench-lang-link {
    font-size: 11px;
    color: #00d4ff;
    text-decoration: none;
    opacity: 0.7;
    transition: opacity 0.2s ease;
}

.bench-lang-link:hover {
    opacity: 1;
    text-decoration: underline;
}

.bench-lang-code {
    background: rgba(0,0,0,0.3);
    padding: 16px;
    font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
    font-size: 12px;
    line-height: 1.5;
    color: #e8e8e8;
    white-space: pre-wrap;
    overflow-x: auto;
    max-height: 0;
    transition: max-height 0.3s ease, padding 0.3s ease;
    padding-top: 0;
    padding-bottom: 0;
}

.bench-lang-code.open {
    max-height: 4000px;
    padding-top: 16px;
    padding-bottom: 16px;
}

/* Compile chart */
.bench-compile-table {
    width: 100%;
    border-collapse: collapse;
}

.bench-compile-table th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.5);
    padding: 8px 12px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.bench-compile-table td {
    padding: 8px 12px;
    font-size: 13px;
    color: rgba(229,231,235,0.8);
    border-bottom: 1px solid rgba(255,255,255,0.04);
}

.bench-compile-table tr:last-child td {
    border-bottom: none;
}

.bench-compile-table .compiler-name {
    font-weight: 500;
    color: #e5e7eb;
}

.bench-compile-table tr.highlight td {
    color: #00d4ff;
}

/* Methodology */
.bench-methodology {
    color: rgba(229,231,235,0.56);
    font-size: 13px;
    line-height: 1.7;
}

.bench-methodology ul {
    padding-left: 20px;
    margin: 8px 0;
}

.bench-methodology li {
    margin-bottom: 4px;
}

.bench-methodology a {
    color: #00d4ff;
    text-decoration: none;
}

.bench-methodology a:hover {
    text-decoration: underline;
}

.bench-methodology h3 {
    font-size: 14px;
    font-weight: 600;
    color: #e5e7eb;
    margin: 16px 0 8px;
}

.bench-version-table {
    width: 100%;
    border-collapse: collapse;
    margin: 8px 0 16px;
}

.bench-version-table th {
    text-align: left;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.5px;
    color: rgba(229,231,235,0.5);
    padding: 6px 10px;
    border-bottom: 1px solid rgba(255,255,255,0.08);
}

.bench-version-table td {
    padding: 6px 10px;
    font-size: 12px;
    color: rgba(229,231,235,0.7);
    border-bottom: 1px solid rgba(255,255,255,0.04);
}

@media (max-width: 768px) {
    .bench-hero h1 { font-size: 32px; }
    .bench-summary { grid-template-columns: 1fr; }
    .bench-source { grid-template-columns: 1fr; }
    .bench-bar-label { width: 80px; font-size: 11px; }
    .bench-summary-value { font-size: 28px; }
    .bench-section-nav { gap: 4px; }
    .bench-section-nav a { font-size: 11px; padding: 4px 10px; }
}

@media (max-width: 480px) {
    .bench-hero h1 { font-size: 26px; }
    .bench-hero p { font-size: 15px; }
    .bench-content { padding: 0 12px 60px; }
}
"#;

#[component]
pub fn Benchmarks() -> Element {
    let data = &*BENCH_DATA;
    let sources = &*SOURCES;
    let mut active_bench = use_signal(|| 0usize);
    let mut stats_open = use_signal(|| false);
    let mut compile_detail_open = use_signal(|| false);
    let mut methodology_open = use_signal(|| false);
    let mut source_open: Signal<[bool; 10]> = use_signal(|| [false; 10]);

    let breadcrumbs = vec![
        BreadcrumbItem { name: "Home", path: "/" },
        BreadcrumbItem { name: "Benchmarks", path: "/benchmarks" },
    ];
    let schemas = vec![
        organization_schema(),
        webpage_schema("LOGOS Benchmarks", seo_pages::BENCHMARKS.description, "/benchmarks"),
        breadcrumb_schema(&breadcrumbs),
    ];

    let logos_vs_c = data.summary.geometric_mean_speedup_vs_c
        .get("logos_release").copied().unwrap_or(0.0);
    let python_speedup = data.summary.geometric_mean_speedup_vs_c
        .get("python").copied().unwrap_or(0.001);
    let logos_vs_python = if python_speedup > 0.0 { logos_vs_c / python_speedup } else { 0.0 };

    let bench = &data.benchmarks[active_bench()];
    let bench_sources = &sources[active_bench()];
    let ref_size = &bench.reference_size;
    let ref_timings = bench.scaling.get(ref_size.as_str());

    let mut chart_entries: Vec<(&str, &str, f64, &str, bool)> = Vec::new();
    if let Some(timings) = ref_timings {
        for lang in &data.languages {
            if let Some(t) = timings.get(&lang.id) {
                chart_entries.push((
                    &lang.label,
                    &lang.color,
                    t.median_ms,
                    &lang.tier,
                    lang.id == "logos_release" || lang.id == "logos_interp",
                ));
            }
        }
    }

    // Find logos_interp from any scaling size (it may run at a smaller n)
    let mut interp_size_label: Option<String> = None;
    if !chart_entries.iter().any(|e| e.0 == "LOGOS (interpreted)") {
        let interp_data: Option<(&str, &TimingResult)> = bench.scaling.iter()
            .find_map(|(size, langs)| langs.get("logos_interp").map(|t| (size.as_str(), t)));
        if let Some((size, t)) = interp_data {
            if let Some(lang) = data.languages.iter().find(|l| l.id == "logos_interp") {
                chart_entries.push((&lang.label, &lang.color, t.median_ms, &lang.tier, true));
                if size != ref_size.as_str() {
                    interp_size_label = Some(format_n(size));
                }
            }
        }
    }

    chart_entries.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    // Split into compiled and interpreted with independent scales
    let compiled_max = chart_entries.iter()
        .filter(|e| e.3 != "interpreted" && e.0 != "LOGOS (interpreted)")
        .map(|e| e.2)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(1.0);

    let interpreted_max = chart_entries.iter()
        .filter(|e| e.3 == "interpreted" || e.0 == "LOGOS (interpreted)")
        .map(|e| e.2)
        .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(1.0);

    let compiled_tier_order = ["systems", "managed", "transpiled", "logos"];
    let mut compiled_grouped: Vec<(&str, Vec<(&str, &str, f64, bool)>)> = Vec::new();
    for &tier in &compiled_tier_order {
        let entries: Vec<_> = chart_entries.iter()
            .filter(|e| e.3 == tier && e.0 != "LOGOS (interpreted)")
            .map(|e| (e.0, e.1, e.2, e.4))
            .collect();
        if !entries.is_empty() {
            compiled_grouped.push((tier, entries));
        }
    }

    let interpreted_flat: Vec<(&str, &str, f64, bool)> = chart_entries.iter()
        .filter(|e| e.3 == "interpreted" || e.0 == "LOGOS (interpreted)")
        .map(|e| (e.0, e.1, e.2, e.4))
        .collect();

    // Stats table entries (all fields, sorted by median)
    let mut stats_entries: Vec<(&str, &str, &TimingResult)> = Vec::new();
    if let Some(timings) = ref_timings {
        for lang in &data.languages {
            if let Some(t) = timings.get(&lang.id) {
                stats_entries.push((&lang.label, &lang.id, t));
            }
        }
    }
    // Add logos_interp from any scaling size if not already present
    if !stats_entries.iter().any(|e| e.1 == "logos_interp") {
        if let Some(t) = bench.scaling.iter()
            .find_map(|(_, langs)| langs.get("logos_interp"))
        {
            if let Some(lang) = data.languages.iter().find(|l| l.id == "logos_interp") {
                stats_entries.push((&lang.label, &lang.id, t));
            }
        }
    }
    stats_entries.sort_by(|a, b| a.2.median_ms.partial_cmp(&b.2.median_ms).unwrap_or(std::cmp::Ordering::Equal));

    // Compilation entries sorted by mean_ms
    let mut compile_entries: Vec<(&str, f64, f64, bool)> = bench.compilation.iter()
        .map(|(name, r)| (name.as_str(), r.mean_ms, r.stddev_ms, name.starts_with("largo")))
        .collect();
    compile_entries.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let compile_max = compile_entries.last().map(|e| e.1).unwrap_or(1.0);

    // Summary chart entries (geometric mean) sorted by value descending
    let mut summary_entries: Vec<(&str, f64, &str, bool)> = Vec::new();
    for lang in &data.languages {
        if let Some(&val) = data.summary.geometric_mean_speedup_vs_c.get(&lang.id) {
            summary_entries.push((&lang.label, val, &lang.color, lang.id == "logos_release"));
        }
    }
    summary_entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let summary_max = summary_entries.first().map(|e| e.1).unwrap_or(1.0);

    // Source code languages to show (not LOGOS — that's always visible)
    let source_langs = ["c", "cpp", "rust", "zig", "go", "java", "js", "python", "ruby", "nim"];

    let commit_url = format!("{}/commit/{}", GITHUB_REPO, data.metadata.commit);
    let release_url = format!("{}/releases/tag/v{}", GITHUB_REPO, data.metadata.logos_version);
    let raw_json_url = format!("{}/blob/main/benchmarks/results/latest.json", GITHUB_REPO);
    let history_url = format!("{}/tree/main/benchmarks/results/history", GITHUB_REPO);
    let runsh_url = format!("{}/blob/main/benchmarks/run.sh", GITHUB_REPO);
    let bench_dir_url = format!("{}/tree/main/benchmarks", GITHUB_REPO);

    rsx! {
        PageHead {
            title: seo_pages::BENCHMARKS.title,
            description: seo_pages::BENCHMARKS.description,
            canonical_path: seo_pages::BENCHMARKS.canonical_path,
        }
        style { "{BENCHMARKS_STYLE}" }
        JsonLdMultiple { schemas }

        div { class: "bench-container",
            MainNav { active: ActivePage::Benchmarks, subtitle: Some("Performance") }

            // Hero + Overview
            section { class: "bench-hero", id: "overview",
                h1 { "Benchmarks" }
                p { "LOGOS compiles to Rust. Rust-level performance, English-level readability." }
                div { class: "bench-pills",
                    a { class: "bench-pill", href: "{release_url}", target: "_blank",
                        strong { "v{data.metadata.logos_version}" }
                    }
                    a { class: "bench-pill", href: "{commit_url}", target: "_blank",
                        "{data.metadata.commit}"
                    }
                    span { class: "bench-pill", "{data.metadata.cpu}" }
                    span { class: "bench-pill", "{data.metadata.os}" }
                    span { class: "bench-pill", "{&data.metadata.date[..10]}" }
                    a { class: "bench-pill link", href: "{raw_json_url}", target: "_blank",
                        "Raw JSON"
                    }
                }
            }

            // Section nav
            nav { class: "bench-section-nav",
                a { href: "#overview", "Overview" }
                a { href: "#performance", "Performance" }
                a { href: "#source", "Source Code" }
                a { href: "#compilation", "Compilation" }
                a { href: "#summary", "Summary" }
                a { href: "#methodology", "Methodology" }
            }

            div { class: "bench-content",
                // Summary cards
                div { class: "bench-summary",
                    div { class: "bench-summary-card",
                        div { class: "bench-summary-value cyan", "{logos_vs_c:.2}x" }
                        div { class: "bench-summary-label", "LOGOS vs C (geometric mean)" }
                    }
                    div { class: "bench-summary-card",
                        div { class: "bench-summary-value green", "{logos_vs_python:.0}x" }
                        div { class: "bench-summary-label", "LOGOS vs Python" }
                    }
                    div { class: "bench-summary-card",
                        div { class: "bench-summary-value purple", "{data.languages.len()}" }
                        div { class: "bench-summary-label", "Languages tested" }
                    }
                }

                // Benchmark tabs (shared across performance, source, compilation)
                div { class: "bench-tabs",
                    for (i, b) in data.benchmarks.iter().enumerate() {
                        button {
                            key: "{i}",
                            class: if active_bench() == i { "bench-tab active" } else { "bench-tab" },
                            onclick: move |_| {
                                active_bench.set(i);
                                stats_open.set(false);
                                compile_detail_open.set(false);
                                source_open.set([false; 10]);
                            },
                            "{b.name}"
                        }
                    }
                }

                // =============== PERFORMANCE ===============
                div { class: "bench-section", id: "performance",
                    div { class: "bench-section-title", "{bench.name}" }
                    div { class: "bench-section-desc",
                        "{bench.description} (n={ref_size})"
                    }

                    div { class: "bench-chart",
                        for (tier, entries) in compiled_grouped.iter() {
                            div { class: "bench-tier-label", "{tier_label(tier)}" }
                            for (label, color, median, is_logos) in entries.iter() {
                                {
                                    let pct = (*median / compiled_max * 100.0).min(100.0);
                                    let time_str = format_time(*median);
                                    let show_inside = pct > 15.0;
                                    let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                    rsx! {
                                        div { class: "bench-bar-row",
                                            div { class: "bench-bar-label", "{label}" }
                                            div { class: "bench-bar-track",
                                                div {
                                                    class: "{bar_class}",
                                                    style: "width: {pct:.1}%; background: {color};",
                                                    if show_inside {
                                                        span { class: "bench-bar-time", "{time_str}" }
                                                    }
                                                }
                                            }
                                            if !show_inside {
                                                span { class: "bench-bar-time-outside", "{time_str}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if !interpreted_flat.is_empty() {
                            div {
                                style: "border-top: 1px solid rgba(255,255,255,0.08); margin: 16px 0 8px; padding-top: 8px; font-size: 10px; font-weight: 600; text-transform: uppercase; letter-spacing: 0.5px; color: rgba(229,231,235,0.3);",
                                "Interpreted"
                            }
                            for (label, color, median, is_logos) in interpreted_flat.iter() {
                                {
                                    let pct = (*median / interpreted_max * 100.0).min(100.0);
                                    let time_str = if *label == "LOGOS (interpreted)" {
                                        if let Some(ref n) = interp_size_label {
                                            format!("{} (n={})", format_time(*median), n)
                                        } else {
                                            format_time(*median)
                                        }
                                    } else {
                                        format_time(*median)
                                    };
                                    let show_inside = pct > 15.0;
                                    let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                    rsx! {
                                        div { class: "bench-bar-row",
                                            div { class: "bench-bar-label", "{label}" }
                                            div { class: "bench-bar-track",
                                                div {
                                                    class: "{bar_class}",
                                                    style: "width: {pct:.1}%; background: {color};",
                                                    if show_inside {
                                                        span { class: "bench-bar-time", "{time_str}" }
                                                    }
                                                }
                                            }
                                            if !show_inside {
                                                span { class: "bench-bar-time-outside", "{time_str}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Collapsible: Detailed Statistics
                    button {
                        class: "bench-collapsible-btn",
                        onclick: move |_| stats_open.set(!stats_open()),
                        span {
                            class: if stats_open() { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                            "\u{25b6}"
                        }
                        "Detailed Statistics"
                    }
                    div {
                        class: if stats_open() { "bench-collapsible-body open" } else { "bench-collapsible-body" },
                        table { class: "bench-stats-table",
                            thead {
                                tr {
                                    th { "Language" }
                                    th { "Mean" }
                                    th { "Median" }
                                    th { "StdDev" }
                                    th { "Min" }
                                    th { "Max" }
                                    th { "CV" }
                                    th { "Runs" }
                                }
                            }
                            tbody {
                                for (label, lid, t) in stats_entries.iter() {
                                    tr {
                                        class: if *lid == "logos_release" || *lid == "logos_interp" { "highlight" } else { "" },
                                        td { "{label}" }
                                        td { "{format_time(t.mean_ms)}" }
                                        td { "{format_time(t.median_ms)}" }
                                        td { "\u{00b1}{format_time(t.stddev_ms)}" }
                                        td { "{format_time(t.min_ms)}" }
                                        td { "{format_time(t.max_ms)}" }
                                        td { "{t.cv:.3}" }
                                        td { "{t.runs}" }
                                    }
                                }
                            }
                        }
                    }
                }

                // =============== SOURCE CODE ===============
                div { class: "bench-section", id: "source",
                    div { class: "bench-section-title", "Source Code" }
                    div { class: "bench-section-desc", "The LOGOS source and the Rust it compiles to." }

                    // Always visible: LOGOS + Generated Rust side-by-side
                    div { class: "bench-source",
                        div { class: "bench-source-panel",
                            div { class: "bench-source-header logos", "LOGOS" }
                            div { class: "bench-source-code", "{bench.logos_source}" }
                        }
                        div { class: "bench-source-panel",
                            div { class: "bench-source-header rust", "Generated Rust" }
                            div { class: "bench-source-code", "{bench.generated_rust}" }
                        }
                    }

                    // Collapsible language source sections
                    for (idx, lang_id) in source_langs.iter().enumerate() {
                        {
                            let src = get_source(bench_sources, lang_id);
                            let color = lang_color(lang_id);
                            let label = lang_label(lang_id);
                            let version = data.metadata.versions.get(*lang_id).map(|s| s.as_str()).unwrap_or("");
                            let ext = lang_ext(lang_id);
                            let file_url = format!("{}/blob/main/benchmarks/programs/{}/{}", GITHUB_REPO, bench.id, ext);
                            let is_open = source_open()[idx];
                            rsx! {
                                div { class: "bench-lang-collapsible",
                                    div {
                                        class: "bench-lang-header",
                                        onclick: move |_| {
                                            let mut arr = source_open();
                                            arr[idx] = !arr[idx];
                                            source_open.set(arr);
                                        },
                                        span {
                                            class: "bench-lang-dot",
                                            style: "background: {color};",
                                        }
                                        span { class: "bench-lang-name", "{label}" }
                                        span { class: "bench-lang-version", "{version}" }
                                        a {
                                            class: "bench-lang-link",
                                            href: "{file_url}",
                                            target: "_blank",
                                            onclick: move |e: Event<MouseData>| e.stop_propagation(),
                                            "View on GitHub \u{2192}"
                                        }
                                        span {
                                            class: if is_open { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                                            "\u{25b6}"
                                        }
                                    }
                                    div {
                                        class: if is_open { "bench-lang-code open" } else { "bench-lang-code" },
                                        "{src}"
                                    }
                                }
                            }
                        }
                    }
                }

                // =============== COMPILATION ===============
                div { class: "bench-section", id: "compilation",
                    div { class: "bench-section-title", "Compilation Times" }
                    div { class: "bench-section-desc", "Time to compile each benchmark from source." }

                    div { class: "bench-chart",
                        for (name, mean, _stddev, is_largo) in compile_entries.iter() {
                            {
                                let pct = (*mean / compile_max * 100.0).min(100.0);
                                let time_str = format_time(*mean);
                                let show_inside = pct > 15.0;
                                let bar_class = if *is_largo { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                let color = if *is_largo { "#00d4ff" } else { "#6b7280" };
                                let display_name = compiler_label(name);
                                rsx! {
                                    div { class: "bench-bar-row",
                                        div { class: "bench-bar-label", "{display_name}" }
                                        div { class: "bench-bar-track",
                                            div {
                                                class: "{bar_class}",
                                                style: "width: {pct:.1}%; background: {color};",
                                                if show_inside {
                                                    span { class: "bench-bar-time", "{time_str}" }
                                                }
                                            }
                                        }
                                        if !show_inside {
                                            span { class: "bench-bar-time-outside", "{time_str}" }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Collapsible: Detailed Compilation Data
                    button {
                        class: "bench-collapsible-btn",
                        onclick: move |_| compile_detail_open.set(!compile_detail_open()),
                        span {
                            class: if compile_detail_open() { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                            "\u{25b6}"
                        }
                        "Detailed Compilation Data"
                    }
                    div {
                        class: if compile_detail_open() { "bench-collapsible-body open" } else { "bench-collapsible-body" },
                        table { class: "bench-compile-table",
                            thead {
                                tr {
                                    th { "Compiler" }
                                    th { "Mean" }
                                    th { "StdDev" }
                                }
                            }
                            tbody {
                                for (name, mean, stddev, is_largo) in compile_entries.iter() {
                                    tr {
                                        class: if *is_largo { "highlight" } else { "" },
                                        td { class: "compiler-name", "{compiler_label(name)}" }
                                        td { "{format_time(*mean)}" }
                                        td { "\u{00b1}{format_time(*stddev)}" }
                                    }
                                }
                            }
                        }
                    }
                }

                // =============== SUMMARY ===============
                div { class: "bench-section", id: "summary",
                    div { class: "bench-section-title", "Cross-Benchmark Summary" }
                    div { class: "bench-section-desc", "Geometric mean speedup vs C across all 6 benchmarks (log scale, higher is better)." }

                    div { class: "bench-chart",
                        for (label, val, color, is_logos) in summary_entries.iter() {
                            {
                                // Use log scale for the bar width so small values are still visible
                                let log_val = if *val > 0.0 { val.log10() } else { -4.0 };
                                let log_max = if summary_max > 0.0 { summary_max.log10() } else { 0.0 };
                                let log_min = -3.5_f64; // floor at 0.0003x
                                let pct = ((log_val - log_min) / (log_max - log_min) * 100.0).clamp(2.0, 100.0);

                                let display = if *val >= 0.01 {
                                    format!("{:.2}x", val)
                                } else {
                                    format!("{:.4}x", val)
                                };
                                let show_inside = pct > 20.0;
                                let bar_class = if *is_logos { "bench-bar-fill logos-highlight" } else { "bench-bar-fill" };
                                rsx! {
                                    div { class: "bench-bar-row",
                                        div { class: "bench-bar-label", "{label}" }
                                        div { class: "bench-bar-track",
                                            div {
                                                class: "{bar_class}",
                                                style: "width: {pct:.1}%; background: {color};",
                                                if show_inside {
                                                    span { class: "bench-bar-time", "{display}" }
                                                }
                                            }
                                        }
                                        if !show_inside {
                                            span { class: "bench-bar-time-outside", "{display}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // =============== METHODOLOGY ===============
                div { class: "bench-section", id: "methodology",
                    button {
                        class: "bench-collapsible-btn",
                        style: "margin-top: 0; border-top: none; padding-top: 0;",
                        onclick: move |_| methodology_open.set(!methodology_open()),
                        span {
                            class: if methodology_open() { "bench-collapsible-chevron open" } else { "bench-collapsible-chevron" },
                            "\u{25b6}"
                        }
                        span { style: "font-size: 18px; font-weight: 700; color: #fff;",
                            "Methodology"
                        }
                    }
                    div {
                        class: if methodology_open() { "bench-collapsible-body open" } else { "bench-collapsible-body" },
                        div { class: "bench-methodology",
                            ul {
                                li { "Each benchmark measured with hyperfine ({data.benchmarks[0].scaling.values().next().and_then(|s| s.values().next()).map(|t| t.runs).unwrap_or(10)} runs, 3 warmup)." }
                                li { "CPU: {data.metadata.cpu}." }
                                li { "OS: {data.metadata.os}." }
                                li { "All benchmarks produce identical verified output across all languages." }
                                li { "Geometric mean speedup computed across all {data.benchmarks.len()} benchmarks at their reference sizes." }
                            }

                            h3 { "Compiler Versions" }
                            table { class: "bench-version-table",
                                thead {
                                    tr {
                                        th { "Language" }
                                        th { "Version" }
                                    }
                                }
                                tbody {
                                    for (lang_id, version) in data.metadata.versions.iter() {
                                        tr {
                                            td { "{lang_label(lang_id)}" }
                                            td { "{version}" }
                                        }
                                    }
                                }
                            }

                            h3 { "Compiler Flags" }
                            table { class: "bench-version-table",
                                thead {
                                    tr {
                                        th { "Language" }
                                        th { "Flags" }
                                    }
                                }
                                tbody {
                                    tr { td { "C" } td { "gcc -O2 -lm" } }
                                    tr { td { "C++" } td { "g++ -O2 -std=c++17" } }
                                    tr { td { "Rust" } td { "rustc --edition 2021 -O" } }
                                    tr { td { "Zig" } td { "zig build-exe -O ReleaseFast" } }
                                    tr { td { "Go" } td { "go build (default)" } }
                                    tr { td { "Java" } td { "javac (JIT)" } }
                                    tr { td { "Nim" } td { "nim c -d:release" } }
                                    tr { td { "LOGOS" } td { "largo build --release (codegen to Rust, then rustc)" } }
                                }
                            }

                            h3 { "Links" }
                            ul {
                                li {
                                    a { href: "{runsh_url}", target: "_blank", "benchmarks/run.sh" }
                                    " \u{2014} benchmark runner script"
                                }
                                li {
                                    a { href: "{raw_json_url}", target: "_blank", "results/latest.json" }
                                    " \u{2014} raw benchmark data"
                                }
                                li {
                                    a { href: "{history_url}", target: "_blank", "results/history/" }
                                    " \u{2014} historical results by version"
                                }
                                li {
                                    a { href: "{bench_dir_url}", target: "_blank", "benchmarks/" }
                                    " \u{2014} all benchmark source code"
                                }
                            }
                        }
                    }
                }
            }

            Footer {}
        }
    }
}
