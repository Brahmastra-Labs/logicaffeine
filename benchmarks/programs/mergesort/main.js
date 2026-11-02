function mergeSort(arr) {
    const n = arr.length;
    if (n < 2) return;
    const mid = Math.floor(n / 2);
    const left = arr.slice(0, mid);
    const right = arr.slice(mid);
    mergeSort(left);
    mergeSort(right);
    let i = 0, j = 0, k = 0;
    while (i < mid && j < n - mid) {
        if (left[i] <= right[j]) arr[k++] = left[i++];
        else arr[k++] = right[j++];
    }
    while (i < mid) arr[k++] = left[i++];
    while (j < n - mid) arr[k++] = right[j++];
}
const n = parseInt(process.argv[2]);
const arr = [];
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    arr.push((seed >> 16) & 0x7fff);
}
mergeSort(arr);
let checksum = 0;
for (const v of arr) checksum = (checksum * 31 + v) & 268435455;
console.log(`${arr[0]} ${arr[n - 1]} ${checksum}`);
