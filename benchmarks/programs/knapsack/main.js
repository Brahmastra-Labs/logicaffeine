const n = parseInt(process.argv[2]);
const capacity = n * 5;
const weight = [];
const value = [];
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    weight.push((seed >> 16) % 50 + 1);
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    value.push((seed >> 16) % 100 + 1);
}
let prev = [];
let curr = [];
for (let j = 0; j <= capacity; j++) {
    prev.push(0);
    curr.push(0);
}
for (let i = 0; i < n; i++) {
    const w = weight[i];
    const v = value[i];
    for (let j = 0; j <= capacity; j++) {
        curr[j] = prev[j];
        if (j >= w && prev[j - w] + v > curr[j]) curr[j] = prev[j - w] + v;
    }
    const t = prev; prev = curr; curr = t;
}
let h = 0;
for (let j = 0; j <= capacity; j++) h = (h * 31 + prev[j]) & 268435455;
console.log(prev[capacity] + " " + h);
