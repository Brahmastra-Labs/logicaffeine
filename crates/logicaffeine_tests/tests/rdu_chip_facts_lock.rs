//! rdu_chip_facts_lock — every RDU chip fact is its pinned source's figure, in the unit the source uses (CHIP-1 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`, section 4.2).
//!
//! A published fact's citation opens with the bibliography key of its source and the figure as that source states it
//! (`[DIAYN26] 1.6 TB/s of peak HBM bandwidth a socket`), and the figure is in the source's pinned text
//! (`benchmarks/samba_papers.tsv`). A derived fact names the keys of the figures it is computed from and writes the
//! arithmetic. A capacity is read as the vendor counts it, in powers of two: SambaNova states 1040 PMUs of 512 KB and
//! 520 MB of SRAM, which agree only when a MB is 1024 KB. A figure two sources state differently is carried once, and
//! the other is kept in `others` with its source, so no published figure is lost.
//!
//! Without the captures (`external/papers/`, tool probe `samba-papers`) the checks against a source's text record an
//! honest skip; the keys, the readings and the arithmetic are checked always.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_chip_facts_lock`

mod common;
#[path = "common/samba_papers.rs"]
mod samba_papers;

use logicaffeine_compile::spatial::rdu_chip::{Provenance, RduChip, RduChipFacts};
use samba_papers::{appears, pins, source_texts};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The bibliography keys a citation names: every `[KEY]` of upper-case letters and digits.
fn keys(cite: &str) -> Vec<&str> {
    cite.match_indices('[')
        .filter_map(|(i, _)| {
            let rest = &cite[i + 1..];
            let key = &rest[..rest.find(']')?];
            (!key.is_empty() && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())).then_some(key)
        })
        .collect()
}

/// The first number of `s`, as written (`1.6`, `432`, `1,040` read `1040`).
fn first_number(s: &str) -> Option<String> {
    let s = samba_papers::ungroup(s);
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let tail = &s[start..];
    let end = tail
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || (c == '.' && tail[i + 1..].starts_with(|d: char| d.is_ascii_digit()))))
        .map_or(tail.len(), |(i, _)| i);
    Some(tail[..end].to_string())
}

/// The figure a published fact's citation states: the text after its key.
fn stated(cite: &str) -> &str {
    cite.split_once("] ").map_or(cite, |(_, rest)| rest)
}

/// Every `Fact` object of the serialized facts, by path: its value, provenance and citation.
fn facts_of(facts: &RduChipFacts) -> Vec<(String, Value, String, String)> {
    fn walk(v: &Value, path: &mut Vec<String>, out: &mut Vec<(String, Value, String, String)>) {
        match v {
            Value::Object(m) if m.contains_key("value") && m.contains_key("provenance") && m.contains_key("cite") => out.push((
                path.join("."),
                m["value"].clone(),
                m["provenance"].as_str().unwrap_or("").to_string(),
                m["cite"].as_str().unwrap_or("").to_string(),
            )),
            Value::Object(m) => {
                for (k, c) in m {
                    path.push(k.clone());
                    walk(c, path, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(&serde_json::to_value(facts).expect("the facts serialize"), &mut Vec::new(), &mut out);
    out
}

/// A capacity as a citation states it, in bytes, each prefix read in powers of two: `432 MB` is 432 · 2^20.
fn capacity_bytes(stated: &str) -> u64 {
    let n = first_number(stated).unwrap_or_else(|| panic!("no figure in {stated:?}"));
    let after = &stated[stated.find(&n[..1]).unwrap()..];
    let unit = after.split_whitespace().nth(1).unwrap_or_else(|| panic!("no unit after {n} in {stated:?}"));
    let shift = match unit.trim_end_matches(|c: char| !c.is_ascii_alphabetic()) {
        "KB" | "KiB" => 10,
        "MB" | "MiB" => 20,
        "GB" | "GiB" => 30,
        "TB" | "TiB" => 40,
        other => panic!("{other:?} in {stated:?} is not a capacity unit"),
    };
    (n.parse::<f64>().unwrap() * (1u64 << shift) as f64) as u64
}

#[test]
fn every_capacity_is_read_as_the_vendor_counts() {
    // The evidence: 1040 PMUs of 512 KB are the 520 MB the vendor states only when a MB is 1024 KB.
    let sn40l = RduChipFacts::sn40l();
    let pmu = sn40l.pmu_kib.expect("the SN40L publishes its PMU capacity");
    assert_eq!((pmu.value, pmu.provenance), (512, Provenance::Published), "{}", pmu.cite);
    assert_eq!(sn40l.pmu_count.value * pmu.value, sn40l.sram_mib.value * 1024, "1040 PMUs × 512 KB = 520 MB, in powers of two");
    assert_ne!(sn40l.pmu_count.value * pmu.value * 1000, sn40l.sram_mib.value * 1_000_000, "and not in powers of ten");
    for facts in RduChip::ALL.map(RduChip::facts) {
        for (what, fact, bytes) in [
            ("SRAM", facts.sram_mib, (facts.sram_mib.value as u64) << 20),
            ("HBM", facts.hbm_gib, (facts.hbm_gib.value as u64) << 30),
            ("DDR", facts.ddr_gib_max, (facts.ddr_gib_max.value as u64) << 30),
        ] {
            assert!(fact.cite.contains("powers of two"), "{:?} {what}: the citation names the reading: {}", facts.chip, fact.cite);
            assert_eq!(capacity_bytes(stated(fact.cite)), bytes, "{:?} {what}: {} is the value read in powers of two", facts.chip, fact.cite);
        }
        assert_eq!(facts.sram_bytes() as u64, (facts.sram_mib.value as u64) << 20, "{:?}: the SRAM in bytes", facts.chip);
    }
}

#[test]
fn the_sn40l_hbm_bandwidth_is_the_published_1_6_tb_s() {
    let sn40l = RduChipFacts::sn40l();
    let hbm = sn40l.hbm_gbps();
    assert_eq!((hbm.value, hbm.provenance), (1600.0, Provenance::Published), "{}", hbm.cite);
    assert!(stated(hbm.cite).starts_with("1.6 TB/s"), "the figure as the source states it: {}", hbm.cite);
    let earlier: Vec<&str> = sn40l.others.iter().filter(|o| o.fact == "hbm_bandwidth_gbps").map(|o| o.stated).collect();
    for figure in ["1.8 TB/s", "about 2 TB/s"] {
        assert!(earlier.contains(&figure), "the SN40L paper's {figure} is kept beside it: {earlier:?}");
    }
    // The SN50's 64 GB of HBM2E is the SN40's own (the vendor's specification table lists both), so it moves as fast.
    let sn50 = RduChipFacts::sn50().hbm_gbps();
    assert_eq!((sn50.value, sn50.provenance), (1600.0, Provenance::Modeled), "{}", sn50.cite);
    for word in ["inherited", "HBM2E", "[SN50WEB]"] {
        assert!(sn50.cite.contains(word), "the SN50's HBM bandwidth says `{word}`: {}", sn50.cite);
    }
    // The roof every plan reads is this figure.
    let roofs = RduChipFacts::sn40l().roofs();
    assert_eq!(roofs.mem.len(), 1);
    assert_eq!(serde_json::to_value(&roofs.mem[0].rate.value).unwrap(), serde_json::json!({ "PerSecond": 1.6e12 }));
}

#[test]
fn ddr_and_p2p_rates_are_facts() {
    let sn40l = RduChipFacts::sn40l();
    let ddr = sn40l.ddr_bandwidth_gbps;
    assert_eq!((ddr.value, ddr.provenance), (100.0, Provenance::Published), "{}", ddr.cite);
    assert!(stated(ddr.cite).starts_with("over 100 GB/s"), "{}", ddr.cite);
    assert!(
        sn40l.others.iter().any(|o| o.fact == "ddr_bandwidth_gbps" && o.stated == "over 200 GB/s"),
        "the SN40L paper's earlier figure is kept"
    );
    let sn50 = RduChipFacts::sn50();
    assert_eq!((sn50.ddr_bandwidth_gbps.value, sn50.ddr_bandwidth_gbps.provenance), (100.0, Provenance::Modeled));
    assert!(sn50.ddr_bandwidth_gbps.cite.contains("inherited"), "{}", sn50.ddr_bandwidth_gbps.cite);
    // Scale-up: the SN50's ten 800G ports carry 2 TB/s an RDU; the SN40L's is a quarter of it, by the SN50's launch.
    let p2p = sn50.p2p_tb_s;
    assert_eq!((p2p.value, p2p.provenance), (2.0, Provenance::Published), "{}", p2p.cite);
    assert_eq!(p2p.value, sn50.scale_up.expect("the SN50's scale-up").value.tb_s_per_chip, "one figure, one value");
    let p2p = sn40l.p2p_tb_s;
    assert_eq!((p2p.value, p2p.provenance), (0.5, Provenance::Derived), "{}", p2p.cite);
    for key in ["[HC26]", "[SN50PR]"] {
        assert!(p2p.cite.contains(key), "the SN40L's rate names {key}: {}", p2p.cite);
    }
}

#[test]
fn a_figure_two_sources_state_differently_keeps_both() {
    let sn40l = RduChipFacts::sn40l();
    let sn50 = RduChipFacts::sn50();
    let other = |facts: &RduChipFacts, fact: &str| -> Vec<(String, String)> {
        facts.others.iter().filter(|o| o.fact == fact).map(|o| (o.stated.to_string(), o.cite.to_string())).collect()
    };
    assert_eq!((sn40l.peak_bf16_tflops.value, sn40l.peak_bf16_tflops.provenance), (638, Provenance::Published));
    assert!(other(&sn40l, "peak_bf16_tflops").iter().any(|(s, c)| s == "640 TFLOPS" && c.contains("[SN50WEB]")));
    assert_eq!(sn40l.ddr_gib_max.value, 1536);
    assert!(other(&sn40l, "ddr_gib_max").iter().any(|(s, c)| s == "Up to 512 GB" && c.contains("[SN50WEB]")));
    assert_eq!(sn50.ddr_gib_max.value, 512);
    assert!(other(&sn50, "ddr_gib_max").iter().any(|(s, c)| s == "Up to 2 TB" && c.contains("[SN50DS]")));
    // Every other figure is about a fact the chip carries, from a pinned source, and is not the fact's own figure.
    let pinned: BTreeSet<String> = pins().iter().map(|p| p["key"].clone()).collect();
    for facts in [&sn40l, &sn50] {
        let paths: BTreeMap<String, String> = facts_of(facts).into_iter().map(|(p, _, _, c)| (p, c)).collect();
        for o in facts.others {
            let own = paths.get(o.fact).unwrap_or_else(|| panic!("{:?}: `{}` names no fact", facts.chip, o.fact));
            assert_ne!(stated(own), o.stated, "{:?} `{}`: the other figure is the fact's own", facts.chip, o.fact);
            let cited = keys(o.cite);
            assert!(!cited.is_empty() && cited.iter().all(|k| pinned.contains(*k)), "{:?} `{}`: {} cites no pinned source", facts.chip, o.fact, o.cite);
        }
    }
}

#[test]
fn every_published_fact_cites_a_pinned_source() {
    let pins = pins();
    let pinned: BTreeSet<String> = pins.iter().map(|p| p["key"].clone()).collect();
    let texts = common::require_tool("samba-papers").then(|| source_texts(&pins));
    let mut checked = 0;
    for facts in RduChip::ALL.map(RduChip::facts) {
        let mut cited = BTreeSet::new();
        for (path, _, provenance, cite) in facts_of(&facts) {
            let named = keys(&cite);
            cited.extend(named.iter().map(|k| k.to_string()));
            for key in &named {
                assert!(pinned.contains(*key), "{:?} `{path}`: [{key}] is not pinned in samba_papers.tsv", facts.chip);
            }
            match provenance.as_str() {
                "Published" => {
                    let key = named.first().copied().filter(|k| cite.starts_with(&format!("[{k}]")));
                    let key = key.unwrap_or_else(|| panic!("{:?} `{path}`: a published citation opens with its source's key: {cite}", facts.chip));
                    if let (Some(texts), Some(n)) = (&texts, first_number(stated(&cite))) {
                        assert!(appears(&texts[key], &n), "{:?} `{path}`: {n} is not in [{key}]'s pinned text: {cite}", facts.chip);
                        checked += 1;
                    }
                }
                "Derived" => assert!(named.len() >= 2 && cite.contains('='), "{:?} `{path}`: a derived figure names its inputs and its arithmetic: {cite}", facts.chip),
                _ => {}
            }
        }
        for o in facts.others {
            cited.extend(keys(o.cite).into_iter().map(str::to_string));
            if let (Some(texts), Some(n)) = (&texts, first_number(o.stated)) {
                let key = keys(o.cite)[0];
                assert!(appears(&texts[key], &n), "{:?} other `{}`: {n} is not in [{key}]'s pinned text", facts.chip, o.fact);
            }
        }
        let listed: BTreeSet<String> = facts.cites.iter().map(|c| c.to_string()).collect();
        let cited_published: BTreeSet<String> = cited.into_iter().filter(|k| pinned.contains(k)).collect();
        assert_eq!(listed, cited_published, "{:?}: the chip's sources are exactly the keys its facts cite", facts.chip);
    }
    if texts.is_some() {
        assert!(checked >= 20, "{checked} published figures found in their sources");
    }
}
