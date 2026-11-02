//! fabric_reconfig_lock — Phase 5: RECONFIGURABLE-FROM-BYTECODE ("the fabric IS the interpreter").
//!
//! P1–P4 built and composed fabric islands and proved the interpreter can hand off to them. The endgame is
//! ONE physical tile array that becomes ANY block's datapath under a CONFIG WORD streamed per block from the
//! bytecode frame — "send the bytes down the wire and the tiled fabric runs them." This generalizes the
//! proven 2-fabric time-multiplexed array (a 1-bit `ctx`) to an N-fabric array selected by a config word
//! (`time_mux_n_bounded`).
//!
//! This lock pins it with our CERTIFIED SAT (Z3-free): several same-shape residual fabrics are placed on ONE
//! array, and for EACH config value `k` we prove `config == k ⇒ array ≡ the bespoke island k`, for ALL
//! inputs — so the reconfiguration is SEMANTICS-PRESERVING, not merely plausible. An eval pass confirms each
//! realized fabric computes ≡ the VM and that the fabrics are genuinely DISTINCT (so the config word does
//! real work). The physical whole-program run (the Phase-3 CFG controller driving the config word as it
//! walks blocks, on one Sky130/FPGA array) is the silicon follow-on; this proves the reconfiguration core.

use logicaffeine_compile::codegen_sva::fabric_grid::{
    certify_reconfig, eval_grid, route_straight_line,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::residual_from_source;
use logicaffeine_compile::compile::vm_outcome;

// Three SAME-SHAPE single-input residuals `k*x + m` (the proven 2-cell straight-line shape): a `Mul` tile
// then an `Add` tile, placing onto identical array dimensions so they share ONE reconfigurable tile grid.
// The config word selects the (k, m) coefficients — i.e. which affine map the array computes.
const SOURCES: &[(&str, &str)] = &[
    (
        "## To f0 (x: Int) -> Int:\n    Return 3 * x + 7.\n## Main\n    Show f0(0).\n",
        "f0",
    ),
    (
        "## To f1 (x: Int) -> Int:\n    Return 5 * x + 2.\n## Main\n    Show f1(0).\n",
        "f1",
    ),
    (
        "## To f2 (x: Int) -> Int:\n    Return 2 * x + 9.\n## Main\n    Show f2(0).\n",
        "f2",
    ),
];

fn ref_of(k: usize, x: i64) -> i64 {
    match k {
        0 => 3 * x + 7,
        1 => 5 * x + 2,
        _ => 2 * x + 9,
    }
}

fn arg(n: i64) -> String {
    if n < 0 {
        format!("(0 - {})", -n)
    } else {
        n.to_string()
    }
}

fn vm_call(k: usize, x: i64) -> i64 {
    let (src, name) = SOURCES[k];
    let base = src.split("## Main").next().unwrap();
    let prog = format!("{base}## Main\n    Show {name}({}).\n", arg(x));
    let out = vm_outcome(&prog);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

fn grids() -> Vec<logicaffeine_compile::codegen_sva::fabric_grid::Grid> {
    SOURCES
        .iter()
        .map(|(src, name)| {
            let (ops, n_in, res) = residual_from_source(src, name);
            route_straight_line(&ops, n_in, res)
        })
        .collect()
}

#[test]
fn config_word_reconfigures_one_array_into_any_fabric() {
    // The certified-SAT heart of Phase 5: for EACH config value, the ONE reconfigurable array is provably
    // EXACTLY the bespoke island for that config, over all inputs. (2-bit config word, 8-bit datapath.)
    let verdicts = certify_reconfig(SOURCES, 8, 2);
    assert_eq!(
        verdicts,
        vec![true, true, true],
        "each config value must reconfigure the ONE array to be exactly that block's datapath (config==k ⇒ array ≡ island k)"
    );
}

#[test]
fn each_realized_fabric_computes_like_the_vm() {
    // The placed grids evaluate ≡ the VM — the reconfiguration targets are correct datapaths, not artifacts.
    let gs = grids();
    let mut st = 0xC0DE_FACE_1234_u64;
    let mut mix = || {
        st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = st;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for _ in 0..200 {
        let x = (mix() % 4001) as i64 - 2000;
        for (k, g) in gs.iter().enumerate() {
            let got = eval_grid(g, &[x]);
            assert_eq!(got, ref_of(k, x), "grid {k} eval ≠ reference on x={x}");
            assert_eq!(got, vm_call(k, x), "grid {k} eval ≠ VM on x={x}");
        }
    }
}

#[test]
fn the_reconfigured_fabrics_are_genuinely_distinct() {
    // The config word does REAL work: on a witness input the three fabrics produce three different results,
    // so reconfiguring the array is not a vacuous mux over identical datapaths.
    let gs = grids();
    let outs: Vec<i64> = gs.iter().map(|g| eval_grid(g, &[1])).collect();
    // f0=3+7=10, f1=5+2=7, f2=2+9=11 — all distinct.
    assert_eq!(
        outs,
        vec![10, 7, 11],
        "the three fabrics must be genuinely distinct datapaths"
    );
}
