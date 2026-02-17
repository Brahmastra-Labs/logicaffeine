const n = parseInt(process.argv[2]);
const arr = new Array(n);
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    arr[i] = (seed >> 16) % 1000;
}
const counts = new Array(1000).fill(0);
for (const v of arr) counts[v]++;
const sorted = [];
for (let v = 0; v < 1000; v++)
    for (let c = 0; c < counts[v]; c++)
        sorted.push(v);
let checksum = 0;
for (const v of sorted) checksum = (checksum + v) % 1000000007;
console.log(`${sorted[0]} ${sorted[n - 1]} ${checksum}`);
