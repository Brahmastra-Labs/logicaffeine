const n = parseInt(process.argv[2]);
const counts = [];
for (let i = 0; i < 1000; i++) counts.push(0);
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    counts[((seed >> 16) & 0x7fff) % 1000]++;
}
let maxFreq = 0, maxIdx = 0;
for (let i = 0; i < 1000; i++) {
    if (counts[i] > maxFreq) { maxFreq = counts[i]; maxIdx = i; }
}
let h = 0;
for (const c of counts) h = (h * 31 + c) & 268435455;
console.log(`${maxFreq} ${maxIdx} ${h}`);
