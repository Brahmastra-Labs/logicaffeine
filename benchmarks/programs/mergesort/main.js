function mergeSort(arr) {
    if (arr.length < 2) return arr;
    const mid = Math.floor(arr.length / 2);
    const left = mergeSort(arr.slice(0, mid));
    const right = mergeSort(arr.slice(mid));
    const result = [];
    let i = 0, j = 0;
    while (i < left.length && j < right.length) {
        if (left[i] <= right[j]) result.push(left[i++]);
        else result.push(right[j++]);
    }
    while (i < left.length) result.push(left[i++]);
    while (j < right.length) result.push(right[j++]);
    return result;
}
const n = parseInt(process.argv[2]);
let arr = new Array(n);
let seed = 42;
for (let i = 0; i < n; i++) {
    seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648;
    arr[i] = (seed >> 16) & 0x7fff;
}
arr = mergeSort(arr);
let checksum = 0;
for (const v of arr) checksum = (checksum + v) % 1000000007;
console.log(`${arr[0]} ${arr[n - 1]} ${checksum}`);
