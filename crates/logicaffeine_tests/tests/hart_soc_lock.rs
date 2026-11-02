//! The board's whole computer, `hart_soc` — the hart, its caches, the boot ROM, the scratch RAM and every device
//! register — under iverilog against the board model `hart::soc::Board`, in lockstep (`support/hart_soc_sim.rs`): every
//! instruction, every store, every device read, every byte on the log UART and the terminal link, every restart and
//! the exit status must be the model's.
//!
//! A device test program in the ROM reaches every register, the flash stream (wrapping past the flash's last byte),
//! the link, the scratch RAM's widths and atomics, the ROM read as data, the DDR2 through the cache, code run from the
//! DDR2 after fence.i, and a restart. Then the real boot ROM and shim boot a program from the flash, and refuse the
//! flashes they must.
//!
//! Tool-gated: `iverilog` and `riscv64-linux-gnu-gcc`.

mod common;
#[path = "support/hart_soc_sim.rs"]
mod hart_soc_sim;
#[path = "support/onboard_board.rs"]
mod onboard_board;

use hart_soc_sim::{board_for, lockstep, run, Setup, SocRun};
use logicaffeine_compile::codegen_sva::hart::soc;

/// Assemble `asm` as the boot ROM: its text at the ROM's base, the reset vector first.
fn rom_program(asm: &str, label: &str) -> Vec<u8> {
    let dir = std::env::temp_dir().join(format!("hart_soc_lock_{}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (src, elf, bin) = (dir.join("rom.s"), dir.join("rom.elf"), dir.join("rom.bin"));
    std::fs::write(&src, asm).unwrap();
    let out = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-march=rv64imac_zicsr_zifencei", "-mabi=lp64", "-nostdlib", "-static", "-fno-pie", "-no-pie", "-Wl,--build-id=none"])
        .arg(format!("-Wl,-Ttext={:#x}", soc::ROM_BASE))
        .arg(&src)
        .arg("-o")
        .arg(&elf)
        .output()
        .expect("run riscv64-linux-gnu-gcc");
    assert!(out.status.success(), "{label} does not assemble:\n{}", String::from_utf8_lossy(&out.stderr));
    let out = std::process::Command::new("riscv64-linux-gnu-objcopy")
        .args(["-O", "binary", "-j", ".text"])
        .arg(&elf)
        .arg(&bin)
        .output()
        .expect("run riscv64-linux-gnu-objcopy");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let bytes = std::fs::read(&bin).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bytes.len() as u64 <= soc::ROM_SIZE, "{label} is {} bytes", bytes.len());
    bytes
}

/// Where the device test's flash stream begins: 64 bytes before the flash's end, so a stream runs off its end.
const STREAM_AT: u64 = soc::FLASH_SIZE - 64;

fn reg(name: &str) -> u64 {
    soc::REGS.iter().find(|r| r.name == name).unwrap().addr
}

fn device_test() -> String {
    format!(
        r#"
  .text
  .globl _start
_start:
  li s0, {sys:#x}
  li s1, {uart:#x}
  li s2, {link:#x}
  li s3, {buttons:#x}
  li s4, {qspi:#x}
  li s5, {ddr:#x}
  li s7, {sram:#x}
  # A word in the DDR2 counts the boots; a restart leaves it as it was.
  li t0, {ram:#x} + 0x100000
  ld t1, 0(t0)
  addi t1, t1, 1
  sd t1, 0(t0)
  li t2, 2
  beq t1, t2, second_boot

  # The registers that never change, and addresses no register holds.
  ld a0, 0x00(s0)
  ld a0, 0x18(s0)
  ld a0, 0x20(s0)
  ld a0, 0x28(s0)
  ld a0, 0x08(s0)
  ld a0, 0x00(s3)
  li a1, 0x1a5c3
  sd a1, 0x08(s3)
  ld a0, 0x08(s3)
  ld a0, 0x40(s0)
  sd a1, 0x40(s0)
  ld a0, 0x18(s1)
  li t0, {io:#x} + 0x6000
  sd a1, 0(t0)
  ld a0, 0(t0)
  li t0, {io:#x} + {io_size:#x} - 8
  ld a0, 0(t0)

  # The DDR2's calibration.
1:
  ld a0, 0(s5)
  andi a0, a0, 1
  beqz a0, 1b
  ld a0, 0x10(s0)
  ld a0, 0x08(s5)

  # The log, waiting whenever its queue is full.
  la a2, msg
  la a3, msg_end
2:
  lbu a4, 0(a2)
3:
  ld a5, 0x08(s1)
  andi a5, a5, 1
  bnez a5, 3b
  sd a4, 0(s1)
  addi a2, a2, 1
  bne a2, a3, 2b

  # The flash: all ones with no stream; a stream; one that runs off the flash's end onto its start; a stream begun
  # at an odd byte; and the stream stopped.
  ld a0, 0x08(s4)
  li a1, {stream_at:#x}
  sd a1, 0(s4)
  ld a0, 0x08(s4)
  ld a0, 0x08(s4)
  ld a0, 0x08(s4)
  ld a0, 0x08(s4)
  ld a0, 0x08(s4)
  ld a0, 0x08(s4)
  li a1, {flash_end:#x} - 4
  sd a1, 0(s4)
  ld a0, 0x08(s4)
  ld a0, 0x08(s4)
  sd zero, 0x10(s4)
  ld a0, 0x08(s4)
  li a1, {stream_at:#x} + 3
  sd a1, 0(s4)
  ld a0, 0x08(s4)
  sd zero, 0x10(s4)

  # The link: every byte the terminal sends comes back, up to and including '!'.
4:
  ld a0, 0x10(s2)
  andi a0, a0, 2
  beqz a0, 4b
  ld a0, 0x08(s2)
  andi a1, a0, 0xff
5:
  ld a5, 0x10(s2)
  andi a5, a5, 1
  bnez a5, 5b
  sd a1, 0(s2)
  li a5, 0x21
  bne a1, a5, 4b
  ld a0, 0x08(s2)

  # The scratch RAM: every width, a load straight after a store to its word, atomics, a reservation.
  li a1, 0x0123456789abcdef
  sd a1, 0(s7)
  sw a1, 8(s7)
  sh a1, 12(s7)
  sb a1, 14(s7)
  ld a0, 0(s7)
  lw a0, 8(s7)
  lwu a0, 8(s7)
  lh a0, 12(s7)
  lhu a0, 12(s7)
  lb a0, 14(s7)
  lbu a0, 14(s7)
  sd a1, 16(s7)
  ld a0, 16(s7)
  sb s0, 17(s7)
  ld a0, 16(s7)
  addi t0, s7, 24
  amoadd.d a0, a1, (t0)
  amoswap.w a0, s1, (t0)
  amomaxu.d a0, a1, (t0)
  lr.d a0, (t0)
  sc.d a2, a1, (t0)
  ld a0, 24(s7)
  li t0, {sram:#x} + {sram_size:#x} - 8
  sd a1, 0(t0)
  ld a0, 0(t0)

  # The boot ROM, read as data.
  la t0, msg
  ld a0, 0(t0)
  lbu a0, 3(t0)
  lh a0, 6(t0)

  # The DDR2 through the cache: three lines that share a cache line, written and read back.
  li t0, {ram:#x} + 0x200000
  li t1, {ram:#x} + 0x204000
  li t2, {ram:#x} + 0x208000
  sd a1, 0(t0)
  sd s0, 8(t1)
  sw s1, 16(t2)
  ld a0, 0(t0)
  ld a0, 8(t1)
  lw a0, 16(t2)

  # Code copied into the DDR2 and run there once fence.i has made it visible.
  la t0, ram_code
  li t1, {ram:#x} + 0x300000
  lw t2, 0(t0)
  sw t2, 0(t1)
  lw t2, 4(t0)
  sw t2, 4(t1)
  fence.i
  jalr ra, 0(t1)

  # The memory map's faults, each stepped over by the handler: part of a device register read and written, an
  # atomic on a device, code in the scratch RAM, an address past the devices.
  la t0, trap
  csrw mtvec, t0
  lw a0, 0(s0)
  sh a1, 0(s1)
  amoadd.d a0, a1, (s0)
  li t0, {sram:#x}
  jalr ra, 0(t0)
  li t0, {io:#x} + {io_size:#x}
  ld a0, 0(t0)
  li t0, {rom:#x} + 0x100
  sd a1, 0(t0)

  # Restart: a wrong key is ignored, the right one restarts the board. Nothing after the restarting store runs —
  # the ecall would trap at once if it did.
  li a1, 0x1234
  sd a1, 0x30(s0)
  li a1, {key:#x}
  sd a1, 0x30(s0)
  ecall
6:
  j 6b

second_boot:
  ld a0, 0x00(s0)
  li a1, 0x5a
  sd a1, 0x38(s0)
  ecall
7:
  j 7b

  # Step over the faulting instruction (two bytes or four); a fetch fault returns to the caller instead.
  .balign 4
trap:
  csrr s8, mcause
  csrr s9, mepc
  li s10, 1
  beq s8, s10, 8f
  lhu s10, 0(s9)
  andi s10, s10, 3
  addi s9, s9, 2
  li s11, 3
  bne s10, s11, 9f
  addi s9, s9, 2
9:
  csrw mepc, s9
  mret
8:
  csrw mepc, ra
  mret

  .option push
  .option norvc
ram_code:
  addi a0, a0, 7
  jalr x0, 0(ra)
  .option pop
  .balign 8
msg:
  .ascii "device test: the log UART, waiting on its queue\n"
msg_end:
"#,
        sys = reg("SYS_ID"),
        uart = reg("UART_TX"),
        link = reg("LINK_TX"),
        buttons = reg("BUTTONS_STATE"),
        qspi = reg("QSPI_START"),
        ddr = reg("DDR_STATUS"),
        sram = soc::SRAM_BASE,
        sram_size = soc::SRAM_SIZE,
        ram = soc::RAM_BASE,
        io = soc::IO_BASE,
        io_size = soc::IO_SIZE,
        rom = soc::ROM_BASE,
        stream_at = STREAM_AT,
        flash_end = soc::FLASH_SIZE,
        key = soc::SOC_RESET_KEY,
    )
}

/// The device test's flash: a pattern in the stream's 64 bytes and in the flash's first 16, erased elsewhere.
fn device_flash() -> Vec<u8> {
    let mut flash = vec![0xff; soc::FLASH_SIZE as usize];
    for i in 0..64 {
        flash[STREAM_AT as usize + i] = (i as u8).wrapping_mul(37) ^ 0x5c;
    }
    for i in 0..16 {
        flash[i] = 0xa0 + i as u8;
    }
    flash
}

fn device_setup(slow: Option<u32>, log_every: u32) -> Setup {
    Setup {
        rom: rom_program(&device_test(), &format!("devices_{log_every}")),
        flash: device_flash(),
        switches: 0xb00f,
        buttons: 0x15,
        link_in: b"hi, board!".to_vec(),
        slow,
        log_every,
        cycles: 400_000,
    }
}

fn checked(setup: &Setup, label: &str) -> SocRun {
    let run = run(setup, label);
    assert!(!run.trap_loop, "{label}: the hart traps without end:\n{}", run.log_text());
    assert!(!run.timed_out, "{label}: the SoC never stopped:\n{}", run.log_text());
    let mut board = board_for(setup);
    lockstep(&run, &mut board, label).unwrap_or_else(|e| panic!("{e}"));
    run
}

#[test]
fn every_device_register_does_what_the_board_model_says() {
    if !hart_soc_sim::tools() {
        return;
    }
    let run = checked(&device_setup(None, 1), "devices");
    assert_eq!(run.exit, Some(0x5a), "the device test did not reach its second boot's exit");
    assert_eq!(run.epochs.len(), 2, "the device test restarts once");
    assert_eq!(run.link_out, b"hi, board!", "the link did not echo");
    assert_eq!(run.log_text(), "device test: the log UART, waiting on its queue\n");
    let causes: Vec<u64> = run.epochs[0]
        .records
        .iter()
        .filter_map(|r| match r {
            hart_soc_sim::Rec::Trap { cause, .. } => Some(*cause),
            _ => None,
        })
        .collect();
    assert_eq!(causes, [5, 7, 7, 1, 5, 7], "the memory map's faults, in order");
}

/// The same, with the DDR2 waiting randomly at every step of its line port and the log UART taking a byte only every
/// thirteenth cycle — so the queue fills and the program waits on it.
#[test]
fn every_device_register_holds_on_slow_memory_and_a_slow_log() {
    if !hart_soc_sim::tools() {
        return;
    }
    for seed in [1u32, 0x50c_0002, 0x50c_0003] {
        let run = checked(&device_setup(Some(seed), 13), &format!("devices_slow_{seed:x}"));
        assert_eq!(run.exit, Some(0x5a));
    }
}

const HELLO: &str = "  .text\n  .globl _start\n_start:\n  li a7, 64\n  li a0, 1\n  la a1, msg\n  li a2, 21\n  ecall\n\
                     \x20 li a7, 93\n  li a0, 0\n  ecall\n  .data\nmsg: .ascii \"hello from the board\\n\"\n";

fn boot_setup(flash: Vec<u8>) -> Setup {
    Setup {
        rom: onboard_board::firmware_c().boot.clone(),
        flash,
        switches: 0,
        buttons: 0,
        link_in: Vec::new(),
        slow: None,
        log_every: 1,
        cycles: 12_000_000,
    }
}

/// The real boot ROM, from the hart's reset vector: the DDR2 tested, the boot image read from the flash, inflated and
/// digested, the shim installed and started, the program loaded and run in user mode — on the RTL as on the model.
#[test]
fn the_soc_boots_a_program_from_its_flash() {
    if !(hart_soc_sim::tools() && onboard_board::tools()) {
        return;
    }
    let elf = onboard_board::program(HELLO, "rv64imac", "soc_hello");
    let setup = boot_setup(onboard_board::flash_with(&onboard_board::image_of(elf, vec![], vec![])));
    let run = checked(&setup, "boot_hello");
    assert_eq!(run.exit, Some(0), "{}", run.log_text());
    assert!(run.log_text().contains("hello from the board\n"), "{}", run.log_text());
}

#[test]
fn the_soc_refuses_an_empty_flash() {
    if !hart_soc_sim::tools() {
        return;
    }
    let run = checked(&boot_setup(vec![0xff; soc::FLASH_SIZE as usize]), "boot_empty");
    assert_eq!(run.exit, Some(0x200));
    assert!(run.log_text().contains("the flash holds no boot image"), "{}", run.log_text());
}

#[test]
fn the_soc_refuses_an_image_for_another_bitstream() {
    if !(hart_soc_sim::tools() && onboard_board::tools()) {
        return;
    }
    let elf = onboard_board::program(HELLO, "rv64imac", "soc_other");
    let image = logicaffeine_compile::codegen_sva::hart::image::build(&logicaffeine_compile::codegen_sva::hart::image::BootImage {
        rtl_md5: *b"another-bitstrm!",
        shim: onboard_board::firmware_c().shim.clone(),
        firmware: elf,
        bundle: vec![],
        args: vec![],
    });
    let run = checked(&boot_setup(onboard_board::flash_with(&image)), "boot_other");
    assert_eq!(run.exit, Some(0x200));
    assert!(run.log_text().contains("the boot image was built for a different bitstream"), "{}", run.log_text());
}

/// The XC7A100T's configuration bitstream: the flash holds it from byte 0.
const BITSTREAM_BYTES: u64 = 3_825_788;

/// `hart::soc` is the one definition of the board's addresses; it must hold together: every register inside the
/// device region, whole-word aligned, distinct, inside its device's 4 KB; the DDR2's layout inside the RAM, in order,
/// with nothing overlapping; the boot image clear of the bitstream; and both generated headers naming every constant
/// and register exactly once.
#[test]
fn the_memory_map_is_one_consistent_definition() {
    let mut seen = std::collections::BTreeSet::new();
    for r in soc::REGS {
        assert!((soc::IO_BASE..soc::IO_BASE + soc::IO_SIZE).contains(&r.addr), "{} is outside the devices", r.name);
        assert_eq!(r.addr % 8, 0, "{} is not a whole word", r.name);
        assert!(seen.insert(r.addr), "{} shares its address", r.name);
        let block = r.name.split('_').next().unwrap();
        let first = soc::REGS.iter().find(|q| q.name.starts_with(block)).unwrap().addr;
        assert_eq!(r.addr & !0xfff, first & !0xfff, "{} is outside its device's 4 KB", r.name);
    }
    let layout = [
        ("shim", soc::SHIM_BASE, soc::SHIM_SIZE),
        ("firmware", soc::FIRMWARE_BASE, soc::MMAP_BASE - soc::FIRMWARE_BASE),
        ("mmap", soc::MMAP_BASE, soc::MMAP_SIZE),
        ("staging", soc::STAGING_BASE, soc::STAGING_SIZE),
        ("stack", soc::STACK_BASE, soc::STACK_SIZE),
    ];
    assert_eq!(layout[0].1, soc::RAM_BASE, "the shim does not begin the RAM");
    for w in layout.windows(2) {
        assert!(w[0].1 + w[0].2 <= w[1].1, "{} runs into {}", w[0].0, w[1].0);
    }
    let (name, base, size) = layout[layout.len() - 1];
    assert!(base + size <= soc::RAM_BASE + soc::RAM_SIZE, "{name} runs past the RAM");
    assert!(soc::IMAGE_OFFSET >= BITSTREAM_BYTES, "the boot image would overwrite the bitstream");
    assert!(soc::IMAGE_OFFSET < soc::FLASH_SIZE);
    let regions = soc::memory_map();
    for (i, a) in regions.iter().enumerate() {
        for b in &regions[i + 1..] {
            assert!(a.base + a.size <= b.base || b.base + b.size <= a.base, "{a:?} overlaps {b:?}");
        }
    }
    assert_eq!(soc::RESET_PC, soc::ROM_BASE);
    let (c, v) = (soc::c_header(), soc::verilog_defines());
    for (name, value) in soc::CONSTANTS.iter().map(|(n, v)| (*n, *v)).chain(soc::REGS.iter().map(|r| (r.name, r.addr))) {
        assert_eq!(c.matches(&format!("#define {name} 0x{value:x}ull\n")).count(), 1, "soc.h and {name}");
        assert_eq!(v.matches(&format!("`define SOC_{name} 64'h{value:016x}\n")).count(), 1, "the defines and {name}");
    }
    assert_eq!(soc::HART_HWCAP, 0x112d, "the hart reports the extensions qemu-riscv64 reports");
    // The firmware's build links it where the shim loads it.
    let script = std::fs::read_to_string(onboard_board::root().join("scripts/hw/build-onboard-firmware.sh")).unwrap();
    assert!(
        script.contains(&format!("-Ttext-segment={:#x}", soc::FIRMWARE_BASE)),
        "scripts/hw/build-onboard-firmware.sh does not link the firmware at FIRMWARE_BASE ({:#x})",
        soc::FIRMWARE_BASE
    );
}

/// The queue between the hart's clock and the terminal's (`hart_cdc_fifo`) carries every byte, once and in order,
/// whichever clock is faster, with both sides starting and stalling at random.
#[test]
fn bytes_cross_between_unrelated_clocks_in_order() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join(format!("hart_soc_lock_{}_cdc", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = logicaffeine_compile::codegen_sva::hart::rtl::board_files()
        .into_iter()
        .find(|(n, _)| *n == "hart_cdc_fifo.v")
        .unwrap();
    std::fs::write(dir.join(file.0), file.1).unwrap();
    for (n, (w, r)) in [(16.0f64, 10.3f64), (10.0, 16.7), (8.0, 8.1)].into_iter().enumerate() {
        let tb = format!(
            r#"
`timescale 1ns/1ps
module tb;
  reg wclk = 1'b0, rclk = 1'b0, wrst = 1'b1, rrst = 1'b1;
  always #{wh} wclk = ~wclk;
  always #{rh} rclk = ~rclk;
  integer seed = {seed}, sent = 0, got = 0, bad = 0;
  reg push = 1'b0, pop = 1'b0;
  reg [7:0] din = 8'd0;
  wire full, empty;
  wire [7:0] head;
  hart_cdc_fifo #(.WIDTH(8), .DEPTH_BITS(3)) q (
    .wclk(wclk), .wrst(wrst), .push(push), .din(din), .full(full),
    .rclk(rclk), .rrst(rrst), .pop(pop), .head(head), .empty(empty));
  always @(posedge wclk) begin
    if (!wrst) begin
      if (push && !full) sent = sent + 1;
      push <= sent < 20000 && ($random(seed) & 3) != 0;
      din <= sent;
    end
  end
  always @(posedge rclk) begin
    if (!rrst) begin
      if (pop && !empty) begin
        if (head !== got[7:0]) bad = bad + 1;
        got = got + 1;
      end
      pop <= ($random(seed) & 3) != 0;
    end
  end
  initial begin
    #100 wrst = 1'b0;
    #37 rrst = 1'b0;
    #2000000 $display("RESULT sent=%0d got=%0d bad=%0d", sent, got, bad);
    $finish;
  end
endmodule
"#,
            wh = w / 2.0,
            rh = r / 2.0,
            seed = 0xcdc0 + n
        );
        std::fs::write(dir.join("tb.v"), tb).unwrap();
        let sim = dir.join("sim");
        let out = std::process::Command::new("iverilog")
            .args(["-g2012", "-o"])
            .arg(&sim)
            .arg(dir.join("tb.v"))
            .arg(dir.join("hart_cdc_fifo.v"))
            .output()
            .expect("run iverilog");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let out = std::process::Command::new("vvp").arg(&sim).output().expect("run vvp");
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        assert!(text.contains("RESULT sent=20000 got=20000 bad=0"), "clocks {w} ns / {r} ns:\n{text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// No hidden tricks: the images that boot the board on their own can be sent nothing. Their tops have no port that
/// receives a byte, and their pins leave the USB UART's receive line (C4) unconnected — the UART is a log the board
/// writes and the laptop only listens to.
#[test]
fn the_self_contained_images_cannot_be_sent_anything() {
    use logicaffeine_compile::codegen_sva::hart::rtl;
    for (top, files) in [("hart_bringup_top", rtl::bringup_files()), ("hart_ddr2_loopback_top", rtl::loopback_files())] {
        let source = &files.iter().find(|(n, _)| *n == format!("{top}.v")).unwrap().1;
        let header = &source[source.find(&format!("module {top}")).unwrap()..];
        let ports = &header[..header.find(");").unwrap()];
        let inputs: Vec<&str> = ports
            .lines()
            .filter(|l| l.trim_start().starts_with("input") || l.trim_start().starts_with("inout"))
            .collect();
        for line in &inputs {
            assert!(!line.contains("rx"), "{top} has a receive port: {line}");
        }
        let xdc = &files.iter().find(|(n, _)| *n == format!("{top}.xdc")).unwrap().1;
        assert!(!xdc.contains("PACKAGE_PIN C4 "), "{top} connects the USB UART's receive pin");
        assert!(!xdc.contains("uart_rx"), "{top} names a UART receive port");
        // The only inputs: the clock, switches, buttons, the flash's data out, and the DDR2's bidirectional lines.
        for line in &inputs {
            let name = line.split_whitespace().last().unwrap().trim_end_matches(',');
            assert!(
                ["clk", "sw", "btnc", "btnu", "btnl", "btnr", "btnd", "qspi_dq1", "ddr2_dq", "ddr2_dqs_p", "ddr2_dqs_n"]
                    .contains(&name),
                "{top} takes an input no self-contained board has: {name}"
            );
        }
    }
}
