//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SPATIAL NEUTRAL v2 LOCK — the `logos.spatial.neutral` v2 envelope is a checked-in, schema-versioned
//! fixture that regenerates BYTE-EQUAL from a fresh lowering; it round-trips; its certificate rechecks
//! against the program it carries; and tampering with any scheduled op inside it is refused.
//!
//! The artifact under test: `compile_to_spatial(gemm 512×256×128, sn50_rdu())` lowered by
//! `RduSpatialBackend::for_chip(RduChip::Sn50)` — the SDK-gated neutral placement a SambaNova engineer
//! receives. The fixture `tests/fixtures/spatial_neutral_v2_gemm.json` is GENERATED, never typed:
//! regenerate it only with `LOGOS_UPDATE_FIXTURES=1`; without the flag a drift prints a diff and fails.
//!
//! ★ MUTATION PROBES (each run once against this tree; the named test went RED, the impl was restored
//! byte-identical — every line below is what the probe ACTUALLY did and what it ACTUALLY reported):
//!   - `rdu.rs`: `NEUTRAL_VERSION = 3` → `the_fixture_is_byte_equal_to_a_fresh_lowering` prints the first
//!     differing byte (44) with its context on both sides;
//!   - `rdu.rs`: serialize a `sections` entry through a `HashMap` (iteration order) →
//!     `lowering_is_deterministic_across_two_processes_worth_of_state`;
//!   - the lock itself: hand `compare` the same bytes on both sides (`if true { return Ok(()) }`) →
//!     `the_byte_comparison_is_not_vacuous` (`a flipped digit is a difference`);
//!   - `certificate.rs`: delete the digest check from `recheck` →
//!     `the_digest_catches_the_tampers_the_marked_graph_cannot_see`.
//!
//! THE PROBE THAT DID NOT FIRE, and what was done about it. The designed probe — delete the digest check
//! from `recheck`, expecting `tampering_any_scheduled_op_in_the_fixture_is_refused` to go red — left that
//! test GREEN, and so did two alternatives (neuter the buffer-balance bound; accept a token-free liveness
//! witness). The reason, measured: a marked graph has one transition per program-order event chained
//! through `Pc` places, so ALL 54 single-op drops the sweep performs shorten that chain — `graph !=
//! self.graph` refuses every one of them on its own and the digest is redundant THERE. Rather than delete a
//! digest that is plainly load-bearing, the missing case was written: `the_digest_catches_the_tampers_the_
//! marked_graph_cannot_see` sweeps count-PRESERVING tampers (a re-addressed `ReadTiles`/`WriteTiles`, a
//! `ReadBarrier` rewritten to a `WriteBarrier`) that rebuild a byte-identical graph — 12 of them, all
//! refused, and for the ordering rewrites the digest is the ONLY gate the shipped artifact has. The
//! original probe now fires on that test.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_neutral_v2_lock`
//! Regenerate: `LOGOS_UPDATE_FIXTURES=1 cargo nextest run -p logicaffeine-tests --test spatial_neutral_v2_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use std::collections::BTreeMap;
use std::path::PathBuf;

use logicaffeine_compile::spatial::certificate::{
    build_marked_graph, certify_choreography, certify_external_spatial, schedule_digest,
};
use logicaffeine_compile::spatial::ir::{ChoreoOp, SpatialProgram};
use logicaffeine_compile::spatial::lower::rdu::{
    parse_neutral, RduSpatialBackend, NEUTRAL_SCHEMA, NEUTRAL_VERSION,
};
use logicaffeine_compile::spatial::lower::{NeutralArtifact, SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::SpatialTarget;
use serde_json::Value;
use spatial_support::*;

const FIXTURE: &str = "crates/logicaffeine_tests/tests/fixtures/spatial_neutral_v2_gemm.json";
const UPDATE_FLAG: &str = "LOGOS_UPDATE_FIXTURES";

fn fixture_path() -> PathBuf {
    repo_root().join(FIXTURE)
}

/// The fixture's GEMM: the smallest standard shape whose int8 product — accumulated in the SN50's INT32 — does not
/// fit one site, so the envelope carries a CROSS-SITE choreography for the tamper sweeps to bite on.
const GEMM: (usize, usize, usize) = (512, 256, 128);

fn program() -> SpatialProgram {
    compile_to_spatial(
        &gemm_program(GEMM.0, GEMM.1, GEMM.2),
        &SpatialTarget::sn50_rdu(),
    )
    .expect("the GEMM compiles for the SN50")
}

fn lower(p: &SpatialProgram) -> NeutralArtifact {
    match RduSpatialBackend::for_chip(RduChip::Sn50)
        .lower(p)
        .expect("the SN50 lane lowers a verified program")
    {
        SpatialArtifact::SdkGated(a) => a,
        other => panic!("the RDU lane is SDK-gated, got {other:?}"),
    }
}

fn fixture() -> String {
    std::fs::read_to_string(fixture_path()).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\n\nno fixture yet — generate it once with {UPDATE_FLAG}=1",
            fixture_path().display()
        )
    })
}

/// Byte comparison with a readable verdict: the first differing offset with its context on both sides,
/// then the first differing line of the pretty-printed JSON when both sides parse.
fn compare(expected: &str, actual: &str) -> Result<(), String> {
    if expected == actual {
        return Ok(());
    }
    let eb = expected.as_bytes();
    let ab = actual.as_bytes();
    let at = eb
        .iter()
        .zip(ab)
        .position(|(x, y)| x != y)
        .unwrap_or(eb.len().min(ab.len()));
    let window = |s: &str| {
        let lo = at.saturating_sub(80);
        let hi = (at + 80).min(s.len());
        let lo = (lo..=at)
            .rev()
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(0);
        let hi = (hi..s.len())
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(s.len());
        s[lo..hi].replace('\n', "\\n")
    };
    let mut report = format!(
        "the fixture and the fresh lowering differ at byte {at} (fixture {} bytes, fresh {} bytes)\n  fixture: …{}…\n  fresh:   …{}…\n",
        eb.len(),
        ab.len(),
        window(expected),
        window(actual)
    );
    if let (Ok(e), Ok(a)) = (
        serde_json::from_str::<Value>(expected),
        serde_json::from_str::<Value>(actual),
    ) {
        let ep = serde_json::to_string_pretty(&e).unwrap();
        let ap = serde_json::to_string_pretty(&a).unwrap();
        for (n, (el, al)) in ep.lines().zip(ap.lines()).enumerate() {
            if el != al {
                report.push_str(&format!(
                    "  first differing pretty line {n}:\n    fixture: {el}\n    fresh:   {al}\n"
                ));
                break;
            }
        }
        if ep.lines().count() != ap.lines().count() {
            report.push_str(&format!(
                "  pretty line counts: fixture {} vs fresh {}\n",
                ep.lines().count(),
                ap.lines().count()
            ));
        }
    }
    report.push_str(&format!(
        "regenerate deliberately with {UPDATE_FLAG}=1 if the envelope changed on purpose\n"
    ));
    Err(report)
}

#[test]
fn the_fixture_is_byte_equal_to_a_fresh_lowering() {
    let fresh = lower(&program()).json;
    if std::env::var(UPDATE_FLAG)
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        std::fs::create_dir_all(fixture_path().parent().unwrap()).unwrap();
        std::fs::write(fixture_path(), &fresh).unwrap();
        println!("wrote {} ({} bytes)", fixture_path().display(), fresh.len());
    }
    compare(&fixture(), &fresh).unwrap_or_else(|diff| panic!("{diff}"));
}

#[test]
fn the_schema_is_versioned_two_and_names_its_lane_target_and_sections() {
    let v: Value = serde_json::from_str(&fixture()).expect("the fixture is JSON");
    assert_eq!(v["schema"], NEUTRAL_SCHEMA);
    assert_eq!(v["version"], 2);
    assert_eq!(NEUTRAL_VERSION, 2);
    assert_eq!(v["lane"], "spatial");
    let t = &v["target"];
    assert_eq!(t["name"], "sn50-rdu");
    assert_eq!(t["chip"], "sn50");
    assert_eq!(t["part"], "SN50");
    assert_eq!(
        t["sites"]["x"].as_u64().unwrap() * t["sites"]["y"].as_u64().unwrap(),
        1040
    );
    assert_eq!(t["silicon"]["compute_tiles"], 1040);
    assert_eq!(t["silicon"]["node_nm"], 5);
    assert_eq!(t["mem"]["sram_kib"], 432 * 1024);
    assert_eq!(t["mem"]["hbm_gib"], 64);
    assert_eq!(t["mem"]["ddr_gib"], 512);
    assert_eq!(
        t["modeled"],
        serde_json::json!([
            "sites",
            "acc_tiles",
            "tile",
            "local_mem_bytes",
            "silicon.compute_tiles",
            "silicon.mem_tiles"
        ])
    );
    assert_eq!(t["unit_model"]["pcu_stages"]["value"], 6);
    assert_eq!(t["unit_model"]["pcu_stages"]["provenance"], "Modeled");
    assert_eq!(t["unit_model"]["pmu"]["value"]["banks"], 16);
    assert!(
        t["unit_model"]["credit"].is_null(),
        "undeclared credit latencies are null, never zero"
    );
    assert_eq!(
        t["precisions"]["witnessed"],
        serde_json::json!(["Int8", "Bf16"])
    );
    assert_eq!(
        t["precisions"]["chip_supported_witness_pending"],
        serde_json::json!(["Fp8", "Fp32"])
    );
    assert_eq!(t["chips_per_node"], 8);
    assert_eq!(t["scale_up"]["tb_s_per_chip"], 2.0);
    assert_eq!(t["scale_up"]["intra_node_links"], 7);
    assert_eq!(t["peak_bf16_tflops"], 1600);
    assert_eq!(t["peak_fp8_tflops"], 3200);
    for k in ["sections", "state", "gathers"] {
        assert!(
            v[k].as_array().is_some_and(Vec::is_empty),
            "`{k}` is an empty array until the composed form lands"
        );
    }
    assert!(
        v["ii_report"].as_object().is_some_and(|o| o.is_empty()),
        "`ii_report` is an empty object until the composed form lands"
    );
    assert!(v["program"].is_object());
    assert!(v["certificate"]["schedule_digest"].is_number());
    // The key order of the envelope is the schema's, not a hash map's: schema, version, lane, target first.
    let text = fixture();
    let pos = |k: &str| {
        text.find(&format!("\"{k}\":"))
            .unwrap_or_else(|| panic!("no `{k}` key"))
    };
    assert!(
        pos("schema") < pos("version")
            && pos("version") < pos("lane")
            && pos("lane") < pos("target")
            && pos("target") < pos("program")
            && pos("program") < pos("certificate")
    );
}

#[test]
fn the_fixture_round_trips_and_its_certificate_rechecks_against_the_parsed_program() {
    let text = fixture();
    let parsed = parse_neutral(&text).expect("the fixture parses back into a program");
    let v: Value = serde_json::from_str(&text).unwrap();
    let digest = v["certificate"]["schedule_digest"]
        .as_u64()
        .expect("a digest");
    assert_eq!(
        schedule_digest(&parsed),
        digest,
        "the digest in the envelope binds to the schedule in the envelope"
    );
    let cert = certify_choreography(&parsed).expect("the parsed program certifies afresh");
    assert_eq!(cert.schedule_digest, digest);
    assert!(cert.recheck(&parsed));
    let fresh = lower(&program());
    assert!(
        fresh.certificate.recheck(&parsed),
        "the shipped certificate rechecks against what a reader parses"
    );
    assert_eq!(fresh.certificate.schedule_digest, digest);
    let mut expect = program();
    expect.certificate = None;
    assert_eq!(
        parsed, expect,
        "round-trip: the parsed program is the program"
    );
    assert_eq!(v["certificate"]["places"], cert.places());
    assert_eq!(v["certificate"]["transitions"], cert.transitions());
}

/// Ops whose removal the marked graph and the dry run see: the certificate must be refused afresh.
/// Memory-ordering guards (`ReadBarrier`, `WriteBarrier`, `Flush`) are arch ordering the marked graph
/// does not model; for them the SHIPPED certificate still fails (the digest binds every byte).
fn load_bearing(op: &ChoreoOp) -> bool {
    !matches!(
        op,
        ChoreoOp::ReadBarrier | ChoreoOp::WriteBarrier | ChoreoOp::Flush { .. }
    )
}

/// ★ THE SWEEPS' PRECONDITION, BY NAME. A tamper sweep over a one-site schedule has almost nothing to tamper with —
/// the fixture once fell to one site when its result became INT32 and the sweeps failed on an op COUNT, not on
/// what they check. The fixture must place on several sites.
#[test]
fn the_fixture_is_a_cross_site_choreography() {
    let parsed = parse_neutral(&fixture()).unwrap();
    let sites = parsed
        .placement
        .as_ref()
        .expect("a placed program")
        .sites()
        .len();
    assert!(sites >= 2, "the fixture's GEMM {GEMM:?} must place on several SN50 sites for the tamper sweeps to mean anything, saw {sites}");
}

#[test]
fn tampering_any_scheduled_op_in_the_fixture_is_refused() {
    let text = fixture();
    let parsed = parse_neutral(&text).unwrap();
    let cert = certify_choreography(&parsed).unwrap();
    let ops = enumerate_ops(parsed.schedule.as_ref().unwrap());
    assert!(
        ops.len() >= 50,
        "a placed GEMM schedules many ops, saw {}",
        ops.len()
    );
    let mut by_kind: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, (_, op)) in ops.iter().enumerate() {
        let kind = format!("{:?}", std::mem::discriminant(op));
        let name = format!("{op:?}");
        let name = name
            .split(|c: char| !c.is_alphanumeric())
            .next()
            .unwrap()
            .to_string();
        by_kind.entry(format!("{name}{kind}")).or_default().push(i);
    }
    assert!(
        by_kind.len() >= 6,
        "several op kinds are scheduled: {:?}",
        by_kind.keys().collect::<Vec<_>>()
    );
    // Every op: the shipped certificate refuses the tampered program (the digest binds it).
    for (path, _) in &ops {
        let broken = without_op(&parsed, path);
        assert!(
            !cert.recheck(&broken),
            "dropping the op at {path:?} must fail the shipped certificate"
        );
    }
    // Every KIND, at its first and last occurrence, plus a deterministic stride: fresh certification is refused too.
    let mut probed = 0usize;
    let mut refused_kinds: Vec<String> = Vec::new();
    for (kind, idxs) in &by_kind {
        let mut picks = vec![idxs[0], *idxs.last().unwrap()];
        picks.extend(idxs.iter().copied().step_by(97).skip(1));
        picks.sort_unstable();
        picks.dedup();
        for i in picks {
            let (path, op) = &ops[i];
            if !load_bearing(op) {
                continue;
            }
            let broken = without_op(&parsed, path);
            assert!(certify_external_spatial(&broken, &parsed).is_none(), "{kind}: dropping the op at {path:?} ({op:?}) must be refused at the import-and-certify seam");
            probed += 1;
        }
        refused_kinds.push(kind.clone());
    }
    assert!(probed >= 12, "the sweep probed only {probed} ops");
    println!("tamper sweep: {} ops digest-refused, {probed} load-bearing drops refused afresh across {} kinds", ops.len(), refused_kinds.len());
}

/// A tamper that keeps the op COUNT and rewrites only a field the marked graph does not model: a
/// re-addressed tile transfer, or a memory-ordering guard rewritten to the other one. Both are real
/// corruptions — the first makes the engine read the wrong tile, the second inverts the arch ordering a
/// reader is told to expect — and neither moves a single place or transition of the graph.
fn count_preserving_tamper(op: &ChoreoOp) -> Option<ChoreoOp> {
    match op {
        ChoreoOp::ReadTiles {
            tensor,
            at,
            block_h,
            block_w,
            stride_w,
            stride_h,
            into,
        } => {
            let mut at = at.clone();
            at.base += 1;
            Some(ChoreoOp::ReadTiles {
                tensor: *tensor,
                at,
                block_h: *block_h,
                block_w: *block_w,
                stride_w: *stride_w,
                stride_h: *stride_h,
                into: *into,
            })
        }
        ChoreoOp::WriteTiles {
            from,
            tensor,
            at,
            block_h,
            block_w,
            stride_w,
            stride_h,
        } => {
            let mut at = at.clone();
            at.base += 1;
            Some(ChoreoOp::WriteTiles {
                from: *from,
                tensor: *tensor,
                at,
                block_h: *block_h,
                block_w: *block_w,
                stride_w: *stride_w,
                stride_h: *stride_h,
            })
        }
        ChoreoOp::ReadBarrier => Some(ChoreoOp::WriteBarrier),
        ChoreoOp::WriteBarrier => Some(ChoreoOp::ReadBarrier),
        _ => None,
    }
}

/// WHAT THE SHIPPED DIGEST BUYS, isolated — the reason `recheck` opens with it.
///
/// The marked graph is a STRUCTURAL certificate: one transition per program-order event, chained through
/// `Pc` places. Every drop the sweep above performs shortens that chain, so `graph != self.graph` refuses
/// on its own and the digest adds nothing there (measured: 0 of 54 drops leave the graph equal). Deleting
/// the digest line from `recheck` therefore leaves that sweep GREEN — which is exactly why this test
/// exists: it holds the tampers the graph is blind to, where the digest is the gate, and for the
/// memory-ordering rewrite the ONLY gate the shipped artifact has (fresh certification accepts it — the
/// marked graph does not model arch ordering, as `load_bearing` above says out loud).
#[test]
fn the_digest_catches_the_tampers_the_marked_graph_cannot_see() {
    let parsed = parse_neutral(&fixture()).unwrap();
    let cert = certify_choreography(&parsed).unwrap();
    let ops = enumerate_ops(parsed.schedule.as_ref().unwrap());
    let mut blind: BTreeMap<String, usize> = BTreeMap::new();
    let mut digest_only = 0usize;
    for (path, op) in &ops {
        let Some(swapped) = count_preserving_tamper(op) else {
            continue;
        };
        let broken = with_op_replaced(&parsed, path, swapped.clone());
        let graph = build_marked_graph(&broken)
            .expect("a count-preserving tamper still builds a marked graph");
        assert_eq!(
            &graph, cert.proof.whole_graph().expect("the single-op fixture is proved whole"),
            "{path:?}: rewriting {op:?} to {swapped:?} moved the marked graph — if the graph learned to model this, \
             the digest's unique contribution shrank and this test must be re-derived against what is left"
        );
        assert!(!cert.recheck(&broken), "{path:?}: the shipped certificate must refuse {op:?} rewritten to {swapped:?} — the digest binds every byte of the schedule");
        let name = format!("{op:?}");
        *blind
            .entry(
                name.split(|c: char| !c.is_alphanumeric())
                    .next()
                    .unwrap()
                    .to_string(),
            )
            .or_default() += 1;
        if !load_bearing(op) && certify_external_spatial(&broken, &parsed).is_some() {
            digest_only += 1;
        }
    }
    let total: usize = blind.values().sum();
    assert!(
        total >= 12,
        "the graph-blind sweep found only {total} count-preserving tampers: {blind:?}"
    );
    assert!(
        blind.len() >= 3,
        "the sweep covers several op kinds, saw {blind:?}"
    );
    for k in ["ReadTiles", "WriteTiles", "ReadBarrier", "WriteBarrier"] {
        assert!(
            blind.contains_key(k),
            "{k} is scheduled by the GEMM and must be swept: {blind:?}"
        );
    }
    assert!(digest_only >= 1, "at least one tamper must be caught by the digest ALONE (fresh certification accepts it), saw {digest_only}");
    println!("digest-only sweep: {total} graph-blind tampers refused across {:?}; {digest_only} of them invisible to fresh certification", blind.keys().collect::<Vec<_>>());
}

#[test]
fn lowering_is_deterministic_across_two_processes_worth_of_state() {
    // Two independent compilations, with an unrelated lowering between them to perturb every allocator and
    // interner the pipeline touches: identical bytes.
    let a = lower(&program()).json;
    let _other = lower(
        &compile_to_spatial(&gemm_program(256, 256, 128), &SpatialTarget::sn50_rdu()).unwrap(),
    );
    let _sn40l = RduSpatialBackend::for_chip(RduChip::Sn40l)
        .lower(&compile_to_spatial(&gemm_program(64, 64, 64), &SpatialTarget::sn40l_rdu()).unwrap())
        .unwrap();
    let b = lower(&program()).json;
    assert_eq!(a, b, "the same program lowers to the same bytes");
    // A parsed-and-reserialized program is the same JSON value: no map iteration order leaks into the envelope.
    let v: Value = serde_json::from_str(&a).unwrap();
    let reparsed: Value =
        serde_json::from_str(&serde_json::to_string(&parse_neutral(&a).unwrap()).unwrap()).unwrap();
    assert_eq!(v["program"], reparsed);
    // And the checked-in fixture was produced by a different process entirely.
    assert_eq!(
        fixture(),
        a,
        "the process that wrote the fixture agrees with this one"
    );
}

#[test]
fn the_byte_comparison_is_not_vacuous() {
    let text = fixture();
    assert!(
        text.len() > 4096,
        "the fixture is a real artifact, not a stub ({} bytes)",
        text.len()
    );
    assert!(text.contains("\"sn50-rdu\"") && text.contains("\"version\":2"));
    compare(&text, &text).expect("equal bytes compare equal");
    // One flipped digit in the digest is caught and located.
    let digest = format!(
        "\"schedule_digest\":{}",
        serde_json::from_str::<Value>(&text).unwrap()["certificate"]["schedule_digest"]
    );
    let at = text.find(&digest).expect("the digest is in the text");
    let mut mutated = text.clone();
    let flipped = mutated[at + digest.len() - 1..].chars().next().unwrap();
    let replacement = if flipped == '0' { '1' } else { '0' };
    mutated.replace_range(
        at + digest.len() - 1..at + digest.len(),
        &replacement.to_string(),
    );
    let err = compare(&text, &mutated).expect_err("a flipped digit is a difference");
    assert!(
        err.contains(&format!("differ at byte {}", at + digest.len() - 1)),
        "{err}"
    );
    assert!(
        err.contains("schedule_digest"),
        "the context shows where: {err}"
    );
    // A structural change is caught through the pretty diff as well.
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["target"]["chip"] = Value::from("sn40l");
    let err = compare(&text, &v.to_string()).expect_err("a changed chip is a difference");
    assert!(err.contains("first differing pretty line"), "{err}");
    assert!(err.contains(UPDATE_FLAG));
}
