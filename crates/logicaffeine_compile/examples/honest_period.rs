//! The period a block must be built at, given the period one of its runs was built at and that run's worst setup
//! slack over every corner — `codegen_sva::signoff_blocks::honest_period_ns`, printed for the flow scripts that
//! search a block's own minimum period (scripts/qor/run-qor-sky130.py), so the rule has one home.
//!
//!   cargo run -q -p logicaffeine-compile --example honest_period -- <period_ns> <worst_setup_slack_ns>

use logicaffeine_compile::codegen_sva::signoff_blocks::honest_period_ns;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let num = |s: &String| s.parse::<f64>().unwrap_or_else(|_| fail(&format!("{s}: not a number of ns")));
    match a.as_slice() {
        [period, slack] => println!("{}", honest_period_ns(num(period), num(slack))),
        _ => fail("usage: honest_period <period_ns> <worst_setup_slack_ns>"),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("honest_period: {msg}");
    std::process::exit(2);
}
