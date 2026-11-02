//! The board as the onboard suites run it: the firmware's C built through `scripts/hw/build-onboard-c.sh` against the
//! `soc.h` the hart's SoC definition writes, static programs assembled at the compiler's address, boot images packed
//! by `hart::image`, and the board model (`hart::soc::Board`) booted from its reset vector until the firmware stops.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use logicaffeine_compile::codegen_sva::hart::image::{self, BootImage};
use logicaffeine_compile::codegen_sva::hart::soc::{self, Board, Stopped};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The fingerprint every image here is built for, and the board model reports.
pub const RTL_MD5: [u8; 16] = *b"onboard-suite-01";

/// The boot ROM and the shim, built once per test binary.
pub struct FirmwareC {
    pub boot: Vec<u8>,
    pub shim: Vec<u8>,
}

pub fn firmware_c() -> &'static FirmwareC {
    static BUILT: OnceLock<FirmwareC> = OnceLock::new();
    BUILT.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("onboard_firmware_c_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let header = dir.join("generated_soc.h");
        std::fs::write(&header, soc::c_header()).unwrap();
        let out = std::process::Command::new("bash")
            .arg(root().join("scripts/hw/build-onboard-c.sh"))
            .arg(dir.join("build"))
            .arg(&header)
            .output()
            .expect("run scripts/hw/build-onboard-c.sh");
        assert!(out.status.success(), "the firmware's C does not build:\n{}", String::from_utf8_lossy(&out.stderr));
        let fw = FirmwareC {
            boot: std::fs::read(dir.join("build/boot.bin")).unwrap(),
            shim: std::fs::read(dir.join("build/shim.bin")).unwrap(),
        };
        let _ = std::fs::remove_dir_all(&dir);
        fw
    })
}

/// Assemble `asm` as a static riscv64 Linux program at the compiler's address (`FIRMWARE_BASE`), as the
/// compiler itself is linked; its ELF's bytes.
pub fn program(asm: &str, march: &str, label: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("onboard_program_{}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, elf) = (dir.join("p.s"), dir.join("p"));
    std::fs::write(&src, asm).unwrap();
    let abi = if march.contains('d') || march.contains('g') { "-mabi=lp64d" } else { "-mabi=lp64" };
    let out = std::process::Command::new("riscv64-linux-gnu-gcc")
        .arg(format!("-march={march}_zicsr_zifencei"))
        .arg(abi)
        .args(["-nostdlib", "-static", "-no-pie"])
        .arg(format!("-Wl,-Ttext-segment={:#x}", soc::FIRMWARE_BASE))
        .arg(&src)
        .arg("-o")
        .arg(&elf)
        .output()
        .expect("run riscv64-linux-gnu-gcc");
    assert!(out.status.success(), "{label} does not assemble:\n{}\n{asm}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&elf).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    bytes
}

/// The flash a board boots: an erased 16 MB with the boot image at `IMAGE_OFFSET`.
pub fn flash_with(image: &[u8]) -> Vec<u8> {
    let mut flash = vec![0xff; soc::FLASH_SIZE as usize];
    let at = soc::IMAGE_OFFSET as usize;
    flash[at..at + image.len()].copy_from_slice(image);
    flash
}

/// A boot image for `firmware` with `bundle` and `args`, built for [`RTL_MD5`].
pub fn image_of(firmware: Vec<u8>, bundle: Vec<(String, Vec<u8>)>, args: Vec<String>) -> Vec<u8> {
    image::build(&BootImage {
        rtl_md5: RTL_MD5,
        shim: firmware_c().shim.clone(),
        firmware,
        bundle,
        args,
    })
}

/// A run of the board: how it stopped, and everything the log UART carried.
pub struct Run {
    pub stopped: Stopped,
    pub uart: Vec<u8>,
}

impl Run {
    pub fn log(&self) -> String {
        String::from_utf8_lossy(&self.uart).to_string()
    }

    /// What the program itself wrote: the log between the shim's "entering" line and its exit line.
    pub fn program_output(&self) -> Vec<u8> {
        let log = &self.uart;
        let enter = b"shim: entering the compiler at ";
        let start = log
            .windows(enter.len())
            .position(|w| w == enter)
            .and_then(|p| log[p..].iter().position(|&c| c == b'\n').map(|q| p + q + 1))
            .unwrap_or_else(|| panic!("the shim never entered the program:\n{}", self.log()));
        let exit = b"\nshim: the compiler exited with status ";
        let end = log[start..]
            .windows(exit.len())
            .rposition(|w| w == exit)
            .map(|e| start + e)
            .unwrap_or(log.len());
        log[start..end].to_vec()
    }
}

/// Boot the board from its reset vector with `flash`, reporting `rtl_md5`, for up to `steps` instructions.
pub fn boot(flash: Vec<u8>, rtl_md5: [u8; 16], steps: u64) -> Run {
    let mut board = Board::new(&firmware_c().boot, flash, rtl_md5);
    let stopped = board.run(steps);
    Run { stopped, uart: board.devices.uart.clone() }
}

/// Run `elf` under `qemu-riscv64`; its standard output.
pub fn on_qemu(elf: &[u8], label: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("onboard_qemu_{}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("p");
    std::fs::write(&path, elf).unwrap();
    // qemu-user runs only an executable file; it refuses any other without a word.
    let mut mode = std::fs::metadata(&path).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&path, mode).unwrap();
    let out = std::process::Command::new("timeout").arg("120").arg("qemu-riscv64").arg(&path).output().expect("qemu");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{label} failed under qemu-riscv64: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

pub fn tools() -> bool {
    super::common::require_tool("riscv64-linux-gnu-gcc") && super::common::require_tool("qemu-riscv64")
}

/// The path a bundled file has on the board.
pub fn library_path(name: &str) -> String {
    format!("/library/{name}")
}

pub fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}
