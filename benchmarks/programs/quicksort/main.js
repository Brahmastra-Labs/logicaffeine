function partition(arr, lo, hi) {
    const pivot = arr[hi]; let i = lo;
    for (let j = lo; j < hi; j++)
        if (arr[j] <= pivot) { const t = arr[i]; arr[i] = arr[j]; arr[j] = t; i++; }
    const t = arr[i]; arr[i] = arr[hi]; arr[hi] = t;
    return i;
}
function qs(arr, lo, hi) {
    if (lo < hi) { const p = partition(arr, lo, hi); qs(arr, lo, p-1); qs(arr, p+1, hi); }
}
const n = parseInt(process.argv[2]);
const arr = [];
let seed = 42;
for (let i = 0; i < n; i++) { seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648; arr.push((seed>>16)&0x7fff); }
qs(arr, 0, n-1);
let checksum = 0;
for (const v of arr) checksum = (checksum * 31 + v) & 268435455;
console.log(`${arr[0]} ${arr[n-1]} ${checksum}`);
