//! The SCALAR TAPE-OUT core RTL differential fuzz — random programs through the REAL `soft_core_scalar.v`.
//!
//! `soft_core_scalar.v` is the minimal core that actually TAPES OUT (Sky130 signoff). Its `$50M` ratchet
//! (`scalar_fifty_million_dollar_test`) proves every opcode it emits is decoded + iverilog-exercised, but only
//! over a curated per-opcode corpus. This harness adds breadth-by-GENERATION: a deterministic PRNG synthesizes
//! scalar-expressible LOGOS programs (the reversible ALU + shared-ALU bitwise/shift/compare + branches + calls +
//! swap), each is baked into an iverilog testbench against the real `soft_core_scalar.v` (+ `alu.v`), and the
//! hardware's `Show` sequence is asserted byte-exact against the VM. A divergence is a real tape-out silicon
//! defect ($50M/defect) and panics with the reproducing seed + source. RevSwap / RevNeg / the reversible adds
//! are exactly the subtle two-register ops where an off-by-one silicon bug hides.
//!
//! SOUND (non-vacuous): the scalar core is 32-bit SIGNED, so the generator is bounded to keep every value in a
//! ±2^30 window where the 32-bit hardware and the VM MUST agree; a program the scalar encoder declines (`Err` —
//! e.g. multiply/mod, which the scalar core cannot do) is SKIPPED (counted), and a per-family floor keeps every
//! datapath genuinely exercised. Deterministic by seed (splitmix64). iverilog-gated (skips loudly if absent).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::scalar_asm::compile_to_scalar_words;
use logicaffeine_compile::compile::vm_outcome;

struct Rng {
    state: u64,
}
impl Rng {
    fn new(seed: u64) -> Self {
        Rng {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// a signed operand in `[-mag, mag]`.
    fn signed(&mut self, mag: i64) -> i64 {
        (self.next() % (2 * mag as u64 + 1)) as i64 - mag
    }
}

// NOTE: user functions are DELIBERATELY absent — `compile_to_scalar_words` is function-free by design (it
// rejects `## To …` with "calling-convention translation not yet implemented"), exactly like it rejects
// multiply/mod. That is a fail-LOUD limitation, not a silent miscompile, so it is out of this fuzz's scope.
const FAMILIES: u64 = 7;
const FAMILY_NAMES: [&str; FAMILIES as usize] = [
    "add_sub_chain",
    "bitwise",
    "not_neg",
    "shifts",
    "cmp_branch",
    "swap",
    "move_chain",
];

/// Synthesize one scalar-expressible, terminating, 30-bit-safe, integer-output LOGOS program.
fn gen(family: u64, rng: &mut Rng) -> String {
    match family {
        // 0 — add/sub chain (the reversible RevAdd/RevSub datapath)
        0 => {
            let (a, b, c) = (rng.signed(30_000), rng.signed(30_000), rng.signed(30_000));
            let (o1, o2) = (
                ["+", "-"][rng.below(2) as usize],
                ["+", "-"][rng.below(2) as usize],
            );
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Let c be {c}.\n    Show (a {o1} b) {o2} c.\n")
        }
        // 1 — bitwise and / or / xor (the shared-ALU BitAnd/BitOr/BitXor); operands < 2^29 so `|` stays < 2^30
        1 => {
            let (a, b) = (rng.below(32_768) as i64, rng.below(32_768) as i64);
            let op = ["&", "|", "xor"][rng.below(3) as usize];
            format!("## Main\n    Let a be {a}.\n    Let b be {b}.\n    Show a {op} b.\n")
        }
        // 2 — bitwise not (`~a`, ALU Not) and arithmetic negation (`0 - a`, RevSub) — the sign paths
        2 => {
            let a = rng.signed(30_000);
            if rng.below(2) == 0 {
                format!("## Main\n    Let a be {a}.\n    Show ~a.\n")
            } else {
                format!("## Main\n    Let a be {a}.\n    Show 0 - a.\n")
            }
        }
        // 3 — logical shifts (Shl/Shr); base + count bounded so the result stays < 2^30
        3 => {
            let a = rng.below(1000) as i64;
            let k = rng.below(16);
            if rng.below(2) == 0 {
                format!("## Main\n    Let a be {a}.\n    Show a shifted left by {k}.\n")
            } else {
                let big = rng.below(32_768) as i64;
                format!("## Main\n    Let a be {big}.\n    Show a shifted right by {k}.\n")
            }
        }
        // 4 — comparison through if/else (the shared-ALU signed compares → 0/1 branch)
        4 => {
            let (x, y) = (rng.signed(30_000), rng.signed(30_000));
            let cmp = [
                "is greater than",
                "is less than",
                "is at most",
                "is at least",
                "is equal to",
                "is not",
            ][rng.below(6) as usize];
            format!("## Main\n    Let x be {x}.\n    Let y be {y}.\n    If x {cmp} y:\n        Show 1.\n    Otherwise:\n        Show 0.\n")
        }
        // 5 — RevSwap: a two-register swap (rf_wd = av; regs[ra] <= regs[dst]) — the subtle reversible op
        5 => {
            let (a, b) = (rng.signed(30_000), rng.signed(30_000));
            format!("## Main\n    Let mutable a be {a}.\n    Let mutable b be {b}.\n    Swap a and b.\n    Show a.\n    Show b.\n")
        }
        // 6 — a Move/let chain (RevAdd-with-0 / Move): value threads through several bindings unchanged
        _ => {
            let a = rng.signed(30_000);
            format!("## Main\n    Let a be {a}.\n    Let b be a.\n    Let c be b.\n    Let d be c.\n    Show d.\n")
        }
    }
}

fn silicon_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// An int-only scalar testbench: preload `prog` into `dut.imem`, run `soft_core_scalar.v`, capture every `Show`
/// off `out`/`out_valid`, and assert the captured 32-bit value SEQUENCE equals `expect`.
fn emit_scalar_tb(prog: &[u32], expect: &[i32], name: &str) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "`timescale 1ns/1ps");
    let _ = writeln!(s, "module tb;");
    let _ = writeln!(
        s,
        "  reg clk = 0, rst = 1; wire [31:0] out; wire out_valid, halted;"
    );
    let _ = writeln!(s, "  soft_core_scalar dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));");
    let _ = writeln!(s, "  integer n = 0; reg [31:0] seen [0:31];");
    let _ = writeln!(s, "  always #5 clk = ~clk;");
    let _ = writeln!(
        s,
        "  always @(posedge clk) if (out_valid) begin seen[n] = out; n = n + 1; end"
    );
    let _ = writeln!(s, "  reg fail;");
    let _ = writeln!(s, "  initial begin");
    let _ = writeln!(s, "    #1;");
    for (i, word) in prog.iter().enumerate() {
        let _ = writeln!(s, "    dut.imem[{i}] = 32'h{word:08X};");
    }
    let _ = writeln!(s, "    #3 rst = 0;");
    let _ = writeln!(s, "    wait (halted);");
    let _ = writeln!(s, "    #10;");
    let _ = writeln!(s, "    fail = 0;");
    let _ = writeln!(s, "    if (n != {}) begin fail = 1; $display(\"FAIL: {name} — got %0d shows, want {}\", n); end", expect.len(), expect.len());
    for (i, v) in expect.iter().enumerate() {
        let _ = writeln!(
            s,
            "    if (!fail && (seen[{i}] !== 32'h{:08X})) begin fail = 1; $display(\"FAIL: {name} — show {i} was %0d, want %0d\", $signed(seen[{i}]), $signed(32'h{:08X})); end",
            *v as u32, *v as u32
        );
    }
    let _ = writeln!(s, "    if (!fail) $display(\"PASS: {name}\");");
    let _ = writeln!(s, "    $finish;");
    let _ = writeln!(s, "  end");
    let _ = writeln!(
        s,
        "  initial begin #2000000 $display(\"FAIL: {name} timeout\"); $finish; end"
    );
    let _ = writeln!(s, "endmodule");
    s
}

fn run_on_rtl(
    prog: &[u32],
    expect: &[i32],
    name: &str,
    workdir: &Path,
    sil: &Path,
) -> Result<(), String> {
    let tb_path = workdir.join(format!("{name}.v"));
    let vvp_path = workdir.join(format!("{name}.vvp"));
    std::fs::write(&tb_path, emit_scalar_tb(prog, expect, name))
        .map_err(|e| format!("write tb: {e}"))?;
    let c = Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&vvp_path)
        .arg(sil.join("alu.v"))
        .arg(sil.join("soft_core_scalar.v"))
        .arg(&tb_path)
        .output()
        .map_err(|e| format!("spawn iverilog: {e}"))?;
    if !c.status.success() {
        return Err(format!(
            "iverilog compile failed:\n{}",
            String::from_utf8_lossy(&c.stderr)
        ));
    }
    let r = Command::new("vvp")
        .arg(&vvp_path)
        .output()
        .map_err(|e| format!("spawn vvp: {e}"))?;
    let out = String::from_utf8_lossy(&r.stdout);
    let meaningful: String = out
        .lines()
        .filter(|l| !l.trim_start().starts_with("WARNING"))
        .collect::<Vec<_>>()
        .join("\n");
    if meaningful.contains(&format!("PASS: {name}")) {
        Ok(())
    } else {
        Err(format!("hardware did not PASS:\n{meaningful}"))
    }
}

const SEEDS_PER_FAMILY: u64 = 6;
const PER_FAMILY_FLOOR: u64 = SEEDS_PER_FAMILY * 9 / 10;
const RAN_FLOOR: u64 = FAMILIES * PER_FAMILY_FLOOR;

/// The scalar RTL fuzz: many deterministic seeds × every scalar datapath family, each run through the REAL
/// `soft_core_scalar.v` (the tape-out core) under iverilog, hardware `Show` sequence asserted byte-exact vs VM.
#[test]
fn scalar_core_rtl_equals_vm_over_a_generated_corpus() {
    if !common::require_tool("iverilog") {
        return;
    }
    let sil = silicon_dir();
    for f in ["alu.v", "soft_core_scalar.v"] {
        assert!(
            sil.join(f).is_file(),
            "missing RTL source {f} in {}",
            sil.display()
        );
    }
    let workdir = std::env::temp_dir().join("logos_soft_core_scalar_rtl_fuzz");
    std::fs::create_dir_all(&workdir).expect("create workdir");

    let mut ran = 0u64;
    let mut boundary = 0u64;
    let mut over_width = 0u64;
    let mut ran_per_family = vec![0u64; FAMILIES as usize];
    let mut opcode_union: BTreeSet<u8> = BTreeSet::new();

    for family in 0..FAMILIES {
        for s in 0..SEEDS_PER_FAMILY {
            let seed = family * 1_000 + s;
            let mut rng = Rng::new(seed);
            let src = gen(family, &mut rng);

            let words = match compile_to_scalar_words(&src) {
                Ok(w) => w,
                Err(_) => {
                    boundary += 1;
                    continue;
                }
            };
            let vm = vm_outcome(&src);
            assert!(
                vm.error.is_none(),
                "the generator produced a VM-erroring program (generator bug)\n  seed={seed}\n  src={src:?}\n  err={:?}",
                vm.error
            );

            // Expected = the VM's Show lines as i32. Every value is bounded to ±2^30 by construction; a line that
            // doesn't parse or exceeds the window is out of the 32-bit envelope — skip (counted).
            let mut expect: Vec<i32> = Vec::new();
            let mut representable = true;
            for line in vm.output.lines().filter(|l| !l.trim().is_empty()) {
                match line.trim().parse::<i64>() {
                    Ok(v) if v.abs() < (1 << 30) => expect.push(v as i32),
                    _ => {
                        representable = false;
                        break;
                    }
                }
            }
            if !representable || expect.is_empty() {
                over_width += 1;
                continue;
            }

            let name = format!("scalarfuzz_{}_s{s}", FAMILY_NAMES[family as usize]);
            match run_on_rtl(&words, &expect, &name, &workdir, &sil) {
                Ok(()) => {
                    ran += 1;
                    ran_per_family[family as usize] += 1;
                    for w in &words {
                        opcode_union.insert((w >> 24) as u8);
                    }
                }
                Err(transcript) => panic!(
                    "SCALAR TAPE-OUT CORE RTL != VM — a real silicon miscompile on the actual Verilog ($50M class)\n  \
                     seed={seed}\n  family={}\n  src={src:?}\n  expect={expect:?}\n  {transcript}",
                    FAMILY_NAMES[family as usize]
                ),
            }
        }
    }

    eprintln!(
        "scalar-core RTL fuzz: {ran} ran on real soft_core_scalar.v via iverilog, {boundary} boundary, \
         {over_width} over-width; per-family={ran_per_family:?}; ~{} distinct opcode-bytes",
        opcode_union.len()
    );

    assert!(
        ran >= RAN_FLOOR,
        "only {ran} programs ran on the real scalar RTL (floor {RAN_FLOOR}) — the harness drifted off the \
         scalar-encoder surface; the silicon differential is near-vacuous"
    );
    for (f, &c) in ran_per_family.iter().enumerate() {
        assert!(
            c >= PER_FAMILY_FLOOR,
            "family {} ({}) reached the real scalar RTL only {c}/{SEEDS_PER_FAMILY} times (floor {PER_FAMILY_FLOOR}) \
             — that datapath fell off the scalar surface; the fuzz's breadth collapsed there",
            f, FAMILY_NAMES[f]
        );
    }
}

/// Reproduction probe: dump src + scalar opcodes + VM output for a family (default 2 = not_neg), so a
/// hardware divergence can be diagnosed. `DUMP_SCALAR_FAMILY=2 cargo nextest run -E 'test(dump_scalar_family)' --run-ignored all --no-capture`.
#[test]
#[ignore]
fn dump_scalar_family() {
    let family = std::env::var("DUMP_SCALAR_FAMILY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2u64);
    for s in 0..SEEDS_PER_FAMILY {
        let seed = family * 1_000 + s;
        let src = gen(family, &mut Rng::new(seed));
        let vm = vm_outcome(&src);
        match compile_to_scalar_words(&src) {
            Ok(words) => {
                let ops: Vec<u8> = words.iter().map(|w| (w >> 24) as u8).collect();
                eprintln!(
                    "seed={seed} src={src:?}\n  opcodes={ops:?}\n  words={:08X?}\n  vm={:?}",
                    words, vm.output
                );
            }
            Err(e) => eprintln!(
                "seed={seed} src={src:?}\n  REJECTED={}",
                e.lines().next().unwrap_or("")
            ),
        }
    }
}

/// The generator is a pure function of its seed: a failure reproduces exactly.
#[test]
fn the_scalar_rtl_generator_is_deterministic() {
    for seed in [0u64, 1, 7, 42, 1000, 7007] {
        for family in 0..FAMILIES {
            let a = gen(family, &mut Rng::new(seed));
            let b = gen(family, &mut Rng::new(seed));
            assert_eq!(
                a, b,
                "gen(seed={seed}, family={family}) is non-deterministic"
            );
        }
    }
}
