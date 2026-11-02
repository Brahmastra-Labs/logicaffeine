//! The board's whole software stack on the board model (`hart::soc::Board`): from the hart's reset vector the boot
//! ROM checks and tests the DDR2, reads the boot image's header from the flash, refuses an image built for another
//! bitstream, inflates the image into the DDR2 while digesting it, and starts the shim, which loads a static riscv64
//! Linux program — the LOGOS compiler itself, or a test program — and runs it in user mode, answering its system
//! calls and its floating-point instructions. Programs that run under `qemu-riscv64` must write exactly what they
//! write there; every refusal must name its reason.
//!
//! Tool-gated: `riscv64-linux-gnu-gcc` and `qemu-riscv64`.

mod common;
#[path = "support/onboard_board.rs"]
mod onboard_board;
#[path = "support/rv_fuzz.rs"]
mod rv_fuzz;

use logicaffeine_compile::codegen_sva::engine_build::Profile;
use logicaffeine_compile::codegen_sva::hart::soc::Stopped;
use logicaffeine_compile::codegen_sva::onboard;
use onboard_board::{boot, flash_with, image_of, on_qemu, program, root, tools, RTL_MD5};

/// Instructions a small program's boot and run may take.
const SMALL: u64 = 50_000_000;

const HELLO: &str = "  .text\n  .globl _start\n_start:\n  li a7, 64\n  li a0, 1\n  la a1, msg\n  li a2, 21\n  ecall\n\
                     \x20 li a7, 93\n  li a0, 0\n  ecall\n  .data\nmsg: .ascii \"hello from the board\\n\"\n";

#[test]
fn the_board_boots_a_program_from_its_flash() {
    if !tools() {
        return;
    }
    let elf = program(HELLO, "rv64imac", "hello");
    let run = boot(flash_with(&image_of(elf, vec![], vec![])), RTL_MD5, SMALL);
    let log = run.log();
    assert_eq!(run.stopped, Stopped::Exited(0), "the board did not stop at the program's exit:\n{log}");
    for line in ["LOGOS boot", "boot: DDR2 ready", "boot: starting the shim", "shim: entering the compiler at"] {
        assert!(log.contains(line), "the boot never said {line:?}:\n{log}");
    }
    assert_eq!(run.program_output(), b"hello from the board\n");
}

#[test]
fn a_boot_image_for_another_bitstream_is_refused() {
    if !tools() {
        return;
    }
    let elf = program(HELLO, "rv64imac", "other_bitstream");
    let run = boot(flash_with(&image_of(elf, vec![], vec![])), *b"another-bitstrm!", SMALL);
    assert_eq!(run.stopped, Stopped::Exited(0x200));
    assert!(run.log().contains("the boot image was built for a different bitstream"), "{}", run.log());
}

#[test]
fn a_damaged_boot_image_is_refused() {
    if !tools() {
        return;
    }
    let elf = program(HELLO, "rv64imac", "damaged");
    let mut image = image_of(elf, vec![], vec![]);
    let at = image.len() - 40;
    image[at] ^= 0x10;
    let run = boot(flash_with(&image), RTL_MD5, SMALL);
    assert_eq!(run.stopped, Stopped::Exited(0x200));
    assert!(run.log().contains("the boot image's digest does not match"), "{}", run.log());
}

#[test]
fn an_empty_flash_is_refused() {
    if !tools() {
        return;
    }
    let run = boot(vec![0xff; 16 << 20], RTL_MD5, SMALL);
    assert_eq!(run.stopped, Stopped::Exited(0x200));
    assert!(run.log().contains("the flash holds no boot image"), "{}", run.log());
}

/// A program's fault stops the board with the shim's named reason and its fatal status.
fn refused_with(asm: &str, label: &str, reason: &str) {
    let elf = program(asm, "rv64imac", label);
    let run = boot(flash_with(&image_of(elf, vec![], vec![])), RTL_MD5, SMALL);
    assert_eq!(run.stopped, Stopped::Exited(0x17f), "{label}: the shim did not stop the board:\n{}", run.log());
    assert!(run.log().contains(reason), "{label}: not refused for {reason:?}:\n{}", run.log());
}

#[test]
fn the_shim_names_what_it_will_not_do() {
    if !tools() {
        return;
    }
    refused_with(
        "  .globl _start\n_start:\n  li a7, 999\n  ecall\n",
        "unknown_syscall",
        "an unimplemented system call",
    );
    refused_with(
        "  .globl _start\n_start:\n  li t0, 0x80000000\n  sd zero, 0(t0)\n",
        "into_the_shim",
        "a store the compiler may not make",
    );
    refused_with(
        "  .globl _start\n_start:\n  li t0, 0x87800010\n  sd zero, 0(t0)\n",
        "stack_overflow",
        "the compiler overflowed its stack",
    );
    refused_with(
        "  .globl _start\n_start:\n  li t0, 0x10001000\n  sd zero, 0(t0)\n",
        "a_device",
        "a store the compiler may not make",
    );
    refused_with(
        "  .globl _start\n_start:\n  .word 0x0000007f\n",
        "illegal",
        "an illegal instruction",
    );
}

/// The F and D extensions the shim emulates, against qemu's native ones: random programs of every floating-point
/// operation, both formats, every rounding mode, the CSRs and misaligned floating-point loads and stores.
#[test]
fn floating_point_on_the_board_is_floating_point_on_qemu() {
    if !tools() {
        return;
    }
    let seed = std::env::var("ONBOARD_FP_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0xf10a_7001u64);
    for n in 0..24 {
        let asm = rv_fuzz::fp_program(seed + n, 250);
        let elf = program(&asm, "rv64imafdc", &format!("fp{n}"));
        let qemu = on_qemu(&elf, &format!("fp{n}"));
        assert_eq!(qemu.len(), 776, "fp program {}: qemu wrote {} bytes", seed + n, qemu.len());
        let run = boot(flash_with(&image_of(elf, vec![], vec![])), RTL_MD5, SMALL);
        assert_eq!(run.stopped, Stopped::Exited(0), "fp program {}:\n{}", seed + n, run.log());
        let board = run.program_output();
        if board != qemu {
            let words = |b: &[u8]| -> Vec<u64> { b.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect() };
            let (q, m) = (words(&qemu), words(&board));
            let diff: Vec<String> = (0..q.len().min(m.len()))
                .filter(|&i| q[i] != m[i])
                .map(|i| {
                    let what = match i {
                        0..=31 => format!("x{i}"),
                        32..=63 => format!("f{}", i - 32),
                        64 => "fcsr".to_string(),
                        _ => format!("buf[{}]", (i - 65) * 8),
                    };
                    format!("{what}: qemu {:#x}, board {:#x}", q[i], m[i])
                })
                .collect();
            panic!("fp program {} differs:\n{}\n{asm}", seed + n, diff.join("\n"));
        }
    }
}

/// THE claim: the LOGOS compiler, booted from the flash on the board's own CPU, compiles every program of the
/// terminal's library — from the source text in the boot image's bundle — to the same register bytecode, fabric wire
/// image and VM golden the laptop's compiler produces.
#[test]
fn the_board_compiles_the_library_as_the_laptop_does() {
    if !(tools() && common::require_tool("riscv64-musl")) {
        return;
    }
    let out = std::process::Command::new("bash")
        .arg(root().join("scripts/hw/build-onboard-firmware.sh"))
        .arg("onboard_compile")
        .output()
        .expect("run scripts/hw/build-onboard-firmware.sh");
    assert!(out.status.success(), "the compiler does not build for the hart:\n{}", String::from_utf8_lossy(&out.stderr));
    let elf_path = String::from_utf8_lossy(&out.stdout).lines().last().unwrap().to_string();
    let dir = std::env::temp_dir().join(format!("onboard_boot_lock_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let stripped = dir.join("compiler");
    let strip = std::process::Command::new("riscv64-linux-gnu-strip")
        .arg("-o")
        .arg(&stripped)
        .arg(&elf_path)
        .status()
        .expect("run riscv64-linux-gnu-strip");
    assert!(strip.success());
    let firmware = std::fs::read(&stripped).unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let programs = root().join("benchmarks/silicon/programs");
    let library_text = std::fs::read_to_string(programs.join("terminal_library.txt")).unwrap();
    let entries = onboard::library(&library_text).expect("the library parses");
    let mut bundle = vec![(onboard_board::library_path("terminal_library.txt"), library_text.clone().into_bytes())];
    for e in &entries {
        bundle.push((onboard_board::library_path(&e.program), std::fs::read(programs.join(&e.program)).unwrap()));
    }
    let args = vec!["--library".to_string(), onboard_board::library_path("terminal_library.txt")];
    let image = image_of(firmware, bundle, args);

    // No hidden tricks: the image holds the programs' source and the compiler, never what the laptop compiled them
    // to — no program's register bytecode and no wire image appears anywhere in what the board inflates.
    let (_, body) = logicaffeine_compile::codegen_sva::hart::image::open(&image).expect("the image opens");
    let mut want = String::new();
    for e in &entries {
        let src = std::fs::read_to_string(programs.join(&e.program)).unwrap();
        let compiled = onboard::compile(&src, &e.run, Profile::Hw, &onboard::hw_caps());
        if let Ok(c) = &compiled {
            for (what, bytes) in [("register bytecode", &c.bytecode), ("wire image", &c.wire)] {
                assert!(bytes.len() >= 16, "{}: its {what} is too short to search for", e.title);
                assert!(
                    !body.windows(bytes.len()).any(|w| w == bytes.as_slice()),
                    "the boot image carries {}'s {what}, compiled on the laptop",
                    e.title
                );
            }
        }
        want.push_str(&format!("entry={}\n", e.title));
        want.push_str(&onboard::report(&compiled));
    }

    let run = boot(flash_with(&image), RTL_MD5, 40_000_000_000);
    assert_eq!(run.stopped, Stopped::Exited(0), "the compiler did not finish on the board:\n{}", run.log());
    let got = String::from_utf8_lossy(&run.program_output()).to_string();
    assert_eq!(got, want, "the board's compiler and the laptop's disagree");
}
