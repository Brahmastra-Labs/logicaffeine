const n = parseInt(process.argv[2]);
let total = 0;
for (let i = 1; i <= n; i++) {
    let k = i;
    while (k !== 1) {
        if (k % 2 === 0) k = k / 2;
        else k = 3 * k + 1;
        total++;
    }
}
console.log(total);
