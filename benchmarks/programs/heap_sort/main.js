function siftDown(arr, start, end) {
    let root = start;
    while (2 * root + 1 <= end) {
        let child = 2 * root + 1;
        let swap = root;
        if (arr[swap] < arr[child]) swap = child;
        if (child + 1 <= end && arr[swap] < arr[child + 1]) swap = child + 1;
        if (swap === root) return;
        const t = arr[root]; arr[root] = arr[swap]; arr[swap] = t;
        root = swap;
    }
}
const n = parseInt(process.argv[2]);
const arr = [];
let seed = 42;
for (let i = 0; i < n; i++) { seed = ((Math.imul(seed, 1103515245) + 12345) >>> 0) % 2147483648; arr.push((seed>>16)&0x7fff); }
for (let s = Math.floor((n-2)/2); s >= 0; s--) siftDown(arr, s, n-1);
for (let end = n-1; end > 0; end--) { const t = arr[0]; arr[0] = arr[end]; arr[end] = t; siftDown(arr, 0, end-1); }
let checksum = 0;
for (const v of arr) checksum = (checksum * 31 + v) & 268435455;
console.log(`${arr[0]} ${arr[n-1]} ${checksum}`);
