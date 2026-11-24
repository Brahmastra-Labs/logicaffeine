//! Tier-up seam: the VM profiles calls and hands HOT functions to a pluggable
//! native backend. The backend (the copy-and-patch JIT in
//! `logicaffeine_forge`) is injected as a trait object by whatever binary
//! links both crates — this crate publishes no new dependencies, and WASM
//! builds simply never install a tier.
//!
//! Deopt contract: the native code only handles the integer subset, so the
//! call site GUARDS — if any argument is not an Int, or compilation bailed,
//! the bytecode path runs instead. Both paths are differentially tested to
//! produce identical outcomes.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::instruction::{Constant, EnumTypeDef, Op};
use super::value::Value;

/// The per-program native ENTRY TABLE (EXODIA 4.7's hot-swap seam): two
/// atomic slots per function — [entry pointer, register count] — written
/// when a function's chain compiles, read by native call stencils through a
/// patched slot address, and the place Tier-2 swaps optimized code in. The
/// backing Vec never reallocates (fixed at construction), so slot addresses
/// are stable for the program's lifetime.
#[derive(Debug)]
pub struct FnTable {
    slots: Vec<AtomicI64>,
}

impl FnTable {
    pub fn new(functions: usize) -> Self {
        FnTable {
            slots: (0..functions * 2).map(|_| AtomicI64::new(0)).collect(),
        }
    }

    /// Publish a compiled entry (pointer + callee register count).
    pub fn publish(&self, fi: usize, entry: i64, register_count: i64) {
        self.slots[fi * 2 + 1].store(register_count, Ordering::Release);
        self.slots[fi * 2].store(entry, Ordering::Release);
    }

    /// The address of function `fi`'s [entry, regcount] slot pair — patched
    /// into call stencils as an immediate.
    pub fn slot_addr(&self, fi: usize) -> i64 {
        &self.slots[fi * 2] as *const AtomicI64 as i64
    }
}

/// Per-program cells the native tier shares with every chain it compiles:
/// the deopt status (any side-exit anywhere unwinds the whole native stack),
/// and the LIVE LOGOS DEPTH the call stencils count against MAX_CALL_DEPTH.
/// `Clone` is an `Arc` bump on each cell — a background-compile request carries a
/// clone so the worker patches the same per-program table/status/depth the
/// interpreter does.
#[derive(Debug, Clone)]
pub struct NativeCtx {
    pub table: Arc<FnTable>,
    pub status: Arc<AtomicI64>,
    pub depth: Arc<AtomicI64>,
}

/// What one native function call produced.
#[derive(Debug)]
pub enum NativeOutcome {
    /// A fully re-boxed return value (list-returning functions: the
    /// backend owns the re-boxing because the allocation registry lives
    /// on its side).
    ReturnValue(Value),
    /// Precise side exit: effects already landed stay landed; the VM
    /// materializes `frames` as real CallFrames and resumes interpreting
    /// at `resume_pc` (region-grade deopt for functions).
    DeoptAt {
        resume_pc: usize,
        frames: Vec<NativeFrame>,
    },
    /// The function ran to its return.
    Return(i64),
    /// A checked op side-exited (zero divisor). The native code touched only
    /// its private frame, and every adaptable body is effect-free, so the
    /// caller simply re-runs the call on bytecode — where the kernel raises
    /// the exact error at the exact point.
    Deopt,
}

/// A final mutable argument: raw scalar bits or a borrowed list handle, or an
/// owned value detached from the native allocation registry before it is drained.
#[derive(Debug)]
pub enum NativeWriteback {
    Raw(i64),
    Value(Value),
}

/// What one native region run produced.
#[derive(Debug)]
pub enum RegionOutcome {
    /// The region ran to its exit; write-back may proceed.
    Completed,
    /// Side exit: discard the private frame (NO write-back) and let the
    /// bytecode loop re-run from the head — VM registers are untouched, so
    /// the replay recomputes deterministically up to the faulting op.
    Deopt,
    /// PRECISE side exit: the region's non-array scalar registers are live in
    /// the frame; materialize them into the VM registers and resume the
    /// bytecode AT `resume_pc` (the faulting op) — no replay-from-head, no
    /// buffer truncate. This is the contract that lets a region with a
    /// `ListPush` coexisting with an in-place `SetIndex` (BFS-style worklists)
    /// tier up soundly: completed iterations' effects stand, and the faulting
    /// op re-runs once on bytecode (raising the exact error or continuing).
    DeoptAt { resume_pc: usize },
    /// A native callee stopped inside this region's call. Restore the region
    /// at the call site, then install the callee frames before resuming it.
    DeoptCall {
        call_pc: usize,
        resume_pc: usize,
        frames: Vec<NativeFrame>,
    },
}

/// A `Return` statement INSIDE the region (the siftDown early-exit shape):
/// the value slot, its re-boxing kind, and the flag slot the return path
/// sets — after write-back the VM performs the actual function return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionReturn {
    pub flag_slot: u16,
    pub value_slot: u16,
    pub kind: RegionReturnKind,
}

/// How a region-return value travels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionReturnKind {
    /// The value slot holds the raw representation, re-boxed by SlotKind.
    Slot(SlotKind),
    /// The value slot holds a VM REGISTER NUMBER whose current value
    /// returns (lists: mutated in place through their pins, the register
    /// still holds the same Rc).
    Register,
}

/// A natively-compiled function: integer registers in, integer result out.
/// `depth` is the LIVE LOGOS frame count at the call (the callee's own frame
/// included) — native self-calls count against the same MAX_CALL_DEPTH the
/// bytecode enforces.
pub trait NativeFn: Send + Sync {
    /// `args` are the marshalled parameter slots (scalars by value, floats
    /// as bits, list params as don't-care placeholders); `pins` are the
    /// pre-extracted `[vec_handle, ptr, len]` triples, one per list
    /// parameter in declaration order, landing in the frame's pin slots.
    fn call(&self, args: &[i64], pins: &[i64], depth: usize) -> NativeOutcome;
    /// On success, return each requested final argument exactly once. A list
    /// output must either own a boxed value or be one of the boundary pin handles;
    /// native temporary storage must remain valid until ownership is transferred.
    /// A precise deopt carries output state in its frames instead. Backends
    /// without an output-argument ABI decline before executing such a call.
    fn call_with_writeback(
        &self,
        args: &[i64],
        pins: &[i64],
        depth: usize,
        slots: &[u16],
        writeback: &mut Vec<(u16, NativeWriteback)>,
    ) -> NativeOutcome {
        let _ = writeback;
        if slots.is_empty() {
            self.call(args, pins, depth)
        } else {
            NativeOutcome::Deopt
        }
    }

    /// Materialize a precise side exit from a call made by a native region.
    /// The frame starts at this function's entry window; pins name the
    /// region's borrowed buffers. Owned values must detach before cleanup.
    fn materialize_side_exit(
        &self,
        _status: i64,
        _entry_depth: usize,
        _frame: &[i64],
        _pins: &[i64],
    ) -> Option<NativeOutcome> {
        None
    }

    /// How the result re-boxes: a scalar from the raw i64, or the IDENTITY
    /// of one of the caller's list arguments (return-by-parameter).
    fn ret(&self) -> NativeRet {
        NativeRet::Scalar(SlotKind::Int)
    }
    /// The chain's entry pointer, for the program's [`FnTable`].
    fn entry_ptr(&self) -> i64;
    /// The regcount value to PUBLISH next to the entry pointer:
    /// `frame_size − 3`, which the call stencil's bound math and limit
    /// planting are calibrated against (the plain register count for the
    /// classic layout).
    fn published_regc(&self) -> i64;
    /// The machine code behind this function, as `(start address, length)` extents — what the
    /// `LOGOS_PERF_MAP` symbol map names. Default: none to report.
    fn code_extents(&self) -> Vec<(usize, usize)> {
        Vec::new()
    }
}

/// How a native function's return value re-boxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRet {
    Scalar(SlotKind),
    /// The function returns its j-th parameter (a list passed through —
    /// the caller re-boxes by cloning the argument's Rc, preserving
    /// identity).
    ListParam(u8),
    /// The function returns a list BY VEC HANDLE: a registry-owned fresh
    /// allocation (detach and wrap) or one of the caller's list arguments
    /// (match the pin handles, clone that argument). The backend resolves
    /// it and emits [`NativeOutcome::ReturnValue`].
    ListByHandle,
    /// The function returns one of the program's enum values: the raw result is an
    /// `Rc::into_raw(Rc<InductiveValue>)` pointer carrying one strong count, which the caller
    /// takes over. The other half of [`ParamKind::Inductive`]'s protocol.
    Inductive,
}

/// A DECLARED parameter's native representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamKind {
    Scalar(SlotKind),
    /// `Seq of <scalar>` — pinned at the boundary exactly like a region
    /// array (one borrow for the whole call, buffer pointer + length in
    /// dedicated frame slots).
    List(PinElem),
    /// One of the program's own enum types. The argument travels as `Rc::as_ptr` of the VM's
    /// shared node: BORROWED for the whole call, since the caller's register keeps the node
    /// alive and a node is never mutated. Native code that keeps a node past the call, by
    /// storing it in a new node or returning it, takes its own strong count.
    Inductive,
}

/// One materialized native frame for a PRECISE deopt (outermost first).
/// `offset`/`return_pc`/`return_reg` describe the link from the PREVIOUS
/// frame (they are meaningless for index 0, whose link belongs to the
/// interpreted call site).
#[derive(Debug)]
pub struct NativeFrame {
    pub offset: usize,
    pub return_pc: usize,
    pub return_reg: u16,
    pub regs: Vec<i64>,
    pub kinds: Vec<RegBox>,
    /// Registers whose values the BACKEND already re-boxed (native-owned
    /// fresh lists detached from the allocation registry at deopt — they
    /// must survive into the materialized frame).
    pub resolved: Vec<(u16, Value)>,
}

/// How one raw register slot re-boxes during frame materialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegBox {
    /// A live empty loan slot.
    Nothing,
    /// Not defined at this point — leave the register as Nothing.
    Dead,
    Int,
    Bool,
    Float,
    /// A `Word32` carried unboxed as its zero-extended `u32` value — re-boxes to
    /// `RuntimeValue::Word(WordVal::W32(Word32(bits as u32)))`.
    Word32,
    /// A `Word64` carried unboxed as its 64-bit pattern — re-boxes to
    /// `RuntimeValue::Word(WordVal::W64(Word64(bits as u64)))`.
    Word64,
    /// A `chr(code)` result carried unboxed as a (guaranteed-valid) Unicode scalar code point — re-boxes to
    /// a 1-char `Text`. The producing native op deopts on an invalid code, so `char::from_u32` never fails.
    CharText,
    /// Holds the identity of list parameter j.
    ListParam(u8),
    /// Backend-resolved (see [`NativeFrame::resolved`]); the kind row
    /// entry is a placeholder.
    Resolved,
}

/// A backend that can try to compile one VM function to native code.
pub trait NativeTier: Send + Sync {
    /// Attempt to compile the function whose bytecode is `code`
    /// (`code[0]` is the instruction at `entry_pc`; jump targets inside are
    /// ABSOLUTE program pcs and need rebasing by `entry_pc`). Return None to
    /// leave the function on the bytecode path forever.
    ///
    /// `enums` is the program's declared enum types, the payload kinds an `Inspect` arm binds.
    ///
    /// `callees` carries every program function's declared signature,
    /// indexed by `FuncIdx` — calls to OTHER functions compile to table
    /// dispatch when the callee's signature is all-scalar (an unpublished
    /// callee deopts at the call until it tiers up on its own).
    ///
    /// `cow_exempt_regs` is the function's `CompiledFunction::cow_exempt_regs` — the ONLY registers
    /// the VM writes a shared list through in place. Every other shared list is copied on its first
    /// write, so a backend that stores into a list in place must prove the target is unshared or in
    /// this set, or leave the function on bytecode.
    ///
    /// `values[p]` is the `Int` or `Bool` every call of the function's warm-up passed for parameter `p`
    /// (`None` when the calls differed, or the parameter is of another type). The code may be specialized
    /// on a value, and must then be versioned at its own entry: native callers reach it through the
    /// dispatch table, past every guard the VM applies, so a call passing another value must run code
    /// that does not depend on it.
    #[allow(clippy::too_many_arguments)]
    fn compile_function(
        &self,
        code: &[Op],
        entry_pc: usize,
        constants: &[Constant],
        enums: &[EnumTypeDef],
        param_count: u16,
        register_count: u16,
        self_fi: u16,
        param_kinds: &[Option<ParamKind>],
        values: &[Option<ScalarValue>],
        cow_exempt_regs: &[u16],
        ret_kind: Option<SlotKind>,
        ctx: &NativeCtx,
        callees: &[CalleeSig],
    ) -> Option<Box<dyn NativeFn>>;

    /// Attempt to compile a loop region. `code[0]` is the op at `head_pc`
    /// (the back-edge target); the slice ends at the back-edge jump
    /// (inclusive). Every jump out of the region must target `exit_pc`.
    /// `named[r]` marks frame registers carrying a user-visible name — the
    /// only slots whose post-region values are observable (scratches are
    /// dead at statement boundaries by the compiler's allocation
    /// discipline). `observed[r]` is the register's runtime kind at this hot
    /// crossing — the speculation seed, re-checked by the guard set on every
    /// entry. `values[r]` is the register's `Int` or `Bool` at the same crossing
    /// (`None` when it holds neither, and for every register once the loop has
    /// fallen back to its generic region): a region may specialize on a value
    /// no op in it changes, and reports each one it did in
    /// [`RegionFn::value_guards`]. `cold` is the code the loop's warm-up never ran
    /// ([`cold_paths`]): a region may leave it to the VM, exiting at each of its
    /// entries. Default: regions stay on bytecode.
    #[allow(clippy::too_many_arguments)]
    fn compile_region(
        &self,
        code: &[Op],
        head_pc: usize,
        exit_pc: usize,
        constants: &[Constant],
        register_count: u16,
        named: &[bool],
        observed: &[ObservedKind],
        values: &[Option<ScalarValue>],
        cold: &ColdPaths,
        ctx: &NativeCtx,
        callees: &[CalleeSig],
    ) -> Option<Box<dyn RegionFn>> {
        let _ = (
            code,
            head_pc,
            exit_pc,
            constants,
            register_count,
            named,
            observed,
            values,
            cold,
            ctx,
            callees,
        );
        None
    }

    /// The name this tier reports under `LOGOS_ENGINE_TRACE`.
    fn engine_name(&self) -> &'static str {
        "native"
    }

    /// Native compilation coverage accumulated by this tier.
    fn compile_summary(&self) -> String {
        String::new()
    }

    /// Stable reasons why attempted native compilations declined.
    fn decline_summary(&self) -> String {
        String::new()
    }

    /// Native-only optimizations that actually fired.
    fn optimization_summary(&self) -> String {
        String::new()
    }

    /// Actual native entries, completions and deoptimizations.
    fn execution_summary(&self) -> String {
        String::new()
    }
}

/// The runtime kind a native frame slot carries: entry guards check the VM
/// register's discriminant against it and copy the raw representation in
/// (f64 travels as bits); write-back re-boxes by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotKind {
    Int,
    Bool,
    Float,
    /// A `chr(code)` result carried as a (guaranteed-valid) Unicode scalar code point; write-back re-boxes
    /// it to a 1-char `Text`. Valid ONLY as a region OUTPUT (write-back) — the region gate rejects it as a
    /// live-in guard input (a `chr` result never enters a region from the VM side).
    CharText,
    /// A `Word32`/`Word16` ring value carried UNBOXED as a zero-extended masked `i64` (`0 ≤ slot < 2³²`).
    /// UNLIKE `CharText`, this is BIDIRECTIONAL: the entry guard verifies the VM register is a
    /// `RuntimeValue::Word(W32)` and copies its masked value in, and write-back re-boxes the low 32 bits to
    /// `RuntimeValue::Word(W32(...))`. This is what lets a loop-carried Word (a `sha256`/MD5 hash word
    /// `h0..h7`) tier as region state.
    Word32,
    /// A `Word64` ring value carried UNBOXED as the full 64-bit pattern in the `i64` slot. Bidirectional:
    /// the entry guard verifies a `RuntimeValue::Word(W64)` and copies its bits in; write-back re-boxes to
    /// `RuntimeValue::Word(W64(...))`.
    Word64,
}

/// A scalar a register held at a region's hot crossing, or a parameter on every call of a function's warm-up: what
/// native code may be specialized on, and what its guard requires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarValue {
    /// An exact `Int` that fits in 64 bits.
    Int(i64),
    /// A `Bool`.
    Bool(bool),
}

impl ScalarValue {
    /// The value as a native frame slot carries it: an `Int` as itself, a `Bool` as 0 or 1.
    pub fn bits(self) -> i64 {
        match self {
            ScalarValue::Int(n) => n,
            ScalarValue::Bool(b) => i64::from(b),
        }
    }
}

/// What a function's warm-up passed for one parameter. A function compiled once its warm-up is over may be
/// specialized on each parameter that [`EntryValue::held`] one value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryValue {
    /// No call has been seen.
    Unseen,
    /// Every call seen passed this value.
    Held(ScalarValue),
    /// Two calls passed different values, or one passed neither an `Int` that fits in 64 bits nor a `Bool`.
    Varied,
}

impl EntryValue {
    /// Fold in one more call's argument (`None` when it is neither an `Int` that fits in 64 bits nor a `Bool`).
    pub(crate) fn observe(&mut self, argument: Option<ScalarValue>) {
        *self = match (*self, argument) {
            (EntryValue::Unseen, Some(value)) => EntryValue::Held(value),
            (EntryValue::Held(held), Some(value)) if held == value => EntryValue::Held(held),
            _ => EntryValue::Varied,
        };
    }

    /// The one value every call passed, if there was one.
    pub(crate) fn held(self) -> Option<ScalarValue> {
        match self {
            EntryValue::Held(value) => Some(value),
            EntryValue::Unseen | EntryValue::Varied => None,
        }
    }
}

/// What the VM OBSERVED in each register at the hot back-edge that triggered
/// region compilation — the kinds the adapter SPECULATES on (sound because
/// every later entry re-checks them via the guard set).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedKind {
    Int,
    Bool,
    Float,
    /// A List whose payload is the unboxed all-Int repr.
    IntList,
    /// A List whose payload is the half-width (`Vec<i32>`) narrowed Int repr
    /// (`ListRepr::IntsI32`). Pinned as a 4-byte-element buffer (sign-extending
    /// loads, truncating stores) under `LOGOS_NARROW_VM`.
    IntListI32,
    /// A List whose payload is the unboxed all-Float repr.
    FloatList,
    /// A List whose payload is the unboxed all-Bool repr.
    BoolList,
    /// A Map (the boxed kernel storage rides a pinned pointer; the int
    /// fast lane verifies keys/values per helper call).
    Map,
    /// An ASCII `Text` carried AS BYTES: char index == byte index and char
    /// count == byte length, so `item i of text` is a pinned 1-byte load and
    /// char compares are integer byte compares. Observed ONLY when the Text is
    /// ASCII; a non-ASCII Text stays [`ObservedKind::Other`] (keeps bailing) so
    /// the per-char decode path runs and the JIT can never diverge.
    TextBytes,
    /// A `Word32`/`Word16` ring value (`RuntimeValue::Word(W32)`) observed at the hot crossing — carried
    /// UNBOXED as a zero-extended masked `i64`. Lets a loop-carried Word (a hash accumulator) tier.
    Word32,
    /// A `Word64` ring value (`RuntimeValue::Word(W64)`) observed at the hot crossing — carried as the full
    /// 64-bit pattern in the `i64` slot.
    Word64,
    /// A packed `Word32` LIST (`ListRepr::Words32`) — pins with the IntList buffer machinery (masked `i64`
    /// elements); an indexed read is tagged `Word32`. Lets a `Seq of Word32` table pin + tier.
    Word32List,
    /// Anything the native frame cannot carry (non-ASCII Text, boxed lists,
    /// Nothing…).
    Other,
}

/// One pinned array for a region run: the register holding the list, the
/// frame slots that receive its buffer pointer and length at entry, and the
/// element kind the region speculated on. The VM borrows each DISTINCT
/// `Rc<RefCell<…>>` exactly once for the whole native run (aliased registers
/// resolve to the same buffer, so native writes through one name are visible
/// through every other — and there is zero refcount or borrow traffic inside
/// the loop). Arrays mutate IN PLACE: no write-back, and the deopt replay is
/// sound by prefix-idempotence (the replay recomputes exactly the values the
/// native prefix already wrote).
/// Element kind of a pinned buffer (decides the stencil's access width:
/// 8-byte for Int/Float bits, 1-byte for Bool).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PinElem {
    Int,
    /// A pinned half-width Int buffer (`ListRepr::IntsI32` = `Vec<i32>`): the
    /// stencil/regalloc access width is 4 bytes — loads sign-extend (`movsxd`),
    /// stores truncate the low 4 bytes. The narrowing proof guarantees every
    /// stored value fits `i32`, so truncation is lossless; a region-entry pin
    /// only admits an `IntsI32` buffer for this lane.
    IntI32,
    /// A pinned packed `Word32` buffer (`ListRepr::Words32` = masked values in `Vec<i64>`): the load/store
    /// width and stencils are IDENTICAL to [`PinElem::Int`] (8-byte); the ONLY difference is that an indexed
    /// read is tagged `Word32` (not `Int`), so the Word ops downstream accept it. Read pattern of a sha256
    /// message schedule / round-constant table.
    Word32,
    Float,
    Bool,
    /// A pinned MAP: `vec_slot` carries `&mut MapStorage as *mut _`;
    /// the ptr/len slots are unused. Get/set/contains go through pure
    /// helpers against the kernel's own storage (iteration order is
    /// untouched by construction).
    Map,
    /// A pinned ASCII `Text` carried AS BYTES: `ptr_slot` receives the string's
    /// byte buffer pointer (`Rc<String>::as_bytes().as_ptr()`) and `len_slot`
    /// its BYTE length (== char count for ASCII). Read-only — the `Rc<String>`
    /// is never mutated in place, so no snapshot/rollback applies; the entry
    /// pin RE-CHECKS ASCII and declines (deopt) on a non-ASCII Text.
    TextBytes,
    /// A pinned MUTABLE `Text` accumulator (`Set text to text + ch`): `vec_slot`
    /// receives a `*mut Value` pointing AT THE VM REGISTER CELL (the cell is
    /// stable for the whole native run, even though the `Rc<String>` inside it
    /// reallocs/COWs on append). The `ptr_slot`/`len_slot` are unused. The
    /// `logos_rt_str_append` helper grows the accumulator THROUGH this pointer
    /// with EXACTLY the VM's `add_assign` semantics — in place when the
    /// `Rc<String>` is sole-owned, copy-on-write (a fresh `Rc` written back into
    /// the cell, the alias untouched) otherwise — so the tiered run is
    /// bit-identical to the tree-walker for every alias case. The entry pin
    /// declines (deopt to bytecode) if the observed value is not a `Text`. A
    /// classic replay-from-head `Deopt` is NOT replay-idempotent over an
    /// already-grown accumulator (it would double-append), so the VM SNAPSHOTS
    /// the register's `Value` on entry and restores it before a classic replay
    /// (see [`ArrayPin::mutated`]); a precise region resumes at the faulting op
    /// and keeps the live grown value.
    TextMut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrayPin {
    pub reg: u16,
    /// Frame slot receiving `&mut Vec<…> as *mut _ as i64` — the handle the
    /// push helper extends through.
    pub vec_slot: u16,
    pub ptr_slot: u16,
    pub len_slot: u16,
    pub elem: PinElem,
    /// This register participates in an in-region list `RevSwap`. Native code
    /// swaps the pin triples in O(1), so on completion (or precise deopt) the
    /// VM register must be rebound to the `Rc` whose vector handle ended in
    /// this pin. Ordinary pinned arrays leave this false and pay no rebind or
    /// refcount traffic at the region boundary.
    pub rebound: bool,
    /// This buffer is written IN PLACE (`SetIndex`) in a region that can take a
    /// recoverable side exit AND replays from the head on deopt (non-precise).
    /// Such a write is NOT replay-idempotent (a read-modify-write or swap would
    /// double-apply), so the VM snapshots this buffer's full contents on region
    /// entry and restores them on a classic `Deopt` before replaying. Read-only
    /// buffers, push-only buffers (length truncate suffices), and precise
    /// regions (resume-at-op, no replay) leave this `false`.
    pub mutated: bool,
}

/// A natively-compiled LOOP REGION (OSR-lite): no arguments, no return —
/// every effect flows through the enclosing frame's registers.
/// What a REGION needs to know about a function it might call: the
/// declared signature (all-scalar = replay-pure = inlinable) and the
/// function index for its table slot.
#[derive(Debug, Clone)]
pub struct CalleeSig {
    /// Explicit mutable outputs, excluding inferred consume-and-return borrows.
    pub inout_params: Vec<u16>,
    pub param_kinds: Vec<Option<ParamKind>>,
    /// Declared scalar return; `None` keeps the callee out of regions.
    pub ret: Option<SlotKind>,
    /// Declared list return element kind. This is separate from `ret` because
    /// scalar returns ride one raw word while list returns ride a Vec handle.
    /// Only element representations the native tier can materialize are
    /// present here; `None` also covers void and unsupported composite returns.
    pub list_ret: Option<PinElem>,
    /// List-param call admission (a region may call a list-parameter function). Sound only when
    /// the callee never reallocates a list-param buffer (no `Push` reaching a
    /// list param) — then the caller's derived raw buffer pointer stays valid
    /// across the call. `true` ⇒ list args don't move under the call.
    pub list_params_stable: bool,
    /// Every `Return` traces (through `Move`s) to a list PARAMETER, so the
    /// returned handle aliases a passed-in (already-pinned) buffer rather than a
    /// fresh one. Lets a LIST-returning callee be admitted; a scalar return is
    /// governed by `ret` and ignores this. `false` for non-list / fresh-list
    /// / multi-element-kind returns.
    pub returns_list_param: bool,
}

/// A hoisted region-entry bounds check (V8 loop bound-check elimination): the
/// loop's covered accesses run unchecked iff, at entry, the pinned array is
/// long enough for the whole loop and the induction floor is in range. The VM
/// declines the region (replays on bytecode) if it fails.
#[derive(Debug, Clone, Copy)]
pub struct HoistGuard {
    /// Frame slot holding the pinned array's length (loaded by the prologue).
    pub len_slot: u16,
    /// VM register holding the loop's upper bound.
    pub bound_reg: u16,
    /// VM register holding the induction variable (its value at entry is the
    /// loop's minimum for the remaining run).
    pub iv_reg: u16,
    /// `length >= bound + add_max` (max access index over the loop).
    pub add_max: i32,
    /// `iv + add_min >= 1` (min access index over the loop, 1-based).
    pub add_min: i32,
}

pub trait RegionFn: Send + Sync {
    /// Slots whose CURRENT values the region may read before writing (or
    /// must preserve across a conditional write): the VM guards each one's
    /// kind and copies its raw representation into the native frame.
    fn guard_set(&self) -> &[(u16, SlotKind)];
    /// Slots whose incoming values are provably DEAD (written before read,
    /// e.g. the loop-condition scratch): no guard, no copy-in.
    fn free_set(&self) -> &[u16];
    /// Slots the region was specialized on, with the kind and value each must hold
    /// at entry: the VM checks them after the kind guards, and an entry that finds
    /// another value runs the loop on bytecode. A slot here need not be in the
    /// guard set: a flag whose branch was folded away is no longer read. A
    /// specialized region that keeps missing is replaced by the generic one.
    /// Default: none.
    fn value_guards(&self) -> &[(u16, ScalarValue)] {
        &[]
    }
    /// Told by the VM when an entry found a value other than one [`Self::value_guards`] names, so the entry ran
    /// on bytecode. Default: nothing to record.
    fn note_value_guard_miss(&self) {}
    /// Told by the VM when it gives this value-specialized region up for the
    /// loop's generic one. Default: nothing to record.
    fn note_retired(&self) {}
    /// Told by the VM when an entry that missed this region's values ran the
    /// loop's generic region instead of bytecode. Default: nothing to record.
    fn note_fallback_entry(&self) {}
    /// Told by the VM when an entry's registers held kinds other than the ones
    /// [`Self::guard_set`] names, so the entry ran on bytecode. Default: nothing
    /// to record.
    fn note_guard_miss(&self) {}
    /// Told by the VM when it gives this region up for bytecode because it kept
    /// missing. Default: nothing to record.
    fn note_demoted(&self) {}
    /// For a region compiled without its cold code: how many times it exited to a
    /// cold entry and how many iterations it ran native, so the VM can demote one
    /// whose cold paths run often after all. Default: `None`, a whole region.
    fn cold_runs(&self) -> Option<(u64, u64)> {
        None
    }
    /// Slots the region writes, with their re-boxing kinds.
    fn write_set(&self) -> &[(u16, SlotKind)];
    /// Arrays the region reads/writes through pinned buffers (see
    /// [`ArrayPin`]). Default: none.
    fn array_set(&self) -> &[ArrayPin] {
        &[]
    }
    /// Hoisted region-entry bounds checks (see [`HoistGuard`]). Default: none.
    fn hoist_guards(&self) -> &[HoistGuard] {
        &[]
    }
    /// In-region `Return` support (see [`RegionReturn`]). Default: none —
    /// such regions bail.
    fn region_return(&self) -> Option<RegionReturn> {
        None
    }
    fn frame_size(&self) -> usize;
    /// Extra arena headroom beyond `frame_size` (regions that CALL
    /// functions window their callees past the frame and need real call
    /// arena depth). Default: none.
    fn arena_slots(&self) -> usize {
        0
    }
    /// PRECISE-deopt re-box kinds at `resume_pc`: one entry per VM register —
    /// `Some(kind)` re-boxes the frame slot's raw bits, `None` keeps the VM
    /// register's current value (a pinned array, or a read-only/unknown slot).
    /// `None` overall ⇒ this region does not use precise deopt. Only meaningful
    /// for a `RegionOutcome::DeoptAt { resume_pc }` outcome.
    fn precise_kinds(&self, _resume_pc: usize) -> Option<&[Option<SlotKind>]> {
        None
    }
    fn run(&self, frame: &mut [i64], depth: usize) -> RegionOutcome;
    /// Run with access to the functions published in this VM's entry table,
    /// so a callee side exit can transfer its live frames before cleanup.
    fn run_with_callees<'a>(
        &self,
        frame: &mut [i64],
        depth: usize,
        _callees: &dyn Fn(u16) -> Option<&'a dyn NativeFn>,
    ) -> RegionOutcome {
        self.run(frame, depth)
    }
    /// The machine code behind this region, as `(start address, length)` extents — what the
    /// `LOGOS_PERF_MAP` symbol map names. Default: none to report.
    fn code_extents(&self) -> Vec<(usize, usize)> {
        Vec::new()
    }
}

/// The process-wide tier, installed once by the binary that links a backend
/// (e.g. `logicaffeine-jit`). The live VM constructors attach it to every
/// program they run; nothing installs it on WASM, so the browser stays pure
/// bytecode.
static INSTALLED_TIER: std::sync::OnceLock<&'static (dyn NativeTier + 'static)> =
    std::sync::OnceLock::new();

/// Install `tier` as the process-wide native tier. Idempotent: the first
/// install wins and later calls return `false`.
pub fn install_native_tier(tier: &'static (dyn NativeTier + 'static)) -> bool {
    INSTALLED_TIER.set(tier).is_ok()
}

/// The installed process-wide tier, if any.
pub fn installed_native_tier() -> Option<&'static (dyn NativeTier + 'static)> {
    INSTALLED_TIER.get().copied()
}

/// Branch runs before one direction never taken counts as cold: below this, a direction that has not run yet is too
/// little evidence that it will not.
pub const COLD_EDGE_MIN_RUNS: u32 = 32;

/// The code of a loop body its warm-up never ran, for a region that compiles only what runs (partial compilation,
/// Whaley, OOPSLA 2001). An op is cold when every path from the loop head to it takes a branch direction that never
/// ran while its branch ran at least [`COLD_EDGE_MIN_RUNS`] times the other way. A region that leaves the cold code
/// to the VM exits at each cold entry, a cold op a warm branch leads to, and the VM resumes there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColdPaths {
    /// Per op of the region's code: whether no warm path reaches it.
    pub cold: Vec<bool>,
    /// The cold ops a warm branch leads to, in code order.
    pub entries: Vec<ColdEntry>,
}

/// Where a region leaves its cold code to the VM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColdEntry {
    /// The op's index in the region's code.
    pub at: usize,
    /// The registers live there: every value the VM may read once it resumes at the op, so each must hold its
    /// current value when the region exits.
    pub live: Vec<u16>,
}

impl ColdPaths {
    /// Whether the warm-up ran every op of the loop.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The cold code of the loop region `code` (the op at `head_pc` first, its back-edge last), from `branches`, each op's
/// `[not taken, taken]` count while the loop warmed (zero for an op that is not a conditional branch). Liveness for the
/// entries is over the whole region: an exit out of it reads every `named` register, and the back-edge reads what the
/// head does.
pub fn cold_paths(
    code: &[Op],
    head_pc: usize,
    branches: &[[u32; 2]],
    named: &[bool],
    functions: &[super::instruction::CompiledFunction],
) -> ColdPaths {
    let n = code.len();
    let local = |pc: usize| pc.checked_sub(head_pc).filter(|&i| i < n);
    // Every edge out of op i: `Some(index)` within the region or `None` leaving it, with whether it ran while the
    // loop warmed.
    let edges = |i: usize| -> Vec<(Option<usize>, bool)> {
        let [not_taken, taken] = branches.get(i).copied().unwrap_or([0, 0]);
        let ran = |this: u32, other: u32| !(this == 0 && other >= COLD_EDGE_MIN_RUNS);
        let next = (i + 1 < n).then_some(i + 1);
        match code[i] {
            Op::Jump { target } => vec![(local(target), true)],
            Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
                vec![(local(target), ran(taken, not_taken)), (next, ran(not_taken, taken))]
            }
            Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => Vec::new(),
            ref op => super::disasm::op_targets(op)
                .into_iter()
                .map(|t| (local(t), true))
                .chain(std::iter::once((next, true)))
                .collect(),
        }
    };
    let mut warm = vec![false; n];
    let mut stack = vec![0usize];
    while let Some(i) = stack.pop() {
        if i < n && !warm[i] {
            warm[i] = true;
            stack.extend(edges(i).into_iter().filter(|e| e.1).filter_map(|e| e.0));
        }
    }
    let cold: Vec<bool> = warm.iter().map(|w| !w).collect();
    let mut at: Vec<usize> = (0..n)
        .filter(|&i| warm[i])
        .flat_map(|i| edges(i).into_iter().filter(|e| !e.1).filter_map(|e| e.0))
        .filter(|&t| cold[t])
        .collect();
    at.sort_unstable();
    at.dedup();
    if at.is_empty() {
        return ColdPaths { cold, entries: Vec::new() };
    }
    let regs = named.len();
    let footprint: Vec<(Vec<u16>, Vec<u16>)> =
        code.iter().map(|op| super::instruction::op_def_uses(op, functions)).collect();
    let mut live_in = vec![vec![false; regs]; n];
    loop {
        let mut changed = false;
        for i in (0..n).rev() {
            let mut live = vec![false; regs];
            for (target, _) in edges(i) {
                match target {
                    Some(t) => live.iter_mut().zip(&live_in[t]).for_each(|(l, &x)| *l |= x),
                    None => live.iter_mut().zip(named).for_each(|(l, &x)| *l |= x),
                }
            }
            let (defs, uses) = &footprint[i];
            for &d in defs {
                if let Some(l) = live.get_mut(d as usize) {
                    *l = false;
                }
            }
            for &u in uses {
                if let Some(l) = live.get_mut(u as usize) {
                    *l = true;
                }
            }
            if live != live_in[i] {
                live_in[i] = live;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let entries = at
        .into_iter()
        .map(|i| ColdEntry {
            at: i,
            live: (0..regs as u16).filter(|&r| live_in[i][r as usize]).collect(),
        })
        .collect();
    ColdPaths { cold, entries }
}

/// A region compiled without its cold code is demoted once it has exited to cold code [`REGION_DEMOTE_AFTER`] times
/// and on more than one iteration in this many: the warm-up's cold path has become a common one, and an exit per
/// iteration costs more than the bytecode it skips.
pub const COLD_EXIT_SHARE: u64 = 16;

/// Calls before a function is considered hot.
pub const NATIVE_TIER_THRESHOLD: u32 = 100;

/// Back-edge crossings before a Main loop is considered hot.
pub const REGION_TIER_THRESHOLD: u32 = 100;

/// Per-region tier state (keyed by loop-head pc).
pub(crate) enum RegionSlot {
    Failed,
    Ready {
        rf: Box<dyn RegionFn>,
        exit_pc: usize,
        /// Kind-guard failures (one for each entry) and side exits, over the rate
        /// window. A region whose misses reach [`REGION_DEMOTE_AFTER`] and outnumber
        /// its `completions` re-runs work on most entries: a value-specialized
        /// region gives way to the generic one, a generic region demotes to Failed
        /// so pure bytecode takes over.
        misses: u32,
        /// Entries that ran native, over the same window as `misses`.
        completions: u32,
        /// Entries a value-specialized `rf` ran.
        hits: u32,
        /// Entries that found other values than a value-specialized `rf` was
        /// specialized on, one for each entry.
        value_misses: u32,
        /// The loop's generic region, which runs an entry whose values a
        /// value-specialized `rf` does not hold.
        fallback: RegionFallback,
    },
}

/// The generic region behind a value-specialized one.
pub(crate) enum RegionFallback {
    /// No entry has missed the specialized region's values yet.
    Untried,
    /// Compiled; `misses` counts its own kind-guard failures and side exits.
    Ready { rf: Box<dyn RegionFn>, misses: u32 },
    /// The loop cannot run native unspecialized (or its generic region kept
    /// missing): an entry that misses the values runs on bytecode.
    Failed,
}

/// Guard failures and side exits a Ready region must reach before it demotes; it demotes then only while they
/// outnumber the entries it ran native, both counted over [`REGION_RATE_WINDOW`].
pub const REGION_DEMOTE_AFTER: u32 = 8;

/// Misses and completed entries together past which a Ready region halves both: demotion weighs the recent entries,
/// not the region's whole life.
pub(crate) const REGION_RATE_WINDOW: u32 = 64;

/// Halve a region's misses and completions once together they fill [`REGION_RATE_WINDOW`].
pub(crate) fn windowed(misses: &mut u32, completions: &mut u32) {
    if *misses + *completions >= REGION_RATE_WINDOW {
        *misses /= 2;
        *completions /= 2;
    }
}

/// Per-function tier state.
pub(crate) enum NativeSlot {
    /// Still profiling (or below threshold).
    Untried,
    /// Submitted to the BACKGROUND compiler (HOTSWAP §6); runs bytecode until the
    /// worker's result is drained and published. Not re-submitted while pending.
    Pending,
    /// Compilation was attempted and bailed — never retried.
    Failed,
    /// Compiled; the guard still applies per call.
    Ready(Box<dyn NativeFn>),
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/native_tier/tests.rs"]
mod tests;
