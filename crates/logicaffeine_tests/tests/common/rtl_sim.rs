//! A testbench and the Verilog it drives, built for Verilator where its probe finds it (`tools/verilator`, fetched by
//! `scripts/d5005/fetch-verilator.sh`) and for iverilog otherwise, and run in place: the large units' locks simulate
//! their designs at Verilator's speed on a box that has it and stay runnable on one that does not.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A testbench, its top module `tb`, built for one simulator.
pub enum Simulator {
    Verilator(PathBuf),
    Icarus(PathBuf),
}

impl Simulator {
    /// The files `sources` in `dir` built with the module `tb` on top.
    pub fn build(dir: &Path, sources: &[&str]) -> Simulator {
        if crate::common::require_tool("verilator") {
            let built = Command::new(crate::common::verilator())
                .args(["--binary", "--timing", "-Wno-fatal", "-Wno-lint", "-Wno-style", "-Wno-UNOPTFLAT", "--unroll-count", "4096", "--top-module", "tb", "-j", "2"])
                .args(["-CFLAGS", "-O3", "-Mdir", "obj"])
                .args(sources)
                .current_dir(dir)
                .output()
                .expect("verilator");
            assert!(built.status.success(), "verilator: {}", String::from_utf8_lossy(&built.stderr));
            return Simulator::Verilator(dir.join("obj/Vtb"));
        }
        Simulator::icarus(dir, sources)
    }

    /// The files `sources` in `dir` built with the module `tb` on top for iverilog, whatever else the box has: its four
    /// states read a register that no reset or write has reached as X, which Verilator's two states read as a value.
    pub fn icarus(dir: &Path, sources: &[&str]) -> Simulator {
        let built = Command::new("iverilog").args(["-g2012", "-s", "tb", "-o", "tb.vvp"]).args(sources).current_dir(dir).output().expect("iverilog");
        assert!(built.status.success(), "iverilog: {}", String::from_utf8_lossy(&built.stderr));
        Simulator::Icarus(dir.join("tb.vvp"))
    }

    /// The testbench run in `at` with `args`.
    pub fn run(&self, at: &Path, args: &[String]) -> Output {
        match self {
            Simulator::Verilator(bin) => Command::new(bin).args(args).current_dir(at).output().expect("the Verilator model"),
            Simulator::Icarus(vvp) => Command::new("vvp").arg(vvp).args(args).current_dir(at).output().expect("vvp"),
        }
    }
}
