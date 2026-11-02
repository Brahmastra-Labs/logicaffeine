//! Hold a synthesized flat D5005 image to the device before its fit (`codegen_sva::preflight::synthesized_holds`): the
//! ALMs and DSP blocks its synthesis report's partitions estimate, and the M20K blocks the fitter gives its memories,
//! against the 1SX280's 933,120 ALMs, 5,760 DSP blocks and 11,721 M20Ks. It prints what the image needs of each and exits
//! 5 naming each the device cannot hold, or when the report gives no estimate, as `quartus.sh`'s budget does.
//!
//!     d5005_budget <syn report>

use std::process::exit;

use logicaffeine_compile::codegen_sva::preflight::{over, synthesized_holds, DEVICE};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [syn] = &args[..] else {
        eprintln!("usage: d5005_budget <syn report>");
        exit(2)
    };
    let text = std::fs::read_to_string(syn).unwrap_or_else(|e| {
        eprintln!("d5005_budget: {syn}: {e}");
        exit(2)
    });
    let Some(needs) = synthesized_holds(&text) else {
        println!("d5005_budget: the synthesis report {syn} gives no estimate: no fit");
        exit(5)
    };
    let share = |n: u64, d: u64| 100 * n / d;
    println!(
        "d5005_budget: the design needs {} ALMs ({}%), {} DSP blocks ({}%) and {} M20K blocks ({}%)",
        needs.alms,
        share(needs.alms, DEVICE.alms),
        needs.dsp,
        share(needs.dsp, DEVICE.dsp),
        needs.m20k,
        share(needs.m20k, DEVICE.m20k)
    );
    let refused = over(needs, DEVICE);
    if !refused.is_empty() {
        for r in &refused {
            println!("d5005_budget: {r}: no fit");
        }
        exit(5)
    }
}
