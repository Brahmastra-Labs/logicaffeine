use super::*;

/// A minimal C program (the shape `codegen_c` emits) wraps into a kernel that
/// renames `main`, drops system includes, defines `kernel_main`, and carries the
/// DPRINT show-bridge; the host + CMake reference the kernel file.
#[test]
fn wraps_minimal_c_into_a_wellformed_tt_program() {
    let c = "#include <stdio.h>\n#include <stdint.h>\nint main(int argc, char **argv) {\n    printf(\"%lld\\n\", (long long)42);\n    return 0;\n}\n";
    let art = wrap_as_tt(c);

    // Kernel: no leftover system include, renamed entry, real kernel_main, show-bridge.
    assert!(
        !art.kernel_cpp.contains("#include <stdio.h>"),
        "system includes must be stripped"
    );
    assert!(
        !art.kernel_cpp.contains("int main(int argc"),
        "main must be renamed"
    );
    assert!(art
        .kernel_cpp
        .contains("static int logos_main(int argc, char **argv)"));
    assert!(art.kernel_cpp.contains("void kernel_main()"));
    assert!(art.kernel_cpp.contains("logos_main(0, (char **)0)"));
    assert!(art.kernel_cpp.contains("LOGOS_TT_SHOW_BRIDGE"));
    assert!(
        art.kernel_cpp
            .contains("printf(\"%lld\\n\", (long long)42);"),
        "compute body preserved"
    );

    // Host + CMake reference the kernel file and the tt-metal API surface.
    assert!(art.host_cpp.contains(KERNEL_FILE_NAME));
    assert!(art.host_cpp.contains("DataMovementProcessor::RISCV_0"));
    assert!(art.cmake.contains("TT_METAL_HOME"));

    // Oracle is the untouched C program.
    assert_eq!(art.oracle_c, c);
}
