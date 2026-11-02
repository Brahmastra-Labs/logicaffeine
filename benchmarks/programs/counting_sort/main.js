const n = parseInt(process.argv[2]);
const arr = new Float64Array(n);
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    arr[i] = (seed >> 16) % 1000;
}
const counts = new Float64Array(1000);
for (let i = 0; i < n; i++) counts[arr[i]]++;
const sorted = new Float64Array(n);
let k = 0;
for (let v = 0; v < 1000; v++)
    for (let c = 0; c < counts[v]; c++)
        sorted[k++] = v;
let checksum = 0;
for (let i = 0; i < n; i++) checksum = (checksum * 31 + sorted[i]) & 268435455;
console.log(`${sorted[0]} ${sorted[n - 1]} ${checksum}`);
