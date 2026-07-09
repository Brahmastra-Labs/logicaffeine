//! WebAssembly code generation from VM bytecode — one shared codegen, two consumers.
//!
//! WASM has only *structured* control flow, so the register-machine bytecode (`&[Op]`, with
//! absolute jumps) is lowered through a basic-block dispatch loop (a `loop` of `block`s with a
//! `br_table` on a "next block" local). That lowering (`cfg`) and the hand-rolled binary
//! encoder (`encode`) are the hard, validated core; both of the following build on it:
//!
//! - `region_jit` — the WS6 browser **JIT tier**: emits one WebAssembly module per hot
//!   *function* and runs it on `wasmi` (native) or the host's real `WebAssembly` (wasm32).
//!   Gated behind the `wasm-jit` feature.
//! - the direct **AOT backend** (`module`) — `assemble_program` (and its source-level wrapper
//!   [`crate::compile::compile_to_wasm`]): emits one self-contained `.wasm` for a *whole program*
//!   with NO rustc/cargo/wasm-bindgen/linker. Feature-independent (needs only
//!   `encode`/`cfg`/`kind`, not `wasmi`/`js_sys`); exposed today as a library API.
//!
//! # The AOT backend
//!
//! ## Value model (`kind`)
//!
//! The bytecode's arithmetic and `Show` are runtime-polymorphic (one `Op::Add` adds Ints or
//! Floats), so a standalone module must know each register's *static* kind. A dataflow fixpoint
//! infers a `kind::Kind` per register, each mapping to exactly one wasm value type:
//!
//! - **Scalars, by value:** `Int`/`Bool`/`Moment` → `i64`, `Float` → `f64`, `Date` → `i32`.
//! - **Heap values, an `i32` handle into linear memory:** `Text`, `Struct`, `Map`, `Set`, `Enum`,
//!   `Closure`, `Tuple` (heterogeneous), and the sequences `SeqInt`/`SeqFloat`/`SeqText`/
//!   `SeqStruct`/`SeqEnum`/`SeqSeqInt` (plus `SeqAny`, a not-yet-refined empty sequence).
//!
//! `Int` and `Bool` share `i64`, so a register reused across them coalesces. A register reused
//! across kinds of *different* value types in disjoint live ranges (e.g. an `Enum` loop variable
//! whose slot a later `Int` reuses) is `Unsupported` — a sound refusal (never a miscompile),
//! pending register-live-range splitting.
//!
//! ## Heap value model (self-contained, no linker)
//!
//! When a program uses a heap op the module gets a linear memory + a bump-allocator pointer global
//! `__heap_ptr` (scalar-only modules stay memory-free). Every heap value is a **stable `i32`
//! handle** so a realloc-on-grow never invalidates the register holding it:
//!
//! - A **sequence** or **Set** is a 16-byte header `[len:i32][cap:i32][data_ptr:i32]` over a
//!   separate buffer of 8-byte slots — a scalar element by value, a handle element (Text / Struct /
//!   Enum / inner sequence) in the slot's low word. Indexing is 1-based and bounds-checked (trap on
//!   OOB); `Push`/`Add` reallocate the buffer and rewrite the header in place. A `Seq of Struct`
//!   flows the element's field layout to an extracted element so `(item N of xs)'s field` resolves.
//! - A **Map** is the same header over 16-byte `[key][value]` entries — Int **or** Text keys,
//!   Int/Float/Bool values — looked up by linear scan, order-independent so byte-identical to the
//!   VM's hashmap for get/insert/contains/length.
//! - A **Struct** is a header over 8-byte field slots (field names are compile-time only; a
//!   `struct_layout` analysis maps each field to its slot + kind). A **Tuple** is the same, by
//!   position. An **Enum** is `[tag:i32][arg slots…]` (the tag is the constructor's constant index,
//!   so an `i32.eq` on tags is the VM's constructor-name compare; payload args follow). A
//!   **Closure** is `[func_idx:i32][captured values…]`, invoked by `call_indirect` through the
//!   module's function table; captures (locals and snapshotted globals) pass as trailing parameters.
//!
//! `Show` of a scalar, a Text, a sequence of scalars/Text, or a `Set` (all insertion-ordered, so
//! deterministic) is byte-identical to the VM; whole-`Map`/whole-`Struct` display (hashmap/field
//! order is non-reproducible against the AOT's insertion order) is the deferred remainder.
//!
//! ## Host interface
//!
//! Raw `env.*` imports (no wasm-bindgen — runs under `wasmi`/`wasmtime`/a browser shim): the
//! `print_*` sinks (the host formats to match the tree-walker's `to_display_string`, reading
//! sequences / Sets / Text out of the exported memory), `pow_ff`/`pow_fi` (exact `powf`/`powi`),
//! `today`/`now` (honoring the test fixed clock), and `fmt_*_into` (interpolating scalar operands
//! into a module buffer). A runtime error — out-of-bounds index, undefined variable, an overflow
//! the oracle could not rule out — lowers to a wasm `unreachable` trap (the standalone module has
//! no VM to surface the message), a contract the lock proves as tw-error ⟺ wasm-trap.
//!
//! ## The lock (`tests/wasm_aot_lock.rs`)
//!
//! WASM == VM == Tree-walker, mirroring the Futamura locks. An EXHAUSTIVE `op_support(&Op)` match
//! (compiler-enforced: a new VM op fails to build until classified Supported/Deferred) plus a
//! behavioural biconditional (a program compiles IFF every op is Supported, then runs
//! byte-identically) plus a full-language coverage proof (every Supported instruction is exercised
//! end-to-end over the curated corpus). Goal: the Deferred set shrinks to ∅.

pub mod encode;
pub mod func;

/// Shared control-flow lowering (basic blocks + the `block`/`loop`/`br_table` dispatch loop)
/// used by both `func` (the JIT region emitter) and the AOT whole-program emitter.
mod cfg;

/// Static per-register kind inference (Int/Bool/Float/…) for the AOT backend — the bytecode's
/// arithmetic/`Show` ops are runtime-polymorphic, so a standalone module needs the static
/// types the JIT tier gets for free from its all-Int runtime gate.
mod kind;

/// The direct AOT backend: a whole program → one self-contained `.wasm` module.
mod module;

/// Register live-range splitting: the structural pre-pass that gives a VM register reused across
/// disjoint live ranges of different wasm value types one local per range (so kind inference, which
/// soundly refuses a single conflicting local, succeeds). Identity on programs already accepted.
mod regsplit;

/// The P2 linker: emit LLVM-compatible RELOCATABLE wasm objects (linking + `reloc.CODE` sections)
/// that call the prebuilt Rust runtime by undefined `logos_rt_*` symbol, and link them with the Rust
/// toolchain's `rust-lld` into one module with one shared linear memory + one allocator. This is how
/// features the emitted wasm can't self-contain (BigInt, the real collection/transport runtime) work
/// Logos→native-wasm without a second, divergent implementation. Gated behind `wasm-jit` while the
/// only consumer is its own `wasmi` link smoke test; a later slice wires it into `compile_to_wasm` and
/// drops the gate + the `dead_code` allowance (emit/link are feature-independent, only running needs
/// `wasmi`).
#[cfg(feature = "wasm-jit")]
#[allow(dead_code)]
mod link;

/// The relocatable-object transform (S4): rewrite a finished self-contained module's `call` targets to
/// relocatable 5-byte LEBs + append the `linking`/`reloc.CODE` sections, so [`link`] can link the real
/// Rust runtime into a real Logos program. A total-or-refuse decoder keeps it sound (converts what it
/// understands, declines otherwise — never miscompiles). Gated like `link` until wired into
/// `compile_to_wasm`.
#[cfg(feature = "wasm-jit")]
#[allow(dead_code)]
mod reloc;

/// The WS6 browser WASM-JIT tier (region/function emit + host instantiation). Only the
/// *running* of emitted modules touches `wasmi`/`js_sys`, so this — and only this — is gated
/// behind the `wasm-jit` feature; `encode`/`func` (pure byte emission) are always built.
#[cfg(feature = "wasm-jit")]
pub mod region_jit;

pub use module::assemble_program;

/// The linker-tier entry points ([`crate::compile::compile_to_wasm_linked`]): the BigInt-aware emitter,
/// the relocatable transform, and the `rust-lld` link against the real `logicaffeine_base` runtime.
pub(crate) use module::assemble_program_linked;
#[cfg(feature = "wasm-jit")]
pub(crate) use reloc::module_to_relocatable;
#[cfg(feature = "wasm-jit")]
pub(crate) use link::link_relocatable_bigint;

/// Why the AOT WebAssembly backend declined to lower a program. The backend is *total* on its
/// supported fragment and rejects everything else explicitly (never miscompiles): the corpus
/// ratchet asserts each program is either lowered-and-correct or on the allowed-unsupported
/// list, so a program silently leaving the fragment fails a test rather than emitting wrong code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasmLowerError {
    /// An op, type, or shape outside the current phase's supported fragment (the `&str` names
    /// what is not yet handled, e.g. `"collection op"`, `"non-scalar parameter"`).
    Unsupported(&'static str),
}

impl std::fmt::Display for WasmLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WasmLowerError::Unsupported(what) => {
                write!(f, "wasm AOT backend: unsupported {what}")
            }
        }
    }
}

impl std::error::Error for WasmLowerError {}
