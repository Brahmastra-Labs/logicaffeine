//! The hart against the RISC-V architecture tests (riscv-tests, vendored under `fixtures/riscv-tests` with its
//! license). Every test in the rv64ui, rv64um, rv64ua, rv64uc and rv64mi suites is assembled for the hart's ISA
//! (RV64IMAC with Zicsr and Zifencei), loaded at 0x8000_0000, and run until it reports through `tohost`: 1 passes,
//! anything else names the failing case. The user-mode suites run in user mode behind the physical-memory-protection
//! entry the test environment opens; the machine-mode suite exercises traps, CSRs and protection directly.
//!
//! Three upstream tests are not vendored because they test what this hart deliberately leaves to software or does
//! not implement: `rv64ui/ma_data` requires misaligned loads and stores to complete in hardware (this hart traps
//! them, and its firmware emulates), `rv64ua/amocas_*` is the Zacas extension, and `rv64mi/breakpoint` probes the
//! Sdtrig debug triggers through `tselect`, a CSR that is absent (and so an illegal instruction) without them.
//!
//! The hart's own contract (`fixtures/hart-contract`) holds what the board's firmware relies on and the architecture
//! tests leave open: a misaligned access traps and does nothing, an illegal instruction's `mtval` is its exact
//! encoding, `mret` leaves MPP at user mode, the counters, the reservations, and protection with MPRV, partial
//! matches and locked entries.
//!
//! Tool-gated: `riscv64-linux-gnu-gcc`.

mod common;
#[path = "support/riscv_elf.rs"]
mod riscv_elf;
#[path = "support/riscv_tests.rs"]
mod riscv_tests;

use riscv_tests::{assemble, tests_of, CONTRACT};

use logicaffeine_compile::codegen_sva::hart::model::{Bus, BusError, Hart, Region};

/// The longest any architecture test runs, in instructions.
const STEP_LIMIT: u64 = 2_000_000;

const RAM_BASE: u64 = 0x8000_0000;
const RAM_BYTES: usize = 1 << 20;

/// The test machine: RAM at 0x8000_0000 and the `tohost` word the test environment reports through.
struct TestBus {
    ram: Vec<u8>,
    tohost: u64,
    verdict: Option<u64>,
}

impl Bus for TestBus {
    fn read(&mut self, addr: u64, size: u8) -> Result<u64, BusError> {
        let o = addr.checked_sub(RAM_BASE).ok_or(BusError)? as usize;
        let bytes = self.ram.get(o..o + size as usize).ok_or(BusError)?;
        Ok(bytes.iter().rev().fold(0u64, |v, &b| (v << 8) | b as u64))
    }

    fn write(&mut self, addr: u64, size: u8, value: u64) -> Result<(), BusError> {
        if addr == self.tohost && value != 0 && self.verdict.is_none() {
            self.verdict = Some(value);
        }
        let o = addr.checked_sub(RAM_BASE).ok_or(BusError)? as usize;
        let bytes = self.ram.get_mut(o..o + size as usize).ok_or(BusError)?;
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (value >> (8 * i)) as u8;
        }
        Ok(())
    }
}

/// Run one test on the model; `Err` names the failing case.
fn on_model(suite: &str, test: &str) -> Result<(), String> {
    let elf = riscv_elf::parse(&assemble(suite, test));
    let tohost = *elf.symbols.get("tohost").expect("the test environment defines tohost");
    let mut bus = TestBus {
        ram: vec![0; RAM_BYTES],
        tohost,
        verdict: None,
    };
    for seg in &elf.segments {
        let o = (seg.addr - RAM_BASE) as usize;
        bus.ram[o..o + seg.bytes.len()].copy_from_slice(&seg.bytes);
    }
    let mut hart = Hart::new(elf.entry, vec![Region::memory(RAM_BASE, RAM_BYTES as u64)]);
    for _ in 0..STEP_LIMIT {
        hart.step(&mut bus);
        if let Some(v) = bus.verdict {
            return if v == 1 {
                Ok(())
            } else {
                Err(format!("{suite}/{test}: case {} failed (tohost {v:#x}, pc {:#x})", v >> 1, hart.pc))
            };
        }
    }
    Err(format!("{suite}/{test}: no verdict after {STEP_LIMIT} instructions (pc {:#x})", hart.pc))
}

fn suite_on_model(suite: &str, at_least: usize) {
    if !common::require_tool("riscv64-linux-gnu-gcc") {
        return;
    }
    let tests = tests_of(suite);
    assert!(tests.len() >= at_least, "{suite} holds {} tests, fewer than the {at_least} vendored", tests.len());
    let failures: Vec<String> = tests.iter().filter_map(|t| on_model(suite, t).err()).collect();
    assert!(
        failures.is_empty(),
        "{} of {} {suite} tests fail on the hart model:\n{}",
        failures.len(),
        tests.len(),
        failures.join("\n")
    );
}

#[test]
fn rv64ui_passes_on_the_model() {
    suite_on_model("rv64ui", 53);
}

#[test]
fn rv64um_passes_on_the_model() {
    suite_on_model("rv64um", 13);
}

#[test]
fn rv64ua_passes_on_the_model() {
    suite_on_model("rv64ua", 19);
}

#[test]
fn rv64uc_passes_on_the_model() {
    suite_on_model("rv64uc", 1);
}

#[test]
fn rv64mi_passes_on_the_model() {
    suite_on_model("rv64mi", 16);
}

#[test]
fn the_hart_contract_holds_on_the_model() {
    suite_on_model(CONTRACT, 6);
}
