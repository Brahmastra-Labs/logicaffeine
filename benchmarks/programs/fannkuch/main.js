const n = parseInt(process.argv[2]);
const perm1 = Array.from({length: n}, (_, i) => i);
const count = new Array(n).fill(0);
let maxFlips = 0, checksum = 0, permCount = 0, r = n;
outer: while (true) {
    while (r > 1) { count[r-1] = r; r--; }
    const perm = [...perm1];
    let flips = 0;
    while (perm[0] !== 0) {
        const k = perm[0] + 1;
        for (let i = 0; i < k >> 1; i++) { const t = perm[i]; perm[i] = perm[k-1-i]; perm[k-1-i] = t; }
        flips++;
    }
    if (flips > maxFlips) maxFlips = flips;
    checksum += (permCount % 2 === 0) ? flips : -flips;
    permCount++;
    while (true) {
        if (r === n) break outer;
        const p0 = perm1[0];
        for (let i = 0; i < r; i++) perm1[i] = perm1[i+1];
        perm1[r] = p0;
        if (--count[r] > 0) break;
        r++;
    }
}
console.log(checksum + "\n" + maxFlips);
