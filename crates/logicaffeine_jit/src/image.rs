//! Precompiled bytecode images for the native JIT.
//!
//! The ordinary CLI path parses, optimizes and lowers source on every process
//! launch. An image moves that deterministic work to the build phase while
//! retaining the actual copy-and-patch tier-up at run time. The image is the
//! existing versioned, checksummed LGB1 wire format; native machine code is
//! never serialized.

use logicaffeine_compile::optimization::Tier;
use logicaffeine_compile::vm::{Compiler, NativeTier, Op, Vm};

use crate::ForgeTier;

/// One image execution's complete observable result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageOutcome {
    /// Lines printed by `Show`, without trailing newlines.
    pub lines: Vec<String>,
    /// Runtime failure, if execution did not complete.
    pub error: Option<String>,
}

/// Compile source through the production eager run-path optimizer into a
/// self-contained LGB1 bytecode image.
///
/// Policy checks are refused explicitly because their condition registry
/// lives beside the parsed AST rather than in `CompiledProgram`; silently
/// dropping that registry would change the program's security semantics.
pub fn build(source: &str) -> Result<Vec<u8>, String> {
    logicaffeine_compile::ui_bridge::with_optimized_program_tiered(
        source,
        Tier::T3,
        |parsed, interner| {
            let (stmts, types, _policies) = parsed?;
            if logicaffeine_compile::interpreter::needs_async(stmts, interner)
                || logicaffeine_compile::concurrency::uses_scheduler(stmts)
            {
                return Err("a JIT image is sync-only; this program requires the scheduler".into());
            }
            let oracle = logicaffeine_compile::optimize::oracle_analyze_with(stmts, interner);
            let program =
                Compiler::compile_with_oracle(stmts, interner, Some(types), Some(oracle))?;
            if program
                .code
                .iter()
                .any(|op| matches!(op, Op::CheckPolicy { .. }))
            {
                return Err(
                    "a JIT image cannot yet carry the policy registry; run from source instead"
                        .into(),
                );
            }
            Ok(logicaffeine_compile::vm::binisa::encode_program(
                &program, interner,
            ))
        },
    )
}

/// Decode and execute an image on the bytecode VM with the supplied private
/// copy-and-patch tier. `program_args` is the full argv, including `argv[0]`.
pub fn run(
    bytes: &[u8],
    program_args: &[String],
    tier: &ForgeTier,
) -> Result<ImageOutcome, String> {
    let mut interner = logicaffeine_compile::Interner::new();
    let program = logicaffeine_compile::vm::binisa::decode_program(bytes, &mut interner)
        .ok_or_else(|| "invalid or incompatible LOGOS bytecode image".to_string())?;
    let mut vm = Vm::new(&program)
        .with_program_args(program_args.to_vec())
        .with_native_tier(tier as &dyn NativeTier);
    let error = vm.run().err();
    Ok(ImageOutcome {
        lines: vm.into_lines(),
        error,
    })
}
