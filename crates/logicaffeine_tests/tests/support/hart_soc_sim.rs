//! The whole computer, `hart_soc`, under iverilog in lockstep with the board model (`hart::soc::Board`). The testbench
//! gives the SoC the board it expects: a DDR2 behind the line port (pages allocated as they are first written, the
//! rest reading zero; every wait zero, or random), the configuration flash as an SPI mode-0 device that answers READ
//! and refuses every other command, the log UART and the terminal link as byte streams, and the DDR2's calibration
//! finishing early in the run.
//!
//! The RTL's commit trace (`C`, `S`, `T` and the device loads `L`, a restart `R`) is replayed on the model one
//! instruction at a time. A device load whose value depends on time — the clock, a queue's state, the DDR2's status,
//! a byte that has or has not arrived on the link — is given to the model as the RTL read it; every other device load
//! must read the same on both. Every store, every byte on the log and the link, and the exit status must agree.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::hart::model::{Bus, BusError, Hart, Step};
use logicaffeine_compile::codegen_sva::hart::{rtl, soc};

// The fingerprint the simulated bitstream reports: the one every test image is built for.
use super::onboard_board::RTL_MD5;

/// What one run's testbench holds and does.
pub struct Setup {
    /// The boot ROM's bytes.
    pub rom: Vec<u8>,
    /// The flash, all 16 MB of it (as the model holds it).
    pub flash: Vec<u8>,
    pub switches: u16,
    pub buttons: u8,
    /// Bytes the terminal sends the SoC over the link, as fast as it takes them.
    pub link_in: Vec<u8>,
    /// Random waits on the DDR2's line port, from this seed.
    pub slow: Option<u32>,
    /// The log UART takes a byte every this many cycles.
    pub log_every: u32,
    /// The longest the run may take, in cycles.
    pub cycles: u64,
}

/// The DDR2 pages (1 KB each) a run may write, and the flash bytes it may hold, in the compiled testbench.
const POOL_PAGES: u64 = 8192;
const FLASH_WINDOW: u64 = 1 << 20;

fn testbench() -> String {
    let (md5_lo, md5_hi) = rtl::fingerprint_words(RTL_MD5);
    format!(
        r#"
`timescale 1ns/1ps
`include "{defines}"
module tb;
  reg clk = 1'b0, rst = 1'b1;
  always #5 clk = ~clk;
  integer seed = 1;
  reg slow = 1'b0;
  integer log_every = 1;
  reg [63:0] cycles_max;
  reg [63:0] cycle = 64'd0;
  reg [15:0] switches = 16'd0;
  reg [4:0] buttons = 5'd0;
  reg [63:0] ddr_status = 64'd0;

  wire mem_req, mem_we, mem_wvalid;
  wire [63:0] mem_addr, mem_wdata;
  reg mem_gnt = 1'b0, mem_wready = 1'b0, mem_rvalid = 1'b0;
  reg [63:0] mem_rdata = 64'd0;
  wire [15:0] leds;
  wire [7:0] log_byte, link_out_byte;
  wire log_valid, link_out_valid, link_in_ready;
  reg log_ready = 1'b0;
  wire [7:0] link_in_byte;
  wire link_in_valid;
  wire qspi_sck, qspi_cs_n, qspi_mosi;
  reg qspi_miso = 1'b1;
  wire exited;
  wire [63:0] exit_status;

  hart_soc #(.RTL_MD5_LO(64'h{md5_lo:016x}), .RTL_MD5_HI(64'h{md5_hi:016x})) dut (
    .clk(clk), .rst(rst),
    .mem_req(mem_req), .mem_we(mem_we), .mem_addr(mem_addr), .mem_gnt(mem_gnt), .mem_wdata(mem_wdata),
    .mem_wvalid(mem_wvalid), .mem_wready(mem_wready), .mem_rvalid(mem_rvalid), .mem_rdata(mem_rdata),
    .ddr_status(ddr_status), .ddr_cal(64'd0),
    .switches(switches), .buttons(buttons), .leds(leds),
    .log_byte(log_byte), .log_valid(log_valid), .log_ready(log_ready),
    .link_out_byte(link_out_byte), .link_out_valid(link_out_valid), .link_out_ready(1'b1),
    .link_in_byte(link_in_byte), .link_in_valid(link_in_valid), .link_in_ready(link_in_ready),
    .qspi_sck(qspi_sck), .qspi_cs_n(qspi_cs_n), .qspi_mosi(qspi_mosi), .qspi_miso(qspi_miso),
    .exited(exited), .exit_status(exit_status));

  // ── the DDR2: 1 KB pages, allocated (zeroed) as they are first written ───────────────────────────────────────
  localparam PAGE_WORDS = 128;
  localparam PAGES = `SOC_RAM_SIZE / (PAGE_WORDS * 8);
  localparam POOL_PAGES = {pool_pages};
  reg [63:0] pool [0:POOL_PAGES * PAGE_WORDS - 1];
  integer page_at [0:PAGES - 1];
  integer pages_used = 0;
  function [63:0] ram_word(input [63:0] w);
    integer p;
    begin
      p = page_at[w / PAGE_WORDS];
      ram_word = (p == 0) ? 64'd0 : pool[(p - 1) * PAGE_WORDS + w % PAGE_WORDS];
    end
  endfunction
  task ram_store(input [63:0] w, input [63:0] v);
    integer k;
    begin
      if (page_at[w / PAGE_WORDS] == 0) begin
        if (pages_used == POOL_PAGES) $fatal(1, "the testbench's DDR2 holds only %0d pages", POOL_PAGES);
        pages_used = pages_used + 1;
        page_at[w / PAGE_WORDS] = pages_used;
        for (k = 0; k < PAGE_WORDS; k = k + 1) pool[(pages_used - 1) * PAGE_WORDS + k] = 64'd0;
      end
      pool[(page_at[w / PAGE_WORDS] - 1) * PAGE_WORDS + w % PAGE_WORDS] = v;
    end
  endtask

  // The line port: a grant, then a write's eight words taken or a read's eight words returned; with SLOW every
  // grant, latency, word and gap waits a random number of cycles.
  localparam M_IDLE = 0, M_GRANTED = 1, M_WRITE = 2, M_READ = 3;
  integer m_state = M_IDLE, m_wait = 0, m_beat = 0;
  reg [63:0] m_line;
  always @(posedge clk) begin
    if (rst) begin
      m_state <= M_IDLE;
      mem_gnt <= 1'b0;
      mem_wready <= 1'b0;
      mem_rvalid <= 1'b0;
      m_wait <= 0;
    end else begin
      mem_gnt <= 1'b0;
      mem_rvalid <= 1'b0;
      case (m_state)
        M_IDLE: if (mem_req) begin
          if (m_wait == 0) begin
            mem_gnt <= 1'b1;
            m_state <= M_GRANTED;
          end else m_wait <= m_wait - 1;
        end
        M_GRANTED: begin
          m_line <= (mem_addr - `SOC_RAM_BASE) >> 3;
          m_beat <= 0;
          m_wait <= slow ? {{$random(seed)}} % 12 : 0;
          if (mem_we) begin
            m_state <= M_WRITE;
            mem_wready <= slow ? $random(seed) : 1'b1;
          end else m_state <= M_READ;
        end
        M_WRITE: begin
          mem_wready <= slow ? $random(seed) : 1'b1;
          if (mem_wvalid && mem_wready) begin
            ram_store(m_line + m_beat, mem_wdata);
            m_beat <= m_beat + 1;
            if (m_beat == 7) begin
              m_state <= M_IDLE;
              mem_wready <= 1'b0;
              m_wait <= slow ? {{$random(seed)}} % 6 : 0;
            end
          end
        end
        default: begin
          if (m_wait != 0) m_wait <= m_wait - 1;
          else if (!slow || ($random(seed) & 3) != 0) begin
            mem_rvalid <= 1'b1;
            mem_rdata <= ram_word(m_line + m_beat);
            m_beat <= m_beat + 1;
            if (m_beat == 7) begin
              m_state <= M_IDLE;
              m_wait <= slow ? {{$random(seed)}} % 6 : 0;
            end
          end
        end
      endcase
    end
  end

  // ── the configuration flash: READ (03h) in SPI mode 0; the window (which may wrap past the last byte) holds the
  // test's bytes, the rest is erased ──
  localparam FLASH_WORDS = {flash_words};
  reg [63:0] flash [0:FLASH_WORDS - 1];
  reg [23:0] flash_at = 24'd0;
  integer flash_len = 0;
  function [7:0] flash_byte(input [23:0] a);
    reg [23:0] o;
    reg [63:0] w;
    begin
      o = a - flash_at;
      if (o < flash_len) begin
        w = flash[o / 8];
        flash_byte = w >> (8 * (o % 8));
      end else flash_byte = 8'hff;
    end
  endfunction
  integer f_bits = 0, f_bit = 0;
  reg [31:0] f_cmd = 32'd0;
  reg [23:0] f_addr = 24'd0;
  reg [7:0] f_byte = 8'hff;
  always @(posedge qspi_cs_n) begin
    f_bits = 0;
    qspi_miso <= 1'b1;
  end
  always @(posedge qspi_sck) if (!qspi_cs_n && f_bits < 32) begin
    f_cmd = {{f_cmd[30:0], qspi_mosi}};
    f_bits = f_bits + 1;
    if (f_bits == 32) begin
      if (f_cmd[31:24] != 8'h03) $fatal(1, "the flash was sent command %02x", f_cmd[31:24]);
      f_addr = f_cmd[23:0];
      f_bit = 0;
    end
  end
  always @(negedge qspi_sck) if (!qspi_cs_n && f_bits == 32) begin
    if (f_bit == 0) f_byte = flash_byte(f_addr);
    qspi_miso <= f_byte[7 - f_bit];
    if (f_bit == 7) begin
      f_bit = 0;
      f_addr = f_addr + 24'd1;
    end else f_bit = f_bit + 1;
  end

  // ── the log, the link, the DDR2's calibration, the end ───────────────────────────────────────────────────────
  reg [7:0] link_in_mem [0:65535];
  integer link_in_len = 0, link_in_next = 0;
  assign link_in_valid = link_in_next < link_in_len;
  assign link_in_byte = link_in_mem[link_in_next];
  reg ended = 1'b0;
  integer after = 0;
  // A hart that traps again and again with nothing completing between has lost its way: stop at once.
  integer traps_in_row = 0;
  always @(posedge clk) begin
    if (dut.cpu.core.e_trap_now) traps_in_row <= traps_in_row + 1;
    else if (dut.cpu.core.m_valid && !dut.cpu.core.m_busy) traps_in_row <= 0;
    if (traps_in_row == 64) begin
      $display("TRAPLOOP");
      $finish;
    end
  end
  always @(posedge clk) begin
    cycle <= cycle + 64'd1;
    log_ready <= (cycle % log_every) == 0;
    if (cycle == 64'd64) ddr_status <= 64'd1;
    if (log_valid && log_ready) $display("U %02x", log_byte);
    if (link_out_valid) $display("K %02x", link_out_byte);
    if (link_in_valid && link_in_ready) link_in_next <= link_in_next + 1;
    if (exited && !ended) begin
      $display("EXIT %016x", exit_status);
      ended <= 1'b1;
    end
    // The board stops at its exit; the log runs on until it has said everything.
    if (ended && !log_valid) begin
      after <= after + 1;
      if (after == 4) $finish;
    end
    if (cycle == cycles_max) begin
      $display("TIMEOUT");
      $finish;
    end
  end

  integer i;
  initial begin
    if (!$value$plusargs("CYCLES=%d", cycles_max)) cycles_max = 64'd10000000;
    if (!$value$plusargs("SEED=%d", seed)) seed = 1;
    if ($test$plusargs("SLOW")) slow = 1'b1;
    if (!$value$plusargs("LOG_EVERY=%d", log_every)) log_every = 1;
    if (!$value$plusargs("SWITCHES=%h", switches)) switches = 16'd0;
    if (!$value$plusargs("BUTTONS=%h", buttons)) buttons = 5'd0;
    if (!$value$plusargs("FLASH_AT=%h", flash_at)) flash_at = 24'd0;
    if (!$value$plusargs("FLASH_LEN=%d", flash_len)) flash_len = 0;
    if (!$value$plusargs("LINK_IN=%d", link_in_len)) link_in_len = 0;
    for (i = 0; i < PAGES; i = i + 1) page_at[i] = 0;
    if (flash_len != 0) $readmemh("flash.hex", flash);
    if (link_in_len != 0) $readmemh("link_in.hex", link_in_mem);
    #20 rst = 1'b0;
  end
endmodule
"#,
        defines = rtl::DEFINES,
        pool_pages = POOL_PAGES,
        flash_words = FLASH_WINDOW / 8,
    )
}

/// The compiled testbench and the directory its sources live in, built once per test binary.
pub struct Sim {
    dir: PathBuf,
    vvp: PathBuf,
}

pub fn sim() -> &'static Sim {
    static SIM: std::sync::OnceLock<Sim> = std::sync::OnceLock::new();
    SIM.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("hart_soc_sim_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in rtl::soc_files() {
            std::fs::write(dir.join(name), text).unwrap();
        }
        std::fs::write(dir.join("tb.v"), testbench()).unwrap();
        let vvp = dir.join("sim");
        let mut cmd = std::process::Command::new("iverilog");
        cmd.args(["-g2012", "-DHART_TRACE", "-I"]).arg(&dir).arg("-o").arg(&vvp).arg(dir.join("tb.v"));
        for (name, _) in rtl::soc_files() {
            if name.ends_with(".v") {
                cmd.arg(dir.join(name));
            }
        }
        let out = cmd.output().expect("run iverilog");
        assert!(out.status.success(), "the SoC does not compile:\n{}", String::from_utf8_lossy(&out.stderr));
        Sim { dir, vvp }
    })
}

/// A record of the RTL's commit trace.
#[derive(Clone, Debug)]
pub enum Rec {
    Commit { pc: u64, raw: u32, rd: usize, value: u64 },
    Trap { pc: u64, cause: u64, tval: u64 },
}

/// The trace between two restarts, in program order.
#[derive(Default, Debug)]
pub struct Epoch {
    pub records: Vec<Rec>,
    pub stores: Vec<(u64, u8, u64)>,
    pub loads: Vec<(u64, u64)>,
}

/// What a run of the SoC did.
pub struct SocRun {
    pub epochs: Vec<Epoch>,
    pub log: Vec<u8>,
    pub link_out: Vec<u8>,
    pub exit: Option<u64>,
    pub timed_out: bool,
    /// The hart trapped sixty-four times in a row with nothing completing between.
    pub trap_loop: bool,
}

impl SocRun {
    pub fn log_text(&self) -> String {
        String::from_utf8_lossy(&self.log).to_string()
    }

    pub fn instructions(&self) -> usize {
        self.epochs.iter().map(|e| e.records.len()).sum()
    }
}

/// The flash outside its longest erased stretch — which, like the flash's address counter, may wrap past the last
/// byte to the first: (first byte, bytes).
fn flash_window(flash: &[u8]) -> (usize, Vec<u8>) {
    let n = flash.len();
    let Some(written) = flash.iter().position(|&b| b != 0xff) else { return (0, Vec::new()) };
    let (mut best, mut best_end, mut erased) = (0usize, written, 0usize);
    for k in 1..=n {
        let i = (written + k) % n;
        if flash[i] == 0xff {
            erased += 1;
            if erased > best {
                best = erased;
                best_end = i;
            }
        } else {
            erased = 0;
        }
    }
    let at = (best_end + 1) % n;
    (at, (0..n - best).map(|k| flash[(at + k) % n]).collect())
}

fn hex_words(bytes: &[u8]) -> String {
    bytes
        .chunks(8)
        .map(|c| {
            let w = c.iter().rev().fold(0u64, |v, &b| (v << 8) | b as u64);
            format!("{w:016x}\n")
        })
        .collect()
}

/// Run the SoC on `setup`; what it did.
pub fn run(setup: &Setup, label: &str) -> SocRun {
    let sim = sim();
    let dir = sim.dir.join(format!("run_{label}"));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("boot_rom.hex"), rtl::rom_hex(&setup.rom).unwrap()).unwrap();
    let (at, window) = flash_window(&setup.flash);
    assert!(window.len() as u64 <= FLASH_WINDOW, "{label}: the flash holds {} bytes; the testbench's window {FLASH_WINDOW}", window.len());
    std::fs::write(dir.join("flash.hex"), hex_words(&window)).unwrap();
    let link: String = setup.link_in.iter().map(|b| format!("{b:02x}\n")).collect();
    std::fs::write(dir.join("link_in.hex"), link).unwrap();
    let mut cmd = std::process::Command::new("vvp");
    cmd.current_dir(&dir).arg(&sim.vvp);
    cmd.arg(format!("+CYCLES={}", setup.cycles))
        .arg(format!("+LOG_EVERY={}", setup.log_every.max(1)))
        .arg(format!("+SWITCHES={:x}", setup.switches))
        .arg(format!("+BUTTONS={:x}", setup.buttons))
        .arg(format!("+FLASH_AT={at:x}"))
        .arg(format!("+FLASH_LEN={}", window.len()))
        .arg(format!("+LINK_IN={}", setup.link_in.len()));
    if let Some(seed) = setup.slow {
        cmd.arg("+SLOW").arg(format!("+SEED={}", seed as i32));
    }
    let out = cmd.output().expect("run vvp");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{label}: vvp failed:\n{}", String::from_utf8_lossy(&out.stderr));
    parse(&String::from_utf8_lossy(&out.stdout))
}

fn parse(stdout: &str) -> SocRun {
    let hex = |s: &str| u64::from_str_radix(s, 16).unwrap();
    let mut run =
        SocRun { epochs: Vec::new(), log: Vec::new(), link_out: Vec::new(), exit: None, timed_out: false, trap_loop: false };
    let mut open = Open::default();
    for line in stdout.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["C", seq, pc, raw, rd, value] => open.records.push((
                seq.parse().unwrap(),
                Rec::Commit { pc: hex(pc), raw: hex(raw) as u32, rd: rd.parse().unwrap(), value: hex(value) },
            )),
            ["T", seq, pc, cause, tval] => open
                .records
                .push((seq.parse().unwrap(), Rec::Trap { pc: hex(pc), cause: cause.parse().unwrap(), tval: hex(tval) })),
            ["S", seq, addr, be, data] => open.stores.push((seq.parse().unwrap(), (hex(addr), hex(be) as u8, hex(data)))),
            ["L", seq, addr, value] => open.loads.push((seq.parse().unwrap(), (hex(addr), hex(value)))),
            ["R"] => run.epochs.push(std::mem::take(&mut open).close()),
            ["U", b] => run.log.push(hex(b) as u8),
            ["K", b] => run.link_out.push(hex(b) as u8),
            ["EXIT", v] => run.exit = Some(hex(v)),
            ["TIMEOUT"] => run.timed_out = true,
            ["TRAPLOOP"] => run.trap_loop = true,
            _ => {}
        }
    }
    run.epochs.push(open.close());
    run
}

/// An epoch's records as the trace prints them, each with its sequence number.
#[derive(Default)]
struct Open {
    records: Vec<(u64, Rec)>,
    stores: Vec<(u64, (u64, u8, u64))>,
    loads: Vec<(u64, (u64, u64))>,
}

impl Open {
    /// Into program order. A store or load whose instruction had not committed when the run stopped has no
    /// counterpart on the model.
    fn close(mut self) -> Epoch {
        self.records.sort_by_key(|(s, _)| *s);
        self.stores.sort_by_key(|(s, _)| *s);
        self.loads.sort_by_key(|(s, _)| *s);
        let committed = self.records.last().map(|(s, _)| *s);
        let keep = |s: u64| committed.is_some_and(|c| s <= c);
        Epoch {
            records: self.records.into_iter().map(|(_, r)| r).collect(),
            stores: self.stores.into_iter().filter(|(s, _)| keep(*s)).map(|(_, x)| x).collect(),
            loads: self.loads.into_iter().filter(|(s, _)| keep(*s)).map(|(_, x)| x).collect(),
        }
    }
}

fn reg(name: &str) -> u64 {
    soc::REGS.iter().find(|r| r.name == name).unwrap_or_else(|| panic!("no register {name}")).addr
}

/// The device registers whose value depends on time, which the model takes from the RTL.
fn timed(addr: u64) -> bool {
    ["SYS_CYCLES", "SYS_FLAGS", "UART_STATUS", "LINK_RX", "LINK_STATUS", "DDR_STATUS", "DDR_CAL"]
        .iter()
        .any(|n| reg(n) == addr)
}

/// The model's bus during a lockstep: the board's devices, every store logged as the RTL presents it, and every device
/// load matched against the RTL's.
struct Replay<'a> {
    devices: &'a mut soc::Devices,
    loads: VecDeque<(u64, u64)>,
    stores: Vec<(u64, u8, u64)>,
    fault: Option<String>,
}

impl Replay<'_> {
    fn fault(&mut self, why: String) {
        self.fault.get_or_insert(why);
    }
}

impl Bus for Replay<'_> {
    fn read(&mut self, addr: u64, size: u8) -> Result<u64, BusError> {
        if !(soc::IO_BASE..soc::IO_BASE + soc::IO_SIZE).contains(&addr) {
            return self.devices.read(addr, size);
        }
        let Some((raddr, rvalue)) = self.loads.pop_front() else {
            self.fault(format!("the model read device {addr:#x}; the RTL read no device"));
            return self.devices.read(addr, size);
        };
        if raddr != addr {
            self.fault(format!("the model read device {addr:#x}; the RTL read {raddr:#x}"));
        }
        if timed(addr) {
            if addr == reg("LINK_RX") && rvalue & 0x100 != 0 {
                let mine = self.devices.link_in.pop_front();
                if mine != Some(rvalue as u8) {
                    self.fault(format!("the RTL took {:#04x} from the link; the model's next byte is {mine:?}", rvalue as u8));
                }
            }
            return Ok(rvalue);
        }
        let mine = self.devices.read(addr, size)?;
        if mine != rvalue {
            self.fault(format!("device {addr:#x} reads {rvalue:#x} on the RTL, {mine:#x} on the model"));
        }
        Ok(mine)
    }

    fn write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), BusError> {
        self.devices.write(addr, size, value)?;
        let lane = addr % 8;
        let mask = ((1u16 << size) - 1) as u8;
        self.stores.push((addr - lane, mask << lane, value << (8 * lane)));
        Ok(())
    }
}

/// Whether `raw` reads the cycle counter (mcycle, cycle or time), which the RTL counts in clock cycles.
fn reads_cycles(raw: u32) -> bool {
    raw & 0x7f == 0x73 && (raw >> 12) & 7 != 0 && matches!(raw >> 20, 0xb00 | 0xc00 | 0xc01)
}

fn masked((a, be, d): (u64, u8, u64)) -> (u64, u8, u64) {
    let mask = (0..8).filter(|i| be & (1 << i) != 0).fold(0u64, |m, i| m | (0xff << (8 * i)));
    (a, be, d & mask)
}

/// Replay `run` on `board` from its reset; the first disagreement, named.
pub fn lockstep(run: &SocRun, board: &mut soc::Board, label: &str) -> Result<(), String> {
    for (e, epoch) in run.epochs.iter().enumerate() {
        if e > 0 {
            if !board.devices.reset {
                return Err(format!("{label}: the RTL restarted; the model did not ask to"));
            }
            board.devices.reset = false;
            board.hart = Hart::new(soc::RESET_PC, soc::memory_map());
        }
        let mut replay = Replay {
            devices: &mut board.devices,
            loads: epoch.loads.iter().copied().collect(),
            stores: Vec::new(),
            fault: None,
        };
        for (n, rec) in epoch.records.iter().enumerate() {
            let hart = &mut board.hart;
            let pc = hart.pc;
            let step = hart.step(&mut replay);
            let ctx = || format!("{label}: epoch {e}, instruction #{n} (model pc {pc:#x}, RTL {rec:x?})");
            if let Some(why) = replay.fault.take() {
                return Err(format!("{}: {why}", ctx()));
            }
            match (rec, step) {
                (Rec::Commit { pc: rpc, raw, rd, value }, Step::Retired) => {
                    if *rpc != pc {
                        return Err(format!("{}: the RTL committed a different pc", ctx()));
                    }
                    if *rd != 0 {
                        if reads_cycles(*raw) {
                            hart.x[*rd] = *value;
                        } else if hart.x[*rd] != *value {
                            return Err(format!("{}: x{rd} is {:#x} on the model", ctx(), hart.x[*rd]));
                        }
                    }
                }
                (Rec::Trap { pc: rpc, cause, tval }, Step::Trapped(t)) => {
                    if *rpc != pc || *cause != t.cause || *tval != t.tval {
                        return Err(format!("{}: the model trapped with {t:?}", ctx()));
                    }
                }
                (_, step) => return Err(format!("{}: the model did {step:?}", ctx())),
            }
            if replay.devices.exit.is_some() && (e + 1 < run.epochs.len() || n + 1 < epoch.records.len()) {
                return Err(format!("{}: the model stopped at its exit; the RTL ran on", ctx()));
            }
        }
        if !replay.loads.is_empty() {
            return Err(format!("{label}: epoch {e}: the RTL read {} more devices than the model", replay.loads.len()));
        }
        if replay.stores.len() != epoch.stores.len() {
            return Err(format!("{label}: epoch {e}: the RTL stored {} times, the model {}", epoch.stores.len(), replay.stores.len()));
        }
        for (k, (r, m)) in epoch.stores.iter().zip(replay.stores.iter()).enumerate() {
            if masked(*r) != masked(*m) {
                return Err(format!("{label}: epoch {e}: store #{k}: RTL {:x?}, model {:x?}", masked(*r), masked(*m)));
            }
        }
    }
    if run.exit != board.devices.exit {
        return Err(format!("{label}: the RTL exited with {:x?}, the model with {:x?}", run.exit, board.devices.exit));
    }
    if run.log != board.devices.uart {
        return Err(format!(
            "{label}: the log differs:\nRTL:\n{}\nmodel:\n{}",
            run.log_text(),
            String::from_utf8_lossy(&board.devices.uart)
        ));
    }
    let model_link: Vec<u8> = board.devices.link_out.iter().copied().collect();
    if run.link_out != model_link {
        return Err(format!("{label}: the link carried {:02x?} from the RTL, {model_link:02x?} from the model", run.link_out));
    }
    Ok(())
}

/// The board model a setup describes, at its reset.
pub fn board_for(setup: &Setup) -> soc::Board {
    let mut board = soc::Board::new(&setup.rom, setup.flash.clone(), RTL_MD5);
    board.devices.switches = setup.switches as u64;
    board.devices.buttons = setup.buttons as u64;
    board.devices.link_in = setup.link_in.iter().copied().collect();
    board
}

pub fn tools() -> bool {
    super::common::require_tool("iverilog") && super::common::require_tool("riscv64-linux-gnu-gcc")
}
