//! Random RV64IMAC programs for differential runs of the hart: a Linux user-mode `_start` that loads every register
//! with a random value, runs a random body — integer and multiply/divide arithmetic, every load and store width,
//! atomics, reservations, forward branches and jumps — then writes its registers and its data buffer to stdout and
//! exits. The same program runs under `qemu-riscv64` and on the hart, so any difference is a difference in the
//! instruction semantics. Deterministic by seed.
#![allow(dead_code)]

/// splitmix64.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x6a09_e667_f3bc_c908)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }

    /// A 64-bit value biased toward the edges arithmetic gets wrong: zero, one, all ones, the sign boundaries.
    fn value(&mut self) -> u64 {
        match self.below(8) {
            0 => 0,
            1 => 1,
            2 => u64::MAX,
            3 => 1 << 63,
            4 => (1 << 63) - 1,
            5 => 0x8000_0000,
            6 => self.below(64),
            _ => self.next(),
        }
    }
}

/// The data buffer's size in bytes.
pub const BUF: u64 = 256;

/// Registers the body may write: all but `s11` (the buffer's base) and `t6` (an address scratch).
const DST: [&str; 30] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7",
    "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "t3", "t4", "t5",
];

const SRC: [&str; 31] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7",
    "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4", "t5",
];

const R_OPS: [&str; 28] = [
    "add", "sub", "sll", "slt", "sltu", "xor", "srl", "sra", "or", "and", "addw", "subw", "sllw", "srlw", "sraw", "mul",
    "mulh", "mulhsu", "mulhu", "div", "divu", "rem", "remu", "mulw", "divw", "divuw", "remw", "remuw",
];

const I_OPS: [&str; 7] = ["addi", "slti", "sltiu", "xori", "ori", "andi", "addiw"];
const SHIFT_OPS: [&str; 3] = ["slli", "srli", "srai"];
const SHIFTW_OPS: [&str; 3] = ["slliw", "srliw", "sraiw"];
const BRANCHES: [&str; 6] = ["beq", "bne", "blt", "bge", "bltu", "bgeu"];
const AMOS: [&str; 9] = ["amoswap", "amoadd", "amoxor", "amoand", "amoor", "amomin", "amomax", "amominu", "amomaxu"];

fn reg(rng: &mut Rng, set: &[&'static str]) -> &'static str {
    set[rng.below(set.len() as u64) as usize]
}

fn one(rng: &mut Rng, out: &mut String) {
    let d = reg(rng, &DST);
    let a = reg(rng, &SRC);
    let b = reg(rng, &SRC);
    match rng.below(16) {
        0..=4 => out.push_str(&format!("  {} {d}, {a}, {b}\n", rng.pick(&R_OPS))),
        5..=6 => {
            let imm = rng.below(4096) as i64 - 2048;
            out.push_str(&format!("  {} {d}, {a}, {imm}\n", rng.pick(&I_OPS)));
        }
        7 => out.push_str(&format!("  {} {d}, {a}, {}\n", rng.pick(&SHIFT_OPS), rng.below(64))),
        8 => out.push_str(&format!("  {} {d}, {a}, {}\n", rng.pick(&SHIFTW_OPS), rng.below(32))),
        9 => {
            let op = if rng.below(2) == 0 { "lui" } else { "auipc" };
            out.push_str(&format!("  {op} {d}, {}\n", rng.below(1 << 20)));
        }
        10 => {
            let (op, size) = [("lb", 1), ("lbu", 1), ("lh", 2), ("lhu", 2), ("lw", 4), ("lwu", 4), ("ld", 8)]
                [rng.below(7) as usize];
            let off = rng.below(BUF / size) * size;
            out.push_str(&format!("  {op} {d}, {off}(s11)\n"));
        }
        11 => {
            let (op, size) = [("sb", 1), ("sh", 2), ("sw", 4), ("sd", 8)][rng.below(4) as usize];
            let off = rng.below(BUF / size) * size;
            out.push_str(&format!("  {op} {a}, {off}(s11)\n"));
        }
        12 => {
            let (w, size) = if rng.below(2) == 0 { ("w", 4) } else { ("d", 8) };
            let off = rng.below(BUF / size) * size;
            out.push_str(&format!("  addi t6, s11, {off}\n  {}.{w} {d}, {b}, (t6)\n", rng.pick(&AMOS)));
        }
        13 => {
            let (w, size) = if rng.below(2) == 0 { ("w", 4) } else { ("d", 8) };
            let off = rng.below(BUF / size) * size;
            let other = if rng.below(3) == 0 { (off + size) % BUF } else { off };
            let d2 = reg(rng, &DST);
            out.push_str(&format!(
                "  addi t6, s11, {off}\n  lr.{w} {d}, (t6)\n  addi t6, s11, {other}\n  sc.{w} {d2}, {b}, (t6)\n"
            ));
        }
        14 => {
            let skip: String = (0..1 + rng.below(3))
                .map(|_| {
                    let mut s = String::new();
                    one_straight(rng, &mut s);
                    s
                })
                .collect();
            out.push_str(&format!("  {} {a}, {b}, 1f\n{skip}1:\n", rng.pick(&BRANCHES)));
        }
        _ => {
            let mut s = String::new();
            one_straight(rng, &mut s);
            out.push_str(&format!("  jal {d}, 1f\n{s}1:\n"));
        }
    }
}

/// An instruction that cannot branch, for the shadow of a branch or jump.
fn one_straight(rng: &mut Rng, out: &mut String) {
    let d = reg(rng, &DST);
    let a = reg(rng, &SRC);
    let b = reg(rng, &SRC);
    out.push_str(&format!("  {} {d}, {a}, {b}\n", rng.pick(&R_OPS)));
}

/// One program's assembly: `instructions` random instructions between the register load and the dump.
pub fn program(seed: u64, instructions: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut s = String::from("  .text\n  .globl _start\n_start:\n  la s11, buf\n");
    for r in DST.iter().skip(1) {
        s.push_str(&format!("  li {r}, {:#x}\n", rng.value()));
    }
    s.push_str(&format!("  li t6, {:#x}\n", rng.value()));
    for i in 0..BUF / 8 {
        s.push_str(&format!("  li t6, {:#x}\n  sd t6, {}(s11)\n", rng.value(), i * 8));
    }
    for _ in 0..instructions {
        one(&mut rng, &mut s);
    }
    s.push_str("  la t6, out\n");
    for (i, r) in SRC.iter().enumerate().skip(1) {
        s.push_str(&format!("  sd {r}, {}(t6)\n", i * 8));
    }
    s.push_str(
        "  li a7, 64\n  li a0, 1\n  la a1, out\n  li a2, 256\n  ecall\n\
         \x20 li a7, 64\n  li a0, 1\n  la a1, buf\n  li a2, 256\n  ecall\n\
         \x20 li a7, 93\n  li a0, 0\n  ecall\n",
    );
    s.push_str("  .data\n  .align 4\nbuf: .zero 256\nout: .zero 256\n");
    s
}

const F_REGS: usize = 32;
const FP_BIN: [&str; 8] = ["fadd", "fsub", "fmul", "fdiv", "fmin", "fmax", "fsgnj", "fsgnjn"];
const FP_CMP: [&str; 3] = ["feq", "flt", "fle"];
const FP_FMA: [&str; 4] = ["fmadd", "fmsub", "fnmsub", "fnmadd"];
const ROUNDING: [&str; 6] = ["rne", "rtz", "rdn", "rup", "rmm", "dyn"];

/// A floating-point bit pattern biased toward what conversions and comparisons get wrong: zeros, infinities, NaNs
/// (quiet and signaling), subnormals, integers near the conversion limits, NaN-boxed and improperly boxed singles.
fn fp_bits(rng: &mut Rng) -> u64 {
    match rng.below(14) {
        0 => 0,
        1 => 1 << 63,
        2 => 0x7ff0_0000_0000_0000,
        3 => 0xfff0_0000_0000_0000,
        4 => 0x7ff8_0000_0000_0000,
        5 => 0x7ff4_0000_0000_0001,
        6 => 0x0000_0000_0000_0001 | (rng.below(1 << 20)),
        7 => 0x43e0_0000_0000_0000 + rng.below(4) - 2,
        8 => 0x41e0_0000_0000_0000 + rng.below(4) - 2,
        9 => 0xffff_ffff_0000_0000 | (rng.next() & 0xffff_ffff),
        10 => 0xffff_ffff_7fc0_0000,
        11 => 0xffff_ffff_7f80_0001,
        12 => rng.next() & 0x0000_ffff_ffff_ffff,
        _ => rng.next(),
    }
}

/// One program of random F and D instructions: every operation, both formats, every rounding mode, the CSRs, loads and
/// stores (some misaligned), on operands from [`fp_bits`]. It writes its integer registers, its floating-point
/// registers and fcsr out (520 bytes) and exits.
pub fn fp_program(seed: u64, instructions: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut s = String::from("  .text\n  .globl _start\n_start:\n  la s11, buf\n");
    for r in DST.iter().skip(1) {
        s.push_str(&format!("  li {r}, {:#x}\n", rng.value()));
    }
    for f in 0..F_REGS {
        s.push_str(&format!("  li t6, {:#x}\n  fmv.d.x f{f}, t6\n", fp_bits(&mut rng)));
    }
    for i in 0..BUF / 8 {
        s.push_str(&format!("  li t6, {:#x}\n  sd t6, {}(s11)\n", fp_bits(&mut rng), i * 8));
    }
    s.push_str(&format!("  li t6, {}\n  fsrm t6\n", rng.below(5)));
    for _ in 0..instructions {
        let fd = rng.below(32);
        let fa = rng.below(32);
        let fb = rng.below(32);
        let fc = rng.below(32);
        let fmt = if rng.below(2) == 0 { "s" } else { "d" };
        let rm = rng.pick(&ROUNDING);
        let x = reg(&mut rng, &DST);
        let xs = reg(&mut rng, &SRC);
        match rng.below(14) {
            0..=2 => {
                let op = rng.pick(&FP_BIN);
                let rounds = matches!(op, "fadd" | "fsub" | "fmul" | "fdiv");
                if rounds {
                    s.push_str(&format!("  {op}.{fmt} f{fd}, f{fa}, f{fb}, {rm}\n"));
                } else {
                    s.push_str(&format!("  {op}.{fmt} f{fd}, f{fa}, f{fb}\n"));
                }
            }
            3 => s.push_str(&format!("  fsqrt.{fmt} f{fd}, f{fa}, {rm}\n")),
            4 => s.push_str(&format!("  {}.{fmt} f{fd}, f{fa}, f{fb}, f{fc}, {rm}\n", rng.pick(&FP_FMA))),
            5 => s.push_str(&format!("  {}.{fmt} {x}, f{fa}, f{fb}\n", rng.pick(&FP_CMP))),
            6 => s.push_str(&format!("  fclass.{fmt} {x}, f{fa}\n")),
            7 => {
                let to = ["w", "wu", "l", "lu"][rng.below(4) as usize];
                s.push_str(&format!("  fcvt.{to}.{fmt} {x}, f{fa}, {rm}\n"));
            }
            8 => {
                // A 32-bit integer converts to a double exactly: that conversion takes no rounding mode.
                let from = ["w", "wu", "l", "lu"][rng.below(4) as usize];
                if fmt == "d" && from.starts_with('w') {
                    s.push_str(&format!("  fcvt.d.{from} f{fd}, {xs}\n"));
                } else {
                    s.push_str(&format!("  fcvt.{fmt}.{from} f{fd}, {xs}, {rm}\n"));
                }
            }
            9 => {
                if fmt == "s" {
                    s.push_str(&format!("  fcvt.s.d f{fd}, f{fa}, {rm}\n"));
                } else {
                    s.push_str(&format!("  fcvt.d.s f{fd}, f{fa}\n"));
                }
            }
            10 => match rng.below(4) {
                0 => s.push_str(&format!("  fmv.x.d {x}, f{fa}\n")),
                1 => s.push_str(&format!("  fmv.x.w {x}, f{fa}\n")),
                2 => s.push_str(&format!("  fmv.d.x f{fd}, {xs}\n")),
                _ => s.push_str(&format!("  fmv.w.x f{fd}, {xs}\n")),
            },
            11 => {
                let (op, size) = if rng.below(2) == 0 { ("flw", 4) } else { ("fld", 8) };
                let off = if rng.below(4) == 0 { rng.below(BUF - 8) } else { rng.below(BUF / size) * size };
                s.push_str(&format!("  {op} f{fd}, {off}(s11)\n"));
            }
            12 => {
                let (op, size) = if rng.below(2) == 0 { ("fsw", 4) } else { ("fsd", 8) };
                let off = if rng.below(4) == 0 { rng.below(BUF - 8) } else { rng.below(BUF / size) * size };
                s.push_str(&format!("  {op} f{fa}, {off}(s11)\n"));
            }
            _ => match rng.below(5) {
                0 => s.push_str(&format!("  frflags {x}\n")),
                1 => s.push_str(&format!("  fsflags {x}, {xs}\n")),
                2 => s.push_str(&format!("  csrrsi {x}, fcsr, {}\n", rng.below(32))),
                3 => s.push_str(&format!("  fsrmi {x}, {}\n", rng.below(5))),
                _ => s.push_str(&format!("  frcsr {x}\n")),
            },
        }
    }
    s.push_str("  la t6, out\n");
    for (i, r) in SRC.iter().enumerate().skip(1) {
        s.push_str(&format!("  sd {r}, {}(t6)\n", i * 8));
    }
    for f in 0..F_REGS {
        s.push_str(&format!("  fsd f{f}, {}(t6)\n", 256 + f * 8));
    }
    s.push_str("  frcsr t5\n  sd t5, 512(t6)\n");
    s.push_str(
        "  li a7, 64\n  li a0, 1\n  la a1, out\n  li a2, 520\n  ecall\n\
         \x20 li a7, 64\n  li a0, 1\n  la a1, buf\n  li a2, 256\n  ecall\n\
         \x20 li a7, 93\n  li a0, 0\n  ecall\n",
    );
    s.push_str("  .data\n  .align 4\nbuf: .zero 256\nout: .zero 520\n");
    s
}

/// Bytes between the regions a cache program aliases: the data and instruction caches' size, so every region's lines
/// fall on the same cache lines and each access may evict another region's dirty line.
pub const CACHE_STRIDE: u64 = 16 << 10;
const CACHE_REGIONS: u64 = 8;
/// The bytes of each region a cache program touches: four lines.
const CACHE_SPAN: u64 = 256;

/// Registers a cache program's body may write: [`DST`] but `s10` (the code slot) and `t5` (the slot's return).
const CACHE_DST: [&str; 28] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7",
    "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "t3", "t4",
];

/// Registers the code slot's instruction may write, by number: x5–x25, x28, x29.
const SLOT_RD: [u32; 23] = [5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 28, 29];

/// `jalr x0, 0(t5)`: the code slot's return.
const SLOT_RETURN: u32 = (30 << 15) | 0x67;

/// One program that works the caches over a line port: loads and stores of every width, atomics and reservations
/// across eight regions that alias on every cache line (so dirty lines are evicted and refilled constantly), and a code
/// slot that is run, rewritten — sometimes a halfword at a time — made visible with fence.i, and run again. It ends by
/// reading back every word it may have written, and with an `ecall`. Machine mode, no system calls: for the lockstep
/// testbenches, where every load's value and every store is compared with the model.
pub fn cache_program(seed: u64, instructions: usize) -> String {
    let mut rng = Rng::new(seed);
    let mut s = String::from("  .text\n  .globl _start\n_start:\n  la s11, area\n");
    s.push_str(&format!("  li s10, {}\n  add s10, s10, s11\n", CACHE_REGIONS * CACHE_STRIDE));
    for r in CACHE_DST.iter().skip(1) {
        s.push_str(&format!("  li {r}, {:#x}\n", rng.value()));
    }
    s.push_str(&format!(
        "  li t6, {SLOT_RETURN:#x}\n  sw t6, 4(s10)\n  li t6, 0x13\n  sw t6, 0(s10)\n  fence.i\n  jalr t5, 0(s10)\n"
    ));
    let address = |rng: &mut Rng, size: u64| -> u64 {
        rng.below(CACHE_REGIONS) * CACHE_STRIDE + rng.below(CACHE_SPAN / size) * size
    };
    for _ in 0..instructions {
        let d = reg(&mut rng, &CACHE_DST);
        let a = reg(&mut rng, &SRC);
        let b = reg(&mut rng, &SRC);
        match rng.below(12) {
            0..=2 => {
                let (op, size) = [("lb", 1), ("lbu", 1), ("lh", 2), ("lhu", 2), ("lw", 4), ("lwu", 4), ("ld", 8)]
                    [rng.below(7) as usize];
                let at = address(&mut rng, size);
                s.push_str(&format!("  li t6, {at}\n  add t6, t6, s11\n  {op} {d}, 0(t6)\n"));
            }
            3..=5 => {
                let (op, size) = [("sb", 1), ("sh", 2), ("sw", 4), ("sd", 8)][rng.below(4) as usize];
                let at = address(&mut rng, size);
                s.push_str(&format!("  li t6, {at}\n  add t6, t6, s11\n  {op} {a}, 0(t6)\n"));
            }
            6 => {
                let (w, size) = if rng.below(2) == 0 { ("w", 4) } else { ("d", 8) };
                let at = address(&mut rng, size);
                s.push_str(&format!("  li t6, {at}\n  add t6, t6, s11\n  {}.{w} {d}, {b}, (t6)\n", rng.pick(&AMOS)));
            }
            7 => {
                // A reservation, sometimes with another region's line evicting its line before the SC.
                let (w, size) = if rng.below(2) == 0 { ("w", 4) } else { ("d", 8) };
                let at = address(&mut rng, size);
                let other = if rng.below(3) == 0 { address(&mut rng, size) } else { at };
                let d2 = reg(&mut rng, &CACHE_DST);
                s.push_str(&format!("  li t6, {at}\n  add t6, t6, s11\n  lr.{w} {d}, (t6)\n"));
                if rng.below(2) == 0 {
                    let evict = address(&mut rng, 8);
                    s.push_str(&format!("  li t6, {evict}\n  add t6, t6, s11\n  sd {a}, 0(t6)\n"));
                }
                s.push_str(&format!("  li t6, {other}\n  add t6, t6, s11\n  sc.{w} {d2}, {b}, (t6)\n"));
            }
            8 => {
                // Rewrite the code slot, make it visible to fetch, and run it.
                let rd = SLOT_RD[rng.below(SLOT_RD.len() as u64) as usize];
                let imm = rng.below(4096) as u32;
                let funct3 = [0u32, 4, 6, 7][rng.below(4) as usize];
                let enc = (imm << 20) | (rd << 15) | (funct3 << 12) | (rd << 7) | 0x13;
                if rng.below(3) == 0 {
                    s.push_str(&format!(
                        "  li t6, {:#x}\n  sh t6, 0(s10)\n  li t6, {:#x}\n  sh t6, 2(s10)\n",
                        enc & 0xffff,
                        enc >> 16
                    ));
                } else {
                    s.push_str(&format!("  li t6, {enc:#x}\n  sw t6, 0(s10)\n"));
                }
                s.push_str("  fence.i\n  jalr t5, 0(s10)\n");
            }
            9 => {
                let imm = rng.below(4096) as i64 - 2048;
                s.push_str(&format!("  {} {d}, {a}, {imm}\n", rng.pick(&I_OPS)));
            }
            _ => s.push_str(&format!("  {} {d}, {a}, {b}\n", rng.pick(&R_OPS))),
        }
    }
    for k in 0..CACHE_REGIONS {
        s.push_str(&format!("  li t6, {}\n  add t6, t6, s11\n", k * CACHE_STRIDE));
        for o in (0..CACHE_SPAN).step_by(8) {
            s.push_str(&format!("  ld t4, {o}(t6)\n"));
        }
    }
    s.push_str("  lw t4, 0(s10)\n  ecall\n");
    s.push_str(&format!("  .bss\n  .align 14\narea: .zero {}\n", CACHE_REGIONS * CACHE_STRIDE + 64));
    s
}
