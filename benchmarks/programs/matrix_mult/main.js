const MOD = 1000000007;
const n = parseInt(process.argv[2]);
const a = new Array(n * n).fill(0);
const b = new Array(n * n).fill(0);
const c = new Array(n * n).fill(0);
for (let i = 0; i < n; i++)
    for (let j = 0; j < n; j++) {
        a[i * n + j] = (i * n + j) % 100;
        b[i * n + j] = (j * n + i) % 100;
    }
for (let i = 0; i < n; i++)
    for (let k = 0; k < n; k++)
        for (let j = 0; j < n; j++)
            c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) % MOD;
let checksum = 0;
for (let i = 0; i < n * n; i++) checksum = (checksum + c[i]) % MOD;
console.log(checksum);
