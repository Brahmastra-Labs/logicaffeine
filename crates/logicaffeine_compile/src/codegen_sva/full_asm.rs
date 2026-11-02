//! full_asm — assemble a compiled LOGOS program (register bytecode `CompiledProgram`) into the
//! instruction words the FULL soft-core (`soft_core.v`) executes: the whole 113-op ISA — user functions
//! (hardware arg-copy `Call`), division/modulo (the sequential shared divider), and the structured heap
//! (lists, tuples, ranges, sets, maps, structs) — so the ENTIRE benchmark suite compiles from `.lg` source
//! to words that run on silicon byte-identically to the VM.
//!
//! Unlike `scalar_asm` (the function-free combinational subset), this targets the full datapath and lays
//! out Main + every function body in one imem image. The op BYTE is always `binisa::opcode(op)` — the
//! append-only ISA ABI the hardware decoder keys on — and every operand field is packed exactly as
//! `codegen_sva::rtl::run_soft_core` decodes it (the ≡VM software model is the field-layout spec).
//!
//! Locked ≡ VM by `rtl.rs::tests::encode_full_matches_vm`.

use crate::vm::CompiledProgram;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/full_asm/survey.rs"]
mod survey;

use crate::vm::binisa::opcode;
use crate::vm::{Constant, Op, Reg};

/// A register index must fit the full core's 256-entry file (8-bit operand fields).
fn reg(r: Reg) -> Result<u32, String> {
    if r > 255 {
        return Err(format!(
            "register r{r} exceeds the full core's 256-register file"
        ));
    }
    Ok(r as u32)
}

/// `[opcode:8][dst:8][a:8][b:8]` — the general full-core word (8-bit operand fields).
fn w(oc: u16, dst: u32, a: u32, b: u32) -> u32 {
    ((oc as u32) << 24) | (dst << 16) | (a << 8) | b
}

// ── CHIP-SYNTHETIC FLOAT OPCODES ──────────────────────────────────────────────────────────────────
// The soft-core is dynamically typed at the VM level (`Add`/`Mul`/… are polymorphic over Int and f64),
// but silicon needs a fixed datapath per op. These opcodes drive the 64-bit float bank + `fpu.v`. They
// sit ABOVE the append-only ISA table (like `LoadConstWide`=200) so they are pure chip-internal decode
// with NO `Op` variant and NO ISA-map entry — the ISA totality/append-only locks stay untouched. The
// encoder emits them by STATICALLY lowering a float-typed op (see [`FloatTyping`]); `run_soft_core`
// models each with real Rust `f64` (bit-identical to the RTL), and `soft_core.v` dispatches them to
// `fpu.v`. Numbers must match `codegen_sva::rtl::run_soft_core_typed` and `soft_core.v` exactly.
const FCONST: u16 = 201; // 3-word: [201][dst][0][0] ‖ bits[31:0] ‖ bits[63:32]
const FADD: u16 = 202;
const FSUB: u16 = 203;
const FMUL: u16 = 204;
const FDIV: u16 = 205;
#[allow(dead_code)]
const FSQRT: u16 = 206; // reached via the `sqrt(x)` builtin lowering (spectral_norm / nbody)
const FLT: u16 = 207; // float `<` -> int 0/1 in regs
const FEQ: u16 = 208; // float `==` -> int 0/1 in regs
const FLE: u16 = 229; // float `<=` -> int 0/1 in regs (le = lt|eq; `>=` is FLe with swapped operands)
const FNE: u16 = 230; // float `!=` -> int 0/1 in regs (ne = !eq)
                      // The `is approximately` (isclose) compare has NO dedicated float opcode: `float_shape` DECOMPOSES it into
                      // the primitive float ops the RTL already sequences (FEq/FSub/FAbs/FMax/FConst/FMul/FLe) combined by an int
                      // `BitOr` — so no bespoke multi-cycle isclose FSM is needed and the SAME image runs on the model and RTL.
const BITOR: u16 = 21; // `eq | le` — the ISA `BitOr` ALU op (combines the two 0/1 isclose results)

// The FLOAT-plane CallBuiltin (opcode 38) coprocessor ordinals — a chip-internal numbering DISTINCT from
// the integer-plane ordinals in `builtin_chip_ordinal` (5/7/8/9): these read `fregs` (an operand proven
// float by the type oracle). 36/37/38 are `abs`/`min`/`max` on floats (fregs → fregs). 39/40/41 are
// `floor`/`ceil`/`round` on a float — fregs → REGS (an INT result: the VM's `f.floor() as i64` etc). The
// model's `38 =>` arm branches on the ordinal to pick the plane; `soft_core.v`'s fpu case mirrors it.
const CB_FABS: u32 = 36;
const CB_FMIN: u32 = 37;
const CB_FMAX: u32 = 38;
const CB_FLOOR: u32 = 39;
const CB_FCEIL: u32 = 40;
const CB_FROUND: u32 = 41;
const INT_TO_F: u16 = 209; // fregs[dst] = regs[src] as f64  (Int operand used in a float context)
#[allow(dead_code)]
const F_TO_INT: u16 = 210; // regs[dst] = fregs[src] as i32
const FMOVE: u16 = 211; // fregs[dst] = fregs[src]
const SHOWF: u16 = 212; // Show a float
                        // ── THE FLOAT-HEAP PLANE (chip-synthetic opcodes 213..=215) ──────────────────────────────────────────
                        // A `Seq of Float` handle is an INT pointer into the integer heap (which keeps the `[length, …]` header +
                        // structure); its f64 element payloads ride a PARALLEL float plane (`fheap`/`fdmem`) at the SAME indices.
                        // The encoder emits these when the float-seq type oracle proves a heap op operates on a `Seq of Float`:
                        // a float `Push`/`Add to`, a float `Set item`, and an `item i of (Seq of Float)` read.
const FLIST_PUSH: u16 = 213; // FListPush { list=dst, value=ra } — int ListPush structure + set fheap[slot]=fregs[ra]
const FINDEX: u16 = 214; // FIndex { dst, coll=ra, index=rb } — fregs[dst] = fheap[regs[ra]+regs[rb]]
const FSET_INDEX: u16 = 215; // FSetIndex { coll=dst, index=ra, value=rb } — fheap[regs[dst]+regs[ra]] = fregs[rb]
const FITER_NEXT: u16 = 231; // FIterNext { dst, exit:16 } — `Repeat for v in (Seq of Float)`: fregs[dst] = fheap[ptr+cursor]
                             // ── THE TEXT (string) PLANE (chip-synthetic opcodes 216/217) ─────────────────────────────────────────
                             // A `Text` is a char-code list `[len, c0, c1, …]` on the integer heap — so its OPERATIONS (Concat = a Text
                             // `Add`, Length, `item i of`, FormatValue) already ride the list/heap datapath. The two things a char-list
                             // identity does NOT supply are a Text CONSTANT (there is no 32-bit-integer encoding of "hello") and a Text
                             // `Show` (the string must be reconstituted from the char list). These two chip-synthetic opcodes fill the
                             // gap; the char data of every distinct Text constant is appended AFTER the code as a STRING-CONSTANT POOL in
                             // imem (compact — one shared block per string), and `TextConst` copies a block into a fresh heap char-list.
/// Pool keys of the Text a failure may need that no program constant spells: the two Bool renderings
/// (`Fail with false`) and the refusal of a non-Text contract reason, one per kind the chip carries.
const POOL_TRUE: u32 = u32::MAX - 1;
const POOL_FALSE: u32 = u32::MAX - 2;
const POOL_REFUSE_INT: u32 = u32::MAX - 3;
const POOL_REFUSE_BOOL: u32 = u32::MAX - 4;
const FAILURE_POOL: &[(u32, &str)] = &[
    (POOL_TRUE, "true"),
    (POOL_FALSE, "false"),
    (
        POOL_REFUSE_INT,
        "The reason after `otherwise` must be Text, but it was Int.",
    ),
    (
        POOL_REFUSE_BOOL,
        "The reason after `otherwise` must be Text, but it was Bool.",
    ),
];

/// How a failure reaches the chip's failure contract: stream the reason over the Text port
/// (`ShowText`), then trap-halt with `reasoned` set (`FailWith`, rb = 1).
enum FailPlan {
    /// The reason is already a Text in `src`.
    Text { src: Reg },
    /// `Fail with <Int>` — render it (`FormatValue`) first.
    Int { src: Reg },
    /// `Fail with <Bool>` — select the pooled `true`/`false` by arithmetic on the 0/1 value, using the
    /// never-returning call's `dst` as the second temporary.
    Bool { src: Reg, dst: Reg },
    /// A pooled Text (a `FailWith` message, a refusal) — its pool key.
    Pooled { key: u32 },
}

impl FailPlan {
    fn width(&self) -> usize {
        match self {
            FailPlan::Text { .. } => 2,
            FailPlan::Int { .. } => 3,
            FailPlan::Bool { .. } => 9,
            FailPlan::Pooled { .. } => 4,
        }
    }
}

/// The `format(x)` builtin on the chip: `x`'s text exactly as `Show` renders it.
enum FormatPlan {
    /// Already Text — the same handle.
    Text { dst: Reg, src: Reg },
    /// An Int — its decimal (`FormatValue`).
    Int { dst: Reg, src: Reg },
    /// A Bool — the pooled `true`/`false`, selected by arithmetic on the 0/1 value.
    Bool { dst: Reg, src: Reg },
}

impl FormatPlan {
    fn width(&self) -> usize {
        match self {
            FormatPlan::Text { .. } | FormatPlan::Int { .. } => 1,
            FormatPlan::Bool { .. } => 7,
        }
    }
}

fn format_plan(i: usize, op: &Op, ft: &FloatTyping) -> Result<Option<FormatPlan>, String> {
    use crate::semantics::builtins::BuiltinId;
    let Op::CallBuiltin {
        dst,
        builtin: BuiltinId::Format,
        args_start,
        arg_count,
    } = op
    else {
        return Ok(None);
    };
    if *arg_count != 1 {
        return Err(format!("instruction {i}: format() takes one value"));
    }
    let (dst, src) = (*dst, *args_start);
    Ok(Some(if ft.is_text(i, src) {
        FormatPlan::Text { dst, src }
    } else if ft.is_float(i, src) {
        return Err(format!(
            "instruction {i}: format() of a Real has no chip rendering (the chip's float formatter is \
             fixed-precision, not `Show`'s)"
        ));
    } else if ft.is_bool(i, src) {
        FormatPlan::Bool { dst, src }
    } else {
        FormatPlan::Int { dst, src }
    }))
}

/// The failure lowering of op `i`, if it is one: a `FailWith` with a Text message, or the `Fail` /
/// `FailReason` builtins. A reason this chip cannot render as `Show` would (a Float) is refused.
fn failure_plan(
    i: usize,
    op: &Op,
    constants: &[Constant],
    ft: &FloatTyping,
) -> Result<Option<FailPlan>, String> {
    use crate::semantics::builtins::BuiltinId;
    Ok(match op {
        Op::FailWith { msg } if matches!(constants.get(*msg as usize), Some(Constant::Text(_))) => {
            Some(FailPlan::Pooled { key: *msg })
        }
        Op::CallBuiltin {
            dst,
            builtin: b @ (BuiltinId::Fail | BuiltinId::FailReason),
            args_start,
            ..
        } => {
            let a = *args_start;
            Some(if ft.is_text(i, a) {
                FailPlan::Text { src: a }
            } else if ft.is_float(i, a) {
                return Err(format!(
                    "instruction {i}: a failure reason that is a Real has no chip rendering (the chip's float \
                     formatter is fixed-precision, not `Show`'s)"
                ));
            } else if *b == BuiltinId::Fail {
                if ft.is_bool(i, a) {
                    FailPlan::Bool { src: a, dst: *dst }
                } else {
                    FailPlan::Int { src: a }
                }
            } else if ft.is_bool(i, a) {
                FailPlan::Pooled {
                    key: POOL_REFUSE_BOOL,
                }
            } else {
                FailPlan::Pooled {
                    key: POOL_REFUSE_INT,
                }
            })
        }
        _ => None,
    })
}

/// The string-pool key of the program name a chip's `args()` answers with ([`crate::semantics::PROGRAM_NAME`],
/// the host-less argv); no real constant index reaches it.
const ARGV0_POOL_KEY: u32 = u32::MAX;
const TEXTCONST: u16 = 216; // TextConst { dst } — 2-word: word0=[216][dst][len:16], word1=pool_word_offset (imem index of c0). Copies len chars imem[off..off+len] into a fresh heap [len, chars…]; regs[dst]=ptr.
const SHOWTEXT: u16 = 217; // ShowText { src=ra } — decode the char list at regs[ra] to a UTF-8 string and Show it
const TEXTCHARAT: u16 = 218; // TextCharAt { dst, coll=ra, index=rb } — a single-char Text: fresh heap [1, heap[regs[ra]+regs[rb]]] (the VM's `item i of text` returns a 1-char string)
const TEXTEQ: u16 = 219; // TextEq { dst, a=ra, b=rb } — char-list CONTENT equality (regs[dst] = 1/0), matching the VM's Text `equals`
const TEXTNE: u16 = 220; // TextNe { dst, a=ra, b=rb } — char-list content INEQUALITY (the negation of TextEq)
                         // ── THE WORD32 (ℤ/2³²) PLANE (chip-synthetic opcode 221) ─────────────────────────────────────────────
                         // A `Word32` is the 32-bit register's own bit pattern, so its VALUE rides the ordinary integer datapath —
                         // `word32(n)`/`intOfWord32(w)` are the identity (`Move`), `word_and`/`word_or`/`word_not` are the existing
                         // `BitAnd`/`BitOr`/`Not` ALU ops (bit-identical to the ring), and a Word32 `+` is `Add` (wrapping i32 ==
                         // wrapping u32). The ONLY things the integer datapath does NOT supply are a ROTATE and a LOGICAL right shift
                         // (the ALU's `Shr` is ARITHMETIC) — routed through the `CallBuiltin` coprocessor (opcode 38) — and an
                         // UNSIGNED `Show`: the integer `Show`=89 prints `i32` SIGNED, but a Word32 (SHA-256 hash word > 2³¹) must
                         // print unsigned. `ShowWord32` is that unsigned Show; the model renders `ShowVal::Word32`, and `soft_core.v`
                         // routes it to a separate output port so the host formats it unsigned (int/float/string Shows unchanged).
const SHOWWORD32: u16 = 221; // ShowWord32 { src=ra } — Show the 32-bit result at regs[ra] as UNSIGNED (0..2³²−1)
                             // ── THE 64-BIT CONSTANT (chip-synthetic opcode 222) ──────────────────────────────────────────────────
                             // The inline 16-bit (opcode 5) and wide 32-bit (opcode 200) LoadConst forms cannot hold a constant that
                             // exceeds i32 (the benchmark PRNGs' `% 2147483648`/`% 4294967296` moduli, 2³¹ and 2³², or a full-range i64
                             // literal). Such a constant is emitted as a 3-word LoadConstWide64 at EVERY datapath width: word0 =
                             // [222][dst][0][0], word1 = bits[31:0], word2 = bits[63:32] — the full i64, mirroring `FConst`'s
                             // two-immediate-word layout. `run_soft_core_typed_w`'s `222 =>` arm retires the FULL i64 irrespective of
                             // width, and the value plane auto-promotes to BigInt on overflow exactly like the VM's unbounded `Int`, so
                             // the full core reproduces every constant — and every arithmetic result — EXACTLY at width 32 as at 64. The
                             // width is a fast-path/area knob (how wide the native adder is), not a correctness switch: BigInt handles
                             // exactness, so the full-core image is width-independent. (The separate 32-bit tape-out core in
                             // `scalar_asm.rs` genuinely wraps at 2³² — that is the small-area chip, distinct from this exact full core.)
const LOADCONSTWIDE64: u16 = 222;
// ── THE INT-KEY MAP PLANE (chip-synthetic opcodes 223/224) ───────────────────────────────────────────
// A `Map of Int to …` stores its keys as DIRECT integers in the key cells (not char-list pointers). The
// shared map ops SetIndex/Index (57/59) compare keys with `key_eq` — a char-list CONTENT compare that reads
// the key as a heap pointer — which is correct for a `Map of Text` key but WRONG for an integer key (it would
// dereference the int as an address). These two chip-synthetic ops carry the int-key path: `MapIndexInt`
// (223) looks up by integer `==`, `MapSetIndexInt` (224) sets/appends by integer `==`. The encoder emits them
// for an `Index`/`SetIndex` whose collection is a MAP and whose key is statically NON-Text; a `Map of Text`
// key (a Text-typed key register) keeps the string-key 57/59 form, and a plain LIST index stays positional
// 57/59. So the string-key map path (locked by `the_soft_core_runs_logos_programs_identically`) is untouched.
const MAPINDEXINT: u16 = 223; // MapIndexInt { dst, map=ra, key=rb } — map lookup by integer key (==)
const MAPSETINDEXINT: u16 = 224; // MapSetIndexInt { map=dst, key=ra, value=rb } — map set/append by integer key (==)
                                 // ── THE FLOAT FORMAT PLANE (chip-synthetic opcode 225) ───────────────────────────────────────────────
                                 // A `Show "{f:.N}"` on a FLOAT renders `fregs[src]` to N decimal places as a Text char-list. The integer
                                 // `FormatValue` (op 64) reads `regs[src]` and builds the decimal string of an i64; a float needs the FPU's
                                 // value and a fixed-precision decimal formatting the integer path can't express. `FormatValueFloat` reads
                                 // the f64 in `fregs[src]`, formats it EXACTLY as the VM (`apply_format_spec` on a `.N` spec = Rust's
                                 // `format!("{:.N$}", f)`), and lays the UTF-8 char-codes out as a fresh heap `[len, chars…]` — the same Text
                                 // char-list shape op 64 / `TextConst` / `Concat` produce, so a downstream `ShowText` (217) decodes it
                                 // byte-identically. The precision N (0..255; the benchmarks use 9 / 15) rides the word's b-byte.
const FORMATVALUE_FLOAT: u16 = 225; // FormatValueFloat { dst, src=ra, prec=rb } — fixed-precision decimal Text of fregs[ra]
                                    // ── THE WORD32 (ℤ/2³²) ARITHMETIC PLANE (chip-synthetic opcodes 226/227/228) ─────────────────────────
                                    // A `Word32 +`/`-`/`*` adds/subtracts/multiplies IN THE RING ℤ/2³² (wrapping mod 2³²), exactly as the VM's
                                    // `Word32::add`/`sub`/`mul` (`u32::wrapping_add`/…). This is DISTINCT from the value-integer ALU (opcodes
                                    // 8/10/11), which is now UNBOUNDED (auto-promoting to BigInt on overflow) so that a plain `Int +` matches
                                    // the VM's exact `Int` — a Word32 sum must NOT promote, it must wrap to 32 bits (SHA-256's `s1 + s0 + …`
                                    // accumulator would otherwise drift past 2³² round over round and corrupt the hash). The model computes
                                    // `((a as u32) wrapping_op (b as u32)) as i32` and stays `Small`; the result Shows UNSIGNED via ShowWord32.
const WORD32ADD: u16 = 226; // Word32Add { dst, a=ra, b=rb } — regs[dst] = (regs[ra] +ᵤ regs[rb]) mod 2³²
const WORD32SUB: u16 = 227; // Word32Sub { dst, a=ra, b=rb } — regs[dst] = (regs[ra] -ᵤ regs[rb]) mod 2³²
const WORD32MUL: u16 = 228; // Word32Mul { dst, a=ra, b=rb } — regs[dst] = (regs[ra] ·ᵤ regs[rb]) mod 2³²
                            // ── THE TEXT / SEQ BUILTIN PLANE (chip-synthetic opcodes 233–237) ──────────────────────────────────
                            // The deterministic string / fill builtins the CHIP compiler emits from source, each producing an on-heap
                            // value from register operands — byte-identical to the matching `semantics::builtins::call_builtin` arm:
                            //   • `Chr(code)` (233) — a ONE-character Text: a fresh heap char-list `[1, codepoint]` (the same shape
                            //     `TextConst`/op 64/`Concat` produce, so a downstream `ShowText` decodes it identically). An invalid
                            //     codepoint (a surrogate / > 0x10FFFF) TRAPS (the VM's `char::from_u32` → `Err`), pre-trap output stands.
                            //   • `TextContains(hay, needle)` (234) — a SUBSTRING char-scan over two Text char-lists → an int 0/1 in
                            //     regs[dst] (the empty needle is contained in everything), matching the VM's `h.contains(n)`.
                            //   • `RepeatSeq(elem, count)` (235) — a fresh `count`-slot Seq, every slot the SAME scalar `regs[elem]`
                            //     (`count ≤ 0` empty), matching the VM's `(0..n).map(|_| element.deep_clone())` for a SCALAR element.
                            //     The encoder emits it only when the element is provably a scalar (not float/text/seq/map); a
                            //     reference element would need per-slot deep copies the flat fill can't express, so it rejects inside.
                            //   • `ParseInt(text)` (236) — a bounded DECIMAL char-scan over the Text char-list at `regs[text]` → the
                            //     i64 value in `regs[dst]`: trim Unicode whitespace, an optional single leading `+`/`-`, then ≥1 ASCII
                            //     digit accumulating `acc = acc*10 − digit` in the NEGATIVE magnitude (so `-9223372036854775808` = i64::MIN
                            //     parses), negating for a positive sign. A non-numeric / empty / OVERFLOWING input TRAPS (halts, pre-trap
                            //     output stands) — byte-identical to the VM's `str::trim().parse::<i64>()` returning `Err` (the VM does NOT
                            //     promote an overflow to BigInt, it errors, and that `?` aborts the run just like the chip's `break`).
                            //   • `TextBytes(text)` (237) — the UTF-8 encoding of the Text char-list at `regs[text]` as a fresh `Seq of
                            //     Int` `[n, b0, b1, …]` (each byte 0..=255), matching the VM's `bytes_to_seq(s.as_bytes())`. A codepoint
                            //     encodes to 1–4 bytes by its range (< 0x80, < 0x800, < 0x10000, else four); a char-list holds only
                            //     scalar values (every producer — TextConst, Chr, FormatValue, Concat — validates), so no surrogate
                            //     reaches the encoder and the op never traps. The byte count is known only after the scan, so the
                            //     header is written last, the same order the RTL's single write port uses.
const CHR: u16 = 233; // Chr { dst, code=ra } — regs[dst] = ptr to a fresh 1-char Text `[1, code]`; trap on an invalid codepoint
const TEXTCONTAINS: u16 = 234; // TextContains { dst, hay=ra, needle=rb } — regs[dst] = 1/0 (needle is a substring of hay)
const REPEATSEQ: u16 = 235; // RepeatSeq { dst, elem=ra, count=rb } — regs[dst] = ptr to a fresh `count`-slot Seq filled with regs[elem]
const PARSEINT: u16 = 236; // ParseInt { dst, text=ra } — regs[dst] = the i64 value of the decimal Text at regs[ra]; trap on a non-numeric / empty / overflowing input (matches the VM's `str::trim().parse::<i64>()` error)
const TEXTBYTES: u16 = 237; // TextBytes { dst, text=ra } — regs[dst] = ptr to a fresh Seq of the UTF-8 bytes of the Text at regs[ra]

// The CallBuiltin (opcode 38) coprocessor ORDINAL for each chip-lowered builtin, carried in the word's
// b-byte. A chip-internal numbering (NOT the `BuiltinId` discriminant) shared with `run_soft_core_typed`'s
// `38 =>` arm and `soft_core.v`. This function names the INTEGER-plane ordinals (they read/write `regs`):
// 5/7/8 are Abs/Min/Max, 9 is `count_ones`; 30/31/32 are the Word32 rotate / logical-shift the integer ALU
// can't express. A builtin with a direct ALU lowering (word32/intOfWord32 → Move, word_and → BitAnd, …) is
// NOT here — it never reaches opcode 38. The FLOAT-plane numeric builtins (`floor`/`ceil`/`round` on a
// float, and `abs`/`min`/`max` on floats) ride `float_shape` with their own ordinals (36..=41), reading
// `fregs`; they are NOT integer-plane ops, so they are not returned here.
fn builtin_chip_ordinal(id: crate::semantics::builtins::BuiltinId) -> Option<u8> {
    use crate::semantics::builtins::BuiltinId::*;
    Some(match id {
        Abs => 5,
        Min => 7,
        Max => 8,
        CountOnes => 9,
        Rotl => 30,
        Rotr => 31,
        Word32Shr => 32,
        _ => return None,
    })
}

/// The chip-synthetic opcode a DETERMINISTIC string / fill builtin lowers to (`Chr`=233 / `contains` on
/// TEXT=234 / `repeatSeq`=235 / `parseInt`=236 / `text_bytes`=237), with the static TYPE GATE that keeps the lowering sound;
/// `Ok(None)` for a builtin handled elsewhere (the op-38 numeric coprocessor, the Word32 vocabulary via
/// `word_shape`) or a genuine host/reference-value boundary (`format`/uuid/…) that reaches the loud reject. An
/// `Err` is the HONEST reject for a supported builtin whose OPERAND SHAPE has no chip form — a `contains`
/// on a non-Text operand (the list `contains` rides `Op::Contains`, not this builtin) or a `repeatSeq` of
/// a float / reference element (the flat fill can't deep-copy a heap element per slot). Both `imem_width`
/// and `encode_op` consult this so their widths and rejects agree exactly. Mirrors `builtin_chip_ordinal`.
fn text_builtin_opcode(
    i: usize,
    builtin: crate::semantics::builtins::BuiltinId,
    args_start: Reg,
    ft: &FloatTyping,
) -> Result<Option<u16>, String> {
    use crate::semantics::builtins::BuiltinId;
    Ok(match builtin {
        // `chr(code)` — a 1-char Text from an INTEGER codepoint. The argument is an ordinary integer
        // register; a float/text/seq codepoint is a type error the VM also rejects (`chr` requires an Int),
        // so gate it to a scalar-int operand (not float / not text / not float-seq).
        BuiltinId::Chr => {
            let code = args_start;
            if ft.is_float(i, code) || ft.is_text(i, code) || ft.is_float_seq(i, code) {
                return Err(format!(
                    "instruction {i}: `chr` needs an integer codepoint (r{code} is not an integer) — no \
                     full-core chip lowering for a non-integer `chr` argument"
                ));
            }
            Some(CHR)
        }
        // `contains(hay, needle)` on TEXT — both operands must be Text char-lists (the LIST `contains` rides
        // `Op::Contains`, opcode 63, never this builtin). A non-Text operand has no substring semantics, so
        // reject it loudly rather than mis-scanning a numeric heap block as codepoints.
        BuiltinId::Contains => {
            let hay = args_start;
            let needle = args_start.saturating_add(1);
            if !(ft.is_text(i, hay) && ft.is_text(i, needle)) {
                return Err(format!(
                    "instruction {i}: `contains` on the full core lowers only for TEXT operands (r{hay} / \
                     r{needle} are not both Text) — a list `contains` uses `Op::Contains` (op 63), not this builtin"
                ));
            }
            Some(TEXTCONTAINS)
        }
        // `repeatSeq(elem, count)` — an `n`-slot fill of a SCALAR element. A float element would need the
        // float heap plane per slot, and a Text/seq/map element a per-slot deep copy the flat fill can't do,
        // so gate to a provably-scalar element (not float / not text / not float-seq / not map). The count is
        // an ordinary integer register (a runtime value the model's sequential fill unit reads).
        BuiltinId::RepeatSeq => {
            let elem = args_start;
            if ft.is_float(i, elem)
                || ft.is_text(i, elem)
                || ft.is_float_seq(i, elem)
                || ft.is_map(i, elem)
            {
                return Err(format!(
                    "instruction {i}: `repeatSeq` on the full core lowers only for a SCALAR element (r{elem} \
                     is a float / reference value) — a float or heap-reference element has no flat-fill chip form"
                ));
            }
            Some(REPEATSEQ)
        }
        // `parseInt(text)` — a bounded decimal char-scan over the Text char-list at `regs[args_start]` → an
        // i64 in `regs[dst]`. The operand MUST be a Text char-list (the char-scan reads it as `[len, cp…]`);
        // the VM itself errors on a non-Text argument (`parseInt requires a Text argument`), and a non-Text
        // register on the chip has no char-list to scan, so gate it to a provably-Text operand rather than
        // mis-reading a numeric/pointer register as a string. The scan itself TRAPS on a non-numeric / empty /
        // overflowing input inside the model (op 236), byte-identical to the VM's `str::parse::<i64>()` `Err`.
        BuiltinId::ParseInt => {
            let text = args_start;
            if !ft.is_text(i, text) {
                return Err(format!(
                    "instruction {i}: `parseInt` on the full core lowers only for a TEXT operand (r{text} is \
                     not a Text char-list) — the decimal char-scan reads its argument as a string; a non-Text \
                     `parseInt` argument (which the VM also rejects) has no chip lowering"
                ));
            }
            Some(PARSEINT)
        }
        // `text_bytes(text)` — the UTF-8 bytes of the Text char-list at `regs[args_start]` as a fresh Int Seq.
        // The scan reads its operand as `[len, cp…]`; the VM rejects a non-Text argument (`text_bytes()
        // requires text`), and a non-Text register has no char-list to encode, so gate it to a provably-Text
        // operand. The result is a plain integer-element Seq, so it sets no plane.
        BuiltinId::TextBytes => {
            let text = args_start;
            if !ft.is_text(i, text) {
                return Err(format!(
                    "instruction {i}: `text_bytes` on the full core lowers only for a TEXT operand (r{text} is \
                     not a Text char-list) — the UTF-8 encoder reads its argument as a string; a non-Text \
                     `text_bytes` argument (which the VM also rejects) has no chip lowering"
                ));
            }
            Some(TEXTBYTES)
        }
        _ => None,
    })
}

/// True when `v` fits the sign-extended 16-bit inline `LoadConst` immediate (1 word); otherwise the
/// encoder emits the 2-word wide `LoadConst` (opcode 200) whose second word is the full 32-bit value.
fn narrow(v: i32) -> bool {
    (-32768..=32767).contains(&v)
}

/// The FULL integer value of a constant (no truncation): an `Int` as its `i64`, a `Bool` as 0/1. This is
/// what a 64-bit datapath must hold — the PRNG moduli `2147483648` (2³¹) / `4294967296` (2³²) exceed i32, so
/// truncating them to `i32` is the 32-bit divergence source. The 32-bit encoder path still applies `as i32`
/// at the emission site (the honest 32-bit limitation); this returns the value in full so a 64-bit datapath
/// can hold it. `Err` on a Float (needs the FPU) or a Text/other constant with no integer representation.
fn const_i64(c: &Constant) -> Result<i64, String> {
    match c {
        Constant::Int(v) => Ok(*v),
        Constant::Bool(b) => Ok(i64::from(*b)),
        // `Nothing` is the compiler's default-fill placeholder for an as-yet-unbumped counter field (a
        // `Tally`/`Counter` struct field starts `Nothing`, then `CrdtBump` accumulates onto it). The VM's
        // `crdt_counter_bump(Nothing, k)` returns `Int(k)` — i.e. Nothing behaves as the additive identity 0
        // — so the chip loads it as 0 and the first bump computes `0 + k == k`, byte-identical to the VM.
        Constant::Nothing => Ok(0),
        // A `Duration` is the nanosecond count carried by a `Sleep`/`After ticks` timeout arm. The chip's
        // deterministic scheduler ignores the actual wall-clock quantity (a `Sleep` is a no-op, a `SelectWait`
        // timeout arm only fires when no recv is ready — see the model's ops 88/103/104), so the value rides
        // the integer datapath as a plain nanosecond count with no effect on output; loading it losslessly
        // keeps the constant pool honest (the over-i32 nanos count uses the 3-word op-222 form like any i64).
        Constant::Duration(nanos) => Ok(*nanos),
        Constant::Float(_) => {
            Err("float constant needs the FPU (P3) — this is a float program".into())
        }
        other => Err(format!(
            "constant {other:?} is not representable on the integer core"
        )),
    }
}

/// True when `v` fits a signed 32-bit field — the wide `LoadConst` (opcode 200) can hold it losslessly, so
/// both the 32-bit and 64-bit datapaths encode it the SAME 1-word/2-word way (width-32 output stays byte-
/// identical). A value OUTSIDE this range needs the 3-word 64-bit constant (opcode 222) on a 64-bit core;
/// on a 32-bit core it is the honest limitation (the historical i32 truncation stands).
fn fits_i32(v: i64) -> bool {
    v >= i64::from(i32::MIN) && v <= i64::from(i32::MAX)
}

/// The fixed decimal precision of a float `FormatValue` — the `N` of a `.N` format spec. The VM's
/// `apply_format_spec` renders a `Float` with a `.N` spec as `format!("{:.N$}", f)`; the soft-core's
/// `FormatValueFloat` (op 225) does the SAME, carrying `N` in the word's b-byte, so the chip's Text is
/// byte-identical to the VM's `Show` line. `spec == u32::MAX` (a bare `{f}`, no spec) or any spec that is
/// not a plain `.N` (currency `$`, alignment `>N`) has no float chip lowering yet — `None`, so a float
/// `FormatValue` outside the `.N` family falls through to the loud rejection rather than mis-rendering.
fn float_format_precision(spec: u32, debug_prefix: u32, constants: &[Constant]) -> Option<u8> {
    if spec == u32::MAX || debug_prefix != u32::MAX {
        return None; // a bare `{f}` (full-precision display) / a `{f=}` debug prefix isn't the `.N` path
    }
    let s = match constants.get(spec as usize) {
        Some(Constant::Text(s)) => s.as_str(),
        _ => return None,
    };
    let n: usize = s.strip_prefix('.')?.parse().ok()?;
    u8::try_from(n).ok() // the b-byte holds 0..=255; the benchmarks use 9 / 15
}

// ── THE TYPE ORACLE: static Int-vs-Float register typing (P3) ─────────────────────────────────────
// The VM is dynamically typed, so nothing in the bytecode says whether a given `Add`/`Mul`/`Div`/`Show`
// operates on integers or f64 — the runtime decides from the value tags. Silicon can't: each op needs a
// fixed datapath. This forward FIXPOINT dataflow recovers, per code index, which registers PROVABLY hold
// an f64 (they live in the chip's parallel `fregs` bank). The seeds are the ground truth the language
// already carries: a `Constant::Float` literal, a `Float`/`Seq of Float` function parameter or return, a
// float global, and the `sqrt` builtin. Float then PROPAGATES: an arithmetic op is float iff either
// operand is float; a `Move` copies floatness; an `item i of (Seq of Float)` reads a float. The loop
// back-edges are handled by the worklist (a register is float on entry iff it is float on ALL reaching
// paths — the join is AND, so a register used as both int and float on different paths stays int and the
// op falls to the integer datapath, which is the conservative, always-sound direction).

/// True when a `BoundaryType` is (or unwraps to) a scalar `Float`. A `Seq of Float` element read yields a
/// float scalar, so its ELEMENT kind matters at `Index`; the seq HANDLE itself is an integer pointer.
fn boundary_is_float_scalar(t: &crate::vm::BoundaryType) -> bool {
    matches!(t, crate::vm::BoundaryType::Float)
}

/// True when a `BoundaryType` is a `Seq of Float` — a heap handle whose element PAYLOADS ride the float
/// plane. The handle register itself is an integer pointer (into the int heap), but `item i of` it reads a
/// float and `Push`/`Set item` on it writes the float plane, so it needs its own lattice bit.
fn boundary_is_float_seq(t: &crate::vm::BoundaryType) -> bool {
    matches!(t, crate::vm::BoundaryType::Seq(inner) if boundary_is_float_scalar(inner))
}

/// True when a `BoundaryType` is `Text`. A Text handle is an int pointer to a char-code list; the encoder
/// carries a `text` lattice bit so a Text `Add`/`AddAssign`/`Show`/param/return lowers to the string ops.
fn boundary_is_text(t: &crate::vm::BoundaryType) -> bool {
    matches!(t, crate::vm::BoundaryType::Text)
}

/// True when a `BoundaryType` is a `Map` — an int-pointer handle to a key-scan heap block. The `map` plane
/// carries this so an `Index`/`SetIndex` on a map lowers to the KEY-SCAN chip ops (int- or string-key by the
/// key register's type) instead of the positional list ops.
fn boundary_is_map(t: &crate::vm::BoundaryType) -> bool {
    matches!(t, crate::vm::BoundaryType::Map(_, _))
}

/// True when a `BoundaryType` is a scalar `Word32` — a native `i32` register whose VALUE is its own bit
/// pattern, but whose `Show` renders UNSIGNED (0..2³²−1). The `word` plane carries this so a Word32
/// param/return/`Show` lowers to `ShowWord32`.
fn boundary_is_word32_scalar(t: &crate::vm::BoundaryType) -> bool {
    matches!(t, crate::vm::BoundaryType::Word32)
}

/// True when a `BoundaryType` is a `Seq of Word32` — an int-pointer handle whose ELEMENTS are Word32 (the
/// SHA-256 message schedule / hash state array). The handle rides the int heap; an `item i of` it reads a
/// Word32 scalar, so it seeds the `wseq` plane (mirroring `boundary_is_float_seq`).
fn boundary_is_word32_seq(t: &crate::vm::BoundaryType) -> bool {
    matches!(t, crate::vm::BoundaryType::Seq(inner) if boundary_is_word32_scalar(inner))
}

/// One register's static float typing at a program point: a `scalar` f64 (rides `fregs`) OR a `seq` handle
/// to a `Seq of Float` (an int pointer whose element payloads ride the float heap plane) OR a `text` handle
/// (an int pointer to a char-code list) OR a `word` scalar (a `Word32`, ℤ/2³²) OR a `wseq` handle to a
/// `Seq of Word32`. The bits are disjoint in practice (a register is one of them), but carried independently
/// so the per-plane join is correct. `Bottom` is all-false — an integer / pointer / unreached register.
///
/// A `Word32` is the 32-bit register's own bit pattern, so its VALUE is indistinguishable from an `i32` on
/// the chip; the plane exists ONLY to render a Word32 `Show` UNSIGNED (SHA-256's hash words exceed 2³¹) — a
/// Word32-typed `Show` lowers to `ShowWord32`. The `word`/`wseq` split mirrors the float `scalar`/`seq`
/// split: the SEQ handle is an int pointer, but an `item i of` it reads a Word32 scalar.
#[derive(Clone, Copy, PartialEq, Eq)]
struct FloatState {
    scalar: [bool; 256], // register holds an f64 scalar
    seq: [bool; 256],    // register holds a `Seq of Float` handle (its elements are floats)
    text: [bool; 256],   // register holds a `Text` handle (a char-code list) — the string plane
    word: [bool; 256],   // register holds a `Word32` scalar (ℤ/2³²) — the unsigned-Show plane
    wseq: [bool; 256],   // register holds a `Seq of Word32` handle (its elements are Word32)
    map: [bool; 256], // register holds a `Map` handle — the key-scan plane (int- vs string-key routing)
}

impl FloatState {
    fn bottom() -> Self {
        FloatState {
            scalar: [false; 256],
            seq: [false; 256],
            text: [false; 256],
            word: [false; 256],
            wseq: [false; 256],
            map: [false; 256],
        }
    }
}

/// Which registers provably hold a Bool at each code index — a MUST analysis (AND at joins, so a
/// register is Bool only when it is Bool on every path reaching the op). Bools come from a Bool
/// constant, a comparison, `not`, a Bool parameter, or a Bool-declared call return / global / struct
/// field; a `Move` carries the fact; any other write to a register ends it.
fn bool_typing(program: &CompiledProgram, structs: &StructTyping) -> Vec<[bool; 256]> {
    use crate::vm::BoundaryType;
    let n = program.code.len();
    let mut entry: Vec<Option<[bool; 256]>> = vec![None; n];
    let mut work: std::collections::VecDeque<usize> = Default::default();
    let join = |entry: &mut Vec<Option<[bool; 256]>>,
                work: &mut std::collections::VecDeque<usize>,
                pc: usize,
                st: [bool; 256]| {
        if pc >= n {
            return;
        }
        let next = match entry[pc] {
            None => st,
            Some(old) => std::array::from_fn(|r| old[r] && st[r]),
        };
        if entry[pc] != Some(next) {
            entry[pc] = Some(next);
            work.push_back(pc);
        }
    };
    let seed = |params: &[Option<BoundaryType>]| -> [bool; 256] {
        std::array::from_fn(|r| matches!(params.get(r), Some(Some(BoundaryType::Bool))))
    };
    join(&mut entry, &mut work, 0, [false; 256]);
    for f in &program.functions {
        join(&mut entry, &mut work, f.entry_pc, seed(&f.param_types));
    }
    while let Some(i) = work.pop_front() {
        let mut out = entry[i].expect("a queued op has an entry state");
        let op = &program.code[i];
        match op {
            Op::LoadConst { dst, idx } => {
                out[*dst as usize & 255] = matches!(
                    program.constants.get(*idx as usize),
                    Some(Constant::Bool(_))
                );
            }
            Op::Lt { dst, .. }
            | Op::Gt { dst, .. }
            | Op::LtEq { dst, .. }
            | Op::GtEq { dst, .. }
            | Op::Eq { dst, .. }
            | Op::NotEq { dst, .. }
            | Op::ApproxEq { dst, .. }
            | Op::Not { dst, .. } => out[*dst as usize & 255] = true,
            Op::Move { dst, src } | Op::DeepClone { dst, src } => {
                out[*dst as usize & 255] = out[*src as usize & 255]
            }
            Op::RevSwap { a, b } => out.swap(*a as usize & 255, *b as usize & 255),
            Op::Call { dst, func, .. } => {
                out[*dst as usize & 255] = matches!(
                    program
                        .functions
                        .get(*func as usize)
                        .and_then(|f| f.return_type.as_ref()),
                    Some(BoundaryType::Bool)
                );
            }
            Op::GlobalGet { dst, idx } => {
                out[*dst as usize & 255] = matches!(
                    program.global_types.get(*idx as usize),
                    Some(Some(BoundaryType::Bool))
                );
            }
            Op::GetField { dst, obj, field } => {
                out[*dst as usize & 255] = matches!(
                    declared_field_type(program, structs, i, *obj, *field),
                    Some(BoundaryType::Bool)
                );
            }
            other if rebinds_same_kind(other) => {}
            other => {
                for d in written_regs(other) {
                    out[d as usize & 255] = false;
                }
            }
        }
        for s in successors(i, op, n, |ci| ci) {
            join(&mut entry, &mut work, s, out);
        }
    }
    entry
        .into_iter()
        .map(|e| e.unwrap_or([false; 256]))
        .collect()
}

/// The whole-program float register typing: for each code index, which registers provably hold an f64
/// scalar (`fregs`) and which hold a `Seq of Float` handle (the float heap plane). Computed once; both
/// encoder passes consult it so their imem widths and emitted words agree. Mirrors the structure of
/// [`isolation_targets`] (same seed/successor logic).
struct FloatTyping {
    entry: Vec<FloatState>,
    /// Per code index, which registers provably hold a Bool ([`bool_typing`]). The chip carries a Bool
    /// as 0/1; rendering it as `true`/`false` needs this static fact.
    bools: Vec<[bool; 256]>,
}

impl FloatTyping {
    fn analyze(program: &CompiledProgram, structs: &StructTyping) -> Self {
        let n = program.code.len();
        const R: usize = 256;
        let mut entry: Vec<FloatState> = vec![FloatState::bottom(); n];
        let mut seen: Vec<bool> = vec![false; n]; // reached at least once (Bottom = false everywhere is ambiguous)
        let mut work: std::collections::VecDeque<usize> = Default::default();

        // Seed a region entry (Main or a function body) with its parameters' float-ness. A float scalar
        // param rides `fregs`; a `Seq of Float` param is an integer pointer whose ELEMENTS are float — it
        // seeds the `seq` plane (its handle isn't a scalar float, but its `item i of` reads a float).
        let seed = |entry: &mut Vec<FloatState>,
                    seen: &mut Vec<bool>,
                    work: &mut std::collections::VecDeque<usize>,
                    pc: usize,
                    param_types: &[Option<crate::vm::BoundaryType>]| {
            if pc >= entry.len() {
                return;
            }
            for (r, pt) in param_types.iter().enumerate().take(R) {
                if let Some(t) = pt {
                    entry[pc].scalar[r] = boundary_is_float_scalar(t);
                    entry[pc].seq[r] = boundary_is_float_seq(t);
                    entry[pc].text[r] = boundary_is_text(t);
                    entry[pc].word[r] = boundary_is_word32_scalar(t);
                    entry[pc].wseq[r] = boundary_is_word32_seq(t);
                    entry[pc].map[r] = boundary_is_map(t);
                }
            }
            seen[pc] = true;
            work.push_back(pc);
        };
        // (Region entries are (re)seeded at the top of every fixpoint sweep below.)

        // The transfer of op `i` given the entry state `st`: the (register, scalar-float, float-seq, text,
        // word32, word32-seq, map) writes this op performs. Reads consult `st` on the matching plane. A write of
        // all-false CLEARS every plane at `r` (the op reassigned that register to an int / pointer), which is
        // essential for the AND-join to converge — a register rebound to an int on one path stops being float
        // / word32. The heap-handle planes (seq / text / wseq / map) join with OR: a handle typing is established
        // once (a constant, a producing op, or a boundary) and, as a heap-object property, isn't lost on a
        // re-derive.
        let transfer = |i: usize,
                        st: &FloatState|
         -> Vec<(usize, bool, bool, bool, bool, bool, bool)> {
            use Op::*;
            let sf = |r: u16| st.scalar[r as usize & 255]; // scalar-float?
            let qf = |r: u16| st.seq[r as usize & 255]; // float-seq?
            let tx = |r: u16| st.text[r as usize & 255]; // text?
            let wd = |r: u16| st.word[r as usize & 255]; // word32 scalar?
            let wq = |r: u16| st.wseq[r as usize & 255]; // word32-seq?
            let mp = |r: u16| st.map[r as usize & 255]; // map handle?
            match &program.code[i] {
                LoadConst { dst, idx } => {
                    let c = program.constants.get(*idx as usize);
                    let is_f = matches!(c, Some(Constant::Float(_)));
                    let is_t = matches!(c, Some(Constant::Text(_)));
                    vec![(*dst as usize & 255, is_f, false, is_t, false, false, false)]
                }
                // Arithmetic: float iff either operand is float; TEXT iff either operand is Text (a Text `Add`
                // is string concat — the non-Text operand is FormatValue'd, so `+` on a mixed Text/int is a
                // Text result); WORD32 iff either operand is Word32 (a Word32 `+`/`-`/`*` stays Word32 —
                // SHA-256's `s1 + s0 + …` accumulator, so the final Show renders unsigned). A Word32 operand
                // is never simultaneously float / text, so the planes stay disjoint.
                Add { dst, lhs, rhs } => {
                    let f = sf(*lhs) || sf(*rhs);
                    let t = tx(*lhs) || tx(*rhs);
                    let w = (wd(*lhs) || wd(*rhs)) && !f && !t;
                    vec![(*dst as usize & 255, f, false, t, w, false, false)]
                }
                Sub { dst, lhs, rhs } | Mul { dst, lhs, rhs } => {
                    let f = sf(*lhs) || sf(*rhs);
                    let w = (wd(*lhs) || wd(*rhs)) && !f;
                    vec![(*dst as usize & 255, f, false, false, w, false, false)]
                }
                Div { dst, lhs, rhs } | ExactDiv { dst, lhs, rhs } | FloorDiv { dst, lhs, rhs } => {
                    vec![(
                        *dst as usize & 255,
                        sf(*lhs) || sf(*rhs),
                        false,
                        false,
                        false,
                        false,
                        false,
                    )]
                }
                // Bitwise `xor`/`and`/`or`/`not` — the Word32 crypto operators (`rotr(x,7) xor …`) that lower
                // straight to the ALU. The result stays Word32 iff an operand is Word32, so the diffusion
                // chain's final value Shows unsigned.
                BitXor { dst, lhs, rhs }
                | BitAnd { dst, lhs, rhs }
                | BitOr { dst, lhs, rhs }
                | Shl { dst, lhs, rhs }
                | Shr { dst, lhs, rhs } => {
                    vec![(
                        *dst as usize & 255,
                        false,
                        false,
                        false,
                        wd(*lhs) || wd(*rhs),
                        false,
                        false,
                    )]
                }
                Not { dst, src } => vec![(
                    *dst as usize & 255,
                    false,
                    false,
                    false,
                    wd(*src),
                    false,
                    false,
                )],
                AddAssign { dst, src } => {
                    let f = sf(*dst) || sf(*src);
                    let t = tx(*dst) || tx(*src);
                    let w = (wd(*dst) || wd(*src)) && !f && !t;
                    vec![(*dst as usize & 255, f, false, t, w, false, false)]
                }
                // Move copies ALL planes — a `Let ys be xs` on a float / Word32 / text / map handle keeps its typing;
                // a `DeepClone` is the same value in a fresh heap block.
                Move { dst, src } | DeepClone { dst, src } => {
                    vec![(
                        *dst as usize & 255,
                        sf(*src),
                        qf(*src),
                        tx(*src),
                        wd(*src),
                        wq(*src),
                        mp(*src),
                    )]
                }
                // `RevSwap` exchanges two registers — and with them, every plane.
                RevSwap { a, b } => {
                    let plane = |r: u16| (sf(r), qf(r), tx(r), wd(r), wq(r), mp(r));
                    let ((a0, a1, a2, a3, a4, a5), (b0, b1, b2, b3, b4, b5)) =
                        (plane(*a), plane(*b));
                    vec![
                        (*a as usize & 255, b0, b1, b2, b3, b4, b5),
                        (*b as usize & 255, a0, a1, a2, a3, a4, a5),
                    ]
                }
                // A slice is the same kind of sequence as the one it cuts: a Text slice is Text, a `Seq of Float`
                // slice a float seq.
                SliceOp {
                    dst, collection, ..
                } => {
                    vec![(
                        *dst as usize & 255,
                        false,
                        qf(*collection),
                        tx(*collection),
                        false,
                        wq(*collection),
                        false,
                    )]
                }
                // `Concat` joins two renderings into a Text; `SeqConcat` keeps the element typing of either side.
                Concat { dst, .. } => {
                    vec![(*dst as usize & 255, false, false, true, false, false, false)]
                }
                SeqConcat { dst, lhs, rhs } => {
                    vec![(
                        *dst as usize & 255,
                        false,
                        qf(*lhs) || qf(*rhs),
                        false,
                        false,
                        wq(*lhs) || wq(*rhs),
                        false,
                    )]
                }
                // `p's f`: the field's DECLARED type — a Text field is a Text handle, a Real field a float. A field
                // whose object type or declaration the oracles cannot name is an integer (all planes clear).
                GetField { dst, obj, field } => {
                    match declared_field_type(program, structs, i, *obj, *field) {
                        Some(t) => vec![(
                            *dst as usize & 255,
                            boundary_is_float_scalar(t),
                            boundary_is_float_seq(t),
                            boundary_is_text(t),
                            boundary_is_word32_scalar(t),
                            boundary_is_word32_seq(t),
                            boundary_is_map(t),
                        )],
                        None => vec![(
                            *dst as usize & 255,
                            false,
                            false,
                            false,
                            false,
                            false,
                            false,
                        )],
                    }
                }
                // Comparisons write an INT bool (never float / text / word / map), regardless of operand types.
                Lt { dst, .. }
                | Gt { dst, .. }
                | LtEq { dst, .. }
                | GtEq { dst, .. }
                | Eq { dst, .. }
                | NotEq { dst, .. }
                | ApproxEq { dst, .. } => {
                    vec![(
                        *dst as usize & 255,
                        false,
                        false,
                        false,
                        false,
                        false,
                        false,
                    )]
                }
                // `item i of xs`: a float SCALAR result iff xs is a float seq, a WORD32 scalar iff xs is a
                // Word32 seq (`item i of (Seq of Word32)`). `item i of text` is a single-CHARACTER Text. A map
                // read (`item k of m`) yields the VALUE — not a map handle — so the map plane is never set here.
                Index {
                    dst, collection, ..
                }
                | IndexUnchecked {
                    dst, collection, ..
                } => {
                    vec![(
                        *dst as usize & 255,
                        qf(*collection),
                        false,
                        tx(*collection),
                        wq(*collection),
                        false,
                        false,
                    )]
                }
                // `Repeat for v in xs`: the loop variable `v` is a float SCALAR iff the governing `IterPrepare`
                // (the compiler emits it at `i-1`, before the loop-head `IterNext`) names a `Seq of Float`
                // iterable — its element rides `fheap`, so the body's `s + v` lowers to the FPU. The iterable's
                // float-seq bit (established at the `IterPrepare`, a read) flows unchanged into this op's entry
                // state `st`, so `qf(iterable)` here IS its float-seq-ness. Else `v` is an integer element.
                IterNext { dst, .. } => {
                    let f = i > 0
                        && matches!(&program.code[i - 1], Op::IterPrepare { .. })
                        && match &program.code[i - 1] {
                            Op::IterPrepare { iterable } => qf(*iterable),
                            _ => false,
                        };
                    vec![(*dst as usize & 255, f, false, false, false, false, false)]
                }
                // A float / Word32 `Push`/`Add to` MARKS its list register as a float-seq / word-seq (the
                // value's plane decides; the list handle stays an int pointer, its seq bit records the element
                // typing). SetAdd (Set membership) rides the same detector.
                ListPush { list, value } => {
                    if sf(*value) {
                        vec![(
                            *list as usize & 255,
                            false,
                            true,
                            false,
                            false,
                            false,
                            false,
                        )]
                    } else if wd(*value) {
                        vec![(
                            *list as usize & 255,
                            false,
                            false,
                            false,
                            false,
                            true,
                            false,
                        )]
                    } else {
                        vec![]
                    }
                }
                SetAdd { set, value } => {
                    if sf(*value) {
                        vec![(*set as usize & 255, false, true, false, false, false, false)]
                    } else if wd(*value) {
                        vec![(*set as usize & 255, false, false, false, false, true, false)]
                    } else {
                        vec![]
                    }
                }
                // A float / Word32 `Set item i of xs to v` likewise CONFIRMS xs's element typing. It does NOT
                // touch the collection's map plane (a map handle stays a map after a `Set item k of m`).
                SetIndex {
                    collection, value, ..
                }
                | SetIndexUnchecked {
                    collection, value, ..
                } => {
                    if sf(*value) {
                        vec![(
                            *collection as usize & 255,
                            false,
                            true,
                            false,
                            false,
                            false,
                            false,
                        )]
                    } else if wd(*value) {
                        vec![(
                            *collection as usize & 255,
                            false,
                            false,
                            false,
                            false,
                            true,
                            false,
                        )]
                    } else {
                        vec![]
                    }
                }
                // A `[a, b, …]` list literal is a float / Word32 seq iff its first element is that scalar kind.
                NewList { dst, start, count } if *count > 0 => {
                    vec![(
                        *dst as usize & 255,
                        false,
                        sf(*start),
                        false,
                        false,
                        wd(*start),
                        false,
                    )]
                }
                // `a new Map of …` — the map handle plane's producer (seeds the key-scan routing).
                NewEmptyMap { dst } | NewEmptyMapWithCapacity { dst, .. } => {
                    vec![(*dst as usize & 255, false, false, false, false, false, true)]
                }
                // Call: the result is a float scalar / float seq / Text / Word32 / Word32 seq per the callee's
                // declared return type (the `ssig0(x: Word32) -> Word32` shape).
                Call { dst, func, .. } => {
                    let rt = program
                        .functions
                        .get(*func as usize)
                        .and_then(|f| f.return_type.as_ref());
                    let is_scalar = rt.map(boundary_is_float_scalar).unwrap_or(false);
                    let is_seq = rt.map(boundary_is_float_seq).unwrap_or(false);
                    let is_text = rt.map(boundary_is_text).unwrap_or(false);
                    let is_word = rt.map(boundary_is_word32_scalar).unwrap_or(false);
                    let is_wseq = rt.map(boundary_is_word32_seq).unwrap_or(false);
                    let is_map = rt.map(boundary_is_map).unwrap_or(false);
                    vec![(
                        *dst as usize & 255,
                        is_scalar,
                        is_seq,
                        is_text,
                        is_word,
                        is_wseq,
                        is_map,
                    )]
                }
                CallBuiltin {
                    dst,
                    builtin,
                    args_start,
                    ..
                } => {
                    use crate::semantics::builtins::BuiltinId::*;
                    let is_f = match builtin {
                        Sqrt => true, // sqrt of a number is always a float (the FPU result)
                        // `floor`/`ceil`/`round` of a FLOAT return an INTEGER (`f.floor() as i64`), so their
                        // dst leaves the float plane — a `Show floor(x)` must render an int, not a float.
                        Floor | Ceil | Round => false,
                        Abs => sf(*args_start), // |x| preserves the operand's kind (float in → float out)
                        // `min`/`max` promote to float if EITHER operand is float (`(x as f64).min(y)`), so the
                        // result is float when either arg is — matching `float_shape`'s either-operand routing.
                        Min | Max => sf(*args_start) || sf(args_start.saturating_add(1)),
                        _ => false,
                    };
                    // The Word32 vocabulary: `word32(n)`/`intOfWord32(w)` and the rotate/shift/bitwise word
                    // ops all yield a Word32 (the register's own 32-bit pattern), so their result Shows
                    // UNSIGNED. `intOfWord32` is an unsigned Int in the VM, but on a 32-bit register its value
                    // is the Word32 bit pattern — indistinguishable except at Show, so it rides the word plane.
                    let is_w = matches!(
                        builtin,
                        Word32 | IntOfWord32 | Rotl | Rotr | Word32Shr | Wand | Wor | Wnot
                    );
                    // `chr(code)` yields a ONE-character TEXT (a char-code list), so its dst is a Text handle —
                    // a downstream `Show`/`item i of`/`contains` on it must route to the string plane. `contains`
                    // yields an int 0/1, and `repeatSeq` and `text_bytes` a plain integer-element Seq (an int
                    // pointer, positionally indexed), so none sets a plane — the all-false default is correct.
                    // `format(x)` renders its argument to Text (a char-code list), like `FormatValue`.
                    let is_t = matches!(builtin, Chr | Format);
                    vec![(*dst as usize & 255, is_f, false, is_t, is_w, false, false)]
                }
                GlobalGet { dst, idx } => {
                    let gt = program
                        .global_types
                        .get(*idx as usize)
                        .and_then(|t| t.as_ref());
                    let is_scalar = gt.map(boundary_is_float_scalar).unwrap_or(false);
                    let is_seq = gt.map(boundary_is_float_seq).unwrap_or(false);
                    let is_text = gt.map(boundary_is_text).unwrap_or(false);
                    let is_word = gt.map(boundary_is_word32_scalar).unwrap_or(false);
                    let is_wseq = gt.map(boundary_is_word32_seq).unwrap_or(false);
                    let is_map = gt.map(boundary_is_map).unwrap_or(false);
                    vec![(
                        *dst as usize & 255,
                        is_scalar,
                        is_seq,
                        is_text,
                        is_word,
                        is_wseq,
                        is_map,
                    )]
                }
                // `FormatValue` renders a value to a Text char-list (integer op 64 or float op 225), so its dst
                // is a TEXT handle — the downstream `Concat` (`"" ++ formatted`) must see a Text-Text concat, not
                // re-FormatValue a register the chip already stringified. Marks only the text plane.
                FormatValue { dst, .. } => {
                    vec![(*dst as usize & 255, false, false, true, false, false, false)]
                }
                op if rebinds_same_kind(op) => vec![],
                // Every other register an op writes holds an integer / pointer: all planes clear.
                op => written_regs(op)
                    .into_iter()
                    .map(|d| (d as usize & 255, false, false, false, false, false, false))
                    .collect(),
            }
        };

        // The two planes have OPPOSITE lattice directions — SCALAR joins with AND (a register is a float
        // scalar only if float on EVERY reaching path; the conservative direction that lets a genuine
        // int/float register reuse fall to the integer datapath), SEQ joins with OR (a `Seq of Float` handle's
        // element typing is a property of the heap object, established once a float is pushed and never lost).
        // A single worklist sweep can't reconcile them: when a seq is first typed by a push INSIDE a loop, the
        // seq bit rises late, but a scalar bit DERIVED from it (an `item i of seq` read) may already have been
        // AND-latched at `false` on an earlier visit (false AND true = false, never rises again) — a loop-built
        // float seq then reads back all-zero. The fix is to iterate the whole sweep to a fixpoint: the seq
        // plane converges first (OR is monotone), and each subsequent sweep re-derives the scalar bits against
        // the now-stable seq, so nothing stays latched. Convergence is monotone, so a handful of sweeps suffice;
        // the bound is a hard backstop.
        for _sweep in 0..16 {
            let before = entry.clone();
            for x in seen.iter_mut() {
                *x = false;
            }
            work.clear();
            seed(&mut entry, &mut seen, &mut work, 0, &[]); // Main: no params
            for f in &program.functions {
                seed(&mut entry, &mut seen, &mut work, f.entry_pc, &f.param_types);
            }
            while let Some(i) = work.pop_front() {
                let mut out = entry[i];
                for (r, sc, sq, tx, wd, wq, mp) in transfer(i, &entry[i]) {
                    out.scalar[r] = sc;
                    out.seq[r] = sq;
                    out.text[r] = tx;
                    out.word[r] = wd;
                    out.wseq[r] = wq;
                    out.map[r] = mp;
                }
                let succs = successors(i, &program.code[i], n, |ci| ci);
                for s in succs {
                    if s >= n {
                        continue;
                    }
                    let mut changed = false;
                    if !seen[s] {
                        seen[s] = true;
                        // SCALAR planes (float `scalar`, `word`): (re)initialize from this predecessor — the
                        // must-analysis start, met with any later predecessors below. On a re-sweep this
                        // DISCARDS the previous sweep's scalar so a bit that was latched low can be re-derived
                        // correctly against the now-stable handle planes. HANDLE planes (seq / text / wseq):
                        // OR-preserve — NEVER drop a `Seq of Float`/`Text`/`Seq of Word32` typing carried over
                        // from a prior sweep (that carried-over, converged handle plane is what un-latches the
                        // scalars — e.g. a loop-built `Seq of Word32` whose element read must stay word-typed).
                        entry[s].scalar = out.scalar;
                        entry[s].word = out.word;
                        for r in 0..R {
                            entry[s].seq[r] |= out.seq[r];
                            entry[s].text[r] |= out.text[r];
                            entry[s].wseq[r] |= out.wseq[r];
                            entry[s].map[r] |= out.map[r];
                        }
                        changed = true;
                    } else {
                        for r in 0..R {
                            let m = entry[s].scalar[r] && out.scalar[r]; // SCALAR = AND
                            if m != entry[s].scalar[r] {
                                entry[s].scalar[r] = m;
                                changed = true;
                            }
                            let mw = entry[s].word[r] && out.word[r]; // WORD32 SCALAR = AND
                            if mw != entry[s].word[r] {
                                entry[s].word[r] = mw;
                                changed = true;
                            }
                            let mq = entry[s].seq[r] || out.seq[r]; // SEQ = OR
                            if mq != entry[s].seq[r] {
                                entry[s].seq[r] = mq;
                                changed = true;
                            }
                            let mt = entry[s].text[r] || out.text[r]; // TEXT = OR
                            if mt != entry[s].text[r] {
                                entry[s].text[r] = mt;
                                changed = true;
                            }
                            let mwq = entry[s].wseq[r] || out.wseq[r]; // WORD32 SEQ = OR
                            if mwq != entry[s].wseq[r] {
                                entry[s].wseq[r] = mwq;
                                changed = true;
                            }
                            let mm = entry[s].map[r] || out.map[r]; // MAP = OR
                            if mm != entry[s].map[r] {
                                entry[s].map[r] = mm;
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        work.push_back(s);
                    }
                }
            }
            if entry == before {
                break;
            }
        }
        FloatTyping {
            entry,
            bools: bool_typing(program, structs),
        }
    }

    /// Is register `r` a float SCALAR on entry to code index `i`?
    fn is_float(&self, i: usize, r: u16) -> bool {
        self.entry
            .get(i)
            .map(|st| st.scalar[r as usize & 255])
            .unwrap_or(false)
    }

    /// Is register `r` a `Seq of Float` handle on entry to code index `i`?
    fn is_float_seq(&self, i: usize, r: u16) -> bool {
        self.entry
            .get(i)
            .map(|st| st.seq[r as usize & 255])
            .unwrap_or(false)
    }

    /// Is register `r` a `Text` (char-code list) handle on entry to code index `i`?
    fn is_bool(&self, i: usize, r: u16) -> bool {
        self.bools
            .get(i)
            .map(|b| b[r as usize & 255])
            .unwrap_or(false)
    }

    fn is_text(&self, i: usize, r: u16) -> bool {
        self.entry
            .get(i)
            .map(|st| st.text[r as usize & 255])
            .unwrap_or(false)
    }

    /// Is register `r` a `Word32` SCALAR on entry to code index `i`? (Its `Show` renders UNSIGNED.) The
    /// `Seq of Word32` handle plane (`wseq`) has no separate accessor: an `item i of (Seq of Word32)` rides
    /// the ordinary integer heap datapath (the value is the 32-bit pattern), so only the RESULT's word
    /// typing matters — the transfer's `Index` arm derives it from the collection's `wseq` bit directly.
    fn is_word32(&self, i: usize, r: u16) -> bool {
        self.entry
            .get(i)
            .map(|st| st.word[r as usize & 255])
            .unwrap_or(false)
    }

    /// Is register `r` a `Map` handle on entry to code index `i`? An `Index`/`SetIndex` on a map lowers to the
    /// KEY-SCAN chip ops (int- or string-key by the key register's own type) rather than the positional list
    /// ops; a register that is not provably a map keeps the positional (list) lowering.
    fn is_map(&self, i: usize, r: u16) -> bool {
        self.entry
            .get(i)
            .map(|st| st.map[r as usize & 255])
            .unwrap_or(false)
    }
}

/// True when the `IterNext` at code index `i` iterates a `Seq of Float`: the loop's governing `IterPrepare`
/// (the compiler emits it at `i - 1`, immediately before the loop-head `IterNext`) names a float-seq iterable.
/// A float iteration reads each element's f64 PAYLOAD off `fheap` (op 231 `FIterNext`) into `fregs[dst]`, so
/// the loop variable is a float scalar. A `None`/non-float-seq iterable is the ordinary integer `IterNext`.
fn is_float_iternext(i: usize, program: &CompiledProgram, ft: &FloatTyping) -> bool {
    if i == 0 {
        return false;
    }
    match &program.code[i - 1] {
        Op::IterPrepare { iterable } => ft.is_float_seq(i - 1, *iterable),
        _ => false,
    }
}

/// The single dst register an op writes (for the integer-default float transfer), or `None` for a control
/// op / a multi- or no-write op we don't need to type (its dst, if any, is left non-float).
/// The registers `op` writes — the ISA's exhaustive write set. Every register typing below re-derives a
/// written register from the writing op ALONE: a fact the register held before the write never survives it.
fn written_regs(op: &Op) -> Vec<Reg> {
    let mut out = Vec::new();
    op.for_each_written_reg(|r| out.push(r));
    out
}

/// A write that rebinds its register to a value of the SAME kind, so every typing carries the register's
/// facts through it: `EnsureOwned` (a copy-on-write clone), `NetSync` (the merged replica written back), and
/// the reversible ring ops (`xor`/`+`/`-`/negation keep a Word a Word and an Int an Int).
fn rebinds_same_kind(op: &Op) -> bool {
    matches!(
        op,
        Op::EnsureOwned { .. }
            | Op::NetSync { .. }
            | Op::RevAdd { .. }
            | Op::RevSub { .. }
            | Op::RevXor { .. }
            | Op::RevNeg { .. }
    )
}

/// The declared type of field `field` (a field-name const) of the struct in register `obj` at code index
/// `i`: the struct oracle names the object's type, the program's registry declares the field.
fn declared_field_type<'p>(
    program: &'p CompiledProgram,
    st: &StructTyping,
    i: usize,
    obj: Reg,
    field: u32,
) -> Option<&'p crate::vm::BoundaryType> {
    let type_name = st.type_names.get(st.struct_of(i, obj)? as usize)?;
    let Some(Constant::Text(fname)) = program.constants.get(field as usize) else {
        return None;
    };
    let def = program.struct_types.iter().find(|d| &d.name == type_name)?;
    def.fields.iter().find(|(n, _)| n == fname).map(|(_, t)| t)
}

// ── THE STRUCT-TYPE ORACLE + USAGE-DERIVED FIELD LAYOUT ────────────────────────────────────────────
// A LOGOS struct (and a bare-int `Counter`/`Tally`, which the compiler lowers to a plain struct) is a heap
// record `[nfields, f0, …]` addressed POSITIONALLY on the chip (model ops 74/75/76/80/81/84). But the
// bytecode addresses a field by NAME (`StructInsert`/`GetField`/`CrdtBump`/`CrdtResolve` carry the
// field-name `ConstIdx`, never a slot index) — the dynamically-typed VM/tree-walker resolve names in a
// `HashMap`. To lower a field op the encoder must map (struct type, field name) → a slot position that is
// (a) CONSISTENT across every op on that struct type (so `StructInsert` writes the slot `GetField` reads),
// and (b) a compact `[0, nfields)` layout (so `NewStruct { num_fields }` allocates enough slots). The slot
// number is a pure chip-internal detail — the program's OUTPUT depends only on WHICH value lands in the
// field, not on its heap offset — so any consistent bijection works.
//
// The struct-type NAME of each object register comes from the ground truth the bytecode carries: a
// `NewStruct { type_name }` const, a `Struct(name)` parameter / global / return type. `StructTyping` is
// the forward dataflow that recovers it (propagating through `Move`). The layout is then DERIVED from
// actual usage — for each struct type, its field names in first-reference order, assigned positions 0.. —
// which is complete by construction (it never depends on the lossy/partial `struct_types` registry) and
// self-consistent (the same field name in the same type always resolves to the same slot).
struct StructTyping {
    entry: Vec<[Option<u16>; 256]>, // per code index, per register: Some(type-name index) or None
    /// Program-global struct-type names (index → name); `NewStruct` consts + `Struct(name)` boundary types.
    type_names: Vec<String>,
    /// Per struct type (by `type_names` index): field name → slot position, in first-reference order.
    field_slots: Vec<std::collections::HashMap<String, usize>>,
    /// Per struct type: field count (== `field_slots[t].len()`) — the `NewStruct { num_fields }` operand.
    field_counts: Vec<usize>,
}

impl StructTyping {
    fn analyze(program: &CompiledProgram) -> Self {
        let n = program.code.len();
        const R: usize = 256;
        // Program-global struct-type name table: every name a `NewStruct` const or a `Struct(_)` boundary
        // type mentions. The index is the register-plane's stable type id.
        let mut type_names: Vec<String> = Vec::new();
        let mut name_index: std::collections::HashMap<String, u16> =
            std::collections::HashMap::new();
        let intern = |name: &str,
                      names: &mut Vec<String>,
                      idx: &mut std::collections::HashMap<String, u16>|
         -> Option<u16> {
            if let Some(&i) = idx.get(name) {
                return Some(i);
            }
            let i = u16::try_from(names.len()).ok()?;
            names.push(name.to_string());
            idx.insert(name.to_string(), i);
            Some(i)
        };
        for op in &program.code {
            if let Op::NewStruct { type_name, .. } = op {
                if let Some(Constant::Text(s)) = program.constants.get(*type_name as usize) {
                    intern(s, &mut type_names, &mut name_index);
                }
            }
        }
        let note_bt = |t: &crate::vm::BoundaryType,
                       names: &mut Vec<String>,
                       idx: &mut std::collections::HashMap<String, u16>| {
            if let crate::vm::BoundaryType::Struct(name) = t {
                intern(name, names, idx);
            }
        };
        for f in &program.functions {
            for pt in f.param_types.iter().flatten() {
                note_bt(pt, &mut type_names, &mut name_index);
            }
            if let Some(rt) = &f.return_type {
                note_bt(rt, &mut type_names, &mut name_index);
            }
        }
        for gt in program.global_types.iter().flatten() {
            note_bt(gt, &mut type_names, &mut name_index);
        }
        // A `BoundaryType::Struct(name)` → its type-name index (a struct-typed param/global/return).
        let bt_index = |t: &crate::vm::BoundaryType| -> Option<u16> {
            match t {
                crate::vm::BoundaryType::Struct(name) => name_index.get(name.as_str()).copied(),
                _ => None,
            }
        };

        let mut entry: Vec<[Option<u16>; 256]> = vec![[None; R]; n];
        let mut seen: Vec<bool> = vec![false; n];
        let mut work: std::collections::VecDeque<usize> = Default::default();
        let seed = |entry: &mut Vec<[Option<u16>; R]>,
                    seen: &mut Vec<bool>,
                    work: &mut std::collections::VecDeque<usize>,
                    pc: usize,
                    param_types: &[Option<crate::vm::BoundaryType>]| {
            if pc >= entry.len() {
                return;
            }
            for (r, pt) in param_types.iter().enumerate().take(R) {
                if let Some(t) = pt {
                    entry[pc][r] = bt_index(t);
                }
            }
            seen[pc] = true;
            work.push_back(pc);
        };

        // The transfer: which register this op re-types (Some(type index) or None = cleared to a non-struct).
        let transfer = |i: usize, st: &[Option<u16>; R]| -> Vec<(usize, Option<u16>)> {
            use Op::*;
            match &program.code[i] {
                // `a new T …` (a struct, or a bare-int Counter/Tally lowered to a struct): the dst holds T.
                NewStruct { dst, type_name } => {
                    let ty = match program.constants.get(*type_name as usize) {
                        Some(Constant::Text(s)) => name_index.get(s.as_str()).copied(),
                        _ => None,
                    };
                    vec![(*dst as usize & 255, ty)]
                }
                // `Let q be p` keeps p's struct typing.
                Move { dst, src } => vec![(*dst as usize & 255, st[*src as usize & 255])],
                // A struct-typed call return / global read seeds the dst from the declared type.
                Call { dst, func, .. } => {
                    let ty = program
                        .functions
                        .get(*func as usize)
                        .and_then(|f| f.return_type.as_ref())
                        .and_then(bt_index);
                    vec![(*dst as usize & 255, ty)]
                }
                GlobalGet { dst, idx } => {
                    let ty = program
                        .global_types
                        .get(*idx as usize)
                        .and_then(|t| t.as_ref())
                        .and_then(bt_index);
                    vec![(*dst as usize & 255, ty)]
                }
                DeepClone { dst, src } => vec![(*dst as usize & 255, st[*src as usize & 255])],
                RevSwap { a, b } => vec![
                    (*a as usize & 255, st[*b as usize & 255]),
                    (*b as usize & 255, st[*a as usize & 255]),
                ],
                // `p's inner` where `inner` is declared a struct: the dst holds that struct.
                GetField { dst, obj, field } => {
                    let ty = st[*obj as usize & 255]
                        .and_then(|t| type_names.get(t as usize))
                        .and_then(|tn| program.struct_types.iter().find(|d| &d.name == tn))
                        .zip(match program.constants.get(*field as usize) {
                            Some(Constant::Text(f)) => Some(f),
                            _ => None,
                        })
                        .and_then(|(d, f)| d.fields.iter().find(|(n, _)| n == f))
                        .and_then(|(_, t)| bt_index(t));
                    vec![(*dst as usize & 255, ty)]
                }
                // A struct is value-typed: `StructInsert`/`CrdtBump`/`CrdtMerge`/`CrdtResolve` mutate the SAME
                // object in place (no register write), and a same-kind rebind keeps its struct.
                op if rebinds_same_kind(op) => vec![],
                // Every other register an op writes holds a non-struct value.
                op => written_regs(op)
                    .into_iter()
                    .map(|d| (d as usize & 255, None))
                    .collect(),
            }
        };

        for _sweep in 0..16 {
            let before = entry.clone();
            for x in seen.iter_mut() {
                *x = false;
            }
            work.clear();
            seed(&mut entry, &mut seen, &mut work, 0, &[]);
            for f in &program.functions {
                seed(&mut entry, &mut seen, &mut work, f.entry_pc, &f.param_types);
            }
            while let Some(i) = work.pop_front() {
                let mut out = entry[i];
                for (r, ty) in transfer(i, &entry[i]) {
                    out[r] = ty;
                }
                let succs = successors(i, &program.code[i], n, |ci| ci);
                for s in succs {
                    if s >= n {
                        continue;
                    }
                    let mut changed = false;
                    if !seen[s] {
                        seen[s] = true;
                        entry[s] = out;
                        changed = true;
                    } else {
                        for r in 0..R {
                            // MUST-agree join: keep the type only if both sides agree, else clear to None.
                            let m = if entry[s][r] == out[r] {
                                entry[s][r]
                            } else {
                                None
                            };
                            if m != entry[s][r] {
                                entry[s][r] = m;
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        work.push_back(s);
                    }
                }
            }
            if entry == before {
                break;
            }
        }

        // Build the usage-derived field layout: for each field op, resolve its object's struct type (from
        // the now-converged register plane) and record its field name in first-reference order. The result
        // is a compact `[0, nfields)` slot table per struct type, complete + self-consistent by construction.
        let mut field_slots: Vec<std::collections::HashMap<String, usize>> =
            vec![std::collections::HashMap::new(); type_names.len()];
        for (i, op) in program.code.iter().enumerate() {
            let (obj, field) = match op {
                Op::StructInsert { obj, field, .. } => (*obj, *field),
                Op::GetField { obj, field, .. } => (*obj, *field),
                Op::CrdtBump { obj, field, .. } => (*obj, *field),
                Op::CrdtResolve { obj, field, .. } => (*obj, *field),
                _ => continue,
            };
            let Some(ty) = entry.get(i).and_then(|st| st[obj as usize & 255]) else {
                continue;
            };
            let Some(Constant::Text(fname)) = program.constants.get(field as usize) else {
                continue;
            };
            let slots = &mut field_slots[ty as usize];
            if !slots.contains_key(fname.as_str()) {
                let pos = slots.len();
                slots.insert(fname.clone(), pos);
            }
        }
        let field_counts = field_slots.iter().map(|s| s.len()).collect();
        StructTyping {
            entry,
            type_names,
            field_slots,
            field_counts,
        }
    }

    /// The type-name index of the struct in register `r` on entry to code index `i`, if provably known.
    fn struct_of(&self, i: usize, r: u16) -> Option<u16> {
        self.entry.get(i).and_then(|st| st[r as usize & 255])
    }

    /// The type-name index of a struct type by NAME (a `NewStruct`'s `type_name` const), if it is a struct
    /// type this program uses. `NewStruct` reads its type from the const directly (the ground truth) rather
    /// than the register plane, which only types the dst on the op's EXIT (successor entry), not at the op.
    fn index_of_name(&self, name: &str) -> Option<u16> {
        self.type_names
            .iter()
            .position(|n| n == name)
            .and_then(|i| u16::try_from(i).ok())
    }

    /// The field count (`NewStruct` operand) of struct type `ty` — the usage-derived slot count.
    fn field_count(&self, ty: u16) -> usize {
        self.field_counts.get(ty as usize).copied().unwrap_or(0)
    }
}

/// Resolve the (slot position, field count) for a field op on register `obj` at code index `i`: the object's
/// struct type (from the oracle) + the field NAME (from the `field` const) via the usage-derived layout.
/// `Err` names the missing piece so an unlowerable struct op fails LOUDLY rather than mis-addressing a heap slot.
fn resolve_field(
    i: usize,
    constants: &[Constant],
    st: &StructTyping,
    obj: Reg,
    field: u32,
) -> Result<(usize, usize), String> {
    let ty = st.struct_of(i, obj).ok_or_else(|| {
        format!(
            "instruction {i}: struct field op on r{obj} whose struct type is not statically known — \
             no full-core chip lowering (the positional slot cannot be resolved)"
        )
    })?;
    let field_name = match constants.get(field as usize) {
        Some(Constant::Text(s)) => s.as_str(),
        _ => {
            return Err(format!(
                "instruction {i}: struct field name const #{field} is not Text"
            ))
        }
    };
    let slot = st
        .field_slots
        .get(ty as usize)
        .and_then(|m| m.get(field_name))
        .copied()
        .ok_or_else(|| {
            format!(
                "instruction {i}: field `{field_name}` not in the usage-derived layout of struct type `{}`",
                st.type_names.get(ty as usize).map(String::as_str).unwrap_or("?")
            )
        })?;
    Ok((slot, st.field_count(ty)))
}

/// An 8-bit variant TAG for an inductive constructor: the constructor-name `ConstIdx` itself. Constants are
/// content-deduped (`Compiler::add_const`), so the SAME constructor name shares ONE index — `NewInductive`'s
/// `ctor` and every `TestArm`'s `variant` referencing that name get the SAME tag, exactly the VM's name
/// equality. Must fit the model's 8-bit tag field; a program with >255 distinct constants that puts a
/// constructor name past 255 is rejected loudly (the tag would truncate).
fn inductive_tag(i: usize, ctor: u32) -> Result<u32, String> {
    if ctor > 255 {
        return Err(format!(
            "instruction {i}: inductive constructor const index {ctor} exceeds the 8-bit tag field"
        ));
    }
    Ok(ctor)
}

/// The FLOAT lowering of a single op, produced from the type oracle: a list of `IntToF` conversions
/// (`(dst_fbank, src_intbank)` — each one imem word) that must precede the main op so its integer
/// operands enter the float bank, then the main float word (self-contained, no branch target). `None`
/// means the op is NOT float-typed and the integer path (`imem_width`/`encode_op`) handles it unchanged.
/// The conversions write `fregs[reg] = regs[reg] as f64` IN PLACE (same register number, disjoint bank),
/// so a subsequent integer use of that register is unaffected.
struct FloatPlan {
    convs: Vec<(u32, u32)>, // (reg to fill in the float bank) — src==dst==the reg, converted in place
    body: Vec<u32>, // the float instruction word(s): one for a primitive op, a whole sequence for a
                    // multi-op lowering (`is approximately` decomposes into FEq/FSub/FAbs/FMax/FConst/FMul/FLe/BitOr).
}

/// Decide the float lowering of code index `i`, or `None` for the integer path. Emits `IntToF` for each
/// integer operand feeding a float op (the Int→Float boundary the type oracle located — e.g. `2.0 * k`
/// converts `k`). Comparisons with a float operand lower to `FLt`/`FEq` (int 0/1 result, so branches work).
/// `scratch` is the program-wide-unused base register (`max index any op touches + 1`); a multi-op float
/// lowering (`is approximately`) claims a small contiguous block starting there for its intermediates.
fn float_shape(
    i: usize,
    op: &Op,
    ft: &FloatTyping,
    scratch: Reg,
) -> Result<Option<FloatPlan>, String> {
    let isf = |r: Reg| ft.is_float(i, r);
    // The `CallBuiltin` opcode (38) carries the FLOAT-plane numeric coprocessor calls (abs/min/max/floor/
    // ceil/round on floats), the ordinal in the b-byte. Same opcode the integer-plane path uses; the
    // model's `38 =>` arm branches on the ordinal to read `fregs` vs `regs`.
    let cb_oc = crate::vm::binisa::opcode_of_name("CallBuiltin")
        .expect("CallBuiltin is a registered opcode");
    // A conversion for an integer operand feeding a float op: fill fregs[r] from regs[r] in place.
    let conv = |r: Reg, plan: &mut Vec<(u32, u32)>| -> Result<(), String> {
        if !isf(r) {
            plan.push((reg(r)?, reg(r)?));
        }
        Ok(())
    };
    let isq = |r: Reg| ft.is_float_seq(i, r);
    Ok(match op {
        Op::LoadConst { .. } => None, // handled in encode_op (needs the constant table); width special-cased
        // ── FLOAT-HEAP PLANE: a `Seq of Float` structure lives in the int heap; its f64 payloads ride the
        // parallel float plane. A float `Push`/`Add to` lowers to `FListPush` (int structure + fheap[slot] =
        // fregs[value]); a float `Set item` lowers to `FSetIndex`. The value is a proven scalar float (that
        // is the detector), so no `IntToF` conversion is needed. The list handle + index stay int registers.
        Op::ListPush { list, value } | Op::SetAdd { set: list, value } if isf(*value) => {
            Some(FloatPlan {
                convs: Vec::new(),
                body: vec![w(FLIST_PUSH, reg(*list)?, reg(*value)?, 0)],
            })
        }
        Op::SetIndex {
            collection,
            index,
            value,
        }
        | Op::SetIndexUnchecked {
            collection,
            index,
            value,
        } if isf(*value) => Some(FloatPlan {
            convs: Vec::new(),
            body: vec![w(FSET_INDEX, reg(*collection)?, reg(*index)?, reg(*value)?)],
        }),
        // `item i of (Seq of Float)` — read the f64 payload off the float plane into `fregs[dst]`. The
        // collection handle + index are int registers (the pointer + 1-based position); the type oracle
        // proved the collection is a float seq so the RESULT is a scalar float.
        Op::Index {
            dst,
            collection,
            index,
        }
        | Op::IndexUnchecked {
            dst,
            collection,
            index,
        } if isq(*collection) => Some(FloatPlan {
            convs: Vec::new(),
            body: vec![w(FINDEX, reg(*dst)?, reg(*collection)?, reg(*index)?)],
        }),
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs }
            if isf(*lhs) || isf(*rhs) =>
        {
            let oc = match op {
                Op::Add { .. } => FADD,
                Op::Sub { .. } => FSUB,
                Op::Mul { .. } => FMUL,
                _ => FDIV, // `/` on floats is IEEE-754 division
            };
            let mut convs = Vec::new();
            conv(*lhs, &mut convs)?;
            conv(*rhs, &mut convs)?;
            Some(FloatPlan {
                convs,
                body: vec![w(oc, reg(*dst)?, reg(*lhs)?, reg(*rhs)?)],
            })
        }
        // Exact/floor division and modulo have no float datapath (they are integer-division semantics);
        // a float operand reaching one is a real type error, rejected rather than silently mis-lowered.
        Op::ExactDiv { lhs, rhs, .. }
        | Op::FloorDiv { lhs, rhs, .. }
        | Op::Mod { lhs, rhs, .. }
            if isf(*lhs) || isf(*rhs) =>
        {
            return Err(format!(
                "instruction {i}: integer division/modulo op `{}` on a float operand has no chip lowering",
                op.name()
            ));
        }
        // `Set x to x + y` where x is a float accumulator: FAdd dst, dst, src (dst already float).
        Op::AddAssign { dst, src } if isf(*dst) || isf(*src) => {
            let mut convs = Vec::new();
            conv(*dst, &mut convs)?;
            conv(*src, &mut convs)?;
            Some(FloatPlan {
                convs,
                body: vec![w(FADD, reg(*dst)?, reg(*dst)?, reg(*src)?)],
            })
        }
        Op::Move { dst, src } if isf(*src) => Some(FloatPlan {
            convs: Vec::new(),
            body: vec![w(FMOVE, reg(*dst)?, reg(*src)?, 0)],
        }),
        // Comparisons where at least one side is float → FLt/FEq (int 0/1 result in regs).
        Op::Lt { dst, lhs, rhs } | Op::Gt { dst, lhs, rhs } if isf(*lhs) || isf(*rhs) => {
            let mut convs = Vec::new();
            conv(*lhs, &mut convs)?;
            conv(*rhs, &mut convs)?;
            // Gt a b == Lt b a.
            let (a, b) = if matches!(op, Op::Gt { .. }) {
                (*rhs, *lhs)
            } else {
                (*lhs, *rhs)
            };
            Some(FloatPlan {
                convs,
                body: vec![w(FLT, reg(*dst)?, reg(a)?, reg(b)?)],
            })
        }
        // `a <= b` → FLe(a,b); `a >= b` → `b <= a` → FLe(b,a) (operand swap, no float NOT needed).
        Op::LtEq { dst, lhs, rhs } | Op::GtEq { dst, lhs, rhs } if isf(*lhs) || isf(*rhs) => {
            let mut convs = Vec::new();
            conv(*lhs, &mut convs)?;
            conv(*rhs, &mut convs)?;
            let (a, b) = if matches!(op, Op::GtEq { .. }) {
                (*rhs, *lhs)
            } else {
                (*lhs, *rhs)
            };
            Some(FloatPlan {
                convs,
                body: vec![w(FLE, reg(*dst)?, reg(a)?, reg(b)?)],
            })
        }
        // `a == b` → FEq; `a != b` → FNe (native `!=`, NaN-true like the VM's f64 `!=`).
        Op::Eq { dst, lhs, rhs } | Op::NotEq { dst, lhs, rhs } if isf(*lhs) || isf(*rhs) => {
            let mut convs = Vec::new();
            conv(*lhs, &mut convs)?;
            conv(*rhs, &mut convs)?;
            let oc = if matches!(op, Op::NotEq { .. }) {
                FNE
            } else {
                FEQ
            };
            Some(FloatPlan {
                convs,
                body: vec![w(oc, reg(*dst)?, reg(*lhs)?, reg(*rhs)?)],
            })
        }
        // `a is approximately b` — the tolerant isclose compare, DECOMPOSED into the primitive float ops the
        // RTL already sequences per instruction (no bespoke multi-cycle isclose FSM). Bit-for-bit reproduces
        // the ONE shared definition `logicaffeine_data::ops::logos_approx_eq`, the SAME IEEE-754 RNE ops in the
        // SAME order, so the SAME image runs on the model AND `soft_core.v`:
        //   eq = FEq(a,b) ; diff = FSub(a,b) ; adiff = FAbs(diff) ; m = FMax(FAbs(a), FAbs(b)) ;
        //   scaled = 1e-9·m ; thresh = FMax(scaled, 1e-12) ; le = FLe(adiff, thresh) ; result = eq | le.
        // `logos_approx_eq(a,b) = (a==b) || |a−b| ≤ max(1e-9·max(|a|,|b|), 1e-12)`; the `eq | le` fast path
        // reproduces the `a==b` short-circuit (and stays 0-on-NaN, since `==`/`<=` are both false on NaN).
        // The float coprocessor `FMax` (op 38, ordinal 38) reads its two operands from CONSECUTIVE float
        // registers (`fregs[ra]`, `fregs[ra+1]`), so the two `|a|`/`|b|` and `scaled`/`1e-12` pairs are placed
        // adjacently. Intermediates live in a contiguous block above `scratch` (program-wide-unused), disjoint
        // per bank: the int results `eq`/`le` reuse indices in the int file that the float temps occupy in the
        // float file (the banks never alias). The final `BitOr` writes the int 0/1 into `dst`.
        Op::ApproxEq { dst, lhs, rhs } if isf(*lhs) || isf(*rhs) => {
            let mut convs = Vec::new();
            conv(*lhs, &mut convs)?;
            conv(*rhs, &mut convs)?;
            let (a, b, d) = (reg(*lhs)?, reg(*rhs)?, reg(*dst)?);
            // Contiguous scratch block s..=s+8 (all above every used register). The two FMax pairs must be
            // adjacent: (fa=|a|, fa+1=|b|) and (fsc=scaled, fsc+1=1e-12).
            let s = reg(scratch)?;
            let need_hi = s + 8;
            if need_hi > 255 {
                return Err(format!(
                    "instruction {i}: `is approximately` needs a 9-register scratch block for its isclose \
                     decomposition, but the program's live registers leave none free (base r{s})"
                ));
            }
            let fa = s; // |a|  (FMax pair lo)
            let fb = s + 1; // |b|  (FMax pair hi = fa+1)
            let fm = s + 2; // m = max(|a|,|b|)
            let fc9 = s + 3; // 1e-9
            let fsc = s + 4; // scaled = 1e-9·m       (FMax pair lo)
            let fc12 = s + 5; // 1e-12                 (FMax pair hi = fsc+1)
            let fth = s + 6; // thresh = max(scaled, 1e-12)
            let fdiff = s + 7; // a − b
            let fadiff = s + 8; // |a − b|
            let ieq = s; // int bank: eq = (a==b)   (disjoint bank — reuses the |a| index)
            let ile = s + 1; // int bank: le = diff ≤ thresh (reuses the |b| index)
            let fconst = |dst: u32, bits: u64| -> [u32; 3] {
                [
                    w(FCONST, dst, 0, 0),
                    (bits & 0xffff_ffff) as u32,
                    (bits >> 32) as u32,
                ]
            };
            let mut body: Vec<u32> = Vec::with_capacity(18);
            body.push(w(FEQ, ieq, a, b)); // eq = FEq(a,b) → regs[ieq]
            body.push(w(FSUB, fdiff, a, b)); // diff = a − b
            body.push(w(cb_oc, fadiff, fdiff, CB_FABS)); // adiff = |diff|
            body.push(w(cb_oc, fa, a, CB_FABS)); // |a|
            body.push(w(cb_oc, fb, b, CB_FABS)); // |b|  (= fa+1)
            body.push(w(cb_oc, fm, fa, CB_FMAX)); // m = FMax(fregs[fa], fregs[fa+1])
            body.extend(fconst(fc9, (1e-9f64).to_bits())); // 1e-9
            body.push(w(FMUL, fsc, fc9, fm)); // scaled = 1e-9 · m
            body.extend(fconst(fc12, (1e-12f64).to_bits())); // 1e-12  (= fsc+1)
            body.push(w(cb_oc, fth, fsc, CB_FMAX)); // thresh = FMax(fregs[fsc], fregs[fsc+1])
            body.push(w(FLE, ile, fadiff, fth)); // le = FLe(adiff, thresh) → regs[ile]
            body.push(w(BITOR, d, ieq, ile)); // result = eq | le → regs[dst]
            Some(FloatPlan { convs, body })
        }
        Op::Show { src } if isf(*src) => Some(FloatPlan {
            convs: Vec::new(),
            body: vec![w(SHOWF, 0, reg(*src)?, 0)],
        }),
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            ..
        } if matches!(builtin, crate::semantics::builtins::BuiltinId::Sqrt) && isf(*args_start) => {
            Some(FloatPlan {
                convs: Vec::new(),
                body: vec![w(FSQRT, reg(*dst)?, reg(*args_start)?, 0)],
            })
        }
        // `abs(x)` on a FLOAT — the FPU magnitude (`f.abs()`), fregs → fregs. Result stays a float scalar.
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            ..
        } if matches!(builtin, crate::semantics::builtins::BuiltinId::Abs) && isf(*args_start) => {
            Some(FloatPlan {
                convs: Vec::new(),
                body: vec![w(cb_oc, reg(*dst)?, reg(*args_start)?, CB_FABS)],
            })
        }
        // `min(a, b)` / `max(a, b)` where EITHER operand is a float — the VM promotes the pair to f64
        // (`(x as f64).min(y)` etc), so any int operand is converted (IntToF) and the compare runs on the
        // float plane (fregs → fregs). The two args are the consecutive registers `args_start` / `args_start+1`.
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            arg_count,
        } if matches!(
            builtin,
            crate::semantics::builtins::BuiltinId::Min | crate::semantics::builtins::BuiltinId::Max
        ) && *arg_count == 2
            && (isf(*args_start) || isf(args_start.saturating_add(1))) =>
        {
            let mut convs = Vec::new();
            conv(*args_start, &mut convs)?;
            conv(args_start.saturating_add(1), &mut convs)?;
            let ord = if matches!(builtin, crate::semantics::builtins::BuiltinId::Min) {
                CB_FMIN
            } else {
                CB_FMAX
            };
            Some(FloatPlan {
                convs,
                body: vec![w(cb_oc, reg(*dst)?, reg(*args_start)?, ord)],
            })
        }
        // `floor(x)` / `ceil(x)` / `round(x)` on a FLOAT — the FPU rounds to an INTEGER (`f.floor() as i64`,
        // round-half-away via Rust's `f64::round`), fregs → REGS. The result is an int scalar, so the dst
        // leaves the float plane (the type oracle marks it int); the coprocessor writes `regs[dst]`.
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            ..
        } if matches!(
            builtin,
            crate::semantics::builtins::BuiltinId::Floor
                | crate::semantics::builtins::BuiltinId::Ceil
                | crate::semantics::builtins::BuiltinId::Round
        ) && isf(*args_start) =>
        {
            use crate::semantics::builtins::BuiltinId;
            let ord = match builtin {
                BuiltinId::Floor => CB_FLOOR,
                BuiltinId::Ceil => CB_FCEIL,
                _ => CB_FROUND,
            };
            Some(FloatPlan {
                convs: Vec::new(),
                body: vec![w(cb_oc, reg(*dst)?, reg(*args_start)?, ord)],
            })
        }
        _ => None,
    })
}

// ── THE WORD32 (ℤ/2³²) LOWERING ─────────────────────────────────────────────────────────────────────
// A `Word32` is the register's own 32-bit pattern, so almost the whole vocabulary is FREE on the integer
// datapath. This shape covers the parts that need a distinct lowering:
//  • the CONSTRUCTOR/COERCION builtins `word32(n)` / `intOfWord32(w)` → `Move` (the value is the bit
//    pattern, already in the register; the coercion is a no-op copy);
//  • the BITWISE word builtins `word_and`/`word_or`/`word_not` → the `BitAnd`/`BitOr`/`Not` ALU ops
//    (bit-identical to the ring), since the source's `xor` OPERATOR already lowers to `BitXor`;
//  • the ROTATE and LOGICAL-SHIFT builtins `rotl`/`rotr`/`word32Shr` → the `CallBuiltin` coprocessor
//    (opcode 38, by ordinal) — the integer ALU has no rotate and only an ARITHMETIC `Shr`. The shift/
//    rotate count rides `regs[args_start+1]` (a plain int), matching the model's coprocessor arm;
//  • a Word32 `+`/`-`/`*` → the WORD32 ARITHMETIC ops (226/227/228), which wrap in the ring ℤ/2³² — the
//    value-integer ALU (8/10/11) is now UNBOUNDED (BigInt on overflow, to match a plain `Int`), so a
//    Word32 sum routed there would drift past 2³² instead of wrapping (SHA-256's `s1 + s0 + …` chain).
//    The oracle's word32-result marking (`wd(lhs) || wd(rhs)`) is on the op's EXIT; here we read the same
//    signal from the ENTRY state (an operand is already Word32), which is exactly when the result wraps.
//  • a Word32-typed `Show` → `ShowWord32` (opcode 221), which renders the value UNSIGNED.
// All are single-word (no operand conversions), so the width is always 1. `None` means the op is not a
// Word32 op and the integer path handles it unchanged (a Word32 `==`/`Move`/`Push`/bitwise already rides
// the integer datapath byte-identically — only its element/Show TYPING differs, carried by the type oracle).
fn word_shape(i: usize, op: &Op, ft: &FloatTyping) -> Result<Option<u32>, String> {
    use crate::semantics::builtins::BuiltinId;
    let move_oc = crate::vm::binisa::opcode_of_name("Move").expect("Move is a registered opcode");
    let and_oc =
        crate::vm::binisa::opcode_of_name("BitAnd").expect("BitAnd is a registered opcode");
    let or_oc = crate::vm::binisa::opcode_of_name("BitOr").expect("BitOr is a registered opcode");
    let not_oc = crate::vm::binisa::opcode_of_name("Not").expect("Not is a registered opcode");
    let cb_oc = crate::vm::binisa::opcode_of_name("CallBuiltin")
        .expect("CallBuiltin is a registered opcode");
    // A Word32 `+`/`-`/`*` (either operand already Word32) wraps in ℤ/2³²: the wrapping opcodes 226/227/228.
    // NOT a Text/float `Add` (those planes take precedence in the transfer; a Text `+` has a Text operand,
    // a float `+` a float operand — neither is Word32-typed here, so the guard `is_word32(operand)` is exact).
    let word32_arith = |dst: &Reg, a: &Reg, b: &Reg, oc: u16| -> Result<Option<u32>, String> {
        if ft.is_word32(i, *a) || ft.is_word32(i, *b) {
            Ok(Some(w(oc, reg(*dst)?, reg(*a)?, reg(*b)?)))
        } else {
            Ok(None)
        }
    };
    Ok(match op {
        Op::Add { dst, lhs, rhs } => return word32_arith(dst, lhs, rhs, WORD32ADD),
        Op::Sub { dst, lhs, rhs } => return word32_arith(dst, lhs, rhs, WORD32SUB),
        Op::Mul { dst, lhs, rhs } => return word32_arith(dst, lhs, rhs, WORD32MUL),
        // `Set w to w + x` on a Word32 accumulator — the in-place add wraps too (lhs = dst).
        Op::AddAssign { dst, src } => return word32_arith(dst, dst, src, WORD32ADD),
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            ..
        } => match builtin {
            // The value IS the register's bit pattern — construction / int-coercion is an identity copy.
            BuiltinId::Word32 | BuiltinId::IntOfWord32 => {
                Some(w(move_oc, reg(*dst)?, reg(*args_start)?, 0))
            }
            // Bitwise word ops → the existing ALU datapath. The two operands ride args_start, args_start+1.
            BuiltinId::Wand => Some(w(
                and_oc,
                reg(*dst)?,
                reg(*args_start)?,
                reg(args_start.saturating_add(1))?,
            )),
            BuiltinId::Wor => Some(w(
                or_oc,
                reg(*dst)?,
                reg(*args_start)?,
                reg(args_start.saturating_add(1))?,
            )),
            BuiltinId::Wnot => Some(w(not_oc, reg(*dst)?, reg(*args_start)?, 0)),
            // Rotate / logical shift → the CallBuiltin coprocessor (opcode 38) by ordinal. dst = result,
            // ra = args_start (the word), and the coprocessor reads the count from regs[args_start+1].
            BuiltinId::Rotl | BuiltinId::Rotr | BuiltinId::Word32Shr => {
                let ord = builtin_chip_ordinal(*builtin).ok_or_else(|| {
                    format!("instruction {i}: word builtin {builtin:?} has no chip ordinal")
                })?;
                // The count register (args_start+1) must fit the 8-bit operand field too.
                reg(args_start.saturating_add(1))?;
                Some(w(cb_oc, reg(*dst)?, reg(*args_start)?, ord as u32))
            }
            _ => None,
        },
        // A Word32-typed `Show` renders UNSIGNED (0..2³²−1) — the integer `Show` would print `i32` signed.
        Op::Show { src } if ft.is_word32(i, *src) => Some(w(SHOWWORD32, 0, reg(*src)?, 0)),
        _ => None,
    })
}

// ── THE TEXT (string) LOWERING ────────────────────────────────────────────────────────────────────
// A `Text` handle is an int pointer to a char-code list. Three op shapes carry Text on the chip:
//  • a Text `LoadConst` → `TextConst` (2 words: op+len then the pool word-offset), copying the string's
//    char block from the imem string-constant pool into a fresh heap char-list;
//  • a Text `Add`/`AddAssign` → the `Concat` datapath (op 32) — the VM's `Text + x` rule is string concat,
//    with a non-Text operand rendered via `to_display_string()`. An Int operand is FormatValue'd first (its
//    decimal char-list); a Text operand feeds the Concat directly. (`AddAssign` = `Set r to r + x`.)
//  • a Text `Show` → `ShowText`, which decodes the char list to a UTF-8 string.
// This mirrors the VM's kernel add: `(Text,Text)` concat, `(Text,other)`/`(other,Text)` stringify-then-concat.
enum TextLower {
    /// A Text `LoadConst`: copy the pooled char block into a fresh heap char-list. `idx` = constant index.
    Const { dst: Reg, idx: u32 },
    /// A Text concat (`Add`/`AddAssign`). `lhs`/`rhs` are the concat operands; `fmt` names the ONE operand
    /// (if any) that is not itself Text and must be FormatValue'd to its decimal char-list first (Int only).
    Concat {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
        fmt: Option<Reg>,
    },
    /// A Text `Show` — decode the char list at `src` and emit the string.
    Show { src: Reg },
    /// `item i of text` — a single-character Text (a fresh `[1, codepoint]` char-list), matching the VM.
    CharAt {
        dst: Reg,
        collection: Reg,
        index: Reg,
    },
    /// A Text `equals`/`is not` (`Eq`/`NotEq` with a Text operand) — char-list CONTENT compare. `eq` selects
    /// equality (`TextEq`) vs inequality (`TextNe`); the result is an int 0/1.
    Compare {
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
        eq: bool,
    },
    /// A Text `contains` (`Op::Contains` on a Text collection, or the `contains(hay, needle)` builtin) —
    /// a SUBSTRING char-scan (`TextContains`, op 234) → int 0/1. Distinct from the LIST/SET/MAP `Op::Contains`
    /// (op 63, element membership): a Text `contains` tests a contiguous run of codepoints, not a single cell.
    Contains { dst: Reg, hay: Reg, needle: Reg },
}

impl TextLower {
    /// The imem word count of this lowering. `Const` = 2 (op word + pool offset word); `Concat` = 1
    /// (a bare Concat) or 2 (a FormatValue then the Concat); every other Text op = 1.
    fn width(&self) -> usize {
        match self {
            TextLower::Const { .. } => 2,
            TextLower::Concat { fmt, .. } => 1 + usize::from(fmt.is_some()),
            TextLower::Show { .. }
            | TextLower::CharAt { .. }
            | TextLower::Compare { .. }
            | TextLower::Contains { .. } => 1,
        }
    }
}

/// Classify code index `i` as a Text op, or `None` for the non-Text path. Uses the Text type plane to decide
/// a Text `Add`/`AddAssign`/`Show`; a Text `LoadConst` is decided from the constant kind (ground truth). A
/// Text concat whose non-Text operand is neither a plain Int (FormatValue-able) nor a Text is rejected — the
/// chip has no `to_display_string` for a Float/Bool inside a string, so it fails loudly rather than mis-lower.
fn text_shape(
    i: usize,
    op: &Op,
    constants: &[Constant],
    ft: &FloatTyping,
) -> Result<Option<TextLower>, String> {
    let ist = |r: Reg| ft.is_text(i, r);
    // The non-Text operand of a Text concat must be a plain Int at runtime (FormatValue renders the decimal
    // string, matching the VM's `Int::to_display_string`). A float there would need a float→string path the
    // chip doesn't have; reject it. We can only prove int-ness by exclusion (not float / not text / not seq),
    // which the benchmarks satisfy (they concat only ints and strings).
    let non_text_ok = |r: Reg| -> Result<(), String> {
        if ft.is_float(i, r) || ft.is_float_seq(i, r) {
            return Err(format!(
                "instruction {i}: a Text concat with a non-integer operand (r{r} is float) has no chip lowering — \
                 only `Text + Int` interpolation and `Text + Text` concat are supported"
            ));
        }
        Ok(())
    };
    Ok(match op {
        Op::LoadConst { dst, idx } => match constants.get(*idx as usize) {
            Some(Constant::Text(_)) => Some(TextLower::Const {
                dst: *dst,
                idx: *idx,
            }),
            _ => None,
        },
        Op::Add { dst, lhs, rhs } if ist(*lhs) || ist(*rhs) => {
            let fmt = if ist(*lhs) && ist(*rhs) {
                None
            } else if ist(*lhs) {
                non_text_ok(*rhs)?;
                Some(*rhs)
            } else {
                non_text_ok(*lhs)?;
                Some(*lhs)
            };
            Some(TextLower::Concat {
                dst: *dst,
                lhs: *lhs,
                rhs: *rhs,
                fmt,
            })
        }
        // An explicit `Concat` — the interpolation `"{x:.N}"` accumulator (`"" ++ formatted ++ …`) emits this,
        // not `Add`. Both operands are Text on the chip (the accumulator started as a Text `LoadConst`; each
        // piece is a Text literal or a `FormatValue`'d char-list, already marked Text by the type oracle), so it
        // is a bare Text-Text `Concat`. A non-Text operand (a raw int piece) is FormatValue'd first, mirroring
        // the `Add` path — so a mixed interpolation lowers the same way regardless of which concat op the
        // compiler chose.
        Op::Concat { dst, lhs, rhs } if ist(*lhs) || ist(*rhs) => {
            let fmt = if ist(*lhs) && ist(*rhs) {
                None
            } else if ist(*lhs) {
                non_text_ok(*rhs)?;
                Some(*rhs)
            } else {
                non_text_ok(*lhs)?;
                Some(*lhs)
            };
            Some(TextLower::Concat {
                dst: *dst,
                lhs: *lhs,
                rhs: *rhs,
                fmt,
            })
        }
        // `Set r to r + x` on a Text accumulator: Concat into r (lhs = dst). The VM's `add_assign` for a Text
        // dst is exactly `r + x`, so this is `Concat r = r ++ x` (FormatValue-ing a non-Text x).
        Op::AddAssign { dst, src } if ist(*dst) || ist(*src) => {
            let fmt = if ist(*dst) && ist(*src) {
                None
            } else if ist(*dst) {
                non_text_ok(*src)?;
                Some(*src)
            } else {
                // A Text src added into a non-Text dst — `stringify(dst) ++ src`; the dst is the fmt operand.
                non_text_ok(*dst)?;
                Some(*dst)
            };
            Some(TextLower::Concat {
                dst: *dst,
                lhs: *dst,
                rhs: *src,
                fmt,
            })
        }
        Op::Show { src } if ist(*src) => Some(TextLower::Show { src: *src }),
        // `item i of text` — a single-char Text (the VM returns a 1-char string). The collection handle +
        // 1-based index are int registers; the result is a fresh char-list.
        Op::Index {
            dst,
            collection,
            index,
        }
        | Op::IndexUnchecked {
            dst,
            collection,
            index,
        } if ist(*collection) => Some(TextLower::CharAt {
            dst: *dst,
            collection: *collection,
            index: *index,
        }),
        // A Text `equals`/`is not` — char-list content compare (both operands are Text handles). The VM
        // compares Text by content; the chip does the same via `TextEq`/`TextNe`.
        Op::Eq { dst, lhs, rhs } if ist(*lhs) || ist(*rhs) => Some(TextLower::Compare {
            dst: *dst,
            lhs: *lhs,
            rhs: *rhs,
            eq: true,
        }),
        Op::NotEq { dst, lhs, rhs } if ist(*lhs) || ist(*rhs) => Some(TextLower::Compare {
            dst: *dst,
            lhs: *lhs,
            rhs: *rhs,
            eq: false,
        }),
        // A Text `contains` (`s contains "sub"`) — the compiler lowers the `contains` OPERATOR to `Op::Contains`
        // (op 63) for EVERY collection kind, but Text membership is SUBSTRING containment, not the element scan
        // op 63 performs (which would compare a codepoint cell against the needle's Text pointer and always
        // miss). When the COLLECTION is a Text char-list, route it to the substring scanner `TextContains`
        // (op 234) — byte-identical to the VM's `Value::contains` for two Texts (`h.contains(needle)`). The
        // NEEDLE must be provably Text too: the VM errors on a non-Text needle against a Text (`Cannot check if
        // Text contains …`), and the char-scan reads it as a char-list, so a non-Text needle is rejected loudly
        // rather than mis-scanned (never silently falling to the op-63 element scan, which is wrong for a Text).
        Op::Contains {
            dst,
            collection,
            value,
        } if ist(*collection) => {
            if !ist(*value) {
                return Err(format!(
                    "instruction {i}: `contains` on a Text needs a Text needle (r{value} is not Text) — the \
                     substring scanner reads the needle as a char-list; a non-Text needle has no chip lowering"
                ));
            }
            Some(TextLower::Contains {
                dst: *dst,
                hay: *collection,
                needle: *value,
            })
        }
        _ => None,
    })
}

/// The imem word count an op lowers to: 2 for `Call` (word2 = callee entry) and a wide `LoadConst`
/// (word2 = 32-bit immediate), 1 for everything else. Target-independent (depends only on the op kind,
/// its float typing, and constant magnitude), so pass 1 can build the pc→imem map before any target is
/// resolved. A float op costs `1 + <IntToF conversions>` (see [`float_shape`]); a float `LoadConst` is a
/// 3-word `FConst`. A Text op costs [`TextLower::width`]. Returns `Err` for an op with no chip lowering, so
/// an unsupported program fails loudly.
fn imem_width(
    i: usize,
    op: &Op,
    constants: &[Constant],
    ft: &FloatTyping,
    slice_fold: &dyn Fn(usize) -> Option<(u8, u8)>,
    scratch: Reg,
) -> Result<usize, String> {
    // A float-typed op: the lowering body (one word for a primitive op, a full sequence for `is
    // approximately`) plus one IntToF per integer operand feeding it.
    if let Some(plan) = float_shape(i, op, ft, scratch)? {
        return Ok(plan.convs.len() + plan.body.len());
    }
    // A Word32 op (construct/coerce/bitwise/rotate/shift/Show): always a single chip word.
    if word_shape(i, op, ft)?.is_some() {
        return Ok(1);
    }
    if let Some(plan) = failure_plan(i, op, constants, ft)? {
        return Ok(plan.width());
    }
    if let Some(plan) = format_plan(i, op, ft)? {
        return Ok(plan.width());
    }
    // A Text-typed op: a TextConst (2), a Concat (1) / FormatValue+Concat (2), or a ShowText (1).
    if let Some(t) = text_shape(i, op, constants, ft)? {
        return Ok(t.width());
    }
    Ok(match op {
        Op::Call { .. } => 2,
        // `args()` materializes `["logos"]`: NewEmptyList + TextConst (2) + ListPush.
        Op::Args { .. } => 4,
        // A `NewList` of a `Seq of Float` literal builds through the float plane: `NewEmptyList` + one
        // `FListPush` per element (`1 + count` words), so `item i of` reads the f64 payload. A non-float
        // `NewList` is the single-word structure op. (See the matching `encode_op` arm.)
        Op::NewList { start, count, .. } if (0..*count).any(|k| ft.is_float(i, start.saturating_add(k))) => {
            1 + *count as usize
        }
        Op::NewList { .. } => 1,
        Op::LoadConst { idx, .. } => {
            let c = constants
                .get(*idx as usize)
                .ok_or_else(|| format!("LoadConst references missing constant #{idx}"))?;
            // A float literal is ALWAYS a 3-word `FConst` (op + the two immediate words = the f64 bits) —
            // the constant kind is ground truth, so it needs no dataflow lookup.
            if let Constant::Float(_) = c {
                let _ = i; // (i unused here; kept in the signature for the float ops)
                return Ok(3);
            }
            let v = const_i64(c)?;
            // A constant wider than i32 (the PRNG moduli 2³¹ / 2³², or a full-range i64 literal) is held in a
            // 3-word LoadConstWide64 (op 222) at EVERY width. The full core's value datapath auto-promotes to
            // BigInt on overflow exactly like the VM, and op 222 retires the full i64 regardless of datapath
            // width, so the chip reproduces the constant EXACTLY at width 32 as well as 64 — the width is a
            // fast-path/area knob, not a correctness switch (BigInt handles exactness). An i32-fitting constant
            // uses the narrow (1) / 2-word (2) inline immediate.
            if !fits_i32(v) {
                3
            } else if narrow(v as i32) {
                1
            } else {
                2
            }
        }
        // Ops with a clean single-word lowering (validated by `encode_op` below).
        Op::Add { .. } | Op::Sub { .. } | Op::Mul { .. } | Op::Div { .. } | Op::Mod { .. }
        | Op::ExactDiv { .. } | Op::FloorDiv { .. } | Op::AddAssign { .. }
        // `Swap a and b` (RevSwap), the tolerant `is approximately` (ApproxEq — int `==` here, float via
        // `float_shape`), fresh set union/intersection (UnionOp/IntersectOp), swap-remove (RemoveFrom),
        // and struct-field push (ListPushField) all lower to one word.
        | Op::RevSwap { .. } | Op::ApproxEq { .. } | Op::UnionOp { .. } | Op::IntersectOp { .. }
        | Op::RemoveFrom { .. } | Op::ListPushField { .. }
        | Op::Lt { .. } | Op::Gt { .. } | Op::LtEq { .. } | Op::GtEq { .. } | Op::Eq { .. } | Op::NotEq { .. }
        | Op::BitXor { .. } | Op::BitAnd { .. } | Op::BitOr { .. } | Op::Shl { .. } | Op::Shr { .. } | Op::Not { .. }
        | Op::Move { .. } | Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. }
        | Op::Return { .. } | Op::ReturnNothing | Op::Show { .. } | Op::Halt | Op::FailWith { .. }
        | Op::NewTuple { .. } | Op::NewEmptyList { .. } | Op::NewEmptyListI32 { .. }
        | Op::NewEmptyListWithCapacity { .. } | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyMapWithCapacity { .. }
        | Op::NewEmptyMap { .. } | Op::NewEmptySet { .. } | Op::NewRange { .. }
        | Op::ListPush { .. } | Op::SetAdd { .. } | Op::Length { .. } | Op::Contains { .. } | Op::ListPop { .. }
        | Op::DeepClone { .. } | Op::SeqConcat { .. }
        | Op::Index { .. } | Op::IndexUnchecked { .. } | Op::SetIndex { .. } | Op::SetIndexUnchecked { .. }
        | Op::IterPrepare { .. } | Op::IterNext { .. } | Op::IterPop
        | Op::GlobalGet { .. } | Op::GlobalSet { .. }
        // Structs (74/75/76), inductives + match (77/78/79), CRDTs (80/81/84/82/83/113), non-capturing
        // closures (40/39), and deterministic concurrency (channels 92/93/94, Spawn 98) all lower to a
        // single self-contained chip word. `encode_op` validates each operand fits its field. (A field op
        // needs the struct oracle to resolve the slot; a closure/spawn resolves an imem entry — none add a word.)
        | Op::NewStruct { .. } | Op::StructInsert { .. } | Op::GetField { .. }
        | Op::NewInductive { .. } | Op::TestArm { .. } | Op::BindArm { .. }
        | Op::CrdtBump { .. } | Op::CrdtMerge { .. } | Op::CrdtJoin { .. }
        | Op::NewCrdt { .. } | Op::CrdtAppend { .. } | Op::CrdtResolve { .. }
        | Op::MakeClosure { .. } | Op::CallValue { .. }
        | Op::ChanNew { .. } | Op::ChanSend { .. } | Op::ChanRecv { .. } | Op::Spawn { .. }
        // The residual deterministic ops the model + RTL already decode, each a single self-contained chip
        // word (validated by `encode_op`): tuple destructure (71), the integer power unit (18), the non-
        // blocking channels (95/96/97), the handle-bound sync launch + task join/abort (99/100/101), the
        // `select` arms + wait (102/103/104), the deterministic scheduler `Sleep` no-op (88), and the offline
        // single-node networking no-ops / loopback inbox (105..=111 — Connect/Listen/Send/Stream/Await/
        // MakePeer/Sync). Each rides the ordinary integer register file and heap; none adds an imem word.
        | Op::DestructureTuple { .. } | Op::Pow { .. } | Op::Sleep { .. }
        | Op::ChanTrySend { .. } | Op::ChanTryRecv { .. } | Op::ChanClose { .. }
        | Op::SpawnHandle { .. } | Op::TaskAwait { .. } | Op::TaskAbort { .. }
        | Op::SelectArmRecv { .. } | Op::SelectArmTimeout { .. } | Op::SelectWait { .. }
        | Op::NetConnect { .. } | Op::NetListen { .. } | Op::NetSend { .. } | Op::NetStream { .. }
        | Op::NetAwait { .. } | Op::NetMakePeer { .. } | Op::NetSync { .. }
        | Op::EnsureOwned { .. } | Op::RegionBoundsGuard { .. } => 1,
        // A standalone `FormatValue` (the `Show "{x:.N}"` render): a single chip word — op 64 (integer
        // decimal) or op 225 (float fixed-precision decimal), decided by the operand's static type in
        // `encode_op`. A float operand needs a `.N` spec (the only float format the chip renders).
        Op::FormatValue { src, spec, debug_prefix, .. } => {
            if ft.is_float(i, *src) && float_format_precision(*spec, *debug_prefix, constants).is_none() {
                return Err(format!(
                    "instruction {i}: float `FormatValue` needs a `.N` precision spec — a bare `{{f}}` / \
                     currency / alignment on a float has no full-core chip lowering yet"
                ));
            }
            1
        }
        // The integer-plane numeric builtin coprocessor (abs/min/max/count_ones) is a single op-38 word. A
        // builtin with no integer-plane ordinal falls through to the reject (its float-plane / Word32 forms
        // were already caught by `float_shape`/`word_shape` above, which return a width before this match).
        Op::CallBuiltin { builtin, .. } if builtin_chip_ordinal(*builtin).is_some() => 1,
        // The deterministic string / fill / parse / encode builtins (`chr`=233 / TEXT `contains`=234 /
        // `repeatSeq`=235 / `parseInt`=236 / `text_bytes`=237) are a single chip word; the type-gate reject
        // inside `text_builtin_opcode` matches `encode_op` exactly.
        Op::CallBuiltin { builtin, args_start, .. } if text_builtin_opcode(i, *builtin, *args_start, ft)?.is_some() => 1,
        // `items i through j of xs` (SliceOp) is a single op-65 word IFF its bounds fold to small constant
        // nibbles; a non-foldable (runtime / large) bound has no chip form — reject to match `encode_op`.
        Op::SliceOp { .. } => {
            if slice_fold(i).is_some() {
                1
            } else {
                return Err(format!(
                    "instruction {i}: `SliceOp` bounds are not provably small integer constants (1..=15) — the \
                     chip's slice unit takes immediate nibble bounds, so a runtime or large slice bound has no \
                     full-core chip lowering yet"
                ));
            }
        }
        other => {
            return Err(format!(
                "`{}` has no full-core chip lowering yet (outside the integer benchmark subset)",
                other.name()
            ))
        }
    })
}

/// Emit one op's imem word(s). `entry_of` maps a callee's `FuncIdx` to its imem entry index, and
/// `imem_of` maps a code index (a Jump/branch/iter target) to its imem word index (both via the pass-1
/// pc→imem table). Word count MUST equal [`imem_width`] for the same op. A float-typed op (per the type
/// oracle `ft` at code index `i`) lowers to its `IntToF` conversions then the float word.
fn encode_op(
    i: usize,
    op: &Op,
    constants: &[Constant],
    ft: &FloatTyping,
    st: &StructTyping,
    entry_of: &dyn Fn(u16) -> Result<u32, String>,
    imem_of: &dyn Fn(usize) -> Result<u32, String>,
    pool_of: &dyn Fn(u32) -> Result<(u32, u32), String>,
    captures_nonempty: &dyn Fn(u16) -> bool,
    is_float_iter: &dyn Fn(usize) -> bool,
    slice_fold: &dyn Fn(usize) -> Option<(u8, u8)>,
    scratch: Reg,
) -> Result<Vec<u32>, String> {
    // ── FLOAT DATAPATH: the type oracle proved this op operates on f64. Emit each integer operand's
    // Int→Float conversion (fregs[r] = regs[r]) then the float instruction. Keeps the integer path below
    // byte-identical (a non-float op returns `None` here and falls through).
    if let Some(plan) = float_shape(i, op, ft, scratch)? {
        let mut words = Vec::with_capacity(plan.convs.len() + plan.body.len());
        for (d, s) in plan.convs {
            words.push(w(INT_TO_F, d, s, 0)); // IntToF fregs[d] <- regs[s]  (d==s: convert in place)
        }
        words.extend(plan.body);
        return Ok(words);
    }
    // ── WORD32 (ℤ/2³²) DATAPATH: `word32`/`intOfWord32` → Move (identity), `word_and`/`word_or`/`word_not`
    // → the BitAnd/BitOr/Not ALU ops, `rotl`/`rotr`/`word32Shr` → the CallBuiltin coprocessor (opcode 38),
    // and a Word32 `Show` → `ShowWord32` (unsigned). A single self-contained word; the integer path below is
    // untouched when nothing is a Word32 op (a Word32 `+`/`Move`/`Push` already rides the integer datapath).
    if let Some(word) = word_shape(i, op, ft)? {
        return Ok(vec![word]);
    }
    // ── TEXT (string) DATAPATH: a Text constant → `TextConst` (pool copy); a Text `Add`/`AddAssign` → the
    // `format(x)` → its Show text (see [`FormatPlan`]).
    if let Some(plan) = format_plan(i, op, ft)? {
        let oc = |name: &str| crate::vm::binisa::opcode_of_name(name).expect("a registered opcode");
        return Ok(match plan {
            FormatPlan::Text { dst, src } => vec![w(oc("Move"), reg(dst)?, reg(src)?, 0)],
            FormatPlan::Int { dst, src } => vec![w(oc("FormatValue"), reg(dst)?, reg(src)?, 0)],
            FormatPlan::Bool { dst, src } => {
                let (t_off, t_len) = pool_of(POOL_TRUE)?;
                let (f_off, f_len) = pool_of(POOL_FALSE)?;
                vec![
                    ((TEXTCONST as u32) << 24) | (reg(dst)? << 16) | (t_len & 0xffff),
                    t_off,
                    ((TEXTCONST as u32) << 24) | (reg(scratch)? << 16) | (f_len & 0xffff),
                    f_off,
                    w(oc("Sub"), reg(dst)?, reg(dst)?, reg(scratch)?), // true − false
                    w(oc("Mul"), reg(dst)?, reg(dst)?, reg(src)?),     // × the 0/1 value
                    w(oc("Add"), reg(dst)?, reg(dst)?, reg(scratch)?), // + false
                ]
            }
        });
    }
    // A FAILURE → stream its reason over the Text port, then trap-halt `reasoned` (see [`FailPlan`]).
    if let Some(plan) = failure_plan(i, op, constants, ft)? {
        let oc = |name: &str| crate::vm::binisa::opcode_of_name(name).expect("a registered opcode");
        let trap = (u32::from(oc("FailWith")) << 24) | 1; // rb = 1: reasoned
        let show = |r: Reg| -> Result<u32, String> { Ok(w(SHOWTEXT, 0, reg(r)?, 0)) };
        let text_const = |dst: Reg, key: u32| -> Result<[u32; 2], String> {
            let (off, len) = pool_of(key)?;
            Ok([
                ((TEXTCONST as u32) << 24) | (reg(dst)? << 16) | (len & 0xffff),
                off,
            ])
        };
        return Ok(match plan {
            FailPlan::Text { src } => vec![show(src)?, trap],
            FailPlan::Int { src } => vec![
                w(oc("FormatValue"), reg(scratch)?, reg(src)?, 0),
                show(scratch)?,
                trap,
            ],
            FailPlan::Bool { src, dst } => {
                let t = text_const(scratch, POOL_TRUE)?;
                let f = text_const(dst, POOL_FALSE)?;
                vec![
                    t[0],
                    t[1],
                    f[0],
                    f[1],
                    w(oc("Sub"), reg(scratch)?, reg(scratch)?, reg(dst)?), // true − false
                    w(oc("Mul"), reg(scratch)?, reg(scratch)?, reg(src)?), // × the 0/1 value
                    w(oc("Add"), reg(scratch)?, reg(scratch)?, reg(dst)?), // + false → the chosen text
                    show(scratch)?,
                    trap,
                ]
            }
            FailPlan::Pooled { key } => {
                let t = text_const(scratch, key)?;
                vec![t[0], t[1], show(scratch)?, trap]
            }
        });
    }
    // `Concat` datapath (FormatValue-ing a non-Text operand); a Text `Show` → `ShowText`. The Text plane
    // proved the shape; the integer path below is untouched when nothing is Text.
    if let Some(t) = text_shape(i, op, constants, ft)? {
        let concat_oc =
            crate::vm::binisa::opcode_of_name("Concat").expect("Concat is a registered opcode");
        let fmt_oc = crate::vm::binisa::opcode_of_name("FormatValue")
            .expect("FormatValue is a registered opcode");
        return Ok(match t {
            TextLower::Const { dst, idx } => {
                // TextConst: word0 = [216][dst][len:16], word1 = the pool word-offset (imem index of c0).
                let (off, len) = pool_of(idx)?;
                if len > 0xffff {
                    return Err(format!(
                        "instruction {i}: Text constant length {len} exceeds the 16-bit field"
                    ));
                }
                vec![
                    ((TEXTCONST as u32) << 24) | (reg(dst)? << 16) | (len & 0xffff),
                    off,
                ]
            }
            TextLower::Concat { dst, lhs, rhs, fmt } => match fmt {
                // Text ++ Text — a bare Concat of the two char-lists.
                None => vec![w(concat_oc, reg(dst)?, reg(lhs)?, reg(rhs)?)],
                // A non-Text operand → FormatValue it into `scratch` (its decimal char-list), then Concat with
                // the scratch substituted for that operand. The scratch is a program-wide-unused register, dead
                // the instant the Concat consumes it, so this needs no liveness dance.
                Some(nt) => {
                    let (a, b) = if nt == lhs {
                        (scratch, rhs)
                    } else {
                        (lhs, scratch)
                    };
                    vec![
                        w(fmt_oc, reg(scratch)?, reg(nt)?, 0),
                        w(concat_oc, reg(dst)?, reg(a)?, reg(b)?),
                    ]
                }
            },
            TextLower::Show { src } => vec![w(SHOWTEXT, 0, reg(src)?, 0)],
            TextLower::CharAt {
                dst,
                collection,
                index,
            } => {
                vec![w(TEXTCHARAT, reg(dst)?, reg(collection)?, reg(index)?)]
            }
            TextLower::Compare { dst, lhs, rhs, eq } => {
                vec![w(
                    if eq { TEXTEQ } else { TEXTNE },
                    reg(dst)?,
                    reg(lhs)?,
                    reg(rhs)?,
                )]
            }
            // A Text `contains` → the substring char-scanner (op 234): [234][dst][hay=ra][needle=rb].
            TextLower::Contains { dst, hay, needle } => {
                vec![w(TEXTCONTAINS, reg(dst)?, reg(hay)?, reg(needle)?)]
            }
        });
    }
    // A float LITERAL: 3-word FConst carrying the exact f64 bits (the constant kind is ground truth).
    if let Op::LoadConst { dst, idx } = op {
        if let Some(Constant::Float(f)) = constants.get(*idx as usize) {
            let bits = f.to_bits();
            return Ok(vec![
                w(FCONST, reg(*dst)?, 0, 0),
                (bits & 0xffff_ffff) as u32,
                (bits >> 32) as u32,
            ]);
        }
    }
    let oc = opcode(op);
    let three = |dst: Reg, a: Reg, b: Reg| -> Result<Vec<u32>, String> {
        Ok(vec![w(oc, reg(dst)?, reg(a)?, reg(b)?)])
    };
    Ok(match op {
        // ── arithmetic / comparison / bitwise — 3-register [op][dst][lhs][rhs] ──
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs }
        | Op::Mod { dst, lhs, rhs }
        | Op::ExactDiv { dst, lhs, rhs }
        | Op::FloorDiv { dst, lhs, rhs }
        | Op::Lt { dst, lhs, rhs }
        | Op::Gt { dst, lhs, rhs }
        | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs }
        | Op::Eq { dst, lhs, rhs }
        | Op::NotEq { dst, lhs, rhs }
        | Op::BitXor { dst, lhs, rhs }
        | Op::BitAnd { dst, lhs, rhs }
        | Op::BitOr { dst, lhs, rhs }
        | Op::Shl { dst, lhs, rhs }
        | Op::Shr { dst, lhs, rhs } => three(*dst, *lhs, *rhs)?,
        Op::Not { dst, src } => three(*dst, *src, 0)?,
        // `is approximately` (ApproxEq, opcode 30) — the tolerant numeric compare. On INTEGER operands
        // (the value plane) `≈` collapses to exact `==` (any two distinct integers differ by ≥ 1, far
        // above the 1e-9 relative floor), which is what the ALU's op-30 arm computes (`a == b`), so a
        // pure-int ApproxEq rides the ordinary 3-register ALU word. A FLOAT ApproxEq is DECOMPOSED into the
        // primitive float ops by `float_shape` above (the isclose sequence), so it never reaches here.
        Op::ApproxEq { dst, lhs, rhs } => three(*dst, *lhs, *rhs)?,
        // `Swap a and b` (RevSwap, opcode 4) — the reversible register exchange. The model's op-4 arm
        // (`regs.swap(dst, ra)`) swaps the two locals in place; the encoder emits [4][a][b][0].
        Op::RevSwap { a, b } => vec![w(oc, reg(*a)?, reg(*b)?, 0)],
        // `Set x to x + y` — in-place add lowers onto the Add datapath (lhs = dst).
        Op::AddAssign { dst, src } => {
            let add = crate::vm::binisa::opcode_of_name("Add").expect("Add opcode");
            vec![w(add, reg(*dst)?, reg(*dst)?, reg(*src)?)]
        }
        Op::Move { dst, src } => three(*dst, *src, 0)?,
        // ── constants ──
        Op::LoadConst { dst, idx } => {
            let c = constants
                .get(*idx as usize)
                .ok_or_else(|| format!("LoadConst references missing constant #{idx}"))?;
            let v = const_i64(c)?;
            if !fits_i32(v) {
                // A constant wider than i32 (PRNG moduli 2³¹ / 2³², full-range i64 literals) is held in a
                // 3-word LoadConstWide64 (op 222) at EVERY width — low word then high word, mirroring FConst's
                // two-immediate layout. The op-222 decode retires the full i64 irrespective of datapath width,
                // and the value plane auto-promotes to BigInt on overflow, so width 32 reproduces the constant
                // EXACTLY (width is a fast-path/area knob, not a correctness switch — BigInt handles exactness).
                let bits = v as u64;
                vec![
                    (u32::from(LOADCONSTWIDE64) << 24) | (reg(*dst)? << 16),
                    (bits & 0xffff_ffff) as u32,
                    (bits >> 32) as u32,
                ]
            } else {
                // An i32-fitting constant uses the inline forms EXACTLY: narrow (16-bit, opcode 5) or the
                // 2-word 32-bit immediate (opcode 200). No truncation happens here — the over-i32 case is
                // handled above by op 222 at every width, so the full core never wraps a constant.
                let vi = v as i32;
                if narrow(vi) {
                    vec![(oc as u32) << 24 | (reg(*dst)? << 16) | ((vi as u32) & 0xffff)]
                } else {
                    vec![(200u32 << 24) | (reg(*dst)? << 16), vi as u32]
                }
            }
        }
        // ── control flow — 16-bit imem targets SPLIT low-byte-first (scalar-compatible) / 32-bit Call entry ──
        // Jump: [op][t_lo:8][t_hi:8][0]; JumpIf: [op][t_lo:8][cond:8][t_hi:8].
        Op::Jump { target } => {
            let t = imem_of(*target)?;
            vec![((oc as u32) << 24) | ((t & 0xff) << 16) | (((t >> 8) & 0xff) << 8)]
        }
        Op::JumpIfFalse { cond, target } | Op::JumpIfTrue { cond, target } => {
            let t = imem_of(*target)?;
            vec![((oc as u32) << 24) | ((t & 0xff) << 16) | (reg(*cond)? << 8) | ((t >> 8) & 0xff)]
        }
        Op::Call {
            dst,
            func,
            args_start,
            arg_count,
        } => {
            if *arg_count > 255 {
                return Err(format!("Call arg_count {arg_count} exceeds 255"));
            }
            reg(args_start.saturating_add(arg_count.saturating_sub(1)))?; // the top arg register must fit too
            vec![
                ((oc as u32) << 24)
                    | (reg(*dst)? << 16)
                    | (reg(*args_start)? << 8)
                    | (*arg_count as u32 & 0xff),
                entry_of(*func)?,
            ]
        }
        Op::Return { src } => vec![w(oc, 0, reg(*src)?, 0)],
        Op::ReturnNothing => vec![(oc as u32) << 24],
        Op::Show { src } => vec![w(oc, 0, reg(*src)?, 0)],
        Op::Halt => vec![(oc as u32) << 24],
        // `args()` — argv, program name first, on every tier. A chip has no host argv, so its argument
        // vector is the program name alone, `["logos"]` — exactly the VM's with no host argv
        // (`semantics::argv_with_program_name`). Built from ops the chip already runs: a fresh list, the
        // pooled Text `logos` into the scratch register, and one push. So `length of args()` is 1 on chip
        // and VM alike.
        Op::Args { dst } => {
            let new_empty =
                crate::vm::binisa::opcode_of_name("NewEmptyList").expect("NewEmptyList opcode");
            let push = crate::vm::binisa::opcode_of_name("ListPush").expect("ListPush opcode");
            let (off, len) = pool_of(ARGV0_POOL_KEY)?;
            vec![
                w(new_empty, reg(*dst)?, 0, 0),
                ((TEXTCONST as u32) << 24) | (reg(scratch)? << 16) | (len & 0xffff),
                off,
                w(push, reg(*dst)?, reg(scratch)?, 0),
            ]
        }
        // `FailWith` — the unrecoverable trap (op 91): the model halts (error strobe raised), matching the
        // VM's error. It sits at the exhaustive-`Inspect` fall-through (a non-`Otherwise` match emits it after
        // the arms), UNREACHABLE for a well-typed scrutinee — but it must lower so the arm chain assembles.
        Op::FailWith { .. } => vec![(oc as u32) << 24],
        // ── structured heap (lists / tuples / ranges / sets / maps) ──
        Op::NewList { dst, start, count } | Op::NewTuple { dst, start, count } => {
            if *count > 255 {
                return Err(format!("NewList/NewTuple count {count} exceeds 255"));
            }
            // A `Seq of Float` literal (`[1.5, 2.5, …]`) built by a plain `NewList` would lay its structure on
            // the INT heap and leave the parallel f64 payload plane (`fheap`) all-zero — so a later `item i of
            // xs` (FIndex) reads 0.0, a SILENT divergence from the VM. Instead build it through the float plane:
            // `NewEmptyList` then one `FListPush` per element (each writes the int structure AND `fheap[slot] =
            // fregs[value]`), exactly the sequence a float `Push` loop emits (which is already proven ≡ VM). The
            // element registers are proven float scalars (`is_float`), so their f64 values live in `fregs`.
            if (0..*count).any(|k| ft.is_float(i, start.saturating_add(k))) {
                if !(0..*count).all(|k| ft.is_float(i, start.saturating_add(k))) {
                    return Err(format!(
                        "instruction {i}: `NewList` with a MIX of float and non-float elements has no chip \
                         lowering (the float-payload plane is uniform per seq)"
                    ));
                }
                reg(start.saturating_add(count.saturating_sub(1)))?;
                let new_empty =
                    crate::vm::binisa::opcode_of_name("NewEmptyList").expect("NewEmptyList opcode");
                let mut words = Vec::with_capacity(1 + *count as usize);
                words.push(w(new_empty, reg(*dst)?, 0, 0));
                for k in 0..*count {
                    words.push(w(FLIST_PUSH, reg(*dst)?, reg(start.saturating_add(k))?, 0));
                }
                return Ok(words);
            }
            reg(start.saturating_add(count.saturating_sub(1)))?;
            vec![w(oc, reg(*dst)?, reg(*start)?, *count as u32 & 0xff)]
        }
        Op::NewEmptyList { dst } | Op::NewEmptyListI32 { dst } | Op::NewEmptySet { dst } => {
            vec![w(oc, reg(*dst)?, 0, 0)]
        }
        // The physical fabric reserves a compile-time-sized collection block. Keep the semantic
        // capacity operand in bytecode/traces, but lower allocation to the existing physical
        // constructor: runtime-sized reserve has no representable field in this instruction word.
        Op::NewEmptyListWithCapacity { dst, .. } => {
            let physical =
                crate::vm::binisa::opcode_of_name("NewEmptyList").expect("NewEmptyList opcode");
            vec![w(physical, reg(*dst)?, 0, 0)]
        }
        Op::NewEmptyListFixed { dst, .. } => {
            let physical =
                crate::vm::binisa::opcode_of_name("NewEmptyList").expect("NewEmptyList opcode");
            vec![w(physical, reg(*dst)?, 0, 0)]
        }
        // `a new Map of …` — an empty MAP_TAG-tagged heap block (opcode 52). Its Index/SetIndex lower to the
        // INT-KEY key-scan ops (223/224) when the key is a non-Text integer, or stay on the string-key ops
        // (57/59, `key_eq`) when the key is a Text handle — the `is_map`/`is_text` routing below.
        Op::NewEmptyMap { dst } => vec![w(oc, reg(*dst)?, 0, 0)],
        Op::NewEmptyMapWithCapacity { dst, .. } => {
            let physical =
                crate::vm::binisa::opcode_of_name("NewEmptyMap").expect("NewEmptyMap opcode");
            vec![w(physical, reg(*dst)?, 0, 0)]
        }
        Op::NewRange { dst, start, end } => three(*dst, *start, *end)?,
        Op::ListPush { list, value } => three(*list, *value, 0)?,
        Op::SetAdd { set, value } => three(*set, *value, 0)?,
        Op::ListPop { list, dst } => three(*dst, *list, 0)?,
        // `copy of xs` — an independent deep heap copy (opcode 66); the value-semantics fresh binding.
        Op::DeepClone { dst, src } => three(*dst, *src, 0)?,
        // `a followed by b` — a fresh sequence `lhs ++ rhs` (opcode 33); Text `+` uses Concat (32) via
        // `text_shape` above, so this arm is the Seq-of-Int/Seq concatenation path.
        Op::SeqConcat { dst, lhs, rhs } => three(*dst, *lhs, *rhs)?,
        // `a union b` / `a intersection b` — a FRESH set (opcodes 68/69): dedup'd union / membership
        // intersection over the two source blocks. Three-register, single-word (the model builds the
        // result block by scanning, byte-identical to the VM's `set_union`/`set_intersection`).
        Op::UnionOp { dst, lhs, rhs } | Op::IntersectOp { dst, lhs, rhs } => {
            three(*dst, *lhs, *rhs)?
        }
        // `Remove v from s` (opcode 56) — swap-out by value equality, shifting the tail down. In-place
        // mutation of the collection register; the model finds the first `== v` slot and compacts.
        Op::RemoveFrom { collection, value } => three(*collection, *value, 0)?,
        // `items i through j of xs` (SliceOp, op 65) — a fresh 1-based-inclusive subsequence. The chip's
        // slice unit (RTL `soft_core.v` op 65) takes the bounds as IMMEDIATE b-byte nibbles (`start=b[7:4]`,
        // `end=b[3:0]`), so both must be provably small integer constants (`1..=15`), recovered by
        // `slice_bound_folds`. A runtime / out-of-range bound has no chip form — rejected loudly (a genuine
        // hardware limit, honestly declined; the op still lowers for the common constant-bound case).
        Op::SliceOp {
            dst, collection, ..
        } => {
            let (s, e) = slice_fold(i).ok_or_else(|| {
                format!(
                    "instruction {i}: `SliceOp` bounds are not provably small integer constants (1..=15) — the \
                     chip's slice unit takes immediate nibble bounds, so a runtime or large slice bound has no \
                     full-core chip lowering yet"
                )
            })?;
            vec![w(
                oc,
                reg(*dst)?,
                reg(*collection)?,
                (u32::from(s) << 4) | u32::from(e),
            )]
        }
        // `Push x to obj's field` (ListPushField, opcode 47) — append to a struct-field seq in place (COW
        // if the field's list is not on top of the heap, then repoint the field). word: [47][obj=dst][
        // src=ra][field=rb]. The field name resolves to its usage-derived positional slot via the struct
        // oracle, exactly as `StructInsert`/`GetField`.
        Op::ListPushField { obj, field, src } => {
            let (slot, _n) = resolve_field(i, constants, st, *obj, *field)?;
            if slot > 255 {
                return Err(format!(
                    "instruction {i}: `ListPushField` field slot {slot} exceeds the 8-bit field"
                ));
            }
            vec![w(oc, reg(*obj)?, reg(*src)?, slot as u32)]
        }
        Op::Length { dst, collection } => three(*dst, *collection, 0)?,
        Op::Contains {
            dst,
            collection,
            value,
        } => three(*dst, *collection, *value)?,
        Op::Index {
            dst,
            collection,
            index,
        }
        | Op::IndexUnchecked {
            dst,
            collection,
            index,
        } => {
            // A map read (`item k of m`) key-scans: the INT-KEY op (223) when the key is a non-Text integer,
            // else the string-key op (59, via `three`, which `key_eq`-scans). A plain LIST index stays positional
            // (59). The `is_map`/`is_text` oracle at this code index picks the family — the run loop then reads
            // the key cell accordingly.
            if ft.is_map(i, *collection) && !ft.is_text(i, *index) {
                vec![w(MAPINDEXINT, reg(*dst)?, reg(*collection)?, reg(*index)?)]
            } else {
                three(*dst, *collection, *index)?
            }
        }
        Op::SetIndex {
            collection,
            index,
            value,
        }
        | Op::SetIndexUnchecked {
            collection,
            index,
            value,
        } => {
            // A map write (`Set item k of m to v`) key-scans: the INT-KEY op (224) for a non-Text integer key,
            // else the string-key op (57). A plain LIST set stays positional (57).
            if ft.is_map(i, *collection) && !ft.is_text(i, *index) {
                vec![w(
                    MAPSETINDEXINT,
                    reg(*collection)?,
                    reg(*index)?,
                    reg(*value)?,
                )]
            } else {
                three(*collection, *index, *value)?
            }
        }
        // ── snapshot iteration (Repeat / for-each) ──
        Op::IterPrepare { iterable } => vec![w(oc, 0, reg(*iterable)?, 0)],
        // IterNext: [op][dst:8][exit_lo:8][exit_hi:8]. A `Repeat for v in (Seq of Float)` iterates the float
        // payload plane — op 231 `FIterNext` reads `fheap[ptr+cursor]` into `fregs[dst]` (so the loop variable
        // is a float scalar); an ordinary iteration keeps op 86 reading the int structure into `regs[dst]`.
        Op::IterNext { dst, exit } => {
            let e = imem_of(*exit)?;
            let iter_oc = if is_float_iter(i) { FITER_NEXT } else { oc };
            vec![w(iter_oc, reg(*dst)?, e & 0xff, (e >> 8) & 0xff)]
        }
        Op::IterPop => vec![(oc as u32) << 24],
        // ── globals (Main bindings referenced inside functions) ──
        Op::GlobalGet { dst, idx } => {
            if *idx > 255 {
                return Err(format!("GlobalGet idx {idx} exceeds 255"));
            }
            vec![w(oc, reg(*dst)?, *idx as u32 & 0xff, 0)]
        }
        Op::GlobalSet { idx, src } => {
            if *idx > 255 {
                return Err(format!("GlobalSet idx {idx} exceeds 255"));
            }
            vec![w(oc, *idx as u32 & 0xff, reg(*src)?, 0)]
        }
        // ── value-transparent metadata — no-ops on the chip (as in run_soft_core / the VM) ──
        Op::EnsureOwned { .. } | Op::RegionBoundsGuard { .. } => vec![(oc as u32) << 24],
        // A standalone `FormatValue` — the `Show "{x:.N}"` render. An INTEGER operand → op 64 (the decimal
        // char-list of regs[src]); a FLOAT operand → op 225 `FormatValueFloat` (fregs[src] to `.N` places, the
        // precision in the b-byte), matching the VM's `apply_format_spec` exactly. Both build a Text char-list
        // a downstream `ShowText`/`Concat` consumes. `oc` is `FormatValue`'s registered opcode (== 64).
        Op::FormatValue {
            dst,
            src,
            spec,
            debug_prefix,
        } => {
            if ft.is_float(i, *src) {
                let prec = float_format_precision(*spec, *debug_prefix, constants).ok_or_else(|| {
                    format!(
                        "instruction {i}: float `FormatValue` needs a `.N` precision spec — a bare `{{f}}` / \
                         currency / alignment on a float has no full-core chip lowering yet"
                    )
                })?;
                vec![w(
                    FORMATVALUE_FLOAT,
                    reg(*dst)?,
                    reg(*src)?,
                    u32::from(prec),
                )]
            } else {
                // Integer FormatValue (op 64) — the decimal string of regs[src]. Any spec/debug on an INT is
                // dropped (the chip has only the bare-decimal integer render); no benchmark exercises that.
                vec![w(oc, reg(*dst)?, reg(*src)?, 0)]
            }
        }
        // ── STRUCTS (model ops 74/75/76) — a heap record `[nfields, f0, …]`, fields addressed POSITIONALLY.
        // The bytecode names a field by const; the struct oracle's usage-derived layout maps (struct type,
        // field name) → its slot. `NewStruct` needs the struct's field COUNT (the slots to allocate).
        Op::NewStruct { dst, type_name } => {
            // The struct type is the ground-truth `type_name` const — resolve it directly (the register
            // plane types the dst only on the op's EXIT, not at the NewStruct itself). `field_count` is the
            // usage-derived slot count for that type.
            let name = match constants.get(*type_name as usize) {
                Some(Constant::Text(s)) => s.as_str(),
                _ => {
                    return Err(format!(
                        "instruction {i}: `NewStruct` type name const #{type_name} is not Text"
                    ))
                }
            };
            let ty = st.index_of_name(name).ok_or_else(|| {
                format!("instruction {i}: `NewStruct` of `{name}` — no usage-derived field layout for this struct type")
            })?;
            let nfields = st.field_count(ty);
            if nfields > 255 {
                return Err(format!(
                    "instruction {i}: struct has {nfields} fields (exceeds the 8-bit count)"
                ));
            }
            // op 74: [74][dst][ra unused][num_fields=rb]
            vec![w(oc, reg(*dst)?, 0, nfields as u32)]
        }
        Op::StructInsert { obj, field, value } => {
            let (slot, _n) = resolve_field(i, constants, st, *obj, *field)?;
            if slot > 255 {
                return Err(format!(
                    "instruction {i}: struct field slot {slot} exceeds the 8-bit field"
                ));
            }
            // op 75: [75][obj=dst][value=ra][field=rb]
            vec![w(oc, reg(*obj)?, reg(*value)?, slot as u32)]
        }
        Op::GetField { dst, obj, field } => {
            let (slot, _n) = resolve_field(i, constants, st, *obj, *field)?;
            if slot > 255 {
                return Err(format!(
                    "instruction {i}: struct field slot {slot} exceeds the 8-bit field"
                ));
            }
            // op 76: [76][dst][obj=ra][field=rb]
            vec![w(oc, reg(*dst)?, reg(*obj)?, slot as u32)]
        }
        // ── INDUCTIVES + MATCH (model ops 77/78/79) — a heap record `[tag, payload0, …]`. The variant TAG is
        // the constructor-name const index (deduped, so `NewInductive`'s ctor and every `TestArm`'s variant on
        // that name agree — exactly the VM's name equality); `BindArm` extracts payload position `index`.
        Op::NewInductive {
            dst,
            ctor,
            args_start,
            count,
            ..
        } => {
            let tag = inductive_tag(i, *ctor)?;
            if *count > 15 {
                return Err(format!(
                    "instruction {i}: `NewInductive` payload count {count} exceeds the 4-bit field"
                ));
            }
            let asr = reg(*args_start)?;
            if asr > 15 {
                return Err(format!(
                    "instruction {i}: `NewInductive` args_start r{args_start} exceeds the 4-bit b[7:4] field"
                ));
            }
            reg(args_start.saturating_add(count.saturating_sub(1)))?; // the top payload register must fit too
                                                                      // op 77: [77][dst][ctor=a-byte][b: args_start(hi nibble) | count(lo nibble)]
            let b = (asr << 4) | (*count as u32 & 0xf);
            vec![w(oc, reg(*dst)?, tag, b)]
        }
        Op::TestArm {
            dst,
            target,
            variant,
        } => {
            let tag = inductive_tag(i, *variant)?;
            // op 78: [78][dst][target=ra][variant=b-byte]
            vec![w(oc, reg(*dst)?, reg(*target)?, tag)]
        }
        Op::BindArm {
            dst, target, index, ..
        } => {
            if *index > 255 {
                return Err(format!(
                    "instruction {i}: `BindArm` index {index} exceeds the 8-bit field"
                ));
            }
            // op 79: [79][dst][target=ra][index=b-byte]
            vec![w(oc, reg(*dst)?, reg(*target)?, *index as u32 & 0xff)]
        }
        // ── CRDTs, single-replica projection (model ops 80/81/84/82/83/113). A `Counter`/`Tally` lowers to a
        // plain struct; `CrdtBump` accumulates a field (`+= amount`, negated for `Decrease`), `CrdtMerge`
        // sum-joins field-wise, `CrdtJoin` per-replica MAX-joins, `CrdtResolve` overwrites (LWW). The field
        // index is the usage-derived slot; it must fit the 4-bit `b[3:0]` (`CrdtBump`) / 8-bit (`CrdtResolve`).
        Op::CrdtBump {
            obj,
            field,
            amount,
            negate,
        } => {
            let (slot, _n) = resolve_field(i, constants, st, *obj, *field)?;
            if slot > 15 {
                return Err(format!(
                    "instruction {i}: CRDT field slot {slot} exceeds the 4-bit b[3:0] field of `CrdtBump`"
                ));
            }
            // op 80: [80][obj=dst][amount=ra][b: negate(bit4) | field(bits 3:0)]
            let b = ((*negate as u32) << 4) | (slot as u32 & 0xf);
            vec![w(oc, reg(*obj)?, reg(*amount)?, b)]
        }
        // op 81: [81][target=dst][source=ra] — field-wise sum-join.
        Op::CrdtMerge { target, source } => vec![w(oc, reg(*target)?, reg(*source)?, 0)],
        // op 113: [113][target=dst][source=ra] — per-replica MAX join.
        Op::CrdtJoin { target, source } => vec![w(oc, reg(*target)?, reg(*source)?, 0)],
        // op 82: [82][dst] — a fresh empty CRDT collection (seq/set, single-replica projection = empty list).
        Op::NewCrdt { dst, .. } => vec![w(oc, reg(*dst)?, 0, 0)],
        // op 83: [83][seq=dst][value=ra] — RGA append (ordered list append, single-replica projection).
        Op::CrdtAppend { seq, value } => vec![w(oc, reg(*seq)?, reg(*value)?, 0)],
        Op::CrdtResolve { obj, field, value } => {
            let (slot, _n) = resolve_field(i, constants, st, *obj, *field)?;
            if slot > 255 {
                return Err(format!(
                    "instruction {i}: CRDT resolve field slot {slot} exceeds the 8-bit field"
                ));
            }
            // op 84: [84][obj=dst][value=ra][field=rb] — register last-write = field overwrite (LWW).
            vec![w(oc, reg(*obj)?, reg(*value)?, slot as u32)]
        }
        // ── NON-CAPTURING CLOSURES (model ops 40/39). `MakeClosure` stores a closure value = `[entry]` on the
        // heap; `CallValue` does an indirect Call through it. Only the NON-capturing case lowers: a closure
        // that captures locals/globals needs a capture frame the single-word op 40 (`[entry]` only) cannot
        // carry, so a capturing closure is rejected loudly (its `captures` list is non-empty).
        Op::MakeClosure { dst, func, .. } => {
            if captures_nonempty(*func) {
                return Err(format!(
                    "instruction {i}: capturing closure has no full-core chip lowering (the single-word \
                     MakeClosure carries only the entry, not a capture frame)"
                ));
            }
            let entry = entry_of(*func)?;
            if entry > 255 {
                return Err(format!(
                    "instruction {i}: closure entry imem index {entry} exceeds the 8-bit a-byte field of `MakeClosure`"
                ));
            }
            // op 40: [40][dst][func=entry a-byte][0] — non-capturing closure = [entry].
            vec![w(oc, reg(*dst)?, entry, 0)]
        }
        Op::CallValue {
            dst,
            callee,
            args_start,
            arg_count,
            ..
        } => {
            // op 39: [39][dst=result][callee=ra][b: args_start(hi nibble) | arg_count(lo nibble)]. The model
            // clones the caller frame, window-copies R[args_start..+arg_count] → R[0..] (so a compiled
            // closure body reads its param at its base r0), then runs the static body and returns.
            let asr = reg(*args_start)?;
            if asr > 15 {
                return Err(format!(
                    "instruction {i}: `CallValue` args_start r{args_start} exceeds the 4-bit b[7:4] field"
                ));
            }
            if *arg_count > 15 {
                return Err(format!(
                    "instruction {i}: `CallValue` arg_count {arg_count} exceeds the 4-bit b[3:0] field"
                ));
            }
            reg(args_start.saturating_add(arg_count.saturating_sub(1)))?; // top arg register must fit too
            let b = (asr << 4) | (*arg_count as u32 & 0xf);
            vec![w(oc, reg(*dst)?, reg(*callee)?, b)]
        }
        // ── DETERMINISTIC CONCURRENCY (model ops 92/93/94, Spawn 98). The single-node deterministic runtime
        // runs Launch SYNCHRONOUSLY and channels FIFO (the offline semantics proven for the WASM AOT backend);
        // the model mirrors it. `ChanNew` = a fresh ring channel; `ChanSend`/`ChanRecv` enqueue/dequeue FIFO.
        Op::ChanNew { dst, .. } => vec![w(oc, reg(*dst)?, 0, 0)],
        // op 93: [93][chan=dst][val=ra] — enqueue.
        Op::ChanSend { chan, val } => vec![w(oc, reg(*chan)?, reg(*val)?, 0)],
        // op 94: [94][dst][chan=ra] — dequeue (FIFO).
        Op::ChanRecv { dst, chan } => vec![w(oc, reg(*dst)?, reg(*chan)?, 0)],
        // op 98: [98][entry=dst byte][args_start=ra][arg_count=rb] — deterministic sync launch. The model
        // arg-copies R[args_start..]→R[0..] into a fresh frame (like Call) then runs the task to completion,
        // returning to Main via the launched body's `ReturnNothing` — so the channel side-effects persist.
        Op::Spawn {
            func,
            args_start,
            arg_count,
        } => {
            if *arg_count > 255 {
                return Err(format!(
                    "instruction {i}: `Spawn` arg_count {arg_count} exceeds 255"
                ));
            }
            reg(args_start.saturating_add(arg_count.saturating_sub(1)))?;
            let entry = entry_of(*func)?;
            if entry > 255 {
                return Err(format!(
                    "instruction {i}: `Spawn` entry imem index {entry} exceeds the 8-bit entry field"
                ));
            }
            vec![w(oc, entry, reg(*args_start)?, *arg_count as u32 & 0xff)]
        }
        // ── INTEGER POWER (model + RTL op 18). `a ** b` for a non-negative integer exponent: the chip's
        // sequential-multiply power unit (a stall-based FSM in `soft_core.v`, mirroring the shared divider)
        // computes `acc = 1; repeat b times: acc *= a`. The model retires it directly (BigInt-exact via the
        // auto-promoting value-integer multiply) so it is byte-identical to the VM's `int_power`. op 18:
        // [18][dst][lhs=a][rhs=b].
        Op::Pow { dst, lhs, rhs } => vec![w(oc, reg(*dst)?, reg(*lhs)?, reg(*rhs)?)],
        // ── INTEGER-PLANE NUMERIC BUILTIN COPROCESSOR (op 38). The Word32 rotate/shift builtins and the
        // FLOAT-plane numeric builtins were already handled by `word_shape`/`float_shape` above; what reaches
        // here is a numeric builtin on the INTEGER value plane — `abs`/`min`/`max`/`count_ones` (ordinals
        // 5/7/8/9 from `builtin_chip_ordinal`). op 38: [38][dst][args_start=ra][ordinal=rb]; the model reads
        // `regs[args_start]` (and `regs[args_start+1]` for the 2-arg min/max) and computes the value-plane
        // result BigInt-exactly, byte-identical to `call_builtin` (Abs's i64::MIN→2^63 promotion, Min/Max's
        // `regval_cmp`, CountOnes's `(n as u64).count_ones()`). A builtin with no integer-plane ordinal (host
        // parse/format/temporal/uuid/…) falls through to the loud reject — it has no deterministic chip form.
        Op::CallBuiltin {
            dst,
            builtin,
            args_start,
            arg_count,
        } => {
            // The top argument register must fit the file (2-arg builtins read args_start+1).
            reg(args_start.saturating_add(arg_count.saturating_sub(1)))?;
            // FIRST the deterministic string / fill builtins (`chr`=233 / TEXT `contains`=234 / `repeatSeq`
            // =235 / `parseInt`=236 / `text_bytes`=237), each a fresh on-heap value or scanned scalar from
            // register operands — [op][dst][args_start][args_start+1] (`parseInt` and `text_bytes` read only
            // `args_start`, the text handle). The type gate inside `text_builtin_opcode` rejects an
            // unsupported operand shape loudly (a non-Text `contains`, a float/reference `repeatSeq` element,
            // a non-Text `parseInt` / `text_bytes`) rather than mis-lowering it.
            if let Some(t_oc) = text_builtin_opcode(i, *builtin, *args_start, ft)? {
                vec![w(
                    t_oc,
                    reg(*dst)?,
                    reg(*args_start)?,
                    reg(args_start.saturating_add(1))?,
                )]
            } else {
                // ELSE the integer-plane numeric coprocessor (op 38): abs/min/max/count_ones by ordinal. A
                // builtin with no chip form at all (host parse/format/temporal/uuid/…) hits the loud reject.
                let ord = builtin_chip_ordinal(*builtin).ok_or_else(|| {
                    format!(
                        "instruction {i}: builtin `{builtin:?}` has no full-core chip lowering yet — only \
                         abs/min/max/count_ones (integer plane), floor/ceil/round/abs/min/max (float plane), \
                         sqrt, chr/contains/repeatSeq/parseInt/text_bytes, and the Word32 vocabulary lower; \
                         `encode_op` rejects the rest loudly"
                    )
                })?;
                vec![w(oc, reg(*dst)?, reg(*args_start)?, u32::from(ord))]
            }
        }
        // ── TUPLE DESTRUCTURE (model + RTL op 71). `Repeat for (k, v) in …` spreads a heap tuple's payload
        // into `count` consecutive registers. op 71: [71][start=dst][src=ra][count=rb]; the model scatters
        // `heap[src+1+i]` → `regs[start+i]` for `i in 0..count`.
        Op::DestructureTuple { src, start, count } => {
            if *count > 255 {
                return Err(format!(
                    "instruction {i}: `DestructureTuple` count {count} exceeds the 8-bit field"
                ));
            }
            reg(start.saturating_add(count.saturating_sub(1)))?; // the top spread register must fit too
            vec![w(oc, reg(*start)?, reg(*src)?, *count as u32 & 0xff)]
        }
        // ── DETERMINISTIC SCHEDULER (model + RTL). On the single-node deterministic runtime a `Sleep` is a
        // pure no-op (op 88 advances the pc), and the non-blocking channel ops ride the SAME FIFO ring the
        // blocking `ChanSend`/`ChanRecv` (93/94) use — `ChanTrySend` (95) reports success in `dst`,
        // `ChanTryRecv` (96) delivers the value or `0`, `ChanClose` (97) sets a flag no receive path reads.
        Op::Sleep { duration } => vec![w(oc, 0, reg(*duration)?, 0)],
        // op 95: [95][success=dst][chan=ra][val=rb].
        Op::ChanTrySend { dst, chan, val } => vec![w(oc, reg(*dst)?, reg(*chan)?, reg(*val)?)],
        // op 96: [96][value=dst][chan=ra].
        Op::ChanTryRecv { dst, chan } => vec![w(oc, reg(*dst)?, reg(*chan)?, 0)],
        // op 97: [97][.][chan=ra] — output-transparent close.
        Op::ChanClose { chan } => vec![w(oc, 0, reg(*chan)?, 0)],
        // ── HANDLE-BOUND SYNC LAUNCH + JOIN (model + RTL). `Let h be Launch a task to f with …` runs `f`
        // SYNCHRONOUSLY (deterministic offline Launch) and binds a task handle. op 99: [99][handle=dst][
        // entry=a-byte][.] — the model saves the caller frame, sets the handle placeholder, and jumps to
        // the callee (sharing the caller's registers, so the task reads the channel handle the caller placed
        // at its base register). `TaskAwait` (100) is the instant join and `TaskAbort` (101, `Stop h`) a
        // no-op — the deterministic child already ran to completion at the launch point.
        Op::SpawnHandle { dst, func, .. } => {
            let entry = entry_of(*func)?;
            if entry > 255 {
                return Err(format!(
                    "instruction {i}: `SpawnHandle` entry imem index {entry} exceeds the 8-bit a-byte field"
                ));
            }
            vec![w(oc, reg(*dst)?, entry, 0)]
        }
        // op 100: [100][dst][handle=ra] — the sync child already completed, so the join passes through.
        Op::TaskAwait { dst, handle } => vec![w(oc, reg(*dst)?, reg(*handle)?, 0)],
        // op 101: [101][.][handle=ra] — abort a task that already ran to completion (a no-op).
        Op::TaskAbort { handle } => vec![w(oc, 0, reg(*handle)?, 0)],
        // ── SELECT / WAIT-ON-MULTIPLE (model + RTL). `Await the first of:` registers each arm, then waits.
        // op 102: [102][chan=dst][var=ra] — a recv arm; op 103: [103][.][ticks=ra] — a timeout arm (the tick
        // quantity is never observed: the timeout fires only when NO recv arm is ready); op 104: [104][
        // dst_arm] — fire the first ready recv (dequeue FIFO) else the timeout arm, writing the arm index.
        Op::SelectArmRecv { chan, var } => vec![w(oc, reg(*chan)?, reg(*var)?, 0)],
        Op::SelectArmTimeout { ticks } => vec![w(oc, 0, reg(*ticks)?, 0)],
        Op::SelectWait { dst_arm } => vec![w(oc, reg(*dst_arm)?, 0, 0)],
        // ── OFFLINE SINGLE-NODE NETWORKING (model + RTL). On the deterministic single-node chip there is no
        // peer or wire, so — exactly like the VM's documented offline model (tw == VM-net == AOT) — `Connect`
        // (105), `Listen` (106), `Stream` (108) and `Sync` (111) are no-ops, `NetMakePeer` (110) mints a
        // fresh loopback peer key, `NetSend` (107) publishes `(peer key, message)` to the loopback inbox, and
        // `NetAwait` (109) drains the inbox entry keyed by that peer — a local round-trip to itself.
        Op::NetConnect { url } => vec![w(oc, 0, reg(*url)?, 0)],
        Op::NetListen { topic } => vec![w(oc, 0, reg(*topic)?, 0)],
        // op 107: [107][.][to=ra][msg=rb] — publish to the loopback inbox.
        Op::NetSend { to, msg } => vec![w(oc, 0, reg(*to)?, reg(*msg)?)],
        // op 108: [108][.][to=ra][values=rb] — offline stream no-op.
        Op::NetStream { to, values } => vec![w(oc, 0, reg(*to)?, reg(*values)?)],
        // op 109: [109][dst][from=ra] — drain the loopback inbox entry keyed by the peer (a `stream` await
        // drains the same inbox, so the flag needs no distinct lowering on the single node).
        Op::NetAwait { dst, from, .. } => vec![w(oc, reg(*dst)?, reg(*from)?, 0)],
        // op 110: [110][dst][addr=ra] — a fresh offline peer key (the loopback identity).
        Op::NetMakePeer { dst, addr } => vec![w(oc, reg(*dst)?, reg(*addr)?, 0)],
        // op 111: [111][dst][topic=ra] — offline sync no-op (no peer to merge; the variable is unchanged).
        Op::NetSync { dst, topic } => vec![w(oc, reg(*dst)?, reg(*topic)?, 0)],
        other => {
            return Err(format!(
                "`{}` has no full-core chip lowering yet (outside the integer benchmark subset)",
                other.name()
            ))
        }
    })
}

// ── Value-semantics isolation (P1.6) ────────────────────────────────────────────────────────────────
// LOGOS collections are value-typed; the VM preserves this with lazy Rc copy-on-write, but the chip's flat
// heap has no refcount, so an in-place mutator (`ListPush`/`SetIndex`/…) on a list that is SHARED with
// another binding would corrupt the alias (nqueens: `Let mutable next be queens; Push col to next`).
// A forward dataflow tracks, per register, whether it PROVABLY holds a uniquely-owned ("Fresh") collection;
// any mutator whose target is not provably Fresh gets an explicit `DeepClone` isolation emitted before it.
// This is always sound (a mutator target is always a collection, so cloning it can never corrupt a scalar),
// keeps list-building loops O(n) (a `new Seq` is Fresh → in-place push, no clone), and clones only true
// aliases. Function params seed as shared (arg-copy aliases the caller's handle).

#[derive(Clone, Copy, PartialEq, Eq)]
enum Origin {
    Bottom,   // unreached
    Fresh,    // provably a uniquely-owned collection (safe to mutate in place)
    NotFresh, // possibly aliased / scalar / unknown
}

/// What an op does to the origin lattice.
enum Act {
    MakeFresh(u16),  // allocates a fresh collection into dst
    Alias(u16, u16), // dst := src (both handles now shared)
    Mutate(u16),     // in-place mutation of a collection register (isolate if not Fresh)
    Write(u16),      // writes dst with a scalar / possibly-shared value → NotFresh
    Ctrl,            // no register write we track
}

fn classify(op: &Op) -> Act {
    use Op::*;
    match op {
        NewList { dst, .. } | NewTuple { dst, .. } | NewEmptyList { dst } | NewEmptyListI32 { dst }
        | NewEmptyListWithCapacity { dst, .. } | NewEmptyListFixed { dst, .. }
        | NewEmptySet { dst } | NewEmptyMap { dst }
        | NewEmptyMapWithCapacity { dst, .. } | NewRange { dst, .. } | DeepClone { dst, .. }
        | SliceOp { dst, .. } | Concat { dst, .. } | SeqConcat { dst, .. } | UnionOp { dst, .. }
        | IntersectOp { dst, .. } | FormatValue { dst, .. } | Args { dst }
        // A freshly built struct / inductive / CRDT record is uniquely owned — so its OWN construction
        // inserts (`StructInsert`/`CrdtBump`) mutate in place with no spurious clone, and a `Let q be p`
        // aliases it (below), forcing an isolating clone only when an ALIAS is then mutated.
        | NewStruct { dst, .. } | NewInductive { dst, .. } | NewCrdt { dst, .. } => Act::MakeFresh(*dst),
        Move { dst, src } => Act::Alias(*dst, *src),
        ListPush { list, .. } => Act::Mutate(*list),
        SetAdd { set, .. } => Act::Mutate(*set),
        SetIndex { collection, .. } | SetIndexUnchecked { collection, .. } => Act::Mutate(*collection),
        RemoveFrom { collection, .. } => Act::Mutate(*collection),
        ListPop { list, .. } => Act::Mutate(*list),
        StructInsert { obj, .. } => Act::Mutate(*obj),
        // `Push x to obj's field` mutates the FIELD's LIST, not the struct record itself — and the model's
        // op-47 already COPY-ON-WRITEs that field list internally (repointing the field when it is shared),
        // so the correct value semantics hold WITHOUT isolating the struct. Isolating the struct here would
        // be WRONG: it would DeepClone the (compiler-Move-aliased) record and repoint the CLONE's field, so
        // the push would never reach the real `b` (a length-0 read). It is therefore control-transparent to
        // the struct-freshness lattice — the field-list COW is the whole isolation this op needs.
        ListPushField { .. } => Act::Ctrl,
        // Struct/CRDT field mutators: an in-place field write on a (possibly aliased) record.
        CrdtBump { obj, .. } | CrdtResolve { obj, .. } => Act::Mutate(*obj),
        CrdtMerge { target, .. } | CrdtJoin { target, .. } => Act::Mutate(*target),
        CrdtAppend { seq, .. } => Act::Mutate(*seq),
        // dst-writing ops whose result may be a shared nested collection (or a scalar) → NotFresh.
        Add { dst, .. } | Sub { dst, .. } | Mul { dst, .. } | Div { dst, .. } | Mod { dst, .. }
        | ExactDiv { dst, .. } | FloorDiv { dst, .. } | AddAssign { dst, .. }
        | Lt { dst, .. } | Gt { dst, .. } | LtEq { dst, .. } | GtEq { dst, .. } | Eq { dst, .. } | NotEq { dst, .. }
        | BitXor { dst, .. } | BitAnd { dst, .. } | BitOr { dst, .. } | Shl { dst, .. } | Shr { dst, .. } | Not { dst, .. }
        | LoadConst { dst, .. } | Length { dst, .. } | Contains { dst, .. }
        | Index { dst, .. } | IndexUnchecked { dst, .. } | IterNext { dst, .. } | GlobalGet { dst, .. } => Act::Write(*dst),
        _ => Act::Ctrl,
    }
}

/// Code-index successors in the linear bytecode (Main then function bodies). A `Call` returns to `i+1`;
/// the callee body is reached from its own seeded entry, so it is a separate dataflow root.
fn successors(i: usize, op: &Op, n: usize, imem_target: impl Fn(usize) -> usize) -> Vec<usize> {
    let next = if i + 1 < n { vec![i + 1] } else { vec![] };
    match op {
        Op::Jump { target } => vec![imem_target(*target)],
        Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
            let mut s = vec![imem_target(*target)];
            s.extend(next);
            s
        }
        Op::IterNext { exit, .. } => {
            let mut s = vec![imem_target(*exit)];
            s.extend(next);
            s
        }
        Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => vec![],
        _ => next,
    }
}

/// The set of mutator code indices whose target is NOT provably uniquely-owned, each mapped to the
/// register that must be `DeepClone`-isolated before the mutation. Fixpoint forward dataflow.
fn isolation_targets(program: &CompiledProgram) -> std::collections::HashMap<usize, u16> {
    let n = program.code.len();
    const R: usize = 256;
    // Targets already live in CODE-index space (Jump targets, entry_pc). Successors use them directly.
    let tgt = |ci: usize| ci;

    let mut entry: Vec<[Origin; R]> = vec![[Origin::Bottom; R]; n];
    let mut work: std::collections::VecDeque<usize> = Default::default();
    let seed = |entry: &mut Vec<[Origin; R]>,
                work: &mut std::collections::VecDeque<usize>,
                pc: usize,
                params: u16| {
        if pc >= entry.len() {
            return;
        }
        let mut changed = false;
        for r in 0..(params as usize).min(R) {
            if entry[pc][r] != Origin::NotFresh {
                entry[pc][r] = Origin::NotFresh; // a param aliases the caller's handle
                changed = true;
            }
        }
        if entry[pc].iter().all(|&o| o == Origin::Bottom) || changed || params == 0 {
            work.push_back(pc);
        }
    };
    seed(&mut entry, &mut work, 0, 0); // Main
    for f in &program.functions {
        seed(&mut entry, &mut work, f.entry_pc, f.param_count);
    }

    let merge = |a: Origin, b: Origin| match (a, b) {
        (Origin::Bottom, x) | (x, Origin::Bottom) => x,
        (Origin::Fresh, Origin::Fresh) => Origin::Fresh,
        _ => Origin::NotFresh,
    };
    while let Some(i) = work.pop_front() {
        let mut out = entry[i];
        match classify(&program.code[i]) {
            Act::MakeFresh(d) => out[d as usize & 255] = Origin::Fresh,
            Act::Alias(d, s) => {
                out[d as usize & 255] = Origin::NotFresh;
                out[s as usize & 255] = Origin::NotFresh;
            }
            Act::Mutate(t) => out[t as usize & 255] = Origin::Fresh, // unique after this op (isolated or already unique)
            Act::Write(d) => out[d as usize & 255] = Origin::NotFresh,
            Act::Ctrl => {}
        }
        for s in successors(i, &program.code[i], n, tgt) {
            if s >= n {
                continue;
            }
            let mut changed = false;
            for r in 0..R {
                let m = merge(entry[s][r], out[r]);
                if m != entry[s][r] {
                    entry[s][r] = m;
                    changed = true;
                }
            }
            if changed {
                work.push_back(s);
            }
        }
    }

    // Final decision: a mutator whose target is not provably Fresh on entry needs isolation.
    let mut iso = std::collections::HashMap::new();
    for (i, op) in program.code.iter().enumerate() {
        if let Act::Mutate(t) = classify(op) {
            if entry[i][t as usize & 255] != Origin::Fresh {
                iso.insert(i, t);
            }
        }
    }
    iso
}

/// The provably-constant integer value a register holds ENTERING code index `use_idx`, recovered by a
/// backward STRAIGHT-LINE scan: walk `use_idx-1 .. 0` for the first op that WRITES `r`. If it is a
/// `LoadConst { dst: r }` of an integer AND no code index strictly between that def and the use is a branch
/// TARGET (so no other value can flow in on a merged path), the constant is proven; a `Move`-through follows
/// the source register. Any other writer, a hit on a branch target, or a non-constant source → `None`
/// (conservatively unknown — the caller then rejects). Sound: it only trusts a value on a single-entry run.
fn const_reg_value(
    code: &[Op],
    constants: &[Constant],
    is_target: &dyn Fn(usize) -> bool,
    use_idx: usize,
    r: Reg,
    depth: u8,
) -> Option<i64> {
    if depth > 8 {
        return None; // bounded Move-chain following
    }
    let mut j = use_idx;
    while j > 0 {
        j -= 1;
        // A branch target between the def and the use means a different value could reach here — unknown.
        if is_target(j + 1) && (j + 1) < use_idx {
            return None;
        }
        if writes_reg(&code[j], r) {
            return match &code[j] {
                Op::LoadConst { dst, idx } if *dst == r => match constants.get(*idx as usize) {
                    Some(Constant::Int(v)) => Some(*v),
                    Some(Constant::Bool(b)) => Some(i64::from(*b)),
                    _ => None,
                },
                Op::Move { dst, src } if *dst == r => {
                    const_reg_value(code, constants, is_target, j, *src, depth + 1)
                }
                _ => None, // some other op defines `r` — not a known constant
            };
        }
    }
    None
}

/// Does `op` WRITE register `r` (a conservative over-approximation — a windowed writer like
/// `DestructureTuple` counts if `r` is in its written range)? Used by the straight-line const scan to stop at
/// the nearest definition of a register.
fn writes_reg(op: &Op, r: Reg) -> bool {
    op_registers_written(op).contains(&r)
}

/// The register(s) an op writes (its destinations). Conservative — used only by the SliceOp const-fold's
/// backward scan, so an over-broad set only makes the fold DECLINE (reject), never mis-fold.
fn op_registers_written(op: &Op) -> Vec<Reg> {
    use Op::*;
    match op {
        LoadConst { dst, .. } | Move { dst, .. } | Not { dst, .. } => vec![*dst],
        Add { dst, .. }
        | Sub { dst, .. }
        | Mul { dst, .. }
        | Div { dst, .. }
        | Mod { dst, .. }
        | ExactDiv { dst, .. }
        | FloorDiv { dst, .. }
        | Pow { dst, .. }
        | Lt { dst, .. }
        | Gt { dst, .. }
        | LtEq { dst, .. }
        | GtEq { dst, .. }
        | Eq { dst, .. }
        | NotEq { dst, .. }
        | ApproxEq { dst, .. }
        | BitXor { dst, .. }
        | BitAnd { dst, .. }
        | BitOr { dst, .. }
        | Shl { dst, .. }
        | Shr { dst, .. }
        | Index { dst, .. }
        | IndexUnchecked { dst, .. }
        | Length { dst, .. }
        | Contains { dst, .. }
        | GetField { dst, .. }
        | GlobalGet { dst, .. }
        | SliceOp { dst, .. }
        | UnionOp { dst, .. }
        | IntersectOp { dst, .. }
        | CallBuiltin { dst, .. } => vec![*dst],
        AddAssign { dst, .. } => vec![*dst],
        DestructureTuple { start, count, .. } => {
            (0..*count).map(|k| start.saturating_add(k)).collect()
        }
        Call { dst, .. } | CallValue { dst, .. } => vec![*dst],
        RevSwap { a, b } => vec![*a, *b],
        // Everything else either writes a collection register we don't const-track or writes nothing an
        // integer bound would read — return empty so the scan continues past it (never a mis-fold).
        _ => Vec::new(),
    }
}

/// The set of code indices that are BRANCH TARGETS (Jump/branch destinations, IterNext exit targets, and
/// function entry points) — a code index a value can reach on a merged path. The SliceOp const-fold uses
/// this to refuse a fold across a control-flow merge (soundness). Targets are in CODE-index space.
fn branch_targets(program: &CompiledProgram) -> std::collections::HashSet<usize> {
    let mut t = std::collections::HashSet::new();
    for op in &program.code {
        match op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. } => {
                t.insert(*target as usize);
            }
            Op::IterNext { exit, .. } => {
                t.insert(*exit as usize);
            }
            _ => {}
        }
    }
    for f in &program.functions {
        t.insert(f.entry_pc);
    }
    t
}

/// For each `SliceOp` code index, the folded 1-based-inclusive bounds `(start, end)` IFF both bounds are
/// provably small integer constants (`1..=15`, fitting the chip word's b-byte nibbles) — the immediate form
/// the RTL slice unit (`soft_core.v` op 65) decodes. A runtime / out-of-nibble-range bound is ABSENT, so the
/// encoder rejects it loudly (a genuine chip limitation, honestly declined — the op still lowers for the
/// common `items 2 through 5 of xs` constant-bound case, exactly like `FormatValue`'s `.N`-only float form).
fn slice_bound_folds(program: &CompiledProgram) -> std::collections::HashMap<usize, (u8, u8)> {
    let targets = branch_targets(program);
    let is_target = |ci: usize| targets.contains(&ci);
    let mut folds = std::collections::HashMap::new();
    for (i, op) in program.code.iter().enumerate() {
        if let Op::SliceOp { start, end, .. } = op {
            let s = const_reg_value(&program.code, &program.constants, &is_target, i, *start, 0);
            let e = const_reg_value(&program.code, &program.constants, &is_target, i, *end, 0);
            if let (Some(s), Some(e)) = (s, e) {
                if (1..=15).contains(&s) && (1..=15).contains(&e) {
                    folds.insert(i, (s as u8, e as u8));
                }
            }
        }
    }
    folds
}

/// Assemble a whole compiled program (from [`crate::vm::Compiler::compile_for_chip`]) into full soft-core
/// imem words. TWO-PASS: pass 1 lays out Main then every function body, computing each code index's imem
/// word offset (a `Call` and a wide `LoadConst` occupy two words); pass 2 emits the words, resolving every
/// Jump/branch/iter target and callee entry through that pc→imem map. `Err` if the program uses an op with
/// no chip lowering (floats, Text, closures, CRDTs, channels, …) or overflows an operand field.
///
/// The full-core image (`width` = 32). Because the value datapath auto-promotes to BigInt on overflow and
/// over-i32 constants are carried in full by op 222 at every width, the emitted words are IDENTICAL at 32 and
/// 64 — the width no longer changes the encoding. Pass an explicit width to [`encode_soft_core_full_w`].
pub fn encode_soft_core_full(program: &CompiledProgram) -> Result<Vec<u32>, String> {
    encode_soft_core_full_w(program, 32)
}

/// The width-parametric form of [`encode_soft_core_full`]. `width` is retained as an API/experimental knob
/// describing the runtime datapath's fast-path size, but it NO LONGER changes the emitted image: every
/// over-i32 constant is a 3-word `LoadConstWide64` (op 222) carrying the full `i64` at every width, and the
/// value plane auto-promotes to BigInt on overflow exactly like the VM's unbounded `Int`. So the full core
/// reproduces the VM EXACTLY — every constant and every arithmetic result, into the BigInt tail — and does so
/// identically at width 32 and 64 (BigInt handles exactness; the width is a perf/area knob, not a correctness
/// switch). Runs on [`crate::codegen_sva::soft_core_model::run_soft_core_w`] at the matching width. (The
/// distinct 32-bit tape-out core in `scalar_asm.rs` genuinely wraps at 2³² — that is the small-area chip.)
pub fn encode_soft_core_full_w(program: &CompiledProgram, width: u32) -> Result<Vec<u32>, String> {
    // Value-semantics isolation set: mutator code indices → the collection register to DeepClone-isolate
    // first (one extra word each). See `isolation_targets`.
    let iso = isolation_targets(program);
    let deep_clone =
        crate::vm::binisa::opcode_of_name("DeepClone").expect("DeepClone is a registered opcode");

    // The STRUCT-TYPE ORACLE: which registers hold which struct type at each code index, plus the
    // usage-derived positional field layout — drives the (field name → slot) lowering of every struct /
    // CRDT field op (`NewStruct`/`StructInsert`/`GetField`/`CrdtBump`/`CrdtResolve`).
    let st = StructTyping::analyze(program);

    // The TYPE ORACLE: which registers hold f64 / a `Seq of Float` handle / a `Text` handle at each code
    // index — drives the Int-vs-Float-vs-Text lowering of every arithmetic/compare/Show/const op (the integer
    // path is untouched when nothing is float or text). A field read takes the field's declared type, which
    // the struct oracle names.
    let ft = FloatTyping::analyze(program, &st);

    // The SLICE-BOUND FOLDS: for each `SliceOp`, its 1-based-inclusive bounds IFF both are provably small
    // integer constants (1..=15, the chip's immediate nibble form). A runtime/large bound is absent → the
    // encoder rejects it (a genuine hardware limit). Computed ONCE so pass 1 (`imem_width`) and pass 2
    // (`encode_op`) agree on which slices lower and their word count.
    let folds = slice_bound_folds(program);
    let slice_fold = |ci: usize| folds.get(&ci).copied();

    // A non-capturing-closure predicate for `MakeClosure`: only a closure whose `captures` list is EMPTY
    // lowers (the single-word op 40 carries only the entry, no capture frame).
    let captures_nonempty = |func: u16| -> bool {
        program
            .functions
            .get(func as usize)
            .map(|f| !f.captures.is_empty())
            .unwrap_or(false)
    };

    // ── STRING-CONSTANT POOL — the char data of every DISTINCT Text constant, appended AFTER the code as one
    // shared block per string (compact: an N-char string is N imem words, not ~2N NewEmptyList+push ops).
    // A `TextConst` copies its block into a fresh heap char-list. Distinct strings are deduped by content
    // (two `""`/two `" "` constants share one block). The pool base (imem index of the first block) is the
    // code word count `total`, resolved after pass 1 sizes the code. `pool_index[const_idx]` maps a Text
    // constant to its distinct-block ordinal; `pool_blocks[ordinal]` is that block's char codepoints.
    let mut pool_blocks: Vec<Vec<i32>> = Vec::new();
    let mut pool_by_content: std::collections::HashMap<&str, usize> =
        std::collections::HashMap::new();
    let mut pool_index: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (ci, c) in program.constants.iter().enumerate() {
        if let Constant::Text(s) = c {
            let ord = *pool_by_content.entry(s.as_str()).or_insert_with(|| {
                let ord = pool_blocks.len();
                // The char-list stores i32 CODE POINTS (one per `char`), matching the model + the VM's char
                // granularity for `item i of` / `length` (ASCII: codepoint == byte, char count == byte length).
                pool_blocks.push(s.chars().map(|ch| ch as i32).collect());
                ord
            });
            pool_index.insert(ci as u32, ord);
        }
    }
    // A failure may need Text no constant spells: the Bool renderings and the refusals.
    if program.code.iter().any(|op| {
        matches!(
            op,
            Op::CallBuiltin {
                builtin: crate::semantics::builtins::BuiltinId::Fail
                    | crate::semantics::builtins::BuiltinId::FailReason
                    | crate::semantics::builtins::BuiltinId::Format,
                ..
            }
        )
    }) {
        for &(key, text) in FAILURE_POOL {
            let ord = *pool_by_content.entry(text).or_insert_with(|| {
                pool_blocks.push(text.chars().map(|ch| ch as i32).collect());
                pool_blocks.len() - 1
            });
            pool_index.insert(key, ord);
        }
    }
    // `args()` needs the program name `logos` in the pool even when no Text constant spells it.
    if program.code.iter().any(|op| matches!(op, Op::Args { .. })) {
        let ord = *pool_by_content
            .entry(crate::semantics::PROGRAM_NAME)
            .or_insert_with(|| {
                pool_blocks.push(
                    crate::semantics::PROGRAM_NAME
                        .chars()
                        .map(|ch| ch as i32)
                        .collect(),
                );
                pool_blocks.len() - 1
            });
        pool_index.insert(ARGV0_POOL_KEY, ord);
    }

    // A SCRATCH register for the FormatValue-then-Concat lowering of a `Text + Int` interpolation: a
    // program-wide-UNUSED register (max index any op touches + 1), so writing it can never clobber a live
    // value. It is dead the instant the Concat consumes it, so no liveness tracking is needed. Only computed
    // (and required to exist) when a Text interpolation is actually emitted.
    let scratch: Reg = {
        let mut hi = 0u16;
        for op in &program.code {
            for r in op_registers(op) {
                hi = hi.max(r);
            }
        }
        for f in &program.functions {
            hi = hi.max((f.register_count.min(256).saturating_sub(1)) as u16);
        }
        hi
    };
    let scratch = scratch
        .checked_add(1)
        .filter(|&s| s <= 255)
        .ok_or_else(|| {
            "no free scratch register for a Text interpolation (the program uses all 256 registers)"
                .to_string()
        })?;

    // Pass 1 — pc (code index) → imem word index.
    let mut pc_to_imem: Vec<usize> = Vec::with_capacity(program.code.len());
    let mut cursor = 0usize;
    for (i, op) in program.code.iter().enumerate() {
        pc_to_imem.push(cursor);
        cursor += imem_width(i, op, &program.constants, &ft, &slice_fold, scratch)
            .map_err(|e| format!("instruction {i}: {e}"))?
            + usize::from(iso.contains_key(&i));
    }
    let total = cursor; // the pool base: string blocks are laid out starting here, after the code.

    // Function entry (FuncIdx → imem word index) and branch-target (code index → imem word index) resolvers.
    let entry_of = |func: u16| -> Result<u32, String> {
        let f = program
            .functions
            .get(func as usize)
            .ok_or_else(|| format!("Call references missing function #{func}"))?;
        let imem = *pc_to_imem
            .get(f.entry_pc)
            .ok_or_else(|| format!("function #{func} entry_pc {} out of range", f.entry_pc))?;
        Ok(imem as u32)
    };
    let imem_of = |code_idx: usize| -> Result<u32, String> {
        let imem = *pc_to_imem
            .get(code_idx)
            .ok_or_else(|| format!("branch target code index {code_idx} out of range"))?;
        if imem > 0xffff {
            return Err(format!(
                "branch target imem index {imem} exceeds the 16-bit target field"
            ));
        }
        Ok(imem as u32)
    };
    // Each distinct string block's (imem word offset of its first char, char length), laid out consecutively
    // after the code. Resolves a Text constant (`TextConst`'s pool operand) to where its chars live in imem.
    let mut block_offset: Vec<u32> = Vec::with_capacity(pool_blocks.len());
    {
        let mut at = total;
        for b in &pool_blocks {
            block_offset.push(at as u32);
            at += b.len();
        }
    }
    let pool_of = |const_idx: u32| -> Result<(u32, u32), String> {
        let ord = *pool_index
            .get(&const_idx)
            .ok_or_else(|| format!("Text constant #{const_idx} missing from the string pool"))?;
        let off = block_offset[ord];
        let len = pool_blocks[ord].len() as u32;
        Ok((off, len))
    };

    // Pass 2 — emit the code.
    let mut imem: Vec<u32> =
        Vec::with_capacity(total + pool_blocks.iter().map(|b| b.len()).sum::<usize>());
    for (i, op) in program.code.iter().enumerate() {
        // Value-semantics barrier: isolate an aliased collection into a uniquely-owned copy before the
        // in-place mutation, so the chip's flat heap preserves LOGOS's value semantics (the VM's Rc-COW).
        if let Some(&target) = iso.get(&i) {
            imem.push(w(deep_clone, reg(target)?, reg(target)?, 0)); // DeepClone(target, target) — isolate in place
        }
        let is_float_iter = |ci: usize| is_float_iternext(ci, program, &ft);
        let words = encode_op(
            i,
            op,
            &program.constants,
            &ft,
            &st,
            &entry_of,
            &imem_of,
            &pool_of,
            &captures_nonempty,
            &is_float_iter,
            &slice_fold,
            scratch,
        )
        .map_err(|e| format!("instruction {i}: {e}"))?;
        debug_assert_eq!(
            words.len() + usize::from(iso.contains_key(&i)),
            imem_width(i, op, &program.constants, &ft, &slice_fold, scratch).unwrap()
                + usize::from(iso.contains_key(&i)),
            "imem_width disagrees with encode_op for {}",
            op.name()
        );
        imem.extend(words);
    }
    debug_assert_eq!(
        imem.len(),
        total,
        "pass-2 code size disagrees with pass-1 layout"
    );
    // Append the string-constant pool AFTER the code — one word per char codepoint, blocks in the same order
    // `block_offset` recorded. `TextConst` copies from `imem[offset..offset+len]`.
    for b in &pool_blocks {
        for &ch in b {
            imem.push(ch as u32);
        }
    }
    Ok(imem)
}

/// The register indices an op reads or writes — a conservative upper set used only to pick a program-wide
/// unused SCRATCH register for the Text-interpolation FormatValue temp. Covers the operand + windowed forms
/// (a `Call`/`NewList` touches its whole `[start .. start+count)` window).
fn op_registers(op: &Op) -> Vec<u16> {
    use Op::*;
    let mut v = Vec::new();
    let mut push = |r: u16| v.push(r);
    match op {
        Add { dst, lhs, rhs }
        | Sub { dst, lhs, rhs }
        | Mul { dst, lhs, rhs }
        | Div { dst, lhs, rhs }
        | Mod { dst, lhs, rhs }
        | ExactDiv { dst, lhs, rhs }
        | FloorDiv { dst, lhs, rhs }
        | Lt { dst, lhs, rhs }
        | Gt { dst, lhs, rhs }
        | LtEq { dst, lhs, rhs }
        | GtEq { dst, lhs, rhs }
        | Eq { dst, lhs, rhs }
        | NotEq { dst, lhs, rhs }
        | ApproxEq { dst, lhs, rhs }
        | BitXor { dst, lhs, rhs }
        | BitAnd { dst, lhs, rhs }
        | BitOr { dst, lhs, rhs }
        | Shl { dst, lhs, rhs }
        | Shr { dst, lhs, rhs } => {
            push(*dst);
            push(*lhs);
            push(*rhs);
        }
        Index {
            dst,
            collection,
            index,
        }
        | IndexUnchecked {
            dst,
            collection,
            index,
        } => {
            push(*dst);
            push(*collection);
            push(*index);
        }
        SetIndex {
            collection,
            index,
            value,
        }
        | SetIndexUnchecked {
            collection,
            index,
            value,
        } => {
            push(*collection);
            push(*index);
            push(*value);
        }
        NewRange { dst, start, end } => {
            push(*dst);
            push(*start);
            push(*end);
        }
        Not { dst, src }
        | Move { dst, src }
        | Length {
            dst,
            collection: src,
        }
        | ListPop { list: src, dst }
        | AddAssign { dst, src } => {
            push(*dst);
            push(*src);
        }
        Show { src } => push(*src),
        Contains {
            dst,
            collection,
            value,
        } => {
            push(*dst);
            push(*collection);
            push(*value);
        }
        SeqConcat { dst, lhs, rhs } => {
            push(*dst);
            push(*lhs);
            push(*rhs);
        }
        DeepClone { dst, src } => {
            push(*dst);
            push(*src);
        }
        ListPush { list, value } | SetAdd { set: list, value } => {
            push(*list);
            push(*value);
        }
        LoadConst { dst, .. }
        | NewEmptyList { dst }
        | NewEmptyListFixed { dst, .. }
        | NewEmptyListI32 { dst }
        | NewEmptySet { dst }
        | NewEmptyMap { dst }
        | GlobalGet { dst, .. }
        | IterNext { dst, .. } => push(*dst),
        NewEmptyListWithCapacity { dst, capacity } | NewEmptyMapWithCapacity { dst, capacity } => {
            push(*dst);
            push(*capacity);
        }
        GlobalSet { src, .. } | IterPrepare { iterable: src } | Return { src } => push(*src),
        Call {
            dst,
            args_start,
            arg_count,
            ..
        } => {
            push(*dst);
            for k in 0..*arg_count {
                push(args_start.saturating_add(k));
            }
        }
        NewList { dst, start, count } | NewTuple { dst, start, count } => {
            push(*dst);
            for k in 0..*count {
                push(start.saturating_add(k));
            }
        }
        CallBuiltin {
            dst,
            args_start,
            arg_count,
            ..
        } => {
            push(*dst);
            for k in 0..*arg_count {
                push(args_start.saturating_add(k));
            }
        }
        JumpIfFalse { cond, .. } | JumpIfTrue { cond, .. } => push(*cond),
        _ => {}
    }
    v
}

/// Render imem words as a `.hex` file body (one 8-hex-digit word per line).
pub fn to_hex(words: &[u32]) -> String {
    let mut s = String::with_capacity(words.len() * 9);
    for x in words {
        s.push_str(&format!("{x:08x}\n"));
    }
    s
}

/// Compile a `.lg` source all the way to full soft-core imem words, through the chip compile path
/// (`compile_for_chip` — plain Div/Mod, no magic reciprocals). `Err` if it doesn't parse/compile or
/// uses anything outside the chip's integer subset. Targets the 32-bit datapath (the tape-out core); for
/// a 64-bit datapath (VM parity on 64-bit-range arithmetic) use [`compile_to_full_words_w`].
pub fn compile_to_full_words(source: &str) -> Result<Vec<u32>, String> {
    compile_to_full_words_w(source, 32)
}

/// The WIDTH-PARAMETRIC form of [`compile_to_full_words`]: compile `.lg` source to full soft-core imem
/// words for a datapath of `width` bits (`32` = the tape-out core, byte-identical to the historical output;
/// `64` = full-i64 VM parity — a constant that exceeds i32, like the benchmark PRNGs' `% 2147483648` /
/// `% 4294967296` moduli, is emitted as a 3-word `LoadConstWide64` instead of being truncated). Pair with
/// [`crate::codegen_sva::soft_core_model::run_soft_core_w`] at the SAME width.
pub fn compile_to_full_words_w(source: &str, width: u32) -> Result<Vec<u32>, String> {
    if let Some(refusal) = crate::compile::exact_refusal(source, "soft core", "the soft core") {
        return Err(refusal);
    }
    // PRUNE the auto-imported stdlib prelude's DEAD functions before the chip encode: `apply_prelude` splices
    // a WHOLE module when the program mentions ANY of its names, but the chip encoder — like the flat C
    // namespace — must LOWER every op in the image, so an UNCALLED prelude helper that reaches a host builtin
    // (`parseFloat`, `format`, …) would make the whole program reject even though it never runs. `prune_
    // unreachable_prelude` drops exactly those dead PRELUDE helpers (never a user function, so LOGOS's
    // non-`name(` call forms like `Launch a task to worker` are always kept), so the encoder sees only the ops
    // the program executes — output-identical to the VM (the pruned functions were dead). `## NoPrelude` stops
    // `parse_seam` re-splicing the full module.
    let spliced = crate::loader::apply_prelude(source);
    let pruned = crate::loader::prune_unreachable_prelude_from(source, spliced.as_ref());
    let shaken = format!("## NoPrelude\n{pruned}");
    crate::ui_bridge::with_parsed_program(&shaken, |parsed, interner| {
        let (stmts, types, _policies) = parsed.map_err(|advice| format!("{advice}"))?;
        let oracle = crate::optimize::oracle_analyze_with(stmts, interner);
        let program =
            crate::vm::Compiler::compile_for_chip(stmts, interner, Some(types), Some(oracle))?;
        crate::compile::refuse_unlinkable(&program, interner, crate::compile::Backend::SoftCore)?;
        encode_soft_core_full_w(&program, width)
    })
}
