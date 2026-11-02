const MOD = 1000000007;
const n = parseInt(process.argv[2]);
const a = [];
const b = [];
const c = [];
let seed = 42;
for (let i = 0; i < n * n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    a.push((seed >> 16) % 100);
}
for (let i = 0; i < n * n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    b.push((seed >> 16) % 100);
}
for (let i = 0; i < n * n; i++) c.push(0);
for (let i = 0; i < n; i++)
    for (let k = 0; k < n; k++)
        for (let j = 0; j < n; j++)
            c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) % MOD;
let checksum = 0;
for (let i = 0; i < n * n; i++) checksum = (checksum * 31 + c[i]) & 268435455;
console.log(checksum);
