//! The bytecode dispatch loop.
//!
//! Registers live in one contiguous `Vec<Value>`; `base` is the current frame's
//! offset into it, so every register access is `registers[base + r]`. Calls use
//! register windowing — the callee's frame starts at the caller's `args_start`,
//! so arguments are passed with zero copying.

use super::instruction::{BoundaryType, CompiledProgram, Constant, FuncIdx, ObsSiteKind, Op, Reg};
use super::value::Value;
use super::MAX_REGISTER_FILE;
use logicaffeine_runtime::{ChanId, RtPayload, SelectArm, TaskId};

/// Callee analysis for list-param call admission — may a region call this function while passing a
/// pinned list argument? Returns `(list_params_stable, returns_list_param)`,
/// both SOUND under-approximations:
/// - `list_params_stable`: no operation which can reallocate receives a list
///   parameter or one of its aliases. Allocating and pushing an unrelated
///   local list is harmless, as are scalar-only helper calls.
/// - `returns_list_param`: every `Return` traces — through `Move`s, to a
///   fixpoint — to a list PARAMETER slot, and all list params share one element
///   kind (the returned list kind is then unambiguous from the signature). A
///   purely scalar return makes this `false` and rides `ret` instead.
fn analyze_list_call_safety(
    body: &[Op],
    param_count: u16,
    param_kinds: &[Option<super::native_tier::ParamKind>],
    register_count: usize,
) -> (bool, bool) {
    use super::native_tier::ParamKind;
    let n = register_count.max(param_count as usize);
    // Slots that (transitively via Move) hold a list-parameter handle.
    let mut is_param_list = vec![false; n];
    for i in 0..param_count as usize {
        if matches!(param_kinds.get(i), Some(Some(ParamKind::List(_)))) {
            is_param_list[i] = true;
        }
    }
    loop {
        let mut changed = false;
        for op in body {
            match *op {
                Op::Move { dst, src } => {
                    let (d, s) = (dst as usize, src as usize);
                    if d < n && s < n && is_param_list[s] && !is_param_list[d] {
                        is_param_list[d] = true;
                        changed = true;
                    }
                }
                Op::RevSwap { a, b } => {
                    let (a, b) = (a as usize, b as usize);
                    if a < n && b < n && (is_param_list[a] || is_param_list[b]) {
                        if !is_param_list[a] || !is_param_list[b] {
                            changed = true;
                        }
                        is_param_list[a] = true;
                        is_param_list[b] = true;
                    }
                }
                _ => {}
            }
        }
        if !changed {
            break;
        }
    }
    let list_params_stable = !body.iter().any(|op| match *op {
        Op::ListPush { list, .. } => is_param_list.get(list as usize).copied().unwrap_or(true),
        Op::Call {
            args_start,
            arg_count,
            ..
        } => (args_start..args_start.saturating_add(arg_count))
            .any(|r| is_param_list.get(r as usize).copied().unwrap_or(true)),
        _ => false,
    });
    // All list params must share a single element kind so the return's list kind
    // is unambiguous from the signature alone.
    let mut elem: Option<super::native_tier::PinElem> = None;
    let mut uniform = true;
    for pk in param_kinds {
        if let Some(ParamKind::List(e)) = pk {
            if elem.is_some() && elem != Some(*e) {
                uniform = false;
            }
            elem = Some(*e);
        }
    }
    let returns: Vec<u16> = body
        .iter()
        .filter_map(|op| {
            if let Op::Return { src } = *op {
                Some(src)
            } else {
                None
            }
        })
        .collect();
    let returns_list_param = uniform
        && !returns.is_empty()
        && returns
            .iter()
            .all(|&s| (s as usize) < n && is_param_list[s as usize]);
    (list_params_stable, returns_list_param)
}

/// The list element representation a native caller may receive by Vec handle.
/// Keep this in lock-step with the function tier's `ListByHandle` lanes.
fn native_list_return(return_type: Option<&BoundaryType>) -> Option<super::native_tier::PinElem> {
    use super::native_tier::PinElem;
    match return_type {
        Some(BoundaryType::Seq(elem)) => match elem.as_ref() {
            BoundaryType::Int => Some(PinElem::Int),
            BoundaryType::Word32 => Some(PinElem::Word32),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `LOGOS_JIT_CANARY=1` armed the region-frame sentinel guard
/// (read once; the per-region path stays branch-cheap). Off by default and
/// in release, so normal runs pay nothing — it is a diagnostic for native
/// out-of-bounds writes.
fn jit_canary_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOGOS_JIT_CANARY").is_ok_and(|v| v == "1"))
}

/// How a native region run left the loop.
enum RegionExit {
    /// Fell out the loop's exit edge — resume at this pc.
    At(usize),
    /// Install the native callee frames after restoring the caller's region.
    Call {
        call_pc: usize,
        resume_pc: usize,
        frames: Vec<super::native_tier::NativeFrame>,
        list_args: Vec<Value>,
    },
    /// Hit an in-region `Return` — perform the function return with this value.
    Return(Value),
}

/// What the native boundary decided for one call.
enum NativeDisposition {
    /// Completed natively; here is the re-boxed result.
    Done(Value),
    /// Run this call on bytecode (not compiled / guard mismatch / replay
    /// deopt).
    Interpret,
    /// Precise deopt: push these frames and resume at `resume_pc`.
    Materialize {
        resume_pc: usize,
        frames: Vec<super::native_tier::NativeFrame>,
        list_args: Vec<Value>,
    },
}

#[derive(Clone, Copy)]
struct CallFrame {
    return_pc: usize,
    return_reg: Reg,
    caller_base: usize,
    restore_len: usize,
    /// Iterator-stack depth at call entry; a Return unwinds any iterators the
    /// callee left open (e.g. `Return` inside a `Repeat`).
    iter_depth: usize,
    /// The function whose body this frame runs — selects the per-frame
    /// named-register map for loop regions tiering up inside it.
    func: u16,
    /// Absolute register index of this call's argument window (`callee_base`) and
    /// how many arguments it holds. On return these slots are nulled: as the
    /// callee's parameters they persist below `restore_len`, and a collection
    /// argument would otherwise leave a live `Rc` clone in the caller's frame,
    /// inflating `strong_count` and forcing needless copy-on-write later. Zero
    /// `arg_count` (native/scheduler frames) clears nothing.
    arg_lo: usize,
    arg_count: u16,
}

/// Which registers of one code region's frame are LIVE entering each op of the region — read on
/// some path before they are next written. A register not live at a pc holds nothing the frame
/// will read again, so a collection it still references is a copy nobody can observe.
///
/// Built from the one register-operand enumeration ([`super::instruction::visit_regs`], through
/// `op_def_uses`) and the ops' own control successors, both over-approximating what an op reads:
/// liveness can only come out too large, never too small, so "dead" is always true.
struct FrameLiveness {
    /// The region `[start, end)` of `program.code` this covers.
    start: usize,
    end: usize,
    /// `u64` words per pc; bit `r` of a pc's words is register `r`.
    words: usize,
    live_in: Vec<u64>,
}

impl FrameLiveness {
    fn of(program: &CompiledProgram, start: usize, end: usize, regs: usize) -> FrameLiveness {
        let words = regs.div_ceil(64).max(1);
        let n = end - start;
        let mut defs = vec![0u64; n * words];
        let mut uses = vec![0u64; n * words];
        let mut succ: Vec<Vec<usize>> = Vec::with_capacity(n);
        // A successor outside the region is a pc this frame cannot see into: everything is live there.
        let mut exits = vec![false; n];
        let set = |bits: &mut [u64], i: usize, r: Reg| {
            if (r as usize) < regs {
                bits[i * words + r as usize / 64] |= 1u64 << (r % 64);
            }
        };
        for i in 0..n {
            let op = &program.code[start + i];
            let (d, u) = super::instruction::op_def_uses(op, &program.functions);
            for r in d {
                set(&mut defs, i, r);
            }
            for r in u {
                set(&mut uses, i, r);
            }
            let mut inside = Vec::new();
            for t in super::instruction::op_successors(op, start + i, end) {
                if (start..end).contains(&t) {
                    inside.push(t - start);
                } else {
                    exits[i] = true;
                }
            }
            succ.push(inside);
        }
        let mut live_in = vec![0u64; n * words];
        let mut out = vec![0u64; words];
        let mut changed = true;
        while changed {
            changed = false;
            for i in (0..n).rev() {
                out.iter_mut()
                    .for_each(|w| *w = if exits[i] { u64::MAX } else { 0 });
                for &t in &succ[i] {
                    for w in 0..words {
                        out[w] |= live_in[t * words + w];
                    }
                }
                for w in 0..words {
                    let v = uses[i * words + w] | (out[w] & !defs[i * words + w]);
                    if v != live_in[i * words + w] {
                        live_in[i * words + w] = v;
                        changed = true;
                    }
                }
            }
        }
        FrameLiveness {
            start,
            end,
            words,
            live_in,
        }
    }

    /// Whether register `r` may still be read entering `pc`. A pc outside the region answers
    /// `true`: nothing is known about it.
    fn is_live(&self, pc: usize, r: Reg) -> bool {
        if !(self.start..self.end).contains(&pc) || r as usize >= self.words * 64 {
            return true;
        }
        let i = pc - self.start;
        self.live_in[i * self.words + r as usize / 64] & (1u64 << (r % 64)) != 0
    }
}

/// The allocation a List, Map or Set value lives in — the identity copy-on-write asks about.
fn collection_ptr(v: &Value) -> Option<usize> {
    use crate::interpreter::RuntimeValue;
    use std::rc::Rc;
    match v.as_runtime_ref()? {
        RuntimeValue::List(rc) => Some(Rc::as_ptr(rc) as *const u8 as usize),
        RuntimeValue::Map(rc) => Some(Rc::as_ptr(rc) as *const u8 as usize),
        RuntimeValue::Set(rc) => Some(Rc::as_ptr(rc) as *const u8 as usize),
        _ => None,
    }
}

/// How many holders a List, Map or Set value's allocation has (its `Rc` strong count); 1 otherwise.
fn holder_count(v: &Value) -> usize {
    use crate::interpreter::RuntimeValue;
    use std::rc::Rc;
    match v.as_runtime_ref() {
        Some(RuntimeValue::List(rc)) => Rc::strong_count(rc),
        Some(RuntimeValue::Map(rc)) => Rc::strong_count(rc),
        Some(RuntimeValue::Set(rc)) => Rc::strong_count(rc),
        _ => 1,
    }
}

/// [`collection_ptr`] of a List, Map or Set that has more than one holder.
fn shared_collection(v: &Value) -> Option<usize> {
    use crate::interpreter::RuntimeValue;
    use std::rc::Rc;
    let shared = match v.as_runtime_ref()? {
        RuntimeValue::List(rc) => Rc::strong_count(rc) > 1,
        RuntimeValue::Map(rc) => Rc::strong_count(rc) > 1,
        RuntimeValue::Set(rc) => Rc::strong_count(rc) > 1,
        _ => false,
    };
    if shared {
        collection_ptr(v)
    } else {
        None
    }
}

/// The outcome of one `run_until_block` slice (T11). A non-concurrent program
/// always returns `Done` on the first slice, so `run()` behaves exactly as the
/// old single-shot loop. A concurrent task returns `Blocked` at each scheduler
/// op; the driver reads [`Vm::take_pending`] and re-enters after the block clears.
pub(crate) enum VmStep {
    /// The (sub)program ran to completion (`Halt` or code exhausted) with the
    /// given result payload (the main/return value, `Nothing` if none).
    Done(crate::interpreter::RuntimeValue),
    /// Suspended at a concurrency op; [`Vm::take_pending`] carries the request.
    Blocked,
    /// Suspended by the debug stepper after exhausting its per-call op budget
    /// (`STEPPED = true` only). Resumable on the next [`Vm::run_steps`]. The
    /// production path (`run_until_block`, `STEPPED = false`) never yields this.
    Paused,
}

/// A read-only view of the paused VM for the Studio debug drawer: the program
/// counter, the live call frames (Main first, current last) with their register
/// values, the named globals, and the output so far. Values are rendered with the
/// same `to_display_string` the `Show` op uses.
pub(crate) struct DebugView {
    pub pc: usize,
    pub frames: Vec<DebugFrameView>,
    pub globals: Vec<(String, String)>,
    pub output: Vec<String>,
}

/// One call frame's registers in a [`DebugView`]. `func` is `None` for Main; `base`
/// is the frame's start offset in the linear register file (its stack address).
pub(crate) struct DebugFrameView {
    pub func: Option<u16>,
    pub base: usize,
    /// `(index, type-name, display-value)` per register, e.g. `(1, "Int", "6")`.
    pub registers: Vec<(u16, String, String)>,
}

/// One heap-allocated object (list / map / set / tuple / text / struct) reachable
/// from the current frame or the globals — the heap-viewer's unit. `id` is the live
/// allocation address (so two roots sharing one object share an `id` → aliasing), and
/// `rc` is its reference count.
pub(crate) struct HeapObjView {
    pub id: usize,
    pub kind: String,
    pub summary: String,
    /// The underlying storage layout (e.g. `packed Vec<i64>`, `columnar`) — teaches
    /// how the data is actually laid out in memory.
    pub storage: String,
    pub rc: usize,
    pub referenced_by: Vec<String>,
}

/// The heap identity of a value — its allocation address, kind, and storage-layout label.
/// `None` for inline scalars (Int/Float/Bool/Char/…), which live in the register slot
/// itself and are not heap objects.
fn heap_identity(val: &Value) -> Option<(usize, String, String)> {
    object_identity(val.as_runtime_ref()?)
}

fn object_identity(rv: &crate::interpreter::RuntimeValue) -> Option<(usize, String, String)> {
    use crate::interpreter::RuntimeValue as RV;
    use std::rc::Rc;
    let s = |x: &str| x.to_string();
    match rv {
        RV::List(rc) => Some((
            Rc::as_ptr(rc) as usize,
            s("list"),
            rc.borrow().storage_label().to_string(),
        )),
        RV::Map(rc) => Some((Rc::as_ptr(rc) as usize, s("map"), s("hash map"))),
        RV::Set(rc) => Some((Rc::as_ptr(rc) as usize, s("set"), s("vec set"))),
        RV::Tuple(rc) => Some((Rc::as_ptr(rc) as usize, s("tuple"), s("fixed tuple"))),
        RV::Text(rc) => Some((Rc::as_ptr(rc) as usize, s("text"), s("Rc<String>"))),
        RV::Struct(b) => Some((&**b as *const _ as usize, s("struct"), s("field map"))),
        RV::Inductive(b) => Some((&**b as *const _ as usize, s("enum"), s("tagged variant"))),
        _ => None,
    }
}

/// How many handles the running program holds on each heap allocation, keyed by the
/// allocation's address: one per root that holds it, plus one per slot of any container
/// the roots reach. A container is entered once however many handles reach it, so its
/// contents count once. This is the program's own reference count — `Rc::strong_count`
/// would also count every copy held outside the roots, such as the debugger's history.
pub(crate) fn count_handles<'a>(
    roots: impl IntoIterator<Item = &'a crate::interpreter::RuntimeValue>,
) -> std::collections::HashMap<usize, usize> {
    use crate::interpreter::RuntimeValue as RV;
    use std::collections::{HashMap, HashSet};

    fn visit(v: &RV, counts: &mut HashMap<usize, usize>, entered: &mut HashSet<usize>) {
        let Some((id, _, _)) = object_identity(v) else {
            return;
        };
        *counts.entry(id).or_insert(0) += 1;
        if !entered.insert(id) {
            return;
        }
        match v {
            RV::List(l) => {
                for item in l.borrow().to_values() {
                    visit(&item, counts, entered);
                }
            }
            RV::Set(items) => {
                for item in items.borrow().iter() {
                    visit(item, counts, entered);
                }
            }
            RV::Map(m) => {
                for (k, item) in m.borrow().iter() {
                    visit(k, counts, entered);
                    visit(item, counts, entered);
                }
            }
            RV::Tuple(items) => {
                for item in items.iter() {
                    visit(item, counts, entered);
                }
            }
            RV::Struct(s) => {
                for item in s.fields.values() {
                    visit(item, counts, entered);
                }
            }
            RV::Inductive(ind) => {
                for item in ind.args.iter() {
                    visit(item, counts, entered);
                }
            }
            _ => {}
        }
    }

    let mut counts = HashMap::new();
    let mut entered = HashSet::new();
    for root in roots {
        visit(root, &mut counts, &mut entered);
    }
    counts
}

/// The resumable execution state of a single-task program — enough to pause it and
/// resume in a freshly-built `tier: None` VM. The debugger owns the
/// [`CompiledProgram`] and rebuilds the VM each step (it cannot hold a borrowing
/// `Vm<'p>` across steps), threading this snapshot through. Concurrency request
/// state is intentionally omitted (the debugger is single-task, bytecode-tier).
#[derive(Clone)]
pub(crate) struct DebugVmState {
    registers: Vec<Value>,
    base: usize,
    globals: Vec<Option<Value>>,
    lines: Vec<String>,
    iter_stack: Vec<(Vec<Value>, usize)>,
    sched_active: bool,
    sched_pc: usize,
    sched_call_stack: Vec<CallFrame>,
}

impl DebugVmState {
    /// The pc the program is stopped at (the op about to execute).
    pub(crate) fn pc(&self) -> usize {
        self.sched_pc
    }
    /// Call-stack depth (0 = in Main), for step-over / step-out.
    pub(crate) fn call_depth(&self) -> usize {
        self.sched_call_stack.len()
    }
    /// What the program has printed up to this state.
    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
    }
}

/// A concurrency request a suspended [`Vm`] hands to the scheduler driver — the
/// VM analog of the tree-walker's `BlockingRequest`. A spawned child travels as a
/// fully-built `Vm` (sharing the parent's `&'p program`), which the driver wraps
/// in its own task.
pub(crate) enum VmBlock {
    /// Create a channel (`None` = the scheduler's default capacity); resume with its id.
    NewChan(Option<usize>),
    /// Send a value into a channel (blocks if full).
    Send(ChanId, RtPayload),
    /// Receive from a channel (blocks if empty); resume with the value.
    Recv(ChanId),
    /// Non-blocking send; resume with `Bool(success)`.
    TrySend(ChanId, RtPayload),
    /// Non-blocking receive; resume with the value or `Nothing`.
    TryRecv(ChanId),
    /// Close a channel.
    Close(ChanId),
    /// Spawn a child *by descriptor* — function index + materialised args — so the
    /// driver builds the child `Vm` (the cooperative driver inline, a work-stealing
    /// worker locally over its own program). `want_handle` distinguishes a launch
    /// that binds a task handle. Resume with the child's `TaskId`.
    SpawnDesc {
        func: FuncIdx,
        args: Vec<RtPayload>,
        want_handle: bool,
    },
    /// Await a task's completion; resume with its result payload.
    Await(TaskId),
    /// Abort a task.
    Abort(TaskId),
    /// Block on the first ready select arm; resume with the winning arm index.
    Select(Vec<SelectArm>),
    /// Sleep for some logical ticks.
    Sleep(u64),
    /// Dial the relay (async); resume when connected. Carries the URL value.
    NetConnect(RtPayload),
    /// Subscribe our inbox (async); resume when subscribed. Carries the topic value.
    NetListen(RtPayload),
    /// Encode + publish to a peer; resume immediately. Carries `(peer, message)`.
    NetSend(RtPayload, RtPayload),
    /// Batch-stream a list to a peer; resume immediately. Carries `(peer, list)`.
    NetStream(RtPayload, RtPayload),
    /// Await a message (or batch stream, if the flag) from a peer (blocks); resume with the value.
    /// Carries `(peer, stream_flag)`.
    NetAwait(RtPayload, bool),
    /// Resolve an address value into a PeerAgent handle (its canonical topic); resume with the peer.
    /// Carries the address value.
    NetMakePeer(RtPayload),
    /// CRDT sync point: publish the current counter, merge what has arrived, resume with the merged
    /// value. Carries `(topic, current)`.
    NetSync(RtPayload, RtPayload),
    /// A tcp.lg native (the `TcpOp` code + materialised operands); resume with the
    /// LOGOS `Result` value from the shared `TcpRegistry::exec_values`.
    Tcp(u8, Vec<RtPayload>),
    /// Serve HTTP: the bound `port` and the handler `FuncIdx`. The async io
    /// driver owns the concurrent accept loop and re-enters the handler as a
    /// fresh child `Vm` per connection. The loop never returns, so this block
    /// never resumes.
    Serve(u16, FuncIdx),
}

pub struct Vm<'p> {
    program: &'p CompiledProgram,
    /// The constant pool MATERIALISED into runtime values once at construction.
    /// A `LoadConst` then clones the pre-built `Value` — for a heap `Text` that
    /// is an `Rc` refcount bump, not a fresh `String`+`Rc` allocation, so a
    /// 1-char literal reloaded every iteration of a hot loop (string_search's
    /// `ch`) costs no heap traffic. The pool keeps a live reference, so a
    /// freshly-loaded literal is never the sole owner and the in-place
    /// `add_assign` append correctly declines to mutate it.
    const_pool: Vec<Value>,
    registers: Vec<Value>,
    base: usize,
    /// One element per `Show` (a shown value may itself contain newlines —
    /// it is still ONE output line, like the tree-walker's emit callback).
    lines: Vec<String>,
    /// Copies and elements copied when a record field must become privately owned.
    record_field_copy_work: (usize, usize),
    /// Live `Repeat` snapshots: (elements, next index). Stack-disciplined —
    /// `IterPrepare` pushes, `IterPop` pops, nesting nests.
    iter_stack: Vec<(Vec<Value>, usize)>,
    /// Promoted globals (None = not yet defined; reading one is the
    /// "Undefined variable" error).
    globals: Vec<Option<Value>>,
    /// Policy registry + interner for `Check` statements (absent ⇒ the
    /// tree-walker's "Security Check requires policies" error).
    policy_ctx: Option<(
        &'p crate::analysis::PolicyRegistry,
        &'p crate::intern::Interner,
    )>,
    /// The pluggable native tier (None = pure bytecode, e.g. WASM).
    tier: Option<&'p dyn super::native_tier::NativeTier>,
    /// FuncIdx of crypto.lg's `pbkdf2Sha256`, when present — the interpreted VM
    /// substitutes the byte-identical native reference (`crypto_accel`) for its
    /// pure Hi() loop (4096 iterations ≈ 16k SHA-256 compressions is far too slow
    /// to run as bytecode for a live SCRAM handshake). Set by `with_policy_ctx`,
    /// which carries the interner. Mirrors the tree-walker's `pbkdf2_accel`.
    accel_pbkdf2: Option<FuncIdx>,
    /// Per-function call counts (profiling toward the tier threshold).
    hot: Vec<u32>,
    /// Per-function native state.
    native: Vec<super::native_tier::NativeSlot>,
    /// Back-edge counts for MAIN loops (keyed by loop-head pc). FxHash: this
    /// is probed once per back-edge crossing of every Main loop that has not
    /// (or cannot) tier up — a per-iteration cost on the bytecode path.
    region_hot: rustc_hash::FxHashMap<usize, u32>,
    /// Compiled Main-loop regions (keyed by loop-head pc; same probe rate).
    regions: rustc_hash::FxHashMap<usize, super::native_tier::RegionSlot>,
    /// Per-region (loop-head pc) collection registers this region mutates IN
    /// PLACE. Under value semantics these are copy-on-write'd at region ENTRY
    /// (`ensure_reg_owned`) so the native code's in-place writes cannot alias a
    /// shared allocation — the perf-preserving follow-up to the correctness-first
    /// decline. Only populated when the region is provably alias-free (a mutated
    /// collection never escapes it), so entry-COW alone isolates it soundly.
    region_cow_regs: rustc_hash::FxHashMap<usize, Vec<u16>>,
    /// Per-pc dead-region bitset: once a loop head is known `Failed`
    /// (un-tierable, or demoted after repeated guard misses) its entry here is
    /// set, so the back-edge hook short-circuits with a single `Vec<bool>`
    /// index instead of re-hashing `regions` on every iteration. Loops that
    /// never tier (effectful bodies, `Text` ops, list-param fns) are the common
    /// case and pay only this O(1) check after the first failure. Indexed by
    /// loop-head pc; sized to the code length.
    region_blacklist: Vec<bool>,
    /// Program arguments for the `args()` system native — full argv, index 0 is
    /// the program name (mirrors the compiled binary's `env::args()`). Empty
    /// when none were supplied.
    program_args: Vec<String>,
    /// Per-program native-tier context: the EXODIA 4.7 entry table plus the
    /// shared deopt-status and live-depth cells every chain patches.
    native_ctx: super::native_tier::NativeCtx,
    /// The off-thread native compiler (HOTSWAP §6), present only when the VM was
    /// given the process-installed `&'static` tier via [`Vm::with_bg_native_tier`].
    /// `None` ⇒ compile synchronously on this thread (the retained fallback, and the
    /// only path for a borrowed `&'p` tier). Native-only: needs `std::thread`+forge.
    #[cfg(not(target_arch = "wasm32"))]
    bg: Option<super::bg_compile::BgCompiler>,
    /// Axis-1 warm-bytecode side-table (HOTSWAP §7 / P11): re-optimized function
    /// bodies appended here, in the same pc space *after* `program.code`. A `Call`
    /// to a function with a `warm_entry` jumps into this buffer instead of the
    /// baseline `entry_pc`. Pure bytecode — no forge, no `rustc` — so it is the
    /// browser's hot-swap tier. Empty until a body is installed, and every read
    /// path is gated on `pc >= program.code.len()`, so the baseline run loop is
    /// byte-for-byte unchanged when nothing is warm.
    warm_code: Vec<Op>,
    /// Per-function warm entry (indexed by function index): the absolute pc of the
    /// body in the unified `program.code ++ warm_code` space, and its register
    /// window. `None` ⇒ the function runs its baseline body.
    warm_entry: Vec<Option<WarmEntry>>,

    /// Resumable-execution state for the scheduler driver (T11). When a task
    /// suspends at a concurrency op, `run_until_block` saves its `pc` + call stack
    /// here and restores them on the next slice. A non-concurrent run never sets
    /// `sched_active`, so it starts fresh at pc 0 — byte-for-byte the old loop.
    sched_active: bool,
    sched_pc: usize,
    sched_call_stack: Vec<CallFrame>,
    /// Ops run by every [`Vm::run_steps`] slice so far: a fueled run's driver reads the difference across a
    /// slice to charge the fuel what the slice spent, whether it paused, blocked, failed or finished.
    stepped: u64,
    /// The concurrency request produced by the last `Blocked` slice (taken by the
    /// driver). `None` between slices and for a non-concurrent run.
    pending: Option<VmBlock>,
    /// The register the next resume value is delivered into (`None` for a block
    /// that yields nothing, e.g. `Send`/`Close`).
    resume_slot: Option<Reg>,
    /// Whether the value resuming into [`Self::resume_slot`] must be wrapped as an `Option`.
    ///
    /// A `Try to receive` answers "a value, or nothing", which IS an Option — but the SCHEDULER
    /// hands back the bare payload (or `Nothing`), and the same generic resume path serves the
    /// blocking `Recv`/`Await` where the value is NOT optional. So the wrap cannot live in the
    /// driver; the VM records here which kind of block reserved the slot.
    resume_wraps_option: bool,
    /// Accumulated `Select` arms awaiting a `SelectWait`: each runtime arm plus
    /// the register a winning recv arm binds its value into. Persists across the
    /// block so `deliver_select` can route the received value to the right arm.
    select_pending: Vec<(SelectArm, Option<Reg>)>,
    /// The first failure a page entry the program took itself ([`BuiltinId::DomDispatchEntry`])
    /// contained and no failure handler absorbed, with the failure site in force when it trapped:
    /// the program's own failure once nothing is left to run ([`Vm::run_until_block`]).
    ///
    /// [`BuiltinId::DomDispatchEntry`]: crate::semantics::builtins::BuiltinId::DomDispatchEntry
    unhandled: Option<(String, Option<u32>)>,
    /// The failure site of the run's end, when a contained failure ended it: what
    /// [`Vm::failing_site`] answers then, the site the failure trapped at.
    ended_at_site: Option<Option<u32>>,
    /// Reverse-execution history tape (Bennett/Landauer garbage), `Some` ONLY during a
    /// [`Vm::run_recording`] pass. The production `run` path never sets or reads it, and every
    /// recording block lives behind the `const RECORDING` monomorph, so the 2.5×-C hot loop is
    /// byte-for-byte unchanged (`RECORDING = false` dead-strips it, exactly like `STEPPED`).
    journal: Option<super::journal::HistoryTape>,
    /// The pebbling plan (per-pc journal decisions) for the active recording mode — `Some` only
    /// during a recording pass. Consulted by `record_op` (skip an elided scratch write) and by the
    /// reverse executor (skip restoring it), so an elided entry is neither pushed nor popped.
    journal_plan: Option<Vec<super::pebble::JournalDecision>>,
    /// The reversibility mode the active recording was set up for. `Adiabatic` restores elided scratch
    /// to its known default on reverse (full fidelity at the Landauer floor); the others do not.
    reverse_mode: super::reverse::ReverseMode,
    /// (`Adiabatic`, Piece 2) Reverse-live scratch RECOMPUTED on reverse instead of journaled, keyed
    /// by `(frame, register)` (register numbers are frame-relative) → its pure defining op, re-run with
    /// `self.base` set to the frame just before a reverse step reads it.
    recompute: std::collections::HashMap<(Option<usize>, Reg), Op>,
    /// (Corner 2) The number of recompute-op re-executions performed during a reverse pass — the
    /// load-bearing instrument for the pebble time/space schedule. `0` outside `Adiabatic`. A
    /// budget-0 (recompute-everything) reverse of a chain read `K` times costs `≈ chain·K`;
    /// checkpointing bounded intermediates drops it toward the Bennett `O(T^{1+ε})` point.
    recompute_ops: usize,
    /// (Corner 2) The pebble CHECKPOINT budget for `Adiabatic`: the max number of recomputable chain
    /// nodes kept ON the tape (journaled once, `landauer = 0` since they are recomputable) so downstream
    /// recomputes stop at the nearest checkpoint instead of re-walking to the chain root. `0` = the
    /// recompute-everything corner (`O(1)` tape, chain·reads time); higher = the space-for-time trade.
    pebble_budget: usize,
    /// WS6 (Phase 13): the browser WASM-JIT tier. Consulted from `Op::Call` only under the
    /// `wasm-jit` feature; entirely absent from the default build (and behind the native x86
    /// forge tier on native, so it is the JIT tier specifically where forge cannot run —
    /// wasm32).
    #[cfg(feature = "wasm-jit")]
    wasm_tier: super::wasm_jit::WasmTier,
    /// How the code stream partitions into Main and each function ([`super::reach::code_layout`]),
    /// resolved the first time a copy-on-write needs a frame's region; `Some(None)` when the layout
    /// does not resolve, and no dead register is ever released.
    code_layout: Option<Option<super::reach::CodeLayout>>,
    /// Each region's [`FrameLiveness`], keyed by its first pc, built the first time a
    /// copy-on-write inside that region finds its collection shared.
    liveness: rustc_hash::FxHashMap<usize, std::rc::Rc<FrameLiveness>>,
    /// The run real numbers the program defines by a function read their functions in, opened the
    /// first time the program defines one ([`Vm::open_defined_run`]).
    defined_run: Option<crate::semantics::defined::RunScope<'static>>,
}

/// A warm function body's location in the unified pc space (`program.code` then
/// `warm_code`) plus the register window it executes in.
#[derive(Clone, Copy, Debug)]
struct WarmEntry {
    entry_pc: usize,
    register_count: usize,
}

impl<'p> Vm<'p> {
    pub fn new(program: &'p CompiledProgram) -> Self {
        Vm {
            program,
            const_pool: program.constants.iter().map(const_to_value).collect(),
            registers: vec![Value::nothing(); program.register_count],
            base: 0,
            lines: Vec::new(),
            record_field_copy_work: (0, 0),
            iter_stack: Vec::new(),
            globals: vec![None; program.globals.len()],
            policy_ctx: None,
            tier: None,
            accel_pbkdf2: None,
            hot: vec![0; program.functions.len()],
            native: (0..program.functions.len())
                .map(|_| super::native_tier::NativeSlot::Untried)
                .collect(),
            region_hot: rustc_hash::FxHashMap::default(),
            regions: rustc_hash::FxHashMap::default(),
            region_cow_regs: rustc_hash::FxHashMap::default(),
            region_blacklist: vec![false; program.code.len()],
            program_args: crate::semantics::argv_with_program_name(Vec::new()),
            native_ctx: super::native_tier::NativeCtx {
                table: std::sync::Arc::new(super::native_tier::FnTable::new(
                    program.functions.len(),
                )),
                status: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
                depth: std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0)),
            },
            #[cfg(not(target_arch = "wasm32"))]
            bg: None,
            warm_code: Vec::new(),
            warm_entry: vec![None; program.functions.len()],
            sched_active: false,
            sched_pc: 0,
            sched_call_stack: Vec::new(),
            stepped: 0,
            pending: None,
            resume_slot: None,
            resume_wraps_option: false,
            select_pending: Vec::new(),
            unhandled: None,
            ended_at_site: None,
            journal: None,
            journal_plan: None,
            reverse_mode: super::reverse::ReverseMode::Observable,
            recompute: std::collections::HashMap::new(),
            recompute_ops: 0,
            pebble_budget: 0,
            #[cfg(feature = "wasm-jit")]
            wasm_tier: super::wasm_jit::WasmTier::new(50),
            code_layout: None,
            liveness: rustc_hash::FxHashMap::default(),
            defined_run: None,
        }
    }

    /// Open the run real numbers the program defines by a function read their functions in, once.
    /// Its evaluator calls a closure on a fresh machine over the same bytecode, which shares nothing
    /// with this one but the program, so a digit read in the middle of an operation leaves this
    /// machine's registers, frames and globals as they were.
    fn open_defined_run(&mut self) -> &crate::semantics::defined::RunScope<'static> {
        if self.defined_run.is_none() {
            let program = std::rc::Rc::new(self.program.clone());
            let evaluator: crate::semantics::defined::Evaluator<'static> =
                std::rc::Rc::new(move |closure, n| {
                    Vm::new(&program).call_closure(closure, vec![crate::interpreter::RuntimeValue::Int(n)])
                });
            self.defined_run = Some(crate::semantics::defined::open_run(evaluator));
        }
        self.defined_run.as_ref().expect("the run is open")
    }

    /// Actual record-field copy work during this VM instance's lifetime:
    /// (copies, elements copied), saturating at usize::MAX.
    pub fn record_field_copy_work(&self) -> (usize, usize) {
        self.record_field_copy_work
    }

    /// Install a re-optimized body as function `fi`'s warm tier (HOTSWAP §7 / P11):
    /// append it to `warm_code` after `program.code`, rebasing its 0-relative jumps
    /// into that unified pc space, and point `warm_entry[fi]` at it. Subsequent calls
    /// to `fi` run this body. The body shares the program's constant pool (a
    /// `FnBytecode` preserves constant indices), so only jumps are relocated.
    pub fn install_warm_bytecode(
        &mut self,
        fi: usize,
        fnbc: &super::fn_bytecode::FnBytecode,
    ) -> bool {
        // Refuse a structurally-invalid body (out-of-range jump/call, missing terminal
        // op) or one whose arity disagrees with the baseline function — a corrupt cache
        // entry or a buggy producer then falls back to baseline instead of fetching past
        // the warm buffer (panic) or reading the wrong registers (HOTSWAP §P12 robustness).
        if !fnbc.is_well_formed(self.program.functions.len()) {
            return false;
        }
        match self.program.functions.get(fi) {
            Some(f) if f.param_count == fnbc.param_count => {}
            _ => return false,
        }
        let abs_base = self.program.code.len() + self.warm_code.len();
        self.warm_code.extend(
            fnbc.code
                .iter()
                .map(|&op| super::fn_bytecode::rebase(op, abs_base as isize)),
        );
        if fi >= self.warm_entry.len() {
            self.warm_entry.resize(fi + 1, None);
        }
        self.warm_entry[fi] = Some(WarmEntry {
            entry_pc: abs_base,
            register_count: fnbc.register_count,
        });
        let name = self.fn_name(fi);
        super::tier_trace::trace_transition(fi, &name, super::tier_trace::ExecTier::Warm);
        true
    }

    /// Mark a loop head permanently `Failed` and record it in the per-pc
    /// blacklist so the back-edge hook never probes `regions` for it again.
    /// Every `RegionSlot::Failed` transition routes through here so the bitset
    /// can never drift out of sync with the map.
    fn mark_region_failed(&mut self, head: usize) {
        self.regions
            .insert(head, super::native_tier::RegionSlot::Failed);
        if let Some(slot) = self.region_blacklist.get_mut(head) {
            *slot = true;
        }
    }

    /// The collection registers a region mutates IN PLACE (the collection
    /// operand of every mutation op in its body).
    fn region_mutated_collection_regs(body: &[Op]) -> rustc_hash::FxHashSet<u16> {
        let mut s = rustc_hash::FxHashSet::default();
        for op in body {
            match op {
                Op::ListPush { list: c, .. }
                | Op::SetAdd { set: c, .. }
                | Op::RemoveFrom { collection: c, .. }
                | Op::SetIndex { collection: c, .. }
                | Op::SetIndexUnchecked { collection: c, .. }
                | Op::ListPop { list: c, .. } => {
                    s.insert(*c);
                }
                _ => {}
            }
        }
        s
    }

    /// Collection registers that hold a FRESH, uniquely-owned collection for the
    /// whole region: a `NewEmpty*{dst=C}` op DOMINATES every in-place mutation of
    /// `C`. Such a collection is created anew on each entry to its live range, so its
    /// mutation can NEVER alias — it needs no entry copy-on-write, and any use of
    /// `C`'s register BEFORE the fresh definition is a disjoint (scalar) live range
    /// that register-recycling left behind (fannkuch's `Set r to r-1` scratch landing
    /// on `perm`'s slot before `perm` is created). Excluding these from the mutated
    /// set keeps the region tier-able under value semantics WITHOUT weakening
    /// soundness: a genuinely shared/aliased mutation has no dominating fresh
    /// definition, so it stays in the set and is COW'd or declined.
    ///
    /// `body` is `program.code[head..=back]`; region-relative index `i` is pc `head+i`.
    fn region_fresh_collection_regs(body: &[Op], head: usize) -> rustc_hash::FxHashSet<u16> {
        let n = body.len();
        let mut out = rustc_hash::FxHashSet::default();
        if n == 0 {
            return out;
        }
        // Region-relative successors (an edge leaving [head, back] is dropped — a
        // fresh definition need only dominate mutations WITHIN the region).
        let rel = |target: usize| -> Option<usize> { target.checked_sub(head).filter(|&r| r < n) };
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, op) in body.iter().enumerate() {
            match op {
                Op::Jump { target } => {
                    if let Some(r) = rel(*target) {
                        succs[i].push(r);
                    }
                }
                Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
                    if let Some(r) = rel(*target) {
                        succs[i].push(r);
                    }
                    if i + 1 < n {
                        succs[i].push(i + 1);
                    }
                }
                Op::Return { .. } | Op::ReturnNothing | Op::Halt => {}
                _ => {
                    if i + 1 < n {
                        succs[i].push(i + 1);
                    }
                }
            }
        }
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, ss) in succs.iter().enumerate() {
            for &s in ss {
                preds[s].push(i);
            }
        }
        // Iterative dominators over the region CFG (entry = relative 0 = `head`).
        // `dom[i][k]` == node k dominates node i. Regions are small, so the O(n²) set
        // representation is fine. Unreachable nodes keep the full (all-true) set —
        // harmless: they never execute, so any "fresh" verdict on them is moot.
        let mut dom: Vec<Vec<bool>> = vec![vec![true; n]; n];
        dom[0] = vec![false; n];
        dom[0][0] = true;
        let mut changed = true;
        while changed {
            changed = false;
            for i in 1..n {
                if preds[i].is_empty() {
                    continue;
                }
                let mut new = vec![true; n];
                for &p in &preds[i] {
                    for (k, nk) in new.iter_mut().enumerate() {
                        *nk &= dom[p][k];
                    }
                }
                new[i] = true;
                if new != dom[i] {
                    dom[i] = new;
                    changed = true;
                }
            }
        }
        // Mutation positions per collection reg, and fresh-definition positions.
        let mut muts: rustc_hash::FxHashMap<u16, Vec<usize>> = rustc_hash::FxHashMap::default();
        let mut news: rustc_hash::FxHashMap<u16, Vec<usize>> = rustc_hash::FxHashMap::default();
        for (i, op) in body.iter().enumerate() {
            match op {
                Op::ListPush { list: c, .. }
                | Op::SetAdd { set: c, .. }
                | Op::RemoveFrom { collection: c, .. }
                | Op::SetIndex { collection: c, .. }
                | Op::SetIndexUnchecked { collection: c, .. }
                | Op::ListPop { list: c, .. } => muts.entry(*c).or_default().push(i),
                Op::NewEmptyList { dst }
                | Op::NewEmptyListWithCapacity { dst, .. }
                | Op::NewEmptyListFixed { dst, .. }
                | Op::NewEmptySet { dst }
                | Op::NewEmptyMap { dst }
                | Op::NewEmptyMapWithCapacity { dst, .. }
                | Op::NewEmptyListI32 { dst } => news.entry(*dst).or_default().push(i),
                _ => {}
            }
        }
        for (c, mpos) in &muts {
            if let Some(npos) = news.get(c) {
                // Fresh iff SOME fresh-definition position dominates EVERY mutation.
                if npos.iter().any(|&q| mpos.iter().all(|&m| dom[m][q])) {
                    out.insert(*c);
                }
            }
        }
        out
    }

    /// True if any mutated-collection register is COPIED, ALIASED, redefined, or
    /// otherwise escapes the region — so region-entry copy-on-write alone cannot
    /// keep it isolated and the region must run on the value-semantic VM. An
    /// in-place collection mutation and a pure read use the collection soundly
    /// (its buffer stays private to the region); ANY other operand use of a
    /// mutated register — or any un-modelled op — fails closed.
    fn region_mutation_escapes(body: &[Op], m: &rustc_hash::FxHashSet<u16>) -> bool {
        let h = |r: &u16| m.contains(r);
        let rng = |s: u16, c: u16| (s..s.saturating_add(c)).any(|r| m.contains(&r));
        body.iter().any(|op| Self::op_escapes_mutated(op, &h, &rng))
    }

    fn op_escapes_mutated(
        op: &Op,
        h: &impl Fn(&u16) -> bool,
        rng: &impl Fn(u16, u16) -> bool,
    ) -> bool {
        match op {
            // In-place mutation: the collection operand is isolated by the entry
            // COW; only the OTHER operands can leak/redefine it.
            Op::ListPush { value, .. }
            | Op::SetAdd { value, .. }
            | Op::RemoveFrom { value, .. } => h(value),
            Op::SetIndex { index, value, .. } | Op::SetIndexUnchecked { index, value, .. } => {
                h(index) || h(value)
            }
            Op::ListPop { dst, .. } => h(dst),
            // Pure reads of a collection.
            Op::Index { dst, index, .. } | Op::IndexUnchecked { dst, index, .. } => {
                h(dst) || h(index)
            }
            Op::Length { dst, .. } => h(dst),
            Op::Contains { dst, value, .. } => h(dst) || h(value),
            Op::RegionBoundsGuard { bound, iv, .. } => h(bound) || h(iv),
            // Creating a FRESH collection in a mutated register is safe: the new
            // buffer is uniquely owned, so its in-place mutation cannot alias
            // (an aliasing copy would still be caught by the other arms). This is
            // the fresh-list-per-iteration pattern (`Let mutable p be a new Seq`).
            Op::NewEmptyList { .. }
            | Op::NewEmptyListFixed { .. }
            | Op::NewEmptySet { .. }
            | Op::NewEmptyMap { .. }
            | Op::NewEmptyListI32 { .. } => false,
            Op::NewEmptyListWithCapacity { capacity, .. }
            | Op::NewEmptyMapWithCapacity { capacity, .. } => h(capacity),
            Op::NewRange { start, end, .. } => h(start) || h(end),
            Op::NewList { start, count, .. } | Op::NewTuple { start, count, .. } => {
                rng(*start, *count)
            }
            // Scalars / control flow: decline only if a mutated reg is an operand
            // (a redefinition of the collection reg, or a scalar read of it).
            Op::LoadConst { dst, .. }
            | Op::GlobalGet { dst, .. }
            | Op::LoadToday { dst }
            | Op::LoadNow { dst }
            | Op::Args { dst }
            | Op::IterNext { dst, .. } => h(dst),
            // A call-site COW barrier may redefine (clone) the register — decline a
            // region if it targets a mutated collection reg. In practice it never
            // appears in a tier-able region (it sits beside a `Call`, and a region
            // with a call declines regardless).
            Op::EnsureOwned { reg } => h(reg),
            Op::Move { dst, src }
            | Op::Not { dst, src }
            | Op::AddAssign { dst, src }
            | Op::FormatValue { dst, src, .. } => h(dst) || h(src),
            Op::Add { dst, lhs, rhs }
            | Op::Sub { dst, lhs, rhs }
            | Op::Mul { dst, lhs, rhs }
            | Op::Div { dst, lhs, rhs }
            | Op::ExactDiv { dst, lhs, rhs }
            | Op::FloorDiv { dst, lhs, rhs }
            | Op::Mod { dst, lhs, rhs }
            | Op::Lt { dst, lhs, rhs }
            | Op::Gt { dst, lhs, rhs }
            | Op::LtEq { dst, lhs, rhs }
            | Op::GtEq { dst, lhs, rhs }
            | Op::Eq { dst, lhs, rhs }
            | Op::NotEq { dst, lhs, rhs }
            | Op::ApproxEq { dst, lhs, rhs }
            | Op::Pow { dst, lhs, rhs }
            | Op::BitXor { dst, lhs, rhs }
            | Op::BitAnd { dst, lhs, rhs }
            | Op::BitOr { dst, lhs, rhs }
            | Op::Shl { dst, lhs, rhs }
            | Op::Shr { dst, lhs, rhs } => h(dst) || h(lhs) || h(rhs),
            Op::MagicDivU { dst, lhs, .. } | Op::DivPow2 { dst, lhs, .. } => h(dst) || h(lhs),
            // Collection-producing binops READ their operands and yield a FRESH,
            // independent result — decline only if a mutated reg is involved.
            Op::Concat { dst, lhs, rhs }
            | Op::SeqConcat { dst, lhs, rhs }
            | Op::UnionOp { dst, lhs, rhs }
            | Op::IntersectOp { dst, lhs, rhs } => h(dst) || h(lhs) || h(rhs),
            // Enum-arm test/bind and struct-field read: scalar-shaped, no alias.
            Op::TestArm { dst, target, .. } | Op::BindArm { dst, target, .. } => {
                h(dst) || h(target)
            }
            Op::GetField { dst, obj, .. } => h(dst) || h(obj),
            Op::DestructureTuple { src, start, count } => h(src) || rng(*start, *count),
            Op::Jump { .. } | Op::ReturnNothing | Op::IterPop => false,
            Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => h(cond),
            Op::IterPrepare { iterable } => h(iterable),
            Op::Sleep { duration } => h(duration),
            // Everything else — a Move/Concat producing a live copy, calls,
            // closures, spawns, channels, CRDTs, global stores, returns, struct/
            // tuple/inductive builders, Show, slices, deep-clones — could retain
            // or alias the collection. Fail closed.
            _ => true,
        }
    }

    /// Back-edge hook for hot loops in ANY frame (`Jump` to an earlier pc):
    /// profile, compile when hot, and — when ready and the guard passes — run
    /// the region natively. `named`/`frame_regs` describe the ENCLOSING frame
    /// (Main's or a function's). `cur_func` is the enclosing function index (for
    /// the region-entry COW's mutable-param check). Returns the pc to resume at
    /// (the loop's exit).
    fn try_region(
        &mut self,
        head: usize,
        back_pc: usize,
        named: &[bool],
        frame_regs: usize,
        depth_now: usize,
        cur_func: Option<u16>,
    ) -> Option<RegionExit> {
        use super::native_tier::{RegionSlot, REGION_TIER_THRESHOLD};
        let tier = self.tier?;
        match self.regions.get(&head) {
            Some(RegionSlot::Failed) => return None,
            Some(RegionSlot::Ready { .. }) => {}
            None => {
                let n = self.region_hot.entry(head).or_insert(0);
                *n += 1;
                if *n < REGION_TIER_THRESHOLD {
                    return None;
                }
                // Region extent: every jump leaving [head, back_pc] must
                // agree on ONE exit pc.
                let body = &self.program.code[head..=back_pc];
                let mut exit: Option<usize> = None;
                for op in body {
                    if let Op::Jump { target }
                    | Op::JumpIfFalse { target, .. }
                    | Op::JumpIfTrue { target, .. } = *op
                    {
                        if !(head..=back_pc).contains(&target) {
                            match exit {
                                None => exit = Some(target),
                                Some(e) if e == target => {}
                                _ => {
                                    self.mark_region_failed(head);
                                    return None;
                                }
                            }
                        }
                    }
                }
                let Some(exit_pc) = exit else {
                    self.mark_region_failed(head);
                    return None;
                };
                // Value semantics: a region that mutates a collection IN PLACE
                // may be writing a SHARED (aliased) allocation — native code
                // writes through the `Rc` directly, a reference-semantics
                // miscompile. We tier it soundly by copy-on-write'ing each
                // mutated collection at region ENTRY (isolating it), PROVIDED no
                // mutated collection escapes the region (then entry-COW is not
                // enough — decline, run on the value-semantic VM). A `mutable`
                // param is intentionally shared with the caller, so its in-place
                // mutation is correct and it is NOT COW'd.
                let mut cow_regs: Vec<u16> = Vec::new();
                if crate::semantics::collections::value_semantics_enabled() {
                    let mut mutated = Self::region_mutated_collection_regs(body);
                    // A collection created FRESH in-region (its `NewEmpty` dominates
                    // every mutation) is uniquely owned by construction: it needs no
                    // entry-COW, and its register's earlier recycled-scratch uses no
                    // longer read as a spurious alias-escape (the fannkuch `perm`
                    // whose slot a `Set r to r-1` scratch reused before `perm` exists).
                    for r in Self::region_fresh_collection_regs(body, head) {
                        mutated.remove(&r);
                    }
                    if !mutated.is_empty() {
                        if Self::region_mutation_escapes(body, &mutated) {
                            self.mark_region_failed(head);
                            return None;
                        }
                        let mutable_params = cur_func
                            .and_then(|fi| self.program.functions.get(fi as usize))
                            .map(|f| f.cow_exempt_regs.as_slice())
                            .unwrap_or(&[]);
                        cow_regs = mutated
                            .into_iter()
                            .filter(|r| !mutable_params.contains(r))
                            .collect();
                    }
                }
                let reg_count = u16::try_from(frame_regs).ok()?;
                // Speculation seed: the kinds sitting in this frame's
                // registers RIGHT NOW. The adapter compiles against them;
                // the guard set re-checks them on every entry.
                let observed: Vec<super::native_tier::ObservedKind> = (0..frame_regs)
                    .map(|r| {
                        use super::native_tier::ObservedKind;
                        use crate::interpreter::{ListRepr, RuntimeValue};
                        let rt = self.registers.get(self.base + r).map(|v| v.as_runtime());
                        match rt.as_deref() {
                            Some(RuntimeValue::Int(_)) => ObservedKind::Int,
                            // A BigInt is a promoted (overflowed) integer — still an
                            // integer kind, so the region tiers the slot as Int. The
                            // entry guard re-checks the representation: a real BigInt in
                            // an Int-guarded slot fails the guard and stays in the exact
                            // VM, so the native i64 fast path is never entered with a box.
                            Some(RuntimeValue::BigInt(_)) => ObservedKind::Int,
                            Some(RuntimeValue::Float(_)) => ObservedKind::Float,
                            Some(RuntimeValue::Bool(_)) => ObservedKind::Bool,
                            Some(RuntimeValue::List(rc)) => match &*rc.borrow() {
                                ListRepr::Ints(_) => ObservedKind::IntList,
                                ListRepr::IntsI32(_) => ObservedKind::IntListI32,
                                // A packed `Word32` list pins with the IntList buffer machinery (its masked
                                // values are `i64`s); the forge tags an indexed read `Word32`.
                                ListRepr::Words32(_) => ObservedKind::Word32List,
                                ListRepr::Floats(_) => ObservedKind::FloatList,
                                ListRepr::Bools(_) => ObservedKind::BoolList,
                                ListRepr::Boxed(_)
                                | ListRepr::Strings { .. }
                                | ListRepr::Structs { .. }
                                | ListRepr::Inductives { .. }
                                | ListRepr::WireStructs { .. }
                                | ListRepr::WireColumn { .. } => ObservedKind::Other,
                            },
                            Some(RuntimeValue::Map(_)) => ObservedKind::Map,
                            // An ASCII Text rides the byte-pin lane (char index ==
                            // byte index, char count == byte length). The metrics
                            // cache makes the ASCII test O(1) per crossing. A
                            // non-ASCII Text stays Other → the region bails and the
                            // per-char decode path runs, so the JIT never diverges.
                            Some(RuntimeValue::Text(rc))
                                if crate::semantics::collections::text_is_ascii(rc) =>
                            {
                                ObservedKind::TextBytes
                            }
                            // A Word ring value carried unboxed as a masked i64 — lets a loop-carried Word
                            // (a hash word `h0..h7`) enter the region as state. A narrow word's ring is
                            // not a lane's: the region would wrap it at 2³², so it never enters unboxed.
                            Some(RuntimeValue::Word(w)) => match w {
                                logicaffeine_base::WordVal::W64(_) => ObservedKind::Word64,
                                logicaffeine_base::WordVal::W32(_) => ObservedKind::Word32,
                                logicaffeine_base::WordVal::W8(_)
                                | logicaffeine_base::WordVal::W16(_) => ObservedKind::Other,
                            },
                            _ => ObservedKind::Other,
                        }
                    })
                    .collect();
                let callees: Vec<super::native_tier::CalleeSig> = {
                    let prog = &self.program;
                    let code_len = prog.code.len();
                    prog.functions
                        .iter()
                        .map(|f| {
                            let end = prog
                                .functions
                                .iter()
                                .map(|h| h.entry_pc)
                                .filter(|&pc| pc > f.entry_pc)
                                .min()
                                .unwrap_or(code_len);
                            let (list_params_stable, returns_list_param) = analyze_list_call_safety(
                                &prog.code[f.entry_pc..end],
                                f.param_count,
                                &f.param_kinds,
                                f.register_count,
                            );
                            super::native_tier::CalleeSig {
                                param_kinds: f.param_kinds.clone(),
                                inout_params: f
                                    .mutable_param_regs
                                    .iter()
                                    .copied()
                                    .filter(|r| !f.cow_exempt_regs.contains(r))
                                    .collect(),
                                ret: f.ret_kind,
                                list_ret: native_list_return(f.return_type.as_ref()),
                                list_params_stable,
                                returns_list_param,
                            }
                        })
                        .collect()
                };
                // A fresh-list return normally REBINDS `dst`. The native
                // region adopts its contents into the destination's pinned
                // buffer, so isolate that buffer at entry like any other
                // in-place mutation to preserve value semantics for aliases.
                if crate::semantics::collections::value_semantics_enabled() {
                    for op in body {
                        let Op::Call { dst, func, .. } = *op else {
                            continue;
                        };
                        if callees
                            .get(func as usize)
                            .is_some_and(|sig| sig.list_ret.is_some() && !sig.returns_list_param)
                            && !cow_regs.contains(&dst)
                        {
                            cow_regs.push(dst);
                        }
                    }
                }
                // PRECISE REGION LIVE-OUT: a name bound INSIDE this loop is
                // lexically dead at the loop exit, so it must NOT be written
                // back — dropping it from `named` lets the JIT's copy-prop / CSE
                // / fusion treat it as true scratch. `loop_locals[head]` is the
                // compiler's exact per-loop set; absent (no loop record) keeps
                // the conservative full `named`.
                let liveout_off = std::env::var("LOGOS_LIVEOUT").as_deref() == Ok("0");
                if std::env::var_os("LOGOS_LIVEOUT_TRACE").is_some() {
                    let ll = self.program.loop_locals.get(&head);
                    let nnamed = named.iter().filter(|&&n| n).count();
                    let freed = ll.map_or(0, |m| {
                        named
                            .iter()
                            .enumerate()
                            .filter(|(r, &n)| n && m.get(*r).copied().unwrap_or(false))
                            .count()
                    });
                    eprintln!(
                        "liveout-trace: head={head} named={nnamed} loop_locals={} freed={freed}",
                        ll.is_some()
                    );
                }
                let region_named: Vec<bool> = match self.program.loop_locals.get(&head) {
                    Some(locals) if !liveout_off => named
                        .iter()
                        .enumerate()
                        .map(|(r, &n)| n && !locals.get(r).copied().unwrap_or(false))
                        .collect(),
                    _ => named.to_vec(),
                };
                match tier.compile_region(
                    body,
                    head,
                    exit_pc,
                    &self.program.constants,
                    reg_count,
                    &region_named,
                    &observed,
                    &self.native_ctx,
                    &callees,
                ) {
                    Some(rf) => {
                        #[cfg(not(target_arch = "wasm32"))]
                        if super::perf_map::observed() {
                            let enclosing = cur_func
                                .map(|fi| self.fn_name(fi as usize))
                                .unwrap_or_default();
                            super::perf_map::record_code(
                                &rf.code_extents(),
                                &super::perf_map::region_symbol(&enclosing, head),
                            );
                        }
                        self.regions.insert(
                            head,
                            RegionSlot::Ready {
                                rf,
                                exit_pc,
                                misses: 0,
                            },
                        );
                        if !cow_regs.is_empty() {
                            self.region_cow_regs.insert(head, cow_regs);
                        }
                    }
                    None => {
                        self.mark_region_failed(head);
                        return None;
                    }
                }
            }
        }
        let result = self.run_ready_region(head, depth_now, cur_func);
        if result.is_none() {
            // Guard failure or side exit: count it; a region that keeps
            // missing re-runs work every entry — demote to pure bytecode.
            let mut demote = false;
            if let Some(RegionSlot::Ready { misses, .. }) = self.regions.get_mut(&head) {
                *misses += 1;
                if *misses >= super::native_tier::REGION_DEMOTE_AFTER {
                    demote = true;
                }
            }
            if demote {
                self.mark_region_failed(head);
            }
        }
        result
    }

    /// The Ready-path body of [`Vm::try_region`]: guards, pinning, the native
    /// run, and write-back. None = guard failure or side exit (the caller
    /// counts misses).
    fn run_ready_region(
        &mut self,
        head: usize,
        depth_now: usize,
        cur_func: Option<u16>,
    ) -> Option<RegionExit> {
        use super::native_tier::RegionSlot;
        // Region-entry copy-on-write: isolate each collection this region mutates
        // in place, so the native code's in-place writes cannot leak through a
        // shared `Rc`. A no-op when already uniquely owned; a one-time deep clone
        // when aliased. `mutable`-param collections were excluded at formation
        // (their sharing is intentional), so this only isolates value bindings.
        if let Some(regs) = self.region_cow_regs.get(&head) {
            for r in regs.clone() {
                self.ensure_reg_owned(r, head, cur_func);
            }
        }
        let Some(RegionSlot::Ready { rf, exit_pc, .. }) = self.regions.get(&head) else {
            unreachable!()
        };
        // Guard: every live-in slot must hold exactly the kind the region
        // speculated on; copy the raw representation in (floats as bits).
        {
            use super::native_tier::SlotKind;
            use crate::interpreter::RuntimeValue;
            for &(r, kind) in rf.guard_set() {
                let v = self.registers.get(self.base + r as usize)?;
                match (kind, &*v.as_runtime()) {
                    (SlotKind::Int, RuntimeValue::Int(_)) => {}
                    (SlotKind::Float, RuntimeValue::Float(_)) => {}
                    (SlotKind::Bool, RuntimeValue::Bool(_)) => {}
                    (SlotKind::Word32, RuntimeValue::Word(logicaffeine_base::WordVal::W32(_))) => {}
                    (SlotKind::Word64, RuntimeValue::Word(logicaffeine_base::WordVal::W64(_))) => {}
                    _ => return None,
                }
            }
        }
        // Frequently re-entered regions (sift-down loops) cannot afford a
        // heap allocation per entry — reuse one thread-local buffer.
        thread_local! {
            static REGION_FRAME: std::cell::RefCell<Vec<i64>> =
                const { std::cell::RefCell::new(Vec::new()) };
        }
        // `LOGOS_JIT_CANARY=1` guards the region frame with a sentinel
        // canary past its live span (`need` = frame proper + call-arena
        // headroom): any region-native write beyond `need` trips it loudly
        // at the source. Off by default (and in release) so normal runs
        // pay nothing.
        let frame_canary: usize = if jit_canary_enabled() { 64 } else { 0 };
        const FRAME_SENTINEL: i64 = 0x6262_6262_6262_6262u64 as i64;
        let need = rf.frame_size() + rf.arena_slots();
        let frame_cell = REGION_FRAME.with(|f| {
            let mut frame = f.take();
            // Zero only the region frame proper. The call-arena headroom is
            // REUSED untouched (16MiB for calling regions — zeroing it per
            // entry would memset megabytes every loop crossing): callee
            // chains write-before-read by the kind gates and the call
            // stencil plants each limit slot, so stale slots are
            // unobservable — the same contract as the function tier's
            // thread-local arena.
            if frame.len() < need + frame_canary {
                // Use the allocator's zero-filled path so the full safety
                // range is logically initialized without eagerly touching
                // every page. Native calls fault in only the recursion frames
                // they actually use.
                frame = vec![0; need + frame_canary];
            }
            frame[..rf.frame_size()].fill(0);
            for c in &mut frame[need..need + frame_canary] {
                *c = FRAME_SENTINEL;
            }
            frame
        });
        let mut frame = frame_cell;
        {
            use super::native_tier::SlotKind;
            use crate::interpreter::RuntimeValue;
            for &(r, kind) in rf.guard_set() {
                frame[r as usize] =
                    match (kind, &*self.registers[self.base + r as usize].as_runtime()) {
                        (SlotKind::Int, RuntimeValue::Int(n)) => *n,
                        (SlotKind::Float, RuntimeValue::Float(f)) => f.to_bits() as i64,
                        (SlotKind::Bool, RuntimeValue::Bool(b)) => *b as i64,
                        // A Word rides UNBOXED as its masked value: W32 zero-extended (`0 ≤ x < 2³²`),
                        // W64 the full bit pattern. Matches how `RegBox::Word{32,64}` re-boxes at exit.
                        (
                            SlotKind::Word32,
                            RuntimeValue::Word(logicaffeine_base::WordVal::W32(w)),
                        ) => w.0 as i64,
                        (
                            SlotKind::Word64,
                            RuntimeValue::Word(logicaffeine_base::WordVal::W64(w)),
                        ) => w.0 as i64,
                        _ => unreachable!("guard verified the discriminant above"),
                    };
            }
        }
        // Pin arrays: borrow each DISTINCT Rc once (held across the whole
        // native run — zero refcount/borrow traffic inside the loop), check
        // the speculated repr, and plant buffer pointer + length in the
        // dedicated frame slots. Aliased registers resolve to the same
        // buffer. Handles drop before write-back (in-place arrays need none;
        // the deopt replay is sound by prefix-idempotence).
        // The register-file base pointer, captured ONCE before any pin takes an
        // (immutable) borrow of `self.registers` through a list/map `Rc`. A
        // `TextMut` pin plants a `*mut Value` to a register CELL derived from
        // this pointer — the cell is stable across the native run (the register
        // file never reallocates while a region runs), so the append helper can
        // grow the accumulator through it. Reading/writing a cell through this
        // raw pointer does not conflict with the handles' shared borrows.
        let reg_base_ptr: *mut Value = self.registers.as_mut_ptr();
        let (outcome, text_mut_snapshots, array_rebindings, list_args) = {
            use crate::interpreter::{ListRepr, RuntimeValue};
            let pins = rf.array_set();
            // Entry identities for the small subset of pinned registers that
            // an in-region list `RevSwap` may rebind. The native loop swaps
            // only raw pin triples; after the live `RefMut`s drop we map those
            // final handles back to the original `Rc` values in O(pin count).
            // Calling regions also need these identities when a callee exits.
            // Ordinary array regions without calls or rebinds avoid the clones.
            let mut entry_pin_values: Vec<Option<Value>> = vec![None; pins.len()];
            let mut handles: Vec<(usize, std::cell::RefMut<'_, ListRepr>)> =
                Vec::with_capacity(pins.len());
            // Parallel to `handles`: a buffer is "mutated" if ANY pin aliasing it
            // writes in place under a deopt-capable, non-precise region — it needs
            // a full-content snapshot/restore across a classic replay deopt.
            let mut handle_mutated: Vec<bool> = Vec::with_capacity(pins.len());
            let mut map_handles: Vec<(
                usize,
                std::cell::RefMut<'_, crate::interpreter::MapStorage>,
            )> = Vec::new();
            // A pinned MUTABLE Text accumulator grows THROUGH the VM register
            // cell (the planted `*mut Value`). A classic replay-from-head Deopt
            // would re-run the appends the native prefix already landed in the
            // cell — a double-append — so snapshot each distinct accumulator's
            // entry `Value` and restore it before a classic `Deopt` replay
            // (precise regions resume at the faulting op and never replay, so
            // they keep the live grown value). `(register slot, entry Value)`.
            let mut text_mut_snapshots: Vec<(usize, Value)> = Vec::new();
            for (pin_index, pin) in pins.iter().enumerate() {
                if pin.elem == super::native_tier::PinElem::Map {
                    let v = self.registers.get(self.base + pin.reg as usize)?;
                    let Some(RuntimeValue::Map(rc)) = v.as_runtime_ref() else {
                        return None;
                    };
                    let key = std::rc::Rc::as_ptr(rc) as usize;
                    if !map_handles.iter().any(|(k, _)| *k == key) {
                        let Ok(b) = rc.try_borrow_mut() else {
                            return None;
                        };
                        map_handles.push((key, b));
                    }
                    let idx = map_handles.iter().position(|(k, _)| *k == key).unwrap();
                    let storage = &mut *map_handles[idx].1;
                    frame[pin.vec_slot as usize] =
                        storage as *mut crate::interpreter::MapStorage as i64;
                    frame[pin.ptr_slot as usize] = 0;
                    frame[pin.len_slot as usize] = 0;
                    continue;
                }
                if pin.elem == super::native_tier::PinElem::TextBytes {
                    // A pinned ASCII Text rides its BYTE buffer: char index ==
                    // byte index, char count == byte length. RE-CHECK ASCII at
                    // every entry (the speculation seed is not a standing
                    // guarantee — a region could be re-entered with a non-ASCII
                    // Text in the same register) — decline (deopt to bytecode)
                    // otherwise so the per-char decode path runs and the output
                    // never diverges from the tree-walker. `Rc<String>` is
                    // read-only (no RefCell): a TextBytes pin is never written,
                    // so the snapshot/rollback machinery does not apply.
                    let v = self.registers.get(self.base + pin.reg as usize)?;
                    let Some(RuntimeValue::Text(rc)) = v.as_runtime_ref() else {
                        return None;
                    };
                    if !crate::semantics::collections::text_is_ascii(rc) {
                        return None;
                    }
                    frame[pin.vec_slot as usize] = 0;
                    frame[pin.ptr_slot as usize] = rc.as_bytes().as_ptr() as i64;
                    frame[pin.len_slot as usize] = rc.len() as i64;
                    continue;
                }
                if pin.elem == super::native_tier::PinElem::TextMut {
                    // A pinned MUTABLE Text accumulator: plant a `*mut Value` to
                    // the VM REGISTER CELL (stable for the run; the `Rc<String>`
                    // inside it reallocs/COWs on append). Decline (deopt to
                    // bytecode) if the observed value is not a Text. Snapshot the
                    // entry `Value` for the classic-Deopt rollback (one per
                    // distinct accumulator register). The cell is reached through
                    // `reg_base_ptr` (no `self.registers` borrow that would
                    // conflict with the live list/map handles).
                    if pin.reg as usize >= self.registers.len().saturating_sub(self.base) {
                        return None;
                    }
                    let slot = self.base + pin.reg as usize;
                    let cell: *mut Value = unsafe { reg_base_ptr.add(slot) };
                    if !matches!(
                        unsafe { (*cell).as_runtime_ref() },
                        Some(RuntimeValue::Text(_))
                    ) {
                        return None;
                    }
                    if pin.mutated && !text_mut_snapshots.iter().any(|(s, _)| *s == slot) {
                        text_mut_snapshots.push((slot, unsafe { (*cell).clone() }));
                    }
                    frame[pin.vec_slot as usize] = cell as i64;
                    frame[pin.ptr_slot as usize] = 0;
                    frame[pin.len_slot as usize] = 0;
                    continue;
                }
                let v = self.registers.get(self.base + pin.reg as usize)?;
                let Some(RuntimeValue::List(rc)) = v.as_runtime_ref() else {
                    return None;
                };
                if pin.rebound || rf.arena_slots() != 0 {
                    entry_pin_values[pin_index] = Some(v.clone());
                }
                let key = std::rc::Rc::as_ptr(rc) as usize;
                if !handles.iter().any(|(k, _)| *k == key) {
                    let Ok(b) = rc.try_borrow_mut() else {
                        return None;
                    };
                    handles.push((key, b));
                    handle_mutated.push(false);
                }
                let idx = handles.iter().position(|(k, _)| *k == key).unwrap();
                if pin.mutated {
                    handle_mutated[idx] = true;
                }
                let payload = &mut *handles[idx].1;
                use super::native_tier::PinElem;
                let (vec_handle, ptr, len) = match (payload, pin.elem) {
                    (ListRepr::Ints(v), PinElem::Int) => (
                        v as *mut Vec<i64> as i64,
                        v.as_mut_ptr() as *mut i64,
                        v.len(),
                    ),
                    // A `Words32` buffer pins with the IntList machinery (8-byte masked elements).
                    (ListRepr::Words32(v), PinElem::Word32) => (
                        v as *mut Vec<i64> as i64,
                        v.as_mut_ptr() as *mut i64,
                        v.len(),
                    ),
                    (ListRepr::IntsI32(v), PinElem::IntI32) => (
                        v as *mut Vec<i32> as i64,
                        v.as_mut_ptr() as *mut i64,
                        v.len(),
                    ),
                    // Maps never reach this arm (their pin path is below) —
                    // a list register observed as Map is a guard failure.
                    (_, PinElem::Map) => return None,
                    (ListRepr::Floats(v), PinElem::Float) => (
                        v as *mut Vec<f64> as i64,
                        v.as_mut_ptr() as *mut i64,
                        v.len(),
                    ),
                    (ListRepr::Bools(v), PinElem::Bool) => (
                        v as *mut Vec<bool> as i64,
                        v.as_mut_ptr() as *mut i64,
                        v.len(),
                    ),
                    // Repr changed since compile (promotion) — guard failure.
                    _ => return None,
                };
                frame[pin.vec_slot as usize] = vec_handle;
                frame[pin.ptr_slot as usize] = ptr as i64;
                frame[pin.len_slot as usize] = len as i64;
                // LEVER B calling-convention invariant (matches the function
                // tier): a list register's frame slot MIRRORS its vec handle, so
                // staging a pinned array into a call's argument window (a `Move`
                // from the register slot) passes the live `*mut Vec`, not the
                // zeroed register cell. Harmless for non-call regions (the slot
                // is never read for a pinned array, and the frame is discarded on
                // deopt — the VM register keeps the real Rc).
                frame[pin.reg as usize] = vec_handle;
            }
            let entry_pin_handles: Vec<i64> = pins
                .iter()
                .map(|pin| frame[pin.vec_slot as usize])
                .collect();
            // HOISTED bounds checks (V8 loop bound-check elimination): with
            // the pinned lengths in hand, verify ONCE that every covered loop
            // access stays in bounds for the whole run. Any failure declines
            // the region — the VM replays the loop on bytecode, where the
            // accesses are checked and produce the exact error.
            for hg in rf.hoist_guards() {
                let len = frame[hg.len_slot as usize];
                let bound = match &*self
                    .registers
                    .get(self.base + hg.bound_reg as usize)?
                    .as_runtime()
                {
                    RuntimeValue::Int(n) => *n,
                    _ => return None,
                };
                let iv = match &*self
                    .registers
                    .get(self.base + hg.iv_reg as usize)?
                    .as_runtime()
                {
                    RuntimeValue::Int(n) => *n,
                    _ => return None,
                };
                if len < bound.saturating_add(hg.add_max as i64)
                    || iv.saturating_add(hg.add_min as i64) < 1
                {
                    return None;
                }
            }
            // Entry lengths of every pinned list buffer — the rollback target
            // if a mid-region side-exit forces discard-and-replay. `ListPush`
            // APPENDS, so it is NOT replay-idempotent; on deopt each pushed
            // buffer is truncated back to its entry length before the VM
            // replays the loop on bytecode, which then re-pushes cleanly
            // instead of duplicating. Read-only and in-place (SetIndex) buffers
            // keep their entry length, so the truncate is a no-op for them.
            let entry_lens: Vec<usize> = handles.iter().map(|(_, h)| h.len()).collect();
            // A buffer written IN PLACE (SetIndex) replays unsoundly under the
            // classic discard-replay deopt — the write already landed in the
            // SHARED buffer, so the bytecode replay-from-head double-applies it
            // (a read-modify-write or swap is not idempotent). Snapshot its full
            // contents on entry; a classic `Deopt` restores them (subsuming the
            // length, so a buffer that BOTH pushes and writes in place is covered
            // too). Push-only / read-only buffers keep the cheap length truncate.
            let entry_snapshots: Vec<Option<ListRepr>> = handles
                .iter()
                .zip(handle_mutated.iter())
                .map(|((_, h), &mt)| if mt { Some((**h).clone()) } else { None })
                .collect();
            let out = rf.run_with_callees(&mut frame[..need], depth_now, &|fi| match self
                .native
                .get(fi as usize)
            {
                Some(super::native_tier::NativeSlot::Ready(nf)) => Some(nf.as_ref()),
                _ => None,
            });
            if matches!(out, super::native_tier::RegionOutcome::Deopt) {
                for (((_, h), &n), snap) in handles
                    .iter_mut()
                    .zip(entry_lens.iter())
                    .zip(entry_snapshots.into_iter())
                {
                    match snap {
                        Some(s) => **h = s,
                        None => h.truncate(n),
                    }
                }
            }
            // The list/map handles drop here, releasing their `self.registers`
            // borrows; the text-accumulator rollback (which needs `&mut
            // self.registers`) runs after the block, gated on a classic Deopt.
            let mut array_rebindings = Vec::new();
            if !matches!(out, super::native_tier::RegionOutcome::Deopt) {
                for pin in pins.iter().filter(|pin| pin.rebound) {
                    let final_handle = frame[pin.vec_slot as usize];
                    let Some(source) = entry_pin_handles
                        .iter()
                        .position(|entry| *entry == final_handle)
                    else {
                        return None;
                    };
                    let Some(value) = entry_pin_values[source].clone() else {
                        return None;
                    };
                    array_rebindings.push((self.base + pin.reg as usize, value));
                }
            }
            let list_args = if matches!(out, super::native_tier::RegionOutcome::DeoptCall { .. }) {
                pins.iter()
                    .map(|pin| {
                        let handle = frame[pin.vec_slot as usize];
                        entry_pin_handles
                            .iter()
                            .position(|h| *h == handle)
                            .and_then(|i| entry_pin_values[i].clone())
                            .unwrap_or_else(Value::nothing)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            (out, text_mut_snapshots, array_rebindings, list_args)
        };
        // Roll each pinned mutable-Text accumulator back to its entry `Value` on
        // a classic Deopt so the bytecode replay-from-head re-appends from the
        // pre-region prefix instead of doubling the native prefix's appends (the
        // appends landed directly in the VM register cell). Precise side exits
        // and successful completions keep the live grown value.
        if matches!(outcome, super::native_tier::RegionOutcome::Deopt) {
            for (slot, snap) in text_mut_snapshots {
                self.registers[slot] = snap;
            }
        }
        // A successful list `RevSwap` changes which original `Rc` each list
        // register names. Apply that identity-only rebind after every list
        // borrow has dropped. Precise deopt keeps the completed native prefix,
        // so it needs the same rebind; classic replay deliberately does not.
        for (slot, value) in array_rebindings {
            self.registers[slot] = value;
        }
        for (k, c) in frame[need..need + frame_canary].iter().enumerate() {
            assert_eq!(
                *c, FRAME_SENTINEL,
                "REGION_FRAME OVERFLOW: canary slot {k} (frame + {}) clobbered by region native code",
                need + k
            );
        }
        let (outcome, callee_exit) = match outcome {
            super::native_tier::RegionOutcome::DeoptCall {
                call_pc,
                resume_pc,
                frames,
            } => (
                super::native_tier::RegionOutcome::DeoptAt { resume_pc: call_pc },
                Some((call_pc, resume_pc, frames)),
            ),
            other => (other, None),
        };
        match outcome {
            super::native_tier::RegionOutcome::DeoptCall { .. } => unreachable!(),
            super::native_tier::RegionOutcome::Completed => {}
            // Side exit: discard the private frame — VM registers still hold
            // the state of this back-edge crossing, so falling back to
            // bytecode re-runs the remaining iterations deterministically up
            // to the faulting op and raises the exact kernel error there.
            // (In-place array writes already landed; the replay recomputes
            // the same prefix values, so they are unobservable.)
            super::native_tier::RegionOutcome::Deopt => return None,
            // PRECISE side exit (push+SetIndex regions): the buffers were NOT
            // truncated (the truncate above is gated on `Deopt`), so completed
            // iterations' pushes and in-place writes stand. Materialize every
            // touched non-array scalar — all-int by the adapter's gate — from
            // the frame into the VM registers, then resume the bytecode AT the
            // faulting op (NOT the loop head): the faulting op re-runs exactly
            // once, raising the precise error or continuing, with no replay of
            // the completed prefix. Array handle registers keep their live Rc
            // (the region mutated through their pins).
            super::native_tier::RegionOutcome::DeoptAt { resume_pc } => {
                use super::native_tier::SlotKind;
                // Re-box each touched register by ITS kind at the faulting op
                // (from the region's kind flow): Int/Bool/Float from the frame's
                // raw bits; `None` keeps the VM register's current value (a
                // pinned array mutated in place, or a read-only/unknown slot).
                // Cloned so the `rf` borrow ends before the register writes.
                let kinds: Option<Vec<Option<SlotKind>>> =
                    rf.precise_kinds(resume_pc).map(|k| k.to_vec());
                let mut regs: Vec<u16> = rf
                    .guard_set()
                    .iter()
                    .map(|(r, _)| *r)
                    .chain(rf.free_set().iter().copied())
                    .chain(rf.write_set().iter().map(|(r, _)| *r))
                    .collect();
                regs.sort_unstable();
                regs.dedup();
                for r in regs {
                    let bits = frame[r as usize];
                    let v = match kinds
                        .as_ref()
                        .and_then(|k| k.get(r as usize).copied().flatten())
                    {
                        Some(SlotKind::Int) => Value::int(bits),
                        Some(SlotKind::Bool) => Value::bool(bits != 0),
                        Some(SlotKind::Float) => Value::float(f64::from_bits(bits as u64)),
                        Some(SlotKind::CharText) => Value::text(
                            char::from_u32(bits as u32)
                                .map(|c| c.to_string())
                                .unwrap_or_default(),
                        ),
                        Some(SlotKind::Word32) => {
                            Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                                logicaffeine_base::WordVal::W32(logicaffeine_base::Word32(
                                    bits as u32,
                                )),
                            ))
                        }
                        Some(SlotKind::Word64) => {
                            Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                                logicaffeine_base::WordVal::W64(logicaffeine_base::Word64(
                                    bits as u64,
                                )),
                            ))
                        }
                        None => continue,
                    };
                    self.set(r, v);
                }
                REGION_FRAME.with(|f| f.replace(frame));
                return Some(match callee_exit {
                    Some((call_pc, resume_pc, frames)) => RegionExit::Call {
                        call_pc,
                        resume_pc,
                        frames,
                        list_args,
                    },
                    None => RegionExit::At(resume_pc),
                });
            }
        }
        let writes: Vec<(u16, super::native_tier::SlotKind)> = rf.write_set().to_vec();
        let region_return = rf.region_return();
        let exit = *exit_pc;
        for (r, kind) in writes {
            use super::native_tier::SlotKind;
            let bits = frame[r as usize];
            let v = match kind {
                SlotKind::Int => Value::int(bits),
                SlotKind::Bool => Value::bool(bits != 0),
                SlotKind::Float => Value::float(f64::from_bits(bits as u64)),
                SlotKind::CharText => Value::text(
                    char::from_u32(bits as u32)
                        .map(|c| c.to_string())
                        .unwrap_or_default(),
                ),
                SlotKind::Word32 => Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                    logicaffeine_base::WordVal::W32(logicaffeine_base::Word32(bits as u32)),
                )),
                SlotKind::Word64 => Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                    logicaffeine_base::WordVal::W64(logicaffeine_base::Word64(bits as u64)),
                )),
            };
            self.set(r, v);
        }
        let result = match region_return {
            Some(rr) if frame[rr.flag_slot as usize] != 0 => {
                use super::native_tier::{RegionReturnKind, SlotKind};
                let bits = frame[rr.value_slot as usize];
                let v = match rr.kind {
                    RegionReturnKind::Slot(SlotKind::Int) => Value::int(bits),
                    RegionReturnKind::Slot(SlotKind::Bool) => Value::bool(bits != 0),
                    RegionReturnKind::Slot(SlotKind::Float) => {
                        Value::float(f64::from_bits(bits as u64))
                    }
                    RegionReturnKind::Slot(SlotKind::CharText) => Value::text(
                        char::from_u32(bits as u32)
                            .map(|c| c.to_string())
                            .unwrap_or_default(),
                    ),
                    RegionReturnKind::Slot(SlotKind::Word32) => {
                        Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                            logicaffeine_base::WordVal::W32(logicaffeine_base::Word32(bits as u32)),
                        ))
                    }
                    RegionReturnKind::Slot(SlotKind::Word64) => {
                        Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                            logicaffeine_base::WordVal::W64(logicaffeine_base::Word64(bits as u64)),
                        ))
                    }
                    RegionReturnKind::Register => self.reg(bits as u16).clone(),
                };
                RegionExit::Return(v)
            }
            _ => RegionExit::At(exit),
        };
        REGION_FRAME.with(|f| f.replace(frame));
        Some(result)
    }

    /// Install a native tier: hot functions in the integer subset run as
    /// JIT-compiled machine code, guarded per call (non-Int args deopt to
    /// the bytecode path).
    pub fn with_native_tier(mut self, tier: &'p dyn super::native_tier::NativeTier) -> Self {
        self.tier = Some(tier);
        self
    }

    /// Pre-install an AOT-native function for `fi` (HOTSWAP §Axis-3): the VM dispatches
    /// to it via the existing `NativeSlot::Ready` path from the first call (it is not
    /// `Untried`, so it skips the hotness threshold and the forge compile). Requires a
    /// native tier to be installed (the dispatch is gated on `self.tier`); on desktop
    /// the forge tier is always present alongside. Absent ⇒ the function stays on
    /// VM+JIT — the AOT artifact is strictly optional, no gap at the seam.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn install_aot_native(&mut self, fi: usize, nf: Box<dyn super::native_tier::NativeFn>) {
        if fi < self.native.len() {
            let name = self.fn_name(fi);
            super::tier_trace::trace_transition(fi, &name, super::tier_trace::ExecTier::NativeAot);
            self.native[fi] = super::native_tier::NativeSlot::Ready(nf);
        }
    }

    /// Copy-on-write for value semantics (VM side; mirrors the tree-walker's
    /// `ensure_collection_owned`). Before mutating the collection in register `reg`
    /// at `pc`, deep-clone it if another holder shares the allocation (`Rc` strong
    /// count > 1). Only a proven consumed-and-returned borrow is exempt. In-out
    /// parameters copy their final values back and still isolate local copies.
    ///
    /// A holder that is a DEAD scratch register of this frame is not a holder: the
    /// statement that read the collection into it has finished with it, and nothing
    /// will read it again ([`Vm::release_dead_aliases`]). Counting it would deep-clone
    /// on every read-then-write of a collection — `Let v be item i of xs.` followed
    /// by `Set item i of xs to v + 1.` would copy the whole list each time round.
    ///
    /// Nor is the module-level binding the value is about to be written back into
    /// ([`Vm::put_back_global`]): mutating a global's collection reads it into a
    /// register, mutates the register, and stores it back, so the mutation in place IS
    /// the global's own. The global keeps its reference throughout — every mutating op
    /// either completes or fails before it writes — so a trap leaves it intact.
    fn ensure_reg_owned(&mut self, reg: Reg, pc: usize, cur_func: Option<u16>) {
        if !crate::semantics::collections::value_semantics_enabled() {
            return;
        }
        if self.is_cow_exempt(reg, cur_func) {
            return;
        }
        let di = self.base + reg as usize;
        let Some(target) = shared_collection(&self.registers[di]) else {
            return;
        };
        self.release_dead_aliases(reg, pc, cur_func);
        let returning = self.put_back_global(reg, pc).is_some_and(|g| {
            self.globals
                .get(g as usize)
                .and_then(|v| v.as_ref())
                .and_then(collection_ptr)
                == Some(target)
        });
        if holder_count(&self.registers[di]) <= 1 + usize::from(returning) {
            return;
        }
        let owned = self.registers[di].as_runtime().deep_clone();
        self.registers[di] = Value::from_runtime(owned);
    }

    /// Whether ownership analysis proved that this register can be mutated
    /// without preserving another value held by the program.
    fn is_cow_exempt(&self, reg: Reg, cur_func: Option<u16>) -> bool {
        cur_func
            .and_then(|fi| self.program.functions.get(fi as usize))
            .is_some_and(|f| f.cow_exempt_regs.contains(&reg))
    }

    /// Copy-on-write for a record's list field: when another value shares the list the `field` of
    /// the record in `reg` holds — a copy of the record, the caller's record behind a plain
    /// parameter — the record in `reg` takes a list of its own (one level, as the tree-walker's
    /// `own_collection` does) before a write reaches it.
    fn own_struct_field_list(&mut self, reg: Reg, field: &str, pc: usize, cur_func: Option<u16>) {
        use crate::interpreter::RuntimeValue;
        use std::rc::Rc;
        let slot = self.base + reg as usize;
        let shared = match self.registers[slot].as_runtime_ref() {
            Some(RuntimeValue::Struct(s)) => matches!(s.fields.get(field),
                Some(RuntimeValue::List(rc)) if Rc::strong_count(rc) > 1),
            _ => false,
        };
        if shared {
            // A dead projection or record temporary can still hold this field.
            // Release those temporaries before deciding whether a real second
            // owner requires a copy. Named and live registers remain intact.
            self.release_dead_scratch(Some(reg), pc, cur_func, None);
        }
        if let RuntimeValue::Struct(s) = self.registers[slot].as_runtime_mut() {
            if let Some(RuntimeValue::List(rc)) = s.fields.get_mut(field) {
                if Rc::strong_count(rc) > 1 || Rc::weak_count(rc) > 0 {
                    self.record_field_copy_work.0 = self.record_field_copy_work.0.saturating_add(1);
                    self.record_field_copy_work.1 = self
                        .record_field_copy_work
                        .1
                        .saturating_add(rc.borrow().len());
                    let own = rc.borrow().clone();
                    *rc = Rc::new(std::cell::RefCell::new(own));
                }
            }
        }
    }

    /// [`Vm::own_struct_field_list`] for every list field of the record in `reg`; nothing for any
    /// other value.
    fn own_struct_fields(&mut self, reg: Reg, pc: usize, cur_func: Option<u16>) {
        use crate::interpreter::RuntimeValue;
        let fields: Vec<String> = match &*self.reg(reg).as_runtime() {
            RuntimeValue::Struct(s) => s
                .fields
                .iter()
                .filter(|(_, v)| matches!(v, RuntimeValue::List(_)))
                .map(|(k, _)| k.clone())
                .collect(),
            _ => return,
        };
        for field in fields {
            self.own_struct_field_list(reg, &field, pc, cur_func);
        }
    }

    /// The global the value in `reg` is stored back into right after the mutation at `pc` —
    /// the first `GlobalSet` of `reg` that follows in straight-line code, with nothing in
    /// between that could read a global or run other code. `None` when none follows.
    fn put_back_global(&self, reg: Reg, pc: usize) -> Option<u16> {
        for op in self.program.code.get(pc + 1..)? {
            match *op {
                Op::GlobalSet { idx, src } if src == reg => return Some(idx),
                Op::GlobalGet { .. }
                | Op::Call { .. }
                | Op::CallValue { .. }
                | Op::CallBuiltin { .. }
                | Op::Jump { .. }
                | Op::JumpIfFalse { .. }
                | Op::JumpIfTrue { .. }
                | Op::Return { .. }
                | Op::ReturnNothing
                | Op::Halt => return None,
                _ => {}
            }
        }
        None
    }

    /// Release every DEAD scratch register of the current frame that references the
    /// same collection as `reg` — unnamed (no binding owns it) and not live entering
    /// `pc` ([`FrameLiveness`]), so no path reads it again before writing it.
    ///
    /// The compiler reuses scratch registers statement by statement but never clears
    /// them, so a read like `item i of xs` leaves a copy of `xs`'s reference in the
    /// scratch register the read went through until something else lands there.
    /// Releasing it here, on the path that was about to copy the whole collection
    /// anyway, costs one scan of the frame and nothing on the path that does not copy.
    fn release_dead_aliases(&mut self, reg: Reg, pc: usize, cur_func: Option<u16>) {
        let Some(target) = shared_collection(&self.registers[self.base + reg as usize]) else {
            return;
        };
        self.release_dead_scratch(Some(reg), pc, cur_func, Some(target));
    }

    /// Release provably dead unnamed temporaries. Collection mutations may
    /// restrict this to direct aliases; a record field can also be retained
    /// inside an otherwise dead record temporary, so its ownership gate clears
    /// all dead scratch values. The active target and every program binding stay.
    fn release_dead_scratch(
        &mut self,
        keep: Option<Reg>,
        pc: usize,
        cur_func: Option<u16>,
        target: Option<usize>,
    ) {
        let Some(live) = self.frame_liveness(pc, cur_func) else {
            return;
        };
        let named: &[bool] = match cur_func {
            None => &self.program.named_regs,
            Some(f) => self
                .program
                .functions
                .get(f as usize)
                .map_or(&[][..], |f| &f.named_regs[..]),
        };
        let regs = live.words * 64;
        for r in 0..regs.min(self.registers.len().saturating_sub(self.base)) {
            let r = r as Reg;
            if keep == Some(r)
                || named.get(r as usize).copied().unwrap_or(true)
                || live.is_live(pc, r)
            {
                continue;
            }
            let slot = self.base + r as usize;
            if target.is_none_or(|target| collection_ptr(&self.registers[slot]) == Some(target)) {
                self.registers[slot] = Value::nothing();
            }
        }
    }

    /// The liveness of the region `pc` lies in, when that region is the current frame's:
    /// Main's for a frame with no function, the function's own otherwise. `None` for a pc
    /// in re-optimized warm code or a layout that does not resolve — no register is known
    /// dead there.
    fn frame_liveness(
        &mut self,
        pc: usize,
        cur_func: Option<u16>,
    ) -> Option<std::rc::Rc<FrameLiveness>> {
        let program = self.program;
        let layout = self
            .code_layout
            .get_or_insert_with(|| super::reach::code_layout(program).ok())
            .as_ref()?;
        let (start, end, regs) = match cur_func {
            None => (0, layout.main_end, program.register_count),
            Some(f) => {
                let (start, end) = *layout.func_region.get(f as usize)?;
                (
                    start,
                    end,
                    program.functions.get(f as usize)?.register_count,
                )
            }
        };
        if !(start..end).contains(&pc) {
            return None;
        }
        Some(
            self.liveness
                .entry(start)
                .or_insert_with(|| std::rc::Rc::new(FrameLiveness::of(program, start, end, regs)))
                .clone(),
        )
    }

    /// Null this call's argument-window registers after it returns. Dead once the
    /// call is over, but as the callee's params they persist below `restore_len`;
    /// left set, a collection argument keeps a live `Rc` clone in the caller frame
    /// that spuriously inflates `strong_count` and forces later copy-on-write. Zero
    /// `arg_count` (native/scheduler frames) does nothing.
    ///
    /// A `mutable` parameter's slot is kept: it holds the value the callee last bound there, which
    /// the caller takes back into the binding it passed (the compiler's copy-out after the call).
    #[inline]
    fn clear_arg_window(&mut self, frame: &CallFrame) {
        let end = (frame.arg_lo + frame.arg_count as usize).min(self.registers.len());
        let kept = self
            .program
            .functions
            .get(frame.func as usize)
            .map_or(&[][..], |f| &f.mutable_param_regs[..]);
        for slot in frame.arg_lo..end {
            if !kept.contains(&((slot - frame.arg_lo) as Reg)) {
                self.registers[slot] = Value::nothing();
            }
        }
    }

    /// Same as [`Vm::clear_arg_window`] for a call that completes INLINE (a native /
    /// WASM / builtin dispatch that never pushes a `CallFrame`): the argument window
    /// starts at the current base's `args_start`.
    /// A `mutable` parameter's slot of a user function `func` is kept, as in
    /// [`Vm::clear_arg_window`].
    #[inline]
    fn clear_args(&mut self, args_start: Reg, arg_count: u16, func: Option<u16>) {
        let lo = self.base + args_start as usize;
        let end = (lo + arg_count as usize).min(self.registers.len());
        let kept = func
            .and_then(|f| self.program.functions.get(f as usize))
            .map_or(&[][..], |f| &f.mutable_param_regs[..]);
        for slot in lo..end {
            if !kept.contains(&((slot - lo) as Reg)) {
                self.registers[slot] = Value::nothing();
            }
        }
    }

    /// Resolve a function's source name for the tier trace; empty when no interner is
    /// available (the trace then prints just the index).
    /// Publish a forge-compiled native body for function `fi`: its entry in the program's `FnTable`,
    /// the `LOGOS_TIER_TRACE` line, the `LOGOS_PERF_MAP` symbol and `LOGOS_JIT_CODE` bytes, and the `Ready` slot.
    fn install_forge_native(&mut self, fi: usize, nf: Box<dyn super::native_tier::NativeFn>) {
        self.native_ctx
            .table
            .publish(fi, nf.entry_ptr(), nf.published_regc());
        let name = self.fn_name(fi);
        super::tier_trace::trace_transition(fi, &name, super::tier_trace::ExecTier::NativeForge);
        #[cfg(not(target_arch = "wasm32"))]
        super::perf_map::record_code(&nf.code_extents(), &super::perf_map::function_symbol(&name));
        self.native[fi] = super::native_tier::NativeSlot::Ready(nf);
    }

    fn fn_name(&self, fi: usize) -> String {
        match (self.program.functions.get(fi), self.policy_ctx) {
            (Some(f), Some((_, interner))) => interner.resolve(f.name).to_string(),
            _ => String::new(),
        }
    }

    /// Install the process tier AND a background compiler (HOTSWAP §6): hot functions
    /// are compiled on a worker thread instead of stalling the interpreter. The tier
    /// must be `&'static` (the process-installed forge backend) so it can cross to the
    /// worker — `&'static dyn NativeTier` is `Send` because `NativeTier: Sync`. The
    /// interpreter still runs the chains and is the sole `FnTable` writer; the worker
    /// only compiles. Falls back to [`Vm::with_native_tier`] (synchronous) for a
    /// borrowed `&'p` tier, which cannot be shared with a thread.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_bg_native_tier(
        mut self,
        tier: &'static dyn super::native_tier::NativeTier,
    ) -> Self {
        self.tier = Some(tier);
        self.bg = Some(super::bg_compile::BgCompiler::new(tier));
        self
    }

    /// Apply every background-compiled result that has come back: publish the native
    /// entry to the `FnTable` and flip the slot to `Ready` (or `Failed`). The
    /// interpreter is the sole `FnTable` writer, so this only ever runs on this
    /// thread, at the profiling points. No-op when there is no background compiler.
    #[cfg(not(target_arch = "wasm32"))]
    fn drain_bg_compiles(&mut self) {
        use super::bg_compile::CompileResult;
        use super::native_tier::NativeSlot;
        loop {
            let res = match self.bg.as_mut() {
                Some(b) => b.try_drain(),
                None => return,
            };
            let Some(res) = res else { return };
            match res {
                CompileResult::Function { fi, nf } => match nf {
                    Some(nf) => self.install_forge_native(fi, nf),
                    None => self.native[fi] = NativeSlot::Failed,
                },
            }
        }
    }

    /// Block until every outstanding background compile has come back and been
    /// published — the determinism hook the differential tests use so the native tier
    /// engages predictably regardless of thread scheduling. No-op without a background
    /// compiler.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn drain_pending_compiles(&mut self) {
        use super::bg_compile::CompileResult;
        use super::native_tier::NativeSlot;
        let results = match self.bg.as_mut() {
            Some(b) => b.drain_blocking(),
            None => return,
        };
        for res in results {
            match res {
                CompileResult::Function { fi, nf } => match nf {
                    Some(nf) => self.install_forge_native(fi, nf),
                    None => self.native[fi] = NativeSlot::Failed,
                },
            }
        }
    }

    /// Supply the program arguments read by the `args()` system native. The
    /// vector is the full argv (index 0 is the program name), matching the
    /// compiled binary's `env::args()`.
    pub fn with_program_args(mut self, args: Vec<String>) -> Self {
        self.program_args = crate::semantics::argv_with_program_name(args);
        self
    }

    /// A new instance of the same program, configured as this one was (native tier, policy
    /// context, argument vector), with nothing run and no global set — what a browser has after
    /// a page reload re-instantiates the module.
    pub fn reinstantiate(&self) -> Vm<'p> {
        let mut vm = Vm::new(self.program);
        vm.tier = self.tier;
        vm.policy_ctx = self.policy_ctx;
        vm.accel_pbkdf2 = self.accel_pbkdf2;
        vm.program_args = self.program_args.clone();
        vm
    }

    /// Tier dispatch for `Call`: Some(result) = the native fast path ran.
    /// Precise-deopt frame materialization — deliberately OUT of the
    /// dispatch loop (cold path; keeping its body inline bloats the hot
    /// match and costs i-cache on every dispatched op).
    #[cold]
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn materialize_native_frames(
        &mut self,
        frames: &[super::native_tier::NativeFrame],
        list_args: &[Value],
        args_start: super::instruction::Reg,
        dst: super::instruction::Reg,
        func: u16,
        return_pc: usize,
        call_stack: &mut Vec<CallFrame>,
    ) -> Result<(), String> {
        use super::native_tier::RegBox;
        let mut frame_base = self.base + args_start as usize;
        let mut caller_base = self.base;
        for (k, fr) in frames.iter().enumerate() {
            if k > 0 {
                caller_base = frame_base;
                let Some(Op::Call { args_start, .. }) = self.program.code.get(fr.return_pc - 1)
                else {
                    return Err("vm: native frame has no call-site arguments".to_string());
                };
                frame_base += *args_start as usize;
            }
            let restore_len = self.registers.len();
            let needed = frame_base + fr.regs.len();
            if needed > MAX_REGISTER_FILE {
                return Err("vm: register file limit exceeded".to_string());
            }
            if self.registers.len() < needed {
                self.registers.resize(needed, Value::nothing());
            }
            call_stack.push(CallFrame {
                return_pc: if k == 0 { return_pc } else { fr.return_pc },
                return_reg: if k == 0 { dst } else { fr.return_reg },
                caller_base,
                restore_len,
                iter_depth: self.iter_stack.len(),
                func,
                arg_lo: frame_base,
                arg_count: self.program.functions[func as usize].param_count,
            });
            for (r, bits) in fr.regs.iter().enumerate() {
                let v = match fr.kinds[r] {
                    RegBox::Dead | RegBox::Resolved => continue,
                    RegBox::Nothing => Value::nothing(),
                    RegBox::Int => Value::int(*bits),
                    RegBox::Bool => Value::bool(*bits != 0),
                    RegBox::Float => Value::float(f64::from_bits(*bits as u64)),
                    RegBox::Word32 => Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                        logicaffeine_base::WordVal::W32(logicaffeine_base::Word32(*bits as u32)),
                    )),
                    RegBox::Word64 => Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                        logicaffeine_base::WordVal::W64(logicaffeine_base::Word64(*bits as u64)),
                    )),
                    // The producing `ChrGuard` deopts on an invalid code, so this scalar is always valid.
                    RegBox::CharText => Value::text(
                        char::from_u32(*bits as u32)
                            .map(|c| c.to_string())
                            .unwrap_or_default(),
                    ),
                    RegBox::ListParam(j) => match list_args.get(j as usize) {
                        Some(v) => v.clone(),
                        None => return Err("vm: deopt pin index out of range".to_string()),
                    },
                };
                self.registers[frame_base + r] = v;
            }
            for (r, v) in &fr.resolved {
                self.registers[frame_base + *r as usize] = v.clone();
            }
        }
        self.base = frame_base;
        Ok(())
    }

    fn try_native(
        &mut self,
        func: u16,
        args_start: super::instruction::Reg,
        arg_count: u16,
        bytecode_depth: usize,
    ) -> NativeDisposition {
        use super::native_tier::{NativeSlot, ParamKind, NATIVE_TIER_THRESHOLD};
        let Some(tier) = self.tier else {
            return NativeDisposition::Interpret;
        };
        let fi = func as usize;
        // Publish any background-compiled chains that have come back (sole writer).
        #[cfg(not(target_arch = "wasm32"))]
        self.drain_bg_compiles();
        match self.native.get(fi) {
            None | Some(NativeSlot::Failed) => return NativeDisposition::Interpret,
            // Background compile in flight — keep running bytecode until it lands.
            Some(NativeSlot::Pending) => return NativeDisposition::Interpret,
            _ => {}
        }
        if matches!(self.native[fi], NativeSlot::Untried) {
            self.hot[fi] += 1;
            if self.hot[fi] < NATIVE_TIER_THRESHOLD {
                return NativeDisposition::Interpret;
            }
            let f = &self.program.functions[fi];
            if !f.captures.is_empty() {
                crate::optimize::mark_backend_fallback(
                    crate::optimization::OptimizationBackend::Vm,
                    "function captures values outside the forge ABI",
                );
                self.native[fi] = NativeSlot::Failed;
                return NativeDisposition::Interpret;
            }
            // A parameter whose declared type has no native representation
            // (Map, Text, nested Seq, …) can never enter native code —
            // fail once instead of bumping the hot counter forever.
            if f.param_kinds.iter().any(|k| k.is_none()) {
                crate::optimize::mark_backend_fallback(
                    crate::optimization::OptimizationBackend::Vm,
                    "function parameter has no forge representation",
                );
                self.native[fi] = NativeSlot::Failed;
                return NativeDisposition::Interpret;
            }
            // The VM/JIT boundary marshals arguments in a fixed stack buffer.
            // Refuse once at tier-up instead of silently retrying bytecode.
            if f.param_count as usize > 16 {
                crate::optimize::mark_backend_fallback(
                    crate::optimization::OptimizationBackend::Vm,
                    "function has more than 16 forge arguments",
                );
                self.native[fi] = NativeSlot::Failed;
                return NativeDisposition::Interpret;
            }
            let end = self
                .program
                .functions
                .iter()
                .map(|g| g.entry_pc)
                .filter(|&e| e > f.entry_pc)
                .min()
                .unwrap_or(self.program.code.len());
            let callees: Vec<super::native_tier::CalleeSig> = {
                let prog = &self.program;
                let code_len = prog.code.len();
                prog.functions
                    .iter()
                    .map(|g| {
                        let end = prog
                            .functions
                            .iter()
                            .map(|h| h.entry_pc)
                            .filter(|&pc| pc > g.entry_pc)
                            .min()
                            .unwrap_or(code_len);
                        let (list_params_stable, returns_list_param) = analyze_list_call_safety(
                            &prog.code[g.entry_pc..end],
                            g.param_count,
                            &g.param_kinds,
                            g.register_count,
                        );
                        super::native_tier::CalleeSig {
                            param_kinds: g.param_kinds.clone(),
                            inout_params: g
                                .mutable_param_regs
                                .iter()
                                .copied()
                                .filter(|r| !g.cow_exempt_regs.contains(r))
                                .collect(),
                            ret: g.ret_kind,
                            list_ret: native_list_return(g.return_type.as_ref()),
                            list_params_stable,
                            returns_list_param,
                        }
                    })
                    .collect()
            };
            // With a background compiler, ship the compile off-thread and keep
            // running bytecode (HOTSWAP §6); the result is drained + published on a
            // later call. Without one (a borrowed `&'p` tier, or wasm), compile
            // synchronously — the retained fallback.
            #[cfg(not(target_arch = "wasm32"))]
            if self.bg.is_some() {
                let req = super::bg_compile::CompileRequest::Function(
                    super::bg_compile::FunctionRequest {
                        fi,
                        code: self.program.code[f.entry_pc..end].to_vec(),
                        entry_pc: f.entry_pc,
                        constants: std::sync::Arc::from(self.program.constants.clone()),
                        enums: std::sync::Arc::from(self.program.enum_types.clone()),
                        param_count: f.param_count,
                        register_count: f.register_count as u16,
                        param_kinds: f.param_kinds.clone(),
                        cow_exempt_regs: f.cow_exempt_regs.clone(),
                        ret_kind: f.ret_kind,
                        callees,
                        ctx: self.native_ctx.clone(),
                    },
                );
                self.bg.as_mut().unwrap().submit(req);
                self.native[fi] = NativeSlot::Pending;
                return NativeDisposition::Interpret;
            }
            match tier.compile_function(
                &self.program.code[f.entry_pc..end],
                f.entry_pc,
                &self.program.constants,
                &self.program.enum_types,
                f.param_count,
                f.register_count as u16,
                func,
                &f.param_kinds,
                &f.cow_exempt_regs,
                f.ret_kind,
                &self.native_ctx,
                &callees,
            ) {
                Some(nf) => self.install_forge_native(fi, nf),
                None => {
                    self.native[fi] = NativeSlot::Failed;
                    return NativeDisposition::Interpret;
                }
            }
        }
        // The per-call guard: every argument must match its DECLARED kind
        // (floats travel as raw bits in the i64 slot; lists pin), else the
        // call stays interpreted.
        let base = self.base + args_start as usize;
        // Hot boundary: a stack buffer instead of a per-call Vec — functions
        // like gcd cross bytecode→native hundreds of thousands of times.
        if arg_count as usize > 16 {
            return NativeDisposition::Interpret;
        }
        let kinds = &self.program.functions[fi].param_kinds;
        let mut args = [0i64; 16];
        for k in 0..arg_count as usize {
            let Some(v) = self.registers.get(base + k) else {
                return NativeDisposition::Interpret;
            };
            args[k] = match kinds.get(k).copied().flatten() {
                Some(ParamKind::Scalar(super::native_tier::SlotKind::Int)) | None => {
                    match v.as_int() {
                        Some(n) => n,
                        None => return NativeDisposition::Interpret,
                    }
                }
                Some(ParamKind::Scalar(super::native_tier::SlotKind::Bool)) => match v.as_bool() {
                    Some(b) => b as i64,
                    None => return NativeDisposition::Interpret,
                },
                Some(ParamKind::Scalar(super::native_tier::SlotKind::Float)) => {
                    match v.as_float() {
                        Some(f) => f.to_bits() as i64,
                        None => return NativeDisposition::Interpret,
                    }
                }
                // `CharText` is a forge-only write-back carrier, never a declared param type; if one ever
                // reaches here, decline to the interpreter (there is no Text → code-point entry marshalling).
                Some(ParamKind::Scalar(super::native_tier::SlotKind::CharText)) => {
                    return NativeDisposition::Interpret;
                }
                // A Word param arrives UNBOXED as its masked value (W32 zero-extended, W64 full bits) — the
                // same carrier the region-internal Word ops use; a mismatched width declines to the VM.
                Some(ParamKind::Scalar(super::native_tier::SlotKind::Word32)) => {
                    match v.as_runtime_ref() {
                        Some(crate::interpreter::RuntimeValue::Word(
                            logicaffeine_base::WordVal::W32(w),
                        )) => w.0 as i64,
                        _ => return NativeDisposition::Interpret,
                    }
                }
                Some(ParamKind::Scalar(super::native_tier::SlotKind::Word64)) => {
                    match v.as_runtime_ref() {
                        Some(crate::interpreter::RuntimeValue::Word(
                            logicaffeine_base::WordVal::W64(w),
                        )) => w.0 as i64,
                        _ => return NativeDisposition::Interpret,
                    }
                }
                // List params ride the pin lane; the register slot is a
                // placeholder native code never reads as a scalar.
                Some(ParamKind::List(_)) => 0,
                // An enum node is lent for the call: this register keeps it alive until the call ends.
                Some(ParamKind::Inductive) => match v.as_runtime_ref() {
                    Some(crate::interpreter::RuntimeValue::Inductive(node)) => {
                        std::rc::Rc::as_ptr(node) as i64
                    }
                    _ => return NativeDisposition::Interpret,
                },
            };
        }
        // Pin list parameters: one borrow per DISTINCT Rc for the whole
        // call (recursion reuses the same pins via pass-through identity).
        // Empty lists retag in place to the declared element repr.
        let output_slots: Vec<u16> = self.program.functions[fi]
            .mutable_param_regs
            .iter()
            .copied()
            .filter(|r| *r < arg_count && !self.program.functions[fi].cow_exempt_regs.contains(r))
            .collect();
        let mut writeback = Vec::new();
        let outcome = {
            use super::native_tier::PinElem;
            use crate::interpreter::{ListRepr, RuntimeValue};
            let mut handles: Vec<(usize, std::cell::RefMut<'_, ListRepr>)> = Vec::new();
            let mut pins: Vec<i64> = Vec::new();
            let mut pin_args: Vec<Value> = Vec::new();
            for (k, pk) in kinds.iter().enumerate().take(arg_count as usize) {
                let Some(ParamKind::List(elem)) = pk else {
                    continue;
                };
                let Some(v) = self.registers.get(base + k) else {
                    return NativeDisposition::Interpret;
                };
                let Some(RuntimeValue::List(rc)) = v.as_runtime_ref() else {
                    return NativeDisposition::Interpret;
                };
                pin_args.push(v.clone());
                let key = std::rc::Rc::as_ptr(rc) as usize;
                if !handles.iter().any(|(hk, _)| *hk == key) {
                    let Ok(mut b) = rc.try_borrow_mut() else {
                        return NativeDisposition::Interpret;
                    };
                    // Declared-elem retag for the empty list (the shared
                    // empty starts as Ints).
                    let empty = match &*b {
                        ListRepr::Ints(v) => v.is_empty(),
                        _ => false,
                    };
                    if empty {
                        match elem {
                            PinElem::Float => *b = ListRepr::Floats(Vec::new()),
                            PinElem::Bool => *b = ListRepr::Bools(Vec::new()),
                            PinElem::IntI32 => *b = ListRepr::IntsI32(Vec::new()),
                            PinElem::Word32 => *b = ListRepr::Words32(Vec::new()),
                            PinElem::Int => {}
                            // Function params never pin maps or texts (declared
                            // kinds only produce Int/Float/Bool list elems).
                            PinElem::Map | PinElem::TextBytes | PinElem::TextMut => {
                                return NativeDisposition::Interpret
                            }
                        }
                    }
                    handles.push((key, b));
                }
                let idx = handles.iter().position(|(hk, _)| *hk == key).unwrap();
                let payload = &mut *handles[idx].1;
                let (vec_handle, ptr, len) = match (payload, elem) {
                    (ListRepr::Ints(v), PinElem::Int) => {
                        (v as *mut Vec<i64> as i64, v.as_mut_ptr() as i64, v.len())
                    }
                    (ListRepr::IntsI32(v), PinElem::IntI32) => {
                        (v as *mut Vec<i32> as i64, v.as_mut_ptr() as i64, v.len())
                    }
                    // A `Words32` buffer pins with the IntList (`Vec<i64>`) machinery — 8-byte elements.
                    (ListRepr::Words32(v), PinElem::Word32) => {
                        (v as *mut Vec<i64> as i64, v.as_mut_ptr() as i64, v.len())
                    }
                    (ListRepr::Floats(v), PinElem::Float) => {
                        (v as *mut Vec<f64> as i64, v.as_mut_ptr() as i64, v.len())
                    }
                    (ListRepr::Bools(v), PinElem::Bool) => {
                        (v as *mut Vec<bool> as i64, v.as_mut_ptr() as i64, v.len())
                    }
                    _ => return NativeDisposition::Interpret,
                };
                pins.push(vec_handle);
                pins.push(ptr);
                pins.push(len as i64);
                // The marshalled argument slot mirrors the vec handle —
                // native list registers always carry their handle.
                args[k] = vec_handle;
            }
            let NativeSlot::Ready(nf) = &self.native[fi] else {
                unreachable!()
            };
            let args_slice = &args[..arg_count as usize];
            // The native callee occupies one LOGOS frame on top of the
            // bytecode stack; its self-calls count from there against
            // MAX_CALL_DEPTH.
            let out = nf.call_with_writeback(
                args_slice,
                &pins,
                bytecode_depth + 1,
                &output_slots,
                &mut writeback,
            );
            drop(handles);
            (out, pin_args, pins)
        };
        let (out, pin_args, pins_for_ret) = outcome;
        if matches!(
            &out,
            super::native_tier::NativeOutcome::Return(_)
                | super::native_tier::NativeOutcome::ReturnValue(_)
        ) {
            for (slot, written) in writeback {
                use super::native_tier::{NativeWriteback, SlotKind};
                let value = match written {
                    NativeWriteback::Value(value) => value,
                    NativeWriteback::Raw(raw) => match kinds[slot as usize] {
                        Some(ParamKind::Scalar(SlotKind::Int)) => Value::int(raw),
                        Some(ParamKind::Scalar(SlotKind::Bool)) => Value::bool(raw != 0),
                        Some(ParamKind::Scalar(SlotKind::Float)) => {
                            Value::float(f64::from_bits(raw as u64))
                        }
                        Some(ParamKind::Scalar(SlotKind::Word32)) => {
                            Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                                logicaffeine_base::WordVal::W32(logicaffeine_base::Word32(
                                    raw as u32,
                                )),
                            ))
                        }
                        Some(ParamKind::Scalar(SlotKind::Word64)) => {
                            Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                                logicaffeine_base::WordVal::W64(logicaffeine_base::Word64(
                                    raw as u64,
                                )),
                            ))
                        }
                        Some(ParamKind::Scalar(SlotKind::CharText)) => Value::text(
                            char::from_u32(raw as u32)
                                .map(|c| c.to_string())
                                .unwrap_or_default(),
                        ),
                        Some(ParamKind::List(_)) => {
                            let index = pins_for_ret.chunks(3)
                                .position(|pin| pin.first() == Some(&raw))
                                .expect("native mutable output must own its list or return a boundary pin");
                            pin_args[index].clone()
                        }
                        Some(ParamKind::Inductive) => {
                            unreachable!("the native tier keeps a function with an in-out enum parameter on the VM")
                        }
                        None => unreachable!(
                            "native mutable output passed the declared-kind entry guard"
                        ),
                    },
                };
                self.registers[base + slot as usize] = value;
            }
        }

        let NativeSlot::Ready(nf) = &self.native[fi] else {
            unreachable!()
        };
        match out {
            // The backend already re-boxed (list-returning functions own
            // their allocation registry).
            super::native_tier::NativeOutcome::ReturnValue(v) => NativeDisposition::Done(v),
            super::native_tier::NativeOutcome::Return(raw) => {
                NativeDisposition::Done(match nf.ret() {
                    super::native_tier::NativeRet::Scalar(k) => match k {
                        super::native_tier::SlotKind::Bool => Value::bool(raw != 0),
                        super::native_tier::SlotKind::Float => {
                            Value::float(f64::from_bits(raw as u64))
                        }
                        super::native_tier::SlotKind::Int => Value::int(raw),
                        super::native_tier::SlotKind::CharText => Value::text(
                            char::from_u32(raw as u32)
                                .map(|c| c.to_string())
                                .unwrap_or_default(),
                        ),
                        super::native_tier::SlotKind::Word32 => {
                            Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                                logicaffeine_base::WordVal::W32(logicaffeine_base::Word32(
                                    raw as u32,
                                )),
                            ))
                        }
                        super::native_tier::SlotKind::Word64 => {
                            Value::from_runtime(crate::interpreter::RuntimeValue::Word(
                                logicaffeine_base::WordVal::W64(logicaffeine_base::Word64(
                                    raw as u64,
                                )),
                            ))
                        }
                    },
                    // By-handle return that was NOT registry-owned: it is
                    // one of the caller's list arguments — match the pin
                    // handles (every triple's first slot) and clone that
                    // argument, preserving identity.
                    super::native_tier::NativeRet::ListByHandle => {
                        let mut found: Option<Value> = None;
                        for (k, chunk) in pins_for_ret.chunks(3).enumerate() {
                            if chunk.first() == Some(&raw) {
                                found = pin_args.get(k).cloned();
                                break;
                            }
                        }
                        match found {
                            Some(v) => v,
                            None => return NativeDisposition::Interpret,
                        }
                    }
                    // SAFETY: `NativeRet::Inductive` hands back an `Rc::into_raw` pointer carrying one strong
                    // count of its own, which this `Rc` takes over.
                    super::native_tier::NativeRet::Inductive => {
                        Value::from_runtime(crate::interpreter::RuntimeValue::Inductive(unsafe {
                            std::rc::Rc::from_raw(raw as *const crate::interpreter::InductiveValue)
                        }))
                    }
                    // Return-by-parameter: the result IS the caller's list
                    // argument (same Rc — identity preserved).
                    super::native_tier::NativeRet::ListParam(j) => {
                        let mut nth = 0usize;
                        let mut found: Option<Value> = None;
                        for (k, pk) in kinds.iter().enumerate().take(arg_count as usize) {
                            if matches!(pk, Some(ParamKind::List(_))) {
                                if k == j as usize {
                                    found = pin_args.get(nth).cloned();
                                    break;
                                }
                                nth += 1;
                            }
                        }
                        match found {
                            Some(v) => v,
                            None => return NativeDisposition::Interpret,
                        }
                    }
                })
            }
            // Plain side exit: every effect was confined to the private
            // frame — replaying the whole call on bytecode is sound and
            // raises the exact kernel error at the exact point.
            super::native_tier::NativeOutcome::Deopt => NativeDisposition::Interpret,
            // Precise side exit: effects landed; materialize the chain.
            super::native_tier::NativeOutcome::DeoptAt { resume_pc, frames } => {
                NativeDisposition::Materialize {
                    resume_pc,
                    frames,
                    list_args: pin_args,
                }
            }
        }
    }

    /// Provide the policy registry (and the interner its symbols live in) for
    /// `Check` statements.
    pub fn with_policy_ctx(
        mut self,
        registry: &'p crate::analysis::PolicyRegistry,
        interner: &'p crate::intern::Interner,
    ) -> Self {
        self.policy_ctx = Some((registry, interner));
        // Resolve the pbkdf2Sha256 acceleration handle now that the interner is available — only
        // when that function IS crypto.lg's definition (`loader::is_library_function`): the native
        // reference is byte-identical to the library body (crypto_accel_lock.rs), and to nothing
        // else. A program's own `pbkdf2Sha256` runs as written.
        self.accel_pbkdf2 = crate::loader::is_library_function("pbkdf2Sha256")
            .then(|| {
                self.program
                    .functions
                    .iter()
                    .position(|f| interner.resolve(f.name) == "pbkdf2Sha256")
                    .map(|i| i as FuncIdx)
            })
            .flatten();
        self
    }

    /// The program this machine runs.
    pub fn program(&self) -> &'p CompiledProgram {
        self.program
    }

    /// The output lines, one per `Show`.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The output as one string (one trailing newline per `Show`).
    pub fn output(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }

    /// Consume the VM, returning its output lines.
    pub fn into_lines(self) -> Vec<String> {
        self.lines
    }

    /// Take this slice's output lines (the scheduler driver merges each task's
    /// output into a shared sink as it is produced, preserving per-task order).
    pub(crate) fn drain_lines(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }

    /// Run the whole program to completion (the non-concurrent entry). A
    /// concurrent program never reaches here — it is driven through
    /// `Vm::run_until_block` by the scheduler.
    pub fn run(&mut self) -> Result<(), String> {
        // Hermetic program start: no ambient exchange rates carried in from a prior run on this
        // thread (mirrors a fresh AOT process), and a stream of its own for the document the run
        // draws into. The resumable `run_until_block` path is left alone so rates installed
        // mid-program survive across concurrency suspensions.
        logicaffeine_base::money::clear_ambient_rates();
        crate::semantics::dom::retire_stream();
        match self.run_until_block()? {
            VmStep::Done(_) => Ok(()),
            VmStep::Blocked => Err("vm: concurrency op requires the scheduler driver".to_string()),
            VmStep::Paused => unreachable!("run_until_block (STEPPED = false) never pauses"),
        }
    }

    /// The `Stmt::Site` id in force at the pc a failed run's `Err` propagated from (Wave 3:
    /// "runtime failures carry a location") — `None` outside any component's action/handler
    /// (`## Main`/`## Test` own top-level statements, which have no marker), or when the failing pc
    /// lies in a hot-swapped warm body `site_at` was never built to cover. Reads `sched_pc`, which
    /// `run_until_block_impl`'s dispatch loop saves before every op — the last write it sees before
    /// an `Err` from that op propagates via `?` straight out of the loop, so this is only meaningful
    /// to call once the caller already has that `Err` in hand; a successful run's `sched_pc` is
    /// simply wherever the loop happened to stop.
    pub fn failing_site(&self) -> Option<u32> {
        if let Some(site) = self.ended_at_site {
            return site;
        }
        if self.sched_pc >= self.program.code.len() {
            return None;
        }
        let idx = self
            .program
            .site_at
            .partition_point(|&(pc, _)| pc as usize <= self.sched_pc);
        if idx == 0 {
            return None;
        }
        let (_, id) = self.program.site_at[idx - 1];
        (id != crate::vm::instruction::NO_SITE).then_some(id)
    }

    /// Call function `fi` from the host, after the program has run: what a browser does when it
    /// calls one of a module's exports. The call enters through the same path `Op::Call` takes
    /// (native tier, warm body, baseline) and runs against the globals the program left behind.
    ///
    /// An `Err` is the callee's failure, and it leaves the machine idle and callable with every
    /// global exactly as the failure found it — a wasm trap unwinds to the host the same way.
    pub fn invoke_function(
        &mut self,
        fi: FuncIdx,
        args: Vec<crate::interpreter::RuntimeValue>,
    ) -> Result<crate::interpreter::RuntimeValue, String> {
        let f = self
            .program
            .functions
            .get(fi as usize)
            .ok_or_else(|| format!("vm: no function at index {fi}"))?;
        if args.len() != f.param_count as usize {
            let name = match self.fn_name(fi as usize) {
                n if n.is_empty() => format!("function #{fi}"),
                n => format!("`{n}`"),
            };
            return Err(format!(
                "{name} expects {} arguments, got {}",
                f.param_count,
                args.len()
            ));
        }
        self.host_call(args, |vm, args_start, arg_count, return_pc, stack| {
            vm.enter_call::<false>(0, fi, args_start, arg_count, return_pc, stack)
        })
    }

    /// Call a closure value from the host: what a browser does when it fires a handler the module
    /// handed it earlier (a fetch completion, a listener, a route handler). Same failure contract
    /// as [`Vm::invoke_function`].
    pub fn call_closure(
        &mut self,
        callee: &crate::interpreter::RuntimeValue,
        args: Vec<crate::interpreter::RuntimeValue>,
    ) -> Result<crate::interpreter::RuntimeValue, String> {
        let closure = match callee {
            crate::interpreter::RuntimeValue::Function(c) => (**c).clone(),
            other => return Err(format!("Cannot call value of type {}", other.type_name())),
        };
        self.host_call(args, |vm, args_start, arg_count, return_pc, stack| {
            vm.enter_closure::<false>(0, &closure, args_start, arg_count, return_pc, stack)
        })
    }

    /// Bring a machine whose run stopped on a failure back to the idle state a host call starts
    /// from: Main's own frame, no open iterator, nothing suspended. Globals and output keep
    /// whatever the failure left, as a trapped wasm instance keeps its memory.
    pub(super) fn quiesce(&mut self) {
        self.base = 0;
        self.registers.truncate(self.program.register_count);
        self.iter_stack.clear();
        self.sched_active = false;
        self.sched_call_stack.clear();
        self.pending = None;
        self.resume_slot = None;
        self.resume_wraps_option = false;
        self.select_pending.clear();
    }

    /// Run one host-initiated call to completion on top of the idle machine.
    ///
    /// The host frame sits past the register file's live end: one register the callee returns
    /// into, then the argument window, so the callee's frame is laid out exactly as if a caller
    /// had executed `Op::Call`. Its return pc is `program.code.len()`, where the dispatch loop
    /// stops once the call stack is empty. Whatever the outcome, the register file, the base and
    /// the iterator stack go back to what they were, so neither a clean return nor a trap leaves
    /// anything behind for the next call.
    fn host_call(
        &mut self,
        args: Vec<crate::interpreter::RuntimeValue>,
        enter: impl FnOnce(&mut Self, Reg, u16, usize, &mut Vec<CallFrame>) -> Result<usize, String>,
    ) -> Result<crate::interpreter::RuntimeValue, String> {
        if self.sched_active || self.pending.is_some() {
            return Err(
                "vm: a host call needs an idle machine, and this one is suspended mid-run"
                    .to_string(),
            );
        }
        let arg_count =
            u16::try_from(args.len()).map_err(|_| "vm: too many arguments".to_string())?;
        let saved_base = self.base;
        let saved_iters = self.iter_stack.len();
        let host_base = self.registers.len();
        if host_base + 1 + args.len() > MAX_REGISTER_FILE {
            return Err("vm: register file limit exceeded".to_string());
        }
        self.registers.push(Value::nothing());
        self.registers
            .extend(args.into_iter().map(Value::from_runtime));
        self.base = host_base;
        let finish = self.program.code.len();
        let mut stack: Vec<CallFrame> = Vec::new();
        let outcome = match enter(self, 1, arg_count, finish, &mut stack) {
            Ok(_) if stack.is_empty() => Ok(()),
            Ok(pc) => {
                self.sched_pc = pc;
                self.sched_call_stack = stack;
                self.sched_active = true;
                match self.run_until_block_impl::<false, false>(u64::MAX) {
                    Ok(VmStep::Done(_)) => Ok(()),
                    Ok(VmStep::Blocked) => {
                        Err("vm: concurrency op requires the scheduler driver".to_string())
                    }
                    Ok(VmStep::Paused) => {
                        unreachable!("run_until_block (STEPPED = false) never pauses")
                    }
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        };
        let result = outcome.map(|()| {
            std::mem::replace(&mut self.registers[host_base], Value::nothing()).into_runtime()
        });
        self.base = saved_base;
        self.registers.truncate(host_base);
        self.iter_stack.truncate(saved_iters);
        self.sched_active = false;
        self.sched_call_stack.clear();
        self.pending = None;
        self.resume_slot = None;
        self.resume_wraps_option = false;
        self.select_pending.clear();
        result
    }

    /// Run forward to completion WITH reversibility recording, capturing the history tape (retrieve
    /// it with [`Vm::take_journal`], then reconstruct the pre-run state with [`Vm::run_reverse`]).
    ///
    /// A cold path (verification / adiabatic reverse execution): the production [`Vm::run`]
    /// monomorph (`RECORDING = false`) dead-strips every recording block, so the 2.5×-C hot loop is
    /// byte-for-byte unchanged — this method merely instantiates a *separate* `RECORDING = true`
    /// copy of the loop that prod never calls. Declines with a clear error (never a silent
    /// mis-reverse) if the program uses an op outside the currently-reversible subset
    /// ([`crate::vm::reverse::is_reversible_supported`]).
    pub fn run_recording(&mut self) -> Result<(), String> {
        self.run_recording_mode(super::reverse::ReverseMode::Observable)
    }

    /// Like [`Vm::run_recording`] but at an explicit [`ReverseMode`](super::reverse::ReverseMode) —
    /// the pebbling fidelity ⇄ space ⇄ time point. `FullState` journals every destroyed value;
    /// `Observable`/`Adiabatic` compute a per-pc [`plan`](super::pebble::plan) and skip journaling
    /// scratch the reverse never reads, shrinking the tape toward the Landauer floor.
    pub fn run_recording_mode(&mut self, mode: super::reverse::ReverseMode) -> Result<(), String> {
        if let Some(op) = self
            .program
            .code
            .iter()
            .find(|op| !super::reverse::is_reversible_supported(op))
        {
            return Err(match super::reverse::external_effect_barrier(op) {
                // A truly-external effect: name the irreversibility barrier (internal state up to it
                // reverses; the effect itself cannot be un-done) — never a silent lie.
                Some(barrier) => format!(
                    "vm: `{}` is an irreversible {barrier} effect — not yet reversible past the honest Landauer wall (an external effect cannot be un-done)",
                    op.name()
                ),
                None => format!("vm: op `{}` is not yet reversible", op.name()),
            });
        }
        logicaffeine_base::money::clear_ambient_rates();
        self.install_reverse_recording(mode);
        match self.run_until_block_impl::<false, true>(u64::MAX)? {
            VmStep::Done(_) => Ok(()),
            VmStep::Blocked => Err("vm: concurrency op requires the scheduler driver".to_string()),
            VmStep::Paused => unreachable!("run_until_block (STEPPED = false) never pauses"),
        }
    }

    /// Like [`Vm::run_recording_mode`] but with an explicit pebble CHECKPOINT budget (corner 2):
    /// `budget` recomputable chain intermediates are kept journaled, so a reverse read stops its
    /// re-walk at the nearest checkpoint instead of the chain root. `budget = 0` is exactly
    /// [`Vm::run_recording_mode`] (the recompute-everything corner).
    pub fn run_recording_budgeted(
        &mut self,
        mode: super::reverse::ReverseMode,
        budget: usize,
    ) -> Result<(), String> {
        self.pebble_budget = budget;
        self.run_recording_mode(mode)
    }

    /// The number of recompute re-executions performed by the last reverse pass (corner 2) — the
    /// time-cost instrument for the pebble time/space schedule.
    pub fn recompute_ops(&self) -> usize {
        self.recompute_ops
    }

    /// Arm this VM for a reversibility RECORDING pass in `mode`: a fresh tape, the per-pc pebbling
    /// plan, the mode, and (Adiabatic only) the reverse-live recompute recipes keyed by `(frame,
    /// register)`. Shared by the single-VM [`Vm::run_recording_mode`] and each task VM of the
    /// concurrency scheduler ([`RevCoop::record_mode`]) — independent frames make per-task recompute
    /// sound.
    fn install_reverse_recording(&mut self, mode: super::reverse::ReverseMode) {
        let budget = self.pebble_budget;
        self.journal = Some(super::journal::HistoryTape::new());
        self.journal_plan = Some(super::pebble::plan_budgeted(self.program, mode, budget));
        self.reverse_mode = mode;
        self.recompute_ops = 0;
        self.recompute = if mode == super::reverse::ReverseMode::Adiabatic {
            // A checkpointed chain intermediate stays JOURNALED (out of the recompute map), so
            // `recompute_into` stops at it (its live forward value serves the read) instead of
            // re-walking to the chain root — the corner-2 space-for-time trade.
            let checkpoints = super::pebble::checkpoint_defs(self.program, budget);
            super::pebble::recomputable(self.program)
                .into_iter()
                .filter(|(_frame, _r, pc, _op)| !checkpoints.contains(pc))
                .map(|(frame, r, _pc, op)| ((frame, r), op))
                .collect()
        } else {
            std::collections::HashMap::new()
        };
    }

    /// Whether the pebbling plan elides the journal entry for the write at `pc` (its destroyed value
    /// is program-invisible scratch no reverse step reads). `false` when there is no plan.
    #[inline]
    fn journal_elided(&self, pc: usize) -> bool {
        // Both `Elide` (invisible scratch) and `Recompute` (reverse-live scratch reconstructed on
        // reverse) skip journaling forward and skip the tape-pop on reverse.
        matches!(
            self.journal_plan.as_ref().and_then(|p| p.get(pc)),
            Some(super::pebble::JournalDecision::Elide | super::pebble::JournalDecision::Recompute)
        )
    }

    /// (`Adiabatic`, Piece 2) Reconstruct a reverse-live scratch register by RE-RUNNING its pure
    /// defining op forward — Bennett uncompute. RECURSIVE: any recomputable INPUT of the def is
    /// reconstructed first, so a whole recomputable CHAIN (`t3=t2+d`, `t2=t1+c`, `t1=a+b`) is rebuilt
    /// bottom-up before the top step runs — the pebble "recompute" strategy (O(1) tape, chain-length
    /// time). A no-op for a register that was not marked recomputable.
    fn recompute_reg(&mut self, frame: Option<usize>, r: Reg) -> Result<(), String> {
        self.recompute_into(frame, r, &mut Vec::new())
    }

    fn recompute_into(
        &mut self,
        frame: Option<usize>,
        r: Reg,
        seen: &mut Vec<Reg>,
    ) -> Result<(), String> {
        let Some(op) = self.recompute.get(&(frame, r)).copied() else {
            return Ok(());
        };
        if seen.contains(&r) {
            return Ok(()); // recomputable is a DAG (a def precedes its use); guard against any cycle
        }
        seen.push(r);
        // The pure two-operand family (corner 3) — kept in lockstep with `pebble::recomputable`.
        let (lhs, rhs) = match op {
            Op::Add { lhs, rhs, .. }
            | Op::Sub { lhs, rhs, .. }
            | Op::Mul { lhs, rhs, .. }
            | Op::BitXor { lhs, rhs, .. }
            | Op::Div { lhs, rhs, .. }
            | Op::ExactDiv { lhs, rhs, .. }
            | Op::FloorDiv { lhs, rhs, .. }
            | Op::Mod { lhs, rhs, .. }
            | Op::Pow { lhs, rhs, .. }
            | Op::BitAnd { lhs, rhs, .. }
            | Op::BitOr { lhs, rhs, .. }
            | Op::Shl { lhs, rhs, .. }
            | Op::Shr { lhs, rhs, .. } => (lhs, rhs),
            _ => return Ok(()),
        };
        // Rebuild recomputable inputs FIRST (the chain), then run this def from live inputs.
        self.recompute_into(frame, lhs, seen)?;
        self.recompute_into(frame, rhs, seen)?;
        self.recompute_ops += 1; // corner 2: count each re-execution (the time cost of the schedule)
        match op {
            Op::Add { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::add)?,
            Op::Sub { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::sub)?,
            Op::Mul { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::mul)?,
            Op::BitXor { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::bitxor)?,
            Op::Div { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::div)?,
            Op::ExactDiv { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::exact_div)?,
            Op::FloorDiv { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::floor_div)?,
            Op::Mod { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::modulo)?,
            Op::Pow { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::pow)?,
            Op::BitAnd { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::bitand)?,
            Op::BitOr { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::bitor)?,
            Op::Shl { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::shl)?,
            Op::Shr { dst, lhs, rhs } => self.binop(dst, lhs, rhs, Value::shr)?,
            _ => {}
        }
        Ok(())
    }

    /// Take the history tape recorded by [`Vm::run_recording`].
    pub fn take_journal(&mut self) -> Option<super::journal::HistoryTape> {
        self.journal.take()
    }

    /// Run forward to completion WITH recording, DRIVING the task through its concurrency blocks —
    /// the single-task reversible-scheduler path. Accumulates one history tape across every slice; a
    /// `Sleep` (virtual time) resumes with nothing. Multi-task/channel blocks are declined here (the
    /// coordinator-driven reversible scheduler over several task VMs is the next layer). Declines a
    /// program with any op outside the reversible subset before running.
    pub fn run_recording_driven(&mut self) -> Result<(), String> {
        if let Some(op) = self
            .program
            .code
            .iter()
            .find(|op| !super::reverse::is_reversible_supported(op))
        {
            return Err(match super::reverse::external_effect_barrier(op) {
                // A truly-external effect: name the irreversibility barrier (internal state up to it
                // reverses; the effect itself cannot be un-done) — never a silent lie.
                Some(barrier) => format!(
                    "vm: `{}` is an irreversible {barrier} effect — not yet reversible past the honest Landauer wall (an external effect cannot be un-done)",
                    op.name()
                ),
                None => format!("vm: op `{}` is not yet reversible", op.name()),
            });
        }
        logicaffeine_base::money::clear_ambient_rates();
        self.journal = Some(super::journal::HistoryTape::new());
        loop {
            match self.run_until_block_impl::<false, true>(u64::MAX)? {
                VmStep::Done(_) => return Ok(()),
                VmStep::Blocked => match self.take_pending() {
                    // Virtual time: no observable effect, resume with nothing (the loop re-enters and
                    // continues from the saved slice).
                    Some(VmBlock::Sleep(_)) => {}
                    Some(_) => {
                        return Err(
                            "vm: reversible single-task driver does not yet service channel/task blocks"
                                .to_string(),
                        )
                    }
                    None => return Err("vm: blocked with no pending request".to_string()),
                },
                VmStep::Paused => unreachable!("recording run is not stepped"),
            }
        }
    }

    /// Run BACKWARD: reconstruct the EXACT pre-run state by replaying the executed op path in strict
    /// reverse order and applying each op's inverse. The control-flow trace (`steps`) gives the exact
    /// executed sequence — so branches and loops reverse with no special-casing (a loop is just the
    /// trace with repeated pcs). A [`crate::vm::reverse::RevClass::Bijection`] op self-inverts
    /// algebraically with NO data-tape interaction (the zero-garbage core, even inside a loop); every
    /// other op that recorded a delta pops it and restores what it destroyed. The data tape stays
    /// aligned because journaled ops are visited in exact reverse push order.
    ///
    /// Sound because reversing in exact reverse order leaves the machine in each op's POST-state when
    /// its inverse is applied, so every operand the inverse reads holds its forward value.
    pub fn run_reverse(&mut self, mut tape: super::journal::HistoryTape) -> Result<(), String> {
        while self.reverse_one_step(&mut tape)? {}
        self.adiabatic_reset();
        Ok(())
    }

    /// Adiabatic (the best-of-both-worlds mode): the elided non-reverse-read scratch was never
    /// journaled — RECOMPUTE it as its known default (`Nothing`, the pristine initial value every
    /// register begins at). This lifts the Observable-floor tape to FULL machine fidelity: every
    /// register, scratch included, returns bit-exact. Bennett uncompute in its cheapest form — the
    /// recomputed value is a constant. A no-op outside `Adiabatic`. Shared by `run_reverse` and the
    /// per-task reversal of the concurrency scheduler (`RevCoop::reverse`).
    fn adiabatic_reset(&mut self) {
        if self.reverse_mode != super::reverse::ReverseMode::Adiabatic {
            return;
        }
        let named = &self.program.named_regs;
        for i in 0..self.registers.len() {
            let observable = named.is_empty() || named.get(i).copied().unwrap_or(true);
            if !observable {
                self.registers[i] = Value::nothing();
            }
        }
    }

    /// Reverse the TOPMOST recorded step of `tape`; returns `false` when its step trace is exhausted.
    /// The composable primitive — `run_reverse` loops it to completion, and the reversible scheduler
    /// reverses each task's SLICE by calling it a slice's-worth of times (schedule-backward).
    fn reverse_one_step(&mut self, tape: &mut super::journal::HistoryTape) -> Result<bool, String> {
        use super::journal::JournalEntry;
        use super::reverse::{records, reversibility, RevClass};
        let (pc, base) = match tape.pop_step() {
            Some(pair) => pair,
            None => return Ok(false),
        };
        {
            // Restore the register window this op ran in, so bijection inverses (which recompute
            // `base + reg`) target the right registers even inside a called function's frame.
            self.base = base;
            let op = *self.program.code.get(pc).ok_or(
                "vm: reverse trace references a warm-body pc (unsupported in this subset)",
            )?;
            // Pebbling: this write's journal entry was ELIDED forward (program-invisible scratch no
            // reverse step reads), so there is nothing to pop — the register keeps its value. Elided
            // ops are always plain single-dst overwrites, so skipping their reverse is exact.
            if self.journal_elided(pc) {
                return Ok(true);
            }
            if matches!(
                op,
                Op::Call { .. } | Op::CallValue { .. } | Op::Return { .. } | Op::ReturnNothing
            ) {
                self.reverse_call_op(&op, tape)?;
            } else if matches!(
                op,
                Op::IterPrepare { .. } | Op::IterNext { .. } | Op::IterPop
            ) {
                self.reverse_iter_op(&op, tape)?;
            } else if matches!(
                op,
                Op::LoadNow { .. } | Op::LoadToday { .. } | Op::NewCrdt { .. }
            ) {
                // A nondeterministic read (clock / drawn CRDT replica id) reverses by restoring dst;
                // the drawn entropy stays recorded.
                match tape.pop() {
                    Some(JournalEntry::NondetRead {
                        dst_slot, dst_old, ..
                    }) => {
                        self.registers[dst_slot] = dst_old;
                    }
                    other => return Err(format!("vm: reverse expected NondetRead, got {other:?}")),
                }
            } else if matches!(op, Op::CrdtAppend { .. }) {
                // Restore the CRDT interior THROUGH the register's live handle (aliases observe it), or
                // fall back to the collection restore when the append was onto a plain list.
                match tape.pop() {
                    Some(JournalEntry::CrdtInterior { slot, snapshot }) => {
                        use crate::interpreter::RuntimeValue;
                        match (
                            self.registers[slot].as_runtime_ref(),
                            snapshot.as_runtime_ref(),
                        ) {
                            (Some(RuntimeValue::Crdt(cur)), Some(RuntimeValue::Crdt(snap))) => {
                                *cur.borrow_mut() = snap.borrow().clone();
                            }
                            _ => {
                                return Err(
                                    "vm: reverse CrdtInterior on a non-CRDT register".to_string()
                                )
                            }
                        }
                    }
                    Some(JournalEntry::CollectionMutate { slot, old }) => {
                        self.reverse_collection_mutate(slot, old)?;
                    }
                    other => {
                        return Err(format!(
                            "vm: reverse expected CrdtInterior/CollectionMutate, got {other:?}"
                        ))
                    }
                }
            } else if matches!(op, Op::CrdtResolve { .. }) {
                // Restore the resolved field: a nested CRDT field through its live handle, or a plain
                // field via the Box snapshot.
                match tape.pop() {
                    Some(JournalEntry::CrdtFieldInterior {
                        obj_slot,
                        field,
                        snapshot,
                    }) => {
                        use crate::interpreter::RuntimeValue;
                        let restored = match (
                            self.registers[obj_slot].as_runtime_ref(),
                            snapshot.as_runtime_ref(),
                        ) {
                            (Some(RuntimeValue::Struct(s)), Some(RuntimeValue::Crdt(snap))) => {
                                match s.fields.get(&field) {
                                    Some(RuntimeValue::Crdt(cur)) => {
                                        *cur.borrow_mut() = snap.borrow().clone();
                                        true
                                    }
                                    _ => false,
                                }
                            }
                            _ => false,
                        };
                        if !restored {
                            return Err(
                                "vm: reverse CrdtFieldInterior on a non-CRDT field".to_string()
                            );
                        }
                    }
                    Some(JournalEntry::CollectionMutate { slot, old }) => {
                        self.reverse_collection_mutate(slot, old)?;
                    }
                    other => {
                        return Err(format!(
                        "vm: reverse expected CrdtFieldInterior/CollectionMutate, got {other:?}"
                    ))
                    }
                }
            } else if matches!(op, Op::Div { dst, lhs, .. } if dst == lhs) {
                // Reverse an in-place division: reconstruct `a_old = q·b + r` from the kept quotient
                // and divisor, or restore the full old value if `record_op` fell back to an overwrite.
                match tape.pop() {
                    Some(JournalEntry::DivReconstruct {
                        dst_slot,
                        divisor,
                        remainder,
                    }) => {
                        use crate::interpreter::RuntimeValue;
                        let q = self.registers[dst_slot].as_int().unwrap_or(0);
                        let b = divisor.as_int().unwrap_or(0);
                        let r = remainder.as_int().unwrap_or(0);
                        self.registers[dst_slot] = Value::from_runtime(RuntimeValue::Int(
                            q.wrapping_mul(b).wrapping_add(r),
                        ));
                    }
                    Some(JournalEntry::RegOverwrite { slot, old }) => self.registers[slot] = old,
                    other => {
                        return Err(format!(
                            "vm: reverse expected DivReconstruct/RegOverwrite, got {other:?}"
                        ))
                    }
                }
            } else if matches!(op, Op::Shr { dst, lhs, .. } | Op::Shl { dst, lhs, .. } if dst == lhs)
                || matches!(op, Op::DivPow2 { dst, lhs, .. } if dst == lhs)
            {
                // Reverse an in-place shift: recombine the kept result with the `k` saved shifted-out
                // bits — `(a << k) | low` for a right shift, `(a >> k) | (high << (64−k))` for a left
                // shift — or restore the full old value if `record_op` fell back to an overwrite.
                match tape.pop() {
                    Some(JournalEntry::ShiftReconstruct {
                        dst_slot,
                        dropped,
                        k,
                        left,
                    }) => {
                        use crate::interpreter::RuntimeValue;
                        let cur = self.registers[dst_slot].as_int().unwrap_or(0);
                        let restored = if left {
                            (((cur as u64) >> k) | ((dropped as u64) << (64 - k))) as i64
                        } else {
                            cur.wrapping_shl(k) | dropped
                        };
                        self.registers[dst_slot] = Value::from_runtime(RuntimeValue::Int(restored));
                    }
                    Some(JournalEntry::RegOverwrite { slot, old }) => self.registers[slot] = old,
                    other => {
                        return Err(format!(
                            "vm: reverse expected ShiftReconstruct/RegOverwrite, got {other:?}"
                        ))
                    }
                }
            } else if matches!(op, Op::BitAnd { dst, lhs, .. } | Op::BitOr { dst, lhs, .. } if dst == lhs)
            {
                // Reverse an in-place mask: re-merge the saved bits under the complementary mask —
                // `a | saved` for AND (saved = the cleared bits), `(a & !mask) | saved` for OR.
                match tape.pop() {
                    Some(JournalEntry::MaskReconstruct {
                        dst_slot,
                        saved,
                        mask,
                        and,
                    }) => {
                        use crate::interpreter::RuntimeValue;
                        let cur = self.registers[dst_slot].as_int().unwrap_or(0);
                        let restored = if and {
                            cur | saved
                        } else {
                            (cur & !mask) | saved
                        };
                        self.registers[dst_slot] = Value::from_runtime(RuntimeValue::Int(restored));
                    }
                    Some(JournalEntry::RegOverwrite { slot, old }) => self.registers[slot] = old,
                    other => {
                        return Err(format!(
                            "vm: reverse expected MaskReconstruct/RegOverwrite, got {other:?}"
                        ))
                    }
                }
            } else if matches!(op, Op::Mod { dst, lhs, .. } if dst == lhs) {
                // Reverse an in-place modulo: `a_old = q·b + r` from the saved quotient, the live
                // divisor, and the kept remainder still in `dst`.
                match tape.pop() {
                    Some(JournalEntry::ModReconstruct {
                        dst_slot,
                        quotient,
                        divisor,
                    }) => {
                        use crate::interpreter::RuntimeValue;
                        let r = self.registers[dst_slot].as_int().unwrap_or(0);
                        let b = divisor.as_int().unwrap_or(0);
                        self.registers[dst_slot] = Value::from_runtime(RuntimeValue::Int(
                            quotient.wrapping_mul(b).wrapping_add(r),
                        ));
                    }
                    Some(JournalEntry::RegOverwrite { slot, old }) => self.registers[slot] = old,
                    other => {
                        return Err(format!(
                            "vm: reverse expected ModReconstruct/RegOverwrite, got {other:?}"
                        ))
                    }
                }
            } else if let RevClass::Bijection(_) = reversibility(&op) {
                // Adiabatic (Piece 2): reconstruct any recomputed reverse-live operand by re-running
                // its pure defining op (in its frame, so `self.base` resolves the registers) from the
                // current live inputs, BEFORE the inverse reads it.
                if !self.recompute.is_empty() {
                    let frame = super::pebble::frame_of(self.program, pc);
                    for r in super::reverse::reverse_reads(&op) {
                        self.recompute_reg(frame, r)?;
                    }
                }
                self.apply_inverse(&op)?;
            } else if records(&op) {
                match tape.pop() {
                    Some(JournalEntry::RegOverwrite { slot, old }) => self.registers[slot] = old,
                    Some(JournalEntry::GlobalOverwrite { idx, old }) => self.globals[idx] = old,
                    Some(JournalEntry::MultiRegOverwrite { slots }) => {
                        for (slot, old) in slots {
                            self.registers[slot] = old;
                        }
                    }
                    Some(JournalEntry::ShowLine) => {
                        self.lines.pop();
                    }
                    Some(JournalEntry::CollectionMutate { slot, old }) => {
                        self.reverse_collection_mutate(slot, old)?;
                    }
                    Some(JournalEntry::ListPop { list_slot, list_old, dst_slot, dst_old }) => {
                        self.reverse_collection_mutate(list_slot, list_old)?;
                        self.registers[dst_slot] = dst_old;
                    }
                    Some(JournalEntry::Branch { .. }) => {}
                    // Frame / iterator entries are consumed by their own reverse handlers, never here.
                    Some(
                        JournalEntry::CallUndo { .. }
                        | JournalEntry::CallValueUndo { .. }
                        | JournalEntry::ReturnUndo { .. }
                        | JournalEntry::IterNextUndo { .. }
                        | JournalEntry::IterPopUndo { .. }
                        | JournalEntry::NondetRead { .. }
                        // Handled by the dedicated Crdt / in-place arithmetic branches, never here.
                        | JournalEntry::CrdtInterior { .. }
                        | JournalEntry::CrdtFieldInterior { .. }
                        | JournalEntry::DivReconstruct { .. }
                        | JournalEntry::ShiftReconstruct { .. }
                        | JournalEntry::MaskReconstruct { .. }
                        | JournalEntry::ModReconstruct { .. },
                    ) => {
                        return Err("vm: control-frame entry reached the data-tape reverse path".to_string());
                    }
                    None => return Err("vm: reverse data-tape underflow".to_string()),
                }
            }
        }
        Ok(true)
    }

    /// Reverse the next `n` recorded steps of `tape` (a scheduled SLICE of one task's execution), or
    /// until its trace runs out. The reversible scheduler calls this per schedule entry, walking the
    /// schedule backward across all task VMs.
    pub(crate) fn reverse_slice(
        &mut self,
        tape: &mut super::journal::HistoryTape,
        n: usize,
    ) -> Result<(), String> {
        for _ in 0..n {
            if !self.reverse_one_step(tape)? {
                break;
            }
        }
        Ok(())
    }

    /// Apply the inverse of a [`crate::vm::reverse::RevClass::Bijection`] op — the zero-garbage,
    /// tape-free reversal. Must handle exactly the bijections
    /// [`crate::vm::reverse::is_reversible_supported`] admits. Sound because the reverse-order
    /// invariant guarantees the machine is in the op's exact POST-state when the inverse is applied,
    /// so every operand the inverse reads holds its forward value.
    fn apply_inverse(&mut self, op: &Op) -> Result<(), String> {
        match op {
            // Involutions — the inverse IS the op itself, re-applied to the post-state.
            Op::RevSwap { a, b } => {
                let sa = self.base + *a as usize;
                let sb = self.base + *b as usize;
                self.registers.swap(sa, sb);
                Ok(())
            }
            Op::RevXor { place, delta } => self.binop(*place, *place, *delta, Value::bitxor),
            Op::RevNeg { place } => self.rev_int_neg(*place),
            // The additive translations invert by the opposite operation, no tape needed.
            Op::RevAdd { place, delta } => self.rev_int_update(*place, *delta, i64::wrapping_sub),
            Op::RevSub { place, delta } => self.rev_int_update(*place, *delta, i64::wrapping_add),
            _ => Ok(()),
        }
    }

    /// In-place modular integer update `R[place] = f(R[place], R[delta])` (wrapping) — the shared
    /// kernel of the reversible additive ops. Requires `Int` operands, like the other numeric ops.
    /// The SAME wrapping semantics run forward and in reverse, so `RevAdd`/`RevSub` are exact mutual
    /// inverses over (ℤ/2⁶⁴, +).
    fn rev_int_update(
        &mut self,
        place: Reg,
        delta: Reg,
        f: fn(i64, i64) -> i64,
    ) -> Result<(), String> {
        use crate::interpreter::RuntimeValue;
        let p = match &*self.reg(place).as_runtime() {
            RuntimeValue::Int(n) => *n,
            _ => return Err("reversible add/subtract requires Int operands".to_string()),
        };
        let d = match &*self.reg(delta).as_runtime() {
            RuntimeValue::Int(n) => *n,
            _ => return Err("reversible add/subtract requires Int operands".to_string()),
        };
        self.set(place, Value::int(f(p, d)));
        Ok(())
    }

    /// Reverse an in-place collection mutation by restoring the pristine pre-mutation snapshot `old`.
    /// Sound because journaling `old` forced the op's copy-on-write, so `R[slot]` now holds a
    /// DIFFERENT `Rc` (the mutated clone) while `old` is the untouched original — restoring it is
    /// bit-identical, including aliasing (a sibling handle that never saw the mutation is left
    /// exactly as it was). If instead `R[slot]` still shares `old`'s `Rc` (a genuine in-place
    /// mutation — only reachable with value-semantics OFF), we cannot un-mutate it from the snapshot,
    /// so we decline rather than mis-restore.
    fn reverse_collection_mutate(&mut self, slot: usize, old: Value) -> Result<(), String> {
        use crate::interpreter::RuntimeValue;
        use std::rc::Rc;
        let shares_rc = matches!(
            (self.registers[slot].as_runtime_ref(), old.as_runtime_ref()),
            (Some(RuntimeValue::List(a)), Some(RuntimeValue::List(b))) if Rc::ptr_eq(a, b)
        ) || matches!(
            (self.registers[slot].as_runtime_ref(), old.as_runtime_ref()),
            (Some(RuntimeValue::Map(a)), Some(RuntimeValue::Map(b))) if Rc::ptr_eq(a, b)
        ) || matches!(
            (self.registers[slot].as_runtime_ref(), old.as_runtime_ref()),
            (Some(RuntimeValue::Set(a)), Some(RuntimeValue::Set(b))) if Rc::ptr_eq(a, b)
        );
        if shares_rc {
            return Err(
                "vm: reverse of an in-place collection mutation requires value semantics"
                    .to_string(),
            );
        }
        self.registers[slot] = old;
        Ok(())
    }

    /// In-place modular negation `R[place] = -R[place]` (wrapping) — self-inverse.
    fn rev_int_neg(&mut self, place: Reg) -> Result<(), String> {
        use crate::interpreter::RuntimeValue;
        let p = match &*self.reg(place).as_runtime() {
            RuntimeValue::Int(n) => *n,
            _ => return Err("reversible negate requires an Int operand".to_string()),
        };
        self.set(place, Value::int(p.wrapping_neg()));
        Ok(())
    }

    /// Journal a function frame's teardown before a `Return`/`ReturnNothing` dismantles it, so the
    /// reverse pass can rebuild the callee window exactly. Captures the callee's saved register
    /// window, the iterator-stack tail, the return-value slot's prior value, and the argument window
    /// (about to be cleared) — all BEFORE any mutation. Called only from the RECORDING monomorph.
    fn record_return(&mut self, frame: &CallFrame) {
        let rv_slot = frame.caller_base + frame.return_reg as usize;
        let arg_hi = (frame.arg_lo + frame.arg_count as usize).min(self.registers.len());
        let frame_from = frame.restore_len;
        let frame_vals = self.registers[frame_from..].to_vec();
        let iter_from = frame.iter_depth;
        let iter_vals = self.iter_stack[iter_from..].to_vec();
        let rv_old = self.registers[rv_slot].clone();
        let arg_old = self.registers[frame.arg_lo..arg_hi].to_vec();
        if let Some(j) = self.journal.as_mut() {
            j.push(super::journal::JournalEntry::ReturnUndo {
                frame_from,
                frame_vals,
                iter_from,
                iter_vals,
                rv_slot,
                rv_old,
                arg_lo: frame.arg_lo,
                arg_old,
            });
        }
    }

    /// Reverse a `Call` / `Return` / `ReturnNothing` by consuming its frame-undo entry — the dual of
    /// [`Vm::record_return`] and the `CallUndo` push. `self.base` was already restored from the step
    /// trace by the caller, so this only rebuilds the register file and iterator stack.
    fn reverse_call_op(
        &mut self,
        op: &Op,
        tape: &mut super::journal::HistoryTape,
    ) -> Result<(), String> {
        use super::journal::JournalEntry;
        match op {
            Op::Call { .. } => match tape.pop() {
                // Undo the callee window growth (the body has already been reversed).
                Some(JournalEntry::CallUndo { restore_len }) => {
                    self.registers.truncate(restore_len);
                    Ok(())
                }
                other => Err(format!("vm: reverse expected CallUndo, got {other:?}")),
            },
            Op::CallValue { .. } => match tape.pop() {
                // Restore any overlap capture slots (survive the truncate), then undo the growth.
                Some(JournalEntry::CallValueUndo {
                    restore_len,
                    captures,
                }) => {
                    for (slot, old) in captures {
                        self.registers[slot] = old;
                    }
                    self.registers.truncate(restore_len);
                    Ok(())
                }
                other => Err(format!("vm: reverse expected CallValueUndo, got {other:?}")),
            },
            Op::Return { .. } | Op::ReturnNothing => match tape.pop() {
                Some(JournalEntry::ReturnUndo {
                    frame_from,
                    frame_vals,
                    iter_from,
                    iter_vals,
                    rv_slot,
                    rv_old,
                    arg_lo,
                    arg_old,
                }) => {
                    // Undo the return-value write and the argument-window clear (surviving region).
                    self.registers[rv_slot] = rv_old;
                    for (i, v) in arg_old.into_iter().enumerate() {
                        self.registers[arg_lo + i] = v;
                    }
                    // Rebuild the callee's register window and iterator stack.
                    self.registers.truncate(frame_from);
                    self.registers.extend(frame_vals);
                    self.iter_stack.truncate(iter_from);
                    self.iter_stack.extend(iter_vals);
                    Ok(())
                }
                other => Err(format!("vm: reverse expected ReturnUndo, got {other:?}")),
            },
            _ => Ok(()),
        }
    }

    /// Journal a nondeterministic read (clock / random): the destination's prior value AND the drawn
    /// entropy. Called only from the RECORDING monomorph. Reverse restores `dst_old`; `drawn` stays on
    /// the tape as the captured entropy (so the recorded execution is fully determined).
    fn record_nondet(&mut self, dst: Reg, drawn: &Value) {
        let dst_slot = self.base + dst as usize;
        let dst_old = self.registers[dst_slot].clone();
        if let Some(j) = self.journal.as_mut() {
            j.push(super::journal::JournalEntry::NondetRead {
                dst_slot,
                dst_old,
                drawn: drawn.clone(),
            });
        }
    }

    /// Reverse an iterator op (`Repeat` machinery) by reconstructing the iterator stack: `IterPop`
    /// re-pushes the dropped iterator, `IterNext` rewinds one element (restoring dst and the cursor)
    /// or does nothing if it was the exhausted exit, and `IterPrepare` pops the (now index-0)
    /// iterator. Walked in reverse execution order, this exactly unwinds a `Repeat` loop.
    fn reverse_iter_op(
        &mut self,
        op: &Op,
        tape: &mut super::journal::HistoryTape,
    ) -> Result<(), String> {
        use super::journal::JournalEntry;
        match op {
            Op::IterPrepare { .. } => {
                self.iter_stack
                    .pop()
                    .ok_or("vm: reverse IterPrepare with no live iterator")?;
                Ok(())
            }
            Op::IterNext { .. } => match tape.pop() {
                Some(JournalEntry::IterNextUndo {
                    advanced,
                    dst_slot,
                    dst_old,
                }) => {
                    if advanced {
                        self.registers[dst_slot] = dst_old;
                        let (_, idx) = self
                            .iter_stack
                            .last_mut()
                            .ok_or("vm: reverse IterNext with no live iterator")?;
                        *idx -= 1;
                    }
                    Ok(())
                }
                other => Err(format!("vm: reverse expected IterNextUndo, got {other:?}")),
            },
            Op::IterPop => match tape.pop() {
                Some(JournalEntry::IterPopUndo { items, idx }) => {
                    self.iter_stack.push((items, idx));
                    Ok(())
                }
                other => Err(format!("vm: reverse expected IterPopUndo, got {other:?}")),
            },
            _ => Ok(()),
        }
    }

    /// Record what `op` is about to DESTROY, before it executes. Called only from the
    /// `RECORDING = true` monomorph, so it is entirely absent from the production loop. Pushes iff
    /// [`crate::vm::reverse::records`] (the dual of `run_reverse`'s pop), keyed off the authoritative
    /// reversibility helpers, never the lossy `disasm::op_io`.
    fn record_op(&mut self, op: &Op, pc: usize) {
        use super::journal::JournalEntry;
        if !super::reverse::records(op) {
            return;
        }
        // Pebbling: the plan elides this write — its destroyed value is program-invisible scratch no
        // reverse step reads, so leave it off the tape (reverse skips it in lockstep via the plan).
        if self.journal_elided(pc) {
            return;
        }
        let entry = match op {
            Op::GlobalSet { idx, .. } => JournalEntry::GlobalOverwrite {
                idx: *idx as usize,
                old: self.globals[*idx as usize].clone(),
            },
            Op::Show { .. } => JournalEntry::ShowLine,
            // A pure builtin writes dst AND clears its argument registers — journal all of them.
            Op::CallBuiltin {
                dst,
                args_start,
                arg_count,
                ..
            } => {
                let base = self.base;
                let dst_slot = base + *dst as usize;
                let mut slots = Vec::with_capacity(1 + *arg_count as usize);
                slots.push((dst_slot, self.registers[dst_slot].clone()));
                for i in 0..*arg_count as usize {
                    let s = base + *args_start as usize + i;
                    slots.push((s, self.registers[s].clone()));
                }
                JournalEntry::MultiRegOverwrite { slots }
            }
            // A tuple destructure writes several registers at once — journal each prior value.
            Op::DestructureTuple { start, count, .. } => {
                let base = self.base;
                let slots = (0..*count as usize)
                    .map(|i| {
                        let slot = base + *start as usize + i;
                        (slot, self.registers[slot].clone())
                    })
                    .collect();
                JournalEntry::MultiRegOverwrite { slots }
            }
            // A pop mutates the list AND overwrites a dst — journal both pristine snapshots.
            Op::ListPop { list, dst } => {
                let list_slot = self.base + *list as usize;
                let dst_slot = self.base + *dst as usize;
                JournalEntry::ListPop {
                    list_slot,
                    list_old: self.registers[list_slot].clone(),
                    dst_slot,
                    dst_old: self.registers[dst_slot].clone(),
                }
            }
            // A CRDT sequence append mutates the shared `Rc<RefCell<CrdtValue>>` interior IN PLACE, so
            // an Rc-bump can't force a copy-on-write — take a DEEP snapshot of the interior instead.
            // (A `CrdtAppend` onto a plain list IS Rc-bump COW-forced, like `ListPush`.)
            Op::CrdtAppend { seq, .. } => {
                use crate::interpreter::RuntimeValue;
                let slot = self.base + *seq as usize;
                match self.registers[slot].as_runtime_ref() {
                    Some(RuntimeValue::Crdt(rc)) => {
                        let deep = rc.borrow().clone();
                        let snapshot = Value::from_runtime(RuntimeValue::Crdt(std::rc::Rc::new(
                            std::cell::RefCell::new(deep),
                        )));
                        JournalEntry::CrdtInterior { slot, snapshot }
                    }
                    _ => JournalEntry::CollectionMutate {
                        slot,
                        old: self.registers[slot].clone(),
                    },
                }
            }
            // A CRDT resolve onto a nested register FIELD mutates that field's `Rc` interior in place
            // (deep field snapshot); onto a plain field it replaces the value (a Box snapshot is exact).
            Op::CrdtResolve { obj, field, .. } => {
                use crate::interpreter::RuntimeValue;
                let obj_slot = self.base + *obj as usize;
                let field_name = match &self.program.constants[*field as usize] {
                    Constant::Text(s) => s.clone(),
                    _ => String::new(),
                };
                let field_crdt = match self.registers[obj_slot].as_runtime_ref() {
                    Some(RuntimeValue::Struct(s)) => match s.fields.get(&field_name) {
                        Some(RuntimeValue::Crdt(rc)) => Some(rc.borrow().clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(deep) = field_crdt {
                    let snapshot = Value::from_runtime(RuntimeValue::Crdt(std::rc::Rc::new(
                        std::cell::RefCell::new(deep),
                    )));
                    JournalEntry::CrdtFieldInterior {
                        obj_slot,
                        field: field_name,
                        snapshot,
                    }
                } else {
                    JournalEntry::CollectionMutate {
                        slot: obj_slot,
                        old: self.registers[obj_slot].clone(),
                    }
                }
            }
            // An IN-PLACE integer division `a = a / b` discards only the remainder — record just that
            // (reconstruct `a_old = q·b + r` on reverse from the kept quotient + divisor), not the
            // whole dividend word. A non-integer or zero divisor falls back to the full overwrite.
            Op::Div { dst, lhs, rhs } if dst == lhs => {
                use crate::interpreter::RuntimeValue;
                let dst_slot = self.base + *dst as usize;
                let divisor = self.registers[self.base + *rhs as usize].clone();
                match (self.registers[dst_slot].as_int(), divisor.as_int()) {
                    (Some(a), Some(b)) if b != 0 => JournalEntry::DivReconstruct {
                        dst_slot,
                        remainder: Value::from_runtime(RuntimeValue::Int(a.wrapping_rem(b))),
                        divisor,
                    },
                    _ => JournalEntry::RegOverwrite {
                        slot: dst_slot,
                        old: self.registers[dst_slot].clone(),
                    },
                }
            }
            // An IN-PLACE right shift `a = a >> k` destroys only the low `k` bits — journal just those
            // (reconstruct `a_old = (a << k) | dropped` on reverse), not the whole word. `k` is masked
            // to the machine width; `k == 0` (identity) or a non-integer falls back to a full overwrite.
            Op::Shr { dst, lhs, rhs } if dst == lhs => {
                let dst_slot = self.base + *dst as usize;
                let k = self.registers[self.base + *rhs as usize]
                    .as_int()
                    .map(|s| (s as u64 & 63) as u32);
                match (self.registers[dst_slot].as_int(), k) {
                    (Some(a), Some(k)) if k != 0 => {
                        let mask = ((1u64 << k) - 1) as i64;
                        JournalEntry::ShiftReconstruct {
                            dst_slot,
                            dropped: a & mask,
                            k,
                            left: false,
                        }
                    }
                    _ => JournalEntry::RegOverwrite {
                        slot: dst_slot,
                        old: self.registers[dst_slot].clone(),
                    },
                }
            }
            // An IN-PLACE `a = a / 2^k` (`DivPow2`) equals the arithmetic right shift only for a_old ≥ 0
            // (negatives truncate toward zero, not −∞) — restrict the low-bit reconstruct to that case.
            Op::DivPow2 { dst, lhs, k } if dst == lhs => {
                let dst_slot = self.base + *dst as usize;
                let k = *k as u32;
                match self.registers[dst_slot].as_int() {
                    Some(a) if a >= 0 && k != 0 && k < 64 => {
                        let mask = ((1u64 << k) - 1) as i64;
                        JournalEntry::ShiftReconstruct {
                            dst_slot,
                            dropped: a & mask,
                            k,
                            left: false,
                        }
                    }
                    _ => JournalEntry::RegOverwrite {
                        slot: dst_slot,
                        old: self.registers[dst_slot].clone(),
                    },
                }
            }
            // An IN-PLACE left shift `a = a << k` destroys only the top `k` bits (shifted out) — journal
            // just those (right-aligned), reconstruct the low bits from the kept result on reverse.
            Op::Shl { dst, lhs, rhs } if dst == lhs => {
                let dst_slot = self.base + *dst as usize;
                let k = self.registers[self.base + *rhs as usize]
                    .as_int()
                    .map(|s| (s as u64 & 63) as u32);
                match (self.registers[dst_slot].as_int(), k) {
                    (Some(a), Some(k)) if k != 0 => {
                        let dropped = ((a as u64) >> (64 - k)) as i64;
                        JournalEntry::ShiftReconstruct {
                            dst_slot,
                            dropped,
                            k,
                            left: true,
                        }
                    }
                    _ => JournalEntry::RegOverwrite {
                        slot: dst_slot,
                        old: self.registers[dst_slot].clone(),
                    },
                }
            }
            // An IN-PLACE `a = a & b` destroys the bits where `b = 0`; `a = a | b` the bits where
            // `b = 1`. Journal only those (the complementary mask recovers the rest from the result).
            Op::BitAnd { dst, lhs, rhs } if dst == lhs => {
                let dst_slot = self.base + *dst as usize;
                match (
                    self.registers[dst_slot].as_int(),
                    self.registers[self.base + *rhs as usize].as_int(),
                ) {
                    (Some(a), Some(m)) => JournalEntry::MaskReconstruct {
                        dst_slot,
                        saved: a & !m,
                        mask: m,
                        and: true,
                    },
                    _ => JournalEntry::RegOverwrite {
                        slot: dst_slot,
                        old: self.registers[dst_slot].clone(),
                    },
                }
            }
            Op::BitOr { dst, lhs, rhs } if dst == lhs => {
                let dst_slot = self.base + *dst as usize;
                match (
                    self.registers[dst_slot].as_int(),
                    self.registers[self.base + *rhs as usize].as_int(),
                ) {
                    (Some(a), Some(m)) => JournalEntry::MaskReconstruct {
                        dst_slot,
                        saved: a & m,
                        mask: m,
                        and: false,
                    },
                    _ => JournalEntry::RegOverwrite {
                        slot: dst_slot,
                        old: self.registers[dst_slot].clone(),
                    },
                }
            }
            // An IN-PLACE `a = a % b` keeps the remainder, discards the QUOTIENT — journal just the
            // quotient, reconstruct `a_old = q·b + r` from the live divisor on reverse.
            Op::Mod { dst, lhs, rhs } if dst == lhs => {
                let dst_slot = self.base + *dst as usize;
                let divisor = self.registers[self.base + *rhs as usize].clone();
                match (self.registers[dst_slot].as_int(), divisor.as_int()) {
                    (Some(a), Some(b)) if b != 0 => JournalEntry::ModReconstruct {
                        dst_slot,
                        quotient: a.wrapping_div(b),
                        divisor,
                    },
                    _ => JournalEntry::RegOverwrite {
                        slot: dst_slot,
                        old: self.registers[dst_slot].clone(),
                    },
                }
            }
            // Journal the pre-mutation collection value (an Rc bump for a shared container). That
            // very bump raises the strong count, forcing the op's copy-on-write, so the original
            // stays pristine for the reverse to restore.
            _ if super::reverse::mutated_collection_reg(op).is_some() => {
                let slot = self.base + super::reverse::mutated_collection_reg(op).unwrap() as usize;
                JournalEntry::CollectionMutate {
                    slot,
                    old: self.registers[slot].clone(),
                }
            }
            _ => {
                let dst = super::reverse::overwritten_reg(op)
                    .expect("records() ⇒ writes a global/output line or has an overwritten_reg");
                let slot = self.base + dst as usize;
                JournalEntry::RegOverwrite {
                    slot,
                    old: self.registers[slot].clone(),
                }
            }
        };
        if let Some(j) = self.journal.as_mut() {
            j.push(entry);
        }
    }

    /// Run one slice: from a fresh start (or resumed from a prior block) until the
    /// program completes ([`VmStep::Done`]) or a concurrency op suspends it
    /// ([`VmStep::Blocked`], request in [`Vm::take_pending`]). Keeping `pc` and the
    /// call stack as loop-locals (restored once on entry, saved once on a block)
    /// leaves the hot dispatch path byte-for-byte identical to the old `run`.
    pub(crate) fn run_until_block(&mut self) -> Result<VmStep, String> {
        let step = self.run_until_block_impl::<false, false>(u64::MAX);
        self.end_with_unhandled(step)
    }

    /// A program that has finished (`Done`) with a contained failure no handler absorbed has
    /// failed — the page's own failure is the program's once nothing is left to run — and the
    /// failure site of that end is the one the failure trapped at.
    fn end_with_unhandled(&mut self, step: Result<VmStep, String>) -> Result<VmStep, String> {
        match step {
            Ok(VmStep::Done(_)) if self.unhandled.is_some() => {
                let (message, site) = self.unhandled.take().expect("checked");
                self.ended_at_site = Some(site);
                Err(message)
            }
            other => other,
        }
    }

    /// The failure a page entry the program took itself contained and no handler absorbed, taken
    /// by the host that is carrying the page on — whose own record of the page's failure it
    /// becomes ([`crate::page_host::LoweredCalls::take_unhandled`]).
    pub fn take_unhandled(&mut self) -> Option<(String, Option<u32>)> {
        self.unhandled.take()
    }

    /// The main program, run to its end with any contained failure left for the host to take
    /// ([`Vm::take_unhandled`]) rather than made the run's own: the start of a page a host keeps
    /// driving.
    pub fn run_keeping_entries(&mut self) -> Result<(), String> {
        logicaffeine_base::money::clear_ambient_rates();
        crate::semantics::dom::retire_stream();
        match self.run_until_block_impl::<false, false>(u64::MAX)? {
            VmStep::Done(_) => Ok(()),
            VmStep::Blocked => Err("vm: concurrency op requires the scheduler driver".to_string()),
            VmStep::Paused => unreachable!("run_until_block (STEPPED = false) never pauses"),
        }
    }

    /// `dispatchEntry(dispatcher, context, hook, node, event, value)` read from the argument window
    /// at `args_start` — [`crate::semantics::builtins::BuiltinId::DomDispatchEntry`]: the page's
    /// dispatcher run as the host runs an entry ([`Vm::invoke_function`], which leaves the machine
    /// as a failure found it), a failure heard through the page's failure door exactly as
    /// `page_host`'s guarded door hears one — the context text first, then the handler, whose own
    /// failure never replaces the one being reported — and recorded for the program's end when no
    /// handler absorbed it. A failed turn was heard.
    #[cold]
    #[inline(never)]
    fn dispatch_entry(
        &mut self,
        args_start: Reg,
    ) -> Result<crate::interpreter::RuntimeValue, String> {
        use crate::interpreter::RuntimeValue;
        let function = |vm: &Self, k: Reg| {
            vm.reg(args_start + k)
                .as_int()
                .and_then(|i| u16::try_from(i).ok())
        };
        let dispatcher =
            function(self, 0).ok_or("dispatchEntry: the page has no dispatcher to enter")?;
        let (context, hook) = (function(self, 1), function(self, 2));
        let args: Vec<RuntimeValue> = (3..6)
            .map(|k| self.reg(args_start + k).as_runtime().clone())
            .collect();
        let message = match self.invoke_function(dispatcher, args) {
            Ok(answer) => return Ok(answer),
            Err(message) => message,
        };
        let site = self.failing_site();
        let context = match context.map(|f| self.invoke_function(f, Vec::new())) {
            Some(Ok(RuntimeValue::Text(text))) => text.to_string(),
            _ => String::new(),
        };
        let message = format!("{context}{message}");
        let absorbed = hook.is_some_and(|f| {
            matches!(
                self.invoke_function(f, vec![RuntimeValue::Text(message.clone().into())]),
                Ok(RuntimeValue::Bool(true))
            )
        });
        if !absorbed && self.unhandled.is_none() {
            self.unhandled = Some((message, site));
        }
        Ok(RuntimeValue::Bool(true))
    }

    /// Like [`Vm::run_until_block`] but with reversibility RECORDING enabled — the per-task recording
    /// primitive for a reversible scheduler. Runs one slice (from a fresh start or resumed from a
    /// prior block) with journaling on, ACCUMULATING the history tape across slices, and returns at
    /// the next concurrency block or completion. The scheduler drives each task through its blocks;
    /// on reverse it walks the schedule backward and reverses each task's slice from this tape (plus
    /// the channel-op deltas the scheduler journals). Zero production cost — a separate monomorph the
    /// hot `run_until_block` (`RECORDING = false`) never instantiates.
    pub(crate) fn run_until_block_recording(&mut self) -> Result<VmStep, String> {
        if self.journal.is_none() {
            self.journal = Some(super::journal::HistoryTape::new());
        }
        self.run_until_block_impl::<false, true>(u64::MAX)
    }

    /// Step forward by at most `step_budget` ops, then pause ([`VmStep::Paused`],
    /// resumable on the next call). Two callers: the Studio debug drawer, and the
    /// fueled run path behind `largo run --max-steps` / the MCP `step_budget`
    /// (`ui_bridge::interpret_for_ui_sync_with_args`). Production unbudgeted
    /// callers use [`Vm::run_until_block`] (`STEPPED = false`), whose
    /// monomorphization elides every budget check — the hot dispatch path stays
    /// the byte-for-byte old loop.
    pub(crate) fn run_steps(&mut self, step_budget: u64) -> Result<VmStep, String> {
        let step = self.run_until_block_impl::<true, false>(step_budget);
        self.end_with_unhandled(step)
    }

    /// Ops run by every [`Vm::run_steps`] slice so far.
    pub(crate) fn steps_run(&self) -> u64 {
        self.stepped
    }

    fn run_until_block_impl<const STEPPED: bool, const RECORDING: bool>(
        &mut self,
        step_budget: u64,
    ) -> Result<VmStep, String> {
        self.ended_at_site = None;
        let mut pc;
        let mut call_stack: Vec<CallFrame>;
        if self.sched_active {
            pc = self.sched_pc;
            call_stack = std::mem::take(&mut self.sched_call_stack);
            self.sched_active = false;
        } else {
            pc = 0usize;
            call_stack = Vec::new();
        }

        // The loop ends when the top-level program code is exhausted. A warm body
        // lives past `program.code.len()` but is only ever entered via a `Call` (which
        // pushes a frame), so `!call_stack.is_empty()` keeps the loop alive while one
        // is executing; without any warm body installed a live frame already implies
        // `pc < program.code.len()`, so this disjunct is a no-op for the baseline path.
        let mut executed: u64 = 0;
        while pc < self.program.code.len() || !call_stack.is_empty() {
            if STEPPED {
                if executed >= step_budget {
                    // Pause exactly as a concurrency block saves its slice (pc +
                    // call stack into the scheduler slots), but with no pending
                    // request — the debugger resumes on the next `run_steps`.
                    self.sched_pc = pc;
                    self.sched_call_stack = call_stack;
                    self.sched_active = true;
                    return Ok(VmStep::Paused);
                }
                executed += 1;
                self.stepped += 1;
            }
            let op = if pc < self.program.code.len() {
                self.program.code[pc]
            } else {
                self.warm_code[pc - self.program.code.len()]
            };
            // Wave 3 ("runtime failures carry a location"): the pc this op is ABOUT to run at,
            // saved before it can fail. An `Err` from any op below propagates via `?` straight out
            // of this loop, so this is the last write `sched_pc` sees before that happens —
            // `failing_site` reads it once the caller has `Err` in hand.
            self.sched_pc = pc;
            // Reversibility recording — trace the executed pc (the control skeleton) and journal what
            // this op is about to destroy, BEFORE it runs. Behind the `const RECORDING` monomorph:
            // absent from the production loop (dead-stripped when `RECORDING = false`, exactly like
            // the `STEPPED` budget check above).
            if RECORDING {
                let base = self.base;
                if let Some(j) = self.journal.as_mut() {
                    j.push_step(pc, base);
                }
                self.record_op(&op, pc);
            }
            match op {
                Op::LoadConst { dst, idx } => {
                    let v = self.const_pool[idx as usize].clone();
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Move { dst, src } => {
                    self.set(dst, self.reg(src).clone());
                    pc += 1;
                }
                Op::RevSwap { a, b } => {
                    let sa = self.base + a as usize;
                    let sb = self.base + b as usize;
                    self.registers.swap(sa, sb);
                    pc += 1;
                }
                Op::RevXor { place, delta } => {
                    self.binop(place, place, delta, Value::bitxor)?;
                    pc += 1;
                }
                Op::RevNeg { place } => {
                    self.rev_int_neg(place)?;
                    pc += 1;
                }
                Op::RevAdd { place, delta } => {
                    self.rev_int_update(place, delta, i64::wrapping_add)?;
                    pc += 1;
                }
                Op::RevSub { place, delta } => {
                    self.rev_int_update(place, delta, i64::wrapping_sub)?;
                    pc += 1;
                }
                Op::EnsureOwned { reg } => {
                    // Call-site copy-on-write barrier: isolate a shared collection — or each list
                    // field of a record — before it is passed to a `mutable` or mutable-borrow
                    // callee. No-op when the register is a `mutable`-exempt param (its writes are
                    // its caller's) or the value is already uniquely owned.
                    let cur_func = call_stack.last().map(|f| f.func);
                    self.ensure_reg_owned(reg, pc, cur_func);
                    if crate::semantics::collections::value_semantics_enabled()
                        && !self.is_cow_exempt(reg, cur_func)
                    {
                        self.own_struct_fields(reg, pc, cur_func);
                    }
                    pc += 1;
                }
                Op::Add { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::add)?;
                    pc += 1;
                }
                Op::AddAssign { dst, src } => {
                    self.add_assign(dst, src)?;
                    pc += 1;
                }
                Op::Sub { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::sub)?;
                    pc += 1;
                }
                Op::Mul { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::mul)?;
                    pc += 1;
                }
                Op::Div { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::div)?;
                    pc += 1;
                }
                Op::ExactDiv { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::exact_div)?;
                    pc += 1;
                }
                Op::FloorDiv { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::floor_div)?;
                    pc += 1;
                }
                Op::Mod { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::modulo)?;
                    pc += 1;
                }
                Op::DivPow2 { dst, lhs, k } => {
                    // `lhs / 2^k` (lhs is Oracle-proven Int) — identical result
                    // to `Op::Div` by `2^k`, so it matches the tree-walker.
                    let v = self.reg(lhs).div(&Value::int(1i64 << k))?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::MagicDivU {
                    dst,
                    lhs,
                    magic,
                    more,
                    mul_back,
                    divisor,
                    wrap,
                } => {
                    // `lhs / c` / `lhs % c` by the precomputed magic reciprocal
                    // (`magic`/`more`). Emitted only for an Oracle-proven Int,
                    // non-negative `lhs`, so the result is bit-identical to
                    // `Op::Div`/`Op::Mod` by the constant `c`. A `wrap` remainder's
                    // `lhs` is also below `2c`: one subtraction at most.
                    let v = match self.reg(lhs).as_int() {
                        Some(x) if wrap && mul_back != 0 => {
                            Value::int(if x >= divisor { x - divisor } else { x })
                        }
                        Some(x) => {
                            Value::int(crate::vm::compiler::magic_eval(x, magic, more, mul_back))
                        }
                        // The optimistic gate does not prove `lhs` fits i64 — a widening loop can make
                        // it a `BigInt` the u64 reciprocal cannot represent. Fall back to the exact
                        // op by the carried `divisor`: `mul_back == 0` is division, else modulo.
                        // Bit-identical to `Op::Div`/`Op::Mod`, so the tier differential holds.
                        None => {
                            let d = Value::int(divisor);
                            if mul_back == 0 {
                                self.reg(lhs).div(&d)?
                            } else {
                                self.reg(lhs).modulo(&d)?
                            }
                        }
                    };
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Lt { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::lt)?;
                    pc += 1;
                }
                Op::Gt { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::gt)?;
                    pc += 1;
                }
                Op::LtEq { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::lte)?;
                    pc += 1;
                }
                Op::GtEq { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::gte)?;
                    pc += 1;
                }
                Op::Eq { dst, lhs, rhs } => {
                    let v = self.reg(lhs).eq_op(self.reg(rhs))?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::ApproxEq { dst, lhs, rhs } => {
                    let v = crate::semantics::arith::approx_eq(
                        self.reg(lhs).as_runtime().clone(),
                        self.reg(rhs).as_runtime().clone(),
                    )
                    .map(Value::from_runtime)?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::NotEq { dst, lhs, rhs } => {
                    let v = self.reg(lhs).neq_op(self.reg(rhs))?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Not { dst, src } => {
                    let v = self.reg(src).not_op()?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Concat { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::concat)?;
                    pc += 1;
                }
                Op::SeqConcat { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::seq_concat)?;
                    pc += 1;
                }
                Op::Pow { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::pow)?;
                    pc += 1;
                }
                Op::BitXor { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::bitxor)?;
                    pc += 1;
                }
                Op::BitAnd { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::bitand)?;
                    pc += 1;
                }
                Op::BitOr { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::bitor)?;
                    pc += 1;
                }
                Op::Shl { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::shl)?;
                    pc += 1;
                }
                Op::Shr { dst, lhs, rhs } => {
                    self.binop(dst, lhs, rhs, Value::shr)?;
                    pc += 1;
                }
                Op::Jump { target } => {
                    // A back-edge is a hot-loop candidate in EVERY frame
                    // (OSR everywhere); the enclosing frame picks the
                    // named-register map the write-back contract consults.
                    // Skip the whole region machinery (the per-frame named-map
                    // selection + the `regions` hashmap probe) once this loop
                    // head is blacklisted — an un-tierable / demoted region pays
                    // only this O(1) `Vec<bool>` index per back-edge instead of
                    // re-hashing every iteration.
                    // `target < program.code.len()` keeps the region machinery
                    // (blacklist indexed by program pc, `program.code[head..]` scans)
                    // off warm-body back-edges; always true on the baseline path.
                    if target < pc
                        && target < self.program.code.len()
                        && self.tier.is_some()
                        && !self.region_blacklist[target]
                    {
                        let (named, frame_regs): (&[bool], usize) = match call_stack.last() {
                            None => (&self.program.named_regs, self.program.register_count),
                            Some(f) => {
                                let fun = &self.program.functions[f.func as usize];
                                (&fun.named_regs, fun.register_count)
                            }
                        };
                        let cur_func = call_stack.last().map(|f| f.func);
                        match self.try_region(
                            target,
                            pc,
                            named,
                            frame_regs,
                            call_stack.len(),
                            cur_func,
                        ) {
                            Some(RegionExit::At(exit)) => {
                                pc = exit;
                                continue;
                            }
                            Some(RegionExit::Call {
                                call_pc,
                                resume_pc,
                                frames,
                                list_args,
                            }) => {
                                let Some(Op::Call {
                                    dst,
                                    func,
                                    args_start,
                                    ..
                                }) = self.program.code.get(call_pc).copied()
                                else {
                                    return Err(
                                        "vm: native region exit has no call site".to_string()
                                    );
                                };
                                self.materialize_native_frames(
                                    &frames,
                                    &list_args,
                                    args_start,
                                    dst,
                                    func,
                                    call_pc + 1,
                                    &mut call_stack,
                                )?;
                                pc = resume_pc;
                                continue;
                            }
                            Some(RegionExit::Return(value)) => {
                                // The region hit a `Return` — perform the
                                // actual function return, exactly like the
                                // Op::Return arm.
                                let frame = call_stack.pop().ok_or("vm: return with no caller")?;
                                self.iter_stack.truncate(frame.iter_depth);
                                self.registers.truncate(frame.restore_len);
                                self.base = frame.caller_base;
                                self.set(frame.return_reg, value);
                                self.clear_arg_window(&frame);
                                pc = frame.return_pc;
                                continue;
                            }
                            None => {}
                        }
                    }
                    pc = target;
                }
                Op::JumpIfFalse { cond, target } => {
                    if !self.reg(cond).decided_truth()? {
                        pc = target;
                    } else {
                        pc += 1;
                    }
                }
                Op::JumpIfTrue { cond, target } => {
                    if self.reg(cond).decided_truth()? {
                        pc = target;
                    } else {
                        pc += 1;
                    }
                }
                Op::GlobalGet { dst, idx } => {
                    match &self.globals[idx as usize] {
                        Some(v) => {
                            let v = v.clone();
                            self.set(dst, v);
                        }
                        None => {
                            return Err(format!(
                                "Undefined variable: {}",
                                self.program.globals[idx as usize]
                            ));
                        }
                    }
                    pc += 1;
                }
                Op::GlobalSet { idx, src } => {
                    self.globals[idx as usize] = Some(self.reg(src).clone());
                    pc += 1;
                }
                Op::MakeClosure {
                    dst,
                    func,
                    locals_start,
                } => {
                    use crate::interpreter::{ClosureValue, RuntimeValue};
                    let f = self
                        .program
                        .functions
                        .get(func as usize)
                        .ok_or("vm: MakeClosure on undefined function index")?;
                    let mut captured_env = std::collections::HashMap::new();
                    let mut local_k: Reg = 0;
                    for (sym, global_idx) in &f.captures {
                        match global_idx {
                            Some(gidx) => {
                                // Snapshot the global IF it is defined; an
                                // undefined one is simply not captured — the
                                // body falls through to the live global.
                                if let Some(v) = &self.globals[*gidx as usize] {
                                    captured_env.insert(*sym, v.as_runtime().deep_clone());
                                }
                            }
                            None => {
                                let v = self.reg(locals_start + local_k).as_runtime().deep_clone();
                                captured_env.insert(*sym, v);
                                local_k += 1;
                            }
                        }
                    }
                    let param_names =
                        vec![crate::intern::Symbol::default(); f.param_count as usize];
                    self.set(
                        dst,
                        Value::from_runtime(RuntimeValue::Function(Box::new(ClosureValue {
                            body_index: func as usize,
                            captured_env,
                            param_names,
                            generated: None,
                        }))),
                    );
                    pc += 1;
                }
                Op::CallValue {
                    dst,
                    callee,
                    args_start,
                    arg_count,
                    name_for_err,
                } => {
                    use crate::interpreter::RuntimeValue;
                    let closure = match &*self.reg(callee).as_runtime() {
                        RuntimeValue::Function(c) => (**c).clone(),
                        other => {
                            return Err(if name_for_err == u32::MAX {
                                format!("Cannot call value of type {}", other.type_name())
                            } else {
                                match &self.program.constants[name_for_err as usize] {
                                    Constant::Text(n) => format!("Unknown function: {}", n),
                                    _ => format!("Cannot call value of type {}", other.type_name()),
                                }
                            });
                        }
                    };
                    pc = self.enter_closure::<RECORDING>(
                        dst,
                        &closure,
                        args_start,
                        arg_count,
                        pc + 1,
                        &mut call_stack,
                    )?;
                }
                Op::CallBuiltin {
                    dst,
                    builtin: crate::semantics::builtins::BuiltinId::DomDispatchEntry,
                    args_start,
                    arg_count,
                } => {
                    let v = self.dispatch_entry(args_start)?;
                    self.set(dst, Value::from_runtime(v));
                    self.clear_args(args_start, arg_count, None);
                    pc += 1;
                }
                Op::CallBuiltin {
                    dst,
                    builtin,
                    args_start,
                    arg_count,
                } => {
                    let mut args = Vec::with_capacity(arg_count as usize);
                    for k in 0..arg_count {
                        args.push(self.reg(args_start + k).as_runtime().clone());
                    }
                    let v = if builtin == crate::semantics::builtins::BuiltinId::DefinedReal {
                        self.open_defined_run().define(args)?
                    } else {
                        crate::semantics::builtins::call_builtin(builtin, args)?
                    };
                    self.set(dst, Value::from_runtime(v));
                    self.clear_args(args_start, arg_count, None);
                    pc += 1;
                }
                Op::Call {
                    dst,
                    func,
                    args_start,
                    arg_count,
                } => {
                    // A suspended caller must not retain dead construction
                    // temporaries as extra owners of a callee's global values.
                    // Liveness includes the whole argument window; named
                    // bindings and arguments needed after nested calls survive.
                    // The old call-destination value needs no separate pin:
                    // if it is also an argument, liveness already protects it.
                    if !RECORDING && crate::semantics::collections::value_semantics_enabled() {
                        self.release_dead_scratch(
                            None,
                            pc,
                            call_stack.last().map(|frame| frame.func),
                            None,
                        );
                    }
                    pc = self.enter_call::<RECORDING>(
                        dst,
                        func,
                        args_start,
                        arg_count,
                        pc + 1,
                        &mut call_stack,
                    )?;
                }
                Op::Return { src } => {
                    let frame = call_stack.pop().ok_or("vm: return with no caller")?;
                    let rv = self.reg(src).clone();
                    if RECORDING {
                        self.record_return(&frame);
                    }
                    self.iter_stack.truncate(frame.iter_depth);
                    self.registers.truncate(frame.restore_len);
                    self.base = frame.caller_base;
                    let slot = self.base + frame.return_reg as usize;
                    self.registers[slot] = rv;
                    self.clear_arg_window(&frame);
                    pc = frame.return_pc;
                }
                Op::ReturnNothing => {
                    let frame = call_stack.pop().ok_or("vm: return with no caller")?;
                    if RECORDING {
                        self.record_return(&frame);
                    }
                    self.iter_stack.truncate(frame.iter_depth);
                    self.registers.truncate(frame.restore_len);
                    self.base = frame.caller_base;
                    let slot = self.base + frame.return_reg as usize;
                    self.registers[slot] = Value::nothing();
                    self.clear_arg_window(&frame);
                    pc = frame.return_pc;
                }
                Op::NewList { dst, start, count } => {
                    let mut items = Vec::with_capacity(count as usize);
                    for k in 0..count {
                        items.push(self.reg(start + k).clone());
                    }
                    self.set(dst, Value::list(items));
                    pc += 1;
                }
                Op::NewEmptyList { dst } => {
                    // Allocation reuse: if `dst` already holds a SOLE-OWNED
                    // Ints list (e.g. the previous loop iteration's, now dead),
                    // clear it in place and reuse its buffer/capacity instead
                    // of allocating a fresh Rc + Vec. Sound: refcount 1 means
                    // no other holder can observe the clear.
                    use crate::interpreter::{ListRepr, RuntimeValue};
                    use std::rc::Rc;
                    let di = self.base + dst as usize;
                    let reused = matches!(
                        self.registers.get(di).map(|v| v.as_runtime()).as_deref(),
                        Some(RuntimeValue::List(rc))
                            if Rc::strong_count(rc) == 1
                                && Rc::weak_count(rc) == 0
                                && matches!(&*rc.borrow(), ListRepr::Ints(_))
                    );
                    if reused {
                        if let RuntimeValue::List(rc) = &*self.registers[di].as_runtime() {
                            if let ListRepr::Ints(buf) = &mut *rc.borrow_mut() {
                                buf.clear();
                            }
                        }
                    } else {
                        self.set(dst, Value::empty_list());
                    }
                    pc += 1;
                }
                Op::NewEmptyListFixed { dst, capacity } => {
                    // Preserve the shared fixed-size proof in the VM: reuse a
                    // unique buffer when possible, otherwise reserve exactly
                    // the proven capacity up front.
                    use crate::interpreter::{ListRepr, RuntimeValue};
                    use std::rc::Rc;
                    let di = self.base + dst as usize;
                    let reused = matches!(
                        self.registers.get(di).map(|v| v.as_runtime()).as_deref(),
                        Some(RuntimeValue::List(rc))
                            if Rc::strong_count(rc) == 1
                                && Rc::weak_count(rc) == 0
                                && matches!(&*rc.borrow(), ListRepr::Ints(_))
                    );
                    if reused {
                        if let RuntimeValue::List(rc) = &*self.registers[di].as_runtime() {
                            if let ListRepr::Ints(buf) = &mut *rc.borrow_mut() {
                                buf.clear();
                                if buf.capacity() < capacity as usize {
                                    buf.reserve_exact(capacity as usize);
                                }
                            }
                        }
                    } else {
                        self.set(dst, Value::empty_list_with_capacity(capacity as usize));
                    }
                    pc += 1;
                }
                Op::NewEmptyListWithCapacity { dst, capacity } => {
                    let capacity = self
                        .reg(capacity)
                        .as_int()
                        .ok_or_else(|| "Sequence capacity requires an Int".to_string())?
                        .max(0) as usize;
                    use crate::interpreter::{ListRepr, RuntimeValue};
                    use std::rc::Rc;
                    let di = self.base + dst as usize;
                    let reused = matches!(
                        self.registers.get(di).map(|v| v.as_runtime()).as_deref(),
                        Some(RuntimeValue::List(rc))
                            if Rc::strong_count(rc) == 1
                                && Rc::weak_count(rc) == 0
                                && matches!(&*rc.borrow(), ListRepr::Ints(_))
                    );
                    if reused {
                        if let RuntimeValue::List(rc) = &*self.registers[di].as_runtime() {
                            if let ListRepr::Ints(buf) = &mut *rc.borrow_mut() {
                                buf.clear();
                                if buf.capacity() < capacity {
                                    let _ = buf.try_reserve_exact(capacity);
                                }
                            }
                        }
                    } else {
                        let mut buf = Vec::new();
                        let _ = buf.try_reserve_exact(capacity);
                        self.set(
                            dst,
                            Value::from_runtime(RuntimeValue::List(Rc::new(
                                std::cell::RefCell::new(ListRepr::Ints(buf)),
                            ))),
                        );
                    }
                    pc += 1;
                }
                Op::NewEmptyListI32 { dst } => {
                    // Mirror NewEmptyList's allocation reuse, but for the
                    // half-width `IntsI32` repr (the narrowing-proven buffer).
                    use crate::interpreter::{ListRepr, RuntimeValue};
                    use std::rc::Rc;
                    let di = self.base + dst as usize;
                    let reused = matches!(
                        self.registers.get(di).map(|v| v.as_runtime()).as_deref(),
                        Some(RuntimeValue::List(rc))
                            if Rc::strong_count(rc) == 1
                                && Rc::weak_count(rc) == 0
                                && matches!(&*rc.borrow(), ListRepr::IntsI32(_))
                    );
                    if reused {
                        if let RuntimeValue::List(rc) = &*self.registers[di].as_runtime() {
                            if let ListRepr::IntsI32(buf) = &mut *rc.borrow_mut() {
                                buf.clear();
                            }
                        }
                    } else {
                        self.set(dst, Value::empty_list_i32());
                    }
                    pc += 1;
                }
                Op::NewEmptySet { dst } => {
                    self.set(dst, Value::empty_set());
                    pc += 1;
                }
                Op::NewEmptyMap { dst } => {
                    self.set(dst, Value::empty_map());
                    pc += 1;
                }
                Op::NewEmptyMapWithCapacity { dst, capacity } => {
                    let capacity = self
                        .reg(capacity)
                        .as_int()
                        .ok_or_else(|| "Map capacity requires an Int".to_string())?
                        .max(0) as usize;
                    self.set(dst, Value::empty_map_with_capacity(capacity));
                    pc += 1;
                }
                Op::NewRange { dst, start, end } => {
                    let (lo, hi) = match (self.reg(start).as_int(), self.reg(end).as_int()) {
                        (Some(lo), Some(hi)) => (lo, hi),
                        _ => return Err("Range requires Int bounds".to_string()),
                    };
                    self.set(dst, Value::int_range(lo, hi));
                    pc += 1;
                }
                Op::ListPush { list, value } => {
                    let v = self.reg(value).clone();
                    self.ensure_reg_owned(list, pc, call_stack.last().map(|f| f.func));
                    self.reg(list).list_push(v)?;
                    pc += 1;
                }
                Op::SetAdd { set, value } => {
                    let v = self.reg(value).clone();
                    self.ensure_reg_owned(set, pc, call_stack.last().map(|f| f.func));
                    self.reg(set).set_add(v)?;
                    pc += 1;
                }
                Op::RemoveFrom { collection, value } => {
                    self.ensure_reg_owned(collection, pc, call_stack.last().map(|f| f.func));
                    self.reg(collection).remove_from(self.reg(value))?;
                    pc += 1;
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
                    use crate::interpreter::RuntimeValue;
                    // Struct field set via index syntax (`Set item "f" of s to
                    // v`) — VALUE semantics, mirroring the tree-walker: clone
                    // the struct, insert, write the new struct back.
                    let is_struct_text = matches!(
                        (
                            &*self.reg(collection).as_runtime(),
                            &*self.reg(index).as_runtime()
                        ),
                        (RuntimeValue::Struct(_), RuntimeValue::Text(_))
                    );
                    if is_struct_text {
                        let field = match &*self.reg(index).as_runtime() {
                            RuntimeValue::Text(t) => t.to_string(),
                            _ => unreachable!(),
                        };
                        let new_val = self.reg(value).as_runtime().clone();
                        let target = self.reg_mut(collection);
                        match target.as_runtime_mut() {
                            RuntimeValue::Struct(st) => {
                                st.fields.insert(field, new_val);
                            }
                            _ => unreachable!(),
                        }
                    } else {
                        let v = self.reg(value).clone();
                        self.ensure_reg_owned(collection, pc, call_stack.last().map(|f| f.func));
                        self.reg(collection).index_set(self.reg(index), v)?;
                    }
                    pc += 1;
                }
                // The bytecode interpreter checks both: a sound proof makes
                // the `IndexUnchecked` check never fire, so keeping it is
                // free defense-in-depth. Only the JIT elides.
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
                    let v = self.reg(collection).index_get(self.reg(index))?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Length { dst, collection } => {
                    let n = self.reg(collection).len()?;
                    self.set(dst, Value::int(n));
                    pc += 1;
                }
                // Pure metadata for the native region tier — the interpreter's
                // checked accesses make it a no-op (the hoist it enables only
                // applies inside a compiled region, verified at region entry).
                Op::RegionBoundsGuard { .. } => {
                    pc += 1;
                }
                Op::Contains {
                    dst,
                    collection,
                    value,
                } => {
                    let b = self.reg(collection).contains(self.reg(value))?;
                    self.set(dst, Value::bool(b));
                    pc += 1;
                }
                Op::ListPushField { obj, field, src } => {
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    let val = self.reg(src).as_runtime().clone();
                    // A `mutable` record parameter is its own copy too: what it holds reaches the
                    // caller through the call's copy-out, never through a list another copy shares.
                    if crate::semantics::collections::value_semantics_enabled() {
                        self.own_struct_field_list(
                            obj,
                            &field_name,
                            pc,
                            call_stack.last().map(|f| f.func),
                        );
                    }
                    crate::semantics::collections::push_to_struct_field(
                        &self.reg(obj).as_runtime(),
                        &field_name,
                        val,
                    )?;
                    pc += 1;
                }
                Op::CheckPolicy {
                    subject,
                    predicate,
                    is_capability,
                    object,
                    source_text,
                } => {
                    let (registry, interner) = match self.policy_ctx {
                        Some(ctx) => ctx,
                        None => {
                            return Err(crate::semantics::policy::NO_POLICIES.to_string());
                        }
                    };
                    let source = match &self.program.constants[source_text as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: check source is not Text: {:?}", other)),
                    };
                    let obj_val = if object != Reg::MAX {
                        Some(self.reg(object).as_runtime().clone())
                    } else {
                        None
                    };
                    crate::semantics::policy::check_policy(
                        registry,
                        interner,
                        &self.reg(subject).as_runtime(),
                        predicate,
                        is_capability,
                        obj_val.as_ref(),
                        &source,
                    )?;
                    pc += 1;
                }
                Op::FormatValue {
                    dst,
                    src,
                    spec,
                    debug_prefix,
                } => {
                    let mut out = String::new();
                    if debug_prefix != u32::MAX {
                        match &self.program.constants[debug_prefix as usize] {
                            Constant::Text(p) => {
                                out.push_str(p);
                                out.push('=');
                            }
                            other => {
                                return Err(format!("vm: debug prefix is not Text: {:?}", other));
                            }
                        }
                    }
                    if spec != u32::MAX {
                        let spec_s = match &self.program.constants[spec as usize] {
                            Constant::Text(s) => s.as_str(),
                            other => {
                                return Err(format!("vm: format spec is not Text: {:?}", other))
                            }
                        };
                        out.push_str(&crate::semantics::format::apply_format_spec(
                            &self.reg(src).as_runtime(),
                            spec_s,
                        ));
                    } else {
                        out.push_str(&self.reg(src).to_display_string());
                    }
                    self.set(dst, Value::text(out));
                    pc += 1;
                }
                Op::SliceOp {
                    dst,
                    collection,
                    start,
                    end,
                } => {
                    let v = crate::semantics::collections::slice(
                        &self.reg(collection).as_runtime(),
                        &self.reg(start).as_runtime(),
                        &self.reg(end).as_runtime(),
                    )?;
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::DeepClone { dst, src } => {
                    let v = self.reg(src).as_runtime().deep_clone();
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::NewTuple { dst, start, count } => {
                    use crate::interpreter::RuntimeValue;
                    let mut items = Vec::with_capacity(count as usize);
                    for k in 0..count {
                        items.push(self.reg(start + k).as_runtime().clone());
                    }
                    self.set(
                        dst,
                        Value::from_runtime(RuntimeValue::Tuple(std::rc::Rc::new(items))),
                    );
                    pc += 1;
                }
                Op::UnionOp { dst, lhs, rhs } => {
                    let v = crate::semantics::collections::union(
                        &self.reg(lhs).as_runtime(),
                        &self.reg(rhs).as_runtime(),
                    )?;
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::IntersectOp { dst, lhs, rhs } => {
                    let v = crate::semantics::collections::intersection(
                        &self.reg(lhs).as_runtime(),
                        &self.reg(rhs).as_runtime(),
                    )?;
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::LoadToday { dst } => {
                    let v = Value::from_runtime(crate::semantics::temporal::today());
                    if RECORDING {
                        self.record_nondet(dst, &v);
                    }
                    self.set(dst, v);
                    pc += 1;
                }
                Op::LoadNow { dst } => {
                    let v = Value::from_runtime(crate::semantics::temporal::now());
                    if RECORDING {
                        self.record_nondet(dst, &v);
                    }
                    self.set(dst, v);
                    pc += 1;
                }
                Op::NewStruct { dst, type_name } => {
                    use crate::interpreter::{RuntimeValue, StructValue};
                    let name = match &self.program.constants[type_name as usize] {
                        Constant::Text(s) => s.clone(),
                        other => {
                            return Err(format!("vm: NewStruct name is not Text: {:?}", other))
                        }
                    };
                    self.set(
                        dst,
                        Value::from_runtime(RuntimeValue::Struct(Box::new(StructValue {
                            type_name: name,
                            fields: std::collections::HashMap::new(),
                        }))),
                    );
                    pc += 1;
                }
                Op::StructInsert { obj, field, value } => {
                    use crate::interpreter::RuntimeValue;
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    let v = self.reg(value).as_runtime().clone();
                    match self.reg_mut(obj).as_runtime_mut() {
                        RuntimeValue::Struct(s) => {
                            s.fields.insert(field_name, v);
                        }
                        _ => return Err("Cannot set field on non-struct value".to_string()),
                    }
                    pc += 1;
                }
                Op::GetField { dst, obj, field } => {
                    use crate::interpreter::RuntimeValue;
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.as_str(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    let v = match &*self.reg(obj).as_runtime() {
                        RuntimeValue::Struct(s) => s
                            .fields
                            .get(field_name)
                            .cloned()
                            .ok_or_else(|| format!("Field '{}' not found", field_name))?,
                        other => {
                            return Err(format!("Cannot access field on {}", other.type_name()));
                        }
                    };
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::NewInductive {
                    dst,
                    type_name,
                    ctor,
                    args_start,
                    count,
                } => {
                    use crate::interpreter::{InductiveValue, RuntimeValue};
                    let inductive_type = match &self.program.constants[type_name as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: enum name is not Text: {:?}", other)),
                    };
                    let constructor = match &self.program.constants[ctor as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: variant name is not Text: {:?}", other)),
                    };
                    let mut args = Vec::with_capacity(count as usize);
                    for k in 0..count {
                        args.push(self.reg(args_start + k).as_runtime().clone());
                    }
                    self.set(
                        dst,
                        Value::from_runtime(RuntimeValue::Inductive(std::rc::Rc::new(
                            InductiveValue {
                                inductive_type,
                                constructor,
                                args,
                            },
                        ))),
                    );
                    pc += 1;
                }
                Op::TestArm {
                    dst,
                    target,
                    variant,
                } => {
                    use crate::interpreter::RuntimeValue;
                    let variant_name = match &self.program.constants[variant as usize] {
                        Constant::Text(s) => s.as_str(),
                        other => return Err(format!("vm: variant name is not Text: {:?}", other)),
                    };
                    let matched = match &*self.reg(target).as_runtime() {
                        // A struct matches its own variant name — and ALSO a `Some` arm, because
                        // the Option encoding is transparent: `some(a new Response …)` IS the
                        // Response. Without the second half, `Option of <any record>` could not be
                        // inspected at all while `Option of Int` worked, since a scalar falls to
                        // the catch-all below. The plan's own middleware signature is
                        // `-> Option of Response`, so the gap was load-bearing.
                        // ★ A STRUCT MATCHES ITS OWN VARIANT NAME, and only that.
                        //
                        // This used to read `|| variant_name == "Some"`, because an Option was
                        // TRANSPARENT: `some (a new Response …)` WAS the Response, so a `Some` arm
                        // had to match a bare struct or `Option of <record>` could not be inspected
                        // at all. An Option now carries its own tag and arrives as an
                        // `Inductive("Option", "Some")`, matched by the arm below — so this clause
                        // no longer covers a real case, and keeping it would make a plain record
                        // match a `Some` arm it has nothing to do with.
                        RuntimeValue::Struct(s) => s.type_name == variant_name,
                        RuntimeValue::Inductive(ind) => ind.constructor == variant_name,
                        // The transparent Option encoding: `some x` IS x and
                        // `none` IS Nothing, so Some/None arms match presence/
                        // absence for non-enum values (the boxed `a new Some …`
                        // form matches through the arms above).
                        // A BARE `Nothing` still answers a `None` arm. It is not an Option — it
                        // is what `Try to receive` and an over-`Pop` produce — but absence is one
                        // thing however it arrived, which is the same rule R2 gives equality.
                        RuntimeValue::Nothing => variant_name == "None",
                        // ★ AND THE TRANSPARENT CATCH-ALL IS GONE. It read `_ => variant_name ==
                        // "Some"`: any value that was not the sentinel answered a `Some` arm. That
                        // was the encoding's whole presence rule, and it is now actively wrong —
                        // a bare `7` is an Int, not a present Option, and saying otherwise is how
                        // `some 0` and `none` became indistinguishable in the first place.
                        _ => false,
                    };
                    self.set(dst, Value::bool(matched));
                    pc += 1;
                }
                Op::BindArm {
                    dst,
                    target,
                    field,
                    index,
                } => {
                    use crate::interpreter::RuntimeValue;
                    let v = match &*self.reg(target).as_runtime() {
                        RuntimeValue::Struct(st) => {
                            let field_name = match &self.program.constants[field as usize] {
                                Constant::Text(s) => s.as_str(),
                                other => {
                                    return Err(format!("vm: field name is not Text: {:?}", other));
                                }
                            };
                            // A real struct arm binds a FIELD. A `Some` arm over a struct binds
                            // the WHOLE value, and the compiler says which by emitting a reserved
                            // marker no identifier can spell instead of a field name.
                            //
                            // "any field miss binds the whole value" was the first version of this
                            // and it was WRONG: `When Circle (r)` — a real struct arm naming a
                            // field that does not exist — errors on the tree-walker and would have
                            // silently bound the whole Circle here, a tier divergence introduced
                            // by the fix for a different one.
                            if field_name == crate::vm::SOME_BINDING_MARKER {
                                Some(self.reg(target).as_runtime().as_ref().clone())
                            } else {
                                match st.fields.get(field_name) {
                                    Some(v) => Some(v.clone()),
                                    // A record arm naming a field the record does not have. The
                                    // tree-walker leaves the name unbound and fails at its first
                                    // USE ("Undefined variable: r"); this tier had no unbound
                                    // register, so it read whatever the slot held and printed it —
                                    // a silent wrong answer where the reference tier errors. In
                                    // the source form `When Circle (r)` the binding name IS the
                                    // field name, so the same words are available here.
                                    None => {
                                        return Err(format!("Undefined variable: {field_name}"));
                                    }
                                }
                            }
                        }
                        RuntimeValue::Inductive(ind) => ind.args.get(index as usize).cloned(),
                        // Transparent `some x`: the matched Some arm's single
                        // binding receives the value itself (BindArm only runs
                        // after this arm's TestArm accepted the target).
                        RuntimeValue::Nothing => None,
                        other => (index == 0).then(|| other.clone()),
                    };
                    if let Some(v) = v {
                        self.set(dst, Value::from_runtime(v));
                    }
                    pc += 1;
                }
                Op::CrdtBump {
                    obj,
                    field,
                    amount,
                    negate,
                } => {
                    use crate::interpreter::RuntimeValue;
                    let amount_int = match &*self.reg(amount).as_runtime() {
                        RuntimeValue::Int(n) => *n,
                        _ => {
                            return Err(if negate {
                                "CRDT decrement amount must be an integer".to_string()
                            } else {
                                "CRDT increment amount must be an integer".to_string()
                            });
                        }
                    };
                    let amount_int = if negate {
                        amount_int.wrapping_neg()
                    } else {
                        amount_int
                    };
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    match self.reg_mut(obj).as_runtime_mut() {
                        RuntimeValue::Struct(s) => {
                            let current = s
                                .fields
                                .get(&field_name)
                                .cloned()
                                .unwrap_or(RuntimeValue::Int(0));
                            let new_val = crate::semantics::arith::crdt_counter_bump(
                                current,
                                amount_int,
                                &field_name,
                            )?;
                            s.fields.insert(field_name, new_val);
                        }
                        _ => {
                            return Err(if negate {
                                "Cannot decrease field on non-struct value".to_string()
                            } else {
                                "Cannot increase field on non-struct value".to_string()
                            });
                        }
                    }
                    pc += 1;
                }
                Op::CrdtMerge { target, source } => {
                    use crate::interpreter::RuntimeValue;
                    let source_fields = match &*self.reg(source).as_runtime() {
                        RuntimeValue::Struct(s) => s.fields.clone(),
                        _ => return Err("Merge source must be a struct".to_string()),
                    };
                    match self.reg_mut(target).as_runtime_mut() {
                        RuntimeValue::Struct(s) => {
                            for (field_name, incoming) in source_fields {
                                let current = s
                                    .fields
                                    .get(&field_name)
                                    .cloned()
                                    .unwrap_or(RuntimeValue::Int(0));
                                let merged =
                                    crate::semantics::arith::crdt_merge_field(&current, incoming);
                                s.fields.insert(field_name, merged);
                            }
                        }
                        _ => return Err("Merge target must be a struct".to_string()),
                    }
                    pc += 1;
                }
                Op::CrdtJoin { target, source } => {
                    // The state-based G-Counter LATTICE JOIN: per-replica MAX (idempotent), the
                    // fixed-lane join the `crdt_merge` hardware block computes. Unlike CrdtMerge's
                    // polymorphic field-wise merge (ADD for bare-int counters), this is unconditional
                    // per-field MAX — so a redelivered/duplicated counter state never double-counts.
                    use crate::interpreter::RuntimeValue;
                    let source_fields = match &*self.reg(source).as_runtime() {
                        RuntimeValue::Struct(s) => s.fields.clone(),
                        _ => return Err("Join source must be a struct".to_string()),
                    };
                    match self.reg_mut(target).as_runtime_mut() {
                        RuntimeValue::Struct(s) => {
                            for (field_name, incoming) in source_fields {
                                let current = s
                                    .fields
                                    .get(&field_name)
                                    .cloned()
                                    .unwrap_or(RuntimeValue::Int(0));
                                let joined =
                                    crate::semantics::arith::crdt_join_field(&current, incoming);
                                s.fields.insert(field_name, joined);
                            }
                        }
                        _ => return Err("Join target must be a struct".to_string()),
                    }
                    pc += 1;
                }
                Op::BigIntWide { kind, dst, a, b } => {
                    // The 256-bit BigInt lane. On the soft-core the operands live in an 8×256-bit wide
                    // register bank routed to bigint_alu/div/mul; in the VM they live in the register
                    // file as BigInt (arbitrary precision), so the same arithmetic reuses Value's ops.
                    use crate::interpreter::RuntimeValue;
                    match kind {
                        0 => {
                            let v = self.reg(a).clone();
                            *self.reg_mut(dst) = v;
                        } // FromInt: R[dst] = R[a]
                        1 => self.binop(dst, a, b, Value::add)?, // Add
                        2 => self.binop(dst, a, b, Value::sub)?, // Sub
                        3 => self.binop(dst, a, b, Value::lt)?,  // Cmp: R[dst] = R[a] < R[b]
                        4 => {
                            // Low: R[dst] = R[a] mod 2^32
                            let lo = match &*self.reg(a).as_runtime() {
                                RuntimeValue::Int(n) => (*n as u32) as i64,
                                _ => 0,
                            };
                            *self.reg_mut(dst) = Value::int(lo);
                        }
                        5 => self.binop(dst, a, b, Value::div)?, // Div
                        6 => self.binop(dst, a, b, Value::mul)?, // Mul
                        _ => {}
                    }
                    pc += 1;
                }
                Op::NewCrdt { dst, kind } => {
                    use crate::interpreter::RuntimeValue;
                    let cv = crate::semantics::crdt::CrdtKind::from_new_crdt_code(kind)
                        .ok_or_else(|| format!("vm: unknown CRDT kind {kind}"))?
                        .fresh();
                    let v = Value::from_runtime(RuntimeValue::Crdt(std::rc::Rc::new(
                        std::cell::RefCell::new(cv),
                    )));
                    // The replica id was drawn from a global counter — captured nondeterminism (the
                    // same treatment as a clock read): reverse restores dst, leaving the counter drawn.
                    if RECORDING {
                        self.record_nondet(dst, &v);
                    }
                    self.set(dst, v);
                    pc += 1;
                }
                Op::CrdtAppend { seq, value } => {
                    use crate::interpreter::RuntimeValue;
                    let v = self.reg(value).as_runtime().clone();
                    let seq_rt = self.reg(seq).as_runtime();
                    match &*seq_rt {
                        RuntimeValue::Crdt(rc) => rc.borrow_mut().append(&v)?,
                        RuntimeValue::List(_) => {
                            crate::semantics::collections::list_push(&seq_rt, v)?
                        }
                        other => return Err(format!("Cannot append to {}", other.type_name())),
                    }
                    pc += 1;
                }
                Op::CrdtResolve { obj, field, value } => {
                    use crate::interpreter::RuntimeValue;
                    let v = self.reg(value).as_runtime().clone();
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    match self.reg_mut(obj).as_runtime_mut() {
                        RuntimeValue::Struct(s) => {
                            // A real divergent register resolves in place via its shared
                            // `Rc`; a plain field is overwritten — same fallback as the
                            // tree-walker's `Resolve`.
                            let is_register =
                                matches!(s.fields.get(&field_name), Some(RuntimeValue::Crdt(_)));
                            if is_register {
                                if let Some(RuntimeValue::Crdt(rc)) = s.fields.get(&field_name) {
                                    rc.borrow_mut().resolve(&v)?;
                                }
                            } else {
                                s.fields.insert(field_name, v);
                            }
                        }
                        other => {
                            return Err(format!("Cannot resolve a field on {}", other.type_name()))
                        }
                    }
                    pc += 1;
                }
                Op::IterPrepare { iterable } => {
                    let items = crate::semantics::collections::iteration_snapshot(
                        &self.reg(iterable).as_runtime(),
                    )?;
                    self.iter_stack
                        .push((items.into_iter().map(Value::from_runtime).collect(), 0));
                    pc += 1;
                }
                Op::IterNext { dst, exit } => {
                    let (items, idx) = self
                        .iter_stack
                        .last_mut()
                        .ok_or("vm: IterNext with no live iterator")?;
                    if *idx < items.len() {
                        let v = items[*idx].clone();
                        *idx += 1;
                        // (the iter_stack borrow ends above via NLL — recording is dead-stripped when
                        // `RECORDING = false`, so the hot loop is byte-for-byte unchanged.)
                        if RECORDING {
                            let dst_slot = self.base + dst as usize;
                            let dst_old = self.registers[dst_slot].clone();
                            if let Some(j) = self.journal.as_mut() {
                                j.push(super::journal::JournalEntry::IterNextUndo {
                                    advanced: true,
                                    dst_slot,
                                    dst_old,
                                });
                            }
                        }
                        self.set(dst, v);
                        pc += 1;
                    } else {
                        if RECORDING {
                            if let Some(j) = self.journal.as_mut() {
                                j.push(super::journal::JournalEntry::IterNextUndo {
                                    advanced: false,
                                    dst_slot: 0,
                                    dst_old: Value::nothing(),
                                });
                            }
                        }
                        pc = exit;
                    }
                }
                Op::IterPop => {
                    if RECORDING {
                        if let Some((items, idx)) = self.iter_stack.last().cloned() {
                            if let Some(j) = self.journal.as_mut() {
                                j.push(super::journal::JournalEntry::IterPopUndo { items, idx });
                            }
                        }
                    }
                    self.iter_stack
                        .pop()
                        .ok_or("vm: IterPop with no live iterator")?;
                    pc += 1;
                }
                Op::ListPop { list, dst } => {
                    self.ensure_reg_owned(list, pc, call_stack.last().map(|f| f.func));
                    let v = crate::semantics::collections::list_pop(&self.reg(list).as_runtime())?;
                    self.set(dst, Value::from_runtime(v));
                    pc += 1;
                }
                Op::Sleep { duration } => {
                    // A VM `Sleep` only ever runs inside a task (a non-concurrent program
                    // with `Sleep` routes to the async tree-walker, never the VM). Route it
                    // through the scheduler's virtual timer — the same logical-tick scale as
                    // a `Select` `After` arm — by yielding `VmBlock::Sleep`. Blocking on a
                    // real host timer here would stall the cooperative scheduler (and errors
                    // outright on wasm).
                    let ticks = self.as_ticks(duration)?;
                    if ticks > 0 {
                        return Ok(self.block(pc + 1, call_stack, VmBlock::Sleep(ticks), None));
                    }
                    pc += 1;
                }
                Op::DestructureTuple { src, start, count } => {
                    use crate::interpreter::RuntimeValue;
                    match &*self.reg(src).as_runtime() {
                        RuntimeValue::Tuple(items) => {
                            // Arity is LOUD — a silent truncation binds ghosts.
                            if items.len() != count as usize {
                                return Err(format!(
                                    "Cannot bind a {}-tuple to {} names",
                                    items.len(),
                                    count
                                ));
                            }
                            let items: Vec<Value> = items
                                .iter()
                                .take(count as usize)
                                .cloned()
                                .map(Value::from_runtime)
                                .collect();
                            for (i, v) in items.into_iter().enumerate() {
                                self.set(start + i as Reg, v);
                            }
                        }
                        other => {
                            return Err(format!(
                                "Expected tuple for pattern, got {}",
                                other.type_name()
                            ));
                        }
                    }
                    pc += 1;
                }
                Op::Show { src } => {
                    self.lines.push(self.reg(src).to_display_string());
                    pc += 1;
                }
                Op::Obs {
                    site,
                    args_start,
                    args_count,
                } => {
                    // Wave 13 — emit the observation event the OBSERVED-profile
                    // compiler recorded. No-op when no sink is capturing, so the
                    // op costs a single branch off the hot path.
                    if crate::observe::capturing() {
                        let desc = &self.program.observation_sites[site as usize];
                        match &desc.kind {
                            ObsSiteKind::SpanEnter => {
                                crate::observe::enter_span(desc.message.clone())
                            }
                            ObsSiteKind::SpanExit => {
                                crate::observe::exit_span(desc.message.clone())
                            }
                            ObsSiteKind::Message(cat) => {
                                let mut vals = Vec::with_capacity(args_count as usize);
                                for i in 0..args_count {
                                    let v = self.reg(args_start + i).to_display_string();
                                    vals.push((desc.payload_names[i as usize].clone(), v));
                                }
                                crate::observe::emit(
                                    *cat,
                                    desc.message.clone(),
                                    vals,
                                    desc.sample,
                                    site as u64,
                                );
                            }
                        }
                    }
                    pc += 1;
                }
                Op::Args { dst } => {
                    use crate::interpreter::RuntimeValue;
                    let items: Vec<RuntimeValue> = self
                        .program_args
                        .iter()
                        .map(|s| RuntimeValue::Text(std::rc::Rc::new(s.clone())))
                        .collect();
                    let list = RuntimeValue::List(std::rc::Rc::new(std::cell::RefCell::new(
                        crate::interpreter::ListRepr::from_values(items),
                    )));
                    self.set(dst, Value::from_runtime(list));
                    pc += 1;
                }
                // Go-like concurrency (T11). Each op materialises its operands,
                // then suspends the slice — `self.block` saves the resume point
                // and the request; the scheduler driver services it and re-enters.
                Op::ChanNew { dst, cap, .. } => {
                    let capacity = if cap < 0 { None } else { Some(cap as usize) };
                    return Ok(self.block(
                        pc + 1,
                        call_stack,
                        VmBlock::NewChan(capacity),
                        Some(dst),
                    ));
                }
                Op::ChanSend { chan, val } => {
                    let ch = self.as_chan(chan)?;
                    let payload = self.materialize_reg(val)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::Send(ch, payload), None));
                }
                Op::ChanRecv { dst, chan } => {
                    let ch = self.as_chan(chan)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::Recv(ch), Some(dst)));
                }
                Op::ChanTrySend { dst, chan, val } => {
                    let ch = self.as_chan(chan)?;
                    let payload = self.materialize_reg(val)?;
                    return Ok(self.block(
                        pc + 1,
                        call_stack,
                        VmBlock::TrySend(ch, payload),
                        Some(dst),
                    ));
                }
                Op::ChanTryRecv { dst, chan } => {
                    let ch = self.as_chan(chan)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::TryRecv(ch), Some(dst)));
                }
                Op::ChanClose { chan } => {
                    let ch = self.as_chan(chan)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::Close(ch), None));
                }
                Op::TaskAwait { dst, handle } => {
                    let tid = self.as_task(handle)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::Await(tid), Some(dst)));
                }
                Op::TaskAbort { handle } => {
                    let tid = self.as_task(handle)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::Abort(tid), None));
                }
                Op::Spawn {
                    func,
                    args_start,
                    arg_count,
                } => {
                    let args = self.materialize_args(args_start, arg_count)?;
                    let req = VmBlock::SpawnDesc {
                        func,
                        args,
                        want_handle: false,
                    };
                    return Ok(self.block(pc + 1, call_stack, req, None));
                }
                Op::SpawnHandle {
                    dst,
                    func,
                    args_start,
                    arg_count,
                } => {
                    let args = self.materialize_args(args_start, arg_count)?;
                    let req = VmBlock::SpawnDesc {
                        func,
                        args,
                        want_handle: true,
                    };
                    return Ok(self.block(pc + 1, call_stack, req, Some(dst)));
                }
                Op::SelectArmRecv { chan, var } => {
                    let ch = self.as_chan(chan)?;
                    self.select_pending.push((SelectArm::Recv(ch), Some(var)));
                    pc += 1;
                }
                Op::SelectArmTimeout { ticks } => {
                    let t = self.as_ticks(ticks)?;
                    self.select_pending.push((SelectArm::Timeout(t), None));
                    pc += 1;
                }
                Op::SelectWait { dst_arm } => {
                    let arms: Vec<SelectArm> =
                        self.select_pending.iter().map(|(a, _)| a.clone()).collect();
                    return Ok(self.block(
                        pc + 1,
                        call_stack,
                        VmBlock::Select(arms),
                        Some(dst_arm),
                    ));
                }
                // Peer networking: materialise the operands and suspend; the async VM driver
                // services the request through the shared `NetInbox` (the same inbox the
                // tree-walker uses) and resumes — `NetAwait` resumes with the received value.
                Op::NetConnect { url } => {
                    let u = self.materialize_reg(url)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::NetConnect(u), None));
                }
                Op::NetListen { topic } => {
                    let t = self.materialize_reg(topic)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::NetListen(t), None));
                }
                Op::NetSend { to, msg } => {
                    let t = self.materialize_reg(to)?;
                    let m = self.materialize_reg(msg)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::NetSend(t, m), None));
                }
                Op::NetStream { to, values } => {
                    let t = self.materialize_reg(to)?;
                    let v = self.materialize_reg(values)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::NetStream(t, v), None));
                }
                Op::NetAwait { dst, from, stream } => {
                    let f = self.materialize_reg(from)?;
                    return Ok(self.block(
                        pc + 1,
                        call_stack,
                        VmBlock::NetAwait(f, stream),
                        Some(dst),
                    ));
                }
                Op::NetMakePeer { dst, addr } => {
                    let a = self.materialize_reg(addr)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::NetMakePeer(a), Some(dst)));
                }
                Op::NetSync { dst, topic } => {
                    let current = self.materialize_reg(dst)?;
                    let t = self.materialize_reg(topic)?;
                    return Ok(self.block(
                        pc + 1,
                        call_stack,
                        VmBlock::NetSync(t, current),
                        Some(dst),
                    ));
                }
                // tcp.lg natives: materialise the operands and suspend; the async io
                // driver services the request through the SAME `TcpRegistry::exec_values`
                // the tree-walker calls, so the tiers cannot drift.
                Op::Tcp {
                    dst,
                    kind,
                    args_start,
                    arg_count,
                } => {
                    let args = self.materialize_args(args_start, arg_count)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::Tcp(kind, args), Some(dst)));
                }
                // `Serve` on the async io driver: read the port and suspend. The
                // driver owns the accept loop and re-enters `handler` per
                // connection as a fresh child VM; the loop never returns, so no
                // resume slot is reserved.
                Op::Serve { port, handler } => {
                    let port = match &*self.reg(port).as_runtime() {
                        crate::interpreter::RuntimeValue::Int(n) if (0..=65535).contains(n) => {
                            *n as u16
                        }
                        crate::interpreter::RuntimeValue::Int(n) => {
                            return Err(format!("Serve: port {n} is out of range (0-65535)"))
                        }
                        other => {
                            return Err(format!(
                                "Serve: port must be an Int, got {}",
                                other.type_name()
                            ))
                        }
                    };
                    return Ok(self.block(pc + 1, call_stack, VmBlock::Serve(port, handler), None));
                }
                Op::FailWith { msg } => {
                    return Err(match &self.program.constants[msg as usize] {
                        Constant::Text(s) => s.clone(),
                        other => format!("vm: FailWith constant is not Text: {:?}", other),
                    });
                }
                Op::Halt => break,
            }
        }
        Ok(VmStep::Done(crate::interpreter::RuntimeValue::Nothing))
    }

    /// Enter function `func` with its arguments in registers `args_start..args_start +
    /// arg_count` of the current frame, answering the pc to continue at.
    ///
    /// A call that completes without a bytecode frame — the pbkdf2 accelerator, the native tier,
    /// the WASM-JIT tier — has already written its result into `dst` and answers `return_pc`.
    /// A precise deopt answers the pc the materialized frames resume at. Otherwise the callee's
    /// frame is pushed (warm body first, then baseline) and its entry pc is answered. The one call
    /// path behind `Op::Call` and [`Vm::invoke_function`].
    #[inline(always)]
    fn enter_call<const RECORDING: bool>(
        &mut self,
        dst: Reg,
        func: FuncIdx,
        args_start: Reg,
        arg_count: u16,
        return_pc: usize,
        call_stack: &mut Vec<CallFrame>,
    ) -> Result<usize, String> {
        // Native-accelerate crypto.lg's pbkdf2Sha256 — byte-identical to the pure body
        // (crypto_accel_lock.rs), but ~16k interpreted SHA-256 compressions is untenable as
        // bytecode for a live SCRAM handshake. Skipped under reverse recording so the pure path
        // stays reversible.
        if !RECORDING && Some(func) == self.accel_pbkdf2 && arg_count == 3 {
            let a0 = self.reg(args_start).as_runtime().clone();
            let a1 = self.reg(args_start + 1).as_runtime().clone();
            let a2 = self.reg(args_start + 2).as_runtime().clone();
            let (pw, salt, iters) = crate::semantics::builtins::pbkdf2_arguments(&a0, &a1, &a2)?;
            let dk = crate::crypto_accel::pbkdf2_sha256(&pw, &salt, iters);
            self.set(
                dst,
                Value::from_runtime(crate::semantics::builtins::bytes_to_seq(&dk)),
            );
            self.clear_args(args_start, arg_count, Some(func));
            return Ok(return_pc);
        }
        if call_stack.len() >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        // Reverse recording covers the interpreter frame teardown only; a native/deopt call has
        // effects the frame journal doesn't capture, so decline rather than mis-reverse. (A fresh
        // recording VM has `tier: None`, so this never fires there.)
        if RECORDING && self.tier.is_some() {
            return Err("vm: reverse recording does not support the native tier".to_string());
        }
        match self.try_native(func, args_start, arg_count, call_stack.len()) {
            NativeDisposition::Done(v) => {
                self.set(dst, v);
                self.clear_args(args_start, arg_count, Some(func));
                return Ok(return_pc);
            }
            NativeDisposition::Interpret => {}
            // Precise deopt: every native frame becomes a real CallFrame (registers re-boxed per
            // the adapter's kind capture), then the interpreter resumes AT the faulting op —
            // effects stay, the kernel raises the exact error from the exact state.
            NativeDisposition::Materialize {
                resume_pc,
                frames,
                list_args,
            } => {
                self.materialize_native_frames(
                    &frames, &list_args, args_start, dst, func, return_pc, call_stack,
                )?;
                return Ok(resume_pc);
            }
        }
        // WS6 (Phase 13): the WASM-JIT tier — the browser JIT path, consulted here (behind the
        // native forge tier above, which is absent on wasm32). A hot pure-integer function runs
        // its emitted WebAssembly module instead of the bytecode; non-Int args or an ineligible
        // body fall through.
        #[cfg(feature = "wasm-jit")]
        {
            let prog = self.program;
            let abase = self.base + args_start as usize;
            let mut ints: Vec<i64> = Vec::with_capacity(arg_count as usize);
            let mut all_int = true;
            for k in 0..arg_count as usize {
                match &*self.registers[abase + k].as_runtime() {
                    crate::interpreter::RuntimeValue::Int(n) => ints.push(*n),
                    _ => {
                        all_int = false;
                        break;
                    }
                }
            }
            if all_int {
                if let Some(r) = self.wasm_tier.on_call(prog, func, &ints) {
                    self.set(dst, Value::int(r));
                    return Ok(return_pc);
                }
            }
        }
        // Dispatch order (HOTSWAP §7): FnTable native (above) → warm_bytecode → baseline. A warm
        // body runs from the unified pc space past `program.code`.
        let (entry_pc, reg_count) = match self.warm_entry.get(func as usize).and_then(|w| *w) {
            Some(w) => (w.entry_pc, w.register_count),
            None => {
                let f = self
                    .program
                    .functions
                    .get(func as usize)
                    .ok_or("vm: call to undefined function index")?;
                (f.entry_pc, f.register_count)
            }
        };
        let callee_base = self.base + args_start as usize;
        let restore_len = self.registers.len();
        let needed = callee_base + reg_count;
        if needed > MAX_REGISTER_FILE {
            return Err("vm: register file limit exceeded".to_string());
        }
        if self.registers.len() < needed {
            self.registers.resize(needed, Value::nothing());
        }
        if RECORDING {
            if let Some(j) = self.journal.as_mut() {
                j.push(super::journal::JournalEntry::CallUndo { restore_len });
            }
        }
        call_stack.push(CallFrame {
            return_pc,
            return_reg: dst,
            caller_base: self.base,
            restore_len,
            iter_depth: self.iter_stack.len(),
            func,
            arg_lo: callee_base,
            arg_count,
        });
        self.base = callee_base;
        Ok(entry_pc)
    }

    /// Enter `closure` with its arguments in registers `args_start..args_start + arg_count` of
    /// the current frame: push its frame, bind its captures, and answer its entry pc. The one
    /// closure-call path behind `Op::CallValue` and [`Vm::call_closure`].
    #[inline(always)]
    fn enter_closure<const RECORDING: bool>(
        &mut self,
        dst: Reg,
        closure: &crate::interpreter::ClosureValue,
        args_start: Reg,
        arg_count: u16,
        return_pc: usize,
        call_stack: &mut Vec<CallFrame>,
    ) -> Result<usize, String> {
        if call_stack.len() >= crate::semantics::MAX_CALL_DEPTH {
            return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
        }
        let f = self
            .program
            .functions
            .get(closure.body_index)
            .ok_or("vm: CallValue on undefined function index")?;
        if arg_count as usize != f.param_count as usize {
            return Err(format!(
                "Closure expects {} arguments, got {}",
                f.param_count, arg_count
            ));
        }
        let entry_pc = f.entry_pc;
        let reg_count = f.register_count;
        let captures = f.captures.clone();
        let param_count = f.param_count;

        let callee_base = self.base + args_start as usize;
        let restore_len = self.registers.len();
        let needed = callee_base + reg_count;
        if needed > MAX_REGISTER_FILE {
            return Err("vm: register file limit exceeded".to_string());
        }
        if self.registers.len() < needed {
            self.registers.resize(needed, Value::nothing());
        }
        call_stack.push(CallFrame {
            return_pc,
            return_reg: dst,
            caller_base: self.base,
            restore_len,
            iter_depth: self.iter_stack.len(),
            func: closure.body_index as u16,
            arg_lo: callee_base,
            arg_count,
        });
        self.base = callee_base;
        // Bind captures: value slots then present flags — both deep-cloned PER CALL (the
        // tree-walker re-clones each invocation).
        let cap_count = captures.len() as Reg;
        if RECORDING {
            let mut caps = Vec::with_capacity(captures.len() * 2);
            for k in 0..cap_count {
                let vslot = self.base + (param_count + k) as usize;
                let pslot = self.base + (param_count + cap_count + k) as usize;
                caps.push((vslot, self.registers[vslot].clone()));
                caps.push((pslot, self.registers[pslot].clone()));
            }
            if let Some(j) = self.journal.as_mut() {
                j.push(super::journal::JournalEntry::CallValueUndo {
                    restore_len,
                    captures: caps,
                });
            }
        }
        for (k, (sym, _)) in captures.iter().enumerate() {
            let (v, present) = match closure.captured_env.get(sym) {
                Some(v) => (Value::from_runtime(v.deep_clone()), true),
                None => (Value::nothing(), false),
            };
            self.set(param_count + k as Reg, v);
            self.set(param_count + cap_count + k as Reg, Value::bool(present));
        }
        Ok(entry_pc)
    }

    #[inline]
    fn reg(&self, r: Reg) -> &Value {
        &self.registers[self.base + r as usize]
    }

    #[inline]
    fn set(&mut self, r: Reg, v: Value) {
        let slot = self.base + r as usize;
        self.registers[slot] = v;
    }

    #[inline]
    fn reg_mut(&mut self, r: Reg) -> &mut Value {
        let slot = self.base + r as usize;
        &mut self.registers[slot]
    }

    // ─── Scheduler-driver hooks (T11) ───────────────────────────────────────

    /// Save the suspended `pc` + call stack and the request; the slice returns
    /// [`VmStep::Blocked`]. `slot` is the register the resume value lands in.
    fn block(
        &mut self,
        resume_pc: usize,
        call_stack: Vec<CallFrame>,
        req: VmBlock,
        slot: Option<Reg>,
    ) -> VmStep {
        self.sched_pc = resume_pc;
        self.sched_call_stack = call_stack;
        self.sched_active = true;
        self.resume_wraps_option = matches!(req, VmBlock::TryRecv(_));
        self.pending = Some(req);
        self.resume_slot = slot;
        VmStep::Blocked
    }

    // ─── Debug-drawer hooks (single-task, bytecode tier) ─────────────────────

    /// Build a [`DebugView`] of the paused VM: the call frames with their register
    /// values, the globals, the output, and the program counter (the op about to
    /// execute). Reconstructs each frame's register window from the saved call
    /// stack — Main has base 0; frame `k`'s base is the next frame's `caller_base`
    /// (the innermost is the live `self.base`).
    pub(crate) fn debug_view(&self) -> DebugView {
        let prog = self.program;
        let frames = self
            .active_frames()
            .into_iter()
            .map(|(func, base, count)| self.frame_view(func, base, count))
            .collect();
        let globals = prog
            .globals
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                self.globals
                    .get(i)
                    .and_then(|o| o.as_ref())
                    .map(|v| (name.clone(), v.to_display_string()))
            })
            .collect();
        DebugView {
            pc: self.sched_pc,
            frames,
            globals,
            output: self.lines.clone(),
        }
    }

    /// The live call frames, Main first: each one's function (`None` for Main), the base
    /// of its registers in the register file, and how many it has.
    fn active_frames(&self) -> Vec<(Option<u16>, usize, usize)> {
        let prog = self.program;
        let cs = &self.sched_call_stack;
        let mut frames = Vec::with_capacity(cs.len() + 1);
        frames.push((None, 0, prog.register_count));
        for (k, fr) in cs.iter().enumerate() {
            let base = cs.get(k + 1).map(|n| n.caller_base).unwrap_or(self.base);
            let count = prog
                .functions
                .get(fr.func as usize)
                .map(|f| f.register_count)
                .unwrap_or(0);
            frames.push((Some(fr.func), base, count));
        }
        frames
    }

    fn frame_view(&self, func: Option<u16>, base: usize, count: usize) -> DebugFrameView {
        let registers = (0..count)
            .filter_map(|i| {
                self.registers.get(base + i).map(|v| {
                    // `as_runtime` (not `as_runtime_ref`) so inline scalars report their
                    // type too — under the narrow-value repr `as_runtime_ref` is `None` for
                    // an inline Int/Float/Bool, but the type is still well-defined.
                    let kind = v.as_runtime().type_name().to_string();
                    (i as u16, kind, v.to_display_string())
                })
            })
            .collect();
        DebugFrameView {
            func,
            base,
            registers,
        }
    }

    /// Walk the roots (the current frame's registers + the globals) and collect the
    /// distinct heap objects they reach, recording each object's reference count and
    /// every root that points at it — so a shared list shows up once with two
    /// referrers (the `let b be a` aliasing that trips up every beginner).
    pub(crate) fn debug_heap(&self) -> Vec<HeapObjView> {
        let prog = self.program;
        let (count, is_main) = match self.sched_call_stack.last() {
            Some(fr) => (
                prog.functions
                    .get(fr.func as usize)
                    .map(|f| f.register_count)
                    .unwrap_or(0),
                false,
            ),
            None => (prog.register_count, true),
        };
        let name_of = |i: usize| -> String {
            if is_main {
                prog.reg_names
                    .iter()
                    .find(|(r, _)| *r as usize == i)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| format!("R{i}"))
            } else {
                format!("R{i}")
            }
        };
        // The program's own handles: every live frame's registers, the globals, and the
        // values loops are iterating — not the copies the debugger keeps for rewinding.
        let frames = self.active_frames();
        let handles = count_handles(
            frames
                .iter()
                .flat_map(|&(_, base, count)| self.registers.iter().skip(base).take(count))
                .chain(self.globals.iter().flatten())
                .chain(self.iter_stack.iter().flat_map(|(items, _)| items.iter()))
                .filter_map(|v| v.as_runtime_ref()),
        );
        let mut objs: Vec<HeapObjView> = Vec::new();
        let add = |v: &Value, root: String, objs: &mut Vec<HeapObjView>| {
            if let Some((id, kind, storage)) = heap_identity(v) {
                let rc = handles.get(&id).copied().unwrap_or(1);
                match objs.iter_mut().find(|o| o.id == id) {
                    Some(o) => {
                        if !o.referenced_by.contains(&root) {
                            o.referenced_by.push(root);
                        }
                    }
                    None => objs.push(HeapObjView {
                        id,
                        kind,
                        summary: v.to_display_string(),
                        storage,
                        rc,
                        referenced_by: vec![root],
                    }),
                }
            }
        };
        for i in 0..count {
            if let Some(v) = self.registers.get(self.base + i) {
                add(v, name_of(i), &mut objs);
            }
        }
        for (i, name) in prog.globals.iter().enumerate() {
            if let Some(Some(v)) = self.globals.get(i) {
                add(v, name.clone(), &mut objs);
            }
        }
        objs
    }

    /// Clone the resumable execution state so the debugger can carry it between
    /// steps (it rebuilds the VM each step — see [`DebugVmState`]).
    pub(crate) fn save_debug_state(&self) -> DebugVmState {
        DebugVmState {
            registers: self.registers.clone(),
            base: self.base,
            globals: self.globals.clone(),
            lines: self.lines.clone(),
            iter_stack: self.iter_stack.clone(),
            sched_active: self.sched_active,
            sched_pc: self.sched_pc,
            sched_call_stack: self.sched_call_stack.clone(),
        }
    }

    /// Restore a snapshot taken by [`Vm::save_debug_state`].
    pub(crate) fn restore_debug_state(&mut self, st: DebugVmState) {
        self.registers = st.registers;
        self.base = st.base;
        self.globals = st.globals;
        self.lines = st.lines;
        self.iter_stack = st.iter_stack;
        self.sched_active = st.sched_active;
        self.sched_pc = st.sched_pc;
        self.sched_call_stack = st.sched_call_stack;
    }

    /// Take the pending concurrency request (the driver services it).
    pub(crate) fn take_pending(&mut self) -> Option<VmBlock> {
        self.pending.take()
    }

    /// Deliver a resume value into the slot the last block reserved (if any).
    pub(crate) fn deliver_resume(&mut self, value: Value) {
        if let Some(slot) = self.resume_slot.take() {
            let value = self.wrap_resume_if_optional(value);
            self.set(slot, value);
        }
    }

    /// Wrap a try-receive's resume value as a real `Option` — see [`Self::resume_wraps_option`].
    fn wrap_resume_if_optional(&mut self, value: Value) -> Value {
        if !std::mem::take(&mut self.resume_wraps_option) {
            return value;
        }
        match &*value.as_runtime() {
            crate::interpreter::RuntimeValue::Nothing => {
                Value::from_runtime(crate::interpreter::option_none())
            }
            other => Value::from_runtime(crate::interpreter::option_some(other.clone())),
        }
    }

    /// Deliver a resume value into the slot the last block reserved, RETURNING the `(absolute slot,
    /// prior value)` it overwrote — so the reversible scheduler can undo the write (the delivery
    /// happens outside the recording loop, so it is not on the task's tape). `None` if the block
    /// reserved no slot (e.g. a `Send`).
    pub(crate) fn deliver_resume_recorded(&mut self, value: Value) -> Option<(usize, Value)> {
        let reg = self.resume_slot.take()?;
        let value = self.wrap_resume_if_optional(value);
        let slot = self.base + reg as usize;
        let old = self.registers[slot].clone();
        self.set(reg, value);
        Some((slot, old))
    }

    /// Deliver a resolved `Select`: the received value (when a recv arm won) into
    /// that arm's var register, and the winning arm index into the `SelectWait`'s
    /// destination register.
    pub(crate) fn deliver_select(&mut self, arm: usize, value: Value) {
        let var = self.select_pending.get(arm).and_then(|(_, v)| *v);
        if let Some(reg) = var {
            self.set(reg, value);
        }
        if let Some(slot) = self.resume_slot.take() {
            self.set(
                slot,
                Value::from_runtime(crate::interpreter::RuntimeValue::Int(arm as i64)),
            );
        }
        self.select_pending.clear();
    }

    /// Like [`deliver_select`], but RECORDS the prior value of each register it overwrites — the
    /// received value into the winning Recv arm's var register (when `value` is `Some`) and the
    /// winning arm index into the `SelectWait` destination — so the reversible scheduler can restore
    /// both. Returns `(var write, arm-index write)` as `(abs slot, prior value)` pairs.
    pub(crate) fn deliver_select_recorded(
        &mut self,
        arm: usize,
        value: Option<Value>,
    ) -> (Option<(usize, Value)>, Option<(usize, Value)>) {
        let var_write = value.and_then(|v| {
            let reg = self.select_pending.get(arm).and_then(|(_, r)| *r)?;
            let slot = self.base + reg as usize;
            let old = self.registers[slot].clone();
            self.set(reg, v);
            Some((slot, old))
        });
        let arm_write = self.resume_slot.take().map(|reg| {
            let slot = self.base + reg as usize;
            let old = self.registers[slot].clone();
            self.set(
                reg,
                Value::from_runtime(crate::interpreter::RuntimeValue::Int(arm as i64)),
            );
            (slot, old)
        });
        self.select_pending.clear();
        (var_write, arm_write)
    }

    /// Read a select-timeout register as a non-negative logical tick count.
    fn as_ticks(&self, r: Reg) -> Result<u64, String> {
        use crate::interpreter::RuntimeValue;
        Ok(match &*self.reg(r).as_runtime() {
            RuntimeValue::Int(n) => (*n).max(0) as u64,
            RuntimeValue::Duration(d) => (*d).max(0) as u64,
            RuntimeValue::Span { months, days } => {
                (((*months as i64) * 30 + *days as i64) * 86_400).max(0) as u64
            }
            other => {
                return Err(format!(
                    "select timeout must be a number or duration, found {}",
                    other.type_name()
                ))
            }
        })
    }

    /// Read a channel handle from register `r`.
    fn as_chan(&self, r: Reg) -> Result<ChanId, String> {
        match &*self.reg(r).as_runtime() {
            crate::interpreter::RuntimeValue::Chan(id) => Ok(*id),
            other => Err(format!("expected a channel, found {}", other.type_name())),
        }
    }

    /// Read a task handle from register `r`.
    fn as_task(&self, r: Reg) -> Result<TaskId, String> {
        match &*self.reg(r).as_runtime() {
            crate::interpreter::RuntimeValue::TaskHandle(id) => Ok(*id),
            other => Err(format!(
                "expected a task handle, found {}",
                other.type_name()
            )),
        }
    }

    /// Materialise register `r`'s value into a Send-able payload for a channel.
    fn materialize_reg(&self, r: Reg) -> Result<RtPayload, String> {
        let rt = self.reg(r).as_runtime();
        crate::concurrency::marshal::materialize(&rt)
            .map_err(|e| format!("cannot send value through a channel: {:?}", e))
    }

    /// Materialise a contiguous register window `[args_start, args_start+count)`
    /// into `Send`-able payloads — the spawn arguments that cross to the child
    /// task (and, under work-stealing, to its worker thread).
    fn materialize_args(&self, args_start: Reg, arg_count: u16) -> Result<Vec<RtPayload>, String> {
        (0..arg_count as Reg)
            .map(|i| self.materialize_reg(args_start + i))
            .collect()
    }

    /// Install the spawn entry-state for `functions[func](args)` into THIS VM:
    /// rebuild the payload args into a base-0 register window, push a sentinel
    /// frame so the body's `Return` terminates the task cleanly (result in
    /// register 0), and arm the scheduler `pc` at the function's `entry_pc`.
    ///
    /// Shared by both drivers: the cooperative one calls it on a freshly-cloned
    /// child (see [`Vm::spawn_task_vm`]); a work-stealing worker calls it on a VM
    /// built locally over its own borrow of the program.
    pub(crate) fn setup_task(&mut self, func: FuncIdx, args: &[RtPayload]) {
        let (entry_pc, reg_count) = {
            let f = &self.program.functions[func as usize];
            (f.entry_pc, f.register_count)
        };
        if self.registers.len() < reg_count {
            self.registers.resize(reg_count, Value::nothing());
        }
        for (i, a) in args.iter().enumerate() {
            self.registers[i] =
                Value::from_runtime(crate::concurrency::marshal::rebuild(a.clone()));
        }
        let restore_len = self.registers.len();
        self.sched_call_stack = vec![CallFrame {
            return_pc: self.program.code.len(),
            return_reg: 0,
            caller_base: 0,
            restore_len,
            iter_depth: 0,
            func,
            arg_lo: 0,
            arg_count: 0,
        }];
        self.sched_active = true;
        self.sched_pc = entry_pc;
    }

    /// Build a fresh child VM that runs `functions[func](args)` — a spawned task —
    /// sharing this VM's `&'p program` and run context (tier, policy, program
    /// args). Used by the cooperative driver, which builds children inline.
    pub(crate) fn spawn_task_vm(&self, func: FuncIdx, args: &[RtPayload]) -> Vm<'p> {
        let mut child = Vm::new(self.program);
        child.policy_ctx = self.policy_ctx;
        child.tier = self.tier;
        child.accel_pbkdf2 = self.accel_pbkdf2;
        child.program_args = self.program_args.clone();
        child.setup_task(func, args);
        child
    }

    /// Re-enter a program function `func` AS BYTECODE with a single materialised
    /// argument value, running it to completion on a fresh child VM that shares
    /// this VM's program + run context. Returns the handler's `Return` value
    /// (register 0) and any `Show` output it produced, in order. This is the
    /// `Serve` accept loop's per-connection dispatch — the exact analogue of the
    /// tree-walker's `call_function_with_values`, keeping the handler on the VM
    /// tier. A handler that suspends on nested IO (its own tcp/net) is refused
    /// with a clear error (→ a 500 for that request), never silently mis-run.
    pub(crate) fn call_handler_bytecode(
        &self,
        func: FuncIdx,
        arg: crate::interpreter::RuntimeValue,
    ) -> (
        Result<crate::interpreter::RuntimeValue, String>,
        Vec<String>,
    ) {
        let payload = match crate::concurrency::marshal::materialize(&arg) {
            Ok(p) => p,
            Err(e) => {
                return (
                    Err(format!("Serve: cannot marshal Request: {e:?}")),
                    Vec::new(),
                )
            }
        };
        let mut child = self.spawn_task_vm(func, &[payload]);
        match child.run_until_block() {
            // The sentinel frame `setup_task` installed carries `return_reg: 0`,
            // so the handler's `Return` leaves its `Response` in register 0 with
            // `base` restored to 0; the top-level `Done` payload is `Nothing`, so
            // read the return slot directly.
            Ok(VmStep::Done(_)) => {
                let value = (*child.registers[0].as_runtime()).clone();
                (Ok(value), child.drain_lines())
            }
            Ok(VmStep::Blocked) => (
                Err(
                    "Serve: the handler suspended on nested IO; the VM tier serves \
                     straight-line handlers (no tcp/net inside a request handler yet)"
                        .to_string(),
                ),
                child.drain_lines(),
            ),
            Ok(VmStep::Paused) => (
                Err("Serve: the handler exhausted its step budget".to_string()),
                child.drain_lines(),
            ),
            Err(e) => (Err(e), child.drain_lines()),
        }
    }

    fn binop(
        &mut self,
        dst: Reg,
        lhs: Reg,
        rhs: Reg,
        f: impl Fn(&Value, &Value) -> Result<Value, String>,
    ) -> Result<(), String> {
        let v = f(self.reg(lhs), self.reg(rhs))?;
        self.set(dst, v);
        Ok(())
    }

    /// `R[dst] = R[dst] + R[src]`, extending in place when `R[dst]` is a
    /// Text this register exclusively owns (`Rc` count 1 — no alias, capture
    /// snapshot, or iterator can observe the mutation). The two in-place arms
    /// transcribe the kernel's `(Text, Text)` / `(Text, other)` add rules;
    /// every other shape — shared Rc included — takes the kernel itself.
    fn add_assign(&mut self, dst: Reg, src: Reg) -> Result<(), String> {
        use crate::interpreter::RuntimeValue;
        use std::rc::Rc;
        let di = self.base + dst as usize;
        let si = self.base + src as usize;
        if di != si {
            let (a, b) = if di < si {
                let (lo, hi) = self.registers.split_at_mut(si);
                (&mut lo[di], &hi[0])
            } else {
                let (lo, hi) = self.registers.split_at_mut(di);
                (&mut hi[0], &lo[si])
            };
            // Only the heap Text arm takes the in-place fast path; peek without
            // promoting (`as_runtime_mut` would box an inline scalar) so the
            // common non-Text case stays inline and falls to the kernel below.
            if matches!(a.as_runtime_ref(), Some(RuntimeValue::Text(_))) {
                if let RuntimeValue::Text(rc) = a.as_runtime_mut() {
                    if let Some(s) = Rc::get_mut(rc) {
                        match &*b.as_runtime() {
                            RuntimeValue::Text(t) => s.push_str(t),
                            other => s.push_str(&other.to_display_string()),
                        }
                        return Ok(());
                    }
                }
            }
        }
        let v = self.reg(dst).add(self.reg(src))?;
        self.set(dst, v);
        Ok(())
    }
}

fn const_to_value(c: &Constant) -> Value {
    use crate::interpreter::RuntimeValue;
    match c {
        Constant::Int(n) => Value::int(*n),
        Constant::Float(f) => Value::float(*f),
        Constant::Bool(b) => Value::bool(*b),
        Constant::Text(s) => Value::text(s.clone()),
        Constant::Char(c) => Value::from_runtime(RuntimeValue::Char(*c)),
        Constant::Nothing => Value::nothing(),
        Constant::Duration(n) => Value::from_runtime(RuntimeValue::Duration(*n)),
        Constant::Date(d) => Value::from_runtime(RuntimeValue::Date(*d)),
        Constant::Moment(n) => Value::from_runtime(RuntimeValue::Moment(*n)),
        Constant::Span { months, days } => Value::from_runtime(RuntimeValue::Span {
            months: *months,
            days: *days,
        }),
        Constant::Time(n) => Value::from_runtime(RuntimeValue::Time(*n)),
    }
}

/// The shared-state after-effect a scheduler slice's terminating block had, so reverse can undo it.
enum ChanEffect {
    None,
    /// Created channel index (reverse pops it off the channel list — LIFO with creation order).
    Create,
    /// Enqueued to channel `idx` (reverse pops the back).
    Send(usize),
    /// Dequeued the front of channel `idx` (reverse pushes the payload back to the front).
    Recv(usize, RtPayload),
    /// Spawned a child task (reversed on its own tape; nothing to undo here).
    Spawn,
    /// Closed channel `idx` (reverse clears its closed flag).
    Close(usize),
}

/// One recorded scheduler slice: task `task` ran `steps` ops, then hit a block the driver serviced.
struct Slice {
    task: usize,
    steps: usize,
    /// The resume value the block delivered into a register (a channel handle / received payload),
    /// captured as `(target task, abs slot, prior value)` so reverse restores it — the delivery
    /// happens OUTSIDE the recording loop, so it is not on the task's own tape. The target task is
    /// usually `task` itself, but for a blocking-receive HANDOFF the SENDER's slice delivers the
    /// payload directly into the parked RECEIVER's register, so the target differs from `task`.
    resume: Option<(usize, usize, Value)>,
    /// A SECOND block-delivered write, for the one block that writes two registers: a resolved
    /// `Select` delivers the received value into the arm's var register (`resume`) AND the winning
    /// arm index into the `SelectWait` destination (`resume2`). `None` for every other block.
    resume2: Option<(usize, usize, Value)>,
    chan: ChanEffect,
}

/// A REVERSIBLE COOPERATIVE SCHEDULER — drives a main VM plus its spawned tasks deterministically
/// with per-task reversibility recording, capturing the interleaved execution SCHEDULE + channel
/// state; then reverses the entire concurrent execution schedule-backward across all task VMs. Each
/// task's slices pop off its own tape in exact reverse order, channel deltas are undone at the block
/// that caused them, and block-delivered resume values are restored — so `forward ; reverse` restores
/// every task's state AND the channel topology.
///
/// Scope: fire-and-forget `Spawn`, `Sleep`, and channels (`ChanNew`/`Send`/`Recv`) — including a
/// BLOCKING receive, where a task parks on an empty channel until a later send hands the payload
/// straight into its register and wakes it (the `do_recv`/`do_send` model). Also `SpawnHandle` +
/// `TaskAwait`: an awaiter parks until the child finishes, whose `Done` slice hands its result off
/// and wakes it (the same cross-task pattern). And `Select` (first-ready-recv-else-timeout): the
/// winning arm's value + index deliveries are both recorded and reversed.
pub(crate) struct RevCoop<'p> {
    tasks: Vec<Vm<'p>>,
    channels: Vec<std::collections::VecDeque<RtPayload>>,
    closed: Vec<bool>,
    schedule: Vec<Slice>,
}

impl<'p> RevCoop<'p> {
    /// Drive `main` forward to quiescence with per-task recording, capturing the schedule + channel
    /// state. Declines a program using any op outside the reversible subset, or a block beyond the
    /// current scope, rather than mis-reversing.
    pub(crate) fn record(main: Vm<'p>) -> Result<Self, String> {
        Self::record_mode(main, super::reverse::ReverseMode::Observable)
    }

    /// Like [`RevCoop::record`] but in an explicit [`super::reverse::ReverseMode`] — `Adiabatic`
    /// RECOMPUTES each task's reverse-live scratch on reverse (per-task register frames are independent,
    /// so per-task recompute is sound), driving the concurrent tape toward the Landauer floor.
    pub(crate) fn record_mode(
        mut main: Vm<'p>,
        mode: super::reverse::ReverseMode,
    ) -> Result<Self, String> {
        if let Some(op) = main
            .program
            .code
            .iter()
            .find(|op| !super::reverse::is_reversible_supported(op))
        {
            return Err(match super::reverse::external_effect_barrier(op) {
                // A truly-external effect: name the irreversibility barrier (internal state up to it
                // reverses; the effect itself cannot be un-done) — never a silent lie.
                Some(barrier) => format!(
                    "vm: `{}` is an irreversible {barrier} effect — not yet reversible past the honest Landauer wall (an external effect cannot be un-done)",
                    op.name()
                ),
                None => format!("vm: op `{}` is not yet reversible", op.name()),
            });
        }
        logicaffeine_base::money::clear_ambient_rates();
        // Concurrency elision/recompute: each task VM records under `mode` (tasks have independent
        // register frames, so per-task elision AND recompute are sound). Empty `named_regs` ⇒ no elision.
        main.install_reverse_recording(mode);
        let mut tasks = vec![main];
        let mut ready = vec![true];
        let mut channels: Vec<std::collections::VecDeque<RtPayload>> = Vec::new();
        let mut closed: Vec<bool> = Vec::new();
        // Per-channel FIFO of task ids parked on an empty receive (the do_recv/do_send handoff model).
        // Forward-routing bookkeeping only — reverse replays the recorded handoffs, never re-routes.
        let mut blocked_receivers: Vec<std::collections::VecDeque<usize>> = Vec::new();
        // Each task's captured `Done` result (for `Await`), and — per parked awaiter — the child task
        // it is blocked awaiting. Grow in lockstep with `tasks`/`ready`. Forward bookkeeping only.
        let mut results: Vec<Option<Value>> = vec![None];
        let mut awaiting: Vec<Option<usize>> = vec![None];
        let mut schedule: Vec<Slice> = Vec::new();
        loop {
            // Deterministic cooperative pick: the lowest-id ready task.
            let tid = match (0..tasks.len()).find(|&i| ready[i]) {
                Some(t) => t,
                None => break,
            };
            let before = tasks[tid].journal.as_ref().map_or(0, |j| j.step_count());
            let step = tasks[tid].run_until_block_impl::<false, true>(u64::MAX)?;
            let steps = tasks[tid].journal.as_ref().map_or(0, |j| j.step_count()) - before;
            // A second delivered write, set only by a resolved `Select` (value + arm-index).
            let mut resume2: Option<(usize, usize, Value)> = None;
            let (resume, chan) = match step {
                VmStep::Done(rv) => {
                    ready[tid] = false;
                    results[tid] = Some(Value::from_runtime(rv));
                    // A parent parked in `Await` on this task takes its result now — a cross-task
                    // handoff into the awaiter's register — and wakes (do_await / deliver-on-complete).
                    if let Some(pid) = (0..tasks.len()).find(|&p| awaiting[p] == Some(tid)) {
                        let v = results[tid].clone().unwrap();
                        awaiting[pid] = None;
                        ready[pid] = true;
                        let r = tasks[pid]
                            .deliver_resume_recorded(v)
                            .map(|(s, o)| (pid, s, o));
                        (r, ChanEffect::None)
                    } else {
                        (None, ChanEffect::None)
                    }
                }
                VmStep::Blocked => {
                    let block = tasks[tid]
                        .take_pending()
                        .ok_or("vm: blocked with no request")?;
                    match block {
                        VmBlock::Sleep(_) => (None, ChanEffect::None),
                        VmBlock::NewChan(_) => {
                            let idx = channels.len();
                            channels.push(std::collections::VecDeque::new());
                            closed.push(false);
                            blocked_receivers.push(std::collections::VecDeque::new());
                            let handle = Value::from_runtime(crate::interpreter::RuntimeValue::Chan(
                                logicaffeine_runtime::ChanId(idx as u64),
                            ));
                            let r = tasks[tid].deliver_resume_recorded(handle).map(|(s, o)| (tid, s, o));
                            (r, ChanEffect::Create)
                        }
                        VmBlock::Send(ch, payload) => {
                            let idx = ch.0 as usize;
                            // Direct handoff (do_send): a parked receiver takes the payload straight
                            // into its register and wakes; nothing enqueues. The delivery is CROSS-TASK
                            // — it lands in the receiver's slot, recorded on the sender's slice.
                            if let Some(rid) = blocked_receivers[idx].pop_front() {
                                let v = Value::from_runtime(
                                    crate::concurrency::marshal::rebuild(payload),
                                );
                                let r = tasks[rid].deliver_resume_recorded(v).map(|(s, o)| (rid, s, o));
                                ready[rid] = true;
                                (r, ChanEffect::None)
                            } else {
                                channels[idx].push_back(payload);
                                (None, ChanEffect::Send(idx))
                            }
                        }
                        VmBlock::Recv(ch) => {
                            let idx = ch.0 as usize;
                            match channels[idx].pop_front() {
                                Some(payload) => {
                                    let v = Value::from_runtime(
                                        crate::concurrency::marshal::rebuild(payload.clone()),
                                    );
                                    let r = tasks[tid]
                                        .deliver_resume_recorded(v)
                                        .map(|(s, o)| (tid, s, o));
                                    (r, ChanEffect::Recv(idx, payload))
                                }
                                // Empty channel: PARK the receiver (do_recv). Its resume_slot + saved pc
                                // persist on the task VM until a later send hands off directly to it.
                                None => {
                                    blocked_receivers[idx].push_back(tid);
                                    ready[tid] = false;
                                    (None, ChanEffect::None)
                                }
                            }
                        }
                        VmBlock::SpawnDesc { func, args, want_handle } => {
                            let child_idx = tasks.len();
                            let mut child = tasks[tid].spawn_task_vm(func, &args);
                            child.install_reverse_recording(mode);
                            tasks.push(child);
                            ready.push(true);
                            results.push(None);
                            awaiting.push(None);
                            // `SpawnHandle` delivers a TaskHandle into the parent's dst; fire-and-forget
                            // `Spawn` writes nothing. The child reverses on its own tape either way.
                            let resume = if want_handle {
                                let handle =
                                    Value::from_runtime(crate::interpreter::RuntimeValue::TaskHandle(
                                        logicaffeine_runtime::TaskId(child_idx as u64),
                                    ));
                                tasks[tid].deliver_resume_recorded(handle).map(|(s, o)| (tid, s, o))
                            } else {
                                None
                            };
                            (resume, ChanEffect::Spawn)
                        }
                        VmBlock::Await(awaited) => {
                            let a = awaited.0 as usize;
                            match results.get(a).and_then(|r| r.clone()) {
                                // Child already finished — take its result now (same slice).
                                Some(v) => {
                                    let r = tasks[tid]
                                        .deliver_resume_recorded(v)
                                        .map(|(s, o)| (tid, s, o));
                                    (r, ChanEffect::None)
                                }
                                // Child still running — park the awaiter; the child's Done slice hands
                                // its result off and wakes it.
                                None => {
                                    awaiting[tid] = Some(a);
                                    ready[tid] = false;
                                    (None, ChanEffect::None)
                                }
                            }
                        }
                        VmBlock::TrySend(ch, payload) => {
                            // Non-blocking send: always succeeds on an unbounded channel (Bool(true)
                            // into dst). Hands off to a parked receiver (resume2, cross-task) or
                            // enqueues, exactly like `Send`.
                            let idx = ch.0 as usize;
                            let chan = if let Some(rid) = blocked_receivers[idx].pop_front() {
                                let v = Value::from_runtime(
                                    crate::concurrency::marshal::rebuild(payload),
                                );
                                resume2 =
                                    tasks[rid].deliver_resume_recorded(v).map(|(s, o)| (rid, s, o));
                                ready[rid] = true;
                                ChanEffect::None
                            } else {
                                channels[idx].push_back(payload);
                                ChanEffect::Send(idx)
                            };
                            let ok = Value::from_runtime(crate::interpreter::RuntimeValue::Bool(true));
                            let r = tasks[tid].deliver_resume_recorded(ok).map(|(s, o)| (tid, s, o));
                            (r, chan)
                        }
                        VmBlock::TryRecv(ch) => {
                            // Non-blocking receive: the queued value, else `Nothing` — never parks.
                            let idx = ch.0 as usize;
                            match channels[idx].pop_front() {
                                Some(payload) => {
                                    // ★ A REAL OPTION, matching the tree-walker's
                                    // `Stmt::TryReceivePipe`. A try-receive answers "a value, or
                                    // nothing", which IS an Option — handing back the bare value
                                    // meant `Inspect x: When Some (v):` had no arm to take.
                                    // R1 keeps `Show x` printing the value; R2 keeps the empty
                                    // case equal to `nothing`.
                                    let v = Value::from_runtime(crate::interpreter::option_some(
                                        crate::concurrency::marshal::rebuild(payload.clone()),
                                    ));
                                    let r = tasks[tid]
                                        .deliver_resume_recorded(v)
                                        .map(|(s, o)| (tid, s, o));
                                    (r, ChanEffect::Recv(idx, payload))
                                }
                                None => {
                                    let nothing =
                                        Value::from_runtime(crate::interpreter::option_none());
                                    let r = tasks[tid]
                                        .deliver_resume_recorded(nothing)
                                        .map(|(s, o)| (tid, s, o));
                                    (r, ChanEffect::None)
                                }
                            }
                        }
                        VmBlock::Close(ch) => {
                            let idx = ch.0 as usize;
                            closed[idx] = true;
                            (None, ChanEffect::Close(idx))
                        }
                        VmBlock::Select(arms) => {
                            use logicaffeine_runtime::SelectArm;
                            // First-ready-recv-else-timeout (the deterministic select semantics): the
                            // lowest-index Recv arm whose channel has a payload wins; otherwise a
                            // Timeout arm fires. Either delivers the winning arm index; a Recv win also
                            // delivers the received value into that arm's var register.
                            let recv_win = arms.iter().enumerate().find_map(|(i, a)| match a {
                                SelectArm::Recv(ch) => {
                                    let idx = ch.0 as usize;
                                    (!channels[idx].is_empty()).then_some((i, idx))
                                }
                                SelectArm::Timeout(_) => None,
                            });
                            if let Some((arm_i, idx)) = recv_win {
                                let payload = channels[idx].pop_front().unwrap();
                                let v = Value::from_runtime(
                                    crate::concurrency::marshal::rebuild(payload.clone()),
                                );
                                let (var_w, arm_w) =
                                    tasks[tid].deliver_select_recorded(arm_i, Some(v));
                                resume2 = arm_w.map(|(s, o)| (tid, s, o));
                                (var_w.map(|(s, o)| (tid, s, o)), ChanEffect::Recv(idx, payload))
                            } else if let Some(arm_i) =
                                arms.iter().position(|a| matches!(a, SelectArm::Timeout(_)))
                            {
                                let (_, arm_w) = tasks[tid].deliver_select_recorded(arm_i, None);
                                resume2 = arm_w.map(|(s, o)| (tid, s, o));
                                (None, ChanEffect::None)
                            } else {
                                return Err("vm: reversible driver: a select with no ready arm and no timeout is not yet supported".to_string());
                            }
                        }
                        _ => {
                            return Err("vm: reversible driver supports spawn / await / select / sleep / channel ops only".to_string())
                        }
                    }
                }
                VmStep::Paused => unreachable!("recording run is not stepped"),
            };
            schedule.push(Slice {
                task: tid,
                steps,
                resume,
                resume2,
                chan,
            });
        }
        Ok(RevCoop {
            tasks,
            channels,
            closed,
            schedule,
        })
    }

    /// Reverse the entire interleaved execution: walk the schedule BACKWARD; for each slice undo its
    /// channel delta and resume-value write, then reverse the task's ops from its own tape.
    pub(crate) fn reverse(&mut self) -> Result<(), String> {
        for i in (0..self.schedule.len()).rev() {
            let (task, steps, resume, resume2) = {
                let s = &self.schedule[i];
                (s.task, s.steps, s.resume.clone(), s.resume2.clone())
            };
            // Undo the shared-state effect of this slice's terminating block.
            match &self.schedule[i].chan {
                ChanEffect::Create => {
                    self.channels.pop();
                }
                ChanEffect::Send(idx) => {
                    self.channels[*idx].pop_back();
                }
                ChanEffect::Recv(idx, payload) => {
                    let (idx, payload) = (*idx, payload.clone());
                    self.channels[idx].push_front(payload);
                }
                ChanEffect::Close(idx) => {
                    self.closed[*idx] = false;
                }
                ChanEffect::None | ChanEffect::Spawn => {}
            }
            // Undo the block-delivered resume-value write (into its TARGET task — for a blocking-recv
            // handoff the sender's slice wrote into the parked receiver, so the target differs).
            if let Some((target, slot, old)) = resume {
                self.tasks[target].registers[slot] = old;
            }
            // A resolved `Select` delivered a SECOND write (the winning arm index) — undo it too.
            if let Some((target, slot, old)) = resume2 {
                self.tasks[target].registers[slot] = old;
            }
            // Reverse the task's ops for this slice from its own tape.
            let mut tape = self.tasks[task]
                .journal
                .take()
                .ok_or("vm: task tape missing")?;
            self.tasks[task].reverse_slice(&mut tape, steps)?;
            self.tasks[task].journal = Some(tape);
        }
        // Adiabatic: once every task's slices are reversed, reset each task's elided scratch to its
        // default — the concurrency analogue of `Vm::run_reverse`'s tail reset (a no-op otherwise).
        for task in &mut self.tasks {
            task.adiabatic_reset();
        }
        Ok(())
    }

    /// The main task's observable state (for the `forward ; reverse ≡ initial` check).
    pub(crate) fn main_state(&self) -> DebugVmState {
        self.tasks[0].save_debug_state()
    }

    /// The main task's `Show` output after the forward run — the reference the soft-core differential
    /// compares against for the scheduler-only ops (`TaskAwait`/`ChanClose`) that no source emits, so a
    /// hand-built bytecode program driven through this scheduler is the only VM oracle for them.
    pub(crate) fn main_output(&self) -> String {
        self.tasks[0].output()
    }

    /// The main task's recorded erased-tape byte count (before reverse) — for asserting the concurrency
    /// pebbling actually shrinks the tape under `Adiabatic` vs `Observable`.
    pub(crate) fn main_erased_bytes(&self) -> usize {
        self.tasks[0]
            .journal
            .as_ref()
            .map_or(0, |t| t.erased_bytes())
    }

    /// The number of live channels (for asserting the channel topology reversed to empty).
    pub(crate) fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/machine/reverse_execution.rs"]
mod reverse_execution;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/machine/string_build_fastpath.rs"]
mod string_build_fastpath;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/machine/debug_stepping.rs"]
mod debug_stepping;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/machine/host_calls.rs"]
mod host_calls;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/machine/record_copy_work.rs"]
mod record_copy_work;

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/machine/handle_count_tests.rs"]
mod handle_count_tests;
