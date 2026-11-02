use std::env;

const MOD: i64 = 1000000007;
const NREG: usize = 8;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut seed: i64 = 42;
    let mut regs = [0i64; NREG];
    for i in 0..NREG {
        regs[i] = i as i64 + 1;
    }
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        let instr = (seed >> 8) & 0x7fffff;
        let op = instr % 8;
        let dst = ((instr / 8) % NREG as i64) as usize;
        let a = ((instr / 64) % NREG as i64) as usize;
        let imm = (instr / 512) % 251;
        match op {
            0 => regs[dst] = imm,
            1 => regs[dst] = (regs[dst] + regs[a]) % MOD,
            2 => regs[dst] = (regs[dst] + MOD - regs[a]) % MOD,
            3 => regs[dst] = (regs[dst] * regs[a] + 1) % MOD,
            4 => regs[dst] = (regs[dst] + imm) % MOD,
            5 => regs[dst] = (regs[dst] * (imm + 1)) % MOD,
            6 => regs[dst] = regs[a],
            _ => regs[dst] = (regs[dst] + regs[a] + imm) % MOD,
        }
    }
    let mut checksum: i64 = 0;
    for i in 0..NREG {
        checksum = (checksum * 31 + regs[i]) & 268435455;
    }
    println!("{}", checksum);
}
