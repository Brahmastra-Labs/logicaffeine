const n = parseInt(process.argv[2]);
const counts = new Array(1000).fill(0);
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    counts[((seed >> 16) & 0x7fff) % 1000]++;
}
let maxFreq = 0, maxIdx = 0, distinct = 0;
for (let i = 0; i < 1000; i++) {
    if (counts[i] > 0) distinct++;
    if (counts[i] > maxFreq) { maxFreq = counts[i]; maxIdx = i; }
}
console.log(`${maxFreq} ${maxIdx} ${distinct}`);
