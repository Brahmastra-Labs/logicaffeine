const n = parseInt(process.argv[2]);
const text = new Uint8Array(n);
let seed = 42;
for (let pos = 0; pos < n; pos++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    text[pos] = 97 + (seed >> 16) % 5;
}
const needle = new TextEncoder().encode("cabed");
const needleLen = 5;
let count = 0;
const positions = [];
for (let i = 0; i <= n - needleLen; i++) {
    let match = true;
    for (let j = 0; j < needleLen; j++) {
        if (text[i + j] !== needle[j]) { match = false; break; }
    }
    if (match) {
        count++;
        positions.push(i);
    }
}
let checksum = 0;
for (const p of positions) checksum = (checksum * 31 + p) & 268435455;
console.log(`${count} ${checksum}`);
