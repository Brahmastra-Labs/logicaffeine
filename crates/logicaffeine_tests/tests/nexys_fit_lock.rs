//! nexys_fit_lock — the headline "the whole 113-op LOGOS CPU FITS a single FPGA"
//! claim is pinned to a fresh Xilinx-synth artifact, so prose can never claim a fit
//! the synthesizer does not actually show.
//!
//! Source of truth: `benchmarks/silicon/nexys_a7_fit.json`, written by
//! `scripts/synth-soft-core-nexys.sh` (yosys `synth_xilinx -family xc7` on the FULL
//! default config — HEAP 256, the real 113-op core — against the Nexys A7-100T budget:
//! 63,400 LUT6 / 126,800 FF / 240 DSP48 / 135 BRAM36).
//!
//! Why this exists: on the ECP5-85K (4-input TRELLIS_COMB) the same core is 128,829
//! LUTs = 154% and does NOT fit; on the Artix-7 (6-input LUTs, ~3x denser) it maps to
//! ~38.5k LUT6 = ~61% and FITS. That flip is the whole reason the Nexys A7 is the target
//! board. If a re-synth ever pushes any resource over budget, `fits` flips and this goes
//! RED; when the artifact is refreshed the DOC is what changes to match it.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> serde_json::Value {
    let p = root().join(rel);
    let raw = fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e} — regenerate via scripts/synth-soft-core-nexys.sh",
            p.display()
        )
    });
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {rel}: {e}"))
}

const FIT: &str = "benchmarks/silicon/nexys_a7_fit.json";

#[test]
fn the_full_soft_core_fits_the_nexys_a7_within_budget() {
    let d = json(FIT);
    let sc = &d["soft_core_full"];
    let bud = &d["budget"];
    let g = |v: &serde_json::Value, k: &str| -> u64 {
        v[k].as_u64()
            .unwrap_or_else(|| panic!("{FIT}: missing/!u64 {k}"))
    };

    let (lut, ff, dsp, bram) = (g(sc, "lut6"), g(sc, "ff"), g(sc, "dsp48"), g(sc, "bram36"));
    let (lut_cap, ff_cap, dsp_cap, bram_cap) = (
        g(bud, "lut6"),
        g(bud, "ff"),
        g(bud, "dsp48"),
        g(bud, "bram36"),
    );

    // The board is the Nexys A7-100T (XC7A100T): its documented budget must be exact,
    // so nobody can silently widen the cap to manufacture a fit.
    assert_eq!(
        lut_cap, 63_400,
        "{FIT}: LUT6 budget must be the XC7A100T's 63,400"
    );
    assert_eq!(
        ff_cap, 126_800,
        "{FIT}: FF budget must be the XC7A100T's 126,800"
    );
    assert_eq!(
        dsp_cap, 240,
        "{FIT}: DSP48 budget must be the XC7A100T's 240"
    );
    assert_eq!(
        bram_cap, 135,
        "{FIT}: BRAM36 budget must be the XC7A100T's 135"
    );

    // Every resource under budget — the actual fit guarantee.
    assert!(
        lut <= lut_cap,
        "{FIT}: LUT6 {lut} exceeds {lut_cap} — the core does NOT fit"
    );
    assert!(ff <= ff_cap, "{FIT}: FF {ff} exceeds {ff_cap}");
    assert!(dsp <= dsp_cap, "{FIT}: DSP48 {dsp} exceeds {dsp_cap}");
    assert!(bram <= bram_cap, "{FIT}: BRAM36 {bram} exceeds {bram_cap}");

    // The recorded verdict + percentage cannot lie about the numbers next to them.
    assert!(
        sc["fits"].as_bool().unwrap(),
        "{FIT}: fits=false but every resource is under budget?"
    );
    let pct = (lut as f64) * 100.0 / (lut_cap as f64);
    assert!(
        (sc["lut6_pct"].as_f64().unwrap() - pct).abs() <= 0.1,
        "{FIT}: lut6_pct {} disagrees with {lut}/{lut_cap} = {pct:.1}",
        sc["lut6_pct"]
    );

    // Sanity floor: this is a real 113-op CPU, not an empty netlist — guard a broken synth
    // that "fits" only because it produced nothing.
    assert!(
        lut >= 20_000,
        "{FIT}: LUT6 {lut} implausibly small — did synth actually run the full core?"
    );
    assert!(
        dsp >= 1,
        "{FIT}: the 32-bit multiply should infer at least one DSP48"
    );
}

/// True iff `word` occurs in `hay` at identifier boundaries (so "out" does not match "output").
fn contains_word(hay: &str, word: &str) -> bool {
    let id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let b = hay.as_bytes();
    let mut from = 0;
    while let Some(i) = hay[from..].find(word) {
        let s = from + i;
        let e = s + word.len();
        let before_ok = s == 0 || !id(b[s - 1] as char);
        let after_ok = e >= b.len() || !id(b[e] as char);
        if before_ok && after_ok {
            return true;
        }
        from = s + 1;
    }
    false
}

/// Strip `//` line comments so a `;` inside a port comment can't fool the header split.
fn strip_line_comments(s: &str) -> String {
    s.lines()
        .map(|l| l.split("//").next().unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_nexys_xdc_pins_only_real_soft_core_ports() {
    let xdc = fs::read_to_string(root().join("benchmarks/silicon/nexys_a7.xdc"))
        .expect("read nexys_a7.xdc");
    let rtl = strip_line_comments(
        &fs::read_to_string(root().join("benchmarks/silicon/soft_core.v"))
            .expect("read soft_core.v"),
    );

    // The module header runs from `module soft_core` to the `);` that closes the port list;
    // the first `;` after the header start is exactly that close (params use `,`, not `;`).
    let start = rtl
        .find("module soft_core")
        .expect("soft_core.v: no `module soft_core`");
    let end = rtl[start..]
        .find(';')
        .map(|i| i + start)
        .expect("soft_core.v: unterminated header");
    let header = &rtl[start..end];

    // Every `get_ports { NAME[idx] }` the XDC constrains must be a REAL declared port; a rename
    // in the RTL that the board file misses is a silent Vivado-time break this catches at test time.
    let mut pinned = std::collections::BTreeSet::new();
    let mut out_max_idx = 0usize;
    let mut any = false;
    for seg in xdc.split("get_ports").skip(1) {
        let inside = match seg.split('{').nth(1).and_then(|s| s.split('}').next()) {
            Some(s) => s.trim(),
            None => continue,
        };
        any = true;
        let base = inside.split('[').next().unwrap_or(inside).trim();
        if let Some(idx) = inside.split('[').nth(1).and_then(|s| s.split(']').next()) {
            if let Ok(n) = idx.trim().parse::<usize>() {
                if base == "out" {
                    out_max_idx = out_max_idx.max(n);
                }
            }
        }
        pinned.insert(base.to_string());
    }
    assert!(
        any && !pinned.is_empty(),
        "nexys_a7.xdc constrains no ports — malformed board file"
    );

    for base in &pinned {
        assert!(
            contains_word(header, base),
            "nexys_a7.xdc pins `{base}`, but soft_core.v's port list declares no such port \
             — the board constraints drifted from the RTL (a silent Vivado break)"
        );
    }
    // `out` is a 32-bit port; no pinned LED bit may exceed that width.
    assert!(
        contains_word(header, "out") && header.contains("[31:0]"),
        "soft_core.v `out` must be declared [31:0] for the XDC's LED mapping"
    );
    assert!(
        out_max_idx <= 31,
        "nexys_a7.xdc pins out[{out_max_idx}] beyond out's 32-bit width"
    );
}

const SOC_FIT: &str = "benchmarks/silicon/nexys_soc_fit.json";

#[test]
fn the_board_soc_fits_the_nexys_a7() {
    // nexys_soc = the full core PLUS the UART load/run/readback wrapper (verified end-to-end by
    // tb_nexys_soc.v). It must still fit the Nexys A7-100T, so the board-runnable claim is real.
    let d = json(SOC_FIT);
    let sc = &d["nexys_soc"];
    let bud = &d["budget"];
    let g = |v: &serde_json::Value, k: &str| {
        v[k].as_u64()
            .unwrap_or_else(|| panic!("{SOC_FIT}: missing {k}"))
    };

    let (lut, ff, dsp) = (g(sc, "lut6"), g(sc, "ff"), g(sc, "dsp48"));
    assert_eq!(
        g(bud, "lut6"),
        63_400,
        "{SOC_FIT}: LUT6 budget must be the XC7A100T's 63,400"
    );
    assert!(
        lut <= g(bud, "lut6"),
        "{SOC_FIT}: SoC LUT6 {lut} exceeds budget — does NOT fit"
    );
    assert!(ff <= g(bud, "ff"), "{SOC_FIT}: SoC FF {ff} exceeds budget");
    assert!(
        dsp <= g(bud, "dsp48"),
        "{SOC_FIT}: SoC DSP48 {dsp} exceeds budget"
    );
    assert!(
        sc["fits"].as_bool().unwrap(),
        "{SOC_FIT}: fits=false with every resource under budget?"
    );
    // It must be strictly LARGER than the bare core (it wraps it) yet still under budget — a sanity
    // floor that catches a synth that silently dropped the core.
    let core = json(FIT)["soft_core_full"]["lut6"].as_u64().unwrap();
    assert!(
        lut >= core,
        "{SOC_FIT}: SoC LUT6 {lut} < bare core {core} — the wrapper can't shrink it"
    );
}

#[test]
fn the_nexys_soc_xdc_pins_only_real_ports() {
    let xdc = fs::read_to_string(root().join("benchmarks/silicon/nexys_soc.xdc"))
        .expect("read nexys_soc.xdc");
    let rtl = strip_line_comments(
        &fs::read_to_string(root().join("benchmarks/silicon/nexys_soc.v"))
            .expect("read nexys_soc.v"),
    );
    let start = rtl.find("module nexys_soc").expect("no `module nexys_soc`");
    let end = rtl[start..]
        .find(';')
        .map(|i| i + start)
        .expect("unterminated header");
    let header = &rtl[start..end];

    let mut pinned = std::collections::BTreeSet::new();
    for seg in xdc.split("get_ports").skip(1) {
        if let Some(inside) = seg.split('{').nth(1).and_then(|s| s.split('}').next()) {
            let base = inside.trim().split('[').next().unwrap_or("").trim();
            if !base.is_empty() {
                pinned.insert(base.to_string());
            }
        }
    }
    assert!(!pinned.is_empty(), "nexys_soc.xdc constrains no ports");
    for base in &pinned {
        assert!(
            contains_word(header, base),
            "nexys_soc.xdc pins `{base}`, but nexys_soc.v declares no such port — XDC drifted from RTL"
        );
    }
    // The UART round-trip needs exactly these to reach the board.
    for required in ["clk", "rst", "uart_rx", "uart_tx", "led"] {
        assert!(
            pinned.contains(required),
            "nexys_soc.xdc must pin `{required}`"
        );
    }
}
