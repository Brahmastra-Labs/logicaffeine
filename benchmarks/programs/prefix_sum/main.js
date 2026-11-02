const n = parseInt(process.argv[2]);
const arr = [];
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    arr.push(((seed >> 16) & 0x7fff) % 1000);
}
for (let i = 1; i < n; i++) arr[i] = (arr[i] + arr[i - 1]) % 1000000007;
let h = 0;
for (let i = 0; i < n; i++) h = (h * 31 + arr[i]) & 268435455;
console.log(arr[n - 1] + " " + h);
