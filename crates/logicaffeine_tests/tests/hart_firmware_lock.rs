//! The board firmware's C (`benchmarks/silicon/onboard/firmware`) against the Rust it must agree with, compiled for
//! this machine so every check runs in milliseconds: the shim's compressed-instruction decompressor against
//! `hart::rvc::expand` over all 65,536 encodings, the boot ROM's inflater against the compressor the boot image is
//! built with, and its streaming sha256 against the crate's. Also: the firmware builds for the hart, and the boot ROM
//! and the shim fit the memory the SoC gives them.
//!
//! Tool-gated: `cc` (and `riscv64-linux-gnu-gcc` for the hart build).

mod common;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "support/onboard_board.rs"]
mod onboard_board;

use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::hart::{image, rvc, soc};
use logicaffeine_compile::crypto_accel::sha256;

fn firmware() -> PathBuf {
    onboard_board::root().join("benchmarks/silicon/onboard/firmware")
}

/// Compile `harness` (a C main) with the named firmware files for this machine; the program's path.
fn native(harness: &str, files: &[&str], label: &str) -> PathBuf {
    let dir = scratch::dir("hart_firmware_lock").join(label);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("soc.h"), soc::c_header()).unwrap();
    std::fs::write(dir.join("harness.c"), harness).unwrap();
    let bin = dir.join("harness");
    let mut cmd = std::process::Command::new("cc");
    cmd.args(["-O2", "-std=c11", "-Wall", "-Werror", "-I"]).arg(&dir).arg("-I").arg(firmware());
    cmd.arg(dir.join("harness.c"));
    for f in files {
        cmd.arg(firmware().join(f));
    }
    let out = cmd.arg("-o").arg(&bin).output().expect("run cc");
    assert!(out.status.success(), "{label} does not compile:\n{}", String::from_utf8_lossy(&out.stderr));
    bin
}

#[test]
fn the_shims_decompressor_is_the_model() {
    if !common::require_tool("cc") {
        return;
    }
    let bin = native(
        r#"
#include <stdio.h>
#include <stdint.h>
uint32_t rvc_expand(uint16_t c);
int main(void) {
  for (uint32_t c = 0; c < 65536; c++) {
    uint32_t e = rvc_expand((uint16_t)c);
    fwrite(&e, 4, 1, stdout);
  }
  return 0;
}
"#,
        &["rvc.c"],
        "rvc",
    );
    let out = std::process::Command::new(&bin).output().unwrap().stdout;
    assert_eq!(out.len(), 4 * 65536);
    for c in 0..=u16::MAX {
        if c & 3 == 3 {
            continue;
        }
        let shim = u32::from_le_bytes(out[4 * c as usize..4 * c as usize + 4].try_into().unwrap());
        let want = rvc::expand(c).unwrap_or(0);
        assert_eq!(shim, want, "encoding {c:#06x}: the shim expands {shim:#010x}, the model {want:#010x}");
    }
}

const INFLATE_HARNESS: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include "boot.h"
static unsigned char *in; static size_t in_len, in_pos;
static uint8_t next(void) { return in_pos < in_len ? in[in_pos++] : 0xee; }
int main(int argc, char **argv) {
  FILE *f = fopen(argv[1], "rb");
  fseek(f, 0, SEEK_END); in_len = (size_t)ftell(f); fseek(f, 0, SEEK_SET);
  in = malloc(in_len + 1);
  if (fread(in, 1, in_len, f) != in_len) return 2;
  fclose(f);
  size_t cap = strtoull(argv[2], 0, 10);
  unsigned char *out = malloc(cap + 1);
  static struct inflate_tables tables;
  int64_t n = inflate(&tables, next, in_len, out, cap);
  if (n < 0) { printf("REFUSED"); return 0; }
  if (in_pos != in_len) { printf("UNDRAWN"); return 0; }
  fwrite(out, 1, (size_t)n, stdout);
  return 0;
}
"#;

#[test]
fn the_boot_rom_inflates_what_the_image_builder_compresses() {
    if !common::require_tool("cc") {
        return;
    }
    let bin = native(INFLATE_HARNESS, &["inflate.c"], "inflate");
    let dir = bin.parent().unwrap().to_path_buf();
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let mut noise = |n: usize| -> Vec<u8> {
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    };
    let text = std::fs::read(onboard_board::root().join("crates/logicaffeine_compile/src/codegen_sva/hart/model.rs")).unwrap();
    let mut mixed = text.clone();
    mixed.extend(noise(9000));
    mixed.extend(vec![0u8; 70_000]);
    mixed.extend(&text);
    // Noise compresses to stored blocks, a few bytes to fixed-Huffman ones, text to dynamic ones.
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("one byte", b"x".to_vec()),
        ("a word", b"hello fabric".to_vec()),
        ("text", text.clone()),
        ("noise", noise(150_000)),
        ("runs", b"logos".repeat(60_000)),
        ("mixed", mixed),
    ];
    for (name, data) in &cases {
        let packed = image::pack(data);
        let path = dir.join("packed.bin");
        std::fs::write(&path, &packed).unwrap();
        let out = std::process::Command::new(&bin).arg(&path).arg((data.len() + 8).to_string()).output().unwrap();
        assert!(out.stdout == *data, "{name}: the boot ROM inflates something other than what the image builder packed");
    }
    let packed = image::pack(&text);
    let path = dir.join("packed.bin");
    std::fs::write(&path, &packed[..packed.len() / 2]).unwrap();
    let out = std::process::Command::new(&bin).arg(&path).arg((text.len() + 8).to_string()).output().unwrap();
    assert_eq!(out.stdout, b"REFUSED", "a truncated stream must be refused");
    std::fs::write(&path, &packed).unwrap();
    let out = std::process::Command::new(&bin).arg(&path).arg("100").output().unwrap();
    assert_eq!(out.stdout, b"REFUSED", "output past the buffer must be refused");
}

#[test]
fn the_boot_roms_streaming_digest_is_sha256() {
    if !common::require_tool("cc") {
        return;
    }
    let bin = native(
        r#"
#include <stdio.h>
#include <stdlib.h>
#include "boot.h"
int main(int argc, char **argv) {
  FILE *f = fopen(argv[1], "rb");
  struct sha256 c;
  sha256_init(&c);
  int ch;
  while ((ch = fgetc(f)) != EOF) sha256_byte(&c, (uint8_t)ch);
  uint8_t d[32];
  sha256_final(&c, d);
  fwrite(d, 1, 32, stdout);
  return 0;
}
"#,
        &["sha256.c", "libc.c"],
        "sha256",
    );
    let dir = bin.parent().unwrap().to_path_buf();
    for len in [0usize, 1, 55, 56, 63, 64, 65, 119, 120, 127, 128, 1000, 1 << 20] {
        let data: Vec<u8> = (0..len).map(|i| (i * 131 + 7) as u8).collect();
        let path = dir.join("data.bin");
        std::fs::write(&path, &data).unwrap();
        let out = std::process::Command::new(&bin).arg(&path).output().unwrap();
        assert_eq!(out.stdout, sha256(&data).to_vec(), "{len} bytes digest differently");
    }
}

#[test]
fn the_firmware_fits_the_memory_the_soc_gives_it() {
    if !common::require_tool("riscv64-linux-gnu-gcc") {
        return;
    }
    let fw = onboard_board::firmware_c();
    assert!(fw.boot.len() as u64 <= soc::ROM_SIZE, "the boot ROM is {} bytes; the ROM holds {}", fw.boot.len(), soc::ROM_SIZE);
    assert!(
        fw.shim.len() as u64 <= soc::SHIM_SIZE - 0x10000,
        "the shim is {} bytes; its region keeps 64 KB for its stack",
        fw.shim.len()
    );
}
