//! Lifts the program's OWN code (the region after [`crate::codegen_c::USER_CODE_SENTINEL`]) onto the
//! GPU as `__device__` functions. The runtime BEFORE the sentinel is tree-shaken and device-lifted
//! separately (see [`super::device_lift`]); this module handles only the user region — renaming the
//! entry, dropping the host-only argv globals, and `__device__`-annotating the program's own defs.

/// The C `main` the C backend emits, renamed so the kernel can call it as a device function without
/// colliding with the host `main` in the emitted `.cu`.
const C_MAIN_SIG: &str = "int main(int argc, char **argv) {";
const DEVICE_MAIN_SIG: &str = "int logos_main(int argc, char **argv) {";

/// Mark each top-level definition/declaration in the user region `__device__`, rename `main`, and drop
/// the two host-only argv globals (unreachable on device — `Args` is refused at the op level).
pub fn lift_user_region(user: &str) -> String {
    // Rename the entry first, so the per-line annotator prefixes it like any other def.
    let renamed = user.replace(C_MAIN_SIG, DEVICE_MAIN_SIG);

    let mut out = String::with_capacity(renamed.len() + 256);
    for line in renamed.lines() {
        let trimmed = line.trim_start();
        // The C `main` sets these two host globals; the device entry never does (`Args`, the only
        // reader, is refused), so drop them rather than lift the globals.
        if trimmed == "_logos_argc = argc;" || trimmed == "_logos_argv = argv;" {
            continue;
        }
        // A top-level line that OPENS a function (`… {`) or IS a forward declaration (`…;`) becomes a
        // device function. Braces/`#`/comment lines, type definitions (a stepped task's frame) and indented
        // body lines are left alone.
        let top_level = line
            .as_bytes()
            .first()
            .is_some_and(|b| *b != b' ' && *b != b'\t');
        let opens_def_or_decl = top_level
            && !trimmed.starts_with('}')
            && !trimmed.starts_with('#')
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("typedef")
            && !trimmed.starts_with("__device__")
            && (trimmed.ends_with('{') || trimmed.ends_with(';'));
        if opens_def_or_decl {
            out.push_str("__device__ ");
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_cuda/wrap/tests.rs"]
mod tests;
