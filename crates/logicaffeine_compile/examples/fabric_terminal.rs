//! fabric_terminal — the Fabric Terminal's protocol table and RTL.
//!
//!   cargo run -q -p logicaffeine-compile --example fabric_terminal -- --table
//!   cargo run -q -p logicaffeine-compile --example fabric_terminal -- --emit benchmarks/silicon
//!
//!   --table      print the link protocol and screen as JSON (what `scripts/logos-terminal.py` reads)
//!   --emit DIR   write the terminal's RTL into DIR (`terminal_rtl::board_files`): the modules, the font ROM and
//!                its license, the pin constraints, the probed fabric engine, and two tops —
//!                `nexys_fabric_terminal` with the engine and `nexys_terminal_shell`, the screen, keyboard and link
//!                alone

use logicaffeine_compile::codegen_sva::{terminal_protocol, terminal_rtl};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--table" => println!("{}", terminal_protocol::table_json()),
        [flag, dir] if flag == "--emit" => {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("create {dir}: {e}"));
            for (name, text) in terminal_rtl::board_files() {
                let path = std::path::Path::new(dir).join(&name);
                std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
                println!("{}", path.display());
            }
        }
        _ => {
            eprintln!("usage: fabric_terminal --table | --emit DIR");
            std::process::exit(2);
        }
    }
}
