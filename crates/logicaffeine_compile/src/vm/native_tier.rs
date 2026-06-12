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

use super::instruction::{Constant, Op};

/// A natively-compiled function: integer registers in, integer result out.
pub trait NativeFn: Send + Sync {
    fn call(&self, args: &[i64]) -> i64;
    /// True when the result re-boxes as Bool (the backend's type inference
    /// proved every return site yields a comparison result).
    fn returns_bool(&self) -> bool {
        false
    }
}

/// A backend that can try to compile one VM function to native code.
pub trait NativeTier: Send + Sync {
    /// Attempt to compile the function whose bytecode is `code`
    /// (`code[0]` is the instruction at `entry_pc`; jump targets inside are
    /// ABSOLUTE program pcs and need rebasing by `entry_pc`). Return None to
    /// leave the function on the bytecode path forever.
    fn compile_function(
        &self,
        code: &[Op],
        entry_pc: usize,
        constants: &[Constant],
        param_count: u16,
        register_count: u16,
    ) -> Option<Box<dyn NativeFn>>;

    /// Attempt to compile a MAIN loop region. `code[0]` is the op at
    /// `head_pc` (the back-edge target); the slice ends at the back-edge
    /// jump (inclusive). Every jump out of the region must target `exit_pc`.
    /// Default: regions stay on bytecode.
    fn compile_region(
        &self,
        code: &[Op],
        head_pc: usize,
        exit_pc: usize,
        constants: &[Constant],
        register_count: u16,
    ) -> Option<Box<dyn RegionFn>> {
        let _ = (code, head_pc, exit_pc, constants, register_count);
        None
    }
}

/// A natively-compiled MAIN-LOOP REGION (OSR-lite): no arguments, no return —
/// every effect flows through the frame of Main's registers.
pub trait RegionFn: Send + Sync {
    /// Slots whose CURRENT values the region may read before writing: the VM
    /// must guard each one is an Int and copy it into the native frame.
    fn guard_set(&self) -> &[u16];
    /// Slots whose incoming values are provably DEAD (written before read,
    /// e.g. the loop-condition scratch): no guard, no copy-in.
    fn free_set(&self) -> &[u16];
    /// Slots the region writes, with re-boxing kind (`true` = Bool).
    fn write_set(&self) -> &[(u16, bool)];
    fn frame_size(&self) -> usize;
    fn run(&self, frame: &mut [i64]);
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

/// Calls before a function is considered hot.
pub const NATIVE_TIER_THRESHOLD: u32 = 100;

/// Back-edge crossings before a Main loop is considered hot.
pub const REGION_TIER_THRESHOLD: u32 = 100;

/// Per-region tier state (keyed by loop-head pc).
pub(crate) enum RegionSlot {
    Failed,
    Ready { rf: Box<dyn RegionFn>, exit_pc: usize },
}

/// Per-function tier state.
pub(crate) enum NativeSlot {
    /// Still profiling (or below threshold).
    Untried,
    /// Compilation was attempted and bailed — never retried.
    Failed,
    /// Compiled; the guard still applies per call.
    Ready(Box<dyn NativeFn>),
}
