//! ════════════════════════════════════════════════════════════════════════════════════════════
//! WASM AOT BACKEND LOCK — the direct `compile_to_wasm` backend must reach FULL VM/tree-walker
//! parity, and no VM feature may be silently skipped on the way there. This is the WebAssembly
//! analog of the Futamura PE == VM == Tree-walker locks: WASM == VM == Tree-walker.
//!
//! TWO complementary locks, mirroring the Futamura coverage design:
//!
//!   • STATIC, CATALOG-COMPLETE ([`op_support`]). Every bytecode `Op` variant is classified in an
//!     explicit, target-aware support bucket. The classification is an EXHAUSTIVE `match` with NO
//!     wildcard arm, so the COMPILER refuses to build this file the moment a new `Op` is added to
//!     the VM until its WebAssembly contract is stated. That is the "no feature silently escapes"
//!     guarantee — the dual of the Futamura `behavioural ∪ excluded == catalog` guard, enforced by
//!     the type system.
//!
//!   • BEHAVIOURAL, END-TO-END ([`wasm_equals_vm_and_treewalker_over_the_corpus`]). For every
//!     corpus program: tree-walker == VM (the base equivalence), and then the AOT module is held
//!     to a BICONDITIONAL — it compiles IFF every op it uses is `Supported`, and when it compiles
//!     its output equals the tree-walker byte-for-byte. So the backend can neither (a) miscompile,
//!     (b) lower a `Deferred` op (that desyncs `op_support` from the real lowering — a RED), nor
//!     (c) REJECT an all-`Supported` program (a coverage GAP to fix in the backend, never a thing
//!     to quietly defer — also a RED).
//!
//! THE GOAL is `Deferred == ∅`: every VM feature has a finished, explicit WebAssembly-target
//! contract. WebAssembly-capable ops work through the backend; capability-incompatible ops live in
//! a small exact target census. The supported-op ratchet ([`supported_op_count_never_regresses`])
//! makes executable coverage strictly monotone — it can only grow, never shrink.
//!
//!  ⚠️  YOU DO NOT GET TO WEAKEN THIS FILE TO MAKE A RED CASE PASS.  ⚠️
//!  A RED means the WASM backend dropped/ miscompiled a feature, or a new op slipped past the
//!  catalog. The fix is in the BACKEND (`vm/wasm/{module,kind,func}.rs`), NEVER by relaxing an
//!  assertion, moving an op from `Supported` to `Deferred`, or adding a wildcard arm.

#![cfg(all(feature = "wasm-jit", not(target_arch = "wasm32")))]

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::builtins::BuiltinId;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::{Compiler, Op};

#[path = "compile_common/mod.rs"]
mod common;

/// How the AOT WebAssembly backend handles an op. Five PRECISE categories make the completeness
/// claim PROVABLE, not asserted: `Deferred` is genuine remaining WORK, and its census is now EMPTY.
/// Every op is `Supported` (self-contained), `Linked` (works in a linked module), `NativeOnly`
/// (requires a capability this WebAssembly target cannot provide), or `Unreachable` (a dead op no
/// source syntax emits). None of those four categories is a TODO.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Support {
    /// Lowered + exercised end-to-end in a SELF-CONTAINED module (no linker).
    Supported,
    /// Lowered + PROVEN in a LINKED module (needs the `logicaffeine_base`/marshal runtime the linker
    /// embeds — a self-contained module has nowhere to hold the exact/handle/boxed value). Not a gap:
    /// see the `linked_*_matches_the_vm` differential tests. The string names the value family.
    Linked(&'static str),
    /// Implemented on the native execution tiers, but semantically requires an OS capability that the
    /// standalone/browser WebAssembly target does not expose. This is target-specific, not unfinished.
    NativeOnly(&'static str),
    /// A DEAD bytecode op no source syntax emits (the compiler never generates it — `Close`/`Await`
    /// route to `Select`/`Net`/other ops). The emitter lowers it anyway (so the backend is complete
    /// over every `Op`), but it cannot be exercised. Not a gap — the language cannot produce it.
    Unreachable(&'static str),
    /// GENUINE deferred WORK — not lowered yet. This census is EMPTY (see the census test).
    Deferred(&'static str),
}

/// ★ STATIC, CATALOG-COMPLETE LOCK ★ — the single source of truth for which VM bytecode ops the
/// AOT backend handles. EXHAUSTIVE on `Op` (no `_` arm): adding a VM instruction fails to compile
/// this file until it is classified, so no feature can be silently skipped. Keep this in lockstep
/// with the lowering in `vm/wasm/module.rs` — the behavioural lock proves they agree.
fn op_support(op: &Op) -> Support {
    use Support::{Deferred as D, Linked as L, NativeOnly as N, Supported as S, Unreachable as U};
    let _ = D; // kept for classifying any FUTURE genuine deferral; the census is currently empty.
    match op {
        // ── P0: whole-program scalar compute + calls + print ──────────────────────────────
        Op::LoadConst { .. } => S,
        Op::Move { .. } => S,
        // The zero-garbage reversible register exchange. `Swap a and b` on two same-typed locals emits
        // it (the compiler's `Stmt::Swap` arm), and the AOT lowers it self-contained to a scratch-free
        // wasm local swap (`get b; get a; set b; set a`) — exercised by the `swap` corpus program, so
        // WASM == VM == Tree-walker.
        Op::RevSwap { .. } => S,
        Op::RevXor { .. } => U("no source syntax emits `RevXor` yet (reverse-execution primitive)"),
        Op::RevNeg { .. } | Op::RevAdd { .. } | Op::RevSub { .. } => {
            U("no source syntax emits the reversible additive ops yet (reverse-execution primitives)")
        }
        // Interpreter-only value-semantics COW barrier; a no-op in the WASM AOT
        // (which copies-on-write at each element write), so self-contained.
        Op::EnsureOwned { .. } => S,
        Op::Add { .. } | Op::AddAssign { .. } | Op::Sub { .. } | Op::Mul { .. } => S,
        Op::Div { .. } | Op::Mod { .. } => S,
        // `a // b` — floor division toward negative infinity: `i64.div_s` corrected by one on a
        // sign-crossing nonzero remainder (`Int`), `f64.floor(a/b)` (`Float`), unsigned `div_u`
        // (`Word`). Traps on `/0` and the `i64::MIN // -1` overflow, like `Div`.
        Op::FloorDiv { .. } => S,
        // `a ** b` — Float results use the host `pow_ff`/`pow_fi`; `Int^Int` is the in-module
        // overflow-trapping squaring loop (the BigInt-promoting frontier traps, like `Mul`).
        Op::Pow { .. } => S,
        Op::BitXor { .. } | Op::BitAnd { .. } | Op::BitOr { .. } | Op::Shl { .. } | Op::Shr { .. } => S,
        Op::Lt { .. } | Op::Gt { .. } | Op::LtEq { .. } | Op::GtEq { .. } => S,
        Op::Eq { .. } | Op::NotEq { .. } => S,
        // `is approximately` lowers to pure f64 instructions (the shared isclose).
        Op::ApproxEq { .. } => S,
        Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => S,
        Op::Not { .. } => S,
        Op::Call { .. } => S,
        Op::Show { .. } => S,
        Op::GlobalGet { .. } | Op::GlobalSet { .. } => S,
        Op::Return { .. } | Op::ReturnNothing | Op::Halt => S,

        // ── P1: scalar completeness ───────────────────────────────────────────────────────
        // The Oracle's division forms — `lhs / 2^k` (`DivPow2`) and `lhs / c` / `lhs % c` by the
        // magic reciprocal (`MagicDivU`). The AOT compiles with the same Oracle the VM uses, so it
        // receives these and lowers them (DivPow2 = plain `i64.div_s` by `1<<k`; MagicDivU mirrors
        // the VM's `magic_eval` Granlund–Montgomery mul-high+shift bit-for-bit).
        Op::DivPow2 { .. } | Op::MagicDivU { .. } => S,
        // `args()` lowers to the `env.args` host import (a `Seq of Text` argv handle the host builds in
        // linear memory); see `wasm_aot_args.rs`. `parseInt` is the `env.parse_int` host import.
        Op::Args { .. } => S,
        // A runtime failure lowers to a wasm trap (the standalone module has no VM to surface the
        // message). The `wasm_traps_where_treewalker_errors` lock proves tw-errors ⟺ wasm-traps.
        Op::FailWith { .. } => S,
        // Numeric builtins are lowered bit-exactly (Int abs's i64::MIN trap, Round's round-half-
        // away, Min/Max's NaN guards, Pow's host `pow_ff`/`pow_fi` for Float results + an integer
        // squaring loop for Int^Int). The string/list builtins land with the heap value model.
        Op::CallBuiltin { builtin, .. } => match builtin {
            BuiltinId::Sqrt
            | BuiltinId::Floor
            | BuiltinId::Ceil
            | BuiltinId::Round
            | BuiltinId::Abs
            | BuiltinId::Min
            | BuiltinId::Max
            | BuiltinId::Pow => S,
            // `parseInt(text)` (host `parse_int`) and `format(x)` (`x.to_display_string()` via the
            // scalar/collection formatters) both lower self-contained.
            BuiltinId::ParseInt | BuiltinId::Format => S,
            // `Fail with <value>.` and a broken contract's `, otherwise <reason>`: the reason handed to the
            // `logos_rt_fail` host as Text, then a trap, the report every other tier gives. The check the
            // seam puts before a pop that may meet an empty list is one.
            BuiltinId::Fail | BuiltinId::FailReason => S,
            // `contains(haystack, needle) -> Bool` — host `text_contains` (two Text handles → i64 0/1).
            BuiltinId::Contains => S,
            // `count_ones(n)` → `i64.popcnt` (Int → Int); `parseFloat(text)` → the `parse_float` host
            // (Text → Float). Both self-contained scalar builtins.
            BuiltinId::CountOnes | BuiltinId::ParseFloat => S,
            // `leading_zeros(n)` → `i64.clz`, `trailing_zeros(n)` → `i64.ctz` (Int → Int).
            BuiltinId::LeadingZeros | BuiltinId::TrailingZeros => S,
            // `chr(code) -> Text` — a one-character Text built inline (UTF-8 encode into a fresh Text
            // object), trapping on an invalid code point. Self-contained (heap, no host).
            BuiltinId::Chr => S,
            // `repeatSeq(x, n)` — a fresh `n`-element sequence of the scalar `x` (`[x] * n` / `n
            // copies of x`): a bump-allocated header + `n*8` buffer filled in a runtime loop. Scalar
            // element kinds (Int/Float); a reference element (deep-copy) defers to the VM.
            BuiltinId::RepeatSeq => S,
            // The Word ring (ℤ/2³²/ℤ/2⁶⁴, the crypto substrate): construct/extract (`word32`/`word64`/
            // `intOfWord32`/`intOfWord64`), rotate (`rotl`/`rotr`), bitwise (`word_and`/`word_or`/
            // `word_not`), the `word64` shift/mask primitives, and `word32Shr` (SHA-256's logical
            // right-shift, `i32.shr_u`) — all native wasm `i32`/`i64` ops.
            BuiltinId::Word32
            | BuiltinId::Word64
            | BuiltinId::IntOfWord32
            | BuiltinId::IntOfWord64
            | BuiltinId::Rotl
            | BuiltinId::Rotr
            | BuiltinId::Wand
            | BuiltinId::Wor
            | BuiltinId::Wnot
            | BuiltinId::Word64Shl
            | BuiltinId::Word64Shr
            | BuiltinId::Word32Shr
            | BuiltinId::Word64And => S,
            // `parse_timestamp(text) -> Moment` (the `parse_timestamp` host) + the calendar/clock
            // component extractors (`the year of m`, …): a `Moment` arg goes through `temporal_component`
            // (nanos), a `Date` arg through `temporal_component_date` (days). Each is computed by the SAME
            // `logicaffeine_base::temporal` the VM uses, so bit-identical. (hour/minute/second of a Date
            // has no meaning — the VM errors, the AOT refuses that op combination at lowering.)
            BuiltinId::ParseTimestamp
            | BuiltinId::YearOf
            | BuiltinId::MonthOf
            | BuiltinId::DayOf
            | BuiltinId::WeekdayOf
            | BuiltinId::HourOf
            | BuiltinId::MinuteOf
            | BuiltinId::SecondOf
            | BuiltinId::WeekOf
            | BuiltinId::QuarterOf => S,
            // Moment arithmetic + calendar/clock extraction — SELF-CONTAINED inline i64/i32 matching the
            // VM exactly: `seconds_between` ((b-a)/1e9), `add_seconds` (m + n·1e9 → Moment), `date_of`
            // (floor-div NANOS_PER_DAY → Date), `time_of` (euclidean remainder → Time). The floor/euclid
            // correction is open-coded branchlessly, so pre-epoch (negative-nanos) Moments agree too.
            BuiltinId::SecondsBetween
            | BuiltinId::AddSeconds
            | BuiltinId::DateOf
            | BuiltinId::TimeOf => S,
            // Byte interop: `text_bytes` (UTF-8 bytes → `Seq of Int`) + `text_from_bytes` (back to `Text`)
            // are self-contained — the emitter builds the seq/Text in linear memory, no runtime. (The
            // `uuid_bytes`/`uuid_from_bytes` pair is LINKER-only, since a `Uuid` handle is, so they stay
            // deferred in the self-contained catalog below.)
            BuiltinId::TextBytes | BuiltinId::TextFromBytes => S,
            // `copy(x)` — the builtin deep clone (an independent heap copy / the value for a scalar),
            // the same self-contained lowering as `Op::DeepClone`.
            BuiltinId::Copy => S,
            // `writeWireResidual(text) -> Int` — the residual-EMIT half of the wire protocol: the Text's
            // bytes are framed to the host wire sink (`write_wire_residual`) and the byte count returned.
            // A pure Text→host-sink op (no marshal, no dynamic value), so self-contained (both modes).
            BuiltinId::WriteWireResidual => S,
            // Everything else is either (a) LINKER-MODE (the numeric tower — `decimal`/`complex`/`modular`/
            // `money`/`quantity`; the whole `uuid` family incl. `uuid_bytes`/`uuid_from_bytes`; the general
            // `LanesV` SIMD vocabulary + the SHA-1 `Lanes4Word32` ops; zoned/calendar temporal —
            // `format_timestamp`/`months_between`/`years_between`/`in_zone`/`local_instant`; money FX —
            // `set_rate`/`set_rates`/`to_currency` over `base::money`'s ambient rate table): ALL COMPILE +
            // run bit-identically in a LINKED module, but a self-contained module has no runtime to hold
            // the exact/handle value, so they are declined here. Or (b) the ONE genuinely ARCHITECTURAL
            // deferral: everything here is LINKER-MODE and WORKING — the numeric tower / uuid / lanes /
            // zoned+calendar temporal / money FX, AND the WHOLE wire subsystem: `wireBytes` (marshal a value
            // via the REAL `encode_value_raw`), `readWireProgram` (`decode_value_raw` → `Kind::Dynamic`, a
            // boxed `RuntimeValue`), `run_accepted` (`AcceptanceContract::apply` sandbox-eval of a wire-
            // received shipped `GenExpr` function), all compiled to wasm32 + linked into the runtime and
            // proven end-to-end (`linked_wire_bytes_*`, `linked_read_wire_program_and_run_accepted_*`);
            // `writeWireResidual` is S (a host sink, above). A self-contained module just has no runtime to
            // hold the exact/handle/boxed value, so these are declined HERE — never a miscompile.
            _ => L("linker-mode value: numeric tower / uuid / lanes / zoned+calendar temporal / money FX / the whole wire subsystem (wireBytes + readWireProgram (Kind::Dynamic) + run_accepted, via the real marshal/decode/apply codec linked into wasm) — all WORKING in a linked module; a self-contained module has no runtime to hold them"),
        },
        Op::LoadToday { .. } | Op::LoadNow { .. } => S,

        // ── P2: heap value model (the linker phase) ───────────────────────────────────────
        // `a / b` in a `Rational` context — the AOT builds an exact reduced `Rational` value
        // (`[num][den]` in linear memory, gcd-reduced) and `Show`s it as `num/den` (or `num` when
        // whole, matching the VM's downsize). Self-contained (bounded i64 num/den; a value that would
        // overflow i64 — the BigInt-Rational case — is the runtime-linked frontier).
        Op::ExactDiv { .. } => S,
        // Text concatenation of two strings lowers to a byte concat; a formatted interpolation piece
        // (`"{x:.9}"`, FormatValue) renders its value with the `.N` precision spec via a host.
        Op::Concat { .. } => S,
        Op::FormatValue { .. } => S,
        // Heap value model (P2): linear memory + bump allocator. Landing incrementally.
        Op::NewEmptyList { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::Length { .. }
        | Op::ListPush { .. }
        | Op::Index { .. }
        | Op::SetIndex { .. }
        | Op::NewList { .. }
        | Op::NewRange { .. }
        | Op::Contains { .. }
        | Op::SliceOp { .. }
        // `Pop from seq into x` (remove + return the last element) is the in-place mirror of
        // `ListPush`: load `data_ptr[len-1]` at the element width, then decrement the header `len`.
        | Op::ListPop { .. }
        | Op::SeqConcat { .. } => S,
        // `NewEmptyListI32` (the optimizer's i32-narrowed empty list) is observably identical to
        // `NewEmptyList` and lowered the same — the i32-fit is a compaction hint, not a semantic change.
        Op::NewEmptyListI32 { .. } => S,
        // `Push x to obj's field` (`ListPushField`, a direct struct-field-seq push) and the Oracle's
        // bounds-check-elimination forms. `IndexUnchecked`/`SetIndexUnchecked` lower identically to the
        // checked `Index`/`SetIndex` (the AOT keeps its bounds check — a safe superset); `RegionBoundsGuard`
        // is a no-op (native-region metadata); `ListPushField` resolves the field slot and pushes.
        Op::ListPushField { .. }
        | Op::IndexUnchecked { .. }
        | Op::SetIndexUnchecked { .. }
        | Op::RegionBoundsGuard { .. } => S,
        Op::NewEmptySet { .. } | Op::SetAdd { .. } => S,
        // `Remove v from s` (swap-remove) lowers for a Set or Map.
        Op::RemoveFrom { .. } => S,
        // `a union b` / `a intersection b` build a fresh combined Set by linear scan + dedup —
        // order-independent, so byte-identical to the VM's Set (a `Vec` with insertion order).
        Op::UnionOp { .. } | Op::IntersectOp { .. } => S,
        Op::NewEmptyMap { .. } | Op::NewEmptyMapWithCapacity { .. } => S,
        Op::DeepClone { .. } => S,
        // A homogeneous tuple `(a, b, …)` lays out identically to a list (a buffer of the shared
        // element kind), so construction (`NewTuple`) + `item N of t` / `t[N]` / `length of t` reuse
        // the sequence machinery. A HETEROGENEOUS tuple (mixed kinds) is a buffer of 8-byte slots each
        // at its own kind (`lower_new_tuple_het`); a constant `item N of t` loads the position's kind.
        // Supported as a local, a parameter (`BoundaryType::Tuple`), a call return, and a struct field.
        Op::NewTuple { .. } => S,
        // `Let (a, b) be …` / `Repeat for (a, b) in pairs:` — bind each destructured register to the
        // source tuple's positional slot (`lower_destructure_tuple`, kinds from `tuple_layouts` or the
        // homogeneous seq element).
        Op::DestructureTuple { .. } => S,
        Op::IterPrepare { .. } | Op::IterNext { .. } | Op::IterPop => S,

        // ── P3: closures + indirect calls ─────────────────────────────────────────────────
        // No-capture closures: `MakeClosure` builds a heap object holding the body's function
        // index; `CallValue` loads it and `call_indirect`s through the function table. (Capturing
        // closures — those that reference enclosing locals — stay deferred until the capture ABI.)
        Op::CallValue { .. } | Op::MakeClosure { .. } => S,

        // ── P4: structs / enums / inspect / temporal / CRDT ───────────────────────────────
        Op::NewStruct { .. } | Op::StructInsert { .. } | Op::GetField { .. } => S,
        // Enum constructors (tag = constructor const index, compared with i32.eq by `TestArm`),
        // including payload-carrying ones (arguments laid out in 8-byte slots after the tag), and
        // `BindArm` (extract the matched variant's `index`-th argument).
        Op::NewInductive { .. } => S,
        Op::TestArm { .. } => S,
        Op::BindArm { .. } => S,
        // `Check that <subj> is <pred>` / `… can <action> <obj>` — the `## Policy` condition is
        // resolved from the registry and compiled inline (field access + `text_eq` + and/or), trapping
        // when false. Text-field / predicate / and-or / cross-field conditions; numeric/bool fields defer.
        Op::CheckPolicy { .. } => S,
        // `Increase/Decrease <obj>'s <counter> by <n>` on a SINGLE-replica `Shared` struct's
        // `ConvergentCount` is a plain struct-field `±=` (the counter rides as an `Int` field). Multi-
        // replica MERGE + the set/seq/register CRDTs need the per-replica runtime object → still deferred.
        Op::CrdtBump { .. } => S,
        // `Merge <src> into <target>` of counter-only `Shared` structs is a per-field `Int` SUM
        // (`crdt_merge_field` on two plain-Int counters). A set/seq/register/GCounter-struct field
        // needs the per-replica runtime object and is refused (that `Merge` stays deferred).
        Op::CrdtMerge { .. } => S,
        // A single-replica CRDT collection IS its underlying collection: `NewCrdt` → an empty
        // Set/Seq/Text, `CrdtAppend` → in-place list/set add (no COW — the CRDT field is mutable-
        // shared), `CrdtResolve` → a divergent-register field overwrite. Multi-replica merge of these
        // (set union / RGA weave / MV-register) needs the per-replica runtime and is a separate `Op`.
        Op::NewCrdt { .. } | Op::CrdtAppend { .. } | Op::CrdtResolve { .. } => S,
        // `CrdtJoin` (op113) is the state-based G-Counter LATTICE JOIN — unconditional per-replica MAX
        // — issued by the SILICON `crdt_merge` coprocessor (`codegen_sva/rtl.rs`) and the binary-ISA
        // decoder. The AST→bytecode compiler that feeds the WASM AOT never emits it (the `Merge`
        // statement lowers to `CrdtMerge`, above); the VM executes it only to cross-check the hardware
        // block. No WASM-targeted program can contain it — Unreachable here, not a gap; the emitter
        // soundly REJECTS it (`WasmLowerError`) if one ever appeared, rather than miscompiling.
        Op::CrdtJoin { .. } => {
            U("no source syntax emits `CrdtJoin` (op113 = the silicon/binary-ISA per-replica-MAX crdt_merge coprocessor; the `Merge` statement lowers to `CrdtMerge`)")
        }
        // `BigIntWide` is the 256-bit wide-BigInt datapath lane (FromInt/Add/Sub/Cmp/Low…) routed to the
        // soft-core's `bigint_alu`; it is produced only by the silicon RTL / binary-ISA paths and
        // executed by the VM to mirror that hardware. The AST→bytecode compiler feeding the WASM AOT
        // never emits it (overflowing arithmetic promotes through the runtime-linked BigInt frontier,
        // not this op). Unreachable here — soundly rejected by the emitter if one appeared.
        Op::BigIntWide { .. } => {
            U("no source syntax emits `BigIntWide` (the 256-bit wide-BigInt datapath lane is silicon-RTL / binary-ISA only; overflowing arithmetic uses the runtime-linked BigInt path)")
        }

        // ── P5: concurrency — NOT excluded. The pure-Rust scheduler is wasm-safe and WS5 proved
        //    true-multicore wasm (shared memory + atomics); these land via the scheduler compiled
        //    into the module plus async host imports for timers/yield. Deferred, never excluded.
        // DETERMINISTIC SINGLE-THREAD concurrency (matches the seeded cooperative scheduler for the
        // non-blocking guide shapes; verified tw == VM(driven, `vm_outcome_concurrent`) == AOT):
        // `Sleep`/`Stop` = no-op (virtual time / already-run task), a `Pipe`/channel = a FIFO queue,
        // a `Launch` task runs synchronously. Blocking channel/try/close + `Await`'s value stay deferred.
        // `Sleep N` advances the deterministic scheduler's VIRTUAL time only (no observable output on
        // the non-racing shapes), so the AOT lowers it to a no-op — verified tw == VM(driven) == AOT.
        Op::Sleep { .. } => S,
        Op::ChanNew { .. } | Op::ChanSend { .. } | Op::ChanRecv { .. } => S,
        Op::Spawn { .. } | Op::SpawnHandle { .. } | Op::TaskAbort { .. } => S,
        // `Try to send` (unbounded FIFO always accepts → append + `Bool(true)`) and `Try to receive`
        // (pop-front → an `Optional`: a `Some` box or `Nothing` on an empty queue) are lowered and
        // RUN-verified tw == VM(driven) == AOT via the Optional value model.
        Op::ChanTrySend { .. } | Op::ChanTryRecv { .. } => S,
        // `ChanClose`/`TaskAwait` are NOT deferred work — they are DEAD bytecode ops the language cannot
        // produce: NO source syntax emits them (`Close`/`Await` compile to `Select`/`Net`/other ops), so no
        // program can exercise them. The emitter DOES lower both (a no-op / a handle pass-through) so the
        // backend is complete over every `Op`, but the exercised-iff-Supported invariant (every `S` op must
        // be run by a corpus program) STRUCTURALLY requires an unemittable op to be classified `D` — the
        // reason is "no emit site", not "TODO". This is the honest floor, not a deferral.
        Op::ChanClose { .. } => U("no source syntax emits `Close` (lowered as a no-op regardless)"),
        Op::TaskAwait { .. } => U("no source syntax emits a task-value `Await` (lowered as a pass-through regardless)"),
        // `select` (`Await the first of …`) resolves deterministically: a recv arm whose FIFO queue is
        // non-empty wins (pop-front into its var), else the timeout arm fires — exactly the seeded
        // scheduler's choice for the non-racing shapes (verified tw == VM(driven) == AOT).
        Op::SelectArmRecv { .. } | Op::SelectArmTimeout { .. } | Op::SelectWait { .. } => S,

        // ── P6: networking — NOT excluded. Lands via per-capability host imports mirroring the
        //    existing browser WS/relay transports (relay_browser, gloo-net). Deferred, never excluded.
        // DETERMINISTIC LOCAL-MODE networking (matches the interpreter's offline mode — a single node
        // with no relay: `Listen`/`Send`/`Stream`/`Sync` are no-ops, `PeerAgent` = its address Text).
        // `Connect` (dials a relay) + `Await` (blocks for a message) stay deferred to the linker/host.
        // `Connect` is a single-node LOCAL NO-OP in the deterministic model (offline: no relay to
        // dial → the following ops run locally, mirroring Listen/Send/Sync). Verified tw == VM-net == AOT.
        Op::NetConnect { .. } | Op::NetListen { .. } | Op::NetSend { .. } | Op::NetSync { .. } | Op::NetMakePeer { .. } => S,
        // `Await`/`Stream` — the offline LOOPBACK: a `Send`/`Stream` delivers into our own local inbox
        // FIFO and a matching `Await` pops it (the oracle output is transport-independent). Verified
        // tw == VM-net == AOT. (A real relay round-trip lands via the P6 linker/host phase.)
        Op::NetAwait { .. } | Op::NetStream { .. } => S,
        // Real socket IO is deliberately a NATIVE-TARGET capability, unlike the deterministic
        // local-mode Net ops above. A standalone/browser WebAssembly module cannot bind a port or
        // own an OS socket registry, so these are neither linked-WASM support nor deferred work.
        Op::Tcp { .. } => {
            N("tcp.lg requires native OS sockets; standalone/browser WebAssembly has no socket capability")
        }
        Op::Serve { .. } => {
            N("Serve requires a native HTTP listener and accept loop; standalone/browser WebAssembly cannot bind ports")
        }
        // Observation (Wave 13) is a VM/tree-walker-only OBSERVED-profile op: it is
        // ERASED at lowering before the AOT/WASM tiers (`elide_observation`, §0.7), so
        // the compiled backends never see it — the honest `Unreachable`, not deferred
        // work. Its dev-tier emission is proven by `obs_tier_differential`.
        Op::Obs { .. } => L("observation emits as the host `obs(site_id)` import in the OBSERVED profile (`compile_to_wasm_observed`); the host resolves the site's category/message/span from the module's `observation_sites`. The default self-contained `compile_to_wasm` ELIDES it (§0.7), so it never appears there — hence Linked, not Supported."),
    }
}

/// A stable name for an op variant — delegates to [`Op::name`], the single source of truth promoted
/// onto the type and shared with the reversibility lock (`tests/reverse_op_lock.rs`) so the two
/// catalogs can never drift. Kept as a thin wrapper so the call sites below read unchanged.
fn op_name(op: &Op) -> &'static str {
    op.name()
}

/// One instance of EVERY VM bytecode op — delegates to [`Op::all_variants`], the single ISA catalog
/// promoted onto the type and shared with the reversibility lock, so this coverage proof and the
/// reversibility census enumerate the exact same instruction set.
fn all_op_variants() -> Vec<Op> {
    Op::all_variants()
}

/// Compile `src` to bytecode (the SAME path `compile_to_wasm` uses) and return its full op
/// stream (Main + every function body), or `None` if it does not reach the VM.
fn program_ops(src: &str) -> Option<Vec<Op>> {
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) = parsed.ok()?;
        // Compile with the ORACLE (matching `compile_to_wasm`) so the op census sees exactly the
        // bytecode the AOT lowers — including the Oracle-only optimizer forms (DivPow2 / MagicDivU /
        // IndexUnchecked / SetIndexUnchecked / RegionBoundsGuard). Otherwise those ops would be
        // Supported-yet-unexercised (the plain `compile_with_types` never emits them).
        let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
        let program =
            Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle)).ok()?;
        Some(program.code.clone())
    })
}

/// Read the sequence at `handle` out of the module's exported memory (header `[len][cap][data_ptr]`
/// then `len` 8-byte elements at `data_ptr`) and format it as the tree-walker's `RuntimeValue::List`
/// would: `[e0, e1, …]`, each element rendered by `fmt` (its scalar display).
fn read_seq(
    c: &wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>,
    handle: i32,
    fmt: impl Fn([u8; 8]) -> String,
) -> String {
    let mem = match c.get_export("memory").and_then(|e| e.into_memory()) {
        Some(m) => m,
        None => return "[]".to_string(),
    };
    let data = mem.data(c);
    let h = handle as usize;
    let len = i32::from_le_bytes(data[h..h + 4].try_into().unwrap()) as usize;
    let dptr = i32::from_le_bytes(data[h + 8..h + 12].try_into().unwrap()) as usize;
    let parts: Vec<String> = (0..len)
        .map(|i| fmt(data[dptr + i * 8..dptr + i * 8 + 8].try_into().unwrap()))
        .collect();
    format!("[{}]", parts.join(", "))
}

/// Read the UTF-8 `Text` at `handle` out of the module's exported memory (header `[len][cap]
/// [data_ptr]`, then `len` bytes at `data_ptr`) as a `String` — a `RuntimeValue::Text` displays
/// as its raw contents.
fn read_text(c: &wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32) -> String {
    let mem = match c.get_export("memory").and_then(|e| e.into_memory()) {
        Some(m) => m,
        None => return String::new(),
    };
    let data = mem.data(c);
    let h = handle as usize;
    let len = i32::from_le_bytes(data[h..h + 4].try_into().unwrap()) as usize;
    let dptr = i32::from_le_bytes(data[h + 8..h + 12].try_into().unwrap()) as usize;
    String::from_utf8_lossy(&data[dptr..dptr + len]).into_owned()
}

/// Read a whole sequence of `Text` at `handle` and format `[s0, s1, …]` (elements unquoted) — the
/// host side of `print_seq_text`. Each slot's low word is a `Text` handle (read via [`read_text`]).
fn read_seq_text(c: &wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32) -> String {
    let handles: Vec<i32> = match c.get_export("memory").and_then(|e| e.into_memory()) {
        Some(mem) => {
            let data = mem.data(c);
            let h = handle as usize;
            let len = i32::from_le_bytes(data[h..h + 4].try_into().unwrap()) as usize;
            let dptr = i32::from_le_bytes(data[h + 8..h + 12].try_into().unwrap()) as usize;
            (0..len)
                .map(|i| {
                    i32::from_le_bytes(data[dptr + i * 8..dptr + i * 8 + 4].try_into().unwrap())
                })
                .collect()
        }
        None => Vec::new(),
    };
    let parts: Vec<String> = handles.iter().map(|&th| read_text(c, th)).collect();
    format!("[{}]", parts.join(", "))
}

/// Read a whole `Set of Int` at `handle` and format `{e0, e1, …}` (the VM's insertion-ordered Set
/// display) — the host side of `print_set_i64`. Same header+8-byte-slot layout as a sequence.
fn read_set_i64(c: &wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32) -> String {
    let mem = match c.get_export("memory").and_then(|e| e.into_memory()) {
        Some(m) => m,
        None => return "{}".to_string(),
    };
    let data = mem.data(c);
    let h = handle as usize;
    let len = i32::from_le_bytes(data[h..h + 4].try_into().unwrap()) as usize;
    let dptr = i32::from_le_bytes(data[h + 8..h + 12].try_into().unwrap()) as usize;
    let parts: Vec<String> = (0..len)
        .map(|i| {
            i64::from_le_bytes(data[dptr + i * 8..dptr + i * 8 + 8].try_into().unwrap()).to_string()
        })
        .collect();
    format!("{{{}}}", parts.join(", "))
}

/// Read a whole `Set of Text` at `handle` and format `{s0, s1, …}` (elements unquoted, insertion
/// order) — the host side of `print_set_text`. Each 8-byte slot's low word is a `Text` handle.
fn read_set_text(c: &wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32) -> String {
    let handles: Vec<i32> = match c.get_export("memory").and_then(|e| e.into_memory()) {
        Some(mem) => {
            let data = mem.data(c);
            let h = handle as usize;
            let len = i32::from_le_bytes(data[h..h + 4].try_into().unwrap()) as usize;
            let dptr = i32::from_le_bytes(data[h + 8..h + 12].try_into().unwrap()) as usize;
            (0..len)
                .map(|i| {
                    i32::from_le_bytes(data[dptr + i * 8..dptr + i * 8 + 4].try_into().unwrap())
                })
                .collect()
        }
        None => Vec::new(),
    };
    let parts: Vec<String> = handles.iter().map(|&th| read_text(c, th)).collect();
    format!("{{{}}}", parts.join(", "))
}

/// Run an emitted module through `wasmi`, capturing each `Show` as one output line via the
/// `env.print_*` host sinks, then calling `main`. `Ok(lines joined by '\n')` if `main` returns,
/// `Err(trap description)` if it traps (the error contract: a standalone module signals a runtime
/// failure by trapping). [`run_aot`] is the success-path wrapper that asserts no trap.
fn run_aot_result(module: &[u8]) -> Result<String, String> {
    let engine = wasmi::Engine::default();
    let m = wasmi::Module::new(&engine, module).expect("emitted bytes are valid wasm");
    let out: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let mut store = wasmi::Store::new(&engine, out.clone());
    let mut linker = wasmi::Linker::<Rc<RefCell<Vec<String>>>>::new(&engine);
    common::wasm_host::link_exact_show(&mut linker, |out, line| out.borrow_mut().push(line));
    // Every import whose answer the ABI fixes — one implementation, shared, so no test can get
    // `fmt_i64_into`'s return value wrong again (see `wasm_host_stub_lock`).
    crate::common::wasm_host::link_value_imports::<Rc<RefCell<Vec<String>>>>(&mut linker);
    // Every program that CAN fail declares `logos_rt_fail`, and a module importing a
    // function its host does not define FAILS TO INSTANTIATE — silently, with no
    // compile error anywhere. This host does not care why; it must simply be here.
    linker
        .func_wrap(
            "env",
            "logos_rt_fail",
            |_c: wasmi::Caller<'_, _>, _h: i32| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "logos_rt_fail_at",
            |_c: wasmi::Caller<'_, _>, _code: i32, _v: i64| {},
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_i64",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
                c.data().borrow_mut().push(v.to_string());
            },
        )
        .unwrap()
        .func_wrap(
            "env",
            "print_rational",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, num: i64, den: i64| {
                c.data().borrow_mut().push(if den == 1 {
                    num.to_string()
                } else {
                    format!("{num}/{den}")
                });
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_nothing",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>| {
                c.data().borrow_mut().push("nothing".into());
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_word",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i64| {
                c.data().borrow_mut().push((v as u64).to_string());
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_bool",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: i32| {
                c.data().borrow_mut().push(if v != 0 {
                    "true".into()
                } else {
                    "false".into()
                });
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_f64",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, v: wasmi::core::F64| {
                c.data().borrow_mut().push(
                    logicaffeine_compile::compile::display_float_like_logos(f64::from(v)),
                );
            },
        )
        .unwrap();
    // `today`/`now` (the SAME thread-local fixed-clock the tree-walker/VM read) are the shared
    // `link_value_imports` host's job now; print_date/print_moment delegate to the real
    // `to_display_string` here — so tw==vm==wasm.
    use logicaffeine_compile::interpreter::RuntimeValue;
    linker
        .func_wrap(
            "env",
            "print_date",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, d: i32| {
                c.data()
                    .borrow_mut()
                    .push(RuntimeValue::Date(d).to_display_string());
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_moment",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, n: i64| {
                c.data()
                    .borrow_mut()
                    .push(RuntimeValue::Moment(n).to_display_string());
            },
        )
        .unwrap();
    // Sequence display: the host reads the stable header `[len][cap][data_ptr]` and the element
    // buffer out of the module's exported linear memory, and formats `[e0, e1, …]` exactly as the
    // tree-walker's `RuntimeValue::List` (each element by its scalar display) — zero drift.
    linker
        .func_wrap(
            "env",
            "print_seq_i64",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
                let s = read_seq(&c, handle, |b| i64::from_le_bytes(b).to_string());
                c.data().borrow_mut().push(s);
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_seq_f64",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
                let s = read_seq(&c, handle, |b| {
                    logicaffeine_compile::compile::display_float_like_logos(f64::from_le_bytes(b))
                });
                c.data().borrow_mut().push(s);
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_text",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
                let s = read_text(&c, handle);
                c.data().borrow_mut().push(s);
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_seq_text",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
                let s = read_seq_text(&c, handle);
                c.data().borrow_mut().push(s);
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_set_i64",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
                let s = read_set_i64(&c, handle);
                c.data().borrow_mut().push(s);
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "print_set_text",
            |c: wasmi::Caller<'_, Rc<RefCell<Vec<String>>>>, handle: i32| {
                let s = read_set_text(&c, handle);
                c.data().borrow_mut().push(s);
            },
        )
        .unwrap();
    // `fmt_seq_i64_into`/`fmt_set_i64_into` (a collection operand in a `+`/`format`) are the shared
    // `link_value_imports` host's job now.
    crate::common::wasm_host::link_unmodelled_imports(&mut linker);
    let instance = linker
        .instantiate(&mut store, &m)
        .unwrap()
        .start(&mut store)
        .unwrap();
    match instance
        .get_typed_func::<(), ()>(&store, "main")
        .unwrap()
        .call(&mut store, ())
    {
        Ok(()) => Ok(out.borrow().clone().join("\n")),
        Err(e) => Err(format!("{e:?}")),
    }
}

/// Run an emitted module on the success path, asserting `main` completes without trapping.
fn run_aot(module: &[u8]) -> String {
    run_aot_result(module).expect("main runs without trapping")
}

/// The growing curated corpus of self-contained scalar programs. Each must run identically on the
/// tree-walker, the VM, and (when fully `Supported`) the emitted WebAssembly. New backend features
/// add programs here that exercise them.
const CORPUS: &[(&str, &str)] = &[
    ("show_int", "## Main\n    Show 7.\n"),
    ("seq_dynamic_capacity", "## Main\n    Let n be 17.\n    Let mutable xs be a new Seq of Int with capacity n.\n    Push 7 to xs.\n    Show xs.\n    Show length of xs.\n"),
    ("map_dynamic_capacity", "## Main\n    Let n be 17.\n    Let mutable m be a new Map of Int to Int with capacity n.\n    Set item 5 of m to 50.\n    Show item 5 of m.\n    Show length of m.\n"),
            // A different valtype down the same path — a fix keyed on integers would miss it.
    ("inspect_option_text", "## Main\n    Let v be some \"hi\".\n    Inspect v:\n        When Some (t):\n            Show t.\n        When None:\n            Show \"-\".\n"),
    // The case that always worked must keep working: the new transparent branch must not swallow
    // a genuine enum bind.
    ("inspect_result_still_enum", "## To f (n: Int) -> Result of Int and Text:\n    If n is greater than 0:\n        Return a new Ok with value n.\n    Return a new Err with error \"neg\".\n\n## Main\n    Inspect f(7):\n        When Ok (v):\n            Show v.\n        When Err (e):\n            Show e.\n"),
    // A mutable-borrow function (`bump` mutates a Seq param in place and returns it) called
    // as the consuming `Set a to bump(a, …)` — the VM compiler emits the call-site
    // `Op::EnsureOwned` copy-on-write barrier before the argument. In the WASM AOT it lowers
    // to a no-op (the backend copies-on-write at each element write), so WASM == VM == Tree-walker.
    ("mut_borrow", "## To bump (arr: Seq of Int, i: Int) -> Seq of Int:\n    Let mutable r be arr.\n    Set item i of r to 99.\n    Return r.\n\n## Main\n    Let mutable a be a new Seq of Int.\n    Push 1 to a.\n    Push 2 to a.\n    Set a to bump(a, 1).\n    Show item 1 of a.\n    Show item 2 of a.\n"),
    ("arith", "## Main\n    Let n be 6 * 7.\n    Show n.\n"),
    ("multi_show", "## Main\n    Show 1.\n    Show 2.\n    Show 3.\n"),
    ("sub_div_mod", "## Main\n    Let a be 100 - 17.\n    Let b be a / 4.\n    Let c be a % 4.\n    Show b.\n    Show c.\n"),
    // `Swap a and b` on two same-typed locals — the zero-garbage reversible register exchange
    // (`Op::RevSwap`). The AOT lowers it to a scratch-free wasm local swap; WASM == VM == Tree-walker
    // (`a` and `b` trade values, so `Show a; Show b` prints `2\n1`). Exercises the `RevSwap` op.
    ("swap", "## Main\n    Let mutable a be 1.\n    Let mutable b be 2.\n    Swap a and b.\n    Show a.\n    Show b.\n"),
    // `**` exercises all three `Op::Pow` lowering paths: `Int^Int` (the in-module squaring loop,
    // register operands so it can't be const-folded away), `Float^Float` (host `pow_ff`), and
    // `Float^Int` (host `pow_fi`). Every value fits i64 / is finite, so no overflow trap.
    ("power", "## Main\n    Let b be 3.\n    Let e be 4.\n    Show b ** e.\n    Let x be 2.0.\n    Show x ** 0.5.\n    Let y be 2.5.\n    Show y ** 3.\n"),
    // `repeatSeq` — `n copies of x` builds a fresh n-element sequence of the scalar x, filled in a
    // runtime loop (Int and Float element kinds). A zero/negative count yields the empty sequence.
    ("repeat_seq", "## Main\n    Let xs be 3 copies of 7.\n    Show xs.\n    Let ys be 2 copies of 1.5.\n    Show ys.\n    Let zs be 0 copies of 9.\n    Show zs.\n"),
    (
        "function_call",
        "## To dbl (x: Int) -> Int:\n    Return x * 2.\n## Main\n    Show dbl(21).\n",
    ),
    (
        "two_functions",
        "## To sq (x: Int) -> Int:\n    Return x * x.\n\
         ## To sumsq (a: Int) (b: Int) -> Int:\n    Return sq(a) + sq(b).\n\
         ## Main\n    Show sumsq(3, 4).\n",
    ),
    (
        "while_loop",
        "## To tri (n: Int) -> Int:\n    \
         Let acc be 0.\n    Let i be n.\n    \
         While i is greater than 0:\n        Set acc to acc + i.\n        Set i to i - 1.\n    \
         Return acc.\n## Main\n    Show tri(100).\n",
    ),
    (
        "if_branch",
        "## To clamp (x: Int) -> Int:\n    \
         If x is greater than 5:\n        Return 5.\n    Return x.\n\
         ## Main\n    Show clamp(8).\n    Show clamp(3).\n",
    ),
    (
        "recursive_fib",
        "## To fib (n: Int) -> Int:\n    \
         If n is less than 2:\n        Return n.\n    \
         Return fib(n - 1) + fib(n - 2).\n## Main\n    Show fib(10).\n",
    ),
    (
        "modulo_function",
        "## To parity (x: Int) -> Int:\n    Let r be x % 2.\n    Return r.\n## Main\n    Show parity(10).\n    Show parity(7).\n",
    ),
    (
        // Floor division `//` (`BinaryOpKind::FloorDivide` → `Op::FloorDiv`) FLOORS toward -infinity,
        // so it differs from the truncating `/` on mixed signs: `-7 // 2 = -4` (not -3), `7 // -2 = -4`,
        // `-7 // -2 = 3`. Exercises `lower_floordiv_regs`' truncate-then-correct path (`q - ((r≠0) &
        // ((r^b)<0))`) against the VM, plus left-associativity (`(10 // 3) // 2 = 1`).
        "floor_division",
        "## Main\n    Let a be 0 - 7.\n    Let b be 0 - 2.\n    \
         Show 7 // 2.\n    Show a // 2.\n    Show 7 // b.\n    Show a // b.\n    Show 10 // 3 // 2.\n",
    ),
    (
        // A top-level Main binding read INSIDE a function is a promoted global (GlobalSet in
        // Main, GlobalGet in the function body).
        "global_read_in_function",
        "## To over (x: Int) -> Int:\n    Let d be x - limit.\n    Return d.\n\
         ## Main\n    Let limit be 50.\n    Show over(80).\n    Show over(20).\n",
    ),
    (
        // Float arithmetic + Show(Float): exercises f64 locals, f64.mul/add, and print_f64
        // (whose host formatting must match the tree-walker's `to_display_string` exactly).
        "float_arith",
        "## Main\n    Let pi be 3.14.\n    Let factor be 2.0.\n    Let area be pi * factor.\n    Show area.\n",
    ),
    (
        // MIXED Int/Float arithmetic + comparison: the tree-walker promotes the Int operand to f64,
        // and so does the backend now (`push_as_f64` in `lower_arith`/`lower_compare`) instead of
        // emitting an f64 op on an i64 (which was invalid wasm). `3 + 1.5`=4.5, `9 / 2.0`=4.5, a
        // Float-vs-Int compare, and a Float var plus an Int literal.
        "mixed_int_float",
        "## Main\n    Let a be 3 + 1.5.\n    Show a.\n    \
         Let b be 9 / 2.0.\n    Show b.\n    \
         If 5 is at least 2.5:\n        Show 1.\n    Otherwise:\n        Show 0.\n    \
         Let x be 2.5.\n    Let y be x + 1.\n    Show y.\n",
    ),
    (
        // Int→Float promotion at the CALL boundary (`half(9)` → an Int arg into a Float param) +
        // TYPE-STRICT equality (`2.0 equals 2` is false — a Float never equals an Int even at the
        // same value, unlike ordering which promotes) + a VOID function call (`greet`, which returns
        // nothing, must not bind a result). All three were invalid-wasm / miscompiles before.
        "mixed_float_calls",
        "## To half (x: Float) -> Float:\n    Return x / 2.0.\n\
         ## To greet (n: Int):\n    Show n.\n\
         ## Main\n    Show half(9).\n    \
         Show 2.0 equals 2.\n    Show 2.0 equals 2.0.\n    \
         greet(42).\n",
    ),
    (
        // SEQUENCE parameters: a function taking `Seq of Int`/`Seq of Float`. A seq crosses the call
        // boundary as one stable i32 handle (the bytecode's heap-value ABI), and the element kind is
        // in the declaration, so the seq kind is self-describing — `param_seeds` maps `List(Int)`→
        // SeqInt / `List(Float)`→SeqFloat and the body's iterate/length/index just work.
        "seq_param",
        "## To sumseq (xs: Seq of Int) -> Int:\n    Let mutable t be 0.\n    \
         Repeat for x in xs:\n        Set t to t + x.\n    Return t.\n\
         ## To fsum (ys: Seq of Float) -> Float:\n    Let mutable t be 0.0.\n    \
         Repeat for y in ys:\n        Set t to t + y.\n    Return t.\n\
         ## Main\n    Show sumseq([10, 20, 30]).\n    Show fsum([1.5, 2.5]).\n",
    ),
    (
        // A function RETURNING a sequence (a collection-producing function — uses the cross-region
        // return machinery, the seq crossing back as one i32 handle), then the caller uses it.
        "fn_returns_seq",
        "## To build () -> Seq of Int:\n    Let mutable a be a new Seq of Int.\n    \
         Push 10 to a.\n    Push 20 to a.\n    Push 30 to a.\n    Return a.\n\
         ## Main\n    Let b be build().\n    Show length of b.\n    Show item 2 of b.\n",
    ),
    (
        // A seq PARAMETER mutated inside the callee (`Push`) — sequences are REFERENCE-semantic (the
        // complement of structs' value semantics + copy-on-write), so the caller SEES the growth.
        "seq_param_mutate",
        "## To addone (xs: Seq of Int):\n    Push 99 to xs.\n\
         ## Main\n    Let mutable a be a new Seq of Int.\n    Push 1 to a.\n    Push 2 to a.\n    \
         addone(a).\n    Show length of a.\n",
    ),
    (
        // A Bool PARAMETER and Bool RETURN (the i64 0/1 boolean lane crossing the call boundary),
        // and a Bool argument formed from a comparison.
        "bool_param",
        "## To negate (b: Bool) -> Bool:\n    Return not b.\n\
         ## Main\n    Show negate(true).\n    Show negate(5 is greater than 3).\n",
    ),
    (
        // STRUCT PARAMETER — `f(p: Point)`. The proper fix: the bytecode now carries the program's
        // struct type registry (`CompiledProgram::struct_types`) + each parameter's resolved type
        // (`CompiledFunction::param_types`), so the AOT seeds the parameter's field layout from the
        // bytecode alone (no AST, no inter-procedural reconstruction) and `p's field` resolves.
        "struct_param",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## To sumpt (p: Point) -> Int:\n    Return p's x + p's y.\n\
         ## Main\n    Let a be a new Point with x 10 and y 32.\n    Show sumpt(a).\n",
    ),
    (
        // TEXT PARAMETER — `f(s: Text)`. `Text` is not a `SlotKind`/`ParamKind`, so before the
        // bytecode carried `param_types` this was rejected; now the parameter types as `Text` (one
        // i32 handle, self-describing) and the body's concat/length just work.
        "text_param",
        "## To wrap (s: Text) -> Text:\n    Return \"[\" combined with s combined with \"]\".\n\
         ## Main\n    Show wrap(\"hi\").\n",
    ),
    (
        // ENUM PARAMETER — `f(c: Color)`. Enums are also carried in `param_types` (as
        // `BoundaryType::Enum(name)`, from the type registry); the parameter types as `Kind::Enum`
        // (one i32 handle whose word 0 is the constructor tag), so the body's `Inspect`/`When`
        // (`TestArm` reads the tag) works with no layout needed.
        "enum_param",
        "## A Color is one of:\n    A Red.\n    A Green.\n    A Blue.\n\n\
         ## To weight (c: Color) -> Int:\n    Inspect c:\n        When Red: Return 1.\n        \
         When Green: Return 2.\n        When Blue: Return 3.\n    Return 0.\n\
         ## Main\n    Show weight(Green).\n    Show weight(Blue).\n",
    ),
    (
        // WHOLE-ENUM `Show` — a nullary variant displays as just its constructor name
        // (`RuntimeValue::Inductive` with empty args → `ind.constructor`). The AOT emits a tag→name
        // dispatch over the enum type's CONSTRUCTED variants (the stored tag = the variant name's
        // constant index; a never-constructed variant can't be the value, so it needs no branch);
        // exactly one branch matches the live tag. Two distinct variants prove the RIGHT branch fires.
        "enum_show_nullary",
        "## A Direction is one of:\n    A North.\n    A South.\n    A East.\n    A West.\n\n\
         ## Main\n    Let a be East.\n    Let b be West.\n    Show a.\n    Show b.\n",
    ),
    (
        // NESTED struct-parameter field access — `b's corner's x`, a field of a struct-typed field
        // of a parameter. The seeded parameter layout now carries each struct field's TYPE NAME, so
        // the first `GetField` re-seeds the result's own (the `Point`'s) layout and the second
        // resolves cross-region. Re-seeding is recursive (resolved one level per `GetField`), so it
        // generalizes to any depth; this previously rejected.
        "nested_struct_param_field",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## A Box has:\n    A corner: Point.\n    A tag: Int.\n\n\
         ## To cornerx (b: Box) -> Int:\n    Return b's corner's x.\n\
         ## Main\n    Let p be a new Point with x 3 and y 4.\n\
         Let bx be a new Box with corner p and tag 9.\n    Show cornerx(bx).\n",
    ),
    (
        // NESTED field access on a CALL RESULT — `b's corner's x` where `b` is a struct RETURNED by a
        // function. The callee's return layout names each struct-typed field's type (from its
        // `NewStruct`, tracked in `struct_name_of`), so the caller re-seeds and resolves the deeper
        // field — symmetric with the parameter path above. The inner `Point` is built inline in the
        // `Return` so the returned struct is fully self-contained in one expression.
        "nested_call_result_field",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## A Box has:\n    A corner: Point.\n    A tag: Int.\n\n\
         ## To makebox () -> Box:\n    \
         Return a new Box with corner (a new Point with x 7 and y 8) and tag 5.\n\
         ## Main\n    Let b be makebox().\n    Show b's corner's x.\n",
    ),
    (
        // MAP PARAMETER — `f(m: Map of Int to Int)`. `Map` is one i32 handle; the access value kind
        // (`item k of m`) is carried by `BoundaryType::Map`'s VALUE element and seeded into the layout
        // pass (`index_value_kind`), so the read/`contains`/`length` resolve cross-region. The key kind
        // comes from the key expression itself. Maps are reference-semantic (the handle is shared), so
        // a mutation through the parameter is visible to the caller — byte-identical to the VM.
        "map_param",
        "## To readv (m: Map of Int to Int) -> Int:\n    Return item 1 of m + item 2 of m.\n\
         ## To has (m: Map of Int to Int) -> Bool:\n    Return m contains 9.\n\
         ## To sz (m: Map of Int to Int) -> Int:\n    Return length of m.\n\
         ## To bump (m: Map of Int to Int):\n    Set item 1 of m to 100.\n\
         ## Main\n    Let mutable m be a new Map of Int to Int.\n    \
         Set item 1 of m to 10.\n    Set item 2 of m to 20.\n    \
         Show readv(m).\n    Show has(m).\n    Show sz(m).\n    bump(m).\n    Show item 1 of m.\n",
    ),
    (
        // MAP PARAMETER with a FLOAT value + a TEXT key — the value kind drives the f64 value-slot
        // load, the key kind drives the byte-equality key scan; both resolved cross-region.
        "map_param_float_textkey",
        "## To score (m: Map of Text to Float) -> Float:\n    Return item \"a\" of m.\n\
         ## Main\n    Let mutable m be a new Map of Text to Float.\n    \
         Set item \"a\" of m to 2.5.\n    Show score(m).\n",
    ),
    (
        // TUPLE PARAMETER — `f(p: Pair of Int and Int)` / `Triple of …`. A HOMOGENEOUS tuple lays out
        // identically to a `Seq` of the shared element kind, so the parameter resolves through the seq
        // path (`BoundaryType::Pair`→`Seq`) and `item N of p` indexes the buffer; no tuple-specific
        // seeding. A heterogeneous tuple has no single element kind and stays rejected.
        "tuple_param",
        "## To addpair (p: Pair of Int and Int) -> Int:\n    Return item 1 of p + item 2 of p.\n\
         ## To mid (t: Triple of Int and Int and Int) -> Int:\n    Return item 2 of t.\n\
         ## Main\n    Let p be (3, 4).\n    Let t be (7, 8, 9).\n    Show addpair(p).\n    Show mid(t).\n",
    ),
    (
        // WHOLE-TUPLE `Show` — the Syntax Guide `tuple-create` example verbatim. The tree-walker
        // displays a tuple as `(e0, e1, …)` (deterministic order), assembled inline from the static
        // layout. The HOMOGENEOUS `(10, 20)` (which the kind pass collapses to `SeqInt`) must still
        // print with tuple parens `(10, 20)`, NOT list brackets `[10, 20]` — keyed on `tuple_layouts`,
        // not the Kind — while the HETEROGENEOUS `("Alice", 25, true)` stringifies Int/Text/Bool each
        // by its own formatter. (A real `NewList` literal keeps its `[…]` sink: it is not a tuple.)
        "tuple_show_whole",
        "## Main\n    Let point be (10, 20).\n    Let record be (\"Alice\", 25, true).\n    \
         Show point.\n    Show record.\n",
    ),
    (
        // PAYLOAD-ENUM PARAMETER — `f(s: Shape)` matched with `When V (binds)`. An enum parameter's
        // construction isn't in scope, so the bound payload kinds are carried in the bytecode's
        // `enum_types` (per-variant field layout) and seeded as `ParamShape::Enum`; `struct_layout`
        // pairs each `BindArm` with its governing `TestArm` variant (last-write-wins, the sum-type
        // analog of the local construct-then-inspect path) and resolves `field_kinds[index]`. The
        // lowering (`lower_bind_arm`, offset `8*(1+index)`) was already ready. Single- and multi-field
        // variants, recursion-free.
        "enum_payload_param",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## To area (s: Shape) -> Int:\n    Inspect s:\n        When Circle (rad): Return rad.\n        \
         When Rectangle (w, h): Return w * h.\n    Return 0.\n\
         ## To twice (s: Shape) -> Int:\n    Return area(s) + area(s).\n\
         ## Main\n    Let c be a new Circle with radius 7.\n    \
         Let r be a new Rectangle with width 3 and height 4.\n    \
         Show area(c).\n    Show area(r).\n    Show twice(c).\n",
    ),
    (
        // PAYLOAD-ENUM PARAMETER mixing a NULLARY variant (`Nothing`, no `BindArm`) with a PAYLOAD one
        // (`Just (v)`) — the nullary arm needs no layout, the payload arm resolves its bind via the
        // seeded variant layout. A Bool payload also rides the same path.
        "enum_payload_param_mixed",
        "## A Maybe is one of:\n    A Nothing.\n    A Just with val Int.\n\n\
         ## To get (o: Maybe) -> Int:\n    Inspect o:\n        When Nothing: Return -1.\n        \
         When Just (v): Return v.\n    Return 0.\n\
         ## Main\n    Let a be a new Just with val 42.\n    Let b be a new Nothing.\n    \
         Show get(a).\n    Show get(b).\n",
    ),
    (
        // STRUCT-WITH-ENUM-FIELD PARAMETER — `f(t: Tagged)` where `Tagged` has an enum field `shape`.
        // The `GetField` (`t's shape`) re-seeds the result with the enum's per-variant payload layout
        // (carried as `FieldNested::Enum` in the struct's `FieldLayout`, resolved from `enum_types`),
        // so the following `Inspect`/`BindArm` resolves cross-region — the field-composed analog of a
        // direct enum parameter.
        "struct_enum_field_param",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## A Tagged has:\n    A label: Int.\n    A shape: Shape.\n\n\
         ## To area (t: Tagged) -> Int:\n    Inspect t's shape:\n        When Circle (r): Return r.\n        \
         When Rectangle (w, h): Return w * h.\n    Return 0.\n\
         ## Main\n    Let c be a new Circle with radius 7.\n    \
         Let tg be a new Tagged with label 1 and shape c.\n    \
         Let r be a new Rectangle with width 3 and height 4.\n    \
         Let tg2 be a new Tagged with label 2 and shape r.\n    Show area(tg).\n    Show area(tg2).\n",
    ),
    (
        // STRUCT-WITH-MAP-FIELD PARAMETER — `f(s: Scores)` where `Scores` has a `Map of Int to Int`
        // field. `boundary_of_field_type` now resolves a `Map` field (so the struct is carried), and
        // the `GetField` (`s's table`) re-seeds the result's map value kind (`FieldNested::Map`), so
        // `item k of s's table` resolves. A struct carrying BOTH a map and an enum field also works.
        "struct_map_field_param",
        "## A St is one of:\n    A Open.\n    A Closed with code Int.\n\n\
         ## A Box has:\n    A id: Int.\n    A m: Map of Int to Int.\n    A st: St.\n\n\
         ## To rd (b: Box) -> Int:\n    Inspect b's st:\n        When Open: Return item 1 of b's m.\n        \
         When Closed (code): Return code.\n    Return 0.\n\
         ## Main\n    Let mutable mm be a new Map of Int to Int.\n    Set item 1 of mm to 99.\n    \
         Let o be a new Open.\n    Let b1 be a new Box with id 1 and m mm and st o.\n    \
         Let c be a new Closed with code 7.\n    Let b2 be a new Box with id 2 and m mm and st c.\n    \
         Show rd(b1).\n    Show rd(b2).\n",
    ),
    (
        // STRUCT-WITH-TUPLE-FIELD PARAMETER — `f(h: Holder)` where `Holder` has a homogeneous tuple
        // field (`Pair`/`Triple`). `boundary_of_field_type` maps a homogeneous `Pair`/`Triple` field
        // to `Seq` (like a tuple PARAMETER), so the struct is carried and `item N of h's pr` resolves
        // through the seq element-kind path — no `FieldNested` re-seed needed (the field's `SeqInt`
        // kind carries its element kind). A struct combining a tuple field WITH a map field also works.
        "struct_tuple_field_param",
        "## A Box has:\n    A pr: Pair of Int and Int.\n    A tr: Triple of Int and Int and Int.\n    \
         A m: Map of Int to Int.\n\n\
         ## To go (b: Box) -> Int:\n    \
         Return item 1 of b's pr + item 2 of b's tr + item 1 of b's m.\n\
         ## Main\n    Let mutable mm be a new Map of Int to Int.\n    Set item 1 of mm to 100.\n    \
         Let p be (3, 4).\n    Let t be (7, 8, 9).\n    \
         Let b be a new Box with pr p and tr t and m mm.\n    Show go(b).\n",
    ),
    (
        // MAP RETURN — `f() -> Map of Int to Int`, then `item k of f()` inline. The callee's declared
        // RETURN type is carried in the bytecode (`CompiledFunction.return_type`) and seeded at the
        // `Call` into the result's value kind (the return-side analog of a map PARAMETER's
        // `ParamShape::Map`), so the caller resolves the returned map's value kind cross-region.
        "map_return",
        "## To build () -> Map of Int to Int:\n    Let mutable m be a new Map of Int to Int.\n    \
         Set item 1 of m to 42.\n    Set item 2 of m to 8.\n    Return m.\n\
         ## Main\n    Let m be build().\n    Show item 1 of m + item 2 of m.\n",
    ),
    (
        // ENUM RETURN used INLINE — `f() -> Shape`, then `Inspect f()` directly (not via a parameter).
        // The callee's `return_type` seeds the result's per-variant payload layout at the `Call`, so
        // the inline `BindArm` resolves cross-region. Both a 1-field and a 2-field variant.
        "enum_return_inline",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## To mkc () -> Shape:\n    Return a new Circle with radius 9.\n\
         ## To mkr () -> Shape:\n    Return a new Rectangle with width 3 and height 4.\n\
         ## Main\n    Let s be mkc().\n    Inspect s:\n        When Circle (r): Show r.\n        \
         When Rectangle (w, h): Show w * h.\n    \
         Let s2 be mkr().\n    Inspect s2:\n        When Circle (r): Show r.\n        \
         When Rectangle (w, h): Show w * h.\n",
    ),
    (
        // HETEROGENEOUS TUPLE as PARAMETER + RETURN. A mixed-kind tuple (`Pair of Int and Bool`,
        // `Triple of …`) is one i32 handle to a buffer of 8-byte slots, each at its own kind — like a
        // local heterogeneous tuple, which already lowered (`lower_new_tuple_het`). The gap was
        // cross-region: `BoundaryType::Tuple(elems)` carries the per-position kinds, seeded as
        // `ParamShape::Tuple` (params) / from `return_type` at the `Call` (returns), so a constant
        // `item N of t` resolves its result kind. A homogeneous tuple still resolves to `Seq` instead.
        "het_tuple_param_return",
        "## To at1 (t: Pair of Int and Bool) -> Int:\n    Return item 1 of t.\n\
         ## To at2 (t: Pair of Int and Bool) -> Bool:\n    Return item 2 of t.\n\
         ## To tri (t: Triple of Int and Bool and Int) -> Int:\n    Return item 1 of t + item 3 of t.\n\
         ## To mk () -> Pair of Int and Bool:\n    Return (42, true).\n\
         ## Main\n    Let p be (7, true).\n    Show at1(p).\n    Show at2(p).\n    \
         Let q be (10, false, 5).\n    Show tri(q).\n    \
         Let r be mk().\n    Show item 1 of r.\n    Show item 2 of r.\n",
    ),
    (
        // STRUCT-WITH-HETEROGENEOUS-TUPLE-FIELD parameter — `GetField` on the tuple field re-seeds the
        // result's per-position kinds (`FieldNested::Tuple`), so `item N of b's pr` resolves like a
        // direct heterogeneous-tuple parameter.
        "struct_het_tuple_field",
        "## A Box has:\n    A name: Int.\n    A pr: Pair of Int and Bool.\n\n\
         ## To f (b: Box) -> Int:\n    Return item 1 of b's pr.\n\
         ## Main\n    Let p be (7, true).\n    Let b be a new Box with name 1 and pr p.\n    Show f(b).\n",
    ),
    (
        // COW soundness ACROSS CONTROL FLOW: a struct mutated inside a loop (and conditionally). The
        // uniqueness pass resets at block leaders, so each in-loop `StructInsert` conservatively
        // copies-on-write — never miscompiling, and `p` here is unaliased so the value tracks `i`.
        "cow_loop_mutate",
        "## A P has:\n    An x: Int.\n\n\
         ## Main\n    Let mutable p be a new P with x 0.\n    Let mutable i be 1.\n    \
         While i is at most 3:\n        Set p's x to i.\n        Set i to i + 1.\n    \
         Show p's x.\n",
    ),
    (
        "float_function",
        "## To scale (x: Float) -> Float:\n    Let y be x * 1.5.\n    Return y.\n\
         ## Main\n    Show scale(4.0).\n",
    ),
    (
        // Show(Bool): a comparison result is a Bool (i64 0/1), displayed via print_bool as
        // "true"/"false" — never mis-shown as 1/0.
        "show_bool",
        "## To gt (x: Int) (y: Int) -> Bool:\n    Let b be x is greater than y.\n    Return b.\n\
         ## Main\n    Show gt(5, 3).\n    Show gt(2, 8).\n",
    ),
    (
        // Logical `not` on a Bool, in a condition (exercises Not(Bool)).
        "logical_not",
        "## To describe (x: Int) -> Int:\n    \
         If not (x is greater than 5):\n        Return 0.\n    Return 1.\n\
         ## Main\n    Show describe(3).\n    Show describe(8).\n",
    ),
    (
        // The words `and`/`or` on Ints are LOGICAL — truthiness in, Bool out,
        // compiled as pure short-circuit control flow (`12 and 10` is `true`).
        "logical_and_or_on_ints",
        "## Main\n    Let a be 12.\n    Let b be 10.\n    Let c be a and b.\n    Let d be a or b.\n    Show c.\n    Show d.\n",
    ),
    (
        // The DEDICATED bitwise `&`/`|` operators on Ints (`Op::BitAnd`/`Op::BitOr`,
        // lowered to `i64.and`/`i64.or`) — distinct from the logical `and`/`or`
        // WORDS above. `12 & 10 = 8`, `12 | 10 = 14`.
        "bitand_bitor_symbols",
        "## Main\n    Let a be 12.\n    Let x be a & 10.\n    Let y be a | 10.\n    Show x.\n    Show y.\n",
    ),
    (
        // `sqrt` (f64.sqrt) over a float expression — the nbody/spectral_norm kernel idiom.
        "sqrt_expr",
        "## Main\n    Let x be 3.0.\n    Let y be 4.0.\n    Let d be sqrt(x * x + y * y).\n    Show d.\n",
    ),
    (
        // `sqrt` of an Int converts to f64 first (matching `(n as f64).sqrt()`).
        "sqrt_int",
        "## Main\n    Let r be sqrt(2).\n    Show r.\n",
    ),
    (
        // `floor`/`ceil` of a Float → Int via the SATURATING truncation (matches `as i64`).
        "floor_ceil",
        "## Main\n    Let a be floor(3.7).\n    Let b be ceil(3.2).\n    Show a.\n    Show b.\n",
    ),
    (
        // `abs`: Int (via x<0?-x:x) and Float (f64.abs).
        "abs_int_float",
        "## Main\n    Let x be 0 - 5.\n    Let r be abs(x).\n    Let f be abs(0.0 - 2.5).\n    Show r.\n    Show f.\n",
    ),
    (
        // `round` is round-half-AWAY-from-zero (not wasm's round-half-even): 2.5→3, -2.5→-3.
        "round_half_away",
        "## Main\n    Let a be round(2.5).\n    Let b be round(0.0 - 2.5).\n    Show a.\n    Show b.\n",
    ),
    (
        // `min`/`max` on Ints (i64 compare + select).
        "min_max_int",
        "## Main\n    Let a be min(3, 7).\n    Let b be max(3, 7).\n    Show a.\n    Show b.\n",
    ),
    (
        // `min`/`max` with a Float operand: both sides promote to f64 and lower to `f64.min`/`f64.max`
        // (0xA4/0xA5) behind the NaN guards. A Float-Int mix promotes the Int.
        "min_max_float",
        "## Main\n    Let a be min(3.5, 7.25).\n    Let b be max(3.5, 7.25).\n    Let c be max(2, 0.5).\n    Let d be min(0.0 - 1.5, 4).\n    Show a.\n    Show b.\n    Show c.\n    Show d.\n",
    ),
    (
        // A Real tensor through `matmul` then `softmax`: the softmax desugar subtracts the row max,
        // so a model kernel reaches the Float `max` lowering.
        "softmax_real_tensor",
        "## Main\n    Let Q be a 4 by 8 tensor of Real.\n    Let K be a 8 by 4 tensor of Real.\n    Set item 1 of Q to 2.0.\n    Set item 1 of K to 1.0.\n    Let S be Q matmul K.\n    Let P be softmax of S.\n    Show item 1 of P.\n    Show item 2 of P.\n",
    ),
    (
        // `count_ones(n)` — the population count of an Int's u64 bit pattern (`i64.popcnt`). 12 =
        // 0b1100 → 2; 255 → 8; 0 → 0. Matches the VM's `(n as u64).count_ones()`.
        "count_ones_popcount",
        "## Main\n    Let a be count_ones(12).\n    Let b be count_ones(255).\n    Let c be count_ones(0).\n    Show a.\n    Show b.\n    Show c.\n",
    ),
    (
        // `parseFloat(text)` — parse a Text into a Float via the `parse_float` host (`str::parse::<f64>`
        // after a trim, matching the VM). "3.14" → 3.14, "-2.5" → -2.5.
        "parse_float",
        "## Main\n    Let x be parseFloat(\"3.14\").\n    Let y be parseFloat(\"-2.5\").\n    Show x.\n    Show y.\n",
    ),
    (
        // `parse_timestamp(text) -> Moment` then the calendar/clock component extractors (`the year of
        // m`, …). Each component is computed by the same `logicaffeine_base::temporal` the VM uses, so
        // the whole `2024-03-10T07:30:45Z` decomposition is bit-identical WASM == VM == tree-walker.
        "temporal_components",
        "## Main\n    Let m be parse_timestamp(\"2024-03-10T07:30:45Z\").\n    \
         Show the year of m.\n    Show the month of m.\n    Show the day of m.\n    \
         Show the hour of m.\n    Show the minute of m.\n    Show the second of m.\n    \
         Show the weekday of m.\n    Show the week of m.\n    Show the quarter of m.\n",
    ),
    (
        // The Date (days-since-epoch) analog: the calendar components of `today` (a `Date`, not a
        // `Moment`) go through `temporal_component_date` — `civil_from_days` straight, no nanos
        // round-trip. hour/minute/second don't apply to a Date (the VM errors), so they're excluded
        // here; year/month/day/weekday/week/quarter are bit-identical WASM == VM == tree-walker.
        "temporal_date_components",
        "## Main\n    Show the year of today.\n    Show the month of today.\n    Show the day of today.\n    \
         Show the weekday of today.\n    Show the week of today.\n    Show the quarter of today.\n",
    ),
    (
        // ★ FULL-RANGE LOCK ★ — Date components of FAR-future / FAR-past ISO date literals whose
        // midnight-nanoseconds would OVERFLOW an `i64` (past ~year 2262). A nanos round-trip would
        // silently wrap here; `temporal_component_date` computes straight from `civil_from_days`, so
        // the full `i32` day range stays bit-identical WASM == VM == tree-walker. `1600-02-29` also
        // pins the leap-year path (1600 is a 400-divisible leap year).
        "temporal_date_extreme_range",
        "## Main\n    Let a be 2400-06-15.\n    Show the year of a.\n    Show the month of a.\n    \
         Show the quarter of a.\n    Let b be 9999-12-31.\n    Show the year of b.\n    Show the weekday of b.\n    \
         Let c be 1600-02-29.\n    Show the year of c.\n    Show the day of c.\n    Show the week of c.\n",
    ),
    (
        // A `Date` LITERAL loaded (`LoadConst Constant::Date`) and Shown directly — its `i32`-days
        // materialization (matching the `Kind::Date` local) rendered `YYYY-MM-DD` by `print_date`,
        // bit-identical to `RuntimeValue::Date::to_display_string`. Covers positive days, the epoch
        // (`1970-01-01` = day 0), and a pre-epoch date (negative days).
        "date_literal_show",
        "## Main\n    Show 2024-03-10.\n    Show 1970-01-01.\n    Show 1837-06-20.\n",
    ),
    (
        // `chr(code) -> Text` — a one-character Text, its UTF-8 encoding built inline. Covers all four
        // byte-lengths: `A` (1, U+0041), `é` (2, U+00E9), `€` (3, U+20AC), `😀` (4, U+1F600). Bit-
        // identical to the VM's `char::from_u32(..).to_string()`.
        "chr_utf8_all_lengths",
        "## Main\n    Show chr(65).\n    Show chr(233).\n    Show chr(8364).\n    Show chr(128512).\n",
    ),
    (
        // The `Word32` ℤ/2³² ring (the MD5/SHA-256 substrate): `word32` construct, `word_and`/`word_or`/
        // `word_not` bitwise, `rotl`/`rotr` rotate, `word32Shr` logical right-shift (SHA-256 `σ`),
        // wrapping `+`/`xor`, `intOfWord32` extract. Words print as their UNSIGNED decimal
        // (`0xF0F0F0F0` → 4042322160), so this stresses the unsigned Show path too. Bit-identical to
        // the VM's `WordVal::W32` wrapping arithmetic.
        "word32_ring",
        "## Main\n    Let a be word32(4042322160).\n    Let b be word32(858993459).\n    \
         Show word_and(a, b).\n    Show word_or(a, b).\n    Show word_not(a).\n    \
         Show rotl(a, 7).\n    Show rotr(a, 7).\n    Show word32Shr(a, 8).\n    Show a + b.\n    \
         Show a xor b.\n    Show a * b.\n    Show intOfWord32(a).\n",
    ),
    (
        // The `Word64` ℤ/2⁶⁴ ring (the Keccak/SHA-3/SHA-512 substrate): `word64` construct (incl. a
        // FULL-64-BIT hex constant with the high bit set — `0xB5C0…`, a real SHA-512 round constant),
        // `word64Shl`/`word64Shr`/`word64And`, `rotl`/`rotr`, wrapping `+`/`xor`, `intOfWord64`.
        // Bit-identical to the VM's `WordVal::W64`.
        "word64_ring",
        "## Main\n    Let x be word64(1311768467463790320).\n    Let y be word64(255).\n    \
         Let k be word64(0xB5C0FBCFEC4D3B2F).\n    \
         Show word64And(x, y).\n    Show word64Shl(y, 12).\n    Show word64Shr(x, 8).\n    \
         Show word_or(x, y).\n    Show rotl(x, 11).\n    Show x + y.\n    Show x xor y.\n    \
         Show k.\n    Show word64Shr(k, 60).\n    Show intOfWord64(y).\n",
    ),
    (
        // A `Seq of Word32` — the crypto message-schedule / state array (SHA-256 `W[0..64]`). Build,
        // fill, iterate (wrapping-sum the words), random-access (`item N`), and `length`. The element
        // kind refines to `SeqWord32` from the first `Word32` `Push`; `item N of w` yields a `Word32`.
        // Bit-identical WASM == VM == tree-walker.
        "seq_of_word32",
        "## Main\n    Let mutable w be a new Seq of Word32.\n    Push word32(1000000000) to w.\n    \
         Push word32(2000000000) to w.\n    Push word32(4000000000) to w.\n    \
         Let mutable s be word32(0).\n    Repeat for x in w:\n        Set s to s + x.\n    \
         Show s.\n    Show item 2 of w.\n    Show length of w.\n    \
         Show word_and(item 1 of w, item 3 of w).\n",
    ),
    (
        // A `Seq of Word64` — the Keccak/SHA-512 lane array. Same shape as `seq_of_word32` at 64-bit
        // width (`SeqWord64`, `i64` elements).
        "seq_of_word64",
        "## Main\n    Let mutable v be a new Seq of Word64.\n    Push word64(1311768467463790320) to v.\n    \
         Push word64(255) to v.\n    Let mutable t be word64(0).\n    Repeat for z in v:\n        Set t to t + z.\n    \
         Show t.\n    Show word64Shr(item 1 of v, 8).\n    Show length of v.\n",
    ),
    (
        // A `Seq of Word32` crossing a FUNCTION BOUNDARY (`xs: Seq of Word32` parameter) — the shape
        // every real crypto round function has (`sha256_compress(schedule: Seq of Word32)`). The
        // parameter's declared element type must resolve to `SeqWord32` so `item N of xs` yields a
        // `Word32` cross-region.
        "seq_of_word32_param",
        "## To mix (xs: Seq of Word32) -> Word32:\n    \
         Let mutable acc be word32(0).\n    Repeat for x in xs:\n        Set acc to acc xor x.\n    \
         Return rotl(acc, 5).\n\n\
         ## Main\n    Let mutable w be a new Seq of Word32.\n    Push word32(2863311530) to w.\n    \
         Push word32(1431655765) to w.\n    Show mix(w).\n",
    ),
    (
        // Bit ops: `xor` (BitXor), `shifted left by` (Shl), `shifted right by` (Shr).
        "bit_ops",
        "## Main\n    Let a be 12.\n    Let x be a xor 10.\n    Let y be a shifted left by 2.\n    Let z be a shifted right by 1.\n    Show x.\n    Show y.\n    Show z.\n",
    ),
    (
        // Equality comparisons: `equals` (Eq), `is not` (NotEq) — each a Bool shown true/false.
        "eq_ne",
        "## Main\n    Let x be 5.\n    Let a be x equals 5.\n    Let b be x is not 4.\n    Show a.\n    Show b.\n",
    ),
    (
        // Ordering comparisons: `is at most` (LtEq), `is at least` (GtEq).
        "le_ge",
        "## Main\n    Let x be 5.\n    Let a be x is at most 9.\n    Let b be x is at least 2.\n    Show a.\n    Show b.\n",
    ),
    (
        // `pow` Int^Int → in-module exponentiation-by-squaring (Int result, overflow-trapping).
        "pow_int",
        "## Main\n    Let r be pow(2, 10).\n    Show r.\n",
    ),
    (
        // `pow` with a Float exponent → host `pow_ff` (f64::powf): 2^0.5 = √2.
        "pow_float",
        "## Main\n    Let r be pow(2.0, 0.5).\n    Show r.\n",
    ),
    (
        // `pow` Float^Int → host `pow_fi` (f64::powi, exact repeated multiply): 3^4 = 81.
        "pow_float_int",
        "## Main\n    Let r be pow(3.0, 4).\n    Show r.\n",
    ),
    (
        // Temporal: `today` (LoadToday → Date, host `today`+`print_date`) and `now` (LoadNow →
        // Moment). Deterministic via the fixed clock set at the top of this test.
        "temporal_today_now",
        "## Main\n    Let d be today.\n    Let m be now.\n    Show d.\n    Show m.\n",
    ),
    (
        // Heap foundation: `new Seq of Int` (NewEmptyList → bump-allocated header) + `length of`
        // (Length → the header's len field). Linear memory + the __heap_ptr bump allocator.
        "empty_seq_length",
        "## Main\n    Let s be a new Seq of Int.\n    Show length of s.\n",
    ),
    (
        // Growable array: NewEmptyList + ListPush (realloc-on-push, stable handle) + Index
        // (1-based, bounds-checked) + Length.
        "seq_push_index",
        "## Main\n    Let mutable arr be a new Seq of Int.\n    \
         Push 10 to arr.\n    Push 20 to arr.\n    Push 30 to arr.\n    \
         Show item 2 of arr.\n    Show length of arr.\n",
    ),
    (
        // `Pop from seq into x` (ListPop) — remove + return the last element, the in-place mirror
        // of `ListPush`. The Syntax Guide `push-pop` example verbatim: build with a list literal +
        // pushes, `Show` the whole sequence, then pop and show both the popped value and the shorter
        // sequence. Exercises whole-`SeqInt` display before AND after the length shrink.
        "seq_push_pop",
        "## Main\n    Let numbers be [1, 2, 3].\n    \
         Push 4 to numbers.\n    Push 5 to numbers.\n    Show numbers.\n    \
         Pop from numbers into last.\n    Show last.\n    Show numbers.\n",
    ),
    (
        "pop_behind_its_check",
        "## To dropped (xs: Seq of Int) and (n: Int) -> Int:\n    Let mutable ys be xs.\n    \
         Repeat for i from 1 to n:\n        Pop from ys.\n    Return length of ys.\n\n\
         ## Main\n    Show dropped([4, 5, 6], 2).\n",
    ),
    (
        // Mutable array element write: `Set item i of arr to v` (SetIndex, bounds-checked).
        "seq_set_index",
        "## Main\n    Let mutable arr be a new Seq of Int.\n    \
         Push 10 to arr.\n    Push 20 to arr.\n    \
         Set item 1 of arr to 99.\n    \
         Show item 1 of arr.\n    Show item 2 of arr.\n",
    ),
    (
        // `[a, b, c]` list literal (NewList, unrolled stores).
        "new_list",
        "## Main\n    Let s be [10, 20, 30].\n    Show item 2 of s.\n    Show length of s.\n",
    ),
    (
        // Float sequence: `new Seq of Float` (SeqAny → refined to SeqFloat by the first push) with
        // f64 elements (f64_load/store), shown via print_f64.
        "seq_float",
        "## Main\n    Let mutable arr be a new Seq of Float.\n    \
         Push 1.5 to arr.\n    Push 2.5 to arr.\n    \
         Show item 2 of arr.\n    Show length of arr.\n",
    ),
    (
        // Realistic array program (the array_fill/prefix_sum shape, sans argv): build [0,1,4,9,16]
        // with a While+Push loop, then sum it with a While+Index loop. Whole heap Int-seq model.
        "array_build_and_sum",
        "## Main\n    Let mutable arr be a new Seq of Int.\n    Let mutable i be 0.\n    \
         While i is less than 5:\n        Push i * i to arr.\n        Set i to i + 1.\n    \
         Let mutable total be 0.\n    Set i to 1.\n    \
         While i is at most 5:\n        Set total to total + item i of arr.\n        Set i to i + 1.\n    \
         Show total.\n",
    ),
    (
        // `Repeat for x in seq` over an Int sequence: IterPrepare snapshots, IterNext loads each
        // element (i64) and advances, IterPop drops the frame. Two sequential loops, with a Push
        // inside the first (the snapshot of `items` is unaffected by growth of `doubled`).
        "repeat_for_in",
        "## Main\n    Let mutable items be a new Seq of Int.\n    \
         Push 1 to items.\n    Push 2 to items.\n    Push 3 to items.\n    \
         Let mutable doubled be a new Seq of Int.\n    \
         Repeat for x in items:\n        Push x * 2 to doubled.\n    \
         Let mutable total be 0.\n    \
         Repeat for y in doubled:\n        Set total to total + y.\n    \
         Show total.\n",
    ),
    (
        // `Repeat for i from X to Y` over Int bounds: the counted loop (`optimize::iteration`,
        // `Fact::IntRange`), its bounds read once and no range value built.
        "repeat_range",
        "## Main\n    Let mutable total be 0.\n    \
         Repeat for i from 1 to 5:\n        Set total to total + i.\n    \
         Show total.\n",
    ),
    (
        // A range whose end a call computes, which the counted-range proof does not decide: NewRange
        // materializes [1..=6] and the iteration ops walk it.
        "repeat_range_call_bound",
        "## To twice (n: Int) -> Int:\n    Return n + n.\n\n## Main\n    Let mutable total be 0.\n    \
         Repeat for i from 1 to twice(3):\n        Set total to total + i.\n    \
         Show total.\n",
    ),
    (
        // `Repeat for x in seq` over a Float sequence: IterNext loads each element as f64 (the
        // loop variable's kind comes from the iterable's element kind via the prepare↔next pair).
        "repeat_float",
        "## Main\n    Let mutable arr be a new Seq of Float.\n    \
         Push 1.5 to arr.\n    Push 2.5 to arr.\n    \
         Let mutable total be 0.0.\n    \
         Repeat for x in arr:\n        Set total to total + x.\n    \
         Show total.\n",
    ),
    (
        // `Show seq` — whole-sequence display via the print_seq_* host sinks (host reads the
        // header + buffer out of exported memory and formats `[e0, …]` like the tree-walker). An
        // Int seq, an empty seq (`[]`), and a Float seq cover all three element-kind sinks.
        "show_seq",
        "## Main\n    Let mutable arr be a new Seq of Int.\n    \
         Push 10 to arr.\n    Push 20 to arr.\n    Push 30 to arr.\n    Show arr.\n    \
         Let mutable empty be a new Seq of Int.\n    Show empty.\n    \
         Let mutable fs be a new Seq of Float.\n    \
         Push 1.5 to fs.\n    Push 2.5 to fs.\n    Show fs.\n",
    ),
    (
        // `seq contains value` — a linear membership scan over a sequence (value equality), used
        // both ways (a hit and a miss) as `If` conditions. Exercises `Contains` → Bool.
        "seq_contains",
        "## Main\n    Let mutable arr be a new Seq of Int.\n    \
         Push 10 to arr.\n    Push 20 to arr.\n    Push 30 to arr.\n    \
         Let mutable hits be 0.\n    \
         If arr contains 20:\n        Set hits to hits + 1.\n    \
         If arr contains 99:\n        Set hits to hits + 100.\n    \
         Show hits.\n",
    ),
    (
        // Label-sensitive Int/Bool register reuse: a `contains` Bool drives an `If`, then an Int
        // literal is `Show`n — the allocator reuses one i64 register for both, and the two differ
        // ONLY by display label (print_bool vs print_i64), which kind inference cannot unify and
        // once REJECTED. Register live-range splitting ([`vm::wasm::regsplit`]) gives the Bool web
        // and each Int web their own local, so this compiles directly (no accumulator workaround).
        // Multiple Bool→Int flips plus a trailing Int stress the per-web label consistency.
        "label_split_int_bool",
        "## Main\n    Let mutable arr be a new Seq of Int.\n    \
         Push 10 to arr.\n    Push 20 to arr.\n    \
         If arr contains 10:\n        Show 1.\n    Otherwise:\n        Show 2.\n    \
         If arr contains 99:\n        Show 3.\n    Otherwise:\n        Show 4.\n    \
         Show 99.\n",
    ),
    (
        // `items i through j of seq` — a 1-based inclusive subsequence (SliceOp). A mid slice and
        // an out-of-range slice (→ empty `[]`) cover the in-range copy and the saturating bounds.
        "seq_slice",
        "## Main\n    Let mutable arr be a new Seq of Int.\n    \
         Push 10 to arr.\n    Push 20 to arr.\n    Push 30 to arr.\n    Push 40 to arr.\n    \
         Let sub be items 2 through 3 of arr.\n    Show sub.\n    Show length of sub.\n    \
         Let sub2 be items 2 through 10 of arr.\n    Show sub2.\n",
    ),
    (
        // `xs followed by ys` — concatenate two sequences into a fresh one (SeqConcat). Exercises
        // the two-source copy (lhs's elements, then rhs's appended after).
        "seq_concat",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 1 to xs.\n    Push 2 to xs.\n    \
         Let mutable ys be a new Seq of Int.\n    Push 3 to ys.\n    Push 4 to ys.\n    Push 5 to ys.\n    \
         Let zs be xs followed by ys.\n    Show zs.\n    Show length of zs.\n",
    ),
    (
        // Text literals → fresh UTF-8 `Text` objects in linear memory (bump-allocated, byte-store
        // chunked), displayed via the print_text host sink. A short string, a multi-chunk string
        // (> 8 bytes), a bound variable, an empty string, and `length` (BYTE count, incl. a 2-byte
        // UTF-8 char) cover the representation end to end.
        "text_basics",
        "## Main\n    Show \"hello\".\n    Show \"a longer string here\".\n    \
         Let g be \"world\".\n    Show g.\n    Show length of g.\n    \
         Show \"\".\n    Show length of \"caf\u{e9}\".\n",
    ),
    (
        // `a combined with b` — byte concatenation of two Text values into a fresh Text (Concat).
        // A normal concat and one with an empty operand cover the two-source byte copy.
        "text_concat",
        "## Main\n    Let g be \"foo\" combined with \"bar\".\n    Show g.\n    Show length of g.\n    \
         Let h be \"\" combined with \"x\".\n    Show h.\n",
    ),
    (
        // Interpolating an Int into a string: `Concat` stringifies the Int operand via the host
        // `fmt_i64_into` (exact `to_display_string`). Both `{n}` interpolation and explicit
        // `combined with`, with the Int on either side and a negative value.
        "text_interp",
        "## Main\n    Let total be 100.\n    Show \"total is \" combined with total.\n    \
         Show \"count: {total}\".\n    \
         Let neg be 0 - 7.\n    Show neg combined with \" items\".\n",
    ),
    (
        // Interpolating a Float and a Bool: `Concat` stringifies via the host `fmt_f64_into`
        // (the `{:.6}`-trimmed display) and `fmt_bool_into` (`true`/`false`).
        "text_interp_float_bool",
        "## Main\n    Let pi be 3.14.\n    Show \"pi is \" combined with pi.\n    \
         Let flag be 1 is less than 2.\n    Show \"flag: \" combined with flag.\n",
    ),
    (
        // A FORMATTED interpolation piece — `"{x:.N}"` renders `x` to N decimal places via
        // `apply_format_spec` (`format!("{:.N}", val)`), the fixed-precision output the numeric
        // benchmarks (nbody `{e:.9}`, pi_leibniz `{result:.15}`) print. A whole-number and a
        // fraction, plus an Int formatted with a precision (rendered as a float).
        "text_format_precision",
        "## Main\n    Let x be 3.14159.\n    Show \"{x:.2}\".\n    Show \"{x:.4}\".\n    \
         Let y be 2.0.\n    Show \"{y:.3}\".\n    Let n be 7.\n    Show \"{n:.1}\".\n",
    ),
    (
        // TUPLE DESTRUCTURING — `Repeat for (a, b) in <seq of pairs>` yields each pair (a homogeneous
        // 2-tuple riding a SeqInt) and `DestructureTuple` binds `a`/`b` to its two slots. Sums a*b over
        // the pairs: 10·1 + 20·2 + 30·3 = 140.
        "destructure_tuple_loop",
        "## Main\n    Let pairs be [(10, 1), (20, 2), (30, 3)].\n    Let mutable sum be 0.\n    \
         Repeat for (a, b) in pairs:\n        Set sum to sum + a * b.\n    Show sum.\n",
    ),
    (
        // Text equality (`equals` / `is not`) — a byte compare returning Bool, used as `If`
        // conditions: a hit, a miss, and a `!=` that holds.
        "text_equality",
        "## Main\n    Let name be \"admin\".\n    Let mutable hits be 0.\n    \
         If name equals \"admin\":\n        Set hits to hits + 1.\n    \
         If name equals \"guest\":\n        Set hits to hits + 100.\n    \
         If name is not \"root\":\n        Set hits to hits + 10.\n    \
         Show hits.\n",
    ),
    (
        // `a copy of x` (DeepClone) — an INDEPENDENT deep copy: mutating the clone must not be
        // seen through the original. Cloning a sequence then pushing to the clone proves the fresh
        // buffer; cloning a Text proves the byte copy.
        "deep_clone",
        "## Main\n    Let mutable a be a new Seq of Int.\n    Push 1 to a.\n    Push 2 to a.\n    \
         Let mutable b be copy of a.\n    Push 3 to b.\n    \
         Show a.\n    Show b.\n    \
         Let s be \"hi\".\n    Let t be copy of s.\n    Show t.\n",
    ),
    (
        // Structs/records — `a new T with f v and …` (NewStruct + StructInsert per field) and
        // `obj's field` access (GetField). Two types, with a HETEROGENEOUS struct (a Text field
        // alongside Int fields) exercising the mixed-width slots (i32 handle vs i64) in one object.
        // The runtime struct is a flat slot array; field names are resolved to slots at compile
        // time by the static layout analysis (which follows the `Move` of the struct handle).
        "struct_basics",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## A Person has:\n    A name: Text.\n    An age: Int.\n\n\
         ## Main\nLet p be a new Point with x 3 and y 4.\nShow p's x.\nShow p's y.\n\
         Let bob be a new Person with name \"Bob\" and age 30.\nShow bob's name.\nShow bob's age.\n",
    ),
    (
        // `Map of Int to Int` — insert (`Set item k of m to v`), update an existing key, get
        // (`item k of m`), `length`, and `contains` (a hit and a miss). These are order-independent
        // so they are byte-identical to the VM's hashmap; the linear-scan map reallocs on each new
        // key (like ListPush). Whole-map `Show`/iteration (hashmap order) stays deferred.
        "map_basics",
        "## Main\n    Let mutable m be a new Map of Int to Int.\n    \
         Set item 5 of m to 10.\n    Set item 7 of m to 20.\n    Set item 5 of m to 99.\n    \
         Set item 9 of m to 40.\n    Remove 9 from m.\n    \
         Show item 5 of m.\n    Show item 7 of m.\n    Show length of m.\n    \
         Let mutable hits be 0.\n    \
         If m contains 7:\n        Set hits to hits + 1.\n    \
         If m contains 9:\n        Set hits to hits + 100.\n    \
         Show hits.\n",
    ),
    (
        // `Set of Int` — add (`Add v to s`, dedup), length, contains (hit + miss). Order-
        // independent → byte-identical to the VM's hashset; linear-scan, reallocs on each new value.
        "set_basics",
        "## Main\n    Let mutable s be a new Set of Int.\n    \
         Add 5 to s.\n    Add 7 to s.\n    Add 5 to s.\n    Remove 7 from s.\n    Show length of s.\n    \
         Let mutable hits be 0.\n    \
         If s contains 5:\n        Set hits to hits + 1.\n    \
         If s contains 7:\n        Set hits to hits + 100.\n    \
         Show hits.\n",
    ),
    (
        // Enums / pattern matching — a nullary-variant enum (`NewInductive` count 0, tag =
        // constructor) matched with `Inspect`/`If it is V` (`TestArm`, tag compared by i32.eq). The
        // chosen variant + an Otherwise fall-through exercise the tag dispatch.
        "enum_basics",
        "## A Color is one of:\n    A Red.\n    A Green.\n    A Blue.\n\n\
         ## Main\nLet c be Green.\n    Let mutable result be 0.\n\
         Inspect c:\n    If it is Red:\n        Set result to 1.\n    \
         If it is Green:\n        Set result to 2.\n    Otherwise:\n        Set result to 3.\n    \
         Show result.\n",
    ),
    (
        // `Map of Text to Int` — string keys compared by byte-equality in the linear scan (each
        // `\"alice\"` literal is a distinct buffer, so identity won't do). Insert, update an existing
        // string key, get, length, contains (hit + miss).
        "text_map",
        "## Main\n    Let mutable m be a new Map of Text to Int.\n    \
         Set item \"alice\" of m to 30.\n    Set item \"bob\" of m to 25.\n    Set item \"alice\" of m to 31.\n    \
         Show item \"alice\" of m.\n    Show item \"bob\" of m.\n    Show length of m.\n    \
         Let mutable hits be 0.\n    \
         If m contains \"bob\":\n        Set hits to hits + 1.\n    \
         If m contains \"carol\":\n        Set hits to hits + 100.\n    \
         Show hits.\n",
    ),
    (
        // Set algebra — `a union b` and `a intersection b` each build a fresh Set by linear scan +
        // dedup. The VM's Set is an insertion-ordered `Vec`, so the result's contents AND order are
        // deterministic and byte-identical (no hashset nondeterminism). Verified via `length of` and
        // membership (`contains`) of the combined sets, not by showing the set itself.
        "set_algebra",
        "## Main\n    Let mutable a be a new Set of Int.\n    Let mutable b be a new Set of Int.\n    \
         Add 1 to a.\n    Add 2 to a.\n    Add 3 to a.\n    \
         Add 3 to b.\n    Add 4 to b.\n    Add 5 to b.\n    \
         Let u be a union b.\n    Let i be a intersection b.\n    \
         Show length of u.\n    Show length of i.\n    \
         Let mutable hits be 0.\n    \
         If u contains 4:\n        Set hits to hits + 1.\n    \
         If i contains 3:\n        Set hits to hits + 10.\n    \
         If i contains 1:\n        Set hits to hits + 100.\n    \
         Show hits.\n",
    ),
    (
        // WHOLE-COLLECTION stringify in a `+`/`format` — the Syntax Guide `example-filter` +
        // `set-operations` + `stdlib-example` shapes. `"…" + seq` (a `Seq of Int`) and `"…" + set`
        // (a `Set of Int`) stringify the collection via its host formatter (`[e0, …]` / `{e0, …}`),
        // and `format(x)` materializes a scalar as Text — all `RuntimeValue::…to_display_string`.
        "collection_stringify",
        "## Main\n    Let data be [-2, 5, -1, 8, 3, -4, 7].\n    Let positives be a new Seq of Int.\n    \
         Repeat for n in data:\n        If n is greater than 0:\n            Push n to positives.\n    \
         Show \"Positives: \" + positives.\n    \
         Let mutable a be a new Set of Int.\n    Add 1 to a.\n    Add 2 to a.\n    Add 3 to a.\n    \
         Show \"Set: \" + a.\n    \
         Show \"len = \" + format(length of positives).\n    Show \"abs = \" + format(abs(-42)).\n",
    ),
    (
        // SET OF TEXT — the Syntax Guide `set-remove` example. Add/Remove/Contains dedup + compare by
        // BYTE equality (the elements are Text HANDLES; two `"red"` literals are distinct handles with
        // equal bytes), and the `{s0, s1, …}` display (`print_set_text`, insertion order) is
        // byte-identical to `RuntimeValue::Set`. A duplicate add is a no-op; a removed element drops.
        "set_of_text",
        "## Main\n    Let colors be a new Set of Text.\n    \
         Add \"red\" to colors.\n    Add \"green\" to colors.\n    Add \"green\" to colors.\n    Add \"blue\" to colors.\n    \
         Show length of colors.\n    Show colors.\n    \
         Remove \"green\" from colors.\n    Show colors.\n    \
         If colors contains \"red\":\n        Show \"has red\".\n",
    ),
    (
        // MEMORY ZONE — `Inside a zone …` is a SEMANTICALLY TRANSPARENT scope: the tree-walker binds
        // the zone name to `Nothing` and runs the body (the arena/size is a memory-layout hint, not a
        // semantic change), and swallows a `Return`/`Break` escaping it. The AOT runs the body
        // identically (the dead `Nothing` name binding materializes as a dummy). Guide `zone-basic` +
        // `zone-sized-kb` shapes: a seq-building body + a sized scalar body, both output-identical.
        "memory_zone_scope",
        "## Main\n    Inside a zone called \"WorkSpace\":\n        Let temp be [1, 2, 3, 4, 5].\n        Show temp.\n    \
         Show \"Zone freed!\".\n    \
         Inside a zone called \"SmallArena\" of size 64 KB:\n        Let x be 42.\n        Let y be 100.\n        Show x + y.\n",
    ),
    (
        // SECURITY POLICY (`Check … is/can …`) — the `## Policy` condition is resolved from the
        // registry and compiled INLINE (struct-field access + byte compare + and/or), trapping when
        // false. The Syntax Guide `security-predicate` + `security-capability` shapes: a Text-field
        // predicate, and a capability whose OR combines a recursive predicate call with a cross-field
        // (`the user's name equals the document's owner`) compare. Both checks PASS → the `Show`s run.
        "security_policy_checks",
        "## Definition\nA User has:\n    a name: Text.\n    a role: Text.\n\n\
         A Document has:\n    an owner: Text.\n\n\
         ## Policy\nA User is admin if the user's role equals \"admin\".\n\
         A User can edit the Document if:\n    The user is admin, OR\n    The user's name equals the document's owner.\n\n\
         ## Main\n    Let root be a new User with name \"Root\" and role \"admin\".\n    Check that root is admin.\n    Show \"admin ok\".\n    \
         Let alice be a new User with name \"Alice\" and role \"editor\".\n    Let doc be a new Document with owner \"Alice\".\n    \
         Check that alice can edit doc.\n    Show \"edit ok\".\n",
    ),
    (
        // SINGLE-REPLICA SHARED (CRDT) STRUCT — a `Profile is Shared` with `LastWriteWins` fields. With
        // ONE replica and no merge, an LWW register is just its last-written value, so the Shared struct
        // compiles + runs like a plain struct (`Set p's name …` → field write; `Show p's name` → field
        // read) — byte-identical to the tree-walker. (Multi-replica MERGE stays deferred: it needs the
        // CRDT runtime.) The empty LWW field is `Nothing`-initialized, exercising the dead-`Nothing` path.
        "shared_struct_lww_single_replica",
        "## A Profile is Shared and has:\n    a name, which is LastWriteWins of Text.\n    a score, which is LastWriteWins of Int.\n\n\
         ## Main\n    Let p be a new Profile.\n    Set p's name to \"Alice\".\n    Set p's score to 100.\n    Show p's name.\n    Show p's score.\n",
    ),
    (
        // SINGLE-REPLICA `Tally` COUNTER (`CrdtBump`) — `Increase/Decrease <c>'s <points> by <n>` on a
        // one-replica `Shared` struct bumps THIS replica's own slot in the per-replica counter object
        // (`Increase` → `inc`, `Decrease` → `dec`); `Show c's points` projects to `Σinc − Σdec`, byte-
        // identical to the tree-walker. A `Tally` (PN-counter) is used BECAUSE this program DECREMENTS: a
        // grow-only `ConvergentCount` rejects a decrement ("use a Tally to decrement") on every tier, so
        // `Tally` is the correct type here. Guide `crdt-basic` shape + a decrement, showing the running
        // total. (Multi-replica MERGE is exercised by `crdt_counter_merge`, an idempotent per-replica MAX.)
        "crdt_single_replica_counter",
        "## A Counter is Shared and has:\n    a points, which is Tally.\n\n\
         ## Main\n    Let c be a new Counter.\n    Increase c's points by 10.\n    Show c's points.\n    \
         Increase c's points by 5.\n    Show c's points.\n    Decrease c's points by 3.\n    Show c's points.\n",
    ),
    (
        // CRDT COUNTER MERGE (`CrdtMerge`) — `Merge <remote> into <local>` of two counter `Shared`
        // structs sums each plain-Int counter field (`crdt_merge_field`), so local(100) ⊔ remote(50) =
        // 150. The Syntax Guide `crdt-merge` shape — byte-identical to the tree-walker's field-wise join.
        "crdt_counter_merge",
        "## A Stats is Shared and has:\n    a views, which is ConvergentCount.\n\n\
         ## Main\n    Let local be a new Stats.\n    Increase local's views by 100.\n    \
         Let remote be a new Stats.\n    Increase remote's views by 50.\n    \
         Merge remote into local.\n    Show local's views.\n",
    ),
    (
        // SINGLE-REPLICA CRDT COLLECTIONS — a `Shared` struct's CRDT field is a live `NewCrdt`, which
        // one-replica IS its underlying collection: a Divergent register (`CrdtResolve` = field
        // overwrite → `Text`), and an RGA `SharedSequence` (`CrdtAppend` = in-place `list_push`, NO
        // COW since the CRDT field is mutable-shared → `Show length`). Guide `crdt-divergent` +
        // `crdt-sequence`/`crdt-collaborative` shapes — byte-identical to the tree-walker.
        "crdt_register_and_sequence",
        "## A WikiPage is Shared and has:\n    a title, which is Divergent Text.\n\
         ## A Document is Shared and has:\n    a lines, which is a SharedSequence of Text.\n\n\
         ## Main\n    Let mutable page be a new WikiPage.\n    Set page's title to \"Draft\".\n    Show page's title.\n    \
         Resolve page's title to \"Final\".\n    Show page's title.\n    \
         Let mutable doc be a new Document.\n    Append \"Line 1\" to doc's lines.\n    Append \"Line 2\" to doc's lines.\n    \
         Show length of doc's lines.\n",
    ),
    (
        // ORACLE HOIST GUARD — a function whose `Int` bound `n` is DISCONNECTED from the `Seq` param's
        // length, so static bounds-proof can't apply but the SPECULATIVE hoist can: the compiler emits
        // `RegionBoundsGuard` (native-region metadata, a no-op in the AOT) alongside `IndexUnchecked`
        // (lowered as the checked `Index`). The caller's `/ 4` → `DivPow2` and `/ 7` / `% 7` → `MagicDivU`
        // (the magic reciprocal) exercise the Oracle's division forms — all in one program.
        "oracle_hoist_and_division",
        "## To sumdiv (xs: Seq of Int, n: Int) -> Int:\n    Let mutable t be 0.\n    Let mutable i be 1.\n    \
         While i is at most n:\n        Set t to t + item i of xs.\n        Set i to i + 1.\n    Return t.\n\n\
         ## Main\n    Let mutable xs be a new Seq of Int.\n    Let mutable b be 1.\n    While b is at most 8:\n        \
         Push b * b to xs.\n        Set b to b + 1.\n    Let s be sumdiv(xs, 5).\n    \
         Show s.\n    Show s / 4.\n    Show s / 7.\n    Show s % 7.\n",
    ),
    (
        // `Let r: Rational be a / b` — division in a `Rational` context (`ExactDiv`): the AOT builds an
        // exact reduced Rational and `Show`s it `num/den`. A whole quotient (`6 / 2`) reduces to `den == 1`
        // and shows just `3` (matching the VM's downsize to Int); `10 / 4` reduces to `5/2`.
        "exact_div_rational",
        "## Main\n    Let a: Rational be 7 / 2.\n    Show a.\n    Let b: Rational be 6 / 2.\n    Show b.\n    \
         Let c: Rational be 10 / 4.\n    Show c.\n",
    ),
    (
        // `Push x to obj's field` — the DIRECT struct-field-seq push (`ListPushField`): the target is a
        // `FieldAccess`, so the compiler resolves the field then pushes in place. The AOT resolves the
        // field slot + element kind and pushes through the shared amortized path.
        "struct_field_push",
        "## A Bag has:\n    a things, which is a Seq of Int.\n\n\
         ## Main\n    Let mutable bag be a new Bag.\n    Push 10 to bag's things.\n    Push 20 to bag's things.\n    \
         Push 30 to bag's things.\n    Show length of bag's things.\n",
    ),
    (
        // SHAREDSET (OR-Set CRDT) FIELD — a `SharedSet of Text` field is MUTABLE-SHARED (`CrdtSetText`,
        // non-cow-clonable), so `Add`/`Remove` on `p's guests` mutate the shared field IN PLACE (a
        // value-semantic `SetText` would clone the `GetField` result and lose the add). Single-replica
        // add/remove/contains/length/show are byte-identical to the VM. Guide `crdt-sharedset` + a
        // whole-set `Show` (`crdt-sharedset-bias` shape). This is the retain/release-placement fix.
        "crdt_sharedset_field",
        "## A Party is Shared and has:\n    a guests, which is a SharedSet of Text.\n\n\
         ## Main\n    Let mutable p be a new Party.\n    Add \"Alice\" to p's guests.\n    Add \"Bob\" to p's guests.\n    Add \"Bob\" to p's guests.\n    \
         Remove \"Alice\" from p's guests.\n    Show length of p's guests.\n    Show p's guests.\n    \
         If p's guests contains \"Bob\":\n        Show \"has bob\".\n",
    ),
    (
        // Enum constructors WITH payloads + `BindArm` extraction. A `Circle` (1 arg) and a
        // `Rectangle` (2 args) are laid out as tag@0 + args@8,16; `Inspect`/`When V (binds)` matches
        // the tag (`TestArm`) then binds each argument by position (`BindArm`, offset 8*(1+index)).
        // The bound payload's kind is traced from the construction site (the enum-payload analog of a
        // struct field's kind). Accumulate so the binds feed arithmetic, not a literal Show.
        "enum_payload",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## Main\nLet c be a new Circle with radius 10.\n\
         Let r be a new Rectangle with width 3 and height 4.\n\
         Let mutable total be 0.\n\
         Inspect c:\n    When Circle (rad): Set total to total + rad.\n    When Rectangle (w, h): Set total to total + w.\n\
         Inspect r:\n    When Circle (rad): Set total to total + rad.\n    When Rectangle (w, h): Set total to total + w * h.\n\
         Show total.\n",
    ),
    (
        // Homogeneous tuple `(…)` — built like a list buffer (`NewTuple`), then read positionally
        // via `item N of t` and 1-based bracket `t[N]` (both `Index`) and `length of t`. Byte-
        // identical to the VM since a same-kind tuple and a sequence share their memory layout.
        "tuple_index",
        "## Main\nLet t be (10, 20, 30).\n\
         Show length of t.\n\
         Show item 2 of t.\n\
         Let sum be t[1] + t[2] + t[3].\n\
         Show sum.\n",
    ),
    (
        // No-capture closures — `(n: Int) -> …` builds a `MakeClosure` (a heap object holding the
        // body's function index), and `f(x)` is a `CallValue` that loads the index and
        // `call_indirect`s through the module's function table. Both an Int-returning and a
        // Bool-returning closure (so the result-kind inference, traced from the construction site,
        // is exercised for `Show`'s sink choice).
        "closure_basic",
        "## Main\nLet doubler be (n: Int) -> n * 2.\n\
         Let add be (a: Int, b: Int) -> a + b.\n\
         Let isPositive be (n: Int) -> n > 0.\n\
         Show doubler(5).\n\
         Show add(3, 7).\n\
         Show isPositive(5).\n\
         Show isPositive(0 - 3).\n",
    ),
    (
        // Capturing closures — `(n) -> n + offset` closes over an enclosing binding. The captured
        // VALUE is snapshotted into the closure object at `MakeClosure` and passed to the body as a
        // trailing parameter (alongside a present-flag); the body's snapshot-vs-live `GlobalGet`
        // dual path lowers from ordinary ops. Single-capture then double-capture.
        "closure_capture",
        "## Main\nLet offset be 10.\n\
         Let addOffset be (n: Int) -> n + offset.\n\
         Show addOffset(5).\n\
         Let base be 100.\n\
         Let scale be 3.\n\
         Let transform be (n: Int) -> base + n * scale.\n\
         Show transform(5).\n",
    ),
    (
        // CLOSURE OVER A COMPOSITE — a closure capturing a `Seq` (`item i of xs`, `length of xs`).
        // The captured value is a heap HANDLE (i32), not an Int: each capture's VALUE slot is now
        // stored / loaded / typed at the captured global's own kind (via `capture_valtype`), so the
        // closure body's seeded signature, `MakeClosure`'s store, and `CallValue`'s load agree on i32.
        // A `Seq` capture is self-describing (its element kind rides the `SeqInt` kind); capturing a
        // scalar alongside it still works.
        "closure_capture_seq",
        "## Main\nLet xs be [10, 20, 30].\n\
         Let pick be (i: Int) -> item i of xs.\n\
         Let sized be (n: Int) -> n + length of xs.\n\
         Let off be 100.\n\
         Let both be (i: Int) -> item i of xs + off.\n\
         Show pick(2).\n    Show sized(5).\n    Show both(3).\n",
    ),
    (
        // CLOSURE OVER A COMPOSITE WITH A SHAPE — capturing a STRUCT (`p's x`) and a MAP (`item k of
        // m`). A `Seq` capture self-describes via its kind, but a struct/map/enum capture also needs
        // its SHAPE (field layout / value kind / variant layout). The compiler resolves each promoted
        // GLOBAL's type from its constructor (`CompiledProgram.global_types`, via `boundary_of_value_expr`)
        // and the AOT seeds the capture's shape through the shared `boundary_to_param_shape` — the
        // capture analog of a composite PARAMETER. A closure mixing a struct and a map capture works.
        "closure_capture_composite",
        "## A Pt has:\n    An x: Int.\n    A y: Int.\n\n\
         ## Main\nLet p be a new Pt with x 3 and y 4.\n\
         Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 99.\n    Set item 2 of m to 8.\n\
         Let getx be (n: Int) -> n + p's x.\n\
         Let lookup be (k: Int) -> item k of m.\n\
         Let mixed be (k: Int) -> p's y + item k of m.\n\
         Show getx(10).\n    Show lookup(1).\n    Show mixed(2).\n",
    ),
    (
        // CLOSURE OVER AN ENUM + a HETEROGENEOUS TUPLE. The enum's shape comes from its `NewVariant`
        // constructor (`global_types`), so a closure with a BLOCK body (`(n) ->:`) can `Inspect` the
        // captured enum (its `BindArm` resolves via the seeded variant layout). A het-tuple LITERAL
        // global resolves through `boundary_of_value_expr`'s tuple+literal handling → `BoundaryType::
        // Tuple`, so a captured `(10, true, 5)`'s constant `item N` resolves its per-position kind.
        "closure_capture_enum_tuple",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## Main\nLet s be a new Circle with radius 7.\nLet t be (10, true, 5).\n\
         Let area be (n: Int) ->:\n    Inspect s:\n        When Circle (r): Return r + n.\n        \
         When Rectangle (w, h): Return w * h.\n    Return 0.\n\
         Let pick be (n: Int) -> n + item 1 of t + item 3 of t.\n\
         Show area(100).\n    Show pick(0).\n",
    ),
    (
        // CROSS-SCOPE CLOSURE CAPTURE — a closure built INSIDE a FUNCTION over that function's local /
        // parameter (no global index). The capture kind/shape comes from where the closure is BUILT:
        // the planner plans non-closure regions first, reads each `MakeClosure`'s capture register
        // kinds (and a local-built struct's field layout) from the enclosing plan, then plans the
        // closure bodies with them. `pick` closes over a `Seq` PARAMETER, `getx` over a locally-built
        // STRUCT — both function-local, resolved like global captures.
        "closure_capture_local",
        "## A Pt has:\n    An x: Int.\n    A y: Int.\n\n\
         ## To run (xs: Seq of Int) -> Int:\n    Let p be a new Pt with x 100 and y 5.\n    \
         Let pick be (i: Int) -> item i of xs.\n    Let getx be (n: Int) -> n + p's x.\n    \
         Return pick(1) + pick(2) + getx(3).\n\
         ## Main\n    Let ys be [10, 20, 30].\n    Show run(ys).\n",
    ),
    (
        // CAPTURE OF A FUNCTION-PARAMETER COMPOSITE — a closure inside a function closing over that
        // function's struct / map PARAMETER. The capture register is a `Move` of the param, so its
        // shape comes from the enclosing plan's unified `reg_shape` (`Move`-aliased), built from the
        // resolved param-seed tracks — covering struct field layout AND map value kind. So `p's x`
        // and `item k of m` resolve in the closure body though `p`/`m` are parameters, not built here.
        "closure_capture_param_composite",
        "## A Pt has:\n    An x: Int.\n    A y: Int.\n\n\
         ## To run (p: Pt, m: Map of Int to Int) -> Int:\n    \
         Let getx be (n: Int) -> n + p's x.\n    Let lookup be (k: Int) -> item k of m.\n    \
         Return getx(10) + lookup(1).\n\
         ## Main\n    Let q be a new Pt with x 100 and y 5.\n    \
         Let mutable mm be a new Map of Int to Int.\n    Set item 1 of mm to 7.\n    \
         Show run(q, mm).\n",
    ),
    (
        // CAPTURE OF A FUNCTION-LOCAL-BUILT COMPOSITE — closures over a map, an enum, and a
        // heterogeneous tuple all CONSTRUCTED in the function (not parameters). Their shapes need the
        // inferred register kinds (the map's value kind, the tuple's positions), so the plan's
        // `reg_shape` is COMPLETED post-inference (`complete_reg_shape`) from the exposed
        // `map_set_value`/`tuple_layouts`/`ind_type_of` tracks; the enum block-body closure `Inspect`s
        // the captured local enum. Completes composite captures across every source.
        "closure_capture_local_built",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## To run () -> Int:\n    Let mutable m be a new Map of Int to Int.\n    Set item 1 of m to 42.\n    \
         Let s be a new Circle with radius 9.\n    Let t be (10, true, 5).\n    \
         Let lookup be (k: Int) -> item k of m.\n    \
         Let pick be (n: Int) -> n + item 1 of t + item 3 of t.\n    \
         Let area be (n: Int) ->:\n        Inspect s:\n            When Circle (r): Return r + n.\n            \
         When Rectangle (w, h): Return w * h.\n        Return 0.\n    \
         Return lookup(1) + pick(0) + area(100).\n\
         ## Main\n    Show run().\n",
    ),
    (
        // NESTED CLOSURE — a closure DEFINED INSIDE another closure's body. The inner closure's body is
        // emitted INLINE in its parent region (the parent jumps over it; it is reached only through the
        // closure call into its own separate function), so that inline copy is statically UNREACHABLE in
        // the parent — `infer_result` skips its `Return` (via the threaded `pc_reach`) or it would poison
        // the parent's result kind. A nested closure also crosses two deferral levels (the inner's
        // captures need the outer planned; the outer's result needs the inner planned), which the
        // FIXPOINT planner converges. `outer` returns a PURE inner-closure result (`Return inner(i)`) —
        // the exact shape the unreachable-inline-`Return` fix unblocks.
        "nested_closure",
        "## To run () -> Int:\n    Let outer be (i: Int) ->:\n        Let inner be (j: Int) -> j * 2.\n        \
         Return inner(i).\n    Return outer(5) + outer(8).\n## Main\n    Show run().\n",
    ),
    (
        // TOLERANT FLOAT COMPARISON — `is approximately` (Python-isclose semantics) lowers to the
        // self-contained f64 sequence; the drifted `0.1 + 0.2` vs `0.3` is the canonical case the
        // operator exists for. Exercises `Op::ApproxEq` end-to-end (WASM == VM == Tree-walker).
        "approx_eq_drifted_floats",
        "## Main\n    Let close be 0.1 + 0.2.\n    If close is approximately 0.3:\n        Show 1.\n    \
         Otherwise:\n        Show 0.\n",
    ),
    (
        // NESTED CLOSURE CAPTURES — an inner closure built inside an outer closure's body, closing over
        // the OUTER's param (`i`), the outer's function-LOCAL seq (`xs`) and enum (`s`), and a promoted
        // GLOBAL (`base`). The fixpoint plans the outer first, then reads the inner's capture kinds /
        // shapes from the now-planned outer body's `reg_shape` — so a nested capture resolves exactly
        // like a top-level one. `useEnum` is itself a BLOCK-body nested closure that `Inspect`s the
        // captured local enum. Covers every capture source reached through a nested closure.
        "nested_closure_capture",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Rectangle with width Int and height Int.\n\n\
         ## To run () -> Int:\n    Let outer be (i: Int) ->:\n        Let xs be [10, 20, 30].\n        \
         Let s be a new Circle with radius 9.\n        Let useSeq be (j: Int) -> i + base + item j of xs.\n        \
         Let useEnum be (n: Int) ->:\n            Inspect s:\n                When Circle (r): Return r + n.\n                \
         When Rectangle (w, h): Return w * h.\n            Return 0.\n        Return useSeq(1) + useEnum(100).\n    \
         Return outer(5).\n## Main\n    Let base be 1000.\n    Show run().\n",
    ),
    (
        // CLOSURE AS A RETURN VALUE — a function factory. `makeAdder` builds a closure capturing its
        // PARAM `n` and RETURNS it; the caller binds the returned handle and calls it (`f(5)`). The
        // actual call is already a `call_indirect` through the closure object's runtime func index, so
        // the only gap was STATIC: a `CallValue` on a returned handle can't trace its callee via a local
        // `MakeClosure`. The planner now publishes WHICH closure a function returns (`Plan.return_closure`
        // = the agreed origin of its reachable closure `Return`s) and seeds `closure_of[Call dst]` from
        // it, so `f`'s call resolves callee/captures/result exactly like a local closure's. Two instances
        // (`f` captures 10, `g` captures 100) keep distinct capture objects; `pickerFor` returns a
        // closure over a `Seq` PARAM, proving composite captures survive the return.
        "closure_return",
        "## To makeAdder (n: Int) -> Closure:\n    Let add be (x: Int) -> x + n.\n    Return add.\n\
         ## To pickerFor (xs: Seq of Int) -> Closure:\n    Let pick be (i: Int) -> item i of xs.\n    \
         Return pick.\n## Main\n    Let f be makeAdder(10).\n    Let g be makeAdder(100).\n    \
         Let ys be [10, 20, 30].\n    Let p be pickerFor(ys).\n    Show f(5).\n    Show g(5).\n    Show p(2).\n",
    ),
    (
        // CLOSURE AS AN ARGUMENT — a higher-order function calling a closure PARAMETER (`apply(f,x):
        // Return f(x)`), the dual of returning one. A `Closure` parameter carries an i32 handle, but
        // there is no `BoundaryType::Closure`, so a whole-program pass (`compute_param_origins`) attributes
        // each call's closure ARGUMENTS to the parameters they feed: when every call passes the same
        // closure, that parameter is typed `Kind::Closure` (i32) and its `closure_of` is seeded, so `f(x)`
        // resolves callee/captures/result like a local one. `apply` takes a NON-capturing closure (`dbl`,
        // called at two sites); `useAdder` takes a CAPTURING one that is itself a RETURNED closure
        // (`makeAdder(100)`), so return→pass→call composes. A parameter passed DIFFERENT closures stays
        // soundly rejected (the current ABI needs one known callee per call site).
        "closure_as_argument",
        "## To apply (f: Closure, x: Int) -> Int:\n    Return f(x).\n\
         ## To useAdder (g: Closure) -> Int:\n    Return g(2) + g(10).\n\
         ## To makeAdder (n: Int) -> Closure:\n    Let add be (x: Int) -> x + n.\n    Return add.\n\
         ## Main\n    Let dbl be (m: Int) -> m * 2.\n    Show apply(dbl, 21).\n    Show apply(dbl, 9).\n    \
         Let made be makeAdder(100).\n    Show useAdder(made).\n",
    ),
    (
        // FLOAT CLOSURES — a closure with a `Float` parameter / capture, every call shape. Closures
        // hardcode an all-Int `param_kinds` (the VM/JIT entry-guard contract), so a `(n: Float)` closure
        // used to mis-size its WASM signature (i64 param vs f64 argument → invalid module). Fix: the
        // closure now records its DECLARED `param_types` (additive — only the AOT reads them; `param_kinds`
        // stays all-Int), and BOTH the non-capturing (`function_param_seeds`) and capturing (real-param)
        // seed paths honor them. `half` is a non-capturing float closure; `addK` captures a `Float` (f64
        // capture slot); `apply` takes a float closure as an argument. Direct, capture, and arg all f64.
        "closure_float",
        "## To apply (f: Closure, x: Float) -> Float:\n    Return f(x).\n\
         ## Main\n    Let half be (n: Float) -> n / 2.0.\n    Let k be 10.0.\n    \
         Let addK be (n: Float) -> n + k.\n    Show half(9.0).\n    Show addK(2.5).\n    \
         Show apply(half, 5.0).\n    Show apply(half, 3.0).\n",
    ),
    (
        // CLOSURE with a COMPOSITE (struct / Text) PARAMETER, passed as an argument. A `(q: Pt)` closure
        // param is an i32 handle — the closure now records its declared `param_types` resolved through the
        // compiler's `user_types` registry (so a struct/enum closure param resolves, not just scalars), and
        // the closure-arg path passes the handle at i32. `q's x` (struct field) and `length of t` (Text)
        // both work inside the closure body. Each higher-order function is monomorphic (one closure).
        "closure_composite_param",
        "## A Pt has:\n    An x: Int.\n\n## To withPt (g: Closure, p: Pt) -> Int:\n    Return g(p).\n\
         ## To withText (h: Closure, s: Text) -> Int:\n    Return h(s).\n\
         ## Main\n    Let getx be (q: Pt) -> q's x.\n    Let len be (t: Text) -> length of t.\n    \
         Let pt be a new Pt with x 42.\n    Show withPt(getx, pt).\n    Show withText(len, \"hello\").\n",
    ),
    (
        // CLOSURE COMPOSITION — a closure that CAPTURES other closures and calls them (`(n) -> dbl(inc(n))`).
        // The captured closure value is an i32 handle; its statically-traced body function index flows from
        // the BUILD site so the body can `call_indirect` it. `twice` captures a function-LOCAL closure
        // (`add1`, resolved from the enclosing region's `closure_of`); `comp` captures GLOBAL closures
        // (`dbl`/`inc` — Main `Let` closures promoted to globals, resolved via `global_closures`). Both
        // forms compose. A capturing closure may mix a captured closure with a captured scalar.
        "closure_compose",
        "## To run () -> Int:\n    Let add1 be (n: Int) -> n + 1.\n    \
         Let twice be (n: Int) -> add1(add1(n)).\n    Return twice(10).\n\
         ## Main\n    Let dbl be (n: Int) -> n * 2.\n    Let inc be (n: Int) -> n + 1.\n    \
         Let comp be (n: Int) -> dbl(inc(n)).\n    Show run().\n    Show comp(10).\n    Show comp(20).\n",
    ),
    (
        // STRUCT with a MAP / ENUM field, accessed on a LOCALLY-BUILT struct. `s's mapfield` /
        // `s's enumfield` must re-seed the field result's VALUE kind / variant layout so `item k of
        // (s's counts)` and `Inspect s's shape` resolve. The cross-region (call-result/parameter) path
        // already did this via `FieldNested`; the locally-built path only carried struct + seq-of-struct
        // layouts — now it also resolves Map/enum/tuple fields from the struct's DECLARED type.
        "struct_composite_field",
        "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\
         ## A Reg has:\n    A counts: Map of Int to Int.\n    A shape: Shape.\n\n\
         ## Main\n    Let mutable c be a new Map of Int to Int.\n    Set item 1 of c to 42.\n    \
         Set item 2 of c to 8.\n    Let s be a new Circle with radius 9.\n    \
         Let r be a new Reg with counts c and shape s.\n    \
         Show item 1 of (r's counts) + item 2 of (r's counts).\n    Inspect r's shape:\n        \
         When Circle (rad): Show rad.\n        When Square (sd): Show sd.\n",
    ),
    (
        // Maps with non-Int VALUES — `Map of Int to Float` (f64 value slot) and `Map of Text to
        // Bool` (i64 slot, shown as true/false). The value kind is traced from the inserted value
        // (the `map_value` track), so `item k of m` reads/displays it at the right width/sink.
        "map_value_kinds",
        "## Main\n    Let mutable scores be a new Map of Int to Float.\n    \
         Set item 1 of scores to 3.5.\n    Set item 2 of scores to 4.25.\n    \
         Show item 1 of scores.\n    Show item 2 of scores.\n    \
         Let mutable flags be a new Map of Text to Bool.\n    \
         Set item \"on\" of flags to true.\n    Set item \"off\" of flags to false.\n    \
         Show item \"on\" of flags.\n    Show item \"off\" of flags.\n",
    ),
    (
        // Sequence of Text — the first HANDLE-element sequence (elements are Text handles in 8-byte
        // slots). A list literal, a `Push`, element access (`item N of`), `length`, whole-sequence
        // Show (`[a, b, c]` via the `print_seq_text` host), and iteration (each loop variable a
        // `Text`, shown per element).
        "seq_text",
        "## Main\n    Let mutable names be [\"alice\", \"bob\"].\n    \
         Push \"carol\" to names.\n    \
         Show length of names.\n    \
         Show item 1 of names.\n    \
         Show item 3 of names.\n    \
         Show names.\n    \
         Repeat for n in names:\n        Show n.\n",
    ),
    (
        // Heterogeneous tuple `(42, \"answer\", 3.5)` — mixed element kinds in a flat slot buffer
        // (a `Kind::Tuple`, distinct from the homogeneous-tuple-as-sequence path). `item N of t`
        // with a CONSTANT N reads slot N-1 at that position's kind/width (Int/Text/Float), resolved
        // via the `tuple_value` track; `length` reads the header.
        "tuple_hetero",
        "## Main\n    Let p be (42, \"answer\", 3.5).\n    \
         Show item 1 of p.\n    Show item 2 of p.\n    Show item 3 of p.\n    \
         Show length of p.\n",
    ),
    (
        // Whole `Set of Int` Show — `{3, 1, 2}`, in INSERTION order with dedup. The VM's Set is an
        // insertion-ordered `Vec` and the AOT set stores elements the same way, so the display is
        // deterministic and byte-identical (a `Map`'s hash order would NOT match, so it stays
        // deferred). Add-only (no swap-remove reordering).
        "set_show",
        "## Main\n    Let mutable s be a new Set of Int.\n    \
         Add 3 to s.\n    Add 1 to s.\n    Add 2 to s.\n    Add 3 to s.\n    \
         Show s.\n    Show length of s.\n",
    ),
    (
        // Sequence of Struct (a list of records). The list literal `[alice, bob]` is a `SeqStruct`
        // (elements are struct handles); `item N of people` extracts a struct whose field LAYOUT is
        // flowed from the sequence's element layout, so `p's field` (GetField) resolves. Verifies a
        // Text field and an Int field on two records.
        "seq_struct",
        "## A Person has:\n    A name: Text.\n    An age: Int.\n\n\
         ## Main\nLet alice be a new Person with name \"Alice\" and age 30.\n\
         Let bob be a new Person with name \"Bob\" and age 25.\n\
         Let people be [alice, bob].\n\
         Let p be item 1 of people.\n    Show p's name.\n    Show p's age.\n\
         Let q be item 2 of people.\n    Show q's name.\n    Show q's age.\n\
         Show length of people.\n",
    ),
    (
        // Sequence of Enum (a list of variant values). `[Red, Green, Blue]` is a `SeqEnum`; iterating
        // it yields each enum, and `Inspect` matches each by tag (`TestArm` — no layout flow needed
        // for nullary variants). Accumulate a weight per variant to verify the dispatch. `length` is
        // shown AFTER the loop ON PURPOSE: the post-loop Int reuses the loop variable's Enum register
        // (an i32 handle), an i32-vs-i64 valtype clash the backend once soundly REJECTED — now
        // resolved by register live-range splitting ([`vm::wasm::regsplit`]) giving each disjoint
        // range its own local, so this compiles unrestructured.
        "seq_enum",
        "## A Color is one of:\n    A Red.\n    A Green.\n    A Blue.\n\n\
         ## Main\nLet colors be [Red, Green, Blue].\n\
         Let mutable total be 0.\n\
         Repeat for c in colors:\n    \
         Inspect c:\n        When Green: Set total to total + 1.\n        \
         When Blue: Set total to total + 10.\n        Otherwise: Set total to total + 100.\n\
         Show total.\n\
         Show length of colors.\n",
    ),
    (
        // Nested sequence (an Int matrix) — `[[1,2,3],[4,5,6]]` is a `SeqSeqInt`; `item N of m` yields
        // a row (`SeqInt`), and indexing that yields an `Int`. The element-kind chain flows through
        // `seq_elem` with no extra tracking. (Rows bound to their own variables, then Int elements
        // to theirs, so no Int/handle register reuse.)
        "seq_nested",
        "## Main\n    Let m be [[1, 2, 3], [4, 5, 6]].\n    \
         Let row1 be item 1 of m.\n    Let row2 be item 2 of m.\n    \
         Let a be item 2 of row1.\n    Let b be item 3 of row2.\n    \
         Show a.\n    Show b.\n",
    ),
    (
        // COMBINATION: ITERATING a Seq of Struct with field access — the loop variable is a struct
        // whose field layout must flow from the sequence's element layout through `IterNext` (not
        // just `Index`), so `p's field` resolves inside the loop body.
        "seq_struct_iter",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## Main\nLet pts be [a new Point with x 1 and y 2, a new Point with x 3 and y 4].\n\
         Let mutable sum be 0.\n\
         Repeat for p in pts:\n        Set sum to sum + p's x.\n        Set sum to sum + p's y.\n\
         Show sum.\n",
    ),
    (
        // COMBINATION: NESTED struct (a struct field that is itself a struct). `b's corner` extracts
        // a `Point` whose field layout must flow from the outer struct's field layout, so the inner
        // `c's x` resolves — the struct-field analog of the Seq-of-Struct element-layout flow.
        "nested_struct",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## A Box has:\n    A corner: Point.\n\n\
         ## Main\nLet p be a new Point with x 5 and y 7.\n\
         Let b be a new Box with corner p.\n\
         Let c be b's corner.\n    Show c's x.\n    Show c's y.\n",
    ),
    (
        // COMBINATION: a struct with a SEQUENCE field. `bag's items` extracts a `Seq of Int`; the
        // field's kind (from the inserted value) flows so `item N of (bag's items)` / `length` work.
        "struct_seq_field",
        "## A Bag has:\n    An items: Seq of Int.\n\n\
         ## Main\nLet bag be a new Bag with items [10, 20, 30].\n\
         Let xs be bag's items.\n    Show item 2 of xs.\n    Show length of xs.\n",
    ),
    (
        // DEEP CLONE of a struct holding a MUTABLE Seq field. A flat field-buffer copy would SHARE
        // the inner sequence handle, so pushing to the clone's sequence would also grow the
        // original's — not a deep clone. The clone must RECURSIVELY clone each handle field (one
        // level: a `Seq of Int` is a flat scalar buffer). Proof of independence: after pushing 99 to
        // the CLONE's items, the original's length is still 2.
        "deepclone_struct_seq_field",
        "## A Bag has:\n    A tag: Int.\n    An items: Seq of Int.\n\n\
         ## Main\nLet b be a new Bag with tag 7 and items [10, 20].\n\
         Let c be copy of b.\n\
         Let cs be c's items.\n    Push 99 to cs.\n\
         Show length of b's items.\n    Show length of c's items.\n    Show c's tag.\n",
    ),
    (
        // DEEP CLONE of a struct with a Text field — exercises the recursive clone's byte-copy
        // (`is_text`) branch. The cloned Text is an independent buffer; here we verify the clone
        // preserves the field VALUES (content + the scalar field) byte-identically to the oracle.
        "deepclone_struct_text_field",
        "## A Person has:\n    A name: Text.\n    An age: Int.\n\n\
         ## Main\nLet p be a new Person with name \"alice\" and age 30.\n\
         Let q be copy of p.\n    Show q's name.\n    Show q's age.\n",
    ),
    (
        // DEEP CLONE of a Seq of Seq (an Int matrix) — runtime-loop recursion: clone the outer
        // handle buffer, then clone EACH inner sequence so the clone owns independent rows. Proof of
        // independence: pushing 99 to the CLONE's first row leaves the original's first row at len 2.
        "deepclone_matrix",
        "## Main\nLet m be [[1, 2], [3, 4]].\n\
         Let c be copy of m.\n\
         Let row be item 1 of c.\n    Push 99 to row.\n\
         Let mrow be item 1 of m.\n    Show length of mrow.\n\
         Let crow be item 1 of c.\n    Show length of crow.\n",
    ),
    (
        // STRUCT VALUE SEMANTICS: a struct extracted by field access (`o's inner`) then MUTATED must
        // NOT write through to the original — Logos structs are value types (tw/vm clone on field
        // access). The AOT holds structs behind shared handles, so this is enforced by COPY-ON-WRITE
        // at the write (`cow_struct_inserts`): `Set q's v` copies the shared `q` first. Reads and
        // construction stay clone-free. Correct result: `o`'s inner is still 10, `q` is 99.
        "struct_value_semantics_setfield",
        "## A Inner has:\n    A v: Int.\n\n\
         ## A Outer has:\n    An inner: Inner.\n\n\
         ## Main\nLet i be a new Inner with v 10.\n\
         Let o be a new Outer with inner i.\n\
         Let mutable q be o's inner.\n    Set q's v to 99.\n\
         Show o's inner's v.\n    Show q's v.\n",
    ),
    (
        // STRUCT VALUE SEMANTICS, the store-then-mutate alias: `p` is stored INTO `o`, then `p` is
        // mutated. `o`'s copy must be independent — the copy-on-write fires at `Set p's x` because
        // storing `p` into `o`'s field marked `p` no-longer-uniquely-owned. Result: `o`'s point is
        // still 0, `p` is 5. (Construction of `o` and `p` themselves never copies.)
        "struct_value_semantics_store",
        "## A Point has:\n    An x: Int.\n\n\
         ## A Box has:\n    A point: Point.\n\n\
         ## Main\nLet mutable p be a new Point with x 0.\n\
         Let o be a new Box with point p.\n    Set p's x to 5.\n\
         Show o's point's x.\n    Show p's x.\n",
    ),
    (
        // COMBINATION: a capturing CLOSURE called inside a LOOP over a sequence. The loop variable
        // feeds the closure (`scale(x)`), whose captured `factor` is read each call.
        "closure_in_loop",
        "## Main\nLet factor be 10.\n\
         Let scale be (n: Int) -> n * factor.\n\
         Let mutable total be 0.\n\
         Repeat for x in [1, 2, 3]:\n        Set total to total + scale(x).\n\
         Show total.\n",
    ),
    (
        // COMBINATION: a Map GET with a loop-variable key inside a loop (`item k of m` for `k` the
        // loop variable over a key list).
        "map_in_loop",
        "## Main\nLet mutable m be a new Map of Int to Int.\n\
         Set item 1 of m to 100.\n    Set item 2 of m to 200.\n    Set item 3 of m to 300.\n\
         Let mutable total be 0.\n\
         Repeat for k in [1, 2, 3]:\n        Set total to total + item k of m.\n\
         Show total.\n",
    ),
    (
        // COMBINATION: build a Seq of Struct incrementally via `Push` (not a literal), then iterate
        // with field access — exercises the `ListPush` element-layout flow + the `IterNext` flow.
        "struct_push_iter",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## Main\nLet mutable pts be a new Seq of Point.\n\
         Push a new Point with x 1 and y 2 to pts.\n    Push a new Point with x 3 and y 4 to pts.\n\
         Let mutable sum be 0.\n\
         Repeat for p in pts:\n        Set sum to sum + p's x.\n\
         Show sum.\n",
    ),
    (
        // COMBINATION (CROSS-REGION): field access on a struct RETURNED from a CLOSURE (`make(5)'s
        // x`). The result's field layout lives in the callee's region; the per-function resolved
        // return layout is threaded to the caller so `p's x` resolves.
        "closure_returns_struct",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## Main\nLet make be (v: Int) -> a new Point with x v and y v.\n\
         Let p be make(5).\n    Show p's x.\n    Show p's y.\n",
    ),
    (
        // COMBINATION (CROSS-REGION): field access on a struct RETURNED from a regular FUNCTION
        // (`Call`, not `CallValue`) — the same cross-region return-layout resolution.
        "function_returns_struct",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## To makept (v: Int) -> Point:\n    Return a new Point with x v and y (v + 1).\n\n\
         ## Main\nLet p be makept(5).\n    Show p's x.\n    Show p's y.\n",
    ),
    (
        // DEEP CLONE of a struct (`copy of p`): an INDEPENDENT copy — mutating the clone's field
        // must not touch the original (a fresh field buffer). Scalar fields carry no shared
        // sub-structure, so the buffer copy is a true deep clone; the layout flows so `c's x` works.
        "deepclone_struct",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## Main\nLet mutable p be a new Point with x 1 and y 2.\n\
         Let mutable c be copy of p.\n\
         Set c's x to 99.\n\
         Show p's x.\n    Show c's x.\n    Show c's y.\n",
    ),
    (
        // DEEP CLONE of a Set (`copy of s`): an independent copy — adding to the clone must not
        // touch the original (a fresh element buffer).
        "deepclone_set",
        "## Main\nLet mutable s be a new Set of Int.\n\
         Add 1 to s.\n    Add 2 to s.\n\
         Let mutable t be copy of s.\n\
         Add 3 to t.\n\
         Show length of s.\n    Show length of t.\n",
    ),
    (
        // CROSS-REGION from a FUNCTION (not just Main): `sumpt` calls the struct-returning `makept`
        // and accesses the result's fields — resolved by the second function-planning pass.
        "fn_calls_struct_fn",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## To makept (v: Int) -> Point:\n    Return a new Point with x v and y (v + 1).\n\n\
         ## To sumpt (v: Int) -> Int:\n    Let p be makept(v).\n    Return p's x + p's y.\n\n\
         ## Main\nShow sumpt(5).\n",
    ),
    // FIRST-CLASS PARAM/RETURN of every value kind — a function that TAKES the value as a parameter
    // AND RETURNS it (the identity round-trip), then the caller observes it. This is the whole point
    // of the first-class-params campaign: the value must survive the function boundary byte-for-byte
    // on the self-contained WASM backend, exactly as on the tree-walker/VM. Each was a WASM gap until
    // the `Real`-alias / `Seq of Seq` / `Set`-boundary / returned-param-struct-layout fixes.
    (
        // A `Real` (float) param + return — carried in an f64 register across the boundary.
        "param_return_real",
        "## To idr (x: Real) -> Real:\n    Return x.\n\n\
         ## Main\nShow idr(3.5).\n    Show floor(idr(2.5) * 4.0).\n",
    ),
    (
        // A `Set of Int` param + return — one i32 handle; cardinality AND elements must survive.
        "param_return_set_int",
        "## To ids (s: Set of Int) -> Set of Int:\n    Return s.\n\n\
         ## Main\nLet mutable s be a new Set of Int.\n    Add 7 to s.\n    Add 3 to s.\n\
         Let r be ids(s).\n    Show length of r.\n    Show r.\n",
    ),
    (
        // A `Set of Text` param + return — the SetText handle (byte-dedup'd) round-trips.
        "param_return_set_text",
        "## To idst (s: Set of Text) -> Set of Text:\n    Return s.\n\n\
         ## Main\nLet mutable s be a new Set of Text.\n    Add \"hi\" to s.\n    Add \"yo\" to s.\n\
         Let r be idst(s).\n    Show length of r.\n    Show r.\n",
    ),
    (
        // A struct param + return, RETURNED AS THE PARAMETER (never built inside the callee) — its
        // field layout is published from the declared param type so the caller resolves `r's field`.
        "param_return_struct",
        "## A Pt has:\n    An x: Int.\n    A y: Int.\n\n\
         ## To idp (p: Pt) -> Pt:\n    Return p.\n\n\
         ## Main\nLet r be idp(a new Pt with x 3 and y 4).\n    Show r's x.\n    Show r's y.\n",
    ),
    (
        // A nested `Seq of (Seq of Int)` param + return — the `SeqSeqInt` kind; nested `item` reads.
        "param_return_seq_seq",
        "## To idss (x: Seq of (Seq of Int)) -> Seq of (Seq of Int):\n    Return x.\n\n\
         ## Main\nLet r be idss([[1, 2], [3]]).\n    Show item 2 of item 1 of r.\n    Show length of r.\n",
    ),
    (
        // A `Seq of (Int, Int)` param + return — a homogeneous-shaped seq of heterogeneous tuples.
        "param_return_seq_tuple",
        "## To idsq (x: Seq of (Int, Int)) -> Seq of (Int, Int):\n    Return x.\n\n\
         ## Main\nLet r be idsq([(1, 2), (3, 4)]).\n    Let p be item 2 of r.\n    Show item 1 of p.\n    Show item 2 of p.\n",
    ),
];

/// Programs that ERROR on the tree-walker/VM. The standalone WASM module has no VM to surface the
/// message, so its contract is to TRAP (`unreachable`). [`wasm_traps_where_treewalker_errors`]
/// proves tw-errors ⟺ wasm-traps; the coverage proof counts the ops these (compiling) programs
/// exercise — so an op reachable only on an error path (e.g. `FailWith`) is still proven.
const ERROR_CORPUS: &[(&str, &str)] = &[
    (
        // A pop from a list it emptied: the check the seam puts before the pop fails with the sentence
        // naming the list, and the module traps where the tree-walker stops.
        "pop_past_the_last_item",
        "## Main\n    Let mutable xs be a new Seq of Int.\n    Push 7 to xs.\n    Pop from xs into a.\n    \
         Show a.\n    Pop from xs into b.\n    Show b.\n",
    ),
    (
        // An undefined variable is a RUNTIME error (the compiler emits `FailWith`); the module
        // traps. Exercises `FailWith` → `unreachable`.
        "undefined_variable",
        "## Main\n    Show missing.\n",
    ),
    (
        // 1-based index past the end: the tree-walker errors "Index 5 out of bounds" and the
        // module's bounds check traps. Exercises the `Index` out-of-bounds trap (the success
        // corpus only ever indexes in bounds).
        "index_out_of_bounds",
        "## Main\n    Let mutable a be a new Seq of Int.\n    Push 1 to a.\n    Show item 5 of a.\n",
    ),
    (
        // `text_from_bytes` on bytes that are not UTF-8. The reference tier raises
        // `text_from_bytes(): invalid UTF-8` and AOT-Rust panics with the same sentence; the module
        // asks `strings.lg`'s `validUtf8` and takes the `logos_rt_fail_at` + `unreachable` path.
        // 0xFF is a lead byte no encoding may begin with.
        "text_from_bytes_lone_ff",
        "## Main\n    Show text_from_bytes([255]).\n",
    ),
    (
        // The SAME fault reached through the stdlib rather than written by the author — this is the
        // caller that falsified the doc comment claiming the bytes are always valid, and it is why
        // the loader demand rule keys on the SELECTED MODULES and not only on the source text.
        // `fromHex("ff")` decodes to the single byte 255 and hands it straight in.
        "text_from_bytes_via_from_hex",
        "## Main\n    Show fromHex(\"ff\").\n",
    ),
    (
        // A truncated multi-byte sequence: 0xE2 announces three bytes and only two follow. Catches a
        // validator that checks lead-byte classes and forgets the length test.
        "text_from_bytes_truncated_sequence",
        "## Main\n    Show text_from_bytes([226, 130]).\n",
    ),
    (
        // An OVERLONG encoding of `/` (0x2F): well-formed in shape, forbidden by the standard. The
        // case a hand-rolled validator gets wrong, and the reason this calls the stdlib rules
        // instead of carrying a second copy of them.
        "text_from_bytes_overlong",
        "## Main\n    Show text_from_bytes([192, 175]).\n",
    ),
    (
        // A UTF-16 surrogate (U+D800) encoded as three bytes: a code point that exists in no
        // well-formed UTF-8 text.
        "text_from_bytes_surrogate",
        "## Main\n    Show text_from_bytes([237, 160, 128]).\n",
    ),
];

/// COMPILE-ONLY corpus for the command-line-argument path (`args()` / `parseInt`). These programs
/// cannot run in the self-contained corpus harness (they read argv), so their WASM == VM ==
/// Tree-walker equivalence is proven separately in `wasm_aot_args.rs` (which supplies the argv host
/// that builds the `Seq of Text` in linear memory). Here they only mark `Args` as exercised in the
/// instruction-coverage proof.
const ARGS_CORPUS: &[(&str, &str)] = &[
    (
        "args_parse_int",
        "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\
         ## Main\n    Let a be args().\n    Let n be parseInt(item 2 of a).\n    Show n * n.\n",
    ),
    (
        // An argv-SIZED `Seq of Int` filled in a loop with values proven to fit i32 (`% 1000000`): the
        // optimizer narrows the list to `NewEmptyListI32`, which the AOT lowers identically to the i64
        // `NewEmptyList`. Run-verified (with the argv host) in `wasm_aot_args.rs::aot_args_fills_an_i32_array`.
        "args_i32_array",
        "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\
         ## Main\n    Let a be args().\n    Let n be parseInt(item 2 of a).\n    \
         Let mutable arr be a new Seq of Int.\n    Let mutable i be 0.\n    While i is less than n:\n        \
         Push (i * 7 + 3) % 1000000 to arr.\n        Set i to i + 1.\n    Let mutable sum be 0.\n    \
         Set i to 1.\n    While i is at most n:\n        Set sum to sum + item i of arr.\n        \
         Set i to i + 1.\n    Show sum.\n",
    ),
];

/// COMPILE-ONLY corpus for the DETERMINISTIC single-thread concurrency ops. The plain-VM oracle the
/// behavioural corpus uses errors on any concurrency op (`requires the scheduler driver`), so these
/// are RUN-verified against the tree-walker AND the scheduler-DRIVEN VM (`vm_outcome_concurrent`) in
/// `wasm_aot_args.rs::assert_concurrent`. Here they only mark `ChanNew/ChanSend/ChanRecv/
/// ChanTrySend/ChanTryRecv/Spawn/SpawnHandle/TaskAbort` as exercised in the instruction-coverage proof.
const CONCURRENCY_CORPUS: &[(&str, &str)] = &[
    (
        // Non-blocking `Try to send` (always accepted on the unbounded FIFO) then `Try to receive`
        // (pops the just-sent value into a present `Some` Optional → Shown as the Int). Exercises
        // `ChanTrySend`/`ChanTryRecv` + the Optional value model; RUN-verified in
        // `wasm_aot_args.rs::aot_try_send_then_try_receive_round_trips`.
        "try_send_recv",
        "## Main\n    Let ch be a new Pipe of Int.\n    Try to send 99 into ch.\n    Try to receive x from ch.\n    Show x.\n",
    ),
    (
        "pipe_fifo",
        "## Main\n    Let messages be a new Pipe of Int.\n    Send 42 into messages.\n    Send 100 into messages.\n    \
         Receive x from messages.\n    Show \"Got: \" + x.\n",
    ),
    (
        "launch_tasks",
        "## To worker (id: Int):\n    Show \"Worker \" + id + \" started\".\n\n\
         ## Main\n    Launch a task to worker(1).\n    Launch a task to worker(2).\n    Show \"Tasks launched\".\n",
    ),
    (
        "task_handle_stop",
        "## To long_running:\n    Show \"Working...\".\n\n\
         ## Main\n    Let job be Launch a task to long_running.\n    Show \"Task spawned\".\n    Stop job.\n    Show \"Task cancelled\".\n",
    ),
    (
        // `select` (`Await the first of …`) over an EMPTY `Pipe of Text` + a timeout — no recv arm is
        // ready, so the timeout arm fires. Exercises `SelectArmRecv`/`SelectArmTimeout`/`SelectWait`;
        // the `Pipe of Text` element type (on `ChanNew`) types the recv arm's `msg` even unused.
        "select_timeout",
        "## Main\n    Let inbox be a new Pipe of Text.\n\n\
         Await the first of:\n    Receive msg from inbox:\n        Show \"Message: \" + msg.\n\
         \n    After 2 seconds:\n        Show \"No message received\".\n",
    ),
    (
        // `Sleep N` in a scheduler-driven program — advances virtual time only, so the send-then-
        // receive still yields the value. Exercises `Op::Sleep` (a no-op in the AOT).
        "sleep_virtual_time",
        "## Main\n    Let p be a new Pipe of Int.\n    Send 5 into p.\n    Sleep 10.\n    Receive x from p.\n    Show \"Got: \" + x.\n",
    ),
    (
        // `select` whose recv arm IS ready (a value was sent first) — the recv arm wins over the
        // timeout, binds `msg`, and runs its body (the first-ready-recv path of `SelectWait`).
        "select_recv_ready",
        "## Main\n    Let inbox be a new Pipe of Text.\n    Send \"ping\" into inbox.\n\n\
         Await the first of:\n    Receive msg from inbox:\n        Show \"Message: \" + msg.\n\
         \n    After 2 seconds:\n        Show \"No message received\".\n",
    ),
];

/// COMPILE-ONLY corpus for DETERMINISTIC LOCAL-MODE networking. RUN-verified against the tree-walker
/// (offline) AND the VM net runner (`vm_outcome_net`, local `NetInbox`) in `wasm_aot_args.rs::
/// assert_net`. Here they mark `NetListen/NetSend/NetSync/NetMakePeer` as exercised.
const NET_CORPUS: &[(&str, &str)] = &[
    (
        "net_listen",
        "## Main\n    Listen on \"/ip4/0.0.0.0/tcp/8000\".\n    Show \"Server listening on port 8000\".\n",
    ),
    (
        "net_peer_send",
        "## A Greeting is Portable and has:\n    a message (Text).\n\n\
         ## Main\n    Let remote be a PeerAgent at \"/ip4/127.0.0.1/tcp/8000\".\n    \
         Let msg be a new Greeting with message \"Hello, peer!\".\n    Show \"Sending: \" + msg's message.\n    Send msg to remote.\n",
    ),
    (
        "net_crdt_sync",
        "## A GameScore is Shared and has:\n    a points, which is ConvergentCount.\n\n\
         ## Main\n    Let mutable score be a new GameScore.\n    Sync score on \"game-leaderboard\".\n    Increase score's points by 100.\n    Show score's points.\n",
    ),
    (
        // `Connect to <addr>` (`NetConnect`) — Syntax Guide `network-connect`. Offline single node:
        // there is no relay to dial, so `Connect` is a local no-op and the `Show` prints. tw ==
        // VM-net == AOT (the deterministic oracles run in offline mode; see `net_is_offline`).
        "net_connect",
        "## Main\n    Let server_addr be \"/ip4/127.0.0.1/tcp/8000\".\n    Connect to server_addr.\n    Show \"Connected to server\".\n",
    ),
    (
        // OFFLINE LOOPBACK message (`NetSend` → `NetAwait`): a `Send … to <self>` delivers into the
        // local inbox FIFO and the following `Await` pops it. The AOT models the inbox as one local
        // queue (`NET_INBOX_ADDR`). Exercises `NetListen`/`NetSend`/`NetAwait`.
        "net_send_await",
        "## Main\n    Listen on \"/ip4/127.0.0.1/tcp/9000\".\n    Let peer be a PeerAgent at \"/ip4/127.0.0.1/tcp/9000\".\n    \
         Send 42 to peer.\n    Await response from peer into x.\n    Show x.\n",
    ),
    (
        // OFFLINE LOOPBACK batch stream (`NetStream` → `NetAwait stream`): `Stream [list] to <self>`
        // delivers the whole list, `Await stream` reads it. Exercises `NetStream` + streaming `NetAwait`.
        "net_stream_await",
        "## Main\n    Listen on \"/ip4/127.0.0.1/tcp/9100\".\n    Let peer be a PeerAgent at \"/ip4/127.0.0.1/tcp/9100\".\n    \
         Stream [10, 20, 30] to peer.\n    Await stream from peer into xs.\n    Show length of xs.\n",
    ),
];

/// ★ CLOSED — `text_from_bytes` on invalid UTF-8 now fails on every tier, and this is HOW. ★
///
/// The tree-walker and the VM raise `text_from_bytes(): invalid UTF-8`; AOT-Rust panics with the
/// same sentence. The wasm lowering used to check **nothing** — it copied the low byte of each
/// element into linear memory and carried on, so a browser answered `"\u{FFFD}"` where every
/// other tier stopped. That is a direct violation of `wasm_traps_where_treewalker_errors`, whose
/// own message says *"Fix vm/wasm/, never this lock."*
///
/// ★ THE PREMISE THAT MADE IT LOOK SAFE WAS FALSE, and it was written in the code:
/// *"inputs originate from `text_bytes`, so the bytes are always valid UTF-8"*. Our own stdlib
/// falsifies it — `fromHex` (encoding.lg) decodes arbitrary hex and hands the result straight in,
/// so `fromHex("ff")` arrives with byte 255. A premise asserted in a doc comment is enforced by
/// nothing, and the caller that violates it lives in a different file from the one that states it.
///
/// ★ THE FIX CALLS THE RULES RATHER THAN COPYING THEM. The tempting route was a hand-emitted
/// UTF-8 DFA in `lower_text_from_bytes`. It is the wrong one: the rules are subtle — the 0xC2
/// floor, the 0xF4 ceiling, overlong forms, the surrogate hole — and a subtly wrong validator that
/// accepts invalid input is strictly worse than a documented gap, because it looks like a fix.
/// They are also ALREADY WRITTEN, in LOGOS: `strings.lg`'s `validUtf8 (data: Seq of Int) -> Bool`
/// takes exactly the value the builtin is handed. So the module emits a call and a branch:
///
///   1. `compiled_fn_by_name` finds `validUtf8` in `program.functions` and CHECKS ITS SIGNATURE —
///      "declarer wins" lets a program define its own function of that name, and a
///      compiler-written call that lands on somebody else's function is worse than no call.
///   2. A false answer takes the existing `logos_rt_fail_at` + `unreachable` path, the same shape
///      the bounds check uses, with `FailCode::InvalidUtf8` beside `IndexOutOfBounds`.
///   3. Two loader rules make the reference visible — a demand rule so `strings.lg` is spliced and
///      a pruner root so `validUtf8` survives. Both are keyed on the SELECTED MODULES as well as
///      on the source, because `fromHex`, `json.lg` and seven others reach `text_from_bytes`
///      without the author ever naming it. Fifth instance of the compiler-written-reference shape
///      after `htmlEscape`, `## Route`, `## Page` and the key modifiers — the first keyed on a
///      BUILTIN rather than on a header.
///
/// ★ WHY THIS TEST SURVIVES ITS OWN CLOSURE. The corpus now carries five cases (a lone 0xFF, the
/// transitive `fromHex` route, a truncated sequence, an overlong form and a surrogate). This one
/// stays because it is the only place the HISTORY is written down, and the failure it names is
/// one a future refactor of `lower_text_from_bytes` re-opens silently: the module still compiles,
/// still runs, and only the answer is wrong.
#[test]
fn wasm_traps_on_invalid_utf8_like_every_other_tier() {
    let src = "## Main\n    Show text_from_bytes([255]).\n";
    let tw = tw_outcome(src);
    assert!(
        tw.error.is_some(),
        "the reference tier must reject invalid UTF-8:\n{src}"
    );
    let module = compile_to_wasm(src).expect("it compiles; the gap is at run time");
    assert!(
        run_aot_result(&module).is_err(),
        "ERROR-PARITY: the module must TRAP where the tree-walker errored ({:?})",
        tw.error
    );
}

/// ★ AN `Inspect` OVER AN `Option` — what compiles, what is refused, and why the line is there. ★
///
/// `some x` IS `x` (`vm/compiler.rs`: `Expr::OptionSome { value } => compile_expr_into(value, dst)`),
/// so an `Option` reaches the backend carrying no variant tag. The wasm lowering used to run the
/// enum path over it anyway, and the two halves failed DIFFERENTLY:
///
///   * over a scalar it emitted `i32_load` on an `i64` local, which wasm validation caught —
///     reported as `payload binding on a non-enum target`, a message naming nothing an author wrote;
///   * over a **Text or a record** the same `i32_load` read that object's own first word as if it
///     were a variant tag and compared it to a constant index. The types line up. The module
///     VALIDATES. The arm it picks is arbitrary. That is a wrong answer with nothing to catch it,
///     and it is the half that would have shipped had the fix stopped at "make it compile".
///
/// The reference tier's rule is PRESENCE, not tag (`machine.rs`'s `Op::TestArm`): `Nothing` matches
/// `None`, anything else matches `Some`. A heap handle can carry that rule exactly — the bump
/// allocator never hands out 0, so `!= 0` is "present".
///
/// ★ AND THE SENTINEL IS WHERE IT STOPS, DELIBERATELY. `nothing` lowers to the scalar `0`, so for
/// an `Option of Int` the values `some 0` and `nothing` are the SAME BITS. No test can separate
/// them, so the backend refuses by name instead of guessing. Closing that needs a boxed `Option` in
/// the BYTECODE — a representation the tree-walker and the VM share — which is an architectural
/// decision, not a wasm-local one. Recorded here so it is a known edge rather than a surprise.
#[test]
fn an_option_inspects_on_wasm_whatever_its_payload() {
    // A Text payload: presence is unambiguous, so it must lower AND agree with the reference tier.
    for (label, src, expect) in [
        (
            "some",
            "## Main\n    Let v be some \"hi\".\n    Inspect v:\n        When Some (t):\n            Show t.\n        When None:\n            Show \"-\".\n",
            "hi",
        ),
        (
            "record",
            "## A Row has:\n    a name (Text).\n\n## Main\n    Let v be some (a new Row with name \"ada\").\n    Inspect v:\n        When Some (r):\n            Show r's name.\n        When None:\n            Show \"-\".\n",
            "ada",
        ),
    ] {
        let tw = tw_outcome(src);
        assert_eq!(tw.error, None, "the reference tier must run `{label}`:\n{src}");
        assert_eq!(tw.output.trim(), expect, "reference-tier answer for `{label}`");
        let module = compile_to_wasm(src)
            .unwrap_or_else(|e| panic!("an `Option` of a handle must lower (`{label}`): {e:?}\n{src}"));
        assert_eq!(
            run_aot(&module).trim(),
            expect,
            "WASM must answer exactly as the reference tier for `{label}` — a transparent `Option` \
             is tested by PRESENCE, not by reading the payload's first word as a tag.\n{src}"
        );
    }

    // A scalar payload: refused, and the refusal must SAY the sentinel is the reason.
    // ★ AND THE SCALAR CASE, WHICH THIS TEST USED TO PIN AS A REFUSAL.
    //
    // `some 3` and `some 0` were once indistinguishable from `none` in a compiled module, because
    // the bytecode threw the `Some` away and `nothing` lowered to the scalar `0`. The wasm backend
    // refused them BY NAME rather than answer a coin flip, and this test pinned that refusal.
    //
    // It is closed, and NOT by a wasm change: giving `Option` a real tag in the bytecode
    // (`vm/compiler.rs`'s `compile_option_ctor`, the same `Op::NewInductive` a `Result` uses) made
    // `TestArm`/`BindArm` read a constructor tag they already knew how to read. Zero lines of
    // backend. `some 0` is the case that matters — it is the one the old encoding could never
    // represent, and the one `pgIntAtOpt` needs to tell a SQL NULL from a stored zero.
    for (label, src, want) in [
        (
            "scalar payload",
            "## Main\n    Let v be some 3.\n    Inspect v:\n        When Some (n):\n            Show n.\n        When None:\n            Show \"none\".\n",
            "3",
        ),
        (
            "scalar payload that IS the old sentinel",
            "## Main\n    Let v be some 0.\n    Inspect v:\n        When Some (n):\n            Show n.\n        When None:\n            Show \"none\".\n",
            "0",
        ),
    ] {
        let tw = tw_outcome(src);
        assert_eq!(tw.error, None, "the reference tier must run `{label}`:\n{src}");
        assert_eq!(tw.output.trim(), want, "reference-tier answer for `{label}`");
        let module = compile_to_wasm(src)
            .unwrap_or_else(|e| panic!("`{label}` must reach wasm — the tag is real now: {e:?}\n{src}"));
        assert_eq!(
            run_aot(&module).trim(),
            want,
            "WASM must answer exactly as the reference tier for `{label}`. If `some 0` ever reads \
             as absent again, the bytecode stopped carrying the tag.\n{src}"
        );
    }

    // ★ AND THE CASE THAT NEEDED THE DECLARED TYPE — an ABSENT Option whose `Some` arm binds a
    // payload. The backend must emit a printer for that arm even though it never runs, and until
    // `Option` was a `BoundaryType` there was nothing to type it with: `boundary_of_type_expr`
    // answered `None` for `-> Option of Int`, so the declared type never reached the backend and it
    // refused with "Show of an unknown-kind value".
    //
    // Closed by carrying the payload type (`BoundaryType::Option`), giving it an `EnumShape` beside
    // `Result`'s, and adding it to the call-RETURN seeding in `kind.rs` — that last one was the
    // actual hole: `Enum(_) | Result(_, _)` was the whole list, so a `-> Option of T` call result
    // got no shape for its arms even once the type resolved.
    let absent_with_binding = "## To lookup (found: Bool) -> Option of Int:\n    If found:\n        Return some 5.\n    Return none.\n\n## Main\n    Inspect lookup(false):\n        When Some (n):\n            Show n.\n        When None:\n            Show \"none\".\n";
    let present_with_binding = "## To lookup (found: Bool) -> Option of Int:\n    If found:\n        Return some 5.\n    Return none.\n\n## Main\n    Inspect lookup(true):\n        When Some (n):\n            Show n.\n        When None:\n            Show \"none\".\n";
    for (label, src, want) in [
        (
            "absent through a declared return",
            absent_with_binding,
            "none",
        ),
        (
            "present through a declared return",
            present_with_binding,
            "5",
        ),
    ] {
        let tw = tw_outcome(src);
        assert_eq!(tw.output.trim(), want, "reference tier for `{label}`");
        let module = compile_to_wasm(src)
            .unwrap_or_else(|e| panic!("`{label}` must reach wasm: {e:?}\n{src}"));
        assert_eq!(
            run_aot(&module).trim(),
            want,
            "WASM must answer as the reference tier for `{label}` — a declared `Option of Int` \
             carries its payload type to the arm that binds it.\n{src}"
        );
    }
}

/// ★ THE ONE WAY THE UTF-8 FIX COULD EAT ITSELF, made impossible to land quietly. ★
///
/// `lower_text_from_bytes` emits a call to `validUtf8`, so if `validUtf8` ever came to use
/// `text_from_bytes` — directly or through anything it calls — a compiled module would recurse
/// until the stack ran out. On a browser that is a dead page, from an edit to a stdlib function
/// that looks entirely local and whose own tests all pass: the recursion exists only in the wasm
/// lowering, one crate away, and no tier but wasm would show it.
///
/// So the constraint is written where the edit happens. This reads `strings.lg` itself rather than
/// a copy of it, walks the call graph out from `validUtf8`, and refuses to let `text_from_bytes`
/// appear anywhere in its closure.
#[test]
fn the_utf8_validator_the_wasm_backend_calls_can_never_call_the_builtin_back() {
    const STRINGS_LG: &str = include_str!("../../logicaffeine_compile/assets/std/strings.lg");
    // Split into `## `-delimited sections and keep the `## To <name>` ones, as the loader does.
    let mut sections: Vec<String> = Vec::new();
    for line in STRINGS_LG.split_inclusive('\n') {
        if line.starts_with("## ") || sections.is_empty() {
            sections.push(String::new());
        }
        sections.last_mut().unwrap().push_str(line);
    }
    let named: Vec<(String, &str)> = sections
        .iter()
        .filter_map(|sec| {
            let head = sec.lines().next()?.strip_prefix("## To ")?;
            let name = head
                .split(|c: char| c == '(' || c == ':' || c.is_whitespace())
                .next()?;
            (!name.is_empty()).then(|| (name.to_string(), sec.as_str()))
        })
        .collect();
    assert!(
        named.iter().any(|(n, _)| n == "validUtf8"),
        "`strings.lg` no longer defines `validUtf8`, which `lower_text_from_bytes` calls by name. \
         The wasm backend now refuses `text_from_bytes` rather than mis-answering it — but that is \
         a capability the framework silently lost, so decide it here rather than discover it in a \
         browser."
    );
    // Transitively close over the functions `validUtf8` reaches, within this module.
    let mut closure: Vec<&str> = vec!["validUtf8"];
    loop {
        let mut grew = false;
        for (name, body) in &named {
            if !closure.contains(&name.as_str()) {
                continue;
            }
            for (cand, _) in &named {
                if !closure.contains(&cand.as_str()) && body.contains(cand.as_str()) {
                    closure.push(cand.as_str());
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    for name in &closure {
        let body = named
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| *b)
            .unwrap();
        assert!(
            !body.contains("text_from_bytes"),
            "`{name}` is reachable from `validUtf8`, and it calls `text_from_bytes`. The wasm \
             lowering of that builtin CALLS `validUtf8` to decide whether the bytes are valid \
             (see `wasm_traps_on_invalid_utf8_like_every_other_tier`), so this closes a loop: a \
             compiled page would recurse until the stack ran out, and only on wasm. Build the \
             text some other way, or teach the lowering to break the cycle — do not delete this."
        );
    }
}

/// ★ ERROR-PARITY LOCK ★ — where the tree-walker (and VM) raise a runtime error, the standalone
/// WASM module must TRAP. It cannot reproduce the message (no VM to surface it), so the contract is
/// existence-of-failure parity: tw errors ⟺ vm errors ⟺ wasm traps. One trap covers explicit
/// failures (`FailWith`) and out-of-bounds indexing alike.
#[test]
fn wasm_traps_where_treewalker_errors() {
    logicaffeine_compile::semantics::temporal::set_fixed_clock(19753, 1_700_000_000_000_000_000);
    for (name, src) in ERROR_CORPUS {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            tw.error, vm.error,
            "BASE EQUIVALENCE BROKEN (tree-walker != VM error) for `{name}`:\n{src}"
        );
        assert!(
            tw.error.is_some(),
            "error-corpus program `{name}` must ERROR on the oracle:\n{src}"
        );
        let module = compile_to_wasm(src).unwrap_or_else(|e| {
            panic!("error-corpus `{name}` must still COMPILE (it traps at runtime, not compile time): {e:?}\n{src}")
        });
        let outcome = run_aot_result(&module);
        assert!(
            outcome.is_err(),
            "ERROR-PARITY BROKEN: `{name}` ran to completion ({outcome:?}) where the tree-walker errored \
             ({:?}). The module must TRAP. Fix vm/wasm/, never this lock.\n{src}",
            tw.error
        );
    }
}

/// ★ BEHAVIOURAL LOCK ★ — WASM == VM == Tree-walker, with the support biconditional. For every
/// corpus program: tw == vm, and the AOT module compiles IFF every op is `Supported` and (then)
/// matches the tree-walker byte-for-byte. Rejecting an all-supported program, or compiling one
/// that uses a `Deferred` op, is a RED to fix in the backend — never to defer away.
#[test]
fn wasm_equals_vm_and_treewalker_over_the_corpus() {
    // Output equality is trailing-newline-insensitive: the tree-walker joins its lines (no
    // trailing '\n'), the VM keeps Show's trailing '\n', and the WASM host sinks join — the same
    // print-vs-join artifact every differential in this repo normalizes with `trim`.
    fn norm(s: &str) -> String {
        s.trim().to_string()
    }

    // Pin the clock so `today`/`now` are deterministic across tree-walker, VM, and the WASM host
    // (which reads this same thread-local). Non-temporal programs are unaffected.
    logicaffeine_compile::semantics::temporal::set_fixed_clock(19753, 1_700_000_000_000_000_000);

    let mut supported_programs = 0usize;
    for (name, src) in CORPUS {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            tw.error, vm.error,
            "BASE EQUIVALENCE BROKEN (tree-walker != VM error) for `{name}`:\n{src}"
        );
        assert_eq!(
            norm(&tw.output),
            norm(&vm.output),
            "BASE EQUIVALENCE BROKEN (tree-walker != VM output) for `{name}`:\n{src}"
        );
        assert_eq!(
            tw.error, None,
            "corpus program `{name}` must run cleanly on the oracle:\n{src}"
        );

        let ops =
            program_ops(src).unwrap_or_else(|| panic!("`{name}` must reach bytecode:\n{src}"));
        // A SELF-CONTAINED module lowers `Supported` AND `Unreachable` ops (the latter are no-op/pass-
        // through and can't appear in a real program anyway); `Linked`, `NativeOnly`, and `Deferred`
        // ops are not self-contained on this target.
        let not_self_contained: Vec<&str> = ops
            .iter()
            .filter_map(|o| match op_support(o) {
                Support::Supported | Support::Unreachable(_) => None,
                Support::Linked(why) | Support::NativeOnly(why) | Support::Deferred(why) => {
                    Some(why)
                }
            })
            .collect();
        let all_supported = not_self_contained.is_empty();

        match compile_to_wasm(src) {
            Ok(module) => {
                assert!(
                    all_supported,
                    "DESYNC: the WASM backend compiled `{name}` even though it uses a non-self-contained op \
                     {not_self_contained:?} — `op_support` disagrees with the real self-contained lowering."
                );
                assert_eq!(
                    norm(&run_aot(&module)),
                    norm(&tw.output),
                    "WASM != TREE-WALKER for `{name}` — a miscompile. Fix vm/wasm/, never this lock.\n{src}"
                );
                supported_programs += 1;
            }
            Err(e) => {
                assert!(
                    !all_supported,
                    "COVERAGE GAP: the WASM backend REJECTED `{name}` ({e:?}) even though every op it \
                     uses is `Supported`. This is a backend gap to FIX (vm/wasm/), not to defer.\n{src}"
                );
            }
        }
    }

    // Ratchet floor: the curated corpus must keep at least this many programs fully lowering to
    // WASM. Raise it as features land; it must never be lowered to make a red pass.
    const SUPPORTED_PROGRAM_FLOOR: usize = 138;
    assert!(
        supported_programs >= SUPPORTED_PROGRAM_FLOOR,
        "WASM coverage REGRESSED: only {supported_programs} curated programs lowered to WASM \
         (floor {SUPPORTED_PROGRAM_FLOOR}). A previously-supported program stopped compiling — fix \
         the backend, do not lower the floor."
    );
}

/// FIRST-CLASS PARAMETER/RETURN on the self-contained WASM backend. A function that takes each value
/// kind as a PARAMETER and RETURNS it (the identity round-trip) must carry the value across the
/// function boundary byte-for-byte — observed identically on the tree-walker, the VM, and the emitted
/// WebAssembly. This is the WASM half of the first-class-params campaign (the four executing tiers are
/// locked by the tests crate's `param_return_tier_coverage_lock`); each program below was a WASM
/// lowering gap until this session's fixes:
///   - `Real` (float) param/return: the AOT bridge `boundary_of_name` was missing the `Real` alias for
///     `Float`, so a `-> Real` return seeded no kind ("unknown-kind return").
///   - `Seq of (Seq of Int)`: `boundary_to_kind` didn't map a nested `SeqInt` element to `SeqSeqInt`.
///   - `Set of Int` / `Set of Text`: there was no `BoundaryType::Set`, so a set param seeded no kind.
///   - a struct RETURNED AS A PARAMETER: `fn_return_struct_layout` only read `reg_layout` (locally
///     built structs), so a passed-through struct published no layout and the caller's `r's field` had
///     no static slot; it now falls back to the resolved `call_result` layout.
/// A DEDICATED lock (not just CORPUS rows) so the capability is named and cannot silently regress — and
/// so it is verifiable independently of any unrelated corpus entry.
///
/// This is the EXECUTION authority for the WASM tier of the campaign: the tests crate's
/// `param_return_tier_coverage_lock` can only LOWER-check WASM (no `wasmi` there), so it delegates
/// execution here. The programs below cover EVERY value kind that ratchet probes — int / real / bool /
/// text / seq_int / seq_text / seq_seq / set_int / struct / tuple_anon / multi_return / record_named /
/// destructure / seq_tuple — PLUS robustness the ratchet does not reach: genuinely HETEROGENEOUS tuples
/// and records (the real `Kind::Tuple`, not the homogeneous Seq path), a float-fielded struct, and
/// TRANSFORM-through-boundary (mutate/rebuild, not identity). ANTI-DRIFT: `param_return_covers_every_ratchet_kind`
/// pins the value-kind set, so adding a ratchet probe without covering it here goes RED.
#[test]
fn first_class_param_return_round_trips_on_wasm() {
    fn norm(s: &str) -> String {
        s.trim().to_string()
    }
    // Each program: `## To id (x: T) -> T: Return x.` then the caller passes a value in, gets it back,
    // and OBSERVES it (elements/fields/nesting), so a boundary that dropped or corrupted the value would
    // diverge from the tree-walker. Reuses the exact CORPUS entries (kept in sync by intent).
    const PROGRAMS: &[(&str, &str)] = &[
        // ── the FULL ratchet surface (mirrors `param_return_tier_coverage_lock`'s 14 PROBES), now
        //    EXECUTION-verified on WASM — the ratchet only LOWERS them (the tests crate has no wasmi). ──
        ("int", "## To ii (x: Int) -> Int:\n    Return x.\n\n## Main\nShow ii(42).\n"),
        (
            "real",
            "## To idr (x: Real) -> Real:\n    Return x.\n\n\
             ## Main\nShow idr(3.5).\n    Show floor(idr(2.5) * 4.0).\n",
        ),
        ("bool", "## To ib (x: Bool) -> Bool:\n    Return x.\n\n## Main\nShow ib(2 is greater than 1).\n"),
        (
            "text",
            "## To idtx (x: Text) -> Text:\n    Return x.\n\n\
             ## Main\nLet r be idtx(\"hi\").\n    Show upper(r).\n",
        ),
        (
            "seq_int",
            "## To isi (x: Seq of Int) -> Seq of Int:\n    Return x.\n\n\
             ## Main\nLet r be isi([10, 20, 30]).\n    Show item 2 of r.\n    Show r.\n",
        ),
        (
            "seq_text",
            "## To ist (x: Seq of Text) -> Seq of Text:\n    Return x.\n\n\
             ## Main\nLet r be ist([\"a\", \"b\"]).\n    Show item 1 of r.\n    Show r.\n",
        ),
        (
            "seq_seq",
            "## To idss (x: Seq of (Seq of Int)) -> Seq of (Seq of Int):\n    Return x.\n\n\
             ## Main\nLet r be idss([[1, 2], [3]]).\n    Show item 2 of item 1 of r.\n    Show length of r.\n",
        ),
        (
            "set_int",
            "## To ids (s: Set of Int) -> Set of Int:\n    Return s.\n\n\
             ## Main\nLet mutable s be a new Set of Int.\n    Add 7 to s.\n    Add 3 to s.\n\
             Let r be ids(s).\n    Show length of r.\n    Show r.\n",
        ),
        (
            "set_text",
            "## To idst (s: Set of Text) -> Set of Text:\n    Return s.\n\n\
             ## Main\nLet mutable s be a new Set of Text.\n    Add \"hi\" to s.\n    Add \"yo\" to s.\n\
             Let r be idst(s).\n    Show length of r.\n    Show r.\n",
        ),
        (
            "struct",
            "## A Pt has:\n    An x: Int.\n    A y: Int.\n\n\
             ## To idp (p: Pt) -> Pt:\n    Return p.\n\n\
             ## Main\nLet r be idp(a new Pt with x 3 and y 4).\n    Show r's x.\n    Show r's y.\n",
        ),
        (
            // ANONYMOUS tuple `(Int, Int)` — HOMOGENEOUS, so it rides the `Seq` layout (item N = seq index).
            "tuple_anon",
            "## To ita (x: (Int, Int)) -> (Int, Int):\n    Return x.\n\n\
             ## Main\nLet r be ita((7, 9)).\n    Show item 1 of r.\n    Show item 2 of r.\n",
        ),
        (
            // MULTIPLE RETURN of a homogeneous tuple, computed (not passed through).
            "multi_return",
            "## To dm (n: Int) -> (Int, Int):\n    Return (n / 5, n mod 5).\n\n\
             ## Main\nLet r be dm(17).\n    Show item 1 of r.\n    Show item 2 of r.\n",
        ),
        (
            // NAMED RECORD (synthetic struct), accessed by field name.
            "record_named",
            "## To dr (n: Int) -> (quotient: Int, remainder: Int):\n    Return (n / 5, n mod 5).\n\n\
             ## Main\nLet r be dr(17).\n    Show r's quotient.\n    Show r's remainder.\n",
        ),
        (
            // CALL-SITE DESTRUCTURING of a tuple return.
            "destructure",
            "## To dd (n: Int) -> (Int, Int):\n    Return (n / 5, n mod 5).\n\n\
             ## Main\nLet (q, rem) be dd(17).\n    Show q.\n    Show rem.\n",
        ),
        (
            "seq_tuple",
            "## To idsq (x: Seq of (Int, Int)) -> Seq of (Int, Int):\n    Return x.\n\n\
             ## Main\nLet r be idsq([(1, 2), (3, 4)]).\n    Let p be item 2 of r.\n    Show item 1 of p.\n    Show item 2 of p.\n",
        ),
        // ── ROBUSTNESS beyond the ratchet: genuinely HETEROGENEOUS tuples/records (the real `Kind::Tuple`
        //    machinery, not the homogeneous Seq path), FLOAT-in-aggregate, and TRANSFORM-through-boundary
        //    (mutate/rebuild, not identity) — the boundary must carry a changed value, not just echo it. ──
        (
            // HETEROGENEOUS tuple `(Int, Text)` — a true mixed-kind Tuple handle across the boundary.
            "tuple_hetero",
            "## To ith (x: (Int, Text)) -> (Int, Text):\n    Return x.\n\n\
             ## Main\nLet r be ith((7, \"hi\")).\n    Show item 1 of r.\n    Let t be item 2 of r.\n    Show upper(t).\n",
        ),
        (
            // HETEROGENEOUS multiple return `(Int, Text)`, computed.
            "multi_return_hetero",
            "## To describe (n: Int) -> (Int, Text):\n    Return (n, \"num\").\n\n\
             ## Main\nLet r be describe(5).\n    Show item 1 of r.\n    Let t be item 2 of r.\n    Show upper(t).\n",
        ),
        (
            // MIXED-type named record `(Text, Int, Real)` — three fields, three kinds, by name. Field
            // names are single letters (safe identifiers, like the `x`/`y` struct fields elsewhere here);
            // an English verb in a field slot (`label`/`value`/`name`) is rejected by the NL parser, not
            // the backend — a language-surface constraint, orthogonal to the WASM aggregate lowering.
            "record_mixed",
            "## To mk (n: Int) -> (p: Text, q: Int, w: Real):\n    Return (\"hi\", n, 1.5).\n\n\
             ## Main\nLet r be mk(9).\n    Show r's p.\n    Show r's q.\n    Let d be r's w.\n    Show floor(d * 2.0).\n",
        ),
        (
            // A struct whose FIELDS are floats — the `Real` fix must hold INSIDE an aggregate, not only as a
            // bare scalar param.
            "struct_real_fields",
            "## A Vec2 has:\n    A dx: Real.\n    A dy: Real.\n\n\
             ## To idv (v: Vec2) -> Vec2:\n    Return v.\n\n\
             ## Main\nLet r be idv(a new Vec2 with dx 1.5 and dy 2.5).\n\
             Let a be r's dx.\n    Show floor(a * 2.0).\n    Let b be r's dy.\n    Show floor(b * 2.0).\n",
        ),
        (
            // TRANSFORM a Set through the boundary (mutate then return) — the returned value differs from
            // the argument, so a boundary that echoed the input would diverge from the tree-walker.
            "set_transform",
            "## To addone (s: Set of Int) -> Set of Int:\n    Let mutable t be s.\n    Add 1 to t.\n    Return t.\n\n\
             ## Main\nLet mutable s be a new Set of Int.\n    Add 5 to s.\n    Let r be addone(s).\n    Show length of r.\n    Show r.\n",
        ),
        (
            // TRANSFORM a Seq through the boundary (push then return).
            "seq_transform",
            "## To bump (x: Seq of Int) -> Seq of Int:\n    Let mutable r be x.\n    Push 99 to r.\n    Return r.\n\n\
             ## Main\nLet r be bump([1, 2]).\n    Show item 3 of r.\n    Show length of r.\n",
        ),
        (
            // A LOCALLY-BUILT struct return that reads the parameter's fields — not the identity path, so
            // both `fn_return_struct_layout` branches (reg_layout AND call_result) are exercised together.
            "struct_transform",
            "## A Pt has:\n    An x: Int.\n    A y: Int.\n\n\
             ## To shift (p: Pt) -> Pt:\n    Let nx be p's x + 10.\n    Return a new Pt with x nx and y p's y.\n\n\
             ## Main\nLet r be shift(a new Pt with x 3 and y 4).\n    Show r's x.\n    Show r's y.\n",
        ),
    ];
    for (name, src) in PROGRAMS {
        let tw = tw_outcome(src);
        let vm = vm_outcome(src);
        assert_eq!(
            tw.error, None,
            "`{name}` must run cleanly on the tree-walker oracle:\n{src}"
        );
        assert_eq!(
            norm(&tw.output),
            norm(&vm.output),
            "BASE EQUIVALENCE BROKEN (tree-walker != VM) for `{name}`:\n{src}"
        );
        // Every op must be SELF-CONTAINED (Supported/Unreachable) — no non-self-contained fallback. This
        // mirrors the corpus DESYNC guard so the win is a true no-runtime lowering, not a linked crutch.
        let ops =
            program_ops(src).unwrap_or_else(|| panic!("`{name}` must reach bytecode:\n{src}"));
        let not_self_contained: Vec<&str> = ops
            .iter()
            .filter_map(|o| match op_support(o) {
                Support::Supported | Support::Unreachable(_) => None,
                Support::Linked(why) | Support::NativeOnly(why) | Support::Deferred(why) => {
                    Some(why)
                }
            })
            .collect();
        assert!(
            not_self_contained.is_empty(),
            "`{name}` uses a non-self-contained op {not_self_contained:?} — the round-trip must lower fully \
             self-contained (no linked runtime).\n{src}"
        );
        let module = compile_to_wasm(src)
            .unwrap_or_else(|e| panic!("`{name}` must lower to self-contained WASM: {e:?}\n{src}"));
        assert_eq!(
            norm(&run_aot(&module)),
            norm(&tw.output),
            "WASM != TREE-WALKER for `{name}` — a first-class param/return miscompile. Fix vm/wasm/, never this lock.\n{src}"
        );
    }

    // ULTIMATE ANTI-DRIFT: this lock is the WASM EXECUTION authority for the campaign — the tests crate's
    // `param_return_tier_coverage_lock` can only LOWER-check WASM (no `wasmi` there) and delegates execution
    // here. Both locks consume the SAME shared canonical set `FIRST_CLASS_VALUE_KINDS` (the single source of
    // truth in the compile crate), so a value kind cannot be covered on one and forgotten on the other:
    // adding a name to that const fails BOTH locks until each covers it. Here, every canonical kind must have
    // an EXECUTABLE WASM program above (this lock may hold MORE — the heterogeneous / transform robustness
    // extras — so it is a superset check).
    let covered: std::collections::HashSet<&str> = PROGRAMS.iter().map(|(n, _)| *n).collect();
    for kind in logicaffeine_compile::compile::FIRST_CLASS_VALUE_KINDS {
        assert!(
            covered.contains(kind),
            "ANTI-DRIFT: canonical value kind `{kind}` (FIRST_CLASS_VALUE_KINDS) has no WASM EXECUTION program \
             here. Add one so the WASM tier stays execution-locked against the tree-walker oracle for every \
             kind the shared spec demands."
        );
    }
}

/// PROOF that there is ZERO genuine deferred WORK. `Deferred` is the only "TODO" category — `Linked`
/// (works in a linked module), `NativeOnly` (outside this target's capabilities), and `Unreachable`
/// (a dead op no source syntax emits) are NOT deferrals.
/// This asserts no `Op` anywhere is classified `Deferred`: the gap to full parity is closed.
#[test]
fn deferred_feature_surface_is_documented_and_shrinking() {
    // Every `Op` variant (not just the corpus) — a genuine deferral anywhere fails this.
    let reasons: BTreeSet<&str> = all_op_variants()
        .iter()
        .filter_map(|op| match op_support(op) {
            Support::Deferred(why) => Some(why),
            Support::Supported
            | Support::Linked(_)
            | Support::NativeOnly(_)
            | Support::Unreachable(_) => None,
        })
        .collect();
    // ZERO genuine deferred work: every `Op` has a non-deferred support contract.
    assert!(
        reasons.is_empty(),
        "a genuinely-DEFERRED op reappeared: {reasons:?} — the deferred census must stay empty (implement \
         it as `Supported`/`Linked`, document a real `NativeOnly` target boundary, prove it `Unreachable`, \
         or acknowledge that it is real remaining work)."
    );
}

/// The target-specific boundary is exact, not a catch-all for inconvenient ops. Native OS socket
/// ownership belongs here; a third entry requires an intentional census update with its capability
/// argument. Reasons are locked too so the target contract cannot decay into a generic exclusion.
#[test]
fn native_only_surface_is_exactly_host_socket_io() {
    let actual: BTreeSet<(&'static str, &'static str)> = all_op_variants()
        .iter()
        .filter_map(|op| match op_support(op) {
            Support::NativeOnly(why) => Some((op_name(op), why)),
            Support::Supported
            | Support::Linked(_)
            | Support::Unreachable(_)
            | Support::Deferred(_) => None,
        })
        .collect();
    let expected = BTreeSet::from([
        (
            "Serve",
            "Serve requires a native HTTP listener and accept loop; standalone/browser WebAssembly cannot bind ports",
        ),
        (
            "Tcp",
            "tcp.lg requires native OS sockets; standalone/browser WebAssembly has no socket capability",
        ),
    ]);
    assert_eq!(
        actual, expected,
        "the native-only WebAssembly census changed — only real host-socket operations belong here; \
         classify any new target boundary explicitly and update this exact contract deliberately"
    );
}

/// ★ FULL-LANGUAGE COVERAGE PROOF ★ — every instruction the backend CLAIMS to support must be
/// EXERCISED end-to-end by a compiling corpus program (where the behavioural lock already proved
/// WASM == VM == Tree-walker). This forbids a vacuous "Supported": an op cannot be classified
/// Supported and then go untested. The instruction catalog (`all_op_variants`) is the whole
/// language; as features land, the Supported subset — and the corpus exercising it — grows toward
/// the entire catalog. THE proof that we have the full language, not a hand-picked corner of it.
#[test]
fn every_supported_instruction_is_exercised_with_wasm_eq_vm_eq_treewalker() {
    // Instructions appearing in corpus programs the WASM backend COMPILES — and which therefore
    // are proven equivalent: success-corpus ops matched the VM and tree-walker
    // (`wasm_equals_vm_and_treewalker_over_the_corpus`), error-corpus ops trapped exactly where the
    // oracle errored (`wasm_traps_where_treewalker_errors`). Both prove WASM == VM == Tree-walker.
    let mut exercised: BTreeSet<&'static str> = BTreeSet::new();
    for (_, src) in CORPUS
        .iter()
        .chain(ERROR_CORPUS)
        .chain(ARGS_CORPUS)
        .chain(CONCURRENCY_CORPUS)
        .chain(NET_CORPUS)
    {
        if compile_to_wasm(src).is_ok() {
            if let Some(ops) = program_ops(src) {
                for op in &ops {
                    exercised.insert(op_name(op));
                }
            }
        }
    }

    let catalog = all_op_variants();
    // Completeness guard: the catalog enumerates ~the whole instruction set, so the proof can
    // never go vacuous by silently dropping variants from `all_op_variants`.
    assert!(
        catalog.len() >= 100,
        "the instruction catalog shrank to {} — a variant was dropped from all_op_variants",
        catalog.len()
    );

    let mut unexercised: Vec<&'static str> = Vec::new();
    for op in &catalog {
        if matches!(op_support(op), Support::Supported) && !exercised.contains(op_name(op)) {
            unexercised.push(op_name(op));
        }
    }
    unexercised.sort_unstable();
    unexercised.dedup();
    assert!(
        unexercised.is_empty(),
        "FULL-LANGUAGE COVERAGE GAP: these instructions are classified Supported but NO compiling \
         corpus program exercises them — so WASM == VM == Tree-walker is UNPROVEN for them: \
         {unexercised:?}. Add a corpus program that uses each (or fix the classification): a feature \
         may not be claimed supported without an end-to-end test."
    );
}

/// The generics corpus (`generics_all_tiers.rs` runs it on the tree-walker, VM, AOT-Rust and C) on
/// WASM. Generics are fully supported, so every program must COMPILE to a self-contained module and
/// print exactly its expected output — a refusal is a gap, not an excuse.
#[test]
fn the_generics_corpus_runs_on_wasm() {
    const GENERICS: &[(&str, &str, &str)] = include!("generics_corpus.in");
    let mut failures = Vec::new();
    for (name, src, expected) in GENERICS {
        match compile_to_wasm(src) {
            Ok(module) => match run_aot_result(&module) {
                Ok(out) if out.trim() == *expected => {}
                Ok(out) => failures.push(format!(
                    "{name}: printed {:?}, expected {expected:?}\n{src}",
                    out.trim()
                )),
                Err(e) => failures.push(format!("{name}: trapped: {e}\n{src}")),
            },
            Err(e) => failures.push(format!("{name}: refused: {e:?}\n{src}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} generic programs fail on WASM:\n\n{}",
        failures.len(),
        GENERICS.len(),
        failures.join("\n\n")
    );
}

/// The failure corpus (`failure_all_tiers.rs` runs it on the tree-walker, VM, AOT-Rust, C, the JIT
/// and the wire format) on WASM: the same output before the stop, and the SAME reason, byte for byte.
#[test]
fn the_failure_corpus_stops_with_the_same_reason_on_wasm() {
    const FAILURES: &[(&str, &str, &str, &str)] = include!("failure_corpus.in");
    let mut failures = Vec::new();
    for (name, src, expected_out, reason) in FAILURES {
        match crate::common::wasm_host::run_outcome(src) {
            Ok(run) => {
                let expected_failure = (!reason.is_empty()).then(|| reason.to_string());
                if run.output.trim() != *expected_out || run.failure != expected_failure {
                    failures.push(format!(
                        "{name}: output {:?}, failure {:?}; expected {expected_out:?}, {reason:?}\n{src}",
                        run.output.trim(),
                        run.failure
                    ));
                }
            }
            Err(e) => failures.push(format!("{name}: {e}\n{src}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} failure programs diverge on WASM:\n\n{}",
        failures.len(),
        FAILURES.len(),
        failures.join("\n\n")
    );
}
