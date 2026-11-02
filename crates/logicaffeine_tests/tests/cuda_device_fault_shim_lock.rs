//! Every host-only libc symbol on the C runtime's FAIL PATHS has a device replacement.
//!
//! The CUDA tier tree-shakes the C oracle onto the device (`device_lift`), rewriting the libc names a
//! GPU thread lacks to `__device__` shims (`exit`→`logos_exit` trap, `fprintf`→no-op, …). The C
//! runtime's traps grew `fflush(stdout)` (so partial output matches the VM before an index/map-miss
//! abort) and the rewrite table did not follow — `nvcc` then failed EVERY program whose lifted
//! runtime carried `logos_idx0`: "calling a __host__ function("fflush") from a __device__ function",
//! "identifier "stdout" is undefined in device code". This lock closes the class WITHOUT a GPU: lift
//! representative programs and assert no bare host-only identifier survives in device code, and that
//! each routed replacement is actually defined by the fault shim.
//!
//! CPU-only (no nvcc, no GPU): it inspects the lifted `.cu` text the CUDA backend would hand to nvcc.

use logicaffeine_compile::compile::compile_to_cuda;

/// libc / stdio identifiers a `__device__` function cannot reference.
const HOST_ONLY: &[&str] = &[
    "fflush",
    "fprintf",
    "printf_unlocked",
    "exit",
    "abort",
    "stdout",
    "stderr",
];

/// Programs whose C runtime carries a fail path: a bounds-checked index (`logos_idx0`), a map
/// lookup (`logos_map_miss`), a division guard, and a negative-exponent power (the `FailWith` trap).
const PROGRAMS: &[(&str, &str)] = &[
    ("index", "## Main\n    Let xs be [1, 2, 3].\n    Show item 2 of xs.\n"),
    ("map_miss", "## Main\n    Let m be a new Map of Text to Int.\n    Set m at \"a\" to 1.\n    Show item \"a\" of m.\n"),
    ("div_zero_guard", "## Main\n    Let x be 3.\n    Show 10 / x.\n"),
    ("negative_pow", "## To p (b: Int) -> Int:\n    Return 2 ** b.\n## Main\n    Show p(2).\n"),
];

/// Whole-word identifier occurrences of `name` in C source, skipping string/char literals and comments.
fn bare_identifier_sites(code: &str, name: &str) -> Vec<usize> {
    let b = code.as_bytes();
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut hits = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                let q = b[i];
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'/' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if i + 1 < b.len() && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            c if is_ident(c) => {
                let start = i;
                while i < b.len() && is_ident(b[i]) {
                    i += 1;
                }
                if &code[start..i] == name {
                    hits.push(start);
                }
            }
            _ => i += 1,
        }
    }
    hits
}

/// The DEVICE half of the emitted `.cu`: everything before the `__global__` entry (the host orchestrator
/// that follows legitimately uses real libc — `fprintf(stderr, …)`, `exit` — and is not lifted).
/// A char-boundary-safe excerpt around byte offset `at` (the `.cu` carries non-ASCII in comments).
fn context(cu: &str, at: usize) -> String {
    let mut lo = at.saturating_sub(120);
    while lo > 0 && !cu.is_char_boundary(lo) {
        lo -= 1;
    }
    let mut hi = (at + 80).min(cu.len());
    while hi < cu.len() && !cu.is_char_boundary(hi) {
        hi += 1;
    }
    cu[lo..hi].to_string()
}

fn lifted(label: &str, src: &str) -> String {
    let art = compile_to_cuda(src)
        .unwrap_or_else(|e| panic!("{label}: CUDA lowering must succeed: {e:?}"));
    let cu = art.kernel_cu;
    let cut = cu
        .find("__global__ void logos_kernel")
        .unwrap_or_else(|| panic!("{label}: no __global__ entry in the .cu"));
    cu[..cut].to_string()
}

#[test]
fn no_host_only_identifier_survives_in_lifted_device_code() {
    for (label, src) in PROGRAMS {
        let cu = lifted(label, src);
        for name in HOST_ONLY {
            let sites = bare_identifier_sites(&cu, name);
            assert!(
                sites.is_empty(),
                "{label}: host-only `{name}` reaches device code at {} site(s) (first at byte {}); route it \
                 through `device_lift::routed_name` and define its shim in CUDA_DEVICE_FAULT_SHIM.\n--- context ---\n{}",
                sites.len(),
                sites[0],
                context(&cu, sites[0])
            );
        }
    }
}

#[test]
fn every_fail_path_replacement_is_defined_by_the_device_shim() {
    let cu = lifted("index", PROGRAMS[0].1);
    for shim in [
        "logos_exit",
        "logos_abort",
        "logos_fprintf",
        "logos_stderr_sink",
        "logos_fflush",
        "logos_stdout_sink",
    ] {
        assert!(
            cu.contains(&format!("__device__ static int {shim}")) || cu.contains(&format!("__device__ static void {shim}")),
            "the lifted device runtime must DEFINE `{shim}` (CUDA_DEVICE_FAULT_SHIM) — a routed name with no definition is an nvcc link error"
        );
    }
}
