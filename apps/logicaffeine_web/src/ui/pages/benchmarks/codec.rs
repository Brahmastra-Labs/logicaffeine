//! The Transport Codec tab.

use super::*;

// The LOGOS wire codec vs the industry serializers (Cap'n Proto, MessagePack, protobuf,
// bincode, postcard, CBOR, Arrow, JSON) — same logical data, same machine. Produced by
// the wirebench harness (crates/logicaffeine_wirebench) via benchmarks/run.sh Phase 7.
#[derive(Deserialize)]
pub(super) struct CodecData {
    #[serde(default)]
    pub(super) schema_version: u32,
    #[serde(default)]
    pub(super) metadata: CodecMetadata,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) iters: u32,
    #[serde(default)]
    pub(super) scenarios: Vec<CodecScenario>,
}

#[derive(Deserialize, Default)]
pub(super) struct CodecMetadata {
    #[serde(default)]
    pub(super) date: String,
    #[serde(default)]
    pub(super) cpu: String,
    #[serde(default)]
    pub(super) os: String,
    #[serde(default)]
    pub(super) features: Vec<String>,
}

#[derive(Deserialize, Clone)]
pub(super) struct CodecScenario {
    pub(super) id: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) n: usize,
    pub(super) kind: String,
    pub(super) rows: Vec<CodecRow>,
}

#[derive(Deserialize, Clone)]
pub(super) struct CodecRow {
    pub(super) codec: String,
    pub(super) size: usize,
    #[serde(default)]
    pub(super) enc_ns: f64,
    #[serde(default)]
    pub(super) dec_ns: f64,
    #[serde(default)]
    pub(super) read_one_ns: Option<f64>,
    #[serde(default)]
    pub(super) chosen: Option<ChosenConfig>,
    /// This codec's smallest size once granted the same compression LOGOS bakes in — set on the
    /// competitors, so the page can show a fair compressed-vs-compressed size fight. `None` on the
    /// LOGOS rows (the all-knobs winner's `size` is already its fair, compression-shopped size).
    #[serde(default)]
    pub(super) fair_size: Option<usize>,
}

/// Which dials the all-knobs winner actually selected — surfaced so a card can say *what* won,
/// not just "all knobs". `columns` names the real per-column encoding the codec shipped (one bare
/// name for a single-column message, or one `"field: encoding"` per field for a record list);
/// `summary` is the line the card shows verbatim. Present only on the `logos (BEST: all knobs)` row.
#[derive(Deserialize, Clone)]
pub(super) struct ChosenConfig {
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) numerics: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) floats: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) compression: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub(super) columns: Vec<String>,
    #[serde(default)]
    pub(super) summary: String,
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) static CODEC_DATA: LazyLock<CodecData> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../../../../benchmarks/results/latest-codec.json"
    ))
    .unwrap()
});
#[cfg(target_arch = "wasm32")]
pub(super) static CODEC_DATA: OnceLock<CodecData> = OnceLock::new();

/// The wire-codec head-to-head feed — same gating contract as [`bench_data`].
pub(super) fn codec_data() -> Option<&'static CodecData> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(&CODEC_DATA)
    }
    #[cfg(target_arch = "wasm32")]
    {
        CODEC_DATA.get()
    }
}

/// Every LOGOS dial (varint / fixed / group-varint / affine / struct-view / all-knobs) is
/// rendered in the cyan family and highlighted; everything else is a competitor.
pub(super) fn is_logos_codec(c: &str) -> bool {
    c.starts_with("logos")
}

pub(super) fn codec_label(c: &str) -> &str {
    match c {
        "logos (BEST: all knobs)" => "LOGOS \u{2014} all knobs",
        "logos (varint)" => "LOGOS \u{2014} varint",
        "logos (fixed)" => "LOGOS \u{2014} fixed (memcpy)",
        "logos (gv/simd)" => "LOGOS \u{2014} group-varint",
        "logos (affine)" => "LOGOS \u{2014} affine",
        "logos (struct-view)" => "LOGOS \u{2014} struct-view",
        "logos (struct-view fixed)" => "LOGOS \u{2014} struct-view (fixed)",
        "logos (zero-copy)" => "LOGOS \u{2014} zero-copy",
        "bincode" => "bincode",
        "postcard" => "postcard",
        "messagepack" => "MessagePack",
        "cbor" => "CBOR",
        "json" => "JSON",
        "arrow (ipc)" => "Apache Arrow",
        "protobuf/grpc" => "Protobuf / gRPC",
        "capnproto" => "Cap'n Proto",
        other => other,
    }
}

/// The class a codec's bar is filled with: the headline LOGOS configuration (every dial on) in the theme's accent, the
/// other LOGOS configurations in its tint, every other serializer in the page's one neutral.
pub(super) fn codec_bar_class(c: &str) -> &'static str {
    match c {
        "logos (BEST: all knobs)" => "bench-codec-fill bench-bar-logos-rust",
        _ if is_logos_codec(c) => "bench-codec-fill bench-bar-logos-c",
        _ => "bench-codec-fill bench-bar-other",
    }
}

/// A nanoseconds-per-op label that stays readable from a single-field read (~40 ns) to a
/// whole-message JSON decode (hundreds of µs).
pub(super) fn format_ns(ns: f64) -> String {
    if ns < 1000.0 {
        format!("{ns:.0} ns")
    } else if ns < 1_000_000.0 {
        format!("{:.2} \u{00b5}s", ns / 1000.0)
    } else {
        format!("{:.2} ms", ns / 1_000_000.0)
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum CodecAxis {
    Size,
    Encode,
    Decode,
    ReadOne,
}

/// One axis of a codec scenario as a sorted (winner-on-top) horizontal bar chart, reusing
/// the page's `bench-bar-*` styling. Rows missing the axis (e.g. no `read_one_ns`) are
/// dropped, so the same helper serves the size/encode/decode charts and the random-access one.
pub(super) fn codec_axis_chart(s: &CodecScenario, axis: CodecAxis) -> Element {
    let val = |r: &CodecRow| -> Option<f64> {
        match axis {
            CodecAxis::Size => Some(r.size as f64),
            CodecAxis::Encode => Some(r.enc_ns),
            CodecAxis::Decode => Some(r.dec_ns),
            CodecAxis::ReadOne => r.read_one_ns,
        }
    };
    let fmt = |r: &CodecRow| -> String {
        match axis {
            CodecAxis::Size => format_bytes(r.size as f64),
            _ => format_ns(val(r).unwrap_or(0.0)),
        }
    };
    let mut rows: Vec<CodecRow> = s
        .rows
        .iter()
        .filter(|r| val(r).is_some())
        .cloned()
        .collect();
    rows.sort_by(|a, b| {
        val(a)
            .unwrap()
            .partial_cmp(&val(b).unwrap())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if rows.is_empty() {
        return rsx! {};
    }
    // Sorted ascending, so the last row is the worst (largest) — it fills the track and
    // every other bar is its honest fraction of it.
    let vmax = val(rows.last().unwrap()).unwrap();
    rsx! {
        div { class: "bench-chart",
            for r in rows.iter() {
                {
                    let v = val(r).unwrap();
                    let pct = linear_bar_pct(v, vmax);
                    let s_str = fmt(r);
                    let fill_class = codec_bar_class(&r.codec);
                    let val_class = if is_logos_codec(&r.codec) { "bench-codec-val logos-val" } else { "bench-codec-val" };
                    let label = codec_label(&r.codec);
                    rsx! {
                        div { class: "bench-bar-row",
                            div { class: "bench-bar-label", "{label}" }
                            div { class: "bench-codec-track",
                                div { class: "{fill_class}", style: "width: {pct:.1}%;" }
                            }
                            div { class: "{val_class}", "{s_str}" }
                        }
                    }
                }
            }
        }
    }
}

/// The provable headline: the count of fair workloads where `logos (BEST: all knobs)` is no
/// larger than EVERY competitor (ties count), out of all fair workloads. True by construction
/// — the all-knobs row tries every dial and reports the minimum — and asserted against the
/// committed data by `codec_headline_is_provable`, so the page can never overclaim.
pub(super) fn codec_size_wins(d: &CodecData) -> (usize, usize) {
    let fair: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
    let wins = fair
        .iter()
        .filter(
            |s| match s.rows.iter().find(|r| r.codec == "logos (BEST: all knobs)") {
                Some(best) => s
                    .rows
                    .iter()
                    .filter(|r| !is_logos_codec(&r.codec))
                    .all(|r| best.size <= r.size),
                None => false,
            },
        )
        .count();
    (wins, fair.len())
}

/// (smallest LOGOS bytes, smallest-competitor label, its bytes) for a scenario — the data
/// behind each "N× smaller than the best of the rest" caption.
pub(super) fn codec_best_vs_competitor(s: &CodecScenario) -> Option<(usize, String, usize)> {
    let ours = s
        .rows
        .iter()
        .filter(|r| is_logos_codec(&r.codec))
        .map(|r| r.size)
        .min()?;
    let rival = s
        .rows
        .iter()
        .filter(|r| !is_logos_codec(&r.codec))
        .min_by_key(|r| r.size)?;
    Some((ours, codec_label(&rival.codec).to_string(), rival.size))
}

/// (fastest LOGOS read-one ns, Cap'n Proto's read-one ns) for the random-access scenario.
pub(super) fn codec_random_access_vs_capnp(s: &CodecScenario) -> Option<(f64, f64)> {
    let ours = s
        .rows
        .iter()
        .filter(|r| is_logos_codec(&r.codec))
        .filter_map(|r| r.read_one_ns)
        .fold(f64::INFINITY, f64::min);
    let capnp = s
        .rows
        .iter()
        .find(|r| r.codec == "capnproto")
        .and_then(|r| r.read_one_ns)?;
    if ours.is_finite() {
        Some((ours, capnp))
    } else {
        None
    }
}

/// The size caption for a scenario ("LOGOS …B vs best of the rest (…) …B — N× smaller").
pub(super) fn codec_size_caption(s: &CodecScenario) -> Option<String> {
    codec_best_vs_competitor(s).map(|(ours, rival, rsize)| {
        format!(
            "LOGOS {} vs best of the rest ({rival}) {} — {:.2}× smaller",
            format_bytes(ours as f64),
            format_bytes(rsize as f64),
            rsize as f64 / ours.max(1) as f64
        )
    })
}

/// The random-access caption ("LOGOS …ns vs Cap'n Proto …ns — N× faster open + read").
pub(super) fn codec_ra_caption(s: &CodecScenario) -> Option<String> {
    codec_random_access_vs_capnp(s).map(|(ours, capnp)| {
        format!(
            "LOGOS {} vs Cap'n Proto {} — {:.2}× faster open + read",
            format_ns(ours),
            format_ns(capnp),
            capnp / ours.max(1.0)
        )
    })
}

/// (#fair workloads where the fastest LOGOS dial is ≤ every competitor on `axis`, total fair) —
/// the scorecard's "fastest encode / decode on N/N" stats, computed from the baked data.
pub(super) fn codec_axis_wins(d: &CodecData, axis: CodecAxis) -> (usize, usize) {
    let v = |r: &CodecRow| -> Option<f64> {
        match axis {
            CodecAxis::Size => Some(r.size as f64),
            CodecAxis::Encode => Some(r.enc_ns),
            CodecAxis::Decode => Some(r.dec_ns),
            CodecAxis::ReadOne => r.read_one_ns,
        }
    };
    let fair: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
    let wins = fair
        .iter()
        .filter(|s| {
            let lo = s
                .rows
                .iter()
                .filter(|r| is_logos_codec(&r.codec))
                .filter_map(&v)
                .fold(f64::INFINITY, f64::min);
            let co = s
                .rows
                .iter()
                .filter(|r| !is_logos_codec(&r.codec))
                .filter_map(&v)
                .fold(f64::INFINITY, f64::min);
            lo.is_finite() && co.is_finite() && lo <= co
        })
        .count();
    (wins, fair.len())
}

/// The biggest structural showcase win (best competitor ÷ smallest LOGOS, by size).
pub(super) fn codec_top_showcase(d: &CodecData) -> Option<f64> {
    d.scenarios
        .iter()
        .filter(|s| s.kind == "showcase")
        .filter_map(|s| {
            codec_best_vs_competitor(s).map(|(ours, _, rival)| rival as f64 / ours.max(1) as f64)
        })
        .fold(None, |acc, r| Some(acc.map_or(r, |m: f64| m.max(r))))
}

/// Geometric-mean size advantage over the best competitor across the fair workloads (×).
pub(super) fn codec_geomean_size(d: &CodecData) -> f64 {
    let ratios: Vec<f64> = d
        .scenarios
        .iter()
        .filter(|s| s.kind == "fair")
        .filter_map(|s| {
            codec_best_vs_competitor(s).map(|(ours, _, rival)| rival as f64 / ours.max(1) as f64)
        })
        .collect();
    if ratios.is_empty() {
        return 1.0;
    }
    (ratios.iter().map(|r| r.ln()).sum::<f64>() / ratios.len() as f64).exp()
}

/// The all-knobs winner row for a scenario (the size-optimal config, whose `chosen` names the
/// dials that won). `None` for a scenario that has no all-knobs row (e.g. random-access).
pub(super) fn codec_best_row(s: &CodecScenario) -> Option<&CodecRow> {
    s.rows.iter().find(|r| r.codec == "logos (BEST: all knobs)")
}

/// A one-line, real-world "what this data shape is for" — so every example reads as a use case,
/// not just a bar chart. Covers every scenario id the harness emits (asserted by a test), so a
/// new scenario can't ship without its copy.
pub(super) fn codec_good_for(id: &str) -> Option<&'static str> {
    Some(match id {
        "ints" => "ID columns, counters, quantities, foreign keys",
        "floats" => "measurements, prices, scientific readings",
        "timeseries" => "sensor / financial streams that vary slowly — xor-delta shines",
        "points" => "coordinates, vectors, spatial data",
        "records" => "row / struct lists — API payloads, database rows, events",
        "strings" => "text-heavy payloads — names, labels, log lines",
        "bools" => "flag columns, feature toggles, bitsets",
        "adv_negative" => "signed data that trips formats without zig-zag",
        "adv_repetitive" => "columns with long runs — status codes, repeated defaults",
        "adv_huge" => "wide 64-bit values — hashes, ids, nanosecond timestamps",
        "random_access" => "reading ONE field of a big message without decoding the rest",
        "affine_showcase" => "arithmetic progressions — row ids, ramps, fixed offsets",
        "poly_showcase" => "polynomial columns — accumulators, quadratic ramps",
        "categorical" => "low-cardinality labels — enums, tags, country codes",
        "int_set" => "membership sets — id sets, allow-lists",
        "int_map" => "int-keyed maps — lookup tables, adjacency lists",
        _ => return None,
    })
}

/// The other end of the size↔speed dial: the fastest-to-decode LOGOS dial that ISN'T the size
/// winner, when it decodes meaningfully faster (a real speed-for-size trade). This is what the
/// card surfaces as "prioritize speed?", answering "if I don't take the smallest, what do I get?".
pub(super) fn codec_speed_pick(s: &CodecScenario) -> Option<(String, usize, f64)> {
    let best = codec_best_row(s)?;
    let pick = s
        .rows
        .iter()
        .filter(|r| {
            is_logos_codec(&r.codec) && r.codec != "logos (BEST: all knobs)" && r.dec_ns > 0.0
        })
        .min_by(|a, b| {
            a.dec_ns
                .partial_cmp(&b.dec_ns)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    (pick.dec_ns < best.dec_ns * 0.9)
        .then(|| (codec_label(&pick.codec).to_string(), pick.size, pick.dec_ns))
}

/// The FAIR-FIGHT size comparison for a scenario: (LOGOS best-compressed size, best competitor's
/// fair-compressed size, its label). LOGOS bakes compression into the codec; here every competitor
/// is granted the SAME compression, so this is an honest compressed-vs-compressed comparison — not
/// LOGOS-compressed-vs-competitor-raw. `None` if no all-knobs winner or no competitor `fair_size`.
pub(super) fn codec_fair_fight(s: &CodecScenario) -> Option<(usize, usize, String)> {
    let logos = codec_best_row(s)?.size; // the BEST row already shops compression
    let rival = s
        .rows
        .iter()
        .filter(|r| !is_logos_codec(&r.codec))
        .filter_map(|r| r.fair_size.map(|f| (f, r.codec.clone())))
        .min_by_key(|(f, _)| *f)?;
    Some((logos, rival.0, codec_label(&rival.1).to_string()))
}

/// (#fair workloads where LOGOS is no larger than every competitor IN THE FAIR FIGHT, total fair) —
/// the honest "even when everyone compresses" size count for the scorecard.
pub(super) fn codec_fair_size_wins(d: &CodecData) -> (usize, usize) {
    let fair: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
    let wins = fair
        .iter()
        .filter(|s| matches!(codec_fair_fight(s), Some((lo, co, _)) if lo <= co))
        .count();
    (wins, fair.len())
}

/// The winner card shown above each example's bar charts: what config won, its size AND encode
/// AND decode together (the trio the three separate charts never connect), what the shape is good
/// for, and the speed-pick alternative. `rsx!{}` (nothing) for a scenario with no all-knobs winner.
pub(super) fn codec_winner_card(s: &CodecScenario) -> Element {
    let Some(best) = codec_best_row(s) else {
        return rsx! {};
    };
    let Some(chosen) = best.chosen.as_ref() else {
        return rsx! {};
    };
    let headline = match codec_best_vs_competitor(s) {
        Some((ours, rival, rsize)) => format!(
            "\u{2714} LOGOS ships {} \u{2014} {:.2}\u{00d7} smaller than {}",
            format_bytes(ours as f64),
            rsize as f64 / ours.max(1) as f64,
            rival
        ),
        None => format!("\u{2714} LOGOS ships {}", format_bytes(best.size as f64)),
    };
    let config_line = format!("won with: {}", chosen.summary);
    let speed_line = format!(
        "encode {} \u{00b7} decode {}",
        format_ns(best.enc_ns),
        format_ns(best.dec_ns)
    );
    let good = codec_good_for(&s.id);
    // The honest compressed-vs-compressed line: when the headline win used compression, this shows
    // whether LOGOS still wins (or ties) once every rival is given the same compression.
    let fair = codec_fair_fight(s).map(|(lo, co, rlabel)| {
        let verdict = if lo < co {
            "still smaller"
        } else if lo == co {
            "tie"
        } else {
            "rival smaller"
        };
        format!(
            "Fair fight (all compressed): LOGOS {} vs {} {} \u{2014} {}",
            format_bytes(lo as f64),
            rlabel,
            format_bytes(co as f64),
            verdict
        )
    });
    let speed_pick = codec_speed_pick(s).map(|(label, size, dec)| {
        format!(
            "Prioritize speed? {} \u{2014} {} \u{00b7} decode {}",
            label,
            format_bytes(size as f64),
            format_ns(dec)
        )
    });
    rsx! {
        div { class: "bench-codec-card",
            div { class: "bench-codec-card-head", "{headline}" }
            div { class: "bench-codec-card-line", "{config_line}" }
            div { class: "bench-codec-card-sub", "{speed_line}" }
            if let Some(g) = good {
                div { class: "bench-codec-card-good", "Good for: {g}" }
            }
            if let Some(f) = fair {
                div { class: "bench-codec-card-fine", "{f}" }
            }
            if let Some(sp) = speed_pick {
                div { class: "bench-codec-card-foot", "{sp}" }
            }
        }
    }
}

/// The dial glossary: every knob the codec exposes, in plain words, with what it's *for* — so the
/// section reads as a menu of trade-offs, not an unexplained pile of bars. The top group is the
/// dials these benchmarks actually exercise; the second is the rest of the codec's capability
/// surface, so the reader sees the whole instrument. Copy tracks the marshal.rs doc comments.
pub(super) fn codec_dial_glossary() -> Element {
    let benched: &[(&str, &str)] = &[
        ("varint", "LEB128 — smallest bytes; the default for a bandwidth-bound network link"),
        ("fixed (memcpy)", "raw 8-byte i64 — fastest decode on a datacenter / RDMA link, ~4\u{00d7} the size"),
        ("group-varint", "varint-class size, SIMD-decoded several ints at once — small AND fast"),
        ("auto structure", "per-column bake-off (delta \u{00b7} delta-of-delta \u{00b7} FOR bit-pack \u{00b7} run-length \u{00b7} dictionary \u{00b7} affine \u{00b7} polynomial) — ships the smallest, never larger than varint"),
        ("xor-delta floats", "Gorilla-style — slowly-varying float streams shrink; high-entropy data falls back to memcpy"),
        ("struct-view", "an offset table per field so any ONE field reads in O(1) — Cap'n Proto's game; larger on the wire"),
        ("compression", "deflate / lz4 / zstd over the whole frame, kept only when smaller — for bulky or redundant payloads"),
    ];
    let also: &[(&str, &str)] = &[
        ("checksum", "an FNV tag so the receiver rejects a corrupted frame (Raw \u{2194} Checked)"),
        ("type-id", "elide struct / enum names on a Logos\u{2194}Logos link once both peers know the schema"),
        ("FEC", "Reed-Solomon shards — reconstruct the message from any K of N (lossy links, multicast)"),
        ("dedup", "a subtree reached by the same reference ships once + backrefs; the decoder rebuilds the sharing"),
    ];
    let item = |name: &'static str, desc: &'static str| {
        rsx! {
            div { class: "bench-codec-dial",
                span { class: "bench-codec-dial-name", "{name}" }
                span { class: "bench-codec-dial-desc", " \u{2014} {desc}" }
            }
        }
    };
    rsx! {
        div { class: "bench-codec-dials",
            div { class: "bench-codec-dials-title", "The dials \u{2014} every form is self-describing, so the decoder needs no hint" }
            div { class: "bench-codec-dials-sub", "In these benchmarks:" }
            for (name, desc) in benched.iter().copied() { {item(name, desc)} }
            div { class: "bench-codec-dials-sub later", "Also available (not exercised by the charts below):" }
            for (name, desc) in also.iter().copied() { {item(name, desc)} }
        }
    }
}

/// The "Serialization — the wire codec" section: the LOGOS wire codec head-to-head against
/// the industry serializers across wire size, encode, decode and single-field random access.
/// Every figure is measured by the wirebench harness on the same logical data; the headline
/// ("smallest wire on N/N workloads") is provable, and the structured/affine showcase is
/// fenced off from the fair results so the math-hack is never conflated with general data.
#[component]
pub fn CodecSection() -> Element {
    let d = codec_data().expect("rendered only inside the loaded Benchmarks page");
    let (wins, total) = codec_size_wins(d);
    let fair: Vec<&CodecScenario> = d.scenarios.iter().filter(|s| s.kind == "fair").collect();
    let adversarial: Vec<&CodecScenario> = d
        .scenarios
        .iter()
        .filter(|s| s.kind == "adversarial")
        .collect();
    let random: Vec<&CodecScenario> = d
        .scenarios
        .iter()
        .filter(|s| s.kind == "random_access")
        .collect();
    let structural: Vec<&CodecScenario> = d
        .scenarios
        .iter()
        .filter(|s| s.kind == "structural")
        .collect();
    let showcase: Vec<&CodecScenario> = d
        .scenarios
        .iter()
        .filter(|s| s.kind == "showcase")
        .collect();

    let mut comp_names = vec!["bincode", "postcard", "MessagePack", "CBOR", "JSON"];
    if d.metadata.features.iter().any(|f| f == "arrow-bench") {
        comp_names.push("Apache Arrow");
    }
    if d.metadata.features.iter().any(|f| f == "protobuf") {
        comp_names.push("Protobuf/gRPC");
    }
    if d.metadata.features.iter().any(|f| f == "capnproto") {
        comp_names.push("Cap'n Proto");
    }
    let comp_list = comp_names.join(", ");

    rsx! {
        div { class: "bench-section", id: "serialization",
            div { class: "bench-section-title", "Serialization \u{2014} the wire codec" }
            div { class: "bench-section-desc",
                "Speed of code is one axis; bytes on the wire are another. The LOGOS wire codec is benched head-to-head against {comp_list} \u{2014} the same logical data through every codec, on the same machine, with seeded-random payloads (never a tidy 0..n sequence that would hand a structural codec a free win). LOGOS wins encode AND decode on every fair workload and is competitive-to-smaller on size; the honest details are on each card."
            }

            // ---- Dial glossary: what each knob does and what it's for ----
            { codec_dial_glossary() }

            // ---- Scorecard: the whole head-to-head at a glance, computed from the baked data ----
            {
                let (enc_wins, _) = codec_axis_wins(d, CodecAxis::Encode);
                let (dec_wins, _) = codec_axis_wins(d, CodecAxis::Decode);
                let (fair_wins, fair_total) = codec_fair_size_wins(d);
                let geo = format!("{:.1}\u{00d7}", codec_geomean_size(d));
                let ra = random.first().and_then(|s| codec_random_access_vs_capnp(s));
                let ra_str = ra.map(|(o, c)| format!("{:.1}\u{00d7}", c / o.max(1.0)));
                let top_str = codec_top_showcase(d).map(|r| format!("{:.0}\u{00d7}", r));
                rsx! {
                    div { class: "bench-summary",
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value green", "{enc_wins}/{total}" }
                            div { class: "bench-summary-label", "fastest encode (fair workloads)" }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value green", "{dec_wins}/{total}" }
                            div { class: "bench-summary-label", "fastest decode (fair workloads)" }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value cyan", "{wins}/{total}" }
                            div { class: "bench-summary-label", "smallest out-of-the-box (we auto-compress; rivals raw)" }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value cyan", "{fair_wins}/{fair_total}" }
                            div { class: "bench-summary-label", "smallest in a fair fight (everyone compressed)" }
                        }
                        if let Some(ra_str) = ra_str {
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value cyan", "{ra_str}" }
                                div { class: "bench-summary-label", "faster random-access read vs Cap'n Proto" }
                            }
                        }
                        if let Some(top_str) = top_str {
                            div { class: "bench-summary-card",
                                div { class: "bench-summary-value purple", "up to {top_str}" }
                                div { class: "bench-summary-label", "structural showcases (smaller)" }
                            }
                        }
                        div { class: "bench-summary-card",
                            div { class: "bench-summary-value cyan", "{geo}" }
                            div { class: "bench-summary-label", "geo-mean smaller, out of the box" }
                        }
                    }
                    div { class: "bench-codec-panel",
                        div { class: "bench-codec-panel-text",
                            "Every figure is measured on the same logical data, same machine, seeded-random. LOGOS wins encode AND decode on every fair workload \u{2014} the default dial is a pure memcpy that never searches at send time (you pick the shape ahead of time), so it beats even bincode's raw copy; decode is faster because the format is columnar and typed. On SIZE we\u{2019}re honest two ways: \u{201c}out of the box\u{201d} LOGOS auto-compresses and rivals ship raw (a real transport advantage), and the \u{201c}fair fight\u{201d} gives every rival the SAME compression \u{2014} there LOGOS wins on structured data and ties on random primitive arrays (a memcpy is a memcpy). The all-knobs winner\u{2019}s exact dials, and both size comparisons, are on each card. Decode is timed \u{201c}to usable\u{201d} (every value touched). The contrived generator showcases (a pure 0,1,2\u{2026} ramp) are fenced off from the fair results."
                        }
                    }
                }
            }

            // ---- Fair workloads: size / encode / decode ----
            for s in fair.iter() {
                div { style: "margin-bottom: 26px;",
                    div { class: "bench-codec-scenario", "{s.title}  \u{00b7}  n = {s.n}" }
                    { codec_winner_card(s) }
                    if let Some(c) = codec_size_caption(s) {
                        div { class: "bench-codec-caption", "{c}" }
                    }
                    div { class: "bench-chart-hint bench-codec-hint", "Wire size \u{2014} smaller is better, bars proportional" }
                    { codec_axis_chart(s, CodecAxis::Size) }
                    div { class: "bench-chart-hint bench-codec-hint", "Encode (ns/op) \u{2014} smaller is better, bars proportional" }
                    { codec_axis_chart(s, CodecAxis::Encode) }
                    div { class: "bench-chart-hint bench-codec-hint", "Decode to usable (ns/op) \u{2014} smaller is better, bars proportional" }
                    { codec_axis_chart(s, CodecAxis::Decode) }
                }
            }

            // ---- Random access: Cap'n Proto's home turf ----
            for s in random.iter() {
                div { style: "margin-bottom: 26px;",
                    div { class: "bench-codec-scenario", "{s.title}  \u{00b7}  n = {s.n}" }
                    div { class: "bench-codec-p",
                        "A message arrives; you open it and read ONE field. This is exactly what Cap'n Proto is built for. LOGOS opens its record-list view (row + field offset tables) in O(1); the self-describing codecs must decode the whole message to index it."
                    }
                    if let Some(c) = codec_ra_caption(s) {
                        div { class: "bench-codec-caption", "{c}" }
                    }
                    div { class: "bench-chart-hint bench-codec-hint", "Open + read one field (ns/op) \u{2014} smaller is better, bars proportional" }
                    { codec_axis_chart(s, CodecAxis::ReadOne) }
                }
            }

            // ---- Adversarial shapes ----
            if !adversarial.is_empty() {
                div { class: "bench-codec-h", "Adversarial shapes" }
                div { class: "bench-codec-p",
                    "The data shapes that wreck fixed-format codecs \u{2014} all dials on."
                }
                for s in adversarial.iter() {
                    div { style: "margin-bottom: 20px;",
                        div { class: "bench-codec-scenario", "{s.title}  \u{00b7}  n = {s.n}" }
                        { codec_winner_card(s) }
                        if let Some(c) = codec_size_caption(s) {
                            div { class: "bench-codec-caption", "{c}" }
                        }
                        div { class: "bench-chart-hint bench-codec-hint", "Wire size \u{2014} smaller is better, bars proportional" }
                        { codec_axis_chart(s, CodecAxis::Size) }
                    }
                }
            }

            // ---- Structural wins on REAL data: general columnar techniques, not contrived ----
            if !structural.is_empty() {
                div { class: "bench-codec-h", "Structural wins on real data" }
                div { class: "bench-codec-p",
                    "Not math tricks \u{2014} the standard columnar techniques (dictionary encoding for low-cardinality labels, bit-packing for booleans) that Parquet and Arrow are built on, applied automatically. Common shapes in real payloads, and LOGOS ships them by default when they win."
                }
                for s in structural.iter() {
                    div { style: "margin-bottom: 22px;",
                        div { class: "bench-codec-scenario", "{s.title}  \u{00b7}  n = {s.n}" }
                        { codec_winner_card(s) }
                        if let Some(c) = codec_size_caption(s) {
                            div { class: "bench-codec-caption", "{c}" }
                        }
                        div { class: "bench-chart-hint bench-codec-hint", "Wire size \u{2014} smaller is better, bars proportional" }
                        { codec_axis_chart(s, CodecAxis::Size) }
                    }
                }
            }

            // ---- Showcases, explicitly fenced off from the fair results ----
            if !showcase.is_empty() {
                div { class: "bench-codec-panel showcase",
                    div { class: "bench-codec-panel-text",
                        "\u{26a1} Showcases, kept separate from the fair results: contrived columns where the codec ships the GENERATING RULE instead of the data \u{2014} an affine progression as (base, stride, n), a polynomial as its finite-difference seeds, a consecutive set as base+stride+count \u{2014} winning by orders of magnitude. Not general-data results; they show the ceiling when structure is perfect."
                    }
                }
                for s in showcase.iter() {
                    div { style: "margin-bottom: 22px;",
                        div { class: "bench-codec-scenario", "{s.title}  \u{00b7}  n = {s.n}" }
                        { codec_winner_card(s) }
                        if let Some(c) = codec_size_caption(s) {
                            div { class: "bench-codec-caption", "{c}" }
                        }
                        div { class: "bench-chart-hint bench-codec-hint", "Wire size \u{2014} smaller is better, bars proportional" }
                        { codec_axis_chart(s, CodecAxis::Size) }
                    }
                }
            }

            div { class: "bench-codec-method",
                "Method: each value space (int/float arrays, point & record lists, strings) is encoded by every codec from the same seeded-random data; size is exact bytes (no envelope), encode/decode are ns per whole-message op, taken with a warm-up. Cap'n Proto is read zero-copy (its best). Bars are linear within each chart — every length is its exact fraction of the slowest, so the gap you see is the true ratio. Measured on {d.metadata.cpu}, {d.metadata.os}."
            }
        }
    }
}
