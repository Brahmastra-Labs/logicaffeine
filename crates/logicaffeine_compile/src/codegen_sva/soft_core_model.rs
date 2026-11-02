//! The cycle-accurate Rust model of `benchmarks/silicon/soft_core.v` — the executable spec of the
//! LOGOS universal interpreter in silicon. Fetch/decode/execute over a chip-word program
//! (`[opcode:8][dst:8][a:8][b:8]`; wide/float/heap ops per the doc on [`run_soft_core_typed`]),
//! returning the `Show` sequence. This is the differential ORACLE the whole spatial-interpreter
//! campaign checks against: `compile_to_full_words` -> [`run_soft_core_typed`] must equal
//! `compile::vm_outcome` for every program, and the iverilog testbenches agree with it. Lifted here
//! (from `rtl.rs`'s test module) so both the in-crate RTL tests and the `logicaffeine_tests`
//! integration locks share ONE oracle.

/// The Rust model of `alu.v` (32-bit), mirroring `emit_alu_datapath_verilog` exactly (wrapping /
/// signed). Div / ExactDiv / Mod / FloorDiv are NOT here — they route to [`soft_core_divider`]
/// (mirroring the soft-core's `shared_divider`). DivPow2 stays (a cheap arithmetic shift).
pub fn soft_core_alu(opcode: u8, a: i32, b: i32) -> i32 {
    match opcode {
        8 | 9 => a.wrapping_add(b), // Add / AddAssign
        10 => a.wrapping_sub(b),    // Sub
        11 => a.wrapping_mul(b),    // Mul
        16 => {
            // DivPow2 (signed ÷2^b, toward zero)
            let k = (b & 31) as u32;
            let bias = if a < 0 {
                (1u32 << k).wrapping_sub(1) as i32
            } else {
                0
            };
            a.wrapping_add(bias) >> k
        }
        19 => a ^ b,
        20 => a & b,
        21 => a | b,
        24 => !a,
        22 => a.wrapping_shl((b & 31) as u32), // Shl
        23 => a.wrapping_shr((b & 31) as u32), // Shr (arithmetic on i32)
        25 => (a < b) as i32,                  // signed comparisons → 0/1
        26 => (a > b) as i32,
        27 => (a <= b) as i32,
        28 => (a >= b) as i32,
        29 | 30 => (a == b) as i32, // Eq / ApproxEq (integer sub-domain: below the 1e-9 relative-tolerance floor, ≈ collapses to =; float-tolerance is the FPU coprocessor)
        31 => (a != b) as i32,
        _ => 0,
    }
}

/// The Rust model of the soft-core's `shared_divider` unit (truncated signed division): Div /
/// ExactDiv → quotient, Mod → remainder (dividend's sign), FloorDiv → floor-adjusted quotient;
/// b==0 → 0. `run_soft_core` issues opcodes 12/13/14/15 here instead of the combinational ALU,
/// mirroring the RTL (the divider is proven == this by iverilog `tb_shared_divider`).
pub fn soft_core_divider(opcode: u8, a: i32, b: i32) -> i32 {
    if b == 0 {
        return 0;
    }
    match opcode {
        12 | 13 => a.wrapping_div(b), // Div / ExactDiv → quotient
        15 => a.wrapping_rem(b),      // Mod → remainder
        14 => {
            // FloorDiv (toward −∞)
            let q = a.wrapping_div(b);
            if a.wrapping_rem(b) != 0 && ((a < 0) != (b < 0)) {
                q - 1
            } else {
                q
            }
        }
        _ => 0,
    }
}

// ── WIDTH TOGGLE (VM⇄Fabric parity) — the datapath width is a PARAMETER, not hard-wired to 32 bits.
// The chip and the VM diverge only because the scalar core is 32-bit while the VM's Int is unbounded (it
// promotes to BigInt): a benchmark PRNG doing `(seed * 1103515245) % 2147483648` needs a 64-bit intermediate
// and a `2^31` constant that a 32-bit core cannot hold. So the fix is a WIDTH toggle: one datapath that builds
// a 32-bit core (small, Sky130 tape-out) or a 64-bit core (VM parity for the i64 range), by computing in i64
// and SIGN-EXTENDING to `width` bits after each op. `soft_core_alu`/`soft_core_divider` (above) are exactly the
// `width == 32` case, kept byte-identical; these are the parametric forms the 64-bit core + the future
// `run_soft_core_w` use.

/// Sign-extend the low `width` bits of a value to a full i64 — a fixed-width datapath's result register.
/// `width >= 64` is the identity; `width == 32` reproduces the 32-bit core's `as i32` truncation exactly. The
/// SAME logic yields a 32-, 64-, or any-power-of-two-width core — the toggle.
pub fn sign_extend_to_width(v: i64, width: u32) -> i64 {
    if width >= 64 {
        v
    } else {
        let s = 64 - width;
        (v << s) >> s
    }
}

/// Width-parametric model of `alu.v` — computes in i64 and sign-extends to `width` bits. [`soft_core_alu`] is
/// the `width == 32` case (proven equal by `alu_width_toggle_is_32bit_faithful_and_64bit_wide`). Div/Mod/
/// FloorDiv route to [`soft_core_divider_w`], mirroring the shared divider.
pub fn soft_core_alu_w(opcode: u8, a: i64, b: i64, width: u32) -> i64 {
    let sh = (b as u64 & u64::from(width - 1)) as u32; // shift amount, masked to the datapath width
    let r = match opcode {
        8 | 9 => a.wrapping_add(b), // Add / AddAssign
        10 => a.wrapping_sub(b),    // Sub
        11 => a.wrapping_mul(b),    // Mul
        16 => {
            // DivPow2 (signed ÷2^b, toward zero)
            let bias = if a < 0 {
                1i64.wrapping_shl(sh).wrapping_sub(1)
            } else {
                0
            };
            a.wrapping_add(bias) >> sh
        }
        19 => a ^ b,
        20 => a & b,
        21 => a | b,
        24 => !a,
        22 => a.wrapping_shl(sh),      // Shl
        23 => a.wrapping_shr(sh),      // Shr (arithmetic)
        25 => return i64::from(a < b), // signed comparisons → 0/1 (already width-safe)
        26 => return i64::from(a > b),
        27 => return i64::from(a <= b),
        28 => return i64::from(a >= b),
        29 | 30 => return i64::from(a == b),
        31 => return i64::from(a != b),
        _ => return 0,
    };
    sign_extend_to_width(r, width)
}

/// Width-parametric model of the shared divider (truncated signed division), sign-extended to `width`.
/// [`soft_core_divider`] is the `width == 32` case.
pub fn soft_core_divider_w(opcode: u8, a: i64, b: i64, width: u32) -> i64 {
    if b == 0 {
        return 0;
    }
    let r = match opcode {
        12 | 13 => a.wrapping_div(b), // Div / ExactDiv → quotient
        15 => a.wrapping_rem(b),      // Mod → remainder
        14 => {
            // FloorDiv (toward −∞)
            let q = a.wrapping_div(b);
            if a.wrapping_rem(b) != 0 && ((a < 0) != (b < 0)) {
                q - 1
            } else {
                q
            }
        }
        _ => return 0,
    };
    sign_extend_to_width(r, width)
}

// ── AUTO-PROMOTING VALUE-INTEGER ALU/DIVIDER (VM-EXACT) ──────────────────────────────────────────────
// The datapath's VALUE integers are unbounded, EXACTLY as the VM's `Int` (`crate::vm::value::Value` /
// `RuntimeValue`): a FAST `Small(i64)` path (the common case, keeping the model fast + the 32-bit oracle
// byte-identical) that AUTO-PROMOTES to an exact `Big(BigInt)` on i64 overflow, then normalizes back to
// `Small` when the result fits `i64` again (`RegVal::from_big`, mirroring `RuntimeValue::from_bigint`).
// So the chip differs NOWHERE from the VM on integers — no `mod 2^width`, no range restriction. The width
// no longer masks value arithmetic; it is a perf/area knob for the fast path and the raw-i64 output view
// (`run_soft_core_w`), never a correctness switch. These mirror `crate::semantics::arith`'s `add`/
// `subtract`/`multiply`/`divide`/`modulo`/`floor_divide` line-for-line (checked-i64 fast path, BigInt
// exact fallback, truncating Div/Mod vs floor FloorDiv, bitwise/shift on the i64 pattern only — the VM's
// bitwise/shift path is i64-only and never promotes).

/// The value-integer ALU: `Add`/`Sub`/`Mul` promote to `BigInt` on i64 overflow (VM-exact); `DivPow2`,
/// bitwise (`^ & | ~`), and shifts compute on the i64 pattern (the VM's bitwise/shift path is i64-only —
/// SHA/Word programs stay in range); comparisons return `Small(0/1)`. Mirrors `arith::{add,subtract,
/// multiply}` and `binary_op`'s bitwise/shift arms exactly.
fn soft_core_alu_regval(opcode: u8, a: &RegVal, b: &RegVal) -> RegVal {
    match opcode {
        8 | 9 => match (a, b) {
            (RegVal::Small(x), RegVal::Small(y)) => match x.checked_add(*y) {
                Some(s) => RegVal::Small(s),
                None => RegVal::from_big(a.to_big().add(&b.to_big())),
            },
            _ => RegVal::from_big(a.to_big().add(&b.to_big())),
        },
        10 => match (a, b) {
            (RegVal::Small(x), RegVal::Small(y)) => match x.checked_sub(*y) {
                Some(s) => RegVal::Small(s),
                None => RegVal::from_big(a.to_big().sub(&b.to_big())),
            },
            _ => RegVal::from_big(a.to_big().sub(&b.to_big())),
        },
        11 => match (a, b) {
            (RegVal::Small(x), RegVal::Small(y)) => match x.checked_mul(*y) {
                Some(p) => RegVal::Small(p),
                None => RegVal::from_big(a.to_big().mul(&b.to_big())),
            },
            _ => RegVal::from_big(a.to_big().mul(&b.to_big())),
        },
        16 => {
            // DivPow2 (signed ÷2^b, toward zero)
            // The optimizer's lowering of `Div` by a power of two — so it must equal the VM's TRUNCATING
            // `Div` (`arith::divide`). `a >> sh` with the round-to-zero bias is exactly `a / 2^sh` toward
            // zero for a `Small`; a `Big` dividend divides exactly through the BigInt truncating `div_rem`.
            let sh = (b.sm() as u64 & 63) as u32;
            match a {
                RegVal::Small(x) => {
                    let bias = if *x < 0 {
                        1i64.wrapping_shl(sh).wrapping_sub(1)
                    } else {
                        0
                    };
                    RegVal::Small(x.wrapping_add(bias) >> sh)
                }
                RegVal::Big(x) => {
                    let divisor = BigInt::from_i64(2).pow(sh);
                    RegVal::from_big(x.div_rem(&divisor).expect("2^sh is nonzero").0)
                }
            }
        }
        19 => RegVal::Small(a.sm() ^ b.sm()), // BitXor — VM: Int ^ Int (i64 only)
        20 => RegVal::Small(a.sm() & b.sm()), // BitAnd
        21 => RegVal::Small(a.sm() | b.sm()), // BitOr
        24 => RegVal::Small(!a.sm()),         // Not (bitwise on the i64 pattern)
        22 => RegVal::Small(a.sm().wrapping_shl(b.sm() as u32)), // Shl — VM: a.wrapping_shl(b as u32)
        23 => RegVal::Small(a.sm().wrapping_shr(b.sm() as u32)), // Shr — VM: a.wrapping_shr(b as u32)
        25 => RegVal::Small(i64::from(regval_cmp(a, b) == std::cmp::Ordering::Less)),
        26 => RegVal::Small(i64::from(regval_cmp(a, b) == std::cmp::Ordering::Greater)),
        27 => RegVal::Small(i64::from(regval_cmp(a, b) != std::cmp::Ordering::Greater)),
        28 => RegVal::Small(i64::from(regval_cmp(a, b) != std::cmp::Ordering::Less)),
        29 | 30 => RegVal::Small(i64::from(regval_cmp(a, b) == std::cmp::Ordering::Equal)),
        31 => RegVal::Small(i64::from(regval_cmp(a, b) != std::cmp::Ordering::Equal)),
        _ => RegVal::Small(0),
    }
}

/// Exact ordering of two value integers — the fast i64 compare when both are `Small`, else an exact
/// `BigInt` compare. Mirrors the VM's `compare` (which promotes both operands to the exact tower).
fn regval_cmp(a: &RegVal, b: &RegVal) -> std::cmp::Ordering {
    match (a, b) {
        (RegVal::Small(x), RegVal::Small(y)) => x.cmp(y),
        _ => a.to_big().cmp(&b.to_big()),
    }
}

/// The value-integer shared divider — TRUNCATING `Div`/`ExactDiv` (op 12/13, toward zero), `Mod` (op 15,
/// remainder carries the dividend's sign), `FloorDiv` (op 14, toward −∞); `b == 0` → `Small(0)` (mirrors
/// the RTL divider's zero guard). Promotes to `BigInt` EXACTLY as `arith::{divide,modulo,floor_divide}`:
/// truncating Div's only i64-overflow case is `i64::MIN / -1 = 2^63`, and FloorDiv floors the exact
/// rational quotient. Big operands divide exactly through `BigInt::div_rem`.
fn soft_core_divider_regval(opcode: u8, a: &RegVal, b: &RegVal) -> RegVal {
    if let RegVal::Small(0) = b {
        return RegVal::Small(0);
    }
    if let RegVal::Big(bb) = b {
        if bb.is_zero() {
            return RegVal::Small(0);
        }
    }
    match (a, b) {
        (RegVal::Small(x), RegVal::Small(y)) => match opcode {
            12 | 13 => match x.checked_div(*y) {
                // Div / ExactDiv — only i64::MIN/-1 overflows
                Some(q) => RegVal::Small(q),
                None => RegVal::from_big(a.to_big().div_rem(&b.to_big()).expect("nonzero").0),
            },
            15 => RegVal::Small(x.checked_rem(*y).unwrap_or(0)), // Mod — i64::MIN % -1 == 0
            14 => match x.checked_div(*y) {
                // FloorDiv (toward −∞)
                // The only i64 overflow is `i64::MIN / -1` (an EXACT division, remainder 0 → no floor
                // adjustment), handled exactly on the BigInt path. Otherwise: the truncating quotient,
                // decremented when the division is inexact and the operands have opposite signs.
                Some(q) => {
                    if x.wrapping_rem(*y) != 0 && ((*x < 0) != (*y < 0)) {
                        RegVal::Small(q - 1)
                    } else {
                        RegVal::Small(q)
                    }
                }
                None => RegVal::from_big(bigint_floor_div(&a.to_big(), &b.to_big())),
            },
            _ => RegVal::Small(0),
        },
        _ => {
            let (aa, bb) = (a.to_big(), b.to_big());
            match opcode {
                12 | 13 => RegVal::from_big(aa.div_rem(&bb).expect("nonzero").0),
                15 => RegVal::from_big(aa.div_rem(&bb).expect("nonzero").1),
                14 => RegVal::from_big(bigint_floor_div(&aa, &bb)),
                _ => RegVal::Small(0),
            }
        }
    }
}

/// `⌊a / b⌋` on `BigInt`s (toward −∞) — the exact floor of the rational quotient, matching the VM's
/// `arith::floor_divide` (`(a/b).floor()`): the truncating quotient, decremented by one when the division
/// is inexact and the operands have opposite signs.
fn bigint_floor_div(a: &BigInt, b: &BigInt) -> BigInt {
    let (q, r) = a.div_rem(b).expect("nonzero divisor");
    if !r.is_zero() && (a.is_negative() != b.is_negative()) {
        q.sub(&BigInt::from_i64(1))
    } else {
        q
    }
}

use logicaffeine_base::BigInt;

/// A VALUE integer register cell — the chip's exact mirror of the VM's `Int`/`BigInt` tower. The common
/// case is a machine word (`Small`, the FAST path that keeps the 32-bit oracle byte-identical and the
/// model fast over the 5e8-cycle budget); an integer that overflows the datapath promotes to an exact
/// `Big` (unbounded [`BigInt`]), EXACTLY as `crate::vm::value::Value` / `RuntimeValue::Int` promotes to
/// `RuntimeValue::BigInt`. INVARIANT (mirrors `RuntimeValue::from_bigint`): a `Big` never holds a value
/// that fits `i64` — every arithmetic result normalizes back to `Small` when it fits — so a value has one
/// canonical representation and the printed decimal is unique. Pointers, list lengths, map tags, iterator
/// cursors, and register indices stay `Small` (they are addresses, never over-i64), so the structural
/// heap/pointer machinery reads them through `RegVal::sm` with no BigInt on the hot structural path.
#[derive(Clone, PartialEq, Debug)]
pub enum RegVal {
    Small(i64),
    Big(BigInt),
}

impl Default for RegVal {
    fn default() -> Self {
        RegVal::Small(0)
    }
}

impl RegVal {
    /// Build a value integer from a `BigInt`, DOWNSIZING to [`RegVal::Small`] whenever it fits `i64` —
    /// the chip's mirror of `RuntimeValue::from_bigint`, which is what keeps the representation canonical
    /// (a `Big` is always genuinely over-i64) so the printed decimal is identical to the VM's.
    fn from_big(b: BigInt) -> RegVal {
        match b.to_i64() {
            Some(n) => RegVal::Small(n),
            None => RegVal::Big(b),
        }
    }

    /// This value as a `BigInt` (exact for both arms) — the operand view the overflow path promotes into.
    fn to_big(&self) -> BigInt {
        match self {
            RegVal::Small(n) => BigInt::from_i64(*n),
            RegVal::Big(b) => b.clone(),
        }
    }

    /// The `Small` machine word — the address/length/tag/cursor view used by the structural heap and
    /// pointer machinery. A `Big` value never reaches here (it is only ever a VALUE integer, never an
    /// address); the `to_i64().unwrap_or` narrows defensively (the low bits) so a stray over-i64 index
    /// wraps rather than panicking, exactly as a fixed-width address register would.
    fn sm(&self) -> i64 {
        match self {
            RegVal::Small(n) => *n,
            RegVal::Big(b) => b.to_i64().unwrap_or(0),
        }
    }

    /// The `Small` machine word as a heap/register index.
    #[inline]
    fn idx(&self) -> usize {
        self.sm() as usize
    }
}

/// One `Show` output of the soft-core, tagged by the datapath that produced it: an integer `Show`
/// (opcode 89, the 32-bit ALU/heap result) or a FLOAT `Show` (opcode 212 `ShowF`, an f64 from the
/// FPU float bank). The chip is dynamically typed at the VM level, but the encoder statically lowers
/// each `Show` to the one whose operand type it proved — so the model carries the same distinction and
/// [`ShowVal::fmt`] renders each exactly as the VM does (ints bare, floats via `fmt_f64`).
#[derive(Clone, PartialEq, Debug)]
pub enum ShowVal {
    Int(i64),
    /// A BIGINT integer `Show` — the same integer datapath as [`ShowVal::Int`], but the value OVERFLOWED
    /// the machine word and promoted to an exact [`BigInt`], mirroring the VM's `RuntimeValue::BigInt`. The
    /// 32-bit oracle and every in-range program stay on `Int` (the fast path is unchanged); only a value
    /// genuinely exceeding `i64` lands here, and [`ShowVal::fmt`] prints its full decimal byte-identically
    /// to `logicaffeine_base::BigInt`'s `Display` (the same string the VM's `Show` emits).
    BigInt(BigInt),
    Float(f64),
    /// A TEXT `Show` (opcode 217 `ShowText`) — the string decoded from the char-code list on the heap.
    /// The chip sends the char bytes over UART (reply tag 2); the host reassembles the string. The model
    /// carries the reconstituted `String` so `fmt` renders it byte-identically to the VM's Text `Show`.
    Text(String),
    /// A WORD32 `Show` (opcode 221 `ShowWord32`) — the same 32-bit ALU/heap result as an integer `Show`,
    /// but the register holds a `Word32` (or an `intOfWord32` unsigned value), so it renders UNSIGNED
    /// (0..2³²−1) rather than the `i32` two's-complement reinterpretation. Carries the `u32` so `fmt`
    /// matches the VM's `Word32`/unsigned-`Int` `Show` exactly (SHA-256's hash words exceed 2³¹).
    Word32(u32),
}
impl ShowVal {
    /// Render exactly as the VM's `Show` does: an integer bare, a float through the ONE float-display
    /// authority (`logicaffeine_data::fmt::fmt_f64` — the same path `RuntimeValue::to_display_string`
    /// uses), a Text as its raw characters (`RuntimeValue::Text(s) => s`), a `Word32` as its UNSIGNED
    /// decimal value (`Word32`'s `Display` prints the underlying `u32`), so the model's output string is
    /// byte-identical to `vm_outcome`.
    pub fn fmt(&self) -> String {
        match self {
            ShowVal::Int(v) => v.to_string(),
            ShowVal::BigInt(b) => b.to_string(),
            ShowVal::Float(f) => logicaffeine_data::fmt::fmt_f64(*f),
            ShowVal::Text(s) => s.clone(),
            ShowVal::Word32(w) => w.to_string(),
        }
    }
    /// The integer payload of an integer `Show` — panics on a float / text / word `Show`. Used only by
    /// the pure integer differential/testbench paths (`run_soft_core`), which never emit
    /// `ShowF`/`ShowText`/`ShowWord32`. Returns the full `i64` (the datapath width toggle carries 64-bit
    /// values at `width == 64`; the 32-bit core's values are the low 32 bits sign-extended, identical).
    pub fn as_i64(&self) -> i64 {
        match self {
            ShowVal::Int(v) => *v,
            ShowVal::BigInt(b) => panic!("integer soft-core run emitted a bigint Show ({b})"),
            ShowVal::Float(f) => panic!("integer soft-core run emitted a float Show ({f})"),
            ShowVal::Text(s) => panic!("integer soft-core run emitted a text Show ({s:?})"),
            ShowVal::Word32(w) => panic!("integer soft-core run emitted a word32 Show ({w})"),
        }
    }
}

/// A cycle-accurate Rust model of `benchmarks/silicon/soft_core.v` — fetch/decode/execute over a
/// chip-word program (`[opcode:8][dst:8][a:8][b:8]`; LoadConst/Jump use `[op][dst][imm:16]`). Returns
/// the sequence of `Show` outputs (integer or float — see [`ShowVal`]). Mirrors the RTL `case` exactly;
/// the executable spec the ≡VM differential checks and the iverilog testbench agrees with.
///
/// FLOAT ARCHITECTURE (mirrors `soft_core.v`'s `fregs`/`fpu.v` integration): a parallel 64-bit float
/// bank `fregs` sits beside the 32-bit `regs`, same index space — VM register R holds either an int in
/// `regs[R]` or an f64 in `fregs[R]`, disambiguated by the op's statically-lowered type. The float
/// opcodes (chip-synthetic, above the ISA table like `LoadConstWide`=200, so no `Op` variant / no ISA
/// drift): `FConst`=201 (3-word: op + 2 immediate words = the f64 bits), `FAdd`=202 `FSub`=203
/// `FMul`=204 `FDiv`=205 `FSqrt`=206 (dispatch to fpu.v, bit-exact via real `f64`), `FLt`=207 `FEq`=208
/// (write an int 0/1 into `regs` so branches work), `IntToF`=209 `FToInt`=210 (the Int↔Float boundary),
/// `FMove`=211, `ShowF`=212. The FLOAT-HEAP PLANE (213..=215) lets a `Seq of Float` ride the heap: the
/// structure (header + int cells) lives in `heap` and the f64 payloads on the parallel `fheap` plane
/// (`FListPush`=213 grows both + COWs both, `FIndex`=214 reads a payload, `FSetIndex`=215 writes one).
///
/// The TEXT / SEQ BUILTIN opcodes (chip-synthetic 233..=236) lower the deterministic string/fill/parse
/// builtins the CHIP compiler emits from source: `Chr`=233 (a 1-char Text `[1, codepoint]` from a codepoint,
/// trap on an invalid one), `TextContains`=234 (a substring char-scan over two Text char-lists → 0/1),
/// `RepeatSeq`=235 (an `n`-slot fresh Seq, every slot the same SCALAR element), `ParseInt`=236 (a bounded
/// decimal char-scan over a Text char-list → the i64 value, trap on a non-numeric / empty / overflowing
/// input). Each is byte-identical to the matching `semantics::builtins::call_builtin` arm; the remaining
/// reference/host builtin families (parseFloat/format/uuid/…) stay off the chip (the encoder rejects them
/// loudly — see `full_asm::encode_op`).
///
/// The integer datapath WIDTH is fixed at 32 bits (the tape-out core). For VM parity on 64-bit-range
/// arithmetic, use [`run_soft_core_typed_w`] with `width == 64`.
thread_local! {
    /// DATA-segment preload for [`run_soft_core_typed_preloaded`]: seeds `heap[0..]` before the run so the
    /// model mirrors the SoC's UART DATA phase (weights streamed into the heap; `loaded()` = the Seq at ptr 0).
    static HEAP_PRELOAD: std::cell::RefCell<Vec<i64>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Run with `preload` seeded into the heap as `heap[0..preload.len()]` — the model twin of the SoC's weight
/// DATA phase. With `preload = [len, w0, w1, …]` that is a `Seq of Int` at pointer 0, so the compiled
/// `loaded()` (pointer 0) makes `item i of loaded()` read `heap[i]`. The bump allocator continues after it.
pub fn run_soft_core_typed_preloaded(
    imem: &[u32],
    max_cycles: usize,
    preload: &[i64],
) -> Vec<ShowVal> {
    HEAP_PRELOAD.with(|h| *h.borrow_mut() = preload.to_vec());
    let out = run_soft_core_typed_w(imem, max_cycles, 32);
    HEAP_PRELOAD.with(|h| h.borrow_mut().clear());
    out
}

pub fn run_soft_core_typed(imem: &[u32], max_cycles: usize) -> Vec<ShowVal> {
    run_soft_core_typed_w(imem, max_cycles, 32)
}

/// The WIDTH-PARAMETRIC form of [`run_soft_core_typed`] — the real fetch/decode/execute loop. Every
/// integer VALUE is an auto-promoting [`RegVal`] (fast `Small(i64)` + exact `Big(BigInt)` on overflow),
/// so the datapath's `Int` is UNBOUNDED exactly like the VM's — the chip differs NOWHERE from the VM on
/// integers at ANY `width`. This is a REFRAME of the old width toggle: `width` no longer masks value
/// arithmetic (a product that overflows i32/i64 PROMOTES, matching the VM, rather than wrapping), so
/// `width == 32` and `width == 64` both equal `vm_outcome` on the `ShowVal` output. `width` survives as a
/// perf/area knob and the datapath-width parameter for the raw-i64 OUTPUT VIEW ([`run_soft_core_w`], which
/// masks each value to `width` bits to model a fixed-width hardware output register). Addresses/iterators
/// stay `usize`; the float bank stays f64 (width-independent).
pub fn run_soft_core_typed_w(imem: &[u32], max_cycles: usize, width: u32) -> Vec<ShowVal> {
    run_soft_core_traced(imem, max_cycles, width).0
}

/// How the core stopped with a FAILURE (a trap-halt rather than a clean `Halt`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChipFailure {
    /// A `FailWith` whose reason the program streamed over the Text port just before it (`reasoned`).
    Reason(String),
    /// A trap with no text: a failed policy check, an invalid codepoint, a `parseInt` error.
    Trap,
}

/// A run of the core: everything it showed, and how it failed if it did.
#[derive(Clone, Debug)]
pub struct SoftCoreRun {
    pub shows: Vec<ShowVal>,
    pub failure: Option<ChipFailure>,
}

/// Run `imem` and report the failure contract the RTL's `failed`/`reasoned` ports carry: a reasoned
/// trap's last Text record IS its reason (so it leaves `shows`); a trap without text is `Trap`.
pub fn run_soft_core_outcome(imem: &[u32], max_cycles: usize) -> SoftCoreRun {
    let (mut shows, trap) = run_soft_core_traced(imem, max_cycles, 32);
    let failure = match trap {
        None => None,
        Some(false) => Some(ChipFailure::Trap),
        Some(true) => match shows.pop() {
            Some(ShowVal::Text(reason)) => Some(ChipFailure::Reason(reason)),
            other => {
                shows.extend(other);
                Some(ChipFailure::Trap)
            }
        },
    };
    SoftCoreRun { shows, failure }
}

/// The fetch/decode/execute loop: the shows, and `Some(reasoned)` when it trap-halted.
fn run_soft_core_traced(
    imem: &[u32],
    max_cycles: usize,
    width: u32,
) -> (Vec<ShowVal>, Option<bool>) {
    let mut trap: Option<bool> = None;
    // The typed loop is now width-TRANSPARENT (value integers are unbounded, VM-exact). `width` is kept in
    // the signature for API stability and for the raw-i64 output view; the loop itself ignores it.
    let _ = width;
    // The value-integer register file — each cell an auto-promoting `RegVal` (a FAST `Small(i64)` common
    // case + an exact `Big(BigInt)` overflow fallback), so a VALUE integer is UNBOUNDED exactly like the
    // VM's `Int`. Pointers, lengths, tags, and cursors live here too but always as `Small` (they are
    // addresses, never over-i64), read through `.sm()`/`.idx()`.
    let mut regs: [RegVal; 256] = std::array::from_fn(|_| RegVal::Small(0));
    // The 64-bit FLOAT BANK: same 256-entry index space as `regs`, mirroring soft_core.v's `fregs`.
    // A register holds an f64 here when the op that wrote it was a float op; an int lives in `regs`.
    // Bits are carried EXACTLY (`f64::from_bits`/`to_bits`) so the model == the RTL == Rust f64. The float
    // bank is WIDTH-INDEPENDENT (IEEE-754 f64) — the width toggle only governs the integer datapath.
    let mut fregs = [0.0f64; 256];
    // The 256-BIT BIGINT LANE: an 8×256-bit wide register bank (4 little-endian u64 limbs each),
    // beside the 32-bit regfile. `BigIntWide` (op 114) routes it to the bigint_alu/div/mul blocks.
    let mut wide: [[u64; 4]; 8] = [[0u64; 4]; 8];
    let mut pc: usize = 0;
    let mut out: Vec<ShowVal> = Vec::new();
    // Call/return frame stack: (saved int window, saved float window, return pc, result reg) — mirrors
    // soft_core.v (the frame engine snapshots the whole 256-register window, both banks, per call).
    let mut frames: Vec<([RegVal; 256], [f64; 256], usize, usize)> = Vec::new();
    // The LOAD-STORE unit's heap: a flat data memory holding structured values. A list at pointer `p`
    // is `heap[p] = length` followed by the `length` elements at `heap[p+1..=p+length]`, so the 1-based
    // `item i of xs` is `heap[p + i]` (mirrors `mem_unit.v` + the VM's `RuntimeValue::List`). A cell holds
    // a `RegVal` because a collection ELEMENT can be an over-i64 `Big` value (a `Seq of Int` of big ints);
    // headers/tags/pointers stay `Small`, read through `.sm()`.
    let mut heap: Vec<RegVal> =
        HEAP_PRELOAD.with(|h| h.borrow().iter().map(|&x| RegVal::Small(x)).collect());
    // The FLOAT-HEAP PLANE — a dense f64 payload plane running PARALLEL to `heap` at the SAME indices
    // (RTL: `fdmem`, two 32-bit BRAMs beside `dmem`). A `Seq of Float` handle is still an INT pointer into
    // `heap` (which keeps the `[length, …]` header + all structure); its element PAYLOADS ride `fheap` at
    // the same slots. INVARIANT: `fheap.len() == heap.len()` ALWAYS — enforced by a single resize at the
    // bottom of the fetch loop, so every op that grows `heap` (int lists, maps, ranges, …) leaves a
    // matching `0.0` float cell behind with no per-op bookkeeping. The float-list ops (213/214/215) read
    // and write this plane; the COW in FListPush copies BOTH planes so a float seq's payload survives.
    let mut fheap: Vec<f64> = vec![0.0; heap.len()]; // INVARIANT: fheap.len() == heap.len() (preload too)
                                                     // The iterator stack: each `for x in xs` pushes a `(collection pointer, 1-based cursor)`; IterNext
                                                     // reads `heap[ptr + cursor]` and advances, jumping to `exit` when the cursor passes the length.
    let mut iters: Vec<(usize, usize)> = Vec::new();
    // A MAP is a header-tagged heap block `[MAP_TAG|npairs, k0, v0, k1, v1, …]`. The tag bit (never set by a
    // plain list length) lets the shared Length/Contains ops branch to key-scan behaviour for maps and stay
    // positional for lists. Each key cell `kᵢ` is EITHER a char-list POINTER (a `Map of Text to …`, accessed
    // through the string-key ops SetIndex/Index 57/59 which compare with `key_eq`) OR a direct INTEGER key (a
    // `Map of Int to …`, accessed through the int-key ops MapSetIndexInt/MapIndexInt 224/223 which compare with
    // `==`). The encoder picks the op family from the key expression's static type — the run loop never mixes
    // them within one map, so the key cell's interpretation is unambiguous per access op.
    const MAP_TAG: i64 = 0x4000_0000;
    // char-list key equality: the string at `p` equals the string at `q` (same length + same codepoints).
    let key_eq = |heap: &[RegVal], p: usize, q: usize| -> bool {
        if heap[p].sm() != heap[q].sm() {
            return false;
        }
        for j in 1..=(heap[p].sm() as usize) {
            if heap[p + j].sm() != heap[q + j].sm() {
                return false;
            }
        }
        true
    };
    // The globals table — Main top-level `Let`s promoted to globals when a function body references them.
    // It lives OUTSIDE the register-window frame stack, so a global survives Call/Return (a callee reads
    // the same slot the caller wrote), exactly as the VM's runtime globals table does.
    let mut globals: [RegVal; 256] = std::array::from_fn(|_| RegVal::Small(0));
    // Select arms: `Await the first of:` registers `(chan_reg, var_reg)` recv arms (chan_reg = usize::MAX
    // marks an `After ticks` timeout arm); SelectWait fires the FIRST ready recv (deterministic), else the
    // timeout arm, and reports the fired arm index.
    let mut sel_arms: Vec<(usize, usize)> = Vec::new();
    // The networking coprocessor's OFFLINE LOOPBACK inbox — the single-node projection of the relay:
    // `NetSend to <peer>` PUBLISHES `(peer key, message)` here; `NetAwait from <peer>` DRAINS the entry
    // whose key matches (the VM's `offline_loopback` — a message a node sends to a peer it also awaits
    // comes back to it). The peer key is the `NetMakePeer` handle (Send and Await name the same peer).
    let mut inbox: Vec<(RegVal, RegVal)> = Vec::new();
    let mut peer_seq: i64 = 0;
    for _ in 0..max_cycles {
        if pc >= imem.len() {
            break;
        }
        let instr = imem[pc];
        let opcode = (instr >> 24) as u8;
        let dst = ((instr >> 16) & 0xff) as usize;
        let ra = ((instr >> 8) & 0xff) as usize;
        let rb = (instr & 0xff) as usize;
        let imm = i64::from(instr as i16); // LoadConst immediate: low 16 bits, SIGN-extended (−32768..=32767); fits any width, no mask
        let target = ((instr >> 16) & 0xff) as usize; // 8-bit entry byte (Spawn/SpawnHandle; wide targets read below)
                                                      // Branch truthiness (`av == 0`) tests the VALUE: a `Big` is never zero (canonical repr), so
                                                      // `is_zero_val` is the exact truthiness the VM's branch uses. Arithmetic arms take the full
                                                      // `RegVal` (`&regs[..]`) so an over-i64 `Big` operand promotes correctly; structural arms read the
                                                      // `.sm()` i64 address view.
        let is_zero_val = |v: &RegVal| matches!(v, RegVal::Small(0));
        match opcode {
            // Reversible/adiabatic charge-recovery core (wire opcodes 0..4) — the abelian bijection
            // datapath (`rev_alu_core.v`), now integrated into the CPU so ONE core executes every op.
            // The reversible core's Add/Sub/Neg AUTO-PROMOTE like every value add/sub/neg (VM-exact); Xor
            // is the i64-only bitwise the VM's `Int ^ Int` computes. `place op= delta` in-place.
            0 => {
                let r = soft_core_alu_regval(8, &regs[dst], &regs[ra]);
                regs[dst] = r;
                pc += 1;
            } // RevAdd { place=dst, delta=ra }
            1 => {
                let r = soft_core_alu_regval(10, &regs[dst], &regs[ra]);
                regs[dst] = r;
                pc += 1;
            } // RevSub
            2 => {
                let r = RegVal::Small(regs[dst].sm() ^ regs[ra].sm());
                regs[dst] = r;
                pc += 1;
            } // RevXor (i64 pattern)
            3 => {
                let r = soft_core_alu_regval(10, &RegVal::Small(0), &regs[dst]);
                regs[dst] = r;
                pc += 1;
            } // RevNeg { place=dst } = 0 - place
            4 => {
                regs.swap(dst, ra);
                pc += 1;
            } // RevSwap { a=dst, b=ra }
            5 => {
                regs[dst] = RegVal::Small(imm);
                pc += 1;
            } // LoadConst — sign-extended 16-bit inline immediate (fits any width)
            200 => {
                regs[dst] = RegVal::Small(i64::from(imem[pc + 1] as i32));
                pc += 2;
            } // LoadConstWide — sign-extended 32-bit immediate in the SECOND word (chip-synthetic opcode, no VM Op)
            222 => {
                // LoadConstWide64 { dst } — 3-word: op + [bits 31:0][bits 63:32] = the full i64 constant
                // The full i64 constant is retired EXACTLY, irrespective of datapath width — NO sign-extension
                // to `width`. This is what lets the full core reproduce a constant wider than i32 (`% 2^31`,
                // `% 2^32` in the benchmark PRNGs, or a full-range i64 literal) at width 32 as faithfully as at
                // 64: the value plane auto-promotes to BigInt on overflow like the VM's unbounded `Int`, so the
                // width is a fast-path/area knob, not a correctness switch. Mirrors `FConst` (op 201): low word
                // then high word. The encoder emits op 222 for every over-i32 constant at every width.
                let bits = (imem[pc + 1] as u64) | ((imem[pc + 2] as u64) << 32);
                regs[dst] = RegVal::Small(bits as i64);
                pc += 3;
            }
            // ── FLOAT DATAPATH (chip-synthetic opcodes 201..=212, mirroring soft_core.v's fregs + fpu.v).
            // A float register lives in `fregs`; ints stay in `regs`. Every arithmetic result is computed
            // with real Rust `f64` (bit-identical to fpu.v's IEEE-754 RNE units) via from_bits/to_bits.
            201 => {
                // FConst { dst } — 3-word: op + [bits 31:0][bits 63:32]
                let bits = (imem[pc + 1] as u64) | ((imem[pc + 2] as u64) << 32);
                fregs[dst] = f64::from_bits(bits);
                pc += 3;
            }
            202 => {
                fregs[dst] = fregs[ra] + fregs[rb];
                pc += 1;
            } // FAdd  (fp_addsub sub=0)
            203 => {
                fregs[dst] = fregs[ra] - fregs[rb];
                pc += 1;
            } // FSub  (fp_addsub sub=1)
            204 => {
                fregs[dst] = fregs[ra] * fregs[rb];
                pc += 1;
            } // FMul  (fp_mul)
            205 => {
                fregs[dst] = fregs[ra] / fregs[rb];
                pc += 1;
            } // FDiv  (fp_div, multi-cycle)
            206 => {
                fregs[dst] = fregs[ra].sqrt();
                pc += 1;
            } // FSqrt (fp_sqrt, multi-cycle)
            207 => {
                regs[dst] = RegVal::Small(i64::from(fregs[ra] < fregs[rb]));
                pc += 1;
            } // FLt -> int 0/1 (fp_cmp lt)
            208 => {
                regs[dst] = RegVal::Small(i64::from(fregs[ra] == fregs[rb]));
                pc += 1;
            } // FEq -> int 0/1 (fp_cmp eq)
            229 => {
                regs[dst] = RegVal::Small(i64::from(fregs[ra] <= fregs[rb]));
                pc += 1;
            } // FLe -> int 0/1 (fp_cmp le = lt|eq; NaN-false, matching the VM's f64 `<=`)
            230 => {
                regs[dst] = RegVal::Small(i64::from(fregs[ra] != fregs[rb]));
                pc += 1;
            } // FNe -> int 0/1 (fp_cmp ne = !eq; NaN-true, matching the VM's f64 `!=`)
            // `is approximately` has NO dedicated opcode: the encoder DECOMPOSES it into FEq/FSub/FAbs (36)/
            // FMax (38)/FConst/FMul/FLe + an int BitOr, reproducing `logos_approx_eq` bit-for-bit with the SAME
            // ops the model already runs above and in the op-38 float coprocessor below.
            209 => {
                fregs[dst] = regs[ra].to_big().to_f64();
                pc += 1;
            } // IntToF { dst, src=ra } — Int -> Float boundary (exact for Big too, VM: `n as f64`/`b.to_f64()`)
            210 => {
                regs[dst] = RegVal::Small(fregs[ra] as i64);
                pc += 1;
            } // FToInt { dst, src=ra } — Float -> Int (truncation toward zero)
            211 => {
                fregs[dst] = fregs[ra];
                pc += 1;
            } // FMove { dst, src=ra } — float register copy
            212 => {
                out.push(ShowVal::Float(fregs[ra]));
                pc += 1;
            } // ShowF { src=ra } — float Show
            // ── FLOAT-HEAP PLANE (chip-synthetic opcodes 213..=215): a `Seq of Float`'s structure lives
            // in the INT heap (header `[length, …]` + int placeholder cells), its f64 element payloads on
            // the parallel `fheap` plane at the SAME slots. Mirrors soft_core.v's `fdmem` beside `dmem`.
            213 => {
                // FListPush { list=dst, value=ra } — integer ListPush structure + float payload
                let ptr = regs[dst].idx();
                let len = heap[ptr].sm() as usize;
                let slot;
                if ptr + 1 + len == heap.len() {
                    // list is on top of the heap: append in place
                    slot = heap.len();
                    heap.push(RegVal::Small(0)); // int placeholder cell (the payload rides fheap)
                    fheap.push(0.0); // keep the plane aligned within the op
                    heap[ptr] = RegVal::Small((len + 1) as i64);
                } else {
                    // else copy-on-write to a fresh block (value semantics)
                    let newptr = heap.len();
                    heap.push(RegVal::Small((len + 1) as i64)); // header
                    fheap.push(0.0);
                    for k in 1..=len {
                        // COPY BOTH PLANES — the float seq's payload survives
                        let v = heap[ptr + k].clone();
                        let f = fheap[ptr + k];
                        heap.push(v);
                        fheap.push(f);
                    }
                    slot = heap.len();
                    heap.push(RegVal::Small(0)); // the appended int placeholder
                    fheap.push(0.0);
                    regs[dst] = RegVal::Small(newptr as i64);
                }
                fheap[slot] = fregs[ra]; // write the f64 payload at the appended slot
                pc += 1;
            }
            214 => {
                // FIndex { dst, coll=ra, index=rb } — read the f64 payload (1-based)
                fregs[dst] = fheap[regs[ra].idx() + regs[rb].idx()];
                pc += 1;
            }
            215 => {
                // FSetIndex { coll=dst, index=ra, value=rb } — write the f64 payload (1-based)
                fheap[regs[dst].idx() + regs[ra].idx()] = fregs[rb];
                pc += 1;
            }
            // ── TEXT (string) PLANE (chip-synthetic opcodes 216/217): a `Text` is a char-code list on the
            // int heap. `TextConst` copies a string's char block out of the imem STRING-CONSTANT POOL into
            // a fresh heap char-list `[len, chars…]`; `ShowText` decodes such a list to a UTF-8 string.
            216 => {
                // TextConst { dst } — 2-word: [216][dst][len:16] ‖ pool_word_offset
                let len = ((instr & 0xffff) as usize) & 0xffff;
                let off = imem[pc + 1] as usize; // imem index of c0 (into the pool, laid out after the code)
                let ptr = heap.len();
                heap.push(RegVal::Small(len as i64)); // header
                for k in 0..len {
                    heap.push(RegVal::Small(i64::from(imem[off + k])));
                } // copy the codepoints (unsigned codepoint)
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 2;
            }
            217 => {
                // ShowText { src=ra } — decode the char list to a UTF-8 string
                let ptr = regs[ra].idx();
                let len = heap[ptr].sm() as usize;
                let mut s = String::with_capacity(len);
                for k in 1..=len {
                    // A char cell is an i32 CODE POINT (as `TextConst`/`FormatValue`/Concat store them).
                    if let Some(ch) = char::from_u32(heap[ptr + k].sm() as u32) {
                        s.push(ch);
                    }
                }
                out.push(ShowVal::Text(s));
                pc += 1;
            }
            218 => {
                // TextCharAt { dst, coll=ra, index=rb } — a single-char Text
                // The VM's `item i of text` returns a ONE-character string; the chip builds a fresh
                // `[1, codepoint]` char-list so a subsequent `char == " "` routes to the char-list compare.
                let cp = heap[regs[ra].idx() + regs[rb].idx()].clone(); // the 1-based codepoint
                let ptr = heap.len();
                heap.push(RegVal::Small(1));
                heap.push(cp);
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            219 | 220 => {
                // TextEq (219) / TextNe (220) — char-list CONTENT compare
                // Text `equals`/`is not` compares by content (same length + same codepoints), mirroring the
                // VM's `values_equal` for two Texts. 219 yields 1 on equal; 220 yields its negation.
                let p = regs[ra].idx();
                let q = regs[rb].idx();
                let mut equal = heap[p] == heap[q];
                if equal {
                    for j in 1..=(heap[p].sm() as usize) {
                        if heap[p + j] != heap[q + j] {
                            equal = false;
                            break;
                        }
                    }
                }
                regs[dst] = RegVal::Small(if opcode == 219 {
                    i64::from(equal)
                } else {
                    i64::from(!equal)
                });
                pc += 1;
            }
            6 => {
                regs[dst] = regs[ra].clone();
                pc += 1;
            } // Move
            // Branch targets are 16-bit, SPLIT so the low byte sits in the scalar core's byte-1 target
            // position: a scalar program (target ≤255, high byte 0) decodes byte-identically, so the wide
            // format is a strict superset of the deployed scalar ISA and the scalar core is untouched.
            34 => pc = (((instr >> 16) & 0xff) | (((instr >> 8) & 0xff) << 8)) as usize, // Jump {t_lo=byte1, t_hi=byte2}
            35 => {
                if is_zero_val(&regs[ra]) {
                    pc = (((instr >> 16) & 0xff) | ((instr & 0xff) << 8)) as usize
                } else {
                    pc += 1
                }
            } // JumpIfFalse {t_lo=byte1, cond=byte2, t_hi=byte3}
            36 => {
                if !is_zero_val(&regs[ra]) {
                    pc = (((instr >> 16) & 0xff) | ((instr & 0xff) << 8)) as usize
                } else {
                    pc += 1
                }
            } // JumpIfTrue {t_lo=byte1, cond=byte2, t_hi=byte3}
            89 => {
                out.push(match &regs[ra] {
                    // Show (integer datapath) — full value, exact
                    RegVal::Small(v) => ShowVal::Int(*v),
                    RegVal::Big(b) => ShowVal::BigInt(b.clone()),
                });
                pc += 1;
            }
            221 => {
                out.push(ShowVal::Word32(regs[ra].sm() as u32));
                pc += 1;
            } // ShowWord32 — same 32-bit result, rendered UNSIGNED (Word32/intOfWord32 Show)
            43 => break, // Halt
            // FailWith {reasoned=rb} — the unrecoverable trap: the core halts with the error strobe raised;
            // rb=1 says the reason was streamed over the Text port just before. Pre-trap output stands.
            91 => {
                trap = Some(rb != 0);
                break;
            }
            37 => {
                // Call — TWO words: [37][result:8][args_start:8][arg_count:8] then [entry:32]
                // Hardware arg-copy calling convention: the callee runs in a FRESH 256-register frame with
                // the caller's r[args_start..args_start+arg_count] seeded into r0.., matching the VM's
                // windowed convention (a function reads its params at its window base r0..). The caller's
                // whole frame is saved and restored on Return, so argument passing never clobbers a caller
                // register. result=dst byte, args_start=ra byte, arg_count=rb byte; the callee's imem entry
                // index is the full 32-bit SECOND word (so it reaches a program of any length).
                // The arg-copy seeds BOTH banks (int + float) at each arg slot — a float param rides
                // `fregs`, an int param rides `regs`; only the statically-typed one is read in the body.
                let arg_cnt = rb;
                let entry = imem[pc + 1] as usize;
                let mut callee: [RegVal; 256] = std::array::from_fn(|_| RegVal::Small(0));
                let mut callee_f = [0.0f64; 256];
                for i in 0..arg_cnt {
                    callee[i] = regs[ra + i].clone();
                    callee_f[i] = fregs[ra + i];
                }
                frames.push((regs, fregs, pc + 2, dst));
                regs = callee;
                fregs = callee_f;
                pc = entry;
            }
            41 => {
                // Return {src=ra}
                // Capture BOTH banks at the source slot; restore the caller window and place both into
                // its result register. The caller reads only the correctly-typed one (the encoder knows
                // dst's type at the call site), so a type-agnostic Return needs no float-return opcode.
                let rv = regs[ra].clone();
                let rvf = fregs[ra];
                match frames.pop() {
                    Some((saved, saved_f, ret_pc, d)) => {
                        regs = saved;
                        fregs = saved_f;
                        regs[d] = rv;
                        fregs[d] = rvf;
                        pc = ret_pc;
                    }
                    None => break,
                }
            }
            42 => match frames.pop() {
                // ReturnNothing
                Some((saved, saved_f, ret_pc, _)) => {
                    regs = saved;
                    fregs = saved_f;
                    pc = ret_pc;
                }
                None => break,
            },
            40 => {
                // MakeClosure { dst, func=a-byte } — closure = [entry] (non-capturing)
                let entry = ((instr >> 8) & 0xff) as i64;
                let ptr = heap.len();
                heap.push(RegVal::Small(entry));
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            39 => {
                // CallValue { dst=result, callee=ra, b: args_start(hi nibble) | arg_count(lo nibble) }
                // Indirect call through a closure value `[entry]`. The frame is the CALLER's (cloned, then
                // restored on Return), so captures and the closure's static body share one register window —
                // but the ARGS ride the caller's window at `args_start`, while a compiled closure body reads
                // its parameters at its window base r0. So window-copy R[args_start..+arg_count] → R[0..] in
                // the cloned frame before entering (arg_count 0 = same-frame, the hand-built closure form).
                let entry = heap[regs[ra].idx()].idx();
                let args_start = ((instr >> 4) & 0xf) as usize;
                let arg_count = (instr & 0xf) as usize;
                let mut callee = regs.clone();
                let mut callee_f = fregs;
                for k in 0..arg_count {
                    callee[k] = regs[args_start + k].clone();
                    callee_f[k] = fregs[args_start + k]; // seed BOTH banks (a float closure arg rides fregs)
                }
                frames.push((regs, fregs, pc + 1, dst));
                regs = callee;
                fregs = callee_f;
                pc = entry;
            }
            // ── LOAD-STORE unit (structured heap) ──────────────────────────────────────────
            48 | 67 => {
                // NewList / NewTuple { dst, start=ra, count=rb }
                let count = rb; // element count (≤ regfile width); tuple ≡ list layout
                let ptr = heap.len();
                heap.push(RegVal::Small(count as i64));
                for i in 0..count {
                    heap.push(regs[(ra + i) & 0xff].clone());
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            49 | 50 | 90 | 82 => {
                let ptr = heap.len();
                heap.push(RegVal::Small(0));
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            } // NewEmptyList / NewEmptyListI32 / Args (offline argv) / NewCrdt (seq/set = empty collection; single-replica projection)
            54 | 83 => {
                // ListPush { list=dst, value=ra } / CrdtAppend (seq CRDT = ordered list, single-replica projection)
                let ptr = regs[dst].idx();
                let len = heap[ptr].sm() as usize;
                if ptr + 1 + len == heap.len() {
                    // list is on top of the heap: append in place
                    heap.push(regs[ra].clone());
                    heap[ptr] = RegVal::Small((len + 1) as i64);
                } else {
                    // else copy-on-write to a fresh block (value semantics)
                    let newptr = heap.len();
                    heap.push(RegVal::Small((len + 1) as i64));
                    for k in 1..=len {
                        let v = heap[ptr + k].clone();
                        heap.push(v);
                    }
                    heap.push(regs[ra].clone());
                    regs[dst] = RegVal::Small(newptr as i64);
                }
                pc += 1;
            }
            59 | 60 => {
                // Index / IndexUnchecked { dst, coll=ra, index/key=rb }
                let ptr = regs[ra].idx();
                if heap[ptr].sm() & MAP_TAG != 0 {
                    // MAP: look up value by char-list key
                    let np = (heap[ptr].sm() & !MAP_TAG) as usize;
                    let key = regs[rb].idx();
                    let mut val = RegVal::Small(0);
                    for i in 0..np {
                        if key_eq(&heap, key, heap[ptr + 1 + 2 * i].idx()) {
                            val = heap[ptr + 2 + 2 * i].clone();
                            break;
                        }
                    }
                    regs[dst] = val;
                } else {
                    // LIST: positional (1-based)
                    regs[dst] = heap[ptr + regs[rb].idx()].clone();
                }
                pc += 1;
            }
            223 => {
                // MapIndexInt { dst, map=ra, key=rb } — lookup by INTEGER key
                // `item k of m` for a `Map of Int to …`: scan the key cells with a direct integer `==` (the key
                // cell holds the int key itself, NOT a heap pointer — the encoder emits this op only when the key
                // is statically non-Text). A missing key yields 0 here — the fabric engine's hardware-natural
                // default. NOTE: this DIVERGES from the scalar tiers, where `item k of m` on an absent key is a
                // runtime TRAP (interpreter/VM/C/riscv); the whole-program corpora reach this op only through
                // `contains`-guarded reads, so the gap is latent. Aligning the fabric to trap is a fabric call.
                let ptr = regs[ra].idx();
                let np = (heap[ptr].sm() & !MAP_TAG) as usize;
                let key = regs[rb].clone();
                let mut val = RegVal::Small(0);
                for i in 0..np {
                    if heap[ptr + 1 + 2 * i] == key {
                        val = heap[ptr + 2 + 2 * i].clone();
                        break;
                    }
                }
                regs[dst] = val;
                pc += 1;
            }
            62 => {
                // Length { dst, coll=ra }
                let ptr = regs[ra].idx();
                let h = heap[ptr].sm();
                regs[dst] = RegVal::Small(if h & MAP_TAG != 0 { h & !MAP_TAG } else { h });
                pc += 1;
            }
            52 => {
                let ptr = heap.len();
                heap.push(RegVal::Small(MAP_TAG));
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            } // NewEmptyMap — tagged, 0 pairs
            64 => {
                // FormatValue { dst, src=ra } — the decimal string of regs[ra]
                // The full BigInt decimal (exact for a `Big` value too) — byte-identical to
                // `logicaffeine_base::BigInt`'s `Display`, the same string the VM renders in interpolation.
                let s = match &regs[ra] {
                    RegVal::Small(v) => v.to_string(),
                    RegVal::Big(b) => b.to_string(),
                };
                let ptr = heap.len();
                heap.push(RegVal::Small(s.chars().count() as i64)); // Text ≡ char-code list [len, chars…]
                for ch in s.chars() {
                    heap.push(RegVal::Small(i64::from(ch as u32)));
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            225 => {
                // FormatValueFloat { dst, src=ra, prec=rb } — the fixed-precision decimal string of fregs[ra]
                // A float `Show "{f:.N}"` renders `fregs[ra]` to `rb` decimal places EXACTLY as the VM does
                // (`crate::semantics::format::apply_format_spec` on a `.N` spec = `format!("{:.prec$}", f, prec)`),
                // then lays the UTF-8 bytes out as a Text char-code list `[len, chars…]` on the heap — the same
                // `[len, codepoints…]` shape `TextConst`/op 64/Concat use, so a downstream `ShowText` (op 217)
                // decodes it byte-identically. The digits/sign/rounding come from Rust's `f64` formatter (the
                // single float-display authority the VM shares), so the model string == the VM's `Show` line.
                let s = format!("{:.prec$}", fregs[ra], prec = rb);
                let ptr = heap.len();
                heap.push(RegVal::Small(s.chars().count() as i64)); // Text ≡ char-code list [len, chars…]
                for ch in s.chars() {
                    heap.push(RegVal::Small(i64::from(ch as u32)));
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            63 => {
                // Contains { dst, coll=ra, value=rb }
                let ptr = regs[ra].idx();
                let target = regs[rb].clone();
                let mut found = 0i64;
                if heap[ptr].sm() & MAP_TAG != 0 {
                    // MAP: membership tests the KEYS (int keys)
                    let np = (heap[ptr].sm() & !MAP_TAG) as usize;
                    for i in 0..np {
                        if heap[ptr + 1 + 2 * i] == target {
                            found = 1;
                            break;
                        }
                    }
                } else {
                    // LIST/SET: membership tests the elements
                    let len = heap[ptr].sm() as usize;
                    for k in 1..=len {
                        if heap[ptr + k] == target {
                            found = 1;
                            break;
                        }
                    }
                }
                regs[dst] = RegVal::Small(found);
                pc += 1;
            }
            70 => {
                // ListPop { dst, list=ra }
                let ptr = regs[ra].idx();
                let len = heap[ptr].sm() as usize;
                regs[dst] = heap[ptr + len].clone(); // last element
                heap[ptr] = RegVal::Small((len - 1) as i64);
                pc += 1;
            }
            74 => {
                // NewStruct { dst, num_fields=rb } — record: [n, f0..f_{n-1}]
                let n = rb;
                let ptr = heap.len();
                heap.push(RegVal::Small(n as i64));
                for _ in 0..n {
                    heap.push(RegVal::Small(0));
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            75 | 84 => {
                // StructInsert { obj=dst, value=ra, field=rb } / CrdtResolve (register last-write = field overwrite, single-replica)
                let ptr = regs[dst].idx();
                heap[ptr + 1 + rb] = regs[ra].clone();
                pc += 1;
            }
            76 => {
                // GetField { dst, obj=ra, field=rb }
                let ptr = regs[ra].idx();
                regs[dst] = heap[ptr + 1 + rb].clone();
                pc += 1;
            }
            57 | 58 => {
                // SetIndex/SetIndexUnchecked { coll=dst, index/key=ra, value=rb }
                let ptr = regs[dst].idx();
                if heap[ptr].sm() & MAP_TAG != 0 {
                    // MAP: update the key's value, or append a new pair
                    let np = (heap[ptr].sm() & !MAP_TAG) as usize;
                    let key = regs[ra].idx();
                    let mut found = None;
                    for i in 0..np {
                        if key_eq(&heap, key, heap[ptr + 1 + 2 * i].idx()) {
                            found = Some(i);
                            break;
                        }
                    }
                    match found {
                        Some(i) => heap[ptr + 2 + 2 * i] = regs[rb].clone(),
                        None => {
                            // append a new `(key_ptr, value)` pair
                            // A map is a CONTIGUOUS block `[header, k0,v0, …]`; appending in place is only sound
                            // when the block is on top of the heap. The string-key path INTERLEAVES a fresh
                            // `TextConst` key char-list before each `Set item "k" of m`, so the map is usually
                            // NOT on top — a bare push would scatter the pair past unrelated cells and the
                            // `ptr+1+2i` addressing would read garbage (the read returns 0). COPY-ON-WRITE the
                            // whole map to a fresh contiguous block (header + all `2*np` pair cells) and repoint
                            // `m`, exactly as ListPush COWs a non-top list; when the map IS on top this still
                            // appends contiguously (the copy is a no-op relocation).
                            if ptr + 1 + 2 * np == heap.len() {
                                // on top: grow in place
                                heap.push(regs[ra].clone());
                                heap.push(regs[rb].clone());
                                heap[ptr] = RegVal::Small(MAP_TAG | ((np + 1) as i64));
                            } else {
                                // else COW to a fresh contiguous block
                                let newptr = heap.len();
                                heap.push(RegVal::Small(MAP_TAG | ((np + 1) as i64))); // header (with the +1 pair)
                                for k in 1..=(2 * np) {
                                    let v = heap[ptr + k].clone();
                                    heap.push(v);
                                }
                                heap.push(regs[ra].clone());
                                heap.push(regs[rb].clone());
                                regs[dst] = RegVal::Small(newptr as i64);
                            }
                        }
                    }
                } else {
                    // LIST: positional (1-based)
                    heap[ptr + regs[ra].idx()] = regs[rb].clone();
                }
                pc += 1;
            }
            224 => {
                // MapSetIndexInt { map=dst, key=ra, value=rb } — set by INTEGER key
                // `Set item k of m to v` for a `Map of Int to …`: find the key cell by a direct integer `==` and
                // overwrite its value, else append the pair `[key, value]` (the map is on top of the heap so the
                // append grows it in place) and bump the header's pair count. Mirrors the string-key SetIndex, but
                // the key cell holds the int key itself. The encoder emits this only for a statically non-Text key.
                let ptr = regs[dst].idx();
                let np = (heap[ptr].sm() & !MAP_TAG) as usize;
                let key = regs[ra].clone();
                let mut found = None;
                for i in 0..np {
                    if heap[ptr + 1 + 2 * i] == key {
                        found = Some(i);
                        break;
                    }
                }
                match found {
                    Some(i) => heap[ptr + 2 + 2 * i] = regs[rb].clone(),
                    None => {
                        // Same CONTIGUITY rule as the string-key SetIndex: append in place only when the map is
                        // on top of the heap, else COW the whole block. An int key rarely allocates (so the map
                        // usually stays on top), but a `Map of Int to <heap value>` can interleave allocations —
                        // the COW keeps the pair block contiguous so `ptr+1+2i` addressing stays sound.
                        if ptr + 1 + 2 * np == heap.len() {
                            // on top: grow in place
                            heap.push(key);
                            heap.push(regs[rb].clone());
                            heap[ptr] = RegVal::Small(MAP_TAG | ((np + 1) as i64));
                        } else {
                            // else COW to a fresh contiguous block
                            let newptr = heap.len();
                            heap.push(RegVal::Small(MAP_TAG | ((np + 1) as i64)));
                            for k in 1..=(2 * np) {
                                let v = heap[ptr + k].clone();
                                heap.push(v);
                            }
                            heap.push(key);
                            heap.push(regs[rb].clone());
                            regs[dst] = RegVal::Small(newptr as i64);
                        }
                    }
                }
                pc += 1;
            }
            33 | 32 => {
                // SeqConcat / Concat (Text ≡ char-code list) — fresh lhs ++ rhs
                let lp = regs[ra].idx();
                let rp = regs[rb].idx();
                let ll = heap[lp].sm() as usize;
                let rl = heap[rp].sm() as usize;
                let ptr = heap.len();
                heap.push(RegVal::Small((ll + rl) as i64));
                for k in 1..=ll {
                    let v = heap[lp + k].clone();
                    heap.push(v);
                }
                for k in 1..=rl {
                    let v = heap[rp + k].clone();
                    heap.push(v);
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            66 => {
                // DeepClone { dst, src=ra } — independent copy
                // A MAP's header carries MAP_TAG in its high bits, so its cell COUNT is `1 + 2*npairs` (key/value
                // pairs), not `heap[sp]` — cloning it as a plain list would read the tagged header as a giant
                // length. Copy the header VERBATIM (preserving the tag) and the `2*npairs` key/value cells; a
                // plain list/set/struct clones `heap[sp]` cells as before. The isolation pass emits this to keep
                // value semantics when a map is mutated in a loop (`Set item k of m`), so a map clone must survive.
                // A `Seq of Float`'s element PAYLOADS ride the parallel `fheap` plane at the same slots, so a deep
                // copy must carry BOTH planes (mirroring FListPush's two-plane COW) — copying only `heap` clones
                // the structure but leaves the cloned block's float payloads 0.0, silently zeroing a float seq
                // passed into (and mutated inside) a function. `fheap` stays length-aligned with `heap` (the
                // bottom-of-loop resize), so we push a matching float cell for every int cell.
                let sp = regs[ra].idx();
                let hdr = heap[sp].sm();
                let cells = if hdr & MAP_TAG != 0 {
                    2 * (hdr & !MAP_TAG) as usize
                } else {
                    hdr as usize
                };
                let ptr = heap.len();
                heap.push(RegVal::Small(hdr));
                fheap.push(fheap[sp]);
                for k in 1..=cells {
                    let v = heap[sp + k].clone();
                    let f = fheap[sp + k];
                    heap.push(v);
                    fheap.push(f);
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            85 => {
                iters.push((regs[ra].idx(), 1));
                pc += 1;
            } // IterPrepare { iterable=ra }
            86 => {
                // IterNext { dst, exit:16 } — exit_lo=byte2, exit_hi=byte3
                let exit = (((instr >> 8) & 0xff) | ((instr & 0xff) << 8)) as usize;
                let (ptr, cursor) = *iters.last().unwrap();
                let len = heap[ptr].sm() as usize;
                if cursor > len {
                    pc = exit; // exhausted → the loop's exit (IterPop)
                } else {
                    regs[dst] = heap[ptr + cursor].clone();
                    iters.last_mut().unwrap().1 = cursor + 1;
                    pc += 1;
                }
            }
            231 => {
                // FIterNext { dst, exit:16 } — float-seq iteration: read the f64 PAYLOAD off `fheap` into `fregs[dst]`
                // A `Repeat for v in xs` over a `Seq of Float` iterates the SAME `[length, …]` int structure,
                // but each element's value rides the parallel `fheap` plane. This mirrors op 86 (IterNext) but
                // reads `fheap[ptr+cursor]` into the float bank, so the loop variable `v` is a float scalar and
                // a downstream `s + v` lowers to the FPU — byte-identical to the VM's f64 iteration.
                let exit = (((instr >> 8) & 0xff) | ((instr & 0xff) << 8)) as usize;
                let (ptr, cursor) = *iters.last().unwrap();
                let len = heap[ptr].sm() as usize;
                if cursor > len {
                    pc = exit;
                } else {
                    fregs[dst] = fheap[ptr + cursor];
                    iters.last_mut().unwrap().1 = cursor + 1;
                    pc += 1;
                }
            }
            87 => {
                iters.pop();
                pc += 1;
            } // IterPop
            45 => {
                let idx = ((instr >> 8) & 0xff) as usize;
                regs[dst] = globals[idx].clone();
                pc += 1;
            } // GlobalGet { dst, idx=a-byte }
            46 => {
                globals[dst] = regs[ra].clone();
                pc += 1;
            } // GlobalSet { idx=dst-field, src=ra }
            // ── CONCURRENCY coprocessor: FIFO channels (the deterministic NoC) ─────────────
            // A channel is a fixed 8-slot ring buffer on the heap: `[head, count, s0..s7]`. Send enqueues
            // at `(head+count) mod 8`, Recv dequeues at `head` — FIFO order, exactly the VM's deterministic
            // pipe semantics for a single task (buffered send-then-receive).
            92 => {
                // ChanNew { dst, cap ignored }
                let ptr = heap.len();
                heap.push(RegVal::Small(0));
                heap.push(RegVal::Small(0)); // head, count
                for _ in 0..8 {
                    heap.push(RegVal::Small(0));
                } // 8 ring slots
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            93 => {
                // ChanSend { chan=dst, val=ra } — enqueue
                let ptr = regs[dst].idx();
                let head = heap[ptr].sm() as usize;
                let count = heap[ptr + 1].sm() as usize;
                let slot = (head + count) % 8;
                heap[ptr + 2 + slot] = regs[ra].clone();
                heap[ptr + 1] = RegVal::Small((count + 1) as i64);
                pc += 1;
            }
            94 => {
                // ChanRecv { dst, chan=ra } — dequeue (FIFO)
                let ptr = regs[ra].idx();
                let head = heap[ptr].sm() as usize;
                let count = heap[ptr + 1].sm() as usize;
                regs[dst] = heap[ptr + 2 + head].clone();
                heap[ptr] = RegVal::Small(((head + 1) % 8) as i64);
                heap[ptr + 1] = RegVal::Small((count - 1) as i64);
                pc += 1;
            }
            95 => {
                // ChanTrySend { dst=success, chan=ra, val=rb } — non-blocking
                let ptr = regs[ra].idx();
                let head = heap[ptr].sm() as usize;
                let count = heap[ptr + 1].sm() as usize;
                if count < 8 {
                    heap[ptr + 2 + (head + count) % 8] = regs[rb].clone();
                    heap[ptr + 1] = RegVal::Small((count + 1) as i64);
                    regs[dst] = RegVal::Small(1); // sent
                } else {
                    regs[dst] = RegVal::Small(0); // channel full
                }
                pc += 1;
            }
            96 => {
                // ChanTryRecv { dst=value, chan=ra } — non-blocking
                let ptr = regs[ra].idx();
                let head = heap[ptr].sm() as usize;
                let count = heap[ptr + 1].sm() as usize;
                if count > 0 {
                    regs[dst] = heap[ptr + 2 + head].clone();
                    heap[ptr] = RegVal::Small(((head + 1) % 8) as i64);
                    heap[ptr + 1] = RegVal::Small((count - 1) as i64);
                } else {
                    regs[dst] = RegVal::Small(0); // nothing
                }
                pc += 1;
            }
            88 => {
                pc += 1;
            } // Sleep { duration } — deterministic scheduler no-op
            72 | 73 => {
                regs[dst] = RegVal::Small(0);
                pc += 1;
            } // LoadToday / LoadNow — placeholder epoch (a real chip reads an RTC)
            // Value-transparent metadata ops: EnsureOwned (ownership), RegionBoundsGuard (hoist marker),
            // Obs (observability emit) — the interpreter treats them as no-ops for VALUES, so the chip does too.
            // Networking (105 NetConnect / 106 NetListen / 107 NetSend / 108 NetStream / 111 NetSync):
            // on the single-node deterministic chip there is no peer or wire, so — exactly like the VM's
            // documented offline model (tw==VM-net==AOT) — they are no-ops. NetSync leaves its synced
            // variable unchanged (no peer to merge). The real NIC/relay is an off-chip transport.
            7 | 61 | 112 | 105 | 106 | 108 | 111 => {
                pc += 1;
            } // EnsureOwned / RegionBoundsGuard / Obs / offline Net no-ops (Connect/Listen/Stream/Sync)
            110 => {
                regs[dst] = RegVal::Small(peer_seq);
                peer_seq += 1;
                pc += 1;
            } // NetMakePeer { dst, addr } — a fresh offline peer key (identity for the loopback inbox)
            107 => {
                inbox.push((regs[ra].clone(), regs[rb].clone()));
                pc += 1;
            } // NetSend { to=ra, msg=rb } — PUBLISH (peer key, message) to the loopback inbox
            109 => {
                // NetAwait { dst, from=ra } — DRAIN the inbox entry keyed by the peer, delivering its message
                let want = regs[ra].clone();
                if let Some(i) = inbox.iter().position(|(k, _)| *k == want) {
                    regs[dst] = inbox.remove(i).1;
                }
                pc += 1;
            }
            98 => {
                // Spawn { entry=target, args_start=ra, arg_count=rb } — deterministic sync launch (fire-and-forget; pairs with ReturnNothing)
                // Launch runs the task SYNCHRONOUSLY (deterministic offline semantics), so a spawn is a
                // fire-and-forget Call: the launched body runs in a FRESH frame with R[args_start..+arg_count]
                // window-copied into r0.. (exactly the Call convention), its channel side-effects persist on
                // the shared heap, and its `ReturnNothing` pops back to Main. This matches the VM's windowed
                // task-arg passing (the arg — a channel handle — reaches the body's parameter at its base r0).
                let arg_cnt = rb;
                let mut callee: [RegVal; 256] = std::array::from_fn(|_| RegVal::Small(0));
                let mut callee_f = [0.0f64; 256];
                for k in 0..arg_cnt {
                    callee[k] = regs[ra + k].clone();
                    callee_f[k] = fregs[ra + k];
                }
                frames.push((regs, fregs, pc + 1, 0));
                regs = callee;
                fregs = callee_f;
                pc = target;
            }
            99 => {
                // SpawnHandle { handle=dst, entry=a-byte } — sync launch + bind a handle
                let entry = ((instr >> 8) & 0xff) as usize;
                regs[dst] = RegVal::Small(0); // the task handle (placeholder; the frame preserves it across the sync run)
                frames.push((regs.clone(), fregs, pc + 1, 0));
                pc = entry;
            }
            101 => {
                pc += 1;
            } // TaskAbort { handle } — no-op (the deterministic task already ran to completion)
            100 => {
                pc += 1;
            } // TaskAwait { dst, handle } — the instant join: the sync child already completed at spawn, so control passes through (the VM delivers `Nothing` as the result — task returns aren't propagated — so the result reg is not a meaningful observable)
            97 => {
                pc += 1;
            } // ChanClose { chan } — sets a closed flag no receive path reads; output-transparent
            44 => {
                // CheckPolicy { subject=ra, cap field=rb } — capability engine (boolean-capability class)
                // The policy grants iff the subject's capability field is truthy; else the check TRAPS
                // (`Security Check Failed`) — enforcing denial, never a no-op. The policy compiler resolves
                // the predicate to the capability field index (rb).
                if heap[regs[ra].idx() + 1 + rb].sm() != 0 {
                    pc += 1;
                } else {
                    trap = Some(false);
                    break;
                }
            }
            38 => {
                // CallBuiltin { dst, args_start=ra, builtin=rb } — kernel-builtin coprocessor
                // The builtin ORDINAL rides the b-byte (a chip-internal numbering, NOT the BuiltinId
                // discriminant — the encoder's `builtin_chip_ordinal` is the single source of truth,
                // mirrored here). Two-arg ops read the second operand from regs[args_start+1]. The WORD
                // coprocessor ordinals (30/31/32) compute over the register's 32-bit pattern AS UNSIGNED:
                // a rotate is `rotate_left/right`, a logical shift zero-fills the vacated high bits
                // (distinct from the ALU's ARITHMETIC `Shr`). Bit-exact to `logicaffeine_base::Word32`.
                let arg1 = regs[(ra + 1) & 0xff].clone();
                let src = regs[ra].clone();
                // FLOAT-plane ordinals (36..=41) read `fregs` — the operand was proven float by the type
                // oracle. `abs`/`min`/`max` on floats write `fregs[dst]` (a float scalar); `floor`/`ceil`/
                // `round` write `regs[dst]` (an INT result: `f.floor() as i64`, round-half-away). All are
                // byte-identical to `call_builtin`'s float arms. They `continue` past the integer store below.
                if (36..=41).contains(&rb) {
                    let a = fregs[ra];
                    let b = fregs[(ra + 1) & 0xff];
                    match rb {
                        36 => fregs[dst] = a.abs(),  // FAbs  (VM: `f.abs()`)
                        37 => fregs[dst] = a.min(b), // FMin  (VM: `x.min(y)`)
                        38 => fregs[dst] = a.max(b), // FMax  (VM: `x.max(y)`)
                        39 => regs[dst] = RegVal::Small(a.floor() as i64), // Floor → Int
                        40 => regs[dst] = RegVal::Small(a.ceil() as i64), // Ceil  → Int
                        _ => regs[dst] = RegVal::Small(a.round() as i64), // Round → Int (round-half-away)
                    }
                    pc += 1;
                    fheap.resize(heap.len(), 0.0);
                    continue;
                }
                regs[dst] = match rb {
                    // Abs promotes on `|i64::MIN|` = 2^63 (exact via BigInt), and takes a `Big` value's exact
                    // abs — mirroring the VM's `BuiltinId::Abs`.
                    5 => match &src {
                        RegVal::Small(n) => match n.checked_abs() {
                            Some(a) => RegVal::Small(a),
                            None => RegVal::from_big(src.to_big().abs()),
                        },
                        RegVal::Big(b) => RegVal::from_big(b.abs()),
                    },
                    7 => {
                        if regval_cmp(&src, &arg1) != std::cmp::Ordering::Greater {
                            src
                        } else {
                            arg1
                        }
                    } // Min
                    8 => {
                        if regval_cmp(&src, &arg1) != std::cmp::Ordering::Less {
                            src
                        } else {
                            arg1
                        }
                    } // Max
                    // `count_ones(n)` — set bits over the full 64-bit two's-complement pattern, EXACTLY the
                    // VM's `(n as u64).count_ones()`. A `Big` operand narrows to its low i64 first (a chip
                    // register is 64-bit; `count_ones` is defined only over `Int` in `call_builtin`).
                    9 => RegVal::Small(i64::from((src.sm() as u64).count_ones())),
                    // The WORD32 ordinals operate on the register's low 32-bit pattern (the `Word32` type is
                    // ALWAYS 32-bit, independent of the datapath width) and sign-extend the 32-bit result.
                    30 => RegVal::Small(i64::from(
                        (src.sm() as u32).rotate_left(arg1.sm() as u32) as i32
                    )), // BuiltinId::Rotl
                    31 => RegVal::Small(i64::from(
                        (src.sm() as u32).rotate_right(arg1.sm() as u32) as i32,
                    )), // BuiltinId::Rotr
                    32 => RegVal::Small(i64::from(
                        (src.sm() as u32).wrapping_shr(arg1.sm() as u32) as i32,
                    )), // BuiltinId::Word32Shr (LOGICAL)
                    _ => RegVal::Small(0),
                };
                pc += 1;
            }
            102 => {
                sel_arms.push((dst, ra));
                pc += 1;
            } // SelectArmRecv { chan=dst, var=ra } — register a recv arm
            103 => {
                sel_arms.push((usize::MAX, ra));
                pc += 1;
            } // SelectArmTimeout { ticks=ra } — register a timeout arm
            104 => {
                // SelectWait { dst_arm } — fire first-ready recv, else timeout
                let mut fired = usize::MAX;
                for (i, &(cr, vr)) in sel_arms.iter().enumerate() {
                    if cr != usize::MAX {
                        // a recv arm
                        let ch = regs[cr].idx();
                        if heap[ch + 1].sm() > 0 {
                            // channel has a buffered value
                            let head = heap[ch].sm() as usize;
                            regs[vr] = heap[ch + 2 + head].clone();
                            heap[ch] = RegVal::Small(((head + 1) % 8) as i64);
                            heap[ch + 1] = RegVal::Small(heap[ch + 1].sm() - 1);
                            fired = i;
                            break;
                        }
                    }
                }
                if fired == usize::MAX {
                    // nothing ready → the timeout arm, if any
                    for (i, &(cr, _)) in sel_arms.iter().enumerate() {
                        if cr == usize::MAX {
                            fired = i;
                            break;
                        }
                    }
                }
                regs[dst] = RegVal::Small(fired as i64);
                sel_arms.clear();
                pc += 1;
            }
            // ── CRDT coprocessor: convergent counter ──────────────────────────────────────
            // A `Counter` is a plain struct (`[nfields, f0..]`); `Increase c's n by k` accumulates the
            // field (`crdt_counter_bump` = field ± amount) and `Merge b into a` joins field-wise —
            // `crdt_merge_field(Int,Int)` is wrapping-ADD, so counters converge by summing. Counter
            // arithmetic is INTENTIONALLY wrapping (modular, over the cyclic group — see the VM's
            // `crdt_counter_bump`/`crdt_merge_field`), so these fields stay `Small` and wrap, NOT promote.
            80 => {
                // CrdtBump { obj=dst, amount=ra, field=b[3:0], negate=b[4] }
                let fb = (instr & 0xff) as usize;
                let field = fb & 0xf;
                let neg = (fb >> 4) & 1;
                let amt = regs[ra].sm();
                let ptr = regs[dst].idx();
                let delta = if neg == 1 { -amt } else { amt };
                heap[ptr + 1 + field] =
                    RegVal::Small(heap[ptr + 1 + field].sm().wrapping_add(delta));
                pc += 1;
            }
            81 => {
                // CrdtMerge { target=dst, source=ra } — field-wise sum-join
                let tp = regs[dst].idx();
                let sp = regs[ra].idx();
                let nf = heap[sp].sm() as usize; // source field count
                for f in 0..nf {
                    heap[tp + 1 + f] =
                        RegVal::Small(heap[tp + 1 + f].sm().wrapping_add(heap[sp + 1 + f].sm()));
                }
                pc += 1;
            }
            113 => {
                // CrdtJoin { target=dst, source=ra } — per-replica MAX
                let tp = regs[dst].idx(); // the state-based G-Counter lattice join
                let sp = regs[ra].idx(); // (idempotent), issued to the crdt_merge block
                let nf = heap[sp].sm() as usize; // source lane count (== target's for a G-Counter)
                for f in 0..nf {
                    heap[tp + 1 + f] =
                        RegVal::Small(heap[tp + 1 + f].sm().max(heap[sp + 1 + f].sm()));
                }
                pc += 1;
            }
            114 => {
                // BigIntWide — the 256-bit lane (kind = dst-byte hi nibble)
                use crate::codegen_sva::bignum_rtl::{
                    limb_add, limb_lt, limb_sub, wdiv256, wmul256,
                };
                let kind = ((instr >> 20) & 0xf) as u8; // operation rides the dst byte's high nibble
                match kind {
                    0 => wide[dst & 7] = [(regs[ra].sm() as u32) as u64, 0, 0, 0], // FromInt: W[dst] = zext(R[a])
                    1 => wide[dst & 7] = limb_add(&wide[ra & 7], &wide[rb & 7]).0, // Add   (bigint_alu)
                    2 => wide[dst & 7] = limb_sub(&wide[ra & 7], &wide[rb & 7]).0, // Sub   (bigint_alu)
                    3 => {
                        regs[dst & 0xf] =
                            RegVal::Small(i64::from(limb_lt(&wide[ra & 7], &wide[rb & 7])))
                    } // Cmp: R[dst] = (W[a] < W[b])
                    4 => {
                        regs[dst & 0xf] = RegVal::Small(i64::from((wide[ra & 7][0] as u32) as i32))
                    } // Low: R[dst] = W[a][31:0] (sign-extended)
                    5 => wide[dst & 7] = wdiv256(&wide[ra & 7], &wide[rb & 7]), // Div   (bigint_div)
                    6 => wide[dst & 7] = wmul256(&wide[ra & 7], &wide[rb & 7]), // Mul   (bigint_mul composed)
                    _ => {}
                }
                pc += 1;
            }
            77 => {
                // NewInductive { dst, ctor=a-byte, args_start=b[7:4], count=b[3:0] }
                let ctor = ((instr >> 8) & 0xff) as i64; // the variant tag (a valid lowering of the VM's ctor id)
                let args_start = ((instr >> 4) & 0xf) as usize;
                let count = (instr & 0xf) as usize;
                let ptr = heap.len();
                heap.push(RegVal::Small(ctor)); // record: [tag, payload0, …]
                for i in 0..count {
                    heap.push(regs[(args_start + i) & 0xff].clone());
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            78 => {
                // TestArm { dst, target=ra, variant=b-byte } → tag == variant
                let variant = (instr & 0xff) as i64;
                let ptr = regs[ra].idx();
                regs[dst] = RegVal::Small(if heap[ptr].sm() == variant { 1 } else { 0 });
                pc += 1;
            }
            79 => {
                // BindArm { dst, target=ra, index=b-byte } → payload[index]
                let index = (instr & 0xff) as usize;
                let ptr = regs[ra].idx();
                regs[dst] = heap[ptr + 1 + index].clone();
                pc += 1;
            }
            51 => {
                let ptr = heap.len();
                heap.push(RegVal::Small(0));
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            } // NewEmptySet
            55 => {
                // SetAdd { set=dst, value=ra } — dedup on insert
                let ptr = regs[dst].idx();
                let len = heap[ptr].sm() as usize;
                let v = regs[ra].clone();
                let mut found = false;
                for k in 1..=len {
                    if heap[ptr + k] == v {
                        found = true;
                        break;
                    }
                }
                if !found {
                    if ptr + 1 + len == heap.len() {
                        // on top: append in place
                        heap.push(v);
                        heap[ptr] = RegVal::Small((len + 1) as i64);
                    } else {
                        // else copy-on-write (value semantics)
                        let newptr = heap.len();
                        heap.push(RegVal::Small((len + 1) as i64));
                        for k in 1..=len {
                            let x = heap[ptr + k].clone();
                            heap.push(x);
                        }
                        heap.push(v);
                        regs[dst] = RegVal::Small(newptr as i64);
                    }
                }
                pc += 1;
            }
            53 => {
                // NewRange { dst, start=ra, end=rb } — inclusive [start..=end]
                let start = regs[ra].sm();
                let end = regs[rb].sm();
                let n = if end >= start {
                    (end - start + 1) as usize
                } else {
                    0
                };
                let ptr = heap.len();
                heap.push(RegVal::Small(n as i64));
                for i in 0..n {
                    heap.push(RegVal::Small(start + i as i64));
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            68 => {
                // UnionOp { dst, lhs=ra, rhs=rb } — fresh lhs ∪ rhs (dedup)
                let lp = regs[ra].idx();
                let rp = regs[rb].idx();
                let ll = heap[lp].sm() as usize;
                let rl = heap[rp].sm() as usize;
                let ptr = heap.len();
                heap.push(RegVal::Small(0));
                let mut count = 0usize;
                for k in 1..=ll {
                    let v = heap[lp + k].clone();
                    heap.push(v);
                    count += 1;
                }
                for k in 1..=rl {
                    let v = heap[rp + k].clone();
                    let mut found = false;
                    for j in 0..count {
                        if heap[ptr + 1 + j] == v {
                            found = true;
                            break;
                        }
                    }
                    if !found {
                        heap.push(v);
                        count += 1;
                    }
                }
                heap[ptr] = RegVal::Small(count as i64);
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            69 => {
                // IntersectOp { dst, lhs=ra, rhs=rb } — fresh lhs ∩ rhs
                let lp = regs[ra].idx();
                let rp = regs[rb].idx();
                let ll = heap[lp].sm() as usize;
                let rl = heap[rp].sm() as usize;
                let ptr = heap.len();
                heap.push(RegVal::Small(0));
                let mut count = 0usize;
                for k in 1..=ll {
                    let v = heap[lp + k].clone();
                    let mut in_rhs = false;
                    for j in 1..=rl {
                        if heap[rp + j] == v {
                            in_rhs = true;
                            break;
                        }
                    }
                    if in_rhs {
                        heap.push(v);
                        count += 1;
                    }
                }
                heap[ptr] = RegVal::Small(count as i64);
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            56 => {
                // RemoveFrom { collection=dst, value=ra } — drop value, shift down
                let ptr = regs[dst].idx();
                let len = heap[ptr].sm() as usize;
                let want = regs[ra].clone();
                let mut idx = 0usize;
                for k in 1..=len {
                    if heap[ptr + k] == want {
                        idx = k;
                        break;
                    }
                }
                if idx > 0 {
                    for k in idx..len {
                        let nv = heap[ptr + k + 1].clone();
                        heap[ptr + k] = nv;
                    }
                    heap[ptr] = RegVal::Small((len - 1) as i64);
                }
                pc += 1;
            }
            71 => {
                // DestructureTuple { start=dst, src=ra, count=rb } — spread into regs
                let sp = regs[ra].idx();
                for i in 0..rb {
                    regs[(dst + i) & 0xff] = heap[sp + 1 + i].clone();
                }
                pc += 1;
            }
            65 => {
                // SliceOp { dst, coll=ra, start=b[7:4], end=b[3:0] } — 1-based inclusive
                // `items i through j of xs`: the 1-based-inclusive bounds ride the b-byte nibbles (the encoder
                // constant-folds them; a runtime/large bound has no chip lowering and is rejected loudly). The
                // VM's `collections::slice` is 1-based-inclusive with an EMPTY out-of-range result: `start0 =
                // (start-1)` (0-based inclusive), `end` (1-based inclusive == 0-based exclusive); empty when
                // `start0 ≥ end` or `end > len`. Copy `[cp+1+start0 ..< cp+1+end]`.
                let start = ((instr >> 4) & 0xf) as usize;
                let end = (instr & 0xf) as usize;
                let cp = regs[ra].idx();
                let len = heap[cp].sm() as usize;
                let start0 = start.saturating_sub(1);
                let (lo, hi) = if start0 < end && end <= len {
                    (start0, end)
                } else {
                    (0, 0)
                };
                let ptr = heap.len();
                heap.push(RegVal::Small((hi - lo) as i64));
                for i in lo..hi {
                    let v = heap[cp + 1 + i].clone();
                    heap.push(v);
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            47 => {
                // ListPushField { obj=dst, src=ra, field=rb } — push to obj's list field
                let obj_ptr = regs[dst].idx();
                let list_ptr = heap[obj_ptr + 1 + rb].idx();
                let len = heap[list_ptr].sm() as usize;
                if list_ptr + 1 + len == heap.len() {
                    // list on top: append in place
                    heap.push(regs[ra].clone());
                    heap[list_ptr] = RegVal::Small((len + 1) as i64);
                } else {
                    // else copy-on-write, and repoint the field
                    let newptr = heap.len();
                    heap.push(RegVal::Small((len + 1) as i64));
                    for k in 1..=len {
                        let v = heap[list_ptr + k].clone();
                        heap.push(v);
                    }
                    heap.push(regs[ra].clone());
                    heap[obj_ptr + 1 + rb] = RegVal::Small(newptr as i64);
                }
                pc += 1;
            }
            // Div / ExactDiv / Mod / FloorDiv issue to the shared divider (not the ALU) — the
            // RTL stalls until `done`; the result is identical, so the model retires it directly. The
            // value-integer divider/ALU AUTO-PROMOTE to BigInt on overflow, EXACTLY like the VM (`arith`),
            // so the chip differs nowhere on integers — the `width` no longer masks value arithmetic.
            12 | 13 | 14 | 15 => {
                let r = soft_core_divider_regval(opcode, &regs[ra], &regs[rb]);
                regs[dst] = r;
                pc += 1;
            }
            // Pow (op 18): `base ** exp` for a non-negative integer exponent — the sequential-multiply power
            // unit (a stall-based FSM in the RTL, mirroring the shared divider), retired directly by the model.
            // The accumulator starts at 1 and multiplies by `base` `exp` times through the SAME auto-promoting
            // value-integer multiply the ALU uses (`soft_core_alu_regval(11, …)`), so it is BigInt-exact —
            // byte-identical to the VM's `int_power`/`BigInt::pow` (`base^0 == 1`; overflow promotes, never
            // wraps). A negative exponent errors in the VM (an Int can't hold a fraction), so no differential
            // program raises to one; the unsigned loop count reflects that the chip's power unit takes exp ≥ 0.
            18 => {
                let base = regs[ra].clone();
                let exp = regs[rb].sm();
                let mut acc = RegVal::Small(1);
                let mut k = if exp < 0 { 0 } else { exp };
                while k > 0 {
                    acc = soft_core_alu_regval(11, &acc, &base);
                    k -= 1;
                }
                regs[dst] = acc;
                pc += 1;
            }
            // ── WORD32 (ℤ/2³²) ARITHMETIC (chip-synthetic 226/227/228) — a Word32 `+`/`-`/`*` wraps in the
            // ring, NOT the unbounded value-integer ALU. The register holds the 32-bit pattern; compute on
            // the low 32 bits, wrap (`u32::wrapping_op`), and stay `Small` — a Word32 is NEVER a BigInt.
            // Bit-exact to the VM's `Word32::add`/`sub`/`mul` (`u32::wrapping_*`). Sign-extended back to i64
            // (`as i32 as i64`) so a Word32 whose high bit is set reads as a negative i64 pattern, exactly
            // as the 32-bit datapath carried it; `ShowWord32` re-reads the low 32 bits UNSIGNED.
            226 => {
                regs[dst] = RegVal::Small(i64::from(
                    (regs[ra].sm() as u32).wrapping_add(regs[rb].sm() as u32) as i32,
                ));
                pc += 1;
            }
            227 => {
                regs[dst] = RegVal::Small(i64::from(
                    (regs[ra].sm() as u32).wrapping_sub(regs[rb].sm() as u32) as i32,
                ));
                pc += 1;
            }
            228 => {
                regs[dst] = RegVal::Small(i64::from(
                    (regs[ra].sm() as u32).wrapping_mul(regs[rb].sm() as u32) as i32,
                ));
                pc += 1;
            }
            233 => {
                // Chr { dst, code=ra } — a 1-char Text from a codepoint
                // `chr(code)` builds a ONE-character Text — the char-code list `[1, codepoint]` on the heap,
                // the SAME shape TextConst/op 64/Concat produce, so a downstream `ShowText` decodes it
                // byte-identically to the VM's `RuntimeValue::Text(c.to_string())`. An INVALID codepoint (a
                // surrogate 0xD800..=0xDFFF or a value > 0x10FFFF) has no character — the VM's `call_builtin`
                // returns `Err` there, aborting the run with the output produced so far standing, so the chip
                // TRAPS (halts, pre-trap output stands), byte-identical to the VM's error-path output.
                let code = regs[ra].sm();
                match u32::try_from(code).ok().and_then(char::from_u32) {
                    Some(_) => {
                        let ptr = heap.len();
                        heap.push(RegVal::Small(1)); // header: a 1-char string
                        heap.push(RegVal::Small(code)); // the codepoint (a valid `char`)
                        regs[dst] = RegVal::Small(ptr as i64);
                        pc += 1;
                    }
                    None => {
                        trap = Some(false);
                        break;
                    } // invalid codepoint → trap (VM returns Err)
                }
            }
            234 => {
                // TextContains { dst, hay=ra, needle=rb } — substring scan
                // `contains(haystack, needle)` over two Text char-lists: `regs[dst] = 1` iff `needle` occurs
                // as a contiguous run of codepoints in `haystack`, else 0 — byte-identical to the VM's
                // `h.contains(n.as_str())` (Rust `str::contains` on the reconstructed strings). The empty
                // needle is contained in EVERYTHING (`"".contains` is always true), matching Rust. The scan
                // compares CODEPOINTS (the char-list cells), which is exactly Rust's UTF-8 substring test
                // since a codepoint match is a byte match for the canonical UTF-8 both sides carry.
                let hp = regs[ra].idx();
                let np = regs[rb].idx();
                let hl = heap[hp].sm() as usize; // haystack char count
                let nl = heap[np].sm() as usize; // needle char count
                let mut found = 0i64;
                if nl == 0 {
                    found = 1; // the empty needle is always contained
                } else if nl <= hl {
                    for start in 0..=(hl - nl) {
                        // every window of `nl` chars in the haystack
                        let mut all = true;
                        for k in 0..nl {
                            if heap[hp + 1 + start + k].sm() != heap[np + 1 + k].sm() {
                                all = false;
                                break;
                            }
                        }
                        if all {
                            found = 1;
                            break;
                        }
                    }
                }
                regs[dst] = RegVal::Small(found);
                pc += 1;
            }
            235 => {
                // RepeatSeq { dst, elem=ra, count=rb } — n-slot scalar fill
                // `repeatSeq(x, n)` builds a fresh `n`-element Seq, every slot the SAME scalar `regs[ra]`
                // (`n ≤ 0` is empty), byte-identical to the VM's `(0..n).map(|_| element.deep_clone())` for a
                // SCALAR element (an Int/Word32 register value deep-clones to itself — the encoder only emits
                // this op for a provably-scalar element, rejecting a float/reference element inside, since a
                // heap-reference element would need per-slot deep copies the flat fill can't express). The
                // count is a runtime register value; the chip's fill unit is a sequential state machine (like
                // the divider), so the model loops here.
                let n = regs[rb].sm().max(0) as usize;
                let elem = regs[ra].clone();
                let ptr = heap.len();
                heap.push(RegVal::Small(n as i64)); // header
                for _ in 0..n {
                    heap.push(elem.clone());
                }
                regs[dst] = RegVal::Small(ptr as i64);
                pc += 1;
            }
            236 => {
                // ParseInt { dst, text=ra } — decimal char-scan → i64
                // `parseInt(text)` reads the Text char-list at `regs[ra]` (`[len, cp0, cp1, …]`) and computes
                // the i64 value EXACTLY as the VM's `s.trim().parse::<i64>()`: trim Unicode whitespace off both
                // ends, accept an optional single leading `+`/`-`, then require ≥1 ASCII digit, accumulating in
                // the NEGATIVE magnitude (`acc = acc*10 − digit`) so `-9223372036854775808` (= i64::MIN) parses
                // where its positive twin overflows — the exact boundary Rust's parser draws. A non-numeric,
                // empty, or OVERFLOWING input has no i64 value: the VM's `parse` returns `Err`, whose `?` aborts
                // the run with the output so far standing, so the chip TRAPS (`break`, pre-trap output stands) —
                // byte-identical. The VM does NOT promote an overflow to BigInt; it errors, so neither do we.
                let ptr = regs[ra].idx();
                let len = heap[ptr].sm() as usize;
                // Reconstruct the codepoint slice, then trim by Unicode whitespace (Rust's `str::trim`).
                let cps: Vec<u32> = (0..len).map(|k| heap[ptr + 1 + k].sm() as u32).collect();
                let is_ws = |cp: u32| char::from_u32(cp).map_or(false, |c| c.is_whitespace());
                let mut lo = 0usize;
                let mut hi = cps.len();
                while lo < hi && is_ws(cps[lo]) {
                    lo += 1;
                }
                while hi > lo && is_ws(cps[hi - 1]) {
                    hi -= 1;
                }
                let body = &cps[lo..hi];
                // Optional single leading sign, then the digit run (matching `i64::from_str_radix(_, 10)`).
                let (neg, digits) = match body.first().copied() {
                    Some(c) if c == '-' as u32 => (true, &body[1..]),
                    Some(c) if c == '+' as u32 => (false, &body[1..]),
                    _ => (false, body),
                };
                // Accumulate as the NEGATIVE magnitude; any non-digit char or an i64 overflow → trap.
                let mut acc: Option<i64> = Some(0);
                let mut saw_digit = false;
                for &cp in digits {
                    if !(('0' as u32)..=('9' as u32)).contains(&cp) {
                        acc = None;
                        break;
                    }
                    saw_digit = true;
                    let d = (cp - '0' as u32) as i64;
                    acc = acc
                        .and_then(|a| a.checked_mul(10))
                        .and_then(|a| a.checked_sub(d));
                    if acc.is_none() {
                        break;
                    }
                }
                // Empty input (no digits at all) is an error too — Rust's parser rejects `""`/`"-"`/`"+"`.
                let parsed = if !saw_digit {
                    None
                } else {
                    acc.and_then(|a| if neg { Some(a) } else { a.checked_neg() })
                };
                match parsed {
                    Some(v) => {
                        regs[dst] = RegVal::Small(v);
                        pc += 1;
                    }
                    None => {
                        trap = Some(false);
                        break;
                    } // non-numeric / empty / overflow → trap (VM returns Err)
                }
            }
            237 => {
                // TextBytes { dst, text=ra } — UTF-8 encode a Text char-list
                // `text_bytes(text)` reads the char-list at `regs[ra]` (`[len, cp0, cp1, …]`) and builds a fresh
                // `Seq of Int` `[n, b0, b1, …]` of its UTF-8 bytes, byte-identical to the VM's
                // `bytes_to_seq(s.as_bytes())`. Each codepoint encodes by its range ([`utf8_of_codepoint`], the
                // RTL's `tbx` encoder); the bytes follow the header in order, and the header, written last,
                // carries the total byte count.
                let ptr = regs[ra].idx();
                let len = heap[ptr].sm() as usize;
                let base = heap.len();
                heap.push(RegVal::Small(0));
                for k in 0..len {
                    let (bytes, n) = utf8_of_codepoint(heap[ptr + 1 + k].sm() as u32);
                    for &b in &bytes[..n] {
                        heap.push(RegVal::Small(i64::from(b)));
                    }
                }
                heap[base] = RegVal::Small((heap.len() - base - 1) as i64);
                regs[dst] = RegVal::Small(base as i64);
                pc += 1;
            }
            _ => {
                let r = soft_core_alu_regval(opcode, &regs[ra], &regs[rb]);
                regs[dst] = r;
                pc += 1;
            }
        }
        // FLOAT-HEAP PLANE INVARIANT: keep `fheap` the SAME LENGTH as `heap` after every op. Any op that
        // grew `heap` (an int list/map/range/struct/…) leaves a matching `0.0` float cell — the float-list
        // ops (213/214/215) keep their own float cells aligned as they push, so this only backfills the
        // integer-structure growth. Mirrors the RTL `fdmem` bump tracking `dmem`.
        fheap.resize(heap.len(), 0.0);
    }
    (out, trap)
}

/// The UTF-8 encoding of one codepoint by its range — one byte below 0x80, two below 0x800, three below
/// 0x10000, else four — returned as the bytes and their count. On every Unicode scalar value this is
/// exactly `char::encode_utf8`; it is written by range because it is the RTL's encoder too (`tbx` in
/// `soft_core.v`), so the model and the silicon share one spec rather than one calling the standard library.
fn utf8_of_codepoint(cp: u32) -> ([u8; 4], usize) {
    let cont = |shift: u32| 0x80 | ((cp >> shift) & 0x3F) as u8;
    if cp < 0x80 {
        ([cp as u8, 0, 0, 0], 1)
    } else if cp < 0x800 {
        ([0xC0 | (cp >> 6) as u8, cont(0), 0, 0], 2)
    } else if cp < 0x1_0000 {
        ([0xE0 | (cp >> 12) as u8, cont(6), cont(0), 0], 3)
    } else {
        (
            [0xF0 | ((cp >> 18) & 0x07) as u8, cont(12), cont(6), cont(0)],
            4,
        )
    }
}

/// The integer-differential view of [`run_soft_core_typed`]: the `Show` sequence as raw `i64` values.
/// The pure integer programs (the hand-assembled ≡VM differential, the testbench emitter, the silicon
/// demo) never emit a float `Show`, so at the 32-bit width this stays value-identical to the pre-FPU
/// model (a 32-bit result carried in the low bits of an `i64`). A float program uses
/// [`run_soft_core_typed`] + [`ShowVal::fmt`] instead (panics loudly here if one leaks a float).
pub fn run_soft_core(imem: &[u32], max_cycles: usize) -> Vec<i64> {
    run_soft_core_w(imem, max_cycles, 32)
}

/// The WIDTH-PARAMETRIC integer-differential view of [`run_soft_core_typed_w`]: the `Show` sequence as raw
/// `i64` values MASKED to the datapath `width` — the fixed-width tape-out core's OUTPUT REGISTER view
/// (`width == 32` truncates each value to i32, `width == 64` is the full i64 pattern). This is DISTINCT
/// from [`run_soft_core_typed_w`], whose `ShowVal` carries the UNBOUNDED value (VM-exact via BigInt): the
/// typed path is the chip that "differs nowhere from the VM"; this raw view models what a `width`-bit
/// hardware output register would show if you read the low bits, so the 32-bit tape-out core's wrap is
/// still observable here. A value that overflowed to `Big` contributes its low-64-bit two's-complement
/// pattern, then sign-extended to `width`. (Float/Text/Word32 `Show`s panic — the pure-integer callers
/// never emit them.)
pub fn run_soft_core_w(imem: &[u32], max_cycles: usize, width: u32) -> Vec<i64> {
    run_soft_core_typed_w(imem, max_cycles, width)
        .iter()
        .map(|sv| sign_extend_to_width(showval_low_i64(sv), width))
        .collect()
}

/// The low-64-bit two's-complement pattern of an integer `Show` — the `i64` for a `Small`, the low limb
/// (with sign) for a `Big`. The width-masked raw view ([`run_soft_core_w`]) reads this; a Float/Text/
/// Word32 `Show` panics (the pure-integer differential/testbench callers never emit one).
fn showval_low_i64(sv: &ShowVal) -> i64 {
    match sv {
        ShowVal::Int(v) => *v,
        ShowVal::BigInt(b) => {
            // The low 64 bits as a two's-complement i64: reduce the exact value mod 2^64 via the
            // signed remainder against 2^64 (an over-i64 `Big` never comes from the 32-bit oracle, so
            // this only serves the width-64 raw view of a genuinely huge value).
            let mask = BigInt::from_u64(u64::MAX).add(&BigInt::from_i64(1)); // 2^64
            let (_, r) = b.div_rem(&mask).expect("2^64 is nonzero");
            // `r` carries the dividend's sign and |r| < 2^64; fold to the u64 bit pattern, then i64.
            let (neg, bytes) = r.to_le_bytes();
            let mut limb = [0u8; 8];
            for (i, byte) in bytes.iter().take(8).enumerate() {
                limb[i] = *byte;
            }
            let u = u64::from_le_bytes(limb);
            let u = if neg { u.wrapping_neg() } else { u };
            u as i64
        }
        ShowVal::Float(f) => panic!("integer soft-core run emitted a float Show ({f})"),
        ShowVal::Text(s) => panic!("integer soft-core run emitted a text Show ({s:?})"),
        ShowVal::Word32(w) => panic!("integer soft-core run emitted a word32 Show ({w})"),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/soft_core_model/width_toggle_tests.rs"]
mod width_toggle_tests;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/soft_core_model/text_bytes_tests.rs"]
mod text_bytes_tests;
