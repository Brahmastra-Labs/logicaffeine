//! Where an emitted CUDA program came from, stated in its first line. Every whole-program emitter takes its
//! header from [`header`], so no program can claim a provenance it does not have — in particular, a hand-written
//! lane cannot say it was compiled from a model's `.lg`.

/// How a CUDA program was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// Compiled from a program's tensor statements: every kernel launch is a stage the tensor front end lifted from
    /// the source (or several, fused), and a statement it cannot lift refuses the compile
    /// ([`super::qwen_emit::forward_graph`], [`super::lane`]).
    Compiled,
    /// A lane written by hand for one model: its kernels and launch sequence are templates in this crate, driven by
    /// the model's configuration. It is held to the model's `.lg` forward by golden-token locks, not compiled from it.
    HandWritten {
        model: &'static str,
        lane: &'static str,
    },
}

/// The program's first line, newline included.
pub fn header(p: Provenance) -> String {
    match p {
        Provenance::Compiled => {
            "// LOGOS: compiled from the program's tensor statements — every launch is a stage lifted from the source.\n"
                .to_string()
        }
        Provenance::HandWritten { model, lane } => format!(
            "// LOGOS: hand-written {lane} lane for {model} — not compiled from its `.lg`; held to it by golden-token locks.\n"
        ),
    }
}
