//! Write planted cryptanalysis instances as DIMACS ([`logicaffeine_proof::cipher`]): preimages of SHA-1's compression
//! function cut to `steps` steps with `known` of the block's 512 bits given, or Trivium states recovered from `n`
//! keystream bits with `known` of the 288 state bits given, one file per seed.
//!
//!     cipher_instances sha1 <steps> <known> <seeds> <dir>
//!     cipher_instances trivium <n> <known> <seeds> <dir>

use std::io::Write;

use logicaffeine_proof::cipher::{Sha1Preimage, TriviumRecovery};
use logicaffeine_proof::dimacs::DimacsCnf;

fn write(path: &str, cnf: &DimacsCnf, comment: &str) {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).expect("the instance file opens"));
    writeln!(out, "c {comment}").expect("the instance is written");
    writeln!(out, "p cnf {} {}", cnf.num_vars, cnf.clauses.len()).expect("the instance is written");
    for c in &cnf.clauses {
        for l in c {
            let v = l.var() as i64 + 1;
            write!(out, "{} ", if l.is_positive() { v } else { -v }).expect("the instance is written");
        }
        writeln!(out, "0").expect("the instance is written");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = || -> ! {
        eprintln!("usage: cipher_instances sha1|trivium <steps|n> <known> <seeds> <dir>");
        std::process::exit(2)
    };
    if args.len() != 5 {
        usage();
    }
    let number = |s: &str| s.parse::<usize>().unwrap_or_else(|_| usage());
    let (size, known, seeds, dir) = (number(&args[1]), number(&args[2]), number(&args[3]), &args[4]);
    std::fs::create_dir_all(dir).expect("the directory is made");
    for seed in 0..seeds as u64 {
        match args[0].as_str() {
            "sha1" => {
                let i = Sha1Preimage::new(size, known, seed);
                let path = format!("{dir}/sha1_s{size}_k{known}_{seed}.cnf");
                write(&path, &i.cnf, &format!("SHA-1 compression, {size} steps, {known} block bits given, seed {seed}"));
            }
            "trivium" => {
                let i = TriviumRecovery::new(size, known, seed);
                let path = format!("{dir}/trivium_n{size}_k{known}_{seed}.cnf");
                write(&path, &i.cnf, &format!("Trivium state, {size} keystream bits, {known} state bits given, seed {seed}"));
            }
            _ => usage(),
        }
    }
}
