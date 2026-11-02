//! chip_host — the host side of the chips' UART protocol (`codegen_sva::chip_protocol`), as a program a non-Rust
//! host drives. `scripts/logos-chip.py` builds it with the other compiler examples.
//!
//!   cargo run -q -p logicaffeine-compile --example chip_host -- --table
//!
//!   --table    print the protocol's byte table as JSON: record markers, frame bytes, value tags, host calls

use logicaffeine_compile::codegen_sva::chip_protocol::table_json;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--table") => println!("{}", table_json()),
        _ => {
            eprintln!("usage: chip_host --table");
            std::process::exit(2);
        }
    }
}
