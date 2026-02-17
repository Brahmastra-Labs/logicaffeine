function makeCheck(d) { return d === 0 ? 1 : 1 + makeCheck(d-1) + makeCheck(d-1); }
const n = parseInt(process.argv[2]), mn = 4;
let mx = Math.max(mn + 2, n);
console.log(`stretch tree of depth ${mx+1}\t check: ${makeCheck(mx+1)}`);
const ll = makeCheck(mx);
for (let d = mn; d <= mx; d += 2) {
    const it = 1 << (mx - d + mn); let tc = 0;
    for (let i = 0; i < it; i++) tc += makeCheck(d);
    console.log(`${it}\t trees of depth ${d}\t check: ${tc}`);
}
console.log(`long lived tree of depth ${mx}\t check: ${ll}`);
