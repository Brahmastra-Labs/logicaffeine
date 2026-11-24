//! `largo specialize` — the three Futamura projections through the offline specializer.
//!
//! `mix` (`optimize/mix.logos`, its binding-time analysis in `optimize/mix_bta.rs`) specializes two-level programs:
//!
//! - `p1 FILE`: `[[mix]](sint, FILE)`, the self-interpreter specialized to one program: a residual program with the
//!   interpreter's dispatch specialized away. The report gives its interpretive dispatch (`count_dispatch`), the measure
//!   of Jones optimality.
//! - `p2`: `[[mix]](mix, sint)`, the specializer specialized to the self-interpreter: a compiler.
//! - `p3`: `[[mix]](mix, mix)`, the specializer specialized to itself: a compiler generator. `--check` runs it on the
//!   self-interpreter and says whether it regenerated the second projection's compiler.
//!
//! ★ THE COMMAND IS THE LIBRARY. Each artifact is the library's output byte for byte, and a program the library refuses
//! is refused with the library's own reason.

use std::path::{Path, PathBuf};

use logicaffeine_compile::compile::{
    count_dispatch, mix_projection1, mix_projection2, mix_projection3, run_mix_cogen, run_native_mix_cogen,
    MixEngine,
};

use crate::ui::{self, CliError};

/// Which projection to build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Projection {
    /// The self-interpreter specialized to a program: a residual program.
    P1,
    /// The specializer specialized to the self-interpreter: a compiler.
    P2,
    /// The specializer specialized to itself: a compiler generator.
    P3,
}

/// Handle `largo specialize`.
pub(crate) fn cmd_specialize(
    projection: Projection,
    file: Option<PathBuf>,
    output: Option<PathBuf>,
    check: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match projection {
        Projection::P1 => {
            let file = file.ok_or_else(|| {
                CliError::with_hint(
                    "the first projection specializes the self-interpreter to a program: name the program",
                    "largo specialize p1 program.lg",
                )
            })?;
            let program = std::fs::read_to_string(&file)
                .map_err(|e| CliError::new(format!("{}: {e}", file.display())))?;
            let residual = mix_projection1(&program).map_err(|reason| {
                CliError::new(format!("{} cannot be specialized: {reason}", file.display()))
            })?;
            emit(&residual, output.as_deref())?;
            ui::note(format!(
                "residual: {} lines, interpretive dispatch {}",
                residual.lines().count(),
                count_dispatch(&residual)
            ));
            Ok(())
        }
        Projection::P2 => {
            refuse_file(file, "p2")?;
            let compiler = mix_projection2().map_err(|e| CliError::new(format!("the second projection failed: {e}")))?;
            emit(&compiler, output.as_deref())?;
            ui::note(format!("compiler: {} lines", compiler.lines().count()));
            Ok(())
        }
        Projection::P3 => {
            refuse_file(file, "p3")?;
            let cogen = mix_projection3().map_err(|e| CliError::new(format!("the third projection failed: {e}")))?;
            emit(&cogen, output.as_deref())?;
            ui::note(format!("compiler generator: {} lines", cogen.lines().count()));
            if check {
                let compiler =
                    mix_projection2().map_err(|e| CliError::new(format!("the second projection failed: {e}")))?;
                let regenerated = match MixEngine::default_engine() {
                    MixEngine::Native => run_native_mix_cogen(&cogen),
                    MixEngine::TreeWalker => run_mix_cogen(&cogen),
                }
                .map_err(|e| CliError::new(format!("the compiler generator did not run: {e}")))?;
                let holds = regenerated == compiler;
                ui::note(format!(
                    "regenerates the second projection's compiler: {}",
                    if holds { "yes" } else { "no" }
                ));
                if !holds {
                    return Err(CliError::new(
                        "the compiler generator, run on the self-interpreter, does not print the second projection's compiler",
                    )
                    .into());
                }
            }
            Ok(())
        }
    }
}

/// The second and third projections take no program: they specialize the specializer.
fn refuse_file(file: Option<PathBuf>, name: &str) -> Result<(), CliError> {
    match file {
        None => Ok(()),
        Some(f) => Err(CliError::with_hint(
            format!("`{name}` specializes the specializer and takes no program (got {})", f.display()),
            "only `p1` takes a program: largo specialize p1 program.lg",
        )),
    }
}

/// Write `text` to `output`, or to stdout.
fn emit(text: &str, output: Option<&Path>) -> Result<(), CliError> {
    match output {
        Some(path) => std::fs::write(path, text).map_err(|e| CliError::new(format!("{}: {e}", path.display()))),
        None => {
            use std::io::Write as _;
            let mut out = std::io::stdout().lock();
            out.write_all(text.as_bytes())
                .and_then(|_| out.flush())
                .map_err(|e| CliError::new(format!("stdout: {e}")))
        }
    }
}
