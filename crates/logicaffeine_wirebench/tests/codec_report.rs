//! Lock-ins for the JSON codec report the web Benchmarks page bakes in.
//!
//! `build_report` runs the same fair head-to-head the stdout table prints, but returns it
//! as a structured `CodecReport` (the shape serialized to `benchmarks/results/latest-codec.json`).
//! These tests guarantee the report (a) survives a JSON round-trip, (b) keeps the provable
//! size guarantee on every fair workload, (c) carries the random-access axis, and (d) matches
//! the field contract the front-end deserializes against. Size assertions are deterministic;
//! the speed assertion (capnp gated) uses the same-run ratio the superiority lock-ins use.

use logicaffeine_wirebench::{build_report, CodecReport};

/// Tiny iter count: the SIZE fields are deterministic regardless of timing, so a small
/// budget keeps the report-shape tests fast while still exercising every scenario.
const TEST_ITERS: u32 = 50;

#[test]
fn report_round_trips_through_json() {
    let report = build_report(TEST_ITERS);
    let json = serde_json::to_string(&report).expect("report must serialize");
    let back: CodecReport = serde_json::from_str(&json).expect("report must deserialize");
    assert_eq!(report, back, "report must survive a JSON round-trip unchanged");
    assert_eq!(report.schema_version, 2, "schema_version is the page's compatibility gate");
    assert!(report.scenarios.iter().any(|s| s.kind == "fair"), "must have ≥1 fair scenario");
}

/// The provable, generalizable size guarantee — mirrors `superiority.rs::we_are_never_beaten
/// _by_postcard_on_size`. postcard has the tightest framing of every competitor, so "never
/// beaten by postcard" is the meaningful, defensible floor. The page's headline ("smallest
/// wire on N/N workloads") is the stronger vs-every-competitor claim, guarded separately
/// against the real committed data in the web crate.
#[test]
fn best_never_beaten_by_postcard_on_size_in_fair_scenarios() {
    let report = build_report(TEST_ITERS);
    let fair: Vec<_> = report.scenarios.iter().filter(|s| s.kind == "fair").collect();
    assert!(!fair.is_empty(), "expected fair scenarios");
    for s in fair {
        let best = s
            .rows
            .iter()
            .find(|r| r.codec == "logos (BEST: all knobs)")
            .unwrap_or_else(|| panic!("fair scenario '{}' missing the BEST row", s.id));
        let postcard = s
            .rows
            .iter()
            .find(|r| r.codec == "postcard")
            .unwrap_or_else(|| panic!("fair scenario '{}' missing the postcard row", s.id));
        assert!(
            best.size <= postcard.size,
            "scenario '{}': BEST {}B must be ≤ postcard {}B (never beaten on minimal framing)",
            s.id,
            best.size,
            postcard.size
        );
    }
}

/// The fair-fight contract: every COMPETITOR row carries a `fair_size` (its smallest size once
/// granted the same compression LOGOS bakes in), and it is never larger than the raw `size` — so a
/// compressed-vs-compressed comparison is possible and never inflates a rival. LOGOS rows leave it
/// `None` (the all-knobs winner's `size` is already its fair, compression-shopped size).
#[test]
fn competitor_rows_carry_a_fair_compressed_size() {
    let report = build_report(TEST_ITERS);
    let mut checked = 0;
    for s in report.scenarios.iter().filter(|s| s.kind == "fair") {
        for r in s.rows.iter().filter(|r| !r.codec.starts_with("logos")) {
            let fair = r.fair_size.unwrap_or_else(|| panic!("'{}' competitor '{}' missing fair_size", s.id, r.codec));
            assert!(fair <= r.size, "'{}' '{}': fair_size {fair} must be ≤ raw size {}", s.id, r.codec, r.size);
            assert!(fair > 0, "'{}' '{}': fair_size must be positive", s.id, r.codec);
            checked += 1;
        }
    }
    assert!(checked > 0, "expected competitor rows with fair_size");
}

/// The user-requested axis: Cap'n Proto's home turf is "open + read one field," and we ship
/// a random-access scenario that measures exactly that for every codec into `read_one_ns`.
#[test]
fn random_access_scenario_carries_read_one_ns() {
    let report = build_report(TEST_ITERS);
    let ra = report
        .scenarios
        .iter()
        .find(|s| s.kind == "random_access")
        .expect("a random_access scenario must exist");
    assert!(
        ra.rows.iter().all(|r| r.read_one_ns.is_some()),
        "every random_access row must populate read_one_ns"
    );
    assert!(
        ra.rows.iter().any(|r| r.codec.starts_with("logos")),
        "random_access must include a LOGOS row"
    );
}

/// Schema-shape contract: the serialized JSON must carry exactly the field names/types the
/// front-end `CodecData` deserializes (the page can't be imported here, so assert on the wire).
#[test]
fn json_shape_matches_front_end_contract() {
    let report = build_report(TEST_ITERS);
    let v: serde_json::Value = serde_json::to_value(&report).unwrap();
    assert!(v["schema_version"].is_number());
    assert!(v["iters"].is_number());
    let md = &v["metadata"];
    for k in ["date", "commit", "logos_version", "cpu", "os", "versions", "features"] {
        assert!(!md[k].is_null(), "metadata.{k} missing");
    }
    let s0 = &v["scenarios"][0];
    for k in ["id", "title", "n", "kind", "rows"] {
        assert!(!s0[k].is_null(), "scenario.{k} missing");
    }
    let r0 = &s0["rows"][0];
    for k in ["codec", "size", "enc_ns", "dec_ns"] {
        assert!(!r0[k].is_null(), "row.{k} missing");
    }
    // The all-knobs winner in a fair scenario must carry the `chosen` config object with the
    // fields the front-end reads — the data behind "what actually won".
    let fair0 = v["scenarios"].as_array().unwrap().iter().find(|s| s["kind"] == "fair").unwrap();
    let best = fair0["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["codec"] == "logos (BEST: all knobs)")
        .expect("fair scenario carries the BEST row");
    let chosen = &best["chosen"];
    assert!(chosen.is_object(), "BEST row must carry a `chosen` object");
    for k in ["numerics", "floats", "compression", "columns", "summary"] {
        assert!(!chosen[k].is_null(), "chosen.{k} missing");
    }
    assert!(chosen["columns"].as_array().is_some_and(|c| !c.is_empty()), "chosen.columns must be non-empty");
}

/// The "what actually won" contract: every fair / adversarial / showcase all-knobs winner names
/// the dials it chose, its `columns` are non-empty and drawn from the codec's own vocabulary, its
/// `summary` states the compression, and a record-list shape (`points`, `records`) reports one
/// `"field: encoding"` per field — tying the description to the winning bytes' real shape.
#[test]
fn best_rows_report_the_dials_that_won() {
    use std::collections::HashSet;
    let report = build_report(TEST_ITERS);
    // The encodings `describe_columns` can name (mirrors marshal::column_tag_name). A record
    // column reads `"field: encoding"`; a single column is the bare encoding.
    let known: HashSet<&str> = [
        "varint", "fixed (memcpy)", "group-varint", "fixed-aligned", "affine (base,stride,n)",
        "delta", "delta-of-delta", "FOR bit-pack", "run-length", "dictionary", "polynomial",
        "geometric", "periodic", "sparse", "generator", "byte column", "linear-recurrence",
        "LFSR", "FCSR", "memcpy floats", "xor-delta floats", "constant floats", "affine floats",
        "sparse floats", "periodic floats", "geometric floats", "aligned floats", "bit-packed bools",
        "periodic bools", "run-length bools", "flat strings", "templated strings",
        "front-coded strings", "affix strings", "dictionary strings", "int set (column menu)",
        "string set (front-coded)", "int-keyed map (columnar)", "value",
    ]
    .into_iter()
    .collect();
    let compression_words: HashSet<&str> = ["none", "deflate", "lz4", "zstd"].into_iter().collect();

    for s in report.scenarios.iter().filter(|s| matches!(s.kind.as_str(), "fair" | "adversarial" | "showcase" | "structural")) {
        let best = s
            .rows
            .iter()
            .find(|r| r.codec == "logos (BEST: all knobs)")
            .unwrap_or_else(|| panic!("scenario '{}' missing the BEST row", s.id));
        let chosen = best.chosen.as_ref().unwrap_or_else(|| panic!("scenario '{}' BEST row missing chosen", s.id));
        assert!(!chosen.columns.is_empty(), "scenario '{}': chosen.columns must be non-empty", s.id);
        assert!(compression_words.contains(chosen.compression.as_str()), "scenario '{}': bad compression word '{}'", s.id, chosen.compression);
        for col in &chosen.columns {
            let enc = col.rsplit(": ").next().unwrap_or(col); // strip an optional "field: " prefix
            assert!(known.contains(enc), "scenario '{}': column encoding '{col}' not in the codec vocabulary", s.id);
        }
        assert!(chosen.summary.contains("compress") || chosen.summary.contains("no compression"),
            "scenario '{}': summary must state the compression: '{}'", s.id, chosen.summary);
    }

    // Record-list scenarios must report one "field: encoding" per struct field.
    let by_id = |id: &str| report.scenarios.iter().find(|s| s.id == id).and_then(|s| s.rows.iter().find(|r| r.codec == "logos (BEST: all knobs)")).and_then(|r| r.chosen.clone());
    if let Some(c) = by_id("points") {
        assert_eq!(c.columns.len(), 2, "points is a 2-field record list");
        assert!(c.columns.iter().all(|col| col.contains(": ")), "points columns must be field-labelled: {:?}", c.columns);
    }
    if let Some(c) = by_id("records") {
        assert_eq!(c.columns.len(), 3, "records is a 3-field record list");
        assert!(c.columns.iter().all(|col| col.contains(": ")), "records columns must be field-labelled: {:?}", c.columns);
    }
}

/// When the capnp toolchain is present, prove the random-access win is real in the report
/// itself: the fastest LOGOS open+read-one-field beats Cap'n Proto's. (Same-run ratio, so
/// machine load slows both equally.)
#[cfg(feature = "capnproto")]
#[test]
fn logos_beats_capnp_on_random_access_in_report() {
    let report = build_report(4000);
    let ra = report
        .scenarios
        .iter()
        .find(|s| s.kind == "random_access")
        .expect("a random_access scenario must exist");
    let logos = ra
        .rows
        .iter()
        .filter(|r| r.codec.starts_with("logos"))
        .filter_map(|r| r.read_one_ns)
        .fold(f64::INFINITY, f64::min);
    let capnp = ra
        .rows
        .iter()
        .find(|r| r.codec == "capnproto")
        .and_then(|r| r.read_one_ns)
        .expect("capnp row must carry read_one_ns under --features capnproto");
    assert!(
        logos < capnp,
        "LOGOS random-access {logos:.0}ns must beat capnp {capnp:.0}ns (cheaper open)"
    );
}
