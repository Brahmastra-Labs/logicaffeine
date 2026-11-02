//! Wraps the LOGOS→C compute (the proven ≡VM body from [`crate::codegen_c`]) into
//! a single-Tensix-core TT-Metalium program: a compute kernel that runs the body
//! on `RISCV_0` and streams `Show` to DPRINT, plus the host orchestrator and CMake.

use super::runtime::{TT_CMAKE_TEMPLATE, TT_HOST_TEMPLATE, TT_KERNEL_PREAMBLE};
use super::{TtArtifact, KERNEL_FILE_NAME};
use crate::codegen_c::runtime::C_GC_COLLECTOR;

/// The C program's entry (`crate::codegen_c` emits exactly this), renamed so the
/// kernel can call it from `kernel_main` without defining a second `main`.
const C_MAIN_SIG: &str = "int main(int argc, char **argv) {";
const KERNEL_MAIN_SIG: &str = "static int logos_main(int argc, char **argv) {";

/// Build the TT-Metalium artifact from the self-contained C translation of the
/// program. The C body is reused verbatim except: system `#include <...>` lines
/// are dropped (the kernel preamble supplies the on-core headers) and `main` is
/// renamed to `logos_main`, which `kernel_main` invokes.
pub fn wrap_as_tt(oracle_c: &str) -> TtArtifact {
    // Host stack/segment discovery cannot run on a Tensix core. The remaining
    // runtime provides allocator fallbacks for the on-core C library.
    let device_c = oracle_c.replace(C_GC_COLLECTOR, "");
    let body = strip_system_includes(&device_c).replace(C_MAIN_SIG, KERNEL_MAIN_SIG);

    let kernel_cpp = format!(
        "{preamble}{body}\n\nvoid kernel_main() {{\n    (void) logos_main(0, (char **)0);\n}}\n",
        preamble = TT_KERNEL_PREAMBLE,
        body = body,
    );

    let host_cpp = TT_HOST_TEMPLATE.replace("{KERNEL_FILE}", KERNEL_FILE_NAME);
    let cmake = TT_CMAKE_TEMPLATE.replace("{KERNEL_FILE}", KERNEL_FILE_NAME);

    TtArtifact {
        kernel_cpp,
        host_cpp,
        cmake,
        oracle_c: oracle_c.to_string(),
    }
}

/// Drop `#include <...>` system-header lines: on a Tensix baby core those come
/// from the kernel preamble / tt-metal's on-core newlib, not glibc. Local
/// `#include "..."` lines (the C backend emits none) are preserved.
fn strip_system_includes(c: &str) -> String {
    let mut out = String::with_capacity(c.len());
    for line in c.lines() {
        let t = line.trim_start();
        if t.starts_with("#include <") && t.ends_with('>') {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_tt/wrap/tests.rs"]
mod tests;
