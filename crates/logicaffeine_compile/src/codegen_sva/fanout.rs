//! fanout — the pit-of-success primitive for antenna-safe wide selects.
//!
//! A wide mux/select driven by ONE bit is a met1-antenna hazard on an ASIC: that single select
//! net must fan out across the FULL datapath (every mux bit it gates), and a long, high-fan-out
//! wire is exactly the metal-1 area-ratio violation antenna checks flag. The GPU's
//! `SequentialFma` datapath (`gpu_simt_rtl`) solved this for its per-cycle conditional-add partial
//! product `sel ? data : 0` by SLICING it: emit `slice` `(* keep *)` copies of the select bit,
//! each ANDing its OWN `width / slice`-bit group of the data wire, so NO net's fan-out exceeds
//! `width / slice`. Every copy equals the same one bit, so the concatenation is bit-for-bit the
//! original select — the value is unchanged; only the physical fan-out is bounded.
//!
//! That is a GENERAL principle, not a GPU quirk: **any wide mux/select driven by a single bit
//! should route it through [`bounded_fanout_select`]** so its fan-out is structurally bounded by
//! `width / slice` and a met1 antenna is impossible by construction, no diode insertion needed.
//! The `slice` knob trades wires for fan-out: smaller `slice` = each copy gates FEWER bits = lower
//! fan-out = less antenna, at the cost of MORE `(* keep *)` copies; larger `slice` = the reverse.

/// Emit a fan-out-bounded image of the one-bit select `sel_expr ? data_wire : 0` over `width`
/// bits, sliced into `slice` groups.
///
/// Returns `(decls, pp_expr)`:
///   * `decls` — the `slice` `(* keep *)` select-bit copies, one per line, each named
///     `{prefix}{s}{suffix}` and driven by `sel_expr`. The `(* keep *)` attribute stops synthesis
///     (yosys) from merging the copies back into one high-fan-out net; iverilog ignores it, so
///     simulation is unchanged.
///   * `pp_expr` — the concatenation `{ {G{prefix_{slice-1}_suffix}} & data[hi:lo] , ... ,
///     {G{prefix_0_suffix}} & data[hi:lo] }` where `G = width / slice`. Groups are concatenated
///     high→low so group 0 lands at the LSB. No net in this expression drives more than `G` mux
///     bits — the fan-out is bounded by `width / slice`, never the full datapath (the met1-antenna
///     root cause).
///
/// `prefix`/`suffix` frame the select-copy family so copy `s` is named `{prefix}{s}{suffix}` (the
/// `suffix` carries any per-instance tag, e.g. a lane suffix `_l3` — pass `""` when there is none).
/// `data_wire` is the wire being gated (e.g. `"mc"`). Panics unless `slice > 0 && width % slice ==
/// 0`, so the byte-groups tile `data_wire` exactly with no dropped or misaligned bit.
pub fn bounded_fanout_select(
    prefix: &str,
    suffix: &str,
    sel_expr: &str,
    data_wire: &str,
    width: usize,
    slice: usize,
) -> (String, String) {
    assert!(slice > 0, "bounded_fanout_select slice must be positive");
    assert_eq!(
        width % slice,
        0,
        "bounded_fanout_select slice ({slice}) must divide width ({width}) evenly"
    );
    let group = width / slice;
    let mut decls = String::new();
    for s in 0..slice {
        decls.push_str(&format!(
            "  (* keep *) wire {prefix}{s}{suffix} = {sel_expr};\n"
        ));
    }
    let terms: Vec<String> = (0..slice)
        .rev()
        .map(|s| {
            let hi = (s + 1) * group - 1;
            let lo = s * group;
            format!("{{{group}{{{prefix}{s}{suffix}}}}} & {data_wire}[{hi}:{lo}]")
        })
        .collect();
    let pp_expr = format!(
        "{{ {} }}",
        terms.join(",\n\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20 ")
    );
    (decls, pp_expr)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/fanout/tests.rs"]
mod tests;
