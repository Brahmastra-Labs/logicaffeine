//! R6 — the Tier-5 **Silicon ISA emitter**. The register-VM opcode set becomes the native ISA; each
//! stencil family maps to a functional unit. This lowers a
//! [`CompiledProgram`] to LOGOS Silicon ISA assembly — the SAME opcodes the interpreter/VM/JIT/AOT
//! run, now as the chip's instruction set.
//!
//! Every emitted instruction carries its [`RevClass`] — the adiabatic datapath contract:
//!   • `Bijection` instructions run backward with ZERO garbage (the charge-recovery core);
//!   • `Journaled` instructions push a minimal Bennett delta;
//!   • `Effectful` instructions hit the effect log / a named irreversibility barrier;
//!   • `Control` instructions reverse on the recorded trace.
//!
//! Because the classifier [`reversibility`] is TOTAL over the ISA (no `NonReversible`, enforced by
//! `reverse_op_lock`), the emitter can never lower an unclassified — hence non-reversible-by-omission —
//! instruction. The `isa_aot_lock` test pins that: every op emits, and every emission is classified.

use crate::vm::instruction::{CompiledProgram, Op};
use crate::vm::reverse::{reversibility, EffectKind, InverseSpec, JournalKind, RevClass};

/// The reversibility-contract tag for one ISA instruction — the adiabatic annotation the Silicon
/// backend reads to decide whether an instruction charge-recovers (Bijection) or needs a history cell.
pub fn rev_tag(op: &Op) -> String {
    match reversibility(op) {
        RevClass::Bijection(spec) => {
            let inv = match spec {
                InverseSpec::Involution => "self".to_string(),
                InverseSpec::SelfFamily => "self-family".to_string(),
                InverseSpec::NamedOp(n) => n.to_string(),
            };
            format!("REV   zero-garbage, inverse={inv}  (adiabatic charge-recovery)")
        }
        RevClass::Journaled(k) => {
            let kind = match k {
                JournalKind::Overwrite => "overwrite",
                JournalKind::Collection => "collection",
                JournalKind::InPlace => "in-place",
                JournalKind::CrdtSnapshot => "crdt-snapshot",
            };
            format!("JRNL  Bennett delta ({kind})")
        }
        RevClass::Effectful(e) => {
            let kind = match e {
                EffectKind::Output => "output-buffer",
                EffectKind::Clock => "clock-capture",
                EffectKind::Timer => "virtual-time",
                EffectKind::Concurrency => "sched-trace",
                EffectKind::Network => "IRREVERSIBLE-BARRIER (network)",
            };
            format!("EFF   {kind}")
        }
        RevClass::Control => "CTRL  reverse on trace".to_string(),
        RevClass::NonReversible(why) => format!("!!!!  UNCLASSIFIED: {why}"),
    }
}

/// The mnemonic + operands for one instruction. The reversible-core ops surface their group operands
/// (place / delta) since those ARE the adiabatic datapath; the rest carry their opcode name.
fn mnemonic(op: &Op) -> String {
    match op {
        Op::RevAdd { place, delta } => format!("REVADD  r{place}, r{delta}"),
        Op::RevSub { place, delta } => format!("REVSUB  r{place}, r{delta}"),
        Op::RevXor { place, delta } => format!("REVXOR  r{place}, r{delta}"),
        Op::RevNeg { place } => format!("REVNEG  r{place}"),
        Op::RevSwap { a, b } => format!("REVSWAP r{a}, r{b}"),
        Op::LoadConst { dst, idx } => format!("LOADK   r{dst}, k{idx}"),
        Op::Move { dst, src } => format!("MOV     r{dst}, r{src}"),
        Op::Show { .. } => "SHOW".to_string(),
        Op::Jump { target } => format!("JMP     L{target}"),
        Op::Halt => "HALT".to_string(),
        other => other.name().to_uppercase(),
    }
}

/// Emit `prog` as LOGOS Silicon ISA assembly text, every instruction annotated with its reversibility
/// class. Totality is guaranteed by the classifier: an unclassified op cannot compile the crate.
pub fn emit_isa(prog: &CompiledProgram) -> String {
    let mut s = String::new();
    s.push_str("; ============================================================================\n");
    s.push_str(";  LOGOS Silicon ISA — Tier 5  (register-VM opcode set = native chip ISA)\n");
    s.push_str(
        ";  MAKE_A_CHIP §10.1 — each instruction annotated with its reversibility contract.\n",
    );
    s.push_str(
        "; ============================================================================\n\n",
    );

    // A reversibility budget summary: how much of the program is zero-garbage (adiabatic) vs journaled.
    let (mut bij, mut jrnl, mut eff, mut ctrl) = (0usize, 0usize, 0usize, 0usize);
    for op in &prog.code {
        match reversibility(op) {
            RevClass::Bijection(_) => bij += 1,
            RevClass::Journaled(_) => jrnl += 1,
            RevClass::Effectful(_) => eff += 1,
            RevClass::Control => ctrl += 1,
            RevClass::NonReversible(_) => {}
        }
    }
    s.push_str(&format!(
        ";  reversibility budget: {bij} zero-garbage / {jrnl} journaled / {eff} effect / {ctrl} control\n\n"
    ));

    s.push_str(".text\n");
    for (i, op) in prog.code.iter().enumerate() {
        s.push_str(&format!(
            "  L{i:<4} {:<24} ; {}\n",
            mnemonic(op),
            rev_tag(op)
        ));
    }

    if !prog.functions.is_empty() {
        s.push_str("\n.functions\n");
        for (fi, f) in prog.functions.iter().enumerate() {
            s.push_str(&format!(
                "  fn{fi}: entry=L{}  params={}  regs={}\n",
                f.entry_pc, f.param_count, f.register_count
            ));
        }
    }

    // The machine-checkable adiabatic contract: what fraction of the datapath charge-recovers.
    let sched = adiabatic_schedule(prog);
    s.push_str(&format!(
        "\n.adiabatic\n  charge-recovery cells: {} ({:.0}%)  |  ancilla (history) cells: {}  |  effect: {}  |  control: {}\n",
        sched.charge_recovery,
        sched.charge_recovery_fraction() * 100.0,
        sched.ancilla,
        sched.effect,
        sched.control,
    ));
    s
}

/// The physical cell a Silicon-ISA instruction maps to — the structured (machine-checkable) form of
/// the `rev_tag` annotation, one per [`RevClass`]. This is the datapath's adiabatic contract:
/// `ChargeRecovery` cells recycle their charge (zero erased bits); `AncillaWrite` cells write a
/// Bennett history bit (the only genuine erased-bit source); the rest carry no erased data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CellClass {
    /// A `Bijection` op — a reversible (adiabatic charge-recovery) functional unit, ZERO erased bits.
    ChargeRecovery,
    /// A `Journaled` op — a Bennett history (ancilla) cell; the only cell that can erase information.
    AncillaWrite,
    /// An `Effectful` op — an effect-log cell (or, for network, a named irreversibility barrier).
    EffectLog,
    /// A `Control` op — reverses on the recorded trace; no datapath cell, no erased data.
    ControlTrace,
}

/// The cell an op maps to — derived from the TOTAL [`reversibility`] classifier, so it is exhaustive
/// by construction (the census `NonReversible` is provably empty, mapped conservatively to an ancilla
/// cell). Keep in lockstep with [`rev_tag`].
pub fn cell_class(op: &Op) -> CellClass {
    match reversibility(op) {
        RevClass::Bijection(_) => CellClass::ChargeRecovery,
        RevClass::Journaled(_) => CellClass::AncillaWrite,
        RevClass::Effectful(_) => CellClass::EffectLog,
        RevClass::Control => CellClass::ControlTrace,
        // The census is empty (locked); a hypothetical unclassified op is worst-cased as erasing.
        RevClass::NonReversible(_) => CellClass::AncillaWrite,
    }
}

/// The adiabatic cell budget of a program — the count of each [`CellClass`] its Silicon datapath needs.
/// The `charge_recovery` fraction is the adiabatic (zero-erasure) share; a program with no ancilla or
/// effect cells is FULLY ADIABATIC — its Landauer floor is 0, since no cell can erase a bit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdiabaticSchedule {
    pub charge_recovery: usize,
    pub ancilla: usize,
    pub effect: usize,
    pub control: usize,
}

impl AdiabaticSchedule {
    /// FULLY ADIABATIC iff the datapath needs no history (ancilla) cell and hits no effect — every
    /// instruction charge-recovers or reverses on the trace, so nothing can erase a bit. A *sufficient*
    /// static condition for a Landauer-0 run (the dynamic floor is [`crate::vm::journal::HistoryTape::landauer_bytes`]).
    pub fn is_fully_adiabatic(&self) -> bool {
        self.ancilla == 0 && self.effect == 0
    }

    /// The fraction of instructions on the zero-erasure charge-recovery datapath (`1.0` for empty).
    pub fn charge_recovery_fraction(&self) -> f64 {
        let total = self.charge_recovery + self.ancilla + self.effect + self.control;
        if total == 0 {
            1.0
        } else {
            self.charge_recovery as f64 / total as f64
        }
    }
}

/// Compute the [`AdiabaticSchedule`] — the per-[`CellClass`] cell budget — for a program.
pub fn adiabatic_schedule(prog: &CompiledProgram) -> AdiabaticSchedule {
    let mut s = AdiabaticSchedule::default();
    for op in &prog.code {
        match cell_class(op) {
            CellClass::ChargeRecovery => s.charge_recovery += 1,
            CellClass::AncillaWrite => s.ancilla += 1,
            CellClass::EffectLog => s.effect += 1,
            CellClass::ControlTrace => s.control += 1,
        }
    }
    s
}

/// Where each ISA op lives in silicon — the FULL-SURFACE map. Every op has a home, so the WHOLE
/// language is silicon-implementable: the realized reversible datapath, the ALU/value datapath (the
/// build frontier), the instruction sequencer, the load/store + structured-memory unit, or an
/// off-core coprocessor (runtime services). This is an EXHAUSTIVE match with no wildcard — a new `Op`
/// cannot compile until it declares its silicon home, so the chip can never silently fail to cover a
/// language feature. `CoreDatapath` is a monotone floor that RISES as datapath ops are realized in
/// RTL and proven ≡ VM by our certified SAT (the `silicon_op_support_ratchet` lock).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SiliconStatus {
    /// Realized TODAY in `rev_alu_core.v` — the abelian charge-recovery reversible datapath (wire opcodes 0..4).
    CoreDatapath,
    /// A register/value ALU op (arithmetic, bitwise, comparison, moves, constants) — belongs on the core
    /// datapath; the frontier being grown from `CoreDatapath` op-by-op.
    Datapath,
    /// The instruction sequencer — branch / call / return / halt / closure-make; the fetch-decode-PC front-end.
    Sequencer,
    /// The load/store + structured-memory unit — collections, structs, enums, tuples, indexing, slicing, iteration, globals.
    LoadStore,
    /// An off-core COPROCESSOR / runtime service — I/O, time, policy, CRDTs, concurrency, networking, kernel builtins.
    Coprocessor,
}

/// Whether an op is REALIZED in synthesizable silicon TODAY — it has emitted RTL AND its operation is
/// proven correct by our own certified SAT (not merely classified). This is the ratchet's PROGRESS
/// metric; it grows as `Datapath` ops are turned into verified hardware. Two realized units:
/// the reversible charge-recovery core (`rev_alu_core.v`; each op proven a bijection/injective) and
/// the general integer ALU (`codegen_sva::rtl::emit_alu_datapath_verilog`; arithmetic/bitwise/signed-
/// comparison ops proven correct by the our-SAT identity battery `the_general_alu_datapath_is_proven
/// _correct_by_our_sat`). RAISE `SILICON_REALIZED_FLOOR` as ops graduate — never lower it.
pub fn silicon_realized(op: &Op) -> bool {
    matches!(
        op,
        // reversible charge-recovery core (wire opcodes 0..4) — proven injective by our SAT
        Op::RevAdd { .. }
            | Op::RevSub { .. }
            | Op::RevXor { .. }
            | Op::RevNeg { .. }
            | Op::RevSwap { .. }
            // general integer ALU — combinational; proven correct by the our-SAT identity battery
            | Op::Add { .. }
            | Op::AddAssign { .. }
            | Op::Sub { .. }
            | Op::Mul { .. }
            | Op::Pow { .. }
            | Op::Div { .. }
            | Op::ExactDiv { .. }
            | Op::Mod { .. }
            | Op::FloorDiv { .. }
            | Op::DivPow2 { .. }
            | Op::MagicDivU { .. }
            | Op::BitXor { .. }
            | Op::BitAnd { .. }
            | Op::BitOr { .. }
            | Op::Not { .. }
            | Op::Shl { .. }
            | Op::Shr { .. }
            | Op::Lt { .. }
            | Op::Gt { .. }
            | Op::LtEq { .. }
            | Op::GtEq { .. }
            | Op::Eq { .. }
            | Op::ApproxEq { .. }
            | Op::NotEq { .. }
            | Op::EnsureOwned { .. }
            // sequencer + movement ops realized in the soft-core CPU (`soft_core.v`) — see `soft_core_executes`
            | Op::Move { .. }
            | Op::LoadConst { .. }
            | Op::Jump { .. }
            | Op::JumpIfFalse { .. }
            | Op::JumpIfTrue { .. }
            | Op::Halt
            | Op::Show { .. }
            | Op::Call { .. }
            | Op::CallValue { .. }
            | Op::MakeClosure { .. }
            | Op::Return { .. }
            | Op::ReturnNothing
            | Op::FailWith { .. }
            // load-store unit (the heap in `soft_core.v` + the ≡VM differential) — see `load_store_realized`
            | Op::NewList { .. }
            | Op::NewEmptyList { .. }
            | Op::NewEmptyListFixed { .. }
            | Op::NewEmptyListI32 { .. }
            | Op::NewEmptyMap { .. }
            | Op::ListPush { .. }
            | Op::Index { .. }
            | Op::IndexUnchecked { .. }
            | Op::Length { .. }
            | Op::FormatValue { .. }
            | Op::NewTuple { .. }
            | Op::ListPop { .. }
            | Op::Contains { .. }
            | Op::DestructureTuple { .. }
            | Op::NewStruct { .. }
            | Op::StructInsert { .. }
            | Op::GetField { .. }
            | Op::SetIndex { .. }
            | Op::SetIndexUnchecked { .. }
            | Op::SeqConcat { .. }
            | Op::Concat { .. }
            | Op::DeepClone { .. }
            | Op::IterPrepare { .. }
            | Op::IterNext { .. }
            | Op::IterPop
            | Op::RegionBoundsGuard { .. }
            | Op::NewInductive { .. }
            | Op::TestArm { .. }
            | Op::BindArm { .. }
            | Op::NewEmptySet { .. }
            | Op::SetAdd { .. }
            | Op::NewRange { .. }
            | Op::UnionOp { .. }
            | Op::IntersectOp { .. }
            | Op::RemoveFrom { .. }
            | Op::SliceOp { .. }
            | Op::ListPushField { .. }
            | Op::GlobalGet { .. }
            | Op::GlobalSet { .. }
            | Op::Args { .. }
            // concurrency coprocessor (the FIFO-channel NoC in `soft_core.v`) — see `coprocessor_realized`
            | Op::ChanNew { .. }
            | Op::ChanSend { .. }
            | Op::ChanRecv { .. }
            | Op::Spawn { .. }
            | Op::CrdtBump { .. }
            | Op::CrdtMerge { .. }
            | Op::CrdtJoin { .. }
            | Op::BigIntWide { .. }
            | Op::NewCrdt { .. }
            | Op::CrdtAppend { .. }
            | Op::CrdtResolve { .. }
            | Op::SelectArmRecv { .. }
            | Op::SelectArmTimeout { .. }
            | Op::SelectWait { .. }
            | Op::ChanTrySend { .. }
            | Op::ChanTryRecv { .. }
            | Op::Sleep { .. }
            | Op::LoadToday { .. }
            | Op::LoadNow { .. }
            | Op::Obs { .. }
            | Op::SpawnHandle { .. }
            | Op::TaskAbort { .. }
            | Op::TaskAwait { .. }
            | Op::ChanClose { .. }
            | Op::CallBuiltin { .. }
            | Op::CheckPolicy { .. }
            // offline networking coprocessor (single-node no-ops in `soft_core.v`) — see `coprocessor_realized`
            | Op::NetConnect { .. }
            | Op::NetListen { .. }
            | Op::NetSend { .. }
            | Op::NetStream { .. }
            | Op::NetAwait { .. }
            | Op::NetMakePeer { .. }
            | Op::NetSync { .. }
    )
}

/// The concurrency/coprocessor ops realized as on-core services in `soft_core.v`: the FIFO-channel NoC
/// (a fixed ring buffer on the heap), proven by the ≡VM differential (buffered send/receive is byte-identical
/// to the VM's deterministic pipe) + iverilog, decoded at their `binisa` wire opcodes.
pub fn coprocessor_realized(op: &Op) -> bool {
    matches!(
        op,
        Op::ChanNew { .. }
            | Op::ChanSend { .. }
            | Op::ChanRecv { .. }
            | Op::Spawn { .. }
            | Op::CrdtBump { .. }
            | Op::CrdtMerge { .. }
            | Op::CrdtJoin { .. }
            | Op::BigIntWide { .. }
            | Op::NewCrdt { .. }
            | Op::CrdtAppend { .. }
            | Op::CrdtResolve { .. }
            | Op::SelectArmRecv { .. }
            | Op::SelectArmTimeout { .. }
            | Op::SelectWait { .. }
            | Op::ChanTrySend { .. }
            | Op::ChanTryRecv { .. }
            | Op::Sleep { .. }
            | Op::LoadToday { .. }
            | Op::LoadNow { .. }
            | Op::Obs { .. }
            | Op::SpawnHandle { .. }
            | Op::TaskAbort { .. }
            | Op::TaskAwait { .. }
            | Op::ChanClose { .. }
            | Op::CallBuiltin { .. }
            | Op::CheckPolicy { .. }
            | Op::NetConnect { .. }
            | Op::NetListen { .. }
            | Op::NetSend { .. }
            | Op::NetStream { .. }
            | Op::NetAwait { .. }
            | Op::NetMakePeer { .. }
            | Op::NetSync { .. }
    )
}

/// The ops the fetch-decode-execute soft-core (`benchmarks/silicon/soft_core.v`) itself decodes and
/// executes — the movement + control-flow + output ops (as opposed to the value-transform ALU ops
/// proven by our-SAT identities). Realized by the CPU running real bytecode (iverilog-proven) and by
/// the `soft_core_decodes_the_wire_opcodes` lock tying its decode to `binisa::opcode`.
pub fn soft_core_executes(op: &Op) -> bool {
    matches!(
        op,
        Op::Move { .. }
            | Op::LoadConst { .. }
            | Op::Jump { .. }
            | Op::JumpIfFalse { .. }
            | Op::JumpIfTrue { .. }
            | Op::Halt
            | Op::Show { .. }
            | Op::Call { .. }
            | Op::CallValue { .. }
            | Op::MakeClosure { .. }
            | Op::Return { .. }
            | Op::ReturnNothing
            | Op::FailWith { .. }
    )
}

/// The load-store ops realized in the soft-core's heap unit (`soft_core.v`'s `dmem`/`hp` + the
/// `run_soft_core` model): sequence construction, push, indexed read, and length. Proven by the
/// `the_soft_core_runs_logos_programs_identically_to_the_vm` differential (heap ≡ VM `RuntimeValue::List`)
/// and decoded at their `binisa` wire opcodes by the `soft_core_decodes_the_wire_opcodes` lock.
pub fn load_store_realized(op: &Op) -> bool {
    matches!(
        op,
        Op::NewList { .. }
            | Op::NewEmptyList { .. }
            | Op::NewEmptyListFixed { .. }
            | Op::NewEmptyListI32 { .. }
            | Op::NewEmptyMap { .. }
            | Op::ListPush { .. }
            | Op::Index { .. }
            | Op::IndexUnchecked { .. }
            | Op::Length { .. }
            | Op::FormatValue { .. }
            | Op::NewTuple { .. }
            | Op::ListPop { .. }
            | Op::Contains { .. }
            | Op::DestructureTuple { .. }
            | Op::NewStruct { .. }
            | Op::StructInsert { .. }
            | Op::GetField { .. }
            | Op::SetIndex { .. }
            | Op::SetIndexUnchecked { .. }
            | Op::SeqConcat { .. }
            | Op::Concat { .. }
            | Op::DeepClone { .. }
            | Op::IterPrepare { .. }
            | Op::IterNext { .. }
            | Op::IterPop
            | Op::RegionBoundsGuard { .. }
            | Op::NewInductive { .. }
            | Op::TestArm { .. }
            | Op::BindArm { .. }
            | Op::NewEmptySet { .. }
            | Op::SetAdd { .. }
            | Op::NewRange { .. }
            | Op::UnionOp { .. }
            | Op::IntersectOp { .. }
            | Op::RemoveFrom { .. }
            | Op::SliceOp { .. }
            | Op::ListPushField { .. }
            | Op::GlobalGet { .. }
            | Op::GlobalSet { .. }
            | Op::Args { .. }
    )
}

/// The silicon home of an op — EXHAUSTIVE (no wildcard), so the full ISA is provably mapped to
/// hardware and a new op must declare its home. Keep the `CoreDatapath` arm in lockstep with the
/// realized `rev_alu_core.v` datapath (`binisa` opcodes 0..4).
pub fn silicon_status(op: &Op) -> SiliconStatus {
    use SiliconStatus::*;
    match op {
        Op::RevAdd { .. }
        | Op::RevSub { .. }
        | Op::RevXor { .. }
        | Op::RevNeg { .. }
        | Op::RevSwap { .. } => CoreDatapath,

        Op::LoadConst { .. }
        | Op::Move { .. }
        | Op::EnsureOwned { .. }
        | Op::Add { .. }
        | Op::AddAssign { .. }
        | Op::Sub { .. }
        | Op::Mul { .. }
        | Op::Div { .. }
        | Op::ExactDiv { .. }
        | Op::FloorDiv { .. }
        | Op::Mod { .. }
        | Op::DivPow2 { .. }
        | Op::MagicDivU { .. }
        | Op::Pow { .. }
        | Op::BitXor { .. }
        | Op::BitAnd { .. }
        | Op::BitOr { .. }
        | Op::Shl { .. }
        | Op::Shr { .. }
        | Op::Not { .. }
        | Op::Lt { .. }
        | Op::Gt { .. }
        | Op::LtEq { .. }
        | Op::GtEq { .. }
        | Op::Eq { .. }
        | Op::ApproxEq { .. }
        | Op::NotEq { .. }
        | Op::Concat { .. }
        | Op::SeqConcat { .. } => Datapath,

        Op::Jump { .. }
        | Op::JumpIfFalse { .. }
        | Op::JumpIfTrue { .. }
        | Op::Call { .. }
        | Op::CallValue { .. }
        | Op::MakeClosure { .. }
        | Op::Return { .. }
        | Op::ReturnNothing
        | Op::Halt => Sequencer,

        Op::GlobalGet { .. }
        | Op::GlobalSet { .. }
        | Op::ListPushField { .. }
        | Op::NewList { .. }
        | Op::NewEmptyList { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyListI32 { .. }
        | Op::NewEmptySet { .. }
        | Op::NewEmptyMap { .. }
        | Op::NewRange { .. }
        | Op::ListPush { .. }
        | Op::SetAdd { .. }
        | Op::RemoveFrom { .. }
        | Op::SetIndex { .. }
        | Op::SetIndexUnchecked { .. }
        | Op::Index { .. }
        | Op::IndexUnchecked { .. }
        | Op::RegionBoundsGuard { .. }
        | Op::Length { .. }
        | Op::Contains { .. }
        | Op::FormatValue { .. }
        | Op::SliceOp { .. }
        | Op::DeepClone { .. }
        | Op::NewTuple { .. }
        | Op::UnionOp { .. }
        | Op::IntersectOp { .. }
        | Op::ListPop { .. }
        | Op::DestructureTuple { .. }
        | Op::NewStruct { .. }
        | Op::StructInsert { .. }
        | Op::GetField { .. }
        | Op::NewInductive { .. }
        | Op::TestArm { .. }
        | Op::BindArm { .. }
        | Op::IterPrepare { .. }
        | Op::IterNext { .. }
        | Op::IterPop => LoadStore,

        Op::CallBuiltin { .. }
        | Op::CheckPolicy { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyMapWithCapacity { .. }
        | Op::CrdtBump { .. }
        | Op::CrdtMerge { .. }
        | Op::CrdtJoin { .. }
        | Op::BigIntWide { .. }
        | Op::NewCrdt { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtResolve { .. }
        | Op::LoadToday { .. }
        | Op::LoadNow { .. }
        | Op::Sleep { .. }
        | Op::Show { .. }
        | Op::Obs { .. }
        | Op::Args { .. }
        | Op::FailWith { .. }
        | Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. }
        | Op::ChanClose { .. }
        | Op::Spawn { .. }
        | Op::SpawnHandle { .. }
        | Op::TaskAwait { .. }
        | Op::TaskAbort { .. }
        | Op::SelectArmRecv { .. }
        | Op::SelectArmTimeout { .. }
        | Op::SelectWait { .. }
        | Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetMakePeer { .. }
        | Op::NetSync { .. }
        | Op::Tcp { .. }
        | Op::Serve { .. } => Coprocessor,
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/isa/tests.rs"]
mod tests;
