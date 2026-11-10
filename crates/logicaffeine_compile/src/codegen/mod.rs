//! Code generation from LOGOS AST to Rust source code.
//!
//! This module transforms the parsed and type-checked LOGOS program into
//! idiomatic Rust code. The generated code uses `logicaffeine_data` types
//! for runtime values and integrates with the kernel for proof verification.
//!
//! # Pipeline Position
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │  LOGOS Source → Lexer → Parser → AST → Analysis → HERE │
//! └─────────────────────────────────────────────────────────┘
//!                                                      ↓
//!                                               Rust Source
//! ```
//!
//! # Code Generation Rules
//!
//! | LOGOS Statement | Rust Output |
//! |-----------------|-------------|
//! | `Let x be 5.` | `let x = 5;` |
//! | `Set x to 10.` | `x = 10;` |
//! | `Give x to y.` | `let y = x;` (move) |
//! | `Show x to show.` | `println!("{}", x);` |
//! | `If x > 0 then...` | `if x > 0 { ... }` |
//! | `Repeat while x > 0...` | `while x > 0 { ... }` |
//! | `Zone "name"...` | `{ /* zone scope */ }` |
//! | `Mount x at "path".` | `x.mount(vfs, "path").await;` |
//! | `Sync x on "topic".` | `x.subscribe("topic").await;` |
//!
//! # Key Features
//!
//! - **Refinement Types**: Generates `debug_assert!` for type predicates
//! - **Policy Enforcement**: Emits capability checks for access control
//! - **Zone Safety**: Translates memory zones to Rust scopes
//! - **CRDT Mutability**: Uses `.set()` for LWWRegister/MVRegister fields
//! - **Async Detection**: Adds `#[tokio::main]` when async operations are present
//! - **VFS Detection**: Injects `get_platform_vfs()` for file operations (io_uring on Linux, NativeVfs elsewhere)
//! - **Mount+Sync Detection**: Uses `Distributed<T>` for combined persistence/sync
//!
//! # Refinement Context
//!
//! The [`RefinementContext`] tracks type predicates across scopes:
//!
//! ```text
//! Let x: { it: Int | it > 0 } be 5.   ←  Register constraint
//!      ↓
//! debug_assert!(5 > 0);               ←  Check at definition
//!      ↓
//! Set x to 10.                        ←  Re-check on mutation
//!      ↓
//! debug_assert!(10 > 0);              ←  Re-emit assertion
//! ```
//!
//! - When a variable with a refinement type is defined, its constraint is registered
//! - When that variable is mutated, the assertion is re-emitted
//! - Variable types are tracked for capability resolution
//!
//! # Entry Point
//!
//! The main entry point is [`codegen_program`], which generates a complete Rust
//! program from a list of statements.

// Module declarations
mod absent;
pub(crate) mod affine_array;
pub(crate) mod bigint_promote;
pub(crate) mod bindings;
pub(crate) mod context;
mod control;
pub(crate) mod detection;
pub(crate) mod entry_guard;
pub(crate) mod expr;
pub(crate) mod fast_div;
pub(crate) mod ffi;
pub(crate) mod flow;
pub(crate) mod hoist;
pub(crate) mod i64_map;
pub(crate) mod instantiation;
pub(crate) mod lent;
pub(crate) mod lowbits;
pub(crate) mod marshal;
pub(crate) mod bits;
pub(crate) mod narrow;
pub(crate) mod outline;
pub(crate) mod peephole;
pub(crate) mod probe;
mod policy;
pub(crate) mod program;
pub(crate) mod rational;
mod replay;
pub(crate) mod stmt;
pub(crate) mod strsearch;
pub(crate) mod tce;
pub(crate) mod tower;
pub(crate) mod types;
pub(crate) mod worklist;
// Per-function codegen slicing for the AOT-native tier (HOTSWAP Axis-3 / P14a).
pub(crate) mod slice;
pub(crate) mod slice_escape;
// AVX2 SIMD reduction of transcendental accumulator loops (math.<transcendental> caps).
pub(crate) mod simd_reduce;

use std::collections::{HashMap, HashSet};

use crate::intern::Symbol;

// ─── External API re-exports ────────────────────────────────────────────────
// These preserve the public API used by compile.rs, ui_bridge.rs, and test files.

pub use bindings::{generate_python_bindings, generate_typescript_bindings};
pub use context::{empty_var_caps, RefinementContext, VariableCapabilities};
pub use detection::{collect_async_functions, collect_pipe_sender_params, collect_pipe_vars};
pub use expr::{codegen_assertion, codegen_expr, codegen_term};
pub use ffi::generate_c_header;
pub use hoist::force_disable_for_test as force_disable_borrow_hoist_for_test;
pub use marshal::codegen_native_tier_export;
pub use program::{codegen_program, codegen_program_mapped, codegen_program_with_proven};
pub use slice::function_slice;
pub use stmt::codegen_stmt;

// ─── Mode-B (deterministic-replay) codegen gate ─────────────────────────────
//
// A compile-global flag: when set, a program that runs on the scheduler lowers every task, pipe,
// timer and select onto `logicaffeine_system::replay` — the interpreter's scheduler, ported — so the
// binary replays the interpreter's schedule at the seed `LOGOS_SEED` names. The DEFAULT (Mode A) is
// off, so default emission is untouched. Codegen is single-threaded per compile, so a thread-local is
// the natural home; `compile::compile_to_rust_deterministic` sets it for the extent of one compile,
// and `codegen_program` narrows it to the programs that use the scheduler.

thread_local! {
    static SEEDED_SELECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static OBSERVED_CODEGEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static EMBED_ASSETS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static BLOCKING_WAITS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Does a `Sleep` block the thread for this compile, as the interpreter's synchronous twin waits, instead of yielding
/// to the runtime? The compiler's own engines compile so: a sequential interpreter that runs a program's `Sleep` waits,
/// and its compiled form stays synchronous, its exported entry callable from a host that runs no runtime.
pub(crate) fn waits_block() -> bool {
    BLOCKING_WAITS.with(|c| c.get())
}

/// Run `f` with blocking waits ([`waits_block`]), restoring the previous mode afterwards (panic-safe).
pub fn with_blocking_waits<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            BLOCKING_WAITS.with(|c| c.set(self.0));
        }
    }
    let _guard = BLOCKING_WAITS.with(|c| {
        let prev = c.get();
        c.set(true);
        Restore(prev)
    });
    f()
}

/// Is the deterministic-replay lowering active for this compile?
pub(crate) fn deterministic_replay_enabled() -> bool {
    SEEDED_SELECT.with(|c| c.get())
}

/// Is the OBSERVED codegen profile active (Wave 13)? When set, observation is NOT
/// elided at lowering and the AOT backend EMITS `logicaffeine_system::obs` calls
/// for `Note`/`Warn`/`Record`/`During`. Default OFF — the release/elided profile,
/// where observation is erased before codegen (byte-identical artifact, §0.7).
pub fn observed_codegen_enabled() -> bool {
    OBSERVED_CODEGEN.with(|c| c.get())
}

/// Run `f` with the OBSERVED codegen profile enabled, restoring the previous mode
/// afterwards (panic-safe). Returns `f`'s result.
pub fn with_observed_codegen<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            OBSERVED_CODEGEN.with(|c| c.set(self.0));
        }
    }
    let _guard = OBSERVED_CODEGEN.with(|c| {
        let prev = c.get();
        c.set(true);
        Restore(prev)
    });
    f()
}

/// Is the EMBEDDED-ASSETS `Serve` lowering active for this compile (B4)? When
/// set, the emitted `Serve` closure consults `logos_assets::lookup(&path)`
/// BEFORE calling the handler — an embedded page/static asset shadows a backend
/// route, exactly the dev router's precedence, so a `largo build` web binary
/// serves the bundled site. Default OFF: a plain `Serve` program lowers
/// byte-identically (the asset hook is inert), which the HTTP differential pins.
pub fn embed_assets_enabled() -> bool {
    EMBED_ASSETS.with(|c| c.get())
}

/// Run `f` with the embedded-assets `Serve` lowering enabled (B4), restoring the
/// previous mode afterwards (panic-safe). `largo build` wraps the web-project
/// compile in this so the generated `Serve` consults `logos_assets`.
pub fn with_embed_assets<R>(f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            EMBED_ASSETS.with(|c| c.set(self.0));
        }
    }
    let _guard = EMBED_ASSETS.with(|c| {
        let prev = c.get();
        c.set(true);
        Restore(prev)
    });
    f()
}

/// Run `f` with the deterministic-replay lowering enabled, restoring the previous mode
/// afterwards (panic-safe). Returns `f`'s result.
pub fn with_deterministic_replay<R>(f: impl FnOnce() -> R) -> R {
    with_replay_mode(true, f)
}

/// Run `f` with the deterministic-replay lowering set to `on`, restoring the previous mode afterwards (panic-safe).
pub(crate) fn with_replay_mode<R>(on: bool, f: impl FnOnce() -> R) -> R {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            SEEDED_SELECT.with(|c| c.set(self.0));
        }
    }
    let _guard = SEEDED_SELECT.with(|c| {
        let prev = c.get();
        c.set(on);
        Restore(prev)
    });
    f()
}

// ─── Internal cross-module re-exports ───────────────────────────────────────
// These allow sibling submodules to use `use super::item` for items
// that moved out of mod.rs into submodule files.

pub(crate) use expr::codegen_expr_with_async;
pub(crate) use ffi::{
    classify_type_for_c_abi, codegen_c_accessors, collect_c_export_ref_structs, CAbiClass,
};
pub(crate) use peephole::{
    try_emit_for_range_pattern, try_emit_prefix_reverse, try_emit_swap_pattern,
    try_emit_vec_fill_pattern,
};
pub(crate) use stmt::{
    get_root_identifier, has_copy_element_type, has_copy_value_type, is_copy_type,
};
pub(crate) use types::field_is_boxed;

/// Check if a name is a Rust keyword (needs `r#` escaping in generated code).
pub(crate) fn is_rust_keyword(name: &str) -> bool {
    matches!(
        name,
        "as" | "async"
            | "await"
            | "break"
            | "const"
            | "continue"
            | "crate"
            | "dyn"
            | "else"
            | "enum"
            | "extern"
            | "false"
            | "fn"
            | "for"
            | "if"
            | "impl"
            | "in"
            | "let"
            | "loop"
            | "match"
            | "mod"
            | "move"
            | "mut"
            | "pub"
            | "ref"
            | "return"
            | "self"
            | "Self"
            | "static"
            | "struct"
            | "super"
            | "trait"
            | "true"
            | "type"
            | "unsafe"
            | "use"
            | "where"
            | "while"
            | "abstract"
            | "become"
            | "box"
            | "do"
            | "final"
            | "macro"
            | "override"
            | "priv"
            | "try"
            | "typeof"
            | "unsized"
            | "virtual"
            | "yield"
    )
}

/// A LOGOS name as a Rust identifier: a keyword becomes a raw identifier (`move` → `r#move`), and the
/// four keywords a raw identifier cannot spell (`self`, `Self`, `crate`, `super`) take a reserved
/// compiler prefix (`__logos_self`). Every position a name reaches Rust in — a variable, a function, a
/// record or variant field — goes through this one spelling, so a declaration and every use agree.
pub(crate) fn escape_rust_ident(name: &str) -> String {
    match name {
        "self" | "Self" | "crate" | "super" => format!("__logos_{name}"),
        _ if is_rust_keyword(name) => format!("r#{name}"),
        _ => name.to_string(),
    }
}

/// A call-site ABI role a function parameter can carry, keyed by parameter index.
/// A single function may play SEVERAL at once — the canonical case is a readonly
/// `Seq` param (de-Rc'd to `&[T]`) alongside a value-semantics `mutable` collection
/// param (passed as `&LogosSeq`/`&LogosMap`).
#[derive(Clone, Copy)]
pub(crate) enum FnRole {
    /// Readonly borrow (`&[T]`) — the `borrow_params_map` opt.
    Borrow,
    /// Element-mutating borrow (`&mut [T]`) — the `mut_borrow_params_map` opt.
    MutBorrow,
    /// Value-semantics `mutable` collection (`&LogosSeq`/`&LogosMap`).
    ValueMutable,
}

/// The parameter indices at which one function plays each [`FnRole`].
#[derive(Clone, Default)]
struct FnRoles {
    borrow: HashSet<usize>,
    mut_borrow: HashSet<usize>,
    value_mutable: HashSet<usize>,
}

thread_local! {
    /// Every function's [`FnRoles`]: a fact of the function, the same at every call site, so it is built once per
    /// program and read by each function's codegen.
    static FN_ROLES: std::cell::RefCell<HashMap<Symbol, FnRoles>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Install the whole program's calling roles, from each role's function → parameter indices map. Called once at
/// codegen entry.
pub(crate) fn set_fn_roles(
    borrow: &HashMap<Symbol, HashSet<usize>>,
    mut_borrow: &HashMap<Symbol, HashSet<usize>>,
    value_mutable: &HashMap<Symbol, HashSet<usize>>,
) {
    let mut roles: HashMap<Symbol, FnRoles> = HashMap::new();
    for (function, at) in borrow {
        roles.entry(*function).or_default().borrow = at.clone();
    }
    for (function, at) in mut_borrow {
        roles.entry(*function).or_default().mut_borrow = at.clone();
    }
    for (function, at) in value_mutable {
        roles.entry(*function).or_default().value_mutable = at.clone();
    }
    FN_ROLES.with(|c| *c.borrow_mut() = roles);
}

/// The parameter indices at which `function` plays `role`: empty for a function that plays it nowhere, and for a
/// name that is no function of the program.
pub(crate) fn fn_role_indices(function: Symbol, role: FnRole) -> HashSet<usize> {
    FN_ROLES.with(|c| {
        c.borrow()
            .get(&function)
            .map(|roles| match role {
                FnRole::Borrow => roles.borrow.clone(),
                FnRole::MutBorrow => roles.mut_borrow.clone(),
                FnRole::ValueMutable => roles.value_mutable.clone(),
            })
            .unwrap_or_default()
    })
}

/// Generated-project metadata is a file prologue, so program text cannot select runtime requirements. A program
/// whose failures a runtime boundary catches — a replay scheduler's task, a served request — opens with this line,
/// and its release build unwinds so the boundary can catch them.
pub(crate) const UNWIND_PROFILE_HEADER: &str = "// LOGOS Rust runtime: unwind-v1\n";

/// The panic strategy of a generated program's release build: `unwind` for one that opens with
/// `UNWIND_PROFILE_HEADER`, `abort` for every other, whose failure hook reports and ends it. Every project writer
/// takes it from here.
pub fn release_panic_strategy(rust_code: &str) -> &'static str {
    if rust_code.starts_with(UNWIND_PROFILE_HEADER) {
        "unwind"
    } else {
        "abort"
    }
}
