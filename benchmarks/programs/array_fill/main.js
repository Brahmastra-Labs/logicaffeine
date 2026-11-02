const n = parseInt(process.argv[2]);
const arr = new Float64Array(n);
for (let i = 0; i < n; i++) arr[i] = (i * 7 + 3) % 1000000;
let h = 0;
for (let i = 0; i < n; i++) h = (h * 31 + arr[i]) & 268435455;
console.log(h);
