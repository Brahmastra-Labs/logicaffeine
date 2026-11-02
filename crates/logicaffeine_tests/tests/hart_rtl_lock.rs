//! The hart's RTL against its executable specification. The decompressor is driven through all 65,536 encodings
//! under iverilog and must agree with `hart::rvc::expand` on each; the core runs the architecture tests and the
//! contract suite, and random programs in lockstep with `hart::model`.
//!
//! Every core run is checked twice: by the program's own verdict (`tohost`, or the end state a fuzz program
//! reaches), and in lockstep — the model takes one step per instruction the RTL commits, and the two must agree on
//! the pc, every register written, every store's address, bytes and data, and every trap's cause and `mtval`.
//! Only a read of the cycle counter is taken from the RTL, which counts clock cycles where the model counts steps.
//!
//! Every program runs on two benches: the core alone over ideal memory, and the core with its caches (`hart_cpu`)
//! over a line-port memory that answers at once or after random waits — so the caches are held to being invisible.
//! Cache programs (`rv_fuzz::cache_program`) work the caches hardest: aliasing dirty lines, reservations across
//! evictions, and code rewritten under fence.i.
//!
//! Tool-gated: `iverilog` (and `riscv64-linux-gnu-gcc` for the programs).

mod common;
#[path = "support/riscv_elf.rs"]
mod riscv_elf;
#[path = "support/riscv_tests.rs"]
mod riscv_tests;
#[path = "support/rv_fuzz.rs"]
mod rv_fuzz;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use logicaffeine_compile::codegen_sva::hart::model::{Bus, BusError, Hart, Region, Step};
use logicaffeine_compile::codegen_sva::hart::{rtl, rvc};

/// A private directory holding every hart RTL file, removed when dropped.
struct Rtl(PathBuf);

impl Rtl {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("hart_rtl_lock_{}_{label}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in rtl::core_files() {
            std::fs::write(dir.join(name), text).unwrap();
        }
        Rtl(dir)
    }

    /// Compile `top` (written beside the RTL as `tb.v`) with the named RTL files and run it; its stdout.
    fn run(&self, tb: &str, files: &[&str]) -> String {
        std::fs::write(self.0.join("tb.v"), tb).unwrap();
        let sim = self.0.join("sim");
        let mut cmd = std::process::Command::new("iverilog");
        cmd.args(["-g2012", "-o"]).arg(&sim).arg(self.0.join("tb.v"));
        for f in files {
            cmd.arg(self.0.join(f));
        }
        let out = cmd.output().expect("run iverilog");
        assert!(out.status.success(), "iverilog failed:\n{}", String::from_utf8_lossy(&out.stderr));
        let out = std::process::Command::new("vvp").arg(&sim).output().expect("run vvp");
        assert!(out.status.success(), "vvp failed:\n{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).to_string()
    }
}

impl Drop for Rtl {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_rtl_decompressor_is_the_model_on_every_encoding() {
    if !common::require_tool("iverilog") {
        return;
    }
    let rtl = Rtl::new("rvc");
    let tb = r#"
module tb;
  reg [15:0] c;
  wire [31:0] inst;
  wire valid;
  hart_rvc dut(.c(c), .inst(inst), .valid(valid));
  integer i;
  initial begin
    for (i = 0; i < 65536; i = i + 1) begin
      c = i;
      #1 $display("%04x %08x %0d", c, inst, valid);
    end
    $finish;
  end
endmodule
"#;
    let out = rtl.run(tb, &["hart_rvc.v"]);
    let mut checked = 0;
    for line in out.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 3 {
            continue;
        }
        let c = u16::from_str_radix(f[0], 16).unwrap();
        if c & 3 == 3 {
            continue;
        }
        let rtl_says = (f[2] == "1").then(|| u32::from_str_radix(f[1], 16).unwrap());
        assert_eq!(rtl_says, rvc::expand(c), "encoding {c:#06x}: the RTL decompressor disagrees with the model");
        checked += 1;
    }
    assert_eq!(checked, 49_152, "the testbench reported {checked} encodings");
}

const RAM_BASE: u64 = 0x8000_0000;
/// The testbench's memory: 4 MB at 0x8000_0000, enough for the architecture tests and the fuzz programs.
const RAM_BYTES: u64 = 4 << 20;
/// The longest an architecture test or fuzz program runs on the RTL, in cycles: a core that loses its way fails
/// here in seconds instead of running on for hours. The cached bench waits on its memory, so it gets more.
const CYCLE_LIMIT: u64 = 200_000;
const CACHED_CYCLE_LIMIT: u64 = 1_000_000;

/// A testbench a program runs on.
#[derive(Clone, Copy, Debug)]
enum Bench {
    /// The core alone over ideal memory.
    Ideal,
    /// The core with its caches (`hart_cpu`) over a line-port memory: every wait zero, or random from `seed`.
    Cached { slow: bool, seed: u64 },
}

impl Bench {
    fn limit(self) -> u64 {
        match self {
            Bench::Ideal => CYCLE_LIMIT,
            Bench::Cached { .. } => CACHED_CYCLE_LIMIT,
        }
    }
}

/// The ideal-memory testbench: an instruction word answers the cycle after its request, a data access completes in
/// the cycle it is presented, fence.i's maintenance finishes the cycle after it is asked for. A write's cycle reads
/// back the complement of memory, so a core that takes data from its own write cycle stores garbage. The image, the
/// reset pc and the `tohost` address arrive as plusargs, so one compiled simulation runs every program.
fn ideal_tb() -> String {
    format!(
        r#"
`timescale 1ns/1ps
module tb;
  localparam [63:0] BASE = 64'h{RAM_BASE:x};
  localparam WORDS = {words};
  reg clk = 1'b0, rst = 1'b1;
  always #5 clk = ~clk;
  reg [63:0] mem [0:WORDS-1];
  reg [63:0] entry, tohost;
  reg done = 1'b0;
  reg stop_on_trap = 1'b0;
  integer after = 0;
  reg [1023:0] image;
  wire if_req; wire [63:0] if_addr; reg if_ready = 1'b0; reg [63:0] if_rdata;
  wire dm_pre_valid; wire [63:0] dm_pre_addr; wire dm_req; wire [63:0] dm_addr; wire dm_we; wire [7:0] dm_be;
  wire [63:0] dm_wdata; wire cm_req; reg cm_done = 1'b0;
  wire [63:0] dm_rdata = dm_we ? ~mem[(dm_addr - BASE) >> 3] : mem[(dm_addr - BASE) >> 3];
  hart_core #(.RAM_BASE(BASE), .RAM_SIZE(64'h{RAM_BYTES:x})) dut (
    .clk(clk), .rst(rst), .reset_pc(entry),
    .if_req(if_req), .if_addr(if_addr), .if_ready(if_ready), .if_rdata(if_rdata),
    .dm_pre_valid(dm_pre_valid), .dm_pre_addr(dm_pre_addr), .dm_req(dm_req), .dm_addr(dm_addr), .dm_we(dm_we),
    .dm_be(dm_be), .dm_wdata(dm_wdata), .dm_ready(1'b1), .dm_rdata(dm_rdata), .cm_req(cm_req), .cm_done(cm_done));
  integer i;
  always @(posedge clk) begin
    if_ready <= if_req;
    if (if_req) if_rdata <= mem[(if_addr - BASE) >> 3];
    cm_done <= cm_req;
    if (dm_req && dm_we) begin
      for (i = 0; i < 8; i = i + 1)
        if (dm_be[i]) mem[(dm_addr - BASE) >> 3][8 * i +: 8] <= dm_wdata[8 * i +: 8];
      if (dm_addr == tohost && dm_be[0] && dm_wdata[31:0] != 32'd0 && !done) begin
        $display("TOHOST %0d", dm_wdata[31:0]);
        done <= 1'b1;
      end
    end
    // A program with no verdict to report ends at its first trap.
    if (stop_on_trap && dut.e_trap_now && !done) done <= 1'b1;
    // The verdict's store is traced as M completes it; run a few cycles more so every record is out.
    if (done) begin
      after <= after + 1;
      if (after == 3) $finish;
    end
  end
  initial begin
    if (!$value$plusargs("IMAGE=%s", image)) $fatal(1, "no IMAGE");
    if (!$value$plusargs("ENTRY=%h", entry)) $fatal(1, "no ENTRY");
    if (!$value$plusargs("TOHOST=%h", tohost)) tohost = 64'hffff_ffff_ffff_fff8;
    if ($test$plusargs("STOP_ON_TRAP")) stop_on_trap = 1'b1;
    for (i = 0; i < WORDS; i = i + 1) mem[i] = 64'd0;
    $readmemh(image, mem);
    #20 rst = 1'b0;
    #({limit} * 10) $display("TIMEOUT");
    $finish;
  end
endmodule
"#,
        words = RAM_BYTES / 8,
        limit = CYCLE_LIMIT
    )
}

/// The cached testbench: `hart_cpu` over a memory behind its line port. The memory grants a request, takes a write's
/// eight words and returns a read's eight words after a latency; with `+SLOW` every grant, latency, word and gap waits
/// a random number of cycles from `+SEED`, without it none does. Plusargs as the ideal bench's.
fn cached_tb() -> String {
    format!(
        r#"
`timescale 1ns/1ps
module tb;
  localparam [63:0] BASE = 64'h{RAM_BASE:x};
  localparam WORDS = {words};
  reg clk = 1'b0, rst = 1'b1;
  always #5 clk = ~clk;
  reg [63:0] mem [0:WORDS-1];
  reg [63:0] entry, tohost;
  reg done = 1'b0;
  reg stop_on_trap = 1'b0;
  integer after = 0;
  integer seed = 1;
  reg slow = 1'b0;
  reg [1023:0] image;
  wire mem_req, mem_we, mem_wvalid, pre_valid, lp_req, lp_we, rom_en;
  wire [63:0] mem_addr, mem_wdata, pre_addr, lp_addr, lp_wdata, rom_index;
  wire [7:0] lp_be;
  reg mem_gnt = 1'b0, mem_wready = 1'b0, mem_rvalid = 1'b0;
  reg [63:0] mem_rdata = 64'd0;
  hart_cpu #(.RAM_BASE(BASE), .RAM_SIZE(64'h{RAM_BYTES:x})) dut (
    .clk(clk), .rst(rst), .reset_pc(entry), .restart(1'b0), .stop(1'b0),
    .pre_valid(pre_valid), .pre_addr(pre_addr), .lp_req(lp_req), .lp_addr(lp_addr), .lp_we(lp_we), .lp_be(lp_be),
    .lp_wdata(lp_wdata), .lp_ready(1'b1), .lp_rdata(64'd0), .rom_en(rom_en), .rom_index(rom_index), .rom_word(64'd0),
    .mem_req(mem_req), .mem_we(mem_we), .mem_addr(mem_addr), .mem_gnt(mem_gnt), .mem_wdata(mem_wdata),
    .mem_wvalid(mem_wvalid), .mem_wready(mem_wready), .mem_rvalid(mem_rvalid), .mem_rdata(mem_rdata));

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
        // The grant is up this cycle: the transaction is the one the port presents now.
        M_GRANTED: begin
          m_line <= (mem_addr - BASE) >> 3;
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
            mem[m_line + m_beat] <= mem_wdata;
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
            mem_rdata <= mem[m_line + m_beat];
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

  integer i;
  always @(posedge clk) begin
    if (dut.core.dm_req && dut.core.dm_we && dut.core.dm_ready && dut.core.dm_addr == tohost && dut.core.dm_be[0]
        && dut.core.dm_wdata[31:0] != 32'd0 && !done) begin
      $display("TOHOST %0d", dut.core.dm_wdata[31:0]);
      done <= 1'b1;
    end
    if (stop_on_trap && dut.core.e_trap_now && !done) done <= 1'b1;
    if (done) begin
      after <= after + 1;
      if (after == 3) $finish;
    end
  end
  initial begin
    if (!$value$plusargs("IMAGE=%s", image)) $fatal(1, "no IMAGE");
    if (!$value$plusargs("ENTRY=%h", entry)) $fatal(1, "no ENTRY");
    if (!$value$plusargs("TOHOST=%h", tohost)) tohost = 64'hffff_ffff_ffff_fff8;
    if ($test$plusargs("STOP_ON_TRAP")) stop_on_trap = 1'b1;
    if ($test$plusargs("SLOW")) slow = 1'b1;
    if (!$value$plusargs("SEED=%d", seed)) seed = 1;
    for (i = 0; i < WORDS; i = i + 1) mem[i] = 64'd0;
    $readmemh(image, mem);
    #20 rst = 1'b0;
    #({limit} * 10) $display("TIMEOUT");
    $finish;
  end
endmodule
"#,
        words = RAM_BYTES / 8,
        limit = CACHED_CYCLE_LIMIT
    )
}

/// A bench's compiled simulation, built once per test binary.
fn sim(bench: Bench) -> &'static Path {
    static IDEAL: OnceLock<PathBuf> = OnceLock::new();
    static CACHED: OnceLock<PathBuf> = OnceLock::new();
    let (cell, label, tb, files) = match bench {
        Bench::Ideal => (&IDEAL, "core", ideal_tb as fn() -> String, rtl::core_files as fn() -> Vec<(&'static str, String)>),
        Bench::Cached { .. } => (&CACHED, "cpu", cached_tb as fn() -> String, rtl::cpu_files as fn() -> Vec<(&'static str, String)>),
    };
    cell.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("hart_rtl_lock_{}_{label}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files() {
            std::fs::write(dir.join(name), text).unwrap();
        }
        std::fs::write(dir.join("tb.v"), tb()).unwrap();
        let sim = dir.join("sim");
        let mut cmd = std::process::Command::new("iverilog");
        cmd.args(["-g2012", "-DHART_TRACE", "-o"]).arg(&sim).arg(dir.join("tb.v"));
        for (name, _) in files() {
            cmd.arg(dir.join(name));
        }
        let out = cmd.output().expect("run iverilog");
        assert!(out.status.success(), "the {label} bench does not compile:\n{}", String::from_utf8_lossy(&out.stderr));
        sim
    })
}

/// A record of the RTL's commit trace.
#[derive(Clone, Debug)]
enum Rec {
    Commit { pc: u64, raw: u32, rd: usize, value: u64 },
    Trap { pc: u64, cause: u64, tval: u64 },
}

struct RtlRun {
    verdict: Option<u64>,
    timed_out: bool,
    records: Vec<Rec>,
    stores: Vec<(u64, u8, u64)>,
}

/// Write `elf`'s segments as a `$readmemh` image of 64-bit words indexed from RAM_BASE.
fn image(elf: &riscv_elf::Elf, path: &Path) {
    let mut words: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
    for seg in &elf.segments {
        for (k, b) in seg.bytes.iter().enumerate() {
            let addr = seg.addr + k as u64;
            let w = words.entry((addr - RAM_BASE) / 8).or_insert(0);
            *w |= (*b as u64) << (8 * (addr % 8));
        }
    }
    let mut text = String::new();
    let mut next = u64::MAX;
    for (i, w) in words {
        if i != next {
            text.push_str(&format!("@{i:x}\n"));
        }
        text.push_str(&format!("{w:016x}\n"));
        next = i + 1;
    }
    std::fs::write(path, text).unwrap();
}

fn run_rtl(bench: Bench, elf_bytes: &[u8], label: &str) -> (riscv_elf::Elf, RtlRun) {
    let elf = riscv_elf::parse(elf_bytes);
    let dir = std::env::temp_dir().join(format!("hart_rtl_lock_{}_{label}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let img = dir.join("image.hex");
    image(&elf, &img);
    let mut cmd = std::process::Command::new("vvp");
    cmd.arg(sim(bench)).arg(format!("+IMAGE={}", img.display())).arg(format!("+ENTRY={:x}", elf.entry));
    match elf.symbols.get("tohost") {
        Some(t) => cmd.arg(format!("+TOHOST={t:x}")),
        None => cmd.arg("+STOP_ON_TRAP"),
    };
    if let Bench::Cached { slow, seed } = bench {
        cmd.arg(format!("+SEED={}", seed as u32 as i32));
        if slow {
            cmd.arg("+SLOW");
        }
    }
    let out = cmd.output().expect("run vvp");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let mut seq_recs: Vec<(u64, Rec)> = Vec::new();
    let mut stores: Vec<(u64, (u64, u8, u64))> = Vec::new();
    let mut verdict = None;
    let mut timed_out = false;
    let hex = |s: &str| u64::from_str_radix(s, 16).unwrap();
    for line in stdout.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        match f.as_slice() {
            ["C", seq, pc, raw, rd, value] => seq_recs.push((
                seq.parse().unwrap(),
                Rec::Commit { pc: hex(pc), raw: hex(raw) as u32, rd: rd.parse().unwrap(), value: hex(value) },
            )),
            ["T", seq, pc, cause, tval] => seq_recs.push((
                seq.parse().unwrap(),
                Rec::Trap { pc: hex(pc), cause: cause.parse().unwrap(), tval: hex(tval) },
            )),
            ["S", seq, addr, be, data] => {
                stores.push((seq.parse().unwrap(), (hex(addr), hex(be) as u8, hex(data))))
            }
            ["TOHOST", v] => verdict = Some(v.parse().unwrap()),
            ["TIMEOUT"] => timed_out = true,
            _ => {}
        }
    }
    seq_recs.sort_by_key(|(s, _)| *s);
    stores.sort_by_key(|(s, _)| *s);
    // A store whose instruction had not committed when the run stopped has no counterpart on the model yet.
    let committed = seq_recs.last().map(|(s, _)| *s).unwrap_or(0);
    stores.retain(|(s, _)| *s <= committed);
    (
        elf,
        RtlRun {
            verdict,
            timed_out,
            records: seq_recs.into_iter().map(|(_, r)| r).collect(),
            stores: stores.into_iter().map(|(_, s)| s).collect(),
        },
    )
}

/// The model's memory, logging every store the way the RTL presents it: (word address, byte mask, shifted data).
struct LogRam {
    bytes: Vec<u8>,
    stores: Vec<(u64, u8, u64)>,
}

impl Bus for LogRam {
    fn read(&mut self, addr: u64, size: u8) -> Result<u64, BusError> {
        let o = addr.checked_sub(RAM_BASE).ok_or(BusError)? as usize;
        let b = self.bytes.get(o..o + size as usize).ok_or(BusError)?;
        Ok(b.iter().rev().fold(0u64, |v, &x| (v << 8) | x as u64))
    }

    fn write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), BusError> {
        let o = addr.checked_sub(RAM_BASE).ok_or(BusError)? as usize;
        let b = self.bytes.get_mut(o..o + size as usize).ok_or(BusError)?;
        for (i, x) in b.iter_mut().enumerate() {
            *x = (value >> (8 * i)) as u8;
        }
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

/// Replay the RTL's commits on the model; the first disagreement, named.
fn lockstep(elf: &riscv_elf::Elf, run: &RtlRun, label: &str) -> Result<(), String> {
    let mut ram = LogRam { bytes: vec![0; RAM_BYTES as usize], stores: Vec::new() };
    for seg in &elf.segments {
        let o = (seg.addr - RAM_BASE) as usize;
        ram.bytes[o..o + seg.bytes.len()].copy_from_slice(&seg.bytes);
    }
    let mut hart = Hart::new(elf.entry, vec![Region::memory(RAM_BASE, RAM_BYTES)]);
    for (n, rec) in run.records.iter().enumerate() {
        let pc = hart.pc;
        let step = hart.step(&mut ram);
        let ctx = || format!("{label}: instruction #{n} (model pc {pc:#x}, RTL {rec:x?})");
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
    }
    if run.stores.len() != ram.stores.len() {
        return Err(format!("{label}: the RTL stored {} times, the model {}", run.stores.len(), ram.stores.len()));
    }
    for (k, (r, m)) in run.stores.iter().zip(ram.stores.iter()).enumerate() {
        let keep = |(a, be, d): (u64, u8, u64)| {
            let mut mask = 0u64;
            for i in 0..8 {
                if be & (1 << i) != 0 {
                    mask |= 0xff << (8 * i);
                }
            }
            (a, be, d & mask)
        };
        if keep(*r) != keep(*m) {
            return Err(format!("{label}: store #{k}: RTL {:x?}, model {:x?}", keep(*r), keep(*m)));
        }
    }
    Ok(())
}

/// The benches every program runs on: ideal memory, and the caches over memory that answers at once and after random
/// waits (the seed differs per program, so each one meets its own timing).
fn benches(n: u64) -> [Bench; 3] {
    [
        Bench::Ideal,
        Bench::Cached { slow: false, seed: 0 },
        Bench::Cached { slow: true, seed: 0x00ca_c4e0 + n },
    ]
}

fn tools() -> bool {
    common::require_tool("iverilog") && common::require_tool("riscv64-linux-gnu-gcc")
}

fn suite_on_rtl(cached: bool, suite: &str, at_least: usize) {
    if !tools() {
        return;
    }
    let tests = riscv_tests::tests_of(suite);
    assert!(tests.len() >= at_least, "{suite} holds {} tests", tests.len());
    let mut failures = Vec::new();
    'tests: for (n, t) in tests.iter().enumerate() {
        let elf = riscv_tests::assemble(suite, t);
        for bench in benches(n as u64).into_iter().filter(|b| matches!(b, Bench::Cached { .. }) == cached) {
            // A core that has lost its way runs every remaining test to the cycle limit; three failures say enough.
            if failures.len() == 3 {
                failures.push("(stopped after three failures)".to_string());
                break 'tests;
            }
            let label = format!("{suite}/{t} on {bench:?}");
            let (elf, run) = run_rtl(bench, &elf, &format!("{suite}_{t}"));
            if let Err(e) = lockstep(&elf, &run, &label) {
                failures.push(e);
            } else if run.timed_out {
                failures.push(format!("{label}: no verdict in {} cycles", bench.limit()));
            } else if run.verdict != Some(1) {
                failures.push(format!("{label}: verdict {:?} (case {:?})", run.verdict, run.verdict.map(|v| v >> 1)));
            }
        }
    }
    assert!(failures.is_empty(), "{} of {} {suite} tests fail on the RTL:\n{}", failures.len(), tests.len(), failures.join("\n"));
}

#[test]
fn rv64ui_passes_on_the_rtl_in_lockstep() {
    suite_on_rtl(false, "rv64ui", 53);
}

#[test]
fn rv64um_passes_on_the_rtl_in_lockstep() {
    suite_on_rtl(false, "rv64um", 13);
}

#[test]
fn rv64ua_passes_on_the_rtl_in_lockstep() {
    suite_on_rtl(false, "rv64ua", 19);
}

#[test]
fn rv64uc_passes_on_the_rtl_in_lockstep() {
    suite_on_rtl(false, "rv64uc", 1);
}

#[test]
fn rv64mi_passes_on_the_rtl_in_lockstep() {
    suite_on_rtl(false, "rv64mi", 16);
}

#[test]
fn the_hart_contract_holds_on_the_rtl_in_lockstep() {
    suite_on_rtl(false, riscv_tests::CONTRACT, 6);
}

#[test]
fn rv64ui_passes_through_the_caches_in_lockstep() {
    suite_on_rtl(true, "rv64ui", 53);
}

#[test]
fn rv64um_passes_through_the_caches_in_lockstep() {
    suite_on_rtl(true, "rv64um", 13);
}

#[test]
fn rv64ua_passes_through_the_caches_in_lockstep() {
    suite_on_rtl(true, "rv64ua", 19);
}

#[test]
fn rv64uc_passes_through_the_caches_in_lockstep() {
    suite_on_rtl(true, "rv64uc", 1);
}

#[test]
fn rv64mi_passes_through_the_caches_in_lockstep() {
    suite_on_rtl(true, "rv64mi", 16);
}

#[test]
fn the_hart_contract_holds_through_the_caches_in_lockstep() {
    suite_on_rtl(true, riscv_tests::CONTRACT, 6);
}

/// Assemble a machine-mode program at the fuzz programs' address.
fn assemble_fuzz(asm: &str, dir: &Path) -> Vec<u8> {
    let src = dir.join("p.s");
    let elf_path = dir.join("p");
    std::fs::write(&src, asm).unwrap();
    let out = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-march=rv64imac_zicsr_zifencei", "-mabi=lp64", "-nostdlib", "-static", "-no-pie", "-Wl,-Ttext-segment=0x80200000"])
        .arg(&src)
        .arg("-o")
        .arg(&elf_path)
        .output()
        .expect("run riscv64-linux-gnu-gcc");
    assert!(out.status.success(), "{}\n{asm}", String::from_utf8_lossy(&out.stderr));
    std::fs::read(&elf_path).unwrap()
}

/// Run each generated program on the ideal or the cached benches in lockstep, up to its first trap — the closing
/// `ecall`; the instructions committed up to that trap across all of them.
fn programs_in_lockstep(what: &str, programs: &[(u64, String)], cached: bool) -> usize {
    let dir = std::env::temp_dir().join(format!("hart_rtl_{what}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut committed = 0usize;
    for (n, (seed, asm)) in programs.iter().enumerate() {
        let elf_bytes = assemble_fuzz(asm, &dir);
        for bench in benches(n as u64).into_iter().filter(|b| matches!(b, Bench::Cached { .. }) == cached) {
            let label = format!("{what} seed {seed} on {bench:?}");
            let (elf, run) = run_rtl(bench, &elf_bytes, &format!("{what}{n}"));
            lockstep(&elf, &run, &label).unwrap_or_else(|e| panic!("{e}\n{asm}"));
            assert!(!run.timed_out, "{label}: the RTL never reached the program's ecall");
            let first_trap = run.records.iter().position(|r| matches!(r, Rec::Trap { .. }));
            assert!(
                matches!(first_trap.map(|i| &run.records[i]), Some(Rec::Trap { cause: 11, .. })),
                "{label}: the program's first trap is not its closing ecall"
            );
            committed += first_trap.unwrap();
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    committed
}

fn seed_from(var: &str, default: u64) -> u64 {
    std::env::var(var).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// Random programs (`support/rv_fuzz.rs`) on the RTL in lockstep with the model, in machine mode, up to the first
/// trap — the `ecall` that ends each program, after it has stored every register.
#[test]
fn random_programs_run_on_the_rtl_in_lockstep() {
    if !tools() {
        return;
    }
    let seed = seed_from("HART_RTL_FUZZ_SEED", 0x5eed_0001);
    let programs: Vec<(u64, String)> = (0..48).map(|n| (seed + n, rv_fuzz::program(seed + n, 300))).collect();
    let committed = programs_in_lockstep("fuzz", &programs, false);
    assert!(committed > 48 * 300, "only {committed} instructions committed across the programs");
}

#[test]
fn random_programs_run_through_the_caches_in_lockstep() {
    if !tools() {
        return;
    }
    let seed = seed_from("HART_RTL_FUZZ_SEED", 0x5eed_0001);
    let programs: Vec<(u64, String)> = (0..48).map(|n| (seed + n, rv_fuzz::program(seed + n, 300))).collect();
    let committed = programs_in_lockstep("fuzz", &programs, true);
    assert!(committed > 2 * 48 * 300, "only {committed} instructions committed across the programs");
}

/// Cache programs (`rv_fuzz::cache_program`): every region aliases on every cache line, so nearly every access
/// evicts a dirty line; reservations span evictions; the code slot is rewritten and run under fence.i. Both benches,
/// so the model, the core alone and the core with its caches all agree.
#[test]
fn cache_programs_run_through_the_caches_in_lockstep() {
    if !tools() {
        return;
    }
    let seed = seed_from("HART_RTL_CACHE_SEED", 0xcac4_e001);
    let programs: Vec<(u64, String)> = (0..32).map(|n| (seed + n, rv_fuzz::cache_program(seed + n, 400))).collect();
    let alone = programs_in_lockstep("cache", &programs, false);
    let cached = programs_in_lockstep("cache", &programs, true);
    assert!(alone > 32 * 400 && cached == 2 * alone, "committed {alone} alone and {cached} through the caches");
}
