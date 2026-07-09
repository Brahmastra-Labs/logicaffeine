//! The bytecode dispatch loop.
//!
//! Registers live in one contiguous `Vec<Value>`; `base` is the current frame's
//! offset into it, so every register access is `registers[base + r]`. Calls use
//! register windowing — the callee's frame starts at the caller's `args_start`,
//! so arguments are passed with zero copying.

use super::instruction::{CompiledProgram, Constant, FuncIdx, Op, Reg};
use super::value::Value;
use super::MAX_REGISTER_FILE;
use logicaffeine_runtime::{ChanId, RtPayload, SelectArm, TaskId};

/// LEVER B callee analysis — may a region CALL this function while passing a
/// pinned list argument? Returns `(list_params_stable, returns_list_param)`,
/// both SOUND under-approximations:
/// - `list_params_stable`: the body has NO `ListPush` and NO sub-`Call` (either
///   could reallocate a list-param's buffer, staling the caller's derived raw
///   pointer). So every list-param buffer keeps its address across the call.
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
            if let Op::Move { dst, src } = *op {
                let (d, s) = (dst as usize, src as usize);
                if d < n && s < n && is_param_list[s] && !is_param_list[d] {
                    is_param_list[d] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let list_params_stable =
        !body.iter().any(|op| matches!(op, Op::ListPush { .. } | Op::Call { .. }));
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
        .filter_map(|op| if let Op::Return { src } = *op { Some(src) } else { None })
        .collect();
    let returns_list_param = uniform
        && !returns.is_empty()
        && returns
            .iter()
            .all(|&s| (s as usize) < n && is_param_list[s as usize]);
    (list_params_stable, returns_list_param)
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
    pub current_func: Option<u16>,
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

/// The heap identity of a value — its allocation address, kind, reference count, and
/// storage-layout label. `None` for inline scalars (Int/Float/Bool/Char/…), which live
/// in the register slot itself and are not heap objects.
fn heap_identity(val: &Value) -> Option<(usize, String, usize, String)> {
    use crate::interpreter::RuntimeValue as RV;
    use std::rc::Rc;
    let s = |x: &str| x.to_string();
    match val.as_runtime_ref()? {
        RV::List(rc) => Some((Rc::as_ptr(rc) as usize, s("list"), Rc::strong_count(rc), rc.borrow().storage_label().to_string())),
        RV::Map(rc) => Some((Rc::as_ptr(rc) as usize, s("map"), Rc::strong_count(rc), s("hash map"))),
        RV::Set(rc) => Some((Rc::as_ptr(rc) as usize, s("set"), Rc::strong_count(rc), s("vec set"))),
        RV::Tuple(rc) => Some((Rc::as_ptr(rc) as usize, s("tuple"), Rc::strong_count(rc), s("fixed tuple"))),
        RV::Text(rc) => Some((Rc::as_ptr(rc) as usize, s("text"), Rc::strong_count(rc), s("Rc<String>"))),
        RV::Struct(b) => Some((&**b as *const _ as usize, s("struct"), 1, s("field map"))),
        RV::Inductive(b) => Some((&**b as *const _ as usize, s("enum"), 1, s("tagged variant"))),
        _ => None,
    }
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
    SpawnDesc { func: FuncIdx, args: Vec<RtPayload>, want_handle: bool },
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
    /// Live `Repeat` snapshots: (elements, next index). Stack-disciplined —
    /// `IterPrepare` pushes, `IterPop` pops, nesting nests.
    iter_stack: Vec<(Vec<Value>, usize)>,
    /// Promoted globals (None = not yet defined; reading one is the
    /// "Undefined variable" error).
    globals: Vec<Option<Value>>,
    /// Policy registry + interner for `Check` statements (absent ⇒ the
    /// tree-walker's "Security Check requires policies" error).
    policy_ctx: Option<(&'p crate::analysis::PolicyRegistry, &'p crate::intern::Interner)>,
    /// The pluggable native tier (None = pure bytecode, e.g. WASM).
    tier: Option<&'p dyn super::native_tier::NativeTier>,
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
    /// The concurrency request produced by the last `Blocked` slice (taken by the
    /// driver). `None` between slices and for a non-concurrent run.
    pending: Option<VmBlock>,
    /// The register the next resume value is delivered into (`None` for a block
    /// that yields nothing, e.g. `Send`/`Close`).
    resume_slot: Option<Reg>,
    /// Accumulated `Select` arms awaiting a `SelectWait`: each runtime arm plus
    /// the register a winning recv arm binds its value into. Persists across the
    /// block so `deliver_select` can route the received value to the right arm.
    select_pending: Vec<(SelectArm, Option<Reg>)>,
    /// WS6 (Phase 13): the browser WASM-JIT tier. Consulted from `Op::Call` only under the
    /// `wasm-jit` feature; entirely absent from the default build (and behind the native x86
    /// forge tier on native, so it is the JIT tier specifically where forge cannot run —
    /// wasm32).
    #[cfg(feature = "wasm-jit")]
    wasm_tier: super::wasm_jit::WasmTier,
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
            iter_stack: Vec::new(),
            globals: vec![None; program.globals.len()],
            policy_ctx: None,
            tier: None,
            hot: vec![0; program.functions.len()],
            native: (0..program.functions.len())
                .map(|_| super::native_tier::NativeSlot::Untried)
                .collect(),
            region_hot: rustc_hash::FxHashMap::default(),
            regions: rustc_hash::FxHashMap::default(),
            region_cow_regs: rustc_hash::FxHashMap::default(),
            region_blacklist: vec![false; program.code.len()],
            program_args: Vec::new(),
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
            pending: None,
            resume_slot: None,
            select_pending: Vec::new(),
            #[cfg(feature = "wasm-jit")]
            wasm_tier: super::wasm_jit::WasmTier::new(50),
        }
    }

    /// Install a re-optimized body as function `fi`'s warm tier (HOTSWAP §7 / P11):
    /// append it to `warm_code` after `program.code`, rebasing its 0-relative jumps
    /// into that unified pc space, and point `warm_entry[fi]` at it. Subsequent calls
    /// to `fi` run this body. The body shares the program's constant pool (a
    /// `FnBytecode` preserves constant indices), so only jumps are relocated.
    pub fn install_warm_bytecode(&mut self, fi: usize, fnbc: &super::fn_bytecode::FnBytecode) -> bool {
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
        self.warm_code
            .extend(fnbc.code.iter().map(|&op| super::fn_bytecode::rebase(op, abs_base as isize)));
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
        self.regions.insert(head, super::native_tier::RegionSlot::Failed);
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
                | Op::NewEmptySet { dst }
                | Op::NewEmptyMap { dst }
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
            Op::ListPush { value, .. } | Op::SetAdd { value, .. } | Op::RemoveFrom { value, .. } => {
                h(value)
            }
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
            | Op::NewEmptySet { .. }
            | Op::NewEmptyMap { .. }
            | Op::NewEmptyListI32 { .. } => false,
            Op::NewRange { start, end, .. } => h(start) || h(end),
            Op::NewList { start, count, .. } | Op::NewTuple { start, count, .. } => rng(*start, *count),
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
            Op::TestArm { dst, target, .. } | Op::BindArm { dst, target, .. } => h(dst) || h(target),
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
                    if let Op::Jump { target } | Op::JumpIfFalse { target, .. }
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
                            .map(|f| f.mutable_param_regs.as_slice())
                            .unwrap_or(&[]);
                        cow_regs =
                            mutated.into_iter().filter(|r| !mutable_params.contains(r)).collect();
                    }
                }
                let reg_count = u16::try_from(frame_regs).ok()?;
                // Speculation seed: the kinds sitting in this frame's
                // registers RIGHT NOW. The adapter compiles against them;
                // the guard set re-checks them on every entry.
                let observed: Vec<super::native_tier::ObservedKind> = (0..frame_regs)
                    .map(|r| {
                        use crate::interpreter::{ListRepr, RuntimeValue};
                        use super::native_tier::ObservedKind;
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
                                ret: f.ret_kind,
                                list_params_stable,
                                returns_list_param,
                            }
                        })
                        .collect()
                };
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
                        named.iter().enumerate().filter(|(r, &n)| n && m.get(*r).copied().unwrap_or(false)).count()
                    });
                    eprintln!("liveout-trace: head={head} named={nnamed} loop_locals={} freed={freed}", ll.is_some());
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
                        self.regions.insert(head, RegionSlot::Ready { rf, exit_pc, misses: 0 });
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
                self.ensure_reg_owned(r, cur_func);
            }
        }
        let Some(RegionSlot::Ready { rf, exit_pc, .. }) = self.regions.get(&head) else {
            unreachable!()
        };
        // Guard: every live-in slot must hold exactly the kind the region
        // speculated on; copy the raw representation in (floats as bits).
        {
            use crate::interpreter::RuntimeValue;
            use super::native_tier::SlotKind;
            for &(r, kind) in rf.guard_set() {
                let v = self.registers.get(self.base + r as usize)?;
                match (kind, &*v.as_runtime()) {
                    (SlotKind::Int, RuntimeValue::Int(_)) => {}
                    (SlotKind::Float, RuntimeValue::Float(_)) => {}
                    (SlotKind::Bool, RuntimeValue::Bool(_)) => {}
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
                frame.resize(need + frame_canary, 0);
            }
            frame[..rf.frame_size()].fill(0);
            for c in &mut frame[need..need + frame_canary] {
                *c = FRAME_SENTINEL;
            }
            frame
        });
        let mut frame = frame_cell;
        {
            use crate::interpreter::RuntimeValue;
            use super::native_tier::SlotKind;
            for &(r, kind) in rf.guard_set() {
                frame[r as usize] =
                    match (kind, &*self.registers[self.base + r as usize].as_runtime()) {
                        (SlotKind::Int, RuntimeValue::Int(n)) => *n,
                        (SlotKind::Float, RuntimeValue::Float(f)) => f.to_bits() as i64,
                        (SlotKind::Bool, RuntimeValue::Bool(b)) => *b as i64,
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
        let (outcome, text_mut_snapshots) = {
            use crate::interpreter::{ListRepr, RuntimeValue};
            let pins = rf.array_set();
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
            for pin in pins {
                if pin.elem == super::native_tier::PinElem::Map {
                    let v = self.registers.get(self.base + pin.reg as usize)?;
                    let Some(RuntimeValue::Map(rc)) = v.as_runtime_ref() else { return None };
                    let key = std::rc::Rc::as_ptr(rc) as usize;
                    if !map_handles.iter().any(|(k, _)| *k == key) {
                        let Ok(b) = rc.try_borrow_mut() else { return None };
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
                    let Some(RuntimeValue::Text(rc)) = v.as_runtime_ref() else { return None };
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
                    if !matches!(unsafe { (*cell).as_runtime_ref() }, Some(RuntimeValue::Text(_))) {
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
                let Some(RuntimeValue::List(rc)) = v.as_runtime_ref() else { return None };
                let key = std::rc::Rc::as_ptr(rc) as usize;
                if !handles.iter().any(|(k, _)| *k == key) {
                    let Ok(b) = rc.try_borrow_mut() else { return None };
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
                    (ListRepr::Ints(v), PinElem::Int) => {
                        (v as *mut Vec<i64> as i64, v.as_mut_ptr() as *mut i64, v.len())
                    }
                    (ListRepr::IntsI32(v), PinElem::IntI32) => {
                        (v as *mut Vec<i32> as i64, v.as_mut_ptr() as *mut i64, v.len())
                    }
                    // Maps never reach this arm (their pin path is below) —
                    // a list register observed as Map is a guard failure.
                    (_, PinElem::Map) => return None,
                    (ListRepr::Floats(v), PinElem::Float) => {
                        (v as *mut Vec<f64> as i64, v.as_mut_ptr() as *mut i64, v.len())
                    }
                    (ListRepr::Bools(v), PinElem::Bool) => {
                        (v as *mut Vec<bool> as i64, v.as_mut_ptr() as *mut i64, v.len())
                    }
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
            // HOISTED bounds checks (V8 loop bound-check elimination): with
            // the pinned lengths in hand, verify ONCE that every covered loop
            // access stays in bounds for the whole run. Any failure declines
            // the region — the VM replays the loop on bytecode, where the
            // accesses are checked and produce the exact error.
            for hg in rf.hoist_guards() {
                let len = frame[hg.len_slot as usize];
                let bound = match &*self.registers.get(self.base + hg.bound_reg as usize)?.as_runtime() {
                    RuntimeValue::Int(n) => *n,
                    _ => return None,
                };
                let iv = match &*self.registers.get(self.base + hg.iv_reg as usize)?.as_runtime() {
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
            let out = rf.run(&mut frame[..need], depth_now);
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
            (out, text_mut_snapshots)
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
        for (k, c) in frame[need..need + frame_canary].iter().enumerate() {
            assert_eq!(
                *c, FRAME_SENTINEL,
                "REGION_FRAME OVERFLOW: canary slot {k} (frame + {}) clobbered by region native code",
                need + k
            );
        }
        match outcome {
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
                        None => continue,
                    };
                    self.set(r, v);
                }
                REGION_FRAME.with(|f| f.replace(frame));
                return Some(RegionExit::At(resume_pc));
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
    /// `ensure_collection_owned`). Before mutating the collection in register
    /// `reg`, deep-clone it if another holder shares the allocation (`Rc` strong
    /// count > 1). Gated behind the migration flag — off by default, so the hot
    /// path is untouched. NOTE: the `mutable`-parameter exemption is NOT yet
    /// wired on the VM (compiled bytecode carries no param-mutability marker);
    /// that is the remaining VM-compiler work before the flag can be flipped on.
    fn ensure_reg_owned(&mut self, reg: Reg, cur_func: Option<u16>) {
        if !crate::semantics::collections::value_semantics_enabled() {
            return;
        }
        // A `mutable` parameter passes by reference: mutate the shared allocation
        // in place so the caller observes it (mirrors the tree-walker's skip-COW).
        let is_mutable_param = cur_func
            .and_then(|fi| self.program.functions.get(fi as usize))
            .is_some_and(|f| f.mutable_param_regs.contains(&reg));
        if is_mutable_param {
            return;
        }
        use crate::interpreter::RuntimeValue;
        use std::rc::Rc;
        let di = self.base + reg as usize;
        let shared = matches!(
            self.registers.get(di).map(|v| v.as_runtime()).as_deref(),
            Some(RuntimeValue::List(rc)) if Rc::strong_count(rc) > 1
        ) || matches!(
            self.registers.get(di).map(|v| v.as_runtime()).as_deref(),
            Some(RuntimeValue::Map(rc)) if Rc::strong_count(rc) > 1
        ) || matches!(
            self.registers.get(di).map(|v| v.as_runtime()).as_deref(),
            Some(RuntimeValue::Set(rc)) if Rc::strong_count(rc) > 1
        );
        if shared {
            let owned = self.registers[di].as_runtime().deep_clone();
            self.registers[di] = Value::from_runtime(owned);
        }
    }

    /// Null this call's argument-window registers after it returns. Dead once the
    /// call is over, but as the callee's params they persist below `restore_len`;
    /// left set, a collection argument keeps a live `Rc` clone in the caller frame
    /// that spuriously inflates `strong_count` and forces later copy-on-write. Zero
    /// `arg_count` (native/scheduler frames) does nothing.
    #[inline]
    fn clear_arg_window(&mut self, frame: &CallFrame) {
        let end = (frame.arg_lo + frame.arg_count as usize).min(self.registers.len());
        for slot in frame.arg_lo..end {
            self.registers[slot] = Value::nothing();
        }
    }

    /// Same as [`Vm::clear_arg_window`] for a call that completes INLINE (a native /
    /// WASM / builtin dispatch that never pushes a `CallFrame`): the argument window
    /// starts at the current base's `args_start`.
    #[inline]
    fn clear_args(&mut self, args_start: Reg, arg_count: u16) {
        let lo = self.base + args_start as usize;
        let end = (lo + arg_count as usize).min(self.registers.len());
        for slot in lo..end {
            self.registers[slot] = Value::nothing();
        }
    }

    /// Resolve a function's source name for the tier trace; empty when no interner is
    /// available (the trace then prints just the index).
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
                    Some(nf) => {
                        self.native_ctx.table.publish(fi, nf.entry_ptr(), nf.published_regc());
                        let name = self.fn_name(fi);
                        super::tier_trace::trace_transition(fi, &name, super::tier_trace::ExecTier::NativeForge);
                        self.native[fi] = NativeSlot::Ready(nf);
                    }
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
                    Some(nf) => {
                        self.native_ctx.table.publish(fi, nf.entry_ptr(), nf.published_regc());
                        let name = self.fn_name(fi);
                        super::tier_trace::trace_transition(fi, &name, super::tier_trace::ExecTier::NativeForge);
                        self.native[fi] = NativeSlot::Ready(nf);
                    }
                    None => self.native[fi] = NativeSlot::Failed,
                },
            }
        }
    }

    /// Supply the program arguments read by the `args()` system native. The
    /// vector is the full argv (index 0 is the program name), matching the
    /// compiled binary's `env::args()`.
    pub fn with_program_args(mut self, args: Vec<String>) -> Self {
        self.program_args = args;
        self
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
        pc: usize,
        call_stack: &mut Vec<CallFrame>,
    ) -> Result<(), String> {
        use super::native_tier::RegBox;
        let mut frame_base = self.base + args_start as usize;
        let mut caller_base = self.base;
        for (k, fr) in frames.iter().enumerate() {
            if k > 0 {
                caller_base = frame_base;
                frame_base += fr.offset;
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
                return_pc: if k == 0 { pc + 1 } else { fr.return_pc },
                return_reg: if k == 0 { dst } else { fr.return_reg },
                caller_base,
                restore_len,
                iter_depth: self.iter_stack.len(),
                func,
                arg_lo: 0,
                arg_count: 0,
            });
            for (r, bits) in fr.regs.iter().enumerate() {
                let v = match fr.kinds[r] {
                    RegBox::Dead | RegBox::Resolved => continue,
                    RegBox::Int => Value::int(*bits),
                    RegBox::Bool => Value::bool(*bits != 0),
                    RegBox::Float => Value::float(f64::from_bits(*bits as u64)),
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
        let Some(tier) = self.tier else { return NativeDisposition::Interpret };
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
                self.native[fi] = NativeSlot::Failed;
                return NativeDisposition::Interpret;
            }
            // A parameter whose declared type has no native representation
            // (Map, Text, nested Seq, …) can never enter native code —
            // fail once instead of bumping the hot counter forever.
            if f.param_kinds.iter().any(|k| k.is_none()) {
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
                            ret: g.ret_kind,
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
                        param_count: f.param_count,
                        register_count: f.register_count as u16,
                        param_kinds: f.param_kinds.clone(),
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
                f.param_count,
                f.register_count as u16,
                func,
                &f.param_kinds,
                f.ret_kind,
                &self.native_ctx,
                &callees,
            ) {
                Some(nf) => {
                    self.native_ctx.table.publish(fi, nf.entry_ptr(), nf.published_regc());
                    let name = self.fn_name(fi);
                    super::tier_trace::trace_transition(fi, &name, super::tier_trace::ExecTier::NativeForge);
                    self.native[fi] = NativeSlot::Ready(nf);
                }
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
                Some(ParamKind::Scalar(super::native_tier::SlotKind::Bool)) => {
                    match v.as_bool() {
                        Some(b) => b as i64,
                        None => return NativeDisposition::Interpret,
                    }
                }
                Some(ParamKind::Scalar(super::native_tier::SlotKind::Float)) => {
                    match v.as_float() {
                        Some(f) => f.to_bits() as i64,
                        None => return NativeDisposition::Interpret,
                    }
                }
                // List params ride the pin lane; the register slot is a
                // placeholder native code never reads as a scalar.
                Some(ParamKind::List(_)) => 0,
            };
        }
        // Pin list parameters: one borrow per DISTINCT Rc for the whole
        // call (recursion reuses the same pins via pass-through identity).
        // Empty lists retag in place to the declared element repr.
        let outcome = {
            use crate::interpreter::{ListRepr, RuntimeValue};
            use super::native_tier::PinElem;
            let mut handles: Vec<(usize, std::cell::RefMut<'_, ListRepr>)> = Vec::new();
            let mut pins: Vec<i64> = Vec::new();
            let mut pin_args: Vec<Value> = Vec::new();
            for (k, pk) in kinds.iter().enumerate().take(arg_count as usize) {
                let Some(ParamKind::List(elem)) = pk else { continue };
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
            let NativeSlot::Ready(nf) = &self.native[fi] else { unreachable!() };
            let args_slice = &args[..arg_count as usize];
            // The native callee occupies one LOGOS frame on top of the
            // bytecode stack; its self-calls count from there against
            // MAX_CALL_DEPTH.
            let out = nf.call(args_slice, &pins, bytecode_depth + 1);
            drop(handles);
            (out, pin_args, pins)
        };
        let (out, pin_args, pins_for_ret) = outcome;
        let NativeSlot::Ready(nf) = &self.native[fi] else { unreachable!() };
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
                NativeDisposition::Materialize { resume_pc, frames, list_args: pin_args }
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
        self
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
    /// [`Vm::run_until_block`] by the scheduler.
    pub fn run(&mut self) -> Result<(), String> {
        // Hermetic program start: no ambient exchange rates carried in from a prior run on this
        // thread (mirrors a fresh AOT process). The resumable `run_until_block` path is left alone so
        // rates installed mid-program survive across concurrency suspensions.
        logicaffeine_base::money::clear_ambient_rates();
        match self.run_until_block()? {
            VmStep::Done(_) => Ok(()),
            VmStep::Blocked => {
                Err("vm: concurrency op requires the scheduler driver".to_string())
            }
            VmStep::Paused => unreachable!("run_until_block (STEPPED = false) never pauses"),
        }
    }

    /// Run one slice: from a fresh start (or resumed from a prior block) until the
    /// program completes ([`VmStep::Done`]) or a concurrency op suspends it
    /// ([`VmStep::Blocked`], request in [`Vm::take_pending`]). Keeping `pc` and the
    /// call stack as loop-locals (restored once on entry, saved once on a block)
    /// leaves the hot dispatch path byte-for-byte identical to the old `run`.
    pub(crate) fn run_until_block(&mut self) -> Result<VmStep, String> {
        self.run_until_block_impl::<false>(u64::MAX)
    }

    /// Step the debug interpreter forward by at most `step_budget` ops, then pause
    /// ([`VmStep::Paused`], resumable on the next call). Used only by the Studio
    /// debug drawer; production callers use [`Vm::run_until_block`]
    /// (`STEPPED = false`), whose monomorphization elides every budget check — the
    /// hot dispatch path stays the byte-for-byte old loop.
    pub(crate) fn run_steps(&mut self, step_budget: u64) -> Result<VmStep, String> {
        self.run_until_block_impl::<true>(step_budget)
    }

    fn run_until_block_impl<const STEPPED: bool>(
        &mut self,
        step_budget: u64,
    ) -> Result<VmStep, String> {
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
            }
            let op = if pc < self.program.code.len() {
                self.program.code[pc]
            } else {
                self.warm_code[pc - self.program.code.len()]
            };
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
                Op::EnsureOwned { reg } => {
                    // Call-site copy-on-write barrier: isolate a shared collection
                    // before it is passed to a mutable-borrow callee. No-op when the
                    // register is a `mutable`-exempt param (its own writes COW) or the
                    // collection is already uniquely owned.
                    self.ensure_reg_owned(reg, call_stack.last().map(|f| f.func));
                    pc += 1;
                }
                Op::Add { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::add)?; pc += 1; }
                Op::AddAssign { dst, src } => { self.add_assign(dst, src)?; pc += 1; }
                Op::Sub { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::sub)?; pc += 1; }
                Op::Mul { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::mul)?; pc += 1; }
                Op::Div { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::div)?; pc += 1; }
                Op::ExactDiv { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::exact_div)?; pc += 1; }
                Op::FloorDiv { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::floor_div)?; pc += 1; }
                Op::Mod { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::modulo)?; pc += 1; }
                Op::DivPow2 { dst, lhs, k } => {
                    // `lhs / 2^k` (lhs is Oracle-proven Int) — identical result
                    // to `Op::Div` by `2^k`, so it matches the tree-walker.
                    let v = self.reg(lhs).div(&Value::int(1i64 << k))?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::MagicDivU { dst, lhs, magic, more, mul_back } => {
                    // `lhs / c` / `lhs % c` by the precomputed magic reciprocal
                    // (`magic`/`more`). Emitted only for an Oracle-proven Int,
                    // non-negative `lhs`, so the result is bit-identical to
                    // `Op::Div`/`Op::Mod` by the constant `c`.
                    let x = self.reg(lhs).as_int().ok_or_else(|| {
                        "MagicDivU on a non-Int operand".to_string()
                    })?;
                    let v = crate::vm::compiler::magic_eval(x, magic, more, mul_back);
                    self.set(dst, Value::int(v));
                    pc += 1;
                }
                Op::Lt { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::lt)?; pc += 1; }
                Op::Gt { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::gt)?; pc += 1; }
                Op::LtEq { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::lte)?; pc += 1; }
                Op::GtEq { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::gte)?; pc += 1; }
                Op::Eq { dst, lhs, rhs } => {
                    let v = self.reg(lhs).eq_op(self.reg(rhs));
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
                    let v = self.reg(lhs).neq_op(self.reg(rhs));
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Not { dst, src } => {
                    let v = self.reg(src).not_op()?;
                    self.set(dst, v);
                    pc += 1;
                }
                Op::Concat { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::concat)?; pc += 1; }
                Op::SeqConcat { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::seq_concat)?; pc += 1; }
                Op::Pow { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::pow)?; pc += 1; }
                Op::BitXor { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::bitxor)?; pc += 1; }
                Op::BitAnd { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::bitand)?; pc += 1; }
                Op::BitOr { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::bitor)?; pc += 1; }
                Op::Shl { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::shl)?; pc += 1; }
                Op::Shr { dst, lhs, rhs } => { self.binop(dst, lhs, rhs, Value::shr)?; pc += 1; }
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
                        match self.try_region(target, pc, named, frame_regs, call_stack.len(), cur_func)
                        {
                            Some(RegionExit::At(exit)) => {
                                pc = exit;
                                continue;
                            }
                            Some(RegionExit::Return(value)) => {
                                // The region hit a `Return` — perform the
                                // actual function return, exactly like the
                                // Op::Return arm.
                                let frame =
                                    call_stack.pop().ok_or("vm: return with no caller")?;
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
                    if !self.reg(cond).is_truthy() { pc = target; } else { pc += 1; }
                }
                Op::JumpIfTrue { cond, target } => {
                    if self.reg(cond).is_truthy() { pc = target; } else { pc += 1; }
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
                Op::MakeClosure { dst, func, locals_start } => {
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
                    let param_names = vec![crate::intern::Symbol::default(); f.param_count as usize];
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
                Op::CallValue { dst, callee, args_start, arg_count, name_for_err } => {
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
                        return_pc: pc + 1,
                        return_reg: dst,
                        caller_base: self.base,
                        restore_len,
                        iter_depth: self.iter_stack.len(),
                        func: closure.body_index as u16,
                        arg_lo: callee_base,
                        arg_count,
                    });
                    self.base = callee_base;
                    // Bind captures: value slots then present flags — both
                    // deep-cloned PER CALL (the tree-walker re-clones each
                    // invocation).
                    let cap_count = captures.len() as Reg;
                    for (k, (sym, _)) in captures.iter().enumerate() {
                        let (v, present) = match closure.captured_env.get(sym) {
                            Some(v) => (Value::from_runtime(v.deep_clone()), true),
                            None => (Value::nothing(), false),
                        };
                        self.set(param_count + k as Reg, v);
                        self.set(param_count + cap_count + k as Reg, Value::bool(present));
                    }
                    pc = entry_pc;
                }
                Op::CallBuiltin { dst, builtin, args_start, arg_count } => {
                    let mut args = Vec::with_capacity(arg_count as usize);
                    for k in 0..arg_count {
                        args.push(self.reg(args_start + k).as_runtime().clone());
                    }
                    let v = crate::semantics::builtins::call_builtin(builtin, args)?;
                    self.set(dst, Value::from_runtime(v));
                    self.clear_args(args_start, arg_count);
                    pc += 1;
                }
                Op::Call { dst, func, args_start, arg_count } => {
                    if call_stack.len() >= crate::semantics::MAX_CALL_DEPTH {
                        return Err(crate::semantics::CALL_DEPTH_ERR.to_string());
                    }
                    match self.try_native(func, args_start, arg_count, call_stack.len()) {
                        NativeDisposition::Done(v) => {
                            self.set(dst, v);
                            self.clear_args(args_start, arg_count);
                            pc += 1;
                            continue;
                        }
                        NativeDisposition::Interpret => {}
                        // Precise deopt: every native frame becomes a real
                        // CallFrame (registers re-boxed per the adapter's
                        // kind capture), then the interpreter resumes AT
                        // the faulting op — effects stay, the kernel
                        // raises the exact error from the exact state.
                        NativeDisposition::Materialize { resume_pc, frames, list_args } => {
                            self.materialize_native_frames(
                                &frames, &list_args, args_start, dst, func, pc, &mut call_stack,
                            )?;
                            pc = resume_pc;
                            continue;
                        }
                    }
                    // WS6 (Phase 13): the WASM-JIT tier — the browser JIT path, consulted
                    // here (behind the native forge tier above, which is absent on wasm32).
                    // A hot pure-integer function runs its emitted WebAssembly module instead
                    // of the bytecode; non-Int args or an ineligible body fall through.
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
                                pc += 1;
                                continue;
                            }
                        }
                    }
                    // Dispatch order (HOTSWAP §7): FnTable native (above) →
                    // warm_bytecode → baseline. A warm body runs from the unified
                    // pc space past `program.code`.
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
                    call_stack.push(CallFrame {
                        return_pc: pc + 1,
                        return_reg: dst,
                        caller_base: self.base,
                        restore_len,
                        iter_depth: self.iter_stack.len(),
                        func,
                        arg_lo: callee_base,
                        arg_count,
                    });
                    self.base = callee_base;
                    pc = entry_pc;
                }
                Op::Return { src } => {
                    let frame = call_stack.pop().ok_or("vm: return with no caller")?;
                    let rv = self.reg(src).clone();
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
                Op::NewEmptySet { dst } => { self.set(dst, Value::empty_set()); pc += 1; }
                Op::NewEmptyMap { dst } => { self.set(dst, Value::empty_map()); pc += 1; }
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
                    self.ensure_reg_owned(list, call_stack.last().map(|f| f.func));
                    self.reg(list).list_push(v)?;
                    pc += 1;
                }
                Op::SetAdd { set, value } => {
                    let v = self.reg(value).clone();
                    self.ensure_reg_owned(set, call_stack.last().map(|f| f.func));
                    self.reg(set).set_add(v)?;
                    pc += 1;
                }
                Op::RemoveFrom { collection, value } => {
                    self.ensure_reg_owned(collection, call_stack.last().map(|f| f.func));
                    self.reg(collection).remove_from(self.reg(value))?;
                    pc += 1;
                }
                Op::SetIndex { collection, index, value }
                | Op::SetIndexUnchecked { collection, index, value } => {
                    use crate::interpreter::RuntimeValue;
                    // Struct field set via index syntax (`Set item "f" of s to
                    // v`) — VALUE semantics, mirroring the tree-walker: clone
                    // the struct, insert, write the new struct back.
                    let is_struct_text = matches!(
                        (&*self.reg(collection).as_runtime(), &*self.reg(index).as_runtime()),
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
                        self.ensure_reg_owned(collection, call_stack.last().map(|f| f.func));
                        self.reg(collection).index_set(self.reg(index), v)?;
                    }
                    pc += 1;
                }
                // The bytecode interpreter checks both: a sound proof makes
                // the `IndexUnchecked` check never fire, so keeping it is
                // free defense-in-depth. Only the JIT elides.
                Op::Index { dst, collection, index }
                | Op::IndexUnchecked { dst, collection, index } => {
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
                Op::Contains { dst, collection, value } => {
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
                    crate::semantics::collections::push_to_struct_field(
                        &self.reg(obj).as_runtime(),
                        &field_name,
                        val,
                    )?;
                    pc += 1;
                }
                Op::CheckPolicy { subject, predicate, is_capability, object, source_text } => {
                    let (registry, interner) = match self.policy_ctx {
                        Some(ctx) => ctx,
                        None => {
                            return Err(
                                "Security Check requires policies. Use compiled Rust or add ## Policy block."
                                    .to_string(),
                            );
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
                Op::FormatValue { dst, src, spec, debug_prefix } => {
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
                            other => return Err(format!("vm: format spec is not Text: {:?}", other)),
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
                Op::SliceOp { dst, collection, start, end } => {
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
                    self.set(dst, Value::from_runtime(crate::semantics::temporal::today()));
                    pc += 1;
                }
                Op::LoadNow { dst } => {
                    self.set(dst, Value::from_runtime(crate::semantics::temporal::now()));
                    pc += 1;
                }
                Op::NewStruct { dst, type_name } => {
                    use crate::interpreter::{RuntimeValue, StructValue};
                    let name = match &self.program.constants[type_name as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: NewStruct name is not Text: {:?}", other)),
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
                Op::NewInductive { dst, type_name, ctor, args_start, count } => {
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
                        Value::from_runtime(RuntimeValue::Inductive(Box::new(InductiveValue {
                            inductive_type,
                            constructor,
                            args,
                        }))),
                    );
                    pc += 1;
                }
                Op::TestArm { dst, target, variant } => {
                    use crate::interpreter::RuntimeValue;
                    let variant_name = match &self.program.constants[variant as usize] {
                        Constant::Text(s) => s.as_str(),
                        other => return Err(format!("vm: variant name is not Text: {:?}", other)),
                    };
                    let matched = match &*self.reg(target).as_runtime() {
                        RuntimeValue::Struct(s) => s.type_name == variant_name,
                        RuntimeValue::Inductive(ind) => ind.constructor == variant_name,
                        _ => false,
                    };
                    self.set(dst, Value::bool(matched));
                    pc += 1;
                }
                Op::BindArm { dst, target, field, index } => {
                    use crate::interpreter::RuntimeValue;
                    let v = match &*self.reg(target).as_runtime() {
                        RuntimeValue::Struct(s) => {
                            let field_name = match &self.program.constants[field as usize] {
                                Constant::Text(s) => s.as_str(),
                                other => {
                                    return Err(format!("vm: field name is not Text: {:?}", other));
                                }
                            };
                            s.fields.get(field_name).cloned()
                        }
                        RuntimeValue::Inductive(ind) => ind.args.get(index as usize).cloned(),
                        _ => None,
                    };
                    if let Some(v) = v {
                        self.set(dst, Value::from_runtime(v));
                    }
                    pc += 1;
                }
                Op::CrdtBump { obj, field, amount, negate } => {
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
                    let amount_int = if negate { amount_int.wrapping_neg() } else { amount_int };
                    let field_name = match &self.program.constants[field as usize] {
                        Constant::Text(s) => s.clone(),
                        other => return Err(format!("vm: field name is not Text: {:?}", other)),
                    };
                    match self.reg_mut(obj).as_runtime_mut() {
                        RuntimeValue::Struct(s) => {
                            let current =
                                s.fields.get(&field_name).cloned().unwrap_or(RuntimeValue::Int(0));
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
                                let current =
                                    s.fields.get(&field_name).cloned().unwrap_or(RuntimeValue::Int(0));
                                let merged =
                                    crate::semantics::arith::crdt_merge_field(&current, incoming);
                                s.fields.insert(field_name, merged);
                            }
                        }
                        _ => return Err("Merge target must be a struct".to_string()),
                    }
                    pc += 1;
                }
                Op::NewCrdt { dst, kind } => {
                    use crate::interpreter::RuntimeValue;
                    use crate::semantics::crdt::{next_replica_id, CrdtValue};
                    let cv = match kind {
                        0 => CrdtValue::new_set(next_replica_id()),
                        1 => CrdtValue::new_seq(next_replica_id()),
                        3 => CrdtValue::new_set_remove_wins(next_replica_id()),
                        _ => CrdtValue::new_register(next_replica_id()),
                    };
                    self.set(
                        dst,
                        Value::from_runtime(RuntimeValue::Crdt(std::rc::Rc::new(
                            std::cell::RefCell::new(cv),
                        ))),
                    );
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
                        self.set(dst, v);
                        pc += 1;
                    } else {
                        pc = exit;
                    }
                }
                Op::IterPop => {
                    self.iter_stack.pop().ok_or("vm: IterPop with no live iterator")?;
                    pc += 1;
                }
                Op::ListPop { list, dst } => {
                    self.ensure_reg_owned(list, call_stack.last().map(|f| f.func));
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
                    return Ok(self.block(pc + 1, call_stack, VmBlock::NewChan(capacity), Some(dst)));
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
                    return Ok(self.block(pc + 1, call_stack, VmBlock::TrySend(ch, payload), Some(dst)));
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
                Op::Spawn { func, args_start, arg_count } => {
                    let args = self.materialize_args(args_start, arg_count)?;
                    let req = VmBlock::SpawnDesc { func, args, want_handle: false };
                    return Ok(self.block(pc + 1, call_stack, req, None));
                }
                Op::SpawnHandle { dst, func, args_start, arg_count } => {
                    let args = self.materialize_args(args_start, arg_count)?;
                    let req = VmBlock::SpawnDesc { func, args, want_handle: true };
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
                    return Ok(self.block(pc + 1, call_stack, VmBlock::Select(arms), Some(dst_arm)));
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
                    return Ok(self.block(pc + 1, call_stack, VmBlock::NetAwait(f, stream), Some(dst)));
                }
                Op::NetMakePeer { dst, addr } => {
                    let a = self.materialize_reg(addr)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::NetMakePeer(a), Some(dst)));
                }
                Op::NetSync { dst, topic } => {
                    let current = self.materialize_reg(dst)?;
                    let t = self.materialize_reg(topic)?;
                    return Ok(self.block(pc + 1, call_stack, VmBlock::NetSync(t, current), Some(dst)));
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
        let cs = &self.sched_call_stack;
        let mut frames = Vec::with_capacity(cs.len() + 1);
        frames.push(self.frame_view(None, 0, prog.register_count));
        for (k, fr) in cs.iter().enumerate() {
            let base = cs.get(k + 1).map(|n| n.caller_base).unwrap_or(self.base);
            let count = prog.functions.get(fr.func as usize).map(|f| f.register_count).unwrap_or(0);
            frames.push(self.frame_view(Some(fr.func), base, count));
        }
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
            current_func: cs.last().map(|f| f.func),
            frames,
            globals,
            output: self.lines.clone(),
        }
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
        DebugFrameView { func, base, registers }
    }

    /// Walk the roots (the current frame's registers + the globals) and collect the
    /// distinct heap objects they reach, recording each object's reference count and
    /// every root that points at it — so a shared list shows up once with two
    /// referrers (the `let b be a` aliasing that trips up every beginner).
    pub(crate) fn debug_heap(&self) -> Vec<HeapObjView> {
        let prog = self.program;
        let (count, is_main) = match self.sched_call_stack.last() {
            Some(fr) => (
                prog.functions.get(fr.func as usize).map(|f| f.register_count).unwrap_or(0),
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
        let mut objs: Vec<HeapObjView> = Vec::new();
        let mut add = |v: &Value, root: String, objs: &mut Vec<HeapObjView>| {
            if let Some((id, kind, rc, storage)) = heap_identity(v) {
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
            self.set(slot, value);
        }
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
            self.set(slot, Value::from_runtime(crate::interpreter::RuntimeValue::Int(arm as i64)));
        }
        self.select_pending.clear();
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
            other => Err(format!("expected a task handle, found {}", other.type_name())),
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
        (0..arg_count as Reg).map(|i| self.materialize_reg(args_start + i)).collect()
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
            self.registers[i] = Value::from_runtime(crate::concurrency::marshal::rebuild(a.clone()));
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
        child.program_args = self.program_args.clone();
        child.setup_task(func, args);
        child
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
        Constant::Span { months, days } => {
            Value::from_runtime(RuntimeValue::Span { months: *months, days: *days })
        }
        Constant::Time(n) => Value::from_runtime(RuntimeValue::Time(*n)),
    }
}

#[cfg(test)]
mod string_build_fastpath {
    //! Structural proof for Task B: the constant pool is materialised ONCE
    //! (a `LoadConst` of a Text bumps a shared `Rc` instead of allocating a
    //! fresh `String`), and the sole-owned in-place append (`AddAssign`)
    //! grows the accumulator's own buffer rather than reallocating each step.
    use super::*;
    use crate::interpreter::RuntimeValue;
    use std::rc::Rc;

    /// The `Rc<String>` backing register `r` (or panic if it isn't a Text).
    fn text_ptr(vm: &Vm, r: usize) -> *const String {
        match vm.registers[r].as_runtime_ref() {
            Some(RuntimeValue::Text(rc)) => Rc::as_ptr(rc),
            other => panic!("register {r} is not a Text: {other:?}"),
        }
    }

    fn text_strong(vm: &Vm, r: usize) -> usize {
        match vm.registers[r].as_runtime_ref() {
            Some(RuntimeValue::Text(rc)) => Rc::strong_count(rc),
            other => panic!("register {r} is not a Text: {other:?}"),
        }
    }

    /// Two `LoadConst`s of the SAME Text-constant index hand out the same
    /// `Rc` allocation — proof the pool is materialised once, so reloading a
    /// literal in a loop is a refcount bump, not a `String` allocation.
    #[test]
    fn loadconst_text_shares_one_allocation() {
        let prog = CompiledProgram {
            constants: vec![Constant::Text("abc".to_string())],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 0 },
                Op::Halt,
            ],
            register_count: 2,
            ..Default::default()
        };
        let mut vm = Vm::new(&prog);
        vm.run().unwrap();
        assert_eq!(
            text_ptr(&vm, 0),
            text_ptr(&vm, 1),
            "two loads of the same Text constant must share one Rc allocation"
        );
        // And the live constant keeps a reference, so a register's clone is
        // never the sole owner — an in-place append on a freshly-loaded
        // literal must therefore NOT fire (it would corrupt the pool).
        assert!(text_strong(&vm, 0) >= 3, "pool + two registers all reference the constant");
    }

    /// `Set s to s + ch` repeated: once the accumulator owns its buffer, each
    /// append reuses the SAME allocation (pointer stable while capacity holds).
    /// The first append, where `s` is the just-loaded shared `""` constant,
    /// must allocate a fresh owned buffer (the pool stays intact); subsequent
    /// appends grow it in place.
    #[test]
    fn add_assign_appends_in_place_after_first() {
        // r0 = "" (shared constant), r1 = "x" (shared constant).
        // r0 += r1, capture; r0 += r1 a few more times, pointer stays put.
        let prog = CompiledProgram {
            constants: vec![Constant::Text(String::new()), Constant::Text("x".to_string())],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::AddAssign { dst: 0, src: 1 },
                Op::AddAssign { dst: 0, src: 1 },
                Op::AddAssign { dst: 0, src: 1 },
                Op::AddAssign { dst: 0, src: 1 },
                Op::Halt,
            ],
            register_count: 2,
            ..Default::default()
        };
        // Run only the first two appends to capture the owned buffer, then the
        // rest, by stepping the whole thing and re-checking — simplest: run to
        // completion, the invariant we assert is the FINAL value's correctness
        // plus that the rhs constant was never mutated.
        let mut vm = Vm::new(&prog);
        vm.run().unwrap();
        // The accumulator built "xxxx".
        match vm.registers[0].as_runtime_ref() {
            Some(RuntimeValue::Text(rc)) => assert_eq!(&***rc, "xxxx"),
            other => panic!("r0 not text: {other:?}"),
        }
        // The shared constant "x" was NOT corrupted by the in-place appends.
        match vm.registers[1].as_runtime_ref() {
            Some(RuntimeValue::Text(rc)) => assert_eq!(&***rc, "x"),
            other => panic!("r1 not text: {other:?}"),
        }
    }

    /// The buffer-stability proof: drive the loop manually so we can read the
    /// accumulator's `Rc::as_ptr` between appends. After the first append
    /// (which clones off the shared constant into an owned buffer with spare
    /// capacity), every later append keeps the SAME allocation.
    #[test]
    fn add_assign_reuses_buffer_allocation() {
        let prog = CompiledProgram {
            constants: vec![Constant::Text("seed-with-capacity-headroom".to_string()), Constant::Text("z".to_string())],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::AddAssign { dst: 0, src: 1 }, // first: clone off the shared constant
                Op::AddAssign { dst: 0, src: 1 }, // owned now → in place
                Op::AddAssign { dst: 0, src: 1 },
                Op::Halt,
            ],
            register_count: 2,
            ..Default::default()
        };
        // Manually dispatch up to the marker so we can inspect between appends.
        // Reserve generous capacity on the first owned buffer so growth never
        // forces a realloc within this test.
        let mut vm = Vm::new(&prog);
        // Step 1+2: loads.
        vm.set(0, const_to_value(&prog.constants[0]));
        vm.set(1, const_to_value(&prog.constants[1]));
        // First append: must produce an OWNED buffer (sole owner now).
        vm.add_assign(0, 1).unwrap();
        // Force headroom so the next appends don't realloc for capacity.
        if let RuntimeValue::Text(rc) = vm.registers[0].as_runtime_mut() {
            if let Some(s) = Rc::get_mut(rc) {
                s.reserve(64);
            }
        }
        let p_after_first = text_ptr(&vm, 0);
        vm.add_assign(0, 1).unwrap();
        let p_after_second = text_ptr(&vm, 0);
        vm.add_assign(0, 1).unwrap();
        let p_after_third = text_ptr(&vm, 0);
        assert_eq!(p_after_first, p_after_second, "append must reuse the owned buffer");
        assert_eq!(p_after_second, p_after_third, "append must reuse the owned buffer");
        match vm.registers[0].as_runtime_ref() {
            Some(RuntimeValue::Text(rc)) => assert_eq!(&***rc, "seed-with-capacity-headroomzzz"),
            other => panic!("r0 not text: {other:?}"),
        }
    }
}

#[cfg(test)]
mod debug_stepping {
    //! The debug stepper (`STEPPED = true`): `run_steps` advances exactly one op
    //! per call, yields `Paused` between ops, exposes the paused state via
    //! `debug_view`, and — the soundness invariant — produces output BYTE-IDENTICAL
    //! to a single-shot `run()`. This is the contract the Studio debug drawer rides.
    use super::*;

    /// `Let x be 6. Let y be 7. Show x times y.` hand-lowered to bytecode.
    fn mul_program() -> CompiledProgram {
        CompiledProgram {
            constants: vec![Constant::Int(6), Constant::Int(7)],
            code: vec![
                Op::LoadConst { dst: 0, idx: 0 },
                Op::LoadConst { dst: 1, idx: 1 },
                Op::Mul { dst: 2, lhs: 0, rhs: 1 },
                Op::Show { src: 2 },
                Op::Halt,
            ],
            register_count: 3,
            ..Default::default()
        }
    }

    #[test]
    fn run_steps_advances_one_op_and_pauses() {
        let prog = mul_program();
        let mut vm = Vm::new(&prog);
        // First op (LoadConst R0 = 6) → paused at pc 1 with R0 visible.
        assert!(matches!(vm.run_steps(1).unwrap(), VmStep::Paused));
        let v = vm.debug_view();
        assert_eq!(v.pc, 1, "stopped before the second instruction");
        assert_eq!(v.frames.len(), 1, "single Main frame");
        assert_eq!(v.frames[0].registers[0], (0u16, "Int".to_string(), "6".to_string()));
        // Second op (LoadConst R1 = 7).
        assert!(matches!(vm.run_steps(1).unwrap(), VmStep::Paused));
        let v = vm.debug_view();
        assert_eq!(v.pc, 2);
        assert_eq!(v.frames[0].registers[1], (1u16, "Int".to_string(), "7".to_string()));
        // Third op (Mul R2 = R0 * R1).
        assert!(matches!(vm.run_steps(1).unwrap(), VmStep::Paused));
        assert_eq!(vm.debug_view().frames[0].registers[2], (2u16, "Int".to_string(), "42".to_string()));
    }

    #[test]
    fn stepped_run_is_byte_identical_to_single_shot() {
        let prog = mul_program();

        // Stepped to completion, one op at a time.
        let mut stepper = Vm::new(&prog);
        let mut pauses = 0usize;
        loop {
            match stepper.run_steps(1).unwrap() {
                VmStep::Paused => pauses += 1,
                VmStep::Done(_) => break,
                VmStep::Blocked => unreachable!("no concurrency op in this program"),
            }
        }
        let stepped_out = stepper.into_lines();

        // Single-shot run of the very same program.
        let mut oneshot = Vm::new(&prog);
        oneshot.run().unwrap();
        let oneshot_out = oneshot.into_lines();

        assert_eq!(stepped_out, oneshot_out, "stepping must not change observable output");
        assert_eq!(stepped_out, vec!["42".to_string()]);
        assert_eq!(pauses, 4, "one pause after each of the 4 ops before Halt");
    }

    #[test]
    fn larger_budget_runs_several_ops_then_pauses() {
        let prog = mul_program();
        let mut vm = Vm::new(&prog);
        // Budget of 3 runs the two loads + the Mul, then pauses at the Show (pc 3).
        assert!(matches!(vm.run_steps(3).unwrap(), VmStep::Paused));
        let v = vm.debug_view();
        assert_eq!(v.pc, 3);
        assert_eq!(v.frames[0].registers[2], (2u16, "Int".to_string(), "42".to_string()));
        // The rest completes.
        assert!(matches!(vm.run_steps(u64::MAX).unwrap(), VmStep::Done(_)));
        assert_eq!(vm.into_lines(), vec!["42".to_string()]);
    }
}
