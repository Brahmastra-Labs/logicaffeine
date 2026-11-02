use super::*;
use std::process::Command;

fn iverilog() -> bool {
    let ok = Command::new("iverilog").arg("-V").output().map(|o| o.status.success()).unwrap_or(false);
    if !ok {
        eprintln!("SKIP: iverilog is not installed");
    }
    ok
}

/// Stream `pairs` one a clock through the tree for `width` cut by `cut`; the simulator's count of products checked
/// and of mismatches against `a · x mod 2^width`.
fn stream(width: u32, cut: impl Fn(usize) -> bool, pairs: &[(u64, u64)]) -> (usize, usize) {
    let t = mul_tree("m", "a", "x", width, cut);
    let mask = if width == 64 { u64::MAX } else { (1u64 << width) - 1 };
    let mut decls = String::new();
    for (r, bits) in &t.regs {
        decls.push_str(&format!("  reg [{}:0] {r};\n", bits - 1));
    }
    let moves: String = t.moves.iter().map(|(r, e)| format!("    {r} <= {e};\n")).collect();
    let rom: String = pairs
        .iter()
        .map(|&(a, x)| format!("{:016x}{:016x}{:016x}\n", a & mask, x & mask, (a & mask).wrapping_mul(x & mask) & mask))
        .collect();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("v.hex"), rom).unwrap();
    let (n, lat, w1) = (pairs.len(), t.latency as usize, width - 1);
    let tb = format!(
        "module tb; reg clk = 0; always #1 clk = ~clk;\n\
         reg [191:0] v [0:{last}]; reg [{w1}:0] a = 0, x = 0; integer i, seen = 0, bad = 0;\n\
         {decls}{wires}  always @(posedge clk) begin\n{moves}  end\n\
         initial begin $readmemh(\"v.hex\", v);\n\
         for (i = 0; i < {n} + {lat}; i = i + 1) begin\n\
           if (i < {n}) begin a = v[i][191:128]; x = v[i][127:64]; end\n\
           if (i >= {lat}) begin #0.5; seen = seen + 1; if ({product} !== v[i - {lat}][{w1}:0]) bad = bad + 1; end\n\
           @(posedge clk); #0.1;\n\
         end\n\
         $display(\"RESULT %0d %0d\", seen, bad); $finish; end\nendmodule\n",
        last = n - 1,
        wires = t.wires,
        product = t.product,
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let b = Command::new("iverilog").args(["-g2005", "-o", "sim", "tb.v"]).current_dir(dir.path()).output().unwrap();
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new("vvp").args(["-n", "sim"]).current_dir(dir.path()).output().unwrap();
    let out = String::from_utf8_lossy(&r.stdout);
    let line = out.lines().find(|l| l.starts_with("RESULT")).unwrap_or_else(|| panic!("no RESULT: {out}"));
    let f: Vec<usize> = line.split_whitespace().skip(1).map(|s| s.parse().unwrap()).collect();
    (f[0], f[1])
}

/// Every pair of operands at every width from 2 to 8 bits, through the tree with no register, a register after
/// every level, and one after every other: each product is `a · x mod 2^w`, the latency the registers it has.
#[test]
fn every_product_at_every_narrow_width_and_every_cut() {
    if !iverilog() {
        return;
    }
    for width in 2..=8u32 {
        let pairs: Vec<(u64, u64)> = (0..1u64 << width).flat_map(|a| (0..1u64 << width).map(move |x| (a, x))).collect();
        let levels = mul_tree_levels(width);
        for (name, cut) in [("none", 0usize), ("every", 1), ("alternate", 2)] {
            let c = move |l: usize| cut != 0 && (l + 1) % cut == 0;
            let t = mul_tree("m", "a", "x", width, c);
            assert_eq!(t.latency as usize, (0..levels).filter(|&l| c(l)).count(), "{width} bits, {name}");
            assert_eq!(stream(width, c, &pairs), (pairs.len(), 0), "{width} bits, cut {name}");
        }
    }
}

/// 64-bit words — the extremes against each other and random words — through a tree registered after every level:
/// the low 64 bits of every product, `mul_tree_levels(64)` clocks later.
#[test]
fn sixty_four_bit_products_stream_one_a_clock() {
    if !iverilog() {
        return;
    }
    let edges = [0u64, 1, 2, u64::MAX, u64::MAX - 1, 1 << 63, (1 << 63) - 1, 0x5555_5555_5555_5555, 0xAAAA_AAAA_AAAA_AAAA, 1 << 32];
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut pairs: Vec<(u64, u64)> = edges.iter().flat_map(|&a| edges.iter().map(move |&x| (a, x))).collect();
    pairs.extend((0..3000).map(|_| (next(), next())));
    assert_eq!(mul_tree_levels(64), 6);
    assert_eq!(mul_tree("m", "a", "x", 64, |_| true).latency, 6);
    assert_eq!(stream(64, |_| true, &pairs), (pairs.len(), 0));
    assert_eq!(stream(64, |_| false, &pairs), (pairs.len(), 0));
}

#[test]
fn the_tree_has_no_multiply_and_adds_only_the_bits_that_change() {
    let t = mul_tree("m", "a", "x", 64, |_| false);
    assert!(!t.wires.contains('*'), "no multiply operator");
    assert_eq!(t.wires.matches(" + ").count(), 63, "sixty-three adds bring sixty-four partial products to one");
    assert!(t.wires.contains("wire [62:0] m_s0_0_h = m_r0[63:1] + m_r1;"), "the first add is as wide as the bits above the lower row's offset");
    assert_eq!(mul_tree_levels(5), 3);
}
