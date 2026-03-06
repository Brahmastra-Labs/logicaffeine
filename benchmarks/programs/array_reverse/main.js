const n = parseInt(process.argv[2]);
const arr = new Array(n);
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    arr[i] = (seed >> 16) & 0x7fff;
}
let lo = 0, hi = n - 1;
while (lo < hi) {
    const tmp = arr[lo]; arr[lo] = arr[hi]; arr[hi] = tmp;
    lo++; hi--;
}
console.log(`${arr[0]} ${arr[n - 1]} ${arr[Math.floor(n / 2)]}`);
