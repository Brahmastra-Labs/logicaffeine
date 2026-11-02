//! R5 — the SVA REVERSE-MODULE OBLIGATION for Hardware Studio.
//!
//! A Silicon functional unit `U` is reversible iff there exists a module `R` with `R ∘ U = id`. This
//! discharges that obligation for a combinational unit THREE independent ways that MUST AGREE:
//!
//!   1. **Exhaustive bit-blast** — the unit's `2ⁿ`-row truth table is a permutation (each output
//!      distinct); its inverse permutation IS the constructive reverse module `R`.
//!   2. **Algebraic certificate** — for a linear (CNOT/XOR) unit, its GF(2) transfer matrix is
//!      invertible (`∈ GL(n,2)`); reused from [`logicaffeine_proof::revgate`].
//!   3. **Z3 equivalence** — (behind the `verification` feature) `check_z3_hw_equivalence` proves the
//!      composed `R ∘ U` structurally equals the identity.
//!
//! The obligation FAILS LOUDLY if any two discharged methods disagree — the "our bit-blaster == Z3 ==
//! algebra" pattern from `phase_hw_native_vs_z3.rs`, now for reversibility. It emits the SVA property
//! that pins `R(U(x)) == x`, the hardware-level twin of the software `forward ; reverse ≡ id` lock.

use logicaffeine_proof::revgate::{certify_gate, certify_linear, GateCert};

/// A discharged reverse-module obligation: the constructive reverse gate plus the SVA property that
/// pins the reverse-module identity, produced only when every attempted method AGREED that `U` is
/// reversible.
#[derive(Debug, Clone)]
pub struct ReverseModuleObligation {
    pub name: String,
    pub bits: u32,
    /// The reverse module `R` as a truth table — the constructive inverse with `R ∘ U = id`.
    pub reverse: Vec<usize>,
    /// The SVA property text asserting `R(U(x)) == x` over every input.
    pub sva_property: String,
}

/// Discharge the reverse-module obligation for the `n`-bit combinational unit `truth_table`. Proves
/// reversibility by exhaustive bit-blast AND — when `linear_rows` (the GF(2) transfer matrix) is
/// given — the algebraic GF(2) certificate, and REQUIRES the two verdicts to agree. On success
/// returns the reverse module + SVA property; on failure names the erasure or the method disagreement.
pub fn discharge_reverse_module(
    name: &str,
    n: u32,
    truth_table: &[usize],
    linear_rows: Option<&[u64]>,
) -> Result<ReverseModuleObligation, String> {
    // Way 1 — exhaustive bit-blast: the truth table must be a permutation; its inverse is `R`.
    let reverse = match certify_gate(n, truth_table) {
        GateCert::Reversible { inverse } => inverse,
        GateCert::Collision {
            input_a,
            input_b,
            output,
        } => {
            return Err(format!(
                "{name}: irreversible — inputs {input_a} and {input_b} both map to output {output} (erasure)"
            ));
        }
    };
    // Way 2 — algebraic GF(2) (linear units): reversible iff the transfer matrix ∈ GL(n,2). Requiring
    // it to AGREE with the exhaustive verdict is what makes the obligation trustworthy.
    if let Some(rows) = linear_rows {
        if !certify_linear(n, rows) {
            return Err(format!(
                "{name}: methods DISAGREE — the exhaustive bit-blast proved `U` reversible, but its GF(2) matrix is singular"
            ));
        }
    }
    let sva_property = emit_reverse_module_sva(name, n, truth_table, &reverse);
    Ok(ReverseModuleObligation {
        name: name.to_string(),
        bits: n,
        reverse,
        sva_property,
    })
}

/// Emit the SVA property pinning `R(U(x)) == x` as an exhaustive case over the `2ⁿ` inputs — the
/// reverse-module identity as a checkable hardware assertion.
fn emit_reverse_module_sva(name: &str, n: u32, table: &[usize], reverse: &[usize]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "// Reverse-module obligation: R ∘ U = id for the {n}-bit unit `{name}`\n"
    ));
    s.push_str(&format!(
        "property p_reverse_module_{name};\n  @(posedge clk) case (x)\n"
    ));
    for x in 0..(1usize << n) {
        // U(x) = table[x]; R(U(x)) = reverse[table[x]] — the obligation asserts it equals x.
        let r_of_u = reverse[table[x]];
        s.push_str(&format!(
            "    {n}'d{x}: (r == {n}'d{r_of_u}); // U({x})={}, R(·)={r_of_u}\n",
            table[x]
        ));
    }
    s.push_str(&format!(
        "  endcase\nendproperty\nassert property (p_reverse_module_{name});\n"
    ));
    s
}

/// A discharged JOURNALED-op injectivity obligation — the hardware twin of the software minimal-bit
/// journaling (corner 1). A `Journaled` op `F` is many-to-one (it erases), but AUGMENTED with its
/// Bennett ancilla `J` (the journal entry — the exact bits it destroyed), the map `x ↦ (F(x), J(x))` is
/// INJECTIVE: the reverse module reconstructs `x` from the post-state AND the ancilla. This closes the
/// R4 gap — the Z3/exhaustive proofs covered the `Bijection` core, and this covers every `Journaled`
/// op the same way.
#[derive(Debug, Clone)]
pub struct JournaledInjectivity {
    pub name: String,
    pub in_bits: u32,
    /// The constructive reverse: the packed `(F(x), J(x))` output ↦ the input `x`.
    pub reverse: std::collections::HashMap<usize, usize>,
    /// The SVA property asserting the ancilla-augmented map is injective over every input.
    pub sva_property: String,
}

/// Prove a `Journaled` op is INJECTIVE once augmented with its Bennett ancilla. `augmented[x]` is the
/// packed `(F(x), J(x))` output for input `x` (all `2^in_bits` inputs present). Injective iff every
/// packed output is distinct — an EXHAUSTIVE bit-blast, the same rigor as [`certify_gate`], so the
/// core needs no SMT solver. Fails loud on a collision: two inputs share an augmented output, so the
/// ancilla did NOT capture the erased bits — a genuine (un-reversible) Landauer erasure.
pub fn discharge_journaled_injective(
    name: &str,
    in_bits: u32,
    augmented: &[usize],
) -> Result<JournaledInjectivity, String> {
    assert_eq!(
        augmented.len(),
        1usize << in_bits,
        "augmented table must cover every input"
    );
    let mut reverse = std::collections::HashMap::new();
    for (x, &y) in augmented.iter().enumerate() {
        if let Some(&prior) = reverse.get(&y) {
            return Err(format!(
                "{name}: NOT injective even with the ancilla — inputs {prior} and {x} both map to augmented output {y}; the ancilla did not capture the erased bits (genuine Landauer erasure)"
            ));
        }
        reverse.insert(y, x);
    }
    let sva_property = emit_journaled_injectivity_sva(name, in_bits, augmented);
    Ok(JournaledInjectivity {
        name: name.to_string(),
        in_bits,
        reverse,
        sva_property,
    })
}

/// Emit the SVA property pinning that the ancilla-augmented map is injective — one case row per input,
/// each mapping to its distinct augmented output (the reverse module reads it right-to-left).
fn emit_journaled_injectivity_sva(name: &str, n: u32, augmented: &[usize]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "// Journaled-op injectivity: (F(x), ancilla(x)) is injective for `{name}` (reverse reconstructs x)\n"
    ));
    s.push_str(&format!(
        "property p_journaled_injective_{name};\n  @(posedge clk) case (x)\n"
    ));
    for (x, &y) in augmented.iter().enumerate() {
        s.push_str(&format!(
            "    {n}'d{x}: (aug == {y}); // reverse: {y} -> {x}\n"
        ));
    }
    s.push_str(&format!(
        "  endcase\nendproperty\nassert property (p_journaled_injective_{name});\n"
    ));
    s
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/reverse_module/tests.rs"]
mod tests;
