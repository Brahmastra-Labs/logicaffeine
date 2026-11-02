//! `Escape to Rust:` — raw Rust source embedded in a program, which only the Rust backends compile.
//!
//! Every other backend refuses a program that reaches one, and they all refuse it with [`REFUSAL`], so the
//! refusal is one message wherever it happens: the tree-walker, the VM, and every lowering of the VM's
//! bytecode report these words and no others.

/// The reason every backend but the two Rust backends refuses an `Escape to Rust:` block or expression.
pub const REFUSAL: &str =
    "`Escape to Rust:` holds raw Rust code, which only the Rust backends compile. \
                           Use `largo build` or `largo run` to compile and run this program.";
