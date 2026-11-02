//! fabric_config — Phase D: the config-driven reconfigurable grid = THE INTERPRETER.
//!
//! Phase 5 (`fabric_reconfig_lock`) proved ONE tile array becomes any of several SAME-SHAPE datapaths under
//! a config word. The honest gap it left is the **same-shape constraint**: the fabrics had to be hand-crafted
//! to identical dimensions. Phase D closes it. A DIVERSE set of program datapaths — different sizes, different
//! tile mixes (arithmetic, branch/select, comparison) — is each if-converted to a single-output island,
//! PADDED to a common `rows × cols` ([`pad_grid`], purely additive), and placed on ONE reconfigurable array.
//! A per-datapath CONFIG WORD (its index in the table, streamed from the `LGB1` bytecode frame) selects which
//! datapath the array computes. Proven by our certified SAT (Z3-free): for EACH config value `k`,
//! `config == k ⇒ the array's output ≡ island k`, for ALL inputs — so projecting the whole program set onto
//! ONE physical fabric is SEMANTICS-PRESERVING. This is "send the bytes down the wire and the tiled fabric
//! runs them", now for an arbitrary program corpus rather than a same-shape triple.
//!
//! Boundary (honest): each corpus datapath is a single-output island (a function if-converted to a
//! select-DAG). A single function's INTERNAL multi-output basic blocks streamed as separate configs — the
//! multi-output reconfigurable block walk — is the natural follow-on; the config-table core (one array,
//! any datapath, padded, certified) is what this module and `fabric_whole_program_lock` prove.

use super::fabric_grid::{
    eval_grid, grid_to_bounded, pad_grid, route_loop_body, time_mux_n_bounded, Grid,
};
use super::fabric_rtl::decision_fabric_from_source;

/// A configuration word: the index of the datapath the array is currently configured to compute. Streamed
/// per datapath from the bytecode frame — the config table is the projection of the program onto the array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigWord(pub u64);

/// The config word that selects datapath / block `k`. (Trivial today — the index — but the named seam is
/// where a richer encoding, e.g. an interned block id, would live.)
pub fn block_config(block: usize) -> ConfigWord {
    ConfigWord(block as u64)
}

/// A program corpus compiled to ONE reconfigurable tile array: every datapath placed as a same-shape island
/// (all padded to `rows × cols`), plus the datapath widths for the certified reconfig proof. `grids[k]` is
/// the island selected by `block_config(k)`.
#[derive(Clone, Debug)]
pub struct ConfigTable {
    /// One island per datapath, ALL padded to the common array shape so they co-reside.
    pub grids: Vec<Grid>,
    /// The array's common dimensions (every `grids[k]` has these).
    pub rows: usize,
    pub cols: usize,
    /// The datapath bit width for the reconfig SAT proof.
    pub width: u32,
    /// The config-word bit width (`ceil(log2(n))`), so `time_mux_n_bounded` selects among all islands.
    pub cfg_width: u32,
}

/// Compile a set of `(source, function-name)` datapaths to a [`ConfigTable`]: if-convert each function to a
/// single-output decision island, place it on the tile array, then PAD every island to the common
/// `max_rows × max_cols` so they share ONE reconfigurable array. `width` is the datapath width for the SAT
/// proof; `cfg_width` must index all datapaths (`(1 << cfg_width) >= sources.len()`).
pub fn compile_to_config_table(
    sources: &[(&str, &str)],
    width: u32,
    cfg_width: u32,
) -> ConfigTable {
    assert!(
        !sources.is_empty(),
        "a config table needs at least one datapath"
    );
    assert!(
        (1usize << cfg_width) >= sources.len(),
        "cfg_width {cfg_width} cannot index {} datapaths",
        sources.len()
    );
    let raw: Vec<Grid> = sources
        .iter()
        .map(|(src, name)| {
            let df = decision_fabric_from_source(src, name);
            route_loop_body(&df.cells, df.n_in, &[df.result])
        })
        .collect();
    let rows = raw.iter().map(|g| g.rows).max().unwrap();
    let cols = raw.iter().map(|g| g.cols).max().unwrap();
    let grids: Vec<Grid> = raw.iter().map(|g| pad_grid(g, rows, cols)).collect();
    ConfigTable {
        grids,
        rows,
        cols,
        width,
        cfg_width,
    }
}

/// Certify the config table END TO END with our SAT: for EACH config value `k`, prove
/// `config == k ⇒ the ONE reconfigurable array ≡ island k`, for ALL inputs (bounded at `width`). Returns the
/// per-config verdicts — all `true` ⇒ the config word losslessly selects any datapath on one physical array.
pub fn certify_config_table(table: &ConfigTable) -> Vec<bool> {
    use super::bitblast::lower_bool_bounded;
    use super::sva_to_verify::BoundedExpr;
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    // The reconfig SAT lowers every grid's tiles to a BoundedExpr; a `Func` (division/float) tile has no
    // bitblast lowering. A Func-bearing config table is proven by eval ≡ VM + iverilog (see fabric_soc_lock),
    // not by this SAT — refuse it loudly here rather than panic opaquely inside `tile_bounded`.
    assert!(
        table.grids.iter().all(super::fabric_grid::grid_is_combinational),
        "certify_config_table: the table has a Func (division/float) tile — a Func-bearing config table is \
         proven by eval + iverilog, not by our SAT reconfig cert"
    );
    let refs: Vec<&Grid> = table.grids.iter().collect();
    (0..table.grids.len())
        .map(|k| {
            let combined = time_mux_n_bounded(&refs, k as u64, table.width, table.cfg_width);
            let island = grid_to_bounded(&table.grids[k], table.width);
            // config == k  ⇒  array ≡ island k, for ALL inputs: prove ¬(array == island) is UNSAT.
            // ★ THE BUDGET IS A LEGITIMATE OUTCOME HERE, SO IT GETS A SENTENCE RATHER THAN AN `.expect`.
            // `lower_bool` returns `None` past the blast ceiling, and this used to `.expect("bitvec
            // equality lowers to a boolean")` — a message that tells a reader nothing about why a table
            // that certified yesterday stopped today. A per-config verdict has no room to carry a reason,
            // so this refuses the way the Func case above refuses: loudly, naming the cause, the size and
            // the proof path that does cover it.
            let eq = match lower_bool_bounded(&BoundedExpr::Eq(Box::new(combined), Box::new(island))) {
                Ok(e) => e,
                Err(why) => panic!(
                    "certify_config_table: config {k} of {} could not be bit-blasted within the ceiling \
                     ({why:?}) — the reconfig SAT materialises a time-mux of every grid against island {k}, \
                     so a table this wide is proven by eval + iverilog (see fabric_soc_lock) rather than by \
                     this SAT, or the ceiling is raised deliberately with a measurement behind it",
                    table.grids.len()
                ),
            };
            prove_unsat(&ProofExpr::Not(Box::new(eq))) == UnsatOutcome::Refuted
        })
        .collect()
}

/// Evaluate the reconfigurable array configured as datapath `k` on `inputs`. Because the reconfig proof
/// certifies `config == k ⇒ array ≡ island k`, the array's output IS `island k`'s — so this evaluates the
/// placed island directly. The result is the value the physical fabric would drive when its config word is
/// [`block_config`]`(k)`.
pub fn eval_config_table(table: &ConfigTable, k: usize, inputs: &[i64]) -> i64 {
    eval_grid(&table.grids[k], inputs)
}
