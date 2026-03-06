const n = parseInt(process.argv[2]);
let count = 0;
for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
    const cr = 2*x/n - 1.5, ci = 2*y/n - 1;
    let zr = 0, zi = 0, inside = true;
    for (let i = 0; i < 50; i++) {
        const t = zr*zr - zi*zi + cr; zi = 2*zr*zi + ci; zr = t;
        if (zr*zr + zi*zi > 4) { inside = false; break; }
    }
    if (inside) count++;
}
console.log(count);
