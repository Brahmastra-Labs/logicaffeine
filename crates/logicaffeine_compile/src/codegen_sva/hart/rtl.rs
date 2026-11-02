//! The hart's RTL, as Verilog source files the board image and the simulations read. The hart is held to
//! [`super::model`] (and its decompressor to [`super::rvc`]) by `hart_rtl_lock`; the computer around it, to the board
//! model [`super::soc::Board`] by `hart_soc_lock`; the DDR2 controller, to the part's data sheet by `hart_ddr2_lock`.

use super::soc;

/// The hart itself: (file name, source).
pub fn core_files() -> Vec<(&'static str, String)> {
    vec![
        ("hart_rvc.v", include_str!("rtl/hart_rvc.v").to_string()),
        ("hart_muldiv.v", include_str!("rtl/hart_muldiv.v").to_string()),
        ("hart_core.v", include_str!("rtl/hart_core.v").to_string()),
    ]
}

/// The hart with its caches, `hart_cpu`: the hart's files, the instruction and data caches, their one way to memory,
/// and the maintenance (fence.i, restart) they share.
pub fn cpu_files() -> Vec<(&'static str, String)> {
    let mut files = core_files();
    files.extend([
        ("hart_icache.v", include_str!("rtl/hart_icache.v").to_string()),
        ("hart_dcache.v", include_str!("rtl/hart_dcache.v").to_string()),
        ("hart_mem.v", include_str!("rtl/hart_mem.v").to_string()),
        ("hart_cpu.v", include_str!("rtl/hart_cpu.v").to_string()),
    ]);
    files
}

/// The memory map's `define`s, which the computer's files include.
pub const DEFINES: &str = "soc_defines.vh";

/// The whole computer, `hart_soc`: [`cpu_files`], the boot ROM, the scratch RAM, the device registers, and [`DEFINES`]
/// written from [`soc::verilog_defines`]. The boot ROM's contents come from the `ROM_INIT` file the build names
/// ([`rom_hex`]).
pub fn soc_files() -> Vec<(&'static str, String)> {
    let mut files = cpu_files();
    files.extend([
        ("hart_rom.v", include_str!("rtl/hart_rom.v").to_string()),
        ("hart_sram.v", include_str!("rtl/hart_sram.v").to_string()),
        ("hart_fifo.v", include_str!("rtl/hart_fifo.v").to_string()),
        ("hart_qspi.v", include_str!("rtl/hart_qspi.v").to_string()),
        ("hart_io.v", include_str!("rtl/hart_io.v").to_string()),
        ("hart_soc.v", include_str!("rtl/hart_soc.v").to_string()),
        (DEFINES, soc::verilog_defines()),
    ]);
    files
}

/// The DDR2's parameters, which the controller includes.
pub const DDR2_PARAMS: &str = "hart_ddr2_params.vh";

/// The DDR2 controller, `hart_ddr2`, with [`DDR2_PARAMS`] written from [`super::ddr2::verilog_params`] for the board's
/// part, and its Artix-7 PHY, `hart_ddr2_phy` (device primitives: the board's build reads it, simulations do not).
pub fn ddr2_files() -> Vec<(&'static str, String)> {
    vec![
        ("hart_ddr2.v", include_str!("rtl/hart_ddr2.v").to_string()),
        (DDR2_PARAMS, super::ddr2::verilog_params(&super::ddr2::MT47H64M16)),
        ("hart_ddr2_phy.v", include_str!("rtl/hart_ddr2_phy.v").to_string()),
    ]
}

/// What the board's top adds around the computer and its DDR2: the queue that carries bytes between the hart's clock
/// and the terminal's.
pub fn board_files() -> Vec<(&'static str, String)> {
    let mut files = soc_files();
    files.extend(ddr2_files());
    files.push(("hart_cdc_fifo.v", include_str!("rtl/hart_cdc_fifo.v").to_string()));
    files
}

/// The bring-up image: the computer alone on Board B (`hart_bringup_top`), booting from the flash with its log on the
/// USB UART — [`board_files`], the log UART's transmitter, the top and its pins.
pub fn bringup_files() -> Vec<(&'static str, String)> {
    let mut files = board_files();
    files.extend([
        ("hart_uart_tx.v", include_str!("rtl/hart_uart_tx.v").to_string()),
        ("hart_bringup_top.v", include_str!("rtl/hart_bringup_top.v").to_string()),
        ("hart_bringup_top.xdc", include_str!("rtl/hart_bringup_top.xdc").to_string()),
    ]);
    files
}

/// The DDR2 PHY's board probe, `hart_ddr2_loopback_top`: the PHY driving its own pads with the DDR2 held idle and
/// sending what it reads back on the UART; its pins are the bring-up image's, those its ports use.
pub fn loopback_files() -> Vec<(&'static str, String)> {
    let xdc: String = include_str!("rtl/hart_bringup_top.xdc")
        .lines()
        .filter(|l| !(l.contains("{ qspi_") || l.contains("{ btn")))
        .map(|l| format!("{l}\n"))
        .collect();
    vec![
        ("hart_ddr2_phy.v", include_str!("rtl/hart_ddr2_phy.v").to_string()),
        ("hart_uart_tx.v", include_str!("rtl/hart_uart_tx.v").to_string()),
        ("hart_ddr2_loopback_top.v", include_str!("rtl/hart_ddr2_loopback_top.v").to_string()),
        ("hart_ddr2_loopback_top.xdc", xdc),
    ]
}

/// A bitstream's fingerprint — what a boot image is built for, and what `SYS_RTL_MD5` reports: the md5 of every file the
/// bitstream is built from, in order, then of the boot ROM's words, so a changed ROM is another bitstream too.
pub fn fingerprint(files: &[(&'static str, String)], rom_hex: &str) -> [u8; 16] {
    let mut all = Vec::new();
    for (name, text) in files {
        all.extend_from_slice(name.as_bytes());
        all.push(0);
        all.extend_from_slice(text.as_bytes());
        all.push(0);
    }
    all.extend_from_slice(rom_hex.as_bytes());
    logicaffeine_base::hash::md5(&all)
}

/// A fingerprint as the SoC's `RTL_MD5_LO` / `RTL_MD5_HI` parameters take it: the words `SYS_RTL_MD5_LO` and
/// `SYS_RTL_MD5_HI` read as, its bytes little-endian.
pub fn fingerprint_words(f: [u8; 16]) -> (u64, u64) {
    (u64::from_le_bytes(f[0..8].try_into().unwrap()), u64::from_le_bytes(f[8..16].try_into().unwrap()))
}

/// The boot ROM as the `$readmemh` words its block RAM starts with: a 64-bit little-endian word a line, zeros to the
/// ROM's end.
pub fn rom_hex(boot: &[u8]) -> Result<String, String> {
    if boot.len() as u64 > soc::ROM_SIZE {
        return Err(format!("the boot ROM is {} bytes; the ROM holds {}", boot.len(), soc::ROM_SIZE));
    }
    Ok((0..(soc::ROM_SIZE / 8) as usize)
        .map(|i| {
            let word = (0..8).rev().fold(0u64, |w, k| (w << 8) | *boot.get(8 * i + k).unwrap_or(&0) as u64);
            format!("{word:016x}\n")
        })
        .collect())
}
