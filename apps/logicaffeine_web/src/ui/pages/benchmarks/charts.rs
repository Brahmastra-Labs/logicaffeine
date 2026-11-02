//! The per-benchmark charts.

use super::*;

// Per-benchmark scaling curve: wall-clock time vs problem size N, log-log, one
// line per language. Shows each language's scaling behaviour across the sizes
// already in the data (and where the LOGOS curve flattens out on a collapse).
pub(super) fn scaling_chart(bench: &Benchmark, languages: &[Language]) -> Element {
    let mut sizes: Vec<(String, f64)> = bench
        .sizes
        .iter()
        .filter(|s| bench.scaling.contains_key(s.as_str()))
        .filter_map(|s| s.parse::<f64>().ok().map(|n| (s.clone(), n)))
        .collect();
    sizes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    if sizes.len() < 2 {
        return rsx! {};
    }

    let mut t_min = f64::INFINITY;
    let mut t_max = 0.0_f64;
    for (s, _) in &sizes {
        if let Some(m) = bench.scaling.get(s) {
            for l in languages {
                if let Some(t) = m.get(&l.id) {
                    if t.median_ms > 0.0 {
                        t_min = t_min.min(t.median_ms);
                        t_max = t_max.max(t.median_ms);
                    }
                }
            }
        }
    }
    if !t_min.is_finite() || t_max <= 0.0 {
        return rsx! {};
    }

    let nx_min = sizes.first().unwrap().1.ln();
    let nx_max = sizes.last().unwrap().1.ln();
    let x_span = (nx_max - nx_min).max(1e-9);
    let y_span = (t_max.ln() - t_min.ln()).max(1e-9);

    let (x0, y0, pw, ph) = (54.0_f64, 14.0_f64, 572.0_f64, 220.0_f64);
    let px = |n: f64| x0 + (n.ln() - nx_min) / x_span * pw;
    let py = |t: f64| y0 + (1.0 - (t.ln() - t_min.ln()) / y_span) * ph;

    // (language id, label, vertex coords) — the field first, the LOGOS family last, so the LOGOS lines draw on top.
    let mut series: Vec<(String, String, Vec<(f64, f64)>)> = Vec::new();
    for l in languages {
        let mut coords: Vec<(f64, f64)> = Vec::new();
        for (s, n) in &sizes {
            if let Some(t) = bench.scaling.get(s).and_then(|m| m.get(&l.id)) {
                if t.median_ms > 0.0 {
                    coords.push((px(*n), py(t.median_ms)));
                }
            }
        }
        if !coords.is_empty() {
            series.push((l.id.clone(), l.label.clone(), coords));
        }
    }
    if series.is_empty() {
        return rsx! {};
    }
    series.sort_by_key(|(id, _, _)| tier_role(id) != "other");

    let y_bottom = y0 + ph;
    let x_right = x0 + pw;

    // Build the SVG inner markup as a string and set it via dangerous_inner_html
    // (the pattern the app's icons already use) so the chart never depends on
    // which individual SVG child attributes the rsx macro happens to expose.
    // Every mark carries a class; the colours are the page's theme tokens (style.css).
    let mut svg_inner = String::new();
    svg_inner.push_str(&format!(
        "<line class='bench-axis' x1='{x0:.1}' y1='{y0:.1}' x2='{x0:.1}' y2='{y_bottom:.1}'/>"
    ));
    svg_inner.push_str(&format!("<line class='bench-axis' x1='{x0:.1}' y1='{y_bottom:.1}' x2='{x_right:.1}' y2='{y_bottom:.1}'/>"));
    svg_inner.push_str(&format!(
        "<text class='bench-axis-label' x='{:.1}' y='{:.1}' text-anchor='end'>{}</text>",
        x0 - 6.0,
        y0 + 4.0,
        format_time(t_max)
    ));
    svg_inner.push_str(&format!(
        "<text class='bench-axis-label' x='{:.1}' y='{:.1}' text-anchor='end'>{}</text>",
        x0 - 6.0,
        y_bottom,
        format_time(t_min)
    ));
    for (_, n) in &sizes {
        svg_inner.push_str(&format!(
            "<text class='bench-axis-label' x='{:.1}' y='{:.1}' text-anchor='middle'>n={}</text>",
            px(*n),
            y_bottom + 16.0,
            fmt_n(*n)
        ));
    }
    for (id, label, coords) in &series {
        let role = tier_role(id);
        let pts = coords
            .iter()
            .map(|(x, y)| format!("{x:.1},{y:.1}"))
            .collect::<Vec<_>>()
            .join(" ");
        svg_inner.push_str(&format!(
            "<g class='bench-series bench-series-{role}'><title>{label}</title>"
        ));
        svg_inner.push_str(&format!("<polyline points='{pts}'/>"));
        for (x, y) in coords {
            svg_inner.push_str(&format!("<circle cx='{x:.1}' cy='{y:.1}' r='2.5'/>"));
        }
        svg_inner.push_str("</g>");
    }

    rsx! {
        div { class: "bench-scaling",
            svg {
                view_box: "0 0 640 266",
                width: "100%",
                role: "img",
                "aria-label": "Time against problem size, log-log, one line per language",
                dangerous_inner_html: "{svg_inner}",
            }
            div { class: "bench-scaling-legend",
                for (id, label, _coords) in series.iter().rev() {
                    div { class: "bench-scaling-legend-item",
                        span { class: swatch_class(id) }
                        "{label}"
                    }
                }
            }
        }
    }
}

/// kB → a human byte size for the memory bars.
pub(super) fn format_mem(kb: f64) -> String {
    if kb >= 1024.0 * 1024.0 {
        format!("{:.1} GB", kb / 1024.0 / 1024.0)
    } else if kb >= 1024.0 {
        format!("{:.1} MB", kb / 1024.0)
    } else {
        format!("{kb:.0} KB")
    }
}

/// bytes → a human size for the footprint bars (the byte-input sibling of `format_mem`).
pub(super) fn format_bytes(bytes: f64) -> String {
    if bytes >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", bytes / 1024.0 / 1024.0 / 1024.0)
    } else if bytes >= 1024.0 * 1024.0 {
        format!("{:.1} MB", bytes / 1024.0 / 1024.0)
    } else {
        format!("{:.0} KB", bytes / 1024.0)
    }
}

/// Bar label for a footprint: the as-built size, plus a `(… stripped)` tag only when the
/// stripped size is smaller AND renders differently at display resolution (so an
/// already-minimal binary never reads as the redundant "327 KB (327 KB stripped)").
pub(super) fn footprint_label(bytes: f64, stripped: Option<f64>) -> String {
    let main = format_bytes(bytes);
    match stripped {
        Some(st) if st > 0.0 && st < bytes => {
            let s = format_bytes(st);
            if s != main {
                format!("{main} ({s} stripped)")
            } else {
                main
            }
        }
        _ => main,
    }
}

/// The shipped footprint: the stripped, code-only binary you actually deploy — or the
/// as-built size when no stripped figure exists (e.g. a JVM class directory). This is the
/// honest, apples-to-apples number: some toolchains strip by default (`largo build
/// --release`) while others leave a symbol table on the as-built artifact, so comparing
/// as-built sizes would pit an already-stripped binary against unstripped ones.
pub(super) fn shipped_bytes(as_built: f64, stripped: Option<f64>) -> f64 {
    match stripped {
        Some(s) if s > 0.0 && s <= as_built => s,
        _ => as_built,
    }
}

/// Bar label for the binary-size chart: the shipped (stripped) size is the headline, and
/// the as-built size is ALWAYS shown in parentheses so every row reads the same way. When a
/// binary already ships stripped (`largo build --release` strips by default) the two numbers
/// are equal — that identity is the honest story, not a reason to hide the second figure.
pub(super) fn shipped_footprint_label(as_built: f64, stripped: Option<f64>) -> String {
    let shipped = format_bytes(shipped_bytes(as_built, stripped));
    let built = format_bytes(as_built);
    format!("{shipped} ({built} as-built)")
}

/// Compiled-artifact footprint — the size analogue of the memory bars, same visual style.
/// Each bar is the SHIPPED (stripped, code-only) binary you actually deploy, so every
/// toolchain is compared on the same basis; the as-built size (with symbols) rides along
/// in the label. Renders nothing until a size run populates `binary_sizes` (older files
/// have `binary_sizes: None`).
pub(super) fn binary_size_bar_chart(bench: &Benchmark, languages: &[Language]) -> Element {
    let sizes = match &bench.binary_sizes {
        Some(s) => s,
        None => return rsx! {},
    };
    // (label, language id, shipped, as_built, stripped)
    let mut bars: Vec<(String, String, f64, f64, Option<f64>)> = languages
        .iter()
        .filter_map(|l| {
            sizes
                .by_language
                .get(&l.id)
                .filter(|s| s.as_built > 0.0)
                .map(|s| {
                    (
                        l.label.clone(),
                        l.id.clone(),
                        shipped_bytes(s.as_built, s.stripped),
                        s.as_built,
                        s.stripped,
                    )
                })
        })
        .collect();
    if bars.is_empty() {
        return rsx! {};
    }
    bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let max = bars.iter().map(|b| b.2).fold(0.0_f64, f64::max).max(1.0);

    rsx! {
        div { class: "bench-mem",
            div { class: "bench-chart-hint", "Shipped size of the compiled program \u{2014} the stripped, code-only binary you actually deploy, so every language is compared on the same basis. Shorter bar ships less; the as-built size (with symbols) is in parentheses." }
            div { class: "bench-chart",
                for (label, id, shipped, as_built, stripped) in bars.iter() {
                    {
                        let pct = linear_bar_pct(*shipped, max);
                        let s = shipped_footprint_label(*as_built, *stripped);
                        let show_inside = pct > 32.0;
                        let class = bar_class(id);
                        rsx! {
                            div { class: "bench-bar-row",
                                div { class: "bench-bar-label", "{label}" }
                                div { class: "bench-bar-track",
                                    div { class: "{class}", style: "width: {pct:.1}%;",
                                        if show_inside {
                                            span { class: "bench-bar-time", "{s}" }
                                        }
                                    }
                                }
                                if !show_inside {
                                    span { class: "bench-bar-time-outside", "{s}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Peak-RSS bar chart at the reference size — the memory analogue of the runtime
/// bars, in the same visual style. Renders nothing until a `MEASURE_MEM=1` run
/// populates `memory` in the result file (older files have `memory: None`).
pub(super) fn memory_bar_chart(bench: &Benchmark, languages: &[Language]) -> Element {
    let mem = match &bench.memory {
        Some(m) => m,
        None => return rsx! {},
    };
    let size = if mem.by_size.contains_key(bench.reference_size.as_str()) {
        bench.reference_size.clone()
    } else {
        let mut ks: Vec<(String, f64)> = mem
            .by_size
            .keys()
            .filter_map(|s| s.parse::<f64>().ok().map(|n| (s.clone(), n)))
            .collect();
        ks.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        match ks.last() {
            Some((s, _)) => s.clone(),
            None => return rsx! {},
        }
    };
    let row = match mem.by_size.get(&size) {
        Some(r) => r,
        None => return rsx! {},
    };
    let mut bars: Vec<(String, String, f64)> = languages
        .iter()
        .filter_map(|l| {
            row.get(&l.id)
                .and_then(|v| *v)
                .filter(|kb| *kb > 0.0)
                .map(|kb| (l.label.clone(), l.id.clone(), kb))
        })
        .collect();
    if bars.is_empty() {
        return rsx! {};
    }
    bars.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    let max = bars.iter().map(|b| b.2).fold(0.0_f64, f64::max);

    rsx! {
        div { class: "bench-mem",
            div { class: "bench-chart-hint", "Peak resident memory at n = {size} \u{2014} shorter bar uses less." }
            div { class: "bench-chart",
                for (label, id, kb) in bars.iter() {
                    {
                        let pct = (kb / max * 100.0).min(100.0);
                        let s = format_mem(*kb);
                        let show_inside = pct > 18.0;
                        let class = bar_class(id);
                        rsx! {
                            div { class: "bench-bar-row",
                                div { class: "bench-bar-label", "{label}" }
                                div { class: "bench-bar-track",
                                    div { class: "{class}", style: "width: {pct:.1}%;",
                                        if show_inside {
                                            span { class: "bench-bar-time", "{s}" }
                                        }
                                    }
                                }
                                if !show_inside {
                                    span { class: "bench-bar-time-outside", "{s}" }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Declared time/space complexity plus the EMPIRICAL growth exponent fit from the
/// measured points: time from the scaling timings (present today), space from the
/// multi-size memory data (present after a `MEASURE_MEM=1` run). Renders nothing
/// when there is neither a declared complexity nor a fittable series.
pub(super) fn complexity_panel(bench: &Benchmark, languages: &[Language]) -> Element {
    let mut sizes: Vec<(String, f64)> = bench
        .sizes
        .iter()
        .filter(|s| bench.scaling.contains_key(s.as_str()))
        .filter_map(|s| s.parse::<f64>().ok().map(|n| (s.clone(), n)))
        .collect();
    sizes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let time_rows: Vec<(String, String, f64)> = languages
        .iter()
        .filter_map(|l| {
            let pts: Vec<(f64, f64)> = sizes
                .iter()
                .filter_map(|(s, n)| {
                    bench
                        .scaling
                        .get(s)
                        .and_then(|m| m.get(&l.id))
                        .filter(|t| t.median_ms > 0.0)
                        .map(|t| (*n, t.median_ms))
                })
                .collect();
            empirical_exponent(&pts).map(|e| (l.id.clone(), l.label.clone(), e))
        })
        .collect();
    // A log-log slope is a polynomial's degree. For a workload declared exponential or factorial it is not one — the
    // fit would print a meaningless "n^14" — so time growth is only measured for a polynomial workload.
    let polynomial = bench
        .complexity
        .as_ref()
        .map_or(true, |c| is_polynomial(&c.time));
    let time_rows: Vec<(String, String, f64)> = if polynomial { time_rows } else { Vec::new() };

    let space_rows: Vec<(String, String, f64)> = match &bench.memory {
        Some(mem) => {
            let mut ms: Vec<(String, f64)> = mem
                .by_size
                .keys()
                .filter_map(|s| s.parse::<f64>().ok().map(|n| (s.clone(), n)))
                .collect();
            ms.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            languages
                .iter()
                .filter_map(|l| {
                    let pts: Vec<(f64, f64)> = ms
                        .iter()
                        .filter_map(|(s, n)| {
                            mem.by_size
                                .get(s)
                                .and_then(|m| m.get(&l.id))
                                .and_then(|v| *v)
                                .filter(|kb| *kb > 0.0)
                                .map(|kb| (*n, kb))
                        })
                        .collect();
                    empirical_exponent(&pts).map(|e| (l.id.clone(), l.label.clone(), e))
                })
                .collect()
        }
        None => Vec::new(),
    };

    if bench.complexity.is_none() && time_rows.is_empty() && space_rows.is_empty() {
        return rsx! {};
    }

    rsx! {
        div { class: "bench-complexity",
            div { class: "bench-complexity-title", "Complexity" }
            if let Some(c) = &bench.complexity {
                div { class: "bench-complexity-declared",
                    span { class: "bench-complexity-chip",
                        "time " strong { "{c.time}" }
                    }
                    span { class: "bench-complexity-chip",
                        "space " strong { "{c.space}" }
                    }
                }
            }
            if !time_rows.is_empty() {
                div { class: "bench-complexity-grid",
                    div { class: "bench-complexity-col-title", "Measured time growth" }
                    for (id, label, e) in time_rows.iter() {
                        div { class: "bench-complexity-row",
                            span { class: swatch_class(id) }
                            span { class: "bench-complexity-lang", "{label}" }
                            span { class: "bench-complexity-exp", "{growth_label(\"t\", *e)}" }
                        }
                    }
                }
            }
            if !space_rows.is_empty() {
                div { class: "bench-complexity-grid",
                    div { class: "bench-complexity-col-title", "Measured space growth" }
                    for (id, label, e) in space_rows.iter() {
                        div { class: "bench-complexity-row",
                            span { class: swatch_class(id) }
                            span { class: "bench-complexity-lang", "{label}" }
                            span { class: "bench-complexity-exp", "{growth_label(\"rss\", *e)}" }
                        }
                    }
                }
            }
        }
    }
}

/// Whether a declared complexity is polynomial in n (`O(n)`, `O(n log n)`, `O(n*m)`, `O(n√n / log n)`) — not
/// exponential (`O(2^n)`), factorial (`O(n!)`) or Ackermann's.
pub(super) fn is_polynomial(declared: &str) -> bool {
    !(declared.contains("^n") || declared.contains('!') || declared.contains("A("))
}

/// A measured growth as the reader should see it: `t ≈ n^1.02`, or "flat" below n^0.1 — a fixed-size table or a
/// startup-bound run, whose noise can even fit a negative slope, which no workload has.
pub(super) fn growth_label(what: &str, exponent: f64) -> String {
    if exponent < 0.1 {
        format!("{what} flat")
    } else {
        format!("{what} \u{2248} n^{exponent:.2}")
    }
}
