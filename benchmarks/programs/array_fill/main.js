const n = parseInt(process.argv[2]);
const arr = new Array(n);
for (let i = 0; i < n; i++) arr[i] = (i * 7 + 3) % 1000000;
let sum = 0;
for (const v of arr) sum = (sum + v) % 1000000007;
console.log(sum);
