const n = parseInt(process.argv[2]);
const capacity = n * 5;
let prev = new Array(capacity + 1).fill(0);
let curr = new Array(capacity + 1).fill(0);
for (let i = 0; i < n; i++) {
    const w = (i * 17 + 3) % 50 + 1, v = (i * 31 + 7) % 100 + 1;
    for (let j = 0; j <= capacity; j++) {
        curr[j] = prev[j];
        if (j >= w && prev[j - w] + v > curr[j]) curr[j] = prev[j - w] + v;
    }
    [prev, curr] = [curr, prev];
}
console.log(prev[capacity]);
