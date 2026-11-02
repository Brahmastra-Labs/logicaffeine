"use strict";

const MOD = 1000000007;
const NREG = 8;

// Every register stays in [0, MOD) and MOD < 2^30, so every result is an exact integer Number except the MULADD
// product, which reaches 2^60. It splits b = bh*2^15 + bl with bh, bl < 2^15: a*bh, (a*bh % MOD)*2^15 and a*bl are each
// below 2^45, so the sum stays below 2^47 and every step is exact.
function mulAddMod(a, b) {
    return ((a * (b >>> 15)) % MOD * 32768 + a * (b & 32767) + 1) % MOD;
}

function main() {
    const n = Number(process.argv[2]);
    let seed = 42;
    const regs = [];
    for (let i = 0; i < NREG; i++) regs.push(i + 1);
    for (let k = 0; k < n; k++) {
        seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
        const instr = (seed >> 8) & 0x7fffff;
        const op = instr % 8;
        const dst = Math.floor(instr / 8) % NREG;
        const a = Math.floor(instr / 64) % NREG;
        const imm = Math.floor(instr / 512) % 251;
        switch (op) {
            case 0: regs[dst] = imm; break;
            case 1: regs[dst] = (regs[dst] + regs[a]) % MOD; break;
            case 2: regs[dst] = (regs[dst] + MOD - regs[a]) % MOD; break;
            case 3: regs[dst] = mulAddMod(regs[dst], regs[a]); break;
            case 4: regs[dst] = (regs[dst] + imm) % MOD; break;
            case 5: regs[dst] = (regs[dst] * (imm + 1)) % MOD; break;
            case 6: regs[dst] = regs[a]; break;
            case 7: regs[dst] = (regs[dst] + regs[a] + imm) % MOD; break;
        }
    }
    let checksum = 0;
    for (let i = 0; i < NREG; i++) {
        checksum = (checksum * 31 + regs[i]) & 268435455;
    }
    console.log(checksum.toString());
}

main();
