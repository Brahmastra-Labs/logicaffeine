//! Write the hard floating-point oracle persona (`codegen_sva::fp_oracle::persona_sources`) as an accelerator tree for
//! `scripts/d5005/build-afu.sh`: an engine build's tree, its compute clock the user clock divided by `<n>`, whose engine
//! is the oracle (the card's floating-point DSP blocks left to Quartus), its simulation twin the oracle beside a
//! stand-in block computing the family member `<member>`, `<latency>` registers deep in every mode or as deep as each
//! of the latencies says: the multiply's, the add's and the multiply-add's fully registered, then the multiply's and the
//! add's with their output register alone, one step unless given (what `scripts/d5005/run-cosim.sh` runs), and the
//! measurement the card runs (`sw/logos_fp_oracle.c` with its class table `sw/logos_fp_oracle.h`). The
//! oracle's classes and vectors are the discriminator ledger's, `benchmarks/silicon/d5005/fp_discriminators.json`
//! unless `--ledger` names another.
//!
//!     d5005_fp_oracle <out dir> [--divide <n>] [--model <member>@<latency>[,<latency>...]] [--ledger <file>]
//!
//! The divide defaults to 16 and the stand-in to the IEEE member four registers deep.

use std::path::PathBuf;
use std::process::exit;

use logicaffeine_compile::codegen_sva::fp_hypotheses::{family, Semantics};
use logicaffeine_compile::codegen_sva::fp_oracle::{persona_config, persona_sources, Depths, Oracle, TAPS};
use logicaffeine_compile::crypto_accel::sha256;
use logicaffeine_compile::spatial::decode_engine::rtl::engine::ENGINE_MIN_DIVIDE;

fn fail(why: impl std::fmt::Display) -> ! {
    eprintln!("d5005_fp_oracle: {why}");
    exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((out, rest)) = args.split_first() else {
        fail("usage: d5005_fp_oracle <out dir> [--divide <n>] [--model <member>@<latency>] [--ledger <file>]")
    };
    let mut ledger = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/d5005/fp_discriminators.json");
    let (mut divide, mut model, mut depths) = (16u32, Semantics::IEEE, Depths { full: [4; 3], least: [1; 2] });
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--divide" => divide = it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| fail("--divide takes a number")),
            "--ledger" => ledger = PathBuf::from(it.next().unwrap_or_else(|| fail("--ledger takes a file"))),
            "--model" => {
                let m = it.next().unwrap_or_else(|| fail("--model takes <member>@<latency>"));
                let (name, l) = m.split_once('@').unwrap_or_else(|| fail(format!("--model {m}: <member>@<latency>")));
                model = family().into_iter().find(|s| s.name() == name).unwrap_or_else(|| fail(format!("no member of the family is named {name}")));
                let each: Vec<usize> = l
                    .split(',')
                    .map(|x| x.parse().ok().filter(|l| (1..=TAPS).contains(l)).unwrap_or_else(|| fail(format!("a latency of 1 to {TAPS}, not {x}"))))
                    .collect();
                depths = match each[..] {
                    [one] => Depths { full: [one; 3], least: [1; 2] },
                    [m, a, f] => Depths { full: [m, a, f], least: [1; 2] },
                    [m, a, f, m1, a1] => Depths { full: [m, a, f], least: [m1, a1] },
                    _ => fail(format!("--model {m}: one latency; the multiply's, the add's and the multiply-add's; and the least-registered multiply's and add's")),
                };
            }
            other => fail(format!("unknown argument {other}")),
        }
    }
    if divide < ENGINE_MIN_DIVIDE || divide % 2 != 0 {
        fail(format!("a divide of {ENGINE_MIN_DIVIDE} or more, and even, not {divide}"));
    }
    let text = std::fs::read_to_string(&ledger).unwrap_or_else(|e| fail(format!("{}: {e}", ledger.display())));
    let oracle = Oracle::from_ledger(&text).unwrap_or_else(|e| fail(e));
    let root = PathBuf::from(out);
    for f in persona_sources(&oracle, divide, &model, depths) {
        let path = root.join(f.path);
        std::fs::create_dir_all(path.parent().expect("a file's directory")).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
        std::fs::write(&path, f.text).unwrap_or_else(|e| fail(format!("{}: {e}", path.display())));
    }
    let cfg = persona_config(&oracle, divide);
    let hex: String = sha256(oracle.engine_verilog().as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    println!(
        "{}: the oracle of {} product and {} sum classes, {} vectors, its compute clock {} MHz / {divide}; engine sha256 {hex}; simulated beside {} {depths:?} deep",
        root.display(),
        oracle.products.len(),
        oracle.sums.len(),
        oracle.vectors.len(),
        cfg.user_clock_mhz,
        model.name()
    );
}
