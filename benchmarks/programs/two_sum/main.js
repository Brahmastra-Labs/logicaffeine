const n = parseInt(process.argv[2]);
const arr = new Array(n);
let seed = 42;
for (let i = 0; i < n; i++) { seed=((Math.imul(seed,1103515245)+12345)>>>0)%2147483648; arr[i]=((seed>>16)&0x7fff)%n; }
const seen = new Set();
let count = 0;
for (const x of arr) { if (n-x>=0 && seen.has(n-x)) count++; seen.add(x); }
console.log(count);
