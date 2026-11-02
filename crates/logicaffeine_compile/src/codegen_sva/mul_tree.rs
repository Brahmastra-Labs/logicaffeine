//! mul_tree — a word multiply as an aligned tree of adds. The product `a · x mod 2^w` is the sum of the `w` partial
//! products `x[i] ? a << i : 0`; the tree adds them pairwise, level by level, each add only as wide as the bits it can
//! still change (a partial product shifted by `i` is zero below bit `i`, and nothing above bit `w − 1` survives), and
//! a register can follow any level. On a fabric with carry chains every add is one chain and nothing multiplies; cut
//! after every level, no stage holds more than one add.

/// The tree of one multiply: its declarations and combinational assignments, its registers with the value each takes
/// every clock, the signal holding the product after the last level, and the clocks from its operands to it.
#[derive(Clone, Debug, PartialEq)]
pub struct MulTree {
    pub wires: String,
    pub regs: Vec<(String, u32)>,
    pub moves: Vec<(String, String)>,
    pub product: String,
    pub latency: u32,
}

/// The levels of the tree over `width` partial products: the pairwise adds that bring them down to one.
pub fn mul_tree_levels(width: u32) -> usize {
    let mut terms = width as usize;
    let mut levels = 0;
    while terms > 1 {
        terms = terms.div_ceil(2);
        levels += 1;
    }
    levels
}

/// The product `a · x mod 2^width` of the `width`-bit Verilog expressions `a` and `x`, its signals named after
/// `prefix`, registered after each level `level` for which `cut(level)` holds (the last level's sum is the product).
pub fn mul_tree(prefix: &str, a: &str, x: &str, width: u32, cut: impl Fn(usize) -> bool) -> MulTree {
    assert!(width >= 2, "a multiply of at least two bits");
    let w = width;
    let mut wires = String::new();
    let mut regs = Vec::new();
    let mut moves = Vec::new();
    // a term: (signal, offset): bits [w-1:offset] of the product's partial sum, in w - offset bits
    let mut terms: Vec<(String, u32)> = (0..w)
        .map(|i| {
            let name = format!("{prefix}_r{i}");
            let bits = w - i;
            wires.push_str(&format!("  wire [{}:0] {name} = {{{bits}{{{x}[{i}]}}}} & {a}[{}:0];\n", bits - 1, bits - 1));
            (name, i)
        })
        .collect();
    let mut latency = 0;
    let mut level = 0;
    while terms.len() > 1 {
        let mut next = Vec::with_capacity(terms.len().div_ceil(2));
        for (k, pair) in terms.chunks(2).enumerate() {
            if pair.len() == 1 {
                next.push(pair[0].clone());
                continue;
            }
            let ((a_name, oa), (b_name, ob)) = (&pair[0], &pair[1]);
            let d = ob - oa;
            let name = format!("{prefix}_s{level}_{k}");
            let (abits, high) = (w - oa, w - ob);
            wires.push_str(&format!("  wire [{}:0] {name}_h = {a_name}[{}:{d}] + {b_name};\n", high - 1, abits - 1));
            wires.push_str(&format!("  wire [{}:0] {name} = {{{name}_h, {a_name}[{}:0]}};\n", abits - 1, d - 1));
            next.push((name, *oa));
        }
        if cut(level) {
            for (name, offset) in next.iter_mut() {
                let reg = format!("{name}_q");
                regs.push((reg.clone(), w - *offset));
                moves.push((reg.clone(), name.clone()));
                *name = reg;
            }
            latency += 1;
        }
        terms = next;
        level += 1;
    }
    MulTree { wires, regs, moves, product: terms.remove(0).0, latency }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/mul_tree/tests.rs"]
mod tests;
