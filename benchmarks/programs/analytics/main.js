// analytics — filter → map → group-by → reduce over N LCG-generated records.
// Reads N from process.argv[2]; prints two lines (kept count, checksum),
// byte-identical across every language. Every value is an exact integer Number:
// the LCG step keeps only the low 31 bits of seed*1103515245+12345, which
// Math.imul and >>> 0 compute exactly mod 2^32; key and value are below 2^15, so
// value*key+7 < 2^31, and every group total stays below 2*MOD < 2^31.
'use strict';

const MOD = 1000000007;
const NGROUP = 32768;

function main() {
  const n = parseInt(process.argv[2], 10);
  let seed = 42;
  const groups = [];
  for (let g = 0; g < NGROUP; g++) groups.push(0);
  let kept = 0;
  for (let r = 0; r < n; r++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    const key = (seed >> 16) & 0x7fff;
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    const value = (seed >> 16) & 0x7fff;
    if (value % 3 === 0) continue;
    const mapped = (value * key + 7) % MOD;
    const g = key % NGROUP;
    groups[g] = (groups[g] + mapped) % MOD;
    kept++;
  }
  let checksum = 0;
  for (let g = 0; g < NGROUP; g++) {
    checksum = (checksum * 31 + groups[g]) & 268435455;
  }
  process.stdout.write(`${kept}\n${checksum}\n`);
}

main();
