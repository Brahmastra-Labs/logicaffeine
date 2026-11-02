//! LOGOS → **RISC-V** AOT codegen tier. Lowers the register bytecode
//! [`CompiledProgram`] — the SAME IR the WASM backend and the Silicon ISA emitter
//! consume — to **RV64GC** assembly text (SysV lp64d ABI, glibc), which a RISC-V
//! GCC assembles+links and `qemu-user` runs, byte-for-byte identical to the VM.
//!
//! Structurally this mirrors [`crate::vm::isa::emit_isa`] (a `CompiledProgram → String`
//! text emitter), and its totality discipline mirrors the WASM tier: every op the
//! current phase does not lower returns [`RiscVLowerError::Unsupported`] — a SOUND
//! refusal, never a miscompile — and the `riscv_coverage_lock` ratchet forces each
//! such op onto a shrink-only KNOWN_GAPS list so nothing is silently wrong.
//!
//! Register model (v1): each VM virtual register is one 8-byte STACK SLOT at
//! `(8*reg)(sp)`; every op loads operands into scratch (`t0..t2`), computes, and
//! stores the result back. No values live in registers across instructions, so
//! lowering is a local, context-free transliteration and calls are trivially
//! correct (no caller-saved value is live across a `call`). A real register
//! allocator is a later optimization.

mod emit;
mod rodata;
mod runtime;

pub use runtime::RISCV_RUNTIME_C;

/// The symbol the assembly of a program the exact extension reaches defines ([`links_value_runtime`]), which the
/// extension's adapter in the shim reads, so neither links without the other.
pub(crate) const VALUE_RUNTIME_SYMBOL: &str = "logos_links_value_runtime";

/// Whether the RV64GC assembly `asm` links the exact extension's value runtime: it defines
/// [`VALUE_RUNTIME_SYMBOL`].
pub fn links_value_runtime(asm: &str) -> bool {
    asm.lines().any(|line| line.strip_suffix(':') == Some(VALUE_RUNTIME_SYMBOL))
}

/// What a RISC-V program links beside its assembly: the C of its runtime shim, and what its link line names after
/// the assembly and the shim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Runtime {
    pub c: String,
    pub link_args: Vec<String>,
}

/// The runtime the assembly `asm` links. A program the exact extension never reaches links [`RISCV_RUNTIME_C`] and the
/// math library. One it reaches links the shim with the extension's adapter after it (the extension's C core over the
/// shim's collector, then the entry points the assembly calls), and `logicaffeine-data`'s value runtime built for
/// RV64GC Linux with the system libraries it calls, statically.
pub fn runtime(asm: &str) -> Result<Runtime, String> {
    if !links_value_runtime(asm) {
        return Ok(Runtime {
            c: RISCV_RUNTIME_C.to_string(),
            link_args: vec!["-lm".to_string()],
        });
    }
    let mut link_args =
        crate::codegen_c::value_runtime::archive_link_args(crate::codegen_c::value_runtime::Target::Riscv64)?;
    link_args.push("-lm".to_string());
    Ok(Runtime {
        c: format!(
            "{RISCV_RUNTIME_C}\n{}\n{}",
            crate::codegen_c::tower::CORE,
            include_str!("tower.c")
        ),
        link_args,
    })
}

use crate::analysis::PolicyRegistry;
use crate::intern::Interner;
use crate::vm::CompiledProgram;

/// An op / type / shape outside the current phase's supported fragment. The `&str` names what is not
/// yet lowered (an op name, or a reason like `"Show of non-scalar value"`). A refusal — the backend
/// declines to emit rather than emit wrong code (mirrors [`crate::vm::wasm::WasmLowerError`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiscVLowerError {
    Unsupported(&'static str),
}

impl std::fmt::Display for RiscVLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiscVLowerError::Unsupported(what) => {
                write!(f, "riscv AOT backend: unsupported {what}")
            }
        }
    }
}

impl std::error::Error for RiscVLowerError {}

/// Whether this tier lowers `op` as a value-semantics mutation of a register's own collection or Text, or of
/// a value in its own record's field: through an `_own` runtime helper that copies first only what the
/// register's owned flag (and, for a field, its field flag) says may be held elsewhere, so the change never
/// reaches an alias and a loop of them runs in place.
pub fn mutates_own_collection(op: &crate::vm::Op) -> bool {
    matches!(
        emit::mutation(op),
        Some(emit::Mutation::Owned(_) | emit::Mutation::Field(_))
    ) || emit::text_append(op).is_some()
}

/// Emit `program` as RV64GC assembly text. Returns [`RiscVLowerError::Unsupported`] if the program
/// uses any op/shape outside the currently-lowered fragment.
pub fn codegen_program_riscv(
    program: &CompiledProgram,
    policies: &PolicyRegistry,
    interner: &Interner,
) -> Result<String, RiscVLowerError> {
    emit::emit(program, policies, interner)
}
