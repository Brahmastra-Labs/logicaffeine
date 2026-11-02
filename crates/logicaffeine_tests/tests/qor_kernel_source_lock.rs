//! qor_kernel_source_lock — the C every HLS candidate is synthesised from computes its kernel.
//!
//! `benchmarks/silicon/qor/kernels/<K>/kernel.c` is the one source of each kernel's arithmetic (and
//! the reference the Dahlia ports follow); the `bambu_*.c` wrappers give it the interfaces Bambu
//! builds candidates from. Before any tool's output is judged, every one of those files is compiled
//! natively and run over the kernel's shaped and edge stimulus: every result must equal the golden
//! the LOGOS VM (or the oracle the RTL generator is locked to) computes. A wrong source would make
//! every HLS row built from it measure the wrong function, and a C file in the kernel tree that no
//! driver here runs is refused.

mod common;

use logicaffeine_compile::codegen_sva::qor_harness::*;
use std::path::PathBuf;
use std::process::Command;

const OPS: usize = 2_000;

fn kernel_dir(id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/qor/kernels").join(id)
}

/// Reading and printing hex words: `rd` fills `n` words of one operation (false at the end of the
/// stimulus), `pr` prints `n` result words as one line.
const PRELUDE: &str = "#include <stdio.h>\n#include <stdint.h>\n#include <string.h>\n\
static int rd(uint64_t *w, int n) { for (int i = 0; i < n; i++) { unsigned long long v; if (scanf(\"%llx\", &v) != 1) return 0; w[i] = v; } return 1; }\n\
static void pr(const uint64_t *w, int n) { for (int i = 0; i < n; i++) printf(\"%llx%c\", (unsigned long long)w[i], i == n - 1 ? '\\n' : ' '); }\n";

/// The K3 packing every K3 driver shares: the row's activations from word `in[0]` (byte r), weight
/// row r from word `in[1 + r]` (byte c), and the eight int32 sums two to a word, low first.
const GEMM_PACKING: &str = "static inline void unpack_a(const uint64_t *in, int8_t a[8]) { for (int r = 0; r < 8; r++) a[r] = (int8_t)(uint8_t)(in[0] >> (8 * r)); }\n\
static inline void unpack_w(const uint64_t *in, int8_t w[8][8]) { for (int r = 0; r < 8; r++) for (int c = 0; c < 8; c++) w[r][c] = (int8_t)(uint8_t)(in[1 + r] >> (8 * c)); }\n\
static inline void pr_sums(const int32_t o[8]) { uint64_t out[4]; for (int c = 0; c < 4; c++) out[c] = (uint64_t)(uint32_t)o[2 * c] | ((uint64_t)(uint32_t)o[2 * c + 1] << 32); pr(out, 4); }\n\
static inline int same_weights(uint64_t in[8][9]) { for (int i = 1; i < 8; i++) if (memcmp(in[i] + 1, in[0] + 1, 8 * sizeof(uint64_t))) { printf(\"weights changed inside a tile\\n\"); return 0; } return 1; }\n";

/// One C file of the kernel tree and the `main` that drives one of its functions: it reads one
/// operation's input words per line and prints that operation's output words per line.
struct Source {
    kernel: &'static str,
    file: &'static str,
    function: &'static str,
    main: &'static str,
}

const SOURCES: &[Source] = &[
    Source {
        kernel: "K1",
        file: "kernel.c",
        function: "decide",
        main: "int64_t decide(int64_t, int64_t, int64_t, int64_t, int64_t, int64_t, int64_t, int64_t);\n\
int main(void) { uint64_t v[8]; while (rd(v, 8)) { uint64_t o = (uint64_t)decide((int64_t)v[0], (int64_t)v[1], (int64_t)v[2], (int64_t)v[3], (int64_t)v[4], (int64_t)v[5], (int64_t)v[6], (int64_t)v[7]); pr(&o, 1); } return 0; }\n",
    },
    Source {
        kernel: "K2",
        file: "kernel.c",
        function: "poly",
        main: "int64_t poly(int64_t);\n\
int main(void) { uint64_t x; while (rd(&x, 1)) { uint64_t y = (uint64_t)poly((int64_t)x); pr(&y, 1); } return 0; }\n",
    },
    Source {
        kernel: "K3",
        file: "kernel.c",
        function: "gemm8_row",
        main: "void gemm8_row(const int8_t a[8], const int8_t w[8][8], int32_t o[8]);\n\
int main(void) { uint64_t in[9]; while (rd(in, 9)) { int8_t a[8], w[8][8]; int32_t o[8]; unpack_a(in, a); unpack_w(in, w); gemm8_row(a, w, o); pr_sums(o); } return 0; }\n",
    },
    Source {
        kernel: "K3",
        file: "kernel.c",
        function: "gemm8_tile",
        main: "void gemm8_tile(const int8_t a[8][8], const int8_t w[8][8], int32_t o[8][8]);\n\
int main(void) { uint64_t in[8][9]; for (;;) { for (int i = 0; i < 8; i++) if (!rd(in[i], 9)) return 0; if (!same_weights(in)) return 1;\n\
  int8_t a[8][8], w[8][8]; int32_t o[8][8]; for (int i = 0; i < 8; i++) unpack_a(in[i], a[i]); unpack_w(in[0], w); gemm8_tile(a, w, o); for (int i = 0; i < 8; i++) pr_sums(o[i]); } }\n",
    },
    Source {
        kernel: "K3",
        file: "bambu_ports.c",
        function: "gemm8_ports",
        main: "void gemm8_ports(uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, int32_t *, int32_t *, int32_t *, int32_t *, int32_t *, int32_t *, int32_t *, int32_t *);\n\
int main(void) { uint64_t in[9]; while (rd(in, 9)) { int32_t o[8]; gemm8_ports(in[0], in[1], in[2], in[3], in[4], in[5], in[6], in[7], in[8], &o[0], &o[1], &o[2], &o[3], &o[4], &o[5], &o[6], &o[7]); pr_sums(o); } return 0; }\n",
    },
    Source {
        kernel: "K3",
        file: "bambu_array.c",
        function: "gemm8_arrays",
        main: "void gemm8_arrays(const int8_t a[64], const int8_t w[64], int32_t o[64]);\n\
int main(void) { uint64_t in[8][9]; for (;;) { for (int i = 0; i < 8; i++) if (!rd(in[i], 9)) return 0; if (!same_weights(in)) return 1;\n\
  int8_t a[8][8], w[8][8]; int32_t o[8][8]; for (int i = 0; i < 8; i++) unpack_a(in[i], a[i]); unpack_w(in[0], w); gemm8_arrays((const int8_t *)a, (const int8_t *)w, (int32_t *)o); for (int i = 0; i < 8; i++) pr_sums(o[i]); } }\n",
    },
    Source {
        kernel: "K4",
        file: "kernel.c",
        function: "keccak_f1600",
        main: "void keccak_f1600(uint64_t st[25]);\n\
int main(void) { uint64_t st[25]; while (rd(st, 25)) { keccak_f1600(st); pr(st, 25); } return 0; }\n",
    },
    Source {
        kernel: "K4",
        file: "bambu_ports.c",
        function: "keccak_ports",
        main: "void keccak_ports(uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t, uint64_t,\n\
  uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *, uint64_t *);\n\
int main(void) { uint64_t i[25], o[25]; while (rd(i, 25)) { keccak_ports(i[0], i[1], i[2], i[3], i[4], i[5], i[6], i[7], i[8], i[9], i[10], i[11], i[12], i[13], i[14], i[15], i[16], i[17], i[18], i[19], i[20], i[21], i[22], i[23], i[24],\n\
  &o[0], &o[1], &o[2], &o[3], &o[4], &o[5], &o[6], &o[7], &o[8], &o[9], &o[10], &o[11], &o[12], &o[13], &o[14], &o[15], &o[16], &o[17], &o[18], &o[19], &o[20], &o[21], &o[22], &o[23], &o[24]); pr(o, 25); } return 0; }\n",
    },
    Source {
        kernel: "K4",
        file: "bambu_array.c",
        function: "keccak_arrays",
        main: "void keccak_arrays(const uint64_t in[25], uint64_t out[25]);\n\
int main(void) { uint64_t i[25], o[25]; while (rd(i, 25)) { keccak_arrays(i, o); pr(o, 25); } return 0; }\n",
    },
    Source {
        kernel: "K5",
        file: "kernel.c",
        function: "ntt_bfly",
        main: "uint32_t ntt_bfly(int16_t, int16_t, int16_t);\n\
int main(void) { uint64_t v[3]; while (rd(v, 3)) { uint32_t r = ntt_bfly((int16_t)v[0], (int16_t)v[1], (int16_t)v[2]); uint64_t o[2] = {r >> 16, r & 0xffff}; pr(o, 2); } return 0; }\n",
    },
    Source {
        kernel: "K6",
        file: "kernel.c",
        function: "simt_axpb_sum",
        main: "void simt_axpb_sum(int64_t, int64_t, uint8_t, const int64_t *, int64_t *, int64_t *);\n\
int main(void) { uint64_t v[7]; while (rd(v, 7)) { int64_t x[4] = {(int64_t)v[3], (int64_t)v[4], (int64_t)v[5], (int64_t)v[6]}, y[4], s;\n\
  simt_axpb_sum((int64_t)v[0], (int64_t)v[1], (uint8_t)v[2], x, y, &s); uint64_t o[5] = {(uint64_t)y[0], (uint64_t)y[1], (uint64_t)y[2], (uint64_t)y[3], (uint64_t)s}; pr(o, 5); } return 0; }\n",
    },
    Source {
        kernel: "K6",
        file: "bambu_ports.c",
        function: "simt_ports",
        main: "void simt_ports(int64_t, int64_t, uint8_t, int64_t, int64_t, int64_t, int64_t, int64_t *, int64_t *, int64_t *, int64_t *, int64_t *);\n\
int main(void) { uint64_t v[7]; while (rd(v, 7)) { int64_t y[4], s;\n\
  simt_ports((int64_t)v[0], (int64_t)v[1], (uint8_t)v[2], (int64_t)v[3], (int64_t)v[4], (int64_t)v[5], (int64_t)v[6], &y[0], &y[1], &y[2], &y[3], &s); uint64_t o[5] = {(uint64_t)y[0], (uint64_t)y[1], (uint64_t)y[2], (uint64_t)y[3], (uint64_t)s}; pr(o, 5); } return 0; }\n",
    },
    Source {
        kernel: "K6",
        file: "bambu_array.c",
        function: "simt_arrays",
        main: "void simt_arrays(int64_t, int64_t, uint8_t, const int64_t x[4], int64_t y[4], int64_t *);\n\
int main(void) { uint64_t v[7]; while (rd(v, 7)) { int64_t x[4] = {(int64_t)v[3], (int64_t)v[4], (int64_t)v[5], (int64_t)v[6]}, y[4], s;\n\
  simt_arrays((int64_t)v[0], (int64_t)v[1], (uint8_t)v[2], x, y, &s); uint64_t o[5] = {(uint64_t)y[0], (uint64_t)y[1], (uint64_t)y[2], (uint64_t)y[3], (uint64_t)s}; pr(o, 5); } return 0; }\n",
    },
];

fn mask(v: u64, bits: u32) -> u64 {
    if bits >= 64 {
        v
    } else {
        v & ((1u64 << bits) - 1)
    }
}

/// Compile `s` with its driver (the HLS pragmas are Bambu's and mean nothing to cc) and run it over
/// `stim`, one line of result words per operation.
fn run_c(s: &Source, stim: &[Words]) -> Vec<Vec<u64>> {
    let dir = tempfile::Builder::new().prefix("qor_kernel_source").tempdir().unwrap();
    let main = dir.path().join("main.c");
    let shared = if s.kernel == "K3" { GEMM_PACKING } else { "" };
    std::fs::write(&main, format!("{PRELUDE}{shared}{}", s.main)).unwrap();
    let exe = dir.path().join("k");
    let build = Command::new("cc")
        .args(["-O2", "-std=c11", "-Wall", "-Werror", "-Wno-unknown-pragmas", "-o"])
        .arg(&exe)
        .arg(kernel_dir(s.kernel).join(s.file))
        .arg(&main)
        .output()
        .expect("run cc");
    assert!(
        build.status.success(),
        "{} {} ({}): does not build:\n{}",
        s.kernel,
        s.file,
        s.function,
        String::from_utf8_lossy(&build.stderr)
    );
    let input: String = stim
        .iter()
        .map(|op| op.iter().flat_map(|p| p.iter()).map(|w| format!("{w:x}")).collect::<Vec<_>>().join(" ") + "\n")
        .collect();
    let input_file = dir.path().join("stimulus.txt");
    std::fs::write(&input_file, input).unwrap();
    let out = Command::new(&exe)
        .stdin(std::fs::File::open(&input_file).unwrap())
        .output()
        .expect("run the kernel");
    assert!(
        out.status.success(),
        "{} {} ({}): the driver refused the stimulus: {}",
        s.kernel,
        s.file,
        s.function,
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.split_whitespace().map(|w| u64::from_str_radix(w, 16).unwrap()).collect())
        .collect()
}

fn check(s: &Source) {
    let k = kernel(s.kernel).unwrap();
    for mode in [Mode::Shaped, Mode::Edge] {
        let stim = stimulus(&k, mode, OPS);
        let want = golden(&k, &stim).unwrap();
        let got = run_c(s, &stim);
        assert_eq!(got.len(), OPS, "{} {} ({}, {mode:?}): one result line per operation", s.kernel, s.file, s.function);
        for (op, (g, w)) in got.iter().zip(&want).enumerate() {
            let flat: Vec<u64> = k
                .outputs
                .iter()
                .zip(w)
                .flat_map(|(p, chunks)| {
                    chunks.iter().enumerate().map(move |(c, v)| mask(*v, p.bits.saturating_sub(64 * c as u32).min(64)))
                })
                .collect();
            assert_eq!(
                g,
                &flat,
                "{} {} ({}, {mode:?}) op {op}: the C disagrees with the golden",
                s.kernel,
                s.file,
                s.function
            );
        }
    }
}

#[test]
fn every_kernel_c_source_computes_its_golden() {
    if !common::require_tool("cc") {
        return;
    }
    for s in SOURCES.iter().filter(|s| s.file == "kernel.c") {
        check(s);
    }
}

#[test]
fn every_bambu_wrapper_computes_its_golden() {
    if !common::require_tool("cc") {
        return;
    }
    for s in SOURCES.iter().filter(|s| s.file != "kernel.c") {
        check(s);
    }
}

#[test]
fn every_c_file_in_the_kernel_tree_has_a_driver() {
    let mut files = Vec::new();
    for k in std::fs::read_dir(kernel_dir("")).unwrap() {
        let k = k.unwrap();
        if !k.file_type().unwrap().is_dir() {
            continue;
        }
        let id = k.file_name().into_string().unwrap();
        for f in std::fs::read_dir(k.path()).unwrap() {
            let name = f.unwrap().file_name().into_string().unwrap();
            if name.ends_with(".c") {
                files.push((id.clone(), name));
            }
        }
    }
    assert!(files.len() >= 6, "the census reads the kernel tree ({} C files)", files.len());
    for (id, name) in &files {
        assert!(
            SOURCES.iter().any(|s| s.kernel == id && s.file == name),
            "{id}/{name} is a C source no driver here runs"
        );
    }
    for s in SOURCES {
        assert!(
            files.iter().any(|(id, name)| id == s.kernel && name == s.file),
            "{} {} is driven here but is not in the kernel tree",
            s.kernel,
            s.file
        );
    }
}
