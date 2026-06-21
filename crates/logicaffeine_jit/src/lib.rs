//! The LOGOS native tier: the bridge from the bytecode VM's tier-up seam to
//! the copy-and-patch forge JIT.
//!
//! The VM profiles calls and Main-loop back-edges; when something goes hot it
//! asks its installed [`NativeTier`] to compile. This crate is that tier:
//! [`ForgeTier`] translates VM bytecode into the forge's [`MicroOp`] subset
//! (functions AND loop regions), compiles it to a native stencil chain, and
//! hands it back. Anything outside the integer subset BAILS (`None`) and stays
//! on bytecode forever — the deopt contract.
//!
//! Soundness rests on three legs, each differentially tested in
//! `logicaffeine-tests`:
//! - the kind-inference dataflow (params are Int via the entry guard,
//!   comparisons are Bool, arithmetic requires Int operands);
//! - per-call / per-entry guards (a non-Int argument routes the call back to
//!   bytecode);
//! - the write-back contract for regions (incoming-dead scratches need no
//!   guard, writes re-box by inferred kind).
//!
//! Production binaries call [`install`] once at startup; the live VM
//! constructors pick the tier up from the global seam. WASM builds compile
//! this crate to nothing — the browser runs pure bytecode.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicU32, Ordering};

use logicaffeine_compile::vm::{
    install_native_tier, ArrayPin, CalleeSig, Constant, FnTable, HoistGuard, NativeCtx, NativeFn,
    NativeFrame, NativeOutcome, NativeRet, NativeTier, ObservedKind, Op, ParamKind, PinElem,
    RegBox, RegionFn, RegionOutcome, SlotKind,
};
use logicaffeine_forge::jit::{
    compile_straightline_coded, compile_straightline_pinned_float, compile_straightline_with,
    ChainOutcome, Cmp, CompiledChain, FOp, IOp, MicroOp, RmwOp,
};

// The precise/self call stencils BAKE the depth limit (their holes are all
// spoken for), so the kernel's `MAX_CALL_DEPTH` and the forge's baked value
// MUST agree — otherwise native recursion would side-exit at a different depth
// than the kernel raises its error, diverging from the tree-walker. Pinned at
// compile time so any future change to either constant fails the BUILD here
// rather than silently miscompiling deep recursion.
const _: () = assert!(
    logicaffeine_compile::semantics::MAX_CALL_DEPTH as i64
        == logicaffeine_forge::jit::BAKED_CALL_DEPTH,
    "MAX_CALL_DEPTH and the forge's baked call-depth limit have drifted; \
     update stencils/int_stencils.rs and jit.rs BAKED_CALL_DEPTH to match",
);

/// Runtime push helpers the push stencil calls indirectly (their addresses
/// are baked as constants — no relocations). Each pushes the value with the
/// kernel's representation and refreshes the pinned pointer/length slots
/// after a possible realloc.
///
/// # Safety
/// The vec-handle slot must hold a live `*mut Vec<…>` of the matching
/// element type, pinned (borrowed) by the VM for the whole region run.
/// The NATIVE ALLOCATION REGISTRY: every list a chain allocates lives
/// here until its boundary. On success the returned list detaches;
/// everything else drops. On deopt everything drops — fresh lists never
/// escape mid-run, so replay is leak-free and double-build-free.
std::thread_local! {
    static ALLOC_REGISTRY: std::cell::RefCell<Vec<*mut Vec<i64>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Registry depth — drained-to-zero is part of every boundary's contract
/// (asserted by the test gates).
pub fn native_alloc_registry_len() -> usize {
    ALLOC_REGISTRY.with(|r| r.borrow().len())
}

fn alloc_registry_drain() {
    ALLOC_REGISTRY.with(|r| {
        for ptr in r.borrow_mut().drain(..) {
            drop(unsafe { Box::from_raw(ptr) });
        }
    });
}

/// Detach one allocation by handle (the boundary's success path for a
/// returned fresh list). `None` when the handle is not registry-owned
/// (a param passthrough).
fn alloc_registry_detach(handle: i64) -> Option<Vec<i64>> {
    ALLOC_REGISTRY.with(|r| {
        let mut reg = r.borrow_mut();
        let pos = reg.iter().position(|&p| p as i64 == handle)?;
        let ptr = reg.swap_remove(pos);
        Some(*unsafe { Box::from_raw(ptr) })
    })
}

/// Allocate a fresh Int list and plant its pin triple.
///
/// # Safety
/// `frame` slots must be this chain's live frame.
pub unsafe extern "C" fn logos_rt_alloc_list_i64(
    frame: *mut i64,
    vec_slot: i64,
    ptr_slot: i64,
    len_slot: i64,
) {
    let boxed: Box<Vec<i64>> = Box::new(Vec::new());
    let raw = Box::into_raw(boxed);
    ALLOC_REGISTRY.with(|r| r.borrow_mut().push(raw));
    *frame.add(vec_slot as usize) = raw as i64;
    *frame.add(ptr_slot as usize) = (*raw).as_mut_ptr() as i64;
    *frame.add(len_slot as usize) = 0;
}

/// Plant the pin triple for a list handle RETURNED by a self-call (the
/// caller's view of the callee's result list).
///
/// # Safety
/// `frame[handle_slot]` must be a live `*mut Vec<i64>` (registry-owned or
/// a pinned param's vec).
pub unsafe extern "C" fn logos_rt_list_triple(
    frame: *mut i64,
    handle_slot: i64,
    vec_slot: i64,
    ptr_slot: i64,
    len_slot: i64,
) {
    let raw = *frame.add(handle_slot as usize) as *mut Vec<i64>;
    *frame.add(vec_slot as usize) = raw as i64;
    *frame.add(ptr_slot as usize) = (*raw).as_mut_ptr() as i64;
    *frame.add(len_slot as usize) = (*raw).len() as i64;
}

pub unsafe extern "C" fn logos_rt_map_get_ii(
    map: i64,
    key: i64,
    frame: *mut i64,
    dst_slot: i64,
) -> i64 {
    use logicaffeine_compile::interpreter::{MapStorage, RuntimeValue};
    let storage = &*(map as *const MapStorage);
    match storage.get(&RuntimeValue::Int(key)) {
        Some(RuntimeValue::Int(v)) => {
            *frame.add(dst_slot as usize) = *v;
            1
        }
        // Miss or non-Int value: the side exit replays on bytecode, where
        // the kernel raises its exact error or hands back the boxed value.
        _ => 0,
    }
}

/// Map insert on the kernel's own storage — identical hashing, identical
/// iteration order.
///
/// # Safety
/// `map` must be a live `*mut MapStorage` pinned for the run.
pub unsafe extern "C" fn logos_rt_map_set_ii(map: i64, key: i64, value: i64) {
    use logicaffeine_compile::interpreter::{MapStorage, RuntimeValue};
    let storage = &mut *(map as *mut MapStorage);
    storage.insert(RuntimeValue::Int(key), RuntimeValue::Int(value));
}

/// Map membership for an Int key.
///
/// # Safety
/// As for [`logos_rt_map_set_ii`].
pub unsafe extern "C" fn logos_rt_map_has_i(map: i64, key: i64) -> i64 {
    use logicaffeine_compile::interpreter::{MapStorage, RuntimeValue};
    let storage = &*(map as *const MapStorage);
    storage.contains_key(&RuntimeValue::Int(key)) as i64
}

/// Push helper for pinned Int lists.
///
/// # Safety
/// `frame` slots must be the caller's live pins.
pub unsafe extern "C" fn logos_rt_push_i64(
    frame: *mut i64,
    vec_slot: i64,
    ptr_slot: i64,
    len_slot: i64,
    value: i64,
) {
    let vec = *frame.add(vec_slot as usize) as *mut Vec<i64>;
    (*vec).push(value);
    *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
    *frame.add(len_slot as usize) = (*vec).len() as i64;
}

/// See [`logos_rt_push_i64`] — f64 value travels as bits.
///
/// # Safety
/// See [`logos_rt_push_i64`].
pub unsafe extern "C" fn logos_rt_push_f64(
    frame: *mut i64,
    vec_slot: i64,
    ptr_slot: i64,
    len_slot: i64,
    value: i64,
) {
    let vec = *frame.add(vec_slot as usize) as *mut Vec<f64>;
    (*vec).push(f64::from_bits(value as u64));
    *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
    *frame.add(len_slot as usize) = (*vec).len() as i64;
}

/// See [`logos_rt_push_i64`] — nonzero stores `true`.
///
/// # Safety
/// See [`logos_rt_push_i64`].
pub unsafe extern "C" fn logos_rt_push_bool(
    frame: *mut i64,
    vec_slot: i64,
    ptr_slot: i64,
    len_slot: i64,
    value: i64,
) {
    let vec = *frame.add(vec_slot as usize) as *mut Vec<bool>;
    (*vec).push(value != 0);
    *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
    *frame.add(len_slot as usize) = (*vec).len() as i64;
}

/// In-place clear of a pinned list: `vec.clear()` (truncate to empty, KEEP the
/// buffer/capacity) and refresh the pinned ptr/len slots (ptr unchanged — clear
/// never reallocs — but refreshed for parity with push). Lowers an in-region
/// `NewEmptyList` on a pinned array: an in-place mutation (like SetIndex), so the
/// PRECISE region resumes over it soundly. See [`logos_rt_push_i64`] for the
/// frame-slot convention.
///
/// # Safety
/// `frame` slots must be the caller's live pins; `vec_slot` a live `*mut Vec<i64>`.
pub unsafe extern "C" fn logos_rt_clear_i64(
    frame: *mut i64,
    vec_slot: i64,
    ptr_slot: i64,
    len_slot: i64,
) {
    let vec = *frame.add(vec_slot as usize) as *mut Vec<i64>;
    (*vec).clear();
    *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
    *frame.add(len_slot as usize) = 0;
}

/// See [`logos_rt_clear_i64`] — `Vec<f64>` buffer.
///
/// # Safety
/// See [`logos_rt_clear_i64`].
pub unsafe extern "C" fn logos_rt_clear_f64(
    frame: *mut i64,
    vec_slot: i64,
    ptr_slot: i64,
    len_slot: i64,
) {
    let vec = *frame.add(vec_slot as usize) as *mut Vec<f64>;
    (*vec).clear();
    *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
    *frame.add(len_slot as usize) = 0;
}

/// See [`logos_rt_clear_i64`] — `Vec<bool>` buffer.
///
/// # Safety
/// See [`logos_rt_clear_i64`].
pub unsafe extern "C" fn logos_rt_clear_bool(
    frame: *mut i64,
    vec_slot: i64,
    ptr_slot: i64,
    len_slot: i64,
) {
    let vec = *frame.add(vec_slot as usize) as *mut Vec<bool>;
    (*vec).clear();
    *frame.add(ptr_slot as usize) = (*vec).as_mut_ptr() as i64;
    *frame.add(len_slot as usize) = 0;
}

/// MUTABLE-Text append helper for the `MicroOp::StrAppend` regalloc lowering of a
/// `Set text to text + <s>` build loop. `frame[handle_slot]` holds a `*mut Value`
/// pointing AT THE VM REGISTER CELL holding the accumulator (planted at region
/// entry). `src`/`src_len` carry the appended operand:
/// * `src_len < 0` — BYTE form: `src` is a single ASCII byte VALUE (0..=127); the
///   appended text is that one character.
/// * `src_len >= 0` — CONST form: `src` is a `*const u8` to `src_len` bytes (a
///   baked ASCII literal).
///
/// The grow reproduces the VM's [`Vm::add_assign`] semantics EXACTLY so the
/// tiered run is BIT-IDENTICAL to the tree-walker for every alias case:
/// * if the accumulator is a sole-owned `Rc<String>` (`Rc::get_mut` succeeds), it
///   is extended IN PLACE (`String::push_str`);
/// * otherwise (the `Rc` is aliased — a captured `Set saved to text`) it is grown
///   COPY-ON-WRITE through the kernel's `arith::add` (`Rc::new(format!("{a}{b}"))`,
///   the SAME path `add_assign` falls through to), and the FRESH `Rc` replaces the
///   accumulator's cell — the alias keeps its own (unchanged) `Rc`;
/// * a non-`Text` accumulator (never planted — the entry pin declines a non-Text)
///   still falls to the kernel `add`, matching the bytecode.
///
/// # Safety
/// `frame[handle_slot]` must be the live `*mut Value` planted by the region-entry
/// pin loop; for the CONST form `src` must point to `src_len` readable ASCII bytes
/// valid for the call. The single accumulator cell is exclusively reachable for
/// the native run (no other code touches that register), so the `&mut Value` is
/// non-aliasing.
pub unsafe extern "C" fn logos_rt_str_append(
    frame: *mut i64,
    handle_slot: i64,
    src: i64,
    src_len: i64,
) {
    use logicaffeine_compile::interpreter::RuntimeValue;
    use logicaffeine_compile::vm::Value;
    use std::rc::Rc;

    let cell = *frame.add(handle_slot as usize) as *mut Value;
    let acc = &mut *cell;

    // The appended bytes as a &str (ASCII — both the byte form and the baked
    // const are guaranteed ASCII by the region's `TextBytes`/`TextByte` gate).
    let byte_buf: [u8; 1];
    let bytes: &[u8] = if src_len < 0 {
        byte_buf = [src as u8];
        &byte_buf
    } else {
        std::slice::from_raw_parts(src as *const u8, src_len as usize)
    };
    let appended: &str = std::str::from_utf8_unchecked(bytes);

    // Mirror `add_assign`: sole-owned Text grows in place; everything else (a
    // shared Rc, or a non-Text) takes the kernel `add` (COW for Text+Text).
    if matches!(acc.as_runtime_ref(), Some(RuntimeValue::Text(_))) {
        if let RuntimeValue::Text(rc) = acc.as_runtime_mut() {
            if let Some(s) = Rc::get_mut(rc) {
                s.push_str(appended);
                return;
            }
        }
    }
    // COW / non-Text fallthrough — bit-identical to `add_assign`'s
    // `self.reg(dst).add(self.reg(src))` for `(Text, Text)`.
    let rhs = Value::text(appended.to_string());
    match acc.add(&rhs) {
        Ok(v) => *acc = v,
        // `add` over two Texts never errors; an Err would mean a non-Text
        // accumulator the pin should have declined. Leave the cell untouched
        // (the region's differential gate would catch any divergence).
        Err(_) => {}
    }
}

/// The deopt sentinel `logos_rt_memmem` returns when the recognized search nest
/// would, on bytecode, take an access the helper cannot reproduce in-bounds — a
/// checked needle index past the needle buffer (`needleLen > len(needle)`). The
/// caller's stencil routes this to a side exit so the VM replays from the region
/// head on bytecode and raises the EXACT same error at the EXACT same point.
/// `i64::MIN` is never a legitimate count (a count is in `[0, haystack_len]`).
pub const LOGOS_MEMMEM_DEOPT: i64 = i64::MIN;

/// Count OVERLAPPING occurrences of an ASCII needle in an ASCII haystack,
/// reproducing the LOGOS naive-search nest BIT-IDENTICALLY (the string_search
/// benchmark idiom). The recognizer in `adapt_region` collapses the per-byte
/// nested-loop region to a single call to this helper.
///
/// Parameters mirror the nest's live values at region entry:
/// - `haystack_ptr` / `haystack_len`: the pinned `text` byte buffer (char index
///   == byte index for the ASCII pin) and `length of text`.
/// - `needle_ptr` / `needle_buf_len`: the pinned `needle` byte buffer and its
///   actual length (the pinned len slot).
/// - `needle_len`: the PROGRAM's `needleLen` value (the inner loop bound), which
///   may differ from `needle_buf_len`.
/// - `start`: the 1-based outer index `i` at region entry.
///
/// Semantics, matching the nest `while i <= textLen - needleLen + 1 { … }`:
/// - the outer bound is `haystack_len - needle_len + 1` (recomputed here from
///   the SAME inputs the nest used, so it agrees exactly);
/// - a position `i` (1-based) counts iff `haystack[i-1 + j] == needle[j]` for
///   all `j` in `[0, needle_len)` (0-based byte compare);
/// - an empty needle (`needle_len == 0`) matches at EVERY outer position;
/// - a needle longer than the haystack (bound `< start`) yields zero;
/// - if a needle index `j` would read past `needle_buf_len` (i.e.
///   `needle_len > needle_buf_len`, the nest's CHECKED `Index` on `needle`),
///   the helper returns [`LOGOS_MEMMEM_DEOPT`] WITHOUT scanning — the caller
///   side-exits and bytecode reproduces the exact `index out of bounds` error.
///   (Text accesses are guaranteed in-bounds by the bound, exactly as the nest's
///   `IndexUnchecked` on `text` relies on.)
///
/// Counting is via a first-byte scan + window verify (`memchr`-style) which is
/// bit-identical to the nested loop: it visits the same positions and applies
/// the same byte equality, just without per-op bytecode dispatch.
///
/// # Safety
/// `haystack_ptr` must point to `haystack_len` readable bytes and `needle_ptr`
/// to `needle_buf_len` readable bytes, both pinned (borrowed) for the call.
pub unsafe extern "C" fn logos_rt_memmem(
    haystack_ptr: i64,
    haystack_len: i64,
    needle_ptr: i64,
    needle_buf_len: i64,
    needle_len: i64,
    start: i64,
    _bound_unused: i64,
) -> i64 {
    // A checked needle access past its buffer is the nest's recoverable error
    // path: bail to bytecode rather than read OOB or guess the count.
    if needle_len > needle_buf_len {
        return LOGOS_MEMMEM_DEOPT;
    }
    // The nest recomputes `bound = textLen - needleLen + 1` every outer
    // iteration from loop-invariant inputs; recompute it here from the same
    // values so the range agrees exactly (the passed `_bound_unused` is the
    // VM's live copy, kept in the ABI for parity but never trusted).
    let bound = haystack_len - needle_len + 1;
    if start > bound {
        return 0;
    }
    let haystack = std::slice::from_raw_parts(haystack_ptr as *const u8, haystack_len as usize);
    let needle = std::slice::from_raw_parts(needle_ptr as *const u8, needle_len as usize);

    // Empty needle: matches at every outer position in `[start, bound]`.
    if needle_len == 0 {
        return bound - start + 1;
    }

    // 1-based positions `[start, bound]` correspond to 0-based haystack offsets
    // `[start - 1, bound - 1]`. Each offset `p` matches iff
    // `haystack[p .. p + needle_len] == needle`.
    let first = needle[0];
    let nlen = needle_len as usize;
    let lo = (start - 1) as usize;
    // `bound - 1` is the last 0-based start offset; the window end is
    // `bound - 1 + nlen == haystack_len`, in bounds by construction.
    let hi = (bound - 1) as usize; // inclusive last start offset
    let mut count = 0i64;
    let mut p = lo;
    while p <= hi {
        // First-byte scan to the next candidate (memchr semantics).
        match memchr(first, &haystack[p..=hi]) {
            Some(off) => {
                let cand = p + off;
                if haystack[cand..cand + nlen] == *needle {
                    count += 1;
                }
                p = cand + 1;
            }
            None => break,
        }
    }
    count
}

/// First-byte scan helper (a tiny `memchr`): the offset of the first `needle`
/// byte in `hay`, or `None`. A plain byte loop — bit-identical to the nest's
/// position-by-position visit; a SIMD (SSE2/AVX2) replacement can drop in here
/// once the scalar path is proven against the differential gate.
#[inline]
fn memchr(needle: u8, hay: &[u8]) -> Option<usize> {
    hay.iter().position(|&b| b == needle)
}

/// Frame-driven entry the [`MicroOp::MemMem`] stencil calls: reads the search
/// nest's live values out of the JIT frame, runs [`logos_rt_memmem`], and on
/// success ADDS the count into `count_slot` and advances `i_slot` to the loop's
/// exit value (`bound + 1`). Returns `1` on success and `0` on the deopt
/// sentinel — the stencil routes `0` to its side-exit continuation, untouched.
///
/// # Safety
/// `frame` must be the running chain's live frame; the named slots must hold the
/// pinned haystack/needle pointer+length pairs, the `needleLen` value, the
/// 1-based `i`, and the `count` accumulator.
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn logos_rt_memmem_frame(
    frame: *mut i64,
    h_ptr_slot: i64,
    h_len_slot: i64,
    n_ptr_slot: i64,
    n_len_slot: i64,
    needle_len_slot: i64,
    i_slot: i64,
    count_slot: i64,
) -> i64 {
    let h_ptr = *frame.add(h_ptr_slot as usize);
    let h_len = *frame.add(h_len_slot as usize);
    let n_ptr = *frame.add(n_ptr_slot as usize);
    let n_buf_len = *frame.add(n_len_slot as usize);
    let needle_len = *frame.add(needle_len_slot as usize);
    let start = *frame.add(i_slot as usize);
    let result = logos_rt_memmem(h_ptr, h_len, n_ptr, n_buf_len, needle_len, start, 0);
    if result == LOGOS_MEMMEM_DEOPT {
        return 0;
    }
    *frame.add(count_slot as usize) += result;
    // Advance `i` to the nest's natural exit value: the first index for which
    // `i > bound` (`bound = h_len - needle_len + 1`). If the loop never ran
    // (`start > bound`), `i` keeps its entry value — exactly the bytecode's
    // `while i <= bound { … i = i + 1 }` post-state.
    let bound = h_len - needle_len + 1;
    let exit_i = core::cmp::max(start, bound + 1);
    *frame.add(i_slot as usize) = exit_i;
    1
}

/// Register kinds for the adapter's sound dataflow.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Unknown,
    Int,
    Bool,
    Float,
    /// A pinned unboxed Int list (the register itself never rides a slot —
    /// its buffer pointer/length live in dedicated pin slots).
    IntList,
    /// A pinned unboxed Float list.
    FloatList,
    /// A pinned unboxed Bool list.
    BoolList,
    /// A pinned MAP (boxed kernel storage behind a pointer pin; int fast
    /// lane verified per helper call).
    IntMap,
    /// A pinned ASCII `Text` carried AS BYTES (char index == byte index, char
    /// count == byte length): `item i of text` is a 1-byte pinned load and the
    /// extracted char rides the [`Kind::TextByte`] lane.
    TextBytes,
    /// A single ASCII character extracted from a [`Kind::TextBytes`] (or a
    /// 1-char ASCII text constant): the integer byte value 0..=127. Equality
    /// between two `TextByte`s is an exact integer compare — which matches the
    /// tree-walker's `Text == Text` string compare for single ASCII chars.
    TextByte,
    /// A pinned MUTABLE-Text accumulator (the `dst` of a `Set text to text + …`
    /// build loop): it rides the [`PinElem::TextMut`] channel (a `*mut Value`
    /// handle), not a scalar frame slot. The only op that consumes it is the
    /// append (`AddAssign`), lowered to [`MicroOp::StrAppend`]; any other use
    /// (a `Move`, an `Index`, …) is rejected so the region bails — keeping the
    /// accumulator's growth confined to the proven-sound append path.
    TextMut,
    /// A MULTI-character ASCII text CONSTANT (`text + "XXXXX"`): a baked byte
    /// slice. Valid ONLY as the source of a `StrAppend`; every other use gate
    /// rejects it (so the region bails).
    TextConst,
    Mixed,
}

impl Kind {
    fn slot_kind(self) -> Option<SlotKind> {
        match self {
            Kind::Int => Some(SlotKind::Int),
            Kind::Bool => Some(SlotKind::Bool),
            Kind::Float => Some(SlotKind::Float),
            _ => None,
        }
    }
}

/// The single ASCII byte of a 1-character ASCII text constant, or `None` if the
/// constant is not exactly one ASCII char. A `Kind::TextByte` comparison loads
/// one byte from the pinned text, so a constant on the other side must be that
/// same single byte; the tree-walker compares the two 1-char Texts as strings,
/// which agrees with the byte compare exactly for single ASCII chars.
fn text_const_byte(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    (b.len() == 1 && b[0] < 128).then(|| b[0])
}

/// Recover the constant ASCII byte slice of a `StrAppend`'s `TextConst` source
/// (`text + "XXXXX"`) and bake it into a `'static` allocation so the regalloc
/// stencil can hold a stable `(ptr, len)`. The source register `src` is loaded by
/// a `LoadConst` in the same region; scan BACKWARD from the append for the nearest
/// def of `src` and require it to be a `LoadConst` of a `Constant::Text`. Returns
/// `None` (declining the region) if the source is not a direct constant load — a
/// conservative bail to bytecode. The leak is one-time per compiled region (there
/// are a handful of such constants) and the bytes outlive the chain.
fn const_text_slice(
    ops: &[Op],
    i: usize,
    base_pc: usize,
    src: u16,
    constants: &[Constant],
) -> Option<(i64, i64)> {
    let _ = base_pc;
    let mut j = i;
    while j > 0 {
        j -= 1;
        let (_, defs) = region_use_def(&ops[j])?;
        if defs.contains(&src) {
            if let Op::LoadConst { idx, .. } = ops[j] {
                if let Some(Constant::Text(s)) = constants.get(idx as usize) {
                    if s.is_ascii() && !s.is_empty() {
                        let leaked: &'static [u8] = s.clone().into_bytes().leak();
                        return Some((leaked.as_ptr() as i64, leaked.len() as i64));
                    }
                }
            }
            // The nearest def of `src` is not a constant Text load — bail.
            return None;
        }
    }
    None
}

fn observed_kind(o: ObservedKind) -> Kind {
    match o {
        ObservedKind::Int => Kind::Int,
        ObservedKind::Bool => Kind::Bool,
        ObservedKind::Float => Kind::Float,
        ObservedKind::IntList => Kind::IntList,
        ObservedKind::FloatList => Kind::FloatList,
        ObservedKind::BoolList => Kind::BoolList,
        ObservedKind::Map => Kind::IntMap,
        ObservedKind::TextBytes => Kind::TextBytes,
        ObservedKind::Other => Kind::Mixed,
    }
}

/// Per-pinned-register slot assignments for translation.
#[derive(Clone, Copy)]
struct PinSlots {
    vec_slot: u16,
    ptr_slot: u16,
    len_slot: u16,
    elem: PinElem,
}

/// Everything a SELF-call lowering needs baked into its stencil.
struct CallCtx<'t> {
    table_addr: i64,
    depth_addr: i64,
    status_addr: i64,
    limit_slot: u16,
    depth_limit: i64,
    self_fi: u16,
    /// The program's function table — non-self calls bake the CALLEE's
    /// slot address for table dispatch.
    table: &'t FnTable,
}

/// The result kind of an arithmetic op over two operand kinds, mirroring the
/// kernel's promotion: Int×Int→Int, any Float→Float. None = not arithmetic
/// (the use gate bails).
fn arith_result(l: Kind, r: Kind) -> Option<Kind> {
    match (l, r) {
        (Kind::Int, Kind::Int) => Some(Kind::Int),
        (Kind::Float, Kind::Float) | (Kind::Int, Kind::Float) | (Kind::Float, Kind::Int) => {
            Some(Kind::Float)
        }
        _ => None,
    }
}

fn join(a: Kind, b: Kind) -> Kind {
    match (a, b) {
        (Kind::Unknown, x) | (x, Kind::Unknown) => x,
        (x, y) if x == y => x,
        _ => Kind::Mixed,
    }
}

/// Forward, flow-SENSITIVE kind dataflow over a bytecode slice (a function
/// body or a loop region). Each reachable op gets the kind vector holding
/// BEFORE it executes; merge points join per slot (`Unknown ⊔ k = k`,
/// `k ⊔ k = k`, else `Mixed`). Constants carry their pool type (Int, Float,
/// Bool); arithmetic promotes like the kernel (any Float operand → Float).
/// `JumpIfInt` — the compiler's eager-vs-short-circuit dispatch for
/// `and`/`or` — resolves STATICALLY: a proven-Int condition takes only the
/// jump edge, a proven-Bool only the fall-through; anything else bails.
/// Jump targets outside the slice are exits. Returns per-op kind-in vectors
/// (None = unreachable), or None when any reachable use violates the gates.
fn kind_flow(
    ops: &[Op],
    base_pc: usize,
    constants: &[Constant],
    entry: Vec<Kind>,
    calls: Option<&dyn Fn(u16) -> Option<(Kind, Vec<Kind>)>>,
) -> Option<Vec<Option<Vec<Kind>>>> {
    let n = ops.len();
    let in_slice = |t: usize| t >= base_pc && t < base_pc + n;
    let const_kind = |idx: u32| -> Kind {
        match constants.get(idx as usize) {
            Some(Constant::Int(_)) => Kind::Int,
            Some(Constant::Float(_)) => Kind::Float,
            Some(Constant::Bool(_)) => Kind::Bool,
            // A 1-char ASCII text/char literal compares against an extracted
            // `item i of text` as an integer byte (the strings benchmark's
            // `item i of result equals " "`). A multi-char or non-ASCII text is
            // not a single byte — it stays Mixed and bails.
            Some(Constant::Char(c)) if c.is_ascii() => Kind::TextByte,
            Some(Constant::Text(s)) if text_const_byte(s).is_some() => Kind::TextByte,
            // A MULTI-char ASCII text constant is valid only as a `StrAppend`
            // source (`text + "XXXXX"`); every other gate rejects `TextConst`.
            Some(Constant::Text(s)) if !s.is_empty() && s.is_ascii() => Kind::TextConst,
            _ => Kind::Mixed,
        }
    };
    let mut kin: Vec<Option<Vec<Kind>>> = vec![None; n];
    kin[0] = Some(entry);
    let mut work: Vec<usize> = vec![0];
    while let Some(i) = work.pop() {
        let Some(cur) = kin[i].clone() else { continue };
        let mut out = cur;
        match ops[i] {
            Op::LoadConst { dst, idx } => out[dst as usize] = const_kind(idx),
            Op::Add { dst, lhs, rhs }
            | Op::Sub { dst, lhs, rhs }
            | Op::Mul { dst, lhs, rhs }
            | Op::Div { dst, lhs, rhs } => {
                out[dst as usize] =
                    arith_result(out[lhs as usize], out[rhs as usize]).unwrap_or(Kind::Mixed)
            }
            Op::AddAssign { dst, src } => {
                out[dst as usize] = if out[dst as usize] == Kind::TextMut {
                    // A pinned mutable-Text accumulator stays the accumulator —
                    // it grows in place / COW through `StrAppend`.
                    Kind::TextMut
                } else {
                    arith_result(out[dst as usize], out[src as usize]).unwrap_or(Kind::Mixed)
                }
            }
            Op::Mod { dst, .. }
            | Op::DivPow2 { dst, .. }
            | Op::MagicDivU { dst, .. }
            | Op::BitXor { dst, .. }
            | Op::Shl { dst, .. }
            | Op::Shr { dst, .. } => out[dst as usize] = Kind::Int,
            Op::AndEager { dst, lhs, rhs } | Op::OrEager { dst, lhs, rhs } => {
                out[dst as usize] = match (out[lhs as usize], out[rhs as usize]) {
                    (Kind::Int, Kind::Int) => Kind::Int,
                    (Kind::Bool, Kind::Bool) => Kind::Bool,
                    _ => Kind::Mixed,
                }
            }
            Op::Not { dst, src } => {
                out[dst as usize] = match out[src as usize] {
                    Kind::Int => Kind::Int,
                    Kind::Bool => Kind::Bool,
                    _ => Kind::Mixed,
                }
            }
            Op::Lt { dst, .. } | Op::Gt { dst, .. } | Op::LtEq { dst, .. }
            | Op::GtEq { dst, .. } | Op::Eq { dst, .. } | Op::NotEq { dst, .. } => {
                out[dst as usize] = Kind::Bool
            }
            Op::Move { dst, src } => out[dst as usize] = out[src as usize],
            Op::Index { dst, collection, .. }
            | Op::IndexUnchecked { dst, collection, .. } => {
                out[dst as usize] = match out[collection as usize] {
                    Kind::IntList => Kind::Int,
                    Kind::FloatList => Kind::Float,
                    Kind::BoolList => Kind::Bool,
                    // Map int fast lane: a non-Int value is a helper miss
                    // (side exit), so the in-region kind is Int.
                    Kind::IntMap => Kind::Int,
                    // `item i of text` on an ASCII text: a single ASCII byte.
                    Kind::TextBytes => Kind::TextByte,
                    _ => Kind::Mixed,
                }
            }
            Op::Length { dst, .. } => out[dst as usize] = Kind::Int,
            Op::Contains { dst, .. } => out[dst as usize] = Kind::Bool,
            // Fresh allocations ride the Int lane (mode B sites).
            Op::NewEmptyList { dst } => out[dst as usize] = Kind::IntList,
            Op::Call { dst, func, .. } => {
                let Some((ret, _)) = calls.and_then(|c| c(func)) else { return None };
                out[dst as usize] = ret;
            }
            Op::CallBuiltin {
                dst,
                builtin: logicaffeine_compile::semantics::builtins::BuiltinId::Sqrt,
                ..
            } => out[dst as usize] = Kind::Float,
            _ => {}
        }
        let mut succs: Vec<usize> = Vec::with_capacity(2);
        match ops[i] {
            Op::Jump { target } => {
                if in_slice(target) {
                    succs.push(target - base_pc);
                }
            }
            Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
                if i + 1 < n {
                    succs.push(i + 1);
                }
                if in_slice(target) {
                    succs.push(target - base_pc);
                }
            }
            // Statically resolved: only the actually-takeable edge flows.
            Op::JumpIfInt { cond, target } => match out[cond as usize] {
                Kind::Int => {
                    if in_slice(target) {
                        succs.push(target - base_pc);
                    }
                }
                Kind::Bool | Kind::Float => {
                    if i + 1 < n {
                        succs.push(i + 1);
                    }
                }
                _ => return None,
            },
            Op::Return { .. } | Op::ReturnNothing => {}
            _ => {
                if i + 1 < n {
                    succs.push(i + 1);
                }
            }
        }
        for s in succs {
            match &mut kin[s] {
                slot @ None => {
                    *slot = Some(out.clone());
                    work.push(s);
                }
                Some(prev) => {
                    let mut changed = false;
                    for (a, b) in prev.iter_mut().zip(&out) {
                        let j = join(*a, *b);
                        if j != *a {
                            *a = j;
                            changed = true;
                        }
                    }
                    if changed {
                        work.push(s);
                    }
                }
            }
        }
    }
    // Gates on every reachable use, at that use's program point.
    for (i, op) in ops.iter().enumerate() {
        let Some(k) = &kin[i] else { continue };
        let int_at = |r: u16| k[r as usize] == Kind::Int;
        let num_at = |r: u16| matches!(k[r as usize], Kind::Int | Kind::Float);
        match *op {
            // Arithmetic and ordered comparisons: numeric operands (mixed
            // Int/Float promotes via an inserted conversion).
            Op::Add { lhs, rhs, .. } | Op::Sub { lhs, rhs, .. } | Op::Mul { lhs, rhs, .. }
            | Op::Div { lhs, rhs, .. }
            | Op::Lt { lhs, rhs, .. } | Op::Gt { lhs, rhs, .. } | Op::LtEq { lhs, rhs, .. }
            | Op::GtEq { lhs, rhs, .. } => {
                if !num_at(lhs) || !num_at(rhs) {
                    return None;
                }
            }
            Op::AddAssign { dst, src } => {
                if k[dst as usize] == Kind::TextMut {
                    // A pinned mutable-Text append: the source must be an
                    // appendable ASCII text — a 1-char runtime byte (`TextByte`)
                    // or a multi-char constant (`TextConst`). `StrAppend` lowers it.
                    if !matches!(k[src as usize], Kind::TextByte | Kind::TextConst) {
                        return None;
                    }
                } else if !num_at(dst) || !num_at(src) {
                    return None;
                }
            }
            // Equality: numeric pairs (promoting), Bool×Bool, or TextByte×
            // TextByte (two extracted ASCII chars, or an extracted char vs a
            // 1-char ASCII literal — both sides are integer bytes, the compare
            // is exact, matching the tree-walker's single-char Text compare).
            Op::Eq { lhs, rhs, .. } | Op::NotEq { lhs, rhs, .. } => {
                let bools = k[lhs as usize] == Kind::Bool && k[rhs as usize] == Kind::Bool;
                let bytes = k[lhs as usize] == Kind::TextByte && k[rhs as usize] == Kind::TextByte;
                if !bools && !bytes && (!num_at(lhs) || !num_at(rhs)) {
                    return None;
                }
            }
            // Int-only ops.
            Op::Mod { lhs, rhs, .. } | Op::BitXor { lhs, rhs, .. }
            | Op::Shl { lhs, rhs, .. } | Op::Shr { lhs, rhs, .. } => {
                if !int_at(lhs) || !int_at(rhs) {
                    return None;
                }
            }
            // Eager and/or: both-Int (bitwise) or both-Bool (logical).
            Op::AndEager { lhs, rhs, .. } | Op::OrEager { lhs, rhs, .. } => {
                let ints = int_at(lhs) && int_at(rhs);
                let bools = k[lhs as usize] == Kind::Bool && k[rhs as usize] == Kind::Bool;
                if !ints && !bools {
                    return None;
                }
            }
            Op::Not { src, .. } => {
                if !matches!(k[src as usize], Kind::Int | Kind::Bool) {
                    return None;
                }
            }
            Op::Move { src, .. } => {
                // List buffers never ride frame slots (they are pinned
                // separately) — but a list-kind Move is legal as STAGING:
                // the slot value is a placeholder and pin identity travels
                // statically. Any real USE of an unpinned destination still
                // fails closed at translate (pins lookup).
                if !matches!(
                    k[src as usize],
                    Kind::Int
                        | Kind::Bool
                        | Kind::Float
                        | Kind::IntList
                        | Kind::FloatList
                        | Kind::BoolList
                        // A 1-char ASCII byte (`text + ch`'s `ch`) rides a plain
                        // frame slot as its integer byte value — a raw-copy Move.
                        | Kind::TextByte
                ) {
                    return None;
                }
            }
            Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => {
                if k[cond as usize] != Kind::Bool {
                    return None;
                }
            }
            Op::Index { collection, index, .. }
            | Op::IndexUnchecked { collection, index, .. } => {
                if !matches!(
                    k[collection as usize],
                    Kind::IntList | Kind::FloatList | Kind::BoolList | Kind::IntMap | Kind::TextBytes
                ) || !int_at(index)
                {
                    return None;
                }
            }
            Op::SetIndex { collection, index, value }
            | Op::SetIndexUnchecked { collection, index, value } => {
                let elem_ok = match k[collection as usize] {
                    Kind::IntList => k[value as usize] == Kind::Int,
                    Kind::FloatList => k[value as usize] == Kind::Float,
                    Kind::BoolList => k[value as usize] == Kind::Bool,
                    Kind::IntMap => k[value as usize] == Kind::Int,
                    _ => false,
                };
                if !elem_ok || !int_at(index) {
                    return None;
                }
            }
            Op::Contains { collection, value, .. } => {
                if k[collection as usize] != Kind::IntMap || !int_at(value) {
                    return None;
                }
            }
            Op::Length { collection, .. } => {
                if !matches!(
                    k[collection as usize],
                    Kind::IntList | Kind::FloatList | Kind::BoolList | Kind::TextBytes
                ) {
                    return None;
                }
            }
            // SELF-calls only (the callee's entry contract is its own: Int
            // params, known return kind); every argument must be Int at the
            // call point — exactly the per-call guard the bytecode boundary
            // enforces.
            Op::Call { func, args_start, arg_count, .. } => {
                let Some((_, params)) = calls.and_then(|c| c(func)) else { return None };
                if arg_count as usize != params.len() {
                    return None;
                }
                for (j, a) in (args_start..args_start + arg_count).enumerate() {
                    if k[a as usize] != params[j] {
                        return None;
                    }
                }
            }
            Op::CallBuiltin {
                builtin: logicaffeine_compile::semantics::builtins::BuiltinId::Sqrt,
                args_start,
                arg_count,
                ..
            } => {
                if arg_count != 1
                    || !matches!(k[args_start as usize], Kind::Int | Kind::Float)
                {
                    return None;
                }
            }
            Op::ListPush { list, value } => {
                let elem_ok = match k[list as usize] {
                    Kind::IntList => k[value as usize] == Kind::Int,
                    Kind::FloatList => k[value as usize] == Kind::Float,
                    Kind::BoolList => k[value as usize] == Kind::Bool,
                    _ => false,
                };
                if !elem_ok {
                    return None;
                }
            }
            Op::Return { src } => {
                // Regions return scalars (re-boxed by SlotKind) or pinned
                // lists (by register); the function adapter's own ret-kind
                // check still restricts FUNCTION returns to Int/Bool.
                if !matches!(
                    k[src as usize],
                    Kind::Int
                        | Kind::Bool
                        | Kind::Float
                        | Kind::IntList
                        | Kind::FloatList
                        | Kind::BoolList
                ) {
                    return None;
                }
            }
            _ => {}
        }
    }
    Some(kin)
}

/// Translate WHOLE-PROGRAM Main bytecode into the J2 micro-op subset.
///
/// Requirements (else `None`):
/// - ops drawn from {LoadConst(Int), Move, Add, Sub, Mul, Lt, Gt, Eq,
///   Jump, JumpIfFalse, JumpIfTrue, Show};
/// - exactly ONE Show, and it is the final op (it becomes Return);
/// - no comparison-produced value is the shown register (bool-taint).
///
/// Returns the micro program and the frame size it needs.
#[allow(clippy::type_complexity)]
pub fn adapt(
    ops: &[Op],
    constants: &[Constant],
    register_count: usize,
) -> Option<(Vec<MicroOp>, usize)> {
    // The shown register must not be a comparison result anywhere.
    let mut tainted: Vec<u16> = Vec::new();
    let mut shows = 0usize;
    for op in ops {
        match *op {
            Op::Lt { dst, .. } | Op::Gt { dst, .. } | Op::LtEq { dst, .. }
            | Op::GtEq { dst, .. } | Op::Eq { dst, .. } | Op::NotEq { dst, .. } => {
                tainted.push(dst)
            }
            Op::Show { .. } => shows += 1,
            _ => {}
        }
    }
    // Terminal shape: `Show; Halt` (or a bare trailing Show). Anything that
    // jumps PAST the Show (to the Halt) would print nothing on the VM while
    // the JIT still returns a value — bail on those.
    let n = ops.len();
    let (show_pc, halt_pc) = match (n.checked_sub(2).map(|i| &ops[i]), ops.last()) {
        (Some(Op::Show { .. }), Some(Op::Halt)) => (n - 2, Some(n - 1)),
        (_, Some(Op::Show { .. })) => (n - 1, None),
        _ => return None,
    };
    if shows != 1 {
        return None;
    }
    for op in ops {
        if let Op::Jump { target } | Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } = *op {
            if target > show_pc {
                return None;
            }
        }
    }
    let _ = halt_pc;

    let frame_size = register_count;

    // Pass 1: translate, recording vm-pc → micro-index. Jump targets hold VM
    // pcs until pass 2 remaps them.
    let mut micro: Vec<MicroOp> = Vec::with_capacity(ops.len());
    let mut pc_to_micro: Vec<usize> = Vec::with_capacity(ops.len());
    // (micro index, vm target) needing remap.
    let mut fixups: Vec<usize> = Vec::new();

    for op in ops {
        pc_to_micro.push(micro.len());
        match *op {
            Op::LoadConst { dst, idx } => match constants.get(idx as usize)? {
                Constant::Int(v) => micro.push(MicroOp::LoadConst { dst, value: *v }),
                _ => return None,
            },
            Op::Move { dst, src } => micro.push(MicroOp::Move { dst, src }),
            Op::Add { dst, lhs, rhs } => micro.push(MicroOp::Add { dst, lhs, rhs }),
            Op::AddAssign { dst, src } => micro.push(MicroOp::Add { dst, lhs: dst, rhs: src }),
            Op::Div { dst, lhs, rhs } => match const_pow2_div(ops, rhs, constants) {
                Some(k) => micro.push(MicroOp::DivPow2 { dst, lhs, k }),
                None => micro.push(MicroOp::Div { dst, lhs, rhs }),
            },
            Op::DivPow2 { dst, lhs, k } => micro.push(MicroOp::DivPow2 { dst, lhs, k: k as u32 }),
            Op::MagicDivU { dst, lhs, magic, more, mul_back } => {
                micro.push(MicroOp::MagicDivU { dst, lhs, magic, more, mul_back })
            }
            Op::Mod { dst, lhs, rhs } => micro.push(MicroOp::Mod { dst, lhs, rhs }),
            Op::Sub { dst, lhs, rhs } => micro.push(MicroOp::Sub { dst, lhs, rhs }),
            Op::Mul { dst, lhs, rhs } => micro.push(MicroOp::Mul { dst, lhs, rhs }),
            Op::Lt { dst, lhs, rhs } => micro.push(MicroOp::Lt { dst, lhs, rhs }),
            Op::Gt { dst, lhs, rhs } => micro.push(MicroOp::Gt { dst, lhs, rhs }),
            Op::Eq { dst, lhs, rhs } => micro.push(MicroOp::Eq { dst, lhs, rhs }),
            Op::LtEq { dst, lhs, rhs } => micro.push(MicroOp::LtEq { dst, lhs, rhs }),
            Op::GtEq { dst, lhs, rhs } => micro.push(MicroOp::GtEq { dst, lhs, rhs }),
            Op::NotEq { dst, lhs, rhs } => micro.push(MicroOp::Neq { dst, lhs, rhs }),
            Op::Jump { target } => {
                fixups.push(micro.len());
                micro.push(MicroOp::Jump { target });
            }
            Op::JumpIfFalse { cond, target } => {
                fixups.push(micro.len());
                micro.push(MicroOp::JumpIfFalse { cond, target });
            }
            Op::JumpIfTrue { cond, target } => {
                fixups.push(micro.len());
                micro.push(MicroOp::JumpIfTrue { cond, target });
            }
            Op::Show { src } => {
                if tainted.contains(&src) {
                    return None;
                }
                micro.push(MicroOp::Return { src });
            }
            // The trailing Halt after the Show: unreachable past Return.
            Op::Halt => {}
            _ => return None,
        }
    }

    // Pass 2: remap jump targets from VM pcs to micro indices.
    for &i in &fixups {
        match &mut micro[i] {
            MicroOp::Jump { target }
            | MicroOp::JumpIfFalse { target, .. }
            | MicroOp::JumpIfTrue { target, .. } => {
                *target = *pc_to_micro.get(*target)?;
            }
            _ => unreachable!(),
        }
    }
    Some((micro, frame_size))
}

/// Translate a FUNCTION body (terminal = Op::Return, absolute pcs rebased by
/// `entry_pc`) into the J2 subset.
///
/// J4 kind dataflow (sound fixpoint): params are Int (the entry guard),
/// comparisons produce Bool, arithmetic requires Int operands (a Bool
/// flowing into `+` errors on the VM — native code must NOT compute it),
/// Move propagates, conflicting writes go Mixed (bail on use). All reachable
/// returns must agree on Int or Bool; the bool flag re-boxes the result.
/// Everything the precise-deopt boundary needs at runtime, built once at
/// adapt time (mode B — functions with list parameters).
struct PreciseInfo {
    register_count: usize,
    pins_start: usize,
    /// Per-register re-box kinds at every potential pause point: each
    /// deoptable op's pc and each self-call's pc.
    kinds_by_pc: std::collections::HashMap<usize, Vec<RegBox>>,
    /// NATIVE-OWNED list registers per pause point: (register, the frame
    /// slot of its pin's vec handle). At materialization the handle either
    /// detaches from the allocation registry (fresh list — re-boxed here)
    /// or matches a boundary pin (param passthrough — rewritten to
    /// ListParam for the VM side).
    owned_by_pc: std::collections::HashMap<usize, Vec<(u16, u16)>>,
    /// Where each boundary pin triple (param order) lands in the frame:
    /// the vec slot of THAT PARAM REGISTER's pin (per-register pins are
    /// not in param order).
    param_pin_scatter: Vec<u16>,
}

/// What adapt_function hands back: micro code, frame size, return shape,
/// per-micro deopt codes (mode B) and the precise-walk tables (mode B).
struct AdaptedFn {
    micro: Vec<MicroOp>,
    frame_size: usize,
    ret: NativeRet,
    deopt_codes: Option<Vec<i64>>,
    precise: Option<PreciseInfo>,
}

/// The precise-deopt tables for a REGION (push+SetIndex worklist). `deopt_codes`
/// tag every micro with its bytecode op's encoded resume pc; `kinds_by_pc` maps
/// each resume pc to the per-register re-box kind used when materializing the
/// frame's scalars back into the VM registers (`None` = keep the VM register's
/// live value — a pinned array, or a read-only/unknown slot).
struct RegionPrecise {
    deopt_codes: Vec<i64>,
    kinds_by_pc: std::collections::HashMap<usize, Vec<Option<SlotKind>>>,
}

/// LIST-REGISTER discovery (shared by the adapter and the frame-size
/// pre-computation): every register that ever holds a list — params,
/// allocation sites, Move aliases, list-returning self-call results.
fn discover_list_regs(
    ops: &[Op],
    list_param_regs: &[u16],
    fn_returns_lists: bool,
    self_fi: u16,
    nregs: usize,
) -> Vec<bool> {
    let mut is_list = vec![false; nregs];
    for &r in list_param_regs {
        is_list[r as usize] = true;
    }
    loop {
        let mut changed = false;
        for op in ops {
            match *op {
                Op::NewEmptyList { dst } => {
                    if !is_list[dst as usize] {
                        is_list[dst as usize] = true;
                        changed = true;
                    }
                }
                Op::Move { dst, src } => {
                    if is_list[src as usize] && !is_list[dst as usize] {
                        is_list[dst as usize] = true;
                        changed = true;
                    }
                }
                Op::Call { dst, func, .. } if func == self_fi && fn_returns_lists => {
                    if !is_list[dst as usize] {
                        is_list[dst as usize] = true;
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }
    is_list
}

fn jt(which: &str) {
    if std::env::var_os("LOGOS_JIT_TRACE").is_some() {
        eprintln!("jit-trace: mode-B bail at {which}");
    }
}

fn adapt_function(
    ops: &[Op],
    entry_pc: usize,
    constants: &[Constant],
    param_count: u16,
    register_count: u16,
    self_fi: u16,
    param_kinds: &[Option<ParamKind>],
    declared_ret: Option<SlotKind>,
    call_ctx_in: &CallCtx,
    callees: &[CalleeSig],
) -> Option<AdaptedFn> {
    // Every jump must stay inside the body slice — a function is self-contained.
    for op in ops {
        if let Op::Jump { target }
        | Op::JumpIfFalse { target, .. }
        | Op::JumpIfTrue { target, .. }
        | Op::JumpIfInt { target, .. } = *op
        {
            if target < entry_pc || target >= entry_pc + ops.len() {
                return None;
            }
        }
    }

    // Flow-sensitive kinds: every parameter starts at its DECLARED kind
    // (the per-call boundary guard checks the matching discriminant before
    // entry, so the seed is sound, not speculative); everything else starts
    // Unknown and the gates inside kind_flow enforce soundness at every
    // reachable use.
    // Return kind: a declared return type pins it in one pass; otherwise
    // the self-recursive Int/Bool inference (assume, verify, retry) stands.
    if param_kinds.len() != param_count as usize {
        return None;
    }
    let mut param_entry: Vec<Kind> = Vec::with_capacity(param_count as usize);
    let mut list_params: Vec<(u16, PinElem)> = Vec::new();
    for (i, pk) in param_kinds.iter().enumerate() {
        param_entry.push(match (*pk)? {
            ParamKind::Scalar(SlotKind::Int) => Kind::Int,
            ParamKind::Scalar(SlotKind::Bool) => Kind::Bool,
            ParamKind::Scalar(SlotKind::Float) => Kind::Float,
            ParamKind::List(elem) => {
                list_params.push((i as u16, elem));
                match elem {
                    PinElem::Int => Kind::IntList,
                    PinElem::Float => Kind::FloatList,
                    PinElem::Bool => Kind::BoolList,
                    // Declared kinds never produce a Map or Text list-param.
                    PinElem::Map | PinElem::TextBytes | PinElem::TextMut => return None,
                }
            }
        });
    }
    // ALLOCATION SITES make a function mode B even without list params.
    let has_sites = ops.iter().any(|op| matches!(op, Op::NewEmptyList { .. }));
    let mode_b = !list_params.is_empty() || has_sites;
    // Mode B leans on the PRECISE call stencil, whose depth limit is baked
    // at the kernel's locked MAX_CALL_DEPTH.
    if mode_b && call_ctx_in.depth_limit != logicaffeine_compile::semantics::MAX_CALL_DEPTH as i64 {
        return None;
    }
    // Does this function RETURN lists? Declared scalar returns say no;
    // otherwise list params or sites make it possible — the fixpoint
    // verifies every return site agrees.
    let fn_returns_lists = declared_ret.is_none() && mode_b;

    // LIST REGISTERS (mode B): one pin triple PER REGISTER that ever
    // holds a list — every list assignment (param entry, allocation,
    // Move alias, self-call result) REFRESHES that register's own triple,
    // so reassignment patterns (quicksort's `result`) cannot conflict.
    // Push eligibility is static: a register every one of whose list
    // sources is an allocation site is definitely registry-owned — a push
    // anywhere else could stale another holder's pinned slots.
    let nregs_pin = register_count as usize + 2;
    let list_param_regs: Vec<u16> = list_params.iter().map(|(r, _)| *r).collect();
    let is_list = discover_list_regs(ops, &list_param_regs, fn_returns_lists, self_fi, nregs_pin);
    // Provenance TAINT, FLOW-SENSITIVE: at each program point, which
    // registers may hold a list some OTHER holder pins (params; call
    // results, which may be param passthroughs). Moves propagate taint;
    // a NewEmptyList CLEANSES its destination (fresh, registry-owned).
    // Pushes are legal only into point-wise clean registers — mergesort's
    // `Set left to mergeSort(left)` taints `left` only AFTER the pushes.
    let mut ret_any_list = false;
    if mode_b {
        let in_body = |t: usize| t >= entry_pc && t < entry_pc + ops.len();
        let mut taint_in: Vec<Option<Vec<bool>>> = vec![None; ops.len()];
        let mut entry_taint = vec![false; nregs_pin];
        for (preg, _) in &list_params {
            entry_taint[*preg as usize] = true;
        }
        taint_in[0] = Some(entry_taint);
        let mut work: Vec<usize> = vec![0];
        while let Some(i) = work.pop() {
            let Some(cur) = taint_in[i].clone() else { continue };
            let mut out = cur;
            match ops[i] {
                Op::NewEmptyList { dst } => out[dst as usize] = false,
                Op::Move { dst, src } => out[dst as usize] = out[src as usize],
                Op::Call { dst, func, .. } if func == self_fi && fn_returns_lists => {
                    out[dst as usize] = true;
                }
                _ => {}
            }
            let mut succs: Vec<usize> = Vec::with_capacity(2);
            match ops[i] {
                Op::Jump { target } => {
                    if in_body(target) {
                        succs.push(target - entry_pc);
                    }
                }
                Op::JumpIfFalse { target, .. }
                | Op::JumpIfTrue { target, .. }
                | Op::JumpIfInt { target, .. } => {
                    if i + 1 < ops.len() {
                        succs.push(i + 1);
                    }
                    if in_body(target) {
                        succs.push(target - entry_pc);
                    }
                }
                Op::Return { .. } | Op::ReturnNothing => {}
                _ => {
                    if i + 1 < ops.len() {
                        succs.push(i + 1);
                    }
                }
            }
            for sidx in succs {
                match &mut taint_in[sidx] {
                    slot @ None => {
                        *slot = Some(out.clone());
                        work.push(sidx);
                    }
                    Some(prev) => {
                        let mut changed = false;
                        for (p, &o) in prev.iter_mut().zip(&out) {
                            if !*p && o {
                                *p = true;
                                changed = true;
                            }
                        }
                        if changed {
                            work.push(sidx);
                        }
                    }
                }
            }
        }
        for (i, op) in ops.iter().enumerate() {
            match *op {
                Op::Return { src } => {
                    if is_list[src as usize] {
                        ret_any_list = true;
                    }
                }
                Op::ListPush { list, .. } => {
                    let clean = taint_in[i]
                        .as_ref()
                        .map(|t| is_list[list as usize] && !t[list as usize])
                        // Unreachable pushes never run.
                        .unwrap_or(true);
                    if !clean {
                        jt("push-outside-site");
                        return None;
                    }
                }
                _ => {}
            }
        }
    }
    let list_regs: Vec<u16> = (0..nregs_pin as u16).filter(|&r| is_list[r as usize]).collect();
    let total_pins = list_regs.len();
    if total_pins > 24 {
        return None;
    }
    let n_params = list_params.len();
    let _ = n_params;
    // pin id = index into list_regs; pin_of mirrors it per register.
    let mut pin_of: Vec<Option<u8>> = vec![None; nregs_pin];
    for (id, &r) in list_regs.iter().enumerate() {
        pin_of[r as usize] = Some(id as u8);
    }
    // Any list-returning site makes the self-call result an Int list
    // (sites are Int-lane; param pins must agree). Otherwise the declared
    // scalar pins one pass and undeclared falls back to Int/Bool inference.
    let assumed_set: Vec<Kind> = if ret_any_list {
        // Sites are Int-lane; list params must agree.
        if list_params.iter().any(|(_, e)| *e != PinElem::Int) {
            return None;
        }
        vec![Kind::IntList]
    } else {
        match declared_ret {
            Some(SlotKind::Int) => vec![Kind::Int],
            Some(SlotKind::Bool) => vec![Kind::Bool],
            Some(SlotKind::Float) => vec![Kind::Float],
            None => vec![Kind::Int, Kind::Bool],
        }
    };
    let mut chosen: Option<(Vec<Option<Vec<Kind>>>, Kind)> = None;
    for assumed in assumed_set {
        let mut entry = vec![Kind::Unknown; register_count as usize + 2];
        entry[..param_count as usize].copy_from_slice(&param_entry);
        // Self-calls resolve against the assumed signature; calls to OTHER
        // functions against their DECLARED all-scalar signatures (the call
        // stencil dispatches through the published table; an uncompiled
        // callee deopts at the call until it tiers up on its own). Mode B
        // keeps self-calls only — its pin staging is self-shaped.
        let resolver = |fi: u16| -> Option<(Kind, Vec<Kind>)> {
            if fi == self_fi {
                return Some((assumed, param_entry.clone()));
            }
            if mode_b {
                return None;
            }
            let sig = callees.get(fi as usize)?;
            let ret = match sig.ret? {
                SlotKind::Int => Kind::Int,
                SlotKind::Bool => Kind::Bool,
                SlotKind::Float => Kind::Float,
            };
            let params = sig
                .param_kinds
                .iter()
                .map(|k| match k {
                    Some(ParamKind::Scalar(SlotKind::Int)) => Some(Kind::Int),
                    Some(ParamKind::Scalar(SlotKind::Bool)) => Some(Kind::Bool),
                    Some(ParamKind::Scalar(SlotKind::Float)) => Some(Kind::Float),
                    _ => None,
                })
                .collect::<Option<Vec<Kind>>>()?;
            Some((ret, params))
        };
        let Some(kin) = kind_flow(ops, entry_pc, constants, entry, Some(&resolver))
        else {
            continue;
        };
        // The trailing implicit ReturnNothing must be dead code — a
        // reachable one would return Nothing, which native code cannot
        // represent. All reachable returns must agree on Int or Bool.
        let mut ret_kind: Option<Kind> = None;
        let mut ok = true;
        for (i, op) in ops.iter().enumerate() {
            let Some(k) = &kin[i] else { continue };
            match *op {
                Op::ReturnNothing => {
                    ok = false;
                    break;
                }
                Op::Return { src } => {
                    let rk = k[src as usize];
                    match ret_kind {
                        None => ret_kind = Some(rk),
                        Some(prev) if prev == rk => {}
                        _ => {
                            ok = false;
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        if !ok {
            continue;
        }
        let Some(rk) = ret_kind else { continue };
        if rk == assumed {
            chosen = Some((kin, rk));
            break;
        }
    }
    if chosen.is_none() {
        jt("chosen");
    }
    let (kin, ret_kind) = chosen?;
    let ret = if ret_any_list {
        NativeRet::ListByHandle
    } else {
        NativeRet::Scalar(ret_kind.slot_kind()?)
    };

    // Liveness for the cmp+branch fusion (functions have no write-back, so
    // deadness inside the body is the whole story).
    let live = liveness(ops, entry_pc, register_count as usize + 2);
    if live.is_none() {
        jt("liveness");
    }
    let live = live?;
    let jump_targets: std::collections::HashSet<usize> = ops
        .iter()
        .filter_map(|op| match *op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. }
            | Op::JumpIfInt { target, .. } => Some(target),
            _ => None,
        })
        .collect();
    let scratch_ok = |_: u16| true;

    let conv = (register_count, register_count + 1);
    let rc = register_count as usize;
    // Frame layout: mode A keeps the classic [regs | conv ×2 | limit]
    // (size rc+3). Mode B inserts the plant slots and pin triples BEFORE
    // the limit: [regs | conv ×2 | window | resume | dst | pins ×3p |
    // limit] — the published table value is frame_size − 3 either way, so
    // the call stencil's bound math and limit planting hold unchanged.
    let npins = total_pins;
    let frame_size = if mode_b { rc + 6 + 3 * npins } else { rc + 3 };
    let plant_window = (rc + 2) as u16;
    let plant_resume = (rc + 3) as u16;
    let plant_dst = (rc + 4) as u16;
    let pins_start = rc + 5;
    let mut micro: Vec<MicroOp> = Vec::with_capacity(ops.len());
    let mut pc_to_micro: Vec<usize> = Vec::with_capacity(ops.len());
    let mut fixups: Vec<usize> = Vec::new();

    // Every pin-aliased register resolves to its pin's slot triple.
    let pin_elem = |j: usize| -> PinElem {
        let reg = list_regs[j];
        list_params
            .iter()
            .find(|(preg, _)| *preg == reg)
            .map(|(_, e)| *e)
            .unwrap_or(PinElem::Int)
    };
    let pin_slots_of = |j: usize| -> PinSlots {
        let base = pins_start + 3 * j;
        PinSlots {
            vec_slot: base as u16,
            ptr_slot: (base + 1) as u16,
            len_slot: (base + 2) as u16,
            elem: pin_elem(j),
        }
    };
    let mut pins: std::collections::HashMap<u16, PinSlots> = std::collections::HashMap::new();
    if mode_b {
        for (r, p) in pin_of.iter().enumerate() {
            if let Some(j) = p {
                pins.insert(r as u16, pin_slots_of(*j as usize));
            }
        }
        // Prologue: my plant is invalid until I make a call (the precise
        // walk stops at the first invalid plant).
        micro.push(MicroOp::LoadConst { dst: plant_window, value: -1 });
    }

    // Per-pause-point register re-box kinds (deopt sites + call sites).
    // Param-pinned list registers re-box as their argument; NATIVE-OWNED
    // pins (sites, call results) record their vec-handle slot so the
    // deopt walk can detach-or-match them.
    let mut kinds_by_pc: std::collections::HashMap<usize, Vec<RegBox>> =
        std::collections::HashMap::new();
    let mut owned_by_pc: std::collections::HashMap<usize, Vec<(u16, u16)>> =
        std::collections::HashMap::new();
    let mut capture = |i: usize,
                       kin: &Vec<Option<Vec<Kind>>>,
                       pin_of: &Vec<Option<u8>>|
     -> Option<()> {
        let k = kin[i].as_ref()?;
        let mut row = Vec::with_capacity(rc);
        let mut owned: Vec<(u16, u16)> = Vec::new();
        for r in 0..rc {
            row.push(match k[r] {
                Kind::Int => RegBox::Int,
                Kind::Bool => RegBox::Bool,
                Kind::Float => RegBox::Float,
                Kind::IntList | Kind::FloatList | Kind::BoolList => {
                    let j = pin_of[r]?;
                    if let Some(pidx) =
                        list_params.iter().position(|(preg, _)| *preg as usize == r)
                    {
                        RegBox::ListParam(pidx as u8)
                    } else {
                        owned.push((r as u16, pin_slots_of(j as usize).vec_slot));
                        RegBox::Resolved
                    }
                }
                _ => RegBox::Dead,
            });
        }
        kinds_by_pc.insert(entry_pc + i, row);
        owned_by_pc.insert(entry_pc + i, owned);
        Some(())
    };

    let mut i = 0usize;
    while i < ops.len() {
        if mode_b {
            let deoptable = matches!(
                ops[i],
                Op::Div { .. }
                    | Op::Mod { .. }
                    | Op::Index { .. }
                    | Op::IndexUnchecked { .. }
                    | Op::SetIndex { .. }
                    | Op::SetIndexUnchecked { .. }
                    | Op::Call { .. }
            );
            if deoptable && kin[i].is_some() {
                if capture(i, &kin, &pin_of).is_none() {
                    jt("capture");
                    return None;
                }
            }
            // Self-call, mode B: the callee windows at base + FRAME_SIZE —
            // fully DISJOINT from this frame, because the interpreter's
            // compact windowing (base + args_start) would let the callee's
            // registers overlap this frame's plant and pin slots and
            // clobber them. Arguments are copied into the fresh window,
            // pins pass through, the plant records the linkage for the
            // precise walk, and the post-return store invalidates it.
            if let Op::Call { dst, args_start, arg_count, func } = ops[i] {
                // Mode B stages pins for OUR OWN parameter shape — a call
                // to any other function cannot use this seam.
                if func != self_fi {
                    jt("non-self-call-mode-b");
                    return None;
                }
                pc_to_micro.push(micro.len());
                let window = frame_size as u16;
                micro.push(MicroOp::LoadConst { dst: plant_window, value: window as i64 });
                micro.push(MicroOp::LoadConst {
                    dst: plant_resume,
                    value: (entry_pc + i + 1) as i64,
                });
                micro.push(MicroOp::LoadConst { dst: plant_dst, value: dst as i64 });
                for j in 0..arg_count {
                    micro.push(MicroOp::Move { dst: window + j, src: args_start + j });
                }
                // Each callee list-param pin receives the triple of WHATEVER
                // pin the staged argument carries (a param passthrough, a
                // fresh site, or a previous call's result).
                for (slot, (preg, pelem)) in list_params.iter().enumerate() {
                    if *pelem != PinElem::Int && *pelem != PinElem::Float && *pelem != PinElem::Bool
                    {
                        return None;
                    }
                    let Some(arg_pin) = pin_of[(args_start + preg) as usize] else {
                        jt("staged-arg-unpinned");
                        return None;
                    };
                    let src_slots = pin_slots_of(arg_pin as usize);
                    let dst_base = window + (pins_start + 3 * slot) as u16;
                    micro.push(MicroOp::Move { dst: dst_base, src: src_slots.vec_slot });
                    micro.push(MicroOp::Move { dst: dst_base + 1, src: src_slots.ptr_slot });
                    micro.push(MicroOp::Move { dst: dst_base + 2, src: src_slots.len_slot });
                }
                let c = call_ctx_in;
                micro.push(MicroOp::Call {
                    dst,
                    args_start: window,
                    table_addr: c.table_addr,
                    depth_addr: c.depth_addr,
                    status_addr: c.status_addr,
                    limit_slot: c.limit_slot,
                    depth_limit: c.depth_limit,
                });
                micro.push(MicroOp::LoadConst { dst: plant_window, value: -1 });
                // A list-returning self-call: plant the result's triple
                // from the returned handle.
                if fn_returns_lists {
                    if let Some(j) = pin_of[dst as usize] {
                        let slots = pin_slots_of(j as usize);
                        micro.push(MicroOp::ListTriple {
                            handle_slot: dst,
                            vec_slot: slots.vec_slot,
                            ptr_slot: slots.ptr_slot,
                            len_slot: slots.len_slot,
                            helper_addr: logos_rt_list_triple as usize as i64,
                        });
                    }
                }
                i += 1;
                continue;
            }
            // Fresh-list allocation: registry-owned, triple planted.
            if let Op::NewEmptyList { dst } = ops[i] {
                pc_to_micro.push(micro.len());
                let j = pin_of[dst as usize].expect("site pins were assigned");
                let slots = pin_slots_of(j as usize);
                micro.push(MicroOp::NewList {
                    vec_slot: slots.vec_slot,
                    ptr_slot: slots.ptr_slot,
                    len_slot: slots.len_slot,
                    helper_addr: logos_rt_alloc_list_i64 as usize as i64,
                });
                // INVARIANT: a list register's frame slot mirrors its vec
                // handle (Moves, staging and returns all ride it).
                micro.push(MicroOp::Move { dst, src: slots.vec_slot });
                i += 1;
                continue;
            }
            // A Move BETWEEN list registers transfers the handle and
            // refreshes the destination's own pin triple from the live
            // vector (per-register pins make reassignment conflict-free).
            if let Op::Move { dst, src } = ops[i] {
                if is_list[src as usize] {
                    pc_to_micro.push(micro.len());
                    let j = pin_of[dst as usize].expect("list regs are pinned");
                    let slots = pin_slots_of(j as usize);
                    micro.push(MicroOp::Move { dst, src });
                    micro.push(MicroOp::ListTriple {
                        handle_slot: dst,
                        vec_slot: slots.vec_slot,
                        ptr_slot: slots.ptr_slot,
                        len_slot: slots.len_slot,
                        helper_addr: logos_rt_list_triple as usize as i64,
                    });
                    i += 1;
                    continue;
                }
            }
            // List return: the vec HANDLE rides the return value (the
            // boundary detaches registry-owned lists or matches a param).
            if let Op::Return { src } = ops[i] {
                if let Some(j) = pin_of[src as usize] {
                    pc_to_micro.push(micro.len());
                    let slots = pin_slots_of(j as usize);
                    micro.push(MicroOp::Move { dst: conv.0, src: slots.vec_slot });
                    micro.push(MicroOp::Return { src: conv.0 });
                    i += 1;
                    continue;
                }
            }
            // OPERATION FUSION (nbody's hot lever): collapse
            //   `t1 = item I of A; t2 = item J of A; D = t1 <op> t2`
            // — two loads from the SAME pinned 8-byte float array feeding one
            // float add/sub/mul — into a single ArrLoad2F so the two loaded
            // f64s never round-trip through the frame. Both scratch loads must
            // be single-use (dead after the binop, read only by it), the two
            // collapsed ops must not be jump targets (no entry lands between),
            // and a bounds failure side-exits to the FIRST Index's bytecode pc
            // (captured above, since Index is deoptable) — resuming there
            // re-runs all three pure ops, every effect still confined.
            if let Some(fused) = fuse_index2_fbinop(
                ops, i, entry_pc, &kin, &live, &jump_targets, &pins,
            ) {
                if std::env::var_os("LOGOS_JIT_TRACE").is_some() {
                    eprintln!("jit-trace: FUSED arrld2 at pc {} -> {:?}", entry_pc + i, fused);
                }
                pc_to_micro.push(micro.len());
                micro.push(fused);
                // The j-Index (i+1) and the binop (i+2) map PAST the fused
                // micro, so the first Index's deopt range covers exactly it.
                pc_to_micro.push(micro.len());
                pc_to_micro.push(micro.len());
                i += 3;
                continue;
            }
        }
        let step = translate_op(
            ops,
            i,
            entry_pc,
            constants,
            &kin,
            &live,
            &jump_targets,
            &scratch_ok,
            conv,
            &pins,
            Some(call_ctx_in),
            None,
            true,
            false,
            &mut micro,
            &mut pc_to_micro,
            &mut fixups,
        );
        if step.is_none() {
            jt("translate");
            if std::env::var_os("LOGOS_JIT_TRACE").is_some() {
                eprintln!("jit-trace:   op {:?}", ops[i]);
            }
        }
        i += step?;
    }
    for &fi in &fixups {
        match &mut micro[fi] {
            MicroOp::Jump { target }
            | MicroOp::JumpIfFalse { target, .. }
            | MicroOp::JumpIfTrue { target, .. }
            | MicroOp::Branch { target, .. }
            | MicroOp::BranchF { target, .. } => {
                let rebased = target.checked_sub(entry_pc)?;
                *target = *pc_to_micro.get(rebased)?;
            }
            _ => unreachable!(),
        }
    }
    // Function bodies may end on a Return-less path only if the VM never
    // reaches it; the chain compiler requires structural termination.
    if !matches!(micro.last(), Some(MicroOp::Return { .. }) | Some(MicroOp::Jump { .. })) {
        return None;
    }
    // Mode B: every micro of bytecode op i carries that op's encoded pc
    // so its side exit resumes precisely.
    let deopt_codes = if mode_b {
        let mut codes = vec![1i64; micro.len()];
        for opi in 0..ops.len() {
            let lo = pc_to_micro[opi];
            let hi = pc_to_micro.get(opi + 1).copied().unwrap_or(micro.len());
            let code = (((entry_pc + opi) as i64) << 2) | 3;
            for c in codes.iter_mut().take(hi).skip(lo) {
                *c = code;
            }
        }
        Some(codes)
    } else {
        None
    };
    let param_pin_scatter: Vec<u16> = list_params
        .iter()
        .map(|(preg, _)| pin_slots_of(pin_of[*preg as usize].unwrap() as usize).vec_slot)
        .collect();
    let precise = if mode_b {
        Some(PreciseInfo {
            register_count: rc,
            pins_start,
            kinds_by_pc,
            owned_by_pc,
            param_pin_scatter,
        })
    } else {
        None
    };
    Some(AdaptedFn { micro, frame_size, ret, deopt_codes, precise })
}

/// If slot `rhs` is a region-constant power of two — its ONLY writer in `ops`
/// is a `LoadConst` of `2^k` with `1 <= k <= 62` — return `k`, so an integer
/// `x / rhs` can lower to the side-exit-free [`MicroOp::DivPow2`] shift instead
/// of an `idiv`. Sound: a single constant writer means `rhs` holds exactly
/// `2^k` at every `Div`, in every loop iteration. Any other writer (or an op
/// whose defs we cannot determine) disqualifies it.
fn const_pow2_div(ops: &[Op], rhs: u16, constants: &[Constant]) -> Option<u32> {
    let mut k_found: Option<u32> = None;
    for op in ops {
        let defs = region_use_def(op)?.1;
        if !defs.contains(&rhs) {
            continue;
        }
        match op {
            Op::LoadConst { dst, idx } if *dst == rhs => match constants.get(*idx as usize) {
                Some(Constant::Int(d)) if *d >= 2 && (*d & (*d - 1)) == 0 => {
                    let k = d.trailing_zeros();
                    if !(1..=62).contains(&k) || k_found.is_some() {
                        return None;
                    }
                    k_found = Some(k);
                }
                _ => return None,
            },
            _ => return None,
        }
    }
    k_found
}

/// The use/def sets of one op from the adaptable subset, or None when the op
/// is outside it. `AddAssign` reads AND writes its dst.
fn region_use_def(op: &Op) -> Option<(Vec<u16>, Vec<u16>)> {
    Some(match *op {
        Op::Return { src } => (vec![src], vec![]),
        Op::ReturnNothing => (vec![], vec![]),
        Op::LoadConst { dst, .. } => (vec![], vec![dst]),
        Op::DivPow2 { dst, lhs, .. } => (vec![lhs], vec![dst]),
        Op::MagicDivU { dst, lhs, .. } => (vec![lhs], vec![dst]),
        Op::Move { dst, src } => (vec![src], vec![dst]),
        Op::AndEager { dst, lhs, rhs }
        | Op::OrEager { dst, lhs, rhs }
        | Op::BitXor { dst, lhs, rhs }
        | Op::Shl { dst, lhs, rhs }
        | Op::Shr { dst, lhs, rhs } => (vec![lhs, rhs], vec![dst]),
        Op::Not { dst, src } => (vec![src], vec![dst]),
        Op::JumpIfInt { cond, .. } => (vec![cond], vec![]),
        Op::Index { dst, collection, index }
        | Op::IndexUnchecked { dst, collection, index } => (vec![collection, index], vec![dst]),
        Op::Contains { dst, collection, value } => (vec![collection, value], vec![dst]),
        Op::NewEmptyList { dst } => (vec![], vec![dst]),
        Op::SetIndex { collection, index, value }
        | Op::SetIndexUnchecked { collection, index, value } => {
            (vec![collection, index, value], vec![])
        }
        Op::Length { dst, collection } => (vec![collection], vec![dst]),
        Op::ListPush { list, value } => (vec![list, value], vec![]),
        Op::Call { dst, args_start, arg_count, .. } => {
            ((args_start..args_start + arg_count).collect(), vec![dst])
        }
        Op::CallBuiltin {
            dst,
            builtin: logicaffeine_compile::semantics::builtins::BuiltinId::Sqrt,
            args_start,
            arg_count,
        } if arg_count == 1 => (vec![args_start], vec![dst]),
        Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs } | Op::Mul { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs } | Op::Mod { dst, lhs, rhs }
        | Op::Lt { dst, lhs, rhs } | Op::Gt { dst, lhs, rhs } | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs } | Op::Eq { dst, lhs, rhs } | Op::NotEq { dst, lhs, rhs } => {
            (vec![lhs, rhs], vec![dst])
        }
        Op::AddAssign { dst, src } => (vec![dst, src], vec![dst]),
        Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => (vec![cond], vec![]),
        Op::Jump { .. } => (vec![], vec![]),
        // Region-entry metadata: emits no native code (the VM verifies it at
        // entry from its own registers + the pinned length). No frame use/def.
        Op::RegionBoundsGuard { .. } => (vec![], vec![]),
        _ => return None,
    })
}

/// Backward liveness over a bytecode slice to a fixpoint: per-op LIVE-IN
/// vectors. Successors are the fall-through and the in-slice jump target;
/// out-of-slice targets are exits with empty live-out (exit-observable slots
/// flow through the caller's write-back, never the native frame), and
/// `Return`/`ReturnNothing` are terminals. None when the slice contains an
/// op outside the adaptable subset.
fn liveness(ops: &[Op], base_pc: usize, nregs: usize) -> Option<Vec<Vec<bool>>> {
    let in_slice = |t: usize| (base_pc..base_pc + ops.len()).contains(&t);
    fn merge_into(out: &mut [bool], row: &[bool]) {
        for (o, &l) in out.iter_mut().zip(row) {
            *o |= l;
        }
    }
    let mut live: Vec<Vec<bool>> = vec![vec![false; nregs]; ops.len()];
    loop {
        let mut changed = false;
        for i in (0..ops.len()).rev() {
            let Some((uses, defs)) = region_use_def(&ops[i]) else {
                if std::env::var_os("LOGOS_RDIAG").is_some() {
                    eprintln!("RDIAG-BAIL liveness use_def-unsupported op={:?}", ops[i]);
                }
                return None;
            };
            let mut out = vec![false; nregs];
            match ops[i] {
                Op::Jump { target } => {
                    if in_slice(target) {
                        merge_into(&mut out, &live[target - base_pc]);
                    }
                }
                Op::JumpIfFalse { target, .. }
                | Op::JumpIfTrue { target, .. }
                | Op::JumpIfInt { target, .. } => {
                    if i + 1 < ops.len() {
                        merge_into(&mut out, &live[i + 1]);
                    }
                    if in_slice(target) {
                        merge_into(&mut out, &live[target - base_pc]);
                    }
                }
                Op::Return { .. } | Op::ReturnNothing => {}
                _ => {
                    if i + 1 < ops.len() {
                        merge_into(&mut out, &live[i + 1]);
                    }
                }
            }
            for &d in &defs {
                out[d as usize] = false;
            }
            for &u in &uses {
                out[u as usize] = true;
            }
            if out != live[i] {
                live[i] = out;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Some(live)
}

/// The [`Cmp`] a comparison op computes, if it is one.
fn cmp_of(op: &Op) -> Option<(Cmp, u16, u16, u16)> {
    Some(match *op {
        Op::Lt { dst, lhs, rhs } => (Cmp::Lt, dst, lhs, rhs),
        Op::Gt { dst, lhs, rhs } => (Cmp::Gt, dst, lhs, rhs),
        Op::LtEq { dst, lhs, rhs } => (Cmp::LtEq, dst, lhs, rhs),
        Op::GtEq { dst, lhs, rhs } => (Cmp::GtEq, dst, lhs, rhs),
        Op::Eq { dst, lhs, rhs } => (Cmp::Eq, dst, lhs, rhs),
        Op::NotEq { dst, lhs, rhs } => (Cmp::NotEq, dst, lhs, rhs),
        _ => return None,
    })
}

/// When `ops[i]` is a comparison whose dst feeds ONLY the immediately
/// following conditional jump, return the fused [`MicroOp::Branch`]
/// parameters `(cmp, lhs, rhs, vm_target)` — `cmp` already negated for the
/// JumpIfTrue polarity (Branch jumps when its cmp is FALSE). Conditions:
/// the dst is dead after the jump (per `live`), no jump lands BETWEEN the
/// pair, and `scratch_ok(dst)` (regions: the dst must be an unobservable
/// scratch — its value write disappears under fusion).
fn fuse_cmp_branch(
    ops: &[Op],
    i: usize,
    base_pc: usize,
    live: &[Vec<bool>],
    jump_targets: &std::collections::HashSet<usize>,
    scratch_ok: &dyn Fn(u16) -> bool,
) -> Option<(Cmp, u16, u16, usize)> {
    let (cmp, dst, lhs, rhs) = cmp_of(&ops[i])?;
    if i + 1 >= ops.len() || jump_targets.contains(&(base_pc + i + 1)) {
        return None;
    }
    let (fused_cmp, target) = match ops[i + 1] {
        Op::JumpIfFalse { cond, target } if cond == dst => (cmp, target),
        Op::JumpIfTrue { cond, target } if cond == dst => (cmp.negated(), target),
        _ => return None,
    };
    if !scratch_ok(dst) {
        return None;
    }
    // Dead after the jump: not live-in at the fall-through nor at an
    // in-slice target (out-of-slice exits are covered by scratch_ok).
    let in_slice = |t: usize| (base_pc..base_pc + ops.len()).contains(&t);
    if i + 2 < ops.len() && live[i + 2][dst as usize] {
        return None;
    }
    if in_slice(target) && live[target - base_pc][dst as usize] {
        return None;
    }
    Some((fused_cmp, lhs, rhs, target))
}

/// Detect the fusable triple at `ops[i]`:
///   `t1 = item I of A; t2 = item J of A; D = t1 <op> t2`
/// — two loads from the SAME pinned 8-byte float array (`A`), both feeding a
/// single float add/sub/mul. Returns the collapsed [`MicroOp::ArrLoad2F`].
///
/// Conditions (all required for soundness/no-loss):
/// - the two `item` reads target the SAME pinned collection, which is a
///   list of 8-byte FLOAT elements (not a map, not a byte/Bool buffer);
/// - both loaded values are FLOATS at the binop (no Int→Float promotion is
///   folded — the fused stencil reads raw f64 bits straight from the buffer);
/// - the binop's operands are exactly the two scratch loads (for the
///   commutative Add/Mul either order; for Sub only `t1 - t2`);
/// - both scratches are SINGLE-USE: distinct, dead after the binop, and read
///   by nothing between (the three ops are adjacent and i+1/i+2 are not jump
///   targets, so no other reader can sneak in);
/// - the destination is not one of the scratch slots (the binop overwrites
///   its inputs only after both are read — the fused stencil does the same,
///   but keeping them distinct avoids any aliasing surprise).
fn fuse_index2_fbinop(
    ops: &[Op],
    i: usize,
    base_pc: usize,
    kin: &[Option<Vec<Kind>>],
    live: &[Vec<bool>],
    jump_targets: &std::collections::HashSet<usize>,
    pins: &std::collections::HashMap<u16, PinSlots>,
) -> Option<MicroOp> {
    if i + 3 > ops.len() {
        return None;
    }
    // No fresh entry may land on the second load or the binop, or a jump
    // target would skip the first load's effects.
    if jump_targets.contains(&(base_pc + i + 1)) || jump_targets.contains(&(base_pc + i + 2)) {
        return None;
    }
    // The first load is the deopt resume point; its kind capture must exist
    // (the mode-B loop captures every deoptable op with a known kind row).
    if kin.get(i)?.is_none() {
        return None;
    }
    let (t1, coll_a, ix) = match ops[i] {
        Op::Index { dst, collection, index }
        | Op::IndexUnchecked { dst, collection, index } => (dst, collection, index),
        _ => return None,
    };
    let (t2, coll_b, jx) = match ops[i + 1] {
        Op::Index { dst, collection, index }
        | Op::IndexUnchecked { dst, collection, index } => (dst, collection, index),
        _ => return None,
    };
    if coll_a != coll_b || t1 == t2 {
        return None;
    }
    let (op, dst, lhs, rhs) = match ops[i + 2] {
        Op::Add { dst, lhs, rhs } => (FOp::Add, dst, lhs, rhs),
        Op::Sub { dst, lhs, rhs } => (FOp::Sub, dst, lhs, rhs),
        Op::Mul { dst, lhs, rhs } => (FOp::Mul, dst, lhs, rhs),
        _ => return None,
    };
    // The binop must consume exactly the two scratch loads, mapping i→j so
    // the result equals `f(A[I]) <op> f(A[J])`. Add/Mul commute; Sub does not.
    let (load_i, load_j) = if lhs == t1 && rhs == t2 {
        (ix, jx)
    } else if (op == FOp::Add || op == FOp::Mul) && lhs == t2 && rhs == t1 {
        (jx, ix)
    } else {
        return None;
    };
    if dst == t1 || dst == t2 {
        return None;
    }
    // The collection is a pinned list of 8-byte float elements.
    let p = pins.get(&coll_a)?;
    if p.elem != PinElem::Float {
        return None;
    }
    // Both loaded values are floats at the binop (the fused stencil reads raw
    // f64 bits — an Int element would need a promotion the stencil omits).
    let k = kin.get(i + 2)?.as_ref()?;
    if k[t1 as usize] != Kind::Float || k[t2 as usize] != Kind::Float {
        return None;
    }
    // Single-use: both scratches dead entering the op after the binop, so the
    // collapse drops their frame writes with no observable loss.
    let after = live.get(i + 3)?;
    if after[t1 as usize] || after[t2 as usize] {
        return None;
    }
    Some(MicroOp::ArrLoad2F {
        dst,
        i: load_i,
        j: load_j,
        ptr_slot: p.ptr_slot,
        len_slot: p.len_slot,
        op,
    })
}

/// Translate the VM op at `i` (or a fused cmp+branch pair) into micro-ops,
/// choosing Int or Float forms from the kinds holding at that point and
/// inserting the kernel's Int→Float promotion through the two conversion
/// scratch slots. Jump-family micro targets keep their RAW VM pcs — the
/// caller remaps them (and routes out-of-slice exits). Pushes the pc→micro
/// map entries for every VM op it consumes; returns how many it consumed,
/// or None when the op (or its kind shape) is outside the subset.
#[allow(clippy::too_many_arguments)]
fn translate_op(
    ops: &[Op],
    i: usize,
    base_pc: usize,
    constants: &[Constant],
    kin: &[Option<Vec<Kind>>],
    live: &[Vec<bool>],
    jump_targets: &std::collections::HashSet<usize>,
    scratch_ok: &dyn Fn(u16) -> bool,
    conv: (u16, u16),
    pins: &std::collections::HashMap<u16, PinSlots>,
    call_ctx: Option<&CallCtx>,
    region_return: Option<(u16, u16)>,
    allow_return: bool,
    region: bool,
    micro: &mut Vec<MicroOp>,
    pc_to_micro: &mut Vec<usize>,
    fixups: &mut Vec<usize>,
) -> Option<usize> {
    pc_to_micro.push(micro.len());
    let k = kin[i].as_ref();
    // Unreachable ops never execute — emit a placeholder of valid shape so
    // pc mapping and structural termination hold.
    let kind_of = |r: u16| k.map(|kk| kk[r as usize]).unwrap_or(Kind::Int);
    // Resolve a numeric operand to (slot, is_float), inserting a conversion
    // into `cslot` when the other side forces a Float promotion.
    let promote = |micro: &mut Vec<MicroOp>, slot: u16, kind: Kind, want_float: bool, cslot: u16| -> u16 {
        if want_float && kind == Kind::Int {
            micro.push(MicroOp::IntToFloat { dst: cslot, src: slot });
            cslot
        } else {
            slot
        }
    };

    // Fused compare-and-branch first.
    if let Some((cmp, lhs, rhs, target)) =
        fuse_cmp_branch(ops, i, base_pc, live, jump_targets, scratch_ok)
    {
        let (lk, rk) = (kind_of(lhs), kind_of(rhs));
        let bools = lk == Kind::Bool && rk == Kind::Bool;
        if bools && matches!(cmp, Cmp::Eq | Cmp::NotEq) {
            pc_to_micro.push(micro.len());
            fixups.push(micro.len());
            micro.push(MicroOp::Branch { cmp, lhs, rhs, target });
            return Some(2);
        }
        let want_float = lk == Kind::Float || rk == Kind::Float;
        if want_float {
            let l = promote(micro, lhs, lk, true, conv.0);
            let r = promote(micro, rhs, rk, true, conv.1);
            pc_to_micro.push(micro.len());
            fixups.push(micro.len());
            micro.push(MicroOp::BranchF { cmp, lhs: l, rhs: r, target });
        } else {
            pc_to_micro.push(micro.len());
            fixups.push(micro.len());
            micro.push(MicroOp::Branch { cmp, lhs, rhs, target });
        }
        return Some(2);
    }

    match ops[i] {
        Op::LoadConst { dst, idx } => match constants.get(idx as usize)? {
            Constant::Int(v) => micro.push(MicroOp::LoadConst { dst, value: *v }),
            Constant::Float(f) => {
                micro.push(MicroOp::LoadConst { dst, value: f.to_bits() as i64 })
            }
            Constant::Bool(b) => micro.push(MicroOp::LoadConst { dst, value: *b as i64 }),
            // A 1-char ASCII text/char literal lowers to its single byte so a
            // `Kind::TextByte` comparison against it is an integer compare (the
            // strings benchmark's `item i of result equals " "`).
            Constant::Char(c) if c.is_ascii() => {
                micro.push(MicroOp::LoadConst { dst, value: *c as i64 })
            }
            Constant::Text(s) if text_const_byte(s).is_some() => {
                micro.push(MicroOp::LoadConst { dst, value: text_const_byte(s).unwrap() as i64 })
            }
            // A MULTI-char ASCII text constant (`text + "XXXXX"`) is consumed ONLY
            // as a `StrAppend` source (`Kind::TextConst`, rejected by every other
            // gate), which bakes the literal's bytes directly — its frame slot is
            // never read as a value. Emit a harmless placeholder so the pc→micro
            // mapping stays 1:1 without materializing the (non-scalar) string.
            Constant::Text(s) if s.is_ascii() && !s.is_empty() => {
                micro.push(MicroOp::LoadConst { dst, value: 0 })
            }
            _ => return None,
        },
        Op::Move { dst, src } => micro.push(MicroOp::Move { dst, src }),
        Op::DivPow2 { dst, lhs, k } => micro.push(MicroOp::DivPow2 { dst, lhs, k: k as u32 }),
        Op::MagicDivU { dst, lhs, magic, more, mul_back } => {
            micro.push(MicroOp::MagicDivU { dst, lhs, magic, more, mul_back })
        }
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs } => {
            let (lk, rk) = (kind_of(lhs), kind_of(rhs));
            if lk == Kind::Float || rk == Kind::Float {
                let l = promote(micro, lhs, lk, true, conv.0);
                let r = promote(micro, rhs, rk, true, conv.1);
                micro.push(match ops[i] {
                    Op::Add { .. } => MicroOp::AddF { dst, lhs: l, rhs: r },
                    Op::Sub { .. } => MicroOp::SubF { dst, lhs: l, rhs: r },
                    Op::Mul { .. } => MicroOp::MulF { dst, lhs: l, rhs: r },
                    _ => MicroOp::DivF { dst, lhs: l, rhs: r },
                });
            } else {
                micro.push(match ops[i] {
                    Op::Add { .. } => MicroOp::Add { dst, lhs, rhs },
                    Op::Sub { .. } => MicroOp::Sub { dst, lhs, rhs },
                    Op::Mul { .. } => MicroOp::Mul { dst, lhs, rhs },
                    // integer `x / 2^k` → side-exit-free shift when the divisor
                    // is a region-constant power of two (collatz, heap_sort i/2,
                    // mergesort (lo+hi)/2).
                    _ => match const_pow2_div(ops, rhs, constants) {
                        Some(k) => MicroOp::DivPow2 { dst, lhs, k },
                        None => MicroOp::Div { dst, lhs, rhs },
                    },
                });
            }
        }
        Op::AddAssign { dst, src } => {
            let (dk, sk) = (kind_of(dst), kind_of(src));
            if dk == Kind::TextMut {
                // A pinned mutable-Text accumulator grows through the `StrAppend`
                // helper. The handle is the pin's vec slot (a `*mut Value`). The
                // source is a 1-char runtime byte (`TextByte` — the byte VALUE
                // rides its frame slot) or a multi-char ASCII constant
                // (`TextConst` — recover the literal's bytes and bake a `'static`
                // slice). The handle slot is force-frame-resident so a non-frame
                // (register-resident) handle is impossible here.
                let p = pins.get(&dst)?;
                let strsrc = match sk {
                    Kind::TextByte => logicaffeine_forge::jit::StrSrc::Byte(src),
                    Kind::TextConst => {
                        let (ptr, len) = const_text_slice(ops, i, base_pc, src, constants)?;
                        logicaffeine_forge::jit::StrSrc::Const { ptr, len }
                    }
                    _ => return None,
                };
                micro.push(MicroOp::StrAppend {
                    text_handle_slot: p.vec_slot,
                    src: strsrc,
                    helper_addr: crate::logos_rt_str_append as usize as i64,
                });
            } else if dk == Kind::Float || sk == Kind::Float {
                let l = promote(micro, dst, dk, true, conv.0);
                let r = promote(micro, src, sk, true, conv.1);
                micro.push(MicroOp::AddF { dst, lhs: l, rhs: r });
            } else {
                micro.push(MicroOp::Add { dst, lhs: dst, rhs: src });
            }
        }
        Op::Mod { dst, lhs, rhs } => micro.push(MicroOp::Mod { dst, lhs, rhs }),
        Op::BitXor { dst, lhs, rhs } => micro.push(MicroOp::BitXor { dst, lhs, rhs }),
        Op::Shl { dst, lhs, rhs } => micro.push(MicroOp::Shl { dst, lhs, rhs }),
        Op::Shr { dst, lhs, rhs } => micro.push(MicroOp::Shr { dst, lhs, rhs }),
        Op::AndEager { dst, lhs, rhs } => micro.push(MicroOp::BitAnd { dst, lhs, rhs }),
        Op::OrEager { dst, lhs, rhs } => micro.push(MicroOp::BitOr { dst, lhs, rhs }),
        Op::Not { dst, src } => match kind_of(src) {
            Kind::Bool => micro.push(MicroOp::NotBool { dst, src }),
            _ => micro.push(MicroOp::NotInt { dst, src }),
        },
        Op::Lt { dst, lhs, rhs }
        | Op::Gt { dst, lhs, rhs }
        | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs }
        | Op::Eq { dst, lhs, rhs }
        | Op::NotEq { dst, lhs, rhs } => {
            let (lk, rk) = (kind_of(lhs), kind_of(rhs));
            if lk == Kind::Float || rk == Kind::Float {
                let l = promote(micro, lhs, lk, true, conv.0);
                let r = promote(micro, rhs, rk, true, conv.1);
                micro.push(match ops[i] {
                    Op::Lt { .. } => MicroOp::LtF { dst, lhs: l, rhs: r },
                    Op::Gt { .. } => MicroOp::GtF { dst, lhs: l, rhs: r },
                    Op::LtEq { .. } => MicroOp::LtEqF { dst, lhs: l, rhs: r },
                    Op::GtEq { .. } => MicroOp::GtEqF { dst, lhs: l, rhs: r },
                    Op::Eq { .. } => MicroOp::EqF { dst, lhs: l, rhs: r },
                    _ => MicroOp::NeqF { dst, lhs: l, rhs: r },
                });
            } else {
                micro.push(match ops[i] {
                    Op::Lt { .. } => MicroOp::Lt { dst, lhs, rhs },
                    Op::Gt { .. } => MicroOp::Gt { dst, lhs, rhs },
                    Op::LtEq { .. } => MicroOp::LtEq { dst, lhs, rhs },
                    Op::GtEq { .. } => MicroOp::GtEq { dst, lhs, rhs },
                    Op::Eq { .. } => MicroOp::Eq { dst, lhs, rhs },
                    _ => MicroOp::Neq { dst, lhs, rhs },
                });
            }
        }
        Op::Jump { target } => {
            fixups.push(micro.len());
            micro.push(MicroOp::Jump { target });
        }
        Op::JumpIfFalse { cond, target } => {
            fixups.push(micro.len());
            micro.push(MicroOp::JumpIfFalse { cond, target });
        }
        Op::JumpIfTrue { cond, target } => {
            fixups.push(micro.len());
            micro.push(MicroOp::JumpIfTrue { cond, target });
        }
        // Statically resolved by kind: a proven-Int condition always takes
        // the jump; a proven-Bool/Float never does (emit nothing).
        Op::JumpIfInt { cond, target } => match kind_of(cond) {
            Kind::Int => {
                fixups.push(micro.len());
                micro.push(MicroOp::Jump { target });
            }
            Kind::Bool | Kind::Float => {}
            _ => return None,
        },
        // `IndexUnchecked` carries the Oracle's in-bounds proof (M9 range
        // analysis): its array load drops the bounds branch entirely
        // (V8/LLVM-style bounds-check elimination). Maps always check
        // through their helper, so the proof only affects the list lane.
        Op::Index { dst, collection, index }
        | Op::IndexUnchecked { dst, collection, index } => {
            // Elision is region-only: the function tier has no per-loop entry
            // guard, so it keeps every access checked (a speculative
            // `IndexUnchecked` there would be unguarded). Static proofs lose
            // nothing measurable in functions.
            let p = pins.get(&collection)?;
            if p.elem == PinElem::Map {
                micro.push(MicroOp::MapGet {
                    dst,
                    key: index,
                    map_slot: p.vec_slot,
                    helper_addr: logos_rt_map_get_ii as usize as i64,
                });
            } else {
                // A Text-as-bytes pin loads ONE byte (the ASCII char codepoint)
                // and is ALWAYS bounds-checked: indexing past an ASCII text must
                // raise the same error at the same point as the tree-walker's
                // per-char path (`IndexUnchecked` elision is never applied to a
                // text — no RegionBoundsGuard is emitted for it). Bool buffers
                // are 1-byte too; Int/Float are 8-byte.
                let text = p.elem == PinElem::TextBytes;
                let byte = text || p.elem == PinElem::Bool;
                let checked =
                    text || !region || matches!(ops[i], Op::Index { .. });
                micro.push(MicroOp::ArrLoad {
                    dst,
                    idx: index,
                    ptr_slot: p.ptr_slot,
                    len_slot: p.len_slot,
                    byte,
                    checked,
                });
            }
        }
        Op::SetIndex { collection, index, value }
        | Op::SetIndexUnchecked { collection, index, value } => {
            // Region-only elision (see `Index`); maps always check.
            let checked = !region || matches!(ops[i], Op::SetIndex { .. });
            let p = pins.get(&collection)?;
            // A Text pin is read-only — `Set item i of text` never tiers (the
            // kind gate already rejects it; bail defensively too).
            if p.elem == PinElem::TextBytes {
                return None;
            }
            if p.elem == PinElem::Map {
                micro.push(MicroOp::MapSet {
                    src: value,
                    key: index,
                    map_slot: p.vec_slot,
                    helper_addr: logos_rt_map_set_ii as usize as i64,
                });
            } else {
                let byte = p.elem == PinElem::Bool;
                micro.push(MicroOp::ArrStore {
                    src: value,
                    idx: index,
                    ptr_slot: p.ptr_slot,
                    len_slot: p.len_slot,
                    byte,
                    checked,
                });
            }
        }
        Op::Contains { dst, collection, value } => {
            let p = pins.get(&collection)?;
            if p.elem != PinElem::Map {
                return None;
            }
            micro.push(MicroOp::MapHas {
                dst,
                key: value,
                map_slot: p.vec_slot,
                helper_addr: logos_rt_map_has_i as usize as i64,
            });
        }
        Op::Length { dst, collection } => {
            let p = pins.get(&collection)?;
            micro.push(MicroOp::Move { dst, src: p.len_slot });
        }
        Op::ListPush { list, value } => {
            let p = pins.get(&list)?;
            let helper_addr = match p.elem {
                PinElem::Int => crate::logos_rt_push_i64 as usize as i64,
                PinElem::Float => crate::logos_rt_push_f64 as usize as i64,
                PinElem::Bool => crate::logos_rt_push_bool as usize as i64,
                // No push onto a map or a text accumulator.
                PinElem::Map | PinElem::TextBytes | PinElem::TextMut => return None,
            };
            micro.push(MicroOp::ArrPush {
                src: value,
                vec_slot: p.vec_slot,
                ptr_slot: p.ptr_slot,
                len_slot: p.len_slot,
                helper_addr,
                // A `Seq of Bool` buffer is 1-byte; the inline fast-path store
                // writes the boolean normalization. Int/Float are 8-byte raw.
                byte: p.elem == PinElem::Bool,
            });
        }
        Op::NewEmptyList { dst } => {
            // In-region `Let x be a new Seq` on a PINNED array: clear it in place
            // (truncate + reset the pinned length, keep the buffer) and reuse it,
            // rather than allocating fresh. An in-place mutation — the precise
            // region resumes over it soundly. Bails (`?`) for a non-pinned dst:
            // such a fresh list has no pin channel to clear here.
            let p = pins.get(&dst)?;
            // SOUNDNESS: reusing the buffer is only valid if the old contents
            // are dead. A `Move { src: dst }` that copies THIS list's handle to
            // another register ALIASES the buffer past the clear — reusing it
            // would wipe the alias's live data (knapsack's `Set prev to curr`,
            // where the cleared row is the one `prev` now points at). But the
            // compiler reuses register slots, so `dst` may also be a Move source
            // while it transiently holds an UNRELATED scalar (fannkuch's `perm`
            // slot is a `Sub` result moved out one op before the `NewEmptyList`
            // re-binds it to the list). Such a move reads the scalar, not the
            // list, and is irrelevant to buffer reuse. The kind flow already
            // resolves which value `dst` holds at each op: a move aliases the
            // list iff `dst` holds a LIST kind there (Mixed = a list joined with
            // a scalar across paths ⇒ conservatively an alias). A scalar kind
            // (or an unreachable/`Unknown` op) provably does not observe the
            // list, so clear-reuse stays sound. This is the same alias test the
            // VM makes dynamically (sole-ownership), lifted to the static kind
            // flow.
            let list_handle_escapes = ops.iter().enumerate().any(|(j, o)| {
                matches!(o, Op::Move { src, .. } if *src == dst)
                    && matches!(
                        kin[j].as_ref().map(|kk| kk[dst as usize]),
                        Some(
                            Kind::IntList
                                | Kind::FloatList
                                | Kind::BoolList
                                | Kind::IntMap
                                | Kind::Mixed
                        )
                    )
            });
            if list_handle_escapes {
                return None;
            }
            let helper_addr = match p.elem {
                PinElem::Int => crate::logos_rt_clear_i64 as usize as i64,
                PinElem::Float => crate::logos_rt_clear_f64 as usize as i64,
                PinElem::Bool => crate::logos_rt_clear_bool as usize as i64,
                // No empty-list clear for a map or text pin.
                PinElem::Map | PinElem::TextBytes | PinElem::TextMut => return None,
            };
            micro.push(MicroOp::ListClear {
                vec_slot: p.vec_slot,
                ptr_slot: p.ptr_slot,
                len_slot: p.len_slot,
                helper_addr,
            });
        }
        Op::CallBuiltin {
            dst,
            builtin: logicaffeine_compile::semantics::builtins::BuiltinId::Sqrt,
            args_start,
            arg_count,
        } => {
            if arg_count != 1 {
                return None;
            }
            let src = match kind_of(args_start) {
                Kind::Float => args_start,
                Kind::Int => {
                    micro.push(MicroOp::IntToFloat { dst: conv.0, src: args_start });
                    conv.0
                }
                _ => return None,
            };
            micro.push(MicroOp::SqrtF { dst, src });
        }
        Op::Call { dst, args_start, arg_count, func } => {
            let c = call_ctx?;
            // The callee windows at a DISJOINT offset past the limit slot —
            // the interpreter's overlapping windowing (callee base = caller
            // base + args_start) would let the callee's registers land on
            // the caller's conversion scratches and arena-limit slot, so
            // the caller's NEXT call would read a clobbered bound and
            // deopt. Args are staged across with explicit moves.
            let window = c.limit_slot + 1;
            // LEVER A — the pinned-argument self-call ABI. A DIRECT self-call
            // whose arguments are a contiguous all-scalar block lying wholly
            // below the callee window stages them inside ONE fused stencil
            // (`CallSelfCopy`) instead of `arg_count` separate frame-to-frame
            // `Move` pieces — the dispatch-reduction win on this piece-bound
            // engine. The copy is bit-identical to the Moves: the same callee
            // slots receive the same 8-byte values (Int/Bool/Float alike),
            // and the depth/deopt machinery is untouched. The disjointness
            // guard (`args_start + arg_count <= window`) is always true for
            // the function tier (`window = limit_slot + 1` sits past every
            // register), but is checked so the fallback is provably safe; any
            // list/map-kinded argument (which the all-scalar mode-A signature
            // never produces, but is guarded for soundness) keeps the Moves.
            let scalar_args = (0..arg_count).all(|j| {
                matches!(kind_of(args_start + j), Kind::Int | Kind::Bool | Kind::Float)
            });
            let fuse_self = func == c.self_fi
                && arg_count >= 1
                && args_start + arg_count <= window
                && scalar_args
                && std::env::var_os("LOGOS_NO_PINNED_ARGS").is_none();
            if fuse_self {
                // DIRECT self-call with the fused argument copy: the entry
                // pool word is patched with this chain's own base after
                // layout; the frame bound is static (self-recursion: the
                // callee frame is OUR OWN size).
                micro.push(MicroOp::CallSelfCopy {
                    dst,
                    args_start: window,
                    src_start: args_start,
                    arg_count,
                    depth_addr: c.depth_addr,
                    status_addr: c.status_addr,
                    limit_slot: c.limit_slot,
                    frame_size: (c.limit_slot as i64) + 1,
                });
                return Some(1);
            }
            for j in 0..arg_count {
                micro.push(MicroOp::Move { dst: window + j, src: args_start + j });
            }
            if func == c.self_fi {
                // DIRECT self-call: the entry pool word is patched with
                // this chain's own base after layout; the frame bound is
                // static (self-recursion: the callee frame is OUR OWN
                // size).
                micro.push(MicroOp::CallSelf {
                    dst,
                    args_start: window,
                    depth_addr: c.depth_addr,
                    status_addr: c.status_addr,
                    limit_slot: c.limit_slot,
                    frame_size: (c.limit_slot as i64) + 1,
                });
            } else {
                // Cross-function call: table dispatch through the CALLEE's
                // slot pair (the kind fixpoint already validated its
                // all-scalar signature); an unpublished entry deopts and
                // the boundary replay stays exact.
                micro.push(MicroOp::Call {
                    dst,
                    args_start: window,
                    table_addr: c.table.slot_addr(func as usize),
                    depth_addr: c.depth_addr,
                    status_addr: c.status_addr,
                    limit_slot: c.limit_slot,
                    depth_limit: c.depth_limit,
                });
            }
        }
        // In-region Return: set the flag, stash the value, leave via the
        // synthetic exit (usize::MAX is the out-of-region marker the remap
        // routes there).
        Op::Return { src } if region_return.is_some() => {
            let (flag_slot, value_slot) = region_return.unwrap();
            micro.push(MicroOp::LoadConst { dst: flag_slot, value: 1 });
            if matches!(kind_of(src), Kind::IntList | Kind::FloatList | Kind::BoolList) {
                // Lists return BY REGISTER: stash the register number; the
                // VM returns that register's current value.
                micro.push(MicroOp::LoadConst { dst: value_slot, value: src as i64 });
            } else {
                micro.push(MicroOp::Move { dst: value_slot, src });
            }
            fixups.push(micro.len());
            micro.push(MicroOp::Jump { target: usize::MAX });
        }
        Op::Return { src } if allow_return => micro.push(MicroOp::Return { src }),
        // Statically dead (gated by the caller); valid-shape placeholder.
        Op::ReturnNothing if allow_return => micro.push(MicroOp::Return { src: 0 }),
        // Region-entry length hoist: pure metadata, no native code. The VM
        // verifies it once at region entry; `adapt_region` collects it.
        Op::RegionBoundsGuard { .. } => {}
        _ => return None,
    }
    Some(1)
}

/// Translate a MAIN-LOOP REGION into the J2 subset.
///
/// The register roles of a recognized naive substring-search nest (the
/// string_search benchmark idiom): the count of OVERLAPPING `needle`
/// occurrences in `text` over the outer index range.
struct MemMemShape {
    text_reg: u16,
    needle_reg: u16,
    needle_len_reg: u16,
    i_reg: u16,
    count_reg: u16,
}

/// Recognize the EXACT naive substring-search nest the LOGOS compiler lowers
/// for `string_search`'s count loop, so the JIT can collapse the per-byte
/// nested-loop region to one [`MicroOp::MemMem`] call.
///
/// The matched outer-loop region (head at `ops[0]`) is, in bytecode:
///
/// ```text
///   Sub(t1 = textLen - needleLen); LoadConst(c=1); Add(bound = t1 + c)
///   LtEq(cmp = i <= bound); JumpIfFalse(cmp -> EXIT)
///   LoadConst(c=1); Move(match = c); LoadConst(c=0); Move(j = c)
///   Lt(cj = j < needleLen); JumpIfFalse(cj -> after_inner)
///     Add(t3 = i + j); IndexUnchecked(ch1 = text[t3])
///     LoadConst(c=1); Add(t4 = j + c); Index(ch2 = needle[t4])     // checked
///     NotEq(ne = ch1 != ch2); JumpIfFalse(ne -> cont)
///       LoadConst(c=0); Move(match = c); Move(t5 = needleLen); Move(j = t5)
///   cont:   LoadConst(c=1); AddAssign(j += c); Jump(-> inner head)
///   after_inner: LoadConst(c=1); Eq(eq = match == c); JumpIfFalse(eq -> skip)
///     LoadConst(c=1); AddAssign(count += c)
///   skip:   LoadConst(c=1); AddAssign(i += c); Jump(-> outer head)
/// ```
///
/// Recognition is STRICT — the shape, the operand wiring (`i`/`j`/`needleLen`/
/// `count`/`text`/`needle` register threading), the two `Index` collections, the
/// jump topology, and the `1`/`0` constants are all checked — so the helper is
/// only ever substituted when it is bit-identical to this nest. Any deviation
/// returns `None` and the region falls back to the per-byte tiered loop.
/// `LOGOS_MEMMEM=0` disables the recognizer entirely.
fn match_memmem_nest(
    ops: &[Op],
    head_pc: usize,
    exit_pc: usize,
    constants: &[Constant],
) -> Option<MemMemShape> {
    if std::env::var("LOGOS_MEMMEM").as_deref() == Ok("0") {
        return None;
    }
    // The template is exactly 33 ops (the outer back-edge is the last).
    if ops.len() != 33 {
        return None;
    }
    let is_const = |i: usize, want: i64| -> bool {
        matches!(ops.get(i), Some(Op::LoadConst { idx, .. })
            if matches!(constants.get(*idx as usize), Some(Constant::Int(v)) if *v == want))
    };
    let const_dst = |i: usize| -> Option<u16> {
        match ops.get(i) {
            Some(Op::LoadConst { dst, .. }) => Some(*dst),
            _ => None,
        }
    };
    // The region-relative targets the jumps must hit.
    let inner_head = head_pc + 9; // op [38] Lt(j < needleLen)
    let after_inner = head_pc + 25; // op [54]
    let cont = head_pc + 22; // op [51]
    let skip = head_pc + 30; // op [59]

    // [0] Sub(t1 = textLen - needleLen)
    let Op::Sub { dst: t1, lhs: text_len, rhs: needle_len } = ops[0] else { return None };
    // [1] LoadConst(c1 = 1); [2] Add(bound = t1 + c1)
    if !is_const(1, 1) {
        return None;
    }
    let c1 = const_dst(1)?;
    let Op::Add { dst: bound, lhs, rhs } = ops[2] else { return None };
    if lhs != t1 || rhs != c1 {
        return None;
    }
    // [3] LtEq(cmp = i <= bound); [4] JumpIfFalse(cmp -> EXIT)
    let Op::LtEq { dst: cmp, lhs: i_reg, rhs } = ops[3] else { return None };
    if rhs != bound {
        return None;
    }
    let Op::JumpIfFalse { cond, target } = ops[4] else { return None };
    if cond != cmp || target != exit_pc {
        return None;
    }
    // [5] LoadConst(=1); [6] Move(match = it)
    if !is_const(5, 1) {
        return None;
    }
    let c5 = const_dst(5)?;
    let Op::Move { dst: match_reg, src } = ops[6] else { return None };
    if src != c5 {
        return None;
    }
    // [7] LoadConst(=0); [8] Move(j = it)
    if !is_const(7, 0) {
        return None;
    }
    let c7 = const_dst(7)?;
    let Op::Move { dst: j_reg, src } = ops[8] else { return None };
    if src != c7 {
        return None;
    }
    // [9] Lt(cj = j < needleLen); [10] JumpIfFalse(cj -> after_inner)
    let Op::Lt { dst: cj, lhs, rhs } = ops[9] else { return None };
    if lhs != j_reg || rhs != needle_len {
        return None;
    }
    let Op::JumpIfFalse { cond, target } = ops[10] else { return None };
    if cond != cj || target != after_inner {
        return None;
    }
    // [11] Add(t3 = i + j); [12] IndexUnchecked(ch1 = text[t3])
    let Op::Add { dst: t3, lhs, rhs } = ops[11] else { return None };
    if lhs != i_reg || rhs != j_reg {
        return None;
    }
    let Op::IndexUnchecked { dst: ch1, collection: text_reg, index } = ops[12] else {
        return None;
    };
    if index != t3 {
        return None;
    }
    // [13] LoadConst(=1); [14] Add(t4 = j + it); [15] Index(ch2 = needle[t4])
    if !is_const(13, 1) {
        return None;
    }
    let c13 = const_dst(13)?;
    let Op::Add { dst: t4, lhs, rhs } = ops[14] else { return None };
    if lhs != j_reg || rhs != c13 {
        return None;
    }
    let Op::Index { dst: ch2, collection: needle_reg, index } = ops[15] else { return None };
    if index != t4 {
        return None;
    }
    // [16] NotEq(ne = ch1 != ch2); [17] JumpIfFalse(ne -> cont)
    let Op::NotEq { dst: ne, lhs, rhs } = ops[16] else { return None };
    if !((lhs == ch1 && rhs == ch2) || (lhs == ch2 && rhs == ch1)) {
        return None;
    }
    let Op::JumpIfFalse { cond, target } = ops[17] else { return None };
    if cond != ne || target != cont {
        return None;
    }
    // [18] LoadConst(=0); [19] Move(match = it)
    if !is_const(18, 0) {
        return None;
    }
    let c18 = const_dst(18)?;
    let Op::Move { dst, src } = ops[19] else { return None };
    if dst != match_reg || src != c18 {
        return None;
    }
    // [20] Move(t5 = needleLen); [21] Move(j = t5)
    let Op::Move { dst: t5, src } = ops[20] else { return None };
    if src != needle_len {
        return None;
    }
    let Op::Move { dst, src } = ops[21] else { return None };
    if dst != j_reg || src != t5 {
        return None;
    }
    // [22] cont: LoadConst(=1); [23] AddAssign(j += it); [24] Jump(-> inner head)
    if !is_const(22, 1) {
        return None;
    }
    let c22 = const_dst(22)?;
    let Op::AddAssign { dst, src } = ops[23] else { return None };
    if dst != j_reg || src != c22 {
        return None;
    }
    let Op::Jump { target } = ops[24] else { return None };
    if target != inner_head {
        return None;
    }
    // [25] after_inner: LoadConst(=1); [26] Eq(eq = match == it); [27] JumpIfFalse(eq -> skip)
    if !is_const(25, 1) {
        return None;
    }
    let c25 = const_dst(25)?;
    let Op::Eq { dst: eq, lhs, rhs } = ops[26] else { return None };
    if !((lhs == match_reg && rhs == c25) || (lhs == c25 && rhs == match_reg)) {
        return None;
    }
    let Op::JumpIfFalse { cond, target } = ops[27] else { return None };
    if cond != eq || target != skip {
        return None;
    }
    // [28] LoadConst(=1); [29] AddAssign(count += it)
    if !is_const(28, 1) {
        return None;
    }
    let c28 = const_dst(28)?;
    let Op::AddAssign { dst: count_reg, src } = ops[29] else { return None };
    if src != c28 {
        return None;
    }
    // [30] skip: LoadConst(=1); [31] AddAssign(i += it); [32] Jump(-> outer head)
    if !is_const(30, 1) {
        return None;
    }
    let c30 = const_dst(30)?;
    let Op::AddAssign { dst, src } = ops[31] else { return None };
    if dst != i_reg || src != c30 {
        return None;
    }
    let Op::Jump { target } = ops[32] else { return None };
    if target != head_pc {
        return None;
    }
    // text and needle must be DISTINCT collections; the count accumulator must
    // be distinct from the loop induction `i`.
    let _ = text_len;
    if text_reg == needle_reg || count_reg == i_reg {
        return None;
    }
    Some(MemMemShape {
        text_reg,
        needle_reg,
        needle_len_reg: needle_len,
        i_reg,
        count_reg,
    })
}

/// Region contract: `ops[0]` is the loop head; the slice ends at the
/// back-edge; every jump out targets `exit_pc` (mapped to a synthetic
/// terminal). The guard set is the LIVE-IN set at the head — computed by
/// textbook backward liveness over the region's CFG — so any slot whose
/// incoming value can be observed is Int-guarded and copied in, while
/// incoming-dead slots (loop scratches, comparison temporaries anywhere in
/// the body) need no guard. Writes are re-boxed by inferred kind.
#[allow(clippy::type_complexity)]
fn adapt_region(
    ops: &[Op],
    head_pc: usize,
    exit_pc: usize,
    constants: &[Constant],
    register_count: u16,
    named: &[bool],
    observed: &[ObservedKind],
    ctx: &NativeCtx,
    callees: &[logicaffeine_compile::vm::CalleeSig],
) -> Option<(
    Vec<MicroOp>,
    usize,
    Vec<(u16, SlotKind)>,
    Vec<u16>,
    Vec<(u16, SlotKind)>,
    Vec<ArrayPin>,
    Option<logicaffeine_compile::vm::RegionReturn>,
    Vec<HoistGuard>,
    // Precise-deopt tables (Some ⇒ this region resumes AT the faulting op on a
    // side exit, re-boxing each register by kind, instead of replaying from the
    // head — required for a sound push+SetIndex region).
    Option<RegionPrecise>,
)> {
    // SOUNDNESS (Bug Report #1, BUG-001): a region runs under a
    // discard-and-replay-from-head deopt contract that is sound only for
    // replay-idempotent effects. `ListPush` APPENDS — not idempotent on its own
    // — but the VM's region deopt path now rolls every pinned buffer back to its
    // entry length before replaying (see machine.rs), so a push followed by a
    // PURE side-exit (Div/Mod div-by-zero, Index/Contains out-of-bounds or
    // fast-lane miss — all read-only or arithmetic) replays cleanly. What stays
    // unsound is push coexisting with another NON-idempotent effect whose replay
    // the truncate cannot undo: an in-place `SetIndex`/`SetIndexUnchecked` write
    // (a read-modify-write would double-apply) or a `Call` (arbitrary effects).
    // Those still disqualify the region; it falls back to the always-correct
    // bytecode interpreter.
    if std::env::var("LOGOS_DUMP_REGION").ok().and_then(|v| v.parse::<usize>().ok()) == Some(head_pc) {
        eprintln!("=== REGION head_pc={head_pc} ops ({}) ===", ops.len());
        for (i, op) in ops.iter().enumerate() {
            eprintln!("  [{}] {op:?}", head_pc + i);
        }
    }
    let has_list_push = ops.iter().any(|op| matches!(op, Op::ListPush { .. }));
    let has_setindex = ops
        .iter()
        .any(|op| matches!(op, Op::SetIndex { .. } | Op::SetIndexUnchecked { .. }));
    let has_call_op = ops.iter().any(|op| matches!(op, Op::Call { .. }));
    // A `ListPush` coexisting with an in-place `SetIndex` or a `Call` is NOT
    // replay-safe under the classic discard-replay-from-head deopt (truncate
    // rolls the pushes back but the in-place write persists, so a self-gated
    // re-push is skipped — the BFS lost-node bug). It can still tier up with
    // PRECISE region deopt (resume AT the faulting op, no replay): sound for any
    // pattern. Precise materialization re-boxes the frame's scalars, so it is
    // admitted only for an ALL-INT region with NO call (a call frame would need
    // the full function-precise walk). A push beside a call is therefore always
    // disqualified; a push beside a SetIndex is admitted here and re-checked for
    // all-int after lowering (where the precise `deopt_codes` are built).
    let needs_precise = has_list_push && (has_setindex || has_call_op);
    if needs_precise && has_call_op {
        return None;
    }

    // Backward liveness: out-of-region exits contribute empty live-out —
    // exit-observable slots flow through the WRITE-BACK, never the frame.
    let nregs = register_count as usize + 2;
    let in_region = |t: usize| (head_pc..head_pc + ops.len()).contains(&t);
    let Some(live) = liveness(ops, head_pc, nregs) else {
        if std::env::var_os("LOGOS_RDIAG").is_some() {
            eprintln!("RDIAG-BAIL head_pc={head_pc} liveness");
        }
        return None;
    };

    // Guard = live-in at the head; touched = every use or def; free =
    // touched − guard; writes = every def (re-boxed by kind on completion).
    let mut guard: Vec<u16> = Vec::new();
    let mut free: Vec<u16> = Vec::new();
    let mut writes: Vec<u16> = Vec::new();
    let mut touched: Vec<u16> = Vec::new();
    for op in ops {
        let Some((uses, defs)) = region_use_def(op) else {
            if std::env::var_os("LOGOS_RDIAG").is_some() {
                eprintln!("RDIAG-BAIL head_pc={head_pc} use_def-unsupported op={op:?}");
            }
            return None;
        };
        for d in defs {
            if !writes.contains(&d) {
                writes.push(d);
            }
            if !touched.contains(&d) {
                touched.push(d);
            }
        }
        for u in uses {
            if !touched.contains(&u) {
                touched.push(u);
            }
        }
    }
    // Array pins: every register used as an Index/SetIndex/Length collection
    // must hold an UNBOXED list right now (the speculation the entry guard
    // re-checks); each gets a pinned (pointer, length) slot pair past the
    // conversion scratches.
    let mut pin_regs: Vec<(u16, PinElem)> = Vec::new();
    for op in ops {
        let coll = match *op {
            Op::Index { collection, .. }
            | Op::IndexUnchecked { collection, .. }
            | Op::SetIndex { collection, .. }
            | Op::SetIndexUnchecked { collection, .. }
            | Op::Length { collection, .. }
            | Op::Contains { collection, .. } => Some(collection),
            Op::ListPush { list, .. } => Some(list),
            _ => None,
        };
        if let Some(c) = coll {
            let elem = match observed.get(c as usize)? {
                ObservedKind::IntList => PinElem::Int,
                ObservedKind::FloatList => PinElem::Float,
                ObservedKind::BoolList => PinElem::Bool,
                ObservedKind::Map => PinElem::Map,
                ObservedKind::TextBytes => {
                    // A Text-as-bytes pin is read-only: the byte buffer pointer
                    // is captured at entry and the `Rc<String>` is never written
                    // in place. A region that REASSIGNS the pinned register (a
                    // `Move`/rebind into `c`, or string growth via `Op::Concat`
                    // — already out of `region_use_def` so it bails on its own)
                    // would leave a stale buffer pointer. Bail if the pinned
                    // register is ever a def in this region.
                    if writes.contains(&c) {
                        if std::env::var_os("LOGOS_RDIAG").is_some() {
                            eprintln!("RDIAG-BAIL head_pc={head_pc} textbytes-pin-reassigned reg={c}");
                        }
                        return None;
                    }
                    PinElem::TextBytes
                }
                _ => {
                    if std::env::var_os("LOGOS_RDIAG").is_some() {
                        eprintln!("RDIAG-BAIL head_pc={head_pc} non-list-collection reg={c}");
                    }
                    return None;
                }
            };
            if !pin_regs.iter().any(|&(r, _)| r == c) {
                pin_regs.push((c, elem));
            }
        }
    }
    // MUTABLE-TEXT accumulator (`Set text to text + ch`): a register that is the
    // `dst` of an `AddAssign` and is OBSERVED as an ASCII Text rides the
    // `TextMut` pin channel (a `*mut Value` handle to its VM register cell), so
    // the append lowers to a `StrAppend` helper call instead of bailing the
    // region on a non-numeric `AddAssign`. Emitted ONLY for the contiguous
    // regalloc backend (the per-piece stencil tier has no `StrAppend` lowering
    // and declines it); when regalloc is off, a `text + ch` region falls back to
    // bytecode exactly as before. A register that is ALSO used as a collection
    // (already pinned above) is not re-pinned as TextMut.
    let strappend_on = logicaffeine_forge::regalloc::regalloc_enabled();
    if strappend_on {
        for op in ops {
            if let Op::AddAssign { dst, .. } = *op {
                if matches!(observed.get(dst as usize), Some(ObservedKind::TextBytes))
                    && !pin_regs.iter().any(|&(r, _)| r == dst)
                {
                    pin_regs.push((dst, PinElem::TextMut));
                }
            }
        }
    }
    // LEVER B (ON by default; kill-switch `LOGOS_LEVERB=0`): a list that flows
    // into a list-parameter CALL but is never directly Indexed here (e.g.
    // `arr = siftDown(arr, …)`) is still a pinnable array — pin every touched
    // slot observed as a list, so it rides the array channel rather than the
    // scalar guard set (which has no `slot_kind` for a list and would bail).
    // Validated across ~2180 tests (878 JIT + 1301 e2e) + the deopt-rollback
    // RED/GREEN; regions calling list-param functions now tier (heap_sort,
    // mergesort, any worklist-via-helper) with discard-replay soundness from
    // the array snapshot/rollback.
    let leverb_on = std::env::var("LOGOS_LEVERB").as_deref() != Ok("0");
    // Does this region CALL a list-parameter function? Only then must a list
    // that's merely passed through (not Indexed here) be pinned. Pinning EVERY
    // touched list would spuriously re-pin lists in SCALAR-call regions and
    // reintroduce the float-pin / mem-form clobber (spectral_norm), so this is
    // strictly scoped to list-param-call regions.
    let has_list_param_call = leverb_on
        && ops.iter().any(|op| match *op {
            Op::Call { func, .. } => callees.get(func as usize).is_some_and(|s| {
                s.param_kinds.iter().any(|p| matches!(p, Some(ParamKind::List(_))))
            }),
            _ => false,
        });
    if has_list_param_call {
        for &r in &touched {
            if pin_regs.iter().any(|&(p, _)| p == r) {
                continue;
            }
            let elem = match observed.get(r as usize) {
                Some(ObservedKind::IntList) => Some(PinElem::Int),
                Some(ObservedKind::FloatList) => Some(PinElem::Float),
                Some(ObservedKind::BoolList) => Some(PinElem::Bool),
                _ => None,
            };
            if let Some(e) = elem {
                pin_regs.push((r, e));
            }
        }
    }
    // In-region Returns: the value's kind must agree across every return
    // site (re-boxed by SlotKind at the boundary).
    let has_return = ops.iter().any(|op| matches!(op, Op::Return { .. }));
    if ops.iter().any(|op| matches!(op, Op::ReturnNothing)) {
        if std::env::var_os("LOGOS_RDIAG").is_some() {
            eprintln!("RDIAG-BAIL head_pc={head_pc} return-nothing");
        }
        return None;
    }
    // An in-place `SetIndex` write is NOT replay-idempotent (a read-modify-write
    // or swap double-applies), so a buffer it targets must be snapshotted on
    // region entry and restored on a classic replay-from-head `Deopt`. That cost
    // is only owed when the region can ACTUALLY take a recoverable side exit:
    // a checked list `Index` (bounds), ANY map `Index`/`Contains` (int-lane
    // miss), `Div`/`Mod` (div-by-zero), or a `Call` (callee deopt). Unchecked
    // list reads, pure arithmetic, and control flow never side-exit — so a
    // deopt-free swap loop (bubble/quick sort) pays nothing. A PRECISE region
    // resumes AT the faulting op (no replay), so it never needs the snapshot.
    let is_map = |c: u16| matches!(observed.get(c as usize), Some(ObservedKind::Map));
    let region_has_deopt_source = ops.iter().any(|op| match *op {
        Op::Div { .. } | Op::Mod { .. } | Op::Call { .. } | Op::Index { .. } => true,
        Op::IndexUnchecked { collection, .. } | Op::Contains { collection, .. } => is_map(collection),
        _ => false,
    });
    let is_setindexed = |reg: u16| {
        ops.iter().any(|op| {
            matches!(*op,
                Op::SetIndex { collection, .. } | Op::SetIndexUnchecked { collection, .. }
                    if collection == reg)
        })
    };
    // A pinned mutable-Text accumulator is grown directly in its VM register cell
    // by the `StrAppend` helper — like an in-place `SetIndex`, that growth is NOT
    // replay-idempotent (a classic replay-from-head would double-append), so its
    // entry `Value` must be snapshotted and restored on a classic deopt.
    let is_text_mut = |reg: u16| pin_regs.iter().any(|&(r, e)| r == reg && e == PinElem::TextMut);
    // LEVER B: a region that CALLS a list-parameter function (`has_list_param_call`,
    // computed above) lets the callee mutate the passed buffer IN PLACE. The
    // region can't see which pinned array flows into the call (it's staged through
    // Moves into the arg window), so conservatively snapshot EVERY pinned array in
    // such a region — its contents must roll back on a classic deopt like a SetIndex.
    let needs_snapshot = !needs_precise && region_has_deopt_source;
    let pin_base = register_count + 2;
    let array_pins: Vec<ArrayPin> = pin_regs
        .iter()
        .enumerate()
        .map(|(k, &(reg, elem))| ArrayPin {
            reg,
            vec_slot: pin_base + 3 * k as u16,
            ptr_slot: pin_base + 3 * k as u16 + 1,
            len_slot: pin_base + 3 * k as u16 + 2,
            elem,
            mutated: needs_snapshot && (is_setindexed(reg) || has_list_param_call || is_text_mut(reg)),
        })
        .collect();
    // Collect the loop's region-entry bounds hoists, resolving each guarded
    // array to its pinned length slot. The guarded array MUST be pinned (it is
    // — the compiler only emits a guard for an array it also accesses); a
    // guard whose array is not pinned is dropped (its accesses then stay
    // checked, which is safe).
    let mut hoist_guards: Vec<HoistGuard> = Vec::new();
    for op in ops {
        if let Op::RegionBoundsGuard { array, bound, iv, add_max, add_min } = *op {
            // The guarded array must be pinned (so its length is available to
            // check). If not, bail the region entirely — never run the
            // speculative accesses without their guard.
            let Some(pin) = array_pins.iter().find(|p| p.reg == array) else {
                if std::env::var_os("LOGOS_RDIAG").is_some() {
                    eprintln!("RDIAG-BAIL head_pc={head_pc} bounds-guard-array-not-pinned");
                }
                return None;
            };
            hoist_guards.push(HoistGuard {
                len_slot: pin.len_slot,
                bound_reg: bound,
                iv_reg: iv,
                add_max,
                add_min,
            });
        }
    }
    let pins: std::collections::HashMap<u16, PinSlots> = array_pins
        .iter()
        .map(|p| {
            (
                p.reg,
                PinSlots {
                    vec_slot: p.vec_slot,
                    ptr_slot: p.ptr_slot,
                    len_slot: p.len_slot,
                    elem: p.elem,
                },
            )
        })
        .collect();

    // Must-write: slots DEFINITELY written on every path from the head to
    // op i (state holding before i executes). Forward dataflow with
    // intersection at merges. A written slot that is not must-written at
    // some exit edge is only written on SOME completing paths — bytecode
    // would leave its old value on the others, so the region must copy the
    // old value IN (guard it) for write-back to restore when the taken path
    // skipped the write. (The primes `isPrime`-flag shape.)
    let mut mw: Vec<Option<Vec<bool>>> = vec![None; ops.len()];
    mw[0] = Some(vec![false; nregs]);
    let mut work: Vec<usize> = vec![0];
    while let Some(i) = work.pop() {
        let Some(cur) = mw[i].clone() else { continue };
        let mut out = cur;
        let Some((_, defs)) = region_use_def(&ops[i]) else {
            if std::env::var_os("LOGOS_RDIAG").is_some() {
                eprintln!("RDIAG-BAIL head_pc={head_pc} mw-use_def-unsupported op={:?}", ops[i]);
            }
            return None;
        };
        for d in defs {
            out[d as usize] = true;
        }
        let mut succs: Vec<usize> = Vec::with_capacity(2);
        match ops[i] {
            Op::Jump { target } => {
                if in_region(target) {
                    succs.push(target - head_pc);
                }
            }
            Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. }
            | Op::JumpIfInt { target, .. } => {
                if i + 1 < ops.len() {
                    succs.push(i + 1);
                }
                if in_region(target) {
                    succs.push(target - head_pc);
                }
            }
            _ => {
                if i + 1 < ops.len() {
                    succs.push(i + 1);
                }
            }
        }
        for s in succs {
            match &mut mw[s] {
                slot @ None => {
                    *slot = Some(out.clone());
                    work.push(s);
                }
                Some(prev) => {
                    let mut changed = false;
                    for (a, &b) in prev.iter_mut().zip(&out) {
                        if *a && !b {
                            *a = false;
                            changed = true;
                        }
                    }
                    if changed {
                        work.push(s);
                    }
                }
            }
        }
    }
    // Intersection of must-write across every reachable exit edge.
    let mut exit_must: Vec<bool> = vec![true; nregs];
    let mut saw_exit = false;
    for (i, op) in ops.iter().enumerate() {
        let Some(m) = &mw[i] else { continue };
        let exits_here = match *op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. }
            | Op::JumpIfInt { target, .. } => !in_region(target),
            Op::Return { .. } => true,
            _ => false,
        };
        if exits_here {
            saw_exit = true;
            for (e, &b) in exit_must.iter_mut().zip(m) {
                *e &= b;
            }
        }
    }
    if !saw_exit {
        exit_must.iter_mut().for_each(|e| *e = false);
    }

    // Observability policy: only NAMED slots can be read after the region
    // (scratches are statement-local and dead at every statement boundary by
    // the compiler's allocation discipline). A named slot written only on
    // SOME completing paths is also copied IN, so write-back restores the
    // pre-region value when the taken path skipped the write.
    let is_named = |r: u16| named.get(r as usize).copied().unwrap_or(false);
    for &r in &touched {
        if pins.contains_key(&r) {
            continue; // pinned arrays ride their own channel
        }
        let conditionally_written =
            is_named(r) && writes.contains(&r) && !exit_must[r as usize];
        if live[0][r as usize] || conditionally_written {
            guard.push(r);
        } else {
            free.push(r);
        }
    }

    // Flow-sensitive kinds: guarded (live-in or conditionally-written) slots
    // enter with the kind the VM OBSERVED at this hot crossing — exactly what
    // the runtime guard will re-check on every entry — free slots as Unknown.
    // A guard slot holding something the frame can't carry kills the region.
    let mut entry = vec![Kind::Unknown; nregs];
    let mut guard_kinds: Vec<(u16, SlotKind)> = Vec::with_capacity(guard.len());
    for &g in &guard {
        let k = observed_kind(*observed.get(g as usize)?);
        let Some(sk) = k.slot_kind() else {
            if std::env::var_os("LOGOS_RDIAG").is_some() {
                eprintln!("RDIAG-BAIL head_pc={head_pc} guard-slot-no-slotkind g={g} kind={k:?}");
            }
            return None;
        };
        entry[g as usize] = k;
        guard_kinds.push((g, sk));
    }
    for &(reg, elem) in &pin_regs {
        entry[reg as usize] = match elem {
            PinElem::Int => Kind::IntList,
            PinElem::Float => Kind::FloatList,
            PinElem::Bool => Kind::BoolList,
            PinElem::Map => Kind::IntMap,
            PinElem::TextBytes => Kind::TextBytes,
            PinElem::TextMut => Kind::TextMut,
        };
    }
    // LEVER B: admit a region that CALLS a list-parameter function. Sound only
    // for a region-safe callee (never reallocs a list param) whose list args are
    // pinned arrays the region snapshots; the region also releases its array
    // borrows around the call so the callee can borrow them. Gated ON by
    // `LOGOS_LEVERB=1` while the call path is being hardened (default rejects,
    // exactly as before). `leverb_on` is bound above (at the pin-regs stage).
    let resolve = |fi: u16| -> Option<(Kind, Vec<Kind>)> {
        let sig = callees.get(fi as usize)?;
        let list_kind = |e: PinElem| -> Option<Kind> {
            match e {
                PinElem::Int => Some(Kind::IntList),
                PinElem::Float => Some(Kind::FloatList),
                PinElem::Bool => Some(Kind::BoolList),
                PinElem::Map | PinElem::TextBytes | PinElem::TextMut => None,
            }
        };
        let ret = match sig.ret {
            Some(SlotKind::Int) => Kind::Int,
            Some(SlotKind::Bool) => Kind::Bool,
            Some(SlotKind::Float) => Kind::Float,
            // A LIST (or void) return: admitted only when it aliases a list
            // parameter (no fresh buffer ⇒ no stale pin) of a stable callee.
            None => {
                if !leverb_on || !sig.list_params_stable || !sig.returns_list_param {
                    if std::env::var_os("LOGOS_RDIAG").is_some() {
                        eprintln!("RDIAG-BAIL head_pc={head_pc} list-return-call");
                    }
                    return None;
                }
                let elem = sig.param_kinds.iter().find_map(|pk| match pk {
                    Some(ParamKind::List(e)) => Some(*e),
                    _ => None,
                })?;
                list_kind(elem)?
            }
        };
        let mut ps = Vec::with_capacity(sig.param_kinds.len());
        for pk in &sig.param_kinds {
            match (*pk)? {
                ParamKind::Scalar(SlotKind::Int) => ps.push(Kind::Int),
                ParamKind::Scalar(SlotKind::Bool) => ps.push(Kind::Bool),
                ParamKind::Scalar(SlotKind::Float) => ps.push(Kind::Float),
                // A list param: the callee mutates the shared buffer in place.
                // Sound iff the callee is stable (no realloc) — the pinned arg's
                // contents ride the deopt snapshot/rollback, and the region
                // releases its borrow so the callee can access it.
                ParamKind::List(elem) => {
                    if !leverb_on || !sig.list_params_stable {
                        if std::env::var_os("LOGOS_RDIAG").is_some() {
                            eprintln!("RDIAG-BAIL head_pc={head_pc} list-param-call");
                        }
                        return None;
                    }
                    ps.push(list_kind(elem)?);
                }
            }
        }
        Some((ret, ps))
    };
    let Some(kin) = kind_flow(ops, head_pc, constants, entry, Some(&resolve)) else {
        if std::env::var_os("LOGOS_RDIAG").is_some() {
            eprintln!("RDIAG-BAIL head_pc={head_pc} kind_flow");
        }
        return None;
    };

    // Write-back kinds come from the EXIT EDGES: for each written slot, join
    // its kind at every reachable out-of-region jump. Mixed (exits disagree)
    // bails; Unknown means no completing path writes it — leave the VM
    // register untouched (exactly what bytecode reaching the exit would do).
    let mut exit_kind: Vec<Kind> = vec![Kind::Unknown; nregs];
    for (i, op) in ops.iter().enumerate() {
        let Some(k) = &kin[i] else { continue };
        let exits_here = match *op {
            Op::Jump { target } => !in_region(target),
            Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. }
            | Op::JumpIfInt { target, .. } => !in_region(target),
            // An in-region Return leaves the loop too — its named writes
            // flow through the same write-back.
            Op::Return { .. } => true,
            _ => false,
        };
        if exits_here {
            for (e, kk) in exit_kind.iter_mut().zip(k) {
                *e = join(*e, *kk);
            }
        }
    }
    let mut write_set: Vec<(u16, SlotKind)> = Vec::new();
    for &r in &writes {
        // Scratches are unobservable after the region — never written back,
        // and their exit kind (often Mixed: the compiler reuses them across
        // Int and Bool statements) is irrelevant.
        if !is_named(r) {
            continue;
        }
        // LEVER B: a pinned array REBOUND to a return-aliased call result
        // (`arr = siftDown(arr, …)`) carries a list exit-kind, but it rides the
        // pin channel, not the scalar write-back: the callee returns its own
        // argument (`returns_list_param`), so the VM register already holds the
        // correct Rc and no write-back is owed. Skip it (else `slot_kind()` on a
        // list kind below would bail the region).
        if leverb_on && array_pins.iter().any(|a| a.reg == r) {
            continue;
        }
        match exit_kind[r as usize] {
            Kind::Unknown => {}
            // A Mixed exit-kind means the compiler REUSED this register across
            // an Int and a Float on different paths — it is never a single
            // typed observable variable. If it is NOT loop-carried (not live-in
            // at the region head, per the region's own backward liveness) it is
            // a per-iteration scratch, dead at the region boundary: skip its
            // write-back rather than decline the whole region (the VM's stale
            // register value is unobservable — the scratch is recomputed every
            // iteration). A genuinely loop-carried Mixed slot still bails.
            Kind::Mixed => {
                if live[0][r as usize] {
                    if std::env::var_os("LOGOS_RDIAG").is_some() {
                        eprintln!("RDIAG-BAIL head_pc={head_pc} mixed-write-set slot={r}");
                    }
                    return None;
                }
            }
            // A text kind has no scalar write-back representation. The pinned
            // `TextMut` accumulator already rode the `array_pins` skip above; a
            // `TextByte`/`TextConst` here is a per-iteration char/const scratch
            // (the `ch` selection, a literal load) — dead at the region boundary
            // when NOT loop-carried, so skip its write-back exactly like a
            // non-loop-carried Mixed scratch. A genuinely loop-carried text
            // (live-in at the head, e.g. an unpinned Text variable) still bails.
            Kind::TextByte | Kind::TextConst | Kind::TextMut => {
                if live[0][r as usize] {
                    if std::env::var_os("LOGOS_RDIAG").is_some() {
                        eprintln!("RDIAG-BAIL head_pc={head_pc} text-write-set-loop-carried slot={r} kind={:?}", exit_kind[r as usize]);
                    }
                    return None;
                }
            }
            k => {
                let Some(sk) = k.slot_kind() else {
                    if std::env::var_os("LOGOS_RDIAG").is_some() {
                        eprintln!("RDIAG-BAIL head_pc={head_pc} write-set-no-slotkind r={r} kind={k:?}");
                    }
                    return None;
                };
                write_set.push((r, sk));
            }
        }
    }

    let ret_base = pin_base + 3 * array_pins.len() as u16;
    let region_return_slots = has_return.then_some((ret_base, ret_base + 1));

    // The agreed return kind across every reachable Return site.
    use logicaffeine_compile::vm::RegionReturnKind;
    let mut ret_kind: Option<RegionReturnKind> = None;
    if has_return {
        for (i, op) in ops.iter().enumerate() {
            let Some(k) = &kin[i] else { continue };
            if let Op::Return { src } = *op {
                let rk = match k[src as usize] {
                    Kind::IntList | Kind::FloatList | Kind::BoolList => {
                        RegionReturnKind::Register
                    }
                    other => RegionReturnKind::Slot(other.slot_kind()?),
                };
                match ret_kind {
                    None => ret_kind = Some(rk),
                    Some(prev) if prev == rk => {}
                    _ => return None,
                }
            }
        }
        // A region whose every Return is unreachable: treat as returnless.
        if ret_kind.is_none() {
            return None;
        }
    }
    let region_return = match (region_return_slots, ret_kind) {
        (Some((flag, val)), Some(kind)) => {
            Some(logicaffeine_compile::vm::RegionReturn { flag_slot: flag, value_slot: val, kind })
        }
        _ => None,
    };

    // Emit: out-of-region jumps go to the synthetic exit terminal. Adjacent
    // cmp + conditional-jump pairs whose scratch is dead FUSE into one
    // Branch micro-op (both VM pcs map to it). Two scratch slots past the
    // register file carry Int→Float promotions; two more per pinned array
    // carry its buffer pointer and length.
    let has_calls = ops.iter().any(|op| matches!(op, Op::Call { .. }));
    let frame_size = register_count as usize
        + 2
        + 3 * array_pins.len()
        + if has_return { 2 } else { 0 }
        + if has_calls { 1 } else { 0 };
    // The arena-limit slot is LAST; callees window immediately past the
    // frame (region frames get real arena headroom when they call).
    let limit_slot = (frame_size - 1) as u16;
    let jump_targets: std::collections::HashSet<usize> = ops
        .iter()
        .filter_map(|op| match *op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. }
            | Op::JumpIfInt { target, .. } => Some(target),
            _ => None,
        })
        .collect();
    let scratch_ok = |r: u16| !is_named(r);
    let conv = (register_count, register_count + 1);
    let mut micro: Vec<MicroOp> = Vec::with_capacity(ops.len());
    let mut pc_to_micro: Vec<usize> = Vec::with_capacity(ops.len());
    let mut fixups: Vec<usize> = Vec::new();

    // NAIVE SUBSTRING-SEARCH COLLAPSE (string_search): if the WHOLE outer-loop
    // region is the exact naive-search nest over two pinned ASCII `TextBytes`
    // buffers (text + needle), replace the per-byte nested loop with ONE
    // `MemMem` helper call. The helper counts overlapping matches, adds them to
    // `count`, and advances `i` to the loop's exit value — bit-identical to the
    // nest, with a deopt exit for the one recoverable disagreement (a checked
    // needle index past the needle buffer). Gated on: no return, no precise
    // requirement, both `text` and `needle` pinned as `TextBytes`, and `count`/
    // `i`/`needleLen` carried as plain Int write-back/guard slots.
    if !has_return && !needs_precise {
        if let Some(shape) = match_memmem_nest(ops, head_pc, exit_pc, constants) {
            let text_pin = pins.get(&shape.text_reg);
            let needle_pin = pins.get(&shape.needle_reg);
            if let (Some(tp), Some(np)) = (text_pin, needle_pin) {
                let both_text = tp.elem == PinElem::TextBytes && np.elem == PinElem::TextBytes;
                // The accumulators must be plain Int write-back slots (the nest's
                // `count`/`i`); `needleLen` must be a guarded Int input. If the
                // shape's registers don't line up as Int the recognizer declines.
                let count_int = write_set
                    .iter()
                    .any(|&(r, k)| r == shape.count_reg && k == SlotKind::Int);
                let i_int = write_set
                    .iter()
                    .any(|&(r, k)| r == shape.i_reg && k == SlotKind::Int);
                let needle_len_int = guard_kinds
                    .iter()
                    .any(|&(r, k)| r == shape.needle_len_reg && k == SlotKind::Int);
                if both_text && count_int && i_int && needle_len_int {
                    let mut mm: Vec<MicroOp> = Vec::with_capacity(2);
                    mm.push(MicroOp::MemMem {
                        h_ptr_slot: tp.ptr_slot,
                        h_len_slot: tp.len_slot,
                        n_ptr_slot: np.ptr_slot,
                        n_len_slot: np.len_slot,
                        needle_len_slot: shape.needle_len_reg,
                        i_slot: shape.i_reg,
                        count_slot: shape.count_reg,
                        helper_addr: logos_rt_memmem_frame as usize as i64,
                    });
                    mm.push(MicroOp::Return { src: 0 });
                    // Only `count` and `i` are observable after the loop (they are
                    // the nest's loop-carried write-back); restrict the write-set
                    // to them so nothing stale is copied back.
                    let mm_write_set: Vec<(u16, SlotKind)> = write_set
                        .iter()
                        .copied()
                        .filter(|&(r, _)| r == shape.count_reg || r == shape.i_reg)
                        .collect();
                    return Some((
                        mm,
                        frame_size,
                        guard_kinds,
                        free,
                        mm_write_set,
                        array_pins,
                        None,
                        hoist_guards,
                        None,
                    ));
                }
            }
        }
    }

    // The frame buffer is reused across entries — the return flag must be
    // explicitly cleared at the head of every run.
    if let Some((flag, _)) = region_return_slots {
        micro.push(MicroOp::LoadConst { dst: flag, value: 0 });
    }

    let mut i = 0usize;
    while i < ops.len() {
        // Region call: the callee windows at base + frame_size (disjoint
        // from registers, pins, return and limit slots), arguments copied
        // into the fresh window. Entry deopt (callee not yet compiled,
        // depth, arena) writes the PLAIN marker — region replay is sound
        // because the resolver admitted only scalar-pure callees.
        if let Op::Call { dst, args_start, arg_count, func } = ops[i] {
            if kin[i].is_some() {
                if resolve(func).is_none() {
                    if std::env::var_os("LOGOS_RDIAG").is_some() {
                        let s = callees.get(func as usize);
                        eprintln!(
                            "RDIAG-BAIL head_pc={head_pc} lowering-resolve func={func} stable={:?} retparam={:?} ret={:?}",
                            s.map(|x| x.list_params_stable),
                            s.map(|x| x.returns_list_param),
                            s.map(|x| x.ret),
                        );
                    }
                    return None;
                }
                pc_to_micro.push(micro.len());
                let window = frame_size as u16;
                for j in 0..arg_count {
                    micro.push(MicroOp::Move { dst: window + j, src: args_start + j });
                }
                micro.push(MicroOp::Call {
                    dst,
                    args_start: window,
                    table_addr: ctx.table.slot_addr(func as usize),
                    depth_addr: std::sync::Arc::as_ptr(&ctx.depth) as i64,
                    status_addr: std::sync::Arc::as_ptr(&ctx.status) as i64,
                    limit_slot,
                    depth_limit: logicaffeine_compile::semantics::MAX_CALL_DEPTH as i64,
                });
            } else {
                pc_to_micro.push(micro.len());
            }
            i += 1;
            continue;
        }
        let Some(advance) = translate_op(
            ops,
            i,
            head_pc,
            constants,
            &kin,
            &live,
            &jump_targets,
            &scratch_ok,
            conv,
            &pins,
            None,
            region_return_slots,
            false,
            true,
            &mut micro,
            &mut pc_to_micro,
            &mut fixups,
        ) else {
            if std::env::var_os("LOGOS_RDIAG").is_some() {
                eprintln!("RDIAG-BAIL head_pc={head_pc} translate_op-failed op={:?}", ops[i]);
            }
            return None;
        };
        i += advance;
    }
    // Out-of-region jumps route to the synthetic exit terminal.
    let exit_micro = micro.len();
    micro.push(MicroOp::Return { src: 0 });
    let _ = exit_pc;
    for &fi in &fixups {
        match &mut micro[fi] {
            MicroOp::Jump { target }
            | MicroOp::JumpIfFalse { target, .. }
            | MicroOp::JumpIfTrue { target, .. }
            | MicroOp::Branch { target, .. }
            | MicroOp::BranchF { target, .. } => {
                *target = if in_region(*target) {
                    *pc_to_micro.get(*target - head_pc)?
                } else {
                    exit_micro
                };
            }
            _ => unreachable!(),
        }
    }
    // A push+SetIndex region is admitted ONLY with PRECISE deopt: on a side
    // exit the VM materializes the frame's scalars back into the VM registers
    // and resumes AT the faulting op. Materialization re-boxes each register by
    // its kind at that op (from the kind flow) — Int/Bool/Float by value;
    // LIST/MAP and read-only/unknown registers are KEPT (the VM register holds
    // the live value: a pinned array is mutated in place through its pins, and a
    // read-only slot still holds its entry value). The ONE thing that breaks
    // "keep" is a list/map REBIND — a def whose value is a collection, which
    // would leave the VM register holding the stale Rc. In-place array
    // mutations (`ListPush`/`SetIndex`) are USES of the handle, never defs, so
    // they never appear in `writes`; a genuine rebind does, and disqualifies.
    let array_reg_set: std::collections::HashSet<u16> =
        array_pins.iter().map(|a| a.reg).collect();
    let rebinds_collection = writes.iter().any(|&r| {
        !array_reg_set.contains(&r)
            && matches!(
                exit_kind[r as usize],
                Kind::IntList | Kind::FloatList | Kind::BoolList | Kind::IntMap
            )
    });
    let precise_region = needs_precise && !rebinds_collection;
    if needs_precise && !precise_region {
        if std::env::var_os("LOGOS_RDIAG").is_some() {
            eprintln!("RDIAG-BAIL head_pc={head_pc} needs-precise-but-rebinds-collection");
        }
        return None;
    }
    // Per-op resume codes + per-op materialization kinds. Each micro of bytecode
    // op `opi` carries `((head_pc+opi) << 2) | 3`; the matching `kinds_by_pc`
    // row says how to re-box every register when the VM resumes there.
    let precise = if precise_region {
        let mut codes = vec![1i64; micro.len()];
        let mut kinds_by_pc: std::collections::HashMap<usize, Vec<Option<SlotKind>>> =
            std::collections::HashMap::new();
        for opi in 0..ops.len() {
            let lo = pc_to_micro[opi];
            let hi = pc_to_micro.get(opi + 1).copied().unwrap_or(micro.len());
            let resume_pc = head_pc + opi;
            let code = ((resume_pc as i64) << 2) | 3;
            for c in codes.iter_mut().take(hi).skip(lo) {
                *c = code;
            }
            if let Some(k) = kin.get(opi).and_then(|x| x.as_ref()) {
                let row: Vec<Option<SlotKind>> = (0..register_count as usize)
                    .map(|r| {
                        if array_reg_set.contains(&(r as u16)) {
                            return None; // pinned array — keep its live Rc
                        }
                        match k.get(r) {
                            Some(Kind::Int) => Some(SlotKind::Int),
                            Some(Kind::Bool) => Some(SlotKind::Bool),
                            Some(Kind::Float) => Some(SlotKind::Float),
                            // list/map/unknown/mixed read-only → keep VM value
                            _ => None,
                        }
                    })
                    .collect();
                kinds_by_pc.insert(resume_pc, row);
            }
        }
        Some(RegionPrecise { deopt_codes: codes, kinds_by_pc })
    } else {
        None
    };
    Some((
        micro,
        frame_size,
        guard_kinds,
        free,
        write_set,
        array_pins,
        region_return,
        hoist_guards,
        precise,
    ))
}

/// Slots in the per-thread native call arena: deep self-recursion windows
/// callee frames upward through this one buffer (16 MiB); the call stencil's
/// arena-limit check deopts before any overflow.
const ARENA_SLOTS: usize = 1 << 21;

struct ChainFn {
    chain: CompiledChain,
    limit_slot: usize,
    depth: std::sync::Arc<std::sync::atomic::AtomicI64>,
    ret: NativeRet,
    /// frame_size − 3: what the call stencil's bound math expects in the
    /// table's regcount slot (mode A: the plain register count).
    published_regc: i64,
    precise: Option<PreciseInfo>,
    stats: std::sync::Arc<RuntimeStats>,
}

impl NativeFn for ChainFn {
    fn call(&self, args: &[i64], pins: &[i64], depth: usize) -> NativeOutcome {
        // One arena per thread: the outermost native call owns it for the
        // whole (possibly deeply recursive) run. Kind gates prove no slot is
        // read before it is written, so stale contents are unobservable.
        thread_local! {
            static ARENA: std::cell::RefCell<Vec<i64>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        self.stats.entries.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ARENA.with(|f| {
            let mut frame = f.borrow_mut();
            if frame.len() < ARENA_SLOTS {
                frame.resize(ARENA_SLOTS, 0);
            }
            frame[..args.len()].copy_from_slice(args);
            if let Some(p) = &self.precise {
                for (k, chunk) in pins.chunks(3).enumerate() {
                    let base = p.param_pin_scatter[k] as usize;
                    frame[base..base + 3].copy_from_slice(chunk);
                }
            }
            let arena_end = unsafe { frame.as_ptr().add(ARENA_SLOTS) } as i64;
            frame[self.limit_slot] = arena_end;
            self.depth.store(depth as i64, std::sync::atomic::Ordering::Relaxed);
            match self.chain.run_with_frame(&mut frame) {
                ChainOutcome::Return(v) => {
                    self.stats
                        .completions
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let out = if matches!(self.ret, NativeRet::ListByHandle) {
                        match alloc_registry_detach(v) {
                            // A fresh, registry-owned list: re-box here.
                            Some(vec) => NativeOutcome::ReturnValue(
                                logicaffeine_compile::vm::Value::int_list(vec),
                            ),
                            // A param passthrough: the VM matches the
                            // handle against its pin arguments.
                            None => NativeOutcome::Return(v),
                        }
                    } else {
                        NativeOutcome::Return(v)
                    };
                    alloc_registry_drain();
                    out
                }
                ChainOutcome::Deopt(raw) => {
                    if std::env::var_os("LOGOS_JIT_TRACE").is_some() {
                        let arena_end = unsafe { frame.as_ptr().add(ARENA_SLOTS) } as i64;
                        eprintln!(
                            "jit-trace: fn deopt raw={raw:#x} depth={depth} limit_slot={} \
                             frame[limit]={:#x} arena_end={:#x} delta={}",
                            self.limit_slot,
                            frame[self.limit_slot],
                            arena_end,
                            arena_end - frame[self.limit_slot],
                        );
                    }
                    let out = if raw & 3 == 3 {
                        self.stats
                            .deopt_ats
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let pc = ((raw & 0xFFFF_FFFF) >> 2) as usize;
                        let frame_count = ((raw >> 32) - depth as i64 + 1) as usize;
                        self.materialize(pc, frame_count, &frame, pins)
                    } else {
                        self.stats
                            .deopts
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        NativeOutcome::Deopt
                    };
                    alloc_registry_drain();
                    out
                }
            }
        })
    }
    fn ret(&self) -> NativeRet {
        self.ret
    }
    fn entry_ptr(&self) -> i64 {
        self.chain.base() as i64
    }
    fn published_regc(&self) -> i64 {
        self.published_regc
    }
}

impl ChainFn {
    /// Precise deopt: walk the active plant chain (every frame's window
    /// plant is −1 unless a call is in flight) and package each native
    /// frame with its pause-point re-box kinds — the innermost pauses at
    /// the faulting pc, every ancestor at its own Call op.
    fn materialize(
        &self,
        pc: usize,
        frame_count: usize,
        frame: &[i64],
        pins: &[i64],
    ) -> NativeOutcome {
        let p = self
            .precise
            .as_ref()
            .expect("precise deopt status from a mode-A chain");
        let rc = p.register_count;
        // Exactly `frame_count` frames are live (the depth rode the status
        // value): a call-entry deopt leaves the innermost frame's plant
        // freshly written with a callee that never materialized, so the
        // plant chain alone cannot terminate the walk.
        let mut raw: Vec<(usize, i64, usize, u16)> = Vec::with_capacity(frame_count);
        let mut base = 0usize;
        for k in 0..frame_count {
            let window = frame[base + rc + 2];
            raw.push((
                base,
                window,
                frame[base + rc + 3] as usize,
                frame[base + rc + 4] as u16,
            ));
            if k + 1 < frame_count {
                assert!(
                    window > 0,
                    "corrupt plant chain: window {window} at depth {k} (pc {pc})"
                );
                base += window as usize;
            }
        }
        let last = raw.len() - 1;
        let mut frames = Vec::with_capacity(raw.len());
        for (k, &(b, _, my_resume, _)) in raw.iter().enumerate() {
            let pause_pc = if k == last { pc } else { my_resume - 1 };
            let mut kinds = p
                .kinds_by_pc
                .get(&pause_pc)
                .unwrap_or_else(|| panic!("missing kind capture for pc {pause_pc}"))
                .clone();
            // Native-owned lists survive the deopt: detach fresh ones into
            // real values; param passthroughs rewrite to their argument.
            let mut resolved: Vec<(u16, logicaffeine_compile::vm::Value)> = Vec::new();
            if let Some(owned) = p.owned_by_pc.get(&pause_pc) {
                for &(reg, vslot) in owned {
                    let handle = frame[b + vslot as usize];
                    if let Some(vec) = alloc_registry_detach(handle) {
                        kinds[reg as usize] = RegBox::Resolved;
                        resolved.push((reg, logicaffeine_compile::vm::Value::int_list(vec)));
                    } else {
                        // A param's vec: match the boundary pin handles.
                        let mut hit = false;
                        for (j, chunk) in pins.chunks(3).enumerate() {
                            if chunk.first() == Some(&handle) {
                                kinds[reg as usize] = RegBox::ListParam(j as u8);
                                hit = true;
                                break;
                            }
                        }
                        if !hit {
                            kinds[reg as usize] = RegBox::Dead;
                        }
                    }
                }
            }
            let (offset, return_pc, return_reg) = if k == 0 {
                (0, 0, 0)
            } else {
                let prev = raw[k - 1];
                (prev.1 as usize, prev.2, prev.3)
            };
            frames.push(NativeFrame {
                offset,
                return_pc,
                return_reg,
                regs: frame[b..b + rc].to_vec(),
                kinds,
                resolved,
            });
        }
        NativeOutcome::DeoptAt { resume_pc: pc, frames }
    }
}

pub static REGION_RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static REGION_DEOPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct RegionChain {
    chain: CompiledChain,
    frame_size: usize,
    /// Set when the region CALLS functions: arena headroom for callee
    /// windows, the limit slot to plant, and the live depth cell.
    call_support: Option<(usize, u16, std::sync::Arc<std::sync::atomic::AtomicI64>)>,
    guard: Vec<(u16, SlotKind)>,
    free: Vec<u16>,
    writes: Vec<(u16, SlotKind)>,
    arrays: Vec<ArrayPin>,
    region_return: Option<logicaffeine_compile::vm::RegionReturn>,
    hoist_guards: Vec<HoistGuard>,
    /// PRECISE region deopt: when `Some`, the chain was compiled with per-op
    /// `deopt_codes`, so a side exit stores `(resume_pc << 2) | 3` (low 32 bits)
    /// in the status cell — decode it to a precise `DeoptAt` and look the
    /// per-register re-box kinds up here by resume pc. `None` ⇒ the classic
    /// plain-replay region.
    precise_kinds: Option<std::collections::HashMap<usize, Vec<Option<SlotKind>>>>,
}

impl RegionFn for RegionChain {
    fn guard_set(&self) -> &[(u16, SlotKind)] {
        &self.guard
    }
    fn free_set(&self) -> &[u16] {
        &self.free
    }
    fn write_set(&self) -> &[(u16, SlotKind)] {
        &self.writes
    }
    fn array_set(&self) -> &[ArrayPin] {
        &self.arrays
    }
    fn region_return(&self) -> Option<logicaffeine_compile::vm::RegionReturn> {
        self.region_return
    }
    fn hoist_guards(&self) -> &[HoistGuard] {
        &self.hoist_guards
    }
    fn frame_size(&self) -> usize {
        self.frame_size
    }
    fn arena_slots(&self) -> usize {
        self.call_support.as_ref().map(|(a, _, _)| *a).unwrap_or(0)
    }
    fn precise_kinds(&self, resume_pc: usize) -> Option<&[Option<SlotKind>]> {
        self.precise_kinds
            .as_ref()
            .and_then(|m| m.get(&resume_pc))
            .map(|v| v.as_slice())
    }
    fn run(&self, frame: &mut [i64], depth: usize) -> RegionOutcome {
        if let Some((_, limit_slot, depth_cell)) = &self.call_support {
            let arena_end = unsafe { frame.as_ptr().add(frame.len()) } as i64;
            frame[*limit_slot as usize] = arena_end;
            depth_cell.store(depth as i64, std::sync::atomic::Ordering::Relaxed);
        }
        match self.chain.run_with_frame(frame) {
            ChainOutcome::Return(_) => {
                REGION_RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                RegionOutcome::Completed
            }
            ChainOutcome::Deopt(raw) => {
                REGION_DEOPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // Precise chains tag every side exit with `(resume_pc << 2) | 3`
                // in the low 32 bits; a plain chain's terminal stores a value
                // whose low 2 bits are not 3. Resume precisely when tagged.
                if self.precise_kinds.is_some() && (raw & 0b11) == 0b11 {
                    let resume_pc = ((raw & 0xFFFF_FFFF) >> 2) as usize;
                    RegionOutcome::DeoptAt { resume_pc }
                } else {
                    RegionOutcome::Deopt
                }
            }
        }
    }
}

/// Runtime boundary counters shared by every ChainFn a tier produces.
/// Compilation is not execution: these count what the native code actually
/// DID — entries across the VM boundary, completed returns, and side exits.
#[derive(Default)]
struct RuntimeStats {
    entries: std::sync::atomic::AtomicU64,
    completions: std::sync::atomic::AtomicU64,
    deopts: std::sync::atomic::AtomicU64,
    deopt_ats: std::sync::atomic::AtomicU64,
}

/// The forge-backed native tier, with compile observability.
#[derive(Default)]
pub struct ForgeTier {
    compiles: AtomicU32,
    successes: AtomicU32,
    region_compiles: AtomicU32,
    region_successes: AtomicU32,
    /// Region compiles whose chain came from the CONTIGUOUS register-allocating
    /// backend (`compile_region_regalloc`) rather than a per-piece stencil tier
    /// — the observable that WS-G's backend actually fired on a region.
    regalloc_regions: AtomicU32,
    /// PRECISE region compiles whose chain came from the contiguous register-
    /// allocating backend (`compile_region_regalloc_precise`) rather than the
    /// per-piece precise stencil tier (`compile_straightline_coded`). A precise
    /// region is the in-place-mutation + reallocating-push shape (the fannkuch /
    /// graph_bfs worklist). Nonzero proves the Wave 21 precise-region regalloc
    /// path fired — distinct from `regalloc_regions`, which counts a program's
    /// non-precise loops too.
    regalloc_precise_regions: AtomicU32,
    /// FUNCTION compiles whose chain came from the contiguous register-allocating
    /// backend (`compile_function_regalloc`) — the observable that the recursion
    /// cluster (self-calls) now goes through the regalloc backend instead of the
    /// per-piece stencil tier.
    regalloc_functions: AtomicU32,
    /// Fused pinned-argument self-calls (`CallSelfCopy`) emitted across all
    /// function compiles — the observable that Lever A fired.
    pinned_self_calls: AtomicU32,
    runtime: std::sync::Arc<RuntimeStats>,
}

impl ForgeTier {
    /// A fresh tier with zeroed counters.
    pub fn new() -> Self {
        ForgeTier::default()
    }

    /// (attempts, successes) for FUNCTION compiles.
    pub fn function_counts(&self) -> (u32, u32) {
        (self.compiles.load(Ordering::SeqCst), self.successes.load(Ordering::SeqCst))
    }

    /// The number of fused pinned-argument self-calls (`CallSelfCopy`)
    /// emitted across every function compile — Lever A's observable. A
    /// nonzero value proves a scalar self-call took the fused ABI rather
    /// than per-argument frame `Move` staging.
    pub fn pinned_self_call_count(&self) -> u32 {
        self.pinned_self_calls.load(Ordering::SeqCst)
    }

    /// (attempts, successes) for Main-loop REGION compiles.
    pub fn region_counts(&self) -> (u32, u32) {
        (
            self.region_compiles.load(Ordering::SeqCst),
            self.region_successes.load(Ordering::SeqCst),
        )
    }

    /// How many region compiles used the CONTIGUOUS register-allocating backend
    /// (WS-G `compile_region_regalloc`) rather than a per-piece stencil tier.
    /// Nonzero proves the regalloc backend fired on a real program's hot loop.
    pub fn regalloc_region_count(&self) -> u32 {
        self.regalloc_regions.load(Ordering::SeqCst)
    }

    /// How many PRECISE region compiles used the contiguous register-allocating
    /// backend (`compile_region_regalloc_precise`) rather than the per-piece
    /// precise stencil tier. Nonzero proves the in-place-mutation + reallocating-
    /// push shape (fannkuch / graph_bfs worklist) tiered through the regalloc
    /// backend — Wave 21's lever.
    pub fn regalloc_precise_region_count(&self) -> u32 {
        self.regalloc_precise_regions.load(Ordering::SeqCst)
    }

    /// How many FUNCTION compiles used the contiguous register-allocating backend
    /// (`compile_function_regalloc`) rather than the per-piece stencil tier.
    /// Nonzero proves a recursive (self-calling) function tiered through the
    /// regalloc backend — the recursion cluster's lever.
    pub fn regalloc_function_count(&self) -> u32 {
        self.regalloc_functions.load(Ordering::SeqCst)
    }

    /// Runtime FUNCTION-boundary truth: (entries, completions, plain
    /// deopts, precise deopts). Entries count VM→native crossings — native
    /// self-recursion inside one entry is invisible here, which is exactly
    /// the point: a hot recursive function shows FEW entries.
    pub fn runtime_stats(&self) -> (u64, u64, u64, u64) {
        (
            self.runtime.entries.load(Ordering::SeqCst),
            self.runtime.completions.load(Ordering::SeqCst),
            self.runtime.deopts.load(Ordering::SeqCst),
            self.runtime.deopt_ats.load(Ordering::SeqCst),
        )
    }
}

/// Loop-nesting depth of each micro op, from its enclosing back-edges (a
/// jump/branch whose target is at or before it opens a loop over
/// `[target, here]`). Depth is how many such loops contain an op — the
/// frequency proxy for region pin selection.
fn loop_depths(ops: &[MicroOp]) -> Vec<u32> {
    let n = ops.len();
    let mut depth = vec![0u32; n];
    for (i, op) in ops.iter().enumerate() {
        let target = match *op {
            MicroOp::Jump { target }
            | MicroOp::JumpIfFalse { target, .. }
            | MicroOp::JumpIfTrue { target, .. }
            | MicroOp::Branch { target, .. }
            | MicroOp::BranchF { target, .. } => Some(target),
            _ => None,
        };
        if let Some(t) = target {
            if t <= i && t < n {
                for d in depth.iter_mut().take(i + 1).skip(t) {
                    *d += 1;
                }
            }
        }
    }
    depth
}

/// Linear-scan pin selection. Each slot's profit sums its per-op deltas —
/// variant-family uses save a memory access (positive), mem-form uses force
/// a frame spill (negative).
///
/// `region` switches the FREQUENCY MODEL. A FUNCTION body is itself the hot
/// loop (recursion is a Call, not a back-edge), so every op counts equally
/// (the historical flat model — preserved exactly). A REGION's depth-0 ops
/// run once per entry while its INNER loops dominate, so each op is weighted
/// by `BASE^loop_depth` — a slot wins only when its register-resident
/// variant uses outweigh its spill costs AT THE FREQUENCY THEY EXECUTE.
/// (Without this, a counter feeding an inner-loop array INDEX — mem-form,
/// spills every iteration — looks neutral statically but is a net loss; see
/// histogram vs sieve.) Conversion scratches and beyond-register slots
/// never pin; the four most profitable net-positive slots win.
///
/// The slots a micro-op READS, by value (for read-counting). Call/CallSelf are
/// handled by the caller (copy-prop bails on any region containing one), so
/// their frame-window arg reads need no accounting here.
fn micro_read_slots(op: &MicroOp) -> Vec<u16> {
    use MicroOp::*;
    match *op {
        LoadConst { .. } | Jump { .. } | NewList { .. } => vec![],
        Move { src, .. } | DivPow2 { lhs: src, .. } | MagicDivU { lhs: src, .. } | NotInt { src, .. } | NotBool { src, .. }
        | IntToFloat { src, .. } | SqrtF { src, .. } | Return { src } | ListTriple { handle_slot: src, .. } => vec![src],
        JumpIfFalse { cond, .. } | JumpIfTrue { cond, .. } => vec![cond],
        Add { lhs, rhs, .. } | Sub { lhs, rhs, .. } | Mul { lhs, rhs, .. } | Div { lhs, rhs, .. }
        | Mod { lhs, rhs, .. } | Lt { lhs, rhs, .. } | Gt { lhs, rhs, .. } | Eq { lhs, rhs, .. }
        | LtEq { lhs, rhs, .. } | GtEq { lhs, rhs, .. } | Neq { lhs, rhs, .. } | BitAnd { lhs, rhs, .. }
        | BitOr { lhs, rhs, .. } | BitXor { lhs, rhs, .. } | Shl { lhs, rhs, .. } | Shr { lhs, rhs, .. }
        | Branch { lhs, rhs, .. } | AddF { lhs, rhs, .. } | SubF { lhs, rhs, .. } | MulF { lhs, rhs, .. }
        | DivF { lhs, rhs, .. } | LtF { lhs, rhs, .. } | GtF { lhs, rhs, .. } | LtEqF { lhs, rhs, .. }
        | GtEqF { lhs, rhs, .. } | EqF { lhs, rhs, .. } | NeqF { lhs, rhs, .. }
        | BranchF { lhs, rhs, .. } => vec![lhs, rhs],
        MapGet { key, map_slot, .. } | MapHas { key, map_slot, .. } => vec![key, map_slot],
        MapSet { src, key, map_slot, .. } => vec![src, key, map_slot],
        ArrLoad { idx, ptr_slot, len_slot, .. } => vec![idx, ptr_slot, len_slot],
        ArrLoadAffine { a, op, b, ptr_slot, len_slot, .. } => {
            if op == logicaffeine_forge::jit::AffOp::None {
                vec![a, ptr_slot, len_slot]
            } else {
                vec![a, b, ptr_slot, len_slot]
            }
        }
        ArrLoad2F { i, j, ptr_slot, len_slot, .. } => vec![i, j, ptr_slot, len_slot],
        ArrLoad2 { i, j, ptr_a, len_a, ptr_b, len_b, .. } => vec![i, j, ptr_a, len_a, ptr_b, len_b],
        ArrStore { src, idx, ptr_slot, len_slot, .. } => vec![src, idx, ptr_slot, len_slot],
        ArrRMW { idx, operand, ptr_slot, len_slot, .. } => vec![idx, operand, ptr_slot, len_slot],
        ArrCondSwap { idx1, idx2, ptr_slot, len_slot, .. } => vec![idx1, idx2, ptr_slot, len_slot],
        ArrSwap { idx1, idx2, ptr_slot, len_slot, .. } => vec![idx1, idx2, ptr_slot, len_slot],
        FmaF { a, b, c, .. } => vec![a, b, c],
        ArrPush { src, vec_slot, ptr_slot, len_slot, .. } => vec![src, vec_slot, ptr_slot, len_slot],
        ListClear { vec_slot, ptr_slot, len_slot, .. } => vec![vec_slot, ptr_slot, len_slot],
        StrAppend { text_handle_slot, src, .. } => match src {
            logicaffeine_forge::jit::StrSrc::Byte(s) => vec![text_handle_slot, s],
            logicaffeine_forge::jit::StrSrc::Const { .. } => vec![text_handle_slot],
        },
        MemMem {
            h_ptr_slot,
            h_len_slot,
            n_ptr_slot,
            n_len_slot,
            needle_len_slot,
            i_slot,
            count_slot,
            ..
        } => vec![
            h_ptr_slot,
            h_len_slot,
            n_ptr_slot,
            n_len_slot,
            needle_len_slot,
            i_slot,
            count_slot,
        ],
        Call { .. } | CallSelf { .. } | CallSelfCopy { .. } => vec![],
    }
}

/// Mutable references to a micro-op's READ operands (for in-place copy
/// substitution). Mirrors [`micro_read_slots`].
fn micro_reads_mut(op: &mut MicroOp) -> Vec<&mut u16> {
    use MicroOp::*;
    match op {
        LoadConst { .. } | Jump { .. } | NewList { .. } | Call { .. } | CallSelf { .. }
        | CallSelfCopy { .. } => vec![],
        Move { src, .. } | DivPow2 { lhs: src, .. } | MagicDivU { lhs: src, .. } | NotInt { src, .. } | NotBool { src, .. }
        | IntToFloat { src, .. } | SqrtF { src, .. } | Return { src } | ListTriple { handle_slot: src, .. } => vec![src],
        JumpIfFalse { cond, .. } | JumpIfTrue { cond, .. } => vec![cond],
        Add { lhs, rhs, .. } | Sub { lhs, rhs, .. } | Mul { lhs, rhs, .. } | Div { lhs, rhs, .. }
        | Mod { lhs, rhs, .. } | Lt { lhs, rhs, .. } | Gt { lhs, rhs, .. } | Eq { lhs, rhs, .. }
        | LtEq { lhs, rhs, .. } | GtEq { lhs, rhs, .. } | Neq { lhs, rhs, .. } | BitAnd { lhs, rhs, .. }
        | BitOr { lhs, rhs, .. } | BitXor { lhs, rhs, .. } | Shl { lhs, rhs, .. } | Shr { lhs, rhs, .. }
        | Branch { lhs, rhs, .. } | AddF { lhs, rhs, .. } | SubF { lhs, rhs, .. } | MulF { lhs, rhs, .. }
        | DivF { lhs, rhs, .. } | LtF { lhs, rhs, .. } | GtF { lhs, rhs, .. } | LtEqF { lhs, rhs, .. }
        | GtEqF { lhs, rhs, .. } | EqF { lhs, rhs, .. } | NeqF { lhs, rhs, .. }
        | BranchF { lhs, rhs, .. } => vec![lhs, rhs],
        MapGet { key, map_slot, .. } | MapHas { key, map_slot, .. } => vec![key, map_slot],
        MapSet { src, key, map_slot, .. } => vec![src, key, map_slot],
        ArrLoad { idx, ptr_slot, len_slot, .. } => vec![idx, ptr_slot, len_slot],
        ArrLoadAffine { a, b, ptr_slot, len_slot, .. } => vec![a, b, ptr_slot, len_slot],
        ArrLoad2F { i, j, ptr_slot, len_slot, .. } => vec![i, j, ptr_slot, len_slot],
        ArrLoad2 { i, j, ptr_a, len_a, ptr_b, len_b, .. } => vec![i, j, ptr_a, len_a, ptr_b, len_b],
        ArrStore { src, idx, ptr_slot, len_slot, .. } => vec![src, idx, ptr_slot, len_slot],
        ArrRMW { idx, operand, ptr_slot, len_slot, .. } => vec![idx, operand, ptr_slot, len_slot],
        ArrCondSwap { idx1, idx2, ptr_slot, len_slot, .. } => vec![idx1, idx2, ptr_slot, len_slot],
        ArrSwap { idx1, idx2, ptr_slot, len_slot, .. } => vec![idx1, idx2, ptr_slot, len_slot],
        FmaF { a, b, c, .. } => vec![a, b, c],
        ArrPush { src, vec_slot, ptr_slot, len_slot, .. } => vec![src, vec_slot, ptr_slot, len_slot],
        ListClear { vec_slot, ptr_slot, len_slot, .. } => vec![vec_slot, ptr_slot, len_slot],
        StrAppend { text_handle_slot, src, .. } => match src {
            logicaffeine_forge::jit::StrSrc::Byte(s) => vec![text_handle_slot, s],
            logicaffeine_forge::jit::StrSrc::Const { .. } => vec![text_handle_slot],
        },
        MemMem {
            h_ptr_slot,
            h_len_slot,
            n_ptr_slot,
            n_len_slot,
            needle_len_slot,
            i_slot,
            count_slot,
            ..
        } => vec![
            h_ptr_slot,
            h_len_slot,
            n_ptr_slot,
            n_len_slot,
            needle_len_slot,
            i_slot,
            count_slot,
        ],
    }
}

/// The slots a micro-op DEFINES (writes).
fn micro_defs(op: &MicroOp) -> Vec<u16> {
    use MicroOp::*;
    match *op {
        LoadConst { dst, .. } | Move { dst, .. } | Add { dst, .. } | Sub { dst, .. } | Mul { dst, .. }
        | Div { dst, .. } | DivPow2 { dst, .. } | MagicDivU { dst, .. } | Mod { dst, .. } | Lt { dst, .. } | Gt { dst, .. }
        | Eq { dst, .. } | LtEq { dst, .. } | GtEq { dst, .. } | Neq { dst, .. } | BitAnd { dst, .. }
        | BitOr { dst, .. } | BitXor { dst, .. } | Shl { dst, .. } | Shr { dst, .. } | NotInt { dst, .. }
        | NotBool { dst, .. } | AddF { dst, .. } | SubF { dst, .. } | MulF { dst, .. } | DivF { dst, .. }
        | LtF { dst, .. } | GtF { dst, .. } | LtEqF { dst, .. } | GtEqF { dst, .. } | EqF { dst, .. }
        | NeqF { dst, .. } | IntToFloat { dst, .. } | SqrtF { dst, .. } | ArrLoad { dst, .. }
        | ArrLoadAffine { dst, .. }
        | ArrLoad2F { dst, .. } | ArrLoad2 { dst, .. } | FmaF { dst, .. } | MapGet { dst, .. } | MapHas { dst, .. }
        | Call { dst, .. } | CallSelf { dst, .. } | CallSelfCopy { dst, .. } => vec![dst],
        ArrPush { ptr_slot, len_slot, .. } => vec![ptr_slot, len_slot],
        ListClear { ptr_slot, len_slot, .. } => vec![ptr_slot, len_slot],
        NewList { vec_slot, ptr_slot, len_slot, .. } => vec![vec_slot, ptr_slot, len_slot],
        ListTriple { vec_slot, ptr_slot, len_slot, .. } => vec![vec_slot, ptr_slot, len_slot],
        MemMem { i_slot, count_slot, .. } => vec![i_slot, count_slot],
        Branch { .. } | BranchF { .. } | Jump { .. } | JumpIfFalse { .. } | JumpIfTrue { .. }
        | MapSet { .. } | ArrStore { .. } | ArrRMW { .. } | ArrCondSwap { .. } | ArrSwap { .. } | Return { .. }
        // The mutable-Text append writes the off-frame VM register cell (through
        // the planted `*mut Value`), not a frame slot — it defines NO frame slot.
        | StrAppend { .. } => vec![],
    }
}

/// A mutable reference to a micro-op's branch/jump TARGET (a micro index), if any.
fn micro_target_mut(op: &mut MicroOp) -> Option<&mut usize> {
    use MicroOp::*;
    match op {
        Jump { target } | Branch { target, .. } | BranchF { target, .. }
        | JumpIfFalse { target, .. } | JumpIfTrue { target, .. } => Some(target),
        _ => None,
    }
}

/// COPY PROPAGATION + dead-Move elimination over a region's micro stream — kills
/// the redundant copy chains the bytecode lowering leaves in hot loops (e.g.
/// mandelbrot's `t40←t36; t46←t40; zr←t46`, three Moves to land one value),
/// each of which is a wasted stencil dispatch every iteration. SOUNDNESS: it
/// only ever (1) rewrites a READ to a value provably equal to it within the
/// same basic block (copies are cleared at every jump target and across every
/// branch/jump), and (2) removes a `Move` whose destination is an UN-NAMED
/// scratch temp (never written back to a VM register — `named[d]` is false) and
/// has zero remaining reads. Named slots (loop-carried / observable) are never
/// touched. The caller restricts this to NON-PRECISE, CALL-FREE regions, so the
/// only index-bearing references that survive removal are jump targets, which
/// are remapped here; `deopt` is replay-from-head (no per-micro resume).
fn copy_propagate(
    micro: &mut Vec<MicroOp>,
    named: &[bool],
    register_count: u16,
    pinned: &std::collections::HashSet<u16>,
) {
    if micro.iter().any(|op| matches!(op, MicroOp::Call { .. } | MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. })) {
        return;
    }
    // A slot is REMOVABLE only if it is an un-named, un-PINNED scratch temp: its
    // value is never written back to a VM register (not named) and it is not
    // held live in a register by the pin allocator. This lets the pass run AFTER
    // pin selection without disturbing the chosen pins (named/pinned defining ops
    // and their reads are left intact).
    let removable = |slot: u16| -> bool {
        slot < register_count
            && !named.get(slot as usize).copied().unwrap_or(true)
            && !pinned.contains(&slot)
    };
    let n = micro.len();
    let mut leader = vec![false; n];
    for op in micro.iter() {
        if let Some(t) = micro_target_const(op) {
            if t < n {
                leader[t] = true;
            }
        }
    }
    // Phase 1: forward block-local value propagation. `root[d] = s` means slot d
    // currently holds an unmodified copy of slot s; reads of d are rewritten to
    // s. `const_holder[v] = slot` tracks the live slot holding integer-bits
    // constant `v`, so a re-`LoadConst` of the same value aliases the existing
    // holder (CONSTANT CSE — hot loops re-load the same small constants every
    // iteration). Both maps clear at block boundaries and invalidate when a
    // tracked slot is redefined.
    let mut root: std::collections::HashMap<u16, u16> = std::collections::HashMap::new();
    let mut const_holder: std::collections::HashMap<i64, u16> = std::collections::HashMap::new();
    // `value_holder[(tag, a, b)] = slot` — the live slot already holding the
    // result of that pure op over those operands. A redundant recomputation
    // (e.g. `v + 1` evaluated for both the load index and the store index of an
    // array RMW) aliases the existing holder, which (a) cuts a stencil dispatch
    // and (b) makes the two indices the SAME slot, letting `fuse_array_rmw`
    // collapse the load/store. Local value numbering: cleared at block
    // boundaries, invalidated the instant any operand or the holder is redefined.
    let mut value_holder: std::collections::HashMap<(u8, u16, u16), u16> =
        std::collections::HashMap::new();
    for i in 0..n {
        if leader[i] {
            root.clear();
            const_holder.clear();
            value_holder.clear();
        }
        for r in micro_reads_mut(&mut micro[i]) {
            if let Some(&s) = root.get(r) {
                *r = s;
            }
        }
        // Value key + prior holder, captured from the operands AS SUBSTITUTED
        // above and BEFORE this op's own def invalidates the table.
        let value_key = micro_pure_value_key(&micro[i]);
        let cse_hit = value_key.and_then(|(k, _)| value_holder.get(&k).copied());
        let defs = micro_defs(&micro[i]);
        for &d in &defs {
            root.retain(|k, v| *k != d && *v != d);
            const_holder.retain(|_, s| *s != d);
            value_holder.retain(|k, s| k.1 != d && k.2 != d && *s != d);
        }
        match micro[i] {
            MicroOp::Move { dst, src } if dst != src => {
                let sroot = root.get(&src).copied().unwrap_or(src);
                if sroot != dst {
                    root.insert(dst, sroot);
                }
            }
            MicroOp::LoadConst { dst, value } => match const_holder.get(&value) {
                Some(&d2) if d2 != dst => {
                    root.insert(dst, d2);
                }
                Some(_) => {}
                None => {
                    const_holder.insert(value, dst);
                }
            },
            _ => {}
        }
        if let Some((k, dst)) = value_key {
            match cse_hit {
                Some(h) if h != dst => {
                    root.insert(dst, h);
                }
                _ => {
                    value_holder.insert(k, dst);
                }
            }
        }
        // NB: copies/CSE are NOT cleared after a branch. A branch defines no
        // slot, so its fall-through (a single-predecessor block) validly inherits
        // them; the only multi-predecessor blocks are JUMP TARGETS, which the
        // leader-clear at the top of the loop already resets. (Clearing here too
        // was overly conservative — it dropped copies that hold across the
        // compare-branch into a loop's conditional body, e.g. a sort's swap.)
    }
    // Phase 2: remove dead scratch Moves / LoadConsts / pure binops (0 remaining
    // reads after propagation), remapping jump targets after each removal.
    loop {
        let mut reads: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for op in micro.iter() {
            for s in micro_read_slots(op) {
                *reads.entry(s).or_insert(0) += 1;
            }
        }
        let dead = |dst: u16| removable(dst) && reads.get(&dst).copied().unwrap_or(0) == 0;
        let victim = micro.iter().position(|op| match *op {
            MicroOp::Move { dst, src } => dst != src && dead(dst),
            MicroOp::LoadConst { dst, .. } => dead(dst),
            _ => micro_pure_value_key(op).is_some_and(|(_, dst)| dead(dst)),
        });
        let Some(idx) = victim else { break };
        micro.remove(idx);
        for op in micro.iter_mut() {
            if let Some(t) = micro_target_mut(op) {
                if *t > idx {
                    *t -= 1;
                }
            }
        }
    }
    // Phase 3: DEAD-STORE elimination — a pure def whose dst is REDEFINED before
    // it is ever read is dead even if the dst is read LATER (register reuse, e.g.
    // a sort's recomputed `j+1` index whose slot is reused for the loop-increment
    // constant). Global read-counting (phase 2) keeps these; a forward straight-
    // line scan removes them. Conservative: stop the scan at any control-flow op
    // or jump target (the value could be live on another path → keep). Only
    // un-named/un-pinned scratch (the `removable` slots) is eligible.
    loop {
        let mut victim = None;
        'scan: for i in 0..micro.len() {
            let d = match micro[i] {
                MicroOp::Move { dst, src } if dst != src => dst,
                MicroOp::LoadConst { dst, .. } => dst,
                ref other => match micro_pure_value_key(other) {
                    Some((_, dst)) => dst,
                    None => continue,
                },
            };
            if !removable(d) {
                continue;
            }
            // Walk forward: `d` is dead at `i` iff it is REDEFINED before any
            // read. We stop at a control-flow SOURCE op (a branch/jump could
            // carry `d`'s value to a taken path that reads it) — but NOT at a
            // mere jump TARGET: a leader that redefines `d` as its first action
            // kills every incoming `d`, so checking it is sound.
            for op in micro.iter().skip(i + 1) {
                if micro_read_slots(op).contains(&d) {
                    break; // read before redef — live; keep.
                }
                if micro_defs(op).contains(&d) {
                    victim = Some(i); // redefined before any read — op `i` is dead.
                    break 'scan;
                }
                if micro_target_const(op).is_some() {
                    break; // control transfers away — be conservative; keep.
                }
            }
        }
        let Some(idx) = victim else { break };
        micro.remove(idx);
        for op in micro.iter_mut() {
            if let Some(t) = micro_target_mut(op) {
                if *t > idx {
                    *t -= 1;
                }
            }
        }
    }
}

/// ARRAY READ-MODIFY-WRITE FUSION — collapses the `ArrLoad(t = arr[i]) ;
/// <int ALU>(t2 = t OP operand) ; ArrStore(arr[i] = t2)` idiom (histogram's
/// `counts[x] += 1`, counting sort, bitset marks) into ONE [`MicroOp::ArrRMW`]:
/// one stencil dispatch + one bounds check instead of three, and the element
/// never round-trips the frame. The JIT is per-op DISPATCH-bound, so collapsing
/// three hot ops into one is a direct win on the indexed-RMW loops that lose
/// worst to V8.
///
/// SOUNDNESS: the load and store hit the SAME pinned 8-byte array and the SAME
/// index slot (unchanged between them — the ALU only writes the scratch `t2`),
/// so the read cell and the written cell coincide. A single pre-write bounds
/// check (`checked = c1 || c2`) is bit-identical to the original pair because
/// both tested that same index. The fused op side-exits BEFORE any effect and,
/// occupying the ArrLoad's micro slot, replays all three ops from that bytecode
/// pc (region deopt is replay-from-head). The loaded value `t` and the result
/// `t2` must be single-use, un-named, un-pinned scratch (read only by the ALU /
/// the store) — exactly the staleness contract `copy_propagate` already relies
/// on, so a dead frame cell is never observably written back. None of the three
/// ops may be a jump target (no entry lands mid-idiom). Caller restricts this to
/// NON-PRECISE regions (removing micro ops renumbers jump targets — remapped
/// here — but would break per-op precise resume codes).
fn fuse_array_rmw(
    micro: &mut Vec<MicroOp>,
    named: &[bool],
    register_count: u16,
    pinned: &std::collections::HashSet<u16>,
) {
    if micro.iter().any(|op| matches!(op, MicroOp::Call { .. } | MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. })) {
        return;
    }
    let removable = |slot: u16, reads: &std::collections::HashMap<u16, usize>| -> bool {
        slot < register_count
            && !named.get(slot as usize).copied().unwrap_or(true)
            && !pinned.contains(&slot)
            && reads.get(&slot).copied().unwrap_or(0) == 1
    };
    loop {
        let n = micro.len();
        // Block leaders: an op any branch/jump lands on. Recomputed each round
        // (a fusion renumbers the stream).
        let mut is_target = vec![false; n];
        for op in micro.iter() {
            if let Some(t) = micro_target_const(op) {
                if t < n {
                    is_target[t] = true;
                }
            }
        }
        let mut reads: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for op in micro.iter() {
            for s in micro_read_slots(op) {
                *reads.entry(s).or_insert(0) += 1;
            }
        }
        let mut hit: Option<(usize, MicroOp)> = None;
        for k in 0..n.saturating_sub(2) {
            // The ALU and the store must not be block leaders.
            if is_target[k + 1] || is_target[k + 2] {
                continue;
            }
            let MicroOp::ArrLoad { dst: t, idx: i, ptr_slot: p, len_slot: l, byte: false, checked: c1 } =
                micro[k]
            else {
                continue;
            };
            // The ALU op: the loaded value `t` is one operand; the OTHER is the
            // RMW operand. Sub is non-commutative — fuse only when `t` is on the
            // left (`buf[i] - operand`).
            let (t2, op, operand) = match micro[k + 1] {
                MicroOp::Add { dst, lhs, rhs } if lhs == t => (dst, RmwOp::Add, rhs),
                MicroOp::Add { dst, lhs, rhs } if rhs == t => (dst, RmwOp::Add, lhs),
                MicroOp::Sub { dst, lhs, rhs } if lhs == t => (dst, RmwOp::Sub, rhs),
                MicroOp::Mul { dst, lhs, rhs } if lhs == t => (dst, RmwOp::Mul, rhs),
                MicroOp::Mul { dst, lhs, rhs } if rhs == t => (dst, RmwOp::Mul, lhs),
                MicroOp::BitAnd { dst, lhs, rhs } if lhs == t => (dst, RmwOp::And, rhs),
                MicroOp::BitAnd { dst, lhs, rhs } if rhs == t => (dst, RmwOp::And, lhs),
                MicroOp::BitOr { dst, lhs, rhs } if lhs == t => (dst, RmwOp::Or, rhs),
                MicroOp::BitOr { dst, lhs, rhs } if rhs == t => (dst, RmwOp::Or, lhs),
                MicroOp::BitXor { dst, lhs, rhs } if lhs == t => (dst, RmwOp::Xor, rhs),
                MicroOp::BitXor { dst, lhs, rhs } if rhs == t => (dst, RmwOp::Xor, lhs),
                // Float RMW (nbody's `v[i] = v[i] + dx*mag`): the array holds
                // f64 bits; the fused op reinterprets. Same commutativity rules.
                MicroOp::AddF { dst, lhs, rhs } if lhs == t => (dst, RmwOp::AddF, rhs),
                MicroOp::AddF { dst, lhs, rhs } if rhs == t => (dst, RmwOp::AddF, lhs),
                MicroOp::SubF { dst, lhs, rhs } if lhs == t => (dst, RmwOp::SubF, rhs),
                MicroOp::MulF { dst, lhs, rhs } if lhs == t => (dst, RmwOp::MulF, rhs),
                MicroOp::MulF { dst, lhs, rhs } if rhs == t => (dst, RmwOp::MulF, lhs),
                _ => continue,
            };
            let MicroOp::ArrStore { src, idx: i2, ptr_slot: p2, len_slot: l2, byte: false, checked: c2 } =
                micro[k + 2]
            else {
                continue;
            };
            // Same array + index, and the store consumes exactly the ALU result.
            if src != t2 || i2 != i || p2 != p || l2 != l {
                continue;
            }
            // `t` and `t2` are single-use scratch we can erase; `operand` is
            // re-read by the fused op (so it must NOT be one of them), and the
            // index `i` likewise stays live.
            if !removable(t, &reads) || !removable(t2, &reads) || operand == t || operand == t2 {
                continue;
            }
            hit = Some((
                k,
                MicroOp::ArrRMW { idx: i, operand, ptr_slot: p, len_slot: l, op, checked: c1 || c2 },
            ));
            break;
        }
        let Some((k, rmw)) = hit else { break };
        if std::env::var_os("LOGOS_FUSE_TRACE").is_some() {
            eprintln!("fuse-trace: ARRRMW {rmw:?}");
        }
        micro[k] = rmw;
        micro.remove(k + 2);
        micro.remove(k + 1);
        // Two ops vanished at k+1, k+2 (neither a jump target). Shift targets
        // above them down; a jump to the fused op (k) stays put.
        for op in micro.iter_mut() {
            if let Some(t) = micro_target_mut(op) {
                if *t > k + 2 {
                    *t -= 2;
                } else if *t > k {
                    *t -= 1;
                }
            }
        }
    }
}

/// TWO-BUFFER INTEGER LOAD+BINOP FUSION — collapses `ArrLoad(t1 = a[i]); …;
/// ArrLoad(t2 = b[j]); …; {Add,Sub,Mul}(dst = t1 OP t2)` (the matrix-multiply /
/// dot-product idiom `c += a[..] * b[..]`) into one [`MicroOp::ArrLoad2`] placed
/// at the binop: ONE stencil dispatch + ONE combined bounds gate instead of
/// three ops, and the two loaded i64s never round-trip the frame. The JIT is
/// per-op DISPATCH-bound, so folding three hot inner-loop ops into one is a
/// direct win on the indexed arithmetic loops (matrix_mult) that lose worst to
/// V8. The two loads need NOT be adjacent (matmul computes `b`'s index between
/// them) — only their values, indices, and buffers must reach the binop
/// unchanged. The two buffers may be distinct (`a`, `b`) or the same (a self dot
/// product); each must be a pinned 8-byte INT buffer.
///
/// SOUNDNESS — the fused op evaluates `a[i-1] OP b[j-1]` with the kernel's
/// wrapping i64 semantics, bit-identical to the original ArrLoad+ArrLoad+ALU,
/// provided the two loaded values are exactly the ones the binop consumed:
/// - both loaded scratch values `t1`/`t2` are SINGLE-USE (read only by the
///   binop), distinct, un-named, un-pinned — so erasing their frame writes
///   loses no observable value (the `copy_propagate` staleness contract);
/// - between the EARLIER load and the binop, NOTHING redefines either index
///   (`i`/`j`) or either pointer/length slot, and NOTHING writes memory (no
///   array store/RMW/swap/push, map set, or list rebuild) — so neither buffer
///   nor index can change under the loads being hoisted to the binop site;
/// - no block leader (jump target) lands strictly AFTER the earlier load up to
///   and including the binop — otherwise an entry would skip a load.
/// The combined `checked = c1 || c2` is at least as safe as the pair (an
/// unchecked load proven in-bounds is in range by hypothesis, so re-checking it
/// never fires). The fused op side-exits BEFORE any effect on out-of-bounds and,
/// in a replay-from-head region, the deopt re-runs the whole region from its
/// entry — so moving the bounds gates to the binop site changes no observable
/// result (the only intervening ops are pure arithmetic on dead scratch).
/// Add/Mul commute (`t1`/`t2` in either order); Sub fuses only `a[i] - b[j]`
/// (the first load on the left). BYTE (Bool) buffers never fuse — the fused
/// stencil reads raw i64s. Caller restricts this to NON-PRECISE regions (it
/// renumbers micro indices).
fn fuse_array_ld2(
    micro: &mut Vec<MicroOp>,
    named: &[bool],
    register_count: u16,
    pinned: &std::collections::HashSet<u16>,
) {
    if micro.iter().any(|op| matches!(op, MicroOp::Call { .. } | MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. })) {
        return;
    }
    let removable = |slot: u16, reads: &std::collections::HashMap<u16, usize>| -> bool {
        slot < register_count
            && !named.get(slot as usize).copied().unwrap_or(true)
            && !pinned.contains(&slot)
            && reads.get(&slot).copied().unwrap_or(0) == 1
    };
    // An op that writes anywhere in memory (any pinned array / map / list) —
    // we cannot statically tell which buffer it aliases, so any such op between
    // a load and the binop blocks the hoist conservatively.
    let writes_memory = |op: &MicroOp| -> bool {
        matches!(
            op,
            MicroOp::ArrStore { .. }
                | MicroOp::ArrRMW { .. }
                | MicroOp::ArrSwap { .. }
                | MicroOp::ArrCondSwap { .. }
                | MicroOp::ArrPush { .. }
                | MicroOp::MapSet { .. }
                | MicroOp::NewList { .. }
                | MicroOp::ListClear { .. }
                | MicroOp::ListTriple { .. }
        )
    };
    loop {
        let n = micro.len();
        let mut is_target = vec![false; n];
        for op in micro.iter() {
            if let Some(t) = micro_target_const(op) {
                if t < n {
                    is_target[t] = true;
                }
            }
        }
        let mut reads: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for op in micro.iter() {
            for s in micro_read_slots(op) {
                *reads.entry(s).or_insert(0) += 1;
            }
        }
        // Anchor on the binop; locate the two ArrLoads that define its operands.
        let mut hit: Option<(usize, usize, usize, MicroOp)> = None;
        'anchor: for m in 0..n {
            let (binop_kind, dst, lhs, rhs) = match micro[m] {
                MicroOp::Add { dst, lhs, rhs } => (IOp::Add, dst, lhs, rhs),
                MicroOp::Sub { dst, lhs, rhs } => (IOp::Sub, dst, lhs, rhs),
                MicroOp::Mul { dst, lhs, rhs } => (IOp::Mul, dst, lhs, rhs),
                _ => continue,
            };
            if lhs == rhs || dst == lhs || dst == rhs {
                continue;
            }
            // The most-recent ArrLoad defining a slot strictly before m, if its
            // value provably still holds at m (single-use scratch, and the load
            // is not itself re-defined in between — guaranteed by single-use of
            // its dst plus the no-rebind interval check below).
            let find_load = |slot: u16| -> Option<usize> {
                (0..m).rev().find(|&k| micro_defs(&micro[k]).contains(&slot))
            };
            let (Some(ka), Some(kb)) = (find_load(lhs), find_load(rhs)) else { continue };
            if ka == kb {
                continue;
            }
            let MicroOp::ArrLoad { dst: _, idx: ia, ptr_slot: pa, len_slot: lena, byte: false, checked: ca } =
                micro[ka]
            else {
                continue;
            };
            let MicroOp::ArrLoad { dst: _, idx: ib, ptr_slot: pb, len_slot: lenb, byte: false, checked: cb } =
                micro[kb]
            else {
                continue;
            };
            // For Sub the order is fixed: lhs must be `a[ia]` (the load at ka).
            // Add/Mul commute. We always store the operand that came from ka as
            // `a`/`i` and from kb as `b`/`j` so `eval` reproduces lhs OP rhs.
            // (Sub: lhs == a[ia], rhs == b[ib] ⇒ a - b. Correct by construction.)
            // Both scratch loads must be single-use removable; dst already
            // checked distinct from both above.
            if !removable(lhs, &reads) || !removable(rhs, &reads) {
                continue;
            }
            // Interval soundness. The fused op sits at the binop and reads each
            // index + pointer/length from the frame at THAT point, so each
            // load's index/buffer must reach the binop unchanged FROM ITS OWN
            // LOAD onward (the index is naturally produced just before the load
            // — that write is fine). Concretely:
            //  - NO jump target in `[lo, m]` (the loads are removed and the fused
            //    op moves to the binop, so any entry there would skip a load);
            //  - `ia`,`pa`,`lena` not redefined in `(ka, m)`; `ib`,`pb`,`lenb`
            //    not redefined in `(kb, m)`;
            //  - NO memory write in `[lo, m)` (could mutate either buffer).
            let lo = ka.min(kb);
            let guard_a: [u16; 3] = [ia, pa, lena];
            let guard_b: [u16; 3] = [ib, pb, lenb];
            for p in lo..m {
                if p < n && is_target[p] {
                    continue 'anchor;
                }
                if p == ka || p == kb {
                    continue;
                }
                if writes_memory(&micro[p]) {
                    continue 'anchor;
                }
                let defs = micro_defs(&micro[p]);
                if p > ka && defs.iter().any(|d| guard_a.contains(d)) {
                    continue 'anchor;
                }
                if p > kb && defs.iter().any(|d| guard_b.contains(d)) {
                    continue 'anchor;
                }
            }
            // A jump target landing exactly on the binop would skip both loads.
            if m < n && is_target[m] {
                continue 'anchor;
            }
            hit = Some((
                ka,
                kb,
                m,
                MicroOp::ArrLoad2 {
                    dst,
                    i: ia,
                    j: ib,
                    ptr_a: pa,
                    len_a: lena,
                    ptr_b: pb,
                    len_b: lenb,
                    op: binop_kind,
                    checked: ca || cb,
                },
            ));
            break;
        }
        let Some((ka, kb, m, ld2)) = hit else { break };
        if std::env::var_os("LOGOS_FUSE_TRACE").is_some() {
            eprintln!("fuse-trace: ARRLD2 {ld2:?}");
        }
        // Place the fused op at the binop, then remove the two loads (higher
        // index first so the lower index stays valid).
        micro[m] = ld2;
        let (hi, low) = if ka > kb { (ka, kb) } else { (kb, ka) };
        micro.remove(hi);
        micro.remove(low);
        // Two ops vanished at `low` and `hi` (neither a jump target — the
        // interval check forbids targets after the earlier load). Shift later
        // targets down by however many removed ops precede them. The fused op
        // at `m` is itself shifted by the removals below it.
        for op in micro.iter_mut() {
            if let Some(t) = micro_target_mut(op) {
                let shift = (*t > low) as usize + (*t > hi) as usize;
                *t -= shift;
            }
        }
    }
}

/// AFFINE-INDEX LOAD FUSION — folds the index-arithmetic chain that builds a
/// read-only array index INTO the load, so `<binop>(t = a OP b); Add(t2 = t +
/// Kc); ArrLoad(arr[t2])` (the `w - wi + 1`, `i*n + j + 1` shape) — or the
/// shorter `Add(t = a + Kc); ArrLoad(arr[t])` (`w + 1`) and the bare
/// `<binop>(t = a OP b); ArrLoad(arr[t])` (`i*n + j`) — collapse into one
/// [`MicroOp::ArrLoadAffine`] stencil that computes the 1-based index and loads
/// in a single dispatch. The JIT is per-op DISPATCH-bound, so removing the 1-2
/// index-arithmetic ops per access is a direct win on the indexed inner loops
/// (knapsack `prev[w - wi + 1]`, the heap-sift read region, graph adjacency
/// reads) that lose worst to V8.
///
/// SOUNDNESS — the fused stencil recomputes `idx = (frame[a] OP frame[b])
/// .wrapping_add(const_offset)` (or `frame[a] + const_offset` for the no-`b`
/// shape) with the kernel's EXACT i64 wrapping semantics, bit-identical to the
/// chain it replaces, PROVIDED the surviving operands reach the load unchanged:
/// - the load's index slot is SINGLE-USE scratch (read only by this load), so
///   erasing the index-producing op loses no observable value;
/// - the trailing-const `Add`'s product `t` (the two-slot binop's dst) is also
///   single-use scratch, so the inner binop is removable too;
/// - the constant offset is a LoadConst VALUE whose source slot is NOT redefined
///   between its load and the index `Add` (the LoadConst itself is KEPT — it is
///   usually reused — only its value is folded);
/// - between each removed op and the load NOTHING redefines a surviving operand
///   (`a`, `b`) and NO jump target lands in the removed span (an entry there
///   would skip a removed op).
/// The fused op SIDE-EXITS before any effect on out-of-bounds, identical to the
/// plain [`MicroOp::ArrLoad`] it replaces (replay-from-head recomputes the same
/// index). BYTE (Bool) buffers never fuse (the affine stencils read raw i64s).
/// Runs AFTER `fuse_array_ld2` so the matmul `a[..] * b[..]` two-buffer loads
/// keep fusing into `ArrLoad2` (which consumes the raw `ArrLoad`s) — this pass
/// only claims the plain read-only `ArrLoad`s that `ArrLoad2`/swap fusion left
/// behind. Caller restricts to NON-PRECISE regions (it renumbers micro indices).
/// `LOGOS_AFFINE=0` is the kill-switch.
fn fuse_index_affine(
    micro: &mut Vec<MicroOp>,
    named: &[bool],
    register_count: u16,
    pinned: &std::collections::HashSet<u16>,
) {
    use logicaffeine_forge::jit::AffOp;
    if micro.iter().any(|op| matches!(op, MicroOp::Call { .. } | MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. })) {
        return;
    }
    // A scratch temp removable iff it is in-range, un-named, un-pinned, and
    // read EXACTLY once (its only reader is the op we erase or replace).
    let single_use_scratch = |slot: u16, reads: &std::collections::HashMap<u16, usize>| -> bool {
        (slot as usize) < register_count as usize
            && !named.get(slot as usize).copied().unwrap_or(true)
            && !pinned.contains(&slot)
            && reads.get(&slot).copied().unwrap_or(0) == 1
    };
    // The integer binop family that can build an affine index.
    let as_iop = |op: &MicroOp| -> Option<(IOp, u16, u16, u16)> {
        match *op {
            MicroOp::Add { dst, lhs, rhs } => Some((IOp::Add, dst, lhs, rhs)),
            MicroOp::Sub { dst, lhs, rhs } => Some((IOp::Sub, dst, lhs, rhs)),
            MicroOp::Mul { dst, lhs, rhs } => Some((IOp::Mul, dst, lhs, rhs)),
            _ => None,
        }
    };
    let iop_to_aff = |op: IOp| match op {
        IOp::Add => AffOp::Add,
        IOp::Sub => AffOp::Sub,
        IOp::Mul => AffOp::Mul,
    };
    loop {
        let n = micro.len();
        let mut is_target = vec![false; n];
        for op in micro.iter() {
            if let Some(t) = micro_target_const(op) {
                if t < n {
                    is_target[t] = true;
                }
            }
        }
        let mut reads: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for op in micro.iter() {
            for s in micro_read_slots(op) {
                *reads.entry(s).or_insert(0) += 1;
            }
        }
        // The reaching constant value of a slot at position `m`, if it is a
        // LoadConst not redefined in `(def, m)`. Returns (value, def_index).
        let reaching_const = |slot: u16, m: usize| -> Option<(i64, usize)> {
            let k = (0..m).rev().find(|&p| micro_defs(&micro[p]).contains(&slot))?;
            if let MicroOp::LoadConst { value, .. } = micro[k] {
                Some((value, k))
            } else {
                None
            }
        };
        // The reaching def index of a slot strictly before `m`.
        let reaching_def = |slot: u16, m: usize| -> Option<usize> {
            (0..m).rev().find(|&p| micro_defs(&micro[p]).contains(&slot))
        };
        // No jump target lands in [lo, m]; no surviving operand redefined in the
        // open spans after each removed op up to the load.
        let mut hit: Option<(Vec<usize>, usize, MicroOp)> = None;
        'anchor: for m in 0..n {
            let MicroOp::ArrLoad { dst, idx, ptr_slot, len_slot, byte: false, checked } = micro[m]
            else {
                continue;
            };
            // The index must be single-use scratch defined by an integer binop.
            if !single_use_scratch(idx, &reads) {
                continue;
            }
            let Some(kidx) = reaching_def(idx, m) else { continue };
            let Some((idx_op, _idst, ilhs, irhs)) = as_iop(&micro[kidx]) else { continue };

            // Decompose into (a, op, b, const_offset) + the chain ops to remove.
            // Three shapes, tried in order of specificity.
            let mut removed: Vec<usize> = Vec::new();
            let (a, aff_op, b, coff): (u16, AffOp, u16, i64);

            // Does either operand of the index binop come from an inner two-slot
            // binop whose dst is single-use (the `t` of `t + Kc`)? If the OTHER
            // operand is a reaching const, this is the trailing-const shape.
            let inner_of = |slot: u16| -> Option<(usize, IOp, u16, u16)> {
                if !single_use_scratch(slot, &reads) {
                    return None;
                }
                let k = reaching_def(slot, kidx)?;
                let (op, d, l, r) = as_iop(&micro[k])?;
                Some((k, op, l, r)).filter(|_| d == slot)
            };

            if idx_op == IOp::Add {
                // `Add(idx = LHS + RHS)`: classify each side.
                let lconst = reaching_const(ilhs, kidx);
                let rconst = reaching_const(irhs, kidx);
                let linner = inner_of(ilhs);
                let rinner = inner_of(irhs);
                if let (Some((kc, _)), Some((kin, iop, l, r))) = (rconst, linner) {
                    // `(l OP r) + Kc` — Kc on the right.
                    let _ = kc;
                    a = l; b = r; aff_op = iop_to_aff(iop); coff = rconst.unwrap().0;
                    removed.push(kin);
                    removed.push(kidx);
                } else if let (Some((kc, _)), Some((kin, iop, l, r))) = (lconst, rinner) {
                    // `Kc + (l OP r)` — Kc on the left.
                    let _ = kc;
                    a = l; b = r; aff_op = iop_to_aff(iop); coff = lconst.unwrap().0;
                    removed.push(kin);
                    removed.push(kidx);
                } else if let Some((_, _)) = rconst {
                    // `a + Kc` (the `w + 1` shape): single slot + const.
                    a = ilhs; b = ilhs; aff_op = AffOp::None; coff = rconst.unwrap().0;
                    removed.push(kidx);
                } else if let Some((_, _)) = lconst {
                    a = irhs; b = irhs; aff_op = AffOp::None; coff = lconst.unwrap().0;
                    removed.push(kidx);
                } else {
                    // `a + b` (no const): a bare two-slot add.
                    a = ilhs; b = irhs; aff_op = AffOp::Add; coff = 0;
                    removed.push(kidx);
                }
            } else {
                // `Sub`/`Mul(idx = a OP b)` with no trailing const (the const,
                // when present, is always the OUTER `+ Kc` Add handled above; a
                // bare Sub/Mul index folds with const_offset = 0).
                a = ilhs; b = irhs; aff_op = iop_to_aff(idx_op); coff = 0;
                removed.push(kidx);
            }

            // Interval soundness over the union of removed ops and the load.
            let lo = *removed.iter().min().unwrap();
            let survivors: [u16; 2] = [a, b];
            for p in lo..m {
                if p < n && is_target[p] {
                    continue 'anchor;
                }
                if removed.contains(&p) {
                    continue;
                }
                let defs = micro_defs(&micro[p]);
                if defs.iter().any(|d| survivors.contains(d)) {
                    continue 'anchor;
                }
            }
            // A jump target landing exactly on the load would skip the removed
            // index ops.
            if m < n && is_target[m] {
                continue 'anchor;
            }
            removed.sort_unstable();
            hit = Some((
                removed,
                m,
                MicroOp::ArrLoadAffine {
                    dst,
                    a,
                    op: aff_op,
                    b,
                    const_offset: coff,
                    ptr_slot,
                    len_slot,
                    checked,
                },
            ));
            break;
        }
        let Some((removed, m, affine)) = hit else { break };
        if std::env::var_os("LOGOS_FUSE_TRACE").is_some() {
            eprintln!("fuse-trace: AFFINE {affine:?}");
        }
        // Place the fused op at the load, then remove the index-chain ops
        // (highest index first so lower indices stay valid). None of the removed
        // ops is a jump target (the interval check forbids targets in [lo, m]).
        micro[m] = affine;
        for &r in removed.iter().rev() {
            micro.remove(r);
        }
        for op in micro.iter_mut() {
            if let Some(t) = micro_target_mut(op) {
                let shift = removed.iter().filter(|&&r| r < *t).count();
                *t -= shift;
            }
        }
    }
}

/// FLOAT MULTIPLY-ADD FUSION — collapses `MulF(t = a*b); AddF(d = t + c)` (the
/// product `t` single-use, AddF commutative so `t` may be either operand) into
/// one [`MicroOp::FmaF`] computing `(a*b) + c`. The JIT is per-op DISPATCH-bound,
/// and the float arithmetic chains that dominate the float cluster (nbody's
/// `dz*dz + sum`, dot products) are a stream of two-input AddF/MulF; merging a
/// product into its consuming add removes a stencil dispatch per occurrence.
///
/// SOUNDNESS: FmaF rounds the product and the add SEPARATELY (`(a*b)+c`, two
/// roundings — NOT a hardware single-rounding FMA), bit-identical to MulF+AddF.
/// It is mem-form (all operands read from the frame), so it never disturbs the
/// threaded XMM pins. To keep it a SPILL-FREE win on its INPUTS, fuse only when
/// the surviving inputs (`a`, `b`, the addend `c`) are FRAME-RESIDENT (not
/// pinned) — a pinned input would be spilled BEFORE the mem-form op, possibly
/// costing more than the saved dispatch. The RESULT `d` MAY be pinned (Lever
/// 1a): the pinned compiler's mem-form path reloads a pinned `d` from the frame
/// after the FmaF, exactly as the unfused register-form `AddF` would settle it,
/// so a pinned `d` adds no input spill (the nbody distance-sum's `dz*dz +
/// partial`, whose `d` is the float-pinned running sum). The product `t` must be
/// single-use, un-named, un-pinned scratch (its only reader, the AddF, is
/// removed). Caller restricts to NON-PRECISE regions (renumbering).
fn fuse_fma(
    micro: &mut Vec<MicroOp>,
    named: &[bool],
    register_count: u16,
    pinned: &std::collections::HashSet<u16>,
) {
    if micro.iter().any(|op| matches!(op, MicroOp::Call { .. } | MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. })) {
        return;
    }
    let frame_resident = |slot: u16| -> bool { !pinned.contains(&slot) };
    loop {
        let n = micro.len();
        let mut is_target = vec![false; n];
        for op in micro.iter() {
            if let Some(t) = micro_target_const(op) {
                if t < n {
                    is_target[t] = true;
                }
            }
        }
        let mut reads: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for op in micro.iter() {
            for s in micro_read_slots(op) {
                *reads.entry(s).or_insert(0) += 1;
            }
        }
        let mut hit: Option<(usize, MicroOp)> = None;
        for k in 0..n.saturating_sub(1) {
            if is_target[k + 1] {
                continue;
            }
            let MicroOp::MulF { dst: t, lhs: a, rhs: b } = micro[k] else { continue };
            // The AddF consumes the product `t` (either operand) plus the addend.
            let (d, c) = match micro[k + 1] {
                MicroOp::AddF { dst, lhs, rhs } if lhs == t => (dst, rhs),
                MicroOp::AddF { dst, lhs, rhs } if rhs == t => (dst, lhs),
                _ => continue,
            };
            // `t` is single-use, un-named, un-pinned scratch we can erase.
            let t_dead = (t as usize) < register_count as usize
                && !named.get(t as usize).copied().unwrap_or(true)
                && !pinned.contains(&t)
                && reads.get(&t).copied().unwrap_or(0) == 1;
            // INPUT-spill-free (Lever 1a): require only the surviving INPUTS
            // (`a`, `b`, the addend `c`) frame-resident. `FmaF` is mem-form, so a
            // pinned RESULT `d` is spilled/reloaded by the pinned compiler's
            // mem-form path exactly as the unfused `AddF`'s register-form write
            // would settle it — fusing a pinned `d` adds no INPUT spill, removing
            // the `MulF` dispatch (the nbody distance-sum's `dz*dz + partial`,
            // whose `d` is the float-pinned sum). The addend `c` must NOT be
            // pinned: in the `s = s + a*b` accumulator shape `c == d == s`, a
            // pinned `c` would be spilled BEFORE the FmaF (a real extra spill),
            // costing more pieces than the register-form `AddF` it replaces.
            if !t_dead || !frame_resident(a) || !frame_resident(b) || !frame_resident(c) {
                continue;
            }
            hit = Some((k, MicroOp::FmaF { dst: d, a, b, c }));
            break;
        }
        let Some((k, fma)) = hit else { break };
        if std::env::var_os("LOGOS_FUSE_TRACE").is_some() {
            eprintln!("fuse-trace: FMAF {fma:?}");
        }
        micro[k] = fma;
        micro.remove(k + 1);
        for op in micro.iter_mut() {
            if let Some(t) = micro_target_mut(op) {
                if *t > k {
                    *t -= 1;
                }
            }
        }
    }
}

/// CONDITIONAL-SWAP FUSION — collapses the sort inner-loop idiom
/// `ArrLoad(a, i1); [pure index ops]; ArrLoad(b, i2); Branch(cmp, a, b, skip);
/// ArrStore(i1, b); ArrStore(i2, a)` into one [`MicroOp::ArrCondSwap`] — the
/// biggest per-iteration dispatch sink for bubble/insertion/quick sort. Unlocked
/// by the precise-region-live-out keystone: the loaded values flow through
/// loop-local `Let a`/`Let b`, which are now dropped from write-back, so
/// copy_propagate erases their Moves and the loaded temps become single-use.
///
/// SOUNDNESS: `ArrCondSwap` re-reads `buf[i1]`/`buf[i2]` and swaps atomically
/// (both writes or neither) — bit-identical to the load/compare/cross-store it
/// replaces because NOTHING writes the array between the original loads and the
/// stores (only the pure index ops and the branch sit there). One pre-write
/// bounds check covers both indices. The loaded `a`, `b` must be single-use
/// scratch (read only by the branch + their cross-store — exactly two reads
/// each — un-named, un-pinned), the branch must target precisely past both
/// stores, the comparison must be an ordering (Gt/Lt/GtEq/LtEq), and up to two
/// PURE ops (a const + the `i+1` add) may sit between the loads. No fused op may
/// be a jump target. Non-precise regions only.
fn fuse_cond_swap(
    micro: &mut Vec<MicroOp>,
    named: &[bool],
    register_count: u16,
    pinned: &std::collections::HashSet<u16>,
) {
    if micro.iter().any(|op| matches!(op, MicroOp::Call { .. } | MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. })) {
        return;
    }
    let dead_scratch = |slot: u16, reads: &std::collections::HashMap<u16, usize>| -> bool {
        (slot as usize) < register_count as usize
            && !named.get(slot as usize).copied().unwrap_or(true)
            && !pinned.contains(&slot)
            && reads.get(&slot).copied().unwrap_or(0) == 2 // the branch + its cross-store
    };
    loop {
        let n = micro.len();
        let mut is_target = vec![false; n];
        for op in micro.iter() {
            if let Some(t) = micro_target_const(op) {
                if t < n {
                    is_target[t] = true;
                }
            }
        }
        let mut reads: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for op in micro.iter() {
            for s in micro_read_slots(op) {
                *reads.entry(s).or_insert(0) += 1;
            }
        }
        let mut hit: Option<(usize, usize, MicroOp)> = None; // (k, gap, condswap)
        // The compare's SECOND index can be a multi-op affine computation
        // (`2*root+2` = LoadConst + Mul + Add, or `i*n + j + 1`) sitting between
        // the two loads. The `between_ok` purity guard below — not this window —
        // is what proves the gap ops cannot perturb the swap (pure, and they
        // redefine none of the first loaded value / first index / pointer /
        // length); the window only bounds the linear search, so it must be wide
        // enough to span a realistic index expression.
        const MAXGAP: usize = 5;
        'outer: for k in 0..n {
            let MicroOp::ArrLoad { dst: a, idx: i1, ptr_slot: p, len_slot: l, byte: false, checked: c1 } =
                micro[k]
            else {
                continue;
            };
            for gap in 0..=MAXGAP {
                let lb = k + 1 + gap;
                if lb + 3 >= n {
                    break;
                }
                let between_ok = (k + 1..lb).all(|x| {
                    let pure = micro_pure_value_key(&micro[x]).is_some()
                        || matches!(micro[x], MicroOp::LoadConst { .. });
                    let defs = micro_defs(&micro[x]);
                    pure && !defs.contains(&a) && !defs.contains(&i1) && !defs.contains(&p)
                        && !defs.contains(&l)
                });
                if !between_ok {
                    continue;
                }
                let MicroOp::ArrLoad { dst: b, idx: i2, ptr_slot: p2, len_slot: l2, byte: false, checked: c2 } =
                    micro[lb]
                else {
                    continue;
                };
                if p2 != p || l2 != l {
                    continue;
                }
                let MicroOp::Branch { cmp, lhs, rhs, target } = micro[lb + 1] else { continue };
                if !matches!(cmp, Cmp::Lt | Cmp::Gt | Cmp::LtEq | Cmp::GtEq) || lhs != a || rhs != b {
                    continue;
                }
                let MicroOp::ArrStore { src: s1, idx: si1, ptr_slot: sp1, len_slot: sl1, byte: false, checked: c3 } =
                    micro[lb + 2]
                else {
                    continue;
                };
                let MicroOp::ArrStore { src: s2, idx: si2, ptr_slot: sp2, len_slot: sl2, byte: false, checked: c4 } =
                    micro[lb + 3]
                else {
                    continue;
                };
                if si1 != i1 || si2 != i2 || s1 != b || s2 != a || sp1 != p || sp2 != p || sl1 != l || sl2 != l {
                    continue;
                }
                if target != lb + 4 {
                    continue;
                }
                if (k..=lb + 3).any(|x| is_target.get(x).copied().unwrap_or(false)) {
                    continue;
                }
                if !dead_scratch(a, &reads) || !dead_scratch(b, &reads) {
                    continue;
                }
                hit = Some((
                    k,
                    gap,
                    MicroOp::ArrCondSwap {
                        idx1: i1,
                        idx2: i2,
                        ptr_slot: p,
                        len_slot: l,
                        cmp,
                        checked: c1 || c2 || c3 || c4,
                    },
                ));
                break 'outer;
            }
        }
        let Some((k, gap, swap)) = hit else { break };
        if std::env::var_os("LOGOS_FUSE_TRACE").is_some() {
            eprintln!("fuse-trace: CONDSWAP {swap:?}");
        }
        // Span k ..= k+4+gap. Keep the between ops, turn the b-load slot into the
        // swap, drop a-load + branch + both stores (4 removed). First op after the
        // span (old k+gap+5) lands at k+gap+1.
        micro[k + gap + 1] = swap;
        micro.drain(k + gap + 2..=k + gap + 4);
        micro.remove(k);
        for op in micro.iter_mut() {
            if let Some(t) = micro_target_mut(op) {
                if *t >= k + gap + 5 {
                    *t -= 4;
                }
            }
        }
    }
}

/// UNCONDITIONAL-SWAP FUSION — collapses the `Let tmp = arr[i]; arr[i] = arr[j];
/// arr[j] = tmp` exchange (quicksort/heap_sort/mergesort) — micro
/// `ArrLoad(a, i1); ArrLoad(b, i2); ArrStore(i1, b); ArrStore(i2, a)` — into one
/// [`MicroOp::ArrSwap`] (4 ops → 1). The conditionality (`if arr[j] <= pivot`)
/// is a SEPARATE branch before the loads and stays.
///
/// SOUNDNESS: `ArrSwap` re-reads `buf[i1]`/`buf[i2]` and swaps atomically —
/// bit-identical because nothing writes the array between the loads and stores.
/// `a`, `b` must be LOCALLY single-use (each read only by its cross-store before
/// its next redefinition — a slot may be REUSED later, so global read-counting
/// is too coarse here), un-named, un-pinned. No control flow between the four
/// ops; none a jump target. Non-precise regions only.
fn fuse_swap(
    micro: &mut Vec<MicroOp>,
    named: &[bool],
    register_count: u16,
    pinned: &std::collections::HashSet<u16>,
) {
    if micro.iter().any(|op| matches!(op, MicroOp::Call { .. } | MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. })) {
        return;
    }
    let erasable = |slot: u16| -> bool {
        (slot as usize) < register_count as usize
            && !named.get(slot as usize).copied().unwrap_or(true)
            && !pinned.contains(&slot)
    };
    loop {
        let n = micro.len();
        let mut is_target = vec![false; n];
        for op in micro.iter() {
            if let Some(t) = micro_target_const(op) {
                if t < n {
                    is_target[t] = true;
                }
            }
        }
        // `slot` (defined at def_i) is read ONLY at `want` before its next redef.
        let local_single_use = |slot: u16, def_i: usize, want: usize| -> bool {
            let mut seen = false;
            for j in def_i + 1..micro.len() {
                if micro_read_slots(&micro[j]).contains(&slot) {
                    if j != want {
                        return false;
                    }
                    seen = true;
                }
                if micro_defs(&micro[j]).contains(&slot) {
                    break; // next redefinition — a separate live range
                }
            }
            seen
        };
        let mut hit: Option<(usize, MicroOp)> = None;
        for k in 0..n.saturating_sub(3) {
            let MicroOp::ArrLoad { dst: a, idx: i1, ptr_slot: p, len_slot: l, byte: false, checked: c1 } =
                micro[k]
            else {
                continue;
            };
            let MicroOp::ArrLoad { dst: b, idx: i2, ptr_slot: p2, len_slot: l2, byte: false, checked: c2 } =
                micro[k + 1]
            else {
                continue;
            };
            if p2 != p || l2 != l || a == b {
                continue;
            }
            let MicroOp::ArrStore { src: s1, idx: si1, ptr_slot: sp1, len_slot: sl1, byte: false, checked: c3 } =
                micro[k + 2]
            else {
                continue;
            };
            let MicroOp::ArrStore { src: s2, idx: si2, ptr_slot: sp2, len_slot: sl2, byte: false, checked: c4 } =
                micro[k + 3]
            else {
                continue;
            };
            // Cross-store swap to the SAME array+indices: buf[i1]=b, buf[i2]=a.
            if si1 != i1 || si2 != i2 || s1 != b || s2 != a || sp1 != p || sp2 != p || sl1 != l || sl2 != l {
                continue;
            }
            if is_target[k + 1] || is_target[k + 2] || is_target[k + 3] {
                continue;
            }
            // `a` consumed only by the store at k+3, `b` only by the store at k+2.
            if !erasable(a) || !erasable(b) || !local_single_use(a, k, k + 3) || !local_single_use(b, k + 1, k + 2) {
                continue;
            }
            hit = Some((
                k,
                MicroOp::ArrSwap {
                    idx1: i1,
                    idx2: i2,
                    ptr_slot: p,
                    len_slot: l,
                    checked: c1 || c2 || c3 || c4,
                },
            ));
            break;
        }
        let Some((k, swap)) = hit else { break };
        if std::env::var_os("LOGOS_FUSE_TRACE").is_some() {
            eprintln!("fuse-trace: ARRSWAP {swap:?}");
        }
        micro[k] = swap;
        micro.drain(k + 1..=k + 3); // remove the 2nd load + both stores
        for op in micro.iter_mut() {
            if let Some(t) = micro_target_mut(op) {
                if *t >= k + 4 {
                    *t -= 3;
                }
            }
        }
    }
}


/// A canonical value key for a PURE, side-effect-free, deterministic micro-op —
/// the basis for local value numbering (CSE) in [`copy_propagate`]. Returns
/// `((tag, a, b), dst)`: a unique op tag, the two operand slots (commutative ops
/// sort them so `v + 1` and `1 + v` share a key), and the result slot. Returns
/// None for anything that loads memory, calls, side-exits (Div/Mod, whose
/// duplicate could diverge on a zero divisor), branches, or carries hidden
/// state — those must never be deduplicated. Every operand position is a real
/// frame slot (never an immediate), so the invalidation in `copy_propagate` can
/// drop a key the moment any slot it names is redefined.
fn micro_pure_value_key(op: &MicroOp) -> Option<((u8, u16, u16), u16)> {
    use MicroOp::*;
    let comm = |t: u8, a: u16, b: u16, d: u16| Some(((t, a.min(b), a.max(b)), d));
    let ord = |t: u8, a: u16, b: u16, d: u16| Some(((t, a, b), d));
    match *op {
        Add { dst, lhs, rhs } => comm(1, lhs, rhs, dst),
        Mul { dst, lhs, rhs } => comm(2, lhs, rhs, dst),
        BitAnd { dst, lhs, rhs } => comm(3, lhs, rhs, dst),
        BitOr { dst, lhs, rhs } => comm(4, lhs, rhs, dst),
        BitXor { dst, lhs, rhs } => comm(5, lhs, rhs, dst),
        Eq { dst, lhs, rhs } => comm(6, lhs, rhs, dst),
        Neq { dst, lhs, rhs } => comm(7, lhs, rhs, dst),
        Sub { dst, lhs, rhs } => ord(8, lhs, rhs, dst),
        Shl { dst, lhs, rhs } => ord(9, lhs, rhs, dst),
        Shr { dst, lhs, rhs } => ord(10, lhs, rhs, dst),
        Lt { dst, lhs, rhs } => ord(11, lhs, rhs, dst),
        Gt { dst, lhs, rhs } => ord(12, lhs, rhs, dst),
        LtEq { dst, lhs, rhs } => ord(13, lhs, rhs, dst),
        GtEq { dst, lhs, rhs } => ord(14, lhs, rhs, dst),
        NotInt { dst, src } => ord(15, src, src, dst),
        NotBool { dst, src } => ord(16, src, src, dst),
        _ => None,
    }
}

/// The branch/jump TARGET of a micro-op (a micro index), if any — by value.
fn micro_target_const(op: &MicroOp) -> Option<usize> {
    use MicroOp::*;
    match *op {
        Jump { target } | Branch { target, .. } | BranchF { target, .. }
        | JumpIfFalse { target, .. } | JumpIfTrue { target, .. } => Some(target),
        _ => None,
    }
}

/// Returns TWO disjoint pin sets: `(gp_pins, float_pins)`. A slot that ever
/// holds an f64 VALUE (used by any float-arithmetic / float-compare op) is a
/// FLOAT slot — it can only ride an XMM register (f0..f3), so it goes into
/// `float_pins` and is NEVER put in the GP set, which threads i64s through
/// r0..r3. The two budgets are independent (the threaded ABI passes both
/// r0..r3 and f0..f3), so each is ranked and capped at four on its own.
fn select_pins(ops: &[MicroOp], register_count: u16, region: bool) -> (Vec<u16>, Vec<u16>) {
    const BASE: i64 = 16;
    const CAP: u32 = 4;
    let depth = if region { loop_depths(ops) } else { Vec::new() };
    let weight = |i: usize| -> i64 {
        if region {
            BASE.pow(depth[i].min(CAP))
        } else {
            1
        }
    };
    // Every REAL call saves/restores each live pinned register (SysV makes
    // them caller-saved): call-heavy bodies (fib) lose to pinning while
    // loop-heavy bodies (gcd, nqueens' solver) win — charge each slot a
    // per-call penalty, frequency-weighted in regions (a call inside an
    // inner loop is far costlier than one at top level).
    let call_penalty: i64 = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| matches!(op, MicroOp::Call { .. } | MicroOp::CallSelf { .. } | MicroOp::CallSelfCopy { .. }))
        .map(|(i, _)| 3 * weight(i))
        .sum();
    // FLOAT-slot set: every slot that ever holds an f64 VALUE. The float
    // arithmetic ops (AddF/SubF/MulF/DivF) carry floats in all three slots;
    // the float comparisons/branch carry floats only in the OPERANDS (their
    // dst is the Int 0/1 result); IntToFloat produces a float in dst from an
    // Int src; SqrtF and the fused float load produce/consume floats. A float
    // slot can only live in an XMM register, NEVER a GP one (the GP stencils
    // store raw i64 bits and the float arith reads `fN` directly), so EVERY
    // float slot is excluded from the GP budget below.
    //
    // A float slot is XMM-PINNABLE only when every op that touches it as a
    // float is XMM-aware: the V_FBINOP arith threads it through `fN`, and the
    // mem-form float ops (DivF, the float compares/branch, IntToFloat, SqrtF,
    // the fused load) spill/reload it around their frame-form stencils. The
    // register-form ops that resolve operands through the GP location only —
    // `LoadConst`, `Move`, and the function `Return` value — have NO XMM form,
    // so a float slot they touch must stay FRAME-resident (blocked from the
    // float-pin budget); the float arith then reads it from the frame (floc 0)
    // correctly. (`main_float_loop`: the `0.25` constant slot is a LoadConst
    // dst AND an AddF operand — float-valued but pin-blocked.)
    let mut float_slots: std::collections::HashSet<u16> = std::collections::HashSet::new();
    let mut float_blocked: std::collections::HashSet<u16> = std::collections::HashSet::new();
    let mut mark_float = |slot: u16, set: &mut std::collections::HashSet<u16>| {
        if slot < register_count {
            set.insert(slot);
        }
    };
    for op in ops {
        match *op {
            MicroOp::AddF { dst, lhs, rhs }
            | MicroOp::SubF { dst, lhs, rhs }
            | MicroOp::MulF { dst, lhs, rhs }
            | MicroOp::DivF { dst, lhs, rhs } => {
                for s in [dst, lhs, rhs] {
                    mark_float(s, &mut float_slots);
                }
            }
            MicroOp::LtF { lhs, rhs, .. }
            | MicroOp::GtF { lhs, rhs, .. }
            | MicroOp::LtEqF { lhs, rhs, .. }
            | MicroOp::GtEqF { lhs, rhs, .. }
            | MicroOp::EqF { lhs, rhs, .. }
            | MicroOp::NeqF { lhs, rhs, .. }
            | MicroOp::BranchF { lhs, rhs, .. } => {
                for s in [lhs, rhs] {
                    mark_float(s, &mut float_slots);
                }
            }
            MicroOp::IntToFloat { dst, .. } => mark_float(dst, &mut float_slots),
            MicroOp::SqrtF { dst, src } => {
                for s in [dst, src] {
                    mark_float(s, &mut float_slots);
                }
            }
            MicroOp::ArrLoad2F { dst, .. } => mark_float(dst, &mut float_slots),
            // A float value loaded from / stored to an array stays FRAME-RESIDENT
            // (block it from the XMM pin budget): the array access itself is
            // mem-form, so a pinned value would round-trip the frame anyway, and
            // the float-pin + mem-form-array interaction is not yet correct
            // (spectral_norm SIGSEGV). The array load/store still works mem-form.
            MicroOp::ArrLoad { dst, .. } => mark_float(dst, &mut float_blocked),
            MicroOp::ArrStore { src, .. } => mark_float(src, &mut float_blocked),
            // Register-form ops with no XMM lowering: a float slot they touch
            // is pin-blocked (it must read/write through the frame). `Move` is
            // NOT here — it threads XMM pins through V_FMOV (f→f, or frame↔f when
            // one end is unpinned), so a loop-carried float reassigned via `Set
            // acc to next` keeps its register (mandelbrot's `Set zr to zr2`).
            MicroOp::LoadConst { dst, .. } => mark_float(dst, &mut float_blocked),
            MicroOp::Return { src } => mark_float(src, &mut float_blocked),
            _ => {}
        }
    }
    // INTEGER-role slots: any slot read/written as an i64 by a non-float op —
    // int arithmetic, a branch/jump condition, an array index/pointer/length/
    // handle, an `IntToFloat` SOURCE, a map key/handle. A slot that is BOTH
    // float-valued (above) AND integer-role is a TYPE-REUSED scratch cell, and
    // must stay FRAME-resident (never float-pinned): the int stencil writes raw
    // i64 bits to its frame cell while a float pin would keep an f64 in an XMM
    // register, and the mem-form spill of the float pin clobbers the integer
    // the int op wrote. (spectral_norm SIGSEGV: a `MulF` result slot was reused
    // as an `ArrStore` index; the float-pin spill wrote f64 bits over the
    // integer index, so the store indexed through a garbage pointer.) Marking
    // the integer role into `float_blocked` blocks the pin; a pure-integer slot
    // is never in `float_slots`, so this is a no-op for it — only genuinely
    // mixed slots are forced frame-resident.
    for op in ops {
        match *op {
            MicroOp::Add { dst, lhs, rhs }
            | MicroOp::Sub { dst, lhs, rhs }
            | MicroOp::Mul { dst, lhs, rhs }
            | MicroOp::BitAnd { dst, lhs, rhs }
            | MicroOp::BitOr { dst, lhs, rhs }
            | MicroOp::BitXor { dst, lhs, rhs }
            | MicroOp::Shl { dst, lhs, rhs }
            | MicroOp::Shr { dst, lhs, rhs }
            | MicroOp::Lt { dst, lhs, rhs }
            | MicroOp::Gt { dst, lhs, rhs }
            | MicroOp::LtEq { dst, lhs, rhs }
            | MicroOp::GtEq { dst, lhs, rhs }
            | MicroOp::Eq { dst, lhs, rhs }
            | MicroOp::Neq { dst, lhs, rhs }
            | MicroOp::Div { dst, lhs, rhs }
            | MicroOp::Mod { dst, lhs, rhs } => {
                for s in [dst, lhs, rhs] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::DivPow2 { dst, lhs, .. } | MicroOp::MagicDivU { dst, lhs, .. } => {
                for s in [dst, lhs] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::NotInt { dst, src } | MicroOp::NotBool { dst, src } => {
                for s in [dst, src] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::Branch { lhs, rhs, .. } => {
                for s in [lhs, rhs] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::JumpIfFalse { cond, .. } | MicroOp::JumpIfTrue { cond, .. } => {
                mark_float(cond, &mut float_blocked);
            }
            MicroOp::IntToFloat { src, .. } => mark_float(src, &mut float_blocked),
            MicroOp::ArrLoad { idx, ptr_slot, len_slot, .. } => {
                for s in [idx, ptr_slot, len_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::ArrStore { idx, ptr_slot, len_slot, .. } => {
                for s in [idx, ptr_slot, len_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::ArrRMW { idx, operand, ptr_slot, len_slot, .. } => {
                for s in [idx, operand, ptr_slot, len_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::ArrCondSwap { idx1, idx2, ptr_slot, len_slot, .. } => {
                for s in [idx1, idx2, ptr_slot, len_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::ArrSwap { idx1, idx2, ptr_slot, len_slot, .. } => {
                for s in [idx1, idx2, ptr_slot, len_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::ArrLoad2F { i: ix, j: jx, ptr_slot, len_slot, .. } => {
                for s in [ix, jx, ptr_slot, len_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            // The fused int two-load result is an i64; every operand and slot
            // is integer-role (block them from the float-pin budget, exactly
            // like the mem-form array ops).
            MicroOp::ArrLoad2 { dst, i: ix, j: jx, ptr_a, len_a, ptr_b, len_b, .. } => {
                for s in [dst, ix, jx, ptr_a, len_a, ptr_b, len_b] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::ArrPush { vec_slot, ptr_slot, len_slot, .. } => {
                for s in [vec_slot, ptr_slot, len_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::NewList { vec_slot, ptr_slot, len_slot, .. } => {
                for s in [vec_slot, ptr_slot, len_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, .. } => {
                for s in [handle_slot, vec_slot, ptr_slot, len_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::MapGet { key, map_slot, .. } => {
                for s in [key, map_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::MapSet { key, map_slot, .. } => {
                for s in [key, map_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            MicroOp::MapHas { dst, key, map_slot, .. } => {
                for s in [dst, key, map_slot] {
                    mark_float(s, &mut float_blocked);
                }
            }
            _ => {}
        }
    }
    let mut profit: std::collections::HashMap<u16, i64> = std::collections::HashMap::new();
    let bump = |slot: u16, delta: i64, profit: &mut std::collections::HashMap<u16, i64>| {
        if slot < register_count {
            *profit.entry(slot).or_insert(0) += delta;
        }
    };
    for (i, op) in ops.iter().enumerate() {
        // Variant uses save a memory access (+); mem-form uses force a
        // spill (−). Each scaled by execution frequency (loop depth).
        let v = 2 * weight(i);
        let mv = 1 * weight(i);
        let m = -4 * weight(i);
        match *op {
            MicroOp::Add { dst, lhs, rhs }
            | MicroOp::Sub { dst, lhs, rhs }
            | MicroOp::Mul { dst, lhs, rhs }
            | MicroOp::BitAnd { dst, lhs, rhs }
            | MicroOp::BitOr { dst, lhs, rhs }
            | MicroOp::BitXor { dst, lhs, rhs }
            | MicroOp::Shl { dst, lhs, rhs }
            | MicroOp::Shr { dst, lhs, rhs }
            | MicroOp::Lt { dst, lhs, rhs }
            | MicroOp::Gt { dst, lhs, rhs }
            | MicroOp::LtEq { dst, lhs, rhs }
            | MicroOp::GtEq { dst, lhs, rhs }
            | MicroOp::Eq { dst, lhs, rhs }
            | MicroOp::Neq { dst, lhs, rhs } => {
                for s in [dst, lhs, rhs] {
                    bump(s, v, &mut profit);
                }
            }
            MicroOp::Branch { lhs, rhs, .. } => {
                for s in [lhs, rhs] {
                    bump(s, v, &mut profit);
                }
            }
            MicroOp::Move { dst, src } => {
                for s in [dst, src] {
                    bump(s, mv, &mut profit);
                }
            }
            MicroOp::LoadConst { dst, .. } => bump(dst, mv, &mut profit),
            MicroOp::Return { src } => bump(src, mv, &mut profit),
            MicroOp::JumpIfFalse { cond, .. } | MicroOp::JumpIfTrue { cond, .. } => {
                bump(cond, -3 * weight(i), &mut profit);
            }
            MicroOp::Div { dst, lhs, rhs } | MicroOp::Mod { dst, lhs, rhs } => {
                for s in [dst, lhs, rhs] {
                    bump(s, m, &mut profit);
                }
            }
            // DivPow2 is a single mem-form stencil (reads lhs, writes dst).
            // MagicDivU bails the pinned/function tier (no stencil), but its
            // operands still profit from residency on the regalloc path.
            MicroOp::DivPow2 { dst, lhs, .. } | MicroOp::MagicDivU { dst, lhs, .. } => {
                for s in [dst, lhs] {
                    bump(s, m, &mut profit);
                }
            }
            // Float add/sub/mul are register-threaded now (V_FBINOP) → variant.
            MicroOp::AddF { dst, lhs, rhs }
            | MicroOp::SubF { dst, lhs, rhs }
            | MicroOp::MulF { dst, lhs, rhs } => {
                for s in [dst, lhs, rhs] {
                    bump(s, v, &mut profit);
                }
            }
            // DivF is register-threaded now (V_DIVF) → variant. The float
            // comparisons have no XMM variant yet → mem-form.
            MicroOp::DivF { dst, lhs, rhs } => {
                for s in [dst, lhs, rhs] {
                    bump(s, v, &mut profit);
                }
            }
            MicroOp::LtF { dst, lhs, rhs }
            | MicroOp::GtF { dst, lhs, rhs }
            | MicroOp::LtEqF { dst, lhs, rhs }
            | MicroOp::GtEqF { dst, lhs, rhs }
            | MicroOp::EqF { dst, lhs, rhs }
            | MicroOp::NeqF { dst, lhs, rhs } => {
                for s in [dst, lhs, rhs] {
                    bump(s, m, &mut profit);
                }
            }
            // BranchF is register-threaded now (V_BRANCHF) → variant.
            MicroOp::BranchF { lhs, rhs, .. } => {
                for s in [lhs, rhs] {
                    bump(s, v, &mut profit);
                }
            }
            // SqrtF is register-threaded now (V_SQRTF) → variant.
            MicroOp::SqrtF { dst, src } => {
                for s in [dst, src] {
                    bump(s, v, &mut profit);
                }
            }
            // IntToFloat is register-threaded now (V_I2F) → variant.
            MicroOp::IntToFloat { dst, src } => {
                for s in [dst, src] {
                    bump(s, v, &mut profit);
                }
            }
            MicroOp::NotInt { dst, src } | MicroOp::NotBool { dst, src } => {
                for s in [dst, src] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ArrLoad { dst, idx, ptr_slot, len_slot, .. } => {
                for s in [dst, idx, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ArrLoadAffine { dst, a, b, ptr_slot, len_slot, .. } => {
                for s in [dst, a, b, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ArrLoad2F { dst, i: ix, j: jx, ptr_slot, len_slot, .. } => {
                for s in [dst, ix, jx, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ArrLoad2 { dst, i: ix, j: jx, ptr_a, len_a, ptr_b, len_b, .. } => {
                for s in [dst, ix, jx, ptr_a, len_a, ptr_b, len_b] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ArrStore { src, idx, ptr_slot, len_slot, .. } => {
                for s in [src, idx, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ArrRMW { idx, operand, ptr_slot, len_slot, .. } => {
                for s in [idx, operand, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ArrCondSwap { idx1, idx2, ptr_slot, len_slot, .. } => {
                for s in [idx1, idx2, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ArrSwap { idx1, idx2, ptr_slot, len_slot, .. } => {
                for s in [idx1, idx2, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            // FmaF is introduced AFTER pin selection, so this arm never executes
            // at runtime — it exists only to keep the match exhaustive. Treat it
            // as a mem-form op for completeness.
            MicroOp::FmaF { dst, a, b, c } => {
                for s in [dst, a, b, c] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ArrPush { src, vec_slot, ptr_slot, len_slot, .. } => {
                for s in [src, vec_slot, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ListClear { vec_slot, ptr_slot, len_slot, .. } => {
                for s in [vec_slot, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            // The str-append's byte source is read across a SysV call (mem-form);
            // the handle slot is forced frame-resident, so count both as mem-form.
            MicroOp::StrAppend { text_handle_slot, src, .. } => {
                bump(text_handle_slot, m, &mut profit);
                if let logicaffeine_forge::jit::StrSrc::Byte(s) = src {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::Call { dst, .. } | MicroOp::CallSelf { dst, .. } => {
                bump(dst, m, &mut profit)
            }
            MicroOp::CallSelfCopy { dst, src_start, arg_count, .. } => {
                bump(dst, m, &mut profit);
                for s in src_start..src_start + arg_count {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::MapGet { dst, key, map_slot, .. } => {
                for s in [dst, key, map_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::MapSet { src, key, map_slot, .. } => {
                for s in [src, key, map_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::MapHas { dst, key, map_slot, .. } => {
                for s in [dst, key, map_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::NewList { vec_slot, ptr_slot, len_slot, .. } => {
                for s in [vec_slot, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::ListTriple { handle_slot, vec_slot, ptr_slot, len_slot, .. } => {
                for s in [handle_slot, vec_slot, ptr_slot, len_slot] {
                    bump(s, m, &mut profit);
                }
            }
            // A MemMem region runs UNPINNED (it reads/writes every slot through
            // the frame in the helper); count every touched slot as mem-form so
            // none would be selected even if the no-pin gate were ever lifted.
            MicroOp::MemMem {
                h_ptr_slot,
                h_len_slot,
                n_ptr_slot,
                n_len_slot,
                needle_len_slot,
                i_slot,
                count_slot,
                ..
            } => {
                for s in [
                    h_ptr_slot,
                    h_len_slot,
                    n_ptr_slot,
                    n_len_slot,
                    needle_len_slot,
                    i_slot,
                    count_slot,
                ] {
                    bump(s, m, &mut profit);
                }
            }
            MicroOp::Jump { .. } => {}
        }
    }
    // Threshold: flat model wants any net win (>2, historical); the region
    // model wants at least one depth-1 net benefit (>BASE) so cold depth-0
    // slots never pin.
    let threshold = if region { BASE } else { 2 };
    let ranked: Vec<(u16, i64)> = {
        let mut r: Vec<(u16, i64)> = profit
            .into_iter()
            .map(|(s, p)| (s, p - call_penalty))
            .filter(|&(_, p)| p > threshold)
            .collect();
        r.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        r
    };
    // Split by budget: a float slot can only ride an XMM register, an int/bool
    // slot only a GP register; each budget independently takes its four best. A
    // float slot that is pin-blocked (touched by a register-form op with no XMM
    // lowering) gets NO pin — it stays frame-resident — but is still kept out
    // of the GP budget (its value is f64 bits the GP stencils would mishandle).
    let mut gp_pins: Vec<u16> = Vec::new();
    let mut float_pins: Vec<u16> = Vec::new();
    // A float pin lives in a caller-saved XMM register, threaded through the
    // stencil chain by the register-form float ops (AddF/SubF/MulF). It is only
    // sound in a REGION when EVERY op on its live range provably leaves the
    // other live XMM pins alone behind the chain's back — otherwise a pin is
    // clobbered and memory corrupts (spectral_norm SIGSEGV'd at tier-up: its
    // inner loop kept the float product in a pinned XMM across both the `aVal`
    // CALL, when un-inlined, and the inlined `1.0/denom` DivF + IntToFloat).
    // Rather than blocklist the clobbering ops — which silently misses any new
    // one — WHITELIST the ops proven NOT to disturb a live float pin:
    // register-form float arith (AddF/SubF/MulF and the fused float load), the
    // array load/store stencils, the mem-form float stencils that spill/reload
    // only the slot THEY touch (DivF/SqrtF/IntToFloat/BranchF and — Lever 2a —
    // the float ORDERING compares LtF/LtEqF/GtF/GtEqF), and the pure integer/
    // pointer/control ops (no XMM at all). ANYTHING else blocks: a call or
    // helper-call (Call/CallSelf/Map*/NewList/ListTriple/ArrPush — all wipe the
    // caller-saved xmm0–15). Float EQUALITY (EqF/NeqF) is NOT whitelisted here —
    // it stays on its (epsilon-correct) mem-form path but a region carrying it
    // still drops its float pins, a pre-existing limitation outside Lever 2a's
    // ordering-only scope. A MicroOp variant added later is unmatched and so
    // blocks by default: sound until it is classified. GP pins and the function
    // tier are unaffected.
    let xmm_pin_safe = |op: &MicroOp| {
        matches!(
            op,
            MicroOp::LoadConst { .. }
                | MicroOp::Move { .. }
                | MicroOp::Add { .. }
                | MicroOp::Sub { .. }
                | MicroOp::Mul { .. }
                | MicroOp::Div { .. }
                | MicroOp::DivPow2 { .. }
                | MicroOp::MagicDivU { .. }
                | MicroOp::Mod { .. }
                | MicroOp::Lt { .. }
                | MicroOp::Gt { .. }
                | MicroOp::Eq { .. }
                | MicroOp::LtEq { .. }
                | MicroOp::GtEq { .. }
                | MicroOp::Neq { .. }
                | MicroOp::Branch { .. }
                | MicroOp::BitAnd { .. }
                | MicroOp::BitOr { .. }
                | MicroOp::BitXor { .. }
                | MicroOp::Shl { .. }
                | MicroOp::Shr { .. }
                | MicroOp::NotInt { .. }
                | MicroOp::NotBool { .. }
                | MicroOp::AddF { .. }
                | MicroOp::SubF { .. }
                | MicroOp::MulF { .. }
                | MicroOp::DivF { .. }
                | MicroOp::SqrtF { .. }
                | MicroOp::IntToFloat { .. }
                | MicroOp::BranchF { .. }
                | MicroOp::LtF { .. }
                | MicroOp::LtEqF { .. }
                | MicroOp::GtF { .. }
                | MicroOp::GtEqF { .. }
                | MicroOp::ArrLoad { .. }
                | MicroOp::ArrLoad2F { .. }
                | MicroOp::ArrLoad2 { .. }
                | MicroOp::ArrStore { .. }
                | MicroOp::ArrRMW { .. }
                | MicroOp::ArrCondSwap { .. }
                | MicroOp::ArrSwap { .. }
                | MicroOp::FmaF { .. }
                | MicroOp::Jump { .. }
                | MicroOp::JumpIfFalse { .. }
                | MicroOp::JumpIfTrue { .. }
                | MicroOp::Return { .. }
        )
    };
    // LEVER 3 (fusion-aware pin selection): a float slot that is the RESULT `d`
    // of an `AddF` which `fuse_fma` will collapse into an `FmaF` — its product
    // operand is the single-use, un-named result of an immediately-preceding
    // `MulF` — is KEPT OUT of the float-pin budget. `fuse_fma` runs after pin
    // selection; a mem-form `FmaF` writing a PINNED `d` must reload that pin from
    // the frame afterwards (one extra piece), which exactly cancels the dispatch
    // the fusion saved. Leaving `d` frame-resident lets the `FmaF` write the
    // frame directly with NO reload — a true 2-pieces-to-1 win (the nbody
    // distance-sum's `dz*dz + partial`, whose single-use partial-sum was
    // wastefully pinned). Frame traffic on this temp is L1-free next to the
    // saved dispatch (the engine is per-op DISPATCH-bound). Excluding a slot
    // that turns out NOT to fuse is at worst neutral (one un-pinned float temp).
    // The product `t` is read by the AddF being removed; if `t` were pinned the
    // fusion would not fire, so it is left to its own pin eligibility.
    let mut fma_result_temps: std::collections::HashSet<u16> = std::collections::HashSet::new();
    {
        let mut read_counts: std::collections::HashMap<u16, usize> = std::collections::HashMap::new();
        for op in ops {
            for s in micro_read_slots(op) {
                *read_counts.entry(s).or_insert(0) += 1;
            }
        }
        for k in 0..ops.len().saturating_sub(1) {
            let MicroOp::MulF { dst: t, .. } = ops[k] else { continue };
            // The addend `c` is the AddF operand that is NOT the product `t`.
            let (d, c) = match ops[k + 1] {
                MicroOp::AddF { dst, lhs, rhs } if lhs == t => (dst, rhs),
                MicroOp::AddF { dst, lhs, rhs } if rhs == t => (dst, lhs),
                _ => continue,
            };
            // The ACCUMULATOR shape `s = s + a*b` (`c == d`) is NOT a Lever-3
            // candidate: there `d` is a loop-carried float that must stay pinned,
            // and Lever 1a deliberately refuses to fuse it (a pinned addend `c`
            // would have to be spilled). Only a result `d` DISTINCT from its
            // addend is the genuine throwaway partial-sum the FmaF absorbs.
            if d == c {
                continue;
            }
            // `t` is single-use scratch (only the AddF reads it) — `fuse_fma`'s
            // removability precondition for the product (named `t`s are rejected
            // there too, but excluding their `d` here is at worst neutral: one
            // un-pinned float temp, free next to the per-op dispatch).
            let t_dead = (t as usize) < register_count as usize
                && read_counts.get(&t).copied().unwrap_or(0) == 1;
            if t_dead {
                fma_result_temps.insert(d);
            }
        }
    }
    // XMM float pinning is enabled for Main-loop REGIONS whose ops are all
    // XMM-aware. Array regions are now admitted: the earlier spectral_norm
    // SIGSEGV was a TYPE-REUSED slot (a `MulF` result reused as an `ArrStore`
    // index) being float-pinned — fixed above by blocking every integer-role
    // slot from the float-pin budget, so a mem-form array op never spills an
    // f64 over an integer index. FUNCTIONS still keep their floats frame-
    // resident (the mode-B call boundary round-trips them anyway).
    let block_float_pins = !region || !ops.iter().all(xmm_pin_safe);
    for (slot, _) in ranked {
        if float_slots.contains(&slot) {
            if !block_float_pins
                && !float_blocked.contains(&slot)
                && !fma_result_temps.contains(&slot)
                && float_pins.len() < 6
            {
                float_pins.push(slot);
            }
        } else if gp_pins.len() < 4 {
            gp_pins.push(slot);
        }
    }
    (gp_pins, float_pins)
}

impl NativeTier for ForgeTier {
    fn compile_function(
        &self,
        code: &[Op],
        entry_pc: usize,
        constants: &[Constant],
        param_count: u16,
        register_count: u16,
        self_fi: u16,
        param_kinds: &[Option<ParamKind>],
        ret_kind: Option<SlotKind>,
        ctx: &NativeCtx,
        callees: &[CalleeSig],
    ) -> Option<Box<dyn NativeFn>> {
        self.compiles.fetch_add(1, Ordering::SeqCst);
        // The frame layout (and therefore the limit slot the Call micros
        // bake) depends on the mode — compute it before adapting, with
        // EXACTLY the adapter's pin accounting (params + allocation sites
        // + list-returning self-call results); the adapter asserts the
        // agreement.
        let n_list_params = param_kinds
            .iter()
            .filter(|k| matches!(k, Some(ParamKind::List(_))))
            .count();
        let has_sites = code.iter().any(|op| matches!(op, Op::NewEmptyList { .. }));
        let mode_b = n_list_params > 0 || has_sites;
        let fn_returns_lists = ret_kind.is_none() && mode_b;
        let list_param_regs: Vec<u16> = param_kinds
            .iter()
            .enumerate()
            .filter_map(|(i, k)| {
                matches!(k, Some(ParamKind::List(_))).then_some(i as u16)
            })
            .collect();
        let npins = discover_list_regs(
            code,
            &list_param_regs,
            fn_returns_lists,
            self_fi,
            register_count as usize + 2,
        )
        .iter()
        .filter(|&&b| b)
        .count();
        let frame_size = if mode_b {
            register_count as usize + 6 + 3 * npins
        } else {
            register_count as usize + 3
        };
        // Self-recursion advances one full frame per level and stages the
        // callee window BEFORE the depth check runs — the arena must hold
        // MAX_CALL_DEPTH + 1 frames so staging writes can never cross the
        // buffer even on the deopt path.
        if frame_size * (logicaffeine_compile::semantics::MAX_CALL_DEPTH + 1) > ARENA_SLOTS {
            return None;
        }
        let call_ctx = CallCtx {
            table_addr: ctx.table.slot_addr(self_fi as usize),
            depth_addr: std::sync::Arc::as_ptr(&ctx.depth) as i64,
            status_addr: std::sync::Arc::as_ptr(&ctx.status) as i64,
            limit_slot: (frame_size - 1) as u16,
            depth_limit: logicaffeine_compile::semantics::MAX_CALL_DEPTH as i64,
            self_fi,
            table: ctx.table.as_ref(),
        };
        let adapted = adapt_function(
            code,
            entry_pc,
            constants,
            param_count,
            register_count,
            self_fi,
            param_kinds,
            ret_kind,
            &call_ctx,
            callees,
        )?;
        debug_assert_eq!(adapted.frame_size, frame_size);
        // LINEAR SCAN (EXODIA 3.1), mode A only: mode B's precise deopt
        // reads the frame mid-run, which the pinning contract forbids.
        // Profit = variant-family uses − spill/reload churn at memory-form
        // ops − the prologue reload; the four best positive slots pin.
        // The value-form float `>`/`>=` compares (`GtF`/`GtEqF`) now have a
        // mem-form lowering (Lever 2a: swap operands, reuse the LtF/LeF stencil —
        // IEEE-exact, NaN-false), so they spill/reload their pinned operands like
        // any mem-form op and no longer force the whole function unpinned.
        // `ListClear` is still excluded: the mem-form emitter cannot lower it
        // without leaving a pinned ptr/len register stale (it would miscompile —
        // observed on knapsack).
        let pinnable = !adapted.micro.iter().any(|op| {
            matches!(op, MicroOp::ListClear { .. })
        });
        // WS-G WAVE 12: the CONTIGUOUS regalloc FUNCTION backend. For a
        // non-precise mode-A function whose every op (the self-calls included)
        // is in the backend's supported subset, emit ONE register-allocated
        // x86-64 function with a real SysV self-call — 4-6x faster than the
        // per-piece tier on the recursion cluster. Returns None on any
        // unsupported op (a cross-function Call, list/map/byte ops, …), in which
        // case we fall through to the pinned per-piece tier, byte-identical to
        // today. The self-entry patch is INTERNAL (an entry cell), so the
        // stencil patch path below is skipped for this chain. `LOGOS_REGALLOC=0`
        // is the kill-switch.
        #[cfg(target_arch = "x86_64")]
        let regalloc_enabled = logicaffeine_forge::regalloc::regalloc_enabled()
            && !std::env::var("LOGOS_JIT_REGALLOC").is_ok_and(|v| v == "0");
        #[cfg(target_arch = "x86_64")]
        let regalloc_chain: Option<CompiledChain> = if !regalloc_enabled {
            None
        } else if adapted.precise.is_none() {
            // MODE A (scalar, classic replay deopt): the original contiguous
            // FUNCTION backend.
            logicaffeine_forge::regalloc::compile_function_regalloc(
                &adapted.micro,
                Some(ctx.status.clone()),
                call_ctx.depth_addr,
            )
        } else if std::env::var("LOGOS_REGALLOC_PRECISE").as_deref() != Ok("0") {
            // MODE B (list-param, in-place array mutation): the PRECISE
            // contiguous backend — a side exit materializes the native frame and
            // resumes AT the faulting op (no replay-from-head double-apply). Only
            // when the adapter built the per-op deopt-code table. `None` on any
            // unsupported op (e.g. a list-RETURN allocation/Map shape) → fall back
            // to the per-piece precise stencil tier, byte-identical to today.
            // `LOGOS_REGALLOC_PRECISE=0` is the targeted kill-switch.
            adapted.deopt_codes.as_deref().and_then(|codes| {
                logicaffeine_forge::regalloc::compile_function_regalloc_precise(
                    &adapted.micro,
                    Some(ctx.status.clone()),
                    call_ctx.depth_addr,
                    codes,
                )
            })
        } else {
            None
        };
        #[cfg(not(target_arch = "x86_64"))]
        let regalloc_chain: Option<CompiledChain> = None;

        let chain = if let Some(c) = regalloc_chain {
            self.regalloc_functions.fetch_add(1, Ordering::SeqCst);
            // A `CallSelfCopy` in the stream is a FUSED contiguous-arg self-call
            // (Lever A's fusion, decided by the adapter) — the regalloc backend
            // stages the contiguous arg block in one go just like the stencil
            // tier did, so it remains the same observable. Count it (the
            // kill-switch path emits per-`Move` `CallSelf` instead, leaving this
            // at zero).
            let fused = adapted
                .micro
                .iter()
                .filter(|op| matches!(op, MicroOp::CallSelfCopy { .. }))
                .count() as u32;
            if fused != 0 {
                self.pinned_self_calls.fetch_add(fused, Ordering::SeqCst);
            }
            c
        } else {
            let (gp_pins, float_pins): (Vec<u16>, Vec<u16>) = if pinnable
                && adapted.precise.is_none()
                && !std::env::var("LOGOS_JIT_REGALLOC").is_ok_and(|v| v == "0")
            {
                select_pins(&adapted.micro, register_count, false)
            } else {
                (Vec::new(), Vec::new())
            };
            let chain = if gp_pins.is_empty() && float_pins.is_empty() {
                compile_straightline_coded(
                    &adapted.micro,
                    Some(ctx.status.clone()),
                    adapted.deopt_codes.as_deref(),
                    call_ctx.depth_addr,
                )
                .ok()?
            } else {
                compile_straightline_pinned_float(
                    &adapted.micro,
                    &gp_pins,
                    &float_pins,
                    Some(ctx.status.clone()),
                )
                .ok()?
            };
            // Self-entry patch: write this chain's base into every direct
            // self-call site. Targets without post-seal patching (MAP_JIT)
            // fail here — fall back to bytecode rather than run unpatched.
            if chain.has_patch_marks() && chain.patch_marked(chain.base()).is_err() {
                return None;
            }
            let fused = adapted
                .micro
                .iter()
                .filter(|op| matches!(op, MicroOp::CallSelfCopy { .. }))
                .count() as u32;
            if fused != 0 {
                self.pinned_self_calls.fetch_add(fused, Ordering::SeqCst);
            }
            chain
        };
        self.successes.fetch_add(1, Ordering::SeqCst);
        Some(Box::new(ChainFn {
            chain,
            limit_slot: call_ctx.limit_slot as usize,
            depth: ctx.depth.clone(),
            ret: adapted.ret,
            published_regc: (frame_size - 3) as i64,
            precise: adapted.precise,
            stats: self.runtime.clone(),
        }))
    }

    fn compile_region(
        &self,
        code: &[Op],
        head_pc: usize,
        exit_pc: usize,
        constants: &[Constant],
        register_count: u16,
        named: &[bool],
        observed: &[ObservedKind],
        ctx: &NativeCtx,
        callees: &[logicaffeine_compile::vm::CalleeSig],
    ) -> Option<Box<dyn RegionFn>> {
        self.region_compiles.fetch_add(1, Ordering::SeqCst);
        let dbg = std::env::var_os("LOGOS_RDIAG").is_some();
        let Some((mut micro, frame_size, guard, free, writes, arrays, region_return, hoist_guards, precise)) =
            adapt_region(
            code,
            head_pc,
            exit_pc,
            constants,
            register_count,
            named,
            observed,
            ctx,
            callees,
        ) else {
            if dbg { eprintln!("RDIAG head_pc={head_pc}: adapt_region -> None"); }
            return None;
        };
        let has_calls = code.iter().any(|op| matches!(op, Op::Call { .. }));
        // A calling region's chain must check the SHARED status cell — the
        // call stencil writes its deopt marker there. A PRECISE region needs it
        // too: every side exit stores its encoded resume pc through it.
        let status =
            (has_calls || precise.is_some()).then(|| ctx.status.clone());
        // REGION REGISTER ALLOCATION (EXODIA 3.1, sprint 13's deferred half).
        // Pin the hottest INTEGER slots that are not in the write-set or the
        // array-handle set: the write-set must stay frame-resident for the
        // VM's success copy-back, and dead-at-exit slots (loop counters,
        // read-only bounds, scratch) need only the prologue reload — so NO
        // exit-spill machinery is required, and a mid-loop side-exit keeps
        // the discard-and-replay contract untouched (the VM replays from the
        // region head on its OWN registers; the native frame's scalars are
        // never read back). The value-form float `>`/`>=` compares (`GtF`/
        // `GtEqF`) now have a mem-form lowering (Lever 2a: swap operands, reuse
        // the LtF/LeF stencil — IEEE-exact, NaN-false), so a region carrying a
        // value-form float `>`/`>=` keeps its XMM pins instead of dropping all
        // of them. (A float `>` USED AS A BRANCH already fuses into the register-
        // form `BranchF`; only the VALUE form produces a surviving `GtF`/`GtEqF`.)
        //
        // `ListClear` is still excluded: the mem-form emitter has no arm for it
        // (it resets a list's vec/ptr/len through a helper, which would leave a
        // pinned ptr/len register stale), so a pinned region containing one
        // would miscompile (observed: knapsack's DP-row reuse). Such a region
        // stays unpinned and runs the correct unpinned path.
        let pinnable = !micro.iter().any(|op| {
            // A `MemMem` region reads/writes every operand through the frame in
            // its helper, so it must run UNPINNED (no register-resident slot the
            // helper would miss); `ListClear`'s mem-form emitter has no pinned arm.
            matches!(op, MicroOp::ListClear { .. } | MicroOp::MemMem { .. })
        });
        let (gp_pins, float_pins): (Vec<u16>, Vec<u16>) = if !pinnable
            // Precise regions materialize the frame's scalars on a side exit, so
            // every scalar must stay frame-resident — no GP register pinning
            // (mirrors the function tier's `adapted.precise.is_none()` gate).
            || precise.is_some()
            || std::env::var("LOGOS_JIT_REGALLOC").is_ok_and(|v| v == "0")
        {
            (Vec::new(), Vec::new())
        } else {
            // Phase 2 (write-set pinning): the WRITE-SET is now eligible —
            // the pinned compiler's success-exit epilogue spills each pinned
            // slot back to its frame cell, so the VM's write-back still sees
            // loop-carried values. Only ARRAY handle registers stay excluded
            // (they are pinned through the dedicated ptr/len triple, not GP
            // register threading).
            let array_regs: std::collections::HashSet<u16> =
                arrays.iter().map(|a| a.reg).collect();
            let (gp, fp) = select_pins(&micro, register_count, true);
            let mut gp: Vec<u16> =
                gp.into_iter().filter(|r| !array_regs.contains(r)).collect();
            let fp: Vec<u16> = fp.into_iter().filter(|r| !array_regs.contains(r)).collect();
            // REGISTER-FORM array access: hand the hottest INT array whose base
            // pointer is STABLE (never pushed-to in this region → no realloc) a
            // free GP register, so its word ArrLoad/ArrStore read the pointer
            // from a register instead of re-loading the frame cell every access.
            // Parked default-OFF: measured ~0% (the interpreter is per-op
            // DISPATCH-bound, not frame-traffic-bound — re-reading the array
            // pointer from its L1-hot frame cell is free next to the stencil
            // dispatch). `LOGOS_ARRPTR=1` re-enables for experiments.
            if gp.len() < 4 && std::env::var("LOGOS_ARRPTR").is_ok() {
                let pushed: std::collections::HashSet<u16> = micro
                    .iter()
                    .filter_map(|op| match op {
                        MicroOp::ArrPush { vec_slot, .. } => Some(*vec_slot),
                        _ => None,
                    })
                    .collect();
                let mut best: Option<(u16, usize)> = None;
                for a in &arrays {
                    if a.elem != PinElem::Int || pushed.contains(&a.vec_slot) {
                        continue;
                    }
                    let cnt = micro
                        .iter()
                        .filter(|op| match op {
                            MicroOp::ArrLoad { ptr_slot, byte: false, .. }
                            | MicroOp::ArrStore { ptr_slot, byte: false, .. } => {
                                *ptr_slot == a.ptr_slot
                            }
                            _ => false,
                        })
                        .count();
                    if cnt > 0 && best.map_or(true, |(_, b)| cnt > b) {
                        best = Some((a.ptr_slot, cnt));
                    }
                }
                if let Some((ptr_slot, _)) = best {
                    gp.push(ptr_slot);
                }
            }
            (gp, fp)
        };
        // COPY-PROPAGATION + CONSTANT CSE — kills the redundant scratch-Move
        // chains and re-loaded constants the bytecode lowering leaves in hot
        // loops (the per-op DISPATCH is the floor, so fewer ops = faster). Run
        // AFTER pin selection, protecting the chosen pins, so it never perturbs
        // the greedy allocator (the reason the earlier pre-selection version had
        // to be array-free-gated) — now it applies in array regions too. Sound
        // only for replay-from-head (non-precise) regions: removing micro ops
        // renumbers jump targets (remapped) but would break per-op precise
        // resume codes. `LOGOS_COPYPROP=0` is the kill-switch.
        if precise.is_none() && std::env::var("LOGOS_COPYPROP").as_deref() != Ok("0") {
            let pinset: std::collections::HashSet<u16> =
                gp_pins.iter().chain(float_pins.iter()).copied().collect();
            copy_propagate(&mut micro, named, register_count, &pinset);
        }
        // WS-G PRE-FUSION SNAPSHOT (approach a). The array-fusion passes below
        // (`ArrRMW`/`ArrLoad2`/`ArrLoadAffine`/`ArrCondSwap`/`ArrSwap`) exist to
        // cut PIECE count in the per-piece stencil tier. The CONTIGUOUS regalloc
        // backend has no per-piece overhead and handles RAW `ArrLoad`/`ArrStore`
        // directly, so it must see the UN-fused stream. Snapshot the cleaned
        // (copy-propagated) micro here, BEFORE the fusions, and try regalloc on
        // it at the dispatch below; only if regalloc declines do the fusions and
        // the per-piece stencil tier take over the (mutated) `micro`. The
        // snapshot is taken only for a non-precise region (the only kind
        // regalloc compiles); precise regions never reach it.
        #[cfg(target_arch = "x86_64")]
        let prefusion_micro: Option<Vec<MicroOp>> =
            (precise.is_none() && logicaffeine_forge::regalloc::regalloc_enabled())
                .then(|| micro.clone());
        // ARRAY READ-MODIFY-WRITE FUSION: after copy-prop/CSE has cleaned the
        // stream and deduped the operand constant, collapse `arr[i] = arr[i] OP
        // operand` (ArrLoad + int ALU + ArrStore) into one ArrRMW stencil — the
        // dispatch-reduction lever for the indexed-RMW loops (histogram,
        // counting sort). Non-precise only (it renumbers micro indices).
        // `LOGOS_RMW=0` is the kill-switch.
        if precise.is_none() && std::env::var("LOGOS_RMW").as_deref() != Ok("0") {
            let pinset: std::collections::HashSet<u16> =
                gp_pins.iter().chain(float_pins.iter()).copied().collect();
            fuse_array_rmw(&mut micro, named, register_count, &pinset);
        }
        // TWO-BUFFER INTEGER LOAD+BINOP FUSION: collapse `a[i] OP b[j]`
        // (ArrLoad + ArrLoad + int Add/Sub/Mul) into one ArrLoad2 stencil — the
        // dispatch-reduction lever for the integer dot-product / matrix-multiply
        // inner loop, where two distinct pinned buffers feed a binop. Runs after
        // RMW (the c-accumulator's load+add+mod+store is not a clean RMW, so the
        // two passes never contend for the same ops). Non-precise only (it
        // renumbers micro indices). `LOGOS_LD2=0` is the kill-switch.
        if precise.is_none() && std::env::var("LOGOS_LD2").as_deref() != Ok("0") {
            let pinset: std::collections::HashSet<u16> =
                gp_pins.iter().chain(float_pins.iter()).copied().collect();
            fuse_array_ld2(&mut micro, named, register_count, &pinset);
        }
        // FLOAT MULTIPLY-ADD FUSION: merge `a*b; +c` (MulF feeding a single-use
        // AddF) into one FmaF stencil — the dispatch-reduction lever for the
        // float arithmetic chains in the float cluster (nbody). Only fuses
        // frame-resident operands (spill-free). `LOGOS_FMA=0` is the kill-switch.
        if precise.is_none() && std::env::var("LOGOS_FMA").as_deref() != Ok("0") {
            let pinset: std::collections::HashSet<u16> =
                gp_pins.iter().chain(float_pins.iter()).copied().collect();
            fuse_fma(&mut micro, named, register_count, &pinset);
        }
        // CONDITIONAL-SWAP FUSION: collapse the sort inner-loop compare-and-swap
        // (ArrLoad×2 + Branch + ArrStore×2 → 1) — now fires thanks to the
        // precise-region-live-out keystone freeing the loop-local a/b.
        // `LOGOS_CONDSWAP=0` kills it.
        if std::env::var("LOGOS_DUMP_MICRO").ok().and_then(|v| v.parse::<usize>().ok()) == Some(head_pc) {
            eprintln!("=== MICRO head_pc={head_pc} ({} ops) ===", micro.len());
            for (i, m) in micro.iter().enumerate() {
                eprintln!("  [{i}] {m:?}");
            }
        }
        if precise.is_none() && std::env::var("LOGOS_CONDSWAP").as_deref() != Ok("0") {
            let pinset: std::collections::HashSet<u16> =
                gp_pins.iter().chain(float_pins.iter()).copied().collect();
            fuse_cond_swap(&mut micro, named, register_count, &pinset);
        }
        // UNCONDITIONAL-SWAP FUSION: collapse the `tmp` exchange (quick/heap/merge
        // sort: load,load,store,store → 1). `LOGOS_SWAP=0` kills it.
        if precise.is_none() && std::env::var("LOGOS_SWAP").as_deref() != Ok("0") {
            let pinset: std::collections::HashSet<u16> =
                gp_pins.iter().chain(float_pins.iter()).copied().collect();
            fuse_swap(&mut micro, named, register_count, &pinset);
        }
        // AFFINE-INDEX LOAD FUSION: fold the index arithmetic of a read-only
        // array load (`prev[w - wi + 1]`, the heap-sift reads, graph adjacency
        // reads) into the load stencil. Runs LAST among the array fusions so the
        // two-buffer (`ArrLoad2`), RMW, and swap fusions claim their raw loads
        // first; this pass only takes the plain `ArrLoad`s they leave behind.
        // `LOGOS_AFFINE=0` is the kill-switch.
        if precise.is_none() && std::env::var("LOGOS_AFFINE").as_deref() != Ok("0") {
            let pinset: std::collections::HashSet<u16> =
                gp_pins.iter().chain(float_pins.iter()).copied().collect();
            fuse_index_affine(&mut micro, named, register_count, &pinset);
        }
        if dbg {
            eprintln!(
                "RDIAG head_pc={head_pc}: adapt OK frame={frame_size} pinnable={pinnable} gp_pins={gp_pins:?} float_pins={float_pins:?} arrays={}",
                arrays.len()
            );
        }
        // WS-G CONTIGUOUS REGALLOC BACKEND (LOGOS_REGALLOC default ON).
        // For a non-precise integer region whose every op is in the contiguous
        // backend's supported subset, emit ONE register-allocated function (no
        // per-piece stencil boundaries). Returns None on any unsupported op →
        // we fall through to the existing per-piece tiers, so behavior with the
        // flag off — or on an unsupported region — is byte-identical to today.
        //
        // It is tried on the PRE-FUSION snapshot (approach a): the backend has
        // no per-piece cost, so it wants the RAW `ArrLoad`/`ArrStore` stream,
        // not the fused `ArrRMW`/`ArrLoad2`/… the array fusions produced for the
        // stencil tier. If regalloc declines (an unsupported op — float arrays,
        // byte loads, calls, globals, list ops, …) we fall through to the
        // already-fused `micro` and the per-piece tier, byte-identical to today.
        #[cfg(target_arch = "x86_64")]
        let regalloc_chain: Option<CompiledChain> = prefusion_micro
            .as_ref()
            .and_then(|m| logicaffeine_forge::regalloc::compile_region_regalloc(m, status.clone()));
        #[cfg(not(target_arch = "x86_64"))]
        let regalloc_chain: Option<CompiledChain> = None;

        let chain = if let Some(c) = regalloc_chain {
            self.regalloc_regions.fetch_add(1, Ordering::SeqCst);
            if dbg {
                eprintln!("RDIAG head_pc={head_pc}: CONTIGUOUS REGALLOC backend (arrays={})", arrays.len());
            }
            c
        } else if let Some(p) = precise.as_ref() {
            // PRECISE region (the fannkuch / graph_bfs worklist shape: an
            // in-place `SetIndex` beside a reallocating `ArrPush`). The depth
            // cell value rides the resume tag's high 32 bits; the VM masks it out
            // of the region resume (only the low-32 resume code is read back).
            let depth_addr = std::sync::Arc::as_ptr(&ctx.depth) as i64;
            // WS-G WAVE 21: try the CONTIGUOUS regalloc backend FIRST, with the
            // per-op precise deopt codes. It register-allocates the region's
            // scalars (the per-piece precise stencil tier kept every scalar
            // frame-resident and paid the 4-6× per-piece dispatch overhead) while
            // flushing every resident-written scalar to its frame slot at each
            // precise side exit, so the VM's resume reads correct values. No
            // fusions / copy-prop ran (all gated `precise.is_none()`), so `micro`
            // is the raw lowered stream `p.deopt_codes` is parallel to. On any
            // unsupported op the backend declines and the per-piece precise tier
            // takes over — byte-identical to before this wave.
            #[cfg(target_arch = "x86_64")]
            let precise_regalloc: Option<CompiledChain> = (logicaffeine_forge::regalloc::regalloc_enabled()
                && std::env::var("LOGOS_REGALLOC_PRECISE").as_deref() != Ok("0"))
            .then(|| {
                logicaffeine_forge::regalloc::compile_region_regalloc_precise(
                    &micro,
                    status.clone(),
                    depth_addr,
                    &p.deopt_codes,
                )
            })
            .flatten();
            #[cfg(not(target_arch = "x86_64"))]
            let precise_regalloc: Option<CompiledChain> = None;
            if let Some(c) = precise_regalloc {
                self.regalloc_regions.fetch_add(1, Ordering::SeqCst);
                self.regalloc_precise_regions.fetch_add(1, Ordering::SeqCst);
                if dbg {
                    eprintln!("RDIAG head_pc={head_pc}: CONTIGUOUS REGALLOC backend (PRECISE, arrays={})", arrays.len());
                }
                c
            } else {
                match compile_straightline_coded(&micro, status, Some(&p.deopt_codes), depth_addr) {
                    Ok(c) => c,
                    Err(e) => {
                        if dbg { eprintln!("RDIAG head_pc={head_pc}: compile_straightline_coded(precise) -> Err({e:?})"); }
                        return None;
                    }
                }
            }
        } else if gp_pins.is_empty() && float_pins.is_empty() {
            match compile_straightline_with(&micro, status) {
                Ok(c) => c,
                Err(e) => {
                    if dbg { eprintln!("RDIAG head_pc={head_pc}: compile_straightline_with -> Err({e:?})"); }
                    return None;
                }
            }
        } else {
            match compile_straightline_pinned_float(&micro, &gp_pins, &float_pins, status) {
                Ok(c) => c,
                Err(e) => {
                    if dbg { eprintln!("RDIAG head_pc={head_pc}: compile_straightline_pinned_float -> Err({e:?})"); }
                    return None;
                }
            }
        };
        self.region_successes.fetch_add(1, Ordering::SeqCst);
        let call_support = has_calls.then(|| {
            (ARENA_SLOTS, (frame_size - 1) as u16, ctx.depth.clone())
        });
        Some(Box::new(RegionChain {
            chain,
            frame_size,
            call_support,
            guard,
            free,
            writes,
            arrays,
            region_return,
            hoist_guards,
            precise_kinds: precise.map(|p| p.kinds_by_pc),
        }))
    }
}

/// Install the forge tier as the process-wide native tier. Idempotent —
/// the first call wins; later calls return the already-installed tier.
///
/// Call once at binary startup (CLI, server). The live VM constructors in
/// `logicaffeine-compile` pick it up for every program they run.
///
/// `LOGOS_NATIVE_TIER=0` skips installation entirely — the diagnostic
/// kill-switch that isolates pure-bytecode wall time from tiered runs.
/// Install the diagnostic SIGSEGV/SIGBUS tracer (no-op unless `LOGOS_SEGV_TRAP`
/// is set). Used to localize faults inside JIT'd code while root-causing.
pub fn segv_trace_install() {
    logicaffeine_forge::segv_trace::install();
}

pub fn install() -> &'static ForgeTier {
    static TIER: std::sync::OnceLock<ForgeTier> = std::sync::OnceLock::new();
    let tier = TIER.get_or_init(ForgeTier::new);
    if !std::env::var("LOGOS_NATIVE_TIER").is_ok_and(|v| v == "0") {
        install_native_tier(tier);
    }
    tier
}

#[cfg(test)]
mod select_pins_tests {
    use super::*;

    /// A slot used as BOTH an f64 (a `MulF` result, float-valued) AND an i64
    /// (an `ArrStore` index, integer-valued) is type-reused scratch. It must
    /// NEVER be float-pinned: the mem-form `ArrStore` spills the float pin to
    /// the frame cell, overwriting the integer index the `Add` wrote, so the
    /// store indexes through an f64 bit-pattern → a wild write. This was the
    /// spectral_norm SIGSEGV. The integer-role classification in `select_pins`
    /// must keep the mixed slot out of the float-pin budget (frame-resident).
    #[test]
    fn float_result_reused_as_array_index_is_not_float_pinned() {
        // register_count = 8 → slots 0..7 are registers; 8/9 stand in for the
        // pinned array's ptr/len cells. Slot 5 is the type-reused cell.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 6 },
            MicroOp::MulF { dst: 5, lhs: 2, rhs: 3 }, // slot 5 holds an f64 …
            MicroOp::AddF { dst: 4, lhs: 4, rhs: 5 }, // … consumed as a float
            MicroOp::Add { dst: 5, lhs: 0, rhs: 1 },  // slot 5 reused as an i64 index
            MicroOp::ArrStore {
                src: 4,
                idx: 5,
                ptr_slot: 8,
                len_slot: 9,
                byte: false,
                checked: false,
            },
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 0 },
        ];
        let (_gp, fpins) = select_pins(&ops, 8, true);
        assert!(
            !fpins.contains(&5),
            "slot 5 (an f64 result reused as an integer array index) must NOT be \
             float-pinned — pinning it spills f64 bits over the integer index; got fpins={fpins:?}"
        );
    }

    /// LEVER 3 — a single-use float partial-sum that is the DESTINATION of an
    /// `AddF` consuming an immediately-preceding single-use `MulF` product (the
    /// nbody distance-sum's `partial = a*a + (b*b)` shape, `d` distinct from its
    /// addend) is the result an FmaF will absorb. It must be KEPT OUT of the
    /// float-pin budget so the (later) `fuse_fma` writes the frame directly with
    /// NO reload — a 2-pieces-to-1 win — instead of pinning it and paying a reload
    /// after the mem-form FmaF.
    #[test]
    fn fma_result_temp_is_not_float_pinned() {
        // partial = aa + b*b, where aa is a frame temp and b*b is single-use.
        // slot 4 = partial (the FmaF dst); slot 6 = b*b product (single-use);
        // slot 5 = aa (the addend, distinct from 4); slot 7 reads partial after.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 6 },
            MicroOp::MulF { dst: 6, lhs: 2, rhs: 2 }, // b*b → single-use product
            MicroOp::AddF { dst: 4, lhs: 5, rhs: 6 }, // partial = aa + (b*b), 4 != 5
            MicroOp::SqrtF { dst: 7, src: 4 },        // partial's only reader
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 0 },
        ];
        let (_gp, fpins) = select_pins(&ops, 8, true);
        assert!(
            !fpins.contains(&4),
            "slot 4 (a single-use FmaF-candidate partial-sum, distinct from its \
             addend) must NOT be float-pinned (Lever 3); got fpins={fpins:?}"
        );
    }

    /// LEVER 3 boundary — a LOOP-CARRIED accumulator `s = s + a*b` (`c == d == s`)
    /// is NOT an FmaF candidate (Lever 1a refuses to fuse a pinned addend), so
    /// Lever 3 must NOT exclude it from the float-pin budget. (The accumulator is
    /// read+written by the AddF as a float; the region returns an unrelated int so
    /// the `Return`-blocks-float rule does not interfere with the assertion.)
    #[test]
    fn loop_carried_float_accumulator_not_excluded_by_lever3() {
        // s = s + a*b: slot 4 = s (read + written by the AddF), 6 = a*b product.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 4 },
            MicroOp::MulF { dst: 6, lhs: 2, rhs: 3 }, // a*b → product
            MicroOp::AddF { dst: 4, lhs: 4, rhs: 6 }, // s = s + product (c == d == 4)
            MicroOp::Jump { target: 0 },
            MicroOp::Return { src: 0 }, // returns an int, not the accumulator
        ];
        let (_gp, fpins) = select_pins(&ops, 8, true);
        assert!(
            fpins.contains(&4),
            "slot 4 (a loop-carried float accumulator, c == d) must STILL be \
             float-pin-eligible — it is not an FmaF candidate, so Lever 3 must not \
             exclude it; got fpins={fpins:?}"
        );
    }
}

#[cfg(test)]
mod fuse_rmw_tests {
    use super::*;
    use std::collections::HashSet;

    // register_count = 8 (slots 0..7 are VM registers); slots 6/7 stand in for
    // the pinned array's ptr/len cells. `named` marks slot 5 (the index) live;
    // the scratch temps 3 (loaded value) and 4 (sum) are un-named.
    fn named8(live: &[u16]) -> Vec<bool> {
        let mut n = vec![false; 8];
        for &s in live {
            n[s as usize] = true;
        }
        n
    }

    /// The canonical histogram idiom `arr[i] = arr[i] + operand` — ArrLoad into
    /// a single-use scratch, Add with the operand, ArrStore the result back to
    /// the SAME array+index — collapses to one ArrRMW.
    #[test]
    fn load_add_store_to_same_cell_fuses_to_rmw() {
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Add { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Return { src: 0 },
        ];
        fuse_array_rmw(&mut micro, &named8(&[2, 5]), 8, &HashSet::new());
        assert_eq!(micro.len(), 2, "the 3-op idiom must collapse to 1 op + Return");
        match micro[0] {
            MicroOp::ArrRMW { idx, operand, ptr_slot, len_slot, op, checked } => {
                assert_eq!((idx, operand, ptr_slot, len_slot), (5, 2, 6, 7));
                assert_eq!(op, RmwOp::Add);
                assert!(checked);
            }
            ref other => panic!("expected ArrRMW, got {other:?}"),
        }
    }

    /// Subtraction is non-commutative: it fuses ONLY when the loaded element is
    /// the left operand (`buf[i] - operand`), and the operand is the right.
    #[test]
    fn sub_fuses_only_with_loaded_value_on_the_left() {
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: false },
            MicroOp::Sub { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: false },
            MicroOp::Return { src: 0 },
        ];
        fuse_array_rmw(&mut micro, &named8(&[2, 5]), 8, &HashSet::new());
        match micro[0] {
            MicroOp::ArrRMW { op: RmwOp::Sub, operand: 2, checked: false, .. } => {}
            ref other => panic!("expected unchecked ArrRMW Sub operand=2, got {other:?}"),
        }

        // operand - buf[i] is NOT an RMW (the loaded value is on the right).
        let mut rev = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: false },
            MicroOp::Sub { dst: 4, lhs: 2, rhs: 3 },
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: false },
            MicroOp::Return { src: 0 },
        ];
        fuse_array_rmw(&mut rev, &named8(&[2, 5]), 8, &HashSet::new());
        assert!(
            matches!(rev[0], MicroOp::ArrLoad { .. }),
            "operand - buf[i] must NOT fuse: {rev:?}"
        );
    }

    /// A bounds-checked load with an unchecked store (or vice versa) fuses to a
    /// CHECKED rmw — a single pre-write check covers both (same index).
    #[test]
    fn mixed_checked_load_unchecked_store_fuses_checked() {
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::BitOr { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: false },
            MicroOp::Return { src: 0 },
        ];
        fuse_array_rmw(&mut micro, &named8(&[2, 5]), 8, &HashSet::new());
        match micro[0] {
            MicroOp::ArrRMW { op: RmwOp::Or, checked: true, .. } => {}
            ref other => panic!("expected checked ArrRMW Or, got {other:?}"),
        }
    }

    /// The loaded value must be single-use: if `t` is read again (here also
    /// stored elsewhere), folding it away would lose that read — no fusion.
    #[test]
    fn multi_use_loaded_value_blocks_fusion() {
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Add { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Move { dst: 1, src: 3 }, // second use of the loaded value
            MicroOp::Return { src: 1 },
        ];
        fuse_array_rmw(&mut micro, &named8(&[1, 2, 5]), 8, &HashSet::new());
        assert!(matches!(micro[0], MicroOp::ArrLoad { .. }), "multi-use t must block fusion");
    }

    /// Load and store to DIFFERENT arrays (or different indices) is not an
    /// in-place RMW and must never fuse.
    #[test]
    fn different_array_or_index_blocks_fusion() {
        // different ptr_slot
        let mut a = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Add { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 1, len_slot: 7, byte: false, checked: true },
            MicroOp::Return { src: 0 },
        ];
        fuse_array_rmw(&mut a, &named8(&[1, 2, 5]), 8, &HashSet::new());
        assert!(matches!(a[0], MicroOp::ArrLoad { .. }), "different array must block fusion");
        // different index
        let mut b = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Add { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::ArrStore { src: 4, idx: 1, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Return { src: 0 },
        ];
        fuse_array_rmw(&mut b, &named8(&[1, 2, 5]), 8, &HashSet::new());
        assert!(matches!(b[0], MicroOp::ArrLoad { .. }), "different index must block fusion");
    }

    /// A jump landing ON the store (or the ALU) means another path enters
    /// mid-idiom — fusing would skip the load on that entry. No fusion.
    #[test]
    fn jump_target_inside_idiom_blocks_fusion() {
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Add { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::JumpIfTrue { cond: 2, target: 2 }, // jumps onto the store
            MicroOp::Return { src: 0 },
        ];
        fuse_array_rmw(&mut micro, &named8(&[2, 5]), 8, &HashSet::new());
        assert!(
            matches!(micro[0], MicroOp::ArrLoad { .. }),
            "a jump target inside the idiom must block fusion"
        );
    }

    /// THE HISTOGRAM SHAPE: the index `v + 1` is computed TWICE (separate
    /// slots, as the bytecode lowers `Set item (v+1) of A to (item (v+1) of A)
    /// + 1`). `copy_propagate`'s value-numbering must merge the two `v + 1` into
    /// one slot so the load and store share an index, and only then does
    /// `fuse_array_rmw` collapse the idiom. This pins the CSE→fusion handoff.
    #[test]
    fn duplicated_index_expr_is_cse_d_then_fused() {
        // v=1 (named loop var); c=2 the `1`; 3/5/6 scratch; 8/9 ptr/len.
        let mut micro = vec![
            MicroOp::LoadConst { dst: 2, value: 1 },
            MicroOp::Add { dst: 3, lhs: 1, rhs: 2 }, // idx_a = v + 1
            MicroOp::ArrLoad { dst: 4, idx: 3, ptr_slot: 8, len_slot: 9, byte: false, checked: true },
            MicroOp::Add { dst: 5, lhs: 4, rhs: 2 }, // t2 = arr[idx_a] + 1
            MicroOp::Add { dst: 6, lhs: 1, rhs: 2 }, // idx_b = v + 1  (recomputed)
            MicroOp::ArrStore { src: 5, idx: 6, ptr_slot: 8, len_slot: 9, byte: false, checked: true },
            MicroOp::Return { src: 1 },
        ];
        let named = named8_n(&[1], 10);
        copy_propagate(&mut micro, &named, 10, &HashSet::new());
        fuse_array_rmw(&mut micro, &named, 10, &HashSet::new());
        // The dead recomputed index is gone; the idiom is one ArrRMW.
        let rmw = micro.iter().find(|op| matches!(op, MicroOp::ArrRMW { .. }));
        match rmw {
            Some(&MicroOp::ArrRMW { idx, operand, op, .. }) => {
                assert_eq!(op, RmwOp::Add);
                assert_eq!(operand, 2, "operand is the deduped `1`");
                // idx is whichever surviving slot holds v+1 (the load's index).
                assert!(idx == 3, "load+store must share the CSE'd index slot; got {idx}");
            }
            _ => panic!("CSE+fusion failed to collapse the duplicated-index idiom: {micro:?}"),
        }
        assert!(
            !micro.iter().any(|op| matches!(op, MicroOp::ArrStore { .. })),
            "the store must be folded into the ArrRMW"
        );
    }

    fn named8_n(live: &[u16], n: usize) -> Vec<bool> {
        let mut v = vec![false; n];
        for &s in live {
            v[s as usize] = true;
        }
        v
    }

    /// FLOAT RMW (nbody `v[i] = v[i] + dx*mag`): an ArrLoad feeding an AddF whose
    /// result stores back to the same cell collapses to a float ArrRMW. The
    /// operand (the product) stays live; commutative AddF takes `t` on either
    /// side, SubF only on the left.
    #[test]
    fn float_load_addf_store_fuses_to_float_rmw() {
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 8, len_slot: 9, byte: false, checked: false },
            MicroOp::AddF { dst: 4, lhs: 3, rhs: 2 }, // v[i] + product
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 8, len_slot: 9, byte: false, checked: false },
            MicroOp::Return { src: 0 },
        ];
        fuse_array_rmw(&mut micro, &named8(&[2, 5]), 10, &HashSet::new());
        match micro[0] {
            MicroOp::ArrRMW { op: RmwOp::AddF, operand: 2, idx: 5, .. } => {}
            ref other => panic!("expected float ArrRMW AddF operand=2, got {other:?}"),
        }

        // operand - v[i] must NOT fuse (loaded value on the right of SubF).
        let mut rev = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 8, len_slot: 9, byte: false, checked: true },
            MicroOp::SubF { dst: 4, lhs: 2, rhs: 3 },
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 8, len_slot: 9, byte: false, checked: true },
            MicroOp::Return { src: 0 },
        ];
        fuse_array_rmw(&mut rev, &named8(&[2, 5]), 10, &HashSet::new());
        assert!(matches!(rev[0], MicroOp::ArrLoad { .. }), "operand - v[i] must not fuse: {rev:?}");
    }

    /// FLOAT MULTIPLY-ADD: `MulF(t = a*b); AddF(d = t + c)` with `t` single-use
    /// collapses to one FmaF. Commutative — the product may be either AddF
    /// operand; the addend `c` stays live.
    #[test]
    fn mulf_addf_fuses_to_fma() {
        for swap in [false, true] {
            let add = if swap {
                MicroOp::AddF { dst: 4, lhs: 2, rhs: 3 } // c + t
            } else {
                MicroOp::AddF { dst: 4, lhs: 3, rhs: 2 } // t + c
            };
            let mut micro = vec![
                MicroOp::MulF { dst: 3, lhs: 0, rhs: 1 }, // t = a*b
                add,
                MicroOp::Return { src: 4 },
            ];
            fuse_fma(&mut micro, &named8(&[0, 1, 2, 4]), 8, &HashSet::new());
            match micro[0] {
                MicroOp::FmaF { dst: 4, a: 0, b: 1, c: 2 } => {}
                ref other => panic!("expected FmaF(dst4,a0,b1,c2), got {other:?} (swap={swap})"),
            }
            assert!(!micro.iter().any(|op| matches!(op, MicroOp::AddF { .. })), "AddF folded away");
        }
    }

    /// A multi-use product must NOT fuse (the second reader would lose it), and a
    /// pinned operand blocks fusion (mem-form FmaF would need a spill).
    #[test]
    fn fma_blocks_multiuse_product_and_pinned_operands() {
        // product read twice
        let mut multi = vec![
            MicroOp::MulF { dst: 3, lhs: 0, rhs: 1 },
            MicroOp::AddF { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::AddF { dst: 5, lhs: 3, rhs: 2 }, // second use of the product
            MicroOp::Return { src: 4 },
        ];
        fuse_fma(&mut multi, &named8(&[0, 1, 2, 4, 5]), 8, &HashSet::new());
        assert!(matches!(multi[0], MicroOp::MulF { .. }), "multi-use product must block FMA");

        // a pinned operand (slot 0) blocks the spill-free fusion
        let mut pinned_in = vec![
            MicroOp::MulF { dst: 3, lhs: 0, rhs: 1 },
            MicroOp::AddF { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::Return { src: 4 },
        ];
        let pins: HashSet<u16> = [0u16].into_iter().collect();
        fuse_fma(&mut pinned_in, &named8(&[0, 1, 2, 4]), 8, &pins);
        assert!(matches!(pinned_in[0], MicroOp::MulF { .. }), "pinned operand must block FMA");
    }

    /// LEVER 1a — a pinned DESTINATION `d` (the accumulator/result XMM pin) must
    /// NOT block fusion: `FmaF` is mem-form, so a pinned `d` is spilled/reloaded
    /// by `compile_straightline_pinned_float`'s mem-form path exactly as it would
    /// be by the unfused `AddF`'s register-form result — fusing adds no extra
    /// spill on the INPUTS (which stay frame-resident). The nbody distance-sum's
    /// `MulF(t = dz*dz); AddF(d = sum + t)` with `d` float-pinned is the case the
    /// guard previously refused. Inputs `a`, `b`, the addend `c` stay frame-resident.
    #[test]
    fn fma_fuses_with_pinned_destination() {
        for swap in [false, true] {
            let add = if swap {
                MicroOp::AddF { dst: 4, lhs: 2, rhs: 3 } // c + t
            } else {
                MicroOp::AddF { dst: 4, lhs: 3, rhs: 2 } // t + c
            };
            let mut micro = vec![
                MicroOp::MulF { dst: 3, lhs: 0, rhs: 1 }, // t = a*b (a,b,t frame-resident)
                add,                                      // d = t + c, d pinned
                MicroOp::Return { src: 4 },
            ];
            let pins: HashSet<u16> = [4u16].into_iter().collect(); // ONLY the dst is pinned
            fuse_fma(&mut micro, &named8(&[0, 1, 2, 4]), 8, &pins);
            match micro[0] {
                MicroOp::FmaF { dst: 4, a: 0, b: 1, c: 2 } => {}
                ref other => panic!(
                    "expected FmaF(dst4,a0,b1,c2) with pinned dst, got {other:?} (swap={swap})"
                ),
            }
            assert!(
                !micro.iter().any(|op| matches!(op, MicroOp::AddF { .. })),
                "AddF must fold into the FmaF even when the dst is pinned"
            );
        }
    }

    /// LEVER 1a soundness boundary — a pinned ADDEND `c` (the loop-carried
    /// accumulator read on the FmaF's frame-input side, the `s = s + a*b` shape
    /// where `c == d == s`) must STILL block: spilling the pinned accumulator
    /// around the mem-form FmaF would cost more pieces than the register-form
    /// `AddF` it replaces. Only a pinned `d` that is NOT also an input is relaxed.
    #[test]
    fn fma_pinned_addend_still_blocks() {
        // accumulator shape: t = a*b ; s = s + t  (c == d == slot 4, pinned)
        let mut micro = vec![
            MicroOp::MulF { dst: 3, lhs: 0, rhs: 1 },
            MicroOp::AddF { dst: 4, lhs: 4, rhs: 3 }, // s = s + t, s is both c and d
            MicroOp::Return { src: 4 },
        ];
        let pins: HashSet<u16> = [4u16].into_iter().collect();
        fuse_fma(&mut micro, &named8(&[0, 1, 4]), 8, &pins);
        assert!(
            matches!(micro[0], MicroOp::MulF { .. }),
            "a pinned addend (loop-carried accumulator) must block FMA"
        );
    }

    /// Jump targets AFTER the idiom are remapped down by the two removed ops.
    #[test]
    fn fusion_remaps_later_jump_targets() {
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 3, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::Add { dst: 4, lhs: 3, rhs: 2 },
            MicroOp::ArrStore { src: 4, idx: 5, ptr_slot: 6, len_slot: 7, byte: false, checked: true },
            MicroOp::JumpIfTrue { cond: 2, target: 5 }, // -> the Return
            MicroOp::Add { dst: 1, lhs: 1, rhs: 2 },
            MicroOp::Return { src: 1 },
        ];
        fuse_array_rmw(&mut micro, &named8(&[1, 2, 5]), 8, &HashSet::new());
        // [ArrRMW, JumpIfTrue->3, Add, Return] — the target slid 5 -> 3.
        assert!(matches!(micro[0], MicroOp::ArrRMW { .. }));
        match micro[1] {
            MicroOp::JumpIfTrue { target, .. } => assert_eq!(target, 3, "target must remap 5->3"),
            ref other => panic!("expected JumpIfTrue, got {other:?}"),
        }
        assert!(matches!(micro[3], MicroOp::Return { .. }));
    }
}

#[cfg(test)]
mod fuse_ld2_tests {
    use super::*;
    use logicaffeine_forge::jit::IOp;
    use std::collections::HashSet;

    fn named_n(live: &[u16], n: usize) -> Vec<bool> {
        let mut v = vec![false; n];
        for &s in live {
            v[s as usize] = true;
        }
        v
    }

    /// THE MATMUL DOT-PRODUCT SHAPE: `ArrLoad(t1 = a[ix]); ArrLoad(t2 = b[jx]);
    /// Mul(d = t1 * t2)` — the two elements come from TWO DISTINCT pinned int
    /// buffers (a at ptr 10/len 11, b at ptr 12/len 13). The two single-use
    /// scratch loads collapse into ONE fused ArrLoad2 (two loads + multiply,
    /// three dispatches → one), so the loaded i64s never round-trip the frame.
    #[test]
    fn two_buffer_load_mul_fuses_to_arrld2() {
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 3, idx: 1, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::ArrLoad { dst: 4, idx: 2, ptr_slot: 12, len_slot: 13, byte: false, checked: true },
            MicroOp::Mul { dst: 5, lhs: 3, rhs: 4 },
            MicroOp::Return { src: 5 },
        ];
        fuse_array_ld2(&mut micro, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
        assert_eq!(micro.len(), 2, "the 3-op dot-product idiom must collapse to 1 op + Return");
        match micro[0] {
            MicroOp::ArrLoad2 { dst, i, j, ptr_a, len_a, ptr_b, len_b, op, checked } => {
                assert_eq!((dst, i, j), (5, 1, 2));
                assert_eq!((ptr_a, len_a, ptr_b, len_b), (10, 11, 12, 13));
                assert_eq!(op, IOp::Mul);
                assert!(checked);
            }
            ref other => panic!("expected ArrLoad2, got {other:?}"),
        }
    }

    /// Add commutes — the loads may feed the binop in either order, and the
    /// fused op records the index order that reproduces `a[i] OP b[j]`.
    #[test]
    fn add_fuses_in_either_operand_order() {
        for swap in [false, true] {
            let add = if swap {
                MicroOp::Add { dst: 5, lhs: 4, rhs: 3 } // b[j] + a[i]
            } else {
                MicroOp::Add { dst: 5, lhs: 3, rhs: 4 } // a[i] + b[j]
            };
            let mut micro = vec![
                MicroOp::ArrLoad { dst: 3, idx: 1, ptr_slot: 10, len_slot: 11, byte: false, checked: false },
                MicroOp::ArrLoad { dst: 4, idx: 2, ptr_slot: 12, len_slot: 13, byte: false, checked: false },
                add,
                MicroOp::Return { src: 5 },
            ];
            fuse_array_ld2(&mut micro, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
            match micro[0] {
                MicroOp::ArrLoad2 { op: IOp::Add, checked: false, .. } => {}
                ref other => panic!("expected unchecked ArrLoad2 Add (swap={swap}), got {other:?}"),
            }
        }
    }

    /// Subtraction is non-commutative, but the fusion is anchored on the binop
    /// and tracks WHICH load feeds the left vs. right operand, so it fuses BOTH
    /// orderings correctly by recording the left load as `a`/`i` and the right
    /// load as `b`/`j` (the stencil computes `a[i] - b[j]`).
    #[test]
    fn sub_fuses_in_binop_order() {
        // `X[1] - Y[2]`: left load = X (ptr 10), right load = Y (ptr 12).
        let mut fwd = vec![
            MicroOp::ArrLoad { dst: 3, idx: 1, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::ArrLoad { dst: 4, idx: 2, ptr_slot: 12, len_slot: 13, byte: false, checked: true },
            MicroOp::Sub { dst: 5, lhs: 3, rhs: 4 }, // X[1] - Y[2]
            MicroOp::Return { src: 5 },
        ];
        fuse_array_ld2(&mut fwd, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
        match fwd[0] {
            MicroOp::ArrLoad2 { op: IOp::Sub, i: 1, j: 2, ptr_a: 10, ptr_b: 12, .. } => {}
            ref other => panic!("expected ArrLoad2 Sub (X-Y): i=1 j=2 ptr_a=10 ptr_b=12, got {other:?}"),
        }

        // `Y[2] - X[1]`: left load = Y (ptr 12), right load = X (ptr 10). The
        // fused op must SWAP which buffer is `a`/`b` so it still computes Y - X.
        let mut rev = vec![
            MicroOp::ArrLoad { dst: 3, idx: 1, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::ArrLoad { dst: 4, idx: 2, ptr_slot: 12, len_slot: 13, byte: false, checked: true },
            MicroOp::Sub { dst: 5, lhs: 4, rhs: 3 }, // Y[2] - X[1]
            MicroOp::Return { src: 5 },
        ];
        fuse_array_ld2(&mut rev, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
        match rev[0] {
            MicroOp::ArrLoad2 { op: IOp::Sub, i: 2, j: 1, ptr_a: 12, ptr_b: 10, .. } => {}
            ref other => panic!("expected ArrLoad2 Sub (Y-X): i=2 j=1 ptr_a=12 ptr_b=10, got {other:?}"),
        }
    }

    /// A byte (Bool) buffer is not an 8-byte int load — the fused int stencil
    /// reads raw i64s, so a byte load must never fuse.
    #[test]
    fn byte_buffer_blocks_fusion() {
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 3, idx: 1, ptr_slot: 10, len_slot: 11, byte: true, checked: true },
            MicroOp::ArrLoad { dst: 4, idx: 2, ptr_slot: 12, len_slot: 13, byte: false, checked: true },
            MicroOp::Mul { dst: 5, lhs: 3, rhs: 4 },
            MicroOp::Return { src: 5 },
        ];
        fuse_array_ld2(&mut micro, &named_n(&[1, 2, 5], 14), 14, &HashSet::new());
        assert!(matches!(micro[0], MicroOp::ArrLoad { byte: true, .. }), "byte load must block fusion");
    }

    /// A multi-use loaded value must NOT be folded away (a later reader would
    /// lose it), and a jump landing on the second load or the binop blocks it.
    #[test]
    fn multiuse_load_and_inner_jump_target_block_fusion() {
        let mut multi = vec![
            MicroOp::ArrLoad { dst: 3, idx: 1, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::ArrLoad { dst: 4, idx: 2, ptr_slot: 12, len_slot: 13, byte: false, checked: true },
            MicroOp::Mul { dst: 5, lhs: 3, rhs: 4 },
            MicroOp::Move { dst: 6, src: 3 }, // second use of a[i]
            MicroOp::Return { src: 5 },
        ];
        fuse_array_ld2(&mut multi, &named_n(&[1, 2, 5, 6], 14), 14, &HashSet::new());
        assert!(matches!(multi[0], MicroOp::ArrLoad { .. }), "multi-use load must block fusion");

        let mut jt = vec![
            MicroOp::ArrLoad { dst: 3, idx: 1, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::ArrLoad { dst: 4, idx: 2, ptr_slot: 12, len_slot: 13, byte: false, checked: true },
            MicroOp::Mul { dst: 5, lhs: 3, rhs: 4 },
            MicroOp::JumpIfTrue { cond: 0, target: 1 }, // lands on the 2nd load
            MicroOp::Return { src: 5 },
        ];
        fuse_array_ld2(&mut jt, &named_n(&[0, 1, 2, 5], 14), 14, &HashSet::new());
        assert!(matches!(jt[0], MicroOp::ArrLoad { .. }), "inner jump target must block fusion");
    }
}

#[cfg(test)]
mod fuse_affine_tests {
    use super::*;
    use logicaffeine_forge::jit::AffOp;
    use std::collections::HashSet;

    // slots 0..n are registers; named marks the live ones. The const-holding
    // slot is left live (it is reused), the index temps are scratch (un-named).
    fn named_n(live: &[u16], n: usize) -> Vec<bool> {
        let mut v = vec![false; n];
        for &s in live {
            v[s as usize] = true;
        }
        v
    }

    /// THE `w + 1` SHAPE — `LoadConst(kc = 1); Add(idx = w + kc); ArrLoad(arr[idx])`.
    /// The index `idx` is single-use scratch, the const slot is reused; folds to
    /// one `ArrLoadAffine{op: None, a: w, const_offset: 1}` and the `Add` vanishes
    /// while the `LoadConst` survives.
    #[test]
    fn slot_plus_const_folds_to_affine_none() {
        // slot 4 = w (live index var), 8 = kc (live const), 5 = idx (scratch),
        // 6 = loaded value, 10/11 = ptr/len.
        let mut micro = vec![
            MicroOp::LoadConst { dst: 8, value: 1 },
            MicroOp::Add { dst: 5, lhs: 4, rhs: 8 },
            MicroOp::ArrLoad { dst: 6, idx: 5, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::Return { src: 6 },
        ];
        fuse_index_affine(&mut micro, &named_n(&[4, 8, 6], 12), 12, &HashSet::new());
        assert_eq!(micro.len(), 3, "Add folds into the load; LoadConst kept");
        match micro[1] {
            MicroOp::ArrLoadAffine { dst, a, op, const_offset, ptr_slot, len_slot, checked, .. } => {
                assert_eq!((dst, a, op, const_offset), (6, 4, AffOp::None, 1));
                assert_eq!((ptr_slot, len_slot, checked), (10, 11, true));
            }
            ref other => panic!("expected ArrLoadAffine None, got {other:?}"),
        }
        assert!(matches!(micro[0], MicroOp::LoadConst { dst: 8, value: 1 }), "shared const kept");
    }

    /// THE `w - wi + 1` SHAPE — `Sub(t = w - wi); LoadConst(kc = 1); Add(idx = t + kc);
    /// ArrLoad(arr[idx])`. Both `t` and `idx` are single-use scratch; folds to one
    /// `ArrLoadAffine{op: Sub, a: w, b: wi, const_offset: 1}` (the inner Sub AND the
    /// trailing Add vanish, the LoadConst survives).
    #[test]
    fn sub_plus_const_folds_to_affine_sub() {
        // 4 = w, 3 = wi, 8 = kc (live), 7 = t (scratch), 5 = idx (scratch).
        let mut micro = vec![
            MicroOp::Sub { dst: 7, lhs: 4, rhs: 3 },
            MicroOp::LoadConst { dst: 8, value: 1 },
            MicroOp::Add { dst: 5, lhs: 7, rhs: 8 },
            MicroOp::ArrLoad { dst: 6, idx: 5, ptr_slot: 10, len_slot: 11, byte: false, checked: false },
            MicroOp::Return { src: 6 },
        ];
        fuse_index_affine(&mut micro, &named_n(&[4, 3, 8, 6], 12), 12, &HashSet::new());
        assert_eq!(micro.len(), 3, "Sub + Add fold into the load; LoadConst kept");
        match micro[1] {
            MicroOp::ArrLoadAffine { dst, a, op, b, const_offset, checked, .. } => {
                assert_eq!((dst, a, op, b, const_offset, checked), (6, 4, AffOp::Sub, 3, 1, false));
            }
            ref other => panic!("expected ArrLoadAffine Sub, got {other:?}"),
        }
    }

    /// THE `i*n + j + 1` MATMUL READ SHAPE — `Mul(t1 = i*n)` is MULTI-USE (the
    /// matmul reuses `i*n`), so it must NOT be removed, but the trailing
    /// `Add(t2 = t1 + j); Add(idx = t2 + kc); ArrLoad` still folds: the inner
    /// add `t2` is single-use scratch over the slot `t1`, giving
    /// `ArrLoadAffine{op: Add, a: t1, b: j, const_offset: 1}`.
    #[test]
    fn matmul_read_folds_keeping_shared_mul() {
        // 6 = i, 1 = n, 14 = j, 8 = kc (live), 18 = t1 = i*n (MULTI-USE: read by
        // the inner add AND a later op), 17 = t2 (scratch), 16 = idx (scratch).
        let mut micro = vec![
            MicroOp::Mul { dst: 18, lhs: 6, rhs: 1 },        // i*n, reused
            MicroOp::Add { dst: 17, lhs: 18, rhs: 14 },      // i*n + j  (scratch)
            MicroOp::LoadConst { dst: 8, value: 1 },
            MicroOp::Add { dst: 16, lhs: 17, rhs: 8 },       // + 1      (scratch)
            MicroOp::ArrLoad { dst: 23, idx: 16, ptr_slot: 40, len_slot: 41, byte: false, checked: true },
            MicroOp::Move { dst: 99, src: 18 },              // second use of i*n
            MicroOp::Return { src: 23 },
        ];
        fuse_index_affine(&mut micro, &named_n(&[6, 1, 14, 8, 23, 99], 100), 100, &HashSet::new());
        // The two inner Adds collapse into the load; the Mul and LoadConst stay.
        match micro.iter().find(|o| matches!(o, MicroOp::ArrLoadAffine { .. })) {
            Some(MicroOp::ArrLoadAffine { dst, a, op, b, const_offset, .. }) => {
                assert_eq!((*dst, *a, *op, *b, *const_offset), (23, 18, AffOp::Add, 14, 1));
            }
            other => panic!("expected ArrLoadAffine, got {other:?}"),
        }
        assert!(micro.iter().any(|o| matches!(o, MicroOp::Mul { dst: 18, .. })), "shared i*n Mul kept");
    }

    /// THE BARE `a + b` SHAPE (no const) — `Add(idx = a + b); ArrLoad` folds to
    /// `ArrLoadAffine{op: Add, const_offset: 0}`.
    #[test]
    fn bare_two_slot_add_folds_with_zero_offset() {
        let mut micro = vec![
            MicroOp::Add { dst: 5, lhs: 4, rhs: 3 },
            MicroOp::ArrLoad { dst: 6, idx: 5, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::Return { src: 6 },
        ];
        fuse_index_affine(&mut micro, &named_n(&[4, 3, 6], 12), 12, &HashSet::new());
        assert_eq!(micro.len(), 2, "the Add folds into the load");
        match micro[0] {
            MicroOp::ArrLoadAffine { a, op, b, const_offset, .. } => {
                assert_eq!((a, op, b, const_offset), (4, AffOp::Add, 3, 0));
            }
            ref other => panic!("expected ArrLoadAffine Add, got {other:?}"),
        }
    }

    /// A MULTI-USE INDEX (the slot is read by a store too, as matmul's `c[idx]`)
    /// must NOT fold — folding would drop the index the store still needs.
    #[test]
    fn multiuse_index_blocks_fusion() {
        let mut micro = vec![
            MicroOp::LoadConst { dst: 8, value: 1 },
            MicroOp::Add { dst: 5, lhs: 4, rhs: 8 },
            MicroOp::ArrLoad { dst: 6, idx: 5, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::ArrStore { src: 6, idx: 5, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::Return { src: 6 },
        ];
        fuse_index_affine(&mut micro, &named_n(&[4, 8, 6], 12), 12, &HashSet::new());
        assert!(
            matches!(micro[2], MicroOp::ArrLoad { .. }),
            "an index reused by a store must block the fold: {micro:?}"
        );
    }

    /// A BYTE (Bool) load never folds — the affine stencils read raw i64s.
    #[test]
    fn byte_load_blocks_fusion() {
        let mut micro = vec![
            MicroOp::LoadConst { dst: 8, value: 1 },
            MicroOp::Add { dst: 5, lhs: 4, rhs: 8 },
            MicroOp::ArrLoad { dst: 6, idx: 5, ptr_slot: 10, len_slot: 11, byte: true, checked: true },
            MicroOp::Return { src: 6 },
        ];
        fuse_index_affine(&mut micro, &named_n(&[4, 8, 6], 12), 12, &HashSet::new());
        assert!(matches!(micro[2], MicroOp::ArrLoad { byte: true, .. }), "byte load must not fold");
    }

    /// A PINNED index temp must not fold (the fused op reads it from the frame;
    /// folding past a pin would lose the pinned value's settling).
    #[test]
    fn pinned_index_temp_blocks_fusion() {
        let mut micro = vec![
            MicroOp::Add { dst: 5, lhs: 4, rhs: 3 },
            MicroOp::ArrLoad { dst: 6, idx: 5, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::Return { src: 6 },
        ];
        let mut pins = HashSet::new();
        pins.insert(5u16);
        fuse_index_affine(&mut micro, &named_n(&[4, 3, 6], 12), 12, &pins);
        assert!(matches!(micro[0], MicroOp::Add { .. }), "a pinned index must block the fold");
    }

    /// A jump landing on the load (or between the index op and the load) must
    /// block the fold — an entry there would skip the index computation.
    #[test]
    fn jump_target_on_load_blocks_fusion() {
        let mut micro = vec![
            MicroOp::Add { dst: 5, lhs: 4, rhs: 3 },
            MicroOp::ArrLoad { dst: 6, idx: 5, ptr_slot: 10, len_slot: 11, byte: false, checked: true },
            MicroOp::JumpIfTrue { cond: 4, target: 1 }, // lands on the load
            MicroOp::Return { src: 6 },
        ];
        fuse_index_affine(&mut micro, &named_n(&[4, 3, 6], 12), 12, &HashSet::new());
        assert!(matches!(micro[0], MicroOp::Add { .. }), "jump onto the load blocks the fold");
    }

    /// COND-SWAP WITH A COMPUTED SECOND INDEX (Priority 2) — the sort inner-loop
    /// compare-and-swap idiom `ArrLoad(a, i1); <index arith for i2>; ArrLoad(b, i2);
    /// Branch(cmp, a, b); ArrStore(i1, b); ArrStore(i2, a)` must still fuse into one
    /// `ArrCondSwap` when `Mul`/`Add`/`LoadConst` (the `2*root+2`-style index
    /// computation of the SECOND access) sit BETWEEN the two loads. The
    /// between-ops are pure and redefine neither the first loaded value, the
    /// first index, nor the pointer/length, so they do not block the fusion.
    #[test]
    fn cond_swap_tolerates_computed_index_between_loads() {
        // i1 = slot 5 (live), root = slot 4 (live). i2 = 2*root + 2 computed
        // between the loads (slots 8 = kc2, 7 = 2*root, 6 = i2; all scratch).
        // a = slot 9, b = slot 10 (the two loaded values, dead after the swap).
        let mut micro = vec![
            MicroOp::ArrLoad { dst: 9, idx: 5, ptr_slot: 20, len_slot: 21, byte: false, checked: true },
            MicroOp::LoadConst { dst: 8, value: 2 },
            MicroOp::Mul { dst: 7, lhs: 4, rhs: 8 },     // 2*root
            MicroOp::Add { dst: 6, lhs: 7, rhs: 8 },     // 2*root + 2  (the i2 index)
            MicroOp::ArrLoad { dst: 10, idx: 6, ptr_slot: 20, len_slot: 21, byte: false, checked: true },
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 9, rhs: 10, target: 8 },
            MicroOp::ArrStore { src: 10, idx: 5, ptr_slot: 20, len_slot: 21, byte: false, checked: true },
            MicroOp::ArrStore { src: 9, idx: 6, ptr_slot: 20, len_slot: 21, byte: false, checked: true },
            MicroOp::Return { src: 0 },
        ];
        // gap from load1 to load2 is 3 (LoadConst, Mul, Add) — raise the search
        // window by running with the same named/pin config the pipeline uses.
        fuse_cond_swap(&mut micro, &named_n(&[4, 5], 22), 22, &HashSet::new());
        assert!(
            micro.iter().any(|o| matches!(o, MicroOp::ArrCondSwap { idx1: 5, idx2: 6, .. })),
            "the compare-and-swap must fuse despite the 2*root+2 index arith between \
             the loads: {micro:?}"
        );
    }

    /// EVAL PARITY — `AffOp::eval` reproduces the kernel's wrapping i64 index
    /// arithmetic exactly (the differential gate's bit-for-bit contract).
    #[test]
    fn affop_eval_matches_wrapping_kernel_semantics() {
        assert_eq!(AffOp::None.eval(7, 999, 1), 8);
        assert_eq!(AffOp::Add.eval(40, 3, 1), 44);
        assert_eq!(AffOp::Sub.eval(10, 4, 1), 7);
        assert_eq!(AffOp::Mul.eval(6, 7, 1), 43);
        // Wrapping at the i64 edges, identical to the kernel.
        assert_eq!(AffOp::Add.eval(i64::MAX, 1, 0), i64::MAX.wrapping_add(1));
        assert_eq!(AffOp::Mul.eval(i64::MIN, -1, 0), i64::MIN.wrapping_mul(-1));
        assert_eq!(AffOp::Sub.eval(i64::MIN, 1, 0), i64::MIN.wrapping_sub(1));
    }
}
