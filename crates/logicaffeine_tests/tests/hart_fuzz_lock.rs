//! The hart model against `qemu-riscv64` on random programs (`support/rv_fuzz.rs`): every program loads its
//! registers with random values biased toward the edges, runs a random RV64IMAC body — compressed forms included,
//! wherever the assembler picks them — and writes its registers and its data buffer out. Under qemu the program is a
//! Linux process; on the model it runs in user mode behind an open protection entry, its `write` and `exit` calls
//! served by this harness. The two outputs must be byte-identical: the architecture tests pin the edge cases one
//! instruction at a time, this pins the instructions against an independent implementation on values no one chose.
//!
//! Tool-gated: `riscv64-linux-gnu-gcc` and `qemu-riscv64`.

mod common;
#[path = "support/riscv_elf.rs"]
mod riscv_elf;
#[path = "support/rv_fuzz.rs"]
mod rv_fuzz;

use logicaffeine_compile::codegen_sva::hart::model::{
    Bus, BusError, Hart, Privilege, Region, Step, CAUSE_USER_ECALL,
};

const PROGRAMS: u64 = 160;
const INSTRUCTIONS: usize = 400;
const RAM_BASE: u64 = 0x8000_0000;
const RAM_BYTES: usize = 32 << 20;
const STEP_LIMIT: u64 = 1_000_000;

struct Ram(Vec<u8>);

impl Bus for Ram {
    fn read(&mut self, addr: u64, size: u8) -> Result<u64, BusError> {
        let o = addr.checked_sub(RAM_BASE).ok_or(BusError)? as usize;
        let bytes = self.0.get(o..o + size as usize).ok_or(BusError)?;
        Ok(bytes.iter().rev().fold(0u64, |v, &b| (v << 8) | b as u64))
    }

    fn write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), BusError> {
        let o = addr.checked_sub(RAM_BASE).ok_or(BusError)? as usize;
        let bytes = self.0.get_mut(o..o + size as usize).ok_or(BusError)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

/// Assemble and link `asm` as a static executable at the firmware's address; the ELF's path.
fn build(asm: &str, dir: &std::path::Path) -> std::path::PathBuf {
    let src = dir.join("p.s");
    let elf = dir.join("p");
    std::fs::write(&src, asm).unwrap();
    let out = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args(["-march=rv64imac", "-mabi=lp64", "-nostdlib", "-static", "-no-pie", "-Wl,-Ttext-segment=0x80200000"])
        .arg(&src)
        .arg("-o")
        .arg(&elf)
        .output()
        .expect("run riscv64-linux-gnu-gcc");
    assert!(out.status.success(), "assembly failed:\n{}\n{asm}", String::from_utf8_lossy(&out.stderr));
    elf
}

fn on_qemu(elf: &std::path::Path) -> Vec<u8> {
    let out = std::process::Command::new("timeout")
        .arg("60")
        .arg("qemu-riscv64")
        .arg(elf)
        .output()
        .expect("run qemu-riscv64");
    assert!(out.status.success(), "qemu-riscv64 failed: {}", String::from_utf8_lossy(&out.stderr));
    out.stdout
}

/// Run the program on the model in user mode, serving `write` (64) and `exit` (93); its standard output.
fn on_model(elf: &[u8]) -> Result<Vec<u8>, String> {
    let elf = riscv_elf::parse(elf);
    let mut ram = Ram(vec![0; RAM_BYTES]);
    for seg in &elf.segments {
        let o = (seg.addr - RAM_BASE) as usize;
        ram.0[o..o + seg.bytes.len()].copy_from_slice(&seg.bytes);
    }
    let mut hart = Hart::new(elf.entry, vec![Region::memory(RAM_BASE, RAM_BYTES as u64)]);
    hart.pmpaddr[0] = (1 << 53) - 1;
    hart.pmpcfg[0] = 0x1f;
    hart.privilege = Privilege::User;
    let mut stdout = Vec::new();
    for _ in 0..STEP_LIMIT {
        match hart.step(&mut ram) {
            Step::Retired => {}
            Step::Trapped(t) if t.cause == CAUSE_USER_ECALL => {
                match hart.x[17] {
                    64 => {
                        let (buf, len) = (hart.x[11], hart.x[12]);
                        for k in 0..len {
                            stdout.push(ram.read(buf + k, 1).map_err(|_| "write from unmapped memory")? as u8);
                        }
                        hart.x[10] = len;
                    }
                    93 => return Ok(stdout),
                    n => return Err(format!("unexpected syscall {n}")),
                }
                hart.pc = hart.mepc + 4;
                hart.privilege = Privilege::User;
            }
            Step::Trapped(t) => return Err(format!("trap {t:?} at {:#x}", hart.mepc)),
        }
    }
    Err(format!("no exit after {STEP_LIMIT} instructions"))
}

#[test]
fn random_programs_run_on_the_model_exactly_as_on_qemu() {
    if !(common::require_tool("riscv64-linux-gnu-gcc") && common::require_tool("qemu-riscv64")) {
        return;
    }
    let seed = std::env::var("HART_FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x4a27_0001u64);
    let dir = std::env::temp_dir().join(format!("hart_fuzz_lock_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut compressed = 0usize;
    for n in 0..PROGRAMS {
        let asm = rv_fuzz::program(seed + n, INSTRUCTIONS);
        let elf = build(&asm, &dir);
        let bytes = std::fs::read(&elf).unwrap();
        let qemu = on_qemu(&elf);
        let model = on_model(&bytes).unwrap_or_else(|e| panic!("program {} (seed {}): {e}\n{asm}", n, seed + n));
        assert_eq!(qemu.len(), 512, "program {}: qemu wrote {} bytes", seed + n, qemu.len());
        if model != qemu {
            let words = |b: &[u8]| -> Vec<u64> {
                b.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect()
            };
            let (q, m) = (words(&qemu), words(&model));
            let diff: Vec<String> = (0..q.len())
                .filter(|&i| q[i] != m.get(i).copied().unwrap_or(!q[i]))
                .map(|i| {
                    let what = if i < 32 { format!("x{i}") } else { format!("buf[{}]", (i - 32) * 8) };
                    format!("{what}: qemu {:#x}, model {:#x}", q[i], m.get(i).copied().unwrap_or(0))
                })
                .collect();
            panic!("program seed {} differs:\n{}\n{asm}", seed + n, diff.join("\n"));
        }
        let dis = std::process::Command::new("riscv64-linux-gnu-objdump")
            .args(["-d", "-M", "no-aliases"])
            .arg(&elf)
            .output()
            .expect("run riscv64-linux-gnu-objdump");
        compressed += String::from_utf8_lossy(&dis.stdout).lines().filter(|l| l.contains("\tc.")).count();
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        compressed as u64 >= PROGRAMS * 20,
        "only {compressed} compressed instructions across {PROGRAMS} programs; the fuzz no longer reaches RVC"
    );
    eprintln!("{PROGRAMS} random programs of {INSTRUCTIONS} instructions ({compressed} compressed): model ≡ qemu");
}
